"""Guards the list of apps that may name a web page's origin to the Android passkey provider (assets/passkey/privileged_apps.json).

Scanning a computer's QR code with the phone (the hybrid transport) is run by Google Play services, and a request that
arrives that way is handled as a browser's: the provider takes the origin and the hash of the client data from it, which
only works while `com.google.android.gms` is on this list with its signing certificates. Trimming or breaking the list when
it is refreshed by hand would switch that off without any test failing, so these tests fail instead.

Run from the repository root:  python3 -m unittest discover -s scripts/tests -t .
"""
import json
import re
import unittest
from pathlib import Path

LIST = Path(__file__).resolve().parent.parent.parent / "assets" / "passkey" / "privileged_apps.json"
FINGERPRINT = re.compile(r"^([0-9A-F]{2}:){31}[0-9A-F]{2}$")


class PrivilegedApps(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.apps = json.loads(LIST.read_text(encoding="utf-8"))["apps"]
        cls.by_package = {a["info"]["package_name"]: a["info"] for a in cls.apps if a.get("type") == "android"}

    def test_google_play_services_is_on_the_list_with_release_certificates(self):
        gms = self.by_package.get("com.google.android.gms")
        self.assertIsNotNone(gms, "without Play services the phone cannot be the authenticator for a computer's QR code")
        self.assertTrue([s for s in gms["signatures"] if s.get("build") == "release"], "a release certificate is needed")

    def test_the_browsers_people_use_are_on_it(self):
        for package in ("com.android.chrome", "org.mozilla.firefox", "com.brave.browser", "com.microsoft.emmx"):
            self.assertIn(package, self.by_package, package)

    def test_every_entry_names_a_package_and_well_formed_certificate_fingerprints(self):
        self.assertGreaterEqual(len(self.apps), 50, "the list looks truncated")
        for app in self.apps:
            self.assertEqual(app.get("type"), "android", app)
            info = app["info"]
            self.assertTrue(re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+", info["package_name"]), info["package_name"])
            self.assertTrue(info["signatures"], info["package_name"])
            for signature in info["signatures"]:
                self.assertRegex(signature["cert_fingerprint_sha256"], FINGERPRINT, info["package_name"])

    def test_no_package_is_listed_twice(self):
        packages = [a["info"]["package_name"] for a in self.apps]
        self.assertEqual(len(packages), len(set(packages)))


if __name__ == "__main__":
    unittest.main()
