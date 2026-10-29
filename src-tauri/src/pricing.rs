use std::collections::HashMap;

/// USD per 1M tokens (approximate list prices for cost estimation).
pub fn estimate_cost_usd(model: &str, input: i64, output: i64, cache: i64) -> f64 {
    let m = model.to_lowercase();
    let (in_m, out_m, cache_m) = price_for_model(&m);
    (input as f64 / 1_000_000.0) * in_m
        + (output as f64 / 1_000_000.0) * out_m
        + (cache as f64 / 1_000_000.0) * cache_m
}

fn price_for_model(model: &str) -> (f64, f64, f64) {
    let table: HashMap<&str, (f64, f64, f64)> = HashMap::from([
        ("gpt-4o", (2.5, 10.0, 1.25)),
        ("gpt-4.1", (2.0, 8.0, 0.5)),
        ("gpt-5", (1.25, 10.0, 0.125)),
        ("o3", (2.0, 8.0, 0.5)),
        ("o4-mini", (1.1, 4.4, 0.275)),
        ("claude-sonnet-4", (3.0, 15.0, 0.3)),
        ("claude-opus-4", (15.0, 75.0, 1.5)),
        ("claude-haiku", (0.8, 4.0, 0.08)),
        ("composer", (3.0, 15.0, 0.3)),
        ("grok", (3.0, 15.0, 0.75)),
    ]);

    for (key, prices) in table {
        if model.contains(key) {
            return prices;
        }
    }
    (3.0, 15.0, 0.5)
}

pub fn plan_catalog() -> Vec<crate::models::ProviderPlan> {
    vec![
        crate::models::ProviderPlan {
            id: "chatgpt_plus".into(),
            name: "ChatGPT Plus".into(),
            monthly_token_limit: None,
            monthly_usd_limit: Some(20.0),
            period: "month".into(),
        },
        crate::models::ProviderPlan {
            id: "chatgpt_pro".into(),
            name: "ChatGPT Pro".into(),
            monthly_token_limit: None,
            monthly_usd_limit: Some(200.0),
            period: "month".into(),
        },
        crate::models::ProviderPlan {
            id: "claude_pro".into(),
            name: "Claude Pro".into(),
            monthly_token_limit: None,
            monthly_usd_limit: Some(20.0),
            period: "month".into(),
        },
        crate::models::ProviderPlan {
            id: "claude_max".into(),
            name: "Claude Max".into(),
            monthly_token_limit: None,
            monthly_usd_limit: Some(100.0),
            period: "month".into(),
        },
        crate::models::ProviderPlan {
            id: "cursor_pro".into(),
            name: "Cursor Pro".into(),
            monthly_token_limit: None,
            monthly_usd_limit: Some(20.0),
            period: "month".into(),
        },
        crate::models::ProviderPlan {
            id: "copilot_individual".into(),
            name: "GitHub Copilot Individual".into(),
            monthly_token_limit: None,
            monthly_usd_limit: Some(10.0),
            period: "month".into(),
        },
        crate::models::ProviderPlan {
            id: "grok_premium".into(),
            name: "Grok Premium".into(),
            monthly_token_limit: None,
            monthly_usd_limit: Some(30.0),
            period: "month".into(),
        },
        crate::models::ProviderPlan {
            id: "codex_plus".into(),
            name: "Codex Plus".into(),
            monthly_token_limit: None,
            monthly_usd_limit: None,
            period: "month".into(),
        },
    ]
}
