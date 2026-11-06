"use strict";

const T = window.__TAURI__;
const invoke = T.core.invoke;
const listen = T.event.listen;

// ------------------------------------------------------------------ prefs
const PROVIDERS = {
  claude: { name: "Claude Code", tag: "CC" },
  codex: { name: "Codex", tag: "CD" },
  opencode: { name: "OpenCode", tag: "OC" },
  copilot: { name: "GitHub Copilot", tag: "GH" },
  gemini: { name: "Gemini CLI", tag: "GM" },
  grok: { name: "Grok Build", tag: "GK" },
};
const UI_DEFAULTS = {
  theme: "default",
  scheme: "dark",          // dark | light | system
  colorOverrides: {},      // provider -> #rrggbb, on top of the theme palette
  density: "normal",
  warn: 80,
  crit: 90,
  sort: "recent",
  showIdle: true,
  showChat: true,
  showTask: true,
  cpBuckets: "premium",
  claudeLocal: true,
  rateGroup: "provider",
  rateHidden: { provider: [], project: [] },
  creditUnit: "credits",   // credits | usd
  usdPerCredit: 0.01,      // price of one Copilot AI credit, editable in Settings
};

const state = {
  snap: null,
  quota: [],        // Copilot quota, one entry per account
  claude: null,     // Claude OAuth usage (5h / 7d %)
  local: null,      // Claude local consumption from transcripts
  settings: null,   // backend settings view
  ui: structuredClone(UI_DEFAULTS),
  acctColors: new Map(),   // copilot@login -> color
  mode: "compact",
  onTop: true,
  selected: null,
  expanded: null,
  filter: "",
  rate: [],
  sessionOrder: new Map(),
  sessionSort: null,
  nextSessionRank: 0,
};
const RATE_POINTS = 200;
const $ = (id) => document.getElementById(id);

// ------------------------------------------------------------- formatting
function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}
function fmtTok(n) {
  n = Number(n) || 0;
  if (n >= 1e9) return (n / 1e9).toFixed(1) + "B";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1) + "k";
  return String(Math.round(n));
}
function fmtNum(n, d = 0) {
  return Number(n || 0).toLocaleString("en-US", { maximumFractionDigits: d });
}
function fmtDur(secs) {
  secs = Math.max(0, Math.floor(secs));
  if (secs < 60) return secs + "s";
  if (secs < 3600) return Math.floor(secs / 60) + "m";
  if (secs < 86400) return Math.floor(secs / 3600) + "h " + Math.floor((secs % 3600) / 60) + "m";
  return Math.floor(secs / 86400) + "d " + Math.floor((secs % 86400) / 3600) + "h";
}
function untilEpoch(sec) {
  return sec ? fmtDur(sec - Date.now() / 1000) : "";
}
function pvKey(cli) { return PROVIDERS[cli] ? cli : "other"; }
function pv(cli) { return "pv-" + pvKey(cli); }
function provName(cli) { return PROVIDERS[cli]?.name || cli; }
function level(pct) {
  return pct >= state.ui.crit ? "lvl-crit" : pct >= state.ui.warn ? "lvl-warn" : "";
}
function bar(pct, kind = "") {
  const p = Math.max(0, Math.min(100, pct || 0));
  const lv = kind ? "" : pct >= state.ui.crit ? "crit" : pct >= state.ui.warn ? "warn" : "";
  return `<div class="bar ${kind} ${lv}"><i style="width:${p.toFixed(1)}%"></i></div>`;
}
function pctText(pct, digits = 0) {
  const warn = pct >= state.ui.crit ? " ⚠" : "";
  return `<span class="${level(pct)}">${(pct || 0).toFixed(digits)}%${warn}</span>`;
}
const SURFACE = { Cli: "", App: "·app", Ide: "·ide" };
const STATUS = { Thinking: "think", Executing: "exec", Waiting: "wait", RateLimited: "rate", Unknown: "unknown", Done: "done" };
function keyOf(s) { return s.agent_cli + ":" + s.session_id; }
function liveSessions() {
  return (state.snap?.sessions || []).filter((s) =>
    s.agent_cli !== "codex" || s.model?.toLowerCase() !== "codex-auto-review");
}
function shortModel(m) { return String(m || "-").replace(/^claude-/, ""); }
/** Disabled in Settings → Providers (the backend stops collecting it too). */
function isHidden(cli) {
  const off = state.settings?.settings?.disabled_providers || [];
  return off.some((p) => p.toLowerCase() === pvKey(cli));
}
function fmtBytes(n) {
  n = Number(n) || 0;
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return (i ? n.toFixed(n < 10 ? 2 : n < 100 ? 1 : 0) : n) + " " + u[i];
}
function homeAbbrev(p) {
  return String(p).replace(/^[A-Za-z]:\\Users\\[^\\]+/, "~").replace(/^\/(Users|home)\/[^/]+/, "~");
}
function storageColorId(provider) {
  if (provider !== "copilot") return provider;
  const a = cliAccount();
  return a ? "copilot@" + a : copilotIds()[0] || "copilot";
}
function cliAccount() { return state.quota.find((q) => q.is_cli)?.login || null; }

// ----------------------------------------------------------- identities
// An "identity" is what gets a color: a provider, or — for GitHub Copilot —
// each account separately ("copilot@login"). Live Copilot sessions belong to
// the account the Copilot CLI/app is logged in with.
function idOf(s) {
  if (s.agent_cli === "copilot") { const a = cliAccount(); return a ? "copilot@" + a : "copilot"; }
  return pvKey(s.agent_cli);
}
function idName(id) {
  return id.startsWith("copilot@") ? "Copilot · " + id.slice(8) : PROVIDERS[id]?.name || id;
}
function idTag(id) {
  return id.startsWith("copilot@") ? "GH" : PROVIDERS[id]?.tag || id.slice(0, 2).toUpperCase();
}
function idColor(id) {
  const o = state.ui.colorOverrides?.[id];
  if (o) return o;
  if (id.startsWith("copilot@")) return state.acctColors.get(id) || providerColor("copilot");
  return providerColor(id);
}
function idStyle(id) { return `style="--pc:${idColor(id)}"`; }
function copilotIds() {
  const logins = state.quota.map((q) => q.login).filter(Boolean);
  return logins.map((l) => "copilot@" + l);
}
function allIds() { return [...Object.keys(PROVIDERS).filter((k) => k !== "copilot"), ...(copilotIds().length ? copilotIds() : ["copilot"])]; }

function hexToHsl(hex) {
  const [r, g, b] = hexRgb(hex).map((v) => v / 255);
  const max = Math.max(r, g, b), min = Math.min(r, g, b), l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min, s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h * 60, s, l];
}
function hslToHex(h, s, l) {
  const f = (n) => { const k = (n + h / 30) % 12, a = s * Math.min(l, 1 - l); return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1)))); };
  return "#" + [f(0), f(8), f(4)].map((v) => v.toString(16).padStart(2, "0")).join("");
}
/** First account keeps the theme's Copilot color; others get hue-shifted
 *  variants that stay clearly distinct from every other identity. */
function computeAccountColors() {
  state.acctColors = new Map();
  const ids = copilotIds();
  if (!ids.length) return;
  const base = providerColor("copilot");
  const used = Object.keys(PROVIDERS).filter((k) => k !== "copilot").map((k) => providerColor(k));
  const [h, s, l] = hexToHsl(base);
  ids.forEach((id, i) => {
    if (state.ui.colorOverrides?.[id]) { used.push(state.ui.colorOverrides[id]); return; }
    let pick = base;
    if (i > 0 || used.some((c) => colorDistance(c, base) < 110)) {
      let best = null, bestScore = -1;
      for (const off of [40, -40, 80, -80, 120, -120, 160, -160, 200]) {
        for (const dl of [0, 0.12, -0.12]) {
          const c = hslToHex((h + off + 360) % 360, s, Math.min(0.8, Math.max(0.3, l + dl)));
          const score = Math.min(...used.map((u) => colorDistance(u, c)));
          if (score > bestScore) { bestScore = score; best = c; }
          if (score >= 130) break;
        }
        if (bestScore >= 130) break;
      }
      pick = best;
    }
    used.push(pick);
    state.acctColors.set(id, pick);
  });
}
function fmtCredits(cr) {
  cr = Number(cr) || 0;
  if (state.ui.creditUnit === "usd") {
    const equivalent = cr * (Number(state.ui.usdPerCredit) || 0);
    return "≈$" + equivalent.toFixed(equivalent < 10 ? 2 : 0);
  }
  return fmtNum(cr, cr < 10 ? 2 : 0) + " cr";
}

