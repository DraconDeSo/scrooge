# Architecture

```text
┌──────────────────────────── Scrooge (Tauri 2) ─────────────────────────────┐
│                                                                             │
│  ui/ (HTML · CSS · JS, no build step)                                       │
│    compact panel · full-screen dashboard · settings · themes                │
│        ▲ events: snapshot, quota, claude-usage, claude-local, mode          │
│        │ commands: get_*, update_settings, set_mode, check_update, …        │
│  src-tauri/ (Rust)                                                          │
│    main.rs         windows, tray, global shortcut, autostart, commands      │
│    settings.rs     persisted settings (JSON)                                │
│    quota.rs        GitHub Copilot quota (per gh account)        ── HTTPS ──▶ api.github.com
│    claude_usage.rs Claude plan limits                           ── HTTPS ──▶ api.anthropic.com
│    claude_local.rs Claude token usage from transcripts                      │
│    updater         signed updates from GitHub Releases          ── HTTPS ──▶ github.com
│        │                                                                    │
│  crates/abtop (session engine, vendored fork of abtop)                      │
│    collector/  claude · codex · opencode · copilot · gemini · grok          │
│                process tree, ports, git status                              │
│    app.rs      App::tick_no_summaries() → snapshot::Snapshot (JSON)          │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Threads

| Thread | Interval | Work |
| --- | --- | --- |
| `abtop-collector` | Session refresh (default 2 s) | Owns the `abtop::App` (not `Send`), ticks it, serialises a `Snapshot` and emits `snapshot`. |
| `quota` | Quota refresh (default 5 min) | Copilot quota for every account, then Claude plan limits. |
| `claude-local` | 60 s | Incremental scan of Claude transcripts for token usage windows. |
| updater | startup + 6 h | Checks `latest.json` on GitHub Releases. |

Every thread can be woken immediately by **Refresh now**.

## Session engine

`crates/abtop` is the collection core of [abtop](https://github.com/graykode/abtop) with
additional collectors. Each collector implements `AgentCollector::collect(&SharedProcessData)`
and returns `AgentSession`s; `MultiCollector` merges them, adds git status and detects orphan
ports. Transcript files are read incrementally (byte offsets, partial-line buffering) so a
tick only parses what was appended.

## Frontend

The UI is static (`ui/index.html`, `style.css`, `themes.css`, `app.js`) and talks to the
backend through `window.__TAURI__` (`withGlobalTauri`). State lives in one `state` object;
`render()` redraws panels from the latest snapshot. Colors are CSS custom properties: themes
set the palette, providers use `--pc` per element ("identities": a provider, or a single
GitHub Copilot account).

## Window modes

- **Compact**: frameless, resizable, optionally always on top, skipped from the taskbar;
  bounds are saved and clamped to the monitor's work area on restore.
- **Full screen**: the same window, borderless and sized to the current monitor (native
  fullscreen is avoided because of WebView sizing issues on scaled displays).

## Security model

See [SECURITY.md](../SECURITY.md): local, read-only data collection; two optional read-only
API calls with tokens obtained on demand and never stored; secret redaction in transcript
text; signed updates.
