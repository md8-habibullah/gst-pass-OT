#!/usr/bin/env python3
"""Build and test src-tauri/src/crypto WITHOUT Tauri or any GUI system library.

The crypto module has no GUI dependency, but the `hexis` crate as a whole needs
webkit2gtk-4.1 to compile. This script generates a bare crate that `#[path]`-includes the very
same source files, with a Cargo.toml derived from src-tauri/Cargo.toml (tauri entries dropped),
so dependency versions cannot drift.

    python3 scripts/crypto-harness.py                 # cargo test --lib
    python3 scripts/crypto-harness.py clippy          # clippy, warnings are errors
    python3 scripts/crypto-harness.py check
    python3 scripts/crypto-harness.py test -- --nocapture kem::   # extra args go to cargo/test

Environment: HEXIS_HARNESS_DIR (default ~/.cache/hexis-crypto-harness), CARGO_BUILD_JOBS (default 2).
Note: the harness has its own Cargo.lock; src-tauri/Cargo.lock (once it exists) may pin other patch versions.
"""
import os
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
MANIFEST = REPO / "src-tauri" / "Cargo.toml"
CRYPTO_MOD = REPO / "src-tauri" / "src" / "crypto" / "mod.rs"
HARNESS = pathlib.Path(os.environ.get("HEXIS_HARNESS_DIR", pathlib.Path.home() / ".cache" / "hexis-crypto-harness"))


def derive_manifest() -> str:
    keep, section = [], None
    for line in MANIFEST.read_text().splitlines():
        header = re.match(r"^\[(.+)\]\s*$", line)
        if header:
            section = header.group(1)
        if section == "dependencies" or (section or "").startswith("profile."):
            # GUI crates, plus the Phase 4a network stack (rusty-s3, ureq, url): the crypto module
            # needs none of them, and rustls/ring would only slow this build down. The mobile-only
            # barcode scanner sits in a [target.*] section, which this filter never keeps.
            if re.match(r"^(tauri|tauri-build|tauri-plugin-[a-z0-9-]+|rusty-s3|ureq|url)\s*=", line):
                continue
            keep.append(line)
    head = (
        '[package]\nname = "hexis-crypto-harness"\nversion = "0.0.0"\nedition = "2021"\npublish = false\n\n'
        '[lib]\npath = "src/lib.rs"\n\n'
    )
    return head + "\n".join(keep).rstrip() + "\n"


def write_if_changed(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists() or path.read_text() != text:
        path.write_text(text)


def main() -> int:
    args = sys.argv[1:]
    command = args.pop(0) if args and args[0] in {"test", "clippy", "check"} else "test"
    write_if_changed(HARNESS / "Cargo.toml", derive_manifest())
    write_if_changed(HARNESS / "src" / "lib.rs", f'#[path = "{CRYPTO_MOD}"]\npub mod crypto;\n')

    env = dict(os.environ)
    env.setdefault("CARGO_BUILD_JOBS", "2")
    env.setdefault("CARGO_PROFILE_DEV_DEBUG", "0")
    env.setdefault("CARGO_INCREMENTAL", "0")
    if command == "test":
        cmd = ["cargo", "test", "--lib"] + (["--"] + args[1:] if args[:1] == ["--"] else args)
    elif command == "clippy":
        cmd = ["cargo", "clippy", "--all-targets"] + args + ["--", "-D", "warnings"]
    else:
        cmd = ["cargo", "check", "--all-targets"] + args
    print(f"[harness] {HARNESS}\n[harness] $ {' '.join(cmd)}", flush=True)
    return subprocess.call(cmd, cwd=HARNESS, env=env)


if __name__ == "__main__":
    sys.exit(main())
