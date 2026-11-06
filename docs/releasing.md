# Releasing

Releases are built by GitHub Actions ([`.github/workflows/release.yml`](../.github/workflows/release.yml))
for Windows (x64), macOS (Apple silicon and Intel) and Linux (x64), and published as a
**draft** GitHub Release together with `latest.json`, the manifest the in-app updater reads.

## One-time setup: update-signing key

The updater only installs packages signed with the project's key.

1. Generate a key pair (keep the password):

   ```bash
   npx tauri signer generate -w ~/.tauri/scrooge.key
   ```

2. Put the **public** key (`~/.tauri/scrooge.key.pub`, a single base64 line) in
   `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`, and commit it.
3. Add two repository secrets (*Settings → Secrets and variables → Actions*):
   - `TAURI_SIGNING_PRIVATE_KEY` — contents of `~/.tauri/scrooge.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — its password
4. Keep a backup of the private key. If it is lost, existing installations can no longer
   update automatically (users must reinstall manually once).

## OS code signing

Tagged releases require Windows Authenticode and macOS Developer ID signatures. The
certificates must belong to the publisher or team. Add the credentials in the
repository's Actions secrets.

- For macOS, use a [Developer ID Application certificate](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/).
  Set `APPLE_CERTIFICATE` (base64 .p12), `APPLE_CERTIFICATE_PASSWORD` and
  `APPLE_SIGNING_IDENTITY`. For notarization also set `APPLE_ID`, `APPLE_PASSWORD`
  (app-specific password) and `APPLE_TEAM_ID`.
- For Windows, the certificate-store integration uses `WINDOWS_CERTIFICATE` (base64
  .pfx) and `WINDOWS_CERTIFICATE_PASSWORD`. If the issuer keeps the private key in a
  hardware or cloud signer, use its Tauri
  [signing command integration](https://v2.tauri.app/distribute/sign/windows/).

Run **Release** manually on `main` to build test artifacts and verify the updater
signatures without a tag or GitHub release. Once platform credentials are configured,
enable `sign_platform` in that manual run to check Windows and macOS trust as well.

## Release checklist

1. Update `CHANGELOG.md`: move **Unreleased** entries under the new version.
2. Bump the version in `package.json` and `src-tauri/Cargo.toml` (same value), then run
   `cargo check` so `Cargo.lock` is updated.
3. Commit: `chore: release vX.Y.Z`.
4. Tag and push:

   ```bash
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin main --tags
   ```

5. Wait for the **Release** workflow to pass, including installer signature
   verification. Check that the draft contains every installer and `latest.json`,
   then **Publish** it. Installed apps see the update on their next check.

## How updates reach users

`src-tauri/tauri.conf.json` points the updater to
`https://github.com/DraconDeSo/scrooge/releases/latest/download/latest.json`. That URL always
resolves to the newest *published* (non-draft, non-prerelease) release, so publishing the
draft is what rolls an update out. Before the first published release, the app reports that
no release is available. If a release is published but its update feed is unavailable,
the app reports an error. To withdraw a bad release, unpublish it (turn it back into a
draft): clients then see the previous release as latest.
