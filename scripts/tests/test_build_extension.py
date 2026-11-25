"""Tests of scripts/build_extension.py: the packed files are the ones the app writes, and the zips repeat byte for byte.

Run from the repository root:  python3 -m unittest discover -s scripts/tests
"""
import importlib.util
import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "build_extension.py"
_spec = importlib.util.spec_from_file_location("build_extension", SCRIPT)
be = importlib.util.module_from_spec(_spec)
sys.modules["build_extension"] = be
_spec.loader.exec_module(be)


class BuildExtension(unittest.TestCase):
    def test_sources_agree(self):
        self.assertEqual(be.problems(), [])

    def test_the_file_list_is_the_one_the_app_writes(self):
        self.assertEqual([n for n, _ in be.shared_files()], be.rust_names())

    def test_zips_hold_the_shared_files_and_the_family_manifest_and_repeat(self):
        with tempfile.TemporaryDirectory() as tmp:
            be.DIST = Path(tmp)
            for family in ("chromium", "firefox"):
                first, digest = be.pack(family)
                second, again = be.pack(family)
                self.assertEqual(digest, again, "the same sources give the same bytes")
                with zipfile.ZipFile(first) as z:
                    names = sorted(z.namelist())
                    self.assertEqual(names, sorted(["manifest.json"] + be.rust_names()))
                    manifest = json.loads(z.read("manifest.json"))
                self.assertEqual(manifest, json.loads((be.EXT / f"manifest.{family}.json").read_text()))

    def test_a_problem_is_reported(self):
        original = be.EXT
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp)
            (fake / "src").mkdir(); (fake / "icons").mkdir()
            for name in ("page.js", "content.js", "background.js"):
                (fake / "src" / name).write_text("//")
            for fam, extra in (("chromium", {"key": "k"}), ("firefox", {"browser_specific_settings": {"gecko": {"id": "x@y"}}})):
                (fake / f"manifest.{fam}.json").write_text(json.dumps({"manifest_version": 3, "version": "1" if fam == "chromium" else "2", "permissions": ["nativeMessaging", "tabs"], **extra,
                                                                       "background": {"scripts": ["background.js"]}, "content_scripts": [{"js": ["missing.js"]}]}))
            be.EXT = fake
            try:
                found = be.problems()
            finally:
                be.EXT = original
        text = "\n".join(found)
        self.assertIn("different versions", text)
        self.assertIn("expected only nativeMessaging", text)
        self.assertIn("missing.js", text)


if __name__ == "__main__":
    unittest.main()
