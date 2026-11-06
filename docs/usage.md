# Using Scrooge

## Views

### Compact panel

A small frameless panel docked to the bottom-right corner of the screen, above other windows
by default. Drag it by the title bar, resize it from the edges; position and size are
remembered (and always kept fully on screen).
On Windows, release the panel near an edge to snap it. Drop the pointer at the top edge to
open the dashboard on that monitor.

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
  New sessions appear below the existing ones. Change **Initial session order** in Settings
  to sort the list again.

### Historical usage

Choose **Projects** or **Providers** in either window mode. Select Today, Last 7 days,
Last 30 days or custom dates in your local time zone. Expand a row to see models, token
counts and available charges. Linked Git worktrees share their original project's row when
their Git metadata is accessible. Provider rows also show live quota and reset times from
the existing quota sources, independent of the selected period.

**Data sources** reports which logs or imports are available. Antigravity and Antigravity
IDE are scanned separately. Some versions omit token counts or the model from their logs.
For Gemini App activity, put the Gemini Apps activity JSON array from Google Takeout in
the usage imports folder as `gemini-activity.json`. Its timestamps count activity, with
token counts and live quota shown only when a source exposes them.

Use **Open usage imports folder** under **Data sources** to add actual billing records.
Save one JSON object per line in a `.jsonl` file. For example:

```json
{"id":"cloud-bill-unique-row-id","timestamp":"2026-11-06T08:30:00Z","provider":"gemini","kind":"api","currency":"EUR","amount_micros":1250000,"model":"gemini-flash"}
```

Use `api` for API charges and `plan_extra` for paid subscription overages. The example
amount is EUR 1.25. Preserve the currency, stable billing row ID and timestamp from the
source report. The supported provider IDs are `claude`, `codex`, `copilot`, `gemini`,
`opencode` and `grok`. Optional `account`, `model` and `project` fields let you
attribute the charge where the source provides a match. For a local project, set
`project` to the checkout path. Unattributed costs are shown under **Without a project**.
Scrooge reads these imports locally and shows billing as unavailable when none covers
the selection.

### Full screen

A dashboard covering the current monitor. **Live sessions** shows these panels:

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
