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
/// Emit incremental progress during long directory walks at this cadence.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(350);

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
    pub scanning: bool,
}

struct Candidate {
    provider: &'static str,
    label: &'static str,
    path: PathBuf,
}

fn unquote_yaml(v: &str) -> &str {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')))
    {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

fn collect_worktree_roots_for_path(
    path: &Path,
    from_session: bool,
    roots: &mut Vec<PathBuf>,
    standalone: &mut Vec<PathBuf>,
) {
    let mut inside_copilot_worktrees = false;
    for ancestor in path.ancestors() {
        if ancestor
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("copilot-worktrees"))
            && ancestor.is_dir()
        {
            roots.push(ancestor.to_path_buf());
            inside_copilot_worktrees = true;
        }
        let sibling = ancestor.join("copilot-worktrees");
        if sibling.is_dir() {
            roots.push(sibling);
        }
    }

    let git_worktrees = path.join(".git").join("worktrees");
    if let Ok(entries) = fs::read_dir(&git_worktrees) {
        for entry in entries.flatten() {
            let wt_meta_dir = entry.path();
            let Ok(gitdir_raw) = fs::read_to_string(wt_meta_dir.join("gitdir")) else {
                continue;
            };
            let trimmed = gitdir_raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let target = Path::new(trimmed);
            let resolved = if target.is_absolute() {
                target.to_path_buf()
            } else {
                wt_meta_dir.join(target)
            };
            let wt_dir = resolved.parent().unwrap_or(&resolved);
            let mut found_pool = false;
            for ancestor in wt_dir.ancestors() {
                if ancestor
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("copilot-worktrees"))
                    && ancestor.is_dir()
                {
                    roots.push(ancestor.to_path_buf());
                    found_pool = true;
                }
            }
            if !found_pool
                && wt_dir.is_dir()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("copilot")
            {
                standalone.push(wt_dir.to_path_buf());
            }
        }
    }

    if from_session && !inside_copilot_worktrees && path.join(".git").is_file() && path.is_dir() {
        standalone.push(path.to_path_buf());
    }
}

