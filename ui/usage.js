"use strict";

const usageState = {
  views: { compact: "live", full: "projects" },
  report: null,
  fetchedAt: 0,
  pending: false,
  queued: false,
};

const exportState = {
  preset: "project_tokens",
  format: "csv",
  report: null,
  daily: null,
  rangeKey: "",
  selectedProject: "__ALL__",
  checkedProviders: new Set(),
  lastSavedPath: null,
  generated: { filename: "scrooge-export.csv", content: "" },
};

const USAGE_SOURCES = {
  claude: "Claude Code",
  codex: "Codex CLI and desktop",
  copilot: "Copilot CLI and desktop",
  "gemini-cli": "Gemini CLI",
  "gemini-app": "Gemini App",
  antigravity: "Antigravity",
  "antigravity-ide": "Antigravity IDE",
  "open-code": "OpenCode",
  grok: "Grok Build",
  billing: "Billing imports",
  "provider-billing": "Connected billing",
};
const TOKEN_FIELDS = ["input", "output", "cache_read", "cache_write"];

const EXPORT_PRESET_META = {
  project_tokens: {
    heading: "Export tokens by project",
    desc: "Select a project and choose which of the providers that worked on it to include.",
    period: true,
    project: true,
    providers: true,
    grouping: true,
  },
  provider_stats: {
    heading: "Export usage statistics by provider & period",
    desc: "Choose the time period and the AI providers to include in the usage statistics export.",
    period: true,
    project: false,
    providers: true,
    grouping: true,
  },
  daily_timeline: {
    heading: "Export daily usage timeline",
    desc: "Day-by-day token and request history filtered by period, project and providers.",
    period: true,
    project: true,
    providers: true,
    grouping: true,
  },
  billing_quota: {
    heading: "Export billing charges & live quotas",
    desc: "Export recorded API / plan overage charges for the selected period alongside current live quotas.",
    period: true,
    project: false,
    providers: true,
    grouping: false,
  },
  live_sessions: {
    heading: "Export live sessions & disk storage",
    desc: "Snapshot of currently running agent sessions, token windows, listening ports and disk space.",
    period: false,
    project: false,
    providers: false,
    grouping: false,
  },
};

function usageView() { return usageState.views[state.mode]; }
function usageProviderName(provider) { return provider === "gemini" ? "Google" : provName(provider); }
function projectShortName(path) { return (path || "").split(/[\\/]/).filter(Boolean).pop() || "Without a project"; }

