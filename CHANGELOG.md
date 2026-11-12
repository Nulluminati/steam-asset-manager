# Changelog

All notable changes to steam-asset-manager are in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and steam-asset-manager uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). See [docs/releasing.md](docs/releasing.md).

## [Unreleased]

## [0.2.0] - 2026-11-12

### Added

- The library lists installed Steam games, owned games and non-Steam shortcuts of the most recent Steam user, with a search box. A tag marks non-Steam shortcuts.
- The detail view shows the current art of the four slots: portrait capsule, wide capsule, hero and logo. A click on a slot selects it as the target.
- The SteamGridDB tab searches art for the selected game, shows 4 thumbnails per page, and pages through all results.
- The Generate tab makes new art from a prompt with the OpenAI image API.
- The Modify tab edits the current art of the target slot from a prompt, and shows that source image.
- Codex (with a ChatGPT plan) can make the images instead of the OpenAI API. It needs no OpenAI API key.
- Apply writes the chosen image to Steam's grid folder at the exact Steam size. A logo gets its transparent borders trimmed.
- The settings dialog stores the SteamGridDB, OpenAI and Steam Web API keys in the system keyring, and sets the Steam folder, the OpenAI model, the image source and the number of images per request (1 to 4).
- Archives for Windows and Linux. On Windows, the app opens without a console window and shows its icon.
