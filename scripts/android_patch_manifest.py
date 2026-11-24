#!/usr/bin/env python3
"""Apply the Q2 disguise ("App icon and name") to a generated Tauri Android project.

The script is tested against synthetic projects (`python3 -m unittest discover -s scripts/tests`) and, since 2026-11-24, has
been run against a real `tauri android init` project: the Gradle build, the manifest merge and both Kotlin plugins compile into
a debug APK. The launcher refresh and the plugins' behaviour on a device are checked separately (see
ai-docs/CHANGELOG_AND_STATE.md).

    python3 scripts/android_patch_manifest.py src-tauri/gen/android            # apply
    python3 scripts/android_patch_manifest.py src-tauri/gen/android --dry-run  # print the plan, write nothing
    python3 scripts/android_patch_manifest.py src-tauri/gen/android --check    # exit 1 if anything is missing or stale

Run it once after `npm run tauri android init` (which creates src-tauri/gen/android) and again
whenever assets/disguise/ changes. It is idempotent: a second run changes nothing, byte for byte.
Standard library only; icons are rendered with `rsvg-convert` (librsvg), which must be installed.

What it does, from assets/disguise/presets.json (the single source of truth for ids and names):

* AndroidManifest.xml (edited with xml.etree, comments and everything else kept): removes the
  LAUNCHER intent filter from the main activity, so there is exactly one icon, and adds one
  `<activity-alias>` per preset plus one for the original look (`.AliasOriginal`, label from
  `@string/app_label_original`, a copy of the app's app_name, icon `@mipmap/ic_launcher`). Only the
  original alias is enabled; the others are enabled one at a time at run time by the local Kotlin plugin below.
* res/values/disguise_strings.xml (the launcher labels), res/values/disguise_colors.xml (adaptive
  icon backgrounds), res/mipmap-*/ic_launcher_<id>.png (legacy icons, 48..192 px, from
  assets/disguise/<id>.svg), res/mipmap-*/ic_launcher_<id>_foreground.png (108 dp, from
  <id>-fg.svg) and res/mipmap-anydpi-v26/ic_launcher_<id>.xml (adaptive icons; minSdk is 28, so
  every device uses these).
* A small local Tauri plugin, DisguisePlugin.kt, next to MainActivity.kt: one command, `setAlias`,
  that enables the chosen alias and disables the others through
  PackageManager.setComponentEnabledSetting(..., DONT_KILL_APP). The Rust side (commands/disguise.rs)
  registers it with `register_android_plugin`; no change to MainActivity is needed for the disguise itself.
* MainActivity.kt, rewritten from scripts/android/MainActivity.kt.tmpl: Back on the first screen asks the page to close a
  sheet or step back and otherwise sends the app to the background (finishing it crashed the process on shutdown), and the
  content view is padded by the keyboard's height (edge-to-edge no longer resizes the window, so sheets sat behind it).
* Backup off: `android:allowBackup="false"`, `android:fullBackupContent="false"` and `android:dataExtractionRules` on `<application>`,
  plus res/xml/data_extraction_rules.xml excluding every domain. Without this Android may restore the vault files from a cloud
  backup after the app is uninstalled and installed again, which would contradict "uninstall leaves nothing behind".
* app/hexis.pro, copied from scripts/android/hexis.pro: R8 keep rules. Without them the release APK crashed the app on the
  first camera frame, because ML Kit creates its component registrars by reflection and R8 had removed their constructors.

It never GRANTS the app a permission (no notifications, no foreground service). It does add two removal markers to the
manifest: `<uses-permission android:name="android.permission.VIBRATE" tools:node="remove"/>` (the QR-scanner plugin declares
it for a command Hexis never uses) and `<uses-feature android:name="android.hardware.camera.any" android:required="false"
tools:node="replace"/>` (the scan has a typed-key alternative, so the app must stay installable without a camera).

Limits that no script can lift: a launcher label is a static resource, so a free-text custom name
changes only the title inside the app and the recent-apps label (set by the plugin); the lower-case
option is applied inside the app only; launchers cache icons and may need a moment (or a restart)
to show a switch.

Exit status: 0 done / up to date, 1 `--check` found work to do, 2 bad input or a missing tool.
"""
from __future__ import annotations

import argparse
import copy
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path

