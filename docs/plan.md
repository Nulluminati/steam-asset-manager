# steam-asset-manager: project plan

## Goal
`steam-asset-manager` is a desktop app in Rust with the iced GUI library. It lets a Steam user set the custom library art (portrait capsule, wide capsule, hero, logo) for Steam games and non-Steam shortcuts. Art comes from SteamGridDB, from the OpenAI image API, or from an OpenAI edit of the current art.

## Decisions
- **Platforms: Linux, macOS and Windows from the start.** Put every OS-specific path in one module (`src/steam/paths.rs`). CI runs on Linux and Windows. Manual testing runs on another PC with Steam (Windows or macOS), because the Fedora dev machine has no Steam.
- **Milestone 1 is the full scope.** Library browse, SteamGridDB pull, OpenAI generate and OpenAI modify all ship in milestone 1.
- **Entries: Steam games and non-Steam shortcuts.** Non-Steam shortcuts come from the binary `shortcuts.vdf`.
- **Steam games: owned and installed.** Use the Steam Web API `GetOwnedGames` with a Steam Web API key, merged with the installed app manifests. Without a Steam key, the app shows installed games only.
- **API keys (SteamGridDB, OpenAI, Steam Web API) live in the system keyring** (`keyring` crate: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows). Env vars `STEAMGRIDDB_API_KEY`, `OPENAI_API_KEY` and `STEAM_API_KEY` override the keyring, for development and tests.
- **Sizing: request a custom OpenAI size, then auto center-crop.** Ask the image API for a custom size that obeys its rules and matches the Steam ratio where possible. Then center-crop to the exact Steam ratio and resize to the exact pixel size. No crop UI. See "OpenAI facts" in `AGENTS.md` for the per-kind sizes.
- **Overwrite with no backup.** Applying an asset replaces the file in the grid folder. No history and no undo.
- **No Steam process detection.** The app does not check if Steam runs and does not restart it. Steam reads the grid folder only at start; the README says so.
- **Icons are out of scope.** Manage portrait, wide, hero and logo only.
- **Account: most recent user only.** Read `config/loginusers.vdf` and use the account with `MostRecent "1"`, else the account with the newest `Timestamp`. No account switcher.
- **OpenAI model: `gpt-image-2.6-sunburst` by default; the user can change it in the config.** OpenAI's models page (https://developers.openai.com/api/docs/models, read 2026-11-07) lists `gpt-image-2`, `gpt-image-2.5-sunburst`, `gpt-image-2.5-flare`, `gpt-image-2.6-sunburst` and `gpt-image-2.6-flare`. Sunburst is for "detailed visual work" and Flare is for "fast iteration". Library art is a one-time job, so the default picks quality: `gpt-image-2.6-sunburst`. The image guide (https://developers.openai.com/api/docs/guides/image-generation, read 2026-11-07) gives the custom size rules: width and height as multiples of 16, an aspect ratio between 1:3 and 3:1, no edge above 3840, and 655,360 to 8,294,400 total pixels. No exact Steam size passes those rules, so the client requests a custom size per kind and then center-crops.
- **UI: the approved mockup `docs/mockup-main-screen.png`.** Dark theme, steel blue accent. Left: searchable library list; only a non-Steam shortcut gets a "Non-Steam" tag (this differs from the mockup, which tags every entry). Center: game title, then the four slots laid out like the Steam library page (hero on top, portrait and wide capsule side by side, logo below). Each slot shows its target size and three buttons: SteamGridDB, Generate, Modify. Right: a panel with a segmented control `SteamGridDB | Generate | Modify`, a Target dropdown, a prompt box (Generate and Modify tabs), a primary action button and a 2x2 grid of candidates, each with an Apply button. A slot button selects that slot as the target (highlighted border) and opens the matching tab.
- **Name:** crate and binary `steam-asset-manager`.

## Crates
`iced` (latest, features `image` and `tokio`), `tokio`, `reqwest` (features `rustls-tls`, `json`, `multipart`), `serde` (feature `derive`), `serde_json`, `keyvalues-parser` (text VDF), `image`, `keyring`, `directories`, `thiserror`, `anyhow`, `crc32fast`, `toml`, `base64`. Windows only: `winreg`. Dev: `tempfile`, `wiremock`.

