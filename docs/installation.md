# Installation and updates

Installers, portable archives and package-manager manifests for every supported system are attached to each
[GitHub Release](https://github.com/DraconDeSo/scrooge/releases/latest) along with `SHA256SUMS.txt`.

## Quick install (terminal one-liner)

- **Windows (PowerShell)**:
  ```powershell
  irm https://raw.githubusercontent.com/DraconDeSo/scrooge/main/scripts/install.ps1 | iex
  ```
  *(Pass `-Portable` to extract the portable `.zip` into `%LOCALAPPDATA%\Programs\Scrooge` without running an installer).*

- **macOS & Linux**:
  ```bash
  curl -fsSL https://raw.githubusercontent.com/DraconDeSo/scrooge/main/scripts/install.sh | sh
  ```
  *(Automatically selects `.dmg` on macOS, `.deb`/`.rpm` when `apt`/`dnf` is available on Linux, or installs the `.AppImage` to `~/.local/bin/scrooge`, verifying SHA-256 against `SHA256SUMS.txt`).*

## Package managers

| Platform | Manager | Command |
| --- | --- | --- |
| Windows | **Winget** | `winget install DraconDeSo.Scrooge` |
| Windows | **Scoop** | `scoop install https://github.com/DraconDeSo/scrooge/releases/latest/download/scrooge.scoop.json` |
| macOS | **Homebrew** | `brew install --cask https://github.com/DraconDeSo/scrooge/releases/latest/download/scrooge.rb` |
| Arch Linux | **makepkg / AUR** | `curl -fsSLO https://github.com/DraconDeSo/scrooge/releases/latest/download/PKGBUILD && makepkg -si` |
| Any (CLI TUI) | **Cargo** | `cargo install --git https://github.com/DraconDeSo/scrooge abtop` |

## Direct downloads

| System | Installer | Portable | Notes |
| --- | --- | --- | --- |
| Windows 10 / 11 (x64) | `Scrooge_<version>_x64-setup.exe` | `Scrooge_<version>_x64-portable.zip` | Recommended. Per-user install, no admin rights needed. |
| Windows (managed) | `Scrooge_<version>_x64_en-US.msi` | `Scrooge_<version>_x64-portable.zip` | For deployment tools (Intune, GPO, SCCM). |
| macOS 11+ Apple silicon | `Scrooge_<version>_aarch64.dmg` | `Scrooge_<version>_aarch64-portable.tar.gz` | M1 and later. |
| macOS 11+ Intel | `Scrooge_<version>_x64.dmg` | `Scrooge_<version>_x64-portable.tar.gz` | |
| Debian / Ubuntu | `scrooge_<version>_amd64.deb` | `scrooge_<version>_amd64-portable.tar.gz` | `sudo apt install ./scrooge_<version>_amd64.deb` |
| Fedora / openSUSE | `scrooge-<version>-1.x86_64.rpm` | `scrooge_<version>_amd64-portable.tar.gz` | `sudo dnf install ./scrooge-<version>-1.x86_64.rpm` |
| Any Linux | `scrooge_<version>_amd64.AppImage` | `scrooge_<version>_amd64-portable.tar.gz` | `chmod +x` and run. Supports in-app updates. |

## Windows

1. Download and run the `-setup.exe` (or run the PowerShell one-liner / Scoop / Winget).
2. If SmartScreen shows *"Windows protected your PC"* (on builds without a paid Authenticode certificate), choose
   **More info → Run anyway**.
3. Scrooge starts in the tray (bottom-right). Left-click the icon to show the panel.

WebView2 is preinstalled on Windows 11 and recent Windows 10; the installer downloads it if
it is missing.

## macOS

1. Open the `.dmg` and drag **Scrooge** to **Applications** (or use `install.sh`, which clears the quarantine attribute automatically).
2. On first launch of a build that is not notarized with a paid Apple Developer ID, macOS blocks it: right-click the app →
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
through `install.sh` / your package manager).

## Uninstalling

- **Windows**: Settings → Apps → Scrooge → Uninstall (or `scoop uninstall scrooge` / `winget uninstall DraconDeSo.Scrooge`).
- **macOS**: move `/Applications/Scrooge.app` to the Trash (or `brew uninstall --cask scrooge`).
- **Linux**: `sudo apt remove scrooge` / `sudo dnf remove scrooge`, or delete `~/.local/bin/scrooge`.

To remove settings as well, delete the `scrooge` folder in the configuration directory
listed above. Disable *Start with Windows/at login* first if you enabled it.
