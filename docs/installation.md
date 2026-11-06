# Installation and updates

Installers for every supported system are attached to each
[GitHub Release](https://github.com/DraconDeSo/scrooge/releases/latest).

| System | Download | Notes |
| --- | --- | --- |
| Windows 10 / 11 (x64) | `Scrooge_<version>_x64-setup.exe` | Recommended. Per-user install, no admin rights needed. |
| Windows (managed) | `Scrooge_<version>_x64_en-US.msi` | For deployment tools (Intune, GPO, SCCM). |
| macOS 11+ Apple silicon | `Scrooge_<version>_aarch64.dmg` | M1 and later. |
| macOS 11+ Intel | `Scrooge_<version>_x64.dmg` | |
| Debian / Ubuntu | `scrooge_<version>_amd64.deb` | `sudo apt install ./scrooge_<version>_amd64.deb` |
| Fedora / openSUSE | `scrooge-<version>-1.x86_64.rpm` | `sudo dnf install ./scrooge-<version>-1.x86_64.rpm` |
| Any Linux | `scrooge_<version>_amd64.AppImage` | `chmod +x` and run. Supports in-app updates. |

## Windows

1. Download and run the `-setup.exe`.
2. If SmartScreen shows *"Windows protected your PC"* (unsigned builds), choose
   **More info → Run anyway**.
3. Scrooge starts in the tray (bottom-right). Left-click the icon to show the panel.

WebView2 is preinstalled on Windows 11 and recent Windows 10; the installer downloads it if
it is missing.

## macOS

1. Open the `.dmg` and drag **Scrooge** to **Applications**.
2. On first launch of a build that is not notarized, macOS blocks it: right-click the app →
   **Open** → **Open**, or run `xattr -dr com.apple.quarantine /Applications/Scrooge.app`.
3. The icon appears in the menu bar.

## Linux

Install the `.deb` or `.rpm`, or run the `.AppImage`. The tray icon needs an
AppIndicator-capable desktop (GNOME with the *AppIndicator* extension, KDE Plasma, Xfce,
Cinnamon…). Runtime libraries: WebKitGTK 4.1 and libayatana-appindicator3, already pulled in
by the `.deb`/`.rpm` packages.

## Optional tools

| Tool | Needed for |
| --- | --- |
| [GitHub CLI](https://cli.github.com/) (`gh`), logged in | GitHub Copilot quota. Every account in `gh auth status` is shown. |
| Claude Code CLI login (`claude` → `/login`) | Claude plan limits (5h / 7d %). Local Claude token usage works without it. |
| `sqlite3` on `PATH` | OpenCode sessions. |

## Updating

Scrooge updates itself from signed GitHub Releases.

- **Automatic check**: at startup and every 6 hours (Settings → About → *Check for updates
  automatically*). When a new version exists, a banner appears with **Install & restart**.
- **Manual check**: Settings → About → **Check for updates**.
- Packages are verified against the public key built into the app before installation; a
  tampered or unsigned package is rejected.
- Settings, colors and window position are kept across updates
  (`%APPDATA%\scrooge\settings.json`, `~/Library/Application Support/scrooge/`,
  `~/.config/scrooge/`).

In-app updates work for the Windows installers, the macOS app and the Linux AppImage. The
`.deb` and `.rpm` packages are updated by downloading and installing the new package (or
through your package manager if you host a repository).

## Uninstalling

- **Windows**: Settings → Apps → Scrooge → Uninstall.
- **macOS**: move `/Applications/Scrooge.app` to the Trash.
- **Linux**: `sudo apt remove scrooge` / `sudo dnf remove scrooge`, or delete the AppImage.

To remove settings as well, delete the `scrooge` folder in the configuration directory
listed above. Disable *Start with Windows/at login* first if you enabled it.
