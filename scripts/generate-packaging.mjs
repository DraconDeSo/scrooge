import { createHash } from "node:crypto";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";

const root = process.argv[2];
const outDir = process.argv[3] || root;
if (!root) {
  throw new Error("Usage: node scripts/generate-packaging.mjs <bundles-dir> [output-dir]");
}

const pkg = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8"));
const version = pkg.version;
const repo = process.env.GITHUB_REPOSITORY || "DraconDeSo/scrooge";

const allFiles = (await readdir(root, { recursive: true, withFileTypes: true }))
  .filter((entry) => entry.isFile())
  .map((entry) => join(entry.parentPath, entry.name));

const releaseExts = [".exe", ".msi", ".dmg", ".app.tar.gz", ".deb", ".rpm", ".AppImage", ".zip", ".tar.gz", ".sig"];
const hashes = new Map();

for (const filePath of allFiles) {
  const name = basename(filePath);
  if (!releaseExts.some((ext) => name.endsWith(ext))) continue;
  const buf = await readFile(filePath);
  const sha256 = createHash("sha256").update(buf).digest("hex");
  hashes.set(name, sha256);
}

if (hashes.size === 0) {
  throw new Error(`No release artifacts found in ${root}`);
}

await mkdir(outDir, { recursive: true });

// 1. SHA256SUMS.txt (excluding .sig files)
const sumLines = [...hashes.entries()]
  .filter(([name]) => !name.endsWith(".sig"))
  .sort(([a], [b]) => a.localeCompare(b))
  .map(([name, hash]) => `${hash}  ${name}`);
await writeFile(join(outDir, "SHA256SUMS.txt"), sumLines.join("\n") + "\n", "utf8");

const findBySuffix = (suffix) => {
  for (const [name, hash] of hashes.entries()) {
    if (name.endsWith(suffix)) return { name, hash };
  }
  return null;
};

const winSetup = findBySuffix("_x64-setup.exe");
const winPortable = findBySuffix("_x64-portable.zip") || winSetup;
const macArm = findBySuffix("_aarch64.dmg");
const macIntel = findBySuffix("_x64.dmg");
const linuxDeb = findBySuffix("_amd64.deb");

// 2. Scoop manifest
if (winPortable) {
  const scoop = {
    version,
    description: "Desktop monitor for AI coding agents (Claude Code, Codex CLI, OpenCode, GitHub Copilot, Gemini CLI, Grok Build)",
    homepage: `https://github.com/${repo}`,
    license: "MIT",
    architecture: {
      "64bit": {
        url: `https://github.com/${repo}/releases/download/v${version}/${winPortable.name}`,
        hash: winPortable.hash,
      },
    },
    bin: ["scrooge.exe", "abtop.exe"],
    shortcuts: [["scrooge.exe", "Scrooge"]],
    checkver: { github: `https://github.com/${repo}` },
    autoupdate: {
      architecture: {
        "64bit": {
          url: `https://github.com/${repo}/releases/download/v$version/Scrooge_$version_x64-portable.zip`,
          hash: {
            url: `https://github.com/${repo}/releases/download/v$version/SHA256SUMS.txt`,
          },
        },
      },
    },
  };
  await writeFile(join(outDir, "scrooge.scoop.json"), JSON.stringify(scoop, null, 2) + "\n", "utf8");
}

// 3. Winget manifest
if (winSetup) {
  const winget = `# yaml-language-server: $schema=https://aka.ms/winget-manifest.singleton.1.6.0.schema.json
PackageIdentifier: DraconDeSo.Scrooge
PackageVersion: ${version}
PackageLocale: en-US
Publisher: The Scrooge contributors
PublisherUrl: https://github.com/${repo}
PackageName: Scrooge
PackageUrl: https://github.com/${repo}
License: MIT
LicenseUrl: https://github.com/${repo}/blob/main/LICENSE
ShortDescription: Desktop monitor for AI coding agents
Description: Scrooge shows every AI coding agent running on your machine (Claude Code, Codex CLI, OpenCode, GitHub Copilot, Gemini CLI, Grok Build) with sessions, tokens, context windows and quotas.
Moniker: scrooge
Tags:
  - ai
  - claude
  - codex
  - copilot
  - gemini
  - monitor
  - developer-tools
Installers:
  - Architecture: x64
    InstallerType: nullsoft
    Scope: user
    InstallerUrl: https://github.com/${repo}/releases/download/v${version}/${winSetup.name}
    InstallerSha256: ${winSetup.hash.toUpperCase()}
ManifestType: singleton
ManifestVersion: 1.6.0
`;
  await writeFile(join(outDir, "DraconDeSo.Scrooge.winget.yaml"), winget, "utf8");
}

