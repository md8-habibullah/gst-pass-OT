#!/usr/bin/env python3
"""Packs the Hexis browser extension for the two stores: extension/dist/hexis-passkeys-{chromium,firefox}.zip.

    python3 scripts/build_extension.py            # write both zips and print their SHA-256
    python3 scripts/build_extension.py --check    # only check that the sources agree (versions, file list), write nothing

The files are the ones the app itself carries and writes to <data folder>/extension/<family> (src-tauri/src/bridge/extension.rs):
the shared scripts and icons plus the family's manifest as manifest.json. The zips are reproducible (sorted names, fixed
timestamps), so the same sources give the same bytes. Standard library only. Nothing is uploaded anywhere: publishing to
Mozilla (addons.mozilla.org) or the Chrome Web Store is done by the account owner.
"""
import hashlib
import json
import re
import sys
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXT = REPO / "extension"
DIST = EXT / "dist"
RUST = REPO / "src-tauri" / "src" / "bridge" / "extension.rs"
STAMP = (2024, 1, 1, 0, 0, 0)  # the earliest date a zip can hold is 1980; any fixed one makes the bytes repeatable


def shared_files() -> list[tuple[str, Path]]:
    """The shared files, named as in the app: src/*.js and icons/*.png."""
    return sorted(
        [(p.name, p) for p in (EXT / "src").glob("*.js")] + [(p.name, p) for p in (EXT / "icons").glob("*.png")]
    )


def rust_names() -> list[str]:
    """The file names the app writes for every family (the `name:` of each part in extension.rs), and the two manifests."""
    text = RUST.read_text()
    return sorted(re.findall(r'name: "([^"]+)"', text))


def problems() -> list[str]:
    found = []
    names = [n for n, _ in shared_files()]
    if names != rust_names():
        found.append(f"the app writes {rust_names()} but extension/ holds {names}")
    manifests = {fam: json.loads((EXT / f"manifest.{fam}.json").read_text()) for fam in ("chromium", "firefox")}
    versions = {fam: m.get("version") for fam, m in manifests.items()}
    if len(set(versions.values())) != 1:
        found.append(f"the two manifests have different versions: {versions}")
    for fam, m in manifests.items():
        if m.get("manifest_version") != 3:
            found.append(f"{fam}: manifest_version is not 3")
        if m.get("permissions") != ["nativeMessaging"]:
            found.append(f"{fam}: permissions are {m.get('permissions')}, expected only nativeMessaging")
        for script in [s for c in m.get("content_scripts", []) for s in c.get("js", [])] + ([] if fam == "chromium" else m["background"]["scripts"]):
            if script not in names:
                found.append(f"{fam}: {script} is named in the manifest but is not packed")
    if "key" not in manifests["chromium"]:
        found.append("chromium: the manifest has no key, so an unpacked copy would get a different id than the host manifest allows")
    if manifests["firefox"].get("browser_specific_settings", {}).get("gecko", {}).get("id") is None:
        found.append("firefox: no gecko id")
    return found


def pack(family: str) -> tuple[Path, str]:
    DIST.mkdir(exist_ok=True)
    out = DIST / f"hexis-passkeys-{family}.zip"
    files = [("manifest.json", EXT / f"manifest.{family}.json")] + shared_files()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for name, path in sorted(files):
            info = zipfile.ZipInfo(name, STAMP)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, path.read_bytes())
    return out, hashlib.sha256(out.read_bytes()).hexdigest()


def main(argv: list[str]) -> int:
    bad = problems()
    for line in bad:
        print(f"PROBLEM: {line}", file=sys.stderr)
    if bad:
        return 1
    if "--check" in argv:
        print("the extension sources agree")
        return 0
    for family in ("chromium", "firefox"):
        path, digest = pack(family)
        print(f"{path.relative_to(REPO)}  sha256 {digest}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
