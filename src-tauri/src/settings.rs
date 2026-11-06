//! Persistent widget settings: `%APPDATA%\scrooge\settings.json`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// GitHub logins whose Copilot quota is hidden in the panel.
    pub hidden_copilot_accounts: Vec<String>,
    /// Providers that are not scanned at all (`claude`, `codex`, `opencode`,
    /// `copilot`, `gemini`, `grok`). No files are read and no quota calls are
    /// made for them.
    pub disabled_providers: Vec<String>,
    /// Keep the compact panel above other windows.
    pub always_on_top: bool,
    /// Last compact-panel position/size in physical pixels.
    pub compact_bounds: Option<Bounds>,
    /// Mode at launch: `compact` or `full`.
    pub start_mode: String,
    /// When started by Windows at login, stay in the tray.
    pub start_hidden: bool,
    /// Session collector interval, seconds (1-10).
    pub tick_secs: u64,
    /// Copilot / Claude quota refresh interval, minutes (1-60).
    pub quota_minutes: u64,
    /// Global show/hide shortcut.
    pub shortcut_enabled: bool,
    pub shortcut: String,
    /// Check GitHub Releases for updates at startup and every few hours.
    pub auto_update_check: bool,
    /// Frontend-only preferences (colors, density, thresholds, ...), opaque here.
    pub ui: Value,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hidden_copilot_accounts: Vec::new(),
            disabled_providers: Vec::new(),
            always_on_top: true,
            compact_bounds: None,
            start_mode: "compact".into(),
            start_hidden: false,
            tick_secs: 2,
            quota_minutes: 5,
            shortcut_enabled: true,
            shortcut: "Ctrl+Alt+A".into(),
            auto_update_check: true,
            ui: Value::Object(Default::default()),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("scrooge").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Settings = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        s.clamp();
        s
    }

    pub fn provider_enabled(&self, name: &str) -> bool {
        !self
            .disabled_providers
            .iter()
            .any(|p| p.eq_ignore_ascii_case(name))
    }

    pub fn clamp(&mut self) {
        self.tick_secs = self.tick_secs.clamp(1, 10);
        self.quota_minutes = self.quota_minutes.clamp(1, 60);
        if self.start_mode != "full" {
            self.start_mode = "compact".into();
        }
        if !self.ui.is_object() {
            self.ui = Value::Object(Default::default());
        }
    }

    /// Shallow-merge a JSON patch (`{"tick_secs": 5, "ui": {...}}`). `ui` is
    /// merged key by key so the frontend can update one preference at a time.
    pub fn merge(&mut self, patch: &Value) -> Result<(), String> {
        let mut cur = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        if let (Some(obj), Some(p)) = (cur.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                if k == "ui" {
                    if let (Some(ui), Some(pv)) = (
                        obj.get_mut("ui").and_then(|u| u.as_object_mut()),
                        v.as_object(),
                    ) {
                        for (uk, uv) in pv {
                            ui.insert(uk.clone(), uv.clone());
                        }
                        continue;
                    }
                }
                obj.insert(k.clone(), v.clone());
            }
        }
        let mut next: Settings = serde_json::from_value(cur).map_err(|e| e.to_string())?;
        next.clamp();
        *self = next;
        Ok(())
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_patches_fields_and_ui_keys() {
        let mut s = Settings::default();
        s.merge(&serde_json::json!({"ui": {"density": "cozy"}}))
            .unwrap();
        s.merge(&serde_json::json!({"tick_secs": 99, "ui": {"warn": 70}}))
            .unwrap();
        assert_eq!(s.tick_secs, 10, "clamped");
        assert_eq!(s.ui["density"], "cozy");
        assert_eq!(s.ui["warn"], 70);
    }

    #[test]
    fn old_settings_files_still_load() {
        let s: Settings =
            serde_json::from_str(r#"{"copilot_login":"x","always_on_top":false}"#).unwrap();
        assert!(!s.always_on_top);
        assert_eq!(s.tick_secs, 2);
    }
}
