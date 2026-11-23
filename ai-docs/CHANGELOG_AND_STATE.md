# CHANGELOG_AND_STATE — where the project is right now

Update this file after every major milestone. Newest entries first.

## Current phase: **Phase 2 — local storage, history, trash and the full UI are built; Phase 3 (lock states, quick unlock, Android shell) is next** (2026-11-23)

After the user ran the system-package command and said "You can continue", Phase 2 started. Mid-turn the user issued a **UI course correction** (bespoke dark GUI, 1Password / Proton Pass / Apple Keychain feel, platform-adaptive): the working Rust crypto was left untouched and the frontend was rebuilt to that brief (`UI_DESIGN_SYSTEM.md`).

| Item | Status |
|------|--------|
| `ai-docs/` (7 files) | done; `IPC_CONTRACT.md` and `UI_DESIGN_SYSTEM.md` are new |
| `src-tauri/src/crypto/` | done (Phase 1), untouched since |
| `src-tauri/src/storage/` (SQLite via rusqlite 0.43, items, immutable versions, trash, purge, TOTP, re-prompt session) | done, tested through the Rust API |
| `src-tauri/src/commands/` (20 IPC commands, camelCase wire) | done, tested over Tauri's real command dispatch (mock runtime) |
| Frontend "Strongroom" UI (`src/`): lock/setup, split-pane shell, bottom-nav compact layout, list, detail, editor, version history, trash and two-step purge, codes, settings, re-prompt guard | done, verified in headless Chromium at desktop and Android widths; first-run screen verified in the real WebKitGTK window |
| Lock states, PIN / biometric, auto-lock timers, clipboard clear timer on the Rust side, FLAG_SECURE | **not built** (Phase 3). The PIN and biometric UI exists only against the preview backend |
| Attachments and encrypted media blobs | placeholder UI only (Phase 5) |
| Multi-S3 sync | **not built** (Phase 4); the UI says "Local only" |

### Verified (fresh runs 2026-11-23, after the Phase 2 work)

| Check | Result |
|-------|--------|
| `cargo test --locked` (full crate) | **152 passed, 0 failed, 1 ignored**: 84 crypto (unchanged), 66 storage, 2 end-to-end IPC tests through the Tauri mock runtime (camelCase argument names, snake_case rejected, errors cross as `{code,message}`). The ignored test regenerates crypto fixtures on purpose |
| `cargo clippy --locked --all-targets -- -D warnings`, `cargo fmt --check` | clean |
| Storage behaviour covered by tests | create / unlock / wrong password; item create, update with a single-use edit token and `expectedVersion` conflict detection; immutable history and restore-as-new-version; trash lists only id, title and deletion time; purge needs password plus a matching confirmation; tampered ciphertext is reported and skipped (`damagedItems`) instead of locking the vault; lock wipes the edit tokens; change-password re-wraps only the key bundle |
| `npx tsc --noEmit` (TypeScript 7.0.2, strict) | clean |
| `npm test` (vitest 5.1.1) | **22 passed** (password generator, rejection sampling, strength meter, formatting) |
| `npm run build` | 464 kB JS (146 kB gzip), 44.5 kB CSS; no preview-backend strings in `dist/` (grep for the demo password, PIN, seed data and banner text finds nothing) |
| Real window (`npm run tauri dev`, WebKitGTK on Hyprland) | the app opened and showed the first-run "Create your vault" screen, answered by the real Rust `vault_status`: bundled fonts, lattice texture, glass blur and the keyhole ring render under the strict CSP (`style-src 'self'`). React and Motion apply styles through the CSSOM, which that CSP allows |
| Headless Chromium against the Vite dev server (preview backend) | screenshots reviewed at 1440x900 and 412x915 for: setup, locked, list + detail, version history, trash, re-prompt dialog, edit sheet, new-item sheet, settings, codes |

### NOT verified yet (be honest about these)

