#!/usr/bin/env python3
"""Run a BUILT Hexis (a deb's /usr/bin/hexis, an AppImage, the tar.gz binary) as the browser's native messaging host.

The Rust tests and the e2e scripts run the host from a debug build. This script is the one that runs what is shipped:

  1. `hexis --native-host` with no Hexis running must answer one framed message `fallback: not_running` and exit.
  2. With a stand-in Hexis listening on the private socket (folder 0700, socket 0600, as the app makes them), the host must
     hand the browser's message over unchanged and write back exactly what Hexis answered.
  3. With --via-launcher the same two runs go through a launcher written the way `bridge::install::launcher_text` writes
     it (`#!/bin/sh` + `exec '<program>' --native-host "$@"`), started the way a browser starts a host (the extension's
     address as its one argument, nothing else in the environment that Hexis set), which is the path an AppImage depends on.

Native messaging is strict: standard output may hold nothing but length-prefixed messages. One stray line printed by a
wrapper (an AppImage's AppRun, a library) breaks every request, so the script fails on any byte beyond the one frame.

Usage:
  scripts/check_packaged_host.py PROGRAM [--via-launcher] [--extract-and-run] [--timeout SECONDS]

  --extract-and-run  set APPIMAGE_EXTRACT_AND_RUN=1 (for an AppImage on a machine without FUSE, such as a CI runner)

Exit 0 when every check passes, 1 otherwise. Nothing is installed and nothing outside a private temporary folder is touched.
"""
from __future__ import annotations

import argparse
import json
import os
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

REQUEST_ID = "packaged01"
STATUS = {"v": 1, "id": REQUEST_ID, "op": "status"}
EXTENSION_ARG = "chrome-extension://pejbejenceojhjhcanlfnibgdihclnhm/"


def frame(message: dict) -> bytes:
    body = json.dumps(message, separators=(",", ":")).encode()
    return struct.pack("=I", len(body)) + body  # the machine's byte order, as the browsers write it


def launcher_text(program: str) -> str:
    """Byte for byte what `bridge::install::launcher_text` writes."""
    return "#!/bin/sh\nexec '{}' --native-host \"$@\"\n".format(program.replace("'", "'\\''"))


class Reading:
    """What one run of the host produced."""

    def __init__(self, code: int | None, out: bytes, err: bytes, seconds: float):
        self.code, self.out, self.err, self.seconds = code, out, err, seconds
        self.answer: dict | None = None
        self.problem: str | None = None
        if len(out) < 4:
            self.problem = f"standard output holds {len(out)} byte(s), not a framed message"
            return
        (size,) = struct.unpack("=I", out[:4])
        if len(out) != 4 + size:
            self.problem = f"standard output holds {len(out)} bytes but the frame says {4 + size}: anything beyond one framed message breaks the browser"
            return
        try:
            self.answer = json.loads(out[4:])
        except ValueError:
            self.problem = "the framed message is not JSON"


def run_host(argv: list[str], runtime_dir: Path, timeout: float, extra_env: dict[str, str]) -> Reading:
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(runtime_dir),
        "XDG_RUNTIME_DIR": str(runtime_dir),
        "XDG_DATA_HOME": str(runtime_dir / "data"),
        "XDG_CONFIG_HOME": str(runtime_dir / "config"),
        "XDG_CACHE_HOME": str(runtime_dir / "cache"),
        **extra_env,
    }
    # A real browser passes its own display; a host must not need it, so keep it only when the caller has one.
    for name in ("DISPLAY", "WAYLAND_DISPLAY"):
        if name in os.environ:
            env[name] = os.environ[name]
    started = time.monotonic()
    proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, cwd=runtime_dir)
    assert proc.stdin is not None
    try:
        proc.stdin.write(frame(STATUS))
        proc.stdin.flush()  # stdin stays open: a browser keeps the pipe until the page is done
        out = b""
        # The host answers once and ends; read until it does, or time out.
        deadline = started + timeout
        while time.monotonic() < deadline and proc.poll() is None:
            time.sleep(0.05)
        code = proc.poll()
        if code is None:
            proc.kill()
        out = proc.stdout.read() if proc.stdout else b""
        err = proc.stderr.read() if proc.stderr else b""
        proc.wait(timeout=5)
        return Reading(code, out, err, time.monotonic() - started)
    finally:
        for stream in (proc.stdin, proc.stdout, proc.stderr):
            try:
                if stream:
                    stream.close()
            except OSError:
                pass


