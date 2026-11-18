//! Google Antigravity desktop/IDE live session collector.
//!
//! Antigravity stores Gemini usage events below
//! `~/.gemini/{antigravity,antigravity-ide}/brain/<session>/` while the app is
//! running.  The historical usage scanner already understands these files;
//! this collector makes the same local data visible in the live dashboard.

use super::copilot::collect_children;
use super::{process, AgentCollector, SharedProcessData};
use crate::model::{AgentSession, LaunchSurface, SessionStatus};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const GEMINI_CONTEXT: u64 = 1_048_576;
const RECENT: Duration = Duration::from_secs(24 * 3600);
const MAX_READ_PER_TICK: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct ParsedTranscript {
    offset: u64,
    partial: String,
    started_at_ms: u64,
    cwd: String,
    model: String,
    input: u64,
    output: u64,
    cache_read: u64,
    turns: u32,
    last_prompt_tokens: u64,
    token_history: Vec<u64>,
    response_done: bool,
}

pub struct AntigravityCollector {
    roots: Vec<PathBuf>,
    parsed: HashMap<PathBuf, ParsedTranscript>,
    candidates: Vec<PathBuf>,
    last_pids: Vec<u32>,
}

impl Default for AntigravityCollector {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn is_antigravity_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("antigravity")
        && !lower.contains("--type=")
        && !lower.contains("crashpad")
        && !lower.contains("update")
}

fn timestamp_ms(v: &Value) -> u64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis().max(0) as u64)
        .unwrap_or(0)
}

impl AntigravityCollector {
    pub fn new() -> Self {
        let home = std::env::var_os("GEMINI_CLI_HOME")
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        Self::with_base(home.join(".gemini"))
    }

    pub fn with_base(base: PathBuf) -> Self {
        Self {
            roots: vec![base.join("antigravity"), base.join("antigravity-ide")],
            parsed: HashMap::new(),
            candidates: Vec::new(),
            last_pids: Vec::new(),
        }
    }

    fn scan_transcripts(&self) -> Vec<PathBuf> {
        let cutoff = SystemTime::now() - RECENT;
        let mut found = Vec::new();
        for root in &self.roots {
            let Ok(entries) = fs::read_dir(root.join("brain")) else {
                continue;
            };
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let logs = entry.path().join(".system_generated").join("logs");
                let full = logs.join("transcript_full.jsonl");
                let path = if full.is_file() {
                    full
                } else {
                    logs.join("transcript.jsonl")
                };
                let Ok(modified) = fs::metadata(&path).and_then(|m| m.modified()) else {
                    continue;
                };
                if modified >= cutoff {
                    found.push((modified, path));
                }
            }
        }
        found.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        found.into_iter().map(|(_, path)| path).collect()
    }

    fn refresh(&mut self, path: &Path) {
        let Ok(meta) = fs::metadata(path) else { return };
        let len = meta.len();
        let state = self.parsed.entry(path.to_path_buf()).or_default();
        if len < state.offset {
            *state = ParsedTranscript::default();
        }
        if len == state.offset {
            return;
        }
        let Ok(mut file) = fs::File::open(path) else {
            return;
        };
        if file.seek(SeekFrom::Start(state.offset)).is_err() {
            return;
        }
        let to_read = (len - state.offset).min(MAX_READ_PER_TICK);
        let mut bytes = Vec::with_capacity(to_read as usize);
        if file.take(to_read).read_to_end(&mut bytes).is_err() {
            return;
        }
        state.offset += bytes.len() as u64;
        let mut text = std::mem::take(&mut state.partial);
        text.push_str(&String::from_utf8_lossy(&bytes));
        let complete = text.ends_with('\n');
        let mut lines: Vec<&str> = text.split('\n').collect();
        if !complete {
            state.partial = lines.pop().unwrap_or_default().to_string();
        }
        for line in lines {
            let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if value["type"] != "PLANNER_RESPONSE" {
                continue;
            }
            state.response_done = value["status"] == "DONE";
            if !state.response_done {
                continue;
            }
            let ts = timestamp_ms(&value["created_at"]);
            if state.started_at_ms == 0 || (ts > 0 && ts < state.started_at_ms) {
                state.started_at_ms = ts;
            }
            if let Some(cwd) = value["cwd"].as_str().filter(|s| !s.is_empty()) {
                state.cwd = cwd.to_string();
            }
            if let Some(model) = value["model"].as_str().filter(|s| !s.is_empty()) {
                state.model = model.to_string();
            }
            let input = value["input_tokens"].as_u64().unwrap_or(0);
            let cache = value["cache_read_tokens"].as_u64().unwrap_or(0);
            let output = value["output_tokens"].as_u64().unwrap_or(0);
            state.input = state.input.saturating_add(input.saturating_sub(cache));
            state.cache_read = state.cache_read.saturating_add(cache);
            state.output = state.output.saturating_add(output);
            state.turns = state.turns.saturating_add(1);
            state.last_prompt_tokens = input;
            if state.token_history.len() < 10_000 {
                state.token_history.push(input.saturating_add(output));
            }
        }
    }

    fn session_id(path: &Path) -> String {
        path.parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "antigravity".into())
    }

    fn surface(path: &Path) -> LaunchSurface {
        if path.components().any(|part| {
            part.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("antigravity-ide")
        }) {
            LaunchSurface::Ide
        } else {
            LaunchSurface::App
        }
    }
}

