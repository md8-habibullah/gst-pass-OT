# FEATURE_QUEUE — specs received 2026-11-23 and how they are built

Six specs arrived after the Phase 2 UI and the accounts/backup work. They build on each other (later ones say "keep the earlier ones intact"), and **none of the earlier ones existed in the code when they arrived**, so they are built in the order below, each as a verified slice. Rust slices run one at a time (low-RAM machine; they also share `storage/app.rs`, `commands/mod.rs` and `lib.rs`). Status lives in `CHANGELOG_AND_STATE.md`; this file is the source of the acceptance criteria and of the decisions that deviate from, or sharpen, the wording of a spec.

Order: **Q0** event-driven sync (adapts the backup work already in flight) → **Q1** Stage 1 gate + strict lock separation + wipe/panic → **Q2** disguise (names, icons, launcher) → **Q3** encrypted drive → **Q4** importer/exporter.

**Status (2026-11-24): Q0, Q1, Q2, Q3 and Q4 are all built, merged into one tree and verified** (what was and was not verified is in `CHANGELOG_AND_STATE.md`; the finished contracts are in `IPC_CONTRACT.md` §7 to §10 and `STORAGE_AND_SYNC.md` §11 to §14). The sections below stay as the record of what was asked and of the decisions that sharpen the spec text; where they say "queued" or "not built", read "built". Still open: running the Android build on a device (toolchain not complete yet), the Android `content://` file pickers, and real-vendor checks listed under "NOT verified".

---

## Q0 — Zero notifications, three sync triggers only

Acceptance:
* No push notifications, tray icons or background daemons. No `POST_NOTIFICATIONS` (nor `FOREGROUND_SERVICE*`) in the Android manifest; no notification or tray plugin on Linux.
* S3 work happens on exactly three triggers: **(1)** vault unlock: pull and reconcile; **(2)** every create, update (new version) or move to Trash: push to the master and all other enabled targets; **(3)** app exit or backgrounding: flush the pending queue. Linux hooks the window `CloseRequested` event; Android hooks `onPause` / `onStop`.

Decisions:
* The existing queue worker keeps working **only while one of those triggers is draining it**: no timers, no polling, no periodic retry. A job that fails stays queued with its error and is retried at the next trigger. The explicit **Back up now** button stays: it is the user pulling a trigger by hand, not a daemon.
* Flush on exit is bounded (about 8 s total, then the app closes anyway and the queue survives on disk). On Android `onStop` has no guarantee of running to completion; the queue is durable, so nothing is lost, it just waits for the next trigger.
* Reconcile on unlock (Phase 4a made single-writer safe; this adds the merge): per item compare the remote head with the local head. Remote strictly ahead (local head is an ancestor) fast-forwards local. Local ahead is pushed. **Diverged**: the local head stays current and the remote head is stored as an extra immutable version labelled as coming from another device, so no edit is ever dropped; the item shows a "Changed on another device" note.

## Q1 — Stage 1 gate, strict lock separation, wipe, panic, decoy math

### Two stages, never mixed
* **Stage 1 (optional, Settings toggle):** unlocks only the app shell and shows the account/vault list. It holds no key and unlocks no secret. **Stage 2:** opening any vault, or re-opening after the inactivity timer, **always** needs the full master password. Keys live only in RAM and are `zeroize`d when the vault locks or the app closes or backgrounds.
* **Nothing wraps or caches the `VaultRootKey`** in the Android Keystore or a Linux keyring. This removes the earlier preview-only "quick unlock PIN / biometric for the vault" path from the UI and from `IPC_CONTRACT.md`. A fingerprint can only pass Stage 1.
* Inactivity lock timer: configurable (1, 5, 15 or 30 minutes; there is no "never"), enforced in Rust, not in the webview.

