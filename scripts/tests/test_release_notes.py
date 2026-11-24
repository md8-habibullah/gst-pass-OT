"""Tests of scripts/release_notes.py, the GitHub release description writer. Standard library only."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE.parent / "release_notes.py"
sys.path.insert(0, str(HERE.parent))
import release_notes as rn  # noqa: E402

CHANGELOG = """# Changelog

## [Unreleased]

Nothing yet.

## [0.2.0] - 2026-12-01

### Added

* A new thing.

## [0.1.0] - 2026-11-24

First public build.

### Added

* The vault.

### Known limits

* Not audited.
"""

SUMS = (
    "a" * 64 + "  Hexis_0.1.0_amd64.deb\n"
    + "b" * 64 + "  Hexis-0.1.0-1.x86_64.rpm\n"
    + "c" * 64 + "  Hexis_0.1.0_amd64.AppImage\n"
    + "d" * 64 + "  hexis-0.1.0-linux-x86_64.tar.gz\n"
    + "e" * 64 + "  hexis-0.1.0-android-arm64-testkey.apk\n"
)


class Notes(unittest.TestCase):
    def test_changelog_section_is_the_one_for_the_version(self):
        body = rn.changelog_section(CHANGELOG, "0.1.0")
        self.assertIn("The vault.", body)
        self.assertIn("Not audited.", body)
        self.assertNotIn("A new thing.", body)
        self.assertEqual(rn.changelog_section(CHANGELOG, "9.9.9"), "")

    def test_headings_are_pushed_one_level_down(self):
        self.assertEqual(rn.shift_headings("### Added\n* x\n#### Deep\n"), "#### Added\n* x\n##### Deep\n")

    def test_downloads_table_has_a_row_per_attachment_and_flags_the_test_key(self):
        t = rn.downloads_table(SUMS)
        for name in ("Hexis_0.1.0_amd64.deb", "Hexis-0.1.0-1.x86_64.rpm", "Hexis_0.1.0_amd64.AppImage",
                     "hexis-0.1.0-linux-x86_64.tar.gz", "hexis-0.1.0-android-arm64-testkey.apk"):
            self.assertIn(name, t)
        self.assertIn("throw-away key", t)
        self.assertEqual(t.count("\n"), 6)  # header, rule, five rows
        self.assertIn("`" + "a" * 16 + "…`", t)

    def test_the_description_has_every_section_and_the_real_commands(self):
        text = rn.build("0.1.0", "owner/project", "https://owner.github.io/project/", SUMS, CHANGELOG)
        for heading in ("## Install", "## Downloads", "## Update and remove", "## What is new", "## More"):
            self.assertIn(heading, text)
        self.assertIn("curl -fsS https://owner.github.io/project/install.sh | sh\n", text)
        self.assertIn("--update", text)
        self.assertIn("--uninstall", text)
        self.assertIn("#### Added", text)
        self.assertNotIn("//install.sh", text)  # a trailing slash on the site must not double up
        self.assertNotIn("Passman", text)

    def test_without_a_changelog_section_it_links_to_the_file(self):
        text = rn.build("0.3.0", "o/p", "https://x.example", SUMS, CHANGELOG)
        self.assertIn("blob/v0.3.0/CHANGELOG.md", text)

    def test_command_line(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "SHA256SUMS").write_text(SUMS)
            (Path(d) / "CHANGELOG.md").write_text(CHANGELOG)
            r = subprocess.run([sys.executable, "-I", str(SCRIPT), "--version", "v0.1.0", "--repo", "o/p", "--site", "https://s.example",
                                "--sums", str(Path(d) / "SHA256SUMS"), "--changelog", str(Path(d) / "CHANGELOG.md")],
                               capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("**Hexis 0.1.0**", r.stdout)
            bad = subprocess.run([sys.executable, "-I", str(SCRIPT), "--version", "latest", "--repo", "o/p", "--site", "https://s.example"],
                                 capture_output=True, text=True)
            self.assertEqual(bad.returncode, 2)


if __name__ == "__main__":
    unittest.main()
