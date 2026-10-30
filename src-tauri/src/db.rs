use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::{AppSettings, ModelUsageRow, QuotaSnapshot, RunRecord};

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open() -> Result<Self> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path).context("open sqlite")?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS runs (
              provider TEXT NOT NULL,
              run_id TEXT NOT NULL,
              started_at TEXT NOT NULL,
              model TEXT NOT NULL,
              input_tokens INTEGER NOT NULL DEFAULT 0,
              output_tokens INTEGER NOT NULL DEFAULT 0,
              cache_tokens INTEGER NOT NULL DEFAULT 0,
              cost_usd REAL NOT NULL DEFAULT 0,
              title TEXT NOT NULL DEFAULT '',
              updated_at TEXT NOT NULL,
              PRIMARY KEY (provider, run_id)
            );
            CREATE INDEX IF NOT EXISTS idx_runs_cost ON runs(cost_usd DESC);
            CREATE INDEX IF NOT EXISTS idx_runs_started ON runs(started_at DESC);

            CREATE TABLE IF NOT EXISTS quotas (
              provider TEXT PRIMARY KEY,
              payload TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              payload TEXT NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    pub fn upsert_run(&self, run: &RunRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO runs (provider, run_id, started_at, model, input_tokens, output_tokens, cache_tokens, cost_usd, title, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(provider, run_id) DO UPDATE SET
               started_at=excluded.started_at,
               model=excluded.model,
               input_tokens=excluded.input_tokens,
               output_tokens=excluded.output_tokens,
               cache_tokens=excluded.cache_tokens,
               cost_usd=excluded.cost_usd,
               title=excluded.title,
               updated_at=excluded.updated_at",
            params![
                run.provider,
                run.run_id,
                run.started_at,
                run.model,
                run.input_tokens,
                run.output_tokens,
                run.cache_tokens,
                run.cost_usd,
                run.title,
                now
            ],
        )?;
        Ok(())
    }

    pub fn list_runs(
        &self,
        provider: Option<&str>,
        limit: u32,
        sort: &str,
    ) -> Result<Vec<RunRecord>> {
        let conn = self.conn.lock().unwrap();
        let order = match sort {
            "started_at" => "started_at DESC",
            "cost_asc" => "cost_usd ASC, started_at DESC",
            "model" => "model ASC, started_at DESC",
            _ => "cost_usd DESC, started_at DESC",
        };
        let sql = if let Some(p) = provider {
            format!(
                "SELECT provider, run_id, started_at, model, input_tokens, output_tokens, cache_tokens, cost_usd, title
                 FROM runs WHERE provider = ?1 ORDER BY {order} LIMIT ?2"
            )
        } else {
            format!(
                "SELECT provider, run_id, started_at, model, input_tokens, output_tokens, cache_tokens, cost_usd, title
                 FROM runs ORDER BY {order} LIMIT ?1"
            )
        };

        let mut stmt = conn.prepare(&sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            Ok(RunRecord {
                provider: row.get(0)?,
                run_id: row.get(1)?,
                started_at: row.get(2)?,
                model: row.get(3)?,
                input_tokens: row.get(4)?,
                output_tokens: row.get(5)?,
                cache_tokens: row.get(6)?,
                cost_usd: row.get(7)?,
                title: row.get(8)?,
            })
        };

        let rows = if let Some(p) = provider {
            stmt.query_map(params![p, limit], map)?
        } else {
            stmt.query_map(params![limit], map)?
        };

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn month_spend_usd(&self) -> Result<f64> {
        let conn = self.conn.lock().unwrap();
        let start = billing_cycle_start(Utc::now(), 1);
        let start_s = start.to_rfc3339();
        let v: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM runs WHERE started_at >= ?1",
            params![start_s],
            |r| r.get(0),
        )?;
        Ok(v)
    }

    pub fn provider_model_breakdown(
        &self,
        provider: &str,
        anchor_day: u8,
    ) -> Result<Vec<ModelUsageRow>> {
        let conn = self.conn.lock().unwrap();
        let start = billing_cycle_start(Utc::now(), anchor_day);
        let start_s = start.to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT model, COALESCE(SUM(cost_usd), 0)
             FROM runs
             WHERE provider = ?1 AND started_at >= ?2
             GROUP BY model
             ORDER BY SUM(cost_usd) DESC",
        )?;
        let rows = stmt.query_map(params![provider, start_s], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })?;
        let mut items = Vec::new();
        for r in rows {
            items.push(r?);
        }
        let total: f64 = items.iter().map(|(_, c)| c).sum();
        Ok(items
            .into_iter()
            .map(|(model, cost_usd)| {
                let share_percent = if total > 0.0 {
                    (cost_usd / total) * 100.0
                } else {
                    0.0
                };
                ModelUsageRow {
                    model,
                    cost_usd,
                    share_percent,
                }
            })
            .collect())
    }

    pub fn provider_month_spend(&self, provider: &str) -> Result<f64> {
        let conn = self.conn.lock().unwrap();
        let start = billing_cycle_start(Utc::now(), 1);
        let start_s = start.to_rfc3339();
        let v: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM runs WHERE provider = ?1 AND started_at >= ?2",
            params![provider, start_s],
            |r| r.get(0),
        )?;
        Ok(v)
    }

    pub fn provider_month_tokens(&self, provider: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let start = billing_cycle_start(Utc::now(), 1);
        let start_s = start.to_rfc3339();
        let v: i64 = conn.query_row(
            "SELECT COALESCE(SUM(input_tokens + output_tokens + cache_tokens), 0)
             FROM runs WHERE provider = ?1 AND started_at >= ?2",
            params![provider, start_s],
            |r| r.get(0),
        )?;
        Ok(v)
    }

    pub fn delete_runs_with_title_prefix(&self, provider: &str, prefix: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM runs WHERE provider = ?1 AND title LIKE ?2",
            params![provider, format!("{prefix}%")],
        )?;
        Ok(n)
    }

    pub fn save_quota(&self, quota: &QuotaSnapshot) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let payload = serde_json::to_string(quota)?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO quotas (provider, payload, updated_at) VALUES (?1,?2,?3)
             ON CONFLICT(provider) DO UPDATE SET payload=excluded.payload, updated_at=excluded.updated_at",
            params![quota.provider, payload, now],
        )?;
        Ok(())
    }

    pub fn list_quotas(&self) -> Result<Vec<QuotaSnapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT payload FROM quotas ORDER BY provider")?;
        let rows = stmt.query_map([], |r| {
            let s: String = r.get(0)?;
            Ok(s)
        })?;
        let mut out = Vec::new();
        for r in rows {
            let s = r?;
            if let Ok(q) = serde_json::from_str::<QuotaSnapshot>(&s) {
                out.push(q);
            }
        }
        Ok(out)
    }

    pub fn load_settings(&self) -> Result<AppSettings> {
        let conn = self.conn.lock().unwrap();
        let res: Result<String, _> = conn.query_row(
            "SELECT payload FROM settings WHERE id = 1",
            [],
            |r| r.get(0),
        );
        match res {
            Ok(s) => Ok(serde_json::from_str(&s)?),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(AppSettings::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let payload = serde_json::to_string(settings)?;
        conn.execute(
            "INSERT INTO settings (id, payload) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
            params![payload],
        )?;
        Ok(())
    }
}

pub fn db_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("token-monitor")
        .join("token-monitor.db")
}

pub fn billing_cycle_start(now: DateTime<Utc>, anchor_day: u8) -> DateTime<Utc> {
    use chrono::{Datelike, TimeZone, Timelike};
    let day = anchor_day.clamp(1, 28) as u32;
    let (y, m) = (now.year(), now.month());
    let this_anchor = Utc
        .with_ymd_and_hms(y, m, day, 0, 0, 0)
        .single()
        .unwrap_or(now);
    if now >= this_anchor {
        this_anchor
    } else {
        let prev = now - chrono::Months::new(1);
        Utc.with_ymd_and_hms(prev.year(), prev.month(), day, 0, 0, 0)
            .single()
            .unwrap_or(this_anchor)
    }
}

pub fn projected_month_spend(month_spend: f64, cycle_start: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    let elapsed = (now - cycle_start).num_seconds().max(1) as f64;
    let month_secs = 30.0 * 24.0 * 3600.0;
    if elapsed >= month_secs {
        return month_spend;
    }
    month_spend * (month_secs / elapsed)
}
