"""Tests of scripts/version.py, the one place that knows the version. Standard library only.

The files are copies of the REAL ones (package.json, Cargo.lock, ...), because the failure that matters is the script
not recognising the layout of a real file, not a made-up one.
"""
import contextlib
import difflib
import io
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE.parent))
import version as v  # noqa: E402

FILES = ["VERSION", "package.json", "package-lock.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml",
         "src-tauri/Cargo.lock", "packaging/arch/PKGBUILD", "CHANGELOG.md"]


def run(root, *args):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        rc = v.main(["--root", str(root), *args])
    return rc, out.getvalue(), err.getvalue()


def changed_lines(before: str, after: str) -> int:
    return sum(1 for d in difflib.ndiff(before.splitlines(), after.splitlines()) if d.startswith("- "))


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        for f in FILES:
            (self.root / f).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / f, self.root / f)
        self.current = (self.root / "VERSION").read_text().strip()
        self.write_changelog("### Added\n\n* Something new.\n")

    def tearDown(self):
        self.tmp.cleanup()

    def write_changelog(self, unreleased_body):
        text = (self.root / "CHANGELOG.md").read_text(encoding="utf-8")
        start, end, _ = v.unreleased(text)
        head = text[start:text.index("\n", start) + 1]
        (self.root / "CHANGELOG.md").write_text(text[:start] + head + "\n" + unreleased_body + "\n" + text[end:], encoding="utf-8")

    def text(self, f):
        return (self.root / f).read_text(encoding="utf-8")


class RealRepository(unittest.TestCase):
    def test_every_file_agrees_with_the_version_file(self):
        rc, out, err = run(ROOT, "check")
        self.assertEqual(rc, 0, err)

    def test_every_target_is_found_in_the_real_files(self):
        self.assertEqual(len(v.found(ROOT)), len(v.TARGETS))

    def test_ci_and_the_local_checks_run_the_check(self):
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        self.assertIn("python3 scripts/version.py check --release", ci, "a release must not go out with files that disagree")
        self.assertIn("scripts/version.py", (ROOT / "scripts" / "verify_all.sh").read_text(encoding="utf-8"))


class Setting(Base):
    def test_set_changes_the_version_line_in_every_file_and_nothing_else(self):
        before = {f: self.text(f) for f in FILES}
        rc, out, err = run(self.root, "set", "7.8.9", "--date", "2031-02-03")
        self.assertEqual(rc, 0, err)
        expected = {"VERSION": 1, "package.json": 1, "package-lock.json": 2, "src-tauri/tauri.conf.json": 1,
                    "src-tauri/Cargo.toml": 1, "src-tauri/Cargo.lock": 1, "packaging/arch/PKGBUILD": 1}
        for f, n in expected.items():
            self.assertEqual(changed_lines(before[f], self.text(f)), n, f)
        self.assertEqual(run(self.root, "check")[0], 0)
        self.assertEqual(run(self.root, "show")[1].strip(), "7.8.9")
        self.assertIn('version = "7.8.9"', self.text("src-tauri/Cargo.toml"))
        self.assertIn("pkgver=7.8.9\n", self.text("packaging/arch/PKGBUILD"))

    def test_other_packages_with_the_same_old_version_are_left_alone(self):
        # Cargo.lock lists many crates; only the `hexis` entry may change
        lock = self.text("src-tauri/Cargo.lock")
        run(self.root, "set", "7.8.9")
        after = self.text("src-tauri/Cargo.lock")
        self.assertEqual(lock.count("[[package]]"), after.count("[[package]]"))
        self.assertEqual(changed_lines(lock, after), 1)

    def test_the_changelog_heading_is_released_and_a_new_empty_one_opened(self):
        run(self.root, "set", "7.8.9", "--date", "2031-02-03")
        log = self.text("CHANGELOG.md")
        i = log.index("## [Unreleased]")
        j = log.index("## [7.8.9] - 2031-02-03")
        self.assertLess(i, j)
        self.assertEqual(log[i:j].strip(), "## [Unreleased]", "the new Unreleased section is empty")
        self.assertIn("* Something new.", log[j:log.index("## [", j + 5)])

    def test_running_it_twice_changes_nothing(self):
        run(self.root, "set", "7.8.9", "--date", "2031-02-03")
        once = {f: self.text(f) for f in FILES}
        rc, out, _ = run(self.root, "set", "7.8.9", "--date", "2031-02-04")
        self.assertEqual(rc, 0)
        self.assertEqual({f: self.text(f) for f in FILES}, once)

    def test_a_lower_version_is_refused_unless_forced(self):
        rc, _, err = run(self.root, "set", "0.0.1")
        self.assertEqual(rc, 1)
        self.assertIn("lower", err)
        self.assertEqual(self.text("VERSION").strip(), self.current, "a refused change touches nothing")
        self.assertEqual(run(self.root, "set", "0.0.1", "--force")[0], 0)

    def test_a_pre_release_is_older_than_its_release_and_pacman_gets_an_underscore(self):
        self.assertEqual(run(self.root, "set", "9.0.0-rc1")[0], 0)
        self.assertIn("pkgver=9.0.0_rc1\n", self.text("packaging/arch/PKGBUILD"))
        self.assertEqual(run(self.root, "check")[0], 0, "the underscore form is what check expects")
        self.assertEqual(run(self.root, "set", "9.0.0")[0], 0, "the release after its own release candidate is not a downgrade")

    def test_not_a_version_is_refused(self):
        for bad in ("1.2", "1.2.3.4", "x", "01.2.3", ""):
            with self.subTest(bad=bad):
                self.assertEqual(run(self.root, "set", bad)[0], 1)


