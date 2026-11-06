# Troubleshooting

### A running agent does not appear
- Check that its provider is not hidden (Settings → Sessions → Providers) and that *Show idle
  sessions* is on if the agent is waiting for input.
- Press `R` to refresh. New sessions can take one refresh interval to show up.
- OpenCode needs `sqlite3` on `PATH`.
- Gemini CLI and Grok Build are matched to their most recent session file: start a prompt so
  the file is written.

### Copilot quota shows an error
- `GitHub CLI (gh) not found`: install the [GitHub CLI](https://cli.github.com/).
- `gh has no token for '<login>'`: run `gh auth login` with that account.
- `no Copilot access for this account`: the account has no Copilot subscription; hide it in
  Settings → Quota & accounts.

### Claude shows "% limits: needs Claude Code CLI login"
Plan limits need the Claude Code CLI login (`claude` → `/login`). Local token consumption is
shown regardless. If you only use the Claude desktop app, limits are not available.

### The panel is off screen or too small
Use the tray menu → *Compact panel*: the window is moved back inside the screen. Deleting
`compact_bounds` from `settings.json` restores the default position.

### The global shortcut does not work
Another application may own the combination. Settings → General shows the registration error;
choose a different combination.

### Updates fail
- Check that `github.com` is reachable from your network.
- `.deb` / `.rpm` installations update by installing the new package manually.
- A signature error means the downloaded package was not produced by the official release
  pipeline: download the installer from the Releases page instead.

### Linux: no tray icon
Install an AppIndicator extension (GNOME) or use a desktop with tray support; the panel can
still be toggled with the global shortcut.