// ------------------------------------------------------------ appearance
const THEMES = [
  ["default", "Default"], ["fallout", "Fallout"], ["starwars", "Star Wars"], ["forest", "Forest"], ["nordic", "Nordic"],
  ["oldpaper", "Old Paper"], ["cyberpunk", "Cyberpunk"], ["fantasy", "Fantasy"], ["kawaii", "Kawaii"], ["developer", "Developer"],
];
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
function effectiveScheme() {
  return state.ui.scheme === "system" ? (darkQuery.matches ? "dark" : "light") : state.ui.scheme === "light" ? "light" : "dark";
}
function applyUi() {
  const html = document.documentElement;
  html.dataset.theme = THEMES.some(([k]) => k === state.ui.theme) ? state.ui.theme : "default";
  html.dataset.scheme = effectiveScheme();
  // provider colors: theme palette unless the user picked an override
  for (const k of Object.keys(PROVIDERS)) {
    const o = state.ui.colorOverrides?.[k];
    if (o) html.style.setProperty("--p-" + k, o); else html.style.removeProperty("--p-" + k);
  }
  document.body.classList.remove("density-compact", "density-normal", "density-comfortable");
  document.body.classList.add("density-" + state.ui.density);
  computeAccountColors();
}
darkQuery.addEventListener("change", () => { if (state.ui.scheme === "system") { applyUi(); render(); } });
/** Effective color of a provider in the current theme (override or theme default). */
function providerColor(k) {
  return getComputedStyle(document.documentElement).getPropertyValue("--p-" + k).trim() || "#8b949e";
}
function cssVar(name, fallback) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}
function loadUi(ui) {
  const u = structuredClone(UI_DEFAULTS);
  if (ui && typeof ui === "object") {
    for (const k of Object.keys(UI_DEFAULTS)) if (ui[k] !== undefined) u[k] = ui[k];
  }
  state.ui = u;
  applyUi();
}
async function saveUi(patch) {
  Object.assign(state.ui, patch);
  applyUi();
  render();
  try { state.settings = await invoke("update_settings", { patch: { ui: patch } }); } catch (e) { console.error(e); }
}

// ---------------------------------------------------------------- data
function orderedSessions() {
  const all = liveSessions();
  const now = Date.now();
  if (state.sessionSort !== state.ui.sort) {
    state.sessionSort = state.ui.sort;
    state.sessionOrder.clear();
    state.nextSessionRank = 0;
  }
  const keys = new Set(all.map(keyOf));
  for (const [key, entry] of state.sessionOrder) {
    if (!keys.has(key) && now - entry.lastSeen > 60_000) {
      state.sessionOrder.delete(key);
      if (state.selected === key) state.selected = null;
      if (state.expanded === key) state.expanded = null;
    }
  }
  const act = (s) => (s.status === "Executing" ? 0 : s.status === "Thinking" ? 1 : s.status === "RateLimited" ? 2 : s.status === "Waiting" ? 3 : 4);
  const sorters = {
    recent: (a, b) => b.started_at_ms - a.started_at_ms,
    activity: (a, b) => act(a) - act(b) || b.started_at_ms - a.started_at_ms,
    provider: (a, b) => a.agent_cli.localeCompare(b.agent_cli) || b.started_at_ms - a.started_at_ms,
    context: (a, b) => (b.context_percent || 0) - (a.context_percent || 0),
    tokens: (a, b) => b.total_tokens - a.total_tokens,
  };
  const sorter = sorters[state.ui.sort] || sorters.recent;
  const added = all.filter((s) => !state.sessionOrder.has(keyOf(s)));
  added.sort((a, b) => sorter(a, b) || keyOf(a).localeCompare(keyOf(b)));
  for (const s of added) state.sessionOrder.set(keyOf(s), { rank: state.nextSessionRank++, lastSeen: now });
  for (const s of all) state.sessionOrder.get(keyOf(s)).lastSeen = now;
  return all.sort((a, b) => state.sessionOrder.get(keyOf(a)).rank - state.sessionOrder.get(keyOf(b)).rank);
}
function visibleSessions() {
  const f = state.filter.trim().toLowerCase();
  return orderedSessions().filter((s) => {
    if (isHidden(s.agent_cli)) return false;
    if (!state.ui.showIdle && !["Thinking", "Executing", "RateLimited"].includes(s.status)) return false;
    return !f || [s.project_name, s.model, s.summary, s.agent_cli, s.git_branch, s.cwd]
      .some((v) => String(v || "").toLowerCase().includes(f));
  });
}
function currentRate() {
  const snap = state.snap;
  if (!snap) return 0;
  return (snap.token_rate || 0) / Math.max(1, (snap.interval_ms || 2000) / 1000);
}

// ---------------------------------------------------------------- render
function renderHeader() {
  const all = liveSessions().filter((s) => !isHidden(s.agent_cli));
  const active = all.filter((s) => s.status === "Thinking" || s.status === "Executing").length;
  $("pulse").classList.toggle("live", active > 0);
  $("counts").textContent = all.length ? `${all.length} session${all.length > 1 ? "s" : ""} · ${active} active` : "no agents";
  const present = [...new Set(all.map(idOf))];
  $("legend").innerHTML = present.map((id) => `<span ${idStyle(id)}>${esc(idName(id))}</span>`).join("");
  // token split across live sessions: read (input), written (output), cache
  const sum = (f) => all.reduce((a, s) => a + (s[f] || 0), 0);
  const inp = sum("input_tokens"), out = sum("output_tokens"), cr = sum("cache_read_tokens"), cw = sum("cache_create_tokens");
  const credits = all.reduce((a, s) => a + (s.copilot ? (s.copilot.nano_aiu || 0) / 1e9 : 0), 0);
  const tip = `Live sessions — read (input) ${fmtNum(inp)} · written (output) ${fmtNum(out)} · cache read ${fmtNum(cr)} · cache write ${fmtNum(cw)}`;
  const h = state.snap?.host;
  $("vitals").title = tip;
  $("vitals").innerHTML = [
    `<span title="read: input tokens">↓<b>${fmtTok(inp)}</b><span class="v-opt"> in</span></span>`,
    `<span title="written: output tokens">↑<b>${fmtTok(out)}</b><span class="v-opt"> out</span></span>`,
    `<span title="cache read ${fmtNum(cr)} · cache write ${fmtNum(cw)}">⟳<b>${fmtTok(cr + cw)}</b><span class="v-opt"> cache</span></span>`,
    credits ? `<span class="pv-copilot" title="Copilot AI credits used by live sessions" style="--pc:${idColor(cliAccount() ? "copilot@" + cliAccount() : "copilot")}">◆<b class="pc">${fmtCredits(credits)}</b></span>` : "",
    `<span class="v-opt">rate <b>${fmtTok(currentRate())}/s</b></span>`,
    h ? `<span class="v-opt">cpu <b>${h.cpu_pct.toFixed(0)}%</b> mem <b>${h.mem_pct.toFixed(0)}%</b></span>` : "",
  ].join("");
}

function copilotBlocks(full) {
  const out = [];
  if (isHidden("copilot")) return out;
  const liveCp = (state.snap?.sessions || []).filter((s) => s.copilot);
  const livePremium = liveCp.reduce((a, s) => a + (s.copilot.premium_requests || 0), 0);
  if (!state.quota.length) {
    out.push(`<div class="q-group pv-copilot"><div class="q-head"><span class="src">COPILOT</span></div><div class="empty">loading…</div></div>`);
  }
  const label = { premium_interactions: "premium", chat: "chat", completions: "compl" };
  for (const q of state.quota) {
    let body = "";
    let meta = "";
    if (q.error) {
      body = `<div class="q-err">${esc(q.error)}</div>`;
    } else {
      const reset = q.reset_date ? new Date(q.reset_date + "T00:00:00").toLocaleDateString("en-GB", { day: "numeric", month: "short" }) : "";
      meta = [q.plan, reset ? "↻ " + reset : ""].filter(Boolean).join(" · ");
      const all = full || state.ui.cpBuckets === "all";
      const buckets = all ? q.buckets : q.buckets.filter((b) => b.id === "premium_interactions" || (!b.unlimited && b.entitlement > 0 && b.percent_remaining < 100));
      for (const b of buckets) {
        const name = label[b.id] || b.id;
        if (!b.unlimited && !b.entitlement) {
          body += `<div class="q-row"><span class="lbl">${esc(name)}</span>${bar(0, "off")}<span class="val"><small>not included</small></span></div>`;
        } else if (b.unlimited) {
          const used = b.credits_used ? `${fmtCredits(b.credits_used)} <small>used</small>` : "∞";
          body += `<div class="q-row"><span class="lbl">${esc(name)}</span>${bar(100, "unl")}<span class="val" title="Unlimited">${used}</span></div>`;
        } else {
          const usedPct = 100 - (b.percent_remaining || 0);
          const over = b.overage_count ? ` <small>+${fmtNum(b.overage_count)}</small>` : "";
          body += `<div class="q-row"><span class="lbl">${esc(name)}</span>${bar(usedPct)}<span class="val" title="${usedPct.toFixed(0)}% used"><span class="${level(usedPct)}">${fmtNum(b.remaining)}</span><small>/${fmtNum(b.entitlement)} left</small>${over}</span></div>`;
        }
      }
      if (!buckets.length) body = `<div class="empty">no quota data</div>`;
    }
    if (q.is_cli && liveCp.length) {
      body += `<div class="q-note ell">live: <b>${fmtNum(livePremium, 2)}</b> premium req in ${liveCp.length} session${liveCp.length > 1 ? "s" : ""}</div>`;
    }
    const tag = q.is_cli ? `<span class="tag" title="Account used by the Copilot CLI / app">CLI</span>` : "";
    out.push(`<div class="q-group" ${idStyle("copilot@" + q.login)}><div class="q-head"><span class="src">COPILOT</span><span class="acct" title="${esc(q.login)}">${esc(q.login || "?")}</span>${tag}<span class="meta" title="${esc(meta)}">${esc(meta)}</span></div>${body}</div>`);
  }
  return out;
}