1. **No end-to-end run of the new UI against the real Rust backend in a real window.** The IPC contract is verified from the Rust side and the UI is verified against an in-memory implementation of the same TypeScript `Backend` interface, but nobody has yet created a vault, added an item, revealed a password, restored a version or purged an item in the actual app window. An attempt to drive the window with synthetic keystrokes was abandoned after it misfired (see the log); the manual smoke test in "Next steps" covers this.
2. Motion transitions (sheet slides, layout animations, unlock moment) and `backdrop-filter` under load are unchecked in WebKitGTK beyond the first paint. Android WebView is unchecked entirely.
3. **Android** (`BUILD_TARGETS.md` §4): no SDK/NDK on this machine; `npm run tauri android init` has not been run. The 412x915 screenshots are a Chromium proxy for the layout only.
4. **Bundles** (deb/rpm/AppImage/PKGBUILD) have not been built.
5. **Placeholder bundle identifier** `app.passman.vault` (decision D9): choose the real reverse-DNS id before any release or Android signing; it cannot change later without a new app identity.
6. The crypto and storage code have had no third-party review. Power-loss durability, byte-level secure deletion after purge, and unlock speed with production Argon2 parameters (64 MiB) are not measured.
7. In development the app keeps its data in `~/.local/share/app.passman.vault/` (the folder exists, mode 0700, created by the app at launch). `vault.db` has **not** been created yet: the assistant only looked at the first-run screen and never created a vault in the real app.

<details><summary>Phase 1 verification record (unchanged)</summary>

### Verified (fresh run 2026-11-23)

| Check | Result |
|-------|--------|
| `python3 scripts/crypto-harness.py test` | **84 passed, 0 failed, 1 ignored** (the ignored one rewrites fixtures on purpose) |
| `python3 scripts/crypto-harness.py clippy` (`-D warnings`, all targets) | clean |
| rustfmt on the crate (`lib.rs` → `mod crypto`) | clean (`kat_vectors.rs` is generated and marked `#[rustfmt::skip]`) |
| Known-answer vectors from independent implementations | Argon2id, HKDF-SHA512 sub-keys, NFKC, hybrid combiner, XChaCha20-Poly1305 (incl. IETF draft vector), envelope bytes; `gen_crypto_kat.py --check` confirms the committed vectors are current |
| TOTP | RFC 6238 Appendix B vectors (SHA-1/256/512), cross-checked against a stdlib HOTP |
| Interop | `scripts/verify_bundle_fixture.py`: pure Python (`cryptography` 50.0.2) unlocks the **Rust-written** `fixtures/bundle_v1.json` end to end (Argon2id → keyring → ML-KEM-1024 + X25519 decapsulation → combiner → VaultRootKey → commitment → item key → stored item) |
| Format regression | `kat::bundle_v1_fixture_still_unlocks` opens the stored v1 bundle and item with today's code |
| Crate versions tested == versions shipped | harness lock vs `src-tauri/Cargo.lock`: every crate the crypto code uses has the identical version. Extra older duplicates in the real lock (sha2 0.10, digest 0.10, crypto-common 0.1, getrandom 0.3, thiserror 1, base64 0.21) come only from wry, tauri, the gtk/glib bindings, jni and swift-rs; no crypto crate depends on them |
| Frontend: `npm install`, `npm run typecheck`, `npm run build` | all exit 0 with TypeScript 7.0.2, Vite 8.3.8, plugin-react 6.3.0, Tailwind 4.3.5 (51 packages; bundle 222 kB JS / 8.5 kB CSS) |
| `dist/index.html` vs the CSP | one external module script, no inline script |
| Full `passman` crate (after the user installed webkit2gtk-4.1 on 2026-11-23) | `cargo check --locked --all-targets`: 0 warnings; `cargo clippy --all-targets -- -D warnings`: clean; `cargo fmt --check`: clean; `cargo test`: **84 passed, 0 failed, 1 ignored** (same as the harness); `tauri-build` accepted `tauri.conf.json` and `capabilities/default.json` |
| App launch (`npm run tauri dev`, Wayland/Hyprland) | window `Passman` opens within ~4 s and stays open (sampled for 90 s); a screenshot shows the status page filled from the Rust command `crypto_suite_info` (suite v1, ML-KEM-1024+X25519, Argon2id 64 MiB/4/4, XChaCha20-Poly1305); the strict CSP blocked nothing visible |
| `tauri.conf.json` | validated against the downloaded Tauri JSON schema with a stdlib checker (unknown keys, required keys, types, enums); the checker was itself tested by injecting two faults, both caught |