ANDROID_NS = "http://schemas.android.com/apk/res/android"
A = "{%s}" % ANDROID_NS
TOOLS_NS = "http://schemas.android.com/tools"
T = "{%s}" % TOOLS_NS
# Permissions that a library adds to the merged manifest and that Hexis never uses: removed with tools:node="remove".
# (The barcode-scanner plugin declares VIBRATE for a vibrate command that is not granted to the app.)
UNUSED_PERMISSIONS = ("android.permission.VIBRATE",)
# The camera is optional: the QR scan has a typed-key alternative, so the app must stay installable on devices without one.
OPTIONAL_FEATURES = ("android.hardware.camera.any",)
# Nothing of the app may travel to a cloud backup, to a new phone or into an `adb backup`: the vaults are sealed, but "uninstall
# leaves nothing behind" must also hold on Android, where a backup could put the data back after a reinstall.
BACKUP_RULES_NAME = "data_extraction_rules"
BACKUP_ATTRIBUTES = {
    "allowBackup": "false",
    "fullBackupContent": "false",  # Android 11 and older
    "dataExtractionRules": "@xml/" + BACKUP_RULES_NAME,  # Android 12 and newer
}
BACKUP_DOMAINS = {
    "cloud-backup": ("root", "file", "database", "sharedpref", "external"),
    "device-transfer": ("root", "file", "database", "sharedpref", "external", "device_root", "device_file"),
}
ACTION_MAIN = "android.intent.action.MAIN"
CATEGORY_LAUNCHER = "android.intent.category.LAUNCHER"

# density -> (legacy launcher icon px, adaptive foreground px = 108 dp)
DENSITIES = {
    "mdpi": (48, 108),
    "hdpi": (72, 162),
    "xhdpi": (96, 216),
    "xxhdpi": (144, 324),
    "xxxhdpi": (192, 432),
}
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
MANAGED_ALIAS = re.compile(r"^\.Alias[A-Z][A-Za-z0-9]*$")
ORIGINAL_ID = "original"
# The Gradle template passes every `*.pro` under app/ to R8, so this name is all it takes for the rules to apply.
KEEP_RULES_NAME = "hexis.pro"
GENERATED_NOTE = "Generated by scripts/android_patch_manifest.py from assets/disguise/presets.json. Do not edit."


class ScriptError(Exception):
    """Bad input or a missing tool (exit status 2)."""


# ------------------------------------------------------------------ presets


@dataclass(frozen=True)
class Preset:
    id: str
    name: str
    background: str


def alias_name(preset_id: str) -> str:
    """`kerf` -> `AliasKerf`; the original look is `AliasOriginal` (same rule as commands/disguise.rs)."""
    return "Alias" + preset_id[:1].upper() + preset_id[1:]


def load_presets(path: Path) -> list[Preset]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        raw = data["presets"]
    except (OSError, ValueError, KeyError, TypeError) as e:
        raise ScriptError(f"cannot read the presets from {path}: {e}") from None
    if not isinstance(raw, list) or not raw:
        raise ScriptError(f"{path} has no presets")
    out: list[Preset] = []
    seen: set[str] = set()
    for item in raw:
        if not isinstance(item, dict):
            raise ScriptError(f"{path}: a preset is not an object")
        pid, name, bg = item.get("id"), item.get("name"), item.get("background")
        if not isinstance(pid, str) or not re.fullmatch(r"[a-z]+", pid):
            raise ScriptError(f"{path}: preset id {pid!r} must match ^[a-z]+$")
        if pid == ORIGINAL_ID:
            raise ScriptError(f"{path}: the preset id {ORIGINAL_ID!r} is reserved for the original look")
        if pid in seen:
            raise ScriptError(f"{path}: duplicate preset id {pid!r}")
        seen.add(pid)
        if not isinstance(name, str) or not name.strip() or len(name) > 60 or any(ord(c) < 32 for c in name):
            raise ScriptError(f"{path}: preset {pid!r} needs a name of 1 to 60 printable characters")
        if not isinstance(bg, str) or not re.fullmatch(r"#[0-9A-Fa-f]{6}", bg):
            raise ScriptError(f"{path}: preset {pid!r} needs a #RRGGBB background")
        out.append(Preset(pid, name.strip(), bg.upper()))
    return out


# ------------------------------------------------------------------ escaping


