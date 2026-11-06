# Contributing to Scrooge

Thanks for your interest in improving Scrooge! Bug reports, feature ideas, new provider
collectors, themes and documentation fixes are all welcome.

## Ground rules

- Be respectful: this project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
- Open an issue before large changes, so the approach can be agreed first.
- Keep pull requests focused: one feature or fix per PR.
- All code, comments, commit messages and documentation are written in English.

## Development setup

See [docs/development.md](docs/development.md) for prerequisites per OS. In short:

```bash
npm install
npx tauri dev            # run the app with hot reload of the UI
cargo test --workspace   # run all tests
```

## Project layout

| Path | Contents |
| --- | --- |
| `crates/abtop` | Session-collection engine (vendored fork of abtop) and the original terminal UI |
| `src-tauri` | Desktop app: windows, tray, settings, quota and usage services, updater |
| `ui` | Frontend (plain HTML, CSS and JavaScript, no build step) |
| `docs` | User and developer documentation |
| `.github/workflows` | CI and release pipelines |

## Adding a provider

1. Create `crates/abtop/src/collector/<name>.rs` implementing `AgentCollector`.
2. Register it in `MultiCollector::with_hidden_and_claude_config_dirs` (`collector/mod.rs`).
3. Add unit tests with small, anonymised fixture files (never real transcripts).
4. Add the provider to `PROVIDERS` in `ui/app.js` and give it a color in every theme in
   `ui/themes.css`.
5. Document its data source in [docs/providers.md](docs/providers.md).

## Adding a theme

Themes live in `ui/themes.css`. Each theme needs both a `dark` and a `light` block defining
the full set of variables used by the existing themes, including the six `--p-*` provider
colors. Provider colors must stay clearly distinct from each other and readable on the
panel background (WCAG contrast ≥ 3:1 for colors, ≥ 7:1 for body text). Then add the theme
to `THEMES` in `ui/app.js`.

## Before opening a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Update `CHANGELOG.md` under **Unreleased**.
- Update the documentation if behaviour or settings change.
- Never commit real session data, tokens, personal paths or screenshots containing them.

## Commit messages

Use the conventional prefixes: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
