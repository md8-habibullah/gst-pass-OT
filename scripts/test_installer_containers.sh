#!/usr/bin/env bash
# Run the real installer inside real distribution containers, against the real release files.
#
#   scripts/test_installer_containers.sh FILES_DIR IMAGE[=EXPECT] ...
#
# FILES_DIR holds the files one build produced (deb, rpm, tar.gz, AppImage, optionally the APK); a SHA256SUMS is
# written if missing. For every IMAGE this script
#   1. builds the signed apt, rpm and (when an archlinux image is listed) pacman repositories from those files with a
#      throw-away key, inside containers, so it needs only docker,
#   2. serves the repositories and the release files on 127.0.0.1 (the containers share the host network),
#   3. runs scripts/tests/container_case.sh in the image: install, check libraries, start the program under Xvfb,
#      install again, uninstall.
# EXPECT is "ok" (default) or "refuse:<text>" for systems the installer must turn away, e.g. alpine:3.20=refuse:musl.
# SMOKE=1 (default) also starts the program; SMOKE=0 skips that.
# Example:  scripts/test_installer_containers.sh release-files ubuntu:22.04 fedora:latest alpine:3.20=refuse:musl
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
FILES=${1:?usage: $0 FILES_DIR IMAGE[=EXPECT]...}; shift
[ $# -gt 0 ] || { echo "no images given" >&2; exit 2; }
FILES=$(cd "$FILES" && pwd)
command -v docker >/dev/null || { echo "docker is needed" >&2; exit 2; }
# two free ports on the loopback address (the containers share the host network)
free_port() { python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'; }
SITE_PORT=${SITE_PORT:-$(free_port)}; FILES_PORT=${FILES_PORT:-$(free_port)}

WORK=$(mktemp -d)
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done; docker run --rm -v "$WORK:/w" ubuntu:22.04 chown -R "$(id -u):$(id -g)" /w >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT

# the release folder the "download" paths read: files + SHA256SUMS
mkdir -p "$WORK/release"
for f in "$FILES"/*.deb "$FILES"/*.rpm "$FILES"/*.tar.gz "$FILES"/*.AppImage "$FILES"/*.apk; do [ -f "$f" ] && cp "$f" "$WORK/release/"; done
(cd "$WORK/release" && sha256sum -- * > SHA256SUMS)
DEB=$(ls "$WORK"/release/Hexis_*_amd64.deb); RPM=$(ls "$WORK"/release/Hexis-*.x86_64.rpm)

echo "== building the signed repositories (throw-away key) inside a container"
docker build -q -t hexis-repotools - >/dev/null <<'EOF'
FROM ubuntu:22.04
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq apt-utils createrepo-c rpm gnupg >/dev/null
EOF
mkdir -p "$WORK/site"
mkdir -p "$WORK/keys"
docker run --rm -v "$ROOT/scripts:/src/scripts:ro" -v "$WORK/release:/in:ro" -v "$WORK/site:/out" -v "$WORK/keys:/keys" hexis-repotools bash -c '
  set -euo pipefail
  export GNUPGHOME=$(mktemp -d); chmod 700 "$GNUPGHOME"
  gpg --batch --passphrase "" --quick-generate-key "Hexis CI Test Key <ci@hexis.invalid>" rsa4096 sign never >/dev/null 2>&1
  export REPO_SIGNING_KEY="$(gpg --batch --armor --export-secret-keys)"; unset GNUPGHOME
  printf "%s\n" "$REPO_SIGNING_KEY" > /keys/secret.asc
  bash /src/scripts/make-repos.sh --out /out --site http://127.0.0.1:'"$SITE_PORT"' --deb /in/'"$(basename "$DEB")"' --rpm /in/'"$(basename "$RPM")"' >/dev/null
  chown -R '"$(id -u):$(id -g)"' /out'
test -s "$WORK/site/apt/dists/stable/InRelease" && echo "   apt and rpm repositories built"
case " $* " in
    *" archlinux"*)
        echo "== building the signed pacman repository in an archlinux container"
        REPO_SIGNING_KEY=$(cat "$WORK/keys/secret.asc") "$ROOT/scripts/make-arch-repo.sh" --out "$WORK/site" --site "http://127.0.0.1:$SITE_PORT" \
            --tarball "$(ls "$WORK"/release/hexis-*-linux-x86_64.tar.gz)" >/dev/null
        test -s "$WORK/site/arch/x86_64/hexis.db" && echo "   pacman repository built" ;;
esac

# started directly (not inside a subshell) so $! is the server itself and the cleanup trap really stops it
python3 -m http.server "$SITE_PORT" --bind 127.0.0.1 --directory "$WORK/site" >/dev/null 2>&1 &
PIDS+=("$!")
python3 -m http.server "$FILES_PORT" --bind 127.0.0.1 --directory "$WORK/release" >/dev/null 2>&1 &
PIDS+=("$!")
ready=0
for _ in $(seq 1 50); do
    if curl -fsS -o /dev/null "http://127.0.0.1:$SITE_PORT/apt/dists/stable/InRelease" 2>/dev/null \
        && curl -fsS -o /dev/null "http://127.0.0.1:$FILES_PORT/SHA256SUMS" 2>/dev/null; then ready=1; break; fi
    sleep 0.2
done
[ "$ready" = 1 ] || { echo "the local test servers did not come up" >&2; exit 1; }
echo "   site on :$SITE_PORT, release files on :$FILES_PORT"

failed=0
for spec in "$@"; do
    image=${spec%%=*}; expect=ok
    case $spec in *=*) expect=${spec#*=} ;; esac
    echo
    echo "================ $image (expect: $expect)"
    if docker run --rm --network host \
        -e EXPECT="$expect" -e SMOKE="${SMOKE:-1}" \
        -e HEXIS_REPO=owner/project -e HEXIS_SITE="http://127.0.0.1:$SITE_PORT" -e HEXIS_BASE_URL="http://127.0.0.1:$FILES_PORT" \
        -v "$ROOT/scripts:/src/scripts:ro" "$image" sh /src/scripts/tests/container_case.sh; then
        echo "---- $image: ok"
    else
        echo "---- $image: FAILED"; failed=$((failed + 1))
    fi
done
echo
if [ "$failed" -gt 0 ]; then echo "$failed image(s) failed"; exit 1; fi
echo "all images passed"