class StandInHexis:
    """What the app's socket looks like to the host: one JSON line in, one JSON line out, the folder 0700 and the socket 0600."""

    def __init__(self, runtime_dir: Path):
        folder = runtime_dir / "hexis"
        folder.mkdir(mode=0o700)
        os.chmod(folder, 0o700)
        self.path = folder / "bridge.sock"
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(str(self.path))
        os.chmod(self.path, 0o600)
        self.server.listen(4)
        self.server.settimeout(0.2)
        self.received: list[str] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()

    def _serve(self) -> None:
        while not self._stop.is_set():
            try:
                conn, _ = self.server.accept()
            except (socket.timeout, OSError):
                continue
            with conn:
                conn.settimeout(5)
                data = b""
                try:
                    while not data.endswith(b"\n"):
                        chunk = conn.recv(4096)
                        if not chunk:
                            break
                        data += chunk
                except OSError:
                    continue
                line = data.decode(errors="replace").strip()
                self.received.append(line)
                try:
                    request_id = json.loads(line).get("id", "")
                except ValueError:
                    request_id = ""
                conn.sendall((json.dumps({"v": 1, "id": request_id, "ok": True, "enabled": True}) + "\n").encode())

    def close(self) -> None:
        self._stop.set()
        self._thread.join(timeout=2)
        self.server.close()


class Report:
    def __init__(self) -> None:
        self.failed = 0

    def check(self, name: str, ok: bool, detail: str = "") -> None:
        print(f"  {'ok  ' if ok else 'FAIL'} {name}" + (f"\n         {detail}" if detail and not ok else ""))
        if not ok:
            self.failed += 1


def private_dir(parent: str) -> Path:
    path = Path(tempfile.mkdtemp(prefix="hexis-host-", dir=parent))
    os.chmod(path, 0o700)
    return path


def check_run(report: Report, label: str, reading: Reading, expect: dict) -> None:
    report.check(f"{label}: the host ends by itself with exit 0", reading.code == 0, f"exit {reading.code}, stderr: {reading.err[-400:]!r}")
    report.check(f"{label}: standard output is exactly one framed message", reading.problem is None, reading.problem or "")
    if reading.answer is None:
        return
    mismatches = {k: (reading.answer.get(k), v) for k, v in expect.items() if reading.answer.get(k) != v}
    report.check(f"{label}: the answer is the expected one", not mismatches, f"got {reading.answer}, differences (got, wanted): {mismatches}")
    print(f"         answered in {reading.seconds:.2f} s")


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("program", help="the built hexis binary or AppImage")
    ap.add_argument("--via-launcher", action="store_true", help="start it through a launcher script, as a browser does")
    ap.add_argument("--extract-and-run", action="store_true", help="APPIMAGE_EXTRACT_AND_RUN=1, for an AppImage without FUSE")
    ap.add_argument("--timeout", type=float, default=60.0, help="seconds each run may take (an AppImage mounts first)")
    args = ap.parse_args(argv)

    program = os.path.abspath(args.program)
    if not (os.path.isfile(program) and os.access(program, os.X_OK)):
        print(f"{program} is not an executable file", file=sys.stderr)
        return 2
    extra = {"APPIMAGE_EXTRACT_AND_RUN": "1"} if args.extract_and_run else {}
    tmp_parent = tempfile.gettempdir()
    report = Report()

    with tempfile.TemporaryDirectory(prefix="hexis-host-launcher-", dir=tmp_parent) as launcher_dir:
        if args.via_launcher:
            launcher = Path(launcher_dir) / "hexis-native-host"
            launcher.write_text(launcher_text(program))
            os.chmod(launcher, 0o700)
            command = [str(launcher), EXTENSION_ARG]
            how = "through the launcher"
            report.check("the launcher is private and executable", stat.S_IMODE(launcher.stat().st_mode) == 0o700)
        else:
            command = [program, "--native-host", EXTENSION_ARG]
            how = "directly"
        print(f"{program}\n  started {how}")

        # 1. nobody listening
        run1 = private_dir(tmp_parent)
        try:
            reading = run_host(command, run1, args.timeout, extra)
            check_run(report, "no Hexis running", reading, {"v": 1, "id": REQUEST_ID, "ok": False, "code": "fallback", "reason": "not_running"})
        finally:
            subprocess.run(["rm", "-rf", str(run1)], check=False)

        # 2. a stand-in Hexis answers on the private socket
        run2 = private_dir(tmp_parent)
        hexis = StandInHexis(run2)
        try:
            reading = run_host(command, run2, args.timeout, extra)
            check_run(report, "Hexis answers on the socket", reading, {"v": 1, "id": REQUEST_ID, "ok": True, "enabled": True})
            report.check("Hexis received the browser's message unchanged", len(hexis.received) == 1 and json.loads(hexis.received[0]) == STATUS, repr(hexis.received))
        finally:
            hexis.close()
            subprocess.run(["rm", "-rf", str(run2)], check=False)

        # 3. a socket in a folder that is not private must be refused (the host re-checks it every time)
        run3 = private_dir(tmp_parent)
        hexis = StandInHexis(run3)
        os.chmod(run3 / "hexis", 0o755)
        try:
            reading = run_host(command, run3, args.timeout, extra)
            check_run(report, "a socket folder others can enter", reading, {"v": 1, "id": REQUEST_ID, "ok": False, "code": "fallback", "reason": "not_running"})
            report.check("nothing was sent to that socket", hexis.received == [], repr(hexis.received))
        finally:
            hexis.close()
            subprocess.run(["rm", "-rf", str(run3)], check=False)

    print("\n" + ("all checks passed" if report.failed == 0 else f"{report.failed} check(s) FAILED"))
    return 0 if report.failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
