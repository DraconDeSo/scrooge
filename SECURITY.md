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

CI checks the locked Rust packages against OSV and runs npm audit. The Linux
build uses a [reviewable glib backport](vendor/glib-0.18.5/SCROOGE-BACKPORT.md)
for RUSTSEC-2024-0429. A Linux regression test exercises both iterator directions
with the patched crate compiled under optimization.

## How Scrooge handles sensitive data

- **Local usage history.** Scrooge reads supported local provider logs and imports in
  the Scrooge settings folder. Historical usage keeps timestamps, project paths,
  model identifiers, token counts and supplied billing totals in memory. It discards
  message text. Prompts, tool arguments and replies are never sent anywhere.
- **Network calls.** Optional quota checks query the provider directly:
  - `GET https://api.github.com/copilot_internal/user` for Copilot quota, authenticated with
    a token obtained on demand from the GitHub CLI (`gh auth token`).
  - `GET https://api.anthropic.com/api/oauth/usage` for Claude plan limits, authenticated
    with the Claude Code CLI login (`~/.claude/.credentials.json`).
  Automatic billing is opt-in. Scrooge calls the configured provider's fixed HTTPS
  billing API for the selected dates and caches totals in memory. The Google Cloud
  adapter starts a BigQuery job under the configured byte limit. The update check
  contacts `github.com` to download `latest.json` and signed installers. If the feed
  is unavailable, it can query the public release status on `api.github.com`.
- **No token storage.** Scrooge reads credentials on demand and keeps them in memory
  for the request. Billing configuration stores credential names only. Google Cloud
  can obtain a temporary access token through the installed Google Cloud CLI.
- **Redaction.** Common secret patterns (API keys, GitHub tokens, bearer headers…) are
  redacted before transcript text is displayed.
- **Signed updates.** Update packages are verified against the public key embedded in the
  app before they are installed.
