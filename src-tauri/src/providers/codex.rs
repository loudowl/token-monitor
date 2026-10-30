use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::db::Db;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::pricing::estimate_cost_usd;
use crate::providers::Provider;

pub struct CodexProvider;

#[derive(Default, Clone)]
struct SessionAgg {
    model: String,
    started_at: Option<DateTime<Utc>>,
    input: i64,
    output: i64,
    cache: i64,
    title: String,
    primary_pct: Option<f64>,
    secondary_pct: Option<f64>,
    plan_type: Option<String>,
    line_time: Option<DateTime<Utc>>,
}

#[derive(Default, Clone)]
struct RateLimitPick {
    primary_pct: Option<f64>,
    secondary_pct: Option<f64>,
    plan_type: Option<String>,
    message: Option<String>,
    resets_at: Option<String>,
    line_time: Option<DateTime<Utc>>,
    source: &'static str,
}

#[derive(Deserialize)]
struct CodexAuthFile {
    tokens: Option<CodexAuthTokens>,
}

#[derive(Deserialize)]
struct CodexAuthTokens {
    access_token: String,
    account_id: Option<String>,
}

#[async_trait]
impl Provider for CodexProvider {
    fn id(&self) -> &'static str {
        "codex"
    }

    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()> {
        if !settings.enabled_providers.iter().any(|p| p == "codex") {
            return Ok(());
        }

        let mut best_session = RateLimitPick::default();

        for base in session_roots() {
            if !base.exists() {
                continue;
            }
            for entry in WalkDir::new(&base)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
                        .unwrap_or(false)
                })
            {
                let path = entry.path();
                let run_id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                if let Ok(agg) = parse_rollout(path) {
                    if let Some(pick) = rate_pick_from_session(&agg) {
                        if is_newer(&pick, &best_session) {
                            best_session = pick;
                        }
                    }
                    let started = agg.started_at.unwrap_or_else(Utc::now).to_rfc3339();
                    let model = if agg.model.is_empty() {
                        "codex".into()
                    } else {
                        agg.model.clone()
                    };
                    let cost = estimate_cost_usd(&model, agg.input, agg.output, agg.cache);
                    db.upsert_run(&RunRecord {
                        provider: "codex".into(),
                        run_id,
                        started_at: started,
                        model,
                        input_tokens: agg.input,
                        output_tokens: agg.output,
                        cache_tokens: agg.cache,
                        cost_usd: cost,
                        title: agg.title,
                        conversation_id: String::new(),
                        task_context: String::new(),
                        task_inherited: false,
                    })?;
                }
            }
        }

        let mut best = best_session;
        if let Ok(live) = fetch_live_quota().await {
            if is_newer(&live, &best) || live.primary_pct.is_some() {
                best = live;
            }
        }

        let quota = build_quota(&best, db, settings)?;
        db.save_quota(&quota)?;
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        if !settings.enabled_providers.contains(&"codex".to_string()) {
            return Ok(disabled());
        }
        if let Ok(list) = db.list_quotas() {
            if let Some(q) = list.into_iter().find(|q| q.provider == "codex") {
                return Ok(q);
            }
        }
        Ok(build_quota(&RateLimitPick::default(), db, settings)?)
    }
}

fn session_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(h) = dirs::home_dir() {
        roots.push(h.join(".codex").join("sessions"));
        roots.push(h.join(".codex").join("archived_sessions"));
    }
    roots
}

fn codex_auth_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("auth.json"))
}

fn wham_usage_url() -> String {
    if let Some(h) = dirs::home_dir() {
        let cfg = h.join(".codex").join("config.toml");
        if let Ok(raw) = std::fs::read_to_string(&cfg) {
            for line in raw.lines() {
                let line = line.trim();
                if line.starts_with("chatgpt_base_url") {
                    if let Some(url) = line.split('=').nth(1) {
                        let base = url.trim().trim_matches('"').trim_end_matches('/');
                        if !base.is_empty() {
                            if base.contains("chatgpt.com") {
                                return format!("{base}/wham/usage");
                            }
                            return format!("{base}/api/codex/usage");
                        }
                    }
                }
            }
        }
    }
    "https://chatgpt.com/backend-api/wham/usage".into()
}

