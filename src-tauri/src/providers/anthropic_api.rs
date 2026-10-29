use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::db::{billing_cycle_start, Db};
use crate::keychain;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::providers::manual;
use crate::providers::Provider;

pub struct AnthropicProvider;

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "claude_ai"
    }

    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()> {
        if !settings.enabled_providers.iter().any(|p| p == "claude_ai") {
            return Ok(());
        }
        let key = match keychain::get_secret("anthropic_admin_key")? {
            Some(k) => k,
            None => return Ok(()),
        };
        let cycle = billing_cycle_start(Utc::now(), settings.billing_anchor_day);
        let start = cycle.format("%Y-%m-%d").to_string();
        let end = Utc::now().format("%Y-%m-%d").to_string();

        let client = reqwest::Client::new();
        let resp = client
            .get("https://api.anthropic.com/v1/organizations/usage_report/messages")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .query(&[("starting_at", start.as_str()), ("ending_at", end.as_str())])
            .send()
            .await;

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            _ => return Ok(()),
        };
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
            for row in data {
                let model = row
                    .get("model")
                    .and_then(|m| m.as_str())
                    .unwrap_or("claude")
                    .to_string();
                let input = row.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
                let output = row.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
                let run_id = format!("{model}-{}", Utc::now().timestamp());
                let cost = crate::pricing::estimate_cost_usd(&model, input, output, 0);
                db.upsert_run(&RunRecord {
                    provider: "claude_ai".into(),
                    run_id,
                    started_at: Utc::now().to_rfc3339(),
                    model,
                    input_tokens: input,
                    output_tokens: output,
                    cache_tokens: 0,
                    cost_usd: cost,
                    title: "Anthropic admin usage".into(),
                })?;
            }
        }
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        manual::build_consumer_quota(
            "claude_ai",
            "Claude.ai",
            db,
            settings,
            keychain::get_secret("anthropic_admin_key")?.is_some(),
        )
    }
}
