#!/usr/bin/env python3
"""Write the GitHub release description for one version (Markdown, on stdout). Standard library only.

    python3 scripts/release_notes.py --version 0.1.0 --repo owner/name --site https://owner.github.io/name \\
        [--sums files/SHA256SUMS] [--changelog CHANGELOG.md] [--nightly-of SHA]

The CI release job runs this with the checksums of the files it is about to attach, so the table in the description
always matches the attachments. "What is new" is the matching section of CHANGELOG.md.
"""
import argparse
import pathlib
import re
import sys

# file name pattern -> (what it is, who it is for)
KINDS = [
    (r"^Hexis_.+_amd64\.deb$", "Debian package", "Debian, Ubuntu, Linux Mint, Pop!_OS and relatives (64-bit x86)"),
    (r"^Hexis-.+\.x86_64\.rpm$", "RPM package", "Fedora, RHEL, Rocky, openSUSE and relatives (64-bit x86)"),
    (r"^Hexis_.+_amd64\.AppImage$", "AppImage", "Any Linux with glibc 2.35 or newer; no root needed (64-bit x86)"),
    (r"^hexis-.+-linux-x86_64\.tar\.gz$", "Tarball", "Any Linux; also what the installer uses on Arch-family systems (64-bit x86)"),
    (r"^hexis-.+-android-arm64.*\.apk$", "Android app", "Android 9 or newer, 64-bit ARM phones and tablets"),
]


def changelog_section(text: str, version: str) -> str:
    """The body of `## [version]` in a Keep a Changelog file, without its heading, or an empty string."""
    m = re.search(rf"^## \[{re.escape(version)}\][^\n]*\n(.*?)(?=^## \[|\Z)", text, re.S | re.M)
    return m.group(1).strip() if m else ""


def shift_headings(md: str) -> str:
    """`### Added` becomes `#### Added`: the notes already use `##` for their own sections."""
    return re.sub(r"^(#{3,5}) ", lambda m: m.group(1) + "# ", md, flags=re.M)


def downloads_table(sums_text: str) -> str:
    rows = []
    for line in sums_text.splitlines():
        parts = line.split()
        if len(parts) != 2:
            continue
        digest, name = parts[0], parts[1].lstrip("*")
        for pattern, kind, who in KINDS:
            if re.match(pattern, name):
                note = " (signed with a throw-away key; it cannot update an app installed from another build)" if "-testkey" in name else ""
                rows.append((kind, f"`{name}`", who + note, f"`{digest[:16]}…`"))
                break
    if not rows:
        return ""
    out = ["| Download | File | For | SHA-256 (start) |", "| --- | --- | --- | --- |"]
    out += [f"| {k} | {f} | {w} | {d} |" for k, f, w, d in rows]
    return "\n".join(out)


def build(version: str, repo: str, site: str, sums: str, changelog: str) -> str:
    site = site.rstrip("/")
    new = shift_headings(changelog_section(changelog, version)) or f"See [CHANGELOG.md](https://github.com/{repo}/blob/v{version}/CHANGELOG.md)."
    table = downloads_table(sums)
    parts = [
        f"**Hexis {version}** is a local-first password, one-time-code, note and file vault with post-quantum encryption "
        "(ML-KEM-1024 with X25519, Argon2id, XChaCha20-Poly1305). Your vault lives on your device; backup to your own "
        "S3-compatible storage is optional, and nothing is sent anywhere else.",
        "> Early software, not independently audited. Keep a separate backup of anything you cannot lose.",
        "## Install",
        "**Linux**, one command. It detects your system and installs the signed package for it, or a checked download:",
        f"```sh\ncurl -fsS {site}/install.sh | sh\n```",
        f"Prefer to read it first? It is plain shell: {site}/download/install-script/. "
        "Add `--nightly` for the newest build of `main`, `--user` for an install in your home folder without root.",
        f"**Android**: download the `.apk` below and open it on the phone (allow installs from your file manager), or run "
        f"`curl -fsS {site}/install.sh | sh -s -- --apk` on a computer or in Termux.",
    ]
    if table:
        parts += ["## Downloads", table,
                  "Check a download: `sha256sum -c SHA256SUMS --ignore-missing` (the file `SHA256SUMS` is attached too). "
                  "The installer does this check for you."]
    parts += [
        "## Update and remove",
        "```sh\n"
        f"curl -fsS {site}/install.sh | sh -s -- --check       # is there a newer version?\n"
        f"curl -fsS {site}/install.sh | sh -s -- --update      # update, the way it was installed\n"
        f"curl -fsS {site}/install.sh | sh -s -- --uninstall   # remove it; your vaults are kept\n"
        "```",
        "Installs from the package repository also update with `apt upgrade`, `dnf upgrade`, `zypper update` or `pacman -Syu`. "
        "Files installed without a package manager get a `hexis-update` command next to the program.",
        "## What is new",
        new,
        "## More",
        f"* Website and guides: {site}/\n"
        f"* Privacy: nothing is collected; see [PRIVACY.md](https://github.com/{repo}/blob/v{version}/PRIVACY.md)\n"
        f"* Security reports: [SECURITY.md](https://github.com/{repo}/blob/v{version}/SECURITY.md)",
    ]
    return "\n\n".join(parts) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--version", required=True)
    ap.add_argument("--repo", required=True, help="owner/name")
    ap.add_argument("--site", required=True)
    ap.add_argument("--sums", help="a SHA256SUMS file of the attachments")
    ap.add_argument("--changelog", default=str(pathlib.Path(__file__).resolve().parent.parent / "CHANGELOG.md"))
    a = ap.parse_args()
    version = a.version.lstrip("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?", version):
        print(f"not a version: {a.version}", file=sys.stderr)
        return 2
    sums = pathlib.Path(a.sums).read_text() if a.sums else ""
    changelog = pathlib.Path(a.changelog).read_text() if pathlib.Path(a.changelog).exists() else ""
    sys.stdout.write(build(version, a.repo, a.site, sums, changelog))
    return 0


if __name__ == "__main__":
    sys.exit(main())
