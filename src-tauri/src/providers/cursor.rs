use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use std::sync::Mutex;

use crate::db::{billing_cycle_start, Db};
use crate::keychain;
use crate::models::{AppSettings, QuotaSnapshot, RunRecord};
use crate::providers::Provider;

pub struct CursorProvider;

static CAPTURED_COOKIE: Mutex<Option<String>> = Mutex::new(None);

pub fn remember_session_cookie(value: &str) {
    if value.trim().is_empty() {
        return;
    }
    *CAPTURED_COOKIE.lock().unwrap() = Some(value.trim().to_string());
}

#[derive(Debug, Clone, Serialize)]
pub struct CursorProbeResult {
    pub ok: bool,
    pub email: Option<String>,
    pub events: u32,
    pub percent: Option<f64>,
    pub error: Option<String>,
}

struct CursorSession {
    cookie_value: String,
    email: Option<String>,
    user_id: Option<i64>,
}

fn cursor_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("Application Support")
            .join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb")
    })
}

fn read_vscdb_string(key: &str) -> Result<Option<String>> {
    let Some(path) = cursor_db_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("open cursor vscdb")?;
    let res: Result<String, _> =
        conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |r| {
            r.get(0)
        });
    match res {
        Ok(raw) => {
            let trimmed = raw.trim().trim_matches('"').to_string();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn clean_cookie_input(raw: &str) -> String {
    let mut s = raw.trim().trim_matches('"').trim().to_string();
    if let Some(rest) = s.strip_prefix("Cookie:") {
        s = rest.trim().to_string();
    }
    // Full header: "WorkosCursorSessionToken=...; other=..."
    if s.contains("WorkosCursorSessionToken=") {
        if let Some(part) = s
            .split(';')
            .find(|p| p.trim().starts_with("WorkosCursorSessionToken="))
        {
            s = part
                .trim()
                .trim_start_matches("WorkosCursorSessionToken=")
                .trim()
                .to_string();
        }
    } else if let Some(rest) = s.strip_prefix("WorkosCursorSessionToken=") {
        s = rest.trim().to_string();
    }
    s
}

fn looks_like_jwt(s: &str) -> bool {
    let token = s.split("%3A%3A").last().unwrap_or(s).split("::").last().unwrap_or(s);
    token.starts_with("eyJ") && token.matches('.').count() >= 2
}

fn jwt_sub(jwt: &str) -> Option<String> {
    use base64::Engine;
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .or_else(|_| {
            let mut padded = payload.to_string();
            while padded.len() % 4 != 0 {
                padded.push('=');
            }
            base64::engine::general_purpose::URL_SAFE.decode(padded)
        })
        .ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let sub = v.get("sub")?.as_str()?;
    Some(sub.rsplit('|').next().unwrap_or(sub).to_string())
}

fn cookie_candidates(raw: &str) -> Vec<String> {
    let cleaned = clean_cookie_input(raw);
    if cleaned.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    push(cleaned.clone());
    if cleaned.contains("::") {
        push(cleaned.replace("::", "%3A%3A"));
    }
    if cleaned.contains("%3A%3A") {
        push(cleaned.replace("%3A%3A", "::"));
    }
    if looks_like_jwt(&cleaned) && !cleaned.contains("::") && !cleaned.contains("%3A%3A") {
        if let Some(uid) = jwt_sub(&cleaned) {
            push(format!("{uid}%3A%3A{cleaned}"));
            push(format!("{uid}::{cleaned}"));
        }
    }
    out
}

fn source_secrets() -> Result<Vec<String>> {
    let mut raws = Vec::new();
    if let Some(captured) = CAPTURED_COOKIE.lock().unwrap().clone() {
        raws.push(captured);
    }
    if let Some(manual) = keychain::get_secret("cursor_session_token")? {
        raws.push(manual);
    }
    if let Ok(Some(local)) = read_vscdb_string("cursorAuth/accessToken") {
        // Newer Cursor builds store an opaque/encrypted blob, not a dashboard cookie.
        if looks_like_jwt(&local) || local.contains("::") || local.contains("%3A%3A") {
            raws.push(local);
        }
    }
    Ok(raws)
}

fn cursor_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn cookie_header(token: &str) -> String {
    format!("WorkosCursorSessionToken={token}")
}

fn snippet_error(status: u16, body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            return format!("HTTP {status}: {err}");
        }
    }
    let trimmed = body.trim();
    if trimmed.starts_with('<') || trimmed.contains("AuthKit") || status >= 300 && status < 400 {
        return format!("HTTP {status}: redirected to login (cookie not accepted)");
    }
    format!("HTTP {status}")
}

fn json_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let cleaned = s.replace(['$', ',', '%'], "");
            cleaned.trim().parse().ok()
        }
        _ => None,
    }
}

fn pointer_f64(root: &Value, path: &str) -> Option<f64> {
    root.pointer(path).and_then(json_f64)
}

fn cents_to_usd(cents: f64) -> f64 {
    cents / 100.0
}

fn ratio_pct(used: Option<f64>, limit: Option<f64>) -> Option<f64> {
    match (used, limit) {
        (Some(used), Some(limit)) if used.is_finite() && limit.is_finite() && limit > 0.0 => {
            Some((used / limit * 100.0).min(999.0))
        }
        _ => None,
    }
}

