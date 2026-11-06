//! In-app updates from signed GitHub Releases.
//!
//! The updater reads `latest.json` from the endpoint configured in
//! `tauri.conf.json` (`plugins.updater`), verifies the package signature with
//! the embedded public key, installs it and restarts the app.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};

const RELEASES_API: &str = "https://api.github.com/repos/DraconDeSo/scrooge/releases";

/// Re-check interval while the app is running.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// Delay before the first automatic check, so startup stays quick.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<Update>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum UpdateStatus {
    Available,
    UpToDate,
    NoPublishedRelease,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    status: UpdateStatus,
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
    let (found, status) = match updater.check().await {
        Ok(Some(update)) => (Some(update), UpdateStatus::Available),
        Ok(None) => (None, UpdateStatus::UpToDate),
        Err(UpdaterError::ReleaseNotFound) => {
            tauri::async_runtime::spawn_blocking(check_release_status)
                .await
                .map_err(|e| e.to_string())??;
            (None, UpdateStatus::NoPublishedRelease)
        }
        Err(error) => return Err(error.to_string()),
    };
    let info = UpdateInfo {
        status,
        current,
        version: found.as_ref().map(|u| u.version.clone()),
        notes: found.as_ref().and_then(|u| u.body.clone()),
        date: found.as_ref().and_then(|u| u.date.map(|d| d.to_string())),
    };
    let available = found.is_some();
    *app.state::<UpdateState>().pending.lock().unwrap() = found;
    if available {
        let _ = app.emit("update-available", &info);
    }
    Ok(info)
}

fn check_release_status() -> Result<(), String> {
    let agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .new_agent();
    confirm_no_public_release(|suffix| {
        let response = agent
            .get(format!("{RELEASES_API}{suffix}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "scrooge-widget")
            .call();
        match response {
            Ok(response) => Ok(response.status().as_u16()),
            Err(ureq::Error::StatusCode(code)) => Ok(code),
            Err(_) => Err("Could not reach GitHub to check for a published release.".into()),
        }
    })
}

fn confirm_no_public_release(status: impl Fn(&str) -> Result<u16, String>) -> Result<(), String> {
    let code = match status("/latest")? {
        200 => {
            return Err(
                "GitHub has a published release, but its update feed is unavailable.".into(),
            )
        }
        // GitHub also returns 404 for inaccessible repositories. Verify that
        // the releases list is public before reporting that no release exists.
        404 => match status("?per_page=1")? {
            200 => return Ok(()),
            code => code,
        },
        code => code,
    };
    Err(format!(
        "GitHub could not confirm release availability (HTTP {code})."
    ))
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

#[cfg(test)]
mod tests {
    use super::confirm_no_public_release;

    #[test]
    fn missing_latest_with_public_releases_list_means_no_release() {
        assert!(
            confirm_no_public_release(|path| Ok(if path == "/latest" { 404 } else { 200 })).is_ok()
        );
    }

    #[test]
    fn missing_feed_for_a_published_release_is_an_error() {
        let error = confirm_no_public_release(|path| {
            assert_eq!(path, "/latest");
            Ok(200)
        })
        .unwrap_err();
        assert!(error.contains("update feed is unavailable"));
    }

    #[test]
    fn github_errors_do_not_hide_as_no_release() {
        for code in [404, 429, 503] {
            let error = confirm_no_public_release(|_| Ok(code)).unwrap_err();
            assert!(error.contains(&format!("HTTP {code}")));
        }
    }
}
