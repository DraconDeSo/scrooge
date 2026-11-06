//! Claude subscription usage (5-hour / 7-day windows).
//!
//! Source: `GET https://api.anthropic.com/api/oauth/usage` — the endpoint the
//! Claude Code `/usage` command uses. Authenticated with the Claude Code CLI
//! OAuth access token from `<config-root>/.credentials.json`
//! (`claudeAiOauth.accessToken`). The token is read on each fetch, never
//! stored, and never refreshed by Scrooge: refreshing would rotate the
//! refresh token behind Claude Code's back. When it has expired, the panel
//! asks the user to run `claude` once so Claude Code refreshes it itself.
//!
//! Upstream abtop's status-line file (`abtop-rate-limits.json`) still takes
//! precedence in the UI when present.

use abtop::model::RateLimitInfo;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct ExtraWindow {
    /// e.g. `opus 7d`, `sonnet 7d`
    pub label: String,
    pub pct: f64,
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ClaudeUsage {
    /// Same shape abtop uses for its quota panel (`source = "claude"`).
    pub limits: Option<RateLimitInfo>,
    /// Model-specific weekly windows, when the account has them.
    pub extra: Vec<ExtraWindow>,
    pub subscription: Option<String>,
    pub error: Option<String>,
    pub fetched_at_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Claude Code config root: `CLAUDE_CONFIG_DIR`, else `~/.claude`.
fn config_root() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
}

struct OAuth {
    token: String,
    expires_at_ms: u64,
    subscription: Option<String>,
}

fn read_oauth() -> Result<OAuth, String> {
    let path = config_root().join(".credentials.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "no Claude Code login found (run `claude` and /login)".to_string())?;
    let v: Value =
        serde_json::from_str(&text).map_err(|_| "unreadable Claude credentials".to_string())?;
    let o = &v["claudeAiOauth"];
    let token = o["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "no Claude Code login found (run `claude` and /login)".to_string())?;
    Ok(OAuth {
        token: token.to_string(),
        expires_at_ms: o["expiresAt"].as_u64().unwrap_or(0),
        subscription: o["subscriptionType"].as_str().map(str::to_string),
    })
}

fn iso_to_epoch(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(if n > 10_000_000_000 { n / 1000 } else { n });
    }
    let s = v.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp().max(0) as u64)
}

fn window(v: &Value) -> Option<(f64, Option<u64>)> {
    if v.is_null() {
        return None;
    }
    let pct = v["utilization"].as_f64()?;
    Some((pct, iso_to_epoch(&v["resets_at"])))
}

pub fn fetch() -> ClaudeUsage {
    let mut u = ClaudeUsage {
        fetched_at_ms: now_ms(),
        ..Default::default()
    };
    let auth = match read_oauth() {
        Ok(a) => a,
        Err(e) => {
            u.error = Some(e);
            return u;
        }
    };
    u.subscription = auth.subscription.clone();
    if auth.expires_at_ms > 0 && auth.expires_at_ms <= now_ms() {
        u.error = Some("Claude Code token expired: run `claude` once to refresh it".into());
        return u;
    }
    let resp = ureq::get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {}", auth.token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .header("User-Agent", "scrooge-widget")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .max_redirects(0)
        .build()
        .call();
    let body: Value = match resp {
        Ok(mut r) => match r.body_mut().read_json() {
            Ok(v) => v,
            Err(e) => {
                u.error = Some(format!("invalid usage response: {e}"));
                return u;
            }
        },
        Err(ureq::Error::StatusCode(401)) => {
            u.error =
                Some("Claude rejected the token (401): run `claude` once to refresh it".into());
            return u;
        }
        Err(ureq::Error::StatusCode(code)) => {
            u.error = Some(format!("Anthropic API error {code}"));
            return u;
        }
        Err(e) => {
            u.error = Some(format!("network error: {e}"));
            return u;
        }
    };
    parse_into(&mut u, &body);
    u
}

fn parse_into(u: &mut ClaudeUsage, body: &Value) {
    let five = window(&body["five_hour"]);
    let seven = window(&body["seven_day"]);
    if five.is_some() || seven.is_some() {
        u.limits = Some(RateLimitInfo {
            source: "claude".into(),
            five_hour_pct: five.map(|w| w.0),
            five_hour_resets_at: five.and_then(|w| w.1),
            five_hour_window_minutes: Some(300),
            seven_day_pct: seven.map(|w| w.0),
            seven_day_resets_at: seven.and_then(|w| w.1),
            seven_day_window_minutes: Some(10080),
            updated_at: Some(u.fetched_at_ms / 1000),
        });
    } else {
        u.error = Some("no usage windows in response (API plan or unsupported account)".into());
    }
    for (key, label) in [
        ("seven_day_opus", "opus 7d"),
        ("seven_day_sonnet", "sonnet 7d"),
    ] {
        if let Some((pct, resets_at)) = window(&body[key]) {
            u.extra.push(ExtraWindow {
                label: label.into(),
                pct,
                resets_at,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_windows() {
        let body: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":35.0,"resets_at":"2026-11-06T03:00:00+00:00"},
                "seven_day":{"utilization":12.5,"resets_at":"2026-11-10T00:00:00Z"},
                "seven_day_opus":{"utilization":40.0,"resets_at":null},
                "seven_day_sonnet":null}"#,
        )
        .unwrap();
        let mut u = ClaudeUsage::default();
        parse_into(&mut u, &body);
        let l = u.limits.unwrap();
        assert_eq!(l.source, "claude");
        assert_eq!(l.five_hour_pct, Some(35.0));
        assert_eq!(l.five_hour_resets_at, Some(1793934000));
        assert_eq!(l.seven_day_pct, Some(12.5));
        assert_eq!(u.extra.len(), 1);
        assert_eq!(u.extra[0].label, "opus 7d");
        assert!(u.error.is_none());
    }

    #[test]
    fn missing_windows_reports_error() {
        let mut u = ClaudeUsage::default();
        parse_into(&mut u, &serde_json::json!({}));
        assert!(u.limits.is_none());
        assert!(u.error.is_some());
    }
}