fn used_limit_from(obj: &Value) -> (Option<f64>, Option<f64>) {
    if !obj.is_object() {
        return (None, None);
    }
    let limit = obj
        .get("limit")
        .and_then(json_f64)
        .or_else(|| obj.get("pooledLimit").and_then(json_f64))
        .or_else(|| obj.get("individualLimit").and_then(json_f64));
    let remaining = obj
        .get("remaining")
        .and_then(json_f64)
        .or_else(|| obj.get("pooledRemaining").and_then(json_f64))
        .or_else(|| obj.get("individualRemaining").and_then(json_f64));
    let used = obj
        .get("used")
        .and_then(json_f64)
        .or_else(|| obj.get("totalSpend").and_then(json_f64))
        .or_else(|| obj.get("includedSpend").and_then(json_f64))
        .or_else(|| obj.get("pooledUsed").and_then(json_f64))
        .or_else(|| obj.get("individualUsed").and_then(json_f64))
        .or_else(|| match (limit, remaining) {
            (Some(limit), Some(remaining)) => Some((limit - remaining).max(0.0)),
            _ => None,
        });
    (used, limit)
}

fn find_named_f64(value: &Value, name: &str) -> Option<f64> {
    match value {
        Value::Object(map) => {
            if let Some(v) = map.get(name).and_then(json_f64).filter(|n| n.is_finite()) {
                return Some(v);
            }
            map.values().find_map(|v| find_named_f64(v, name))
        }
        Value::Array(arr) => arr.iter().find_map(|v| find_named_f64(v, name)),
        _ => None,
    }
}

fn percent_from_message(usage: &Value) -> Option<f64> {
    for key in [
        "displayMessage",
        "autoModelSelectedDisplayMessage",
        "namedModelSelectedDisplayMessage",
    ] {
        let Some(msg) = usage.get(key).and_then(|x| x.as_str()) else {
            continue;
        };
        let Some(idx) = msg.find('%') else {
            continue;
        };
        let start = msg[..idx]
            .rfind(|c: char| !c.is_ascii_digit() && c != '.')
            .map(|i| i + 1)
            .unwrap_or(0);
        if let Ok(n) = msg[start..idx].parse::<f64>() {
            // Team dashboards still say "You've used 0% of included usage"
            // even when the real monthly cap is individualUsage.overall.
            if n.is_finite() && n > 0.0 {
                return Some(n);
            }
        }
    }
    None
}

fn positive_pct(pct: Option<f64>) -> Option<f64> {
    pct.filter(|p| p.is_finite() && *p > 0.0)
}

fn is_team_shaped(usage: &Value) -> bool {
    let membership = usage
        .get("membershipType")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let limit_type = usage
        .get("limitType")
        .or_else(|| usage.pointer("/spendLimitUsage/limitType"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let pooled = pointer_f64(usage, "/spendLimitUsage/pooledLimit").unwrap_or(0.0) > 0.0;
    matches!(membership.as_str(), "enterprise" | "team" | "business")
        || limit_type == "team"
        || pooled
        || usage.get("teamUsage").is_some()
}

#[derive(Debug, Clone, PartialEq)]
struct CycleUsage {
    percent: Option<f64>,
    used_usd: Option<f64>,
    limit_usd: Option<f64>,
    resets_at: Option<String>,
}

fn extract_cycle_usage(usage: &Value, fallback_spend_usd: f64) -> CycleUsage {
    let resets_at = usage
        .get("billingCycleEnd")
        .and_then(|x| x.as_str())
        .map(String::from);

    let plan = usage
        .pointer("/planUsage")
        .or_else(|| usage.pointer("/individualUsage/plan"));
    let plan_pct = plan
        .and_then(|p| p.get("totalPercentUsed").and_then(json_f64))
        .filter(|n| n.is_finite())
        .or_else(|| find_named_f64(usage, "totalPercentUsed"));
    let (plan_used, plan_limit) = plan.map(used_limit_from).unwrap_or((None, None));
    let plan_ratio = ratio_pct(plan_used, plan_limit);

    let spend_pairs = [
        used_limit_from(usage.pointer("/individualUsage/overall").unwrap_or(&Value::Null)),
        used_limit_from(usage.pointer("/teamUsage/pooled").unwrap_or(&Value::Null)),
        (
            pointer_f64(usage, "/spendLimitUsage/individualUsed"),
            pointer_f64(usage, "/spendLimitUsage/individualLimit"),
        ),
        (
            pointer_f64(usage, "/spendLimitUsage/pooledUsed"),
            pointer_f64(usage, "/spendLimitUsage/pooledLimit"),
        ),
        used_limit_from(usage.pointer("/spendLimitUsage").unwrap_or(&Value::Null)),
        used_limit_from(usage.pointer("/teamUsage/onDemand").unwrap_or(&Value::Null)),
        used_limit_from(usage.pointer("/teamUsage/plan").unwrap_or(&Value::Null)),
        used_limit_from(usage.pointer("/individualUsage/onDemand").unwrap_or(&Value::Null)),
    ];
    let (spend_used_cents, spend_limit_cents) = spend_pairs
        .into_iter()
        .find(|(_, limit)| limit.map(|n| n > 0.0).unwrap_or(false))
        .unwrap_or((None, None));
    let spend_pct = ratio_pct(spend_used_cents, spend_limit_cents);
    let dollar_limit = pointer_f64(usage, "/perUserMonthlyLimitDollars")
        .or_else(|| pointer_f64(usage, "/hardLimitPerUser"))
        .or_else(|| pointer_f64(usage, "/hardLimit"))
        .filter(|n| *n > 0.0);

    let mut percent = if is_team_shaped(usage) {
        spend_pct
            .or_else(|| positive_pct(plan_pct))
            .or_else(|| positive_pct(plan_ratio))
            .or_else(|| percent_from_message(usage))
    } else {
        positive_pct(plan_pct)
            .or_else(|| positive_pct(plan_ratio))
            .or(spend_pct)
            .or_else(|| percent_from_message(usage))
            .or(plan_pct)
            .or(plan_ratio)
    };

    let limit_usd = spend_limit_cents
        .map(cents_to_usd)
        .or(dollar_limit)
        .or_else(|| plan_limit.map(cents_to_usd));
    let used_usd = spend_used_cents
        .map(cents_to_usd)
        .or_else(|| plan_used.map(cents_to_usd))
        .or_else(|| (fallback_spend_usd > 0.0).then_some(fallback_spend_usd));

    if percent.is_none() || (percent == Some(0.0) && used_usd.unwrap_or(0.0) > 1.0) {
        percent = ratio_pct(used_usd, limit_usd);
    }

    CycleUsage {
        percent,
        used_usd,
        limit_usd,
        resets_at,
    }
}

fn merge_json(base: Value, incoming: Value) -> Value {
    match (base, incoming) {
        (Value::Null, other) => other,
        (prev, Value::Null) => prev,
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                let next = match a.remove(&k) {
                    Some(prev) => merge_json(prev, v),
                    None => v,
                };
                a.insert(k, next);
            }
            Value::Object(a)
        }
        (prev, _) => prev,
    }
}

