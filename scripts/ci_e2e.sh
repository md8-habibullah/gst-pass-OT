#!/usr/bin/env bash
# Runs the real-core end-to-end scripts exactly as CI does: build the dev-only HTTP shim (an example binary, never part of
# a release), start the Vite dev server, run every e2e/*.mjs script against them, and stop what it started.
#   scripts/ci_e2e.sh                 needs: node, a Chrome or Chromium (CHROME_BIN, default `chromium`), the Tauri system libraries
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
export SP="${SP:-$(mktemp -d)}"; mkdir -p "$SP/shots2"
export E2E_BIN="$ROOT/src-tauri/target/debug/examples/e2e_server"
(cd src-tauri && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo build --locked --example e2e_server)
npm run dev -- --port 1431 --strictPort > "$SP/vite.log" 2>&1 &
VITE=$!
trap 'kill "$VITE" 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do curl -fs http://localhost:1431/ >/dev/null 2>&1 && break; sleep 1; done
curl -fs http://localhost:1431/ >/dev/null || { echo "Vite did not start"; tail -20 "$SP/vite.log"; exit 1; }
scripts/verify_all.sh --e2e-only
