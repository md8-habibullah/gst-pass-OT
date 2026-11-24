#!/bin/sh
# One installer scenario, run INSIDE a distribution container by scripts/test_installer_containers.sh.
# POSIX sh (it must run under dash and busybox ash too). Environment from the driver:
#   EXPECT  ok | refuse:<text the refusal must contain>
#   SMOKE   1 = also start the program for a few seconds under Xvfb
#   HEXIS_SITE, HEXIS_BASE_URL, HEXIS_REPO  where the fake project site and release files are served
set -u
fail() { printf 'FAIL: %s\n' "$*"; exit 1; }
ok() { printf 'PASS: %s\n' "$*"; }

# ---- what this container is, printed before anything is installed (public facts only: no environment, no secrets)
printf 'container: %s, %s, kernel %s\n' "$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release 2>/dev/null | tr -d '"')" "$(uname -m)" "$(uname -r)"
printf '           /bin/sh is %s; %s cpus; memory %s; limit of this container %s; disk free on / %s\n' \
    "$(readlink -f /bin/sh 2>/dev/null || echo unknown)" "$(nproc 2>/dev/null || echo '?')" \
    "$(awk '/MemTotal/{t=$2} /MemAvailable/{a=$2} END{printf "%.1f GiB total, %.1f GiB available", t/1048576, a/1048576}' /proc/meminfo 2>/dev/null)" \
    "$(cat /sys/fs/cgroup/memory.max 2>/dev/null || cat /sys/fs/cgroup/memory/memory.limit_in_bytes 2>/dev/null || echo unknown)" \
    "$(df -h / 2>/dev/null | awk 'NR==2{print $4}')"

# ---- what a minimal image lacks: curl, and (for the smoke test) a virtual display
if [ -r /etc/os-release ]; then ID=$(sed -n 's/^ID=//p' /etc/os-release | tr -d '"'); else ID=unknown; fi
quiet() { "$@" >/dev/null 2>&1; }
case $ID in
    debian|ubuntu)
        quiet apt-get update; export DEBIAN_FRONTEND=noninteractive
        quiet apt-get install -y curl ca-certificates
        [ "${SMOKE:-0}" = 1 ] && quiet apt-get install -y xvfb dbus-x11 ;;
    fedora) command -v curl >/dev/null || quiet dnf install -y curl; [ "${SMOKE:-0}" = 1 ] && quiet dnf install -y xorg-x11-server-Xvfb dbus-daemon dbus-x11 ;;
    opensuse*|sles) command -v curl >/dev/null || quiet zypper --non-interactive install curl
        [ "${SMOKE:-0}" = 1 ] && quiet zypper --non-interactive install xorg-x11-server-Xvfb dbus-1-daemon dbus-1-x11 ;;
    arch) quiet pacman-key --init # a fresh container has no local signing key; a real Arch install does
        quiet pacman -Sy --noconfirm --needed curl; [ "${SMOKE:-0}" = 1 ] && quiet pacman -S --noconfirm --needed xorg-server-xvfb xorg-xauth dbus ;;
    alpine) quiet apk add --no-cache curl ;;
esac

run() { sh /src/scripts/install.sh "$@" >/tmp/install.log 2>&1; echo $? >/tmp/install.rc; cat /tmp/install.log; return 0; }

case ${EXPECT:-ok} in
    refuse:*)
        want=${EXPECT#refuse:}
        run
        [ "$(cat /tmp/install.rc)" != 0 ] || fail "the installer should have refused this system"
        grep -q "$want" /tmp/install.log || fail "refusal does not mention '$want'"
        ok "refused with: $want"
        exit 0 ;;
esac

# ---- a normal install
run
[ "$(cat /tmp/install.rc)" = 0 ] || fail "install exited with $(cat /tmp/install.rc)"
bin=$(command -v hexis || true)
if [ -z "$bin" ] && [ -x /usr/local/bin/hexis ]; then bin=/usr/local/bin/hexis; fi
if [ -z "$bin" ] || [ ! -x "$bin" ]; then fail "no hexis program after the install"; fi
ok "installed at $bin (plan: $(sed -n 's/^Plan: \([a-z]*\).*/\1/p' /tmp/install.log))"

missing=$(ldd "$bin" 2>&1 | grep -c 'not found')
[ "$missing" = 0 ] || { ldd "$bin" | grep 'not found'; fail "$missing shared libraries are missing"; }
ok "every shared library resolves"

if [ "${SMOKE:-0}" = 1 ] && command -v xvfb-run >/dev/null; then
    export WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
    timeout 12 xvfb-run -a dbus-run-session -- "$bin" >/tmp/run.log 2>&1
    rc=$?
    [ "$rc" = 124 ] || { tail -n 15 /tmp/run.log; fail "the program did not stay running for 12 s (exit $rc)"; }
    ok "the program started and kept running for 12 s under Xvfb"
fi

run
[ "$(cat /tmp/install.rc)" = 0 ] || fail "running the installer a second time failed"
ok "a second run succeeds (upgrade in place)"

# what the app keeps for this account (the smoke run above may have made some of it already)
data="${HOME:-/root}/.local/share/dev.habibullah.hexis"; cache="${HOME:-/root}/.cache/dev.habibullah.hexis"
mkdir -p "$data/localstorage" "$cache"; echo sealed >"$data/vault.db"; echo cached >"$cache/blob"

# no terminal and no --yes: the uninstall must refuse and change nothing (it would delete the vault)
export HEXIS_TTY=/nonexistent/terminal
run --uninstall
unset HEXIS_TTY
[ "$(cat /tmp/install.rc)" != 0 ] || fail "uninstall removed data without anyone confirming"
[ -f "$data/vault.db" ] || fail "a refused uninstall still touched the data"
# (a subshell: dash remembers where `command -v` found a program, and would then still "find" it after it was removed)
if ! ( command -v hexis >/dev/null 2>&1 ) && [ ! -e /usr/local/bin/hexis ]; then fail "a refused uninstall still removed the program"; fi
ok "uninstall without a terminal or --yes refuses and changes nothing"

run --uninstall --yes
[ "$(cat /tmp/install.rc)" = 0 ] || fail "uninstall exited with $(cat /tmp/install.rc)"
hash -r 2>/dev/null || true
if command -v hexis >/dev/null 2>&1 || [ -e /usr/local/bin/hexis ]; then fail "hexis is still there after the uninstall"; fi
ok "uninstalled; nothing left on the PATH"
if [ -e "$data" ] || [ -e "$cache" ]; then fail "Hexis's data or cache folder is still there after the uninstall"; fi
grep -q "Nothing is kept" /tmp/install.log || fail "the uninstall did not say that nothing is kept"
ok "its data and cache folders are gone, and the uninstall said so"
