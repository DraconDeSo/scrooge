//! Google Gemini CLI session collector.
//!
//! Layout (`@google/gemini-cli`, JSONL recorder):
//!
//! ```text
//! ~/.gemini/tmp/<project>/.project_root            working directory of the project
//! ~/.gemini/tmp/<project>/chats/session-<ts>-<id8>.jsonl
//! ```
//!
//! Line kinds: a header (`sessionId`, `projectHash`, `startTime`), `$set`
//! metadata lines (may carry a `messages` array), `$rewindTo` lines, and
//! message lines (`id`, `timestamp`, `type` = `user` | `gemini` | `info` |
//! `error`, `content`, `model`, `tokens` {input, cached, output, thoughts,
//! tool, total}, `toolCalls` [{id, name, status}]). A message can be appended
//! again when updated: the latest line for an `id` wins.
//!
//! The CLI keeps no PID registry, so live sessions are matched heuristically:
//! with N running `gemini` processes, the N most recently written chats
//! (touched in the last 24h) are shown. `GEMINI_CLI_HOME` overrides the home.
//! Legacy single-document `session-*.json` files are not read.

use super::copilot::{clean_text, collect_children, push_bounded, push_chat};
use super::{process, AgentCollector, SharedProcessData};
use crate::model::{AgentSession, ChatMessage, ChatRole, LaunchSurface, SessionStatus, ToolCall};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Gemini 2.x / 3.x models expose a 1M-token context window.
const GEMINI_CONTEXT: u64 = 1_048_576;
const RECENT: Duration = Duration::from_secs(24 * 3600);
const MAX_READ_PER_TICK: u64 = 64 * 1024 * 1024;

#[derive(Default, Clone, Copy)]
struct Tok {
    input: u64,
    cached: u64,
    output: u64,
    thoughts: u64,
    tool: u64,
}

#[derive(Default)]
struct ParsedChat {
    offset: u64,
    partial: String,
    session_id: String,
    started_at_ms: u64,
    model: String,
    /// message id -> tokens (latest line wins)
    tokens: BTreeMap<String, Tok>,
    reply_order: Vec<String>,
    /// reply ids whose text is already in the chat tail
    seen_replies: HashSet<String>,
    user_ids: HashSet<String>,
    last_type: String,
    pending_tools: Vec<(String, String)>,
    tool_ids: HashSet<String>,
    tool_calls: Vec<ToolCall>,
    initial_prompt: String,
    first_reply: String,
    chat: Vec<ChatMessage>,
    last_prompt_tokens: u64,
    token_history: Vec<u64>,
}

pub struct GeminiCollector {
    root: PathBuf,
    parsed: HashMap<PathBuf, ParsedChat>,
    candidates: Vec<PathBuf>,
    last_pids: Vec<u32>,
}

impl Default for GeminiCollector {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn is_gemini_command(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    if lower.contains("antigravity") {
        return false;
    }
    process::cmd_has_binary(cmd, "gemini")
        || lower.contains("@google/gemini-cli")
        || lower.contains("@google\\gemini-cli")
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn ts_ms(v: &Value) -> u64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis().max(0) as u64)
        .unwrap_or(0)
}

