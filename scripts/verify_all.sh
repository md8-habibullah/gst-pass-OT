#!/usr/bin/env bash
# One command that runs every check the project has, in the order that fails fastest.
#   scripts/verify_all.sh            everything except the real-core end-to-end scripts
#   scripts/verify_all.sh --e2e      also run the headless-browser end-to-end scripts (needs chromium, a free port 1431/144x)
# Heavy parts (cargo) run one after another; the machine this was written on has little RAM.
set -u
cd "$(dirname "$0")/.."
ROOT=$(pwd)
fail=0
step() { printf '\n=== %s\n' "$1"; shift; if "$@"; then printf '    ok\n'; else printf '    FAILED: %s\n' "$*"; fail=1; fi; }

step "frontend types"            npx --no-install tsc --noEmit
step "frontend unit tests"       npx --no-install vitest run
step "frontend production build" npx --no-install vite build
step "no dev-only strings in the production bundle" bash -c '! grep -rlE "seedPreview|PREVIEW_PASSWORD|e2e_server|simulateOtherDeviceEdit|Marlowe" dist/'
step "no notification or tray code" python3 scripts/check_no_notifications.py
step "android patch script tests" bash -c 'if [ -d scripts/tests ]; then python3 -m unittest discover -s scripts/tests -t . ; else echo "(no script tests)"; fi'
( cd src-tauri || exit 1
  step "rust format"  cargo fmt --check
  step "rust clippy"  env CARGO_BUILD_JOBS=2 cargo clippy --locked --all-targets -- -D warnings
  step "rust tests"   env CARGO_BUILD_JOBS=2 cargo test --locked
) || fail=1

if [ "${1:-}" = "--e2e" ]; then
  # The end-to-end scripts drive the REAL Rust core (through the dev-only HTTP shim) from a headless Chromium
  # that talks to the Vite dev server. They need: `npm run dev -- --port 1431` running, chromium on the PATH,
  # and E2E_BIN pointing at the shim built with `cargo build --example e2e_server` (target/debug/examples/e2e_server).
  export SP="${SP:-/tmp/passman-e2e}"; mkdir -p "$SP/shots2"
  export E2E_BIN="${E2E_BIN:-$ROOT/src-tauri/target/debug/examples/e2e_server}"
  for t in run real-gate real-drive real-import real-disguise real-backup real-sync; do
    [ -f "e2e/$t.mjs" ] && step "e2e: $t" node "e2e/$t.mjs"
  done
fi
printf '\n%s\n' "$([ $fail = 0 ] && echo 'ALL CHECKS PASSED' || echo 'SOMETHING FAILED (see above)')"
exit $fail
