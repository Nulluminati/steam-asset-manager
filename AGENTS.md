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

- `src/main.rs`: starts the iced app. A Windows release build uses the
  GUI subsystem, so no console window opens; a debug build keeps the
  console for panic messages.
- `build.rs`: on Windows, embeds `assets/icon.ico` in the `.exe` with
  `winresource` (needs the Windows SDK resource compiler, part of the Visual
  Studio C++ build tools).
- `assets/`: `icon.png` (256x256, window and taskbar icon) and `icon.ico`
  (16 to 256 px, Windows executable icon). Codex generated the artwork.
- `src/lib.rs`: declares the library modules, so tests and `main.rs` share
  them and unused public items do not fail clippy.
- `src/app.rs`: the iced `State`, `Message`, `update` and `view`, plus
  `Services` (keys, Steam environment, HTTP base URLs, config path) so tests
  replace every outside dependency. `State::boot` returns the start-up
  `Task`; `load_library` reads the config, the Steam root, the account,
  installed games, shortcuts and owned games, and reports a failure in the
  status line. Each panel action returns a `Task` and sets the busy flag;
  a second press does nothing while a call runs. `Message::CandidatesArrived`
  carries the appid and `AssetKind` the images were made for: a reply that
  no longer matches the selection or target is dropped, and
  `Message::ApplyPressed` writes with that stored pair, never with the slot
  the user looks at now. Selecting an entry (and each library reload)
  starts a `Task` that loads the art of all four slots (`load_slot_art`:
  custom grid file, else Steam's default art, else empty) into
  `Message::SlotArtLoaded`; a reply for another entry is dropped. Apply
  puts the written image in its slot. Modify needs art in the target slot.
  `Message::SettingsSaved` writes only the changed keys, saves the config,
  then reloads the library; a failure keeps the dialog open. Opening the
  settings starts a `Task` that looks for Codex (`Message::CodexChecked`).
  Generate and Modify use the `ImageBackend` of `Config::image_source`:
  the OpenAI API (needs the key) or Codex (`Services::codex`,
  `Services::codex_home`), and ask for `Config::images_per_request`
  images. A SteamGridDB search keeps every result in `SgdbResults` (bound
  to the appid and `AssetKind`), shows `GRID_CELLS` (4) per page with
  `Message::SgdbPrevious` and `Message::SgdbNext`, and loads the
  thumbnails of the current page only (`Message::SgdbThumbsLoaded`).
  `Next` past the loaded list asks for the next API page after a full
  one (`Message::SgdbListed`). Apply on SteamGridDB downloads the full
  image (`Message::SgdbDownloaded`) and then writes it. `view` calls
  `ui::view`.
- `src/ui/`: the iced views. `mod.rs` holds `theme` (dark, steel blue
  accent), `window_icon` (decodes `assets/icon.png`) and `view` (library, detail and panel in a row; the settings
  dialog in a `stack` on top). `library.rs` (search box, rows with
  `tag_text` and `install_note`), `detail.rs` (`SlotArt`, `slot_caption`,
  the four slot frames; a click on a frame sets the target), `panel.rs`
  (tabs, `TargetOption` dropdown, prompt `text_editor`, action button,
  2x2 candidates; on SteamGridDB, the page thumbnails and the pager with
  `page_counter`; on Modify, the source thumbnail from `modify_source`;
  `modify_enabled`, `action_label`), `settings.rs`
  (`SettingsForm`: key inputs prefilled from `Secrets::get_key`,
  `changed_keys`, `to_config`, the image source radio buttons, the
  "Images per request" `pick_list` of `IMAGE_COUNT_OPTIONS`; `codex_note`,
  `install_steps`, `model_note`). Keep view code thin and unit-test the
  helpers that decide what a view shows.
- `src/assets.rs`: `AssetKind`.
- `src/steam/`: `paths.rs`, `users.rs`, `library.rs`, `owned.rs`,
  `shortcuts.rs`, `grid.rs`, `defaults.rs`. `mod.rs` holds `SteamError`
  and the shared text VDF helpers (keys match without regard to case).
- `src/imaging.rs`: crop and resize. `prepare(image, kind)` gives the
  exact Steam size, or trims a logo's transparent borders.
- `src/config.rs`: `Config` (`steam_root`, `openai_model`, `image_source`,
  `images_per_request`) as TOML at `config::default_path()`. A missing file
  or field gives the default. `ImageSource` is `OpenAiApi` (default) or
  `Codex`. `images_per_request` is 1 by default; a stored value outside 1
  to `MAX_IMAGES_PER_REQUEST` (4) moves to the nearest edge on load.
- `src/secrets.rs`: API keys. `Secrets::get_key(Service)` reads the env
  var, then the `KeyStore`. `Secrets::system()` uses the OS keyring
  (service `steam-asset-manager`); tests pass a fake store and env lookup.
  `set_key` with an empty key deletes the stored key.
- `src/sgdb.rs`: SteamGridDB client. `SgdbClient::candidates(entry, kind)`
  lists thumbnail and full URLs (a non-Steam entry uses the first name
  search match; HTTP 404 or no match gives an empty list).
  `candidates_page(entry, kind, page)` asks for one API page (`page`
  query parameter, from 0; `SGDB_PAGE_SIZE` 50 is assumed).
  `download(candidate, kind)` fetches without the key and runs `prepare`;
  `thumbnail(candidate)` gives the thumbnail bytes.
- `src/openai.rs`: OpenAI image client. `OpenAiClient::generate(prompt, kind,
  count)` makes new images; `modify(&DynamicImage, prompt, kind, count)`
  makes edits in a multipart request. Both ask for a custom size per kind
  (see "OpenAI facts"), send `n = count`, and run `imaging::prepare` on
  each image.
  A Logo request asks for a transparent background. `OpenAiError` maps
  HTTP 401 to `BadKey`, other failures to `Api` with OpenAI's message or
  the raw body, and bad result data to `Decode`, `Image` or
  `MissingImage`.
- `src/codex.rs`: Codex CLI image source. `CodexRunner` (trait, like
  `KeyStore`) runs `codex`; `SystemCodex` finds it on the `PATH`
  (`codex.cmd` or `codex.exe` on Windows, started with `CREATE_NO_WINDOW`).
  `CodexClient::generate` and `modify` start `count` runs at the same time in
  a temp work folder, with `instruction` (one line), `exec_args`,
  `parse_events` and `saved_image`, then run `imaging::prepare`. One
  failed run does not hide the others. `check` gives a `CodexStatus` for
  the settings dialog. `paths::codex_home` is the one Codex home lookup.
- `tests/fixtures/steam/`: a fake Steam root for tests. Account ID
  `39734273` is the most recent user and owns the `shortcuts.vdf` fixture.
  `appcache/librarycache/` holds app 440 in the flat layout and app 620 in
  the per-app folder layout (with one content-hash subfolder).
- `tests/fixtures/libraries/{a,b}/`: two library folders with app
  manifests. Tests write a `libraryfolders.vdf` that points at them, because
  that file holds absolute paths.
- `bin/release.py` and `bin/release_notes.py`: the release scripts (Python
  standard library only). `bin/tests/` holds their `unittest` tests.
- `skills/steam-asset-manager-release/SKILL.md`: how an agent cuts a release.
- `.github/workflows/release.yml`: on a `v*` tag, checks the version, calls
  `ci.yml`, packages the archives and `SHA256SUMS`, and publishes the GitHub
  Release.

Update this map when a card adds or moves a module.

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

All four must pass with clean output. CI (`.github/workflows/ci.yml`) runs
them on Linux and Windows.

The release scripts have their own tests. CI runs them on Linux:

```sh
python3 -I -m unittest discover -s bin/tests
```

## Releases

Read `docs/releasing.md`. To cut a release, follow
`skills/steam-asset-manager-release/SKILL.md`: you pick the version and write
the `CHANGELOG.md` section, the user approves, then `bin/release.py X.Y.Z`
commits, tags and pushes. Never edit the version in `Cargo.toml` by hand, and
never commit a release yourself.

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

## Codex facts

Proven with codex-cli 0.167.0 on Linux on 2026-11-10 (herdboard card
`tign`). Correct this list when a fact turns out to be wrong.

- Command: `codex exec --json --skip-git-repo-check --ephemeral -s read-only
  -C <dir> [-i <image>] -- "<instruction>"`, stdin closed (else Codex
  waits for input). `-i` takes every value after it, so `--` ends it.
- Events are JSON lines: `thread.started` (`thread_id`), `item.completed`
  with an `agent_message` item (`text`), `error` (`message`) and
  `turn.failed` (`error.message`).
- Codex saves the image in `<codex home>/generated_images/<thread_id>/`.
  Codex home is `CODEX_HOME`, else `~/.codex`. The app takes the newest
  PNG there and deletes it; the fallback is a PNG path in the last agent
  message.
- A hero run gave 2172x724 (3:1) in about 36 s. Each run uses 77k to 155k
  input tokens of the plan, because Codex loads the user's AGENTS.md and
  skills.
- Not tested yet: portrait and wide shapes, Modify with `-i`, Windows
  (`codex.cmd`, sandbox set-up, openai/codex#45871).

## Dependency notes

- `reqwest` 0.13 names its TLS feature `rustls` (not `rustls-tls`). The crate
  uses `default-features = false` with `rustls`, `http2`, `json`, `multipart`,
  `query`.
- `reqwest` 0.13 puts `RequestBuilder::query` behind the `query` feature.
- A `reqwest::Error` shows the request URL. Call `without_url()` on errors
  from requests that put a key in the URL (Steam Web API).
