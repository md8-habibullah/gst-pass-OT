"""Tests of scripts/check_no_notifications.py: the passkey provider is the one <service> it lets through.

Standard library only. Each case writes a small AndroidManifest.xml into a temporary project and runs the real check on it.
"""
import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import check_no_notifications as chk  # noqa: E402

PROVIDER = """
        <service
            android:name="dev.habibullah.hexis.provider.HexisCredentialProviderService"
            android:enabled="false"
            android:exported="true"
            android:permission="android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE">
            <intent-filter>
                <action android:name="android.service.credentials.CredentialProviderService" />
            </intent-filter>
        </service>
"""


def manifest(body: str, perms: str = "") -> str:
    return (
        '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
        f"    {perms}\n    <application>\n{body}    </application>\n</manifest>\n"
    )


def problems_for(xml: str) -> list[str]:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        target = root / "src-tauri" / "gen" / "android" / "app" / "src" / "main"
        target.mkdir(parents=True)
        (target / "AndroidManifest.xml").write_text(xml, encoding="utf-8")
        found: list[str] = []
        with contextlib.redirect_stdout(io.StringIO()):
            chk.check_android(root, found)
        return found


class ProviderServiceTests(unittest.TestCase):
    def test_the_plain_provider_service_is_allowed(self):
        self.assertEqual(problems_for(manifest(PROVIDER)), [])

    def test_any_other_service_still_fails(self):
        other = PROVIDER.replace("HexisCredentialProviderService", "SyncService")
        found = problems_for(manifest(other))
        self.assertEqual(len(found), 1)
        self.assertIn("not the Hexis credential provider service", found[0])

    def test_a_look_alike_class_name_fails(self):
        for name in ("com.evil.HexisCredentialProviderService", "dev.habibullah.hexis.provider.HexisCredentialProviderService2"):
            lookalike = PROVIDER.replace("dev.habibullah.hexis.provider.HexisCredentialProviderService", name)
            found = problems_for(manifest(lookalike))
            self.assertTrue(any("not the Hexis credential provider service" in p for p in found), (name, found))

    def test_a_second_copy_of_the_provider_fails(self):
        found = problems_for(manifest(PROVIDER + PROVIDER))
        self.assertTrue(any("2 times" in p for p in found), found)

    def test_a_provider_that_anyone_could_bind_fails(self):
        open_service = PROVIDER.replace(
            '\n            android:permission="android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE"', ""
        )
        found = problems_for(manifest(open_service))
        self.assertTrue(any("not guarded by android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE" in p for p in found), found)

    def test_a_foreground_type_fails(self):
        typed = PROVIDER.replace('android:enabled="false"', 'android:enabled="false" android:foregroundServiceType="dataSync"')
        found = problems_for(manifest(typed))
        self.assertTrue(any("foregroundServiceType" in p for p in found), found)

    def test_a_second_entry_point_fails(self):
        boot = PROVIDER.replace(
            "</intent-filter>",
            '<action android:name="android.intent.action.BOOT_COMPLETED" /></intent-filter>',
        )
        found = problems_for(manifest(boot))
        self.assertTrue(any("intent filter must answer to" in p for p in found), found)

    def test_a_provider_without_its_action_fails(self):
        bare = PROVIDER.replace("android.service.credentials.CredentialProviderService", "com.example.OTHER")
        self.assertTrue(problems_for(manifest(bare)))

    def test_permissions_are_still_refused_next_to_the_provider(self):
        found = problems_for(
            manifest(PROVIDER, '<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />')
        )
        self.assertTrue(any("POST_NOTIFICATIONS" in p for p in found), found)
        found = problems_for(
            manifest(PROVIDER, '<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />')
        )
        self.assertTrue(any("FOREGROUND_SERVICE" in p for p in found), found)


if __name__ == "__main__":
    unittest.main()
