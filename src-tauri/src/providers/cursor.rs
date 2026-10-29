use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::Value;
use std::path::PathBuf;

use crate::db::{billing_cycle_start, Db};
use crate::keychain;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::providers::Provider;

pub struct CursorProvider;

fn cursor_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("Application Support")
            .join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb")
    })
}

fn read_cursor_access_token() -> Result<Option<String>> {
    let Some(path) = cursor_db_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .context("open cursor vscdb")?;
    let key = "cursorAuth/accessToken";
    let res: Result<String, _> = conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        [key],
        |r| r.get(0),
    );
    match res {
        Ok(raw) => {
            let trimmed = raw.trim_matches('"').to_string();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[async_trait]
impl Provider for CursorProvider {
    fn id(&self) -> &'static str {
        "cursor"
    }

    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()> {
        if !settings.enabled_providers.iter().any(|p| p == "cursor") {
            return Ok(());
        }
        let token = match read_cursor_access_token()? {
            Some(t) => t,
            None => return Ok(()),
        };

        let client = reqwest::Client::new();
        let cycle = billing_cycle_start(Utc::now(), settings.billing_anchor_day);
        let body = serde_json::json!({
            "teamId": 0,
            "startDate": cycle.format("%Y-%m-%d").to_string(),
            "endDate": Utc::now().format("%Y-%m-%d").to_string(),
            "page": 1,
            "pageSize": 100
        });

        let resp = client
            .post("https://www.cursor.com/api/dashboard/get-filtered-usage-events")
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            _ => return Ok(()),
        };

        let v: Value = resp.json().await.unwrap_or(Value::Null);
        let events = v
            .get("usageEventsDisplay")
            .or_else(|| v.get("events"))
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();

        for ev in events {
            let run_id = ev
                .get("requestId")
                .or_else(|| ev.get("id"))
                .and_then(|x| x.as_str())
                .unwrap_or_else(|| "unknown")
                .to_string();
            let model = ev
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or("cursor")
                .to_string();
            let input = ev.get("inputTokens").and_then(|x| x.as_i64()).unwrap_or(0)
                + ev
                    .get("promptTokens")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
            let output = ev
                .get("outputTokens")
                .or_else(|| ev.get("completionTokens"))
                .and_then(|x| x.as_i64())
                .unwrap_or(0);
            let cost = ev
                .get("costCents")
                .and_then(|x| x.as_f64())
                .map(|c| c / 100.0)
                .or_else(|| {
                    ev.get("cost")
                        .and_then(|x| x.as_f64())
                })
                .unwrap_or(0.0);
            let started = ev
                .get("timestamp")
                .or_else(|| ev.get("createdAt"))
                .and_then(|x| x.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc).to_rfc3339())
                .unwrap_or_else(|| Utc::now().to_rfc3339());

            db.upsert_run(&RunRecord {
                provider: "cursor".into(),
                run_id,
                started_at: started,
                model,
                input_tokens: input,
                output_tokens: output,
                cache_tokens: 0,
                cost_usd: cost,
                title: "Cursor request".into(),
            })?;
        }

        if let Ok(usage_resp) = client
            .get("https://www.cursor.com/api/usage")
            .bearer_auth(&token)
            .send()
            .await
        {
            if usage_resp.status().is_success() {
                if let Ok(usage) = usage_resp.json::<Value>().await {
                    let quota = build_quota_from_usage(usage, db, settings)?;
                    db.save_quota(&quota)?;
                }
            }
        }

        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        if !settings.enabled_providers.contains(&"cursor".to_string()) {
            return Ok(disabled("cursor", "Cursor"));
        }
        if let Ok(list) = db.list_quotas() {
            if let Some(q) = list.into_iter().find(|q| q.provider == "cursor") {
                return Ok(q);
            }
        }
        let spend = db.provider_month_spend("cursor").unwrap_or(0.0);
        Ok(QuotaSnapshot {
            provider: "cursor".into(),
            label: "Cursor".into(),
            used_percent_month: Some((spend / 20.0 * 100.0).min(999.0)),
            used_percent_year: None,
            used_tokens_month: Some(db.provider_month_tokens("cursor").unwrap_or(0)),
            limit_tokens_month: None,
            used_usd_month: Some(spend),
            limit_usd_month: Some(20.0),
            primary_window_label: None,
            primary_window_percent: None,
            secondary_window_label: None,
            secondary_window_percent: None,
            resets_at: None,
            status: "ok".into(),
            message: Some("Usage API may be unavailable; showing estimates from events".into()),
        })
    }
}

fn build_quota_from_usage(usage: Value, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
    let spend = db.provider_month_spend("cursor").unwrap_or(0.0);
    let limit = usage
        .get("hardLimit")
        .or_else(|| usage.get("limit"))
        .and_then(|x| x.as_f64())
        .unwrap_or(20.0);
    let used_pct = if limit > 0.0 {
        Some((spend / limit * 100.0).min(999.0))
    } else {
        None
    };
    Ok(QuotaSnapshot {
        provider: "cursor".into(),
        label: "Cursor".into(),
        used_percent_month: used_pct,
        used_percent_year: None,
        used_tokens_month: Some(db.provider_month_tokens("cursor").unwrap_or(0)),
        limit_tokens_month: None,
        used_usd_month: Some(spend),
        limit_usd_month: Some(limit),
        primary_window_label: None,
        primary_window_percent: None,
        secondary_window_label: None,
        secondary_window_percent: None,
        resets_at: usage
            .get("billingCycleEnd")
            .and_then(|x| x.as_str())
            .map(String::from),
        status: "ok".into(),
        message: None,
    })
}

fn disabled(provider: &str, label: &str) -> QuotaSnapshot {
    QuotaSnapshot {
        provider: provider.into(),
        label: label.into(),
        used_percent_month: None,
        used_percent_year: None,
        used_tokens_month: None,
        limit_tokens_month: None,
        used_usd_month: None,
        limit_usd_month: None,
        primary_window_label: None,
        primary_window_percent: None,
        secondary_window_label: None,
        secondary_window_percent: None,
        resets_at: None,
        status: "disabled".into(),
        message: None,
    }
}

// silence unused import warning for keychain in future token override
#[allow(dead_code)]
fn _cursor_token_override() -> Option<String> {
    keychain::get_secret("cursor_token").ok().flatten()
}