/// Discover GitHub Copilot worktree directories across `~/.copilot`, trusted
/// repository folders, `.git/worktrees` links and session `workspace.yaml` files.
fn copilot_worktree_dirs(copilot_home: &Path, home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        copilot_home.join("repos").join("copilot-worktrees"),
        copilot_home.join("worktrees"),
        home.join("copilot-worktrees"),
        home.join("repos").join("copilot-worktrees"),
    ];
    let mut standalone = Vec::new();

    if let Ok(cfg_text) = fs::read_to_string(copilot_home.join("config.json")) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&cfg_text) {
            if let Some(folders) = val["trustedFolders"].as_array() {
                for folder in folders.iter().filter_map(|v| v.as_str()) {
                    collect_worktree_roots_for_path(
                        Path::new(folder),
                        false,
                        &mut roots,
                        &mut standalone,
                    );
                }
            }
        }
    }

    if let Ok(sessions) = fs::read_dir(copilot_home.join("session-state")) {
        for entry in sessions.flatten() {
            let yaml_path = entry.path().join("workspace.yaml");
            let Ok(yaml) = fs::read_to_string(&yaml_path) else {
                continue;
            };
            for line in yaml.lines() {
                let trimmed = line.trim();
                for prefix in ["cwd:", "git_root:", "worktree:", "worktree_path:"] {
                    if let Some(rest) = trimmed.strip_prefix(prefix) {
                        let p = unquote_yaml(rest);
                        if !p.is_empty() {
                            collect_worktree_roots_for_path(
                                Path::new(p),
                                true,
                                &mut roots,
                                &mut standalone,
                            );
                        }
                    }
                }
            }
        }
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut canon_roots = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let canon = fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        if seen.insert(canon.clone()) {
            canon_roots.push(canon);
            out.push(root);
        }
    }
    for wt in standalone {
        if !wt.is_dir() {
            continue;
        }
        let canon = fs::canonicalize(&wt).unwrap_or_else(|_| wt.clone());
        if canon_roots.iter().any(|r| canon.starts_with(r)) {
            continue;
        }
        if seen.insert(canon) {
            out.push(wt);
        }
    }
    out
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
    let claude_home = home.join(".claude");
    add(
        "claude",
        "Claude Code worktrees",
        claude_home.join("worktrees"),
    );
    add(
        "claude",
        "Claude Code data (sessions, projects, settings)",
        claude_home,
    );
    if let Some(d) = env_dir("CLAUDE_CONFIG_DIR") {
        add("claude", "Claude Code worktrees", d.join("worktrees"));
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
    let codex_home = env_dir("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
    add("codex", "Codex worktrees", codex_home.join("worktrees"));
    add("codex", "Codex CLI data (sessions, settings)", codex_home);
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
    let copilot_home = env_dir("COPILOT_HOME").unwrap_or_else(|| home.join(".copilot"));
    for wt_dir in copilot_worktree_dirs(&copilot_home, &home) {
        add("copilot", "Copilot worktrees", wt_dir);
    }
    add(
        "copilot",
        "Copilot workspaces",
        copilot_home.join("workspaces"),
    );
    add(
        "copilot",
        "Copilot CLI / app data (sessions, logs, databases)",
        copilot_home,
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

/// Size and file count of a directory tree, without following symlinks or
/// descending into excluded child candidate directories.
fn measure_with_exclusions(
    path: &Path,
    excluded_canon: &[PathBuf],
    mut on_tick: impl FnMut(u64, u64),
) -> (u64, u64, bool) {
    let started = Instant::now();
    let mut last_emit = started;
    let (mut bytes, mut files) = (0u64, 0u64);
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let now = Instant::now();
        if now.duration_since(started) > LOCATION_BUDGET {
            return (bytes, files, true);
        }
        if now.duration_since(last_emit) >= PROGRESS_INTERVAL {
            last_emit = now;
            on_tick(bytes, files);
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
                let subdir = e.path();
                if !excluded_canon.is_empty() {
                    let sub_name = subdir.file_name();
                    if excluded_canon.iter().any(|ex| {
                        ex.file_name() == sub_name
                            && fs::canonicalize(&subdir).map(|c| &c == ex).unwrap_or(false)
                    }) {
                        continue;
                    }
                }
                stack.push(subdir);
            } else if let Ok(m) = e.metadata() {
                bytes += m.len();
                files += 1;
            }
        }
    }
    (bytes, files, false)
}

#[cfg(test)]
fn measure(path: &Path) -> (u64, u64, bool) {
    measure_with_exclusions(path, &[], |_, _| {})
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn build_snapshot(
    mut providers: Vec<ProviderStorage>,
    started: Instant,
    scanning: bool,
) -> StorageReport {
    for p in &mut providers {
        p.locations.sort_by_key(|a| std::cmp::Reverse(a.bytes));
    }
    providers.sort_by_key(|a| std::cmp::Reverse(a.total_bytes));
    StorageReport {
        providers,
        computed_at_ms: now_ms(),
        duration_ms: started.elapsed().as_millis() as u64,
        scanning,
    }
}

fn upsert_location(providers: &mut Vec<ProviderStorage>, provider: &str, loc: Location) {
    match providers.iter_mut().find(|p| p.provider == provider) {
        Some(p) => {
            if let Some(existing) = p.locations.iter_mut().find(|l| l.path == loc.path) {
                p.total_bytes = p.total_bytes.saturating_sub(existing.bytes) + loc.bytes;
                *existing = loc;
            } else {
                p.total_bytes += loc.bytes;
                p.locations.push(loc);
            }
        }
        None => providers.push(ProviderStorage {
            provider: provider.to_string(),
            total_bytes: loc.bytes,
            locations: vec![loc],
        }),
    }
}

/// Measure every existing location of every enabled provider, emitting
/// intermediate snapshots via `on_progress` while `scanning` is `true`.
pub fn scan_progressive(
    enabled: impl Fn(&str) -> bool,
    mut on_progress: impl FnMut(&StorageReport),
) -> StorageReport {
    let started = Instant::now();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut active: Vec<(Candidate, PathBuf)> = Vec::new();
    for cand in candidates() {
        if !enabled(cand.provider) || !cand.path.is_dir() {
            continue;
        }
        let canon = fs::canonicalize(&cand.path).unwrap_or_else(|_| cand.path.clone());
        if seen.insert(canon.clone()) {
            active.push((cand, canon));
        }
    }

    let all_canons: Vec<PathBuf> = active.iter().map(|(_, c)| c.clone()).collect();
    let mut providers: Vec<ProviderStorage> = Vec::new();

    for (cand, canon) in &active {
        let excluded: Vec<PathBuf> = all_canons
            .iter()
            .filter(|other| *other != canon && other.starts_with(canon))
            .cloned()
            .collect();
        let path_str = cand.path.to_string_lossy().into_owned();
        let label_str = cand.label.to_string();

        let (bytes, files, partial) =
            measure_with_exclusions(&cand.path, &excluded, |partial_bytes, partial_files| {
                if partial_bytes == 0 {
                    return;
                }
                let mut temp_providers = providers.clone();
                upsert_location(
                    &mut temp_providers,
                    cand.provider,
                    Location {
                        label: label_str.clone(),
                        path: path_str.clone(),
                        bytes: partial_bytes,
                        files: partial_files,
                        partial: false,
                    },
                );
                let snap = build_snapshot(temp_providers, started, true);
                on_progress(&snap);
            });

        upsert_location(
            &mut providers,
            cand.provider,
            Location {
                label: label_str,
                path: path_str,
                bytes,
                files,
                partial,
            },
        );
        let snap = build_snapshot(providers.clone(), started, true);
        on_progress(&snap);
    }

    build_snapshot(providers, started, false)
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
    fn excludes_child_candidates_from_parent_totals() {
        let tmp = tempfile::tempdir().unwrap();
        let child = tmp.path().join("repos/copilot-worktrees");
        fs::create_dir_all(&child).unwrap();
        fs::write(tmp.path().join("root.db"), vec![0u8; 120]).unwrap();
        fs::write(child.join("wt.bin"), vec![0u8; 500]).unwrap();

        let excluded = vec![fs::canonicalize(&child).unwrap()];
        let (parent_bytes, parent_files, _) =
            measure_with_exclusions(tmp.path(), &excluded, |_, _| {});
        let (child_bytes, child_files, _) = measure(&child);
        assert_eq!((parent_bytes, parent_files), (120, 1));
        assert_eq!((child_bytes, child_files), (500, 1));
    }

    #[test]
    fn discovers_copilot_worktrees_from_sessions_and_trusted_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let copilot_home = home.join(".copilot");
        let internal_wt = copilot_home.join("repos").join("copilot-worktrees");
        let repos_root = tmp.path().join("repos");
        let project = repos_root.join("my-app");
        let external_wt_pool = repos_root.join("copilot-worktrees");
        let wt_instance = external_wt_pool.join("my-app").join("branch-1");
        let session_dir = copilot_home.join("session-state").join("s1");

        fs::create_dir_all(&internal_wt).unwrap();
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&wt_instance).unwrap();
        fs::create_dir_all(&session_dir).unwrap();

        fs::write(
            copilot_home.join("config.json"),
            serde_json::json!({
                "trustedFolders": [project.to_string_lossy()]
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            session_dir.join("workspace.yaml"),
            format!("cwd: {}\n", wt_instance.display()),
        )
        .unwrap();

        let found: Vec<PathBuf> = copilot_worktree_dirs(&copilot_home, &home)
            .into_iter()
            .map(|p| fs::canonicalize(&p).unwrap_or(p))
            .collect();
        assert!(found.contains(&fs::canonicalize(&internal_wt).unwrap()));
        assert!(found.contains(&fs::canonicalize(&external_wt_pool).unwrap()));
        // The nested worktree instance inside external_wt_pool is not duplicated.
        assert!(!found.contains(&fs::canonicalize(&wt_instance).unwrap()));
    }

    #[test]
    fn every_candidate_belongs_to_a_known_provider() {
        let known = ["claude", "codex", "opencode", "copilot", "gemini", "grok"];
        assert!(candidates().iter().all(|c| known.contains(&c.provider)));
    }
}
