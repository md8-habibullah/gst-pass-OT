#!/usr/bin/env python3
"""Read-only live monitor for the Android toolchain install (stdlib only, no network).

  python3 scripts/monitor_android_setup.py          live view, q to quit
  python3 scripts/monitor_android_setup.py --once   one frame and exit

It reads: the install log ($PASSMAN_ANDROID_LOG, default ~/.cache/passman-android/install.log), the size of
~/Android/Sdk (what the job is writing), and whether the installer processes are running. It never writes anything.
"""
import os, sys, time, subprocess, select, termios, tty

SDK = os.environ.get("ANDROID_HOME", os.path.expanduser("~/Android/Sdk"))
LOG = os.environ.get("PASSMAN_ANDROID_LOG", os.path.expanduser("~/.cache/passman-android/install.log"))
WANT = ["platform-tools", "platforms/android-36", "build-tools/36.0.0", "ndk/28.2.13676358"]


def du(path):
    total = 0
    for root, _dirs, files in os.walk(path):
        for f in files:
            try:
                total += os.lstat(os.path.join(root, f)).st_size
            except OSError:
                pass
    return total


def running():
    try:
        out = subprocess.run(["pgrep", "-af", "android.*sdk|sdkmanager|setup_android_toolchain"], capture_output=True, text=True).stdout
    except OSError:
        return False
    me = str(os.getpid())
    return any(l.strip() and not l.startswith(me + " ") and "monitor_android_setup" not in l and "pgrep" not in l for l in out.splitlines())


def tail(path, n=8):
    try:
        with open(path, "rb") as f:
            f.seek(0, 2); size = f.tell(); f.seek(max(0, size - 6000))
            return f.read().decode("utf8", "replace").replace("\r", "\n").splitlines()[-n:]
    except OSError:
        return ["(no log yet)"]


def frame(prev):
    now = time.time(); size = du(SDK)
    rate = (size - prev[1]) / (now - prev[0]) if prev else 0.0
    lines = ["Passman Android toolchain install   (reads only; q to quit)", ""]
    lines.append(f"job        : {'RUNNING' if running() else 'idle / finished'}")
    lines.append(f"writing to : {SDK}  ({size / 1e6:,.0f} MB, {rate / 1e6:,.1f} MB/s)")
    lines.append(f"log        : {LOG}")
    lines.append("")
    for w in WANT:
        lines.append(f"  [{'x' if os.path.isdir(os.path.join(SDK, w)) else ' '}] {w}")
    lic = os.path.isdir(os.path.join(SDK, "licenses")) and bool(os.listdir(os.path.join(SDK, "licenses")))
    lines += ["", f"licence files in SDK: {'yes' if lic else 'none'}", "", "last log lines:"]
    lines += ["  " + l[:110] for l in tail(LOG)]
    return "\n".join(lines), (now, size)


def main():
    if "--once" in sys.argv:
        print(frame(None)[0]); return
    fd = sys.stdin.fileno(); old = termios.tcgetattr(fd); prev = None
    try:
        tty.setcbreak(fd)
        while True:
            text, prev = frame(prev)
            sys.stdout.write("\x1b[2J\x1b[H" + text + "\n"); sys.stdout.flush()
            if select.select([sys.stdin], [], [], 2.0)[0] and sys.stdin.read(1).lower() == "q":
                break
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old)


if __name__ == "__main__":
    main()