### Credentials (Argon2id hashes only, never plaintext)
Three modes, default **Numeric PIN**: **A** numeric PIN, **B** alphanumeric password (`inputMode="text"`, full keyboard), **C** numbered 3×3 grid (tap = digit, drag across ≥ 2 nodes = pattern serialised as digits, e.g. `12589`). Every credential, and the optional custom panic code, must be **≥ 4 characters / nodes**. Biometric (Android) works alongside all three.

### Decoy math and the 3-character rule (Rust decides, the webview never sees the answer)
* The core generates the equation (2 or 3 integers 1–100, operators `+ - * /`, brackets, all divisions exact, result always 1–999) and keeps the answer. The UI only sends what was typed.
* Input of **1–3 characters**: a math attempt. Never checks a credential, never touches the attempt counter. Right answer: subtle "Correct!" and a casual-streak counter; wrong: "Incorrect, try again"; either way a new equation.
* Input of **≥ 4 characters**: a security attempt, in this order: real credential → open the vault list; panic credential (reversed credential or custom panic code) → silent wipe and a clean empty app; otherwise → persistent failed counter +1, banner "N attempts left before this device is erased" (`15 − attempts`), new equation. The **15th** failure wipes.
* Submit is explicit (Enter or the submit key) because the length of the real credential must not be guessable from the UI. Flexible length (4, 5, 6, 7 or any) therefore falls out of the ≥ 4 rule.

### Counter
Stored in a small state file next to the registry, written and fsynced **before** the credential is checked (write-ahead), so force-closing mid-attempt cannot dodge the count; reset to 0 on success. It carries a MAC, so casual edits are detected. A damaged or hand-edited counter counts as one extra failed attempt (never as "exhausted"), so file corruption cannot wipe the device by accident. **Honest limit:** nobody can make a file on a PC or phone truly tamper-proof; someone who copies the data folder, then restores the copy, resets the counter. Stage 1 is a privacy gate. The cryptographic protection is the master password and the hybrid post-quantum envelope; the counter protects against casual access and force-closing, not against file copying.

### Panic / duress
* Trigger A, **reversed credential** (PIN, password or pattern in reverse). Needs the **palindrome guard**: when Reverse-Panic is on, a credential that equals its own reverse is rejected in Settings with a clear message.
* Trigger B, an optional **custom panic code** (≥ 4, may not equal the real credential or its reverse).
* Trigger C, an optional discreet lock-screen trigger (Settings toggle "Show subtle Panic trigger on Lock Screen"): long-press (≥ 1.2 s) **or** deliberate double-tap on a low-profile mark.
* Result of any trigger: `secure_wipe_all_local_data()`, then the UI goes straight to the first-run state. **No** banner, toast, log line or animation that says anything happened. Panic hashes live next to the credential hash, so a wrong guess costs the same time as a right one (both are always computed).
* Manual reset: a discreet icon / long-press on the equation header runs a **3-step confirmation** ("Forgot passcode" / "Reset app data"), each step a different warning, then the wipe. This one is allowed to say what it does.

### `secure_wipe_all_local_data()` (Rust)
Deletes **only known Passman paths under the configured data directory** (never `rm -rf <dir>` blindly, and refuses to run on a directory that does not look like a Passman data directory): every `vaults/<id>/` (SQLite, WAL, SHM, attachments), `profiles.json`, S3 credentials (inside the vaults), Stage 1 hashes, counters, app settings, disguise state and temp files. Files are overwritten (zeros) and fsynced before unlinking; the in-memory session is dropped and zeroized; the backup worker is stopped first; directories are removed; the app resets to "welcome". **Honest limit:** on SSDs, flash and copy-on-write file systems an overwrite cannot be proven to reach the old physical blocks. That is why vault contents are encrypted and why the key bundle (the only thing needed to read them) is destroyed first: the wipe is a cryptographic erase plus a best-effort overwrite. **Remote S3 copies are not touched** (that is also your recovery path; the local S3 credentials are gone, so restoring needs them again).

Tests only ever run against temp directories. The wipe function takes the directory from `AppCore`, never from a constant.