def xml_text(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def android_string(s: str) -> str:
    """A value for a `<string>` resource: XML-escaped, then the aapt escapes (\\ ' " leading @ ? and newlines)."""
    out = s.replace("\\", "\\\\").replace("'", "\\'").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t")
    if out[:1] in ("@", "?"):
        out = "\\" + out
    return xml_text(out)


# ------------------------------------------------------------------ plan


@dataclass
class Step:
    kind: str  # patch | write | render
    path: Path
    detail: str
    apply: object  # callable taking no arguments


@dataclass
class Plan:
    steps: list[Step] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)

    def add(self, kind: str, path: Path, detail: str, apply) -> None:
        self.steps.append(Step(kind, path, detail, apply))


def write_atomic(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_bytes(data)
    os.replace(tmp, path)


def plan_text_file(plan: Plan, project: Path, path: Path, text: str, what: str) -> None:
    """Write `text` unless the file already holds exactly that."""
    try:
        current = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        current = None
    except (OSError, UnicodeDecodeError):
        current = ""
    if current == text:
        return
    verb = "create" if current is None else "update"
    plan.add("write", path, f"{verb} {what}", lambda p=path, t=text: write_atomic(p, t.encode("utf-8")))


# ------------------------------------------------------------------ PNG helpers


def png_size(data: bytes) -> tuple[int, int] | None:
    if len(data) < 24 or data[:8] != PNG_SIGNATURE or data[12:16] != b"IHDR":
        return None
    return struct.unpack(">II", data[16:24])  # type: ignore[return-value]


def png_ok(path: Path, size: int) -> bool:
    try:
        with path.open("rb") as f:
            return png_size(f.read(32)) == (size, size)
    except OSError:
        return False


def render_png(rsvg: str, svg: Path, size: int) -> bytes:
    try:
        out = subprocess.run(
            [rsvg, "-w", str(size), "-h", str(size), "-f", "png", str(svg)],
            check=True, capture_output=True, timeout=60,
        ).stdout
    except (OSError, subprocess.SubprocessError) as e:
        detail = getattr(e, "stderr", b"") or b""
        raise ScriptError(f"rsvg-convert failed on {svg.name}: {detail.decode(errors='replace').strip() or e}") from None
    if png_size(out) != (size, size):
        raise ScriptError(f"rsvg-convert did not produce a {size}x{size} PNG from {svg.name}")
    return out


# ------------------------------------------------------------------ project layout


@dataclass
class Layout:
    project: Path
    manifest: Path
    res: Path
    java_root: Path

    @staticmethod
    def of(project: Path) -> "Layout":
        main = project / "app" / "src" / "main"
        return Layout(project, main / "AndroidManifest.xml", main / "res", main / "java")


def find_namespace(project: Path, override: str | None) -> str:
    """The Kotlin package: `--package`, else `namespace` in app/build.gradle(.kts), else MainActivity.kt's package."""
    pattern = re.compile(r"^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$")
    if override is not None:
        if not pattern.match(override):
            raise ScriptError(f"--package {override!r} is not a valid package name")
        return override
    for name in ("build.gradle.kts", "build.gradle"):
        f = project / "app" / name
        if f.is_file():
            m = re.search(r"""^\s*namespace\s*=?\s*["']([^"']+)["']""", f.read_text(encoding="utf-8", errors="replace"), re.M)
            if m and pattern.match(m.group(1)):
                return m.group(1)
    for src in find_main_activity_sources(project):
        m = re.search(r"^\s*package\s+([\w.]+)", src.read_text(encoding="utf-8", errors="replace"), re.M)
        if m and pattern.match(m.group(1)):
            return m.group(1)
    raise ScriptError("cannot find the app's package (no `namespace` in app/build.gradle.kts and no MainActivity.kt); pass --package")


def find_main_activity_sources(project: Path) -> list[Path]:
    out: list[Path] = []
    for root in ("java", "kotlin"):
        base = project / "app" / "src" / "main" / root
        if base.is_dir():
            out.extend(sorted(base.rglob("MainActivity.kt")))
    return out


def app_label(layout: Layout) -> str:
    f = layout.res / "values" / "strings.xml"
    try:
        for s in ET.parse(f).getroot().iter("string"):
            if s.get("name") == "app_name" and (s.text or "").strip():
                return (s.text or "").strip().replace("\\'", "'").replace('\\"', '"')
    except (OSError, ET.ParseError):
        pass
    return "Hexis"


# ------------------------------------------------------------------ manifest


def _local(tag) -> str:
    return tag if isinstance(tag, str) else ""


def filter_has(filt: ET.Element, tag: str, name: str) -> bool:
    return any(c.get(A + "name") == name for c in filt.findall(tag))


def is_launcher_filter(filt: ET.Element) -> bool:
    return filter_has(filt, "category", CATEGORY_LAUNCHER)


def launcher_filters(elem: ET.Element) -> list[ET.Element]:
    return [f for f in elem.findall("intent-filter") if is_launcher_filter(f)]


def has_main_launcher(elem: ET.Element) -> bool:
    return any(filter_has(f, "action", ACTION_MAIN) and is_launcher_filter(f) for f in elem.findall("intent-filter"))


def default_filter() -> ET.Element:
    f = ET.Element("intent-filter")
    ET.SubElement(f, "action", {A + "name": ACTION_MAIN})
    ET.SubElement(f, "category", {A + "name": CATEGORY_LAUNCHER})
    return f


def format_subtree(elem: ET.Element, nl: str, unit: str) -> None:
    """Indent `elem`'s descendants: children one `unit` deeper than `nl`; the caller sets elem's own tail."""
    kids = list(elem)
    if not kids:
        return
    inner = nl + unit
    elem.text = inner
    for i, k in enumerate(kids):
        format_subtree(k, inner, unit)
        k.tail = inner if i < len(kids) - 1 else nl


def remove_child(parent: ET.Element, child: ET.Element) -> None:
    """Remove `child`, keeping the whitespace around its neighbours tidy."""
    kids = list(parent)
    i = kids.index(child)
    if i == 0:
        if len(kids) == 1:
            parent.text = child.tail
    else:
        kids[i - 1].tail = child.tail
    parent.remove(child)


def detect_indent(root: ET.Element, application: ET.Element) -> tuple[str, str]:
    """(newline + indentation of the application's children, one indentation step)."""
    def ws(s):
        return s if s and s.strip() == "" and "\n" in s else None
    child_nl = ws(application.text) or "\n        "
    app_nl = ws(root.text)
    unit = "    "
    if app_nl and child_nl.startswith(app_nl) and len(child_nl) > len(app_nl):
        unit = child_nl[len(app_nl):]
    return child_nl, unit


def _insert_before(root: ET.Element, anchor: ET.Element, elem: ET.Element, nl: str) -> None:
    """Insert `elem` just before `anchor`, indented like its siblings."""
    kids = list(root)
    i = kids.index(anchor)
    if i == 0:
        elem.tail = root.text
        root.text = nl
    else:
        elem.tail = kids[i - 1].tail
        kids[i - 1].tail = nl
    root.insert(i, elem)


def patch_manifest(root: ET.Element, presets: list[Preset]) -> tuple[list[str], list[str]]:
    """Mutate the manifest in place. Returns (changes, warnings); no changes means it is already right."""
    changes: list[str] = []
    warnings: list[str] = []
    application = root.find("application")
    if application is None:
        raise ScriptError("AndroidManifest.xml has no <application> element")

    desired = [(ORIGINAL_ID, "AliasOriginal", "@mipmap/ic_launcher", "@string/app_label_original")] + [
        (p.id, alias_name(p.id), f"@mipmap/ic_launcher_{p.id}", f"@string/app_label_{p.id}") for p in presets
    ]
    want_names = {"." + a for _, a, _, _ in desired}
    aliases = {e.get(A + "name"): e for e in application.findall("activity-alias")}

    # 1. the main activity: the one that has the MAIN/LAUNCHER filter (or, on a later run, the aliases' target)
    main = next((a for a in application.findall("activity") if has_main_launcher(a)), None)
    if main is None:
        targets = {aliases[n].get(A + "targetActivity") for n in aliases if n in want_names}
        main = next((a for a in application.findall("activity") if a.get(A + "name") in targets), None) if len(targets) == 1 else None
    if main is None:
        raise ScriptError("cannot find the main activity (an <activity> with a MAIN/LAUNCHER intent filter)")
    main_name = main.get(A + "name")
    if not main_name:
        raise ScriptError("the main activity has no android:name")

    # 2. take the LAUNCHER filter off the main activity (its categories, e.g. LEANBACK_LAUNCHER, are kept for the aliases)
    template = None
    for filt in launcher_filters(main):
        if template is None:
            template = copy.deepcopy(filt)
        remove_child(main, filt)
        changes.append(f"remove the LAUNCHER intent filter from {main_name}")

    # 3. one alias per look
    child_nl, unit = detect_indent(root, application)

    def new_filter() -> ET.Element:
        if template is None:
            return default_filter()
        filt = copy.deepcopy(template)
        for child in list(filt):
            if not isinstance(child.tag, str):  # a comment: not repeated in every alias
                filt.remove(child)
        return filt

    missing = []
    for pid, alias, icon, label in desired:
        name = "." + alias
        enabled = "true" if pid == ORIGINAL_ID else "false"
        want = {
            A + "name": name,
            A + "enabled": enabled,
            A + "exported": "true",
            A + "icon": icon,
            A + "label": label,
            A + "targetActivity": main_name,
        }
        elem = aliases.get(name)
        if elem is None:
            missing.append((name, want))
            continue
        for key, value in want.items():
            if elem.get(key) != value:
                elem.set(key, value)
                changes.append(f"fix {name}: {key.replace(A, 'android:')}={value}")
        if not any(filter_has(f, "action", ACTION_MAIN) and is_launcher_filter(f) for f in elem.findall("intent-filter")):
            elem.append(new_filter())
            format_subtree(elem, child_nl, unit)
            changes.append(f"fix {name}: add the MAIN/LAUNCHER intent filter")

    if missing:
        existing = [e for n, e in aliases.items() if n in want_names]
        anchor = existing[-1] if existing else main
        for name, want in missing:
            elem = ET.Element("activity-alias", want)
            elem.append(new_filter())
            kids = list(application)
            pos = kids.index(anchor) + 1
            application.insert(pos, elem)
            elem.tail = anchor.tail
            anchor.tail = child_nl
            format_subtree(elem, child_nl, unit)
            anchor = elem
            changes.append(f"add <activity-alias> {name}")

    # 3b. no backup of any kind (cloud, device-to-device, adb): a reinstall must start empty
    for key, value in BACKUP_ATTRIBUTES.items():
        if application.get(A + key) != value:
            application.set(A + key, value)
            changes.append(f"set android:{key}={value} on <application> (no backup of the app's data)")

    # 4. permissions and features that libraries add but that Hexis does not need (removal markers, never a grant)
    ET.register_namespace("tools", TOOLS_NS)
    root_nl = child_nl[: -len(unit)] if child_nl.endswith(unit) else "\n    "  # newline + the indentation of <application> itself
    anchor_el = application
    for perm in UNUSED_PERMISSIONS:
        have = [e for e in root.findall("uses-permission") if e.get(A + "name") == perm]
        if any(e.get(T + "node") == "remove" for e in have):
            continue
        if have:
            for e in have:
                e.set(T + "node", "remove")
        else:
            elem = ET.Element("uses-permission", {A + "name": perm, T + "node": "remove"})
            _insert_before(root, anchor_el, elem, root_nl)
        changes.append(f"remove the {perm.rsplit('.', 1)[-1]} permission that a library adds (Hexis never uses it)")
    for feat in OPTIONAL_FEATURES:
        have = [e for e in root.findall("uses-feature") if e.get(A + "name") == feat]
        if any(e.get(A + "required") == "false" and e.get(T + "node") == "replace" for e in have):
            continue
        if have:
            for e in have:
                e.set(A + "required", "false")
                e.set(T + "node", "replace")
        else:
            elem = ET.Element("uses-feature", {A + "name": feat, A + "required": "false", T + "node": "replace"})
            _insert_before(root, anchor_el, elem, root_nl)
        changes.append(f"make {feat} optional (the QR scan has a typed-key alternative)")

    # 5. aliases of this kind that presets.json no longer has: report, never delete
    for name in aliases:
        if name and MANAGED_ALIAS.match(name) and name not in want_names:
            warnings.append(f"the manifest has {name}, which is not in presets.json; left alone")
    return changes, warnings


def read_manifest(path: Path, declared: dict[str, str]) -> tuple[str, ET.Element]:
    """Parse the manifest (comments kept). `declared` receives the namespace prefixes the file declares."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        raise ScriptError(
            f"{path} not found: run `npm run tauri android init` first (it creates src-tauri/gen/android)"
        ) from None
    try:
        for _, (prefix, uri) in ET.iterparse(path, events=("start-ns",)):
            if prefix and not re.fullmatch(r"ns\d+", prefix):
                ET.register_namespace(prefix, uri)
                declared[prefix] = uri
        tree = ET.parse(path, parser=ET.XMLParser(target=ET.TreeBuilder(insert_comments=True)))
    except (ET.ParseError, OSError) as e:
        raise ScriptError(f"{path} is not valid XML: {e}") from None
    root = tree.getroot()
    if root.tag != "manifest":
        raise ScriptError(f"{path} is not an Android manifest (root element {root.tag!r})")
    return text, root


def serialize_manifest(original: str, root: ET.Element, declared: dict[str, str]) -> str:
    """The new text: the original prolog (declaration, comments before the root) and epilog kept verbatim."""
    masked = re.sub(r"<!--.*?-->", lambda m: " " * len(m.group()), original, flags=re.S)
    start = re.search(r"<manifest[\s>/]", masked)
    end = masked.rfind("</manifest>")
    prolog = original[: start.start()] if start else ""
    epilog = original[end + len("</manifest>"):] if end >= 0 else "\n"
    if not prolog.strip():
        prolog = '<?xml version="1.0" encoding="utf-8"?>\n'
    body = ET.tostring(root, encoding="unicode")
    # ElementTree only declares the namespaces that are used; keep the ones the file declared anyway
    # (`xmlns:tools` is typically declared and used later by hand).
    extra = "".join(f' xmlns:{p}="{u}"' for p, u in declared.items() if f"xmlns:{p}=" not in body.split(">", 1)[0])
    if extra:
        body = body.replace("<manifest", "<manifest" + extra, 1)
    return prolog + body + epilog


# ------------------------------------------------------------------ Kotlin


def kotlin_source(package: str, aliases: list[str]) -> str:
    alias_list = ",\n    ".join(f'"{a}"' for a in aliases)
    return f"""// {GENERATED_NOTE}
// UNVERIFIED on a device: written against the Tauri 2.12 Android plugin API, never compiled here.
package {package}

import android.app.Activity
import android.app.ActivityManager
import android.content.ComponentName
import android.content.pm.PackageManager
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@InvokeArg
class SetAliasArgs {{
  lateinit var alias: String
  var label: String? = null
}}

/**
 * Switches the launcher icon and name by enabling one `<activity-alias>` of AndroidManifest.xml and
 * disabling the others. Registered from Rust (`register_android_plugin`); needs no permission.
 */
@TauriPlugin
class DisguisePlugin(private val activity: Activity) : Plugin(activity) {{
  private val aliases = listOf(
    {alias_list}
  )

  @Command
  fun setAlias(invoke: Invoke) {{
    val args = invoke.parseArgs(SetAliasArgs::class.java)
    if (args.alias !in aliases) {{
      invoke.reject("Unknown launcher alias")
      return
    }}
    try {{
      val pm = activity.packageManager
      // The chosen alias first, so the app never has no launcher icon; a failure here changes nothing else.
      pm.setComponentEnabledSetting(component(args.alias), PackageManager.COMPONENT_ENABLED_STATE_ENABLED, PackageManager.DONT_KILL_APP)
      for (other in aliases) {{
        if (other != args.alias) {{
          pm.setComponentEnabledSetting(component(other), PackageManager.COMPONENT_ENABLED_STATE_DISABLED, PackageManager.DONT_KILL_APP)
        }}
      }}
    }} catch (e: Exception) {{
      invoke.reject("The launcher icon could not be switched", e)
      return
    }}
    // The recent-apps label follows the name the person chose, a custom name included.
    val label = args.label?.trim()?.take(60)
    if (!label.isNullOrEmpty()) {{
      activity.runOnUiThread {{
        @Suppress("DEPRECATION")
        activity.setTaskDescription(ActivityManager.TaskDescription(label))
      }}
    }}
    invoke.resolve()
  }}

  private fun component(alias: String) = ComponentName(activity.packageName, "{package}." + alias)
}}
"""


# ------------------------------------------------------------------ the whole plan


def build_plan(project: Path, repo: Path, presets_file: Path, assets: Path, package: str | None, refresh_icons: bool, rsvg: str | None) -> Plan:
    layout = Layout.of(project)
    presets = load_presets(presets_file)
    plan = Plan()

    # manifest (read first: an empty folder gets the "run tauri android init" hint, not a package error)
    declared: dict[str, str] = {}
    original, root = read_manifest(layout.manifest, declared)
    ns = find_namespace(project, package)
    label = app_label(layout)
    changes, warnings = patch_manifest(root, presets)
    plan.warnings.extend(warnings)
    if changes:
        new_text = serialize_manifest(original, root, declared)
        plan.add("patch", layout.manifest, "; ".join(dict.fromkeys(changes)),
                 lambda p=layout.manifest, t=new_text: write_atomic(p, t.encode("utf-8")))

    # strings and colours
    rows = [f'    <string name="app_label_{ORIGINAL_ID}">{android_string(label)}</string>']
    rows += [f'    <string name="app_label_{p.id}">{android_string(p.name)}</string>' for p in presets]
    plan_text_file(plan, project, layout.res / "values" / "disguise_strings.xml",
                   '<?xml version="1.0" encoding="utf-8"?>\n<!-- ' + GENERATED_NOTE + ' -->\n<resources>\n' + "\n".join(rows) + "\n</resources>\n",
                   "the launcher labels")
    colors = [f'    <color name="ic_launcher_{p.id}_background">{p.background}</color>' for p in presets]
    plan_text_file(plan, project, layout.res / "values" / "disguise_colors.xml",
                   '<?xml version="1.0" encoding="utf-8"?>\n<!-- ' + GENERATED_NOTE + ' -->\n<resources>\n' + "\n".join(colors) + "\n</resources>\n",
                   "the adaptive icon backgrounds")

    # the backup rules the manifest points to: every domain is excluded, for the cloud and for a phone-to-phone transfer
    sections = []
    for section, domains in BACKUP_DOMAINS.items():
        rows = "".join(f'        <exclude domain="{d}" path="."/>\n' for d in domains)
        sections.append(f"    <{section}>\n{rows}    </{section}>\n")
    plan_text_file(plan, project, layout.res / "xml" / f"{BACKUP_RULES_NAME}.xml",
                   '<?xml version="1.0" encoding="utf-8"?>\n<!-- ' + GENERATED_NOTE + ' -->\n<data-extraction-rules>\n' + "".join(sections) + "</data-extraction-rules>\n",
                   "the backup rules (nothing is backed up or transferred)")

    # adaptive icons
    for p in presets:
        plan_text_file(
            plan, project, layout.res / "mipmap-anydpi-v26" / f"ic_launcher_{p.id}.xml",
            '<?xml version="1.0" encoding="utf-8"?>\n<!-- ' + GENERATED_NOTE + ' -->\n'
            '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
            f'  <foreground android:drawable="@mipmap/ic_launcher_{p.id}_foreground"/>\n'
            f'  <background android:drawable="@color/ic_launcher_{p.id}_background"/>\n'
            '</adaptive-icon>\n',
            f"the adaptive icon of {p.id}",
        )

    # rendered PNGs
    for p in presets:
        for density, (legacy, fg) in DENSITIES.items():
            for src_name, out_name, size in (
                (f"{p.id}.svg", f"ic_launcher_{p.id}.png", legacy),
                (f"{p.id}-fg.svg", f"ic_launcher_{p.id}_foreground.png", fg),
            ):
                out = layout.res / f"mipmap-{density}" / out_name
                if png_ok(out, size) and not refresh_icons:
                    continue
                svg = assets / src_name
                if not svg.is_file():
                    raise ScriptError(f"missing icon source {svg}")
                verb = "refresh" if out.exists() else "render"
                plan.add("render", out, f"{verb} {size}px from {src_name}",
                         lambda o=out, s=svg, n=size: write_atomic(o, render_png(rsvg or "rsvg-convert", s, n)))

    # Kotlin plugin, next to MainActivity.kt when there is one
    sources = find_main_activity_sources(project)
    kt_dir = sources[0].parent if sources else layout.java_root.joinpath(*ns.split("."))
    aliases = ["AliasOriginal"] + [alias_name(p.id) for p in presets]
    plan_text_file(plan, project, kt_dir / "DisguisePlugin.kt", kotlin_source(ns, aliases), "the Kotlin plugin DisguisePlugin")

    # The activity (Back goes to the background, the page shrinks for the keyboard) and the R8 keep rules (without them
    # the release APK crashes the app on the first camera frame). See the two files for the reasons.
    plan_text_file(plan, project, kt_dir / "MainActivity.kt", main_activity_source(repo, ns), "MainActivity (Back to the background, keyboard resize)")
    plan_text_file(plan, project, layout.project / "app" / KEEP_RULES_NAME, android_file(repo, "hexis.pro"), "the R8 keep rules (ML Kit)")
    return plan


def android_file(repo: Path, name: str) -> str:
    """One of the files under scripts/android/ that this script installs into the generated project."""
    path = repo / "scripts" / "android" / name
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as e:
        raise ScriptError(f"cannot read {path}: {e}") from None


def main_activity_source(repo: Path, package: str) -> str:
    return android_file(repo, "MainActivity.kt.tmpl").replace("__PACKAGE__", package)


def rsvg_available(rsvg: str | None) -> bool:
    return bool(rsvg) and (Path(rsvg).is_file() or shutil.which(rsvg) is not None)


def describe(plan: Plan, project: Path) -> list[str]:
    def rel(p: Path) -> str:
        try:
            return str(p.relative_to(project))
        except ValueError:
            return str(p)
    return [f"  {s.kind:<6} {rel(s.path)}: {s.detail}" for s in plan.steps]


def main(argv: list[str] | None = None, repo_root: Path | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="android_patch_manifest.py",
        description="Apply the Q2 disguise (launcher aliases, icons, labels, Kotlin plugin) to a generated Tauri Android project.",
        epilog="Run once after `npm run tauri android init`, and again when assets/disguise/ changes. Idempotent. UNVERIFIED on a device.",
    )
    parser.add_argument("project", nargs="?", help="the generated Android project (src-tauri/gen/android)")
    parser.add_argument("--check", action="store_true", help="write nothing; exit 1 if anything is missing or out of date")
    parser.add_argument("--dry-run", action="store_true", help="print the plan and write nothing")
    parser.add_argument("--package", help="Kotlin package of the app (default: namespace from app/build.gradle.kts)")
    parser.add_argument("--rsvg-convert", dest="rsvg", help="path of rsvg-convert (default: look it up on PATH)")
    parser.add_argument("--refresh-icons", action="store_true", help="render every icon again, even those that already exist")
    parser.add_argument("--presets", help="presets.json to use (default: assets/disguise/presets.json of this repository)")
    parser.add_argument("--assets", help="folder with <id>.svg and <id>-fg.svg (default: assets/disguise of this repository)")
    args = parser.parse_args(argv)
    if args.project is None:
        parser.print_help()
        return 2

    repo = repo_root or Path(__file__).resolve().parent.parent
    project = Path(args.project)
    presets_file = Path(args.presets) if args.presets else repo / "assets" / "disguise" / "presets.json"
    assets = Path(args.assets) if args.assets else repo / "assets" / "disguise"
    try:
        if not project.is_dir():
            raise ScriptError(f"{project} is not a folder")
        rsvg = args.rsvg or shutil.which("rsvg-convert")
        plan = build_plan(project, repo, presets_file, assets, args.package, args.refresh_icons, rsvg)
        for w in plan.warnings:
            print(f"warning: {w}")
        writing = not (args.check or args.dry_run)
        renders = [s for s in plan.steps if s.kind == "render"]
        if renders and writing and not rsvg_available(rsvg):
            raise ScriptError(
                "rsvg-convert is required to render the icons and was not found "
                "(install librsvg, for example `sudo pacman -S librsvg` or `sudo apt install librsvg2-bin`), "
                "or pass --rsvg-convert <path>; nothing was written"
            )
        if not plan.steps:
            print("up to date: nothing to do")
            return 0
        if args.check:
            print("not up to date:")
            print("\n".join(describe(plan, project)))
            return 1
        if args.dry_run:
            print("plan (nothing is written):")
            print("\n".join(describe(plan, project)))
            if renders and not rsvg_available(rsvg):
                print("note: rsvg-convert was not found; a real run would stop before writing anything")
            return 0
        for step in plan.steps:
            step.apply()
        print(f"done: {len(plan.steps)} change(s)")
        print("\n".join(describe(plan, project)))
        return 0
    except ScriptError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
