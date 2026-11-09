# Configuration

Open **Settings** with the ⚙ button, the `S` key or the tray menu. Changes apply
immediately and are saved to `settings.json` in the Scrooge configuration directory:

| System | Path |
| --- | --- |
| Windows | `%APPDATA%\scrooge\settings.json` |
| macOS | `~/Library/Application Support/scrooge/settings.json` |
| Linux | `~/.config/scrooge/settings.json` |

## General

| Setting | Default | Description |
| --- | --- | --- |
| Start with Windows / at login | off | Registers Scrooge to start when you sign in. |
| Stay in the tray at login | off | When started at login, don't open the panel. |
| Open in | Compact | View shown at startup. |
| Always on top | on | Keep the compact panel above other windows. |
| Global shortcut | `Ctrl+Alt+A` | Show/hide from anywhere. Click the field and press a new combination (at least one modifier). |
| Session refresh | 2 s | How often agents, processes and transcripts are scanned. |
| Quota refresh | 5 min | How often Copilot and Claude quota are requested. |

## Appearance

| Setting | Description |
| --- | --- |
| Theme | Default, Fallout, Star Wars, Forest, Nordic, Old Paper, Cyberpunk, Fantasy, Kawaii, Developer. |
| Dark / Light / System | Each theme has both variants; *System* follows the OS setting. |
| Density | Small, Normal, Large text and spacing. |
| Provider colors | Each theme ships its own palette. Overrides are per provider and per Copilot account; colors too similar to another provider are rejected. *Use theme colors* removes all overrides. |
| Warning / critical thresholds | Percentages at which context and quota values turn yellow / red. |

## Providers

| Setting | Description |
| --- | --- |
| Enabled providers | Switch each provider (Claude Code, Codex, OpenCode, GitHub Copilot, Gemini / Antigravity, Grok Build) on or off. A disabled provider is **not scanned at all**: its files are not read and no quota requests are made for it. Each row shows running sessions and disk usage. |
| Disk usage → Measure now | Re-measure the storage folders immediately (otherwise every 15 minutes). |

## Sessions

| Setting | Description |
| --- | --- |
| Sort by | Most recent, activity, provider, context usage, tokens. |
| Show idle sessions | Include sessions waiting for input or in unknown state. |
| Show chat in details | Show the last prompts and replies in the detail view. |
| Show current task line | The `└` line under each card. |

## Quota & accounts

| Setting | Description |
| --- | --- |
| GitHub Copilot accounts | Every account in `gh auth status`; untick to hide one. Each account is a separate provider with its own color. |
| Show AI credits as | Credits or US dollars. |
| Price per AI credit (USD) | Your plan's rate, used for the dollar view (default 0.01). |
| Copilot rows in compact panel | *Premium only* or all buckets. |
| Show local Claude consumption | Token totals from Claude Code transcripts. |

The *Claude* box shows the state of both Claude sources: plan limits (requires the Claude
Code CLI login) and local consumption (always available).

## abtop configuration

The session engine also reads `~/.config/abtop/config.toml` (shared with the abtop terminal
UI), for example to add extra Claude profile directories:

```toml
claude_config_dirs = ["~/.claude-work"]
hidden_agents = ["opencode"]
```

## Environment variables

| Variable | Effect |
| --- | --- |
| `CLAUDE_CONFIG_DIR` | Claude Code configuration root |
| `COPILOT_HOME` | GitHub Copilot CLI data directory (default `~/.copilot`) |
| `GEMINI_CLI_HOME` | Home used by Gemini CLI (data in `<home>/.gemini`) |
| `GROK_HOME` | Grok Build data directory (default `~/.grok`) |
