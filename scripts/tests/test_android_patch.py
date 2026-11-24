"""Tests of scripts/android_patch_manifest.py against synthetic Android projects.

Run from the repository root:  python3 -m unittest discover -s scripts/tests
(or `python3 -m unittest`). Standard library only. `rsvg-convert` is replaced by a stub that writes
a valid PNG of the requested size, so the tests are fast and do not need librsvg; one test uses the
real `rsvg-convert` when it is installed. Nothing here runs Gradle or touches a device: what the
generated project does on Android is UNVERIFIED.
"""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPTS = HERE.parent
REPO = SCRIPTS.parent
SCRIPT = SCRIPTS / "android_patch_manifest.py"

_spec = importlib.util.spec_from_file_location("android_patch_manifest", SCRIPT)
apm = importlib.util.module_from_spec(_spec)
sys.modules["android_patch_manifest"] = apm
_spec.loader.exec_module(apm)

A = "{http://schemas.android.com/apk/res/android}"
T = "{http://schemas.android.com/tools}"

# Modelled on the manifest `tauri android init` generates (Tauri 2.x template): comments, a blank
# line before the provider, a leanback category in the launcher filter, the `tools` namespace.
MANIFEST = """<?xml version="1.0" encoding="utf-8" ?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools">
    <uses-permission android:name="android.permission.INTERNET" />

    <!-- AndroidTV support -->
    <uses-feature android:name="android.software.leanback" android:required="false" />

    <application
        android:icon="@mipmap/ic_launcher"
        android:label="@string/app_name"
        android:theme="@style/Theme.hexis"
        android:usesCleartextTraffic="false">
        <activity
            android:configChanges="orientation|keyboardHidden|keyboard|screenSize|locale|smallestScreenSize|screenLayout|uiMode"
            android:launchMode="singleTask"
            android:label="@string/main_activity_title"
            android:name=".MainActivity"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
                <!-- AndroidTV support -->
                <category android:name="android.intent.category.LEANBACK_LAUNCHER" />
            </intent-filter>
        </activity>

        <provider
          android:name="androidx.core.content.FileProvider"
          android:authorities="${applicationId}.fileprovider"
          android:exported="false"
          android:grantUriPermissions="true">
          <meta-data
            android:name="android.support.FILE_PROVIDER_PATHS"
            android:resource="@xml/file_paths" />
        </provider>
    </application>
</manifest>
"""

PRESETS = {
    "presets": [
        {"id": "kerf", "name": "Kerf", "description": "Cursor and dot", "background": "#0F1013"},
        {"id": "tally", "name": "Tally", "description": "Minimal grid", "background": "#1F2A33"},
        {"id": "hostile", "name": "Tom's \"Tab\" & <Co> @home", "description": "x", "background": "#abcdef"},
        {"id": "at", "name": "@work", "description": "x", "background": "#000000"},
    ]
}

STUB_RSVG = """#!{python}
import struct, sys, zlib
a = sys.argv[1:]
w = int(a[a.index("-w") + 1]); h = int(a[a.index("-h") + 1])
if "FAIL" in open(a[-1], encoding="utf-8", errors="replace").read():
    sys.stderr.write("stub failure"); sys.exit(1)
row = b"\\x00" + b"\\x20\\x40\\x60\\xff" * w
raw = row * h
def chunk(t, d):
    c = struct.pack(">I", len(d)) + t + d
    return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
png = b"\\x89PNG\\r\\n\\x1a\\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
sys.stdout.buffer.write(png)
"""


