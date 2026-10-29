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

pub struct ClaudeCodeProvider;

fn projects_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

#[derive(Default)]
struct SessionAgg {
    session_id: String,
    model: String,
    started_at: Option<DateTime<Utc>>,
    input: i64,
    output: i64,
    cache: i64,
    title: String,
}

#[async_trait]
impl Provider for ClaudeCodeProvider {
    fn id(&self) -> &'static str {
        "claude_code"
    }

    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()> {
        if !settings.enabled_providers.iter().any(|p| p == "claude_code") {
            return Ok(());
        }
        let Some(base) = projects_dir() else {
            return Ok(());
        };
        if !base.exists() {
            return Ok(());
        }

        let mut sessions: HashMap<String, SessionAgg> = HashMap::new();

        for entry in WalkDir::new(&base)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
        {
            parse_jsonl(entry.path(), &mut sessions)?;
        }

        for (_, agg) in sessions {
            if agg.session_id.is_empty() {
                continue;
            }
            let model = if agg.model.is_empty() {
                "claude-sonnet".into()
            } else {
                agg.model.clone()
            };
            let started = agg
                .started_at
                .unwrap_or_else(Utc::now)
                .to_rfc3339();
            let cost = estimate_cost_usd(&model, agg.input, agg.output, agg.cache);
            let title = if agg.title.is_empty() {
                format!("Claude session {}", &agg.session_id[..agg.session_id.len().min(8)])
            } else {
                agg.title
            };
            db.upsert_run(&RunRecord {
                provider: "claude_code".into(),
                run_id: agg.session_id,
                started_at: started,
                model,
                input_tokens: agg.input,
                output_tokens: agg.output,
                cache_tokens: agg.cache,
                cost_usd: cost,
                title,
            })?;
        }
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        let tokens = db.provider_month_tokens("claude_code").unwrap_or(0);
        let spend = db.provider_month_spend("claude_code").unwrap_or(0.0);
        Ok(QuotaSnapshot {
            provider: "claude_code".into(),
            label: "Claude Code".into(),
            used_percent_month: None,
            used_percent_year: None,
            used_tokens_month: Some(tokens),
            limit_tokens_month: None,
            used_usd_month: Some(spend),
            limit_usd_month: None,
            primary_window_label: None,
            primary_window_percent: None,
            secondary_window_label: None,
            secondary_window_percent: None,
            resets_at: None,
            status: if settings.enabled_providers.contains(&"claude_code".to_string()) {
                "ok".into()
            } else {
                "disabled".into()
            },
            message: Some("Estimated from local session logs".into()),
        })
    }
}

fn parse_jsonl(path: &std::path::Path, sessions: &mut HashMap<String, SessionAgg>) -> Result<()> {
    let file = File::open(path).context("open claude jsonl")?;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let session_id = v
            .get("sessionId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        if session_id.is_empty() {
            continue;
        }

        let entry = sessions.entry(session_id.clone()).or_insert_with(|| SessionAgg {
            session_id: session_id.clone(),
            ..Default::default()
        });

        if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
            if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
                let u = dt.with_timezone(&Utc);
                if entry.started_at.is_none() || Some(u) < entry.started_at {
                    entry.started_at = Some(u);
                }
            }
        }

        if v.get("type").and_then(|t| t.as_str()) == Some("user") {
            if entry.title.is_empty() {
                if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
                    entry.title = msg.chars().take(80).collect();
                }
            }
        }

        if let Some(msg) = v.get("message") {
            if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                if let Some(model) = msg.get("model").and_then(|m| m.as_str()) {
                    entry.model = model.to_string();
                }
                if let Some(usage) = msg.get("usage") {
                    entry.input += usage
                        .get("input_tokens")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0)
                        + usage
                            .get("cache_creation_input_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0)
                        + usage
                            .get("cache_read_input_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0);
                    entry.output += usage
                        .get("output_tokens")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0);
                    let cache = usage
                        .get("cache_read_input_tokens")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0);
                    entry.cache += cache;
                }
            }
        }
    }
    Ok(())
}