## Q2 — Disguise ("App icon and name") with 11 presets

Names, single word, with the icon each maps to:

| # | Name | Icon |
|---|------|------|
| 1 | Tally | minimal grid |
| 2 | Vellum | stacked paper sheets |
| 3 | Drizzle | minimalist cloud / drop |
| 4 | Trim | simple storage bar |
| 5 | Plinth | wireframe nested prism |
| 6 | Kerf | minimal dark cursor and dot |
| 7 | Basalt | geometric wireframe mesh |
| 8 | Cairn | hexagon circuit node |
| 9 | Kovren | two overlapping matte cards |
| 10 | Tivlo | 3×3 muted dot matrix |
| 11 | Orvyn | two off-centre dusk circles |

* Settings → "App icon and name": the 11 presets, a **lower-case display toggle** (`kerf`, `kovren`), and the custom-name field (kept). The lower-case toggle changes how the name is shown and written to the launcher label.
* Linux: a `.desktop` updater writes `~/.local/share/applications/<app-id>.desktop` (`Name=`, `Icon=` path to the preset PNG, `StartupWMClass`) and refreshes the desktop database; the window title follows the name.
* Android: one `<activity-alias>` per preset in `AndroidManifest.xml` (each with its own label and icon), switched by enabling one alias and disabling the others through `PackageManager.setComponentEnabledSetting`. The Android project is generated by `tauri android init`, so the aliases are applied by `scripts/android_patch_manifest.py` after init; **not verifiable here (no Android SDK)**.
* **Limit that applies to a custom name on Android:** launcher labels are static resources, so a free-text name can change the in-app title and recents label but not the launcher label; only the presets change the launcher label and icon. The Settings text says so.

## Q3 — Encrypted drive (Files tab + attachments), 50 MB per file

* Dedicated **Files** section plus attachments inside items. Limit **52,428,800 bytes** enforced in the UI and again in Rust.
* The webview never carries file bytes: the UI asks Rust to open the native picker, Rust reads and encrypts in **1 MiB chunks** with XChaCha20-Poly1305. Per-file key = HKDF(vault key, "file" ‖ file UUID). Per-chunk nonce is random 24 bytes; AAD = `suite ‖ file UUID ‖ chunk index (u64) ‖ total chunks (u64) ‖ last-chunk flag`, so reordering, dropping, duplicating or truncating chunks fails authentication. Plaintext size and filename are stored sealed in the metadata, not in the blob header.
* Blobs are stored locally and replicated to `vault/attachments/<uuid>.bin` on the master and every other target.
* Preview inside the app for images, text and PDFs (decrypted in RAM, dropped when the preview closes; `FLAG_SECURE` on Android). "Save decrypted copy" needs the master password re-prompt and writes a 0600 file.

## Q4 — Importer / exporter in Rust