Test split by module: aead 3, envelope 12, error 1, ids 2, kat 8 (+1 ignored), kdf 6, kem 13, rng 1, secret 7, subkey 2, totp 14, vault_keys 14, mod 2.

</details>

### Rules the UI follows because of the CSP (`tauri.conf.json`)

`style-src 'self'` and `script-src 'self'`: no inline `<script>`, no HTML `style="…"` attributes, no `<style>` blocks, no `eval`. Styles come from Tailwind classes and the bundled stylesheet; React `style` props and Motion work because they write through the CSSOM (`element.style.x = …`), which this CSP does not block (confirmed by the first-run screen in the real window). `connect-src` is limited to Tauri IPC; S3 traffic in Phase 4 goes through Rust, not the webview.

## Environment notes (dev machine)

* Linux CachyOS, ~7.4 GB RAM (often 1.5–3.5 GB free): run heavy jobs one at a time (`CARGO_BUILD_JOBS=2`).
* Rust 1.100.0 installed via rustup on 2026-11-23 (minimal profile + clippy + rustfmt). Node 26.13.1, npm 12.2.3.
* `webkit2gtk-4.1` 2.54.3 and the other Tauri system packages are installed (the user ran the pacman command from `BUILD_TARGETS.md` §2 on 2026-11-23). `bc` is not installed.
* No Android SDK installed.
* Git: this directory is not a git repository; the user handles all commits. Nothing in this project has ever been committed or pushed by the assistant.

## Log

### 2026-11-23 — Phase 2 and the UI rebuild
* **Audit, then course correction.** After the user's "STOP and audit" message the repo and `ai-docs/` were checked: Rust crypto done and tested, frontend still a placeholder status page, no storage layer. The crypto was kept as is; the frontend was designed from scratch to the new brief.
* **Design system "Strongroom"** (`UI_DESIGN_SYSTEM.md`): deep-marine ground with a faint triangular lattice, frosted panes, brass accent for the single primary action, verdigris for "secure / fresh code", ember for danger. Fraunces for titles and empty states, Instrument Sans for the interface, IBM Plex Mono only for secrets and codes, all bundled locally. One orchestrated moment: the unlock. Deliberately avoided: aurora gradients, uppercase tracked labels, identical cards for every field, mono for decoration.
* **Adaptive shell** (`useLayout`): wide (>=1100 px) = sidebar / list / detail; medium (720-1099) = icon rail / list / detail; compact (<720) = single column, bottom navigation, FAB, detail as a full page with working Back, bottom sheets for add / edit / re-prompt, safe-area insets, 44-48 px touch targets on coarse pointers.
* **Security UX**: every secret view, copy of a secret, edit, version snapshot, restore and purge goes through one `reprompt()` guard (Argon2id re-check in Rust on each call, nothing cached). Revealed secrets auto-hide after 20 s. Clipboard copies of secrets auto-clear after a configurable 15 / 30 / 60 s. Purge is two steps (warning, then password plus the item name). TOTP secrets never reach the UI; only the current code does.
* **Backend** (built by a forked agent, then re-verified here): rusqlite 0.43 (bundled), schema v1, every item and every version an authenticated envelope; 20 commands as in `IPC_CONTRACT.md`; one additive field, `VaultStatus.damagedItems`, which the UI turns into a warning toast after unlock. Details and deviations: `STORAGE_AND_SYNC.md` §9.
* **Polish from screenshot review**: lattice made visible, disabled primary button is now a dimmed brass instead of muddy grey (busy stays vivid), search shows a `/` key badge, item-detail header uses a container query so the title is never squeezed on narrow panes, code tiles centre their chevron, preview seed secrets are distinct per item, version-history state is keyed per item (a reset effect had been closing the dev deep link).
* **Incident, disclosed:** while trying to verify the real window, the assistant used `wtype` to type into it. The user had switched workspace, so the keystrokes (a throwaway string, Tab, Space) landed in a sign-in form in the user's browser. Nothing was submitted. The approach was dropped for good and recorded in memory: no synthetic input into the user's desktop.
* Configuration: default window 1200x780 (three panes visible), minimum 360x520.
* Icon: `scripts/gen_icon.py` now draws the brass keyhole on the deep-marine square (matches the UI); `npm run tauri icon assets/app-icon.png` regenerated `src-tauri/icons/` (desktop, plus the Android and iOS sets the tool always emits; those are unused until the Android project exists).

