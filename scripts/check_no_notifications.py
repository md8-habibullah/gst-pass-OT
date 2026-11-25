#!/usr/bin/env python3
"""Fail if anything that could notify the user, run in the background or sit in a tray is in the project.

Hexis's rule (ai-docs/FEATURE_QUEUE.md Q0, decision D18): no push notifications, no tray
icon, no background daemon, no foreground service. Sync runs on four triggers only (unlock, a
committed change, app exit or backgrounding, the device coming back online), so none of the following may ever appear:

  * Android: any AndroidManifest.xml under src-tauri/gen/android/ that declares POST_NOTIFICATIONS,
    any FOREGROUND_SERVICE* permission, or a <service> element. The one exception is the passkey
    provider (ai-docs/PASSKEYS.md): Android's Credential Manager binds it for a few seconds while
    a sign-in sheet is open; it never runs by itself, shows nothing and is not a foreground
    service. It is allowed only in exactly that shape (see `is_credential_provider` below);
    any other <service>, or this one with a foreground type or another entry point, still fails.
  * Rust: a dependency on tauri-plugin-notification, a tray crate (tray-icon, tauri-plugin-tray,
    ksni, systray, trayicon ...), or the tauri "tray-icon" feature, in Cargo.toml, or such a crate
    in the dependency graph that Cargo.lock describes and Cargo would really compile (Cargo.lock
    also lists optional dependencies that stay switched off, for example tauri's tray-icon; those
    are reported as notes, not failures, once `cargo tree` confirms they are not built).
  * Tauri config: the words "notification" or "tray" in src-tauri/capabilities/*.json or
    src-tauri/tauri.conf.json (a permission, a plugin entry or a trayIcon block).

Standard library only (it calls `cargo tree`, offline, when Cargo is installed). Exit status 0 = clean, 1 = at least one hit. Run it from anywhere:

    python3 scripts/check_no_notifications.py [--root <project folder>]
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ANDROID_NS = "{http://schemas.android.com/apk/res/android}"

# The only <service> Hexis may declare: the passkey provider the system binds on demand.
PROVIDER_SERVICE = "dev.habibullah.hexis.provider.HexisCredentialProviderService"
PROVIDER_PERMISSION = "android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE"
PROVIDER_ACTION = "android.service.credentials.CredentialProviderService"

# Crates and features that put something in a tray or send a notification. Matched against the
# `name = "..."` lines of Cargo.lock and the dependency/feature text of Cargo.toml.
TRAY_OR_NOTIFY_NAMES = (
    "tauri-plugin-notification",
    "notify-rust",
    "tray-icon",
    "tauri-plugin-tray",
    "tauri-plugin-positioner",  # only useful for tray popups; flag it so it is a conscious choice
    "ksni",
    "systray",
    "trayicon",
    "libappindicator",
    "libayatana-appindicator",
    "app-indicator",
    "status-icon",
)
# `tauri-plugin-positioner` is deliberately not flagged as a hit by itself unless the tray feature
# is also on; keep the list of hard failures separate.
HARD_NAMES = tuple(n for n in TRAY_OR_NOTIFY_NAMES if n != "tauri-plugin-positioner")

WORD_RE = re.compile(r"notification|tray", re.IGNORECASE)


def rel(path: Path, root: Path) -> str:
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def is_credential_provider(service: ET.Element) -> list[str]:
    """Why `service` is NOT the plain passkey provider; an empty list means it is exactly that.

    It must be the Hexis provider class, guarded by the platform-only bind permission (so no other
    app can start it), have no foreground type, and answer to nothing but the Credential Manager's
    own action (no boot receiver style trigger, no custom action another app could send).
    """
    why: list[str] = []
    name = service.attrib.get(f"{ANDROID_NS}name", "")
    if name != PROVIDER_SERVICE:
        why.append(f"it is not the Hexis credential provider service ({PROVIDER_SERVICE})")
    if service.attrib.get(f"{ANDROID_NS}permission") != PROVIDER_PERMISSION:
        why.append(f"it is not guarded by {PROVIDER_PERMISSION}")
    if service.attrib.get(f"{ANDROID_NS}foregroundServiceType") is not None:
        why.append("it declares a foregroundServiceType")
    actions = [a.attrib.get(f"{ANDROID_NS}name", "") for f in service.findall("intent-filter") for a in f.findall("action")]
    if actions != [PROVIDER_ACTION]:
        why.append(f"its intent filter must answer to {PROVIDER_ACTION} only (found {actions or 'none'})")
    return why


def check_android(root: Path, problems: list[str]) -> None:
    android = root / "src-tauri" / "gen" / "android"
    if not android.is_dir():
        print("no Android project generated yet (src-tauri/gen/android does not exist)")
        return
    manifests = sorted(p for p in android.rglob("AndroidManifest.xml") if "build" not in p.parts)
    if not manifests:
        print("src-tauri/gen/android exists but holds no AndroidManifest.xml yet")
        return
    for manifest in manifests:
        where = rel(manifest, root)
        try:
            tree = ET.parse(manifest)
        except ET.ParseError as exc:
            problems.append(f"{where}: cannot be parsed as XML ({exc}); fix it so it can be checked")
            continue
        providers = [e for e in tree.getroot().iter("service") if e.attrib.get(f"{ANDROID_NS}name") == PROVIDER_SERVICE]
        if len(providers) > 1:
            problems.append(f"{where}: declares the credential provider <service> {len(providers)} times (once is allowed)")
        for el in tree.getroot().iter():
            tag = el.tag
            name = el.attrib.get(f"{ANDROID_NS}name", "")
            if tag == "service":
                why = is_credential_provider(el)
                if why:
                    problems.append(f"{where}: declares a <service> ({name or 'unnamed'}): {'; '.join(why)}")
            if tag in ("uses-permission", "uses-permission-sdk-23", "permission"):
                if name.endswith("POST_NOTIFICATIONS") or "FOREGROUND_SERVICE" in name:
                    problems.append(f"{where}: declares {name}")
        # A text search as well, so a permission hidden in a comment-like or odd construct is
        # still caught (manifest merger placeholders, tools:node="..." and so on).
        text = manifest.read_text(encoding="utf-8", errors="replace")
        for needle in ("POST_NOTIFICATIONS", "FOREGROUND_SERVICE"):
            if needle in text and not any(needle in p and where in p for p in problems):
                problems.append(f"{where}: mentions {needle}")
    print(f"checked {len(manifests)} AndroidManifest.xml file(s)")


def lock_packages(lock: Path) -> set[str]:
    return set(re.findall(r'^name = "([^"]+)"', lock.read_text(encoding="utf-8", errors="replace"), re.MULTILINE))


def active_in_build(root: Path, crate: str) -> bool | None:
    """Is `crate` part of the dependency graph that is really compiled (any target, default features)?

    Cargo.lock lists the *optional* dependencies of every crate whether or not a feature switches
    them on (tauri's lock entry names tray-icon although the tray-icon feature is off), so the lock
    file alone cannot tell. `cargo tree -i` asks Cargo itself, offline and without compiling.
    Returns None when Cargo is not available or cannot answer.
    """
    try:
        out = subprocess.run(
            ["cargo", "tree", "--locked", "--offline", "--target", "all", "-i", crate],
            cwd=root / "src-tauri",
            capture_output=True,
            text=True,
            timeout=180,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if "nothing to print" in out.stderr:
        return False
    if out.returncode == 0 and out.stdout.strip():
        return True
    return None


def check_cargo(root: Path, problems: list[str], notes: list[str]) -> None:
    toml = root / "src-tauri" / "Cargo.toml"
    if toml.is_file():
        where = rel(toml, root)
        for lineno, line in enumerate(toml.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            stripped = line.strip()
            if stripped.startswith("#"):
                continue
            lowered = stripped.lower()
            for crate in HARD_NAMES:
                # a quoted crate name, a `crate = ...` key or a `[dependencies.crate]` table
                if re.search(rf'(^|[\s"\[.]){re.escape(crate)}($|[\s"=\].])', lowered):
                    problems.append(f"{where}:{lineno}: {crate}")
            if re.search(r'"tray-icon"', lowered) or re.search(r"tray-icon\s*=", lowered):
                problems.append(f"{where}:{lineno}: the tauri tray-icon feature")
    lock = root / "src-tauri" / "Cargo.lock"
    if lock.is_file():
        where = rel(lock, root)
        present = lock_packages(lock)
        for crate in HARD_NAMES:
            if crate not in present:
                continue
            active = active_in_build(root, crate)
            if active:
                problems.append(f"{where}: {crate} is part of the dependency graph that gets compiled")
            elif active is None and crate == "tauri-plugin-notification":
                problems.append(f"{where}: {crate} is in the lock file (and Cargo could not say whether it is used)")
            elif active is None:
                notes.append(f"{where}: {crate} is listed (an optional dependency of another crate) and Cargo could not say whether it is switched on")
            else:
                notes.append(f"{where}: {crate} is only a dormant optional dependency (Cargo does not compile it)")


def check_tauri_config(root: Path, problems: list[str]) -> None:
    files = sorted((root / "src-tauri" / "capabilities").glob("*.json"))
    conf = root / "src-tauri" / "tauri.conf.json"
    if conf.is_file():
        files.append(conf)
    for path in files:
        where = rel(path, root)
        for lineno, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            if WORD_RE.search(line):
                problems.append(f"{where}:{lineno}: {line.strip()[:120]}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="project folder (default: the folder above scripts/)",
    )
    args = parser.parse_args()
    root: Path = args.root.resolve()
    problems: list[str] = []
    notes: list[str] = []
    check_android(root, problems)
    check_cargo(root, problems, notes)
    check_tauri_config(root, problems)
    for n in notes:
        print(f"note: {n}")
    if problems:
        print("\nFAIL: notifications, tray icons or background services are not allowed in Hexis:")
        for p in problems:
            print(f"  - {p}")
        return 1
    print("OK: no notification permission, foreground service, tray crate or notification/tray config found")
    return 0


if __name__ == "__main__":
    sys.exit(main())