## Module map
- `src/main.rs`: starts the iced app.
- `src/app.rs`: iced `State`, `Message`, `update`, `view`.
- `src/ui/`: `library.rs`, `detail.rs`, `panel.rs`, `settings.rs`.
- `src/assets.rs`: `AssetKind`.
- `src/steam/`: `paths.rs`, `users.rs`, `library.rs`, `owned.rs`, `shortcuts.rs`, `grid.rs`, `defaults.rs`.
- `src/imaging.rs`: crop and resize.
- `src/config.rs`, `src/secrets.rs`: settings and API keys.
- `src/sgdb.rs`: SteamGridDB client.
- `src/openai.rs`: OpenAI image client.
- `tests/fixtures/steam/`: a fake Steam root for tests.

## Steam facts
Verify each one against a real Steam install or current community docs before relying on it. Correct this section if a fact is wrong.

- Steam root:
  - Linux: `~/.local/share/Steam`, then `~/.steam/steam`, then Flatpak `~/.var/app/com.valvesoftware.Steam/.local/share/Steam`.
  - macOS: `~/Library/Application Support/Steam`.
  - Windows: registry `HKCU\Software\Valve\Steam\SteamPath` (crate `winreg`), fallback `C:\Program Files (x86)\Steam`.
- Account: `<root>/config/loginusers.vdf` (text VDF) keys are SteamID64. Current clients do not write `MostRecent`; each user has a `timestamp` (Unix seconds, lower case) instead, and the newest one is the last sign-in (seen on a real Windows install, November 2026). Account ID = SteamID64 − 76561197960265728. User folder: `<root>/userdata/<accountid>/`.
- Owned Steam games: `GET https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key=<key>&steamid=<SteamID64>&include_appinfo=1&include_played_free_games=1` returns `response.games[]` with `appid` and `name`. The profile's game details must be public, or the key must belong to the same account. Get a key at https://steamcommunity.com/dev/apikey.
- Installed Steam games: `<root>/steamapps/libraryfolders.vdf` lists library folders. Each folder has `steamapps/appmanifest_<appid>.acf` with `appid` and `name`. The current format is `"0" { "path" "..." }`; old files use `"1" "D:\\SteamLibrary"`. The root itself is usually entry `"0"`. Paths use escaped backslashes.
- Text VDF key case is not stable across Steam versions (`MostRecent` and `mostrecent`, `AppName` and `appname`). Match keys without regard to case.
- Non-Steam shortcuts: `<root>/userdata/<accountid>/config/shortcuts.vdf` (binary VDF). Each entry has `appid` (stored as i32; use it as u32), `AppName`, `Exe`. If `appid` is missing, compute `crc32(Exe + AppName) | 0x80000000`.
- Grid folder: `<root>/userdata/<accountid>/config/grid/` (create it if missing). File names per asset kind:
  - Portrait capsule 600x900: `<appid>p.png`
  - Wide capsule 920x430: `<appid>.png`
  - Hero 1920x620: `<appid>_hero.png`
  - Logo (no fixed size, transparent PNG): `<appid>_logo.png`. Do not touch `<appid>.json` (logo position).
  - Steam also accepts `.jpg`. When writing a slot, delete the other extension of the same slot so the new file wins.
