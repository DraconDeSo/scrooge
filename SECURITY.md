# Security policy

## Supported versions

Security fixes are released for the latest minor version only. Scrooge updates itself
from signed GitHub Releases, so staying on the latest version is the recommended setup.

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Use GitHub's private
vulnerability reporting instead:
[Report a vulnerability](https://github.com/DraconDeSo/scrooge/security/advisories/new).

Include the affected version, your operating system, steps to reproduce and the impact you
expect. You should receive an acknowledgement within a few days; a fix and an advisory are
published once a patched release is available.

## Dependency checks

CI checks the locked Rust packages against OSV and runs npm audit. On Linux, Tauri 2 still
requires GTK 3 bindings that pin `glib` to the 0.18 series. CI reports
[RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) as a known
upstream exception for `glib 0.18.5`. Revisit this exception when Tauri updates its Linux stack.

## How Scrooge handles sensitive data

- **Local reads only.** Agent sessions are discovered from local files and process tables.
  Prompts, tool arguments and replies are never sent anywhere.
- **Network calls.** Only two optional, read-only calls leave the machine:
  - `GET https://api.github.com/copilot_internal/user` for Copilot quota, authenticated with
    a token obtained on demand from the GitHub CLI (`gh auth token`).
  - `GET https://api.anthropic.com/api/oauth/usage` for Claude plan limits, authenticated
    with the Claude Code CLI login (`~/.claude/.credentials.json`).
  The update check contacts `github.com` to download `latest.json` and signed installers.
- **No token storage.** Tokens are read at the moment of each request and kept only in
  memory for that request. Scrooge never refreshes or rewrites another tool's credentials.
- **Redaction.** Common secret patterns (API keys, GitHub tokens, bearer headers…) are
  redacted before transcript text is displayed.
- **Signed updates.** Update packages are verified against the public key embedded in the
  app before they are installed.