fn extract_team_ids(value: &Value) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut push = |id: i64| {
        if id > 0 && !ids.contains(&id) {
            ids.push(id);
        }
    };
    if let Some(id) = value.get("id").and_then(|x| x.as_i64()) {
        push(id);
    }
    if let Some(id) = value.get("teamId").and_then(|x| x.as_i64()) {
        push(id);
    }
    if let Some(arr) = value
        .get("teams")
        .or_else(|| value.get("data"))
        .and_then(|t| t.as_array())
    {
        for team in arr {
            if let Some(id) = team.get("id").or_else(|| team.get("teamId")).and_then(|x| x.as_i64()) {
                push(id);
            }
        }
    }
    ids
}

async fn auth_me(client: &reqwest::Client, cookie_value: &str) -> Result<Option<CursorSession>, String> {
    let resp = client
        .get("https://cursor.com/api/auth/me")
        .header("Cookie", cookie_header(cookie_value))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status == 204 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        return Err(snippet_error(status, &body));
    }
    let v: Value = serde_json::from_str(&body).map_err(|_| snippet_error(status, &body))?;
    let email = v
        .get("email")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string());
    let user_id = v.get("id").and_then(|id| id.as_i64()).or_else(|| {
        v.get("id")
            .and_then(|id| id.as_str())
            .and_then(|s| s.parse().ok())
    });
    if email.is_none() && user_id.is_none() {
        return Ok(None);
    }
    Ok(Some(CursorSession {
        cookie_value: cookie_value.to_string(),
        email,
        user_id,
    }))
}

async fn establish_session(client: &reqwest::Client) -> Result<CursorSession, String> {
    let raws = source_secrets().map_err(|e| e.to_string())?;
    if raws.is_empty() {
        return Err(
            "No Cursor session yet. Click Sign in to Cursor and stay on cursor.com until this window closes itself."
                .into(),
        );
    }
    let mut last_err = "Cookie was not accepted by cursor.com/api/auth/me".to_string();
    for raw in raws {
        for candidate in cookie_candidates(&raw) {
            match auth_me(client, &candidate).await {
                Ok(Some(session)) => return Ok(session),
                Ok(None) => {
                    last_err = "Cursor returned no identity for this cookie (204). Copy WorkosCursorSessionToken from the Network request, not a random cookie.".into();
                }
                Err(e) => last_err = e,
            }
        }
    }
    Err(last_err)
}

async fn post_dashboard(
    client: &reqwest::Client,
    session: &CursorSession,
    path: &str,
    body: Value,
) -> Result<Value, String> {
    let resp = client
        .post(format!("https://cursor.com{path}"))
        .header("Cookie", cookie_header(&session.cookie_value))
        .header("Origin", "https://cursor.com")
        .header("Referer", "https://cursor.com/dashboard?tab=usage")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(snippet_error(status, &text));
    }
    serde_json::from_str(&text).map_err(|_| snippet_error(status, &text))
}

async fn get_json(
    client: &reqwest::Client,
    session: &CursorSession,
    url: &str,
) -> Result<Value, String> {
    let resp = client
        .get(url)
        .header("Cookie", cookie_header(&session.cookie_value))
        .header("Accept", "application/json")
        .header("Origin", "https://cursor.com")
        .header("Referer", "https://cursor.com/dashboard?tab=usage")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(snippet_error(status, &text));
    }
    serde_json::from_str(&text).map_err(|_| snippet_error(status, &text))
}

fn local_cached_team_ids() -> Vec<i64> {
    let Ok(Some(raw)) = read_vscdb_string("cursorAuth/cachedTeam") else {
        return Vec::new();
    };
    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
        return extract_team_ids(&v);
    }
    Vec::new()
}