class Bumping(Base):
    def numbers(self):
        a, b, c, _ = v.parse(self.current)
        return a, b, c

    def test_explicit_bumps(self):
        a, b, c = self.numbers()
        for kind, want in (("patch", f"{a}.{b}.{c + 1}"), ("minor", f"{a}.{b + 1}.0"), ("major", f"{a + 1}.0.0")):
            with self.subTest(kind=kind):
                self.tearDown(); self.setUp()
                self.assertEqual(run(self.root, "bump", kind)[0], 0)
                self.assertEqual(self.text("VERSION").strip(), want)
                self.assertEqual(run(self.root, "check", "--release")[0], 0)

    def test_auto_reads_the_changelog(self):
        cases = [
            ("1.4.2", "### Fixed\n\n* a bug\n", "patch"),
            ("1.4.2", "### Security\n\n* a hole\n", "patch"),
            ("1.4.2", "### Added\n\n* a thing\n", "minor"),
            ("1.4.2", "### Changed\n\n* a thing\n", "minor"),
            ("1.4.2", "### Added\n\n* a thing\n\n### Fixed\n\n* a bug\n", "minor"),
            ("1.4.2", "### Removed\n\n* a thing\n", "major"),
            ("1.4.2", "### Changed\n\n* **BREAKING** a stored format\n", "major"),
            ("0.3.0", "### Removed\n\n* a thing\n", "minor"),
            ("0.3.0", "### Changed\n\n* BREAKING: a stored format\n", "minor"),
            ("0.3.0", "### Fixed\n\n* a bug\n", "patch"),
        ]
        for current, body, want in cases:
            with self.subTest(current=current, want=want, body=body[:20]):
                self.assertEqual(v.decide(current, body)[0], want)

    def test_auto_with_nothing_to_release_refuses_and_touches_nothing(self):
        self.write_changelog("")
        before = {f: self.text(f) for f in FILES}
        rc, _, err = run(self.root, "bump", "auto")
        self.assertEqual(rc, 1)
        self.assertIn("nothing", err)
        self.assertEqual({f: self.text(f) for f in FILES}, before)

    def test_bump_refuses_when_the_files_have_drifted(self):
        p = self.root / "src-tauri" / "tauri.conf.json"
        p.write_text(p.read_text().replace(f'"version": "{self.current}"', '"version": "0.0.9"', 1))
        rc, _, err = run(self.root, "bump", "patch")
        self.assertEqual(rc, 1)
        self.assertIn("sync", err)

    def test_suggest_prints_the_next_version_and_why(self):
        rc, out, _ = run(self.root, "suggest")
        self.assertEqual(rc, 0)
        self.assertIn("minor", out)
        self.assertIn(self.current, out)


class Checking(Base):
    def test_a_file_that_disagrees_is_named(self):
        p = self.root / "package.json"
        p.write_text(p.read_text().replace(f'"version": "{self.current}"', '"version": "9.9.9"', 1))
        rc, _, err = run(self.root, "check")
        self.assertEqual(rc, 1)
        self.assertIn("package.json", err)
        self.assertIn("sync", err)
        self.assertEqual(run(self.root, "sync")[0], 0)
        self.assertEqual(run(self.root, "check")[0], 0)

    def test_release_check_needs_the_changelog_section(self):
        rc, _, err = run(self.root, "check", "--release")
        # the current version has its section in the real changelog, so remove it to see the failure
        self.assertEqual(rc, 0, err)
        p = self.root / "CHANGELOG.md"
        p.write_text(p.read_text().replace(f"## [{self.current}]", "## [removed]", 1))
        rc, _, err = run(self.root, "check", "--release")
        self.assertEqual(rc, 1)
        self.assertIn("CHANGELOG.md", err)

    def test_a_changed_layout_is_an_error_not_a_silent_skip(self):
        p = self.root / "src-tauri" / "Cargo.toml"
        p.write_text(p.read_text().replace('name = "hexis"', 'name = "other"', 1))
        rc, _, err = run(self.root, "check")
        self.assertEqual(rc, 1)
        self.assertIn("Cargo.toml", err)


if __name__ == "__main__":
    unittest.main()
