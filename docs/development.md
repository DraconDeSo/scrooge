# Development

## Prerequisites

| | Windows | macOS | Linux (Debian/Ubuntu) |
| --- | --- | --- | --- |
| Rust | [rustup](https://rustup.rs) stable (MSVC) | rustup stable | rustup stable |
| Native toolchain | Visual Studio Build Tools: *Desktop development with C++* (MSVC + Windows 11 SDK) | Xcode Command Line Tools | `build-essential` |
| WebView | WebView2 (preinstalled on Windows 11) | built in | `libwebkit2gtk-4.1-dev` |
| Tray | — | — | `libayatana-appindicator3-dev`, `librsvg2-dev` |
| Node.js | LTS | LTS | LTS |

Linux one-liner:

```bash
sudo apt install build-essential libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

## Commands

```bash
npm install                    # Tauri CLI
npx tauri dev                  # run the desktop app (UI changes reload on save)
npx tauri build                # release build + installers for the current OS
cargo test --workspace         # all tests (engine + app)
cargo run -p abtop             # the terminal UI from the session engine
cargo run -p abtop -- --json   # print one snapshot as JSON
node scripts/check-themes.mjs ui/themes.css   # theme contrast/color validation
```

Release bundles are written to `target/release/bundle/` (`nsis/`, `msi/`, `dmg/`, `macos/`,
`deb/`, `rpm/`, `appimage/`).

`npx tauri build` also produces signed update packages, so it needs the update-signing key
in the environment (see [releasing.md](releasing.md)):

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/scrooge.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="…"
```

Without a key, build installers only with
`npx tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'`, or just compile the
app with `cargo build --release -p scrooge`.

## Working on the UI without the backend

`ui/` is plain HTML/CSS/JS. Serve it with any static server and provide a stub for
`window.__TAURI__` (`core.invoke`, `event.listen`) that returns a snapshot, for example the
output of `cargo run -p abtop -- --json --demo`, which contains only demo data.

## Tests and fixtures

- Collector tests build small synthetic session folders in a temporary directory.
- Never commit real transcripts, tokens or machine-specific paths; use neutral names such as
  `C:\repos\demo` or `/home/me/app`.

## Versioning

The version lives in `package.json`; `src-tauri/tauri.conf.json` reads it from there and
`src-tauri/Cargo.toml` must match it (the release checklist in
[releasing.md](releasing.md) bumps both).
