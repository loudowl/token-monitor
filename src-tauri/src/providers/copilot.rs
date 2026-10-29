use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::db::Db;
use crate::keychain;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::providers::Provider;

pub struct CopilotProvider;

#[async_trait]
impl Provider for CopilotProvider {
    fn id(&self) -> &'static str {
        "copilot"
    }

    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()> {
        if !settings.enabled_providers.iter().any(|p| p == "copilot") {
            return Ok(());
        }
        let token = match keychain::get_secret("github_pat")? {
            Some(t) => t,
            None => return Ok(()),
        };
        let username = settings
            .github_username
            .clone()
            .or_else(|| std::env::var("GITHUB_USER").ok())
            .unwrap_or_else(|| "me".into());

        let url = format!(
            "https://api.github.com/users/{username}/settings/billing/premium_request/usage"
        );
        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(&token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(());
        }

        let v: Value = resp.json().await?;
        if let Some(days) = v.get("usage").and_then(|u| u.as_array()) {
            for day in days {
                let date = day.get("date").and_then(|d| d.as_str()).unwrap_or("");
                if let Some(models) = day.get("models").and_then(|m| m.as_array()) {
                    for m in models {
                        let model = m
                            .get("model")
                            .and_then(|x| x.as_str())
                            .unwrap_or("copilot")
                            .to_string();
                        let requests = m.get("requests").and_then(|x| x.as_i64()).unwrap_or(0);
                        let run_id = format!("{date}-{model}");
                        let cost = requests as f64 * 0.04;
                        db.upsert_run(&RunRecord {
                            provider: "copilot".into(),
                            run_id,
                            started_at: format!("{date}T12:00:00Z"),
                            model,
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_tokens: 0,
                            cost_usd: cost,
                            title: format!("{requests} premium requests"),
                        })?;
                    }
                }
            }
        }

        let quota = quota_from_json(v, settings)?;
        db.save_quota(&quota)?;
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        if !settings.enabled_providers.contains(&"copilot".to_string()) {
            return Ok(disabled());
        }
        if let Ok(list) = db.list_quotas() {
            if let Some(q) = list.into_iter().find(|q| q.provider == "copilot") {
                return Ok(q);
            }
        }
        Ok(QuotaSnapshot {
            provider: "copilot".into(),
            label: "GitHub Copilot".into(),
            used_percent_month: None,
            used_percent_year: None,
            used_tokens_month: None,
            limit_tokens_month: None,
            used_usd_month: Some(db.provider_month_spend("copilot").unwrap_or(0.0)),
            limit_usd_month: Some(10.0),
            primary_window_label: None,
            primary_window_percent: None,
            secondary_window_label: None,
            secondary_window_percent: None,
            resets_at: None,
            status: "needs_pat".into(),
            message: Some("Add a GitHub PAT with billing read scope in Settings".into()),
        })
    }
}

fn quota_from_json(v: Value, settings: &AppSettings) -> Result<QuotaSnapshot> {
    let used = v
        .get("usageSummary")
        .and_then(|s| s.get("usedRequests"))
        .and_then(|x| x.as_i64())
        .or_else(|| {
            v.get("usage")
                .and_then(|u| u.as_array())
                .map(|arr| arr.len() as i64)
        })
        .unwrap_or(0);
    let limit = v
        .get("usageSummary")
        .and_then(|s| s.get("includedRequests"))
        .and_then(|x| x.as_i64())
        .unwrap_or(300);
    let pct = if limit > 0 {
        Some(used as f64 / limit as f64 * 100.0)
    } else {
        None
    };
    Ok(QuotaSnapshot {
        provider: "copilot".into(),
        label: "GitHub Copilot".into(),
        used_percent_month: pct,
        used_percent_year: None,
        used_tokens_month: None,
        limit_tokens_month: None,
        used_usd_month: None,
        limit_usd_month: Some(10.0),
        primary_window_label: Some("Premium requests".into()),
        primary_window_percent: pct,
        secondary_window_label: None,
        secondary_window_percent: None,
        resets_at: None,
        status: if settings.enabled_providers.contains(&"copilot".to_string()) {
            "ok".into()
        } else {
            "disabled".into()
        },
        message: Some(format!("{used} / {limit} premium requests")),
    })
}

fn disabled() -> QuotaSnapshot {
    QuotaSnapshot {
        provider: "copilot".into(),
        label: "GitHub Copilot".into(),
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