function rateRows(rl) {
  const rows = [];
  if (rl.five_hour_pct != null) {
    const lbl = rl.five_hour_window_minutes && rl.five_hour_window_minutes !== 300 ? Math.round(rl.five_hour_window_minutes / 60) + "h" : "5h";
    rows.push(`<div class="q-row" title="${lbl} rate-limit window"><span class="lbl">${lbl}</span>${bar(rl.five_hour_pct)}<span class="val">${pctText(rl.five_hour_pct)} <small>${untilEpoch(rl.five_hour_resets_at)}</small></span></div>`);
  }
  if (rl.seven_day_pct != null) {
    const mins = rl.seven_day_window_minutes || 10080;
    const lbl = mins >= 1440 ? Math.round(mins / 1440) + "d" : Math.round(mins / 60) + "h";
    rows.push(`<div class="q-row" title="${lbl} rate-limit window"><span class="lbl">${lbl}</span>${bar(rl.seven_day_pct)}<span class="val">${pctText(rl.seven_day_pct)} <small>${untilEpoch(rl.seven_day_resets_at)}</small></span></div>`);
  }
  return rows;
}

function claudeBlock(full) {
  if (isHidden("claude")) return "";
  const fromFile = (state.snap?.rate_limits || []).find((r) => String(r.source).toLowerCase() === "claude");
  const cu = state.claude;
  const limits = fromFile || cu?.limits;
  const rows = limits ? rateRows(limits) : [];
  if (full && cu?.limits && !fromFile) {
    for (const x of cu.extra || []) rows.push(`<div class="q-row"><span class="lbl">${esc(x.label)}</span>${bar(x.pct)}<span class="val">${pctText(x.pct)} <small>${untilEpoch(x.resets_at)}</small></span></div>`);
  }
  const L = state.local;
  if (state.ui.claudeLocal && L && L.computed_at_ms) {
    const blockEnd = L.block_start ? L.block_start + 5 * 3600 : 0;
    const blockPct = L.block_start ? ((Date.now() / 1000 - L.block_start) / (5 * 3600)) * 100 : 0;
    rows.push(`<div class="q-row" title="Tokens in the current 5-hour block (in ${fmtTok(L.five_hour.input)} · out ${fmtTok(L.five_hour.output)} · cache ${fmtTok(L.five_hour.cache_read + L.five_hour.cache_write)})"><span class="lbl">block</span>${L.block_start ? bar(blockPct, "time") : bar(0, "off")}<span class="val"><b>${fmtTok(L.five_hour.total)}</b> <small>${L.block_start ? "ends " + untilEpoch(blockEnd) : "idle"}</small></span></div>`);
    const max = Math.max(...L.hourly, 1);
    const hist = `<div class="hist" title="Tokens per hour, last 24h">${L.hourly.map((v) => `<i style="height:${Math.max(4, (v / max) * 100).toFixed(0)}%"></i>`).join("")}</div>`;
    rows.push(`<div class="q-row" title="Last 24 hours: ${fmtNum(L.day.messages)} replies, output ${fmtTok(L.day.output)}"><span class="lbl">24h</span>${hist}<span class="val"><b>${fmtTok(L.day.total)}</b></span></div>`);
    if (full) {
      rows.push(`<div class="q-row"><span class="lbl">7d</span><span class="lbl ell">${fmtNum(L.week.messages)} replies · out ${fmtTok(L.week.output)}</span><span class="val"><b>${fmtTok(L.week.total)}</b></span></div>`);
      for (const m of (L.models || []).slice(0, 3)) {
        rows.push(`<div class="q-row"><span class="lbl ell" title="${esc(m.model)}">${esc(shortModel(m.model))}</span>${bar(L.week.total ? (m.total / L.week.total) * 100 : 0, "time")}<span class="val">${fmtTok(m.total)}</span></div>`);
      }
    }
  }
  let note = "";
  if (!limits && cu?.error) {
    note = `<div class="q-note ell" title="Plan limits unavailable: ${esc(cu.error)}">% limits: needs Claude Code CLI login · see ⚙</div>`;
  }
  if (!rows.length && !note) return "";
  const meta = [cu?.subscription, state.ui.claudeLocal && L?.computed_at_ms ? "local tokens" : ""].filter(Boolean).join(" · ");
  return `<div class="q-group pv-claude"><div class="q-head"><span class="src">CLAUDE</span><span class="meta">${esc(meta)}</span></div>${rows.join("")}${note}</div>`;
}

function renderQuota() {
  const full = state.mode === "full";
  const parts = [...copilotBlocks(full)];
  parts.push(claudeBlock(full));
  for (const rl of state.snap?.rate_limits || []) {
    const src = String(rl.source || "").toLowerCase();
    if (src === "claude" || isHidden(src)) continue;
    const rows = rateRows(rl);
    if (rows.length) parts.push(`<div class="q-group ${pv(src)}"><div class="q-head"><span class="src">${esc(src.toUpperCase())}</span></div>${rows.join("")}</div>`);
  }
  const html = parts.filter(Boolean).join("");
  $("quota").innerHTML = html || `<div class="empty">no quota sources</div>`;
  updateUsageQuotas();
}

function sparkSvg(values) {
  const v = (values || []).slice(-64);
  if (v.length < 2) return "";
  const max = Math.max(...v, 1);
  const w = 200, h = 36;
  const pts = v.map((x, i) => `${(i / (v.length - 1)) * w},${h - (x / max) * (h - 3) - 1}`).join(" ");
  return `<svg class="spark" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none"><polyline points="${pts}" fill="none" stroke="var(--pc)" stroke-width="1.5" vector-effect="non-scaling-stroke"/></svg>`;
}

function cardHtml(s) {
  const k = keyOf(s);
  const tag = PROVIDERS[s.agent_cli]?.tag || s.agent_cli.slice(0, 2).toUpperCase();
  const sel = state.selected === k ? " sel" : "";
  const ctx = s.context_percent || 0;
  const expanded = state.mode === "compact" && state.expanded === k;
  const task = state.ui.showTask ? `<div class="c-task">${esc(s.current_task)}</div>` : "";
  const id = idOf(s);
  const cr = s.copilot ? ` · <b class="pc" title="${fmtNum(s.copilot.premium_requests, 2)} premium requests">◆${fmtCredits((s.copilot.nano_aiu || 0) / 1e9)}</b>` : "";
  return `<div class="card${sel}" ${idStyle(id)} data-k="${esc(k)}">
    <div class="c-top">
      <span class="badge" title="${esc(idName(id))} · ${esc(s.launch_surface)}">${tag}${SURFACE[s.launch_surface] || ""}</span>
      <span class="c-proj" title="${esc(s.cwd)}">${esc(s.project_name)}</span>
      <span class="st st-${esc(s.status)}">${STATUS[s.status] || esc(s.status)}</span>
      <span class="c-model" title="${esc(s.model)} ${esc(s.effort)}">${esc(shortModel(s.model))}</span>
    </div>
    <div class="c-sum" title="${esc(s.summary)}">${esc(s.summary)}</div>
    <div class="c-meta">
      <div class="ctx" title="context ${ctx.toFixed(1)}% of ${fmtTok(s.context_window)}">${bar(ctx)}${pctText(ctx)}</div>
      <div class="nums" title="read (input) ${fmtNum(s.input_tokens)} · written (output) ${fmtNum(s.output_tokens)} · cache read ${fmtNum(s.cache_read_tokens)} · cache write ${fmtNum(s.cache_create_tokens)} · ${s.turn_count} turns · ${fmtDur(s.elapsed_secs)}">↓<b>${fmtTok(s.input_tokens)}</b> ↑<b>${fmtTok(s.output_tokens)}</b> ⟳<b>${fmtTok(s.cache_read_tokens + s.cache_create_tokens)}</b>${cr}</div>
    </div>
    ${task}
    ${expanded ? `<div class="det">${detailHtml(s)}</div>` : ""}
  </div>`;
}

