//! GitHub Copilot CLI / GitHub Copilot app session collector.
//!
//! Data layout (Copilot CLI >= 1.0, shared by the `copilot` CLI and the
//! GitHub Copilot desktop app, which drives `copilot-runtime --server`):
//!
//! ```text
//! ~/.copilot/session-state/<sessionId>/
//!     events.jsonl        append-only event log (one JSON object per line)
//!     workspace.yaml      flat `key: value` metadata (cwd, branch, name, client_name, ...)
//!     inuse.<PID>.lock    present while process <PID> holds the session open
//! ```
//!
//! Discovery: a session is live when its directory contains an
//! `inuse.<PID>.lock` whose PID is alive and is a Copilot process.
//!
//! Event types consumed (`{"type": ..., "data": {...}, "timestamp": ...}`):
//! - `session.start` / `session.resume`: cwd, branch, model, effort, version
//! - `session.usage_record`: per-call tokens, model, `maxPromptTokens`
//! - `session.usage_checkpoint` / `session.shutdown`: premium requests, AIU, code changes
//! - `assistant.turn_start` / `assistant.turn_end`: thinking state
//! - `tool.execution_start` / `tool.execution_complete`: running tools + timeline
//! - `user.message` / `assistant.message`: chat tail
//!
//! All parsing is defensive: these are undocumented Copilot internals.

