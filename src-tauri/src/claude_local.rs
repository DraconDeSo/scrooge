//! Local Claude consumption, computed from Claude Code transcripts.
//!
//! Works without any login: scans `<root>/projects/**/*.jsonl` (roots:
//! `CLAUDE_CONFIG_DIR`, `~/.claude`, `~/.claude-*` with a `projects/` dir) for
//! `assistant` lines and sums `message.usage` per time window. Files are read
//! incrementally (byte offsets), only files touched in the last 7 days are
//! considered, and streamed duplicates of the same API response are counted
//! once (key = `message.id` + `requestId`).

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const WEEK_MS: u64 = 7 * 24 * 3600 * 1000;
const FIVE_H_MS: u64 = 5 * 3600 * 1000;
const DAY_MS: u64 = 24 * 3600 * 1000;

#[derive(Clone, Copy)]
struct Event {
    ts_ms: u64,
    model: u16,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct WindowTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
    pub messages: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ModelTotals {
    pub model: String,
    pub total: u64,
    pub output: u64,
    pub messages: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ClaudeLocal {
    pub five_hour: WindowTotals,
    pub day: WindowTotals,
    pub week: WindowTotals,
    /// Start of the current 5-hour usage block (first message after a gap
    /// of >= 5h), epoch seconds; `None` when idle for 5h.
    pub block_start: Option<u64>,
    /// Token totals per hour for the last 24 hours, oldest first.
    pub hourly: Vec<u64>,
    /// Last 7 days by model, largest first.
    pub models: Vec<ModelTotals>,
    pub roots: Vec<String>,
    pub files_scanned: usize,
    pub computed_at_ms: u64,
}

struct FileState {
    offset: u64,
    partial: String,
}

pub struct ClaudeLocalScanner {
    files: HashMap<PathBuf, FileState>,
    events: Vec<Event>,
    seen: HashSet<String>,
    models: Vec<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn claude_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        roots.push(PathBuf::from(d));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".claude"));
        if let Ok(entries) = fs::read_dir(&home) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(".claude-") && e.path().join("projects").is_dir() {
                    roots.push(e.path());
                }
            }
        }
    }
    let mut seen = HashSet::new();
    roots.retain(|r| r.join("projects").is_dir() && seen.insert(r.clone()));
    roots
}

fn collect_jsonl(dir: &Path, cutoff: SystemTime, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            collect_jsonl(&p, cutoff, out, depth + 1);
        } else if ft.is_file() && p.extension().is_some_and(|x| x == "jsonl") {
            let fresh = e
                .metadata()
                .and_then(|m| m.modified())
                .map(|m| m >= cutoff)
                .unwrap_or(false);
            if fresh {
                out.push(p);
            }
        }
    }
}

