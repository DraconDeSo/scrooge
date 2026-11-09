# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.1] - 2026-11-09

### Added

- Usage by model under each provider in `quota & usage`, for a selectable period.
- Button above the session list to jump to completed sessions and back.
- Tokens per model call in a session's details for Claude Code, Codex and Copilot, with the
  input, output and cache split and the tools each call requested.

### Changed

- Completed sessions now stay at the bottom of the list with a green check.

### Fixed

- Live Claude Code token and turn totals no longer count a response once per transcript
  line; they had been inflated, by about three times in one measured session.
- Count Gemini usage from live Antigravity desktop and IDE transcripts, without requiring
  a provider CLI login.
- Keep cached Codex allowance visible after Codex exits and show a local usage fallback for
  every running provider when an official quota is unavailable.
- Show Copilot session consumption even when GitHub CLI is not installed, and avoid
  double-counting Antigravity history when default and configured Gemini homes coincide.
- The compact header no longer clips or breaks its text: name and buttons sit on the first
  row, session counts and token totals on the second.

## [0.1.0]

### Added

- Desktop widget for Windows, macOS and Linux with a compact always-on-top panel, a
  full-screen dashboard and a tray icon.
- Session monitoring for Claude Code, Codex CLI and OpenCode (via the abtop engine),
  plus GitHub Copilot CLI / GitHub Copilot app, Gemini CLI and Grok Build.
- Read / written / cached token totals in the header and on every session.
- GitHub Copilot quota for every account logged into the GitHub CLI; each account is shown
  as its own provider with its own color. AI credits can be shown as credits or in USD.
- Claude subscription limits (5h / 7d) via the Claude Code login, and local Claude token
  consumption (current 5-hour block, 24 hours, 7 days) from transcripts.
- Token-rate chart stacked by provider or by project, with a toggleable legend.
- Ten themes, each with a dark and a light variant, plus a system-following mode.
- Settings: start with the OS, start in tray, global shortcut, refresh intervals, density,
  provider colors, warning thresholds, provider visibility, sorting.
- Disk usage per provider and storage location in the full-screen view.
- Enable/disable each provider; disabled providers are not scanned and make no network calls.
- Data Export modal (`E` shortcut and per-row quick-export buttons) with 5 presets (`Project tokens by provider`, `Provider usage statistics`, `Daily usage timeline`, `Quota & billing snapshot`, `Live sessions & storage`) in `CSV`, `JSON` and `Markdown`, live preview, clipboard copy and local file export.
- Ultra-lightweight runtime mode: pauses WebView timers and IPC payloads when hidden in the system tray, caps snapshot arrays, interns historical usage strings (`Arc<str>`) and shares a single OS process table scan across collectors.
- Cross-platform distribution via native installers (`.msi`, `.exe`, `.dmg`, `.deb`, `.rpm`, `.AppImage`), portable archives, PowerShell / POSIX one-liner installers, and manifests for Winget, Scoop, Chocolatey, Homebrew Cask, Arch AUR, Flatpak and Snapcraft.
- Complete user manual (`docs/manual.md`) and updated high-resolution UI documentation screenshots.
- Signed in-app updates from GitHub Releases.

[Unreleased]: https://github.com/DraconDeSo/scrooge/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/DraconDeSo/scrooge/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/DraconDeSo/scrooge/releases/tag/v0.1.0
