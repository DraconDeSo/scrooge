// Validate ui/themes.css: every theme variant must keep the six provider
// colors distinct and meet WCAG contrast for text, status colors and badges.
// usage: node scripts/check-themes.mjs ui/themes.css
import fs from "node:fs";
const css = fs.readFileSync(process.argv[2], "utf8");
const re = /html\[data-theme="(\w+)"\]\[data-scheme="(\w+)"\]\s*\{([^}]*)\}/g;
const hex = (h) => { h = h.replace("#", ""); if (h.length === 3) h = [...h].map((c) => c + c).join(""); const n = parseInt(h, 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; };
const lum = ([r, g, b]) => { const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
const contrast = (a, b) => { const [x, y] = [lum(hex(a)), lum(hex(b))].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const dist = (a, b) => { const [r1, g1, b1] = hex(a), [r2, g2, b2] = hex(b); const rm = (r1 + r2) / 2, dr = r1 - r2, dg = g1 - g2, db = b1 - b2; return Math.sqrt((2 + rm / 256) * dr * dr + 4 * dg * dg + (2 + (255 - rm) / 256) * db * db); };
const P = ["claude", "codex", "opencode", "copilot", "gemini", "grok"];
let problems = 0;
for (const m of css.matchAll(re)) {
  const [, theme, scheme, body] = m;
  const v = {};
  for (const d of body.matchAll(/--([\w-]+):\s*(#[0-9a-fA-F]{3,6})\b/g)) v[d[1]] = d[2];
  const issues = [];
  const panel = v["panel"] || v["panel-2"];
  const ck = (name, a, b, min) => { const c = contrast(a, b); if (c < min) issues.push(`${name} ${c.toFixed(2)}<${min}`); };
  ck("fg/panel", v.fg, panel, 7);
  ck("dim/panel", v.dim, panel, 4.5);
  ck("faint/panel", v.faint, panel, 2);
  for (const k of ["ok", "warn", "crit", "link"]) ck(`${k}/panel`, v[k], panel, 3);
  let minD = 1e9, pair = "";
  for (let i = 0; i < P.length; i++) {
    const c = v["p-" + P[i]];
    ck(`${P[i]}/panel`, c, panel, 3);
    ck(`badge-on-${P[i]}`, v["badge-fg"], c, 3.5);
    for (let j = i + 1; j < P.length; j++) { const d = dist(c, v["p-" + P[j]]); if (d < minD) { minD = d; pair = P[i] + "~" + P[j]; } }
  }
  if (minD < 110) issues.push(`providers too close ${pair} ${minD.toFixed(0)}`);
  if (v["title-fg"] && v["title-bg"]) ck("title fg/bg", v["title-fg"], v["title-bg"], 4.5);
  problems += issues.length;
  console.log(`${(theme + "/" + scheme).padEnd(20)} minDist ${minD.toFixed(0).padStart(3)} ${issues.length ? "| " + issues.join("; ") : "ok"}`);
}
console.log(problems ? `\n${problems} issue(s)` : "\nall themes pass");
if (problems) process.exit(1);
