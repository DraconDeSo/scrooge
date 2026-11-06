import { readFile } from "node:fs/promises";

// GTK 3 bindings required by Tauri 2 on Linux have no patched glib 0.18 release.
// Remove this exact exception once the upstream Linux stack can move past 0.18.
const existing = new Map([
  ["glib@0.18.5", new Set(["GHSA-wrw7-89jp-8q8g", "RUSTSEC-2024-0429"])],
]);

const lock = await readFile(new URL("../Cargo.lock", import.meta.url), "utf8");
const entries = [...lock.matchAll(/^name = "([^"]+)"\r?\nversion = "([^"]+)"\r?\nsource = "registry\+https:\/\/github\.com\/rust-lang\/crates\.io-index"$/gm)];
if (!entries.length) throw new Error("Cargo.lock has no crates.io packages.");
const queries = entries.map(([, name, version]) => ({ package: { name, ecosystem: "crates.io" }, version }));
const findings = new Map();
for (let offset = 0; offset < queries.length; offset += 100) {
  const batch = queries.slice(offset, offset + 100);
  const response = await fetch("https://api.osv.dev/v1/querybatch", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ queries: batch }),
    signal: AbortSignal.timeout(30_000),
  });
  if (!response.ok) throw new Error("OSV query failed with HTTP " + response.status);
  const { results } = await response.json();
  if (results?.length !== batch.length) throw new Error("OSV returned an incomplete result.");
  for (const [index, result] of results.entries()) {
    const { package: { name }, version } = batch[index];
    for (const { id } of result.vulns || []) {
      const packages = findings.get(id) || new Set();
      packages.add(name + "@" + version);
      findings.set(id, packages);
    }
  }
}

for (const [id, packages] of findings) {
  if (!/^[A-Za-z0-9-]+$/.test(id)) throw new Error("Invalid advisory ID.");
  const response = await fetch("https://api.osv.dev/v1/vulns/" + id, { signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error("OSV advisory lookup failed with HTTP " + response.status);
  const advisory = await response.json();
  const informational = advisory.database_specific?.informational || advisory.affected?.some(
    (entry) => entry.database_specific?.informational || entry.ecosystem_specific?.informational);
  for (const pkg of packages) {
    const finding = pkg + " " + id;
    if (informational) console.warn("Informational advisory:", finding);
    else if (existing.get(pkg)?.has(id)) console.warn("Known upstream dependency advisory:", finding);
    else {
      console.error("Vulnerable dependency:", finding);
      process.exitCode = 1;
    }
  }
}
console.log("Checked " + queries.length + " locked crates.io package versions.");