function detailHtml(s) {
  const kv = (k, v) => `<span class="k">${k}</span><span class="v">${v}</span>`;
  const git = s.git_branch
    ? `${esc(s.git_branch)} <span style="color:var(--ok)">+${s.git_added}</span> <span style="color:var(--warn)">~${s.git_modified}</span>`
    : "—";
  let h = "";
  if (state.mode === "full") {
    h += `<div class="det-head"><span class="badge">${idTag(idOf(s))}</span><span class="t">${esc(idName(idOf(s)))} · ${esc(s.project_name)}</span></div>`;
  }
  h += `<h4>session</h4><div class="kv">
    ${kv("pid", s.pid + (s.launch_surface !== "Cli" ? ` <span style="color:var(--dim)">(${esc(s.launch_surface)})</span>` : ""))}
    ${s.copilot && cliAccount() ? kv("account", esc(cliAccount())) : ""}
    ${kv("cwd", `<a data-reveal="${esc(s.cwd)}" title="Open in Explorer">${esc(s.cwd)}</a>`)}
    ${kv("branch", git)}
    ${kv("model", esc(s.model) + (s.effort ? ` · ${esc(s.effort)}` : ""))}
    ${kv("version", esc(s.version || "—"))}
    ${kv("context", `${pctText(s.context_percent || 0, 1)} of ${fmtTok(s.context_window)}${s.compaction_count ? ` · ${s.compaction_count} compactions` : ""}`)}
    ${kv("memory", s.mem_mb + " MB")}
    ${kv("id", `<span style="color:var(--faint)" title="${esc(s.session_id)}">${esc(s.session_id)}</span>`)}
  </div>`;
  if (s.copilot) {
    const c = s.copilot;
    h += `<h4>copilot usage</h4><div class="cp-box">
      <div><b>${fmtNum(c.premium_requests, 2)}</b><small>premium req</small></div>
      <div><b>${fmtCredits((c.nano_aiu || 0) / 1e9)}</b><small>AI credits</small></div>
      <div><b><span style="color:var(--ok)">+${fmtNum(c.lines_added)}</span> <span style="color:var(--crit)">-${fmtNum(c.lines_removed)}</span></b><small>lines</small></div>
    </div>`;
  }
  h += `<h4>tokens</h4><div class="kv">
    ${kv("input", fmtTok(s.input_tokens))}
    ${kv("output", fmtTok(s.output_tokens) + (s.copilot?.reasoning_tokens ? ` <span style="color:var(--dim)">(${fmtTok(s.copilot.reasoning_tokens)} reasoning)</span>` : ""))}
    ${kv("cache read", fmtTok(s.cache_read_tokens))}
    ${kv("cache write", fmtTok(s.cache_create_tokens))}
  </div>${sparkSvg(s.token_history)}`;
  const tools = (s.tool_calls || []).slice(-10).reverse();
  if (tools.length) {
    h += `<h4>tools</h4><div class="tl">${tools.map((t) =>
      `<span class="n" title="${esc(t.name)}">${esc(t.name)}</span><span class="a" title="${esc(t.arg)}">${esc(t.arg)}</span><span class="d">${t.duration_ms ? (t.duration_ms / 1000).toFixed(1) + "s" : ""}</span>`).join("")}</div>`;
  }
  if (s.children?.length) {
    h += `<h4>children</h4>${s.children.slice(0, 12).map((c) =>
      `<div class="kid"><span class="pid">${c.pid}</span>${c.port ? `<span class="port">:${c.port}</span>` : ""}<span class="cmd" title="${esc(c.command)}">${esc(c.command)}</span></div>`).join("")}`;
  }
  if (s.subagents?.length) {
    h += `<h4>subagents</h4>${s.subagents.map((a) => `<div class="kid"><span class="cmd">${esc(a.name)}</span><span>${esc(a.status)}</span><span>${fmtTok(a.tokens)}</span></div>`).join("")}`;
  }
  const chat = state.ui.showChat ? (s.chat_messages || []).slice(-6) : [];
  if (chat.length) {
    h += `<h4>chat</h4><div class="chat">${chat.map((m) => `<div class="m ${esc(m.role)}">${esc(m.text)}</div>`).join("")}</div>`;
  }
  return h;
}

function renderSessions() {
  const list = visibleSessions();
  const box = $("sessions");
  const sel = window.getSelection();
  const compactSelecting = state.mode === "compact" && sel && sel.toString() && sel.anchorNode &&
    box.contains(sel.anchorNode) && sel.anchorNode.parentElement?.closest(".det");
  if (!compactSelecting) {
    const keys = new Set(list.map(keyOf));
    const top = box.getBoundingClientRect().top;
    const anchor = box.scrollTop > 0 && [...box.children].find((card) =>
      keys.has(card.dataset.k) && card.getBoundingClientRect().bottom > top);
    const offset = anchor ? anchor.getBoundingClientRect().top : 0;
    if (!list.length) {
      const msg = !state.snap ? "collecting…" : state.filter ? "no match" : "No running agents. Start Claude Code, Codex, OpenCode or GitHub Copilot.";
      box.innerHTML = `<div class="empty">${msg}</div>`;
    } else {
      box.innerHTML = list.map(cardHtml).join("");
      if (anchor) {
        const card = [...box.children].find((node) => node.dataset.k === anchor.dataset.k);
        if (card) box.scrollTop += card.getBoundingClientRect().top - offset;
      }
    }
  }
  if (state.mode === "full") {
    const det = $("detail");
    const selecting = sel && sel.toString() && det.contains(sel.anchorNode);
    if (!selecting) {
      const s = list.find((x) => keyOf(x) === state.selected) || list[0];
      det.className = "detail";
      if (s) det.style.setProperty("--pc", idColor(idOf(s)));
      det.innerHTML = s ? `<div class="det">${detailHtml(s)}</div>` : "";
    }
  }
}

function renderFullPanels() {
  const all = liveSessions().filter((s) => !isHidden(s.agent_cli));
  $("ctx-bars").innerHTML = all.length
    ? all.map((s, i) => `<div class="cb" ${idStyle(idOf(s))}><span class="i">S${i + 1}</span><span class="p" title="${esc(idName(idOf(s)))} · ${esc(s.cwd)}">${esc(s.project_name)}</span>${bar(s.context_percent)}<span class="pc2">${pctText(s.context_percent || 0)}</span></div>`).join("")
    : `<div class="empty">—</div>`;
  $("rate-now").textContent = fmtTok(currentRate()) + "/s";
  renderRateLegend();
  drawRate();

  // tokens: selected session or totals with a per-provider split
  const s = all.find((x) => keyOf(x) === state.selected);
  const src = s ? [s] : all;
  const sum = (f, list = src) => list.reduce((a, x) => a + (x[f] || 0), 0);
  const turns = sum("turn_count");
  const total = sum("total_tokens");
  const byProv = {};
  for (const x of all) byProv[idOf(x)] = (byProv[idOf(x)] || 0) + x.total_tokens;
  const split = s ? "" : `<div class="tok-split">${Object.entries(byProv).map(([k, v]) => `<i style="--pc:${idColor(k)};width:${total ? (v / total) * 100 : 0}%" title="${esc(idName(k))}: ${fmtTok(v)}"></i>`).join("")}</div>`;
  $("tokens").innerHTML = `<div class="ell" style="color:${s ? idColor(idOf(s)) : "var(--dim)"}">${s ? esc(idName(idOf(s))) + " · " + esc(s.project_name) : "all sessions"}</div>
    <div class="tok-big">${fmtTok(total)}</div>${split}
    <div class="tok-rows">
      <span>input</span><span>${fmtTok(sum("input_tokens"))}</span>
      <span>output</span><span>${fmtTok(sum("output_tokens"))}</span>
      <span>cache read</span><span>${fmtTok(sum("cache_read_tokens"))}</span>
      <span>cache write</span><span>${fmtTok(sum("cache_create_tokens"))}</span>
      <span>turns</span><span>${turns}${turns ? ` · ${fmtTok(total / turns)}/t` : ""}</span>
    </div>${s ? `<div ${idStyle(idOf(s))}>${sparkSvg(s.token_history)}</div>` : ""}`;

  // projects, with a colored dot per provider working in it
  const byCwd = new Map();
  for (const x of all) {
    const p = byCwd.get(x.cwd) || { s: x, provs: new Set() };
    p.provs.add(idOf(x));
    byCwd.set(x.cwd, p);
  }
  $("projects").innerHTML = byCwd.size
    ? [...byCwd.values()].map(({ s: x, provs }) => `<div class="proj"><div class="pn" title="${esc(x.cwd)}">${esc(x.project_name)}<span class="dots">${[...provs].map((k) => `<i ${idStyle(k)} title="${esc(idName(k))}"></i>`).join("")}</span></div><div class="pb">${esc(x.git_branch || "—")} ${x.git_added || x.git_modified ? `<span class="add">+${x.git_added}</span> <span class="mod">~${x.git_modified}</span>` : `<span style="color:var(--ok)">✓ clean</span>`}</div></div>`).join("")
    : `<div class="empty">—</div>`;

  const rows = [];
  for (const x of all) for (const c of x.children || []) if (c.port) rows.push(`<tr ${idStyle(idOf(x))}><td>:${c.port}</td><td class="pc">${esc(x.project_name)}</td><td title="${esc(c.command)}">${esc(c.command.split(/[\\/]/).pop())}</td></tr>`);
  const orphans = (state.snap?.orphan_ports || []).map((o) => `<tr class="orphan"><td>:${o.port}</td><td>${esc(o.project_name)}</td><td title="${esc(o.command)}">${esc(o.command.split(/[\\/]/).pop())}</td></tr>`);
  $("ports").innerHTML = (rows.length ? `<table class="ports-t">${rows.join("")}</table>` : `<div class="empty">no open ports</div>`) +
    (orphans.length ? `<div class="sub-h">ORPHAN PORTS</div><table class="ports-t">${orphans.join("")}</table>` : "");
  renderStorage();
}

