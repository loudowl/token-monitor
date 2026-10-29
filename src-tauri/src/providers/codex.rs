use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use walkdir::WalkDir;

use crate::db::Db;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::pricing::estimate_cost_usd;
use crate::providers::Provider;

pub struct CodexProvider;

fn sessions_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("sessions"))
}

#[derive(Default)]
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
        let Some(base) = sessions_dir() else {
            return Ok(());
        };
        if !base.exists() {
            return Ok(());
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
            if let Ok(mut agg) = parse_rollout(path) {
                let started = agg
                    .started_at
                    .unwrap_or_else(Utc::now)
                    .to_rfc3339();
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
                })?;

                if agg.primary_pct.is_some() || agg.secondary_pct.is_some() {
                    let quota = QuotaSnapshot {
                        provider: "codex".into(),
                        label: "Codex".into(),
                        used_percent_month: None,
                        used_percent_year: None,
                        used_tokens_month: Some(db.provider_month_tokens("codex")?),
                        limit_tokens_month: None,
                        used_usd_month: Some(db.provider_month_spend("codex")?),
                        limit_usd_month: None,
                        primary_window_label: Some("5h window".into()),
                        primary_window_percent: agg.primary_pct,
                        secondary_window_label: Some("Weekly".into()),
                        secondary_window_percent: agg.secondary_pct,
                        resets_at: None,
                        status: "ok".into(),
                        message: agg.plan_type.map(|p| format!("Plan: {p}")),
                    };
                    db.save_quota(&quota)?;
                }
            }
        }
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        if let Ok(list) = db.list_quotas() {
            if let Some(q) = list.into_iter().find(|q| q.provider == "codex") {
                return Ok(q);
            }
        }
        Ok(QuotaSnapshot {
            provider: "codex".into(),
            label: "Codex".into(),
            used_percent_month: None,
            used_percent_year: None,
            used_tokens_month: Some(db.provider_month_tokens("codex").unwrap_or(0)),
            limit_tokens_month: None,
            used_usd_month: Some(db.provider_month_spend("codex").unwrap_or(0.0)),
            limit_usd_month: None,
            primary_window_label: Some("5h window".into()),
            primary_window_percent: None,
            secondary_window_label: Some("Weekly".into()),
            secondary_window_percent: None,
            resets_at: None,
            status: if settings.enabled_providers.contains(&"codex".to_string()) {
                "ok".into()
            } else {
                "disabled".into()
            },
            message: None,
        })
    }
}

fn parse_rollout(path: &std::path::Path) -> Result<SessionAgg> {
    let file = File::open(path).context("open rollout")?;
    let reader = BufReader::new(file);
    let mut agg = SessionAgg::default();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if v.is_null() {
            continue;
        }

        if let Some(obj) = v.as_object() {
            if obj.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
                if let Some(model) = obj
                    .get("payload")
                    .and_then(|p| p.get("model"))
                    .and_then(|m| m.as_str())
                {
                    agg.model = model.to_string();
                }
                if let Some(ts) = obj.get("timestamp").and_then(|t| t.as_str()) {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
                        agg.started_at = Some(dt.with_timezone(&Utc));
                    }
                }
            }
        }

        if let Some(payload) = v.get("payload") {
            if payload.get("type").and_then(|t| t.as_str()) == Some("token_count") {
                if let Some(info) = payload.get("info") {
                    if let Some(total) = info.get("total_token_usage") {
                        agg.input = total
                            .get("input_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(agg.input);
                        agg.output = total
                            .get("output_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(agg.output);
                        let cached = total
                            .get("cached_input_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0);
                        agg.cache = cached;
                    }
                    if let Some(limits) = info.get("rate_limits") {
                        agg.primary_pct = limits
                            .get("primary")
                            .and_then(|p| p.get("used_percent"))
                            .and_then(|x| x.as_f64());
                        agg.secondary_pct = limits
                            .get("secondary")
                            .and_then(|p| p.get("used_percent"))
                            .and_then(|x| x.as_f64());
                        agg.plan_type = limits.get("plan_type").and_then(|x| x.as_str()).map(String::from);
                    }
                }
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
        }

        if agg.title.is_empty() {
            if let Some(session) = v.get("sessionId").and_then(|s| s.as_str()) {
                agg.title = format!("Session {}", &session[..session.len().min(8)]);
            }
        }
    }

    Ok(agg)
}