impl GeminiCollector {
    pub fn new() -> Self {
        let home = std::env::var_os("GEMINI_CLI_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
        Self::with_root(home.join(".gemini"))
    }

    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            parsed: HashMap::new(),
            candidates: Vec::new(),
            last_pids: Vec::new(),
        }
    }

    /// Recently written chat files, newest first.
    fn scan_chats(&self) -> Vec<PathBuf> {
        let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
        let Ok(projects) = fs::read_dir(self.root.join("tmp")) else {
            return Vec::new();
        };
        let cutoff = SystemTime::now() - RECENT;
        for p in projects.flatten() {
            let Ok(chats) = fs::read_dir(p.path().join("chats")) else {
                continue;
            };
            for c in chats.flatten() {
                let path = c.path();
                let is_session = path.extension().is_some_and(|e| e == "jsonl")
                    && path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("session-"));
                if !is_session {
                    continue;
                }
                if let Ok(m) = c.metadata().and_then(|m| m.modified()) {
                    if m >= cutoff {
                        found.push((m, path));
                    }
                }
            }
        }
        found.sort_by_key(|a| std::cmp::Reverse(a.0));
        found.into_iter().map(|(_, p)| p).collect()
    }

    fn chat_cwd(path: &Path) -> String {
        path.parent()
            .and_then(|c| c.parent())
            .and_then(|d| fs::read_to_string(d.join(".project_root")).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    fn refresh(&mut self, path: &Path) {
        let Ok(meta) = fs::metadata(path) else { return };
        let len = meta.len();
        let st = self.parsed.entry(path.to_path_buf()).or_default();
        if len < st.offset {
            *st = ParsedChat::default();
        }
        if len == st.offset {
            return;
        }
        let Ok(mut f) = fs::File::open(path) else {
            return;
        };
        if f.seek(SeekFrom::Start(st.offset)).is_err() {
            return;
        }
        let to_read = (len - st.offset).min(MAX_READ_PER_TICK);
        let mut buf = Vec::with_capacity(to_read as usize);
        if f.take(to_read).read_to_end(&mut buf).is_err() {
            return;
        }
        st.offset += buf.len() as u64;
        let mut text = std::mem::take(&mut st.partial);
        text.push_str(&String::from_utf8_lossy(&buf));
        let complete = text.ends_with('\n');
        let mut lines: Vec<&str> = text.split('\n').collect();
        if !complete {
            st.partial = lines.pop().unwrap_or("").to_string();
        }
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                apply(st, &v);
            }
        }
    }
}

fn apply(st: &mut ParsedChat, v: &Value) {
    if let Some(set) = v.get("$set") {
        if let Some(msgs) = set["messages"].as_array() {
            for m in msgs {
                apply_message(st, m);
            }
        }
        return;
    }
    if v.get("$rewindTo").is_some() {
        st.pending_tools.clear();
        st.last_type = "gemini".into();
        return;
    }
    if v.get("id").is_none() {
        // header
        if let Some(sid) = v["sessionId"].as_str() {
            st.session_id = sid.to_string();
        }
        let t = ts_ms(&v["startTime"]);
        if t > 0 {
            st.started_at_ms = t;
        }
        return;
    }
    apply_message(st, v);
}

fn apply_message(st: &mut ParsedChat, m: &Value) {
    let Some(id) = m["id"].as_str() else { return };
    let ty = m["type"].as_str().unwrap_or("");
    if st.started_at_ms == 0 {
        st.started_at_ms = ts_ms(&m["timestamp"]);
    }
    match ty {
        "user" => {
            let text = clean_text(&text_of(&m["content"]), 200);
            st.last_type = "user".into();
            if st.user_ids.insert(id.to_string()) && !text.is_empty() {
                if st.initial_prompt.is_empty() {
                    st.initial_prompt = clean_text(&text, 80);
                }
                push_chat(&mut st.chat, ChatRole::User, text);
            }
        }
        "gemini" => {
            st.last_type = "gemini".into();
            if let Some(model) = m["model"].as_str() {
                st.model = model.to_string();
            }
            let t = &m["tokens"];
            if t.is_object() {
                let g = |k: &str| t[k].as_u64().unwrap_or(0);
                let tok = Tok {
                    input: g("input"),
                    cached: g("cached"),
                    output: g("output"),
                    thoughts: g("thoughts"),
                    tool: g("tool"),
                };
                let first = !st.tokens.contains_key(id);
                st.tokens.insert(id.to_string(), tok);
                if first {
                    st.reply_order.push(id.to_string());
                    st.last_prompt_tokens = tok.input;
                    push_bounded(&mut st.token_history, tok.input + tok.output + tok.thoughts);
                }
            }
            if st.seen_replies.insert(id.to_string()) {
                let text = clean_text(&text_of(&m["content"]), 200);
                if !text.is_empty() {
                    if st.first_reply.is_empty() {
                        st.first_reply = clean_text(&text, 80);
                    }
                    push_chat(&mut st.chat, ChatRole::Assistant, text);
                }
            }
            st.pending_tools.clear();
            for tc in m["toolCalls"].as_array().into_iter().flatten() {
                let tid = tc["id"].as_str().unwrap_or("").to_string();
                let name = tc["name"].as_str().unwrap_or("tool").to_string();
                let status = tc["status"].as_str().unwrap_or("");
                let done = matches!(status, "success" | "error" | "cancelled");
                if done {
                    if st.tool_ids.insert(tid) {
                        let arg = tc["args"]
                            .as_object()
                            .and_then(|a| {
                                [
                                    "file_path",
                                    "path",
                                    "absolute_path",
                                    "command",
                                    "pattern",
                                    "query",
                                    "url",
                                ]
                                .iter()
                                .find_map(|k| a.get(*k).and_then(|x| x.as_str()))
                            })
                            .map(|s| clean_text(s, 60))
                            .unwrap_or_default();
                        st.tool_calls.push(ToolCall {
                            name,
                            arg,
                            duration_ms: 0,
                        });
                        if st.tool_calls.len() > 200 {
                            st.tool_calls.remove(0);
                        }
                    }
                } else {
                    st.pending_tools.push((tid, name));
                }
            }
        }
        _ => {}
    }
}

