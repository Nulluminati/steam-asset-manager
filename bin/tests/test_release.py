"""bin/release.py: the checks before a release, and the version bump, commit, tag and push."""

import contextlib
import io
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts import load_script

release = load_script("release")

CARGO = (
    '[package]\nname = "steam-asset-manager"\nversion = "0.1.0"\nedition = "2024"\n\n'
    '[dependencies]\nimage = "0.25.10"\n\n[build-dependencies]\nversion = "9.9.9"\n'
)
CHANGELOG = (
    "# Changelog\n\n## [Unreleased]\n\n"
    "## [0.2.0] - 2026-11-11\n\n### Added\n\n- A thing.\n\n"
    "## [0.2.0-rc.1] - 2026-11-10\n\n### Added\n\n- A test build.\n"
)
MAIN = "1111111111111111111111111111111111111111"


class FakeGit:
    """Answers the release's commands like a clean main that equals origin/main."""

    def __init__(self):
        self.calls = []
        self.status = ""
        self.branch = "main"
        self.origin = MAIN
        self.tags = set()
        self.fail = None

    def __call__(self, argv, root):
        self.calls.append(argv)
        if self.fail and argv[: len(self.fail)] == self.fail:
            return subprocess.CompletedProcess(argv, 1, "", "it broke\n")
        out = ""
        if argv == ["git", "status", "--porcelain"]:
            out = self.status
        elif argv == ["git", "rev-parse", "--abbrev-ref", "HEAD"]:
            out = self.branch + "\n"
        elif argv == ["git", "rev-parse", "main", "origin/main"]:
            out = f"{MAIN}\n{self.origin}\n"
        elif argv[:4] == ["git", "rev-parse", "-q", "--verify"]:
            tag = argv[4].removeprefix("refs/tags/")
            return subprocess.CompletedProcess(argv, 0 if tag in self.tags else 1, "", "")
        return subprocess.CompletedProcess(argv, 0, out, "")

    def writes(self):
        """The commands that change something: everything after the checks."""
        reads = (["git", "status"], ["git", "rev-parse"], ["git", "fetch"])
        return [argv for argv in self.calls if not any(argv[: len(read)] == read for read in reads)]


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "Cargo.toml").write_text(CARGO)
        (self.root / "CHANGELOG.md").write_text(CHANGELOG)
        self.git = FakeGit()

    def set_version(self, version):
        (self.root / "Cargo.toml").write_text(CARGO.replace('version = "0.1.0"', f'version = "{version}"'))

    def run_release(self, *argv, expect=0):
        out, err = io.StringIO(), io.StringIO()
        with (
            patch.object(release, "command", self.git),
            contextlib.redirect_stdout(out),
            contextlib.redirect_stderr(err),
        ):
            code = release.main(list(argv), root=self.root)
        self.assertEqual(code, expect, f"stdout={out.getvalue()!r} stderr={err.getvalue()!r}")
        return out.getvalue(), err.getvalue()

    def assert_unchanged(self, cargo=CARGO):
        self.assertEqual((self.root / "Cargo.toml").read_text(), cargo)
        self.assertEqual(self.git.writes(), [])

    def refuse(self, *argv, message):
        cargo = (self.root / "Cargo.toml").read_text()
        _, err = self.run_release(*argv, expect=1)
        self.assertIn(message, err)
        self.assert_unchanged(cargo)

    def test_a_release_bumps_commits_tags_and_pushes(self):
        out, _ = self.run_release("0.2.0")
        cargo = (self.root / "Cargo.toml").read_text()
        self.assertIn('[package]\nname = "steam-asset-manager"\nversion = "0.2.0"\n', cargo)
        # Only the first version line changes.
        self.assertIn('[build-dependencies]\nversion = "9.9.9"\n', cargo)
        self.assertEqual(
            self.git.writes(),
            [
                ["cargo", "update", "--workspace"],
                ["git", "add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md"],
                ["git", "commit", "-m", "chore: release v0.2.0"],
                ["git", "tag", "-a", "v0.2.0", "-m", "v0.2.0"],
                ["git", "push", "origin", "main"],
                ["git", "push", "origin", "v0.2.0"],
            ],
        )
        self.assertIn(["git", "fetch", "origin", "main"], self.git.calls)
        self.assertIn("v0.2.0", out)

    def test_a_pre_release_bumps_to_the_pre_release_version(self):
        self.run_release("0.2.0-rc.1")
        self.assertIn('version = "0.2.0-rc.1"\n', (self.root / "Cargo.toml").read_text())
        self.assertIn(["git", "tag", "-a", "v0.2.0-rc.1", "-m", "v0.2.0-rc.1"], self.git.writes())

    def test_a_release_follows_its_pre_release(self):
        self.set_version("0.2.0-rc.1")
        self.run_release("0.2.0")
        self.assertIn('version = "0.2.0"\n', (self.root / "Cargo.toml").read_text())

    def test_no_push_stops_after_the_tag(self):
        out, _ = self.run_release("0.2.0", "--no-push")
        self.assertEqual(self.git.writes()[-1], ["git", "tag", "-a", "v0.2.0", "-m", "v0.2.0"])
        self.assertNotIn("push", [argv[1] for argv in self.git.writes()])
        self.assertIn("git push origin main v0.2.0", out)

    def test_an_uncommitted_changelog_goes_into_the_release_commit(self):
        self.git.status = " M CHANGELOG.md\n"
        self.run_release("0.2.0")
        self.assertIn(["git", "commit", "-m", "chore: release v0.2.0"], self.git.writes())

    def test_refuses_a_version_that_is_not_semver(self):
        for version in ("0.2", "v0.2.0", "0.2.0-", "0.2.0-rc..1", "0.2.0+build.1", "latest"):
            with self.subTest(version=version):
                self.refuse(version, message="X.Y.Z")

    def test_refuses_a_version_that_is_not_higher(self):
        self.refuse("0.1.0", message="higher than 0.1.0")
        self.refuse("0.0.9", message="higher than 0.1.0")
        self.refuse("0.1.0-rc.1", message="higher than 0.1.0")

    def test_orders_pre_releases_by_semver(self):
        self.set_version("0.2.0-rc.2")
        self.refuse("0.2.0-rc.1", message="higher than 0.2.0-rc.2")
        self.refuse("0.2.0-beta.9", message="higher than 0.2.0-rc.2")
        (self.root / "CHANGELOG.md").write_text(CHANGELOG + "\n## [0.2.0-rc.10] - 2026-11-11\n\n- More.\n")
        self.run_release("0.2.0-rc.10")
        self.assertIn('version = "0.2.0-rc.10"\n', (self.root / "Cargo.toml").read_text())

    def test_refuses_a_dirty_working_tree(self):
        self.git.status = " M src/app.rs\n?? notes.txt\n"
        self.refuse("0.2.0", message="src/app.rs")

    def test_refuses_another_branch(self):
        self.git.branch = "feat/x"
        self.refuse("0.2.0", message="main")

    def test_refuses_a_main_that_differs_from_origin(self):
        self.git.origin = "2222222222222222222222222222222222222222"
        self.refuse("0.2.0", message="origin/main")

    def test_refuses_when_the_fetch_fails(self):
        self.git.fail = ["git", "fetch"]
        self.refuse("0.2.0", message="it broke")

    def test_refuses_an_existing_tag(self):
        self.git.tags.add("v0.2.0")
        self.refuse("0.2.0", message="v0.2.0 exists")

    def test_refuses_a_changelog_without_the_section(self):
        self.refuse("0.3.0", message="## [0.3.0] - YYYY-MM-DD")

    def test_refuses_an_empty_changelog_section(self):
        for text in (
            "# Changelog\n\n## [Unreleased]\n\n## [0.2.0] - 2026-11-11\n",
            "# Changelog\n\n## [0.2.0] - 2026-11-11\n\n\n## [0.1.0] - 2026-10-01\n\n- Old.\n",
        ):
            with self.subTest(text=text):
                (self.root / "CHANGELOG.md").write_text(text)
                self.refuse("0.2.0", message="empty")

    def test_refuses_a_missing_changelog(self):
        (self.root / "CHANGELOG.md").unlink()
        self.refuse("0.2.0", message="CHANGELOG.md")

    def test_a_failed_lock_update_restores_cargo_toml(self):
        self.git.fail = ["cargo", "update"]
        _, err = self.run_release("0.2.0", expect=1)
        self.assertIn("it broke", err)
        self.assertEqual((self.root / "Cargo.toml").read_text(), CARGO)
        self.assertEqual(self.git.writes(), [["cargo", "update", "--workspace"]])

    def test_a_failed_push_names_the_command_to_finish(self):
        self.git.fail = ["git", "push", "origin", "main"]
        _, err = self.run_release("0.2.0", expect=1)
        self.assertIn("it broke", err)
        self.assertIn("git push origin main v0.2.0", err)


if __name__ == "__main__":
    unittest.main()
