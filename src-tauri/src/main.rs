// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod claude_local;
mod claude_usage;
mod quota;
mod settings;
mod storage;
mod updates;
mod usage;

use claude_local::{ClaudeLocal, ClaudeLocalScanner};
use claude_usage::ClaudeUsage;
use quota::{CopilotQuota, GhAccount};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use settings::{Bounds, Settings};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, State, WebviewWindow, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

const CLAUDE_LOCAL_INTERVAL: Duration = Duration::from_secs(60);
const COMPACT_W: u32 = 400;
const COMPACT_H: u32 = 600;
const AUTOSTART_ARG: &str = "--autostart";

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Compact,
    Full,
}

struct Shared {
    snapshot: Mutex<String>,
    quota: Mutex<Vec<CopilotQuota>>,
    claude: Mutex<ClaudeUsage>,
    claude_local: Mutex<ClaudeLocal>,
    settings: Mutex<Settings>,
    mode: Mutex<Mode>,
    refresh_tx: Mutex<Sender<()>>,
    quota_tx: Mutex<Sender<()>>,
    local_tx: Mutex<Sender<()>>,
    storage: Mutex<storage::StorageReport>,
    storage_tx: Mutex<Sender<()>>,
    usage: Mutex<usage::History>,
    on_top_item: Mutex<Option<CheckMenuItem<Wry>>>,
    shortcut_error: Mutex<Option<String>>,
}

type AppState = Arc<Shared>;

