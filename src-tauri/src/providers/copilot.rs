use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{Datelike, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::db::Db;
use crate::keychain;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::providers::Provider;

pub struct CopilotProvider;

#[derive(Debug, Clone, Serialize)]
pub struct CopilotProbeResult {
    pub ok: bool,
    pub used_requests: i64,
    pub limit_requests: i64,
    pub percent: Option<f64>,
    pub source: Option<String>,
    pub error: Option<String>,
}

struct ParsedPremiumUsage {
    used_requests: i64,
    limit_requests: i64,
    cost_usd: f64,
    line_items: Vec<(String, i64, f64)>,
    period_label: String,
}

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
            None => {
                db.save_quota(&needs_pat(
                    "Add a GitHub PAT with billing read access in Settings",
                ))?;
                return Ok(());
            }
        };

        match fetch_and_parse(&token, settings).await {
            Ok(parsed) => {
                ingest_runs(db, &parsed)?;
                db.save_quota(&quota_from_parsed(&parsed, settings))?;
            }
            Err(e) => {
                db.save_quota(&quota_error(e.to_string()))?;
            }
        }
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
        if keychain::get_secret("github_pat")?.is_none() {
            return Ok(needs_pat(
                "Add a GitHub PAT with billing read scope in Settings",
            ));
        }
        Ok(needs_pat("Save & sync after configuring GitHub username or org"))
    }
}

impl CopilotProvider {
    pub async fn probe(settings: &AppSettings) -> CopilotProbeResult {
        let token = match keychain::get_secret("github_pat") {
            Ok(Some(t)) => t,
            _ => {
                return CopilotProbeResult {
                    ok: false,
                    used_requests: 0,
                    limit_requests: 0,
                    percent: None,
                    source: None,
                    error: Some("GitHub PAT not saved".into()),
                };
            }
        };
        match fetch_and_parse(&token, settings).await {
            Ok(parsed) => {
                let pct = percent(parsed.used_requests, parsed.limit_requests);
                CopilotProbeResult {
                    ok: true,
                    used_requests: parsed.used_requests,
                    limit_requests: parsed.limit_requests,
                    percent: pct,
                    source: Some(parsed.period_label),
                    error: None,
                }
            }
            Err(e) => CopilotProbeResult {
                ok: false,
                used_requests: 0,
                limit_requests: 0,
                percent: None,
                source: None,
                error: Some(e.to_string()),
            },
        }
    }
}

async fn fetch_and_parse(token: &str, settings: &AppSettings) -> Result<ParsedPremiumUsage> {
    let client = reqwest::Client::new();
    let username = resolve_github_username(&client, token, settings).await?;
    let now = Utc::now();
    let year = now.year();
    let month = now.month() as i32;

    let mut errors: Vec<String> = Vec::new();

    if let Some(org) = settings
        .github_org
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        let url = format!(
            "https://api.github.com/organizations/{org}/settings/billing/premium_request/usage?year={year}&month={month}&user={username}"
        );
        match gh_get(&client, token, &url).await {
            Ok(v) => {
                if let Some(parsed) = parse_premium_json(&v, year, now.month()) {
                    return Ok(parsed);
                }
                errors.push(format!("Org {org}: empty usageItems"));
            }
            Err(e) => errors.push(format!("Org {org}: {e}")),
        }
    }

    let user_url = format!(
        "https://api.github.com/users/{username}/settings/billing/premium_request/usage?year={year}&month={month}"
    );
    match gh_get(&client, token, &user_url).await {
        Ok(v) => {
            if let Some(parsed) = parse_premium_json(&v, year, now.month()) {
                return Ok(parsed);
            }
            errors.push("User billing: empty usageItems (Copilot may be org-billed — set GitHub org in Settings)".into());
        }
        Err(e) => errors.push(format!("User billing: {e}")),
    }

    Err(anyhow::anyhow!(errors.join(" · ")))
}

async fn gh_get(client: &reqwest::Client, token: &str, url: &str) -> Result<Value> {
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "token-monitor")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("HTTP {status}: {body}");
    }
    serde_json::from_str(&body).context("parse GitHub JSON")
}

async fn resolve_github_username(
    client: &reqwest::Client,
    token: &str,
    settings: &AppSettings,
) -> Result<String> {
    if let Some(u) = settings
        .github_username
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return Ok(u.trim().to_string());
    }
    if let Ok(u) = std::env::var("GITHUB_USER") {
        if !u.is_empty() {
            return Ok(u);
        }
    }
    let v = gh_get(client, token, "https://api.github.com/user").await?;
    v.get("login")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .context("GitHub /user missing login — set GitHub username in Settings")
}