impl Default for ClaudeLocalScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeLocalScanner {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            events: Vec::new(),
            seen: HashSet::new(),
            models: Vec::new(),
        }
    }

    fn model_idx(&mut self, m: &str) -> u16 {
        if let Some(i) = self.models.iter().position(|x| x == m) {
            return i as u16;
        }
        self.models.push(m.to_string());
        (self.models.len() - 1) as u16
    }

    fn apply_line(&mut self, line: &str, min_ts: u64) {
        // Cheap prefilter before JSON parsing.
        if !line.contains("\"type\":\"assistant\"") || !line.contains("\"usage\"") {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if v["type"] != "assistant" {
            return;
        }
        let msg = &v["message"];
        let usage = &msg["usage"];
        if !usage.is_object() {
            return;
        }
        let ts_ms = v["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis().max(0) as u64)
            .unwrap_or(0);
        if ts_ms < min_ts {
            return;
        }
        let key = format!(
            "{}:{}",
            msg["id"].as_str().unwrap_or(""),
            v["requestId"].as_str().unwrap_or("")
        );
        if key != ":" && !self.seen.insert(key) {
            return;
        }
        let model = msg["model"].as_str().unwrap_or("unknown");
        if model == "<synthetic>" {
            return;
        }
        let model = self.model_idx(model);
        let g = |k: &str| usage[k].as_u64().unwrap_or(0);
        self.events.push(Event {
            ts_ms,
            model,
            input: g("input_tokens"),
            output: g("output_tokens"),
            cache_read: g("cache_read_input_tokens"),
            cache_write: g("cache_creation_input_tokens"),
        });
    }

    fn read_file(&mut self, path: &Path, min_ts: u64) {
        let Ok(meta) = fs::metadata(path) else { return };
        let len = meta.len();
        let (offset, partial) = match self.files.get(path) {
            Some(st) if len >= st.offset => (st.offset, st.partial.clone()),
            _ => (0, String::new()),
        };
        if len == offset {
            return;
        }
        let Ok(mut f) = fs::File::open(path) else {
            return;
        };
        if f.seek(SeekFrom::Start(offset)).is_err() {
            return;
        }
        let mut buf = Vec::with_capacity((len - offset) as usize);
        if f.take(len - offset).read_to_end(&mut buf).is_err() {
            return;
        }
        let mut text = partial;
        text.push_str(&String::from_utf8_lossy(&buf));
        let complete = text.ends_with('\n');
        let mut lines: Vec<&str> = text.split('\n').collect();
        let rest = if complete {
            String::new()
        } else {
            lines.pop().unwrap_or("").to_string()
        };
        let owned: Vec<String> = lines
            .iter()
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect();
        for l in owned {
            if !l.is_empty() {
                self.apply_line(&l, min_ts);
            }
        }
        self.files.insert(
            path.to_path_buf(),
            FileState {
                offset: offset + buf.len() as u64,
                partial: rest,
            },
        );
    }

    /// Incrementally scan transcripts and return fresh totals.
    pub fn scan(&mut self) -> ClaudeLocal {
        let now = now_ms();
        let min_ts = now.saturating_sub(WEEK_MS);
        let cutoff = SystemTime::now() - Duration::from_millis(WEEK_MS);
        let roots = claude_roots();
        let mut files = Vec::new();
        for r in &roots {
            collect_jsonl(&r.join("projects"), cutoff, &mut files, 0);
        }
        for p in &files {
            self.read_file(p, min_ts);
        }
        // Forget files that left the window; drop events older than 7 days.
        let live: HashSet<&PathBuf> = files.iter().collect();
        self.files.retain(|p, _| live.contains(p));
        self.events.retain(|e| e.ts_ms >= min_ts);
        self.events.sort_by_key(|e| e.ts_ms);

        let mut out = ClaudeLocal {
            roots: roots
                .iter()
                .map(|r| r.to_string_lossy().into_owned())
                .collect(),
            files_scanned: files.len(),
            computed_at_ms: now,
            hourly: vec![0; 24],
            ..Default::default()
        };
        let mut per_model: HashMap<u16, ModelTotals> = HashMap::new();
        for e in &self.events {
            let total = e.input + e.output + e.cache_read + e.cache_write;
            let add = |w: &mut WindowTotals| {
                w.input += e.input;
                w.output += e.output;
                w.cache_read += e.cache_read;
                w.cache_write += e.cache_write;
                w.total += total;
                w.messages += 1;
            };
            add(&mut out.week);
            let age = now.saturating_sub(e.ts_ms);
            if age < DAY_MS {
                add(&mut out.day);
                let hour = (age / 3_600_000) as usize; // 0 = current hour
                if hour < 24 {
                    out.hourly[23 - hour] += total;
                }
            }
            if age < FIVE_H_MS {
                add(&mut out.five_hour);
            }
            let m = per_model.entry(e.model).or_insert_with(|| ModelTotals {
                model: self.models[e.model as usize].clone(),
                ..Default::default()
            });
            m.total += total;
            m.output += e.output;
            m.messages += 1;
        }
        // Current 5h block: walk back while gaps are < 5h.
        if let Some(last) = self.events.last() {
            if now.saturating_sub(last.ts_ms) < FIVE_H_MS {
                let mut start = last.ts_ms;
                for w in self.events.windows(2).rev() {
                    if w[1].ts_ms.saturating_sub(w[0].ts_ms) >= FIVE_H_MS {
                        break;
                    }
                    start = w[0].ts_ms;
                }
                // Blocks are 5h long: if the chain is longer, the current block
                // started at the last 5h boundary of that chain.
                let elapsed = now.saturating_sub(start);
                let block = start + (elapsed / FIVE_H_MS) * FIVE_H_MS;
                out.block_start = Some(block / 1000);
            }
        }
        let mut models: Vec<ModelTotals> = per_model.into_values().collect();
        models.sort_by(|a, b| b.total.cmp(&a.total));
        out.models = models;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ts: &str, id: &str, req: &str, out: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{req}","timestamp":"{ts}","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":10,"output_tokens":{out},"cache_read_input_tokens":100,"cache_creation_input_tokens":5}}}}}}"#
        )
    }

    #[test]
    fn dedupes_streamed_lines_and_sums_windows() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.jsonl");
        let now = chrono::Utc::now();
        let recent = (now - chrono::Duration::minutes(30)).to_rfc3339();
        let old = (now - chrono::Duration::hours(30)).to_rfc3339();
        let body = [
            line(&recent, "m1", "r1", 50),
            line(&recent, "m1", "r1", 50), // streamed duplicate
            line(&old, "m2", "r2", 7),
            r#"{"type":"user","message":{"content":"hi"}}"#.to_string(),
        ]
        .join("\n")
            + "\n";
        fs::write(&p, body).unwrap();
        let mut s = ClaudeLocalScanner::new();
        let min = now_ms() - WEEK_MS;
        s.read_file(&p, min);
        assert_eq!(s.events.len(), 2);
        s.events.sort_by_key(|e| e.ts_ms);
        let n = now_ms();
        let five: u64 = s
            .events
            .iter()
            .filter(|e| n - e.ts_ms < FIVE_H_MS)
            .map(|e| e.output)
            .sum();
        assert_eq!(five, 50);
    }
}