* Parsers (crates `csv`, `quick-xml`, `zip`, `keepass`): KeePass `.kdbx` (password and keyfile asked in a modal, decrypted in memory, TOTP and attachments extracted) and KeePass XML; Bitwarden JSON/CSV; Proton Pass; 1Password `.1pux`/CSV; LastPass; Dashlane; NordPass; Enpass; Brave/Chrome/Firefox/Edge/Safari CSV; Aegis; 2FAS; plain `otpauth://` lists; `otpauth-migration://`. `inspect_csv_headers(path)` returns delimiter, headers and 5 sample rows for the manual column mapper.
* Everything is parsed in Rust from a file path (no plaintext crosses IPC except the preview summary). `ImportedEntry` derives `Zeroize` and `ZeroizeOnDrop`. This replaces the current TypeScript importer, which sealed items one at a time through the webview.
* Modes: `skip_duplicates`, `append_version`, `import_all`. Preview shows counts (entries, TOTP seeds, attachments, duplicates). One SQLite transaction; every accepted item sealed individually; all new or changed items are queued for sync; temporary plaintext zeroized.
* Export behind the master-password re-prompt: native encrypted archive, or Bitwarden-compatible JSON / CSV with an explicit plaintext warning and a 0600 file. (This reverses the earlier "no plaintext export" decision, at the user's request.)
* **Honest limit:** formats without a real sample on this machine (Proton Pass, Dashlane, Enpass, 1pux, Aegis, 2FAS, Safari) are written from the public descriptions and tested with synthetic fixtures; they are marked "not verified against a real export" until the user tries one.

---

## IPC additions for Q1 and Q2 (contract the UI and the core are both built against)

These move into `IPC_CONTRACT.md` when the core implements them. All of them are app-level (they work with no vault open) unless stated.

```ts
// ---- Q2 disguise ----
interface DisguiseInput { presetId: string | null; customName: string | null; lowercase: boolean }  // presetId null = the original icon
interface DisguiseView extends DisguiseInput { appliedName: string; note: string | null }           // note: what the launcher could and could not do
// disguise_get() -> DisguiseView        disguise_set(input) -> DisguiseView
// presetId must be one of assets/disguise/presets.json (the core embeds the same file); customName 1..30 chars, trimmed.

// ---- Q1 Stage 1 gate ----
type GateMode = "pin" | "password" | "grid";
interface GateStatus {
  enabled: boolean;            // the gate is configured and switched on
  open: boolean;               // Stage 1 passed in this run; the vault list may be shown
  mode: GateMode;
  biometric: boolean;          // available on this device AND switched on
  panicButton: boolean;        // show the discreet trigger on the lock screen
  equation: string | null;     // decoy math to show while closed, e.g. "(48 + 27) - 19"
  attemptsLeft: number | null; // null until the first failed attempt of this streak; otherwise 15 - attempts
  streak: number;              // casual math streak (cosmetic)
}
type GateOutcome =
  | { kind: "math"; correct: boolean; equation: string; streak: number }   // input was 1..3 characters
  | { kind: "opened" }                                                     // real credential (or biometric)
  | { kind: "failed"; attemptsLeft: number; equation: string }             // 4+ characters, wrong
  | { kind: "wiped" };                                                     // panic or 15th failure: the UI goes straight to first run, says nothing
// gate_status() -> GateStatus
// gate_submit(input: string) -> GateOutcome       (typed text, keypad digits, or a serialised pattern such as "12589")
// gate_biometric() -> GateOutcome                  (Android only; never counts as a failed attempt; "invalid_input" elsewhere)
// gate_close() -> void                             (back to Stage 1 and lock the open vault; "Lock app")
// app_reset(confirmedSteps: 3) -> void             (manual 3-step reset; same wipe as panic; allowed with the gate closed)
interface GateConfigInput {
  enabled: boolean; mode: GateMode;
  credential: string | null;       // new credential; null keeps the stored one (required when enabling or changing mode)
  reversePanic: boolean;
  panicCode: string | null;        // null keeps, "" removes
  biometric: boolean; panicButton: boolean;
  masterPassword: string;          // of the open vault: changing the outer gate is a gated action
}
// gate_configure(input) -> GateStatus
//   invalid_input when: length < 4 (characters, or nodes for a pattern); a pattern with a repeated node or a digit outside 1..9;
//   a PIN with non-digits; reversePanic on and the credential equals its reverse (palindrome); the panic code equals the
//   credential or its reverse; wrong_password when the master password is wrong.

// ---- Q1 inactivity lock (replaces the preview-only PIN / biometric quick unlock of the vault) ----
interface AppSettings { lockAfterMinutes: 1 | 5 | 15 | 30 }
// app_settings_get() -> AppSettings    app_settings_set(input: AppSettings) -> AppSettings
// activity_ping() -> void   (the UI calls it on input, at most every 10 s; the core enforces the timer and emits "vault-locked")
```

The wire format is the existing one (`{ cmd, args }` over Tauri IPC or the dev HTTP shim). The webview never receives the math answer, the credential hash or the attempt count beyond `attemptsLeft`.

Addition to Q1: `gate_panic() -> GateOutcome` is the discreet lock-screen trigger. It works only while "Show a discreet panic mark" is on (otherwise `invalid_input`), and returns `{ kind: "wiped" }` after the same wipe as a panic credential. `GateStatus` also carries `biometricAvailable: boolean`.

## IPC additions for Q3 (drive)

```ts
interface FileEntry { id: string; name: string; size: number; mime: string | null; itemId: string | null; addedAt: number }
// file_list(itemId: string | null) -> FileEntry[]     null = the standalone Files section, otherwise that item's attachments
// file_add(itemId: string | null) -> FileEntry[] | null
//     The core opens the native picker (several files allowed), rejects any file over 52,428,800 bytes with invalid_input
//     ("<name> is larger than 50 MB"), encrypts the rest in 1 MiB chunks straight from disk. null when cancelled.
// file_rename(id, name) -> FileEntry                 file_delete(id, password) -> void   (permanent, re-prompt)
// file_read(id) -> bytes            raw ArrayBuffer (tauri::ipc::Response, never base64 or JSON), only for the in-app preview
// file_save_copy(id, password) -> { path: string } | null     native save dialog in the core, stream-decrypts to a 0600 file
```
The preview shows images (`<img>` from a blob URL), text (first 2 MiB, UTF-8) and PDFs (pdf.js rendered to canvas; WebKitGTK and Android WebView cannot show a PDF natively). The blob URL is revoked and the bytes dropped when the preview closes. Android sets `FLAG_SECURE` while a preview is open.

## IPC additions for Q4 (import and export)

The webview never learns a path. The core opens the native picker and keeps `handle -> path`; handles expire after 30 minutes or at lock.

```ts
type ImportFormat = "keepass-kdbx" | "keepass-xml" | "bitwarden-json" | "bitwarden-csv" | "proton-json" | "proton-zip" | "proton-csv"
  | "1password-1pux" | "1password-csv" | "lastpass-csv" | "dashlane-csv" | "dashlane-json" | "nordpass-csv" | "enpass-csv" | "enpass-json"
  | "chrome-csv" | "firefox-csv" | "edge-csv" | "brave-csv" | "safari-csv" | "aegis-json" | "2fas-json" | "otpauth-list" | "google-migration" | "custom-csv";
interface ImportPick { handle: string; fileName: string; size: number; format: ImportFormat | "unknown"; needsPassword: boolean; needsKeyfile: boolean }
// import_pick() -> ImportPick | null            native picker; the core sniffs the format from content, not the name
// import_pick_keyfile() -> { handle: string; fileName: string } | null
// inspect_csv_headers(handle) -> { delimiter: string; headers: string[]; sample: string[][] }      first 5 rows
interface CsvMapping { title?: string; username?: string; password?: string; url?: string; totp?: string; notes?: string; folder?: string }  // header names
type ImportMode = "skip_duplicates" | "append_version" | "import_all";
interface ImportOptions { handle: string; format: ImportFormat; mode: ImportMode; password?: string; keyfileHandle?: string; mapping?: CsvMapping }
interface ImportPreview { entries: number; totpSeeds: number; attachments: number; duplicates: number; skipped: number; warnings: string[]; titles: string[] }  // up to 50 titles, no secrets
// import_preview(options) -> ImportPreview     parses in the core; entries stay in core memory, zeroized on drop
// import_commit(options) -> { added: number; appended: number; skipped: number; failed: number }   one SQLite transaction, then queued for sync
// import_cancel(handle) -> void
// export_plain(format: "bitwarden-json" | "csv", password: string) -> { path: string } | null     re-prompt, native save dialog, 0600 file
// export_vault_file(password)  (existing) is the native encrypted archive
```
Deviation from the spec text: `inspect_csv_headers` takes the picker `handle`, not a path, so a compromised webview cannot ask the core to read arbitrary files.
