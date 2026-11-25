#!/usr/bin/env bash
# One command that runs every check the project has, in the order that fails fastest.
#   scripts/verify_all.sh            everything except the real-core end-to-end scripts
#   scripts/verify_all.sh --e2e      also run the headless-browser end-to-end scripts (needs chromium, a free port 1431/144x)
#   scripts/verify_all.sh --e2e-only only the end-to-end scripts (CI runs the checks and these as two jobs; see scripts/ci_e2e.sh)
# Heavy parts (cargo) run one after another; the machine this was written on has little RAM.
set -u
cd "$(dirname "$0")/.."
ROOT=$(pwd)
fail=0
step() { printf '\n=== %s\n' "$1"; shift; if "$@"; then printf '    ok\n'; else printf '    FAILED: %s\n' "$*"; fail=1; fi; }

mode="${1:-}"
if [ "$mode" != "--e2e-only" ]; then
step "frontend types"            npx --no-install tsc --noEmit
step "frontend unit tests"       npx --no-install vitest run
step "frontend production build" npx --no-install vite build
step "no dev-only strings in the production bundle" bash -c '! grep -rlE "seedPreview|PREVIEW_PASSWORD|e2e_server|simulateOtherDeviceEdit|simulatePasskeySaved|Marlowe" dist/'
step "no notification or tray code" python3 scripts/check_no_notifications.py
step "one version everywhere (VERSION, package.json, Cargo, tauri.conf, PKGBUILD)" python3 scripts/version.py check
step "shellcheck: installer and repository scripts" bash -c 'if command -v shellcheck >/dev/null 2>&1; then shellcheck -s sh scripts/install.sh scripts/tests/container_case.sh && shellcheck scripts/make-repos.sh scripts/make-arch-repo.sh scripts/test_installer_containers.sh; else echo "(shellcheck is not installed; skipped)"; fi'
# CI runs on ubuntu-22.04, whose shellcheck (0.8.0) flags things a newer local one does not (SC2015 failed a push once). Same version, in a container.
step "shellcheck as CI runs it (ubuntu:22.04, shellcheck 0.8.0)" bash -c 'if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then docker run --rm -v "$PWD":/w:ro -w /w ubuntu:22.04 bash -c "apt-get update -qq >/dev/null 2>&1 && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq shellcheck >/dev/null 2>&1 && shellcheck --version | sed -n 2p && shellcheck -s sh scripts/install.sh scripts/tests/container_case.sh && shellcheck scripts/make-repos.sh scripts/make-arch-repo.sh scripts/test_installer_containers.sh scripts/check_installer_in_containers.sh"; else echo "(Docker is not available; the CI version of shellcheck was not run)"; fi'
step "android patch script tests" bash -c 'if [ -d scripts/tests ]; then python3 -m unittest discover -s scripts/tests -t . ; else echo "(no script tests)"; fi'
step "extension unit tests (node --test)" node --test extension/tests/
step "extension sources agree with what the app writes" python3 scripts/build_extension.py --check
# Not in a subshell: a failed step sets `fail` in this shell, so the final line and the exit status tell the truth.
step "rust format"  bash -c 'cd src-tauri && cargo fmt --check'
step "rust clippy"  bash -c 'cd src-tauri && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo clippy --locked --all-targets -- -D warnings'
step "rust tests"   bash -c 'cd src-tauri && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo test --locked'
fi

if [ "$mode" = "--e2e" ] || [ "$mode" = "--e2e-only" ]; then
  # The end-to-end scripts drive the REAL Rust core (through the dev-only HTTP shim) from a headless Chromium
  # that talks to the Vite dev server. They need: `npm run dev -- --port 1431` running, chromium on the PATH,
  # and E2E_BIN pointing at the shim built with `cargo build --example e2e_server` (target/debug/examples/e2e_server).
  export SP="${SP:-/tmp/hexis-e2e}"; mkdir -p "$SP/shots2"
  export E2E_BIN="${E2E_BIN:-$ROOT/src-tauri/target/debug/examples/e2e_server}"
  for t in run theme-ui editor-ui account-pin scan-ui provider-ui bridge-ui real-scan real-bridge real-gate real-uninstall real-drive real-import real-lists real-pin real-disguise real-backup real-sync; do
    [ -f "e2e/$t.mjs" ] && step "e2e: $t" node "e2e/$t.mjs"
  done
  # Firefox, with the extension as a temporary add-on. Needs a firefox that can start the native host (not a snap). CI does not run it
  # unless HEXIS_E2E_FIREFOX=1: the runner's Firefox is not one this was developed against.
  if { [ -z "${CI:-}" ] || [ -n "${HEXIS_E2E_FIREFOX:-}" ]; } && command -v "${FIREFOX_BIN:-firefox}" >/dev/null 2>&1 && ! readlink -f "$(command -v "${FIREFOX_BIN:-firefox}")" | grep -q '/snap/'; then
    step "e2e: real-bridge-firefox" node e2e/real-bridge-firefox.mjs
  else
    printf '\nskipped e2e: real-bridge-firefox (no suitable firefox, or CI without HEXIS_E2E_FIREFOX=1)\n'
  fi
  # Several buckets at once against real S3-compatible servers (two throw-away containers on loopback). Needs Docker.
  if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    step "e2e: real-multi-s3 (S3 servers in Docker)" node e2e/real-multi-s3.mjs
  else
    printf '\nskipped e2e: real-multi-s3 (Docker is not available)\n'
  fi
fi
printf '\n%s\n' "$([ $fail = 0 ] && echo 'ALL CHECKS PASSED' || echo 'SOMETHING FAILED (see above)')"
exit $fail
