//! Disk space used by each provider, per storage location.
//!
//! Every provider keeps data in several places (CLI data, session logs,
//! caches, desktop-app data, bundled runtimes). This module lists the known
//! locations for the current OS, keeps the ones that exist and measures them.
//! Symlinks are not followed; unreadable entries are skipped.

use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Stop measuring a single location after this long (huge caches).
const LOCATION_BUDGET: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Serialize)]
pub struct Location {
    /// What lives there, e.g. "Sessions & settings".
    pub label: String,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    /// True when the walk hit the time budget (size is a lower bound).
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderStorage {
    pub provider: String,
    pub total_bytes: u64,
    pub locations: Vec<Location>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct StorageReport {
    pub providers: Vec<ProviderStorage>,
    pub computed_at_ms: u64,
    pub duration_ms: u64,
}

struct Candidate {
    provider: &'static str,
    label: &'static str,
    path: PathBuf,
}

fn candidates() -> Vec<Candidate> {
    let home = dirs::home_dir().unwrap_or_default();
    // Windows: %APPDATA%, %LOCALAPPDATA%; macOS: ~/Library/Application Support;
    // Linux: ~/.config, ~/.local/share, ~/.cache.
    let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    let data = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
    let local = dirs::data_local_dir().unwrap_or_else(|| data.clone());
    let cache = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));
    let env_dir = |var: &str| std::env::var_os(var).map(PathBuf::from);

    let mut c = Vec::new();
    let mut add = |provider: &'static str, label: &'static str, path: PathBuf| {
        c.push(Candidate {
            provider,
            label,
            path,
        });
    };

    // Claude Code + Claude desktop app
    add(
        "claude",
        "Claude Code data (sessions, projects, settings)",
        home.join(".claude"),
    );
    if let Some(d) = env_dir("CLAUDE_CONFIG_DIR") {
        add("claude", "Claude Code data (CLAUDE_CONFIG_DIR)", d);
    }
    if let Ok(entries) = fs::read_dir(&home) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(".claude-") && e.path().is_dir() {
                add("claude", "Claude Code profile", e.path());
            }
        }
    }
    add("claude", "Claude desktop app data", config.join("Claude"));
    add(
        "claude",
        "Claude desktop app (installed)",
        local.join("AnthropicClaude"),
    );
    add("claude", "Claude desktop app cache", cache.join("Claude"));

    // Codex
    add(
        "codex",
        "Codex CLI data (sessions, settings)",
        env_dir("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
    );
    add(
        "codex",
        "Codex app & runtimes",
        local.join("OpenAI").join("Codex"),
    );
    add("codex", "Codex app data", config.join("Codex"));

    // OpenCode
    add(
        "opencode",
        "OpenCode data (database, sessions)",
        home.join(".local").join("share").join("opencode"),
    );
    add("opencode", "OpenCode data", local.join("opencode"));
    add("opencode", "OpenCode data", data.join("opencode"));
    add(
        "opencode",
        "OpenCode settings",
        home.join(".config").join("opencode"),
    );
    add("opencode", "OpenCode cache", cache.join("opencode"));

    // GitHub Copilot
    add(
        "copilot",
        "Copilot CLI / app data (sessions, logs, databases)",
        env_dir("COPILOT_HOME").unwrap_or_else(|| home.join(".copilot")),
    );
    add(
        "copilot",
        "Copilot runtime (SDK)",
        local.join("github-copilot-sdk"),
    );
    add(
        "copilot",
        "Copilot language-server cache",
        local.join("github-copilot"),
    );
    add(
        "copilot",
        "Copilot language-server cache",
        config.join("github-copilot"),
    );
    add(
        "copilot",
        "GitHub Copilot app (installed)",
        local.join("Programs").join("GitHub Copilot"),
    );
    add(
        "copilot",
        "GitHub Copilot app data",
        config.join("GitHub Copilot"),
    );
    for editor in ["Code", "Code - Insiders", "Cursor", "VSCodium"] {
        let gs = config.join(editor).join("User").join("globalStorage");
        add(
            "copilot",
            "Copilot Chat (editor storage)",
            gs.join("github.copilot-chat"),
        );
        add(
            "copilot",
            "Copilot (editor storage)",
            gs.join("github.copilot"),
        );
    }

    // Gemini CLI
    let gemini_home = env_dir("GEMINI_CLI_HOME").unwrap_or_else(|| home.clone());
    add(
        "gemini",
        "Gemini CLI data (chats, settings)",
        gemini_home.join(".gemini"),
    );

    // Grok Build
    add(
        "grok",
        "Grok Build data (sessions, logs)",
        env_dir("GROK_HOME").unwrap_or_else(|| home.join(".grok")),
    );

    c
}

/// Size and file count of a directory tree, without following symlinks.
fn measure(path: &Path) -> (u64, u64, bool) {
    let started = Instant::now();
    let (mut bytes, mut files) = (0u64, 0u64);
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if started.elapsed() > LOCATION_BUDGET {
            return (bytes, files, true);
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(e.path());
            } else if let Ok(m) = e.metadata() {
                bytes += m.len();
                files += 1;
            }
        }
    }
    (bytes, files, false)
}

/// Measure every existing location of every enabled provider.
pub fn scan(enabled: impl Fn(&str) -> bool) -> StorageReport {
    let started = Instant::now();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut providers: Vec<ProviderStorage> = Vec::new();
    for cand in candidates() {
        if !enabled(cand.provider) || !cand.path.is_dir() {
            continue;
        }
        let key = fs::canonicalize(&cand.path).unwrap_or_else(|_| cand.path.clone());
        if !seen.insert(key) {
            continue;
        }
        let (bytes, files, partial) = measure(&cand.path);
        let loc = Location {
            label: cand.label.to_string(),
            path: cand.path.to_string_lossy().into_owned(),
            bytes,
            files,
            partial,
        };
        match providers.iter_mut().find(|p| p.provider == cand.provider) {
            Some(p) => {
                p.total_bytes += bytes;
                p.locations.push(loc);
            }
            None => providers.push(ProviderStorage {
                provider: cand.provider.to_string(),
                total_bytes: bytes,
                locations: vec![loc],
            }),
        }
    }
    for p in &mut providers {
        p.locations.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    }
    providers.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));
    StorageReport {
        providers,
        computed_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_files_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        fs::write(tmp.path().join("one.txt"), vec![0u8; 100]).unwrap();
        fs::write(tmp.path().join("a/b/two.txt"), vec![0u8; 250]).unwrap();
        let (bytes, files, partial) = measure(tmp.path());
        assert_eq!((bytes, files, partial), (350, 2, false));
    }

    #[test]
    fn every_candidate_belongs_to_a_known_provider() {
        let known = ["claude", "codex", "opencode", "copilot", "gemini", "grok"];
        assert!(candidates().iter().all(|c| known.contains(&c.provider)));
    }
}