def write(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def make_project(root: Path, manifest: str = MANIFEST, namespace: str = "dev.habibullah.hexis") -> Path:
    project = root / "android"
    main = project / "app" / "src" / "main"
    write(main / "AndroidManifest.xml", manifest)
    write(main / "res" / "values" / "strings.xml",
          '<resources>\n    <string name="app_name">Hexis</string>\n    <string name="main_activity_title">Hexis</string>\n</resources>\n')
    write(project / "app" / "build.gradle.kts", f'plugins {{ id("com.android.application") }}\nandroid {{\n    namespace = "{namespace}"\n    compileSdk = 36\n}}\n')
    write(main / "java" / Path(*namespace.split(".")) / "MainActivity.kt",
          f"package {namespace}\n\nclass MainActivity : TauriActivity()\n")
    return project


def make_stub(root: Path) -> Path:
    stub = root / "rsvg-convert-stub"
    stub.write_text(STUB_RSVG.format(python=sys.executable), encoding="utf-8")
    stub.chmod(stub.stat().st_mode | stat.S_IXUSR)
    return stub


def make_assets(root: Path, presets: dict) -> tuple[Path, Path]:
    assets = root / "assets"
    for p in presets["presets"]:
        write(assets / f"{p['id']}.svg", "<svg/>")
        write(assets / f"{p['id']}-fg.svg", "<svg/>")
    pf = write(root / "presets.json", json.dumps(presets))
    return pf, assets


def tree_state(root: Path) -> dict:
    out = {}
    for p in sorted(root.rglob("*")):
        if p.is_file():
            out[str(p.relative_to(root))] = (hashlib.sha256(p.read_bytes()).hexdigest(), p.stat().st_mtime_ns)
    return out


def parse(path: Path) -> ET.Element:
    return ET.parse(path, parser=ET.XMLParser(target=ET.TreeBuilder(insert_comments=True))).getroot()


class Base(unittest.TestCase):
    presets = PRESETS

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.project = make_project(self.root)
        self.stub = make_stub(self.root)
        self.presets_file, self.assets = make_assets(self.root, self.presets)
        self.manifest = self.project / "app" / "src" / "main" / "AndroidManifest.xml"
        self.res = self.project / "app" / "src" / "main" / "res"

    def run_script(self, *extra, project=None, with_stub=True):
        argv = [str(project or self.project), "--presets", str(self.presets_file), "--assets", str(self.assets)]
        if with_stub:
            argv += ["--rsvg-convert", str(self.stub)]
        argv += list(extra)
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = apm.main(argv, repo_root=REPO)
        return code, out.getvalue(), err.getvalue()

    def apply(self):
        code, out, err = self.run_script()
        self.assertEqual(code, 0, out + err)
        return out

    def aliases(self):
        return parse(self.manifest).find("application").findall("activity-alias")


class ManifestTests(Base):
    def test_the_main_activity_loses_its_launcher_filter_and_aliases_are_added(self):
        self.apply()
        root = parse(self.manifest)
        app = root.find("application")
        main = app.find("activity")
        self.assertEqual(main.get(A + "name"), ".MainActivity")
        self.assertEqual(main.findall("intent-filter"), [], "no launcher filter is left on MainActivity: one icon only")

        aliases = app.findall("activity-alias")
        names = [a.get(A + "name") for a in aliases]
        self.assertEqual(names, [".AliasOriginal", ".AliasKerf", ".AliasTally", ".AliasHostile", ".AliasAt"])
        for a in aliases:
            self.assertEqual(a.get(A + "targetActivity"), ".MainActivity")
            self.assertEqual(a.get(A + "exported"), "true")
            pid = a.get(A + "name")[len(".Alias"):].lower()
            if pid == "original":
                self.assertEqual(a.get(A + "enabled"), "true")
                self.assertEqual(a.get(A + "icon"), "@mipmap/ic_launcher")
                self.assertEqual(a.get(A + "label"), "@string/app_label_original")
            else:
                self.assertEqual(a.get(A + "enabled"), "false", pid)
                self.assertEqual(a.get(A + "icon"), f"@mipmap/ic_launcher_{pid}")
                self.assertEqual(a.get(A + "label"), f"@string/app_label_{pid}")
            filt = a.findall("intent-filter")
            self.assertEqual(len(filt), 1)
            self.assertEqual([x.get(A + "name") for x in filt[0].findall("action")], ["android.intent.action.MAIN"])
            cats = [x.get(A + "name") for x in filt[0].findall("category")]
            self.assertIn("android.intent.category.LAUNCHER", cats)
            # the TV category of the original filter travels with the launcher filter
            self.assertIn("android.intent.category.LEANBACK_LAUNCHER", cats)
        self.assertEqual(sum(a.get(A + "enabled") == "true" for a in aliases), 1, "exactly one enabled alias")

    def test_everything_else_in_the_manifest_is_kept(self):
        before = parse(self.manifest)
        self.apply()
        text = self.manifest.read_text(encoding="utf-8")
        after = parse(self.manifest)
        self.assertTrue(text.startswith('<?xml version="1.0" encoding="utf-8" ?>\n<manifest'), text[:80])
        self.assertTrue(text.endswith("</manifest>\n"))
        self.assertIn('xmlns:android="http://schemas.android.com/apk/res/android"', text)
        self.assertIn('xmlns:tools="http://schemas.android.com/tools"', text, "a declared namespace is kept even when unused")
        self.assertIn("<!-- AndroidTV support -->", text, "comments survive")
        self.assertIn('android:authorities="${applicationId}.fileprovider"', text)
        before_app, after_app = before.find("application"), after.find("application")
        backup = {A + k for k in ("allowBackup", "fullBackupContent", "dataExtractionRules")}
        self.assertEqual(before_app.attrib, {k: v for k, v in after_app.attrib.items() if k not in backup})
        self.assertEqual(
            [(e.tag, e.attrib) for e in before.iter() if isinstance(e.tag, str) and e.tag not in ("intent-filter", "action", "category")],
            [(e.tag, {k: v for k, v in e.attrib.items() if k not in backup}) for e in after.iter()
             if isinstance(e.tag, str) and e.tag not in ("intent-filter", "action", "category", "activity-alias")
             # the two removal markers the script adds on purpose (VIBRATE, optional camera): they are checked in their own test
             and not (e.tag in ("uses-permission", "uses-feature") and e.get(T + "node") in ("remove", "replace"))],
        )
        prov = after_app.find("provider")
        self.assertEqual(prov.find("meta-data").get(A + "resource"), "@xml/file_paths")
        main = after_app.find("activity")
        self.assertEqual(main.get(A + "launchMode"), "singleTask")
        self.assertEqual(main.get(A + "label"), "@string/main_activity_title")

    def test_nothing_is_ever_backed_up_or_transferred(self):
        """After an uninstall a reinstall must start empty: Android's cloud backup must not put the vault files back."""
        self.apply()
        app = parse(self.manifest).find("application")
        self.assertEqual(app.get(A + "allowBackup"), "false")
        self.assertEqual(app.get(A + "fullBackupContent"), "false")
        self.assertEqual(app.get(A + "dataExtractionRules"), "@xml/data_extraction_rules")
        rules = parse(self.res / "xml" / "data_extraction_rules.xml")
        self.assertEqual(rules.tag, "data-extraction-rules")
        for section, domains in (
            ("cloud-backup", {"root", "file", "database", "sharedpref", "external"}),
            ("device-transfer", {"root", "file", "database", "sharedpref", "external", "device_root", "device_file"}),
        ):
            node = rules.find(section)
            self.assertIsNotNone(node, section)
            self.assertEqual({e.get("domain") for e in node.findall("exclude")}, domains, section)
            self.assertEqual(node.findall("include"), [], "nothing is ever included")
            self.assertTrue(all(e.get("path") == "." for e in node.findall("exclude")))

    def test_a_manifest_that_allows_backup_is_corrected(self):
        text = self.manifest.read_text(encoding="utf-8").replace(
            '<application\n', '<application\n        android:allowBackup="true"\n        android:fullBackupContent="@xml/old_rules"\n', 1)
        self.manifest.write_text(text, encoding="utf-8")
        self.assertEqual(self.run_script("--check")[0], 1)
        self.apply()
        app = parse(self.manifest).find("application")
        self.assertEqual(app.get(A + "allowBackup"), "false")
        self.assertEqual(app.get(A + "fullBackupContent"), "false")
        self.assertEqual(self.run_script("--check")[0], 0)

    def test_no_permission_is_ever_granted(self):
        before = {e.get(A + "name") for e in parse(self.manifest).iter("uses-permission")}
        self.apply()
        granted = {e.get(A + "name") for e in parse(self.manifest).iter("uses-permission") if e.get(T + "node") != "remove"}
        self.assertEqual(before, granted)
        for p in self.project.rglob("*"):
            if p.is_file() and p.suffix in (".xml", ".kt") and p != self.manifest:
                self.assertNotIn("android.permission", p.read_text(encoding="utf-8"), p)
                self.assertNotIn("POST_NOTIFICATIONS", p.read_text(encoding="utf-8"), p)
                self.assertNotIn("FOREGROUND_SERVICE", p.read_text(encoding="utf-8"), p)
        self.assertNotIn("<service", self.manifest.read_text(encoding="utf-8"))

    def test_the_new_elements_are_indented_like_the_rest(self):
        self.apply()
        text = self.manifest.read_text(encoding="utf-8")
        self.assertIn('\n        <activity-alias android:name=".AliasOriginal" android:enabled="true"', text)
        self.assertIn("\n            <intent-filter>\n                <action android:name=\"android.intent.action.MAIN\" />", text)
        # the blank line that separated MainActivity from the provider now follows the last alias
        self.assertRegex(text, r"</activity-alias>\n\n        <provider")
        self.assertNotIn("\t", text)

    def test_other_intent_filters_on_the_main_activity_are_kept(self):
        extra = """            <intent-filter>
                <action android:name="android.intent.action.VIEW" />
                <category android:name="android.intent.category.DEFAULT" />
                <data android:scheme="hexis" />
            </intent-filter>
"""
        text = MANIFEST.replace("            </intent-filter>\n        </activity>", "            </intent-filter>\n" + extra + "        </activity>")
        self.assertNotEqual(text, MANIFEST)
        self.manifest.write_text(text, encoding="utf-8")
        self.apply()
        main = parse(self.manifest).find("application").find("activity")
        filters = main.findall("intent-filter")
        self.assertEqual(len(filters), 1)
        self.assertEqual(filters[0].find("data").get(A + "scheme"), "hexis")

    def test_the_vibrate_permission_and_the_required_camera_are_dealt_with(self):
        self.apply()
        root = parse(self.manifest)
        vib = [e for e in root.findall("uses-permission") if e.get(A + "name") == "android.permission.VIBRATE"]
        self.assertEqual([e.get(T + "node") for e in vib], ["remove"])
        cam = [e for e in root.findall("uses-feature") if e.get(A + "name") == "android.hardware.camera.any"]
        self.assertEqual([(e.get(A + "required"), e.get(T + "node")) for e in cam], [("false", "replace")])
        text = self.manifest.read_text(encoding="utf-8")
        self.assertIn("xmlns:tools=", text.split(">", 2)[1] if text.startswith("<?xml") else text)
        self.assertNotIn("ns0", text)
        # they sit before <application>, indented like the other children of <manifest>
        self.assertLess(text.index("android.permission.VIBRATE"), text.index("<application"))
        self.assertRegex(text, r'\n    <uses-permission android:name="android.permission.VIBRATE"')

    def test_an_existing_vibrate_permission_is_turned_into_a_removal_marker(self):
        text = MANIFEST.replace(
            '<uses-permission android:name="android.permission.INTERNET" />',
            '<uses-permission android:name="android.permission.INTERNET" />\n    <uses-permission android:name="android.permission.VIBRATE" />',
        )
        self.manifest.write_text(text, encoding="utf-8")
        self.apply()
        vib = [e for e in parse(self.manifest).findall("uses-permission") if e.get(A + "name") == "android.permission.VIBRATE"]
        self.assertEqual(len(vib), 1)
        self.assertEqual(vib[0].get(T + "node"), "remove")

    def test_tools_attributes_survive(self):
        text = MANIFEST.replace(
            '<uses-permission android:name="android.permission.INTERNET" />',
            '<uses-permission android:name="android.permission.INTERNET" />\n    <uses-permission android:name="android.permission.VIBRATE" tools:node="remove" />',
        )
        self.manifest.write_text(text, encoding="utf-8")
        self.apply()
        out = self.manifest.read_text(encoding="utf-8")
        self.assertIn('tools:node="remove"', out)
        self.assertNotIn("ns0", out)

    def test_a_manifest_without_comments_or_a_declaration_is_handled(self):
        text = re.sub(r"<!--.*?-->\s*", "", MANIFEST, flags=re.S).split("?>", 1)[1].lstrip()
        self.manifest.write_text(text.rstrip("\n"), encoding="utf-8")
        self.apply()
        out = self.manifest.read_text(encoding="utf-8")
        self.assertTrue(out.startswith("<?xml"), "a declaration is added when there was none")
        self.assertEqual(len(self.aliases()), 5)

    def test_stale_aliases_are_reported_and_left_alone(self):
        self.apply()
        text = self.manifest.read_text(encoding="utf-8").replace(
            '<activity-alias android:name=".AliasKerf"', '<activity-alias android:name=".AliasGone" android:enabled="false" android:targetActivity=".MainActivity"/>\n        <activity-alias android:name=".AliasKerf"', 1)
        self.manifest.write_text(text, encoding="utf-8")
        code, out, err = self.run_script()
        self.assertEqual(code, 0, out + err)
        self.assertIn("warning: the manifest has .AliasGone", out)
        self.assertIn(".AliasGone", self.manifest.read_text(encoding="utf-8"))

    def test_alias_names_match_the_rust_side(self):
        # the same table is asserted in src-tauri/src/commands/disguise/tests.rs
        self.assertEqual(apm.alias_name("kerf"), "AliasKerf")
        self.assertEqual(apm.alias_name("tally"), "AliasTally")
        self.assertEqual(apm.alias_name("orvyn"), "AliasOrvyn")


class ResourceTests(Base):
    def test_strings_are_escaped_for_xml_and_for_aapt(self):
        self.apply()
        text = (self.res / "values" / "disguise_strings.xml").read_text(encoding="utf-8")
        self.assertIn('<string name="app_label_original">Hexis</string>', text)
        self.assertIn('<string name="app_label_kerf">Kerf</string>', text)
        self.assertIn('<string name="app_label_hostile">Tom\\\'s \\"Tab\\" &amp; &lt;Co&gt; @home</string>', text)
        self.assertIn('<string name="app_label_at">\\@work</string>', text, "a leading @ would be a resource reference")
        ET.fromstring(text)  # well-formed

    def test_the_original_label_is_the_app_name(self):
        write(self.res / "values" / "strings.xml", '<resources><string name="app_name">My Vault &amp; Co</string></resources>')
        self.apply()
        text = (self.res / "values" / "disguise_strings.xml").read_text(encoding="utf-8")
        self.assertIn('<string name="app_label_original">My Vault &amp; Co</string>', text)

    def test_colors_and_adaptive_icons(self):
        self.apply()
        colors = ET.parse(self.res / "values" / "disguise_colors.xml").getroot()
        got = {c.get("name"): c.text for c in colors}
        self.assertEqual(got["ic_launcher_kerf_background"], "#0F1013")
        self.assertEqual(got["ic_launcher_hostile_background"], "#ABCDEF")
        xml = ET.parse(self.res / "mipmap-anydpi-v26" / "ic_launcher_kerf.xml").getroot()
        self.assertEqual(xml.tag, "adaptive-icon")
        self.assertEqual(xml.find("foreground").get(A + "drawable"), "@mipmap/ic_launcher_kerf_foreground")
        self.assertEqual(xml.find("background").get(A + "drawable"), "@color/ic_launcher_kerf_background")
        self.assertFalse((self.res / "mipmap-anydpi-v26" / "ic_launcher_original.xml").exists(), "the original reuses the project's own icon")

    def test_every_density_gets_its_icons_at_the_right_size(self):
        self.apply()
        for pid in ("kerf", "tally", "hostile", "at"):
            for density, (legacy, fg) in apm.DENSITIES.items():
                for name, size in ((f"ic_launcher_{pid}.png", legacy), (f"ic_launcher_{pid}_foreground.png", fg)):
                    f = self.res / f"mipmap-{density}" / name
                    self.assertTrue(f.is_file(), f)
                    self.assertEqual(apm.png_size(f.read_bytes()), (size, size), f)
        self.assertEqual({d: v[0] for d, v in apm.DENSITIES.items()},
                         {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192})
        self.assertFalse(list(self.res.glob("mipmap-*/ic_launcher_original*.png")))

    def test_a_wrong_sized_or_broken_icon_is_rendered_again(self):
        self.apply()
        f = self.res / "mipmap-hdpi" / "ic_launcher_kerf.png"
        f.write_bytes(b"not a png")
        code, out, _ = self.run_script("--check")
        self.assertEqual(code, 1)
        self.assertIn("ic_launcher_kerf.png", out)
        self.assertEqual(self.run_script()[0], 0)
        self.assertEqual(apm.png_size(f.read_bytes()), (72, 72))


class KotlinTests(Base):
    def kt(self):
        return (self.project / "app" / "src" / "main" / "java" / "dev" / "habibullah" / "hexis" / "DisguisePlugin.kt")

    def test_the_plugin_is_written_next_to_main_activity_with_the_real_api(self):
        self.apply()
        src = self.kt().read_text(encoding="utf-8")
        self.assertIn("package dev.habibullah.hexis\n", src)
        for needle in (
            "import app.tauri.annotation.Command", "import app.tauri.annotation.TauriPlugin",
            "import app.tauri.annotation.InvokeArg", "import app.tauri.plugin.Invoke", "import app.tauri.plugin.Plugin",
            "@TauriPlugin", "@Command", "fun setAlias(invoke: Invoke)", "class DisguisePlugin(private val activity: Activity) : Plugin(activity)",
            "setComponentEnabledSetting", "COMPONENT_ENABLED_STATE_ENABLED", "COMPONENT_ENABLED_STATE_DISABLED",
            "PackageManager.DONT_KILL_APP", 'ComponentName(activity.packageName, "dev.habibullah.hexis." + alias)',
            "invoke.parseArgs(SetAliasArgs::class.java)", "args.alias !in aliases",
        ):
            self.assertIn(needle, src)
        for alias in ("AliasOriginal", "AliasKerf", "AliasTally", "AliasHostile", "AliasAt"):
            self.assertIn(f'"{alias}"', src)
        self.assertNotIn("android.permission", src)
        self.assertNotIn("uses-permission", src)
        # balanced braces: a cheap guard against a broken template
        self.assertEqual(src.count("{"), src.count("}"))

    def test_the_package_comes_from_the_namespace_or_the_override(self):
        shutil.rmtree(self.project)
        self.project = make_project(self.root, namespace="com.example.notes")
        self.manifest = self.project / "app" / "src" / "main" / "AndroidManifest.xml"
        self.apply()
        src = (self.project / "app/src/main/java/com/example/notes/DisguisePlugin.kt").read_text(encoding="utf-8")
        self.assertIn("package com.example.notes\n", src)
        code, _, err = self.run_script("--package", "not a package")
        self.assertEqual(code, 2)
        self.assertIn("--package", err)

    def test_a_project_without_main_activity_source_uses_the_namespace_folder(self):
        for f in (self.project / "app/src/main/java").rglob("MainActivity.kt"):
            f.unlink()
        self.apply()
        self.assertTrue(self.kt().is_file())


class ActivityAndKeepRuleTests(Base):
    """The two fixes found on a real phone on 2026-11-24: the release build crashed on the first camera frame
    (R8 removed the constructors ML Kit needs), Back finished the app (a native abort) and the keyboard hid sheets."""

    def activity(self):
        return self.project / "app" / "src" / "main" / "java" / "dev" / "habibullah" / "hexis" / "MainActivity.kt"

    def test_main_activity_sends_back_to_the_background_and_resizes_for_the_keyboard(self):
        self.apply()
        src = self.activity().read_text(encoding="utf-8")
        self.assertIn("package dev.habibullah.hexis\n", src)
        self.assertNotIn("__PACKAGE__", src)
        for needle in (
            "class MainActivity : TauriActivity()", "enableEdgeToEdge()", "moveTaskToBack(true)",
            "window.__hexisBack", "WindowInsetsCompat.Type.ime()", "OnBackPressedCallback", "override fun onWebViewCreate(webView: WebView)",
            # the system bars' icon colour follows what the page draws (Settings > Theme), not only the phone's own setting
            '@JavascriptInterface', 'fun setLook(look: String)', 'addJavascriptInterface(BarsBridge(), "HexisBars")',
            "isAppearanceLightStatusBars = light", "isAppearanceLightNavigationBars = light",
        ):
            self.assertIn(needle, src)
        self.assertEqual(src.count("{"), src.count("}"))

    def test_the_activity_follows_the_namespace(self):
        shutil.rmtree(self.project)
        self.project = make_project(self.root, namespace="com.example.notes")
        self.manifest = self.project / "app" / "src" / "main" / "AndroidManifest.xml"
        self.apply()
        src = (self.project / "app/src/main/java/com/example/notes/MainActivity.kt").read_text(encoding="utf-8")
        self.assertIn("package com.example.notes\n", src)

    def test_the_keep_rules_protect_the_ml_kit_registrars_and_sit_where_gradle_looks(self):
        self.apply()
        rules = (self.project / "app" / apm.KEEP_RULES_NAME).read_text(encoding="utf-8")
        self.assertTrue(apm.KEEP_RULES_NAME.endswith(".pro"), "the Gradle template only passes *.pro files to R8")
        self.assertIn("-keep class * implements com.google.firebase.components.ComponentRegistrar { <init>(); }", rules)
        self.assertIn("-keep class com.google.mlkit.** { <init>(); }", rules)
        for line in rules.splitlines():
            if line.strip() and not line.startswith("#"):
                self.assertTrue(line.startswith("-keep"), line)

    def test_a_stale_activity_or_rules_file_is_brought_up_to_date(self):
        self.apply()
        self.activity().write_text("package dev.habibullah.hexis\n\nclass MainActivity : TauriActivity()\n", encoding="utf-8")
        code, out, _ = self.run_script("--check")
        self.assertEqual(code, 1)
        self.assertIn("MainActivity", out)
        self.apply()
        self.assertIn("moveTaskToBack", self.activity().read_text(encoding="utf-8"))


class IdempotencyTests(Base):
    def test_the_second_run_changes_nothing_byte_for_byte(self):
        self.apply()
        before = tree_state(self.project)
        code, out, _ = self.run_script()
        self.assertEqual(code, 0)
        self.assertIn("up to date", out)
        self.assertEqual(tree_state(self.project), before, "same bytes and same modification times")
        self.assertEqual(self.run_script("--check")[0], 0)

    def test_dry_run_and_check_write_nothing(self):
        before = tree_state(self.project)
        code, out, _ = self.run_script("--dry-run")
        self.assertEqual(code, 0)
        self.assertIn("plan (nothing is written)", out)
        self.assertIn("activity-alias", out)
        self.assertIn("render", out)
        self.assertEqual(tree_state(self.project), before)
        code, out, _ = self.run_script("--check")
        self.assertEqual(code, 1)
        self.assertIn("not up to date", out)
        self.assertEqual(tree_state(self.project), before)
        self.assertFalse((self.manifest.with_name("AndroidManifest.xml.tmp")).exists())

    def test_check_and_dry_run_do_not_need_rsvg(self):
        code, _, _ = self.run_script("--check", with_stub=False)
        self.assertEqual(code, 1)
        code, out, _ = self.run_script("--dry-run", with_stub=False)
        self.assertEqual(code, 0)

    def test_a_damaged_manifest_is_repaired_and_a_second_run_is_quiet(self):
        self.apply()
        text = self.manifest.read_text(encoding="utf-8")
        # delete one alias, flip another one's enabled flag and icon
        text = re.sub(r'\s*<activity-alias android:name="\.AliasTally".*?</activity-alias>', "", text, flags=re.S)
        text = text.replace('android:enabled="false" android:exported="true" android:icon="@mipmap/ic_launcher_kerf"',
                            'android:enabled="true" android:exported="true" android:icon="@mipmap/wrong"')
        self.manifest.write_text(text, encoding="utf-8")
        self.assertEqual(self.run_script("--check")[0], 1)
        code, out, _ = self.run_script()
        self.assertEqual(code, 0)
        self.assertIn(".AliasTally", out)
        aliases = {a.get(A + "name"): a for a in self.aliases()}
        self.assertEqual(aliases[".AliasKerf"].get(A + "enabled"), "false")
        self.assertEqual(aliases[".AliasKerf"].get(A + "icon"), "@mipmap/ic_launcher_kerf")
        self.assertEqual(len(aliases), 5)
        self.assertEqual(self.run_script("--check")[0], 0)
        state = tree_state(self.project)
        self.run_script()
        self.assertEqual(tree_state(self.project), state)

    def test_a_changed_preset_name_updates_only_the_strings(self):
        self.apply()
        before = tree_state(self.project)
        presets = json.loads(self.presets_file.read_text())
        presets["presets"][0]["name"] = "Kerf II"
        self.presets_file.write_text(json.dumps(presets))
        self.assertEqual(self.run_script()[0], 0)
        after = tree_state(self.project)
        changed = sorted(k for k in after if before.get(k) != after[k])
        self.assertEqual(changed, ["app/src/main/res/values/disguise_strings.xml"])

    def test_refresh_icons_renders_them_again(self):
        self.apply()
        f = self.res / "mipmap-mdpi" / "ic_launcher_kerf.png"
        before = f.stat().st_mtime_ns
        code, out, _ = self.run_script("--refresh-icons")
        self.assertEqual(code, 0)
        self.assertIn("refresh", out)
        self.assertGreaterEqual(f.stat().st_mtime_ns, before)


class FailureTests(Base):
    def test_a_missing_rsvg_convert_fails_clearly_before_writing_anything(self):
        before = tree_state(self.project)
        old = os.environ.get("PATH", "")
        os.environ["PATH"] = str(self.root / "empty")
        try:
            code, out, err = self.run_script(with_stub=False)
        finally:
            os.environ["PATH"] = old
        self.assertEqual(code, 2)
        self.assertIn("rsvg-convert is required", err)
        self.assertIn("nothing was written", err)
        self.assertEqual(tree_state(self.project), before)

    def test_a_failing_renderer_is_reported(self):
        write(self.assets / "kerf.svg", "FAIL")
        code, _, err = self.run_script()
        self.assertEqual(code, 2)
        self.assertIn("rsvg-convert failed on kerf.svg", err)

    def test_a_missing_icon_source_is_reported(self):
        (self.assets / "tally-fg.svg").unlink()
        code, _, err = self.run_script()
        self.assertEqual(code, 2)
        self.assertIn("missing icon source", err)

    def test_not_an_android_project(self):
        empty = self.root / "empty-project"
        empty.mkdir()
        code, _, err = self.run_script(project=empty)
        self.assertEqual(code, 2)
        self.assertIn("npm run tauri android init", err)
        code, _, err = self.run_script(project=self.root / "does-not-exist")
        self.assertEqual(code, 2)
        self.assertIn("not a folder", err)

    def test_a_manifest_that_is_not_what_we_expect(self):
        for text, needle in (
            ("<manifest", "not valid XML"),
            ("<resources/>", "not an Android manifest"),
            ('<manifest xmlns:android="http://schemas.android.com/apk/res/android"></manifest>', "no <application>"),
            ('<manifest xmlns:android="http://schemas.android.com/apk/res/android"><application><activity android:name=".A" xmlns:android="http://schemas.android.com/apk/res/android"/></application></manifest>', "cannot find the main activity"),
        ):
            self.manifest.write_text(text, encoding="utf-8")
            before = self.manifest.read_text(encoding="utf-8")
            code, _, err = self.run_script()
            self.assertEqual(code, 2, text)
            self.assertIn(needle, err)
            self.assertEqual(self.manifest.read_text(encoding="utf-8"), before, "nothing is written on failure")

    def test_bad_presets_are_refused(self):
        good = PRESETS["presets"][0]
        cases = {
            "bad id": [{**good, "id": "Kerf"}],
            "path id": [{**good, "id": "../etc"}],
            "digit id": [{**good, "id": "k3rf"}],
            "reserved": [{**good, "id": "original"}],
            "duplicate": [good, good],
            "empty name": [{**good, "name": "  "}],
            "newline name": [{**good, "name": "a\nb"}],
            "bad colour": [{**good, "background": "red"}],
            "none": [],
        }
        for what, presets in cases.items():
            self.presets_file.write_text(json.dumps({"presets": presets}))
            code, _, err = self.run_script()
            self.assertEqual(code, 2, what)
            self.assertIn("error:", err, what)
        self.presets_file.write_text("not json")
        self.assertEqual(self.run_script()[0], 2)
        self.presets_file.unlink()
        self.assertEqual(self.run_script()[0], 2)

    def test_a_package_that_cannot_be_found(self):
        (self.project / "app" / "build.gradle.kts").unlink()
        for f in (self.project / "app/src/main/java").rglob("MainActivity.kt"):
            f.unlink()
        code, _, err = self.run_script()
        self.assertEqual(code, 2)
        self.assertIn("pass --package", err)


class CommandLineTests(unittest.TestCase):
    def test_no_arguments_prints_usage(self):
        r = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("usage:", r.stdout)
        self.assertIn("--check", r.stdout)
        self.assertIn("--dry-run", r.stdout)

    def test_the_real_presets_json_yields_one_alias_per_preset_plus_the_original(self):
        # the repository's own presets, the stub renderer: 11 presets + the original look = 12 aliases
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            project = make_project(root)
            stub = make_stub(root)
            r = subprocess.run([sys.executable, str(SCRIPT), str(project), "--rsvg-convert", str(stub)], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            manifest = project / "app/src/main/AndroidManifest.xml"
            aliases = parse(manifest).find("application").findall("activity-alias")
            self.assertEqual(len(aliases), 12)
            presets = json.loads((REPO / "assets/disguise/presets.json").read_text())["presets"]
            self.assertEqual(
                [a.get(A + "name") for a in aliases],
                [".AliasOriginal"] + [".Alias" + p["id"].capitalize() for p in presets],
            )
            strings = ET.parse(project / "app/src/main/res/values/disguise_strings.xml").getroot()
            labels = {s.get("name"): s.text for s in strings}
            for p in presets:
                self.assertEqual(labels[f"app_label_{p['id']}"], p["name"])
            self.assertEqual(sum(a.get(A + "enabled") == "true" for a in aliases), 1)
            # idempotent on the full set too
            r = subprocess.run([sys.executable, str(SCRIPT), str(project), "--rsvg-convert", str(stub), "--check"], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout)

    @unittest.skipUnless(shutil.which("rsvg-convert"), "rsvg-convert is not installed")
    def test_real_rsvg_convert_renders_the_real_icons(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            project = make_project(root)
            presets = root / "presets.json"
            presets.write_text(json.dumps({"presets": [
                {"id": "kerf", "name": "Kerf", "description": "x", "background": "#0F1013"},
                {"id": "orvyn", "name": "Orvyn", "description": "x", "background": "#2A2440"},
            ]}))
            r = subprocess.run([sys.executable, str(SCRIPT), str(project), "--presets", str(presets)], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            res = project / "app/src/main/res"
            for density, (legacy, fg) in apm.DENSITIES.items():
                self.assertEqual(apm.png_size((res / f"mipmap-{density}/ic_launcher_kerf.png").read_bytes()), (legacy, legacy))
                self.assertEqual(apm.png_size((res / f"mipmap-{density}/ic_launcher_orvyn_foreground.png").read_bytes()), (fg, fg))
            again = subprocess.run([sys.executable, str(SCRIPT), str(project), "--presets", str(presets), "--check"], capture_output=True, text=True)
            self.assertEqual(again.returncode, 0, again.stdout)


class HelperTests(unittest.TestCase):
    def test_android_string_escapes(self):
        self.assertEqual(apm.android_string("a'b"), "a\\'b")
        self.assertEqual(apm.android_string('a"b'), 'a\\"b')
        self.assertEqual(apm.android_string("@x"), "\\@x")
        self.assertEqual(apm.android_string("?x"), "\\?x")
        self.assertEqual(apm.android_string("a&b<c>"), "a&amp;b&lt;c&gt;")
        self.assertEqual(apm.android_string("a\nb"), "a\\nb")
        self.assertEqual(apm.android_string("a\\b"), "a\\\\b")

    def test_png_size(self):
        self.assertIsNone(apm.png_size(b""))
        self.assertIsNone(apm.png_size(b"x" * 40))


if __name__ == "__main__":
    unittest.main()
