# ARCHITECTURE — Hexis (post-quantum, zero-knowledge vault with offline-first multi-S3 sync)

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
2. While the vault is open, showing, copying, editing and cloning an item work directly, like in Bitwarden. An item whose own **re-prompt flag** is set asks for the master password every time, and a few commands always do (permanent delete, change of the master password, exports, saving or deleting drive files, backup secrets, changing the app lock). The UI sends the password with the command; Rust verifies it via `verify_password` (Argon2id) and does not cache it. A command that carries a password always has it verified, flag or no flag.
3. Trash policy ("only title + deletedAt visible") is enforced in the Rust command layer, not only in the UI.
4. TOTP: the UI receives the current code + seconds remaining, never the secret.
5. CSP is closed: `default-src 'self'`, `connect-src ipc: http://ipc.localhost`. All network I/O happens in Rust: the S3 backups, and the optional time check (`clock/net.rs`, three HEAD requests to public HTTPS sites, switchable in Settings, D27).

## 5. Lock state machine (strict separation, decided 2026-11-23; built in Q1, see `FEATURE_QUEUE.md`)

Two independent stages. **Stage 1 never touches a key.**

| State | In memory | Leaves it by |
|-------|-----------|--------------|
| **Gate closed** (Stage 1, optional; cold start, "Lock app", idle) | nothing: no account list, no key | passcode (PIN, password or number grid) or fingerprint. 1 to 3 characters are only answers to the decoy maths |
| **Account list** (Stage 1 open) | account display names | choosing an account |
| **Vault locked** | nothing | the **master password, always** (Argon2id + hybrid KEM) |
| **Vault unlocked** | VaultRootKey in `Zeroizing` | inactivity timer (1 / 5 / 15 / 30 min / 1 h / 4 h), manual lock, app exit; on phones also leaving the app for longer than the chosen time (right away to 4 h, default 5 min): the key is zeroized and the next open needs the master password again |

Rules: no PIN, fingerprint or pattern ever wraps, caches or derives the VaultRootKey; nothing is stored in the Android Keystore or a Linux keyring for the vault; the Re-Prompt Guard applies in the unlocked state too. The earlier "soft lock with a session blob in the OS keystore" design is dropped (decision D13).

### 5.1 The account PIN: the middle layer (decided 2026-11-25, D45; amends D13)

Three layers now, from the outside in: **(1) the app lock** (Stage 1, above), **(2) the account PIN**, **(3) the master password**. The PIN is optional and per account (4 to 64 characters, set in Settings > Security with the master password).

| State | In memory | Leaves it by |
|-------|-----------|--------------|
| **Vault parked** (an account with a PIN, after Lock or after switching to another account) | the VaultRootKey, still in `Zeroizing` inside its `Session`, **out of reach**: every command on it answers `locked` | its PIN (`vault_unlock_pin`), or the wipe below |
| **Vault limited** (shown again with the PIN) | the same session | the master password typed once (`vault_elevate`: now an ordinary unlocked vault), or Lock again (parked once more, limited again on the next PIN) |

What the PIN is: an **Argon2id hash in `profiles.json`** (the entry's `pin`, gate cost parameters, never above the vault's own). It is a check on a person standing at a vault that is *already open in RAM*. **It never wraps, caches or derives any key, and nothing new is written to disk to open a vault.** Whoever copies the files and cracks the hash learns a short word and can open nothing.

When a parked vault is wiped (keys zeroized, only the master password opens it afterwards): the inactivity timer ends, the app is left for longer than the chosen time (phone), the app lock closes, "Log out" (`vault_lock_all`), the app exits, the account is removed or its PIN is removed, the app is erased, or the PIN is wrong **5 times in a row** (`pin_locked`).

What a limited vault refuses until the master password is typed, **enforced in the core, not by the screens** (`Session::gated_item` and `require_full`): every item operation that returns a secret or starts an edit (`item_reveal`, `item_begin_edit`, `item_clone`, `totp_now`, `history_get`, `history_restore`: `password_required`, flagged or not); adding, changing, removing or choosing the master of a backup target, `app_settings_set` (the lock timers) and `file_read` (attachment previews): `master_required`; `item_search` compares no note text. The count of five wrong PINs is never restarted by Lock or by choosing the account again while it is parked (found by the 2026-11-25 security review and fixed with two regression tests). Permanent delete, exports, secret access keys, changing the master password, removing an account and changing the app lock already carry the master password in the call itself, with or without a PIN.

Order of the locks in `AppCore` (to avoid deadlocks): ops, registry, active, parked. The Argon2id check of a PIN runs with none of them held.

Honest limits. While a parked vault waits, the key is in RAM: someone who can read this process's memory can take it, exactly as for an open vault. The timer is the bound on that, and a user who wants no such window simply sets no PIN. Five tries is a rate limit on the screen, not on someone with the files: they have nothing to try the PIN against that opens a vault.

**Status (2026-11-24).** Q1 is **implemented** (desktop verified; Android fingerprint unverified). D13 holds in code: nothing
derives, wraps or caches a `VaultRootKey` outside RAM, and `AppCore` tests prove the session is gone after
lock, profile switch/create, `gate_close`, idle lock, profile removal, wipe and app exit.

