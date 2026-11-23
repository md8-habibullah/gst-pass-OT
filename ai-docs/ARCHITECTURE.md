# ARCHITECTURE — Passman (post-quantum, zero-knowledge, local-first vault)

Read this first, then `CRYPTO_SPEC.md` (wire formats), `STORAGE_AND_SYNC.md` (data + sync), `BUILD_TARGETS.md` (packaging), `CHANGELOG_AND_STATE.md` (what exists now).

## 1. What this is

A standalone Tauri v2 app (Rust core + React/TypeScript/Tailwind UI) that stores passwords, TOTP secrets, secure notes and encrypted media.
Zero knowledge: every byte that leaves the device is ciphertext; the master password and all keys exist only in Rust memory.
Local-first: full offline read/write; optional direct-to-S3 sync (1 primary + N mirrors), no middleman server.
Targets: Linux desktop (X11/Wayland; deb, rpm, AppImage, Arch PKGBUILD) and Android (minSdk 28 → targetSdk 36).

**Non-goals (by decision):** OS autofill services, browser extensions, any server component, telemetry.

## 2. Verified stack (checked 2026-11-23)

| Layer | Choice |
|-------|--------|
| Rust toolchain | stable **1.100.0** (rustup, minimal profile + clippy + rustfmt) |
| App shell | `tauri 2.12.7`, `tauri-build 2.7.6`, `@tauri-apps/cli 2.12.6`, `@tauri-apps/api 2.12.5` |
| Crypto | see `CRYPTO_SPEC.md` §2 (argon2 0.6, ml-kem 0.3, x25519-dalek 3, chacha20poly1305 0.11, hkdf 0.13, totp-rs 6) |
| UI | React 19.3.1, TypeScript 7.0.2, Vite 8.3.8, `@vitejs/plugin-react` 6.3.0, Tailwind CSS 4.3.5 (`@tailwindcss/vite`) |

Rule for any new dependency (user standing rule): look up the latest **stable** version online first, read the real API if it is unfamiliar, record it here.
Many RustCrypto crates changed major versions in 2026 (`rand_core 0.10`, traits moved to `kem`/`aead 0.6`); do not code these from memory.

## 3. Threat model

**Protects against**
* Theft/loss of a locked device or a copy of the local files and the S3 bucket(s): attacker gets ciphertext + a public key bundle; must brute-force the master password through Argon2id (64 MiB × 4 passes default).
* Storage-provider compromise or a malicious provider: sees object keys, sizes, timestamps (metadata leakage, see `STORAGE_AND_SYNC.md` §8), never content. Tampering is detected by AEAD; substituting the root key is detected by the keyring commitment (`CRYPTO_SPEC.md` §9).
* "Harvest now, decrypt later" quantum adversary: hybrid ML-KEM-1024 + X25519 wrap, 256-bit symmetric keys.
* Shoulder-surfing/accidental exposure: secrets hidden behind the Master-Password Re-Prompt Guard; clipboard auto-clear; `FLAG_SECURE` on Android.
* Forgotten/idle unlocked app: configurable soft-lock timer, background-lock option.

**Does not protect against**
* A compromised OS, root malware or a keylogger while the vault is unlocked (it can read process memory or the screen). Mitigations are best-effort (memory wiping, `mlock`, no core dumps).
* A weak master password.
* A storage provider that **rolls back** objects to older (validly encrypted) versions: partially mitigated by per-item local high-water marks, not fully preventable without a trusted counter.
* Traffic/metadata analysis of S3 access patterns.

## 4. Trust boundaries

```
 ┌──────────────── Webview (React) — UNTRUSTED for secrets ────────────────┐
 │  renders UI state, forwards user intent, shows only what Rust returns   │
 └──────────────▲───────────────────────────────────────┬──────────────────┘
        Tauri IPC (typed commands, capability-gated, strict CSP)
 ┌──────────────┴───────────────────────────────────────▼──────────────────┐
 │ Rust core — TRUSTED                                                      │
 │  commands/  → lock state machine → crypto/ → storage/ → sync/            │
 │  holds VaultRootKey only in `Zeroizing` memory while unlocked            │
 └──────────────────────────────┬───────────────────────────────────────────┘
        local SQLite (opaque envelopes)        S3 targets (opaque envelopes)
```

IPC rules (enforced from Phase 2 on):
1. No command ever returns a key, seed, password, or hash of one.
2. Commands that reveal or change secrets (reveal password, view version history secrets, edit, permanent delete, export) require a fresh **re-prompt token**: the UI sends the master password with the command; Rust verifies it via `verify_password` (Argon2id) and does not cache it.
3. Trash policy ("only title + deletedAt visible") is enforced in the Rust command layer, not only in the UI.
4. TOTP: the UI receives the current code + seconds remaining, never the secret.
5. CSP is closed: `default-src 'self'`, `connect-src ipc: http://ipc.localhost`. All network I/O (S3) happens in Rust.

## 5. Lock state machine (Phase 3)