use super::{process, redact_secrets, sanitize_terminal_text, AgentCollector, SharedProcessData};
use crate::model::{
    AgentSession, ChatMessage, ChatRole, ChildProcess, CopilotUsage, LaunchSurface, SessionStatus,
    ToolCall, MAX_CHAT_MESSAGES,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Keep at most this many tool calls / history points per session.
const MAX_TOOL_CALLS: usize = 200;
const MAX_HISTORY: usize = 500;
/// Cap a single incremental read so a huge first scan cannot stall a tick.
const MAX_READ_PER_TICK: u64 = 64 * 1024 * 1024;

/// Incrementally parsed state of one `events.jsonl`.
#[derive(Default)]
struct ParsedSession {
    offset: u64,
    partial: String,

    session_id: String,
    started_at_ms: u64,
    version: String,
    cwd: String,
    branch: String,
    model: String,
    effort: String,
    shut_down: bool,

    input_tokens: u64,
    output_tokens: u64,
    cache_read: u64,
    cache_write: u64,
    reasoning_tokens: u64,
    last_prompt_tokens: u64,
    max_prompt_tokens: u64,
    usage_records: u32,
    token_history: Vec<u64>,
    context_history: Vec<u64>,
    compaction_count: u32,

    premium_requests: f64,
    nano_aiu: u64,
    user_requests: u64,
    lines_added: u64,
    lines_removed: u64,

    user_turns: u32,
    turn_open: bool,
    thinking_since_ms: u64,
    /// toolCallId -> (name, arg, start ms)
    tools_in_flight: HashMap<String, (String, String, u64)>,
    tool_calls: Vec<ToolCall>,
    initial_prompt: String,
    first_assistant_text: String,
    chat: Vec<ChatMessage>,
    last_event_ms: u64,
}

#[derive(Default, Clone)]
struct Workspace {
    cwd: String,
    branch: String,
    name: String,
    client_name: String,
}

struct CachedWorkspace {
    mtime: Option<SystemTime>,
    ws: Workspace,
}

pub struct CopilotCollector {
    root: PathBuf,
    parsed: HashMap<PathBuf, ParsedSession>,
    workspaces: HashMap<PathBuf, CachedWorkspace>,
    /// (session dir, pid) pairs from the last full lock scan.
    live_locks: Vec<(PathBuf, u32)>,
    last_copilot_pids: Vec<u32>,
}

impl Default for CopilotCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl CopilotCollector {
    pub fn new() -> Self {
        let root = std::env::var_os("COPILOT_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".copilot"));
        Self::with_root(root)
    }

    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            parsed: HashMap::new(),
            workspaces: HashMap::new(),
            live_locks: Vec::new(),
            last_copilot_pids: Vec::new(),
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.root.join("session-state")
    }

    /// Scan every session dir for `inuse.<PID>.lock` files.
    fn scan_locks(&self) -> Vec<(PathBuf, u32)> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(self.state_dir()) else {
            return out;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let Ok(files) = fs::read_dir(&dir) else {
                continue;
            };
            for f in files.flatten() {
                let name = f.file_name();
                if let Some(pid) = parse_inuse_lock(&name.to_string_lossy()) {
                    out.push((dir.clone(), pid));
                }
            }
        }
        out
    }

    fn workspace(&mut self, dir: &Path) -> Workspace {
        let path = dir.join("workspace.yaml");
        let mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        if let Some(c) = self.workspaces.get(dir) {
            if c.mtime == mtime {
                return c.ws.clone();
            }
        }
        let ws = fs::read_to_string(&path)
            .map(|s| parse_workspace_yaml(&s))
            .unwrap_or_default();
        self.workspaces.insert(
            dir.to_path_buf(),
            CachedWorkspace {
                mtime,
                ws: ws.clone(),
            },
        );
        ws
    }

    fn refresh_events(&mut self, dir: &Path) {
        let path = dir.join("events.jsonl");
        let Ok(meta) = fs::metadata(&path) else {
            return;
        };
        let len = meta.len();
        let state = self.parsed.entry(dir.to_path_buf()).or_default();
        if len < state.offset {
            // Truncated / rewritten: start over.
            *state = ParsedSession::default();
        }
        if len == state.offset {
            return;
        }
        let Ok(mut file) = fs::File::open(&path) else {
            return;
        };
        if file.seek(SeekFrom::Start(state.offset)).is_err() {
            return;
        }
        let to_read = (len - state.offset).min(MAX_READ_PER_TICK);
        let mut buf = Vec::with_capacity(to_read as usize);
        if file.take(to_read).read_to_end(&mut buf).is_err() {
            return;
        }
        state.offset += buf.len() as u64;
        let chunk = String::from_utf8_lossy(&buf);
        let mut text = std::mem::take(&mut state.partial);
        text.push_str(&chunk);
        let ends_with_newline = text.ends_with('\n');
        let mut lines: Vec<&str> = text.split('\n').collect();
        if !ends_with_newline {
            if let Some(last) = lines.pop() {
                state.partial = last.to_string();
            }
        }
        for line in lines {
            let line = line.trim_end_matches('\r');
            if !line.is_empty() {
                apply_line(state, line);
            }
        }
    }

    fn build_session(
        &mut self,
        dir: &Path,
        pid: u32,
        shared: &SharedProcessData,
        owns_process: bool,
    ) -> Option<AgentSession> {
        let ws = self.workspace(dir);
        self.refresh_events(dir);
        let p = self.parsed.get(dir)?;
        if p.shut_down {
            return None;
        }

        let session_id = if p.session_id.is_empty() {
            dir.file_name()?.to_string_lossy().into_owned()
        } else {
            p.session_id.clone()
        };
        let cwd = if !p.cwd.is_empty() {
            p.cwd.clone()
        } else {
            ws.cwd.clone()
        };
        let project_name = process::last_path_segment(&cwd)
            .filter(|s| !s.is_empty())
            .unwrap_or("?")
            .to_string();

        let status = if !p.tools_in_flight.is_empty() {
            SessionStatus::Executing
        } else if p.turn_open || p.thinking_since_ms > 0 {
            SessionStatus::Thinking
        } else {
            SessionStatus::Waiting
        };

        let current_tasks = match status {
            SessionStatus::Executing => {
                let mut running: Vec<&(String, String, u64)> = p.tools_in_flight.values().collect();
                running.sort_by_key(|t| t.2);
                running
                    .iter()
                    .map(|(name, arg, _)| {
                        if arg.is_empty() {
                            name.clone()
                        } else {
                            format!("{name} {arg}")
                        }
                    })
                    .collect()
            }
            SessionStatus::Thinking => vec!["thinking...".to_string()],
            _ => vec!["waiting for input".to_string()],
        };

        let (children, mem_mb) = if owns_process {
            collect_children(pid, shared)
        } else {
            (Vec::new(), 0)
        };

        let context_window = if p.max_prompt_tokens > 0 {
            p.max_prompt_tokens
        } else {
            super::context_window_for_model(&p.model, "", 0)
        };
        let context_percent = if context_window > 0 {
            (p.last_prompt_tokens as f64 / context_window as f64 * 100.0).min(100.0)
        } else {
            0.0
        };

        let launch_surface = surface_for_client(&ws.client_name);
        let title = if !ws.name.is_empty() {
            ws.name.clone()
        } else {
            p.initial_prompt.clone()
        };
        let mut pending_since_ms = 0;
        for (_, _, start) in p.tools_in_flight.values() {
            if pending_since_ms == 0 || *start < pending_since_ms {
                pending_since_ms = *start;
            }
        }

        Some(AgentSession {
            agent_cli: "copilot",
            launch_surface,
            pid,
            session_id,
            cwd,
            project_name,
            started_at: p.started_at_ms,
            status,
            model: if p.model.is_empty() {
                "-".into()
            } else {
                p.model.clone()
            },
            effort: p.effort.clone(),
            context_percent,
            total_input_tokens: p.input_tokens,
            total_output_tokens: p.output_tokens,
            total_cache_read: p.cache_read,
            total_cache_create: p.cache_write,
            // Copilot's own accounting (`totalUserRequests`) is authoritative
            // once a checkpoint exists; before that, count prompts seen.
            turn_count: (p.user_requests as u32).max(p.user_turns),
            current_tasks,
            mem_mb,
            version: p.version.clone(),
            git_branch: if !p.branch.is_empty() {
                p.branch.clone()
            } else {
                ws.branch.clone()
            },
            git_added: 0,
            git_modified: 0,
            token_history: p.token_history.clone(),
            context_history: p.context_history.clone(),
            compaction_count: p.compaction_count,
            context_window,
            subagents: vec![],
            mem_file_count: 0,
            mem_line_count: 0,
            children,
            initial_prompt: title,
            first_assistant_text: p.first_assistant_text.clone(),
            chat_messages: p.chat.clone(),
            tool_calls: p.tool_calls.clone(),
            pending_since_ms,
            thinking_since_ms: p.thinking_since_ms,
            file_accesses: vec![],
            copilot: Some(CopilotUsage {
                premium_requests: p.premium_requests,
                nano_aiu: p.nano_aiu,
                user_requests: p.user_requests,
                lines_added: p.lines_added,
                lines_removed: p.lines_removed,
                client_name: ws.client_name.clone(),
                reasoning_tokens: p.reasoning_tokens,
            }),
            config_root: super::abbrev_path(&self.root),
        })
    }
}

