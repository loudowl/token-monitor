use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::db::{billing_cycle_start, Db};
use crate::keychain;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::providers::manual;
use crate::providers::Provider;

pub struct OpenAiProvider;

#[async_trait]
impl Provider for OpenAiProvider {
    fn id(&self) -> &'static str {
        "chatgpt"
    }

    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()> {
        if !settings.enabled_providers.iter().any(|p| p == "chatgpt") {
            return Ok(());
        }
        let key = match keychain::get_secret("openai_admin_key")? {
            Some(k) => k,
            None => return Ok(()),
        };
        let cycle = billing_cycle_start(Utc::now(), settings.billing_anchor_day);
        let start = cycle.format("%Y-%m-%d").to_string();
        let end = Utc::now().format("%Y-%m-%d").to_string();

        let client = reqwest::Client::new();
        let resp = client
            .get("https://api.openai.com/v1/organization/costs")
            .query(&[("start_time", start.as_str()), ("end_time", end.as_str())])
            .bearer_auth(&key)
            .send()
            .await;

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            _ => return Ok(()),
        };
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
            for bucket in data {
                let run_id = bucket
                    .get("start_time")
                    .and_then(|x| x.as_i64())
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| Utc::now().timestamp().to_string());
                let cost = bucket
                    .get("results")
                    .and_then(|r| r.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|x| x.get("amount").and_then(|a| a.get("value")))
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0);
                db.upsert_run(&RunRecord {
                    provider: "chatgpt".into(),
                    run_id,
                    started_at: Utc::now().to_rfc3339(),
                    model: "openai-api".into(),
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_tokens: 0,
                    cost_usd: cost,
                    title: "OpenAI org cost bucket".into(),
                    conversation_id: String::new(),
                    task_context: String::new(),
                    task_inherited: false,
                })?;
            }
        }
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        manual::build_consumer_quota(
            "chatgpt",
            "ChatGPT",
            db,
            settings,
            keychain::get_secret("openai_admin_key")?.is_some(),
        )
    }
}
