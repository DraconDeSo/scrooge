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

#### Automatic billing

To sync billed usage, create `billing-sources.json` in the same folder. Include one
entry per payer. Use unique account labels for the same provider and avoid overlapping
scopes, such as a GitHub organization and the enterprise that pays for it.

```json
[
  {"provider":"openai","account":"OpenAI work","key_env":"SCROOGE_BILLING_OPENAI"},
  {"provider":"anthropic","account":"Claude Console","key_env":"SCROOGE_BILLING_ANTHROPIC"},
  {"provider":"github","account":"Copilot work","owner":"example-org","scope":"organization","key_env":"SCROOGE_BILLING_GITHUB"},
  {"provider":"xai","account":"Grok team","team_id":"your-team-id","key_env":"SCROOGE_BILLING_XAI"},
  {"provider":"google","account":"Gemini API","query_project":"query-project","table":"export-project.billing.gcp_billing_export_v1_example","location":"EU","service_ids":["your-billing-service-id"],"maximum_bytes_billed":10000000}
]
```

Set each named environment variable in the environment that launches Scrooge, then
restart it. The JSON file contains credential names only. Keep just the entries for
accounts you want to sync.

| Provider | Credential and data |
| --- | --- |
| OpenAI | An organization Admin API key. Reads [organization costs](https://developers.openai.com/api/reference/resources/admin/subresources/organization/subresources/usage/methods/costs) by project and line item. |
| Anthropic | A Claude Console Admin API key. Reads the [Console cost report](https://platform.claude.com/docs/en/manage-claude/usage-cost-api) by workspace and model. |
| GitHub Copilot | A token with access to [AI credit billing usage](https://docs.github.com/en/rest/billing/usage) for the configured `user`, `organization`, or `enterprise` payer. Shows net AI credit charges as plan extras. |
| xAI | A management key with [billing access](https://docs.x.ai/developers/rest-api-reference/management/billing) to the configured team. Reads API spend in USD. |
| Google Cloud | A [standard BigQuery billing export](https://docs.cloud.google.com/billing/docs/how-to/export-data-bigquery-tables/standard-usage). Install the Google Cloud CLI and run `gcloud auth application-default login` with BigQuery Job User on the query project and read access to the export. You can instead set `key_env` to the name of an OAuth access-token variable. Select the Gemini billing service IDs, optionally narrow by `sku_ids`, and set the byte limit for the BigQuery query. |

Remote costs use UTC calendar days for a selection of up to 366 days. Scrooge caches
reports for 15 minutes. Providers may post charges with a delay. Optional
`project_paths` maps remote project or workspace IDs to local checkout paths.
`model_names` can map OpenAI line items, xAI billing descriptions and Google Cloud SKU
IDs to local model names. Costs with no model or project mapping remain unattributed.

Use the same `account` label in a manual import and its automatic source. For a matching
provider, account and charge type, a successful sync supersedes the manual import for
the selected period. Token counts come from the usage sources listed separately in
**Data sources**. Gemini Apps and Antigravity personal subscription history is limited
to the data available in exports or local logs. A Google Cloud billing connection
covers the selected Cloud services.

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
