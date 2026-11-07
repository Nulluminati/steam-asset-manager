# steam-asset-manager

A desktop app for the custom art in your Steam library. Pick a game or a
non-Steam shortcut, then set its portrait capsule, wide capsule, hero or
logo. The art can come from SteamGridDB, from the OpenAI image API, or from
an OpenAI edit of the art you already have. The app crops and resizes every
image to the size Steam expects and writes it to your Steam grid folder.

It runs on Linux, macOS and Windows.

## Build

You need a Rust toolchain (stable). On Linux you also need the
`libxkbcommon` development package.

```sh
cargo build --release
./target/release/steam-asset-manager
```

## API keys

Set these in the app's settings. They are stored in your system keyring.

- SteamGridDB API key, for art from SteamGridDB.
- OpenAI API key, to generate or edit art.
- Steam Web API key (optional). Without it, the app lists installed games
  only. With it, the app also lists games you own but have not installed.

## Restart Steam after changes

Steam reads the grid folder only when it starts. Restart Steam to see new
art. The app does not check whether Steam is running.

Applying art replaces the old file. There is no backup or undo.
