# Third-party notices

## abtop

Scrooge is built on top of [**abtop**](https://github.com/graykode/abtop), the terminal
monitor for AI coding agents created by **Tae Hwan Jung**. Its session-discovery and
transcript-parsing engine is the foundation of this project, and we are grateful for it.

- Location in this repository: [`crates/abtop`](crates/abtop)
- License: MIT — Copyright (c) Tae Hwan Jung — see [`crates/abtop/LICENSE`](crates/abtop/LICENSE)
- Based on upstream commit `4b96568666c1f3b887202d14b0a1c6171315e520`
- Modifications are listed in [`crates/abtop/README.md`](crates/abtop/README.md)

The original copyright and license notice are retained as required by the MIT License.

## Other dependencies

Scrooge is built with [Tauri](https://tauri.app) and many Rust crates and npm packages,
each under its own license (mostly MIT and/or Apache-2.0). Their license texts are available
in the respective packages; `cargo tree` and `npm ls` list the full dependency graph.

## Trademarks

Claude, Codex, OpenCode, GitHub Copilot, Gemini and Grok are trademarks of their respective
owners. Scrooge is an independent project and is not affiliated with or endorsed by any of
them; product names are used only to identify the tools it can monitor.