impl AgentCollector for CopilotCollector {
    fn collect(&mut self, shared: &SharedProcessData) -> Vec<AgentSession> {
        let mut copilot_pids: Vec<u32> = shared
            .process_info
            .iter()
            .filter(|(_, info)| is_copilot_command(&info.command))
            .map(|(pid, _)| *pid)
            .collect();
        copilot_pids.sort_unstable();
        if copilot_pids.is_empty() {
            self.live_locks.clear();
            self.last_copilot_pids.clear();
            return Vec::new();
        }

        // Full lock scan on slow ticks or whenever the Copilot PID set changes;
        // otherwise also rescan cheaply if nothing was found yet.
        if shared.slow_tick || copilot_pids != self.last_copilot_pids || self.live_locks.is_empty()
        {
            self.live_locks = self.scan_locks();
            self.last_copilot_pids = copilot_pids.clone();
        }

        let pid_set: HashSet<u32> = copilot_pids.iter().copied().collect();
        let mut live: Vec<(PathBuf, u32)> = self
            .live_locks
            .iter()
            .filter(|(dir, pid)| {
                pid_set.contains(pid) && dir.join(format!("inuse.{pid}.lock")).exists()
            })
            .cloned()
            .collect();

        // One runtime process (the desktop app) can host several sessions.
        // Attribute its children/memory to the most recently active one only.
        live.sort_by_key(|(dir, _)| {
            std::cmp::Reverse(
                fs::metadata(dir.join("events.jsonl"))
                    .and_then(|m| m.modified())
                    .ok(),
            )
        });
        let mut owner_assigned: HashSet<u32> = HashSet::new();
        let mut sessions = Vec::new();
        let mut seen_dirs = HashSet::new();
        for (dir, pid) in &live {
            if !seen_dirs.insert(dir.clone()) {
                continue;
            }
            let owns = owner_assigned.insert(*pid);
            if let Some(s) = self.build_session(dir, *pid, shared, owns) {
                sessions.push(s);
            }
        }

        // Drop parse state for sessions that are no longer live.
        self.parsed.retain(|dir, _| seen_dirs.contains(dir));
        self.workspaces.retain(|dir, _| seen_dirs.contains(dir));

        sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
        sessions
    }

