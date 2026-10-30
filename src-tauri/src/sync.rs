use anyhow::Result;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::db::Db;
use crate::models::UsageSummary;
use crate::providers;

pub struct SyncEngine {
    db: Arc<Db>,
    syncing: Mutex<bool>,
}

impl SyncEngine {
    pub fn new(db: Arc<Db>) -> Self {
        Self {
            db,
            syncing: Mutex::new(false),
        }
    }

    pub async fn sync_all(&self, app: Option<AppHandle>) -> Result<UsageSummary> {
        let mut guard = self.syncing.lock().await;
        if *guard {
            return self.summary().await;
        }
        *guard = true;
        drop(guard);

        let settings = self.db.load_settings()?;
        for p in providers::all_providers() {
            if settings.enabled_providers.iter().any(|id| id == p.id()) {
                let _ = p.sync(self.db.as_ref(), &settings).await;
                if let Ok(q) = p.quota(self.db.as_ref(), &settings).await {
                    let _ = self.db.save_quota(&q);
                }
            }
        }

        let summary = self.summary().await?;

        {
            let mut guard = self.syncing.lock().await;
            *guard = false;
        }

        if let Some(handle) = app {
            let _ = handle.emit("usage-updated", &summary);
        }
        Ok(summary)
    }

    pub async fn summary(&self) -> Result<UsageSummary> {
        use chrono::Utc;
        use crate::db::{billing_cycle_start, projected_month_spend};

        let settings = self.db.load_settings()?;
        let quotas = self.db.list_quotas()?;
        let month_spend = self.db.month_spend_usd()?;
        let cycle = billing_cycle_start(Utc::now(), settings.billing_anchor_day);
        let projected = projected_month_spend(month_spend, cycle, Utc::now());
        Ok(UsageSummary {
            quotas,
            projected_month_usd: projected,
            month_spend_usd: month_spend,
        })
    }
}

pub fn spawn_file_watchers(app: AppHandle, engine: Arc<SyncEngine>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let mut paths: Vec<PathBuf> = Vec::new();
        if let Some(h) = dirs::home_dir() {
            paths.push(h.join(".codex").join("sessions"));
            paths.push(h.join(".codex").join("archived_sessions"));
            paths.push(h.join(".codex").join("auth.json"));
            paths.push(h.join(".claude").join("projects"));
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default().with_poll_interval(Duration::from_secs(2)),
        )
        .expect("watcher");

        for p in paths {
            if p.exists() {
                let _ = watcher.watch(&p, RecursiveMode::Recursive);
            }
        }

        loop {
            if rx.recv().is_ok() {
                let app_clone = app.clone();
                let engine_clone = engine.clone();
                rt.block_on(async move {
                    let _ = engine_clone.sync_all(Some(app_clone)).await;
                });
            }
        }
    });
}

pub fn spawn_poll_loop(app: AppHandle, engine: Arc<SyncEngine>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let _ = engine.sync_all(Some(app.clone())).await;
            tokio::time::sleep(Duration::from_secs(120)).await;
        }
    });
}