function localDate(date) {
  const two = (n) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())}`;
}

function computeRangeFromInputs(periodVal, fromVal, throughVal) {
  const range = (start, end, label) => {
    const utcDay = (date) => Date.UTC(date.getFullYear(), date.getMonth(), date.getDate());
    return {
      start: +start,
      end: +end,
      billingStart: utcDay(start),
      billingEnd: utcDay(end),
      fromDate: localDate(start),
      throughDate: localDate(new Date(+end - 86_400_000)),
      label,
    };
  };
  if (periodVal === "custom") {
    if (!fromVal || !throughVal) return null;
    const start = new Date(fromVal + "T00:00:00");
    const end = new Date(throughVal + "T00:00:00");
    end.setDate(end.getDate() + 1);
    return +end > +start ? range(start, end, `${fromVal}_to_${throughVal}`) : null;
  }
  const end = new Date();
  end.setHours(0, 0, 0, 0);
  end.setDate(end.getDate() + 1);
  const start = new Date(end);
  const days = periodVal === "today" ? 1 : Number(periodVal);
  start.setDate(start.getDate() - days);
  return range(start, end, periodVal === "today" ? "today" : `last_${days}d`);
}

function usageRange() {
  return computeRangeFromInputs($("usage-period").value, $("usage-from").value, $("usage-through").value);
}

function exportRange() {
  return computeRangeFromInputs($("exp-period").value, $("exp-from").value, $("exp-through").value);
}

function applyUsageView() {
  const view = usageView();
  $("app").dataset.view = view;
  for (const button of $("usage-view").querySelectorAll("button")) {
    const active = button.dataset.view === view;
    button.classList.toggle("on", active);
    button.setAttribute("aria-pressed", String(active));
  }
  if (view === "live") return;
  renderUsage();
  if (!usageState.report || Date.now() - usageState.fetchedAt >= 60_000) loadUsage();
}

async function loadUsage() {
  if (usageView() === "live") return;
  if (usageState.pending) { usageState.queued = true; return; }
  const range = usageRange();
  if (!range) {
    $("usage-status").textContent = "Choose a valid date range.";
    return;
  }
  usageState.pending = true;
  $("usage-status").textContent = "Reading usage…";
  try {
    const report = await invoke("get_usage_report", range);
    const current = usageRange();
    if (!current || current.start !== range.start || current.end !== range.end) return;
    usageState.report = report;
    usageState.fetchedAt = Date.now();
    renderUsage();
    $("usage-status").textContent = "";
  } catch (err) {
    $("usage-status").textContent = String(err || "Usage data could not be read.");
  } finally {
    usageState.pending = false;
    if (usageState.queued) {
      usageState.queued = false;
      loadUsage();
    }
  }
}

function usageTotals(rows) {
  const tokens = Object.fromEntries(TOKEN_FIELDS.map((key) => [key, 0]));
  const charges = new Map();
  let requests = 0;
  let activities = 0;
  for (const row of rows) {
    requests += row.requests;
    activities += row.activities;
    for (const key of TOKEN_FIELDS) tokens[key] += row.tokens[key];
    for (const charge of row.charges || []) {
      const key = charge.kind + ":" + charge.currency;
      const sum = charges.get(key) || { ...charge, amount_micros: 0 };
      sum.amount_micros += charge.amount_micros;
      charges.set(key, sum);
    }
  }
  return { tokens, requests, activities, charges: [...charges.values()] };
}

function formatChargesText(charges) {
  if (!charges || !charges.length) return "";
  return charges.map((c) => {
    const label = c.kind === "api" ? "API" : "Plan extra";
    return `${label} ${c.currency} ${(c.amount_micros / 1e6).toFixed(4)}`;
  }).join("; ");
}

function usageMetrics(rows, details) {
  const { tokens, requests, activities, charges } = usageTotals(rows);
  const total = TOKEN_FIELDS.reduce((sum, key) => sum + tokens[key], 0);
  const tokenDetail = `Input ${fmtNum(tokens.input)} · output ${fmtNum(tokens.output)} · cache read ${fmtNum(tokens.cache_read)} · cache write ${fmtNum(tokens.cache_write)}`;
  const money = charges.map((charge) => {
    const amount = new Intl.NumberFormat(undefined, {
      style: "currency", currency: charge.currency, currencyDisplay: "code", maximumFractionDigits: 6,
    }).format(charge.amount_micros / 1e6);
    return `<span title="Reported billing amount">${charge.kind === "api" ? "API" : "Plan extra"} <b>${esc(amount)}</b></span>`;
  }).join("");
  return `<span class="usage-metrics">
    ${requests ? `<span title="${esc(tokenDetail)}"><b>${fmtTok(total)}</b> tokens</span>` : '<span>Tokens unavailable</span>'}
    ${activities ? `<span><b>${fmtNum(activities)}</b> Gemini App activities</span>` : ""}
    ${details && requests ? `<small>↓${fmtTok(tokens.input)} ↑${fmtTok(tokens.output)} ⟳${fmtTok(tokens.cache_read + tokens.cache_write)}</small>` : ""}
    ${money || '<span>Billing unavailable</span>'}
  </span>`;
}

function usageModelRows(rows) {
  const models = new Map();
  for (const row of rows) {
    const key = JSON.stringify([row.provider, row.model || row.source, row.account]);
    if (!models.has(key)) models.set(key, []);
    models.get(key).push(row);
  }
  const sorted = [...models.values()].sort(([a], [b]) =>
    usageProviderName(a.provider).localeCompare(usageProviderName(b.provider)) ||
    (a.model || "").localeCompare(b.model || "") ||
    (a.account || "").localeCompare(b.account || ""));
  return sorted.map((group) => {
    const { provider, model, account, source } = group[0];
    const label = model || (source === "gemini-app" ? "Gemini App activity" : source.endsWith("billing") ? "Unassigned model" : "Unknown model");
    const sources = [...new Set(group.map((r) => USAGE_SOURCES[r.source]))].filter(Boolean).join(" · ");
    return `<div class="usage-model" ${idStyle(provider)}>
      <div><b>${esc(label)}</b><small>${esc([usageProviderName(provider), account, sources].filter(Boolean).join(" · "))}</small></div>
      ${usageMetrics(group, true)}
    </div>`;
  }).join("");
}

function renderUsage() {
  const view = usageView();
  if (view === "live") return;
  $("usage-heading").textContent = view === "projects" ? "Usage by project" : "Usage by provider";
  const box = $("usage-rows");
  const open = new Set([...box.querySelectorAll("details[open]")].map((entry) => entry.dataset.key));
  const rows = (usageState.report?.rows || []).filter((r) => !isHidden(r.provider));
  const groups = new Map();
  if (view === "providers") {
    for (const provider of Object.keys(PROVIDERS).filter((p) => !isHidden(p))) groups.set(provider, []);
  }
  for (const row of rows) {
    const key = view === "projects" ? row.project || "" : row.provider;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(row);
  }
  box.innerHTML = [...groups].map(([key, group]) => {
    const label = view === "providers" ? usageProviderName(key) : projectShortName(key);
    const entryKey = view + ":" + key;
    const quotas = view === "providers" ? `<span class="usage-quota" data-provider="${esc(key)}"></span>` : "";
    const expAttr = view === "projects"
      ? `data-exp-project="${esc(key)}" title="Export tokens for ${esc(label)}"`
      : `data-exp-provider="${esc(key)}" title="Export usage for ${esc(label)}"`;
    return `<details class="usage-entry" data-key="${esc(entryKey)}" ${open.has(entryKey) ? "open" : ""}>
      <summary><b title="${esc(view === "projects" ? key : label)}">${esc(label)}</b><span>${usageMetrics(group, false)}<button type="button" class="mini-btn exp-row-btn" ${expAttr}>⤓</button></span>${quotas}</summary>
      ${usageModelRows(group) || '<p class="empty">No model usage for this period.</p>'}
    </details>`;
  }).join("") || '<p class="empty">No recorded usage for this period.</p>';
  $("usage-sources").innerHTML = (usageState.report?.coverage || []).map((source) => {
    let status = source.supported ? "No local records" : "History unavailable";
    if (source.source === "gemini-app") status = "Import a Gemini activity export";
    if (source.source === "billing") status = "Import a provider billing report";
    if (!source.readable) status = "Partial · some records could not be read";
    else if (source.earliest_ms) status = `${fmtNum(source.records_in_period)} in period · first record read ${new Date(source.earliest_ms).toLocaleDateString()}`;
    else if (source.files) status = "Logs found · no token or billing records";
    return `<div class="usage-source"><b>${esc(source.label || USAGE_SOURCES[source.source])}</b><span>${esc(source.detail || status)}</span></div>`;
  }).join("");
  updateUsageQuotas();
}

function quotaReset(seconds) {
  if (!seconds) return "reset unavailable";
  const date = new Date(seconds * 1000);
  const countdown = seconds * 1000 > Date.now() ? ` (${untilEpoch(seconds)})` : "";
  return `resets ${date.toLocaleString([], { dateStyle: "short", timeStyle: "short" })}${countdown}`;
}

function usageQuotaLabels(provider) {
  const stamp = (ms) => ms ? `updated ${new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : "";
  if (provider === "copilot") {
    return state.quota.flatMap((quota) => {
      const prefix = quota.login ? quota.login + " · " : "";
      if (quota.error) return [prefix + "quota unavailable"];
      const reset = quota.reset_date ? `renews ${quota.reset_date}` : "reset unavailable";
      return quota.buckets.filter((b) => b.unlimited || b.entitlement > 0).map((bucket) => {
        const label = bucket.id.replaceAll("_", " ");
        const remaining = bucket.unlimited ? "unlimited" : `${bucket.percent_remaining.toFixed(0)}% left`;
        return [prefix + label + " " + remaining, reset, stamp(quota.fetched_at_ms)].filter(Boolean).join(" · ");
      });
    });
  }
  const local = (state.snap?.rate_limits || []).find((r) => String(r.source).toLowerCase() === provider);
  const remote = provider === "claude" && !local ? state.claude : null;
  const limits = local || remote?.limits;
  if (!limits) return [];
  const windows = [
    [fmtDur((limits.five_hour_window_minutes || 300) * 60), limits.five_hour_pct, limits.five_hour_resets_at],
    [fmtDur((limits.seven_day_window_minutes || 10080) * 60), limits.seven_day_pct, limits.seven_day_resets_at],
    ...(remote?.extra || []).map((q) => [q.label, q.pct, q.resets_at]),
  ];
  return windows.filter(([, pct]) => pct != null).map(([label, pct, reset]) =>
    [`${label} · ${Math.max(0, Math.min(100, 100 - pct)).toFixed(0)}% left`, quotaReset(reset), stamp(remote?.fetched_at_ms)].filter(Boolean).join(" · "));
}

