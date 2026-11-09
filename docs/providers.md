# Providers and data sources

Scrooge reads what each agent already writes on disk. Everything is read-only; a provider
only appears while it is running.

| Provider | Sessions | Tokens | Context % | Quota / limits | Live process match |
| --- | :---: | :---: | :---: | :---: | --- |
| Claude Code | ✅ | ✅ in/out/cache | ✅ | ✅ 5h / 7d (CLI login) + local usage | PID → `sessions/<pid>.json` |
| Codex CLI | ✅ | ✅ | ✅ | ✅ 5h / 7d | PID → open rollout file |
| OpenCode | ✅ | ✅ | ✅ | — | PID → working directory |
| GitHub Copilot | ✅ | ✅ in/out/cache/reasoning | ✅ | ✅ per account, monthly | `inuse.<pid>.lock` |
| Gemini CLI | ✅ | ✅ in/out/cache | ✅ | — | most recent chat per process |
| Gemini via Antigravity | ✅ | ✅ in/out/cache | ✅ | local usage | recent brain transcript per process |
| Grok Build | ✅ | ≈ total only | ✅ | — | most recent session per process |

## Claude Code

- Sessions: `~/.claude/sessions/<pid>.json` and transcripts in
  `~/.claude/projects/<encoded-path>/<session>.jsonl` (plus `~/.claude-*` profiles and
  `CLAUDE_CONFIG_DIR`).
- **Plan limits**: `GET https://api.anthropic.com/api/oauth/usage`, authenticated with the
  Claude Code CLI login in `~/.claude/.credentials.json`. Scrooge never refreshes that
  token; if it is missing or expired, run `claude` once. The Claude desktop app keeps its own
  login, which Scrooge does not read.
- **Local consumption**: assistant `usage` blocks in all transcripts touched in the last
  7 days, de-duplicated per API response; shown as the current 5-hour block, the last
  24 hours (hourly histogram) and 7 days per model.
- An abtop status-line file (`abtop --setup`) takes precedence for limits when present.

## Codex CLI

`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`; rate limits come from `token_count`
events in the rollout.

## OpenCode

`~/.local/share/opencode/opencode.db` (also `%LOCALAPPDATA%`/`%APPDATA%` on Windows), read
with `sqlite3 -readonly`.

## GitHub Copilot

Covers the `copilot` CLI and the GitHub Copilot desktop app (which runs the same engine).

```text
~/.copilot/session-state/<session-id>/
    events.jsonl        append-only event log
    workspace.yaml      cwd, branch, name, client
    inuse.<pid>.lock    present while process <pid> holds the session
```

| Event | Used for |
| --- | --- |
| `session.start` / `session.resume` | working directory, branch, model, reasoning effort, version |
| `session.usage_record` | per-call tokens (input incl. cache, output, cache read/write, reasoning), model, `maxPromptTokens` |
| `session.usage_checkpoint` / `session.shutdown` | premium requests, AI credits (`totalNanoAiu`), lines added/removed |
| `assistant.turn_start` / `assistant.turn_end` | thinking state |
| `tool.execution_start` / `tool.execution_complete` | running tools and timeline |
| `user.message` / `assistant.message` | chat tail (redacted) |

**Quota**: `GET https://api.github.com/copilot_internal/user` for every github.com account
logged into the GitHub CLI (`gh auth token --user <login>`). Buckets: premium interactions,
chat, completions — entitlement, remaining, unlimited flag, overage and credits used. The
account in `~/.copilot/config.json` (`lastLoggedInUser`) is tagged **CLI**; live sessions are
attributed to it. Each account is shown as its own provider with its own color.

Copilot Chat inside editors is not covered: its local files contain no token or model data.

## Gemini CLI

`~/.gemini/tmp/<project>/chats/session-*.jsonl` (JSONL recorder). Replies carry `tokens`
(`input`, `cached`, `output`, `thoughts`, `tool`) and `model`; a re-appended message replaces
the earlier one. The project's working directory is read from `.project_root`. Gemini CLI
keeps no process registry, so each running `gemini` process is matched to the most recently
written chat. Legacy single-document `session-*.json` files are not read.

## Gemini via Antigravity

Antigravity desktop and IDE sessions are read from
`~/.gemini/{antigravity,antigravity-ide}/brain/<session>/.system_generated/logs/`.
`transcript_full.jsonl` is preferred, with `transcript.jsonl` as a fallback. Completed
`PLANNER_RESPONSE` events provide input, output and cache-read tokens. This local accounting
does not require a Google login to be exposed to Scrooge; provider authentication only
affects whether an official remote allowance can be shown.

## Grok Build

`~/.grok/sessions/<url-encoded cwd>/<session-id>/` — title, model and timestamps from
`summary.json`; turns, context usage/window and token totals from `signals.json`. Grok does
not split input and output tokens, so its total is shown as approximate. Each running
`grok` process is matched to the most recently updated session.

## Stability

All of these formats are internal to the respective tools and may change between their
releases. Parsers are defensive: unknown fields are ignored and missing ones degrade to
"not available" rather than errors.
