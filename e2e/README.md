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
node e2e/theme-ui.mjs                               # light, dark and device setting in the browser preview, nothing stored by the page
node e2e/real-gate.mjs                              # Stage 1 gate, decoy maths, panic, wipe, idle lock (scaled timer)
node e2e/real-uninstall.mjs                         # theme and clipboard time kept by the core, "Remove Hexis from this device" leaves no file
node e2e/real-drive.mjs                             # encrypted drive, 50 MB limit, attachments
node e2e/real-import.mjs                            # import preview and commit, plaintext export, round trip
node e2e/real-scan.mjs                              # the all-in-one scanner and passkeys on the real core: suggests the login with the same account, adds a code, re-prompt, a passkey made through the provider's entry stays out of every view
node e2e/real-lists.mjs                             # folders first, search through notes and old versions, filters and order, several web addresses, open in search
node e2e/real-disguise.mjs                          # app name and icon (writes a launcher entry in a temp XDG folder)
node e2e/real-backup.mjs                            # backup targets against scripts/mock_s3.py
node e2e/real-sync.mjs                              # two devices, pull and reconcile, the generator settings, no idle traffic
node e2e/real-multi-s3.mjs                          # one primary and two secondary buckets on real S3-compatible servers (needs Docker)
```

`scripts/verify_all.sh --e2e` runs all of them one after another (the machine this was written on has little RAM: never run two at once).

* `real-multi-s3.mjs` starts two throw-away [Versity S3 Gateway](https://github.com/versity/versitygw) containers on loopback (v1.11.0, pinned by digest; random credentials; data in RAM) and removes them when it ends. It proves the multi-bucket behaviour against a real S3-compatible server. It does **not** prove any hosted provider (Amazon S3, Cloudflare R2, Backblaze B2).
* `real-gate.mjs` starts the shim with `--idle-unit-ms` (scales the inactivity and the "after leaving the app" timers) and, in the steps that look at the waits after wrong passcodes, with `--lock-unit-ms` (one "minute" of waiting lasts that many milliseconds; 0 means no waits).
* `real-import.mjs` also imports **your own** exports when you point it at them (read only, never copied):
  `REAL_BW=~/Downloads/bitwarden_export_….json REAL_ENTE=~/Downloads/ente-auth-codes-plaintext-….txt node e2e/real-import.mjs`.
  Without those variables it runs only the synthetic steps.
* `real-uninstall.mjs` plants what an older build's web view left in the data folder, tries the removal with the app lock closed (refused) and with a wrong sum (nothing removed), then removes with the right one and checks that no file is left but the shim's own `.hexis-e2e` marker, and that nothing is written afterwards. `theme-ui.mjs` emulates a light and a dark device (`prefers-color-scheme`) against every theme choice.
* `gate-ui`, `mobile-ui`, `import-ui`, `files-ui`, `sync-ui`, `theme-ui` check the browser **preview** backend (`npm run dev` with deep links such as `?vault=unlocked&filter=import`), not the real core.
* The shim answers `POST /e2e/pick-next {"path": "/abs"}` or `{"paths": [...]}` for the next native dialog (open file, key file, add files, save a copy, save an export; an empty queue cancels) and `GET /e2e/last-saved` for the last saved file. It is dev-only: it is an example binary, not part of any build.
* `scan-ui` and `provider-ui` check the browser **preview** backend: the scanner (every kind of text, the suggestions, "Create a new login") and Settings > Passkeys. `real-scan.mjs` runs the same flows on the real core and also calls the provider's own entry (`POST /e2e/provider {"op","args"}` on the shim, the same `Provider::call` that Android's service uses) to create and use a passkey for a made-up website. `cargo run --example passkey_fixture` prints a made-up website's request and the answer, for comparing with another WebAuthn implementation.

## Phone checks for passkeys (by hand, not run by any script)

Nothing in this folder can start Android's Credential Manager. After installing a build on an Android 14 or newer phone, check these, and write down the phone model and Android version with the result:

1. Settings > Passkeys: switch on "Offer Hexis for passkeys", tap "Open Android settings", choose Hexis. Hexis appears in the phone's list of passkey providers.
2. In a browser on the privileged list (Chrome, Firefox, Brave, Edge …) open a passkey test site, create a passkey: the Hexis sheet opens, asks for the fingerprint, and the passkey is on a login in the app afterwards (without any way to see its key).
3. Sign in on that site with it: the sheet lists the login, asks for the fingerprint, and the site accepts it.
4. A login that asks for the master password (re-prompt) makes the sheet ask for it.
5. With the app lock closed, the sheet refuses (`app_locked`) instead of opening anything.
6. With the vault locked, the sheet asks for the master password and never accepts the fingerprint alone.
7. A native app that has a Digital Asset Links file: it signs in; with "Check that a website lists the app" switched off in Settings > Passkeys it does not, and the phone makes no request to that website.
8. On a computer, "Use a passkey on a phone or tablet", scan the code with the **system camera** (not Hexis): Hexis is offered, and it signs in. (Hybrid transport: unverified until done.)
9. Switch the provider off again: Hexis disappears from the list, and the app still works as before.
