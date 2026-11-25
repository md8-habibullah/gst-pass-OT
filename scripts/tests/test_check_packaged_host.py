"""Tests of scripts/check_packaged_host.py with stand-in hosts, so the check itself cannot pass by accident.

The stand-ins speak the browser's framing and the private socket the way `hexis --native-host` does; the broken ones do what a
bad package would (print a line first, never end, answer something else, exit with an error). The check must pass the first
and fail every one of the others.

Run from the repository root:  python3 -m unittest discover -s scripts/tests -t .
"""
import importlib.util
import os
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "check_packaged_host.py"
_spec = importlib.util.spec_from_file_location("check_packaged_host", SCRIPT)
cph = importlib.util.module_from_spec(_spec)
sys.modules["check_packaged_host"] = cph
_spec.loader.exec_module(cph)

# A stand-in for `hexis --native-host`. MODE picks how it misbehaves.
HOST = textwrap.dedent(
    '''\
    #!{python}
    import json, os, socket, stat, struct, sys, time

    MODE = {mode!r}
    if MODE == "hang":
        time.sleep(600)
    (size,) = struct.unpack("=I", sys.stdin.buffer.read(4))
    message = sys.stdin.buffer.read(size).decode()
    request_id = json.loads(message)["id"]

    def say(obj):
        body = json.dumps(obj, separators=(",", ":")).encode()
        sys.stdout.buffer.write(struct.pack("=I", len(body)) + body)
        sys.stdout.buffer.flush()

    if MODE == "noisy":
        sys.stdout.write("GTK-Message: something an AppRun hook printed\\n")
        sys.stdout.flush()
    if MODE == "badexit":
        say({{"v": 1, "id": request_id, "ok": False, "code": "fallback", "reason": "not_running"}})
        sys.exit(3)
    if MODE == "wrong":
        say({{"v": 1, "id": request_id, "ok": True, "enabled": False}})
        sys.exit(0)

    path = os.path.join(os.environ["XDG_RUNTIME_DIR"], "hexis", "bridge.sock")
    try:
        folder = os.stat(os.path.dirname(path))
        if stat.S_IMODE(folder.st_mode) != 0o700 and MODE != "trusting":
            raise OSError("the folder is not private")
        s = socket.socket(socket.AF_UNIX)
        s.connect(path)
        s.sendall(message.encode() + b"\\n")
        line = b""
        while not line.endswith(b"\\n"):
            chunk = s.recv(4096)
            if not chunk:
                break
            line += chunk
        say(json.loads(line))
    except OSError:
        say({{"v": 1, "id": request_id, "ok": False, "code": "fallback", "reason": "not_running"}})
    sys.exit(0)
    '''
)


def make_host(directory: Path, mode: str, name: str = "hexis") -> Path:
    path = directory / name
    path.write_text(HOST.format(python=sys.executable, mode=mode))
    path.chmod(0o755)
    return path


def run_check(program: Path, *flags: str, timeout: int = 60) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), str(program), "--timeout", "2", *flags],
        capture_output=True, text=True, timeout=timeout,
    )


class PackagedHostCheck(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def test_a_good_host_passes_directly_and_through_the_launcher(self):
        host = make_host(self.dir, "good")
        for flags in ((), ("--via-launcher",)):
            done = run_check(host, *flags)
            self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
            self.assertIn("all checks passed", done.stdout)

    def test_a_program_path_with_a_space_and_a_quote_works_through_the_launcher(self):
        folder = self.dir / "Hexis app" / "it's here"
        folder.mkdir(parents=True)
        done = run_check(make_host(folder, "good", "Hexis_0.5.0_amd64.AppImage"), "--via-launcher")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_a_line_printed_before_the_answer_fails_it(self):
        done = run_check(make_host(self.dir, "noisy"))
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("anything beyond one framed message", done.stdout + done.stderr)

    def test_a_host_that_never_ends_fails_it(self):
        done = run_check(make_host(self.dir, "hang"))
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("FAIL", done.stdout)

    def test_a_host_that_exits_with_an_error_fails_it(self):
        done = run_check(make_host(self.dir, "badexit"))
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("exit 3", done.stdout)

    def test_a_host_that_answers_something_else_fails_it(self):
        done = run_check(make_host(self.dir, "wrong"))
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("the answer is the expected one", done.stdout)

    def test_a_host_that_trusts_a_socket_in_a_folder_others_can_enter_fails_it(self):
        done = run_check(make_host(self.dir, "trusting"))
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("nothing was sent to that socket", done.stdout)

    def test_something_that_is_not_a_program_is_refused_before_anything_runs(self):
        plain = self.dir / "notes.txt"
        plain.write_text("hello")
        done = run_check(plain)
        self.assertEqual(done.returncode, 2)

    def test_the_launcher_text_is_what_the_app_writes(self):
        # the same line as src-tauri/src/bridge/install.rs `launcher_text`; the Rust test pins the quoting of the app's side
        self.assertEqual(
            cph.launcher_text("/opt/Hexis app/it's.AppImage"),
            "#!/bin/sh\nexec '/opt/Hexis app/it'\\''s.AppImage' --native-host \"$@\"\n",
        )
        rust = (Path(__file__).resolve().parents[2] / "src-tauri/src/bridge/install.rs").read_text()
        self.assertIn('"#!/bin/sh\\nexec {} --native-host \\"$@\\"\\n"', rust)

    def test_the_frame_is_four_bytes_in_the_machines_order_then_the_json(self):
        framed = cph.frame({"a": 1})
        self.assertEqual(framed[4:], b'{"a":1}')
        self.assertEqual(int.from_bytes(framed[:4], sys.byteorder), 7)


if __name__ == "__main__":
    unittest.main()