function updateUsageQuotas() {
  for (const node of $("usage-rows").querySelectorAll(".usage-quota")) {
    const labels = usageQuotaLabels(node.dataset.provider);
    node.innerHTML = (labels.length ? labels : ["Live quota unavailable"]).map((label) => `<span>${esc(label)}</span>`).join("");
  }
}

// ------------------------------------------------------------------ export
function exportOpen() {
  return !$("export-modal").classList.contains("hidden");
}

function closeExportModal() {
  $("export-modal").classList.add("hidden");
}

async function openExportModal(opts = {}) {
  // Sync initial period with main toolbar if not already customized
  if (!exportState.report) {
    $("exp-period").value = $("usage-period").value;
    $("exp-from").value = $("usage-from").value;
    $("exp-through").value = $("usage-through").value;
    $("exp-dates-row").classList.toggle("hidden", $("exp-period").value !== "custom");
  }
  if (opts.preset) {
    exportState.preset = opts.preset;
  } else if (usageView() === "projects") {
    exportState.preset = "project_tokens";
  } else if (usageView() === "providers") {
    exportState.preset = "provider_stats";
  }
  if (opts.project !== undefined) {
    exportState.selectedProject = opts.project;
  }
  $("export-modal").classList.remove("hidden");
  $("exp-status").textContent = "";
  $("exp-reveal").classList.add("hidden");
  applyExportPresetUI();
  await ensureExportData();
  populateExportControls(opts);
  buildExportPreview();
}