- Default (Steam's own) art, used as the "current image" when no custom file exists and as the Modify source: local cache `<root>/appcache/librarycache/`. The layout differs between Steam versions; support both `<appid>_library_600x900.jpg` style files (old clients) and the per-app `<appid>/` subfolder (current clients). In the subfolder, a file can sit one level deeper in a folder with a content-hash name, for example `<appid>/<hash>/library_600x900.jpg`; other hash-named files there (such as the icon) are not library art. Community reports, not verified on a real install (November 2026). Fallback: the Steam CDN `https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/<appid>/{library_600x900.jpg,header.jpg,library_hero.jpg,logo.png}`. For some newer apps the CDN serves the portrait only under a content-hash path, so the fixed path returns 404; the app then shows no default art. Non-Steam shortcuts have no default art.

## Cards

| N | Title | Depends on |
|---|-------|------------|
| 1 | Steam data readers | the scaffold only |
| 2 | Grid files, default art and image sizing | card 1 (`AssetKind`, Steam root, account) |
| 3 | Config and API key storage | the scaffold only |
| 4 | SteamGridDB client | cards 2 and 3 (`fit_exact`, keys) |
| 5 | OpenAI image client | cards 2 and 3 (`fit_exact`, `nearest_openai_size`, keys, model ID) |
| 6 | App state and messages | cards 1 to 5 |
| 7 | Main screen views and settings | card 6 |

Every card uses TDD: write the failing test, run it, write the code, run it again. Tests never touch a real Steam folder, the real keyring or a real API. Test output must be clean.

### Card 1: Steam data readers
1. **Asset kinds** (`src/assets.rs`). `enum AssetKind { Portrait, Wide, Hero, Logo }` with `target_size() -> Option<(u32, u32)>` (Logo returns `None`), `grid_file_stem(appid)` and a display label. Unit-test every file name.
2. **Steam root discovery** (`src/steam/paths.rs`). `fn find_steam_root(env: &dyn PathEnv) -> Option<PathBuf>`, with a small trait so tests can fake the home folder and the Windows registry. Test each OS candidate order with temp folders. Accept an optional override path (card 3 adds the config value).
3. **Most recent user** (`src/steam/users.rs`). Parse `loginusers.vdf`, pick `MostRecent "1"`, return both the SteamID64 and the account ID. Fixture tests: one user, several users, no `MostRecent`.
4. **Installed games** (`src/steam/library.rs`). Parse `libraryfolders.vdf` and all `appmanifest_*.acf`. Return `Vec<LibraryEntry { appid: u32, name: String, source: EntrySource, installed: bool }>`, sorted by name. Fixture tests with two library folders.
5. **Owned games** (`src/steam/owned.rs`). `fetch_owned(key, steamid64, base_url)` calls `GetOwnedGames`. `merge(installed, owned)` dedupes by appid: installed entries keep `installed: true`, owned-only entries get `installed: false`. `wiremock` tests: normal list, empty `response` (private profile), 403 (bad key). Merge unit tests. Return errors as values; the app falls back to installed games.
6. **Non-Steam shortcuts** (`src/steam/shortcuts.rs`). Read-only binary VDF parser (map 0x00, string 0x01, int32 0x02, end 0x08). Return entries with `EntrySource::NonSteam`. Fixture test with a hand-built `shortcuts.vdf` byte array; test the crc32 fallback.

### Card 2: Grid files, default art and image sizing
1. **Grid read and write** (`src/steam/grid.rs`). `current_custom(grid_dir, appid, kind) -> Option<PathBuf>` and `write_asset(grid_dir, appid, kind, &DynamicImage)`. Writes PNG, deletes the other-extension twin, creates the folder. Temp-folder tests.
2. **Default art** (`src/steam/defaults.rs`). Find the local cache file; else download from the CDN URL. Tests: cache hit for both layouts (fixtures), CDN fallback with `wiremock` (make the base URL injectable).
3. **Image sizing** (`src/imaging.rs`). `fn fit_exact(img, (w, h)) -> DynamicImage`: center-crop to the target ratio, then resize with Lanczos3. `fn nearest_openai_size(target, supported: &[(u32, u32)]) -> (u32, u32)` by aspect ratio. For Logo: no crop; trim fully transparent borders. Unit tests with generated images: exact output size, crop is centered, logo trim.

### Card 3: Config and API key storage
1. **Config** (`src/config.rs`). TOML in the `directories` config folder: `steam_root` (optional), `openai_model`. Load with defaults when the file is missing; save. Temp-folder tests.
2. **Secrets** (`src/secrets.rs`). `enum Service { SteamGridDb, OpenAi, SteamWeb }`. `get_key(Service) -> Option<String>` reads the env var first, then the keyring (service name `steam-asset-manager`). `set_key` writes the keyring. Put keyring calls behind a trait so tests use a fake store. Test env-var priority and fake-store reads and writes.

### Card 4: SteamGridDB client
1. **Client** (`src/sgdb.rs`). Base `https://www.steamgriddb.com/api/v2` (injectable for tests), header `Authorization: Bearer <key>`. Endpoints: `/grids/steam/{appid}?dimensions=600x900` (portrait), `/grids/steam/{appid}?dimensions=920x430,460x215` (wide), `/heroes/steam/{appid}`, `/logos/steam/{appid}`. For non-Steam entries: `/search/autocomplete/{name}`, take the first game ID, then the same endpoints with `/game/{id}`. Return thumbnail and full URLs.
2. **Download** the full image, then apply `fit_exact` (SteamGridDB sizes can differ a little).
3. **Tests** with `wiremock`: each asset kind, the non-Steam search path, 401 (bad key), no results.

### Card 5: OpenAI image client
1. **Read OpenAI's docs first** (see Decisions). Record the model ID and supported sizes in `AGENTS.md` and set the `openai_model` default in `src/config.rs`.
2. **Client** (`src/openai.rs`). Base URL injectable for tests. `generate(prompt, kind, n = 4)` calls `POST /v1/images/generations`. `modify(source_png, prompt, kind, n = 4)` calls `POST /v1/images/edits` (multipart). Decode `b64_json`. Logo: request a transparent background. Build each prompt from the user text plus a kind hint (for example "wide banner, no text" for Hero). Pass every result through `fit_exact`, or request the exact size if the model accepts it. The API can return fewer images than asked for; the client returns the list it got and the app handles an empty one.
3. **Tests** with `wiremock`: generate, modify (check the multipart parts), an API error body.

### Card 6: App state and messages
1. **State and messages** (`src/app.rs`, `src/main.rs`). iced Elm-style app: `State` (entries, filter text, selected entry, target `AssetKind`, panel tab, prompt, candidates, busy flag, status or error text), `Message` enum, `update`, `view` (a plain placeholder view is enough; card 7 builds the real one).
2. **Start-up** loads config, finds the Steam root and account, and loads installed games, owned games and shortcuts in a `Task`. If owned games fail, show the reason in the status line and keep installed games.
3. **Async work.** Run all HTTP calls in `Task`s. Set the busy flag while a call runs.
4. **Tests** for `update`: select entry, slot button sets target and tab, filter text, Apply writes through `grid::write_asset` (temp root), a missing key shows a clear message, an API error shows in the status line, an empty candidate list shows a message instead of an empty grid.

### Card 7: Main screen views and settings
Build the layout of `docs/mockup-main-screen.png`.
1. **Library list** (`src/ui/library.rs`): search box, entries (a non-Steam shortcut has a "Non-Steam" tag; a Steam game has no tag), owned-only games with a dim "Not installed" note.
2. **Detail view** (`src/ui/detail.rs`): game title and the four slots (custom art, else default art, else an empty placeholder), each with its target size and the SteamGridDB, Generate and Modify buttons. The target slot has a highlighted border.
3. **Right panel** (`src/ui/panel.rs`): the three tabs, Target dropdown, prompt box, action button, 2x2 candidates with Apply. The Modify tab uses the current slot image as the source and is disabled when no image exists.
4. **Settings dialog** (`src/ui/settings.rs`): three password fields for the API keys (writes the keyring), Steam root override, model ID.
5. **Tests**: keep view logic thin. Unit-test any helper that decides what a view shows (slot image choice, Modify disabled state, tag text).

## Testing
- Unit and integration tests as listed per card. Keep Steam data in fixtures under `tests/fixtures/steam/` (a fake root with `config/loginusers.vdf`, `steamapps/`, `userdata/<id>/config/shortcuts.vdf`, `appcache/librarycache/`).
- HTTP tests use `wiremock`. No test calls the real Steam Web API, SteamGridDB or OpenAI.
- By hand, after card 7, on another PC with Steam (Windows or macOS):
  1. Set all three keys in Settings. Check that owned-but-not-installed games appear.
  2. Select a Steam game. Check that the default art shows.
  3. Apply a SteamGridDB hero. Check that `userdata/<id>/config/grid/<appid>_hero.png` exists at 1920x620.
  4. Generate a portrait capsule. Check that the file is 600x900.
  5. Modify the wide capsule. Check that the file is 920x430.
  6. Apply art to a non-Steam shortcut.
  7. Restart Steam. Check that the new art shows in the library.

## Out of scope
- Icons.
- Backups, history or undo.
- Detecting, closing or restarting Steam.
- Account switching.
- A manual crop UI.
- Editing `shortcuts.vdf` or logo position (`<appid>.json`).
- Packaging and installers (Flatpak, .dmg, .msi).
