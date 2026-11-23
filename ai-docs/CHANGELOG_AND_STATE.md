# CHANGELOG_AND_STATE — where the project is right now

Update this file after every major milestone. Newest entries first.

## Current phase: **Phase 1 — complete, awaiting user confirmation** (2026-11-23)

Phase 2 (local storage, version history, trash) must **not** start until the user confirms.

| Item | Status |
|------|--------|
| `ai-docs/` (5 files) | done |
| Tauri v2 scaffold (`src-tauri/`, React/TS/Tailwind UI) | done: frontend builds, the full crate compiles and tests pass, and the app window opens and shows data from the Rust side |
| `Cargo.toml` + `Cargo.lock` with verified crate versions | done; full graph (470 packages, tauri included) resolves |
| `src-tauri/src/crypto/` | done, verified outside Tauri (below) |
| Icons | placeholder generated (`scripts/gen_icon.py` → `assets/app-icon.png` → `tauri icon`) |

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

### NOT verified yet (be honest about these)

1. **Android** (`BUILD_TARGETS.md` §4) is unverified: no SDK/NDK on this machine; `npm run tauri android init` has not been run.
2. **Bundles** (deb/rpm/AppImage/PKGBUILD) have not been built.
3. **Placeholder bundle identifier** `app.passman.vault` (decision D9): the user must choose the real reverse-DNS id before any release or Android signing, because it cannot change later without a new app identity.
4. The crypto code has had no third-party review. The design is conservative and tested against independent implementations, but treat it as unreviewed until someone audits it.

### Rules the UI must follow because of the CSP (`tauri.conf.json`)

`style-src 'self'` and `script-src 'self'`: no inline `<script>`, no `style={…}` attributes, no `eval`; use Tailwind classes. `connect-src` is limited to Tauri IPC; S3 traffic in Phase 4 goes through Rust, not the webview.

## Environment notes (dev machine)

* Linux CachyOS, ~7.4 GB RAM (often 1.5–3.5 GB free): run heavy jobs one at a time (`CARGO_BUILD_JOBS=2`).
* Rust 1.100.0 installed via rustup on 2026-11-23 (minimal profile + clippy + rustfmt). Node 26.13.1, npm 12.2.3.
* `webkit2gtk-4.1` 2.54.3 and the other Tauri system packages are installed (the user ran the pacman command from `BUILD_TARGETS.md` §2 on 2026-11-23). `bc` is not installed.
* No Android SDK installed.
* Git: this directory is not a git repository; the user handles all commits. Nothing in this project has ever been committed or pushed by the assistant.

## Log

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

1. **User:** review Phase 1; install the system packages; run `cd src-tauri && cargo check && cargo test`; choose the real bundle identifier; then confirm Phase 2.
2. Phase 2: local SQLite storage, immutable version history, trash lifecycle, re-prompt guard commands (`STORAGE_AND_SYNC.md` §1–5).
