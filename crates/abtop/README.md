# abtop (vendored)

This crate is a vendored fork of [**abtop**](https://github.com/graykode/abtop) by
Tae Hwan Jung, distributed under the MIT License (see [`LICENSE`](LICENSE)). It provides the
session-collection core used by Scrooge and still builds the original terminal UI:

```bash
cargo run -p abtop            # terminal UI
cargo run -p abtop -- --json  # one JSON snapshot
```

## JSON snapshot

`--json` prints one machine-readable snapshot (the same structure Scrooge receives
in-process through `App::to_snapshot`). The JSON snapshot includes, per session: provider,
PID, working directory, status, model, token counters, context usage, git state, child
processes, a `summary` (the session title or a sanitised first prompt), recent tool calls
and `chat_messages` — a short, secret-redacted tail of user/assistant text. Tool inputs and
file contents are never included. Treat the output as sensitive if you store or share it.

## Changes from upstream

| Area | Change |
| --- | --- |
| `collector/copilot.rs` | GitHub Copilot CLI / GitHub Copilot app sessions |
| `collector/gemini.rs` | Google Gemini CLI sessions |
| `collector/antigravity.rs` | Gemini sessions from Antigravity desktop / IDE transcripts |
| `collector/grok.rs` | xAI Grok Build sessions |
| `model::AgentSession::copilot`, `snapshot::SessionView::copilot` | Copilot usage extras (premium requests, AI credits, code changes) |
| `collector/process.rs` | `command()` helper that never opens a console window on Windows |
| `app.rs` | session-title generation (`claude --print`) limited to Claude Code, Codex and OpenCode |
| `ui/sessions.rs` | terminal labels for the new providers |

The upstream commit this fork is based on is recorded in the repository's
[`NOTICE.md`](../../NOTICE.md). For the original project, documentation and releases, see
[github.com/graykode/abtop](https://github.com/graykode/abtop).