async fn fetch_live_quota() -> Result<RateLimitPick> {
    let path = codex_auth_path().context("home dir")?;
    let raw = std::fs::read_to_string(&path).context("read codex auth.json")?;
    let auth: CodexAuthFile = serde_json::from_str(&raw).context("parse auth.json")?;
    let tokens = auth.tokens.context("auth.json missing tokens — sign in via Codex CLI")?;
    let client = reqwest::Client::new();
    let mut req = client
        .get(wham_usage_url())
        .header("Accept", "application/json")
        .header("User-Agent", "token-monitor")
        .bearer_auth(&tokens.access_token);
    if let Some(id) = tokens
        .account_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        req = req.header("ChatGPT-Account-Id", id.as_str());
    }
    let resp = req.send().await.context("wham/usage request")?;
    if !resp.status().is_success() {
        anyhow::bail!("wham/usage HTTP {}", resp.status());
    }
    let v: Value = resp.json().await.context("wham/usage json")?;
    parse_wham_usage(&v).context("parse wham/usage")
}

fn parse_wham_usage(v: &Value) -> Result<RateLimitPick> {
    let plan_type = v
        .get("plan_type")
        .and_then(|x| x.as_str())
        .map(String::from);

    if let Some(il) = v.pointer("/spend_control/individual_limit") {
        let used_pct = il.get("used_percent").and_then(as_f64);
        if used_pct.is_some() {
            let used = il.get("used").and_then(|x| x.as_str()).unwrap_or("0");
            let limit = il.get("limit").and_then(|x| x.as_str()).unwrap_or("?");
            let unit = il.get("unit").and_then(|x| x.as_str()).unwrap_or("credit");
            let reset_at = il
                .get("reset_at")
                .and_then(|x| x.as_i64())
                .and_then(unix_to_rfc3339);
            return Ok(RateLimitPick {
                primary_pct: used_pct,
                secondary_pct: None,
                plan_type: plan_type.clone(),
                message: Some(format!("{used} / {limit} {unit}s")),
                resets_at: reset_at,
                line_time: Some(Utc::now()),
                source: "live",
            });
        }
    }

    if let Some(rl) = v.get("rate_limit").filter(|x| !x.is_null()) {
        let primary = window_pct(rl.get("primary_window").or_else(|| rl.get("primary")));
        let secondary = window_pct(rl.get("secondary_window").or_else(|| rl.get("secondary")));
        if primary.is_some() || secondary.is_some() {
            return Ok(RateLimitPick {
                primary_pct: primary,
                secondary_pct: secondary,
                plan_type: plan_type.clone(),
                message: plan_type.map(|p| format!("Plan: {p}")),
                resets_at: None,
                line_time: Some(Utc::now()),
                source: "live",
            });
        }
    }

    if let Some(reached) = v
        .get("rate_limit_reached_type")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        return Ok(RateLimitPick {
            primary_pct: Some(100.0),
            secondary_pct: None,
            plan_type,
            message: Some(format!("Limit: {reached}")),
            resets_at: None,
            line_time: Some(Utc::now()),
            source: "live",
        });
    }

    anyhow::bail!("wham/usage had no usable quota windows")
}

fn window_pct(w: Option<&Value>) -> Option<f64> {
    let w = w.filter(|x| !x.is_null())?;
    w.get("used_percent")
        .or_else(|| w.get("usedPercent"))
        .and_then(as_f64)
}

fn rate_pick_from_session(agg: &SessionAgg) -> Option<RateLimitPick> {
    if agg.primary_pct.is_none() && agg.secondary_pct.is_none() {
        return None;
    }
    Some(RateLimitPick {
        primary_pct: agg.primary_pct,
        secondary_pct: agg.secondary_pct,
        plan_type: agg.plan_type.clone(),
        message: agg.plan_type.clone().map(|p| format!("Plan: {p}")),
        resets_at: None,
        line_time: agg.line_time,
        source: "session",
    })
}

fn is_newer(a: &RateLimitPick, b: &RateLimitPick) -> bool {
    match (a.line_time, b.line_time) {
        (Some(at), Some(bt)) => at >= bt,
        (Some(_), None) => true,
        _ => false,
    }
}

fn build_quota(pick: &RateLimitPick, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
    let primary_label = if pick.message.as_ref().is_some_and(|m| m.contains("credit")) {
        "Monthly credits"
    } else {
        "5h window"
    };
    Ok(QuotaSnapshot {
        provider: "codex".into(),
        label: "Codex".into(),
        used_percent_month: pick.primary_pct,
        used_percent_year: pick.secondary_pct,
        used_tokens_month: Some(db.provider_month_tokens("codex").unwrap_or(0)),
        limit_tokens_month: None,
        used_usd_month: Some(db.provider_month_spend("codex").unwrap_or(0.0)),
        limit_usd_month: None,
        primary_window_label: Some(primary_label.into()),
        primary_window_percent: pick.primary_pct,
        secondary_window_label: pick.secondary_pct.map(|_| "Weekly".into()),
        secondary_window_percent: pick.secondary_pct,
        resets_at: pick.resets_at.clone(),
        status: if settings.enabled_providers.contains(&"codex".to_string()) {
            if pick.primary_pct.is_some() {
                "ok".into()
            } else {
                "ok".into()
            }
        } else {
            "disabled".into()
        },
        message: pick.message.clone().or_else(|| {
            if pick.primary_pct.is_none() {
                Some("Open Codex or run Sync after a session — quota comes from auth.json + session logs".into())
            } else {
                None
            }
        }),
    })
}