impl AgentCollector for GeminiCollector {
    fn collect(&mut self, shared: &SharedProcessData) -> Vec<AgentSession> {
        let mut pids: Vec<u32> = shared
            .process_info
            .iter()
            .filter(|(_, p)| is_gemini_command(&p.command))
            .map(|(pid, _)| *pid)
            .collect();
        // node wrappers: keep only the outermost gemini process of a tree
        let set: HashSet<u32> = pids.iter().copied().collect();
        pids.retain(|pid| {
            shared
                .process_info
                .get(pid)
                .is_none_or(|p| !set.contains(&p.ppid))
        });
        pids.sort_unstable();
        if pids.is_empty() {
            self.parsed.clear();
            self.candidates.clear();
            self.last_pids.clear();
            return Vec::new();
        }
        if shared.slow_tick || pids != self.last_pids || self.candidates.is_empty() {
            self.candidates = self.scan_chats();
            self.last_pids = pids.clone();
        }
        let pairs =
            process::match_pids_by_cwd(&pids, &self.candidates, &shared.process_info, |p| {
                Self::chat_cwd(p)
            });
        let mut sessions = Vec::new();
        for (path, pid) in &pairs {
            self.refresh(path);
            let Some(st) = self.parsed.get(path) else {
                continue;
            };
            let project_dir = path.parent().and_then(|c| c.parent());
            let cwd = Self::chat_cwd(path);
            let project_name = process::last_path_segment(&cwd)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    project_dir
                        .and_then(|d| d.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "?".into());
            let fresh = fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|e| e < Duration::from_secs(120));
            let status = if !st.pending_tools.is_empty() {
                SessionStatus::Executing
            } else if st.last_type == "user" && fresh {
                SessionStatus::Thinking
            } else {
                SessionStatus::Waiting
            };
            let current_tasks = match status {
                SessionStatus::Executing => {
                    st.pending_tools.iter().map(|(_, n)| n.clone()).collect()
                }
                SessionStatus::Thinking => vec!["thinking...".into()],
                _ => vec!["waiting for input".into()],
            };
            let (mut input, mut cache_read, mut output) = (0u64, 0u64, 0u64);
            for t in st.tokens.values() {
                input += t.input.saturating_sub(t.cached) + t.tool;
                cache_read += t.cached;
                output += t.output + t.thoughts;
            }
            let (children, mem_mb) = collect_children(*pid, shared);
            sessions.push(AgentSession {
                agent_cli: "gemini",
                launch_surface: LaunchSurface::Cli,
                pid: *pid,
                session_id: if st.session_id.is_empty() {
                    path.file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                } else {
                    st.session_id.clone()
                },
                cwd,
                project_name,
                started_at: st.started_at_ms,
                status,
                model: if st.model.is_empty() {
                    "-".into()
                } else {
                    st.model.clone()
                },
                effort: String::new(),
                context_percent: (st.last_prompt_tokens as f64 / GEMINI_CONTEXT as f64 * 100.0)
                    .min(100.0),
                total_input_tokens: input,
                total_output_tokens: output,
                total_cache_read: cache_read,
                total_cache_create: 0,
                turn_count: st.user_ids.len() as u32,
                current_tasks,
                mem_mb,
                version: String::new(),
                git_branch: String::new(),
                git_added: 0,
                git_modified: 0,
                token_history: st.token_history.clone(),
                context_history: vec![],
                compaction_count: 0,
                context_window: GEMINI_CONTEXT,
                subagents: vec![],
                mem_file_count: 0,
                mem_line_count: 0,
                children,
                initial_prompt: st.initial_prompt.clone(),
                first_assistant_text: st.first_reply.clone(),
                chat_messages: st.chat.clone(),
                tool_calls: st.tool_calls.clone(),
                pending_since_ms: 0,
                thinking_since_ms: 0,
                file_accesses: vec![],
                copilot: None,
                config_root: super::abbrev_path(&self.root),
            });
        }
        let keep: HashSet<&PathBuf> = pairs.iter().map(|(p, _)| p).collect();
        self.parsed.retain(|p, _| keep.contains(p));
        sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_gemini_processes_but_not_antigravity() {
        assert!(is_gemini_command(
            r"C:\Program Files\nodejs\node.exe C:\Users\me\AppData\Roaming\npm\node_modules\@google\gemini-cli\dist\index.js"
        ));
        assert!(is_gemini_command("gemini --yolo"));
        assert!(!is_gemini_command(
            r"C:\Users\me\AppData\Local\Programs\Antigravity\Antigravity.exe --gemini"
        ));
    }