    fn discovered_config_dirs(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// `inuse.<PID>.lock` -> PID
fn parse_inuse_lock(name: &str) -> Option<u32> {
    name.strip_prefix("inuse.")?
        .strip_suffix(".lock")?
        .parse()
        .ok()
}

/// Copilot CLI binary, the SDK runtime used by the GitHub Copilot app, or the
/// npm-installed CLI running under node.
pub(crate) fn is_copilot_command(cmd: &str) -> bool {
    process::cmd_has_binary(cmd, "copilot")
        || process::cmd_has_binary(cmd, "copilot-runtime")
        || cmd.contains("@github/copilot")
        || cmd.contains("github-copilot-sdk")
}

fn surface_for_client(client: &str) -> LaunchSurface {
    let c = client.to_ascii_lowercase();
    if c.contains("vscode")
        || c.contains("jetbrains")
        || c.contains("visualstudio")
        || c.contains("ide")
    {
        LaunchSurface::Ide
    } else if c.is_empty() || c.ends_with("/cli") {
        LaunchSurface::Cli
    } else {
        LaunchSurface::App
    }
}

/// Minimal flat YAML reader: `key: value` per line, optional quotes.
fn parse_workspace_yaml(s: &str) -> Workspace {
    let mut ws = Workspace::default();
    for line in s.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = unquote(v.trim());
        match k.trim() {
            "cwd" => ws.cwd = v,
            "branch" => ws.branch = v,
            "name" => ws.name = clean_text(&v, 120),
            "client_name" => ws.client_name = v,
            _ => {}
        }
    }
    ws
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')))
    {
        let inner = &v[1..v.len() - 1];
        if v.starts_with('"') {
            return inner.replace("\\\\", "\\").replace("\\\"", "\"");
        }
        return inner.replace("''", "'");
    }
    v.to_string()
}

pub(super) fn clean_text(s: &str, max_chars: usize) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = sanitize_terminal_text(&redact_secrets(&one_line));
    if cleaned.chars().count() > max_chars {
        let mut t: String = cleaned.chars().take(max_chars.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        cleaned
    }
}

/// Cheap `"type"` extraction: Copilot writes `{"type":"...",...}` first.
fn event_type(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("{\"type\":\"")?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn parse_ts_ms(v: &Value) -> u64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis().max(0) as u64)
        .unwrap_or(0)
}

fn u(v: &Value) -> u64 {
    v.as_u64()
        .or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))
        .unwrap_or(0)
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

const INTERESTING: &[&str] = &[
    "session.start",
    "session.resume",
    "session.shutdown",
    "session.usage_record",
    "session.usage_checkpoint",
    "assistant.turn_start",
    "assistant.turn_end",
    "assistant.message",
    "user.message",
    "tool.execution_start",
    "tool.execution_complete",
    "abort",
];