async fn fetch_team_ids(client: &reqwest::Client, session: &CursorSession) -> Vec<i64> {
    let mut ids = local_cached_team_ids();
    // GET is 405; the dashboard uses POST.
    if let Ok(v) = post_dashboard(client, session, "/api/dashboard/teams", serde_json::json!({})).await
    {
        for id in extract_team_ids(&v) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

async fn fetch_usage_documents(
    client: &reqwest::Client,
    session: &CursorSession,
    default_body: Value,
    team_ids: &[i64],
) -> Value {
    let mut merged = Value::Null;
    if let Ok(v) = get_json(client, session, "https://cursor.com/api/usage-summary").await {
        merged = merge_json(merged, v);
    }

    let mut bodies = vec![serde_json::json!({}), default_body.clone()];
    for id in team_ids {
        let mut with_id = default_body.clone();
        with_id["teamId"] = Value::from(*id);
        bodies.push(with_id);
        bodies.push(serde_json::json!({ "teamId": id }));
    }

    for body in bodies {
        if let Ok(v) = post_dashboard(
            client,
            session,
            "/api/dashboard/get-current-period-usage",
            body.clone(),
        )
        .await
        {
            merged = merge_json(merged, v);
        }
        if let Ok(v) = post_dashboard(client, session, "/api/dashboard/get-hard-limit", body).await {
            merged = merge_json(merged, v);
        }
    }
    merged
}

fn parse_event_timestamp(ev: &Value) -> String {
    let raw = ev
        .get("timestamp")
        .or_else(|| ev.get("createdAt"))
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(s) = raw.as_str() {
        if let Ok(ms) = s.parse::<i64>() {
            if let Some(dt) = Utc.timestamp_millis_opt(ms).single() {
                return dt.to_rfc3339();
            }
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return dt.with_timezone(&Utc).to_rfc3339();
        }
        return s.to_string();
    }
    if let Some(n) = raw.as_i64() {
        if let Some(dt) = Utc.timestamp_millis_opt(n).single() {
            return dt.to_rfc3339();
        }
    }
    Utc::now().to_rfc3339()
}

fn parse_usage_based_cost(value: &Value) -> Option<f64> {
    if let Some(n) = value.as_f64() {
        return Some(n);
    }
    if let Some(s) = value.as_str() {
        let cleaned = s.replace('$', "").replace(',', "");
        return cleaned.parse().ok();
    }
    None
}

fn parse_event_cost(ev: &Value) -> f64 {
    if let Some(cents) = ev.get("chargedCents").and_then(|x| x.as_f64()) {
        return cents / 100.0;
    }
    if let Some(cents) = ev
        .get("tokenUsage")
        .and_then(|t| t.get("totalCents"))
        .and_then(|x| x.as_f64())
    {
        return cents / 100.0;
    }
    if let Some(v) = ev.get("usageBasedCosts").and_then(parse_usage_based_cost) {
        return v;
    }
    0.0
}

fn parse_event_tokens(ev: &Value) -> (i64, i64, i64) {
    let tu = ev.get("tokenUsage");
    let input = tu
        .and_then(|t| t.get("inputTokens"))
        .and_then(|x| x.as_i64())
        .or_else(|| ev.get("inputTokens").and_then(|x| x.as_i64()))
        .unwrap_or(0);
    let output = tu
        .and_then(|t| t.get("outputTokens"))
        .and_then(|x| x.as_i64())
        .or_else(|| ev.get("outputTokens").and_then(|x| x.as_i64()))
        .unwrap_or(0);
    let cache = tu
        .and_then(|t| t.get("cacheWriteTokens"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0)
        + tu.and_then(|t| t.get("cacheReadTokens"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
    (input, output, cache)
}

fn collapse_preview(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(max).collect();
    if flat.chars().count() > max {
        out.push('…');
    }
    out
}

fn extract_tagged(text: &str, tag: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = lower.find(&open)? + open.len();
    let end = lower.get(start..)?.find(&close)? + start;
    let inner = text.get(start..end)?.trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    }
}

fn strip_xml_tags(text: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn clean_prompt_text(text: &str) -> String {
    let body = extract_tagged(text, "user_query")
        .or_else(|| extract_tagged(text, "user-query"))
        .unwrap_or_else(|| strip_xml_tags(text));
    collapse_preview(&body, 80)
}

fn humanize_kind(kind: &str) -> String {
    let trimmed = kind.trim_start_matches("USAGE_EVENT_KIND_").replace('_', " ");
    trimmed
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn user_message_text(value: &Value) -> Option<String> {
    let msg = value.get("message")?;
    if let Some(s) = msg.as_str() {
        return Some(s.to_string());
    }
    let content = msg.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    let arr = content.as_array()?;
    for part in arr {
        if let Some(text) = part.get("text").and_then(|x| x.as_str()) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn prompt_body(raw: &str) -> Option<String> {
    let body = extract_tagged(raw, "user_query")
        .or_else(|| extract_tagged(raw, "user-query"))
        .unwrap_or_else(|| strip_xml_tags(raw));
    let flat: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() < 8 {
        None
    } else {
        Some(flat)
    }
}

fn is_main_agent_transcript(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(grandparent) = parent.parent() else {
        return false;
    };
    grandparent.file_name().and_then(|n| n.to_str()) == Some("agent-transcripts")
        && parent.file_name().and_then(|n| n.to_str()) == path.file_stem().and_then(|n| n.to_str())
}

fn all_user_prompts_from_transcript(path: &Path) -> Vec<String> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut prompts = Vec::new();
    let mut last = String::new();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let Some(text) = user_message_text(&v) else {
            continue;
        };
        let Some(body) = prompt_body(&text) else {
            continue;
        };
        if body == last {
            continue;
        }
        prompts.push(body.clone());
        last = body;
    }
    prompts
}

fn remember_prompts(
    prompts: &mut HashMap<String, Vec<String>>,
    ranks: &mut HashMap<String, u8>,
    id: &str,
    entries: Vec<String>,
    rank: u8,
) {
    let id = id.trim();
    if id.is_empty() || entries.is_empty() {
        return;
    }
    if ranks.get(id).copied().unwrap_or(u8::MAX) <= rank {
        return;
    }
    ranks.insert(id.to_string(), rank);
    prompts.insert(id.to_string(), entries);
}

fn load_conversation_prompts() -> HashMap<String, Vec<String>> {
    let mut prompts = HashMap::new();
    let mut ranks = HashMap::new();
    let Some(home) = dirs::home_dir() else {
        return prompts;
    };

    let projects = home.join(".cursor").join("projects");
    if projects.exists() {
        for entry in walkdir::WalkDir::new(&projects).max_depth(4).into_iter().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") || !is_main_agent_transcript(path)
            {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|n| n.to_str()) else {
                continue;
            };
            let entries = all_user_prompts_from_transcript(path);
            remember_prompts(&mut prompts, &mut ranks, id, entries, 0);
        }
    }

    let chats = home.join(".cursor").join("chats");
    if chats.exists() {
        for entry in walkdir::WalkDir::new(&chats).max_depth(3).into_iter().flatten() {
            if entry.file_name() != "meta.json" {
                continue;
            }
            let Ok(raw) = fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            if let Some(title) = v.get("title").and_then(|t| t.as_str()) {
                if let Some(id) = entry.path().parent().and_then(|p| p.file_name()).and_then(|n| n.to_str())
                {
                    if let Some(body) = prompt_body(title) {
                        remember_prompts(&mut prompts, &mut ranks, id, vec![body], 1);
                    }
                }
            }
        }
    }

    let workspaces = home
        .join("Library")
        .join("Application Support")
        .join("Cursor")
        .join("User")
        .join("workspaceStorage");
    if workspaces.exists() {
        for entry in walkdir::WalkDir::new(&workspaces).max_depth(2).into_iter().flatten() {
            if entry.file_name() != "state.vscdb" {
                continue;
            }
            let Ok(conn) = Connection::open_with_flags(
                entry.path(),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) else {
                continue;
            };
            let Ok(raw) = conn.query_row(
                "SELECT value FROM ItemTable WHERE key = 'composer.composerData'",
                [],
                |r| r.get::<_, String>(0),
            ) else {
                continue;
            };
            let Ok(data) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let Some(composers) = data.get("allComposers").and_then(|c| c.as_array()) else {
                continue;
            };
            for composer in composers {
                let Some(id) = composer.get("composerId").and_then(|x| x.as_str()) else {
                    continue;
                };
                if let Some(name) = composer.get("name").and_then(|x| x.as_str()) {
                    if let Some(body) = prompt_body(name) {
                        remember_prompts(&mut prompts, &mut ranks, id, vec![body], 2);
                    }
                } else if let Some(subtitle) = composer.get("subtitle").and_then(|x| x.as_str()) {
                    if let Some(body) = prompt_body(subtitle) {
                        remember_prompts(&mut prompts, &mut ranks, id, vec![body], 3);
                    }
                }
            }
        }
    }

    prompts
}

fn conversation_id(ev: &Value) -> Option<&str> {
    ev.get("conversationId")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty() && *s != "null")
}

fn event_timestamp_ms(ev: &Value) -> i64 {
    let raw = ev
        .get("timestamp")
        .or_else(|| ev.get("createdAt"))
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(s) = raw.as_str() {
        if let Ok(ms) = s.parse::<i64>() {
            return ms;
        }
    }
    raw.as_i64().unwrap_or(0)
}

fn fallback_title(ev: &Value) -> String {
    if let Some(id) = conversation_id(ev) {
        return format!("Chat {}", &id[..id.len().min(8)]);
    }
    ev.get("kind")
        .and_then(|k| k.as_str())
        .map(humanize_kind)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Cursor request".into())
}

fn assign_event_titles(events: &[Value], prompts: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut titles = vec![String::new(); events.len()];
    let mut groups: HashMap<String, Vec<(i64, usize)>> = HashMap::new();

    for (idx, ev) in events.iter().enumerate() {
        if let Some(id) = conversation_id(ev) {
            groups
                .entry(id.to_string())
                .or_default()
                .push((event_timestamp_ms(ev), idx));
        } else {
            titles[idx] = fallback_title(ev);
        }
    }

    for (id, mut items) in groups {
        items.sort_by_key(|(ts, _)| *ts);
        let n = items.len().max(1);
        if let Some(list) = prompts.get(&id).filter(|l| !l.is_empty()) {
            let m = list.len();
            for (i, (_, idx)) in items.iter().enumerate() {
                let prompt_idx = if m == 1 {
                    0
                } else {
                    ((i * m) / n).min(m - 1)
                };
                titles[*idx] = collapse_preview(&list[prompt_idx], 80);
            }
        } else {
            for (_, idx) in &items {
                titles[*idx] = format!("Chat {}", &id[..id.len().min(8)]);
            }
        }
    }

    titles
}

fn ingest_events(db: &Db, events: &[Value], titles: &[String]) -> Result<u32> {
    let mut n = 0u32;
    for (ev, title) in events.iter().zip(titles.iter()) {
        let started = parse_event_timestamp(ev);
        let model = ev
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("cursor")
            .to_string();
        let run_id = ev
            .get("requestId")
            .or_else(|| ev.get("id"))
            .or_else(|| ev.get("usageEventId"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{started}-{model}-{n}"));
        let (input, output, cache) = parse_event_tokens(ev);
        let cost = parse_event_cost(ev);
        db.upsert_run(&RunRecord {
            provider: "cursor".into(),
            run_id,
            started_at: started,
            model,
            input_tokens: input,
            output_tokens: output,
            cache_tokens: cache,
            cost_usd: cost,
            title: if title.is_empty() {
                fallback_title(ev)
            } else {
                title.clone()
            },
            conversation_id: conversation_id(ev).unwrap_or("").to_string(),
            task_context: String::new(),
            task_inherited: false,
        })?;
        n += 1;
    }
    Ok(n)
}

fn quota_from_period(usage: &Value, db: &Db, email: Option<&str>, events: u32) -> QuotaSnapshot {
    let spend = db.provider_month_spend("cursor").unwrap_or(0.0);
    let cycle = extract_cycle_usage(usage, spend);
    let used_usd = cycle.used_usd.or(Some(spend));
    let who = email.map(|e| format!(" as {e}")).unwrap_or_default();
    let pct_label = cycle
        .percent
        .map(|p| format!("{p:.0}"))
        .unwrap_or_else(|| "—".into());
    let money = match (used_usd, cycle.limit_usd) {
        (Some(used), Some(limit)) => format!(" · ${used:.0}/${limit:.0}"),
        _ => String::new(),
    };
    QuotaSnapshot {
        provider: "cursor".into(),
        label: "Cursor".into(),
        used_percent_month: cycle.percent,
        used_percent_year: None,
        used_tokens_month: Some(db.provider_month_tokens("cursor").unwrap_or(0)),
        limit_tokens_month: None,
        used_usd_month: used_usd,
        limit_usd_month: cycle.limit_usd,
        primary_window_label: None,
        primary_window_percent: None,
        secondary_window_label: None,
        secondary_window_percent: None,
        resets_at: cycle.resets_at,
        status: "ok".into(),
        message: Some(format!(
            "Connected{who}. {events} runs this sync · cycle {pct_label}%{money}"
        )),
    }
}

fn needs_session(message: Option<String>) -> QuotaSnapshot {
    QuotaSnapshot {
        provider: "cursor".into(),
        label: "Cursor".into(),
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
        status: "needs_session".into(),
        message: message.or_else(|| {
            Some(
                "Paste WorkosCursorSessionToken from DevTools Network → a cursor.com request → Cookie"
                    .into(),
            )
        }),
    }
}

fn disabled(provider: &str, label: &str) -> QuotaSnapshot {
    QuotaSnapshot {
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
    }
}

async fn sync_with_session(
    db: &Db,
    settings: &AppSettings,
    client: &reqwest::Client,
    session: &CursorSession,
) -> Result<(u32, QuotaSnapshot), String> {
    let cycle = billing_cycle_start(Utc::now(), settings.billing_anchor_day);
    let start_ms = cycle.timestamp_millis();
    let end_ms = Utc::now().timestamp_millis();
    let mut body = serde_json::json!({
        "startDate": start_ms.to_string(),
        "endDate": end_ms.to_string(),
    });
    if let Some(id) = session.user_id {
        body["userId"] = json_num_or_int(id);
    }

    let team_ids = fetch_team_ids(client, session).await;
    if let Some(team_id) = team_ids.first() {
        body["teamId"] = Value::from(*team_id);
    }
    let period = fetch_usage_documents(client, session, body.clone(), &team_ids).await;
    let prompts = load_conversation_prompts();
    let mut all_events = Vec::new();

    for page in 1..=8 {
        let mut page_body = body.clone();
        page_body["page"] = Value::from(page);
        page_body["pageSize"] = Value::from(100);
        let v = match post_dashboard(
            client,
            session,
            "/api/dashboard/get-filtered-usage-events",
            page_body,
        )
        .await
        {
            Ok(v) => v,
            Err(_) if page == 1 => {
                // Broad window if the billing-cycle filter comes back empty/invalid.
                let mut fallback = serde_json::json!({
                    "startDate": "0",
                    "endDate": "9999999999999",
                    "page": 1,
                    "pageSize": 100,
                    "userId": session.user_id,
                });
                if let Some(team_id) = team_ids.first() {
                    fallback["teamId"] = Value::from(*team_id);
                }
                post_dashboard(
                    client,
                    session,
                    "/api/dashboard/get-filtered-usage-events",
                    fallback,
                )
                .await?
            }
            Err(e) => return Err(e),
        };
        let events = v
            .get("usageEventsDisplay")
            .or_else(|| v.get("events"))
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        let n = events.len();
        all_events.extend(events);
        if n < 100 {
            break;
        }
    }

    let titles = assign_event_titles(&all_events, &prompts);
    let mut events_count =
        ingest_events(db, &all_events, &titles).map_err(|e| e.to_string())? as u32;
    let _ = db.delete_runs_with_title_prefix("cursor", "USAGE_EVENT_KIND");

    if events_count == 0 {
        if let Ok(agg) = post_dashboard(
            client,
            session,
            "/api/dashboard/get-aggregated-usage-events",
            body,
        )
        .await
        {
            if let Some(rows) = agg.get("aggregations").and_then(|a| a.as_array()) {
                for (i, row) in rows.iter().enumerate() {
                    let model = row
                        .get("model")
                        .and_then(|m| m.as_str())
                        .unwrap_or("cursor")
                        .to_string();
                    let input = row.get("inputTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                    let output = row.get("outputTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                    let cache = row.get("cacheReadTokens").and_then(|x| x.as_i64()).unwrap_or(0)
                        + row
                            .get("cacheWriteTokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0);
                    let cost = row
                        .get("totalCents")
                        .and_then(|x| x.as_f64())
                        .unwrap_or(0.0)
                        / 100.0;
                    db.upsert_run(&RunRecord {
                        provider: "cursor".into(),
                        run_id: format!("agg-{model}-{i}"),
                        started_at: Utc::now().to_rfc3339(),
                        model,
                        input_tokens: input,
                        output_tokens: output,
                        cache_tokens: cache,
                        cost_usd: cost,
                        title: "Aggregated Cursor usage".into(),
                        conversation_id: String::new(),
                        task_context: String::new(),
                        task_inherited: false,
                    })
                    .map_err(|e| e.to_string())?;
                    events_count += 1;
                }
            }
        }
    }

    let quota = quota_from_period(&period, db, session.email.as_deref(), events_count);
    db.save_quota(&quota).map_err(|e| e.to_string())?;
    Ok((events_count, quota))
}

fn json_num_or_int(id: i64) -> Value {
    Value::from(id)
}

impl CursorProvider {
    pub async fn probe(db: &Db, settings: &AppSettings) -> CursorProbeResult {
        Self::probe_with_cookie(db, settings, None).await
    }

    pub async fn probe_with_cookie(
        db: &Db,
        settings: &AppSettings,
        cookie: Option<String>,
    ) -> CursorProbeResult {
        if let Some(value) = cookie.as_deref() {
            remember_session_cookie(value);
            let _ = keychain::set_secret("cursor_session_token", value.trim());
        }
        let client = cursor_client();
        match establish_session(&client).await {
            Ok(session) => match sync_with_session(db, settings, &client, &session).await {
                Ok((events, quota)) => CursorProbeResult {
                    ok: true,
                    email: session.email,
                    events,
                    percent: quota.used_percent_month,
                    error: None,
                },
                Err(e) => CursorProbeResult {
                    ok: false,
                    email: session.email,
                    events: 0,
                    percent: None,
                    error: Some(e),
                },
            },
            Err(e) => CursorProbeResult {
                ok: false,
                email: None,
                events: 0,
                percent: None,
                error: Some(e),
            },
        }
    }
}

#[async_trait]
impl Provider for CursorProvider {
    fn id(&self) -> &'static str {
        "cursor"
    }

    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()> {
        if !settings.enabled_providers.iter().any(|p| p == "cursor") {
            return Ok(());
        }
        let result = Self::probe(db, settings).await;
        if !result.ok {
            db.save_quota(&needs_session(result.error))?;
        }
        Ok(())
    }

    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot> {
        if !settings.enabled_providers.contains(&"cursor".to_string()) {
            return Ok(disabled("cursor", "Cursor"));
        }
        if let Ok(list) = db.list_quotas() {
            if let Some(q) = list.into_iter().find(|q| q.provider == "cursor") {
                return Ok(q);
            }
        }
        Ok(needs_session(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn personal_pro_uses_plan_percent() {
        let usage = json!({
            "billingCycleEnd": "2026-11-01T00:00:00.000Z",
            "planUsage": {
                "totalSpend": 1529,
                "limit": 2000,
                "totalPercentUsed": 10.19
            }
        });
        let cycle = extract_cycle_usage(&usage, 0.0);
        assert_eq!(cycle.percent, Some(10.19));
        assert_eq!(cycle.limit_usd, Some(20.0));
        assert!((cycle.used_usd.unwrap() - 15.29).abs() < 0.001);
    }

    #[test]
    fn team_on_demand_limit_becomes_percent() {
        let usage = json!({
            "membershipType": "enterprise",
            "limitType": "team",
            "billingCycleEnd": "2026-11-01T00:00:00.000Z",
            "teamUsage": {
                "onDemand": {
                    "enabled": true,
                    "used": 21752,
                    "limit": 20000,
                    "remaining": -1752
                }
            }
        });
        let cycle = extract_cycle_usage(&usage, 0.0);
        assert!((cycle.percent.unwrap() - 108.76).abs() < 0.01);
        assert_eq!(cycle.limit_usd, Some(200.0));
        assert!((cycle.used_usd.unwrap() - 217.52).abs() < 0.001);
    }

    #[test]
    fn team_spend_limit_preferred_over_missing_plan_percent() {
        let usage = json!({
            "billingCycleEnd": "2026-11-01T00:00:00.000Z",
            "spendLimitUsage": {
                "limitType": "team",
                "pooledUsed": 15000,
                "pooledLimit": 20000,
                "pooledRemaining": 5000
            }
        });
        let cycle = extract_cycle_usage(&usage, 50.0);
        assert_eq!(cycle.percent, Some(75.0));
        assert_eq!(cycle.limit_usd, Some(200.0));
        assert_eq!(cycle.used_usd, Some(150.0));
    }

    #[test]
    fn usage_summary_individual_plan_still_works() {
        let usage = json!({
            "individualUsage": {
                "plan": {
                    "used": 500,
                    "limit": 2000,
                    "totalPercentUsed": 25.0
                }
            }
        });
        let cycle = extract_cycle_usage(&usage, 0.0);
        assert_eq!(cycle.percent, Some(25.0));
    }

    #[test]
    fn event_spend_fills_percent_when_only_team_limit_is_present() {
        let usage = json!({
            "teamUsage": {
                "onDemand": { "limit": 20000 }
            }
        });
        let cycle = extract_cycle_usage(&usage, 217.52);
        assert!((cycle.percent.unwrap() - 108.76).abs() < 0.01);
        assert_eq!(cycle.limit_usd, Some(200.0));
    }

    #[test]
    fn display_message_is_last_resort() {
        let usage = json!({
            "displayMessage": "You've used 46% of your usage limit"
        });
        let cycle = extract_cycle_usage(&usage, 0.0);
        assert_eq!(cycle.percent, Some(46.0));
    }

    #[test]
    fn assign_event_titles_maps_later_calls_to_later_prompts() {
        let mut prompts = HashMap::new();
        prompts.insert(
            "conv-1".into(),
            vec![
                "how can i layout this ide".into(),
                "never mind".into(),
                "fix the live blog bug".into(),
            ],
        );
        let events = vec![
            json!({"conversationId":"conv-1","timestamp":"1000"}),
            json!({"conversationId":"conv-1","timestamp":"2000"}),
            json!({"conversationId":"conv-1","timestamp":"3000"}),
        ];
        let titles = assign_event_titles(&events, &prompts);
        assert!(titles[0].contains("layout"));
        assert!(titles[1].contains("never mind"));
        assert!(titles[2].contains("live blog"));
    }

    #[test]
    fn assign_event_titles_falls_back_without_transcript() {
        let events = vec![json!({
            "conversationId":"fbcfb9bd-843c-4e4d-b30e-2a5227d5eedc",
            "kind":"USAGE_EVENT_KIND_INCLUDED_IN_BUSINESS"
        })];
        let titles = assign_event_titles(&events, &HashMap::new());
        assert_eq!(titles[0], "Chat fbcfb9bd");
    }

    #[test]
    fn fallback_title_humanizes_kind_when_conversation_is_missing() {
        let ev = json!({ "kind": "USAGE_EVENT_KIND_INCLUDED_IN_BUSINESS" });
        assert_eq!(fallback_title(&ev), "Included In Business");
    }

    #[test]
    fn user_message_preview_reads_transcript_shape() {
        let v = json!({
            "role": "user",
            "message": { "content": [{ "type": "text", "text": "can you debug why the cursor circle is empty" }] }
        });
        assert_eq!(
            user_message_text(&v).as_deref(),
            Some("can you debug why the cursor circle is empty")
        );
        assert_eq!(
            collapse_preview(user_message_text(&v).unwrap().as_str(), 20),
            "can you debug why th…"
        );
    }

    #[test]
    fn clean_prompt_extracts_user_query_and_drops_xml() {
        let raw = "<timestamp>Thursday, Oct 29, 2026, 4:36 PM (UTC-7)</timestamp>\n<user_query>\ncan you debug why the cursor circle is empty\n</user_query>";
        assert_eq!(
            clean_prompt_text(raw),
            "can you debug why the cursor circle is empty"
        );
        assert_eq!(
            clean_prompt_text("<user_query> how can i layout this ide with the agent dialog on the right and files on the left</user_query>"),
            "how can i layout this ide with the agent dialog on the right and files on the le…"
        );
    }

    #[test]
    fn enterprise_overall_limit_beats_zero_included_message() {
        let usage = json!({
            "billingCycleStart": "2026-10-01T00:00:00.000Z",
            "billingCycleEnd": "2026-11-01T00:00:00.000Z",
            "membershipType": "enterprise",
            "limitType": "team",
            "autoModelSelectedDisplayMessage": "You've used 0% of your included total usage",
            "namedModelSelectedDisplayMessage": "You've used 0% of your included API usage",
            "individualUsage": {
                "overall": {
                    "enabled": true,
                    "used": 11132,
                    "limit": 20000,
                    "remaining": 8868
                }
            },
            "teamUsage": {
                "onDemand": {
                    "enabled": false,
                    "used": 0,
                    "limit": null,
                    "remaining": null
                }
            }
        });
        let cycle = extract_cycle_usage(&usage, 219.40);
        assert!((cycle.percent.unwrap() - 55.66).abs() < 0.02);
        assert_eq!(cycle.limit_usd, Some(200.0));
        assert!((cycle.used_usd.unwrap() - 111.32).abs() < 0.001);
    }
}
