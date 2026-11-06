# Using Scrooge

## Views

### Compact panel

A small frameless panel docked to the bottom-right corner of the screen, above other windows
by default. Drag it by the title bar, resize it from the edges; position and size are
remembered (and always kept fully on screen).

- **Header**: live sessions and how many are working, then the token split across live
  sessions — `↓` read (input), `↑` written (output), `⟳` cache (read + write) — and `◆` the
  GitHub Copilot AI credits used by live sessions (credits or USD).
- **Quota & usage**: one block per GitHub Copilot account, Claude (plan limits and local
  consumption), Codex. Bars use each provider's color; values turn yellow at the warning
  threshold and red with ⚠ at the critical one.
- **Sessions**: one card per running agent session. The left border and badge carry the
  provider color. Each card shows context-window usage, read/written/cached tokens and, for
  Copilot, AI credits. Click a card to expand details: working directory (click to open it),
  branch, model, tokens, recent tools, child processes and the last messages.

### Full screen

A dashboard covering the current monitor:

| Panel | Contents |
| --- | --- |
| context | Token-rate chart stacked **by provider** or **by project** (switch above the chart; click legend entries to hide/show series) and the context usage of every session |
| quota & usage | Every quota source, including all Copilot buckets and Claude per-model weekly usage |
| tokens | Totals for the selected session (or all sessions) with a per-provider split |
| projects | Git branch and added/modified files per project, with a dot per provider working in it |
| storage | Disk space used by each provider in every place it stores data (CLI data and sessions, caches, desktop-app data, bundled runtimes, editor storage), largest first. Click a location to open it. Measured every 15 minutes or with ⟳ |
| ports | Listening ports opened by agent processes, and orphan ports left by ended sessions |
| sessions | Session table with a detail pane |

## Tray

- **Left click**: show / hide the panel.
- **Menu**: show/hide, compact panel, full screen, always on top, refresh now, settings, quit.

Closing the window only hides it; quit from the tray menu or Settings → About.

## Keyboard

| Key | Action |
| --- | --- |
| `F` | Toggle full screen |
| `Esc` | Back to compact, or hide the panel |
| `R` | Refresh now |
| `S` | Settings |
| `/` | Filter sessions |
| `↑` / `↓` | Select session |
| `Enter` | Expand the selected card |
| `Ctrl+Alt+A` | Show / hide from anywhere (configurable) |

## Session status

| Label | Meaning |
| --- | --- |
| `● exec` | A tool is running |
| `◐ think` | The model is generating |
| `○ wait` | Waiting for your input |
| `⏳ rate` | Waiting on a provider rate limit |
| `? unknown` | Recent session whose process could not be confirmed |