fn wake_all(state: &AppState) {
    let _ = state.refresh_tx.lock().unwrap().send(());
    let _ = state.quota_tx.lock().unwrap().send(());
    let _ = state.local_tx.lock().unwrap().send(());
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_snapshot(state: State<AppState>) -> String {
    state.snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn get_quota(state: State<AppState>) -> Vec<CopilotQuota> {
    state.quota.lock().unwrap().clone()
}

#[tauri::command]
fn get_claude_usage(state: State<AppState>) -> ClaudeUsage {
    state.claude.lock().unwrap().clone()
}

#[tauri::command]
fn get_claude_local(state: State<AppState>) -> ClaudeLocal {
    state.claude_local.lock().unwrap().clone()
}

#[tauri::command]
fn get_storage(state: State<AppState>) -> storage::StorageReport {
    state.storage.lock().unwrap().clone()
}

#[tauri::command]
async fn get_usage_report(
    state: State<'_, AppState>,
    start: u64,
    end: u64,
) -> Result<usage::Report, String> {
    if start >= end {
        return Err("Choose a valid date range.".into());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings = state.settings.lock().unwrap().clone();
        state.usage.lock().unwrap().report(&settings, start, end)
    })
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
fn open_usage_imports() -> Result<(), String> {
    let dir = usage::imports_dir().ok_or("Settings folder unavailable.")?;
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    reveal(dir.to_string_lossy().into_owned())
}

#[tauri::command]
fn refresh_storage(state: State<AppState>) {
    let _ = state.storage_tx.lock().unwrap().send(());
}

#[tauri::command]
fn refresh(state: State<AppState>) {
    wake_all(&state);
}

#[derive(Serialize)]
struct AccountInfo {
    accounts: Vec<GhAccount>,
    hidden: Vec<String>,
    copilot_cli: Option<String>,
}

#[tauri::command]
async fn list_accounts(state: State<'_, AppState>) -> Result<AccountInfo, String> {
    let hidden = state
        .settings
        .lock()
        .unwrap()
        .hidden_copilot_accounts
        .clone();
    let (mut accounts, copilot_cli) =
        tauri::async_runtime::spawn_blocking(|| (quota::gh_accounts(), quota::copilot_cli_login()))
            .await
            .map_err(|e| e.to_string())?;
    if let Some(cli) = &copilot_cli {
        if !accounts.iter().any(|a| a.login.eq_ignore_ascii_case(cli)) {
            accounts.insert(
                0,
                GhAccount {
                    login: cli.clone(),
                    active: false,
                },
            );
        }
    }
    Ok(AccountInfo {
        accounts,
        hidden,
        copilot_cli,
    })
}

#[tauri::command]
fn set_copilot_account_visible(state: State<AppState>, login: String, visible: bool) {
    {
        let mut s = state.settings.lock().unwrap();
        s.hidden_copilot_accounts
            .retain(|h| !h.eq_ignore_ascii_case(&login));
        if !visible {
            s.hidden_copilot_accounts.push(login);
        }
        s.save();
    }
    let _ = state.quota_tx.lock().unwrap().send(());
}

#[derive(Serialize)]
struct Paths {
    settings_file: String,
    copilot_home: String,
    claude_roots: Vec<String>,
    abtop_config: String,
}

#[derive(Serialize)]
struct SettingsView {
    settings: Settings,
    autostart: bool,
    shortcut_error: Option<String>,
    version: String,
    paths: Paths,
}

fn settings_view(app: &AppHandle) -> SettingsView {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let shortcut_error = state.shortcut_error.lock().unwrap().clone();
    let home = dirs::home_dir().unwrap_or_default();
    SettingsView {
        settings,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        shortcut_error,
        version: env!("CARGO_PKG_VERSION").to_string(),
        paths: Paths {
            settings_file: settings::path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            copilot_home: std::env::var("COPILOT_HOME")
                .unwrap_or_else(|_| home.join(".copilot").to_string_lossy().into_owned()),
            claude_roots: claude_local::claude_roots()
                .iter()
                .map(|r| r.to_string_lossy().into_owned())
                .collect(),
            abtop_config: home
                .join(".config")
                .join("abtop")
                .join("config.toml")
                .to_string_lossy()
                .into_owned(),
        },
    }
}

#[tauri::command]
fn get_settings(app: AppHandle) -> SettingsView {
    settings_view(&app)
}

/// Merge a settings patch, persist it and apply side effects.
#[tauri::command]
fn update_settings(app: AppHandle, patch: Value) -> Result<SettingsView, String> {
    let state = app.state::<AppState>();
    let (before, after) = {
        let mut s = state.settings.lock().unwrap();
        let before = s.clone();
        s.merge(&patch)?;
        s.save();
        (before, s.clone())
    };
    if before.always_on_top != after.always_on_top {
        apply_always_on_top(&app, after.always_on_top);
    }
    if before.shortcut != after.shortcut || before.shortcut_enabled != after.shortcut_enabled {
        register_shortcut(&app);
    }
    if before.tick_secs != after.tick_secs
        || before.quota_minutes != after.quota_minutes
        || before.disabled_providers != after.disabled_providers
    {
        wake_all(&state);
    }
    if before.disabled_providers != after.disabled_providers {
        let _ = state.storage_tx.lock().unwrap().send(());
    }
    let _ = app.emit("settings", &after);
    Ok(settings_view(&app))
}

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> Result<bool, String> {
    let al = app.autolaunch();
    if on { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    al.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_mode(state: State<AppState>) -> Mode {
    *state.mode.lock().unwrap()
}

#[tauri::command]
fn set_mode(app: AppHandle, mode: Mode) {
    apply_mode(&app, mode);
}

#[tauri::command]
fn toggle_mode(app: AppHandle) {
    let cur = *app.state::<AppState>().mode.lock().unwrap();
    apply_mode(
        &app,
        if cur == Mode::Compact {
            Mode::Full
        } else {
            Mode::Compact
        },
    );
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum DragPhase {
    Move,
    Drop,
    Cancel,
}

#[tauri::command]
fn drag_compact_panel(w: WebviewWindow, x: f64, y: f64, phase: DragPhase) -> tauri::Result<()> {
    let app = w.app_handle();
    if *app.state::<AppState>().mode.lock().unwrap() != Mode::Compact || !w.is_visible()? {
        return Ok(());
    }

    let cursor = w.cursor_position()?;
    let scale = w.scale_factor()?;
    let outer = w.outer_position()?;
    let inner = w.inner_position()?;
    let x = x + f64::from(inner.x - outer.x) / scale;
    let y = y + f64::from(inner.y - outer.y) / scale;
    let position = |scale: f64| {
        PhysicalPosition::new(
            (cursor.x - x * scale).round() as i32,
            (cursor.y - y * scale).round() as i32,
        )
    };
    if phase == DragPhase::Move {
        return w.set_position(position(scale));
    }

    let monitor = if phase == DragPhase::Drop {
        w.monitor_from_point(cursor.x, cursor.y)?
    } else {
        w.current_monitor()?
    };
    let Some(monitor) = monitor else {
        save_compact_bounds(app);
        return Ok(());
    };
    let area = monitor.work_area();
    let target_scale = monitor.scale_factor();
    let margin = (16.0 * target_scale).round() as i32;
    let size = w.outer_size()?;
    let pos = if phase == DragPhase::Drop {
        position(target_scale)
    } else {
        outer
    };
    let bounds = snap_bounds(
        &Bounds {
            x: pos.x,
            y: pos.y,
            w: (size.width as f64 * target_scale / scale).round() as u32,
            h: (size.height as f64 * target_scale / scale).round() as u32,
        },
        area.position,
        area.size,
        margin,
    );
    let full = phase == DragPhase::Drop && cursor.y <= f64::from(area.position.y + margin);
    if !full {
        set_bounds(&w, &bounds)?;
    }
    persist_compact_bounds(app, bounds);
    if full {
        apply_mode_on(app, Mode::Full, Some(monitor));
    }
    Ok(())
}

#[tauri::command]
fn get_always_on_top(state: State<AppState>) -> bool {
    state.settings.lock().unwrap().always_on_top
}

#[tauri::command]
fn set_always_on_top(app: AppHandle, on: bool) {
    {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.always_on_top = on;
        s.save();
    }
    apply_always_on_top(&app, on);
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    save_compact_bounds(&app);
    if let Some(w) = main_window(&app) {
        let _ = w.hide();
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    save_compact_bounds(&app);
    app.exit(0);
}

/// Open a directory, or reveal a file, in the system file manager.
#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.is_absolute() {
        return Err("not an absolute path".into());
    }
    let target = if p.exists() {
        p.to_path_buf()
    } else if let Some(parent) = p.parent().filter(|d| d.is_dir()) {
        parent.to_path_buf()
    } else {
        return Err("path does not exist".into());
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer");
        if target.is_file() {
            c.arg(format!("/select,{}", target.display()));
        } else {
            c.arg(&target);
        }
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        if target.is_file() {
            c.arg("-R");
        }
        c.arg(&target);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if target.is_file() {
            target.parent().unwrap_or(&target)
        } else {
            &target
        });
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Result<updates::UpdateInfo, String> {
    updates::check(&app).await
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    updates::install(&app).await
}

// ------------------------------------------------------------ window logic

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn save_compact_bounds(app: &AppHandle) {
    let state = app.state::<AppState>();
    if *state.mode.lock().unwrap() != Mode::Compact {
        return;
    }
    let Some(w) = main_window(app) else { return };
    if !w.is_visible().unwrap_or(false)
        || w.is_minimized().unwrap_or(false)
        || w.is_maximized().unwrap_or(false)
    {
        return;
    }
    if let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) {
        persist_compact_bounds(
            app,
            Bounds {
                x: pos.x,
                y: pos.y,
                w: size.width,
                h: size.height,
            },
        );
    }
}

fn persist_compact_bounds(app: &AppHandle, bounds: Bounds) {
    let state = app.state::<AppState>();
    let mut s = state.settings.lock().unwrap();
    if s.compact_bounds.as_ref() != Some(&bounds) {
        s.compact_bounds = Some(bounds);
        s.save();
    }
}

/// Default compact placement: bottom-right corner of the primary work area.
fn default_compact_bounds(w: &WebviewWindow) -> Bounds {
    let monitor = w
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| w.current_monitor().ok().flatten());
    let scale = monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
    let width = (COMPACT_W as f64 * scale) as u32;
    let height = (COMPACT_H as f64 * scale) as u32;
    let margin = (16.0 * scale) as i32;
    match monitor {
        Some(m) => {
            let area = m.work_area();
            Bounds {
                x: area.position.x + area.size.width as i32 - width as i32 - margin,
                y: area.position.y + area.size.height as i32 - height as i32 - margin,
                w: width,
                h: height,
            }
        }
        None => Bounds {
            x: 100,
            y: 100,
            w: width,
            h: height,
        },
    }
}

/// Prefer the monitor containing the saved center, even near a seam between
/// displays of different sizes. Fall back to the nearest edge if disconnected.
fn monitor_for_bounds(w: &WebviewWindow, b: &Bounds) -> Option<Monitor> {
    w.available_monitors()
        .ok()?
        .into_iter()
        .min_by_key(|m| distance_to_area(b, *m.position(), *m.size()))
}

fn distance_to_area(b: &Bounds, pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> u64 {
    let dx = distance_to_span(i64::from(b.x) + i64::from(b.w) / 2, pos.x, size.width);
    let dy = distance_to_span(i64::from(b.y) + i64::from(b.h) / 2, pos.y, size.height);
    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
}

fn distance_to_span(center: i64, start: i32, length: u32) -> u64 {
    let start = i64::from(start);
    let end = start + i64::from(length.saturating_sub(1));
    center.abs_diff(center.clamp(start, end))
}

fn fit_to_screen(w: &WebviewWindow, b: &Bounds) -> Option<Bounds> {
    let monitor = monitor_for_bounds(w, b)?;
    let area = monitor.work_area();
    Some(clamp_bounds(b, area.position, area.size))
}

fn clamp_bounds(b: &Bounds, pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> Bounds {
    let w = b.w.min(size.width);
    let h = b.h.min(size.height);
    let max_x = pos.x + size.width as i32 - w as i32;
    let max_y = pos.y + size.height as i32 - h as i32;
    Bounds {
        x: b.x.clamp(pos.x, max_x),
        y: b.y.clamp(pos.y, max_y),
        w,
        h,
    }
}

fn snap_bounds(
    b: &Bounds,
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    margin: i32,
) -> Bounds {
    let mut b = clamp_bounds(b, pos, size);
    b.x = snap_axis(b.x, pos.x, pos.x + (size.width - b.w) as i32, margin);
    b.y = snap_axis(b.y, pos.y, pos.y + (size.height - b.h) as i32, margin);
    b
}

fn snap_axis(value: i32, start: i32, end: i32, margin: i32) -> i32 {
    let leading = value - start;
    let trailing = end - value;
    if leading <= margin && leading <= trailing {
        start
    } else if trailing <= margin {
        end
    } else {
        value
    }
}

#[cfg(test)]
mod window_placement_tests {
    use super::*;

    #[test]
    fn snaps_to_work_area_edges_without_resizing() {
        let bounds = Bounds {
            x: -1272,
            y: 192,
            w: 400,
            h: 600,
        };
        let snapped = snap_bounds(
            &bounds,
            PhysicalPosition::new(-1280, 0),
            PhysicalSize::new(1280, 800),
            16,
        );
        assert_eq!(
            snapped,
            Bounds {
                x: -1280,
                y: 200,
                ..bounds
            }
        );
    }

    #[test]
    fn prefers_the_monitor_containing_the_saved_center() {
        let bounds = Bounds {
            x: 0,
            y: 30,
            w: 400,
            h: 600,
        };
        let wide_monitor = distance_to_area(
            &bounds,
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(3840, 2160),
        );
        let adjacent_monitor = distance_to_area(
            &bounds,
            PhysicalPosition::new(-1280, 0),
            PhysicalSize::new(1280, 1024),
        );
        assert_eq!(wide_monitor, 0);
        assert!(adjacent_monitor > wide_monitor);
    }
}

fn set_bounds(w: &WebviewWindow, b: &Bounds) -> tauri::Result<()> {
    // Moving between monitors may change the window size due to DPI scaling.
    w.set_position(PhysicalPosition::new(b.x, b.y))?;
    let outer = w.outer_size()?;
    if outer == PhysicalSize::new(b.w, b.h) {
        return Ok(());
    }
    // The saved rectangle includes the resize frame, but set_size takes the
    // client size. Subtract the insets to avoid growing the panel on each drop.
    let inner = w.inner_size()?;
    w.set_size(PhysicalSize::new(
        b.w.saturating_sub(outer.width.saturating_sub(inner.width)),
        b.h.saturating_sub(outer.height.saturating_sub(inner.height)),
    ))
}

fn compact_placement(w: &WebviewWindow, saved: Option<Bounds>) -> Bounds {
    let b = saved.unwrap_or_else(|| default_compact_bounds(w));
    let Some(monitor) = monitor_for_bounds(w, &b) else {
        return b;
    };
    let area = monitor.work_area();
    let mut b = clamp_bounds(&b, area.position, area.size);
    // Older releases could save a native maximized window as compact.
    if b.w == area.size.width && b.h == area.size.height {
        b.w = (COMPACT_W as f64 * monitor.scale_factor()).round() as u32;
        b.h = (COMPACT_H as f64 * monitor.scale_factor()).round() as u32;
        b = clamp_bounds(&b, area.position, area.size);
    }
    b
}

fn apply_mode(app: &AppHandle, mode: Mode) {
    apply_mode_on(app, mode, None);
}

fn apply_mode_on(app: &AppHandle, mode: Mode, monitor: Option<Monitor>) {
    let Some(w) = main_window(app) else { return };
    let state = app.state::<AppState>();
    if mode == Mode::Full && monitor.is_none() {
        save_compact_bounds(app);
    }
    *state.mode.lock().unwrap() = mode;
    let (on_top, saved) = {
        let s = state.settings.lock().unwrap();
        (s.always_on_top, s.compact_bounds.clone())
    };
    // Native fullscreen on an undecorated WebView2 window mis-sizes the
    // webview on scaled (HiDPI) monitors, pushing content off-screen. Use a
    // borderless window that covers the current monitor instead.
    let _ = w.set_fullscreen(false);
    match mode {
        Mode::Full => {
            let monitor = monitor
                .or_else(|| {
                    if w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false) {
                        w.current_monitor().ok().flatten()
                    } else {
                        saved.as_ref().and_then(|b| monitor_for_bounds(&w, b))
                    }
                })
                .or_else(|| w.primary_monitor().ok().flatten());
            let _ = w.set_always_on_top(false);
            let _ = w.set_skip_taskbar(false);
            if let Some(m) = monitor {
                let _ = w.set_position(*m.position());
                let _ = w.set_size(*m.size());
            } else {
                let _ = w.maximize();
            }
        }
        Mode::Compact => {
            let _ = w.unmaximize();
            let _ = w.set_skip_taskbar(true);
            let _ = w.set_always_on_top(on_top);
            let _ = set_bounds(&w, &compact_placement(&w, saved));
            if let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) {
                let real = Bounds {
                    x: pos.x,
                    y: pos.y,
                    w: size.width,
                    h: size.height,
                };
                if let Some(fixed) = fit_to_screen(&w, &real).filter(|f| *f != real) {
                    let _ = set_bounds(&w, &fixed);
                }
            }
        }
    }
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
    let _ = app.emit("mode", mode);
}

fn apply_always_on_top(app: &AppHandle, on: bool) {
    let state = app.state::<AppState>();
    if *state.mode.lock().unwrap() == Mode::Compact {
        if let Some(w) = main_window(app) {
            let _ = w.set_always_on_top(on);
        }
    }
    if let Some(item) = state.on_top_item.lock().unwrap().as_ref() {
        let _ = item.set_checked(on);
    }
    let _ = app.emit("always-on-top", on);
}

fn toggle_visibility(app: &AppHandle) {
    let Some(w) = main_window(app) else { return };
    if w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false) {
        save_compact_bounds(app);
        let _ = w.hide();
    } else {
        let mode = *app.state::<AppState>().mode.lock().unwrap();
        apply_mode(app, mode);
    }
}

fn register_shortcut(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (enabled, combo) = {
        let s = state.settings.lock().unwrap();
        (s.shortcut_enabled, s.shortcut.clone())
    };
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let err = if enabled && !combo.trim().is_empty() {
        gs.register(combo.as_str())
            .err()
            .map(|e| format!("{combo}: {e}"))
    } else {
        None
    };
    *state.shortcut_error.lock().unwrap() = err;
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn clamps_window_inside_work_area() {
        let area_pos = PhysicalPosition::new(0, 0);
        let area = PhysicalSize::new(3840, 2100);
        // the reported case: 572 px wide at x=3320 sticks out by 52 px
        let b = clamp_bounds(
            &Bounds {
                x: 3320,
                y: 810,
                w: 572,
                h: 1310,
            },
            area_pos,
            area,
        );
        assert_eq!((b.x, b.y, b.w, b.h), (3268, 790, 572, 1310));
        // larger than the screen: shrink and pin to the origin
        let b = clamp_bounds(
            &Bounds {
                x: -50,
                y: -50,
                w: 5000,
                h: 3000,
            },
            area_pos,
            area,
        );
        assert_eq!((b.x, b.y, b.w, b.h), (0, 0, 3840, 2100));
    }
}

// --------------------------------------------------------- background work

fn wait(rx: &mpsc::Receiver<()>, d: Duration) -> bool {
    match rx.recv_timeout(d) {
        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {
            while rx.try_recv().is_ok() {}
            true
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => false,
    }
}

fn spawn_monitor(app: AppHandle, state: AppState, rx: mpsc::Receiver<()>) {
    std::thread::Builder::new()
        .name("abtop-collector".into())
        .spawn(move || {
            // abtop::App is !Send: build and keep it on this thread. It is
            // rebuilt whenever the set of disabled providers changes, so a
            // disabled provider is not scanned at all.
            let cfg = abtop::config::load_config();
            let build = |disabled: &[String]| {
                let mut hidden = cfg.hidden_agents.clone();
                hidden.extend(disabled.iter().cloned());
                abtop::app::App::new_with_config_and_claude_dirs(
                    abtop::theme::Theme::default(),
                    &hidden,
                    cfg.panels,
                    &cfg.claude_config_dirs,
                )
            };
            let mut disabled = state.settings.lock().unwrap().disabled_providers.clone();
            let mut monitor = build(&disabled);
            loop {
                let (tick, now_disabled) = {
                    let s = state.settings.lock().unwrap();
                    (
                        Duration::from_secs(s.tick_secs),
                        s.disabled_providers.clone(),
                    )
                };
                if now_disabled != disabled {
                    disabled = now_disabled;
                    monitor = build(&disabled);
                }
                monitor.tick_no_summaries();
                let snap = monitor.to_snapshot(tick.as_millis() as u64);
                if let Ok(json) = serde_json::to_string(&snap) {
                    *state.snapshot.lock().unwrap() = json.clone();
                    let _ = app.emit("snapshot", json);
                }
                if !wait(&rx, tick) {
                    break;
                }
            }
        })
        .expect("spawn collector thread");
}

fn spawn_quota(app: AppHandle, state: AppState, rx: mpsc::Receiver<()>) {
    std::thread::Builder::new()
        .name("quota".into())
        .spawn(move || loop {
            let (hidden, minutes, copilot_on, claude_on) = {
                let s = state.settings.lock().unwrap();
                (
                    s.hidden_copilot_accounts.clone(),
                    s.quota_minutes,
                    s.provider_enabled("copilot"),
                    s.provider_enabled("claude"),
                )
            };
            // Disabled providers make no network calls.
            let q = if copilot_on {
                quota::fetch_all(&hidden)
            } else {
                Vec::new()
            };
            *state.quota.lock().unwrap() = q.clone();
            let _ = app.emit("quota", q);
            let c = if claude_on {
                claude_usage::fetch()
            } else {
                ClaudeUsage::default()
            };
            *state.claude.lock().unwrap() = c.clone();
            let _ = app.emit("claude-usage", c);
            if !wait(&rx, Duration::from_secs(minutes * 60)) {
                break;
            }
        })
        .expect("spawn quota thread");
}

fn spawn_claude_local(app: AppHandle, state: AppState, rx: mpsc::Receiver<()>) {
    std::thread::Builder::new()
        .name("claude-local".into())
        .spawn(move || {
            let mut scanner = ClaudeLocalScanner::new();
            loop {
                let enabled = state.settings.lock().unwrap().provider_enabled("claude");
                let local = if enabled {
                    scanner.scan()
                } else {
                    ClaudeLocal::default()
                };
                *state.claude_local.lock().unwrap() = local.clone();
                let _ = app.emit("claude-local", local);
                if !wait(&rx, CLAUDE_LOCAL_INTERVAL) {
                    break;
                }
            }
        })
        .expect("spawn claude-local thread");
}

const STORAGE_INTERVAL: Duration = Duration::from_secs(15 * 60);

fn spawn_storage(app: AppHandle, state: AppState, rx: mpsc::Receiver<()>) {
    std::thread::Builder::new()
        .name("storage".into())
        .spawn(move || loop {
            let settings = state.settings.lock().unwrap().clone();
            let report = storage::scan(|p| settings.provider_enabled(p));
            *state.storage.lock().unwrap() = report.clone();
            let _ = app.emit("storage", report);
            if !wait(&rx, STORAGE_INTERVAL) {
                break;
            }
        })
        .expect("spawn storage thread");
}

// -------------------------------------------------------------------- tray

fn build_tray(app: &AppHandle, on_top: bool) -> tauri::Result<CheckMenuItem<Wry>> {
    let show = MenuItem::with_id(app, "toggle", "Show / hide", true, None::<&str>)?;
    let compact = MenuItem::with_id(app, "compact", "Compact panel", true, None::<&str>)?;
    let full = MenuItem::with_id(app, "full", "Full screen", true, None::<&str>)?;
    let top = CheckMenuItem::with_id(app, "on_top", "Always on top", true, on_top, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &show, &sep1, &compact, &full, &top, &refresh, &settings, &sep2, &quit,
        ],
    )?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("Scrooge — AI agent monitor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => toggle_visibility(app),
            "compact" => apply_mode(app, Mode::Compact),
            "full" => apply_mode(app, Mode::Full),
            "on_top" => {
                let state = app.state::<AppState>();
                let on = {
                    let mut s = state.settings.lock().unwrap();
                    s.always_on_top = !s.always_on_top;
                    s.save();
                    s.always_on_top
                };
                apply_always_on_top(app, on);
            }
            "refresh" => wake_all(&app.state::<AppState>()),
            "settings" => {
                let mode = *app.state::<AppState>().mode.lock().unwrap();
                apply_mode(app, mode);
                let _ = app.emit("open-settings", ());
            }
            "quit" => {
                save_compact_bounds(app);
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
                toggle_visibility(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(top)
}

// -------------------------------------------------------------------- main

fn main() {
    let (refresh_tx, refresh_rx) = mpsc::channel();
    let (quota_tx, quota_rx) = mpsc::channel();
    let (local_tx, local_rx) = mpsc::channel();
    let (storage_tx, storage_rx) = mpsc::channel();
    let settings = Settings::load();
    let launched_by_autostart = std::env::args().any(|a| a == AUTOSTART_ARG);
    let state: AppState = Arc::new(Shared {
        snapshot: Mutex::new(String::from("null")),
        quota: Mutex::new(Vec::new()),
        claude: Mutex::new(ClaudeUsage::default()),
        claude_local: Mutex::new(ClaudeLocal::default()),
        settings: Mutex::new(settings),
        mode: Mutex::new(Mode::Compact),
        refresh_tx: Mutex::new(refresh_tx),
        quota_tx: Mutex::new(quota_tx),
        local_tx: Mutex::new(local_tx),
        storage: Mutex::new(storage::StorageReport::default()),
        storage_tx: Mutex::new(storage_tx),
        usage: Mutex::new(usage::History::default()),
        on_top_item: Mutex::new(None),
        shortcut_error: Mutex::new(None),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let mode = *app.state::<AppState>().mode.lock().unwrap();
            apply_mode(app, mode);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::UpdateState::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_visibility(app);
                    }
                })
                .build(),
        )
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_quota,
            get_claude_usage,
            get_claude_local,
            get_storage,
            get_usage_report,
            open_usage_imports,
            refresh_storage,
            refresh,
            list_accounts,
            set_copilot_account_visible,
            get_settings,
            update_settings,
            set_autostart,
            get_mode,
            set_mode,
            toggle_mode,
            drag_compact_panel,
            get_always_on_top,
            set_always_on_top,
            hide_window,
            quit_app,
            reveal,
            check_update,
            install_update,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            #[cfg(target_os = "windows")]
            if let Some(w) = main_window(&handle) {
                w.set_maximizable(false)?;
            }
            let (on_top, start_mode, start_hidden) = {
                let s = state.settings.lock().unwrap();
                (s.always_on_top, s.start_mode.clone(), s.start_hidden)
            };
            let item = build_tray(&handle, on_top)?;
            *state.on_top_item.lock().unwrap() = Some(item);
            register_shortcut(&handle);
            spawn_monitor(handle.clone(), state.clone(), refresh_rx);
            spawn_quota(handle.clone(), state.clone(), quota_rx);
            spawn_claude_local(handle.clone(), state.clone(), local_rx);
            spawn_storage(handle.clone(), state.clone(), storage_rx);
            updates::spawn_auto_check(handle.clone(), |app| {
                app.state::<AppState>()
                    .settings
                    .lock()
                    .unwrap()
                    .auto_update_check
            });
            let mode = if start_mode == "full" {
                Mode::Full
            } else {
                Mode::Compact
            };
            if launched_by_autostart && start_hidden {
                *state.mode.lock().unwrap() = mode;
            } else {
                apply_mode(&handle, mode);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the widget only hides it; quit from the tray menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                save_compact_bounds(window.app_handle());
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Scrooge");
}
