use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::{
    AppSettings, ModelUsagePoint, ModelUsageRow, ModelUsageSeries, QuotaSnapshot, RunRecord,
};

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
        Self::ensure_column(&conn, "runs", "conversation_id", "TEXT NOT NULL DEFAULT ''")?;
        Ok(())
    }

    fn ensure_column(
        conn: &Connection,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<()> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows.flatten() {
            if name == column {
                return Ok(());
            }
        }
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
        Ok(())
    }

    pub fn upsert_run(&self, run: &RunRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO runs (provider, run_id, started_at, model, input_tokens, output_tokens, cache_tokens, cost_usd, title, conversation_id, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(provider, run_id) DO UPDATE SET
               started_at=excluded.started_at,
               model=excluded.model,
               input_tokens=excluded.input_tokens,
               output_tokens=excluded.output_tokens,
               cache_tokens=excluded.cache_tokens,
               cost_usd=excluded.cost_usd,
               title=excluded.title,
               conversation_id=excluded.conversation_id,
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
                run.conversation_id,
                now
            ],
        )?;
        Ok(())
    }

    pub fn list_runs(
        &self,
        provider: Option<&str>,
        model: Option<&str>,
        limit: u32,
        sort: &str,
    ) -> Result<Vec<RunRecord>> {
        let order = match sort {
            "started_at" => "started_at DESC",
            "cost_asc" => "cost_usd ASC, started_at DESC",
            "model" => "model ASC, started_at DESC",
            _ => "cost_usd DESC, started_at DESC",
        };
        let mut conditions: Vec<&str> = Vec::new();
        if provider.is_some() {
            conditions.push("provider = ?1");
        }
        if model.is_some() {
            let clause = if provider.is_some() {
                "model = ?2"
            } else {
                "model = ?1"
            };
            conditions.push(clause);
        }
        let where_sql = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        let limit_param = match (provider.is_some(), model.is_some()) {
            (true, true) => "?3",
            (true, false) | (false, true) => "?2",
            (false, false) => "?1",
        };
        let sql = format!(
            "SELECT provider, run_id, started_at, model, input_tokens, output_tokens, cache_tokens, cost_usd, title, conversation_id
             FROM runs{where_sql} ORDER BY {order} LIMIT {limit_param}"
        );

        let mut out = {
            let conn = self.conn.lock().unwrap();
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
                    conversation_id: row.get(9)?,
                    task_context: String::new(),
                    task_inherited: false,
                })
            };

            let rows = match (provider, model) {
                (Some(p), Some(m)) => stmt.query_map(params![p, m, limit], map)?,
                (Some(p), None) => stmt.query_map(params![p, limit], map)?,
                (None, Some(m)) => stmt.query_map(params![m, limit], map)?,
                (None, None) => stmt.query_map(params![limit], map)?,
            };

            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };

        let conv_ids = crate::task_context::collect_conversation_ids(&out);
        let threads = self.conversation_title_rows(&conv_ids)?;
        crate::task_context::enrich_runs_task_context(&mut out, &threads);
        Ok(out)
    }

    pub fn conversation_title_rows(
        &self,
        conversation_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>> {
        use std::collections::HashMap;
        if conversation_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.conn.lock().unwrap();
        let placeholders: Vec<String> = (1..=conversation_ids.len())
            .map(|i| format!("?{i}"))
            .collect();
        let sql = format!(
            "SELECT conversation_id, started_at, title FROM runs
             WHERE conversation_id != '' AND conversation_id IN ({})
             ORDER BY started_at ASC",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = conversation_ids
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut map: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for r in rows {
            let (conv, started, title) = r?;
            map.entry(conv)
                .or_default()
                .push((started, title));
        }
        Ok(map)
    }

    pub fn list_used_models(&self, provider: Option<&str>) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let sql = if provider.is_some() {
            "SELECT model FROM runs WHERE provider = ?1 GROUP BY model ORDER BY SUM(cost_usd) DESC"
        } else {
            "SELECT model FROM runs GROUP BY model ORDER BY SUM(cost_usd) DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let mut out = Vec::new();
        if let Some(p) = provider {
            let rows = stmt.query_map(params![p], |r| r.get::<_, String>(0))?;
            for r in rows {
                out.push(r?);
            }
        } else {
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            for r in rows {
                out.push(r?);
            }
        }
        Ok(out)
    }

    pub fn provider_runs_in_billing_cycle(
        &self,
        provider: &str,
        anchor_day: u8,
        limit: u32,
    ) -> Result<Vec<RunRecord>> {
        let start = billing_cycle_start(Utc::now(), anchor_day);
        let start_s = start.to_rfc3339();
        let sql = format!(
            "SELECT provider, run_id, started_at, model, input_tokens, output_tokens, cache_tokens, cost_usd, title, conversation_id
             FROM runs
             WHERE provider = ?1 AND started_at >= ?2
             ORDER BY cost_usd DESC, started_at DESC
             LIMIT ?3"
        );
        let mut out = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![provider, start_s, limit], |row| {
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
                    conversation_id: row.get(9)?,
                    task_context: String::new(),
                    task_inherited: false,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };

        let conv_ids = crate::task_context::collect_conversation_ids(&out);
        let threads = self.conversation_title_rows(&conv_ids)?;
        crate::task_context::enrich_runs_task_context(&mut out, &threads);
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
            "SELECT model, COALESCE(SUM(cost_usd), 0), COUNT(*)
             FROM runs
             WHERE provider = ?1 AND started_at >= ?2
             GROUP BY model
             ORDER BY SUM(cost_usd) DESC",
        )?;
        let rows = stmt.query_map(params![provider, start_s], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut items = Vec::new();
        for r in rows {
            items.push(r?);
        }
        let total: f64 = items.iter().map(|(_, c, _)| c).sum();
        Ok(items
            .into_iter()
            .map(|(model, cost_usd, run_count)| {
                let share_percent = if total > 0.0 {
                    (cost_usd / total) * 100.0
                } else {
                    0.0
                };
                ModelUsageRow {
                    model,
                    cost_usd,
                    share_percent,
                    run_count,
                }
            })
            .collect())
    }

    pub fn provider_model_series(
        &self,
        provider: &str,
        anchor_day: u8,
    ) -> Result<ModelUsageSeries> {
        let conn = self.conn.lock().unwrap();
        let start = billing_cycle_start(Utc::now(), anchor_day);
        let start_s = start.to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT substr(started_at, 1, 10), model, COUNT(*), COALESCE(SUM(cost_usd), 0)
             FROM runs
             WHERE provider = ?1 AND started_at >= ?2
             GROUP BY substr(started_at, 1, 10), model
             ORDER BY 1 ASC, 2 ASC",
        )?;
        let rows = stmt.query_map(params![provider, start_s], |r| {
            Ok(ModelUsagePoint {
                day: r.get(0)?,
                model: r.get(1)?,
                run_count: r.get(2)?,
                cost_usd: r.get(3)?,
            })
        })?;
        let mut points = Vec::new();
        for r in rows {
            points.push(r?);
        }
        Ok(ModelUsageSeries {
            cycle_start: start.format("%Y-%m-%d").to_string(),
            points,
        })
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
