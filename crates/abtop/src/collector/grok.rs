//! xAI Grok Build CLI (`grok`) session collector.
//!
//! Layout (`GROK_HOME`, default `~/.grok`):
//!
//! ```text
//! ~/.grok/sessions/<url-encoded cwd>/<session-id>/
//!     summary.json   info, generated_title, session_summary, current_model_id,
//!                    created_at, updated_at, num_messages, num_chat_messages
//!     signals.json   turnCount, contextTokensUsed, contextWindowTokens,
//!                    totalTokensBeforeCompaction, primaryModelId, modelsUsed
//!     updates.jsonl  ACP update stream (written while the agent works)
//! ~/.grok/sessions/<slug>/.cwd   original cwd when the encoded name is too long
//! ```
//!
//! Grok publishes no PID registry: with N running `grok` processes the N
//! most recently updated sessions (last 24h) are shown. `signals.json` has no
//! input/output split, so token totals are approximate (reported as input).

use super::copilot::{clean_text, collect_children};
use super::{process, AgentCollector, SharedProcessData};
use crate::model::{AgentSession, LaunchSurface, SessionStatus};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const RECENT: Duration = Duration::from_secs(24 * 3600);

pub struct GrokCollector {
    root: PathBuf,
    candidates: Vec<PathBuf>,
    last_pids: Vec<u32>,
}

impl Default for GrokCollector {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn is_grok_command(cmd: &str) -> bool {
    process::cmd_first_token_has_binary(cmd, "grok")
}

/// `%2F`-style percent decoding for the cwd folder name.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// RFC 3339 string, or epoch seconds / milliseconds.
fn time_ms(v: &Value) -> u64 {
    if let Some(s) = v.as_str() {
        return chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.timestamp_millis().max(0) as u64)
            .unwrap_or(0);
    }
    match v.as_u64() {
        Some(n) if n > 10_000_000_000 => n,
        Some(n) => n * 1000,
        None => 0,
    }
}

fn read_json(p: &Path) -> Value {
    fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Newest activity timestamp of a session folder.
fn activity(dir: &Path) -> Option<SystemTime> {
    ["updates.jsonl", "signals.json", "summary.json"]
        .iter()
        .filter_map(|f| mtime(&dir.join(f)))
        .max()
}

impl GrokCollector {
    pub fn new() -> Self {
        let root = std::env::var_os("GROK_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".grok"));
        Self::with_root(root)
    }

    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            candidates: Vec::new(),
            last_pids: Vec::new(),
        }
    }

    fn scan_sessions(&self) -> Vec<PathBuf> {
        let cutoff = SystemTime::now() - RECENT;
        let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
        let Ok(groups) = fs::read_dir(self.root.join("sessions")) else {
            return Vec::new();
        };
        for g in groups.flatten() {
            let Ok(sessions) = fs::read_dir(g.path()) else {
                continue;
            };
            for s in sessions.flatten() {
                let dir = s.path();
                if !dir.is_dir() {
                    continue;
                }
                if let Some(t) = activity(&dir).filter(|t| *t >= cutoff) {
                    found.push((t, dir));
                }
            }
        }
        found.sort_by_key(|a| std::cmp::Reverse(a.0));
        found.into_iter().map(|(_, p)| p).collect()
    }

    fn session_cwd(dir: &Path, summary: Option<&Value>) -> String {
        let owned;
        let summary = match summary {
            Some(s) => s,
            None => {
                owned = read_json(&dir.join("summary.json"));
                &owned
            }
        };
        let group = dir.parent();
        // cwd: summary.info.* > .cwd file > decoded folder name
        let info = &summary["info"];
        [
            "cwd",
            "working_directory",
            "workingDirectory",
            "workspace",
            "path",
        ]
        .iter()
        .find_map(|k| info[*k].as_str())
        .map(str::to_string)
        .or_else(|| {
            group
                .and_then(|g| fs::read_to_string(g.join(".cwd")).ok())
                .map(|s| s.trim().to_string())
        })
        .or_else(|| {
            group
                .and_then(|g| g.file_name())
                .map(|n| percent_decode(&n.to_string_lossy()))
        })
        .unwrap_or_default()
    }

    fn build(&self, dir: &Path, pid: u32, shared: &SharedProcessData) -> AgentSession {
        let summary = read_json(&dir.join("summary.json"));
        let signals = read_json(&dir.join("signals.json"));
        let info = &summary["info"];
        let cwd = Self::session_cwd(dir, Some(&summary));
        let session_id = ["session_id", "sessionId", "id"]
            .iter()
            .find_map(|k| info[*k].as_str())
            .map(str::to_string)
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let title = summary["generated_title"]
            .as_str()
            .or_else(|| summary["session_summary"].as_str())
            .map(|s| clean_text(s, 80))
            .unwrap_or_default();
        let model = summary["current_model_id"]
            .as_str()
            .or_else(|| signals["primaryModelId"].as_str())
            .unwrap_or("-")
            .to_string();
        let u = |v: &Value| v.as_u64().unwrap_or(0);
        let ctx_used = u(&signals["contextTokensUsed"]);
        let ctx_window = u(&signals["contextWindowTokens"]);
        let total = u(&signals["totalTokensBeforeCompaction"]) + ctx_used;
        let started = time_ms(&summary["created_at"]);
        let busy = mtime(&dir.join("updates.jsonl"))
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|e| e < Duration::from_secs(15));
        let status = if busy {
            SessionStatus::Thinking
        } else {
            SessionStatus::Waiting
        };
        let project_name = process::last_path_segment(&cwd)
            .filter(|s| !s.is_empty())
            .unwrap_or("?")
            .to_string();
        let (children, mem_mb) = collect_children(pid, shared);
        AgentSession {
            agent_cli: "grok",
            launch_surface: LaunchSurface::Cli,
            pid,
            session_id,
            cwd,
            project_name,
            started_at: started,
            status: status.clone(),
            model,
            effort: String::new(),
            context_percent: if ctx_window > 0 {
                (ctx_used as f64 / ctx_window as f64 * 100.0).min(100.0)
            } else {
                0.0
            },
            total_input_tokens: total,
            total_output_tokens: 0,
            total_cache_read: 0,
            total_cache_create: 0,
            turn_count: u(&signals["turnCount"]) as u32,
            current_tasks: vec![if status == SessionStatus::Thinking {
                "working..."
            } else {
                "waiting for input"
            }
            .into()],
            mem_mb,
            version: String::new(),
            git_branch: String::new(),
            git_added: 0,
            git_modified: 0,
            token_history: vec![],
            context_history: vec![],
            compaction_count: 0,
            context_window: ctx_window,
            subagents: vec![],
            mem_file_count: 0,
            mem_line_count: 0,
            children,
            initial_prompt: title,
            first_assistant_text: String::new(),
            chat_messages: vec![],
            tool_calls: vec![],
            pending_since_ms: 0,
            thinking_since_ms: 0,
            file_accesses: vec![],
            copilot: None,
            config_root: super::abbrev_path(&self.root),
        }
    }
}

