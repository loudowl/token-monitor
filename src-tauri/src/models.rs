use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub provider: String,
    pub run_id: String,
    pub started_at: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub cost_usd: f64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsageRow {
    pub model: String,
    pub cost_usd: f64,
    pub share_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    pub provider: String,
    pub label: String,
    pub used_percent_month: Option<f64>,
    pub used_percent_year: Option<f64>,
    pub used_tokens_month: Option<i64>,
    pub limit_tokens_month: Option<i64>,
    pub used_usd_month: Option<f64>,
    pub limit_usd_month: Option<f64>,
    pub primary_window_label: Option<String>,
    pub primary_window_percent: Option<f64>,
    pub secondary_window_label: Option<String>,
    pub secondary_window_percent: Option<f64>,
    pub resets_at: Option<String>,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPlan {
    pub id: String,
    pub name: String,
    pub monthly_token_limit: Option<i64>,
    pub monthly_usd_limit: Option<f64>,
    pub period: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub enabled_providers: Vec<String>,
    pub billing_anchor_day: u8,
    pub plans: Vec<ManualPlanEntry>,
    pub alert_threshold_percent: f64,
    pub github_username: Option<String>,
    pub widget_x: Option<f64>,
    pub widget_y: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualPlanEntry {
    pub provider: String,
    pub plan_id: String,
    pub period: String,
    pub renewal_day: u8,
    pub manual_used_percent: Option<f64>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            enabled_providers: vec![
                "codex".into(),
                "claude_code".into(),
                "cursor".into(),
                "copilot".into(),
                "chatgpt".into(),
                "claude_ai".into(),
                "grok".into(),
            ],
            billing_anchor_day: 1,
            plans: default_manual_plans(),
            alert_threshold_percent: 80.0,
            github_username: None,
            widget_x: None,
            widget_y: None,
        }
    }
}

pub fn default_manual_plans() -> Vec<ManualPlanEntry> {
    vec![
        ManualPlanEntry {
            provider: "chatgpt".into(),
            plan_id: "chatgpt_plus".into(),
            period: "month".into(),
            renewal_day: 1,
            manual_used_percent: None,
        },
        ManualPlanEntry {
            provider: "claude_ai".into(),
            plan_id: "claude_pro".into(),
            period: "month".into(),
            renewal_day: 1,
            manual_used_percent: None,
        },
        ManualPlanEntry {
            provider: "grok".into(),
            plan_id: "grok_premium".into(),
            period: "month".into(),
            renewal_day: 1,
            manual_used_percent: None,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunsQuery {
    pub provider: Option<String>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    pub quotas: Vec<QuotaSnapshot>,
    pub projected_month_usd: f64,
    pub month_spend_usd: f64,
}