fn apply_line(st: &mut ParsedSession, line: &str) {
    let Some(ty) = event_type(line) else {
        return;
    };
    if !INTERESTING.contains(&ty) {
        return;
    }
    let Ok(ev) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let ts = parse_ts_ms(&ev["timestamp"]);
    if ts > 0 {
        st.last_event_ms = ts;
    }
    let d = &ev["data"];
    match ty {
        "session.start" | "session.resume" => {
            st.shut_down = false;
            if ty == "session.start" {
                st.session_id = s(&d["sessionId"]);
                st.version = s(&d["copilotVersion"]);
                st.started_at_ms = parse_ts_ms(&d["startTime"]);
            }
            if st.started_at_ms == 0 {
                st.started_at_ms = ts;
            }
            let ctx = &d["context"];
            if let Some(cwd) = ctx["cwd"].as_str() {
                st.cwd = cwd.to_string();
            }
            if let Some(b) = ctx["branch"].as_str() {
                st.branch = b.to_string();
            }
            if let Some(m) = d["selectedModel"].as_str() {
                st.model = m.to_string();
            }
            if let Some(e) = d["reasoningEffort"].as_str() {
                st.effort = e.to_string();
            }
            // A resumed process starts idle.
            st.turn_open = false;
            st.thinking_since_ms = 0;
            st.tools_in_flight.clear();
        }
        "session.shutdown" => {
            apply_accounting(st, &d["accountingSnapshot"]);
            st.shut_down = true;
            st.turn_open = false;
            st.thinking_since_ms = 0;
            st.tools_in_flight.clear();
        }
        "session.usage_checkpoint" => {
            if let Some(p) = d["totalPremiumRequests"].as_f64() {
                st.premium_requests = p;
            }
            if let Some(n) = d["totalNanoAiu"].as_f64() {
                st.nano_aiu = n.max(0.0) as u64;
            }
            apply_accounting(st, &d["accountingSnapshot"]);
        }
        "session.usage_record" => {
            let usage = &d["usage"];
            let input_total = u(&usage["inputTokens"]);
            let cache_read = u(&usage["cacheReadTokens"]);
            let cache_write = u(&usage["cacheWriteTokens"]);
            let output = u(&usage["outputTokens"]);
            let fresh_input = input_total.saturating_sub(cache_read + cache_write);
            st.input_tokens += fresh_input;
            st.output_tokens += output;
            st.cache_read += cache_read;
            st.cache_write += cache_write;
            st.reasoning_tokens += u(&usage["reasoningTokens"]);
            st.usage_records += 1;
            if let Some(m) = usage["model"].as_str() {
                st.model = m.to_string();
            }
            let max_prompt = u(&usage["maxPromptTokens"]);
            if max_prompt > 0 {
                st.max_prompt_tokens = max_prompt;
            }
            if st.last_prompt_tokens > 0 && input_total < st.last_prompt_tokens * 7 / 10 {
                st.compaction_count += 1;
            }
            st.last_prompt_tokens = input_total;
            push_bounded(&mut st.token_history, input_total + output);
            push_bounded(&mut st.context_history, input_total);
        }
        "assistant.turn_start" => {
            st.turn_open = true;
            if st.thinking_since_ms == 0 {
                st.thinking_since_ms = ts;
            }
        }
        "assistant.turn_end" | "abort" => {
            st.turn_open = false;
            st.thinking_since_ms = 0;
            if ty == "abort" {
                st.tools_in_flight.clear();
            }
        }
        "user.message" => {
            let text = clean_text(d["content"].as_str().unwrap_or_default(), 200);
            st.user_turns += 1;
            if st.initial_prompt.is_empty() && !text.is_empty() {
                st.initial_prompt = clean_text(&text, 80);
            }
            st.thinking_since_ms = if ts > 0 { ts } else { 1 };
            if !text.is_empty() {
                push_chat(&mut st.chat, ChatRole::User, text);
            }
        }
        "assistant.message" => {
            if let Some(m) = d["model"].as_str() {
                st.model = m.to_string();
            }
            let text = clean_text(d["content"].as_str().unwrap_or_default(), 200);
            if !text.is_empty() {
                if st.first_assistant_text.is_empty() {
                    st.first_assistant_text = clean_text(&text, 80);
                }
                push_chat(&mut st.chat, ChatRole::Assistant, text);
            }
        }
        "tool.execution_start" => {
            let id = s(&d["toolCallId"]);
            let name = s(&d["toolName"]);
            let arg = tool_arg(&d["arguments"]);
            st.tools_in_flight.insert(id, (name, arg, ts));
        }
        "tool.execution_complete" => {
            let id = s(&d["toolCallId"]);
            if let Some((name, arg, start)) = st.tools_in_flight.remove(&id) {
                let duration_ms = if start > 0 && ts >= start {
                    ts - start
                } else {
                    0
                };
                st.tool_calls.push(ToolCall {
                    name,
                    arg,
                    duration_ms,
                });
                if st.tool_calls.len() > MAX_TOOL_CALLS {
                    let excess = st.tool_calls.len() - MAX_TOOL_CALLS;
                    st.tool_calls.drain(..excess);
                }
            }
            if let Some(cur) = d["toolTelemetry"]["metrics"]["context_current_tokens"].as_f64() {
                if cur > 0.0 {
                    st.last_prompt_tokens = cur as u64;
                }
            }
            if let Some(lim) = d["toolTelemetry"]["metrics"]["context_token_limit"].as_f64() {
                if lim > 0.0 {
                    st.max_prompt_tokens = lim as u64;
                }
            }
        }
        _ => {}
    }
}