fn parse_rollout(path: &Path) -> Result<SessionAgg> {
    let file = File::open(path).context("open rollout")?;
    let reader = BufReader::new(file);
    let mut agg = SessionAgg::default();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let line_time = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_timestamp);

        if let Some(obj) = v.as_object() {
            if obj.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
                if let Some(model) = obj
                    .get("payload")
                    .and_then(|p| p.get("model"))
                    .and_then(|m| m.as_str())
                {
                    agg.model = model.to_string();
                }
                if let Some(ts) = line_time {
                    agg.started_at = Some(ts);
                }
            }
        }

        let payload = match v.get("payload") {
            Some(p) => p,
            None => continue,
        };
        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }

        if let Some(info) = payload.get("info").filter(|x| !x.is_null()) {
            if let Some(total) = info.get("total_token_usage") {
                agg.input = total
                    .get("input_tokens")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(agg.input);
                agg.output = total
                    .get("output_tokens")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(agg.output);
                agg.cache = total
                    .get("cached_input_tokens")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(agg.cache);
            }
        }

        let limits = payload
            .get("rate_limits")
            .or_else(|| payload.get("info").and_then(|i| i.get("rate_limits")));
        if let Some(limits) = limits {
            apply_rate_limits(limits, &mut agg, line_time);
        }

        if payload.get("type").and_then(|t| t.as_str()) == Some("user_message") {
            if agg.title.is_empty() {
                if let Some(text) = payload
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                    .and_then(|a| a.first())
                    .and_then(|x| x.get("text"))
                    .and_then(|t| t.as_str())
                {
                    agg.title = text.chars().take(80).collect();
                }
            }
        }

        if agg.title.is_empty() {
            if let Some(session) = v.get("sessionId").and_then(|s| s.as_str()) {
                agg.title = format!("Session {}", &session[..session.len().min(8)]);
            }
        }
    }

    Ok(agg)
}

fn apply_rate_limits(limits: &Value, agg: &mut SessionAgg, line_time: Option<DateTime<Utc>>) {
    if let Some(p) = limits.get("primary").filter(|x| !x.is_null()) {
        agg.primary_pct = p.get("used_percent").and_then(as_f64);
    }
    if let Some(s) = limits.get("secondary").filter(|x| !x.is_null()) {
        agg.secondary_pct = s.get("used_percent").and_then(as_f64);
    }
    agg.plan_type = limits
        .get("plan_type")
        .and_then(|x| x.as_str())
        .map(String::from);
    if let Some(t) = line_time {
        agg.line_time = Some(t);
    }
    if agg.primary_pct.is_none()
        && limits
            .get("rate_limit_reached_type")
            .and_then(|x| x.as_str())
            .is_some()
    {
        agg.primary_pct = Some(100.0);
    }
}

fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|n| n as f64))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn unix_to_rfc3339(secs: i64) -> Option<String> {
    Utc.timestamp_opt(secs, 0)
        .single()
        .map(|d| d.to_rfc3339())
}

fn disabled() -> QuotaSnapshot {
    QuotaSnapshot {
        provider: "codex".into(),
        label: "Codex".into(),
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
    fn parses_wham_spend_control() {
        let v: Value = serde_json::json!({
            "plan_type": "business",
            "rate_limit": null,
            "spend_control": {
                "individual_limit": {
                    "used_percent": 42,
                    "used": "126.0",
                    "limit": "300",
                    "unit": "credit"
                }
            }
        });
        let pick = parse_wham_usage(&v).expect("parse");
        assert_eq!(pick.primary_pct, Some(42.0));
        assert!(pick.message.unwrap().contains("126"));
    }

    #[test]
    fn parses_payload_rate_limits() {
        let line = r#"{"timestamp":"2026-06-24T20:56:01.464Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1}},"rate_limits":{"primary":{"used_percent":5.0},"secondary":{"used_percent":12.0},"plan_type":"plus"}}}"#;
        let v: Value = serde_json::from_str(line).unwrap();
        let mut agg = SessionAgg::default();
        let payload = v.get("payload").unwrap();
        apply_rate_limits(payload.get("rate_limits").unwrap(), &mut agg, None);
        assert_eq!(agg.primary_pct, Some(5.0));
        assert_eq!(agg.secondary_pct, Some(12.0));
    }
}
