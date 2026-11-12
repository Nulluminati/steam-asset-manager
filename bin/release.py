"""Release steam-asset-manager: bump the version, commit, tag vX.Y.Z and push.

Usage: python3 bin/release.py X.Y.Z [--no-push]

Write the CHANGELOG.md section `## [X.Y.Z] - YYYY-MM-DD` first; the release commit includes it. The script
checks everything before it changes anything: the version, a clean main that equals origin/main, a new
tag and the changelog section. The tag push starts the GitHub workflow that builds and publishes the
release. A pre-release version such as 0.2.0-rc.1 gives a GitHub pre-release.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# Semantic Versioning without build metadata: X.Y.Z or X.Y.Z-pre.release. The same rule as
# bin/release_notes.py.
VERSION = re.compile(r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?")
VERSION_LINE = re.compile(r'^version = "([^"]*)"$', re.M)
VERSION_FILE = "Cargo.toml"
LOCK_FILE = "Cargo.lock"
CHANGELOG = "CHANGELOG.md"


class ReleaseError(Exception):
    """A check or a command failed; the message says what to do."""


def command(argv: list[str], root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(argv, cwd=root, capture_output=True, text=True)


def run(argv: list[str], root: Path) -> str:
    """The command's output; ReleaseError with its error output when it fails."""
    try:
        result = command(argv, root)
    except OSError as error:
        raise ReleaseError(f"{argv[0]} did not run: {error}") from error
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip() or f"exit code {result.returncode}"
        raise ReleaseError(f"{' '.join(argv)} failed: {detail}")
    return result.stdout


def current_version(root: Path) -> str:
    match = VERSION_LINE.search((root / VERSION_FILE).read_text(encoding="utf-8"))
    if not match:
        raise ReleaseError(f"{VERSION_FILE} has no version line")
    return match[1]


def precedence(version: str) -> tuple | None:
    """A sort key in Semantic Versioning order: 0.2.0-rc.2 < 0.2.0-rc.10 < 0.2.0. None when invalid."""
    match = VERSION.fullmatch(version)
    if not match:
        return None
    core = (int(match[1]), int(match[2]), int(match[3]))
    if match[4] is None:
        # A release sorts after each of its pre-releases.
        return core, 1, ()
    # Numeric identifiers sort as numbers and before alphanumeric ones.
    ids = tuple((0, int(part), "") if part.isdigit() else (1, 0, part) for part in match[4].split("."))
    return core, 0, ids


def check(version: str, root: Path) -> None:
    """Refuse the release when a precondition fails. Changes nothing."""
    new = precedence(version)
    if new is None:
        raise ReleaseError(f"The version must be X.Y.Z or X.Y.Z-pre.N, such as 0.2.0 or 0.2.0-rc.1, not {version!r}")
    current = current_version(root)
    old = precedence(current)
    if old is None or new <= old:
        raise ReleaseError(f"The version must be higher than {current}")
    changes = [line[3:] for line in run(["git", "status", "--porcelain"], root).splitlines() if line.strip()]
    others = [path for path in changes if path != CHANGELOG]
    if others:
        raise ReleaseError("Commit or remove these changes first: " + ", ".join(others))
    branch = run(["git", "rev-parse", "--abbrev-ref", "HEAD"], root).strip()
    if branch != "main":
        raise ReleaseError(f"Release from main, not {branch}")
    run(["git", "fetch", "origin", "main"], root)
    local, remote = run(["git", "rev-parse", "main", "origin/main"], root).split()
    if local != remote:
        raise ReleaseError("main differs from origin/main. Pull or push first")
    tag = f"v{version}"
    if command(["git", "rev-parse", "-q", "--verify", f"refs/tags/{tag}"], root).returncode == 0:
        raise ReleaseError(f"The tag {tag} exists")
    changelog = root / CHANGELOG
    text = changelog.read_text(encoding="utf-8") if changelog.exists() else ""
    # The same rule as bin/release_notes.py, so the Release workflow does not refuse a pushed tag.
    heading = rf"^## \[{re.escape(version)}\] - \d{{4}}-\d{{2}}-\d{{2}}\n"
    match = re.search(heading + r"(.*?)(?=^## |\Z)", text + "\n", re.M | re.S)
    if not match:
        raise ReleaseError(f"Add the section `## [{version}] - YYYY-MM-DD` to {CHANGELOG} first")
    if not match[1].strip():
        raise ReleaseError(f"The {CHANGELOG} section of {version} is empty. Write the changes in it first")


def bump(version: str, root: Path) -> None:
    """Write the version in Cargo.toml and Cargo.lock; restore Cargo.toml when the lock update fails."""
    path = root / VERSION_FILE
    original = path.read_text(encoding="utf-8")
    path.write_text(VERSION_LINE.sub(f'version = "{version}"', original, count=1), encoding="utf-8")
    try:
        # Updates only this crate's entry in Cargo.lock, not the dependencies.
        run(["cargo", "update", "--workspace"], root)
    except ReleaseError:
        path.write_text(original, encoding="utf-8")
        raise


def release(version: str, root: Path, push: bool) -> None:
    tag = f"v{version}"
    check(version, root)
    bump(version, root)
    run(["git", "add", VERSION_FILE, LOCK_FILE, CHANGELOG], root)
    run(["git", "commit", "-m", f"chore: release {tag}"], root)
    run(["git", "tag", "-a", tag, "-m", tag], root)
    finish = f"git push origin main {tag}"
    if not push:
        print(f"Tagged {tag}. Push it to publish the release: {finish}")
        return
    try:
        run(["git", "push", "origin", "main"], root)
        run(["git", "push", "origin", tag], root)
    except ReleaseError as error:
        raise ReleaseError(f"{error}\nThe commit and the tag exist. Finish with: {finish}") from error
    print(f"Released {tag}. The Release workflow builds and publishes it on GitHub.")


def main(args: list[str] | None = None, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser(prog="release.py", description="Release steam-asset-manager X.Y.Z.")
    parser.add_argument("version", help="the new version, X.Y.Z or X.Y.Z-pre.N")
    parser.add_argument("--no-push", action="store_true", help="commit and tag, but do not push")
    options = parser.parse_args(sys.argv[1:] if args is None else args)
    try:
        release(options.version, root, push=not options.no_push)
    except ReleaseError as error:
        print(f"release: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
