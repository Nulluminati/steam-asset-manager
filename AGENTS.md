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
- `src/imaging.rs`: crop and resize. `prepare(image, kind)` gives the
  exact Steam size, or trims a logo's transparent borders.
- `src/config.rs`: `Config` (`steam_root`, `openai_model`) as TOML at
  `config::default_path()`. A missing file or field gives the default.
- `src/secrets.rs`: API keys. `Secrets::get_key(Service)` reads the env
  var, then the `KeyStore`. `Secrets::system()` uses the OS keyring
  (service `steam-asset-manager`); tests pass a fake store and env lookup.
  `set_key` with an empty key deletes the stored key.
- `src/sgdb.rs`: SteamGridDB client. `SgdbClient::candidates(entry, kind)`
  lists thumbnail and full URLs (a non-Steam entry uses the first name
  search match; HTTP 404 or no match gives an empty list).
  `download(candidate, kind)` fetches without the key and runs `prepare`.
- `src/openai.rs`: OpenAI image client. `OpenAiClient::generate(prompt, kind)`
  makes four new images; `modify(&DynamicImage, prompt, kind)` makes four
  edits in a multipart request. Both ask for a custom size per kind (see
  "OpenAI facts"), send `n = 4`, and run `imaging::prepare` on each image.
  A Logo request asks for a transparent background. `OpenAiError` maps
  HTTP 401 to `BadKey`, other failures to `Api` with OpenAI's message or
  the raw body, and bad result data to `Decode`, `Image` or
  `MissingImage`.
- `tests/fixtures/steam/`: a fake Steam root for tests. Account ID
  `39734273` is the most recent user and owns the `shortcuts.vdf` fixture.
  `appcache/librarycache/` holds app 440 in the flat layout and app 620 in
  the per-app folder layout (with one content-hash subfolder).
- `tests/fixtures/libraries/{a,b}/`: two library folders with app
  manifests. Tests write a `libraryfolders.vdf` that points at them, because
  that file holds absolute paths.

Not written yet: `app.rs`, `ui/`. Update this map when a card adds or moves a module.

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

## OpenAI facts

Read from OpenAI's docs on 2026-11-07:
https://developers.openai.com/api/docs/models,
https://developers.openai.com/api/docs/guides/image-generation and
https://developers.openai.com/api/reference/resources/images/methods/edit.
Correct this list when a fact turns out to be wrong.

- Base URL: `https://api.openai.com`. Endpoints: `POST /v1/images/generations`
  (JSON) and `POST /v1/images/edits` (multipart).
- Image models on the models page: `gpt-image-2` (snapshot
  `gpt-image-2-2026-04-21`), `gpt-image-2.5-sunburst`,
  `gpt-image-2.5-flare`, `gpt-image-2.6-sunburst` and
  `gpt-image-2.6-flare`. Sunburst is "detailed visual work"; Flare is
  "fast iteration". The app default is `gpt-image-2.6-sunburst`.
- Custom `size` is a `WIDTHxHEIGHT` string. Width and height must be
  multiples of 16. The aspect ratio must be between 1:3 and 3:1. No edge
  may exceed 3840 pixels. Total pixels must be in 655,360 to 8,294,400.
  Sizes above 2560x1440 are experimental. The guide states these rules
  in a sentence about the 2.5 models. The 2.6 model pages point at the
  same guide, so custom sizes on 2.6 are inferred until a real request
  confirms them.
- Request size per asset kind (each one obeys the rules above; the client
  then runs `imaging::prepare`):
  - Portrait: `832x1248` (exact 2:3, the Steam ratio).
  - Wide: `1472x688` (exact 920:430, the Steam ratio).
  - Hero: `1920x640` (3:1, the widest ratio allowed; `fit_exact` crops it).
  - Logo: `1536x1024` (then `prepare` trims transparent borders).
- A transparent background needs `background: "transparent"` plus
  `output_format` `png` or `webp`. The default output format is `png`.
- Both endpoints take `model`, `prompt`, `n` (1 to 10), `size`,
  `background` and `output_format`; the edits reference page lists them.
  The edits endpoint also needs at least one `image` file part.

## Dependency notes

- `reqwest` 0.13 names its TLS feature `rustls` (not `rustls-tls`). The crate
  uses `default-features = false` with `rustls`, `http2`, `json`, `multipart`,
  `query`.
- `reqwest` 0.13 puts `RequestBuilder::query` behind the `query` feature.
- A `reqwest::Error` shows the request URL. Call `without_url()` on errors
  from requests that put a key in the URL (Steam Web API).
