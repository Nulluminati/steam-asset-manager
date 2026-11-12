"""bin/release_notes.py: the version check and the release notes of the Release workflow."""

import contextlib
import io
import tempfile
import unittest
from pathlib import Path

from scripts import load_script

notes = load_script("release_notes")

CHANGELOG = """# Changelog

## [Unreleased]

## [0.2.0] - 2026-11-11

### Added

- Page through all SteamGridDB results.

## [0.2.0-rc.1] - 2026-11-10

### Added

- A test build.

## [0.1.0] - 2026-10-01

### Added

- The library list.
"""


class ReleaseNotesTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.write("0.2.0")
        (self.root / "CHANGELOG.md").write_text(CHANGELOG)

    def write(self, version):
        (self.root / "Cargo.toml").write_text(
            f'[package]\nname = "steam-asset-manager"\nversion = "{version}"\n\n'
            '[dependencies]\nimage = { version = "0.25.10" }\n'
        )

    def run_notes(self, version, expect=0):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = notes.main([version], root=self.root)
        self.assertEqual(code, expect, f"stdout={out.getvalue()!r} stderr={err.getvalue()!r}")
        return out.getvalue(), err.getvalue()

    def test_prints_the_section_of_the_version(self):
        out, _ = self.run_notes("0.2.0")
        self.assertEqual(out, "### Added\n\n- Page through all SteamGridDB results.\n")

    def test_prints_the_section_of_a_pre_release(self):
        self.write("0.2.0-rc.1")
        out, _ = self.run_notes("0.2.0-rc.1")
        self.assertEqual(out, "### Added\n\n- A test build.\n")

    def test_the_last_section_runs_to_the_end(self):
        self.write("0.1.0")
        out, _ = self.run_notes("0.1.0")
        self.assertEqual(out, "### Added\n\n- The library list.\n")

    def test_refuses_a_tag_that_differs_from_cargo_toml(self):
        self.write("0.1.0")
        _, err = self.run_notes("0.2.0", expect=1)
        self.assertIn("v0.2.0", err)
        self.assertIn("Cargo.toml has version 0.1.0", err)

    def test_refuses_a_version_without_a_section(self):
        self.write("0.3.0")
        _, err = self.run_notes("0.3.0", expect=1)
        self.assertIn("## [0.3.0]", err)

    def test_refuses_an_empty_section(self):
        (self.root / "CHANGELOG.md").write_text("# Changelog\n\n## [0.2.0] - 2026-11-11\n\n")
        _, err = self.run_notes("0.2.0", expect=1)
        self.assertIn("empty", err)

    def test_refuses_a_missing_changelog(self):
        (self.root / "CHANGELOG.md").unlink()
        _, err = self.run_notes("0.2.0", expect=1)
        self.assertIn("CHANGELOG.md", err)

    def test_refuses_a_version_that_is_not_semver(self):
        for version in ("main", "0.2", "v0.2.0", "0.2.0-", "0.2.0+build.1"):
            with self.subTest(version=version):
                _, err = self.run_notes(version, expect=1)
                self.assertIn("X.Y.Z", err)


if __name__ == "__main__":
    unittest.main()