impl AgentCollector for GrokCollector {
    fn collect(&mut self, shared: &SharedProcessData) -> Vec<AgentSession> {
        let mut pids: Vec<u32> = shared
            .process_info
            .iter()
            .filter(|(_, p)| is_grok_command(&p.command))
            .map(|(pid, _)| *pid)
            .collect();
        let set: HashSet<u32> = pids.iter().copied().collect();
        pids.retain(|pid| {
            shared
                .process_info
                .get(pid)
                .is_none_or(|p| !set.contains(&p.ppid))
        });
        pids.sort_unstable();
        if pids.is_empty() {
            self.candidates.clear();
            self.last_pids.clear();
            return Vec::new();
        }
        if shared.slow_tick || pids != self.last_pids || self.candidates.is_empty() {
            self.candidates = self.scan_sessions();
            self.last_pids = pids.clone();
        }
        process::match_pids_by_cwd(&pids, &self.candidates, &shared.process_info, |dir| {
            Self::session_cwd(dir, None)
        })
        .into_iter()
        .map(|(dir, pid)| self.build(&dir, pid, shared))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn decodes_cwd_folder_names() {
        assert_eq!(percent_decode("C%3A%5Crepos%5Cdemo"), "C:\\repos\\demo");
        assert_eq!(percent_decode("%2Fhome%2Fme%2Fapp"), "/home/me/app");
    }

    #[test]
    fn reads_summary_and_signals() {
        let tmp = tempfile::tempdir().unwrap();
        let (encoded_cwd, cwd) = if cfg!(windows) {
            ("C%3A%5Crepos%5Cdemo", r"C:\repos\demo")
        } else {
            ("%2Fhome%2Fme%2Fdemo", "/home/me/demo")
        };
        let dir = tmp
            .path()
            .join("sessions")
            .join(encoded_cwd)
            .join("0193-abc");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("summary.json"), r#"{"info":{"id":"0193-abc"},"generated_title":"Refactor auth","current_model_id":"grok-code-2","created_at":"2026-11-06T09:00:00Z"}"#).unwrap();
        fs::write(dir.join("signals.json"), r#"{"turnCount":7,"contextTokensUsed":50000,"contextWindowTokens":256000,"totalTokensBeforeCompaction":120000}"#).unwrap();
        let c = GrokCollector::with_root(tmp.path().to_path_buf());
        assert_eq!(c.scan_sessions().len(), 1);
        let shared = SharedProcessData {
            process_info: HashMap::new(),
            children_map: HashMap::new(),
            ports: HashMap::new(),
            slow_tick: true,
            mcp_server_pids: Default::default(),
            mcp_owned_rollouts: Default::default(),
            mcp_suppress: true,
            desktop_rollout_fd_map: HashMap::new(),
        };
        let s = c.build(&dir, 42, &shared);
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.project_name, "demo");
        assert_eq!(s.model, "grok-code-2");
        assert_eq!(s.turn_count, 7);
        assert_eq!(s.total_input_tokens, 170_000);
        assert!((s.context_percent - 19.53).abs() < 0.1);
        assert_eq!(s.initial_prompt, "Refactor auth");
    }
}
