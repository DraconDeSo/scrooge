# Releasing

Releases are built automatically by GitHub Actions ([`.github/workflows/release.yml`](../.github/workflows/release.yml))
for Windows (x64), macOS (Apple silicon and Intel) and Linux (x64).

When a branch is merged into `main` with a version in `package.json` + `src-tauri/Cargo.toml`
that has not been published yet (or when triggered manually / via a `v*` tag):

1. **`preflight`** checks that `package.json` and `src-tauri/Cargo.toml` have the same version
   and checks whether `v<version>` is already published on GitHub. (If `v<version>` is already
   published, normal commits on `main` skip the release build automatically.)
2. **`build`** compiles all 4 OS targets (`Windows x64`, `macOS Apple silicon`, `macOS Intel`,
   `Linux x64`), builds the standalone `abtop` CLI, packages portable archives (`.zip` / `.tar.gz`),
   and uploads all installers + `latest.json` to a **Draft GitHub Release** (`v<version>`).
3. **`verify`** checks all bundle types, verifies the Minisign updater signatures against
   `tauri.conf.json`, generates `SHA256SUMS.txt` and auto-updates the package manager manifests
   (`scrooge.scoop.json`, `DraconDeSo.Scrooge.winget.yaml`, `scrooge.rb`, `PKGBUILD`), attaching
   them to the Draft Release.
4. **`publish`** waits for approval on the `release` GitHub Environment (if configured) and
   publishes the Draft Release live with one click.

## Setting up the 1-click Approval Gate on GitHub

To require manual approval before a Draft Release goes live after merging into `main`:

1. Open your GitHub repository → **Settings → Environments**.
2. Create an environment named **`release`**.
3. Check **Required reviewers** and add your GitHub account.

Now, whenever a PR bumping the version is merged into `main`, GitHub Actions builds and
verifies all artifacts into a Draft Release and pauses at **`Approve & publish release`**,
letting you inspect the draft and click **Approve and deploy** (or publish directly from
the **Releases** tab).

## Free updater signing key (`TAURI_SIGNING_PRIVATE_KEY`)

Tauri's in-app updater uses **Minisign** (100% free, local Ed25519 key pair) so the app only
installs updates signed by your repository:

1. Generate a key pair locally (free):

   ```bash
   npx tauri signer generate -w ~/.tauri/scrooge.key
   ```

2. Put the **public** key (`~/.tauri/scrooge.key.pub`, a single base64 line) in
   `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`, and commit it.
3. Add two repository secrets (*Settings → Secrets and variables → Actions*):
   - `TAURI_SIGNING_PRIVATE_KEY` — contents of `~/.tauri/scrooge.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — its password (if set)

## Optional OS code signing (Windows Authenticode & Apple Developer ID)

OS-level code signing is **completely optional**. If the secrets below are not configured,
the workflow still builds, signs updater bundles with Minisign, and publishes the release
for free (users simply click *More info → Run anyway* on Windows SmartScreen or right-click
*Open* on macOS on their very first install).

If you later obtain OS certificates, add their secrets in *Settings → Secrets and variables → Actions*
and the pipeline will enable them automatically:

- **Windows**: `WINDOWS_CERTIFICATE` (base64 `.pfx`) and `WINDOWS_CERTIFICATE_PASSWORD`.
- **macOS**: `APPLE_CERTIFICATE` (base64 `.p12`), `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password) and
  `APPLE_TEAM_ID`.

## Release workflow (step-by-step)

1. In your feature/release branch, update `CHANGELOG.md` and bump `version` in both
   `package.json` and `src-tauri/Cargo.toml` (run `cargo check` so `Cargo.lock` updates).
2. Open a PR and **merge into `main`**.
3. The **Release** workflow builds all platforms, verifies signatures, generates
   `SHA256SUMS.txt` and package manifests, and prepares the Draft Release `vX.Y.Z`.
4. Approve the `release` environment deployment in GitHub Actions (or click **Publish release**
   on the Draft Release in GitHub).
