#!/usr/bin/env python3
"""Read-only live monitor for long-running dev jobs (rustup / cargo / npm / vite ...).

Stdlib only, no network, reads /proc and one log file. Never writes anything.

    python3 scripts/monitor.py --log /path/to/job.log [--total N | --lock Cargo.lock] [--interval 1.0]
    python3 scripts/monitor.py --log /path/to/job.log --once     # single frame, no TTY needed

Keys: q = quit.
Shows: job running/idle, progress bar (counts cargo "Compiling" lines when --total is
given), CPU/RSS of build processes, free RAM + swap, network and disk rates, log tail.
"""
import argparse
import os
import re
import select
import shutil
import sys
import time

JOB_RE = re.compile(
    r"^(rustup|rustup-init|cargo|rustc|cc1|cc1plus|collect2|ld|ld\.lld|lld|clang|gcc|"
    r"npm|node|pnpm|vite|tsc|esbuild)$"
)
DISK_RE = re.compile(r"^(nvme\d+n\d+|sd[a-z]+|vd[a-z]+|mmcblk\d+)$")
CLK_TCK = os.sysconf("SC_CLK_TCK")
SELF = os.getpid()


def read(path):
    try:
        with open(path, "r", errors="replace") as fh:
            return fh.read()
    except OSError:
        return ""


def meminfo():
    out = {}
    for line in read("/proc/meminfo").splitlines():
        k, _, v = line.partition(":")
        out[k] = int(v.split()[0]) * 1024 if v.split() else 0
    return out


def scan_jobs():
    jobs = {}
    for pid in os.listdir("/proc"):
        if not pid.isdigit() or int(pid) == SELF:
            continue
        stat = read(f"/proc/{pid}/stat")
        if not stat:
            continue
        rp = stat.rfind(")")
        comm = stat[stat.find("(") + 1 : rp]
        if not JOB_RE.match(comm):
            continue
        if comm.startswith("python") and "monitor.py" in read(f"/proc/{pid}/cmdline"):
            continue
        f = stat[rp + 2 :].split()
        ticks = int(f[11]) + int(f[12])  # utime + stime
        rss = 0
        for line in read(f"/proc/{pid}/status").splitlines():
            if line.startswith("VmRSS:"):
                rss = int(line.split()[1]) * 1024
        jobs[int(pid)] = (comm, ticks, rss)
    return jobs


def net_bytes():
    rx = tx = 0
    for line in read("/proc/net/dev").splitlines()[2:]:
        name, _, rest = line.partition(":")
        if name.strip() == "lo":
            continue
        cols = rest.split()
        rx += int(cols[0])
        tx += int(cols[8])
    return rx, tx


def disk_bytes():
    rd = wr = 0
    for line in read("/proc/diskstats").splitlines():
        c = line.split()
        if len(c) > 9 and DISK_RE.match(c[2]):
            rd += int(c[5]) * 512
            wr += int(c[9]) * 512
    return rd, wr


def human(n):
    n = float(n)
    for unit in ("B", "KiB", "MiB", "GiB"):
        if abs(n) < 1024 or unit == "GiB":
            return f"{n:6.1f} {unit}"
        n /= 1024


def tail(path, lines):
    try:
        with open(path, "rb") as fh:
            fh.seek(0, os.SEEK_END)
            size = fh.tell()
            fh.seek(max(0, size - 2_000_000))
            text = fh.read().decode("utf-8", "replace")
    except OSError:
        return [], 0, None, 0
    all_lines = text.splitlines()
    compiled = sum(1 for ln in all_lines if re.match(r"^\s*Compiling ", ln))
    try:
        mtime = os.stat(path).st_mtime
    except OSError:
        mtime = None
    return all_lines[-lines:], compiled, mtime, len(all_lines)


