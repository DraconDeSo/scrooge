//! In-app updates from signed GitHub Releases.
//!
//! The updater reads `latest.json` from the endpoint configured in
//! `tauri.conf.json` (`plugins.updater`), verifies the package signature with
//! the embedded public key, installs it and restarts the app.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Re-check interval while the app is running.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// Delay before the first automatic check, so startup stays quick.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<Update>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

pub async fn check(app: &AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| e.to_string())?;
    let info = match &found {
        Some(u) => UpdateInfo {
            available: true,
            current,
            version: Some(u.version.clone()),
            notes: u.body.clone(),
            date: u.date.map(|d| d.to_string()),
        },
        None => UpdateInfo {
            available: false,
            current,
            version: None,
            notes: None,
            date: None,
        },
    };
    *app.state::<UpdateState>().pending.lock().unwrap() = found;
    if info.available {
        let _ = app.emit("update-available", &info);
    }
    Ok(info)
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    let pending = app.state::<UpdateState>().pending.lock().unwrap().take();
    let update = match pending {
        Some(u) => u,
        None => app
            .updater()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Scrooge is up to date".to_string())?,
    };
    let mut downloaded: u64 = 0;
    let progress_app = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

/// Background loop: first check shortly after startup, then periodically,
/// only while automatic checks are enabled in settings.
pub fn spawn_auto_check(app: AppHandle, enabled: impl Fn(&AppHandle) -> bool + Send + 'static) {
    tauri::async_runtime::spawn(async move {
        tokio_sleep(FIRST_CHECK_DELAY).await;
        loop {
            if enabled(&app) {
                let _ = check(&app).await;
            }
            tokio_sleep(CHECK_INTERVAL).await;
        }
    });
}

async fn tokio_sleep(d: Duration) {
    // tauri's async runtime is tokio; avoid a direct tokio dependency.
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}
