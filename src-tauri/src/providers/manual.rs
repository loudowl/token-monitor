use anyhow::Result;
use async_trait::async_trait;

use crate::db::Db;
use crate::models::{AppSettings, QuotaSnapshot};
use crate::pricing::plan_catalog;
use crate::providers::Provider;

pub struct ManualProvider {
    provider_id: &'static str,
    label: &'static str,
}

impl ManualProvider {
    pub fn new(provider_id: &'static str, label: &'static str) -> Self {
        Self {
            provider_id,
            label,
        }
    }
}

#[async_trait]
impl Provider for ManualProvider {
    fn id(&self) -> &'static str {
        self.provider_id
    }

    async fn sync(&self, _db: &Db, _settings: &AppSettings) -> Result<()> {
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        build_consumer_quota(self.provider_id, self.label, db, settings, false)
    }
}

pub fn build_consumer_quota(
    provider: &str,
    label: &str,
    db: &Db,
    settings: &AppSettings,
    has_api_key: bool,
) -> Result<QuotaSnapshot> {
    if !settings.enabled_providers.iter().any(|p| p == provider) {
        return Ok(QuotaSnapshot {
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
        });
    }

    let plan_entry = settings
        .plans
        .iter()
        .find(|p| p.provider == provider);
    let plan_id = plan_entry.map(|p| p.plan_id.as_str()).unwrap_or("");
    let plan = plan_catalog()
        .into_iter()
        .find(|p| p.id == plan_id);

    let manual_pct = plan_entry.and_then(|p| p.manual_used_percent);
    let spend = db.provider_month_spend(provider).unwrap_or(0.0);
    let limit_usd = plan.as_ref().and_then(|p| p.monthly_usd_limit);
    let computed_pct = limit_usd.map(|lim| (spend / lim * 100.0).min(999.0));
    let used_pct = manual_pct.or(computed_pct);

    let period = plan_entry.map(|p| p.period.as_str()).unwrap_or("month");
    let used_year = if period == "year" {
        used_pct.map(|p| p / 12.0)
    } else {
        None
    };

    Ok(QuotaSnapshot {
        provider: provider.into(),
        label: label.into(),
        used_percent_month: used_pct,
        used_percent_year: used_year,
        used_tokens_month: Some(db.provider_month_tokens(provider).unwrap_or(0)),
        limit_tokens_month: plan.as_ref().and_then(|p| p.monthly_token_limit),
        used_usd_month: if has_api_key || spend > 0.0 {
            Some(spend)
        } else {
            None
        },
        limit_usd_month: limit_usd,
        primary_window_label: None,
        primary_window_percent: None,
        secondary_window_label: None,
        secondary_window_percent: None,
        resets_at: None,
        status: if manual_pct.is_some() || has_api_key || spend > 0.0 {
            "ok".into()
        } else {
            "manual".into()
        },
        message: Some(
            "Set plan usage % in Settings, or add an API key for automatic tracking".into(),
        ),
    })
}
