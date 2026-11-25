#!/usr/bin/env python3
"""The one place that knows the version of Hexis.

`VERSION` (one line, `X.Y.Z` or `X.Y.Z-suffix`) is the source of truth. This script copies it into every file that has to
carry it, and refuses to let them disagree. Nobody (and no AI agent) edits a version number by hand:

    python3 scripts/version.py show                 print the version
    python3 scripts/version.py suggest              which bump the CHANGELOG's [Unreleased] section calls for, and why
    python3 scripts/version.py bump auto            do that bump: VERSION, every file below and the CHANGELOG heading
    python3 scripts/version.py bump minor           (or major, patch) when you decide yourself
    python3 scripts/version.py set 1.0.0            an exact version (never lower than the current one)
    python3 scripts/version.py sync                 copy VERSION into the files (after editing VERSION by hand)
    python3 scripts/version.py check [--release]    fail when a file disagrees; --release also needs the CHANGELOG section

Files kept in step: package.json, package-lock.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml, src-tauri/Cargo.lock
(the `hexis` entry only), packaging/arch/PKGBUILD (`pkgver`, a `-` becomes `_`). Android's versionName and versionCode are
made by Tauri from tauri.conf.json at build time, so there is nothing to keep there. Releasing a version turns the
CHANGELOG's `## [Unreleased]` into `## [X.Y.Z] - date` and opens a new empty one above it.

Which bump `auto` picks (the CHANGELOG is the record of what changed, so it is the input):
  * a `BREAKING` mark or a `### Removed` section: major (before 1.0.0 a minor one: "before 1.0 a minor bump may change a stored format")
  * a `### Added`, `### Changed` or `### Deprecated` section: minor
  * only `### Fixed` or `### Security`: patch
  * nothing under [Unreleased]: refuses, there is nothing to release

CI runs `check --release` before it publishes, so a release can never go out with files that disagree. Standard library only.
"""
from __future__ import annotations

import argparse
import datetime
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z][0-9A-Za-z.-]*))?$")


class Fail(Exception):
    pass


# Each target: (file, what it is, a pattern with three groups: text before the version, the version, text after it).
# Only the first match counts, and a file without a match is an error (the format changed under us).
TARGETS = [
    ("package.json", r'(?P<a>^  "version": ")(?P<v>[^"]*)(?P<z>",$)', re.M),
    ("package-lock.json", r'(?P<a>\A\{\s*"name": "hexis",\s*"version": ")(?P<v>[^"]*)(?P<z>")', 0),
    ("package-lock.json", r'(?P<a>"packages": \{\s*"": \{\s*"name": "hexis",\s*"version": ")(?P<v>[^"]*)(?P<z>")', 0),
    ("src-tauri/tauri.conf.json", r'(?P<a>^  "version": ")(?P<v>[^"]*)(?P<z>",$)', re.M),
    ("src-tauri/Cargo.toml", r'(?P<a>\[package\]\nname = "hexis"\nversion = ")(?P<v>[^"]*)(?P<z>")', 0),
    ("src-tauri/Cargo.lock", r'(?P<a>\[\[package\]\]\nname = "hexis"\nversion = ")(?P<v>[^"]*)(?P<z>")', 0),
    ("packaging/arch/PKGBUILD", r"(?P<a>^pkgver=)(?P<v>\S*)(?P<z>$)", re.M),
]


def pkgver(version: str) -> str:
    """A pacman version cannot contain a hyphen."""
    return version.replace("-", "_")


def wanted(path: str, version: str) -> str:
    return pkgver(version) if path.endswith("PKGBUILD") else version


def parse(version: str) -> tuple[int, int, int, str | None]:
    m = SEMVER.match(version)
    if not m:
        raise Fail(f"{version!r} is not a version (X.Y.Z, or X.Y.Z-suffix)")
    return int(m[1]), int(m[2]), int(m[3]), m[4]


def order(version: str) -> tuple:
    """Sort key: a release is newer than its own pre-releases (0.3.0-rc1 < 0.3.0)."""
    major, minor, patch, suffix = parse(version)
    return major, minor, patch, 1 if suffix is None else 0, suffix or ""


