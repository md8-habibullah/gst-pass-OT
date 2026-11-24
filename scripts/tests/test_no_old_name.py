"""The product is Hexis. The working title it was built under must not come back by accident.

This scans every tracked and new text file for the old name. The few places where it must stay are listed here, each with
the reason, so a new occurrence anywhere else fails the build. Run from the repository root:
python3 -m unittest discover -s scripts/tests -t .  (standard library only; needs git).
"""
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
OLD = re.compile("pass" + "man", re.I)  # spelled in two halves so this file does not match itself
# The repository's address contains the working title on purpose (chosen for search, ARCHITECTURE.md D9). It is a URL, not the
# product name, so exactly this slug is not counted; any other use of the title still fails.
REPO_SLUG = "post-quantum-" + "pass" + "man-gst"
# The address before the repository was renamed. GitHub Pages does not redirect it, so a link to it is dead.
DEAD_SLUG = re.compile("gst-pass-" + "OT", re.I)

# Stored-format identifiers: renaming them would make existing vaults unreadable (ai-docs/CRYPTO_SPEC.md, "Frozen identifiers").
FROZEN_FORMAT = {
    "src-tauri/src/crypto/kem.rs", "src-tauri/src/crypto/subkey.rs", "src-tauri/src/gate/counter.rs",
    "scripts/gen_crypto_kat.py", "scripts/verify_bundle_fixture.py", "ai-docs/CRYPTO_SPEC.md",
}
FROZEN_LINE = re.compile(OLD.pattern + r"/(v1|gate/v1)", re.I)
# Compatibility with data written before the rename (data folder, export extension, export field names).
COMPATIBILITY = {"src-tauri/src/storage/legacy.rs", "src-tauri/src/importer/export.rs"}
# History and the explanation of the rename itself.
HISTORY = {"CHANGELOG.md", "ai-docs/ARCHITECTURE.md", "ai-docs/CHANGELOG_AND_STATE.md"}
# Tests that assert the old name is absent.
GUARDS = {"scripts/tests/test_release_notes.py", "scripts/tests/test_no_old_name.py"}


def files():
    out = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT, capture_output=True, check=True).stdout
    return [f for f in out.decode().split("\0") if f]


class NoOldName(unittest.TestCase):
    def test_no_file_or_folder_is_named_after_it(self):
        self.assertEqual([f for f in files() if OLD.search(f)], [])

    def test_the_old_name_appears_only_where_it_must(self):
        allowed = FROZEN_FORMAT | COMPATIBILITY | HISTORY | GUARDS
        stray, frozen_bad = [], []
        for f in files():
            p = ROOT / f
            if not p.is_file() or p.is_symlink():
                continue
            data = p.read_bytes()
            if b"\0" in data:
                continue
            for n, line in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
                if not OLD.search(line.replace(REPO_SLUG, "")):
                    continue
                if f not in allowed:
                    stray.append(f"{f}:{n}: {line.strip()[:100]}")
                elif f in FROZEN_FORMAT and not FROZEN_LINE.search(line):
                    # only the key-derivation labels belong in these files; prose may say it in the spec
                    if f != "ai-docs/CRYPTO_SPEC.md":
                        frozen_bad.append(f"{f}:{n}: {line.strip()[:100]}")
        self.assertEqual(stray, [], "the old product name is back; use Hexis (or add a reason to this test)")
        self.assertEqual(frozen_bad, [], "only the frozen format labels may keep the old spelling in these files")

    def test_no_link_points_at_the_dead_pre_rename_address(self):
        allowed = {"CHANGELOG.md", "ai-docs/CHANGELOG_AND_STATE.md", "scripts/tests/test_no_old_name.py"}
        stray = []
        for f in files():
            p = ROOT / f
            if f in allowed or not p.is_file() or p.is_symlink():
                continue
            data = p.read_bytes()
            if b"\0" in data:
                continue
            for n, line in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
                if DEAD_SLUG.search(line):
                    stray.append(f"{f}:{n}: {line.strip()[:100]}")
        self.assertEqual(stray, [], "the repository was renamed to " + REPO_SLUG + "; use the new address")

    def test_the_product_name_is_set_where_the_build_reads_it(self):
        import json
        conf = json.loads((ROOT / "src-tauri" / "tauri.conf.json").read_text())
        self.assertEqual(conf["productName"], "Hexis")
        self.assertEqual(conf["identifier"], "dev.habibullah.hexis")
        self.assertEqual(conf["app"]["windows"][0]["title"], "Hexis")
        self.assertEqual(json.loads((ROOT / "package.json").read_text())["name"], "hexis")
        cargo = (ROOT / "src-tauri" / "Cargo.toml").read_text()
        self.assertRegex(cargo, r'(?m)^name = "hexis"$')
        self.assertIn("<title>Hexis</title>", (ROOT / "index.html").read_text())


if __name__ == "__main__":
    unittest.main()
