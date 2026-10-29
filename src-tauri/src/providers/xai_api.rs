use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;

use crate::db::Db;
use crate::keychain;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::providers::manual;
use crate::providers::Provider;

pub struct XaiProvider;

#[async_trait]
impl Provider for XaiProvider {
    fn id(&self) -> &'static str {
        "grok"
    }

    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()> {
        if !settings.enabled_providers.iter().any(|p| p == "grok") {
            return Ok(());
        }
        let key = match keychain::get_secret("xai_api_key")? {
            Some(k) => k,
            None => return Ok(()),
        };

        let client = reqwest::Client::new();
        let resp = client
            .get("https://api.x.ai/v1/usage")
            .bearer_auth(&key)
            .send()
            .await;

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            _ => return Ok(()),
        };

        if let Ok(v) = resp.json::<serde_json::Value>().await {
            let spend = v
                .get("total_cost_usd")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            db.upsert_run(&RunRecord {
                provider: "grok".into(),
                run_id: format!("xai-{}", Utc::now().timestamp()),
                started_at: Utc::now().to_rfc3339(),
                model: "grok".into(),
                input_tokens: 0,
                output_tokens: 0,
                cache_tokens: 0,
                cost_usd: spend,
                title: "xAI usage snapshot".into(),
            })?;
        }
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        manual::build_consumer_quota(
            "grok",
            "Grok",
            db,
            settings,
            keychain::get_secret("xai_api_key")?.is_some(),
        )
    }
}