def read_version(root: Path) -> str:
    f = root / "VERSION"
    if not f.is_file():
        raise Fail("VERSION is missing (one line: the version)")
    v = f.read_text(encoding="utf-8").strip()
    parse(v)
    return v


def found(root: Path) -> list[tuple[str, str]]:
    """(file, version written in it) for every target."""
    out = []
    for path, pattern, flags in TARGETS:
        text = (root / path).read_text(encoding="utf-8")
        m = re.search(pattern, text, flags)
        if not m:
            raise Fail(f"{path}: the version line was not found (did its layout change?)")
        out.append((path, m["v"]))
    return out


def disagreements(root: Path, version: str) -> list[str]:
    return [f"{p}: {got!r} (should be {wanted(p, version)!r})" for p, got in found(root) if got != wanted(p, version)]


def write_targets(root: Path, version: str) -> list[str]:
    changed = []
    texts = {}
    for path, pattern, flags in TARGETS:
        text = texts.get(path) or (root / path).read_text(encoding="utf-8")
        new, n = re.subn(pattern, lambda m: m["a"] + wanted(path, version) + m["z"], text, count=1, flags=flags)
        if n != 1:
            raise Fail(f"{path}: the version line was not found (did its layout change?)")
        texts[path] = new
        if new != (root / path).read_text(encoding="utf-8"):
            changed.append(path)
    for path, text in texts.items():
        (root / path).write_text(text, encoding="utf-8")
    return sorted(set(changed))


# ------------------------------------------------------------------ the CHANGELOG


def unreleased(text: str) -> tuple[int, int, str]:
    """(start, end, body) of the `## [Unreleased]` section: from its heading to the next `## [`."""
    m = re.search(r"^## \[Unreleased\][^\n]*\n", text, re.M)
    if not m:
        raise Fail("CHANGELOG.md has no '## [Unreleased]' section")
    nxt = re.search(r"^## \[", text[m.end():], re.M)
    end = m.end() + nxt.start() if nxt else len(text)
    return m.start(), end, text[m.end():end]


def has_section(text: str, version: str) -> bool:
    return re.search(rf"^## \[{re.escape(version)}\]", text, re.M) is not None


def decide(current: str, body: str) -> tuple[str, str]:
    """('major'|'minor'|'patch', the reason) from the text under [Unreleased]."""
    if not body.strip():
        raise Fail("nothing is listed under '## [Unreleased]' in CHANGELOG.md, so there is nothing to release")
    major_before_1 = parse(current)[0] == 0
    breaking = re.search(r"BREAKING", body, re.I) or re.search(r"^### Removed", body, re.M)
    if breaking:
        if major_before_1:
            return "minor", "a breaking or removed change, and before 1.0.0 that is a minor bump"
        return "major", "a breaking or removed change"
    heads = sorted(set(re.findall(r"^### (Added|Changed|Deprecated)", body, re.M)))
    if heads:
        return "minor", "it lists " + ", ".join(heads)
    return "patch", "it lists only fixes"


def bumped(current: str, kind: str) -> str:
    major, minor, patch, _ = parse(current)
    if kind == "major":
        return f"{major + 1}.0.0"
    if kind == "minor":
        return f"{major}.{minor + 1}.0"
    if kind == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise Fail(f"unknown bump {kind!r}")


def release_changelog(text: str, version: str, date: str) -> str:
    """`## [Unreleased]` becomes `## [version] - date`, with a new empty one above. Already released: unchanged."""
    if has_section(text, version):
        return text
    start, end, body = unreleased(text)
    if not body.strip():
        # the release after its own release candidate: the candidate's section becomes the release's
        rc = re.search(rf"^## \[{re.escape(version)}-[^\]]*\][^\n]*$", text, re.M)
        if rc:
            return text[:rc.start()] + f"## [{version}] - {date}" + text[rc.end():]
        raise Fail(f"CHANGELOG.md has no '## [{version}]' section and nothing under '## [Unreleased]' to release")
    heading = re.match(r"^## \[Unreleased\][^\n]*\n", text[start:], re.M)
    assert heading
    return text[:start] + "## [Unreleased]\n\n" + f"## [{version}] - {date}\n" + text[start + heading.end():]


# ------------------------------------------------------------------ commands


