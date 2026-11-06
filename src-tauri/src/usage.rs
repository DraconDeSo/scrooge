//! Historical consumption from local provider records. Chat text is discarded.
#[cfg(test)]
use crate::billing::ChargeKind;
use crate::{billing, billing::Charge, claude_local, settings::Settings};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LINE: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Source {
    Claude,
    Codex,
    Copilot,
    GeminiCli,
    GeminiApp,
    Antigravity,
    AntigravityIde,
    OpenCode,
    Grok,
    Billing,
    ProviderBilling,
}

impl Source {
    fn provider(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::GeminiCli | Self::GeminiApp | Self::Antigravity | Self::AntigravityIde => {
                "gemini"
            }
            Self::OpenCode => "opencode",
            Self::Grok => "grok",
            Self::Billing | Self::ProviderBilling => "",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
struct Tokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

impl Tokens {
    fn inclusive(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Self {
        Self {
            input: input.saturating_sub(cache_read.saturating_add(cache_write)),
            output,
            cache_read,
            cache_write,
        }
    }

    fn add(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
    }
}

#[derive(Debug)]
struct Record {
    id: Option<String>,
    time: u64,
    project: Option<String>,
    model: Option<String>,
    account: Option<String>,
    tokens: Tokens,
    billing: Option<(String, Charge)>,
}

#[derive(Debug, Default, Serialize)]
struct Totals {
    requests: u64,
    activities: u64,
    tokens: Tokens,
    charges: Vec<Charge>,
}

impl Totals {
    fn record(&mut self, source: Source, record: &Record) {
        if let Some((_, charge)) = &record.billing {
            self.add_charge(charge);
        } else if source == Source::GeminiApp {
            self.activities += 1;
        } else {
            self.requests += 1;
            self.tokens.add(record.tokens);
        }
    }

    fn add_charge(&mut self, charge: &Charge) {
        if let Some(sum) = self
            .charges
            .iter_mut()
            .find(|x| x.currency == charge.currency && x.kind == charge.kind)
        {
            sum.amount_micros = sum.amount_micros.saturating_add(charge.amount_micros);
        } else {
            self.charges.push(charge.clone());
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Group {
    provider: String,
    project: Option<String>,
    model: Option<String>,
    account: Option<String>,
    source: Source,
}

impl Group {
    fn from_record(
        record: &Record,
        provider: &str,
        source: Source,
        projects: &mut HashMap<String, String>,
    ) -> Self {
        Self {
            provider: provider.to_owned(),
            project: record.project.as_ref().map(|path| {
                projects
                    .entry(path.clone())
                    .or_insert_with(|| project_root(path))
                    .clone()
            }),
            model: record.model.clone(),
            account: record.account.clone(),
            source,
        }
    }
}

#[derive(Debug, Serialize)]
struct Row {
    #[serde(flatten)]
    group: Group,
    #[serde(flatten)]
    totals: Totals,
}

#[derive(Debug, Serialize)]
struct Coverage {
    source: Source,
    provider: &'static str,
    files: usize,
    earliest_ms: Option<u64>,
    records_in_period: u64,
    readable: bool,
    supported: bool,
    label: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    rows: Vec<Row>,
    coverage: Vec<Coverage>,
}

struct ParsedFile {
    modified: SystemTime,
    len: u64,
    records: Vec<Record>,
    readable: bool,
}

#[derive(Default)]
pub struct History {
    files: HashMap<PathBuf, ParsedFile>,
    billing: billing::Sync,
}

impl History {
    pub fn report(
        &mut self,
        settings: &Settings,
        start: u64,
        end: u64,
        billing_period: Option<billing::Period>,
    ) -> Report {
        let mut groups: BTreeMap<Group, Totals> = BTreeMap::new();
        let mut coverage = Vec::new();
        let mut visited = HashSet::new();
        let mut projects = HashMap::new();
        let synced = imports_dir()
            .map(|dir| {
                self.billing
                    .report(&dir.join("billing-sources.json"), settings, billing_period)
            })
            .unwrap_or_default();
        for source in [
            Source::Claude,
            Source::Codex,
            Source::Copilot,
            Source::GeminiCli,
            Source::GeminiApp,
            Source::Antigravity,
            Source::AntigravityIde,
            Source::OpenCode,
            Source::Grok,
            Source::Billing,
        ] {
            if !settings.provider_enabled(source.provider()) {
                continue;
            }
            let mut files = source_files(source);
            files.sort_unstable();
            files.dedup();
            let mut status = Coverage {
                source,
                provider: source.provider(),
                files: files.len(),
                earliest_ms: None,
                records_in_period: 0,
                readable: true,
                supported: source != Source::Grok,
                label: None,
                detail: None,
            };
            let mut seen = HashSet::new();
            let mut replaced_imports = 0;
            for path in files {
                let Ok(meta) = path.metadata() else {
                    status.readable = false;
                    continue;
                };
                let modified = meta.modified().unwrap_or(UNIX_EPOCH);
                let earliest = status.earliest_ms;
                let cached = self
                    .files
                    .get(&path)
                    .is_some_and(|c| c.modified == modified && c.len == meta.len() && c.readable);
                if !cached {
                    let (records, readable) = read_records(source, &path);
                    self.files.insert(
                        path.clone(),
                        ParsedFile {
                            modified,
                            len: meta.len(),
                            records,
                            readable,
                        },
                    );
                }
                let file = &self.files[&path];
                status.readable &= file.readable;
                status.earliest_ms = file.records.iter().map(|r| r.time).chain(earliest).min();
                for record in &file.records {
                    if record.time < start || record.time >= end {
                        continue;
                    }
                    let provider = record
                        .billing
                        .as_ref()
                        .map_or(source.provider(), |(provider, _)| provider);
                    if record.billing.as_ref().is_some_and(|(_, charge)| {
                        synced.replaces_import(provider, record.account.as_deref(), charge.kind)
                    }) {
                        replaced_imports += 1;
                        continue;
                    }
                    if !settings.provider_enabled(provider)
                        || record
                            .id
                            .as_ref()
                            .is_some_and(|id| !seen.insert(id.clone()))
                    {
                        continue;
                    }
                    groups
                        .entry(Group::from_record(record, provider, source, &mut projects))
                        .or_default()
                        .record(source, record);
                    status.records_in_period += 1;
                }
                visited.insert(path);
            }
            if source == Source::OpenCode {
                let opencode = abtop::collector::OpenCodeCollector::new();
                if let Some(records) = opencode.query_usage(start, end) {
                    for row in records {
                        let Some(record) = decode_opencode(&row) else {
                            continue;
                        };
                        status.earliest_ms = Some(
                            status
                                .earliest_ms
                                .map_or(record.time, |t| t.min(record.time)),
                        );
                        groups
                            .entry(Group::from_record(
                                &record,
                                source.provider(),
                                source,
                                &mut projects,
                            ))
                            .or_default()
                            .record(source, &record);
                        status.records_in_period += 1;
                    }
                } else {
                    status.readable = false;
                }
            }
            if replaced_imports > 0 {
                status.detail = Some(format!(
                    "{replaced_imports} imported rows superseded by connected billing"
                ));
            }
            coverage.push(status);
        }
        self.files.retain(|path, _| visited.contains(path));
        if let Some(detail) = synced.status {
            coverage.push(Coverage::billing("", None, detail.into()));
        }
        for connection in synced.connections {
            let detail = match connection.result {
                Ok(rows) => {
                    let count = rows.len();
                    for row in rows {
                        let project = row.project.map(|path| {
                            projects
                                .entry(path.clone())
                                .or_insert_with(|| project_root(&path))
                                .clone()
                        });
                        groups
                            .entry(Group {
                                provider: connection.provider.into(),
                                account: Some(connection.account.clone()),
                                project,
                                model: row.model,
                                source: Source::ProviderBilling,
                            })
                            .or_default()
                            .add_charge(&row.charge);
                    }
                    format!("{count} reported charge rows · UTC calendar dates")
                }
                Err(error) => error,
            };
            coverage.push(Coverage::billing(
                connection.provider,
                Some(format!("{} · {}", connection.label, connection.account)),
                detail,
            ));
        }
        let rows = groups
            .into_iter()
            .map(|(group, totals)| Row { group, totals })
            .collect();
        Report { rows, coverage }
    }
}

impl Coverage {
    fn billing(provider: &'static str, label: Option<String>, detail: String) -> Self {
        Self {
            source: Source::ProviderBilling,
            provider,
            supported: true,
            readable: true,
            label,
            detail: Some(detail),
            files: 0,
            earliest_ms: None,
            records_in_period: 0,
        }
    }
}

pub fn imports_dir() -> Option<PathBuf> {
    crate::settings::path()?.parent().map(|p| p.join("imports"))
}

fn source_files(source: Source) -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let gemini_home =
        std::env::var_os("GEMINI_CLI_HOME").map_or_else(|| home.clone(), PathBuf::from);
    let mut files = Vec::new();
    match source {
        Source::Claude => {
            for root in claude_local::claude_roots() {
                collect(
                    &root.join("projects"),
                    5,
                    &|name| name.ends_with(".jsonl"),
                    &mut files,
                );
            }
        }
        Source::Codex => {
            let root =
                std::env::var_os("CODEX_HOME").map_or_else(|| home.join(".codex"), PathBuf::from);
            for name in ["sessions", "archived_sessions"] {
                collect(
                    &root.join(name),
                    3,
                    &|name| name.starts_with("rollout-") && name.ends_with(".jsonl"),
                    &mut files,
                );
            }
        }
        Source::Copilot => {
            let root = std::env::var_os("COPILOT_HOME")
                .map_or_else(|| home.join(".copilot"), PathBuf::from);
            collect(
                &root.join("session-state"),
                1,
                &|name| name == "events.jsonl",
                &mut files,
            );
        }
        Source::GeminiCli => {
            collect(
                &gemini_home.join(".gemini").join("tmp"),
                2,
                &|name| name.starts_with("session-") && name.ends_with(".jsonl"),
                &mut files,
            );
        }
        Source::Antigravity | Source::AntigravityIde => {
            let root = if source == Source::Antigravity {
                "antigravity"
            } else {
                "antigravity-ide"
            };
            for base in [home.join(".gemini"), gemini_home.join(".gemini")] {
                if let Ok(sessions) = fs::read_dir(base.join(root).join("brain")) {
                    for entry in sessions
                        .flatten()
                        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    {
                        let logs = entry.path().join(".system_generated").join("logs");
                        let full = logs.join("transcript_full.jsonl");
                        let selected = if full.is_file() {
                            full
                        } else {
                            logs.join("transcript.jsonl")
                        };
                        if selected.is_file() {
                            files.push(selected);
                        }
                    }
                }
            }
        }
        Source::GeminiApp => {
            if let Some(dir) = imports_dir() {
                let file = dir.join("gemini-activity.json");
                if fs::symlink_metadata(&file).is_ok_and(|meta| meta.is_file()) {
                    files.push(file);
                }
            }
        }
        Source::Billing => {
            if let Some(dir) = imports_dir() {
                collect(&dir, 0, &|name| name.ends_with(".jsonl"), &mut files);
            }
        }
        _ => {}
    }
    files
}

fn collect(dir: &Path, depth: u8, filter: &impl Fn(&str) -> bool, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth > 0 {
            collect(&entry.path(), depth - 1, filter, files);
        } else if kind.is_file() && entry.file_name().to_str().is_some_and(filter) {
            files.push(entry.path());
        }
    }
}

#[derive(Default)]
struct FileContext {
    project: Option<String>,
    model: Option<String>,
    turn_id: Option<String>,
    codex_total: Option<Tokens>,
}

fn read_records(source: Source, path: &Path) -> (Vec<Record>, bool) {
    let Ok(file) = File::open(path) else {
        return (Vec::new(), false);
    };
    if source == Source::GeminiApp {
        // Keep only the timestamps from the Google My Activity JSON export.
        let mut input = Vec::new();
        if file
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut input)
            .is_err()
            || input.len() > 64 * 1024 * 1024
        {
            return (Vec::new(), false);
        }
        let Ok(entries) = serde_json::from_slice::<Vec<Value>>(
            input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&input),
        ) else {
            return (Vec::new(), false);
        };
        let total = entries.len();
        let records: Vec<_> = entries
            .iter()
            .filter_map(|entry| {
                Some(Record {
                    time: time(&entry["time"])?,
                    id: None,
                    project: None,
                    model: None,
                    account: None,
                    tokens: Tokens::default(),
                    billing: None,
                })
            })
            .collect();
        let readable = records.len() == total;
        return (records, readable);
    }
    let mut context = FileContext::default();
    if source == Source::GeminiCli {
        context.project = path
            .parent()
            .and_then(Path::parent)
            .and_then(|project| small_text(&project.join(".project_root")))
            .filter(|project| !project.is_empty());
    } else if source == Source::Copilot {
        context.project = path
            .parent()
            .and_then(|p| small_text(&p.join("workspace.yaml")))
            .and_then(|yaml| {
                yaml.lines().find_map(|line| {
                    line.trim()
                        .strip_prefix("cwd:")
                        .map(|v| v.trim().trim_matches('"').to_string())
                })
            });
    }
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut rows: Vec<Record> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut readable = true;
    loop {
        line.clear();
        let bytes = match (&mut reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) => break,
            Ok(n) => n as u64,
            Err(_) => {
                readable = false;
                break;
            }
        };
        if bytes > MAX_LINE {
            readable = false;
            if !line.ends_with(b"\n") && reader.skip_until(b'\n').is_err() {
                break;
            }
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(event) = serde_json::from_slice::<Value>(&line) else {
            // An append-only log can be observed while its last line is being written.
            readable &= !line.ends_with(b"\n");
            continue;
        };
        if source == Source::GeminiCli {
            if let Some(messages) = event["$set"]["messages"].as_array() {
                for message in messages {
                    if let Some(record) = decode(source, message, &mut context) {
                        insert_latest(&mut rows, &mut by_id, record);
                    }
                }
            }
        }
        if let Some(record) = decode(source, &event, &mut context) {
            if matches!(source, Source::GeminiCli | Source::Claude) {
                insert_latest(&mut rows, &mut by_id, record);
            } else {
                rows.push(record);
            }
        } else if source == Source::Billing {
            readable = false;
        }
    }
    (rows, readable)
}

fn insert_latest(rows: &mut Vec<Record>, by_id: &mut HashMap<String, usize>, mut record: Record) {
    if let Some(id) = &record.id {
        if let Some(&index) = by_id.get(id) {
            record.time = rows[index].time.min(record.time);
            rows[index] = record;
            return;
        }
        by_id.insert(id.clone(), rows.len());
    }
    rows.push(record);
}

fn str_field(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .map(str::to_string)
}

fn time(value: &Value) -> Option<u64> {
    let date = chrono::DateTime::parse_from_rfc3339(value.as_str()?).ok()?;
    u64::try_from(date.timestamp_millis()).ok()
}

fn tokens(value: &Value, field: &str) -> u64 {
    value[field].as_u64().unwrap_or(0)
}

fn decode_opencode(row: &Value) -> Option<Record> {
    let model = str_field(&row["model"]).map(|model| {
        str_field(&row["provider"])
            .map_or_else(|| model.clone(), |provider| format!("{provider}/{model}"))
    });
    Some(Record {
        time: row["time_ms"].as_u64()?,
        project: str_field(&row["cwd"]),
        model,
        id: None,
        account: None,
        tokens: Tokens {
            input: tokens(row, "input"),
            output: tokens(row, "output"),
            cache_read: tokens(row, "cache_read"),
            cache_write: tokens(row, "cache_write"),
        },
        billing: None,
    })
}

fn codex_tokens(value: &Value) -> Tokens {
    Tokens::inclusive(
        tokens(value, "input_tokens"),
        tokens(value, "output_tokens"),
        value["cached_input_tokens"]
            .as_u64()
            .unwrap_or_else(|| tokens(value, "cache_read_input_tokens")),
        0,
    )
}

fn decode(source: Source, value: &Value, context: &mut FileContext) -> Option<Record> {
    let ty = value["type"].as_str().unwrap_or("");
    let mut time = time(&value["timestamp"]);
    let mut project = context.project.clone();
    let mut model = context.model.clone();
    let mut account = None;
    let mut id = None;
    let mut billing = None;
    let counts = match source {
        Source::Claude if ty == "assistant" => {
            let usage = &value["message"]["usage"];
            model = str_field(&value["message"]["model"]).filter(|m| m != "<synthetic>");
            if value["message"]["model"] == "<synthetic>" || !usage.is_object() {
                return None;
            }
            project = str_field(&value["cwd"]);
            id = str_field(&value["requestId"]).or_else(|| str_field(&value["message"]["id"]));
            Tokens {
                input: tokens(usage, "input_tokens"),
                output: tokens(usage, "output_tokens"),
                cache_read: tokens(usage, "cache_read_input_tokens"),
                cache_write: tokens(usage, "cache_creation_input_tokens"),
            }
        }
        Source::Codex => {
            let payload = &value["payload"];
            if matches!(ty, "session_meta" | "turn_context") {
                context.project = str_field(&payload["cwd"]).or(context.project.take());
                if ty == "turn_context" {
                    context.model = str_field(&payload["model"]);
                    context.turn_id = str_field(&payload["turn_id"]);
                }
                return None;
            }
            if ty != "event_msg" || payload["type"] != "token_count" {
                return None;
            }
            let info = &payload["info"];
            if !info["total_token_usage"].is_object() || !info["last_token_usage"].is_object() {
                return None;
            }
            let total = codex_tokens(&info["total_token_usage"]);
            if context.codex_total.replace(total) == Some(total) {
                return None;
            }
            let last = codex_tokens(&info["last_token_usage"]);
            if let Some(turn) = &context.turn_id {
                id = Some(format!(
                    "{turn}:{}:{}:{}:{}",
                    time?, total.input, total.output, total.cache_read
                ));
            }
            last
        }
        Source::Copilot => {
            let data = &value["data"];
            if matches!(ty, "session.start" | "session.resume") {
                context.project = str_field(&data["context"]["cwd"]).or(context.project.take());
                context.model = str_field(&data["selectedModel"]).or(context.model.take());
                return None;
            }
            if ty != "session.usage_record" {
                return None;
            }
            let usage = &data["usage"];
            model = str_field(&usage["model"]).or(model);
            id = str_field(&value["id"]);
            account = str_field(&data["accountLogin"]);
            Tokens::inclusive(
                tokens(usage, "inputTokens"),
                tokens(usage, "outputTokens"),
                tokens(usage, "cacheReadTokens"),
                tokens(usage, "cacheWriteTokens"),
            )
        }
        Source::GeminiCli if ty == "gemini" => {
            let usage = &value["tokens"];
            model = str_field(&value["model"]);
            id = str_field(&value["id"]);
            Tokens::inclusive(
                tokens(usage, "input"),
                tokens(usage, "output").saturating_add(tokens(usage, "thoughts")),
                tokens(usage, "cached"),
                0,
            )
        }
        Source::Antigravity | Source::AntigravityIde
            if ty == "PLANNER_RESPONSE" && value["status"] == "DONE" =>
        {
            time = self::time(&value["created_at"]);
            project = str_field(&value["cwd"]);
            model = str_field(&value["model"]);
            Tokens::inclusive(
                tokens(value, "input_tokens"),
                tokens(value, "output_tokens"),
                tokens(value, "cache_read_tokens"),
                0,
            )
        }
        Source::Billing => {
            let provider = str_field(&value["provider"])?;
            if !matches!(
                provider.as_str(),
                "claude" | "codex" | "copilot" | "gemini" | "opencode" | "grok"
            ) {
                return None;
            }
            let charge: Charge = serde_json::from_value(value.clone()).ok()?;
            if charge.currency.len() != 3
                || !charge.currency.bytes().all(|b| b.is_ascii_uppercase())
            {
                return None;
            }
            id = Some(format!(
                "{provider}:{}:{}",
                value["account"].as_str().unwrap_or(""),
                str_field(&value["id"])?
            ));
            project = str_field(&value["project"]);
            model = str_field(&value["model"]);
            account = str_field(&value["account"]);
            billing = Some((provider, charge));
            Tokens::default()
        }
        _ => return None,
    };
    if billing.is_none() && counts == Tokens::default() {
        return None;
    }
    Some(Record {
        id,
        time: time?,
        project,
        model,
        account,
        tokens: counts,
        billing,
    })
}

fn small_text(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut text = String::new();
    file.take(8193).read_to_string(&mut text).ok()?;
    (text.len() <= 8192).then(|| text.trim().to_string())
}

/// Resolve Git worktrees via the common repository directory without running hooks.
fn project_root(project: &str) -> String {
    let path = Path::new(project);
    #[cfg(windows)]
    if project.starts_with(r"\\") || project.starts_with("//") {
        return project.to_string();
    }
    if !path.is_absolute() {
        return project.to_string();
    }
    for root in path.ancestors() {
        let git = root.join(".git");
        if git.is_dir() {
            return project_name(root);
        }
        if let Some(pointer) = small_text(&git) {
            if let Some(target) = pointer.strip_prefix("gitdir: ") {
                let gitdir = root.join(target);
                if let Some(common) = small_text(&gitdir.join("commondir")) {
                    if let Ok(dir) = gitdir.join(common).canonicalize() {
                        if dir.file_name().is_some_and(|name| name == ".git") {
                            return project_name(dir.parent().unwrap_or(&dir));
                        }
                        return project_name(&dir);
                    }
                }
            }
            return project_name(root);
        }
    }
    project_name(path)
}

fn project_name(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let name = canonical.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc) = name.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        return name.strip_prefix(r"\\?\").unwrap_or(&name).to_string();
    }
    #[cfg(not(windows))]
    name.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn linked_worktree_and_checkout_share_the_same_project() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repository");
        let worktree = temp.path().join("copilot-task");
        let gitdir = project.join(".git").join("worktrees").join("copilot-task");
        fs::create_dir_all(&gitdir).unwrap();
        fs::create_dir_all(worktree.join("src")).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .unwrap();
        fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        assert_eq!(
            project_root(&worktree.join("src").to_string_lossy()),
            project_root(&project.to_string_lossy())
        );
    }

