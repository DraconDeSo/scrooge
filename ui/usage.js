"use strict";

const usageState = {
  views: { compact: "live", full: "projects" },
  report: null,
  fetchedAt: 0,
  pending: false,
  queued: false,
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

function usageView() { return usageState.views[state.mode]; }
function usageProviderName(provider) { return provider === "gemini" ? "Google" : provName(provider); }

function localDate(date) {
  const two = (n) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())}`;
}

function usageRange() {
  const range = (start, end) => {
    const utcDay = (date) => Date.UTC(date.getFullYear(), date.getMonth(), date.getDate());
    return { start: +start, end: +end, billingStart: utcDay(start), billingEnd: utcDay(end) };
  };
  const period = $("usage-period").value;
  if (period === "custom") {
    if (!$("usage-from").value || !$("usage-through").value) return null;
    const start = new Date($("usage-from").value + "T00:00:00");
    const end = new Date($("usage-through").value + "T00:00:00");
    end.setDate(end.getDate() + 1);
    return +end > +start ? range(start, end) : null;
  }
  const end = new Date();
  end.setHours(0, 0, 0, 0);
  end.setDate(end.getDate() + 1);
  const start = new Date(end);
  start.setDate(start.getDate() - (period === "today" ? 1 : Number(period)));
  return range(start, end);
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
    for (const charge of row.charges) {
      const key = charge.kind + ":" + charge.currency;
      const sum = charges.get(key) || { ...charge, amount_micros: 0 };
      sum.amount_micros += charge.amount_micros;
      charges.set(key, sum);
    }
  }
  return { tokens, requests, activities, charges: [...charges.values()] };
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
    const label = view === "providers" ? usageProviderName(key) : key.split(/[\\/]/).filter(Boolean).pop() || "Without a project";
    const entryKey = view + ":" + key;
    const quotas = view === "providers" ? `<span class="usage-quota" data-provider="${esc(key)}"></span>` : "";
    return `<details class="usage-entry" data-key="${esc(entryKey)}" ${open.has(entryKey) ? "open" : ""}>
      <summary><b title="${esc(view === "projects" ? key : label)}">${esc(label)}</b>${usageMetrics(group, false)}${quotas}</summary>
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

function initUsage() {
  const end = new Date();
  const start = new Date(end);
  start.setDate(start.getDate() - 6);
  $("usage-from").value = localDate(start);
  $("usage-through").value = localDate(end);
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
  setInterval(() => { if (!document.hidden) loadUsage(); }, 60_000);
}