def bar(frac, width):
    frac = max(0.0, min(1.0, frac))
    filled = int(frac * width)
    return "[" + "#" * filled + "-" * (width - filled) + f"] {frac * 100:5.1f}%"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--log", required=True, help="log file the job writes to")
    ap.add_argument("--total", type=int, default=0, help="expected number of crates to compile")
    ap.add_argument("--lock", help="Cargo.lock to derive --total from (re-read every frame)")
    ap.add_argument("--interval", type=float, default=1.0)
    ap.add_argument("--once", action="store_true", help="print one frame and exit")
    args = ap.parse_args()

    interactive = sys.stdin.isatty() and sys.stdout.isatty() and not args.once
    old_term = None
    if interactive:
        import termios
        import tty

        old_term = termios.tcgetattr(sys.stdin)
        tty.setcbreak(sys.stdin.fileno())

    started = time.time()
    prev_t = prev_jobs = prev_net = prev_disk = None
    first = True
    try:
        while True:
            now = time.time()
            jobs = scan_jobs()
            net = net_bytes()
            disk = disk_bytes()
            dt = (now - prev_t) if prev_t else 0
            cpu = {}
            if prev_jobs and dt > 0:
                for pid, (comm, ticks, _) in jobs.items():
                    if pid in prev_jobs:
                        cpu[pid] = (ticks - prev_jobs[pid][1]) / CLK_TCK / dt * 100
            net_rate = ((net[0] - prev_net[0]) / dt, (net[1] - prev_net[1]) / dt) if prev_net and dt else (0, 0)
            disk_rate = ((disk[0] - prev_disk[0]) / dt, (disk[1] - prev_disk[1]) / dt) if prev_disk and dt else (0, 0)
            prev_t, prev_jobs, prev_net, prev_disk = now, jobs, net, disk

            cols, rows = shutil.get_terminal_size((100, 30))
            mem = meminfo()
            log_lines, compiled, mtime, nlines = tail(args.log, max(5, rows - 17))
            total = args.total
            if args.lock:
                n = sum(1 for ln in read(args.lock).splitlines() if ln.startswith("name = "))
                total = max(total, n - 1)
            state = "RUNNING" if jobs else "IDLE (no build/install process found)"
            out = []
            out.append(f"passman job monitor   {time.strftime('%H:%M:%S')}   up {int(now - started)}s   (q = quit)")
            out.append("-" * min(cols, 100))
            out.append(f"state : {state}")
            if total:
                elapsed = now - started
                eta = f"ETA ~{int(elapsed / compiled * (total - compiled))}s" if compiled and compiled < total else ""
                out.append(f"crates: {bar(compiled / total, max(10, min(cols, 100) - 42))}  {compiled}/{total} {eta}")
            else:
                out.append(f"crates: {compiled} compiled so far (pass --total N for a bar)")
            out.append(
                f"RAM   : avail {human(mem.get('MemAvailable', 0))} / total {human(mem.get('MemTotal', 0))}"
                f"   swap used {human(mem.get('SwapTotal', 0) - mem.get('SwapFree', 0))}"
            )
            out.append(f"net   : down {human(net_rate[0])}/s   up {human(net_rate[1])}/s")
            out.append(f"disk  : read {human(disk_rate[0])}/s   write {human(disk_rate[1])}/s   (whole-disk, all processes)")
            age = f"{int(now - mtime)}s ago" if mtime else "not created yet"
            out.append(f"log   : {args.log}   {nlines} lines, last write {age}")
            out.append("procs :")
            top = sorted(jobs.items(), key=lambda kv: -cpu.get(kv[0], 0))[:6]
            if not top:
                out.append("   (none)")
            for pid, (comm, _, rss) in top:
                out.append(f"   {pid:>7} {comm:<12} cpu {cpu.get(pid, 0):6.1f}%   rss {human(rss)}")
            out.append("-" * min(cols, 100))
            out.extend(ln[: cols - 1] for ln in log_lines)
            frame = "\n".join(out)

            if interactive:
                sys.stdout.write("\x1b[H\x1b[2J" + frame + "\n")
                sys.stdout.flush()
            else:
                print(frame)
            if args.once:
                return
            if first:
                first = False
            if interactive:
                r, _, _ = select.select([sys.stdin], [], [], args.interval)
                if r and sys.stdin.read(1).lower() == "q":
                    return
            else:
                time.sleep(args.interval)
    except KeyboardInterrupt:
        pass
    finally:
        if old_term is not None:
            import termios

            termios.tcsetattr(sys.stdin, termios.TCSADRAIN, old_term)


if __name__ == "__main__":
    main()
