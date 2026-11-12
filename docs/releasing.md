# Releasing

A release is a `vX.Y.Z` tag on `main` and a GitHub Release. The release has
two archives, a `SHA256SUMS` file, and the `CHANGELOG.md` section as its notes.
The version follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The tag `vX.Y.Z` always equals the version in `Cargo.toml`.

The version is in `Cargo.toml` and `Cargo.lock`. Do not change it by hand;
`bin/release.py` does it.

Releases happen on demand. There is no schedule.

## Cut a release

Ask an agent to release steam-asset-manager. The release skill
(`skills/steam-asset-manager-release/SKILL.md`) tells it to:

1. List the commits since the last tag.
2. Choose the next version from the Conventional Commit types: `feat` is a
   minor bump, `fix` and `perf` are a patch bump, and a breaking change is a
   major bump. While the version is `0.x`, a breaking change is a minor bump.
3. Write the `## [X.Y.Z] - YYYY-MM-DD` section in `CHANGELOG.md`.
4. Show you the version and the section, and wait for your approval.
5. Run `python3 bin/release.py X.Y.Z`.

To release without an agent, write the CHANGELOG section yourself and run the
script.

After the release:

1. Open the GitHub Release. Check that it has both archives and `SHA256SUMS`.
2. Download the Windows zip on the Windows PC.
3. Check the zip: `Get-FileHash steam-asset-manager-X.Y.Z-windows-x86_64.zip`
   must give the hash in `SHA256SUMS`.
4. Start the app from the zip. Check that the window and the taskbar show the
   app icon.

## Pre-releases

A version with a hyphen, such as `0.2.0-rc.1` or `0.2.0-beta.1`, is a
pre-release. The script and the workflow treat it like a release, and GitHub
marks it as a pre-release. It needs its own `CHANGELOG.md` section.

A pre-release sorts before its release: `0.2.0-rc.1` < `0.2.0-rc.2` <
`0.2.0`. So `0.1.0-rc.1` is not possible after `0.1.0`; use `0.2.0-rc.1`.

## bin/release.py

```bash
python3 bin/release.py 0.3.0             # bump, commit, tag and push
python3 bin/release.py 0.3.0-rc.1        # the same, for a pre-release
python3 bin/release.py 0.3.0 --no-push   # bump, commit and tag only
```

The script checks first and changes nothing when a check fails:

- The version is `X.Y.Z` or `X.Y.Z-pre.N` and higher than the version in
  `Cargo.toml`.
- The working tree has no change other than `CHANGELOG.md`.
- The branch is `main`, and `main` equals `origin/main` after a fetch.
- The tag `vX.Y.Z` does not exist.
- `CHANGELOG.md` has the section `## [X.Y.Z] - YYYY-MM-DD` with text.

Then it writes the version in `Cargo.toml` and runs `cargo update --workspace`,
which changes only this crate's entry in `Cargo.lock`. It commits
`Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` as `chore: release vX.Y.Z`. It
creates the annotated tag `vX.Y.Z` and pushes `main` and the tag. When a push
fails, the commit and the tag stay; the message gives the push command to
finish.

## The Release workflow

The tag push starts `.github/workflows/release.yml`:

1. `notes` runs `bin/release_notes.py`. It fails when the tag differs from the
   version in `Cargo.toml`, or when the CHANGELOG section is missing or empty.
2. `ci` calls `.github/workflows/ci.yml`: format, lint, test and release build
   on Linux and Windows. A tag push does not start `ci.yml` by itself, so the
   checks run one time.
3. `publish` packages the binaries, writes `SHA256SUMS`, and runs
   `gh release create`. A tag with a hyphen gets `--prerelease`.

The archives:

- `steam-asset-manager-X.Y.Z-windows-x86_64.zip`
- `steam-asset-manager-X.Y.Z-linux-x86_64.tar.gz`

Each holds one folder `steam-asset-manager-X.Y.Z/` with the binary,
`README.md` and `LICENSE`. These are plain archives, not installers (see "Out
of scope" in `docs/plan.md`).

Only `publish` has `contents: write`. The other jobs can only read.

The Windows `.exe` is not signed. Microsoft Defender SmartScreen shows a
warning the first time it runs. Select "More info", then "Run anyway".

The repository is private, so only people with access to it see the releases.

## Test the workflow

The scripts have tests: `python3 -I -m unittest discover -s bin/tests`. CI
runs them on Linux.

To test the whole workflow, release a pre-release such as `0.2.0-rc.1`. To
see a version mismatch fail, push a tag that differs from `Cargo.toml` on a
test branch, then delete the tag:

```bash
git tag v9.9.9-test.1 && git push origin v9.9.9-test.1
# The "Check the version" job fails: Cargo.toml has another version.
git push origin :refs/tags/v9.9.9-test.1 && git tag -d v9.9.9-test.1
```