// ------------------------------------------------- token rate by group
// Per-session active-token deltas (input + output + cache write, like abtop)
// are converted to tokens/s and summed per provider and per project. Both
// groupings are kept so switching is instant and keeps the history.
const PROJECT_PALETTE = ["#e8c547", "#5cc8ff", "#ff7aa8", "#7ee081", "#c39bff", "#ff9f5a", "#4fd1c5", "#f2f2f2", "#ff6b6b", "#9fb3ff", "#d4e157", "#b0855e"];
const rateSeries = { provider: new Map(), project: new Map() }; // key -> {label, color, values[]}
const prevActive = new Map();
let projectColorIdx = 0;
const projectColors = new Map();

const PROJECT_PALETTE_LIGHT = ["#b08800", "#0a6fb8", "#c2185b", "#2e7d32", "#6a3fb5", "#c45a00", "#00796b", "#37474f", "#c62828", "#3949ab", "#7c8f00", "#7a5230"];
function colorFor(group, key) {
  if (group === "provider") return idColor(key);
  if (!projectColors.has(key)) projectColors.set(key, projectColorIdx++);
  const pal = effectiveScheme() === "light" ? PROJECT_PALETTE_LIGHT : PROJECT_PALETTE;
  return pal[projectColors.get(key) % pal.length];
}

function pushRatePoints() {
  const snap = state.snap;
  const secs = Math.max(1, (snap.interval_ms || 2000) / 1000);
  const byGroup = { provider: new Map(), project: new Map() };
  const labels = { provider: new Map(), project: new Map() };
  for (const s of snap.sessions || []) {
    if (isHidden(s.agent_cli)) continue;
    const k = keyOf(s);
    const active = (s.input_tokens || 0) + (s.output_tokens || 0) + (s.cache_create_tokens || 0);
    const prev = prevActive.get(k);
    prevActive.set(k, active);
    const perSec = prev === undefined ? 0 : Math.max(0, active - prev) / secs;
    const pk = idOf(s);
    byGroup.provider.set(pk, (byGroup.provider.get(pk) || 0) + perSec);
    labels.provider.set(pk, idName(pk));
    const jk = s.cwd || s.project_name;
    byGroup.project.set(jk, (byGroup.project.get(jk) || 0) + perSec);
    labels.project.set(jk, s.project_name);
  }
  const len = state.rate.length; // already includes this tick
  for (const group of ["provider", "project"]) {
    const series = rateSeries[group];
    for (const [key, label] of labels[group]) {
      if (!series.has(key)) series.set(key, { label, values: new Array(Math.max(0, len - 1)).fill(0) });
      series.get(key).label = label;
    }
    for (const [key, ser] of series) {
      ser.values.push(byGroup[group].get(key) || 0);
      if (ser.values.length > RATE_POINTS) ser.values.shift();
      // forget series that left and have no visible history
      if (!labels[group].has(key) && ser.values.every((v) => v === 0)) series.delete(key);
    }
  }
}

function rateGroup() { return state.ui.rateGroup === "project" ? "project" : "provider"; }

function renderRateLegend() {
  const group = rateGroup();
  setSeg("rate-group", group);
  const hidden = new Set(state.ui.rateHidden?.[group] || []);
  const items = [...rateSeries[group].entries()].map(([key, ser]) => {
    const now = ser.values[ser.values.length - 1] || 0;
    return { key, label: ser.label, now, color: colorFor(group, key) };
  }).sort((a, b) => b.now - a.now || a.label.localeCompare(b.label));
  $("rate-legend").innerHTML = items.length
    ? items.map((it) => `<button class="${hidden.has(it.key) ? "off" : ""}" data-key="${esc(it.key)}" style="--sc:${it.color}" title="${esc(it.label)} — click to show/hide"><i></i><span>${esc(it.label)}</span><small>${fmtTok(it.now)}/s</small></button>`).join("")
    : `<span class="empty">no activity yet</span>`;
}

function drawRate() {
  const c = $("rate-canvas");
  const dpr = window.devicePixelRatio || 1;
  const w = c.clientWidth, h = c.clientHeight;
  if (!w || !h) return;
  if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
  }
  const g = c.getContext("2d");
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, w, h);
  g.strokeStyle = cssVar("--line", "#1b212c");
  g.lineWidth = 1;
  for (let i = 1; i < 4; i++) { const y = Math.round((h * i) / 4) + 0.5; g.beginPath(); g.moveTo(0, y); g.lineTo(w, y); g.stroke(); }

  const group = rateGroup();
  const hidden = new Set(state.ui.rateHidden?.[group] || []);
  const series = [...rateSeries[group].entries()].filter(([k]) => !hidden.has(k));
  const n = Math.max(0, ...series.map(([, s]) => s.values.length));
  if (n < 2) return;
  // stacked totals per x
  const totals = new Array(n).fill(0);
  for (const [, s] of series) s.values.forEach((v, i) => { totals[i + (n - s.values.length)] += v; });
  const max = Math.max(...totals, 1);
  const step = w / (RATE_POINTS - 1);
  const x0 = w - (n - 1) * step;
  const yOf = (v) => h - (v / max) * (h - 14) - 1;
  const base = new Array(n).fill(0);
  // biggest series at the bottom for a calmer chart
  series.sort((a, b) => b[1].values.reduce((x, y) => x + y, 0) - a[1].values.reduce((x, y) => x + y, 0));
  for (const [key, s] of series) {
    const color = colorFor(group, key);
    const off = n - s.values.length;
    const top = base.map((b, i) => b + (i >= off ? s.values[i - off] : 0));
    g.beginPath();
    top.forEach((v, i) => { const x = x0 + i * step; i ? g.lineTo(x, yOf(v)) : g.moveTo(x, yOf(v)); });
    for (let i = n - 1; i >= 0; i--) g.lineTo(x0 + i * step, yOf(base[i]));
    g.closePath();
    g.globalAlpha = 0.45;
    g.fillStyle = color;
    g.fill();
    g.globalAlpha = 1;
    g.beginPath();
    top.forEach((v, i) => { const x = x0 + i * step; i ? g.lineTo(x, yOf(v)) : g.moveTo(x, yOf(v)); });
    g.strokeStyle = color;
    g.lineWidth = 1.4;
    g.stroke();
    for (let i = 0; i < n; i++) base[i] = top[i];
  }
  g.fillStyle = cssVar("--faint", "#4b5466");
  g.font = "10px " + getComputedStyle(document.body).fontFamily;
  g.fillText("peak " + fmtTok(max) + "/s", 4, 11);
}

$("rate-group").addEventListener("click", (e) => {
  const b = e.target.closest("button[data-v]");
  if (!b) return;
  saveUi({ rateGroup: b.dataset.v });
});
$("rate-legend").addEventListener("click", (e) => {
  const b = e.target.closest("button[data-key]");
  if (!b) return;
  const group = rateGroup();
  const all = { provider: [], project: [], ...(state.ui.rateHidden || {}) };
  const set = new Set(all[group]);
  set.has(b.dataset.key) ? set.delete(b.dataset.key) : set.add(b.dataset.key);
  saveUi({ rateHidden: { ...all, [group]: [...set] } });
});

function render() {
  if (document.hidden) { state.dirty = true; return; }
  state.dirty = false;
  renderHeader();
  renderQuota();
  renderSessions();
  if (state.mode === "full") renderFullPanels();
}
document.addEventListener("visibilitychange", () => {
  if (!document.hidden && state.dirty) render();
});

// ------------------------------------------------------------------ data
function onSnapshot(json) {
  let next;
  try { next = typeof json === "string" ? JSON.parse(json) : json; } catch { return; }
  if (!next) return;
  if (next.generated_at_ms && state.snap?.generated_at_ms && next.generated_at_ms <= state.snap.generated_at_ms) {
    return;
  }
  state.snap = next;
  state.rate.push(currentRate());
  if (state.rate.length > RATE_POINTS) state.rate.shift();
  pushRatePoints();
  render();
}
function setMode(mode) {
  state.mode = mode;
  document.body.classList.remove("mode-compact", "mode-full");
  document.body.classList.add("mode-" + mode);
  $("btn-mode").textContent = mode === "full" ? "⤡" : "⤢";
  $("btn-mode").title = mode === "full" ? "Back to compact panel (Esc)" : "Full screen (F)";
  $("titlebar").classList.toggle("compact-drag", windowsPanel && mode === "compact");
  applyUsageView();
  render();
}
function setOnTop(on) {
  state.onTop = on;
  $("btn-pin").classList.toggle("on", on);
  $("btn-pin").title = on ? "Always on top: on" : "Always on top: off";
  if ($("s-ontop")) $("s-ontop").checked = on;
}