| State | Memory | Leaves it by |
|-------|--------|--------------|
| **Hard locked** (cold start, app killed) | nothing | Master Password |
| **Soft locked** (timer / background) | VRK wiped; session blob in OS keystore | Quick PIN or biometrics, else Master Password |
| **Unlocked** | VRK in `Zeroizing` | timer, background event, manual lock |

Timer options: Immediate, 30 s, 1/2/5/15/30 min, custom seconds, Never (until app closed); independent toggle "lock when backgrounded/minimized". The Master-Password Re-Prompt Guard applies in the unlocked state too.

## 6. Module map

```
src-tauri/src/
  main.rs            desktop entry (calls passman_lib::run)
  lib.rs             Tauri builder, command registration, `pub mod crypto;`
  crypto/            Phase 1 [done, verified in the GUI-free harness]
    mod.rs           suite constants, SuiteInfo, now_millis, re-exports
    error.rs         CryptoError (opaque, serializable as {code,message})
    ids.rs           object-id validation, reserved ids `_keyring` / `_vrk`
    rng.rs           OS CSPRNG helpers
    secret.rs        SecretKey32 (boxed), MasterPassword (NFKC), SecretVec (zeroizing)
    subkey.rs        KeyPurpose + HKDF-SHA512 per-object sub-keys
    kdf.rs           Argon2id params/validation/derivation (own zeroized work memory)
    aead.rs          XChaCha20-Poly1305 seal/open primitive (private to envelope)
    envelope.rs      header + binary container (AAD = header bytes)
    kem.rs           ML-KEM-1024 + X25519 hybrid, HKDF-SHA512 combiner
    vault_keys.rs    VaultKeyBundle: create / unlock / verify / change password; VaultRootKey
    totp.rs          RFC 6238 engine + otpauth parsing
    kat.rs           (tests) known-answer + stored-fixture tests
    kat_vectors.rs   (tests) GENERATED by scripts/gen_crypto_kat.py, never hand-edited
    fixtures/        bundle_v1.json + item key/ciphertext: on-disk format regression guard
  storage/           Phase 2 (SQLite, items, versions, trash)
  sync/              Phase 4 (S3, mirrors, conflicts)
  lock/              Phase 3 (state machine, quick unlock, biometrics)
src/                 React UI (placeholder status page in Phase 1; Tailwind classes only, see CSP)
assets/app-icon.png  placeholder icon source; `npm run tauri icon assets/app-icon.png` regenerates src-tauri/icons
scripts/
  monitor.py               read-only live monitor for long jobs (`--once`, `--log`, `--lock`)
  crypto-harness.py        build/test/clippy the crypto module without GUI libraries
  gen_crypto_kat.py        regenerate/--check kat_vectors.rs from independent Python implementations
  verify_bundle_fixture.py pure-Python unlock of the Rust-written fixture bundle (interop proof)
  gen_icon.py              draws the placeholder icon (stdlib only)
ai-docs/             these documents
```

## 7. Phase roadmap

1. **Phase 1** — docs, scaffold, `crypto/` (this repo state; see `CHANGELOG_AND_STATE.md`).
2. **Phase 2** — local storage, immutable version history, trash lifecycle, re-prompt guard commands.
3. **Phase 3** — lock state machine, quick PIN/biometric, clipboard timer, `FLAG_SECURE`, settings UI.
4. **Phase 4** — S3 sync engine (primary + mirrors), conflict UI, health indicators.
5. **Phase 5** — real UI, attachments/media (chunked envelope), import/export, packaging for all targets.

Crates are added in the phase that uses them (SQLite in 2, keyring/biometric in 3, S3 in 4) so builds stay light on the low-RAM dev machine.

## 8. Decision log (why things are the way they are)

| # | Decision | Reason |
|---|----------|--------|
| D1 | Crypto lives in `src-tauri/src/crypto/` (not a separate crate) | Spec requirement. Tested without GUI libs through `scripts/crypto-harness.py`, which compiles the same files in a bare crate. |
| D2 | Layered keys: Argon2id → seals KEM secrets; KEM wraps a random VaultRootKey | Password change = re-seal 128 bytes, no data re-encryption; leaves room for extra recipients; matches the hybrid-KEM spec. |
| D3 | ML-KEM keys stored as 64-byte seeds | Upstream deprecated the 3168-byte expanded form; seeds are smaller and always valid. |
| D4 | Keyring commitment to the root key | Public KEM keys let anyone craft a wrap; the commitment inside the password-sealed keyring defeats root-key substitution. |
| D5 | Header JSON bytes are the AAD, stored verbatim | Avoids JSON canonicalization bugs. |
| D6 | Per-object sub-keys via HKDF labels | Domain separation between item/version/attachment/meta. |
| D7 | NFKC-normalized master password | Cross-platform unlock consistency. One-way door, decided before any vault exists. |
| D8 | One-shot AEAD up to 64 MiB | Simplicity now; chunked envelope later for big media. |
| D9 | App identifier `app.passman.vault` is a **placeholder** | Android applicationId cannot change after a Play release; confirm before first public build. |
