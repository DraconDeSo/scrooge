# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
- Signed in-app updates from GitHub Releases.

[Unreleased]: https://github.com/DraconDeSo/scrooge/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/DraconDeSo/scrooge/releases/tag/v0.1.0
