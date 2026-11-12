---
name: steam-asset-manager-release
description: "Cut a steam-asset-manager release: choose the semantic version from the commits since the last tag, write the CHANGELOG entry, then run bin/release.py. Use when the user asks to release steam-asset-manager, publish a build, or bump its version."
---

# Release steam-asset-manager

Run this in a clone of `nulluminati/steam-asset-manager`, on `main`. You choose the version and write the changelog. `bin/release.py` does the file edits, the commit, the tag and the push. The tag push starts `.github/workflows/release.yml`, which builds the archives and publishes the GitHub Release. See `docs/releasing.md`.

## 1. List the changes

```bash
git describe --tags --abbrev=0          # the last release tag, such as v0.2.0
git log v0.2.0..HEAD --format='%h %s%n%b'
```

When there is no tag yet, list every commit: `git log --format='%h %s%n%b'`.

## 2. Choose the version

Read the Conventional Commit type of each commit. The highest bump wins:

| Commit | Bump |
|---|---|
| `!` after the type, such as `feat!:`, or a `BREAKING CHANGE:` footer | major (`1.4.2` → `2.0.0`) |
| `feat` | minor (`1.4.2` → `1.5.0`) |
| `fix`, `perf` | patch (`1.4.2` → `1.4.3`) |
| `chore`, `test`, `ci`, `docs`, `refactor`, `style`, `build` | no bump alone |

While the version is `0.x`, a breaking change bumps the minor (`0.4.2` → `0.5.0`), not the major. `feat` and `fix` bump as in the table.

When there is no tag yet, the first release is `0.2.0`: `Cargo.toml` already has `0.1.0`, and the new version must be higher.

When the user asks for a pre-release, add `-rc.N` (or `-beta.N`) to the version, such as `0.2.0-rc.1`. Count `N` up from the last pre-release tag of that version.

When every commit is of a "no bump" type, tell the user that nothing needs a release, and stop.

## 3. Write the CHANGELOG entry

Edit `CHANGELOG.md`. Add a section under `## [Unreleased]`, with today's date:

```markdown
## [Unreleased]

## [0.3.0] - 2026-11-11

### Added

- The SteamGridDB tab pages through all results.

### Fixed

- ...
```

- Group the entries under `Added`, `Changed`, `Fixed` and `Removed`. Leave out an empty group.
- Write each entry for a person who uses the app: what changed for them, not the commit subject.
- Leave out `chore`, `test`, `ci` and `docs` commits, unless the change matters to a user.
- Move any text under `## [Unreleased]` into the new section. Keep the empty `## [Unreleased]` heading.
- A pre-release gets its own section, such as `## [0.3.0-rc.1] - 2026-11-11`. The release after it repeats the full list of changes since the last release.

## 4. Ask the user

Show the user the version, the reason for the bump, and the CHANGELOG section. Wait for approval. Change the text when they ask.

## 5. Run the release

Do not commit anything yourself. The release commit includes the CHANGELOG change.

```bash
python3 bin/release.py 0.3.0
```

The script refuses, and changes nothing, when the tree has other changes, the branch is not `main`, `main` differs from `origin/main`, the tag exists, or the CHANGELOG has no section for the version. Tell the user the message. Do not work around a refusal.

With `--no-push`, the script commits and tags but does not push. It prints the push command.

After the push, give the user the link to the workflow run: `gh run list --workflow release.yml --limit 1`. Remind them to test the Windows zip on the Windows PC (see "After the release" in `docs/releasing.md`).
