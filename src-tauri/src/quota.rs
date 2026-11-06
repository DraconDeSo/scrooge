//! GitHub Copilot account quota.
//!
//! Source: `GET https://api.github.com/copilot_internal/user` — the same
//! (undocumented) endpoint the VS Code Copilot extension uses to render its
//! quota indicator. Authenticated with the user's existing GitHub CLI login
//! (`gh auth token --user <login>`); Scrooge never stores the token.
//!
//! Accounts: every github.com account known to `gh`, plus the account the
//! Copilot CLI / app is using (`lastLoggedInUser` in `~/.copilot/config.json`),
//! which is listed first. Accounts can be hidden in Scrooge settings.

use serde::Serialize;
use serde_json::Value;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Default)]
pub struct QuotaBucket {
    /// `premium_interactions`, `chat`, `completions`, ...
    pub id: String,
    pub unlimited: bool,
    pub entitlement: f64,
    pub remaining: f64,
    pub percent_remaining: f64,
    pub overage_count: f64,
    pub overage_permitted: bool,
    pub credits_used: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CopilotQuota {
    pub login: Option<String>,
    pub plan: Option<String>,
    pub sku: Option<String>,
    /// ISO date of the next monthly reset (e.g. `2026-12-01`).
    pub reset_date: Option<String>,
    pub buckets: Vec<QuotaBucket>,
    /// Human-readable failure reason; `None` on success.
    pub error: Option<String>,
    pub fetched_at_ms: u64,
    /// True for the account the Copilot CLI / app is currently using.
    pub is_cli: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GhAccount {
    pub login: String,
    pub active: bool,
}

pub fn hidden_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Accounts known to the GitHub CLI for github.com.
pub fn gh_accounts() -> Vec<GhAccount> {
    let Ok(out) = hidden_command("gh")
        .args(["auth", "status", "--json", "hosts"])
        .output()
    else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else {
        return Vec::new();
    };
    v["hosts"]["github.com"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some(GhAccount {
                        login: a["login"].as_str()?.to_string(),
                        active: a["active"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Account the Copilot CLI / app last logged in with.
pub fn copilot_cli_login() -> Option<String> {
    let root = std::env::var_os("COPILOT_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".copilot"));
    let text = std::fs::read_to_string(root.join("config.json")).ok()?;
    // The Copilot CLI writes JSON with `//` comment lines (JSONC).
    let v: Value = serde_json::from_str(&strip_json_comments(&text)).ok()?;
    v["lastLoggedInUser"]["login"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Remove `//` and `/* */` comments outside of JSON strings.
pub fn strip_json_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let (mut in_str, mut escaped) = (false, false);
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_str = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Every account to show: the Copilot CLI account first, then the other gh
/// accounts, minus the ones hidden in settings (case-insensitive).
pub fn accounts_to_show(hidden: &[String]) -> (Vec<String>, Option<String>) {
    let cli = copilot_cli_login();
    let mut logins: Vec<String> = Vec::new();
    if let Some(l) = &cli {
        logins.push(l.clone());
    }
    for a in gh_accounts() {
        if !logins.iter().any(|l| l.eq_ignore_ascii_case(&a.login)) {
            logins.push(a.login);
        }
    }
    logins.retain(|l| !hidden.iter().any(|h| h.eq_ignore_ascii_case(l)));
    (logins, cli)
}

/// Quota for every visible account. With no gh accounts at all, falls back
/// to a single unauthenticated attempt so the panel can show the gh error.
pub fn fetch_all(hidden: &[String]) -> Vec<CopilotQuota> {
    let (logins, cli) = accounts_to_show(hidden);
    if logins.is_empty() {
        return vec![fetch(None)];
    }
    logins
        .iter()
        .map(|l| {
            let mut q = fetch(Some(l));
            q.is_cli = cli.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(l));
            q
        })
        .collect()
}

fn gh_token(login: Option<&str>) -> Result<String, String> {
    let mut cmd = hidden_command("gh");
    cmd.args(["auth", "token"]);
    if let Some(l) = login {
        cmd.args(["--user", l]);
    }
    let out = cmd
        .output()
        .map_err(|_| "GitHub CLI (gh) not found: install it and run `gh auth login`".to_string())?;
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || token.is_empty() {
        return Err(match login {
            Some(l) => format!("gh has no token for '{l}': run `gh auth login` with that account"),
            None => "gh is not logged in: run `gh auth login`".to_string(),
        });
    }
    Ok(token)
}

pub fn fetch(login: Option<&str>) -> CopilotQuota {
    let login = login.map(str::to_string);
    let mut q = CopilotQuota {
        login: login.clone(),
        fetched_at_ms: now_ms(),
        ..Default::default()
    };
    let token = match gh_token(login.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            q.error = Some(e);
            return q;
        }
    };
    let resp = ureq::get("https://api.github.com/copilot_internal/user")
        .set("Authorization", &format!("token {token}"))
        .set("Accept", "application/json")
        .set("User-Agent", "scrooge-widget")
        .set("X-GitHub-Api-Version", "2025-04-01")
        .timeout(std::time::Duration::from_secs(15))
        .call();
    let body: Value = match resp {
        Ok(r) => match r.into_json() {
            Ok(v) => v,
            Err(e) => {
                q.error = Some(format!("invalid quota response: {e}"));
                return q;
            }
        },
        Err(ureq::Error::Status(code, _)) => {
            q.error = Some(match code {
                401 => "GitHub rejected the gh token (401)".to_string(),
                403 | 404 => format!("no Copilot access for this account ({code})"),
                _ => format!("GitHub API error {code}"),
            });
            return q;
        }
        Err(e) => {
            q.error = Some(format!("network error: {e}"));
            return q;
        }
    };
    parse_into(&mut q, &body);
    q
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn parse_into(q: &mut CopilotQuota, body: &Value) {
    q.plan = body["copilot_plan"].as_str().map(str::to_string);
    q.sku = body["access_type_sku"].as_str().map(str::to_string);
    q.reset_date = body["quota_reset_date"].as_str().map(str::to_string);
    if q.login.is_none() {
        q.login = body["login"].as_str().map(str::to_string);
    }
    if let Some(snaps) = body["quota_snapshots"].as_object() {
        let mut buckets: Vec<QuotaBucket> = snaps
            .iter()
            .map(|(id, s)| QuotaBucket {
                id: id.clone(),
                unlimited: s["unlimited"].as_bool().unwrap_or(false),
                entitlement: f(&s["entitlement"]),
                remaining: f(&s["remaining"]),
                percent_remaining: f(&s["percent_remaining"]),
                overage_count: f(&s["overage_count"]),
                overage_permitted: s["overage_permitted"].as_bool().unwrap_or(false),
                credits_used: f(&s["credits_used"]),
            })
            .collect();
        // premium_interactions first: it is the one that actually runs out.
        buckets.sort_by_key(|b| match b.id.as_str() {
            "premium_interactions" => 0,
            "chat" => 1,
            "completions" => 2,
            _ => 3,
        });
        q.buckets = buckets;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_jsonc_comments_but_not_urls_in_strings() {
        let src = "// header\n// more\n{\n  \"url\": \"https://github.com\", /* c */\n  \"a\": 1 // tail\n}\n";
        let v: Value = serde_json::from_str(&strip_json_comments(src)).unwrap();
        assert_eq!(v["url"], "https://github.com");
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn parses_quota_snapshot() {
        let body: Value = serde_json::from_str(
            r#"{"login":"me","copilot_plan":"individual","quota_reset_date":"2026-12-01",
                "quota_snapshots":{
                  "chat":{"unlimited":false,"entitlement":200,"remaining":150,"percent_remaining":75.0},
                  "premium_interactions":{"unlimited":false,"entitlement":300,"remaining":30,"percent_remaining":10.0,"overage_count":0,"overage_permitted":true,"credits_used":12}
                }}"#,
        )
        .unwrap();
        let mut q = CopilotQuota::default();
        parse_into(&mut q, &body);
        assert_eq!(q.login.as_deref(), Some("me"));
        assert_eq!(q.reset_date.as_deref(), Some("2026-12-01"));
        assert_eq!(q.buckets[0].id, "premium_interactions");
        assert_eq!(q.buckets[0].percent_remaining, 10.0);
        assert!(q.buckets[0].overage_permitted);
    }
}