// ------------------------------------------------------------ interaction
const windowsPanel = /Windows/.test(navigator.userAgent);
if (!windowsPanel) {
  if ($("s-autostart-title")) $("s-autostart-title").textContent = "Start at login";
  if ($("s-start-hidden-desc")) $("s-start-hidden-desc").textContent = "When started at login, don't open the panel.";
}
if (windowsPanel) {
  const bar = $("titlebar");
  // Bypass the native move loop so Windows Snap doesn't resize the widget.
  bar.removeAttribute("data-tauri-drag-region");
  bar.classList.add("compact-drag");
  let drag;

  bar.addEventListener("pointerdown", (e) => {
    if (state.mode !== "compact" || drag || e.pointerType !== "mouse" || e.button !== 0 || e.target.closest("button")) return;
    bar.setPointerCapture(e.pointerId);
    drag = { id: e.pointerId, x: e.clientX, y: e.clientY };
    e.preventDefault();
  });

  bar.addEventListener("pointermove", (e) => {
    if (drag?.id !== e.pointerId || drag.ending || drag.pending) return;
    if (!drag.moved && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 4) return;
    const current = drag;
    current.moved = true;
    current.pending = invoke("drag_compact_panel", { x: current.x, y: current.y, phase: "move" })
      .catch(console.error)
      .finally(() => { current.pending = null; });
  });

  async function finishDrag(e, phase) {
    if (drag?.id !== e.pointerId || drag.ending) return;
    drag.ending = true;
    if (drag.moved) {
      await drag.pending;
      await invoke("drag_compact_panel", { x: drag.x, y: drag.y, phase }).catch(console.error);
    }
    drag = null;
  }

  bar.addEventListener("pointerup", (e) => finishDrag(e, "drop"));
  bar.addEventListener("lostpointercapture", (e) => finishDrag(e, "cancel"));
}

$("sessions").addEventListener("click", (e) => {
  const a = e.target.closest("[data-reveal]");
  if (a) { invoke("reveal", { path: a.dataset.reveal }).catch(() => {}); return; }
  if (e.target.closest(".det")) return;
  const card = e.target.closest(".card");
  if (!card) return;
  const k = card.dataset.k;
  state.selected = k;
  if (state.mode === "compact") state.expanded = state.expanded === k ? null : k;
  render();
});
$("detail").addEventListener("click", (e) => {
  const a = e.target.closest("[data-reveal]");
  if (a) invoke("reveal", { path: a.dataset.reveal }).catch(() => {});
});
$("filter").addEventListener("input", (e) => { state.filter = e.target.value; renderSessions(); });
function refreshWidget() {
  invoke("refresh");
  if (usageView() !== "live") loadUsage();
}
$("btn-refresh").onclick = refreshWidget;
$("btn-mode").onclick = () => invoke("toggle_mode");
$("btn-hide").onclick = () => invoke("hide_window");
$("btn-pin").onclick = () => invoke("set_always_on_top", { on: !state.onTop });
$("btn-settings").onclick = () => openSettings();

function moveSelection(d) {
  const list = visibleSessions();
  if (!list.length) return;
  let i = list.findIndex((s) => keyOf(s) === state.selected);
  i = i < 0 ? 0 : Math.max(0, Math.min(list.length - 1, i + d));
  state.selected = keyOf(list[i]);
  render();
  document.querySelector(".card.sel")?.scrollIntoView({ block: "nearest" });
}
function settingsOpen() { return !$("settings").classList.contains("hidden"); }

document.addEventListener("keydown", (e) => {
  if (e.target.id === "s-shortcut") return; // recorder handles its own keys
  if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT") {
    if (e.key === "Escape") { e.target.blur(); if (e.target.id === "filter") { e.target.value = ""; state.filter = ""; renderSessions(); } }
    return;
  }
  if (typeof exportOpen === "function" && exportOpen()) {
    if (e.key === "Escape") closeExportModal();
    return;
  }
  if (settingsOpen()) {
    if (e.key === "Escape") closeSettings();
    return;
  }
  if (usageView() !== "live" && ["ArrowDown", "ArrowUp", "Enter", "j", "k", "/"].includes(e.key)) return;
  switch (e.key) {
    case "Escape": state.mode === "full" ? invoke("set_mode", { mode: "compact" }) : invoke("hide_window"); break;
    case "f": case "F": invoke("toggle_mode"); break;
    case "r": case "R": refreshWidget(); break;
    case "s": case "S": openSettings(); break;
    case "e": case "E": if (typeof openExportModal === "function") openExportModal(); break;
    case "/": e.preventDefault(); $("filter").focus(); break;
    case "ArrowDown": case "j": moveSelection(1); break;
    case "ArrowUp": case "k": moveSelection(-1); break;
    case "Enter":
      if (state.mode === "compact" && state.selected) { state.expanded = state.expanded === state.selected ? null : state.selected; render(); }
      break;
  }
});
window.addEventListener("resize", () => { if (state.mode === "full") drawRate(); });

// --------------------------------------------------------------- settings
function setSeg(id, v) {
  for (const b of $(id).querySelectorAll("button")) b.classList.toggle("on", b.dataset.v === v);
}
function onSeg(id, fn) {
  $(id).addEventListener("click", (e) => {
    const b = e.target.closest("button[data-v]");
    if (!b) return;
    setSeg(id, b.dataset.v);
    fn(b.dataset.v);
  });
}
async function patchSettings(patch) {
  try {
    state.settings = await invoke("update_settings", { patch });
    fillGeneral();
  } catch (e) { console.error(e); }
}
function showTab(tab) {
  for (const b of $("st-nav").querySelectorAll("button")) b.classList.toggle("on", b.dataset.tab === tab);
  for (const p of document.querySelectorAll(".st-pane")) p.classList.toggle("on", p.dataset.pane === tab);
  if (tab === "quota") { fillAccounts(); fillClaudeStatus(); }
  if (tab === "providers") fillProviders();
}
$("st-nav").addEventListener("click", (e) => { const b = e.target.closest("button[data-tab]"); if (b) showTab(b.dataset.tab); });
$("settings-close").onclick = closeSettings;
$("settings").addEventListener("mousedown", (e) => { if (e.target.id === "settings") closeSettings(); });
function closeSettings() { $("settings").classList.add("hidden"); }

async function openSettings(tab) {
  $("settings").classList.remove("hidden");
  try { state.settings = await invoke("get_settings"); } catch (e) { console.error(e); }
  fillGeneral();
  fillAppearance();
  fillSessionsPane();
  fillAbout();
  showTab(tab || document.querySelector("#st-nav button.on")?.dataset.tab || "general");
}

function fillGeneral() {
  const v = state.settings;
  if (!v) return;
  const s = v.settings;
  $("s-autostart").checked = v.autostart;
  $("s-start-hidden").checked = s.start_hidden;
  setSeg("s-start-mode", s.start_mode);
  $("s-ontop").checked = s.always_on_top;
  $("s-shortcut-on").checked = s.shortcut_enabled;
  $("s-shortcut").value = s.shortcut;
  $("s-shortcut").disabled = !s.shortcut_enabled;
  $("s-shortcut-err").textContent = v.shortcut_error ? "Could not register " + v.shortcut_error : "";
  $("s-tick").value = String(s.tick_secs);
  $("s-quota-min").value = String(s.quota_minutes);
}
$("s-autostart").onchange = async (e) => {
  try { e.target.checked = await invoke("set_autostart", { on: e.target.checked }); } catch (err) { e.target.checked = !e.target.checked; alert(err); }
};
$("s-start-hidden").onchange = (e) => patchSettings({ start_hidden: e.target.checked });
onSeg("s-start-mode", (v) => patchSettings({ start_mode: v }));
$("s-ontop").onchange = (e) => invoke("set_always_on_top", { on: e.target.checked });
$("s-shortcut-on").onchange = (e) => patchSettings({ shortcut_enabled: e.target.checked });
$("s-tick").onchange = (e) => patchSettings({ tick_secs: Number(e.target.value) });
$("s-quota-min").onchange = (e) => patchSettings({ quota_minutes: Number(e.target.value) });