def cmd_show(root: Path, _a) -> int:
    print(read_version(root))
    return 0


def cmd_suggest(root: Path, _a) -> int:
    current = read_version(root)
    kind, why = decide(current, unreleased((root / "CHANGELOG.md").read_text(encoding="utf-8"))[2])
    print(f"{current} -> {bumped(current, kind)} ({kind}): {why}")
    return 0


def cmd_check(root: Path, a) -> int:
    version = read_version(root)
    bad = disagreements(root, version)
    if a.release and not has_section((root / "CHANGELOG.md").read_text(encoding="utf-8"), version):
        bad.append(f"CHANGELOG.md: no '## [{version}]' section (run: python3 scripts/version.py set {version})")
    if bad:
        print(f"The version is {version} (file VERSION), but:", file=sys.stderr)
        for line in bad:
            print("  " + line, file=sys.stderr)
        print("Fix: python3 scripts/version.py sync", file=sys.stderr)
        return 1
    print(f"version {version}: every file agrees" + (" and the CHANGELOG has its section" if a.release else ""))
    return 0


def apply(root: Path, version: str, date: str, changelog: bool) -> None:
    (root / "VERSION").write_text(version + "\n", encoding="utf-8")
    changed = ["VERSION"] + write_targets(root, version)
    if changelog:
        path = root / "CHANGELOG.md"
        old = path.read_text(encoding="utf-8")
        new = release_changelog(old, version, date)
        if new != old:
            path.write_text(new, encoding="utf-8")
            changed.append("CHANGELOG.md")
    print(f"version {version}: updated " + (", ".join(changed) if changed else "nothing (already in step)"))
    print("Cargo.lock and package-lock.json are edited in place; run the checks (scripts/verify_all.sh) before you push.")


def cmd_sync(root: Path, a) -> int:
    apply(root, read_version(root), a.date, changelog=False)
    return 0


def cmd_set(root: Path, a) -> int:
    new = a.version.lstrip("v")
    parse(new)
    current = read_version(root)
    if order(new) < order(current) and not a.force:
        raise Fail(f"{new} is lower than the current {current}; pass --force if that is really meant")
    apply(root, new, a.date, changelog=True)
    return 0


def cmd_bump(root: Path, a) -> int:
    current = read_version(root)
    if parse(current)[3]:
        raise Fail(f"{current} has a suffix; use 'set X.Y.Z' to choose the release version")
    bad = disagreements(root, current)
    if bad:
        raise Fail("the files do not agree with VERSION yet (" + "; ".join(bad) + "); run 'sync' first")
    kind = a.kind
    if kind == "auto":
        kind, why = decide(current, unreleased((root / "CHANGELOG.md").read_text(encoding="utf-8"))[2])
        print(f"{kind}: {why}")
    new = bumped(current, kind)
    print(f"{current} -> {new}")
    apply(root, new, a.date, changelog=True)
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="The one place that knows the version of Hexis.", formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    p.add_argument("--root", type=Path, default=ROOT, help="the repository (default: this one; tests use a copy)")
    sub = p.add_subparsers(dest="cmd", required=True)
    today = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")

    def add(name, fn, **kw):
        s = sub.add_parser(name, **kw)
        s.set_defaults(fn=fn)
        return s

    add("show", cmd_show, help="print the version")
    add("suggest", cmd_suggest, help="which bump the CHANGELOG calls for")
    c = add("check", cmd_check, help="fail when a file disagrees with VERSION")
    c.add_argument("--release", action="store_true", help="also require the CHANGELOG section for this version")
    s = add("sync", cmd_sync, help="copy VERSION into the files")
    s.add_argument("--date", default=today)
    s = add("set", cmd_set, help="an exact version")
    s.add_argument("version")
    s.add_argument("--date", default=today, help="the date for the CHANGELOG heading (UTC, default: today)")
    s.add_argument("--force", action="store_true", help="allow a lower version")
    s = add("bump", cmd_bump, help="the next major, minor or patch version (or auto)")
    s.add_argument("kind", choices=["major", "minor", "patch", "auto"])
    s.add_argument("--date", default=today)
    a = p.parse_args(argv)
    try:
        return a.fn(a.root.resolve(), a)
    except Fail as e:
        print(f"version.py: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