function applyExportPresetUI() {
  const preset = exportState.preset;
  const meta = EXPORT_PRESET_META[preset] || EXPORT_PRESET_META.project_tokens;
  for (const b of $("exp-nav").querySelectorAll("button[data-preset]")) {
    b.classList.toggle("on", b.dataset.preset === preset);
  }
  $("exp-heading").textContent = meta.heading;
  $("exp-desc").textContent = meta.desc;
  $("exp-period-group").classList.toggle("hidden", !meta.period);
  $("exp-project-group").classList.toggle("hidden", !meta.project);
  $("exp-providers-group").classList.toggle("hidden", !meta.providers);
  $("exp-group-row").classList.toggle("hidden", !meta.grouping);
  if (preset === "project_tokens") {
    $("exp-prov-title").textContent = "Providers that worked on this project";
    $("exp-prov-hint").textContent = "Only providers with recorded activity on the selected project(s) are listed.";
  } else {
    $("exp-prov-title").textContent = "Providers to include";
    $("exp-prov-hint").textContent = "Select one or more AI providers to include in the export.";
  }
}

async function ensureExportData() {
  const meta = EXPORT_PRESET_META[exportState.preset];
  if (!meta.period) return;
  const range = exportRange();
  if (!range) {
    $("exp-status").textContent = "Choose a valid date range.";
    return;
  }
  const key = `${range.start}:${range.end}`;
  $("exp-status").textContent = "Loading records…";
  try {
    if (exportState.rangeKey !== key || !exportState.report) {
      const mainRange = usageRange();
      if (usageState.report && mainRange && mainRange.start === range.start && mainRange.end === range.end) {
        exportState.report = usageState.report;
      } else {
        exportState.report = await invoke("get_usage_report", range);
      }
      exportState.daily = null;
      exportState.rangeKey = key;
    }
    if (exportState.preset === "daily_timeline" && !exportState.daily) {
      exportState.daily = await invoke("get_usage_daily", { start: range.start, end: range.end });
    }
    $("exp-status").textContent = "";
  } catch (err) {
    $("exp-status").textContent = String(err || "Failed to load export data.");
  }
}

function getExportBaseRows() {
  return (exportState.report?.rows || []).filter((r) => !isHidden(r.provider));
}

function populateExportControls(opts = {}) {
  const rows = getExportBaseRows();
  // 1. Populate project selector
  const projectMap = new Map();
  for (const row of rows) {
    const key = row.project || "";
    if (!projectMap.has(key)) projectMap.set(key, []);
    projectMap.get(key).push(row);
  }
  const sortedProjects = [...projectMap.entries()].sort(([, a], [, b]) => {
    const tokA = TOKEN_FIELDS.reduce((s, f) => s + usageTotals(a).tokens[f], 0);
    const tokB = TOKEN_FIELDS.reduce((s, f) => s + usageTotals(b).tokens[f], 0);
    return tokB - tokA;
  });
  const projSelect = $("exp-project");
  const prevProj = opts.project !== undefined ? opts.project : exportState.selectedProject;
  projSelect.innerHTML = `<option value="__ALL__">All projects (${sortedProjects.length})</option>` +
    sortedProjects.map(([key, pRows]) => {
      const { tokens } = usageTotals(pRows);
      const tot = TOKEN_FIELDS.reduce((s, f) => s + tokens[f], 0);
      const provCount = new Set(pRows.map((r) => r.provider)).size;
      const label = `${projectShortName(key)} — ${fmtTok(tot)} tok · ${provCount} prov`;
      return `<option value="${esc(key)}">${esc(label)}</option>`;
    }).join("");
  if (prevProj !== "__ALL__" && projectMap.has(prevProj)) {
    projSelect.value = prevProj;
    exportState.selectedProject = prevProj;
  } else {
    projSelect.value = "__ALL__";
    exportState.selectedProject = "__ALL__";
  }

  // 2. Populate provider checkboxes based on preset & selected project
  populateExportProviders(opts);
}

