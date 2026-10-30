mod db;
mod keychain;
mod models;
mod pricing;
mod providers;
mod sync;

use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_autostart::MacosLauncher;

use crate::db::Db;
use crate::models::{AppSettings, ModelUsageRow, RunRecord, UsageSummary};
use crate::pricing::plan_catalog;
use crate::sync::{SyncEngine, spawn_file_watchers, spawn_poll_loop};

struct AppState {
    db: Arc<Db>,
    engine: Arc<SyncEngine>,
}

#[tauri::command]
async fn get_usage_summary(state: State<'_, AppState>) -> Result<UsageSummary, String> {
    state.engine.summary().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn sync_now(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<UsageSummary, String> {
    state
        .engine
        .sync_all(Some(app))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_runs(
    state: State<'_, AppState>,
    provider: Option<String>,
    limit: Option<u32>,
    sort: Option<String>,
) -> Result<Vec<RunRecord>, String> {
    let sort = sort.unwrap_or_else(|| "cost".into());
    state
        .db
        .list_runs(provider.as_deref(), limit.unwrap_or(200), &sort)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_provider_model_usage(
    state: State<'_, AppState>,
    provider: String,
) -> Result<Vec<ModelUsageRow>, String> {
    let settings = state.db.load_settings().map_err(|e| e.to_string())?;
    state
        .db
        .provider_model_breakdown(&provider, settings.billing_anchor_day)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state.db.load_settings().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> Result<(), String> {
    state.db.save_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_plan_catalog() -> Vec<crate::models::ProviderPlan> {
    plan_catalog()
}

#[tauri::command]
fn set_secret(provider: String, value: String) -> Result<(), String> {
    if value.trim().is_empty() {
        keychain::delete_secret(&provider).map_err(|e| e.to_string())
    } else {
        keychain::set_secret(&provider, value.trim()).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn secret_configured(provider: String) -> Result<bool, String> {
    Ok(keychain::get_secret(&provider)
        .map_err(|e| e.to_string())?
        .is_some())
}

#[tauri::command]
async fn test_cursor_auth(state: State<'_, AppState>) -> Result<crate::providers::cursor::CursorProbeResult, String> {
    let settings = state.db.load_settings().map_err(|e| e.to_string())?;
    Ok(crate::providers::cursor::CursorProvider::probe(state.db.as_ref(), &settings).await)
}

fn is_cursor_session_cookie(name: &str, value: &str) -> bool {
    if value.trim().is_empty() {
        return false;
    }
    let n = name.to_lowercase();
    n == "workoscursorsessiontoken"
        || (n.contains("workos") && n.contains("session"))
        || (n.contains("cursor") && n.contains("session") && n.contains("token"))
        || ((value.contains("%3A%3A") || value.contains("::")) && value.contains("eyJ"))
}

fn cookies_on_main(app: &tauri::AppHandle, win: WebviewWindow) -> Vec<(String, String)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = app.run_on_main_thread(move || {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut push = |cookies: Vec<tauri::webview::Cookie<'static>>| {
            for c in cookies {
                out.push((c.name().to_string(), c.value().to_string()));
            }
        };
        if let Ok(c) = win.cookies() {
            push(c);
        }
        for raw in [
            "https://cursor.com/",
            "https://www.cursor.com/",
            "https://cursor.com/dashboard",
            "https://authenticator.cursor.sh/",
        ] {
            if let Ok(url) = raw.parse() {
                if let Ok(c) = win.cookies_for_url(url) {
                    push(c);
                }
            }
        }
        let _ = tx.send(out);
    });
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or_default()
}

fn harvest_cursor_cookie(app: &tauri::AppHandle) -> (Option<String>, Vec<String>) {
    let mut names = Vec::new();
    for (_label, win) in app.webview_windows() {
        for (name, value) in cookies_on_main(app, win) {
            if !names.contains(&name) {
                names.push(name.clone());
            }
            if is_cursor_session_cookie(&name, &value) {
                return (Some(value), names);
            }
        }
    }
    (None, names)
}

#[tauri::command]
async fn connect_cursor(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::providers::cursor::CursorProbeResult, String> {
    const LABEL: &str = "cursor-login";
    if let Some(existing) = app.get_webview_window(LABEL) {
        let _ = existing.show();
        let _ = existing.set_focus();
    } else {
        let url = "https://cursor.com/dashboard"
            .parse()
            .map_err(|e| format!("{e}"))?;
        WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::External(url))
            .title("Sign in to Cursor")
            .inner_size(980.0, 760.0)
            .center()
            .build()
            .map_err(|e| e.to_string())?;
    }

    let last_names = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let last_names_task = last_names.clone();
    let app_poll = app.clone();
    let token = tokio::time::timeout(std::time::Duration::from_secs(180), async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            if app_poll.get_webview_window(LABEL).is_none() {
                return Err("Sign-in window was closed before a Cursor session cookie was captured.".to_string());
            }
            let (found, names) = harvest_cursor_cookie(&app_poll);
            *last_names_task.lock().unwrap() = names;
            if let Some(token) = found {
                return Ok(token);
            }
        }
    })
    .await
    .map_err(|_| {
        let names = last_names.lock().unwrap().clone();
        let seen = if names.is_empty() {
            "WebKit returned no cookies (sign-in may have opened in an external browser). Stay inside the Token Monitor window.".to_string()
        } else {
            format!("Saw cookies [{}] but not WorkosCursorSessionToken.", names.join(", "))
        };
        format!("Timed out waiting for Cursor login. {seen}")
    })??;

    crate::providers::cursor::remember_session_cookie(&token);
    let _ = keychain::set_secret("cursor_session_token", &token);
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.close();
    }

    let settings = state.db.load_settings().map_err(|e| e.to_string())?;
    Ok(crate::providers::cursor::CursorProvider::probe_with_cookie(
        state.db.as_ref(),
        &settings,
        Some(token),
    )
    .await)
}

#[tauri::command]
async fn show_window(
    app: tauri::AppHandle,
    label: String,
    provider: Option<String>,
) -> Result<(), String> {
    show_labeled_window(&app, &label, provider.as_deref())
}

#[tauri::command]
async fn hide_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(&label) {
        win.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn save_widget_position(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let mut settings = state.db.load_settings().map_err(|e| e.to_string())?;
    settings.widget_x = Some(x);
    settings.widget_y = Some(y);
    state.db.save_settings(&settings).map_err(|e| e.to_string())?;
    if let Some(win) = app.get_webview_window("widget") {
        let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    }
    Ok(())
}

fn intercept_close(win: &WebviewWindow) {
    let hidden = win.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hidden.hide();
        }
    });
}

fn show_labeled_window(
    app: &tauri::AppHandle,
    label: &str,
    provider: Option<&str>,
) -> Result<(), String> {
    if app.get_webview_window(label).is_none() {
        recreate_window(app, label)?;
    }
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.unminimize();
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    if label == "details" {
        let filter = provider.unwrap_or("all");
        let _ = app.emit("details-filter", filter);
    }
    Ok(())
}

fn recreate_window(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    let (title, url, width, height, transparent, always_on_top, decorations, skip_taskbar) =
        match label {
            "widget" => (
                "Token Monitor",
                "/",
                340.0,
                160.0,
                true,
                true,
                false,
                true,
            ),
            "details" => (
                "Usage Details",
                "/details",
                960.0,
                720.0,
                false,
                false,
                true,
                false,
            ),
            "settings" => (
                "Token Monitor Settings",
                "/settings",
                640.0,
                760.0,
                false,
                false,
                true,
                false,
            ),
            _ => return Err(format!("unknown window {label}")),
        };

    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(width, height)
        .resizable(true)
        .visible(true)
        .decorations(decorations)
        .transparent(transparent)
        .always_on_top(always_on_top)
        .skip_taskbar(skip_taskbar);

    if label != "widget" {
        builder = builder.center();
    } else {
        builder = builder.min_inner_size(280.0, 110.0);
    }

    let win = builder.build().map_err(|e| e.to_string())?;
    intercept_close(&win);
    Ok(())
}

fn restore_widget_position(app: &tauri::AppHandle, settings: &AppSettings) {
    if let (Some(x), Some(y)) = (settings.widget_x, settings.widget_y) {
        if let Some(win) = app.get_webview_window("widget") {
            let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
        }
    }
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show_widget", "Show widget", true, None::<&str>)?;
    let details_i = MenuItem::with_id(app, "show_details", "Usage details", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "show_settings", "Settings", true, None::<&str>)?;
    let sync_i = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show_i, &details_i, &settings_i, &sync_i, &quit_i],
    )?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("Token Monitor")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show_widget" => {
                let _ = show_labeled_window(app, "widget", None);
            }
            "show_details" => {
                let _ = show_labeled_window(app, "details", None);
            }
            "show_settings" => {
                let _ = show_labeled_window(app, "settings", None);
            }
            "sync" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(state) = app.try_state::<AppState>() {
                        let _ = state.engine.sync_all(Some(app.clone())).await;
                    }
                });
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let _ = show_labeled_window(app, "widget", None);
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = Arc::new(Db::open().expect("db open"));
    let engine = Arc::new(SyncEngine::new(db.clone()));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(AppState {
            db: db.clone(),
            engine: engine.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            get_usage_summary,
            sync_now,
            list_runs,
            get_provider_model_usage,
            get_settings,
            save_settings,
            get_plan_catalog,
            set_secret,
            secret_configured,
            test_cursor_auth,
            connect_cursor,
            show_window,
            hide_window,
            save_widget_position,
        ])
        .setup(move |app| {
            build_tray(app.handle())?;
            for label in ["widget", "details", "settings"] {
                if let Some(win) = app.get_webview_window(label) {
                    intercept_close(&win);
                }
            }
            let settings = db.load_settings().unwrap_or_default();
            restore_widget_position(app.handle(), &settings);
            spawn_file_watchers(app.handle().clone(), engine.clone());
            spawn_poll_loop(app.handle().clone(), engine.clone());
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = engine.sync_all(Some(app_handle)).await;
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