**Honest limits of Stage 1.**

* The gate file stores Argon2id hashes. A 4-digit PIN has 10 000 values: whoever can **read** `gate.json` can
  recover it offline in minutes. Stage 1 hides the account list from a person looking at the screen; the
  master password and the vault encryption are the protection against someone with the files.
* The attempt counter is tamper-**evident** against casual edits (HMAC with a per-install key kept in the same
  file). Whoever can read and write the file can forge a record, delete `gate.json` (the gate is then off) or
  restore an older copy of the whole data folder (the counter resets). Within one run, rolling the file back is
  noticed (the core remembers the highest epoch and count it wrote).
* Not configured, or `gate.json` unusable (not JSON, bad hash, wrong version): the file is moved aside as
  `gate.json.corrupt-<ms>` and the gate is **off** (fail open for the screen, never for the vaults, which stay
  behind their master passwords). A damaged *counter inside* a usable file is a penalty, not a reset.

## 6. Module map

```
src-tauri/src/
  main.rs            desktop entry (calls hexis_lib::run)
  lib.rs             Tauri builder, command registration, `pub mod crypto;`, the window hooks (close flush on Linux, `Suspended` flush on phones)
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
  storage/           Phase 2 + accounts + 4a [done]
    app.rs           AppCore: the account registry, the selected profile and its unlocked Session, every command's logic
    profiles.rs      profiles.json registry: display-name rules (NFKC, unique, 1 to 40), legacy vault.db adoption, corruption quarantine
    files.rs         atomic and owner-only file helpers, the FilePicker trait, vault-file header checks
    db.rs            SQLite schema v3 (rusqlite, bundled): items, versions, tombstones, targets, outbox, remote_objects; WAL, secure_delete, owner-only
    model.rs         payload schema 1, drafts, field schemas per item type (login, card, identity, sshKey, note, media), limits, wire types
    vault.rs         unlocked vault: items, history, trash, archive, clone, purge, TOTP, in-memory search index
    session.rs       lock state, the re-prompt guard (Argon2id check per gated call), single-use edit tokens
    uuid.rs          UUIDv4 from the OS CSPRNG with a strict validator
    error.rs         StorageError, serialized as {code,message} (no internals leak)
    links.rs         open_url validation (http and https only) + the UrlOpener seam
    backup/          Phase 4a: S3 backup targets, single-writer replication, restore
      types.rs         wire types + input validation (https-only endpoints)
      remote.rs        ObjectStore seam, key layout, secret-free errors; (tests) MemStore with fault injection
      s3.rs            real client: rusty-s3 presigning + blocking ureq/rustls, no redirects, caps, timeouts
      dbops.rs         SQL for targets / outbox / remote_objects and restore inserts
      probe.rs         connection + conditional-write probe (no unlocked vault needed)
      replicator.rs    queue drain, single-writer rule, health, and the timer-free worker thread (sleeps on a condvar; wakes only on unlock, a committed change or the exit flush); `Timing` lives here
      pull.rs          Q0 pull and reconcile: list, compare ETags, GET only what changed, lineage walk, `VaultLink` (how the worker reaches the unlocked vault)
      pull_apply.rs    Q0 vault side of a pull: authenticate every remote object, classify, transactional race-safe apply (tombstones, new items, fast-forward, other-device copies)
      vault_ext.rs     sealed target settings, shared-targets snapshot, BackupHandle owned by Vault
      restore.rs       download a backup into a new vault file
      prefs.rs         the vault's preferences (the generator's last settings): sealed in `meta`, pushed as `vault/meta/prefs.enc`, pulled by one HEAD per pass, newest write wins
    tests.rs, app_tests.rs   storage and account tests
    (backup/) tests.rs, tests/pull_tests.rs, mock_tests.rs   queue, no-timer and two-device pull tests on MemStore; the real S3Store against mock_s3.py
  commands/          [done]
    mod.rs           thin IPC layer (`invoke_handler()`), the production `DialogPicker`, `open_url`
    tests.rs         end-to-end over Tauri's mock runtime (real command dispatch and arg conversion)
  examples/e2e_server.rs   DEV-ONLY loopback HTTP shim around the same command layer, for headless-browser end-to-end tests
  capabilities/mobile.json phones only: barcode-scanner permissions
  gate/              Q1 [done] Stage 1 (privacy gate only: it holds no key). Tauri-free; AppCore owns one GateCore.
    mod.rs           GateCore: state machine (closed/open), allow-list, write-ahead security attempt, configure, panic
    credential.rs    credential rules (= src/lib/gateRules.ts), NFKC text, Argon2id PHC hashes, equal-cost 3-way verification
    counter.rs       gate.json: hashes + flags + attempt counter record (epoch + HMAC-SHA-256 tag), atomic 0600 writes
    equation.rs      decoy maths by construction (exact integers), seedable RNG seam for tests
    wipe.rs          secure_wipe_all_local_data: pre-flight, cryptographic erase first, zero-overwrite, known paths only
    idle.rs          the inactivity timer (condvar until deadline; monotonic + wall clock)
    settings.rs      app-settings.json (lockAfterMinutes in {1,5,15,30,60,240}, lockAfterLeaveSeconds in {0,15,30,60,300,900,1800,3600,14400})
    biometric.rs     BiometricAuth seam; NoBiometric on desktop; PluginBiometric (tauri-plugin-biometric, Android/iOS only, UNVERIFIED)
  clock/             Q5 [done] trusted time (D27). Tauri-free.
    mod.rs           TrustedClock: boot clock + checked UTC, `now_ms`, `verified_utc_ms`, `deadline_utc_ms`, the status the screen may see
    boot.rs          the boot clock (`/proc/uptime` + kernel boot id on Linux and Android; a per-run id elsewhere) behind the BootClock trait
    net.rs           the HTTPS `Date` header fetcher (google.com/generate_204, cloudflare.com/cdn-cgi/trace, aws.amazon.com), quorum and sanity rules
  storage/device.rs        Q5  device.json: time zone, "setup done", time-check switch, the demo vault's id (D28, D29); survives the erase
  storage/demo.rs          Q5  the believable items of the demo vault (15 to 25, pinned by a test) ("Hexis" persona), made with random passwords and secrets
  storage/app_demo.rs      Q5  AppCore: device_*, clock_*, demo_spawn / demo_open, the demo mark on profiles
  commands/device.rs       Q5  device_get / device_set_time_zone / device_finish_setup / device_set_time_check / clock_status / clock_sync / demo_spawn / demo_open
  storage/app/gate_api.rs   the AppCore glue (wipe order, gate_configure behind the master password, gate_close, status neutral while closed)
  commands/gate.rs          the 10 commands + `guarded`, the deny-by-default wrapper around the whole invoke handler
  disguise.rs        Q2 [done] presets (embedded presets.json + 512/128 px PNGs), DisguiseInput/View, validation, the plaintext
                     disguise.json (load/save/wipe), the Launcher seam (NoLauncher, RecordingLauncher), PNG decoding for window icons
    disguise/desktop.rs  Linux launcher entry writer: <data home>/applications/<id>.desktop + icons, Desktop Entry escaping,
                     ownership marker X-Hexis-Managed=true, best-effort update-desktop-database, remove_launcher_entry()
  storage/app_disguise.rs  AppCore::disguise_get / disguise_set / disguise_apply_saved / disguise_wipe, the Launcher slot, one lock for all disguise operations
  commands/disguise.rs     disguise_get / disguise_set, TauriLauncher (window title+icon, Linux entry, Android alias bridge), install() from setup, android_plugin()
  scripts/android_patch_manifest.py  applies the disguise to a generated Android project (aliases, icons, labels, DisguisePlugin.kt); tests in scripts/tests/
  crypto/stream.rs              Q3  chunked blob encryption (PMDF v1, 1 MiB XChaCha20-Poly1305 chunks, 50 MB limit)
  storage/drive.rs              Q3  file types, metadata, staging/publishing blobs, read/save, files table SQL, schema migration, purge/seed hooks
  storage/vault_drive.rs        Q3  impl Vault: file_* (keys, sealed metadata, list, add 3-phase, rename, delete, startup indexing)
  storage/session_drive.rs      Q3  child of session.rs: add/read/save with the vault lock held only for the short steps
  storage/app_drive.rs          Q3  child of app.rs: file_* on AppCore (pickers, path rules)
  storage/backup/files_sync.rs  Q3  outbox jobs file / file_meta / file_purge, S3 blob+meta sync, keyless restore of files
  commands/drive.rs             Q3  file_* and screen_secure commands (raw-bytes reply for file_read), Android plugin glue
  scripts/android/SecureScreenPlugin.kt   Q3  FLAG_SECURE plugin source (UNVERIFIED)
  drive/             Q3 [queued] 1 MiB chunked XChaCha20-Poly1305 files, attachments, preview and save-copy streams
  importer/          Q4 [done] untrusted-file parsers (all in memory, bounded, panic-proof) and the plaintext export
    mod.rs           ImportFormat (25 ids), options/results types, ParsedImport, parse() dispatch, AttachmentSink + NoSink
    limits.rs        every size limit and every fixed error sentence
    entry.rs         ImportedEntry (zeroizing) -> ItemDraft in one place; DupKey (the duplicate rule)
    sniff.rs         format from content; text.rs charset/delimiter/CSV reader; zipsafe.rs bounded zip reader
    csv_import.rs    all CSV formats (one column finder); bitwarden.rs, keepass_xml.rs, kdbx.rs (+ key derivation guard)
    proton.rs, onepux.rs, vendors_json.rs (Dashlane, Enpass), authenticators.rs (Aegis, 2FAS, otpauth lists), otp.rs (migration links)
    jsonutil.rs      lenient scalars, depth check, ISO times; export.rs plaintext Bitwarden JSON / CSV; tests.rs fixtures, limits, fuzz
  storage/vault_import.rs    Vault::import_batch (one transaction), duplicate index, export_records
  storage/session_import.rs  handle table + parsed previews inside the unlocked session; export_plain_with
  storage/app_import.rs      AppCore: import_pick, import_pick_keyfile, inspect_csv_headers, import_preview, import_commit(_with_sink), import_cancel, export_plain
  commands/importer.rs       the seven Tauri commands
  (sync lives in storage/backup/; event-driven since Q0, see D18)
src/                 React + TypeScript + Tailwind UI "Strongroom" (see UI_DESIGN_SYSTEM.md; Tailwind classes only, see CSP)
  backend/           the `Backend` interface (types.ts), the Tauri adapter (tauri.ts), and a DEV-only in-memory
                     preview implementation (mock.ts, loaded only when import.meta.env.DEV and not in Tauri)
  state/             tiny external store, app state machine (incl. the Stage 1 gate), brand (disguise), activity watcher, editor request store, DEV deep links (dev.ts)
  lib/               format (zone-aware), zones (IANA list, offsets, search), shortcuts (the one table of keys), motion (one easing, three durations), password generator + strength, TOTP ring maths, clipboard auto-clear, useLayout
  styles/            tokens.css (@theme) and base.css (ground, flat surfaces, key surface, focus; the button reset sits in `@layer base` so utilities win)
  components/        ui/ (Button, Inputs, Avatar, Ring, Sheet, Toasts), lock/, shell/ (Sidebar, Rail, BottomNav,
                     Fab, split panes, shortcuts.ts), items/ (list, detail, menu, editor, history, codes), gate/ (Stage 1), files/,
                     tools/ (generator, import, export), backup/, trash/, settings/ (layered: a list of pages on phones, a column of pages on wide screens), setup/ (zone page, first-run sheet),
                     help/ (shortcut list and overlay), reprompt/ (the guard dialog)
assets/disguise/     presets.json + 11 icon sources (generated by scripts/gen_disguise_icons.py); PNGs in src-tauri/icons/disguise/
src/assets/         logo.svg (full squircle icon), logo-mark.svg (transparent H, used in the app), logo-mark-mono.svg: the vector sources
assets/app-icon*.png raster sources rendered from them; `python3 scripts/gen_icons.py` regenerates src-tauri/icons (all platforms)
scripts/
  monitor.py               read-only live monitor for long jobs (`--once`, `--log`, `--lock`)
  crypto-harness.py        build/test/clippy the crypto module without GUI libraries
  gen_crypto_kat.py        regenerate/--check kat_vectors.rs from independent Python implementations
  verify_bundle_fixture.py pure-Python unlock of the Rust-written fixture bundle (interop proof)
  gen_icons.py             renders src/assets/*.svg to every PNG/ICO/ICNS and Android adaptive icon (needs rsvg-convert)
  release_notes.py         writes the GitHub release description from CHANGELOG.md and SHA256SUMS
  gen_disguise_icons.py    draws the 11 disguise icons and presets.json (needs rsvg-convert)
  mock_s3.py               stdlib SigV4-verifying S3 mock for dev and tests
  check_no_notifications.py  fails if a notification or tray plugin, permission or Android manifest entry appears (D18)
ai-docs/             these documents
```