function populateExportProviders(opts = {}) {
  const rows = getExportBaseRows();
  const preset = exportState.preset;
  const selectedProj = exportState.selectedProject;
  const scopedRows = (preset === "project_tokens" || preset === "daily_timeline") && selectedProj !== "__ALL__"
    ? rows.filter((r) => (r.project || "") === selectedProj)
    : rows;

  const provRows = new Map();
  for (const row of scopedRows) {
    if (!provRows.has(row.provider)) provRows.set(row.provider, []);
    provRows.get(row.provider).push(row);
  }

  // For project_tokens with a project selected (or in general), only show providers that worked on it
  let candidateProviders;
  if (preset === "project_tokens") {
    candidateProviders = [...provRows.keys()];
  } else {
    const enabled = Object.keys(PROVIDERS).filter((p) => !isHidden(p));
    candidateProviders = [...new Set([...enabled, ...provRows.keys()])];
  }

  if (opts.provider) {
    exportState.checkedProviders = new Set([opts.provider]);
  } else if (opts.project !== undefined || exportState.checkedProviders.size === 0) {
    exportState.checkedProviders = new Set(candidateProviders);
  } else {
    // Keep existing selection if valid, otherwise select all
    const valid = candidateProviders.filter((p) => exportState.checkedProviders.has(p));
    exportState.checkedProviders = new Set(valid.length ? valid : candidateProviders);
  }

  const box = $("exp-providers");
  if (!candidateProviders.length) {
    box.innerHTML = `<span class="empty">No provider activity recorded in this period.</span>`;
    return;
  }

  box.innerHTML = candidateProviders.map((prov) => {
    const pGroup = provRows.get(prov) || [];
    const { tokens, requests, activities, charges } = usageTotals(pGroup);
    const tot = TOKEN_FIELDS.reduce((s, f) => s + tokens[f], 0);
    const checked = exportState.checkedProviders.has(prov);
    const details = [];
    if (requests) details.push(`${fmtTok(tot)} tok (${fmtNum(requests)} req)`);
    if (activities) details.push(`${fmtNum(activities)} act`);
    if (charges.length) details.push(formatChargesText(charges));
    const sub = details.join(" · ") || "0 tokens in period";
    return `<label class="exp-prov-item ${checked ? "" : "off"}" ${idStyle(storageColorId(prov))}>
      <input type="checkbox" data-exp-prov="${esc(prov)}" ${checked ? "checked" : ""} />
      <span class="badge">${idTag(storageColorId(prov))}</span>
      <span class="meta"><b>${esc(usageProviderName(prov))}</b><small title="${esc(sub)}">${esc(sub)}</small></span>
    </label>`;
  }).join("");
}

function aggregateUsageRows(rows, mode, includeDate) {
  const map = new Map();
  for (const r of rows) {
    const projPath = r.project || "";
    const projName = projectShortName(projPath);
    let keyParts;
    if (mode === "summary") {
      keyParts = exportState.preset === "provider_stats"
        ? [includeDate ? r.date : "", r.provider]
        : [includeDate ? r.date : "", projPath];
    } else if (mode === "provider") {
      keyParts = [includeDate ? r.date : "", projPath, r.provider];
    } else {
      keyParts = [includeDate ? r.date : "", projPath, r.provider, r.model || "", r.account || "", r.source || ""];
    }
    const key = JSON.stringify(keyParts);
    if (!map.has(key)) {
      map.set(key, {
        date: includeDate ? r.date : undefined,
        project: projName,
        project_path: projPath || "(unassigned)",
        provider: r.provider,
        providersSet: new Set(),
        model: mode === "model" ? (r.model || "") : undefined,
        account: mode === "model" ? (r.account || "") : undefined,
        source: mode === "model" ? (USAGE_SOURCES[r.source] || r.source || "") : undefined,
        rawRows: [],
      });
    }
    const entry = map.get(key);
    entry.providersSet.add(usageProviderName(r.provider));
    entry.rawRows.push(r);
  }

  return [...map.values()].map((item) => {
    const { tokens, requests, activities, charges } = usageTotals(item.rawRows);
    const totalTokens = TOKEN_FIELDS.reduce((s, f) => s + tokens[f], 0);
    const out = {};
    if (includeDate) out.date = item.date;
    if (exportState.preset !== "provider_stats" || mode !== "summary") {
      out.project = item.project;
      out.project_path = item.project_path;
    }
    if (mode === "summary" && exportState.preset !== "provider_stats") {
      out.providers = [...item.providersSet].join(", ");
    } else {
      out.provider = usageProviderName(item.provider);
      out.provider_id = item.provider;
    }
    if (mode === "model") {
      out.model = item.model || "unknown";
      out.account = item.account || "";
      out.source = item.source || "";
    }
    out.requests = requests;
    out.input_tokens = tokens.input;
    out.output_tokens = tokens.output;
    out.cache_read_tokens = tokens.cache_read;
    out.cache_write_tokens = tokens.cache_write;
    out.total_tokens = totalTokens;
    if (activities) out.gemini_activities = activities;
    out.charges = formatChargesText(charges);
    return out;
  }).sort((a, b) => {
    if (includeDate && a.date !== b.date) return (b.date || "").localeCompare(a.date || "");
    return (b.total_tokens || 0) - (a.total_tokens || 0);
  });
}