fn parse_premium_json(v: &Value, year: i32, month: u32) -> Option<ParsedPremiumUsage> {
    if let Some(items) = v.get("usageItems").and_then(|x| x.as_array()) {
        if items.is_empty() {
            return None;
        }
        let mut used = 0i64;
        let mut cost = 0.0;
        let mut line_items = Vec::new();
        for item in items {
            let model = item
                .get("model")
                .and_then(|x| x.as_str())
                .unwrap_or("copilot")
                .to_string();
            let qty = quantity_from_item(item);
            let amt = item
                .get("netAmount")
                .and_then(|x| x.as_f64())
                .or_else(|| item.get("grossAmount").and_then(|x| x.as_f64()))
                .unwrap_or(0.0);
            used += qty;
            cost += amt;
            if qty > 0 {
                line_items.push((model, qty, amt));
            }
        }
        let limit = legacy_included_limit(v).unwrap_or(default_premium_limit());
        let period_label = v
            .get("timePeriod")
            .and_then(|tp| {
                let y = tp.get("year")?.as_i64()?;
                let m = tp.get("month").and_then(|x| x.as_i64()).unwrap_or(month as i64);
                Some(format!("{y}-{m:02}"))
            })
            .unwrap_or_else(|| format!("{year}-{month:02}"));
        return Some(ParsedPremiumUsage {
            used_requests: used,
            limit_requests: limit,
            cost_usd: cost,
            line_items,
            period_label,
        });
    }

    // Legacy daily breakdown (pre-2025 report shape)
    let mut used = 0i64;
    let mut line_items = Vec::new();
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
                    used += requests;
                    if requests > 0 {
                        line_items.push((model, requests, requests as f64 * 0.04));
                    }
                    let _ = date;
                }
            }
        }
    }
    if used == 0 && line_items.is_empty() {
        return None;
    }
    let limit = legacy_included_limit(v).unwrap_or(default_premium_limit());
    Some(ParsedPremiumUsage {
        used_requests: used,
        limit_requests: limit,
        cost_usd: line_items.iter().map(|(_, _, a)| a).sum(),
        line_items,
        period_label: format!("{year}-{month:02}"),
    })
}

fn quantity_from_item(item: &Value) -> i64 {
    item.get("netQuantity")
        .and_then(|x| x.as_f64())
        .or_else(|| item.get("grossQuantity").and_then(|x| x.as_f64()))
        .map(|n| n.round() as i64)
        .unwrap_or(0)
}

fn legacy_included_limit(v: &Value) -> Option<i64> {
    v.get("usageSummary")
        .and_then(|s| s.get("includedRequests"))
        .and_then(|x| x.as_i64())
}

fn default_premium_limit() -> i64 {
    300
}

fn ingest_runs(db: &Db, parsed: &ParsedPremiumUsage) -> Result<()> {
    let started_at = format!("{}T23:59:59Z", parsed.period_label);
    for (model, qty, amt) in &parsed.line_items {
        let run_id = format!("{}-{}", parsed.period_label, model);
        db.upsert_run(&RunRecord {
            provider: "copilot".into(),
            run_id,
            started_at: started_at.clone(),
            model: model.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cache_tokens: 0,
            cost_usd: *amt,
            title: format!("{qty} premium requests"),
            conversation_id: String::new(),
            task_context: String::new(),
            task_inherited: false,
        })?;
    }
    Ok(())
}

fn quota_from_parsed(parsed: &ParsedPremiumUsage, settings: &AppSettings) -> QuotaSnapshot {
    let pct = percent(parsed.used_requests, parsed.limit_requests);
    QuotaSnapshot {
        provider: "copilot".into(),
        label: "GitHub Copilot".into(),
        used_percent_month: pct,
        used_percent_year: None,
        used_tokens_month: Some(parsed.used_requests),
        limit_tokens_month: Some(parsed.limit_requests),
        used_usd_month: Some(parsed.cost_usd),
        limit_usd_month: Some(parsed.limit_requests as f64 * 0.04),
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
        message: Some(format!(
            "{} / {} premium requests · {}",
            parsed.used_requests, parsed.limit_requests, parsed.period_label
        )),
    }
}

fn percent(used: i64, limit: i64) -> Option<f64> {
    if limit > 0 {
        Some(used as f64 / limit as f64 * 100.0)
    } else {
        None
    }
}

fn needs_pat(message: &str) -> QuotaSnapshot {
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
        status: "needs_pat".into(),
        message: Some(message.into()),
    }
}

fn quota_error(message: String) -> QuotaSnapshot {
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
        status: "error".into(),
        message: Some(message),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_usage_items() {
        let v: Value = serde_json::json!({
            "timePeriod": { "year": 2026, "month": 3 },
            "user": "octocat",
            "usageItems": [
                {
                    "product": "Copilot",
                    "model": "gpt-4.1",
                    "netQuantity": 42,
                    "netAmount": 0.0
                },
                {
                    "model": "claude-sonnet",
                    "netQuantity": 8,
                    "netAmount": 0.32
                }
            ]
        });
        let p = parse_premium_json(&v, 2026, 3).expect("parse");
        assert_eq!(p.used_requests, 50);
        assert_eq!(p.line_items.len(), 2);
    }
}