fn apply_accounting(st: &mut ParsedSession, acc: &Value) {
    if acc.is_null() {
        return;
    }
    if let Some(p) = acc["totalPremiumRequestCost"].as_f64() {
        st.premium_requests = p;
    }
    if let Some(n) = acc["totalNanoAiu"].as_f64() {
        st.nano_aiu = n.max(0.0) as u64;
    }
    if let Some(r) = acc["totalUserRequests"].as_u64() {
        st.user_requests = r;
    }
    let cc = &acc["codeChanges"];
    if let Some(a) = cc["linesAdded"].as_u64() {
        st.lines_added = a;
    }
    if let Some(r) = cc["linesRemoved"].as_u64() {
        st.lines_removed = r;
    }
}

/// Short, secret-redacted argument preview for a tool call.
fn tool_arg(args: &Value) -> String {
    const KEYS: &[&str] = &[
        "path",
        "file_path",
        "filePath",
        "command",
        "pattern",
        "query",
        "url",
        "description",
        "prompt",
    ];
    for k in KEYS {
        if let Some(v) = args[*k].as_str() {
            if !v.is_empty() {
                return clean_text(v, 60);
            }
        }
    }
    String::new()
}

pub(super) fn push_bounded(v: &mut Vec<u64>, x: u64) {
    v.push(x);
    if v.len() > MAX_HISTORY {
        let excess = v.len() - MAX_HISTORY;
        v.drain(..excess);
    }
}

pub(super) fn push_chat(chat: &mut Vec<ChatMessage>, role: ChatRole, text: String) {
    chat.push(ChatMessage { role, text });
    if chat.len() > MAX_CHAT_MESSAGES {
        let excess = chat.len() - MAX_CHAT_MESSAGES;
        chat.drain(..excess);
    }
}