function buildBillingQuotaRows() {
  const baseRows = getExportBaseRows().filter((r) => exportState.checkedProviders.has(r.provider));
  const out = [];
  for (const r of baseRows) {
    for (const c of r.charges || []) {
      out.push({
        record_type: "billing_charge",
        provider: usageProviderName(r.provider),
        account: r.account || "",
        project: projectShortName(r.project || ""),
        model: r.model || "",
        charge_kind: c.kind === "api" ? "API" : "Plan extra",
        currency: c.currency,
        amount: Number((c.amount_micros / 1e6).toFixed(6)),
        quota_window: "",
        percent_remaining: "",
        reset_info: "",
      });
    }
  }
  if (exportState.checkedProviders.has("copilot")) {
    for (const q of state.quota || []) {
      for (const b of q.buckets || []) {
        if (!b.unlimited && !(b.entitlement > 0)) continue;
        out.push({
          record_type: "live_quota",
          provider: "GitHub Copilot",
          account: q.login || "",
          project: "",
          model: b.id,
          charge_kind: q.plan || "",
          currency: "",
          amount: "",
          quota_window: "monthly",
          percent_remaining: b.unlimited ? "unlimited" : Number(b.percent_remaining.toFixed(1)),
          reset_info: q.reset_date || "",
        });
      }
    }
  }
  for (const prov of ["claude", "codex"]) {
    if (!exportState.checkedProviders.has(prov)) continue;
    const labels = usageQuotaLabels(prov);
    for (const lbl of labels) {
      out.push({
        record_type: "live_quota",
        provider: usageProviderName(prov),
        account: "",
        project: "",
        model: "",
        charge_kind: "",
        currency: "",
        amount: "",
        quota_window: lbl,
        percent_remaining: "",
        reset_info: "",
      });
    }
  }
  return out;
}

function buildLiveSessionRows() {
  const sessions = state.snap?.sessions || [];
  const rows = sessions.map((s) => ({
    section: "live_session",
    provider: provName(s.agent_cli),
    status: s.status || "",
    project: s.project_name || projectShortName(s.cwd || ""),
    cwd: s.cwd || "",
    branch: s.git_branch || "",
    model: s.model || "",
    account: s.copilot?.account || "",
    pid: s.pid ?? "",
    context_pct: s.context_window_pct != null ? Number(s.context_window_pct.toFixed(1)) : "",
    input_tokens: s.tokens?.input || 0,
    output_tokens: s.tokens?.output || 0,
    cache_read_tokens: s.tokens?.cache_read || 0,
    cache_write_tokens: s.tokens?.cache_write || 0,
    total_tokens: (s.tokens?.input || 0) + (s.tokens?.output || 0) + (s.tokens?.cache_read || 0) + (s.tokens?.cache_write || 0),
    copilot_premium_requests: s.copilot?.premium_requests ?? "",
    copilot_ai_credits: s.copilot?.ai_credits ?? "",
    listening_ports: (s.ports || []).join(", "),
    storage_bytes: "",
  }));
  for (const p of state.storage?.providers || []) {
    if (isHidden(p.provider)) continue;
    rows.push({
      section: "disk_storage",
      provider: PROVIDERS[p.provider]?.name || p.provider,
      status: "",
      project: "",
      cwd: p.locations.map((l) => l.path).join("; "),
      branch: "",
      model: "",
      account: "",
      pid: "",
      context_pct: "",
      input_tokens: "",
      output_tokens: "",
      cache_read_tokens: "",
      cache_write_tokens: "",
      total_tokens: "",
      copilot_premium_requests: "",
      copilot_ai_credits: "",
      listening_ports: "",
      storage_bytes: p.total_bytes,
    });
  }
  return rows;
}