impl AgentCollector for AntigravityCollector {
    fn collect(&mut self, shared: &SharedProcessData) -> Vec<AgentSession> {
        let mut pids: Vec<u32> = shared
            .process_info
            .iter()
            .filter(|(_, info)| is_antigravity_command(&info.command))
            .map(|(pid, _)| *pid)
            .collect();
        let set: HashSet<u32> = pids.iter().copied().collect();
        pids.retain(|pid| {
            shared
                .process_info
                .get(pid)
                .is_none_or(|info| !set.contains(&info.ppid))
        });
        pids.sort_unstable();
        if pids.is_empty() {
            self.parsed.clear();
            self.candidates.clear();
            self.last_pids.clear();
            return Vec::new();
        }
        if shared.slow_tick || pids != self.last_pids || self.candidates.is_empty() {
            self.candidates = self.scan_transcripts();
            self.last_pids = pids.clone();
        }

        let pairs: Vec<(PathBuf, u32)> = self
            .candidates
            .iter()
            .take(pids.len())
            .cloned()
            .zip(pids.iter().copied())
            .collect();
        let mut sessions = Vec::new();
        for (path, pid) in &pairs {
            self.refresh(path);
            let Some(state) = self.parsed.get(path) else {
                continue;
            };
            let cwd = state.cwd.clone();
            let project_name = process::last_path_segment(&cwd)
                .filter(|s| !s.is_empty())
                .unwrap_or("Antigravity")
                .to_string();
            let (children, mem_mb) = collect_children(*pid, shared);
            let waiting = state.response_done;
            sessions.push(AgentSession {
                agent_cli: "gemini",
                launch_surface: Self::surface(path),
                pid: *pid,
                session_id: Self::session_id(path),
                cwd,
                project_name,
                started_at: state.started_at_ms,
                status: if waiting {
                    SessionStatus::Waiting
                } else {
                    SessionStatus::Thinking
                },
                model: if state.model.is_empty() {
                    "Gemini".into()
                } else {
                    state.model.clone()
                },
                effort: String::new(),
                context_percent: (state.last_prompt_tokens as f64 / GEMINI_CONTEXT as f64 * 100.0)
                    .min(100.0),
                total_input_tokens: state.input,
                total_output_tokens: state.output,
                total_cache_read: state.cache_read,
                total_cache_create: 0,
                turn_count: state.turns,
                current_tasks: vec![if waiting {
                    "waiting for input".into()
                } else {
                    "thinking...".into()
                }],
                mem_mb,
                version: String::new(),
                git_branch: String::new(),
                git_added: 0,
                git_modified: 0,
                token_history: state.token_history.clone(),
                context_history: Vec::new(),
                compaction_count: 0,
                context_window: GEMINI_CONTEXT,
                subagents: Vec::new(),
                mem_file_count: 0,
                mem_line_count: 0,
                children,
                initial_prompt: String::new(),
                first_assistant_text: String::new(),
                chat_messages: Vec::new(),
                tool_calls: Vec::new(),
                pending_since_ms: 0,
                thinking_since_ms: 0,
                file_accesses: Vec::new(),
                token_calls: Vec::new(),
                copilot: None,
                config_root: super::abbrev_path(path),
            });
        }
        let keep: HashSet<&PathBuf> = pairs.iter().map(|(path, _)| path).collect();
        self.parsed.retain(|path, _| keep.contains(path));
        sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detects_main_antigravity_process_only() {
        assert!(is_antigravity_command(
            r#"C:\Program Files\Antigravity\Antigravity.exe"#
        ));
        assert!(!is_antigravity_command(
            r#"C:\Program Files\Antigravity\Antigravity.exe --type=renderer"#
        ));
    }

    #[test]
    fn reads_local_usage_without_authentication() {
        let temp = tempfile::tempdir().unwrap();
        let logs = temp
            .path()
            .join("antigravity")
            .join("brain")
            .join("session-1")
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&logs).unwrap();
        let transcript = logs.join("transcript_full.jsonl");
        let mut file = fs::File::create(&transcript).unwrap();
        writeln!(
            file,
            r#"{{"type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-06T08:00:00Z","cwd":"C:/work/demo","model":"gemini-3-pro","input_tokens":120,"output_tokens":20,"cache_read_tokens":90}}"#
        )
        .unwrap();

        let mut collector = AntigravityCollector::with_base(temp.path().to_path_buf());
        assert_eq!(collector.scan_transcripts(), vec![transcript.clone()]);
        collector.refresh(&transcript);
        let parsed = collector.parsed.get(&transcript).unwrap();
        assert_eq!(parsed.input, 30);
        assert_eq!(parsed.cache_read, 90);
        assert_eq!(parsed.output, 20);
        assert_eq!(parsed.turns, 1);
        assert_eq!(parsed.cwd, "C:/work/demo");
        assert_eq!(parsed.model, "gemini-3-pro");
    }
}
