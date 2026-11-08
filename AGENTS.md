# steam-asset-manager

Desktop app in Rust with the iced GUI library (0.14). It sets the custom
Steam library art (portrait capsule, wide capsule, hero, logo) for Steam
games and non-Steam shortcuts. Art comes from SteamGridDB, from the OpenAI
image API, or from an OpenAI edit of the current art. Targets Linux, macOS
and Windows.

## Read first

`docs/plan.md` is the source of truth:

- **Decisions**: platforms, API keys, sizing, OpenAI model, UI.
- **Steam facts**: file paths, VDF formats, grid file names, sizes. Correct
  this section when a fact turns out to be wrong.
- **Cards**: the build order. Each herdboard card implements one
  "Card N" section.

The approved UI layout is `docs/mockup-main-screen.png`.

## Module map

- `src/main.rs`: starts the iced app.
- `src/lib.rs`: declares the library modules, so tests and `main.rs` share
  them and unused public items do not fail clippy.
- `src/app.rs`: iced `State`, `Message`, `update`, `view`.
- `src/ui/`: `library.rs`, `detail.rs`, `panel.rs`, `settings.rs`.
- `src/assets.rs`: `AssetKind`.
- `src/steam/`: `paths.rs`, `users.rs`, `library.rs`, `owned.rs`,
  `shortcuts.rs`, `grid.rs`, `defaults.rs`. `mod.rs` holds `SteamError`
  and the shared text VDF helpers (keys match without regard to case).
- `src/imaging.rs`: crop and resize.
- `src/config.rs`, `src/secrets.rs`: settings and API keys.
- `src/sgdb.rs`: SteamGridDB client.
- `src/openai.rs`: OpenAI image client.
- `tests/fixtures/steam/`: a fake Steam root for tests. Account ID
  `39734273` is the most recent user and owns the `shortcuts.vdf` fixture.
- `tests/fixtures/libraries/{a,b}/`: two library folders with app
  manifests. Tests write a `libraryfolders.vdf` that points at them, because
  that file holds absolute paths.

Not written yet: `app.rs`, `ui/`, `steam/grid.rs`, `steam/defaults.rs`,
`imaging.rs`, `config.rs`, `secrets.rs`, `sgdb.rs`, `openai.rs`. Update this
map when a card adds or moves a module.

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

All four must pass with clean output. CI (`.github/workflows/ci.yml`) runs
them on Linux, macOS and Windows.

## Rules

- Work test-first (TDD) on every card.
- Tests never touch a real Steam folder, the real keyring or a real API.
  Use the fixtures under `tests/fixtures/steam/`, temp folders (`tempfile`),
  a fake key store behind a trait, and `wiremock` with an injectable base URL.
- Keep every OS-specific path in `src/steam/paths.rs`.
- The dev machine (Fedora) has no Steam. Manual testing happens on another
  PC with Steam; see "Testing" in `docs/plan.md`.

## Dependency notes

- `reqwest` 0.13 names its TLS feature `rustls` (not `rustls-tls`). The crate
  uses `default-features = false` with `rustls`, `http2`, `json`, `multipart`,
  `query`.
- `reqwest` 0.13 puts `RequestBuilder::query` behind the `query` feature.
- A `reqwest::Error` shows the request URL. Call `without_url()` on errors
  from requests that put a key in the URL (Steam Web API).