pub(super) fn collect_children(pid: u32, shared: &SharedProcessData) -> (Vec<ChildProcess>, u64) {
    let mut children = Vec::new();
    let mut total_kb = shared.process_info.get(&pid).map(|p| p.rss_kb).unwrap_or(0);
    let mut stack: Vec<u32> = shared.children_map.get(&pid).cloned().unwrap_or_default();
    let mut visited = HashSet::new();
    while let Some(cpid) = stack.pop() {
        if !visited.insert(cpid) {
            continue;
        }
        if let Some(cproc) = shared.process_info.get(&cpid) {
            total_kb += cproc.rss_kb;
            let port = shared.ports.get(&cpid).and_then(|v| v.first().copied());
            children.push(ChildProcess {
                pid: cpid,
                command: cproc.command.clone(),
                mem_kb: cproc.rss_kb,
                port,
            });
        }
        if let Some(grand) = shared.children_map.get(&cpid) {
            stack.extend(grand);
        }
    }
    (children, total_kb / 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_inuse_lock_names() {
        assert_eq!(parse_inuse_lock("inuse.13068.lock"), Some(13068));
        assert_eq!(parse_inuse_lock("inuse.x.lock"), None);
        assert_eq!(parse_inuse_lock(".workspace-fork.lock"), None);
    }

    #[test]
    fn parses_workspace_yaml() {
        let ws = parse_workspace_yaml(
            "id: abc\ncwd: C:\\repos\\demo\nbranch: main\nname: \"Fix login bug\"\nclient_name: github/cli\n",
        );
        assert_eq!(ws.cwd, "C:\\repos\\demo");
        assert_eq!(ws.branch, "main");
        assert_eq!(ws.name, "Fix login bug");
        assert_eq!(surface_for_client(&ws.client_name), LaunchSurface::Cli);
        assert_eq!(surface_for_client("github/autopilot"), LaunchSurface::App);
    }

    #[test]
    fn extracts_event_type_cheaply() {
        assert_eq!(
            event_type(r#"{"type":"hook.start","data":{}}"#),
            Some("hook.start")
        );
        assert_eq!(event_type(r#"{"data":{}}"#), None);
    }

    fn sample_events() -> String {
        [
            r#"{"type":"session.start","data":{"sessionId":"s1","copilotVersion":"1.0.102","startTime":"2026-11-05T14:11:20.237Z","selectedModel":"claude-haiku-5.5","reasoningEffort":"high","context":{"cwd":"C:\\repos\\demo","branch":"main"}},"timestamp":"2026-11-05T14:11:20.300Z"}"#,
            r#"{"type":"user.message","data":{"content":"fix the build ghp_secret123","turnId":"0"},"timestamp":"2026-11-05T14:11:21.000Z"}"#,
            r#"{"type":"assistant.turn_start","data":{"turnId":"0"},"timestamp":"2026-11-05T14:11:21.100Z"}"#,
            r#"{"type":"session.usage_record","data":{"usage":{"model":"claude-haiku-5.5","inputTokens":1000,"outputTokens":50,"cacheReadTokens":800,"cacheWriteTokens":100,"reasoningTokens":10,"maxPromptTokens":200000}},"timestamp":"2026-11-05T14:11:22.000Z"}"#,
            r#"{"type":"tool.execution_start","data":{"toolCallId":"t1","toolName":"powershell","arguments":{"command":"cargo build"}},"timestamp":"2026-11-05T14:11:22.100Z"}"#,
        ]
        .join("\n")
            + "\n"
    }

    #[test]
    fn incremental_parse_tracks_tokens_status_and_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("session-state").join("s1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("events.jsonl"), sample_events()).unwrap();
        fs::write(
            dir.join("workspace.yaml"),
            "cwd: C:\\repos\\demo\nname: Demo task\nclient_name: github/cli\n",
        )
        .unwrap();

        let mut c = CopilotCollector::with_root(tmp.path().to_path_buf());
        c.refresh_events(&dir);
        let p = c.parsed.get(&dir).unwrap();
        assert_eq!(p.session_id, "s1");
        assert_eq!(p.input_tokens, 100);
        assert_eq!(p.cache_read, 800);
        assert_eq!(p.cache_write, 100);
        assert_eq!(p.output_tokens, 50);
        assert_eq!(p.max_prompt_tokens, 200_000);
        assert_eq!(p.tools_in_flight.len(), 1);
        assert!(!p.initial_prompt.contains("ghp_secret123"));

        // Append completion + turn end + checkpoint, including a partial line.
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(dir.join("events.jsonl"))
            .unwrap();
        write!(
            f,
            "{}\n{}\n{}",
            r#"{"type":"tool.execution_complete","data":{"toolCallId":"t1","success":true},"timestamp":"2026-11-05T14:11:25.100Z"}"#,
            r#"{"type":"assistant.turn_end","data":{"turnId":"0"},"timestamp":"2026-11-05T14:11:26.000Z"}"#,
            r#"{"type":"session.usage_checkpoint","data":{"totalPremiumRequests":2,"totalNanoAiu":1500,"accountingSnapshot":{"totalUserRequests":1,"codeChanges":{"linesAdded":7,"linesRemoved":2}}},"timestamp":"2026-11-05T14:11:27.000Z"}"#
        )
        .unwrap();
        drop(f);
        c.refresh_events(&dir);
        let p = c.parsed.get(&dir).unwrap();
        assert!(p.tools_in_flight.is_empty());
        assert_eq!(p.tool_calls.len(), 1);
        assert_eq!(p.tool_calls[0].duration_ms, 3000);
        assert!(!p.turn_open);
        assert_eq!(
            p.premium_requests, 0.0,
            "partial line must not be applied yet"
        );

        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(dir.join("events.jsonl"))
            .unwrap();
        writeln!(f).unwrap();
        drop(f);
        c.refresh_events(&dir);
        let p = c.parsed.get(&dir).unwrap();
        assert_eq!(p.premium_requests, 2.0);
        assert_eq!(p.lines_added, 7);
    }
}