// 4. Homebrew Cask
if (macArm && macIntel) {
  const cask = `cask "scrooge" do
  arch arm: "aarch64", intel: "x64"

  version "${version}"
  sha256 arm:   "${macArm.hash}",
         intel: "${macIntel.hash}"

  url "https://github.com/${repo}/releases/download/v#{version}/Scrooge_#{version}_#{arch}.dmg"
  name "Scrooge"
  desc "Desktop monitor for AI coding agents"
  homepage "https://github.com/${repo}"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :big_sur"

  app "Scrooge.app"

  zap trash: [
    "~/Library/Application Support/scrooge",
    "~/Library/Preferences/dev.scrooge.widget.plist",
  ]
end
`;
  await writeFile(join(outDir, "scrooge.rb"), cask, "utf8");
}

// 5. Arch Linux AUR PKGBUILD
if (linuxDeb) {
  const pkgbuild = `# Maintainer: The Scrooge contributors <https://github.com/${repo}>
pkgname=scrooge-bin
pkgver=${version}
pkgrel=1
pkgdesc="Desktop monitor for AI coding agents (Claude Code, Codex CLI, OpenCode, GitHub Copilot, Gemini CLI, Grok Build)"
arch=('x86_64')
url="https://github.com/${repo}"
license=('MIT')
depends=('webkit2gtk-4.1' 'libayatana-appindicator' 'gtk3')
provides=('scrooge')
conflicts=('scrooge')
source=("https://github.com/${repo}/releases/download/v\${pkgver}/${linuxDeb.name}")
sha256sums=('${linuxDeb.hash}')

package() {
  bsdtar -xf "\${srcdir}/data.tar."* -C "\${pkgdir}/"
}
`;
  await writeFile(join(outDir, "PKGBUILD"), pkgbuild, "utf8");

  // 6. Flatpak manifest
  const flatpak = `app-id: dev.scrooge.widget
runtime: org.gnome.Platform
runtime-version: '46'
sdk: org.gnome.Sdk
command: scrooge
finish-args:
  - --share=ipc
  - --socket=fallback-x11
  - --socket=wayland
  - --share=network
  - --filesystem=home:ro
  - --filesystem=xdg-config/scrooge:create
  - --talk-name=org.kde.StatusNotifierWatcher
modules:
  - name: scrooge
    buildsystem: simple
    build-commands:
      - ar x scrooge.deb
      - tar -xf data.tar.*
      - cp -r usr/* /app/
    sources:
      - type: file
        only-arches: [x86_64]
        url: https://github.com/${repo}/releases/download/v${version}/${linuxDeb.name}
        sha256: ${linuxDeb.hash}
        dest-filename: scrooge.deb
`;
  await writeFile(join(outDir, "dev.scrooge.widget.flatpak.yml"), flatpak, "utf8");

  // 7. Snapcraft manifest
  const snap = `name: scrooge
base: core22
version: '${version}'
summary: Desktop monitor for AI coding agents
description: |
  Scrooge shows every AI coding agent running on your machine (Claude Code,
  Codex CLI, OpenCode, GitHub Copilot, Gemini CLI, Grok Build): sessions,
  read/written/cached tokens, context windows and quotas.
grade: stable
confinement: classic

apps:
  scrooge:
    command: usr/bin/scrooge
    desktop: usr/share/applications/Scrooge.desktop
  abtop:
    command: usr/bin/abtop

parts:
  scrooge:
    plugin: dump
    source: https://github.com/${repo}/releases/download/v${version}/${linuxDeb.name}
    source-type: deb
    source-checksum: sha256/${linuxDeb.hash}
    stage-packages:
      - libwebkit2gtk-4.1-0
      - libayatana-appindicator3-1
`;
  await writeFile(join(outDir, "snapcraft.yaml"), snap, "utf8");
}

// 8. Chocolatey install script
if (winSetup) {
  const choco = `$ErrorActionPreference = 'Stop'

$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  fileType       = 'exe'
  url64bit       = 'https://github.com/${repo}/releases/download/v${version}/${winSetup.name}'
  checksum64     = '${winSetup.hash}'
  checksumType64 = 'sha256'
  silentArgs     = '/S'
  validExitCodes = @(0)
}

Install-ChocolateyPackage @packageArgs
`;
  await writeFile(join(outDir, "chocolateyinstall.ps1"), choco, "utf8");
}

console.log(`Generated SHA256SUMS.txt (${sumLines.length} files) and package manifests in ${outDir}`);
