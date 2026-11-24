# End-to-end scripts

These drive the **real Rust core** (the same command layer the app uses, real Argon2id / ML-KEM / XChaCha20, real SQLite files) from a headless Chromium. Nothing here touches the desktop: no synthetic keystrokes, no real windows.

**One command, as CI runs it:** `scripts/ci_e2e.sh` builds the shim, starts Vite, runs every script below and stops what it started.
It uses the `chromium` on the PATH; set `CHROME_BIN` (for example `CHROME_BIN=google-chrome`) to use another Chrome or Chromium.

```
npm run dev -- --port 1431                          # the UI (Vite)
(cd src-tauri && cargo build --example e2e_server)  # the dev-only loopback shim around the core
export SP=/tmp/hexis-e2e; mkdir -p $SP/shots2     # scratch folder: data, profiles, screenshots
export E2E_BIN=$PWD/src-tauri/target/debug/examples/e2e_server
node e2e/run.mjs                                    # accounts, items, history, trash, vault file, import, no plaintext on disk
node e2e/real-gate.mjs                              # Stage 1 gate, decoy maths, panic, wipe, idle lock (scaled timer)
node e2e/real-drive.mjs                             # encrypted drive, 50 MB limit, attachments
node e2e/real-import.mjs                            # import preview and commit, plaintext export, round trip
node e2e/real-disguise.mjs                          # app name and icon (writes a launcher entry in a temp XDG folder)
node e2e/real-backup.mjs                            # backup targets against scripts/mock_s3.py
node e2e/real-sync.mjs                              # two devices, pull and reconcile, no idle traffic
```

`scripts/verify_all.sh --e2e` runs all of them one after another (the machine this was written on has little RAM: never run two at once).

* `real-import.mjs` also imports **your own** exports when you point it at them (read only, never copied):
  `REAL_BW=~/Downloads/bitwarden_export_….json REAL_ENTE=~/Downloads/ente-auth-codes-plaintext-….txt node e2e/real-import.mjs`.
  Without those variables it runs only the synthetic steps.
* `gate-ui`, `mobile-ui`, `import-ui`, `files-ui`, `sync-ui` check the browser **preview** backend (`npm run dev` with deep links such as `?vault=unlocked&filter=import`), not the real core.
* The shim answers `POST /e2e/pick-next {"path": "/abs"}` or `{"paths": [...]}` for the next native dialog (open file, key file, add files, save a copy, save an export; an empty queue cancels) and `GET /e2e/last-saved` for the last saved file. It is dev-only: it is an example binary, not part of any build.