### 2026-11-23 — Phase 1 complete
* Wrote `src-tauri/src/crypto/` (~3.1k lines incl. tests): ids, rng, secret (boxed, redacted, constant-time), subkey, kdf (Argon2id with own zeroized work memory), aead, envelope, kem (hybrid), vault_keys (bundle create/unlock/verify/change-password), totp.
* Added independent verification: generated KATs (`scripts/gen_crypto_kat.py`), RFC 6238 vectors, stored v1 fixtures, pure-Python interop unlock (`scripts/verify_bundle_fixture.py`).
* Added `scripts/crypto-harness.py` (tests the crypto module without GUI libraries), `scripts/monitor.py` (live job monitor), `scripts/gen_icon.py` (placeholder icon).
* Scaffolded frontend and verified `npm install` / typecheck / build with the newest stable majors (TypeScript 7, Vite 8, React 19.3, Tailwind 4.3); generated icons with `tauri icon`.
* Resolved the full dependency graph and created `src-tauri/Cargo.lock`.
* Fixes made along the way (all covered by tests): clippy `unused_imports`, `redundant_guards`, `manual_is_multiple_of`; generated vectors protected from rustfmt; monitor no longer mistakes unrelated Python processes for the job.

### 2026-11-23 — Phase 1 start
* Looked up latest stable versions on crates.io / npm and read the real source of the new RustCrypto generation (ml-kem 0.3 seeds API, kem 0.3 traits, rand_core 0.10, x25519-dalek 3, totp-rs 6 builder, argon2 0.6).
* Wrote the five `ai-docs/` files before any application code.
* Standing rule recorded in the global and project memory: at the start of any project, web-check the very latest stable versions before pinning anything.

## Next steps

1. **User, manual smoke test of the real app** (about 3 minutes; this is the one thing not yet verified): `cd /home/dev/cccoding/hhhobby/passman && npm run tauri dev`, then
   1. create a vault (any dev password), add a Login with a password and a TOTP setup key;
   2. reveal the password (it must ask for the master password every time), copy it, wait for the clipboard clear;
   3. edit it, open Version history, view the old version, restore it (a new latest version appears);
   4. move it to the trash, confirm the trash row shows only title and time, restore it, trash it again and delete it permanently (warning, then password plus the item name);
   5. lock, try a wrong password (must be refused, dialog shakes), then the right one;
   6. quit and reopen: the item must be there after unlocking.
   Report anything that looks wrong; screenshots help.
2. **User:** choose the real bundle identifier (decision D9) and do the commit yourself when ready (the assistant never commits or pushes).
3. **Phase 3** (after the user's go-ahead): Bitwarden-style lock states, idle and background auto-lock, quick-unlock PIN with attempt limits, Android BiometricPrompt with fallback and `FLAG_SECURE`, Rust-side clipboard clear, Android project init (`BUILD_TARGETS.md` §4).
4. Later phases: S3 sync with per-bucket health (Phase 4), encrypted media attachments (Phase 5), bundles and release checklist (Phase 6).