    #[test]
    fn parses_jsonl_with_rewritten_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("tmp").join("demo");
        fs::create_dir_all(proj.join("chats")).unwrap();
        fs::write(proj.join(".project_root"), "C:\\repos\\demo").unwrap();
        let file = proj
            .join("chats")
            .join("session-2026-11-06T10-00-abcd1234.jsonl");
        let lines = [
            r#"{"sessionId":"s-1","projectHash":"h","startTime":"2026-11-06T10:00:00Z","kind":"main"}"#,
            r#"{"id":"u1","timestamp":"2026-11-06T10:00:01Z","type":"user","content":[{"text":"fix the tests"}]}"#,
            r#"{"id":"g1","timestamp":"2026-11-06T10:00:05Z","type":"gemini","model":"gemini-3-pro","content":"On it","tokens":{"input":1000,"cached":600,"output":50,"thoughts":20,"tool":0,"total":1070},"toolCalls":[{"id":"t1","name":"run_shell_command","status":"executing","args":{"command":"npm test"}}]}"#,
            r#"{"id":"g1","timestamp":"2026-11-06T10:00:09Z","type":"gemini","model":"gemini-3-pro","content":"On it","tokens":{"input":1000,"cached":600,"output":50,"thoughts":20,"tool":0,"total":1070},"toolCalls":[{"id":"t1","name":"run_shell_command","status":"success","args":{"command":"npm test"}}]}"#,
        ]
        .join("\n")
            + "\n";
        fs::write(&file, lines).unwrap();
        let mut c = GeminiCollector::with_root(tmp.path().to_path_buf());
        c.refresh(&file);
        let st = c.parsed.get(&file).unwrap();
        assert_eq!(st.session_id, "s-1");
        assert_eq!(st.tokens.len(), 1, "rewritten message counted once");
        assert_eq!(st.tool_calls.len(), 1);
        assert!(st.pending_tools.is_empty());
        assert_eq!(st.model, "gemini-3-pro");
        assert_eq!(st.initial_prompt, "fix the tests");
        assert_eq!(
            st.chat
                .iter()
                .filter(|m| m.role == ChatRole::Assistant)
                .count(),
            1
        );
        assert_eq!(c.scan_chats().len(), 1);
    }
}
