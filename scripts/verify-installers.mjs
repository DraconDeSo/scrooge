import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

const root = process.argv[2];
if (!root) throw new Error("Provide the directory containing downloaded bundle artifacts.");
const skipSigs = process.env.SKIP_SIGNATURE_VERIFY === "1";
const config = JSON.parse(await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
const unpack = (text) => text.trim().startsWith("untrusted comment:")
  ? text.trim() + "\n" : Buffer.from(text.trim(), "base64").toString("utf8").trim() + "\n";
const expected = {
  "x86_64-pc-windows-msvc": skipSigs ? [".exe", ".msi"] : [".exe", ".msi", ".exe.sig", ".msi.sig"],
  "aarch64-apple-darwin": skipSigs ? [".dmg"] : [".dmg", ".app.tar.gz", ".app.tar.gz.sig"],
  "x86_64-apple-darwin": skipSigs ? [".dmg"] : [".dmg", ".app.tar.gz", ".app.tar.gz.sig"],
  "x86_64-unknown-linux-gnu": skipSigs ? [".deb", ".rpm", ".AppImage"] : [".deb", ".rpm", ".AppImage", ".AppImage.sig"],
};
const temp = await mkdtemp(join(tmpdir(), "scrooge-signatures-"));
try {
  const pubkey = join(temp, "updater.pub");
  const unpacked = join(temp, "asset.minisig");
  await writeFile(pubkey, unpack(config.plugins.updater.pubkey));
  for (const [target, suffixes] of Object.entries(expected)) {
    const folder = join(root, "scrooge-" + target);
    const files = (await readdir(folder, { recursive: true, withFileTypes: true }))
      .filter((file) => file.isFile()).map((file) => join(file.parentPath, file.name));
    for (const suffix of suffixes) {
      if (!files.some((file) => file.endsWith(suffix))) throw new Error(target + " is missing " + suffix);
    }
    if (!skipSigs) {
      for (const file of files.filter((file) => file.endsWith(".sig"))) {
        const bundle = file.slice(0, -4);
        if (!files.includes(bundle)) throw new Error("Missing file for updater signature " + file);
        await writeFile(unpacked, unpack(await readFile(file, "utf8")));
        execFileSync("minisign", ["-Vm", bundle, "-p", pubkey, "-x", unpacked, "-q"], { stdio: "inherit" });
      }
    }
    console.log("Verified bundles" + (skipSigs ? "" : " and updater signatures") + " for " + target);
  }
} finally {
  await rm(temp, { recursive: true, force: true });
}