## 7. Phase roadmap

1. **Phase 1** — docs, scaffold, `crypto/` (done).
2. **Phase 2** — local storage, immutable version history, trash lifecycle, re-prompt guard commands, and the full adaptive UI (done).
3. **Accounts and backup (4a)** — several accounts per device, more item types, archive, S3 targets with a master backup, restore (done; real providers not yet exercised).
4. **Queue Q0 to Q4** (`FEATURE_QUEUE.md`): event-driven sync, Stage 1 gate + panic wipe + strict lock separation, icon disguise, 50 MB encrypted drive, Rust importer/exporter. The frontend for all of them exists against the preview backend; the Rust slices are built one at a time. **Q0 is built (2026-11-24):** timer-free sync, the exit and background flush, pull and reconcile with other-device copies (`STORAGE_AND_SYNC.md` §6, §10).
5. **Later** — conflict UI beyond the reconcile-on-unlock rule (4b), Android project init and the native pieces that need it (`FLAG_SECURE`, BiometricPrompt, checking the `onPause` flush on a device (`BUILD_TARGETS.md` §8; `onStop` is not forwarded), launcher aliases), bundles and the release checklist.

Crates are added in the phase that uses them so builds stay light on the low-RAM dev machine.

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
| D9 | The product is **Hexis**, the app identifier `dev.habibullah.hexis` (decided 2026-11-24; the first builds used a working name). | An Android applicationId cannot change after a release. The old desktop data folder (`app.passman.vault`) is adopted on first start (`storage/legacy.rs`). The stored format keeps its historical identifiers (the `passman/v1/...` HKDF labels, the `PMV1` file id): changing them would make every existing vault unreadable, so they are frozen, see `CRYPTO_SPEC.md`. The repository's address, `post-quantum-passman-gst`, was chosen for search and is only a URL (2026-11-24); it is not the product name. |
| D10 | **Changed 2026-11-24.** An open vault shows, copies, edits and clones ordinary items without a prompt; an item with its own `reprompt` flag asks every time (`password_required`, then the UI sends the password), and destructive or exporting commands always ask. A sent password is always verified with Argon2id; nothing is cached on either side. Edits use a single-use token bound to item and version (10 min TTL, cleared on lock). | After QA on a phone the person found a prompt for every copy unusable, and asked for Bitwarden's rule. The flag sits inside the sealed payload. There is still no "recently confirmed" state to steal or bypass; the token stops a stale or replayed edit. A flagged item's live code is withheld from lists too (`totp_now` asks). |
| D11 | UI direction "Strongroom": dark only, bespoke tokens, three adaptive layouts, one orchestrated animation (unlock). **Revised 2026-11-24 (Q5): flat surfaces, no glass blur, lattice or glow; every transition 80 to 180 ms.** | The user rejected a terminal or generic look and asked for a 1Password / Proton Pass / Keychain feel with glass, crisp type and tactile elevation. Details in `UI_DESIGN_SYSTEM.md`. |
| D12 | The frontend talks to a `Backend` interface; a DEV-only in-memory implementation exists for design review and is excluded from production bundles | Lets the UI be reviewed in a browser at Android widths without the GUI stack. Guarded by `import.meta.env.DEV`; verified absent from `dist/`. |
| D13 | Secrets reach the UI only inside a 20 s reveal window held in component state; TOTP secrets never reach it | Limits exposure in the webview; the core stays the only holder of long-lived secrets. |
| D13 | Stage 1 (passcode / fingerprint) is a privacy gate only. It never caches or wraps the VaultRootKey; every vault open and every re-open after the idle timer needs the master password. **Amended 2026-11-25 by D45:** an account PIN may show a vault that is *still open in RAM*; it still never wraps, caches or derives a key, and nothing opens a vault from disk except the master password. | The user's rule of 2026-11-23. A weak, short credential must not become a shortcut to the keys. Supersedes the soft-lock / quick-unlock design. |
| D14 | The decoy maths: 1 to 3 characters are never judged as a credential; 4 or more are. The core generates the equation and holds the answer. | Keeps the app looking like a plain exercise to a bystander and keeps the attempt counter to real security attempts. |
| D15 | The attempt counter is written and fsynced before the credential is checked, and carries a MAC. It resists force-closing and casual edits, not someone who copies the data folder. | Honest limit: files on a PC or phone cannot be made tamper-proof. The cryptographic protection is the master password and the hybrid envelope. |
| D16 | The wipe is a cryptographic erase plus best-effort overwrite of known Hexis paths only, and never touches remote S3 copies. | Flash and copy-on-write file systems cannot prove an overwrite reached old blocks; destroying the key bundle is what makes the rest unreadable. |
| D17 | The disguise embeds `assets/disguise/presets.json` in the core and the UI; the app's own screens follow the chosen name and icon. | One source of truth for names and icons; a renamed app must not say "Hexis" on screen. |
| D18 | S3 work runs on four triggers only (unlock, every save, app exit or backgrounding, and the device coming back online, added in 0.4.0, see D42); no timers, polling or notifications. | The user's rule: no OS noise and no background daemons. Built in Q0: the worker sleeps on a condition variable, a failed job stays queued and waits for the next trigger, the pull runs at unlock and "Back up now", and `scripts/check_no_notifications.py` guards the no-notification rule. |
| D19 | Stage 1 is enforced in one deny-by-default wrapper around the whole IPC handler, not per command. | A forgotten check on a new command would be a bypass; with the wrapper a new command is closed until it is listed. |
| D20 | Gate credential hashes use Argon2id at min(vault cost, 32 MiB / t=3 / p=1) and three verifications always run. | The gate is a privacy screen, cheaper than the vault KDF on purpose (a 4 GiB phone, three hashes per attempt). Equal work for right, panic and wrong inputs (timing). `--fast-kdf` makes it fast in tests. |
| D21 | The attempt counter is written (and fsynced) **before** the input is looked at; a damaged counter costs one extra attempt, capped two below the limit, never a wipe by itself. | A force-close right after typing still costs an attempt; a corrupt file must not wipe a device. |
| D24 | The attempt counter also carries a wall-clock wait (`lockUntil`, same MAC, same write-ahead): 3, 6, 9, 12 and 15 wrong attempts start waits of 1.5 min, 5 min, 45 min, 8 h and 3 days, and the 16th wrong attempt wipes. While a wait runs the input is neither counted nor checked. | Slows guessing without the device being wiped at once. **Superseded in part by D27:** the wait is now measured on the boot clock and on checked UTC, so changing the device date no longer shortens it. Honest limit: editing the files can still shorten a wait, at the cost of a counted attempt; the wipe still applies. The screen warns before attempt 16 and asks for a confirmation. |
| D25 | The manual "erase everything" ends in a long sum made and held by the core (answer 100 to 99,999; `app_reset_challenge`, `app_reset(answer)`). Any answer uses the sum up; only a right one erases. | Three taps are too easy to do by accident or by a bystander; the manual reset is reachable without logging in. |
| D26 | On phones the core runs a second timer for "after leaving the app" (`app_leave` / `app_return`), default 5 minutes, "right away" to 4 hours. The webview only reports leaving and returning. | The old rule (lock the moment the page is hidden) asked for the master password after every glance at another app. The timer lives in the core, so it still fires when the webview is frozen. |
| D22 | A refused wipe (pre-flight) never opens the gate: the vault is locked, the gate stays closed, the error is the fixed sentence `storage` "This device could not be erased." | Fail closed. Only reachable when the data folder is not where Hexis put it. |
| D23 | Drive files are plain `Vault`/`Store` features in their own child modules; a file add is three phases (prepare under the lock, encrypt without it, commit under it) and a lock or account switch cancels it. | Encrypting up to 50 MB must not hold the session lock; locking must always win. |
| D27 | **Trusted time (2026-11-24).** Waits after wrong passcodes are two deadlines that must both pass: one on the boot clock (`/proc/uptime`, valid within a boot, cannot be moved by the date setting) and one in checked UTC (valid after a restart). Checked UTC comes from the `Date` headers of three public HTTPS sites; it needs at least one answer, a disagreement over 15 s between two answers rejects the check, a round trip over 5 s is dropped, and anchors live in memory only. An unknown wait **fails closed** (`clockPending`: no attempt is judged until the time was checked). | The person asked that spoofing the device clock must not bypass the waits. Changing the date moves neither clock. Honest limits: someone who can edit `gate.json` can forge it (D15); a device restart needs one successful time check before a UTC-only deadline can be judged; Windows and macOS use a per-run boot id (an app restart there needs a check); Android's `/proc` readability is unverified. With the check **switched off** (Settings, Date and time) no request is made and a restart is judged by the device clock, so a person who turns it off accepts that a changed date can shorten a wait after a restart. |
| D28 | **Times are stored in UTC and shown in the person's zone (2026-11-24).** Every stored time is milliseconds since 1970 (UTC). The zone (an IANA name such as `Asia/Dhaka`) is chosen on the first page and changeable in Settings; it lives in `device.json` and only changes how times are written on screen and how a time the person types is read. The core validates the name's shape only; the webview's own `Intl` data resolves it. | One format on disk and in backups means a device in another zone, or a moved clock, never changes what is stored. There is no date input in the app today; when one appears, it converts from the zone to UTC at the boundary (`lib/format.ts`). |
| D29 | **The demo vault (2026-11-24).** An app with no vault makes one named "Hexis123" (password "Hexis123", public on purpose) holding about two dozen invented items for one person, Hexis Alden. It opens without a typed password (`demo_open`, only for the id in `device.json`), is never deleted automatically, and after an erase or a forgotten password the app lands in a fresh one. | The person asked for a first start (and an erase) to look like a normal, populated vault instead of a create / import / restore screen, so a bystander sees nothing unusual. It holds nothing real. `demo_spawn` refuses while any vault exists. |
| D30 | **`device.json` survives the erase (2026-11-24).** The wipe knows only its fixed list of files and `device.json` is not on it; only the demo vault's id is forgotten. | A reset must not bring the zone page back (that would announce the erase). The file holds a place name, two switches and a random id. |
| D31 | **The generator's last settings belong to the vault (2026-11-24).** The Generator page and the item editor start from what was chosen last (length and character types); the settings are one small sealed object (`prefs`) in the vault database, replicated to every target as `vault/meta/prefs.enc`, pulled with one `HEAD` per pass and restored with the vault. The newest write wins (the envelope version is a millisecond stamp kept above the version this device holds). Unknown fields are carried through; a copy that fails authentication is never applied; a copy a newer app wrote in a shape this build cannot read is left alone. | The person asked that a chosen length (30, say) show in every place next time, be stored in the database and sync. A device-local `localStorage` value would not follow the vault or reach another device. The object is not an item (no history, no trash, no tombstone) and rides the existing `targets` outbox kind with the ref `prefs`, so no schema migration was needed. A merge is not worth building for one small document: two devices that change it together keep the later change. |
| D32 | **The web view keeps nothing (2026-11-25).** The window is created with `"incognito": true` (`tauri.conf.json`), so on Linux WebKit keeps cookies, cache and local storage in memory only; on Android the WebView's data is private to the app and is removed with it. The front end uses no `localStorage`, `sessionStorage`, cookie or IndexedDB (an e2e step checks this). Builds up to 0.2.0 let the web view write into the data folder (`localstorage`, `WebKitCache`, `CacheStorage`, `storage`, `mediakeys`, `hsts-storage.sqlite`, ...); `storage::webview_data::remove` deletes exactly those fixed names at every start and the wipe calls it as well. | The person asked that an uninstall leave no cache, storage or cookies behind. The two settings that lived in the web view (clipboard time, theme) moved to `device.json` (D33). Honest limits: the incognito flag was checked against the `wry` source and by an end-to-end run of the screen in headless Chromium, **not in a real WebKitGTK window**; the cleanup deletes only the fixed names directly inside the data folder (a link is removed as a link, never followed); system-wide caches (a font cache, the desktop's recent-files list) are not Hexis's and are not touched. |
| D33 | **Theme and clipboard time are device settings (2026-11-25).** `device.json` gains `theme` (`system`, `light`, `dark`; default `system`; a damaged value reads as `system`) and `clipboardClear` (15, 30 or 60 seconds; default 30). `device_get` is allowed while the gate is closed, so the lock screens are drawn in the chosen theme; the setters are closed with it (D19). On desktop the core also tells the native window (title bar, dialogs) through a hook. | They used to be web view storage, which D32 removes. Putting them in the core also means they are there before the first screen, so there is no flash of the wrong theme. The clipboard time starts at 30 once after the upgrade (the old value is not read from the old storage). |
| D34 | **"Remove Hexis from this device" (2026-11-25).** `app_prepare_uninstall(confirmedSteps, answer)`: the same three confirmations and the same long sum as the erase (D25), the same wipe (every vault, app lock, settings and launcher entry overwritten with zeros, then unlinked), then also what an erase keeps on purpose (D30): `device.json`, the web view's leftovers and the data folder when it is empty. Nothing is made again afterwards (no demo vault). On desktop the app closes about 0.9 s later; on a phone the screen stays on its last page and says to uninstall. Not on the closed-gate allow-list. S3 backups are never touched. | The person asked that before uninstalling, everything is cleared, or the uninstall step says that nothing is kept. An antivirus cleaning leftovers afterwards is exactly what this avoids. Honest limits: files are overwritten once with zeros and unlinked, which does not defeat forensic recovery on an SSD or a journaling file system; other accounts on the machine keep their own data; the backups in S3 stay until the person deletes them at the provider. |
| D35 | **The installer's `--uninstall` removes the data too (2026-11-25).** It lists every folder it will remove (the data folder under `$XDG_DATA_HOME` or `~/.local/share`, the same names under config, cache and state, the launcher entry that carries Hexis's own marker line), asks for the word `REMOVE` on the terminal (`--yes` confirms in advance, `--keep-data` removes only the program), refuses while Hexis is running, and removes only paths whose last name is one of Hexis's two identifiers (the current one and the one before the rename) or the launcher file. A package manager alone (`apt remove`, `dnf remove`, `pacman -R`) still removes the program only, as every distribution requires, and the docs say so. | The earlier uninstall left the vaults on purpose; the person now wants nothing kept, and said an uninstall step should state that. Typed confirmation because a vault cannot be brought back. |
| D36 | **Android keeps nothing across a reinstall (2026-11-25).** The manifest patch sets `allowBackup="false"` and `fullBackupContent="false"` and adds a `dataExtractionRules` file that excludes everything (cloud backup and device transfer), so a restored install cannot bring a vault back. The activity also lets the page tell the system bars which colours it draws (`HexisBars.setLook`, D37). | Auto Backup would otherwise copy the data folder to the person's Google account and restore it after a reinstall, which defeats "nothing is kept". Not built or run on a phone in this round. |
| D37 | **Two complete palettes (2026-11-25).** Light and dark are both full palettes defined in `src/styles/tokens.css` as CSS variables; Tailwind v4 theme colours compile to `var(--color-*)`, so one attribute switches everything. `<html data-theme="light|dark">` chooses; with no attribute the stylesheet follows `prefers-color-scheme`. The accent used as text, line or dot is a separate token, `brass-ink` (darker in light so it reaches 4.5:1), while `brass` stays the button fill in both. Shadows are plain variables with hand-written utilities, because Tailwind inlines theme shadows. `light-dark()` is not used (WebKitGTK 2.4x lacks it). `src/styles/tokens.test.ts` keeps the copies identical and checks the contrast of every text colour on every surface in both palettes. On Android the activity sets the status and navigation bar icon colour from what the page draws. | The person asked for a proper light and dark theme instead of dark only, "properly completely". The four copies of the palette (dark `@theme`, dark island for previews, light for a chosen theme, light for "follow the device") are the price of no flash and no script before the first paint; the test makes a drift fail the build. |
| D38 | **Folders are paths, not ids (2026-11-25).** A folder is a path of names joined with `/` (at most 6 levels, 64 characters a name, 128 in all). An item carries its path in the `folder` field it already had; the folders made on purpose live in the synced preferences (`prefs.enc`, `folders`). The folders that exist are the union of both plus every parent. Removing a folder moves what is inside up one level; nothing is deleted. | No new payload field (an older build must still open every state; `ItemPayload` rejects unknown keys), no id to remap when two devices sync, and an import keeps Bitwarden's `Work/Clients` as it is. The price: two devices that each create a different **empty** folder offline can lose the earlier one (the list is one newest-write-wins value). A folder holding items is never lost, because the items carry the path. |
| D39 | **A diverged item is merged, not parked (2026-11-25).** When the same item changed on two devices, the pull keeps the other device's states as snapshots and writes a **merged version** that descends from both heads (`storage/merge.rs`; `ai-docs/STORAGE_AND_SYNC.md` §6). Three-way against the newest common state: one side changed it, take that side; both changed it differently, the newer `updated_at` wins (a tie takes the greater value, so two devices agree) and the loser stays in the history; a value removed on one side and edited on the other keeps the edit; tags, websites, attachments and custom fields are united; notes are merged line by line with a three-way diff and, where both sides changed the same lines, both versions are written out between markers. A merge is recorded as event `edited` with `restored_from` = the copy's number, so no payload or event variant was added. Items of different types are not merged. | The person asked for Google Keep behaviour: no screen to resolve, no data lost. Both earlier states stay in the history, so a wrong guess is one restore away. Using existing fields keeps 0.3.0 builds able to read everything. Limits: a note both sides rewrote the same lines of needs a human tidy (the markers say so); "newer" depends on the device clocks, which is why every earlier state is kept. |
| D40 | **Permanent delete: 183 days, a sum from the core (2026-11-25).** `item_purge_plan` says whether the item went to the trash less than 183 days ago on the core's trusted clock (a time that cannot be placed in the past counts as recent) and, if so, makes a long sum whose answer never leaves Rust; `item_purge(id, password, answer?)` checks the password first, then the sum, which any wrong answer uses up. Older items need only the password. The typed `CONFIRM` is gone. | The person asked for a master password plus a complex sum inside six months and only the password after. Reusing the long-sum generator of the reset keeps one mechanism. Honest limit: the age rests on the device clock when the online check is off, so someone who can move the clock can make an item look old; the master password is still needed either way. |
| D41 | **Search runs in Rust and answers with ids and field names (2026-11-25).** `item_search(query, oldVersions)` reads each item in the core and answers with the id, up to three names of the fields that matched and, for an earlier version, its number; never a value. Titles, folders, tags, the type, every non-secret field, custom-field names, the values of custom fields that are not hidden and the free-text note fields are compared; passwords, card numbers, codes, PINs, identity numbers, seeds, private keys and hidden custom values never are, and the notes of an item that asks for the master password stay sealed. Lower-cased copies used for comparing are wiped. | Only the core can read an item, and the rule "search must not become a way to read a secret" is kept as data with a test. Showing *where* it matched ("Found in Notes") needs no value. |
| D42 | **A fourth sync trigger: the device came back online (2026-11-25).** The page calls `sync_reconnect` from the window's `online` event; the worker pulls, merges and sends the queue. It is still event-driven, with no timer, poll or retry schedule. | The person asked that work done offline syncs on reconnect. The system tells the page when the connection returns, so nothing has to wake up to check. A reconnect with the vault locked does nothing. Not verified in a real WebKitGTK window or the Android WebView (the `online` event is standard in both; only headless Chromium was used). |
| D43 | **Import can merge matching accounts, and does by default (2026-11-25).** `ImportMode::Merge` matches an entry to an existing item by one-time-code secret, or by the same account (case-insensitive) with an alike name, and merges into it as a new version, keeping every value: a differing value goes into a custom field `<Label> (imported)`, notes are joined, websites, tags and custom fields are united. A Totp-only item and a Login of the same account become one Login in place (same id, the type changes, old versions as they were). The other three modes never match across kinds. | Bitwarden has the login and Ente Auth has the code and its notes (which can hold recovery data) for the same account; the person wants one item and no lost note. Nothing is overwritten, and importing the same file twice changes nothing. |
| D44 | **How the list opens and is ordered are device settings (2026-11-25).** `device.json` gains `openInSearch` and `listSort`; the screen sorts and filters what the summaries hold, and the core does the deeper search. | They are about this device, not the vault, like the theme. Older builds ignore the two keys. |
| D45 | **The account PIN, a middle layer (2026-11-25; the user chose "PIN reopens accounts held in memory", amending D13).** Per account, optional, an Argon2id hash in `profiles.json` that touches no key. Lock or switching away keeps a PIN account's session in RAM (`Session` parked, in `AppCore.parked`) until the idle timer, leaving the app, the app lock, log out, exit or five wrong PINs; the PIN shows it again **limited**: every secret and every backup, sync or lock-timer change answers `password_required` or `master_required` in the core until `vault_elevate` takes the master password. Accounts without a PIN are wiped on Lock and on switching, as before. No key is ever written anywhere new. See §5.1. | The user wanted to switch accounts and unlock one without typing a long master password every time, while the master password stays the only way in from nothing and the only way to anything sensitive. Wrapping a key under the PIN (the other options offered) would have made a short code a way into the files. |
| D46 | **A login can hold several sign-ins and several one-time codes (2026-11-25).** Repeats are numbered keys in the same `fields` map: `username`, `username#2` …, `password#n`, `totp#n` (login only, at most 10, canonical spelling only: `totp#1` and `totp#02` are refused). A sign-in is the username and password with the same number; codes stand alone. Gaps are tolerated by the core and closed by the editor on save. Other apps' exports get the extras as hidden custom fields ("Username 2", "Password 2", "One-time code 2") and Hexis restores them on import. `ItemSummary.codeKeys` lists the code keys; `totp_now` takes an optional `key`. No schema change: the stored version is still 1 and an older build keeps the extra keys it cannot name. | The editor offered one password and one code. Reusing the one map avoids a new item type, a migration and a second merge path: the three-way merge already works per key. |