use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

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

fn ingest_events(db: &Db, events: &[Value]) -> Result<u32> {
    let mut n = 0u32;
    for ev in events {
        let started = parse_event_timestamp(ev);
        let model = ev
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("cursor")
            .to_string();
        let run_id = ev
            .get("requestId")
            .or_else(|| ev.get("id"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{started}-{model}-{n}"));
        let (input, output, cache) = parse_event_tokens(ev);
        let cost = parse_event_cost(ev);
        let kind = ev
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("Cursor request");
        db.upsert_run(&RunRecord {
            provider: "cursor".into(),
            run_id,
            started_at: started,
            model,
            input_tokens: input,
            output_tokens: output,
            cache_tokens: cache,
            cost_usd: cost,
            title: kind.to_string(),
        })?;
        n += 1;
    }
    Ok(n)
}

fn quota_from_period(usage: &Value, db: &Db, email: Option<&str>, events: u32) -> QuotaSnapshot {
    let plan = usage
        .pointer("/planUsage")
        .cloned()
        .or_else(|| usage.pointer("/individualUsage/plan").cloned())
        .unwrap_or(Value::Null);
    let pct = plan
        .get("totalPercentUsed")
        .and_then(|x| x.as_f64())
        .or_else(|| {
            let used = plan.get("used").or_else(|| plan.get("totalSpend"));
            let limit = plan.get("limit");
            match (
                used.and_then(|x| x.as_f64()),
                limit.and_then(|x| x.as_f64()),
            ) {
                (Some(u), Some(l)) if l > 0.0 => Some((u / l * 100.0).min(999.0)),
                _ => None,
            }
        });
    let spend = db.provider_month_spend("cursor").unwrap_or(0.0);
    let who = email.map(|e| format!(" as {e}")).unwrap_or_default();
    QuotaSnapshot {
        provider: "cursor".into(),
        label: "Cursor".into(),
        used_percent_month: pct,
        used_percent_year: None,
        used_tokens_month: Some(db.provider_month_tokens("cursor").unwrap_or(0)),
        limit_tokens_month: None,
        used_usd_month: Some(spend),
        limit_usd_month: None,
        primary_window_label: None,
        primary_window_percent: None,
        secondary_window_label: None,
        secondary_window_percent: None,
        resets_at: usage
            .get("billingCycleEnd")
            .and_then(|x| x.as_str())
            .map(String::from),
        status: "ok".into(),
        message: Some(format!(
            "Connected{who}. {events} runs this sync · cycle {}%",
            pct.map(|p| format!("{p:.0}")).unwrap_or_else(|| "—".into())
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
        "teamId": 0,
        "startDate": start_ms.to_string(),
        "endDate": end_ms.to_string(),
    });
    if let Some(id) = session.user_id {
        body["userId"] = json_num_or_int(id);
    }

    let mut period: Option<Value> = None;
    if let Ok(v) = post_dashboard(client, session, "/api/dashboard/get-current-period-usage", body.clone()).await
    {
        period = Some(v);
    } else if let Ok(v) = client
        .get("https://cursor.com/api/usage-summary")
        .header("Cookie", cookie_header(&session.cookie_value))
        .header("Accept", "application/json")
        .send()
        .await
    {
        if v.status().is_success() {
            if let Ok(text) = v.text().await {
                if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                    period = Some(parsed);
                }
            }
        }
    }

    let mut events_count = 0u32;
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
                let fallback = serde_json::json!({
                    "teamId": 0,
                    "startDate": "0",
                    "endDate": "9999999999999",
                    "page": 1,
                    "pageSize": 100,
                    "userId": session.user_id,
                });
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
        events_count += ingest_events(db, &events).map_err(|e| e.to_string())?;
        if n < 100 {
            break;
        }
    }

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
                    })
                    .map_err(|e| e.to_string())?;
                    events_count += 1;
                }
            }
        }
    }

    let quota = quota_from_period(
        period.as_ref().unwrap_or(&Value::Null),
        db,
        session.email.as_deref(),
        events_count,
    );
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
