#!/usr/bin/env python3
"""Read-only live monitor for the Android toolchain install (stdlib only, no network).

  python3 scripts/monitor_android_setup.py                          live view of the SDK install, q to quit
  python3 scripts/monitor_android_setup.py --once                   one frame and exit
  python3 scripts/monitor_android_setup.py --log build.log --dir src-tauri/gen/android   watch a build instead

It reads: a log file (--log, or $PASSMAN_ANDROID_LOG, default ~/.cache/passman-android/install.log), the size of
a folder (--dir, default ~/Android/Sdk: what the job is writing), the memory in use, and whether the installer or
build processes (android-cli, sdkmanager, gradle, cargo, rustc, tauri) are running. It never writes anything.
"""
import os, sys, time, subprocess, select, termios, tty

def arg(name, default):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv and sys.argv.index(name) + 1 < len(sys.argv) else default


SDK = os.path.expanduser(arg("--dir", os.environ.get("ANDROID_HOME", "~/Android/Sdk")))
LOG = os.path.expanduser(arg("--log", os.environ.get("PASSMAN_ANDROID_LOG", "~/.cache/passman-android/install.log")))
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
        out = subprocess.run(["pgrep", "-af", "android-cli|sdkmanager|setup_android_toolchain|gradle|cargo|rustc|tauri"], capture_output=True, text=True).stdout
    except OSError:
        return False
    me = str(os.getpid())
    return any(l.strip() and not l.startswith(me + " ") and "monitor_android_setup" not in l and "pgrep" not in l for l in out.splitlines())


def mem_available():
    try:
        for l in open("/proc/meminfo"):
            if l.startswith("MemAvailable"):
                return f"{int(l.split()[1]) / 1048576:.1f} GiB"
    except OSError:
        pass
    return "?"


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
    lines.append(f"job        : {'RUNNING' if running() else 'idle / finished'}      memory available: {mem_available()}")
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