    #[test]
    fn gemini_revisions_keep_first_timestamp_and_final_counts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        fs::write(&path, concat!(
            r#"{"id":"g1","type":"gemini","model":"gemini-pro","timestamp":"2026-11-06T08:00:00Z","tokens":{"input":60,"cached":50,"output":2}}"#,
            "\n",
            r#"{"id":"g1","type":"gemini","model":"gemini-pro","timestamp":"2026-11-06T08:00:02Z","tokens":{"input":60,"cached":50,"output":20,"thoughts":8}}"#,
            "\n"
        )).unwrap();
        let (rows, readable) = read_records(Source::GeminiCli, &path);
        assert!(readable);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].time, time(&json!("2026-11-06T08:00:00Z")).unwrap());
        assert_eq!(rows[0].tokens, Tokens::inclusive(60, 28, 50, 0));
    }

    #[test]
    fn antigravity_records_usage_even_if_model_is_missing() {
        let event = json!({
            "type": "PLANNER_RESPONSE", "status": "DONE", "created_at": "2026-11-06T08:00:00Z",
            "input_tokens": 120, "output_tokens": 20, "cache_read_tokens": 90
        });
        let record = decode(Source::Antigravity, &event, &mut FileContext::default()).unwrap();
        assert_eq!(record.model, None);
        assert_eq!(record.tokens, Tokens::inclusive(120, 20, 90, 0));
    }

    #[test]
    fn imported_plan_overage_does_not_become_api_cost_or_tokens() {
        let event = json!({
            "provider": "copilot", "id": "charge", "timestamp": "2026-11-06T08:00:00Z",
            "kind": "plan_extra", "amount_micros": 123456, "currency": "USD"
        });
        let record = decode(Source::Billing, &event, &mut FileContext::default()).unwrap();
        let mut totals = Totals::default();
        totals.record(Source::Billing, &record);
        assert_eq!(totals.requests, 0);
        assert_eq!(totals.tokens, Tokens::default());
        assert_eq!(totals.charges[0].kind, ChargeKind::PlanExtra);
        assert_eq!(totals.charges[0].amount_micros, 123456);
    }
}