function rowsToCsv(rows) {
  if (!rows.length) return "";
  const headers = Object.keys(rows[0]);
  const escCsv = (val) => {
    const str = val == null ? "" : String(val);
    return /[",\r\n]/.test(str) ? `"${str.replace(/"/g, '""')}"` : str;
  };
  const lines = [headers.map(escCsv).join(",")];
  for (const row of rows) {
    lines.push(headers.map((h) => escCsv(row[h])).join(","));
  }
  return lines.join("\n") + "\n";
}

function rowsToMarkdown(title, periodText, summaryText, rows) {
  const lines = [
    `# ${title}`,
    "",
    `- **Generated**: ${new Date().toISOString()}`,
    periodText ? `- **Period**: ${periodText}` : null,
    summaryText ? `- **Summary**: ${summaryText}` : null,
    "",
  ].filter((x) => x !== null);
  if (!rows.length) {
    lines.push("_No matching records._");
    return lines.join("\n") + "\n";
  }
  const headers = Object.keys(rows[0]);
  const escMd = (v) => String(v == null ? "" : v).replace(/\|/g, "\\|").replace(/\r?\n/g, " ");
  lines.push("| " + headers.join(" | ") + " |");
  lines.push("| " + headers.map(() => "---").join(" | ") + " |");
  for (const row of rows) {
    lines.push("| " + headers.map((h) => escMd(row[h])).join(" | ") + " |");
  }
  return lines.join("\n") + "\n";
}

function buildExportPreview() {
  const preset = exportState.preset;
  const format = exportState.format;
  const mode = $("exp-breakdown").value;
  const range = exportRange();
  const periodLabel = range ? `${range.fromDate} → ${range.throughDate}` : "live";
  let exportRows = [];
  let filteredRaw = [];

  if (preset === "project_tokens" || preset === "provider_stats") {
    filteredRaw = getExportBaseRows().filter((r) => {
      if (!exportState.checkedProviders.has(r.provider)) return false;
      if (preset === "project_tokens" && exportState.selectedProject !== "__ALL__") {
        return (r.project || "") === exportState.selectedProject;
      }
      return true;
    });
    exportRows = aggregateUsageRows(filteredRaw, mode, false);
  } else if (preset === "daily_timeline") {
    filteredRaw = (exportState.daily || []).filter((r) => {
      if (!exportState.checkedProviders.has(r.provider)) return false;
      if (exportState.selectedProject !== "__ALL__") {
        return (r.project || "") === exportState.selectedProject;
      }
      return true;
    });
    exportRows = aggregateUsageRows(filteredRaw, mode, true);
  } else if (preset === "billing_quota") {
    exportRows = buildBillingQuotaRows();
  } else if (preset === "live_sessions") {
    exportRows = buildLiveSessionRows();
  }

  const { tokens, requests, charges } = usageTotals(filteredRaw);
  const totalTok = TOKEN_FIELDS.reduce((s, f) => s + tokens[f], 0);
  const summaryParts = [`<b>${exportRows.length}</b> rows`];
  if (filteredRaw.length) {
    summaryParts.push(`<b>${fmtTok(totalTok)}</b> total tokens (↓${fmtTok(tokens.input)} ↑${fmtTok(tokens.output)} ⟳${fmtTok(tokens.cache_read + tokens.cache_write)})`);
    summaryParts.push(`<b>${fmtNum(requests)}</b> requests`);
    if (charges.length) summaryParts.push(`<b>${esc(formatChargesText(charges))}</b>`);
  }
  $("exp-summary").innerHTML = summaryParts.join(" · ");

  const title = EXPORT_PRESET_META[preset]?.heading || "Scrooge Export";
  const plainSummary = summaryParts.map((s) => s.replace(/<[^>]+>/g, "")).join(" | ");
  let content = "";
  if (format === "csv") {
    content = rowsToCsv(exportRows);
  } else if (format === "json") {
    content = JSON.stringify({
      exported_at: new Date().toISOString(),
      preset,
      period: range ? { from: range.fromDate, through: range.throughDate } : null,
      project_filter: preset === "project_tokens" || preset === "daily_timeline" ? exportState.selectedProject : null,
      providers_included: [...exportState.checkedProviders],
      summary: {
        rows: exportRows.length,
        requests,
        tokens: { ...tokens, total: totalTok },
        charges,
      },
      rows: exportRows,
    }, null, 2) + "\n";
  } else {
    content = rowsToMarkdown(title, range ? periodLabel : null, plainSummary, exportRows);
  }

  const projSlug = (preset === "project_tokens" && exportState.selectedProject !== "__ALL__")
    ? "-" + projectShortName(exportState.selectedProject).toLowerCase().replace(/[^a-z0-9]+/g, "-")
    : "";
  const rangeSlug = range ? `-${range.label}` : "";
  const filename = `scrooge-${preset.replace(/_/g, "-")}${projSlug}${rangeSlug}.${format}`;

  exportState.generated = { filename, content };
  $("exp-preview").value = content || "(No matching records for the selected filters)";
}

function initUsage() {
  const end = new Date();
  const start = new Date(end);
  start.setDate(start.getDate() - 6);
  $("usage-from").value = localDate(start);
  $("usage-through").value = localDate(end);
  $("exp-from").value = localDate(start);
  $("exp-through").value = localDate(end);

  $("usage-view").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-view]");
    if (!button) return;
    usageState.views[state.mode] = button.dataset.view;
    applyUsageView();
    if (usageView() === "live") render();
  });

  function rangeChanged() {
    $("usage-dates").classList.toggle("hidden", $("usage-period").value !== "custom");
    usageState.report = null;
    renderUsage();
    loadUsage();
  }
  for (const id of ["usage-period", "usage-from", "usage-through"]) $(id).addEventListener("change", rangeChanged);

  $("usage-imports-folder").onclick = () => invoke("open_usage_imports").catch((err) => {
    $("usage-status").textContent = String(err);
  });

  // Row quick-export buttons in Projects / Providers
  $("usage-rows").addEventListener("click", (event) => {
    const btn = event.target.closest(".exp-row-btn");
    if (!btn) return;
    event.preventDefault();
    event.stopPropagation();
    if (btn.dataset.expProject !== undefined) {
      openExportModal({ preset: "project_tokens", project: btn.dataset.expProject });
    } else if (btn.dataset.expProvider !== undefined) {
      openExportModal({ preset: "provider_stats", provider: btn.dataset.expProvider });
    }
  });

  // Export modal events
  $("btn-export").onclick = () => openExportModal();
  $("export-close").onclick = closeExportModal;
  $("export-modal").addEventListener("mousedown", (e) => {
    if (e.target.id === "export-modal") closeExportModal();
  });

  $("exp-nav").addEventListener("click", async (e) => {
    const b = e.target.closest("button[data-preset]");
    if (!b) return;
    exportState.preset = b.dataset.preset;
    applyExportPresetUI();
    await ensureExportData();
    populateExportControls();
    buildExportPreview();
  });

  for (const id of ["exp-period", "exp-from", "exp-through"]) {
    $(id).addEventListener("change", async () => {
      $("exp-dates-row").classList.toggle("hidden", $("exp-period").value !== "custom");
      await ensureExportData();
      populateExportControls();
      buildExportPreview();
    });
  }

  $("exp-project").addEventListener("change", (e) => {
    exportState.selectedProject = e.target.value;
    populateExportProviders({ project: exportState.selectedProject });
    buildExportPreview();
  });

  $("exp-providers").addEventListener("change", (e) => {
    const cb = e.target.closest("input[data-exp-prov]");
    if (!cb) return;
    const prov = cb.dataset.expProv;
    cb.checked ? exportState.checkedProviders.add(prov) : exportState.checkedProviders.delete(prov);
    cb.closest(".exp-prov-item")?.classList.toggle("off", !cb.checked);
    buildExportPreview();
  });

  $("exp-prov-all").onclick = () => {
    for (const cb of $("exp-providers").querySelectorAll("input[data-exp-prov]")) {
      cb.checked = true;
      exportState.checkedProviders.add(cb.dataset.expProv);
      cb.closest(".exp-prov-item")?.classList.remove("off");
    }
    buildExportPreview();
  };

  $("exp-prov-none").onclick = () => {
    exportState.checkedProviders.clear();
    for (const cb of $("exp-providers").querySelectorAll("input[data-exp-prov]")) {
      cb.checked = false;
      cb.closest(".exp-prov-item")?.classList.add("off");
    }
    buildExportPreview();
  };

  $("exp-breakdown").addEventListener("change", buildExportPreview);

  $("exp-format").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-fmt]");
    if (!b) return;
    exportState.format = b.dataset.fmt;
    for (const btn of $("exp-format").querySelectorAll("button[data-fmt]")) {
      btn.classList.toggle("on", btn.dataset.fmt === exportState.format);
    }
    buildExportPreview();
  });

  $("exp-save").onclick = async () => {
    const { filename, content } = exportState.generated;
    if (!content) {
      $("exp-status").textContent = "Nothing to export.";
      return;
    }
    try {
      const savedPath = await invoke("save_export", { filename, content });
      exportState.lastSavedPath = savedPath;
      $("exp-status").textContent = `Saved: ${savedPath}`;
      $("exp-reveal").classList.remove("hidden");
    } catch (err) {
      $("exp-status").textContent = "Export failed: " + err;
    }
  };

  $("exp-reveal").onclick = () => {
    if (exportState.lastSavedPath) {
      invoke("reveal", { path: exportState.lastSavedPath }).catch(() => {});
    }
  };

  $("exp-copy").onclick = async () => {
    const { content } = exportState.generated;
    if (!content) return;
    try {
      await navigator.clipboard.writeText(content);
      $("exp-status").textContent = "Copied to clipboard!";
    } catch (err) {
      $("exp-status").textContent = "Clipboard error: " + err;
    }
  };

  setInterval(() => { if (!document.hidden) loadUsage(); }, 60_000);
}
