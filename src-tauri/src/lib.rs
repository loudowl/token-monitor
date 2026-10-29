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
    Manager, State,
};
use tauri_plugin_autostart::MacosLauncher;

use crate::db::Db;
use crate::models::{AppSettings, RunRecord, UsageSummary};
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
async fn show_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(&label) {
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
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
                if let Some(w) = app.get_webview_window("widget") {
                    let _ = w.show();
                }
            }
            "show_details" => {
                if let Some(w) = app.get_webview_window("details") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "show_settings" => {
                if let Some(w) = app.get_webview_window("settings") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
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
                if let Some(w) = app.get_webview_window("widget") {
                    let _ = w.show();
                }
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
            get_settings,
            save_settings,
            get_plan_catalog,
            set_secret,
            secret_configured,
            show_window,
            hide_window,
            save_widget_position,
        ])
        .setup(move |app| {
            build_tray(app.handle())?;
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