// shortcut recorder
const KEYNAME = (e) => {
  if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
  if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
  if (/^F\d{1,2}$/.test(e.code)) return e.code;
  const map = { Space: "Space", Backquote: "`", Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/", Backslash: "\\", Semicolon: ";", Quote: "'", BracketLeft: "[", BracketRight: "]", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right", Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown", Insert: "Insert", Delete: "Delete" };
  return map[e.code] || null;
};
$("s-shortcut").addEventListener("focus", (e) => { e.target.dataset.prev = e.target.value; e.target.value = "press keys…"; });
$("s-shortcut").addEventListener("blur", (e) => { if (e.target.value === "press keys…") e.target.value = e.target.dataset.prev || ""; });
$("s-shortcut").addEventListener("keydown", (e) => {
  e.preventDefault();
  if (e.key === "Escape") { e.target.blur(); return; }
  const key = KEYNAME(e);
  if (!key) return; // modifier only: keep waiting
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
  if (!mods.length) { $("s-shortcut-err").textContent = "Use at least one modifier (Ctrl, Alt, Shift or Win)."; return; }
  const combo = [...mods, key].join("+");
  e.target.value = combo;
  e.target.dataset.prev = combo;
  e.target.blur();
  patchSettings({ shortcut: combo });
});

// appearance
/** Read a theme variant's variables straight from themes.css (for previews). */
function themeVars(theme, scheme) {
  const out = {};
  for (const sheet of document.styleSheets) {
    let rules;
    try { rules = sheet.cssRules; } catch { continue; }
    for (const r of rules) {
      const sel = r.selectorText || "";
      if (sel.includes(`data-theme="${theme}"`) && sel.includes(`data-scheme="${scheme}"`)) {
        for (const name of ["--bg", "--panel", "--panel-3", "--fg", "--dim", "--line", "--p-claude", "--p-codex", "--p-copilot", "--p-gemini"]) {
          const v = r.style.getPropertyValue(name).trim();
          if (v) out[name] = v;
        }
      }
    }
  }
  return out;
}
function fillThemes() {
  const scheme = effectiveScheme();
  setSeg("s-scheme", state.ui.scheme);
  $("s-themes").innerHTML = THEMES.map(([key, label]) => {
    const v = themeVars(key, scheme);
    const half = (bg) => `<div style="background:${bg}"><span class="ln" style="background:${v["--fg"]};width:70%"></span><span class="ln" style="background:${v["--dim"]};width:45%"></span><span class="dots">${["--p-claude", "--p-codex", "--p-copilot", "--p-gemini"].map((p) => `<i style="background:${v[p]}"></i>`).join("")}</span></div>`;
    return `<button class="theme-card ${state.ui.theme === key ? "on" : ""}" data-theme-key="${key}" title="${esc(label)} (${scheme})"><div class="pv" style="border-bottom:1px solid ${v["--line"]}">${half(v["--bg"])}${half(v["--panel"])}</div><span class="nm">${esc(label)}</span></button>`;
  }).join("");
}
function fillAppearance() {
  fillThemes();
  setSeg("s-density", state.ui.density);
  $("s-warn").value = state.ui.warn; $("s-warn-v").textContent = state.ui.warn + "%";
  $("s-crit").value = state.ui.crit; $("s-crit-v").textContent = state.ui.crit + "%";
  $("s-colors").innerHTML = allIds().map((id) => {
    const c = idColor(id);
    const own = state.ui.colorOverrides?.[id] ? " · custom" : "";
    return `<label class="color-item" ${idStyle(id)}><input type="color" data-prov="${esc(id)}" value="${/^#[0-9a-f]{6}$/i.test(c) ? c : "#888888"}" /><span class="badge">${idTag(id)}</span><span title="${esc(idName(id))}">${esc(idName(id))}<small style="color:var(--dim)">${own}</small></span></label>`;
  }).join("");
  $("s-colors-err").textContent = "";
}
onSeg("s-scheme", (v) => { saveUi({ scheme: v }); fillAppearance(); });
$("s-themes").addEventListener("click", (e) => {
  const b = e.target.closest("[data-theme-key]");
  if (!b) return;
  saveUi({ theme: b.dataset.themeKey });
  fillAppearance();
});
onSeg("s-density", (v) => saveUi({ density: v }));
$("s-warn").oninput = (e) => { $("s-warn-v").textContent = e.target.value + "%"; };
$("s-warn").onchange = (e) => {
  const warn = Number(e.target.value);
  saveUi({ warn, crit: Math.max(state.ui.crit, warn + 1) });
  fillAppearance();
};
$("s-crit").oninput = (e) => { $("s-crit-v").textContent = e.target.value + "%"; };
$("s-crit").onchange = (e) => {
  const crit = Number(e.target.value);
  saveUi({ crit, warn: Math.min(state.ui.warn, crit - 1) });
  fillAppearance();
};
function hexRgb(h) { const n = parseInt(h.slice(1), 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; }
function colorDistance(a, b) {
  // weighted RGB distance ("redmean"), good enough to catch look-alike picks
  const [r1, g1, b1] = hexRgb(a), [r2, g2, b2] = hexRgb(b);
  const rm = (r1 + r2) / 2, dr = r1 - r2, dg = g1 - g2, db = b1 - b2;
  return Math.sqrt((2 + rm / 256) * dr * dr + 4 * dg * dg + (2 + (255 - rm) / 256) * db * db);
}
$("s-colors").addEventListener("change", (e) => {
  const inp = e.target.closest("input[data-prov]");
  if (!inp) return;
  const prov = inp.dataset.prov;
  const clash = allIds().find((k) => k !== prov && colorDistance(idColor(k), inp.value) < 110);
  if (clash) {
    $("s-colors-err").textContent = `Too close to ${idName(clash)}: pick a clearly different color.`;
    inp.value = idColor(prov);
    return;
  }
  $("s-colors-err").textContent = "";
  saveUi({ colorOverrides: { ...(state.ui.colorOverrides || {}), [prov]: inp.value } });
  fillAppearance();
});
$("s-colors-reset").onclick = () => { saveUi({ colorOverrides: {} }); fillAppearance(); };

// sessions pane
function fillSessionsPane() {
  $("s-sort").value = state.ui.sort;
  $("s-idle").checked = state.ui.showIdle;
  $("s-chat").checked = state.ui.showChat;
  $("s-task").checked = state.ui.showTask;
}
// providers pane
function fillProviders() {
  const counts = {};
  for (const s of state.snap?.sessions || []) counts[pvKey(s.agent_cli)] = (counts[pvKey(s.agent_cli)] || 0) + 1;
  const sizes = Object.fromEntries((state.storage?.providers || []).map((p) => [p.provider, p.total_bytes]));
  const off = new Set((state.settings?.settings?.disabled_providers || []).map((p) => p.toLowerCase()));
  $("s-providers").innerHTML = Object.entries(PROVIDERS).map(([k, p]) => {
    const on = !off.has(k);
    const info = !on ? "disabled — not scanned" : [counts[k] ? `${counts[k]} running` : "not running", sizes[k] != null ? fmtBytes(sizes[k]) + " on disk" : ""].filter(Boolean).join(" · ");
    return `<div class="prov-toggle ${on ? "" : "off"}" ${idStyle(storageColorId(k))}><span class="badge">${p.tag}</span><div class="meta"><b>${esc(p.name)}</b><small>${esc(info)}</small></div><label class="sw"><input type="checkbox" data-prov="${k}" ${on ? "checked" : ""} /><i></i></label></div>`;
  }).join("");
  const st = state.storage;
  $("s-storage-info").textContent = st?.computed_at_ms
    ? `Measured ${new Date(st.computed_at_ms).toLocaleTimeString()} in ${(st.duration_ms / 1000).toFixed(1)} s. Shown in the full-screen view.`
    : "Measuring…";
}
$("s-providers").addEventListener("change", async (e) => {
  const cb = e.target.closest("input[data-prov]");
  if (!cb) return;
  const set = new Set(state.settings?.settings?.disabled_providers || []);
  cb.checked ? set.delete(cb.dataset.prov) : set.add(cb.dataset.prov);
  await patchSettings({ disabled_providers: [...set] });
  fillProviders();
  render();
  renderUsage();
  loadUsage();
});
$("s-storage-refresh").onclick = () => { invoke("refresh_storage"); $("s-storage-info").textContent = "Measuring…"; };
$("storage-refresh").onclick = () => { invoke("refresh_storage"); $("storage").innerHTML = `<div class="empty">measuring…</div>`; };

function renderStorage() {
  const st = state.storage;
  if (!st?.computed_at_ms) { $("storage").innerHTML = `<div class="empty">measuring…</div>`; return; }
  const list = (st.providers || []).filter((p) => !isHidden(p.provider));
  if (!list.length) { $("storage").innerHTML = `<div class="empty">no provider data on disk</div>`; return; }
  const max = Math.max(...list.map((p) => p.total_bytes), 1);
  const total = list.reduce((a, p) => a + p.total_bytes, 0);
  $("storage").innerHTML = `<div class="q-note">total <b>${fmtBytes(total)}</b> · ${new Date(st.computed_at_ms).toLocaleTimeString()}</div>` + list.map((p) => {
    const id = storageColorId(p.provider);
    const locs = p.locations.map((l) => `<div class="sto-loc" data-reveal="${esc(l.path)}" title="${esc(l.label)}\n${esc(l.path)}\n${fmtNum(l.files)} files${l.partial ? " (partial)" : ""} — click to open"><span>${esc(l.label)} <span style="color:var(--faint)">${esc(homeAbbrev(l.path))}</span></span><span>${l.partial ? "≥" : ""}${fmtBytes(l.bytes)}</span></div>`).join("");
    return `<div class="sto" ${idStyle(id)}><div class="sto-head"><span class="n">${esc(PROVIDERS[p.provider]?.name || p.provider)}</span><span class="s">${fmtBytes(p.total_bytes)}</span></div>${bar((p.total_bytes / max) * 100, "time")}${locs}</div>`;
  }).join("");
}
$("storage").addEventListener("click", (e) => {
  const a = e.target.closest("[data-reveal]");
  if (a) invoke("reveal", { path: a.dataset.reveal }).catch(() => {});
});
$("s-sort").onchange = (e) => saveUi({ sort: e.target.value });
$("s-idle").onchange = (e) => saveUi({ showIdle: e.target.checked });
$("s-chat").onchange = (e) => saveUi({ showChat: e.target.checked });
$("s-task").onchange = (e) => saveUi({ showTask: e.target.checked });

// quota & accounts pane
async function fillAccounts() {
  const box = $("s-accounts");
  box.innerHTML = `<span class="hint">loading…</span>`;
  setSeg("s-cp-buckets", state.ui.cpBuckets);
  setSeg("s-credit-unit", state.ui.creditUnit);
  $("s-usd-rate").value = state.ui.usdPerCredit;
  $("s-claude-local").checked = state.ui.claudeLocal;
  try {
    const info = await invoke("list_accounts");
    const hidden = new Set((info.hidden || []).map((h) => h.toLowerCase()));
    box.innerHTML = info.accounts.map((a) => {
      const q = state.quota.find((x) => x.login?.toLowerCase() === a.login.toLowerCase());
      const tags = [a.login === info.copilot_cli ? "used by Copilot CLI" : "", a.active ? "gh active" : "", q?.plan || "", q?.error ? "⚠ " + q.error : ""].filter(Boolean).join(" · ");
      return `<label class="pv-copilot"><input type="checkbox" data-login="${esc(a.login)}" ${hidden.has(a.login.toLowerCase()) ? "" : "checked"} /><span><b>${esc(a.login)}</b></span><span class="hint" title="${esc(tags)}">${esc(tags)}</span></label>`;
    }).join("") || `<span class="hint">No accounts.</span>`;
    $("s-acct-hint").textContent = "Every github.com account logged into the GitHub CLI. Add one with `gh auth login`; tokens are read on demand and never stored.";
  } catch (err) {
    box.innerHTML = "";
    $("s-acct-hint").textContent = String(err);
  }
}
$("s-accounts").addEventListener("change", (e) => {
  const cb = e.target.closest("input[data-login]");
  if (cb) invoke("set_copilot_account_visible", { login: cb.dataset.login, visible: cb.checked });
});
$("s-acct-reload").onclick = () => { invoke("refresh"); fillAccounts(); };
onSeg("s-cp-buckets", (v) => saveUi({ cpBuckets: v }));
$("s-claude-local").onchange = (e) => saveUi({ claudeLocal: e.target.checked });
onSeg("s-credit-unit", (v) => saveUi({ creditUnit: v }));
$("s-usd-rate").onchange = (e) => {
  const v = Number(e.target.value);
  if (!(v >= 0)) { e.target.value = state.ui.usdPerCredit; return; }
  saveUi({ usdPerCredit: v });
};

function fillClaudeStatus() {
  const cu = state.claude, L = state.local;
  const oauthOk = !!cu?.limits;
  const oauth = `<div class="${oauthOk ? "ok" : "no"}"><b>Plan limits (5h / 7d %)</b> — ${oauthOk ? "connected" + (cu.subscription ? " · " + esc(cu.subscription) : "") : "not available"}
    <small>${oauthOk ? "From Anthropic's usage endpoint with the Claude Code CLI login." : esc(cu?.error || "waiting for first refresh…") + ". Run <code>claude</code> in a terminal and <code>/login</code> once; the Claude desktop app keeps its own login, which Scrooge does not read."}</small></div>`;
  const localOk = L && L.computed_at_ms;
  const local = `<div class="${localOk ? "ok" : "no"}"><b>Local consumption (tokens)</b> — ${localOk ? `${fmtNum(L.files_scanned)} transcripts, 7d ${fmtTok(L.week.total)}` : "scanning…"}
    <small>${localOk ? "Read from " + L.roots.map(esc).join(", ") + ". Works for CLI and desktop-app sessions; no login needed." : ""}</small></div>`;
  $("s-claude-status").innerHTML = oauth + local;
}

// about
function fillAbout() {
  const v = state.settings;
  if (!v) return;
  $("s-version").textContent = "v" + v.version;
  $("s-upd-auto").checked = v.settings.auto_update_check !== false;
  showUpdate(state.update);
  const p = v.paths;
  const row = (label, path) => `<div class="path-row"><span class="pl">${label}</span><code>${esc(path)}</code><button class="btn" data-reveal="${esc(path)}">Open</button></div>`;
  $("s-paths").innerHTML = row("Settings", p.settings_file) + row("Copilot data", p.copilot_home) +
    p.claude_roots.map((r) => row("Claude data", r)).join("") + row("abtop config", p.abtop_config);
}
$("s-paths").addEventListener("click", (e) => {
  const b = e.target.closest("[data-reveal]");
  if (b) invoke("reveal", { path: b.dataset.reveal }).catch((err) => alert(err));
});
$("s-quit").onclick = () => invoke("quit_app");

// ---------------------------------------------------------------- updates
function showUpdate(info) {
  state.update = info;
  const has = info?.status === "available";
  $("s-upd-row").classList.toggle("hidden", !has);
  if (has) {
    $("s-upd-version").textContent = `Version ${info.version} is available (you have ${info.current})`;
    $("s-upd-notes").textContent = info.notes ? info.notes.split("\n")[0].slice(0, 160) : "";
    $("update-text").textContent = `Scrooge ${info.version} is available`;
  }
  $("update-banner").classList.toggle("hidden", !has || state.updateDismissed);
}
async function checkUpdates() {
  $("s-upd-status").textContent = "Checking…";
  $("s-upd-check").disabled = true;
  try {
    const info = await invoke("check_update");
    showUpdate(info);
    const messages = {
      available: "Update available.",
      up_to_date: `You are on the latest version (${info.current}).`,
      no_published_release: "No release has been published yet. Automatic updates will be available after the first release.",
    };
    $("s-upd-status").textContent = messages[info.status];
  } catch (err) {
    $("s-upd-status").textContent = "Update check failed: " + err;
  } finally {
    $("s-upd-check").disabled = false;
  }
}
async function installUpdate() {
  for (const b of [$("s-upd-install"), $("update-install")]) b.disabled = true;
  $("update-text").textContent = $("s-upd-status").textContent = "Downloading update…";
  $("update-banner").classList.remove("hidden");
  try {
    await invoke("install_update"); // restarts the app on success
  } catch (err) {
    $("update-text").textContent = $("s-upd-status").textContent = "Update failed: " + err;
    for (const b of [$("s-upd-install"), $("update-install")]) b.disabled = false;
  }
}
$("s-upd-check").onclick = checkUpdates;
$("s-upd-install").onclick = installUpdate;
$("update-install").onclick = installUpdate;
$("update-later").onclick = () => { state.updateDismissed = true; $("update-banner").classList.add("hidden"); };
$("s-upd-auto").onchange = (e) => patchSettings({ auto_update_check: e.target.checked });

// ------------------------------------------------------------------ boot
initUsage();
(async function boot() {
  await listen("snapshot", (e) => onSnapshot(e.payload));
  await listen("quota", (e) => { state.quota = e.payload || []; computeAccountColors(); render(); });
  await listen("claude-usage", (e) => { state.claude = e.payload; renderQuota(); if (settingsOpen()) fillClaudeStatus(); });
  await listen("claude-local", (e) => { state.local = e.payload; renderQuota(); if (settingsOpen()) fillClaudeStatus(); });
  await listen("mode", (e) => setMode(e.payload));
  await listen("always-on-top", (e) => setOnTop(e.payload));
  await listen("open-settings", () => openSettings());
  await listen("storage", (e) => { state.storage = e.payload; if (state.mode === "full") renderStorage(); if (settingsOpen()) fillProviders(); });
  await listen("update-available", (e) => showUpdate(e.payload));
  await listen("update-progress", (e) => {
    const { downloaded, total } = e.payload;
    const pct = total ? ` ${Math.round((downloaded / total) * 100)}%` : ` ${fmtTok(downloaded)}B`;
    $("update-text").textContent = $("s-upd-status").textContent = "Downloading update…" + pct;
  });
  try {
    const [snap, quota, claude, local, mode, onTop, settings] = await Promise.all([
      invoke("get_snapshot"), invoke("get_quota"), invoke("get_claude_usage"), invoke("get_claude_local"),
      invoke("get_mode"), invoke("get_always_on_top"), invoke("get_settings"),
    ]);
    state.settings = settings;
    loadUi(settings?.settings?.ui);
    state.quota = quota || [];
    computeAccountColors();
    state.claude = claude && claude.fetched_at_ms ? claude : null;
    state.local = local && local.computed_at_ms ? local : null;
    state.storage = await invoke("get_storage");
    setOnTop(onTop);
    setMode(mode);
    onSnapshot(snap);
  } catch (err) {
    console.error(err);
  }
})();
