# IPC_CONTRACT — Tauri commands between the UI and the Rust core

Single source of truth for the boundary in `ARCHITECTURE.md` §4. The React UI (`src/backend/`) and the Rust command layer (`src-tauri/src/commands/`) both implement exactly this. Change it here first.

Conventions: JSON over Tauri IPC; **camelCase** on the wire (`#[serde(rename_all = "camelCase")]`); JS passes camelCase arguments (Tauri maps them to snake_case Rust parameters); timestamps are UTC milliseconds; ids are lowercase UUIDv4. Every error is `{ "code": string, "message": string }` (the message is safe to show; it never contains secrets).

Error codes: `locked` · `no_vault` · `vault_exists` · `wrong_password` · `not_found` · `in_trash` · `not_in_trash` · `version_conflict` · `invalid_input` · `confirmation_mismatch` · `token_invalid` · `storage` · `crypto` · `profile_name_taken` · `unsupported_file` · `passkey_unsupported` · `passkey_excluded` · `network` · `remote`.
The passkey provider (§14) also answers `app_locked` and `app_check_off`, which only it uses.

## 1. Re-prompt rule (enforced in Rust)

**Changed 2026-11-24.** The master password was asked for every reveal, copy, edit and clone. Now the vault being open is
enough, as in Bitwarden, and an item asks again only when its own flag says so. Rust verifies a password with Argon2id
(`verify_password`) on every call that carries one and **never caches it**.

* **Per-item flag `reprompt`** (`ItemDraft`, `ItemSummary`, `ItemView`; set in the editor with "Ask for the master password
  again"). It is part of the sealed payload, so the server-side copies learn nothing. A normal edit changes it (history lists
  "reprompt" among the changed fields). Items without the flag seal exactly as before (the field is omitted when false).
* **Optional password on the item commands.** `item_reveal`, `item_begin_edit`, `item_clone`, `history_get`,
  `history_restore` and `totp_now` take `password: string | null`.
  * **No password sent:** the core checks, in one visit under the session lock, that the item exists and is not in the
    trash, then whether it is flagged. Unflagged: the operation runs. Flagged: `password_required` (nothing ran, no Argon2id
    run was spent). The UI then opens the password sheet and calls again with the password (`withItemPassword` in
    `Reprompt.tsx`). `totp_now` on a flagged item answers `password_required` too, so the live codes of a flagged item never
    reach the list or the Codes view; the person taps "Show code" and the code shows for its own window.
  * **A password is sent:** it is always verified (`wrong_password` when wrong), flagged item or not.
* **Always password-gated, flag or no flag:** `item_purge`, `vault_change_password`, the plaintext and encrypted exports,
  `file_delete`, `file_save_copy`, the backup secret, `gate_configure` and `confirm_password`. These take a required
  `password`.
* Editing spans two calls, so `item_begin_edit` returns a random single-use **edit token** (256-bit, bound to item id +
  version, expires after 10 minutes, invalidated by lock). `item_update` must present it. The token carries no secret.
* Creating, trashing, archiving and restoring-from-trash never need the password; history *listing* shows only event, time and
  changed field names.
* The TOTP **secret** is never returned by any command except inside `item_reveal` / `item_begin_edit` / `history_get`.
* **Compatibility.** A build older than this change reads a payload that has `"reprompt": true` as damaged (its payload type
  rejects unknown fields). Unflagged items are byte-identical and unaffected.

## 2. Types

```ts
type ItemType = "login" | "card" | "identity" | "sshKey" | "secureNote" | "totp" | "media";

interface SuiteInfo { /* unchanged, see src-tauri/src/crypto/mod.rs */ }

interface VaultStatus {
  exists: boolean;                    // a vault file with a key bundle exists
  unlocked: boolean;
  damagedItems: number;               // additive (Phase 2): items that failed authentication at unlock and were
                                      // skipped instead of locking the whole vault; 0 normally. The UI shows a
                                      // warning toast after unlock when this is > 0. Optional in the TS type
                                      // because the preview backend omits it.
  syncRevision: number;               // additive (R3a): goes up by one every time a pull changed what is stored
                                      // (new item, fast-forward, deletion, other-device copy). In memory, starts at 0
                                      // at every unlock. The page re-reads its lists when it differs from the last
                                      // value it saw (poll vault_status while unlocked; no event is emitted).
  reconciling: boolean;               // additive (R3a): a pull is queued or running ("Checking your backup...")
  remoteDamaged: number;              // additive (R3a, beyond the brief): objects in the backup that failed
                                      // authentication during the last pull; skipped, never applied; 0 normally
}

// There is no quick-unlock and no `capabilities` field any more (decision D13, strict lock separation): the
// Stage 1 gate (section "Stage 1 gate" below) is a privacy screen only and never holds a vault key.

interface ItemSummary {               // list rows; no secrets
  id: string; type: ItemType; title: string;
  subtitle: string | null;            // username or url host, for the row's second line
  favorite: boolean; tags: string[]; folder: string | null;   // a folder is a path of names joined by "/", see §13
  hasTotp: boolean; codeKeys: string[];   // codeKeys: the keys of the item's one-time codes in order ("totp", "totp#2" …), 0.5.0
  createdAt: number; updatedAt: number; version: number;   // createdAt: 0.4.0, for "Newest first"
  archived: boolean;                  // §5.2; item_list returns active AND archived items, never trashed ones
}

type FieldKind = "text" | "url" | "password" | "note" | "totp" | "paragraph";  // paragraph = non-secret multi-line text
interface FieldView {                 // masked view of one field
  key: string; label: string; kind: FieldKind;
  secret: boolean;                    // password / note / totp are secret; text / url / paragraph are not
  value: string | null;               // present only when !secret
  hasValue: boolean;
}
interface ItemView {                  // item_get: everything except secret values
  id: string; type: ItemType; title: string; favorite: boolean;
  tags: string[]; folder: string | null;
  fields: FieldView[];
  customFields: { name: string; hidden: boolean; value: string | null }[]; // value null when hidden
  attachments: { id: string; name: string; mime: string; size: number }[];
  createdAt: number; updatedAt: number; version: number;
  changedElsewhere: boolean;          // additive (R3a): another device changed this item in a way that could not be
                                      // fast-forwarded and its copy is in the history, not yet reviewed (§6.2)
}

interface ItemDraft {                 // full decrypted, editable content (secrets included)
  type: ItemType; title: string; favorite: boolean;
  tags: string[]; folder: string | null;
  fields: Record<string, string>;     // key -> value, see field schemas below
  customFields: { name: string; value: string; hidden: boolean }[];
}

interface TrashEntry { id: string; title: string; deletedAt: number }  // nothing else, by spec

interface VersionEntry {
  version: number; timestamp: number;
  event: "created" | "edited" | "trashed" | "restored" | "restoredFromVersion" | "archived" | "unarchived";
  changedFields: string[];            // field keys / "title" / "tags" ...; never values
  restoredFrom: number | null;
  otherDevice: boolean;               // additive (R3a): a copy that came from another device (§6.2); its `version`
                                      // is a local number above every own entry, so it can sit above the head
}

interface TotpNow { code: string; previous: string; next: string; period: number; digits: number; secondsRemaining: number; validUntilMs: number }
```

### Field schemas (keys inside `ItemDraft.fields`)

| Type | Keys (in display order) |
|------|--------------------------|
| `login` | `username` text · `password` password · `url` url · `totp` totp · `notes` note |
| `card` | `cardholder` text · `brand` text · `number` password · `expMonth` text · `expYear` text · `code` password · `pin` password · `notes` note |
| `identity` | `salutation` text · `firstName` text · `middleName` text · `lastName` text · `email` text · `phone` text · `company` text · `address1` text · `address2` text · `city` text · `state` text · `postalCode` text · `country` text · `nationalId` password · `passport` password · `license` password · `notes` note |
| `sshKey` | `publicKey` paragraph · `privateKey` note · `passphrase` password · `fingerprint` text · `notes` note |
| `totp` | `issuer` text · `account` text · `totp` totp |
| `secureNote` | `body` note |
| `media` | `description` note (attachments arrive in Phase 5) |

Light validation only (never blocks legitimate data): `card.number` digits, spaces and dashes, 8-23 characters; `expMonth` 1-12 and `expYear` 4 digits when present; `sshKey.privateKey` at most 64 KiB. List-row subtitles built in Rust: `card` → `<brand> •••• <last4>` (else cardholder), `identity` → full name (else email), `sshKey` → fingerprint (else first word of the public key comment), others as before.

**Several sign-ins and codes (D46).** A `login` may repeat `username`, `password` and `totp` as `key#n` (`username#2` … `#10`, canonical spelling only: `#1` and `#02` are `invalid_input`); no other type may. A sign-in is the username and password with the same number; codes stand alone. `ItemSummary.codeKeys` lists the code keys in order and `totp_now` takes an optional `key` (default: the first). Views label repeats "Username 1/2", "Password 1/2", "One-time code 1/2": a sign-in's username and password carry the same number, and numbers appear only once a login has more than one of that kind. The first of each kind keeps its plain key, so lists, search subtitles and older builds still work.

`totp` accepts an `otpauth://totp/...` URI **or** a bare base32 secret; Rust validates with `TotpAccount::from_uri` / `new` and rejects with `invalid_input`. Unknown keys are rejected. Empty values are dropped.

## 3. Commands

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `crypto_suite_info` | – | `SuiteInfo` | exists today |
| `vault_status` | – | `VaultStatus` | works while locked |
| `vault_create` | – | – | **removed** in this revision: vaults are created through `profile_create` (§5.1) |
| `vault_unlock` | `password` | `void` | `wrong_password` / `no_vault`; builds the in-memory index |
| `vault_lock` | – | `void` | the Lock button. An account **without** a PIN: drops the VaultRootKey, index and edit tokens (zeroized). An account **with** a PIN (§5.4): the vault stays open in memory, out of reach (every command answers `locked`), and its PIN shows it again; read `vault_status().resumable` to see which |
| `vault_lock_all` | – | `void` | "Log out": every open vault is wiped, those waiting behind a PIN too; the saved accounts stay (§5.4) |
| `vault_unlock_pin` | `pin` | `void` | the selected account's PIN, while its vault waits behind it. `wrong_pin` (message: "That PIN is not right. Tries left: n."), `pin_locked` (the 5th wrong one wiped the vault), `locked` (nothing waits any more: use the master password). The vault is then **limited** (§5.4) |
| `vault_elevate` | `password` | `void` | the master password, typed in a limited vault: it stops being limited until the next lock. `wrong_password`; `locked` |
| `account_pin_set` | `password`, `pin` | `ProfileInfo` | gives the selected, unlocked account a PIN or changes it (4 to 64 characters, `invalid_input` otherwise); the master password is verified (`wrong_password`) |
| `account_pin_remove` | `password` | `ProfileInfo` | removes the PIN (master password verified); a vault waiting behind it is wiped |
| `vault_change_password` | `oldPassword`, `newPassword` | `void` | re-seals the keyring only (`change_password`) |
| `item_list` | – | `ItemSummary[]` | active items only, from the in-memory index. The order is not promised: the screen sorts (§13) |
| `item_get` | `id` | `ItemView` | `in_trash` for trashed items (trash policy lives here, not in the UI) |
| `item_reveal` | `id`, `password?` | `ItemDraft` | password only for a flagged item (`password_required`), else see §1; `in_trash` refused |
| `item_begin_edit` | `id`, `password?` | `{ token: string; draft: ItemDraft; version: number }` | same rule |
| `item_create` | `draft` | `ItemSummary` | version 1 |
| `item_update` | `id`, `token`, `expectedVersion`, `draft` | `ItemSummary` | consumes the token; `version_conflict` if `expectedVersion` ≠ stored; `changedFields` computed in Rust |
| `item_trash` | `id` | `void` | new version, `deleted=true` |
| `item_restore` | `id` | `ItemSummary` | from trash; new version |
| `trash_list` | – | `TrashEntry[]` | **only** id, title, deletedAt |
| `item_purge_plan` | `id` | `PurgePlan` | 0.4.0. `{deletedAt, recent, olderAt, sum}`. `recent` is true when the item went to the trash less than six months (183 days) ago on the core's own clock (checked UTC when a check succeeded, the device clock otherwise; a trash time that cannot be placed in the past counts as recent, the stricter rule). Then `sum` is a freshly made long sum (text such as `"(48*17)+392-71"`; the **answer never leaves the core**), valid for 10 minutes; asking again replaces it. For an older item `sum` is `null`. `olderAt` is when the six months are over. Needs the item to be in the trash (`not_in_trash`) |
| `item_purge` | `id`, `password`, `answer?` | `void` | must be in trash (`not_in_trash`). Always the master password (`wrong_password`, checked first). An item trashed less than six months ago also needs `answer`, the answer to the sum `item_purge_plan` made: `wrong_answer` when it is wrong, missing, not a whole number of at most 9 digits, for another item, or expired, and **the sum is used up by any wrong answer** (a new `item_purge_plan` is needed; a wrong *password* does not use it up). An older item needs only the password and ignores `answer`. Writes tombstone, removes items/versions/attachments/conflicts rows. (Before 0.4.0 the third argument was `confirmation`, the item title or `CONFIRM`; it is gone.) |
| `history_list` | `id` | `VersionEntry[]` | newest first by number; refused for trashed items. Entries with `otherDevice: true` come from another device (§6.2) |
| `history_get` | `id`, `version`, `password?` | `ItemDraft` | same rule as `item_reveal` |
| `history_restore` | `id`, `version`, `password?` | `ItemSummary` | new snapshot (next free number), `event = restoredFromVersion`; restoring an other-device copy makes the new head a descendant of both lines and clears `changedElsewhere` (§6.2) |
| `item_review_other_device` | `id` | `void` | R3a: "got it", marks the item's other-device copies as seen without restoring anything, which clears `changedElsewhere`; idempotent; `not_found` / `in_trash` / `locked` as for the other item commands |
| `totp_now` | `id`, `password?` | `TotpNow` | active items with a `totp` field only; a flagged item needs the password (`password_required`), see §1. `previous` and `next` are the codes of the window before and after `code` (same secret, same algorithm, same digits); they come from the same call, so a flagged item withholds them exactly like `code`. The screen shows `next` (with a copy button) and `previous` under the live code, so the person can see what comes when it runs out. |

Nothing returns keys, seeds, hashes or the master password.

## 4. Behavioural guarantees the tests must cover

* Every state change (create, update, trash, restore, restore-from-version) writes an immutable snapshot **and** the new latest state atomically, with `version = previous + 1`.
* Trashed items: `item_get`, `item_reveal`, `item_begin_edit`, `history_*`, `totp_now` all fail with `in_trash`; `trash_list` exposes only id/title/deletedAt.
* Nothing is auto-purged. `item_purge` is the only deletion path and requires the password, the in-trash state and, within six months of trashing, the answer to the core's sum.
* A tombstoned id can never be re-created or re-imported.
* After `vault_lock` (and while a vault waits behind a PIN), every command except `vault_status`, `vault_unlock`, `vault_unlock_pin`, `vault_create` and `crypto_suite_info` returns `locked`.

## 5. Accounts (profiles), archive, clone, vault files (Phase 2b)

### 5.1 Profiles: several accounts in one app

Each **profile** is a separate vault with its own master password, key bundle and SQLite file. Profiles share nothing: no key, no table, no folder. The login screen lists them and the user picks one before typing its master password.

```ts
interface ProfileInfo { id: string; displayName: string; createdAt: number; lastUsedAt: number | null;
  demo?: boolean;                      // the built-in vault
  hasPin?: boolean;                    // the account has a PIN (§5.4); absent when false
  resumable?: boolean }                // its vault is open in memory behind the PIN right now; absent when false
interface ProfileList { profiles: ProfileInfo[]; selectedId: string | null; lastUsedId: string | null }
interface VaultStatus {                // extended: describes the *selected* profile
  exists: boolean;                     // the selected profile's vault file exists (false when nothing is selected)
  unlocked: boolean;
  profile: ProfileInfo | null;         // null when no profile is selected
  damagedItems: number;
  syncRevision: number; reconciling: boolean; remoteDamaged: number;   // R3a, see §2 and §6.2
  resumable?: boolean;                 // locked, but the PIN opens it (§5.4); absent when false
  limited?: boolean;                   // opened with the PIN: secrets and settings need the master password; absent when false
}
```

* **Registry** `<app data dir>/profiles.json` (plain JSON, not secret: it must be readable before any unlock, so the display names are visible to anyone who can read the folder; documented trade-off). Written atomically (temp file, fsync, rename, fsync of the folder). Each vault lives in its own folder `<data dir>/vaults/<id>/vault.db`; the folder name is the random id, never the display name (no path tricks, no unicode surprises). A pre-existing `<data dir>/vault.db` from the single-vault era is **adopted in place** as a profile named "My vault" (registry `dir` is `"."`); it is never moved or rewritten.
* **Display name rules** (enforced in Rust; the UI mirrors them for instant feedback): Unicode NFKC, trimmed, internal whitespace runs collapsed to one space, 1-40 characters, no control characters, **unique per app** compared case-insensitively on the normalized form (`profile_name_taken`). It is a label, not a secret and not a login id.
* **Lost or corrupt registry:** a registry that fails to parse is renamed to `profiles.json.corrupt-<ms>` (never deleted) and rebuilt by scanning `vaults/*/vault.db` and the legacy file; recovered profiles are named "Recovered vault 1..n" and can be renamed.
* Only one profile is **shown** at a time. Selecting another profile takes the current one out of reach first: an account with a PIN stays open in memory behind it (§5.4), an account without one is wiped.

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `profile_list` | – | `ProfileList` | works while locked; newest-used first |
| `profile_create` | `displayName`, `password` | `ProfileInfo` | validates the name, creates the folder and the vault (min password length 8, Argon2id default params), selects it and leaves it **unlocked**; atomic (nothing is left behind on failure) |
| `profile_select` | `id` | `void` | takes the current profile out of reach (parked behind its PIN, or wiped when it has none), selects this one (locked, or `resumable` when it was parked); `not_found` |
| `profile_rename` | `id`, `displayName` | `ProfileInfo` | the selected, unlocked profile only; `profile_name_taken` |
| `profile_remove` | `id`, `password`, `confirmation` | `void` | verifies that profile's master password (Argon2id) even if it is not selected; `confirmation` must equal its display name or `CONFIRM` (`confirmation_mismatch`); deletes only that profile's folder and registry entry (legacy-adopted vault: its files) |
| `profile_import_file` | `displayName` | `ProfileInfo \| null` | Rust opens the **native open dialog** (the webview never supplies a path), validates the chosen file structurally without any password (SQLite header, `application_id`, `user_version`, key bundle parses), copies it into a new profile folder, selects the profile (locked). `null` if the dialog is cancelled; `unsupported_file` otherwise on a bad file |
| `vault_export_file` | `password` | `{ path: string } \| null` | password-gated; Rust opens the **native save dialog**, writes a consistent copy of the unlocked vault (SQLite backup, already fully encrypted), file name suggestion `<displayName>-<yyyy-mm-dd>.hexis`. Backup-target connection details marked "this device only" are stripped from the copy |

`vault_unlock(password)` and the rest of §3 act on the **selected** profile. `vault_create` no longer exists.

### 5.4 The account PIN (2026-11-25, D45; rules in `ARCHITECTURE.md` §5.1)

* **States.** *Parked*: the vault is open in memory and out of reach; status `unlocked:false, resumable:true`; every command but `vault_status`, `vault_unlock`, `vault_unlock_pin` answers `locked`. *Limited*: shown again with the PIN; status `unlocked:true, limited:true`. A full unlock (`vault_unlock`, or `vault_elevate` on a limited vault) clears it.
* **What a limited vault refuses, in the core.** `password_required` (nothing ran): `item_reveal`, `item_begin_edit`, `item_clone`, `totp_now`, `history_get`, `history_restore`, every item, flagged or not, unless the call carries the master password (then it is verified as always). `master_required` (nothing ran): `backup_add`, `backup_update`, `backup_remove`, `backup_set_master`, `app_settings_set`, and `file_read` (an attachment is a secret, so the preview asks too). `app_settings_set` answers `locked` while the vault is still parked. `item_search` is not refused but searches as if every item asked for the master password again: **no note text is compared** (labels, titles, usernames, web addresses and tags still are), so the search cannot be used to probe a note. Unchanged and always password-carrying: `item_purge`, `vault_export_file`, `backup_reveal_secret`, `vault_change_password`, `profile_remove`, `gate_configure`, the app reset.
* **Wipe.** A parked vault is wiped with every other open one by the inactivity timer, leaving the app too long, `gate_close`, `vault_lock_all`, exit, `profile_remove`, `account_pin_remove` and the app reset; and by the fifth wrong PIN in a row (`pin_locked`). The count of wrong PINs restarts only after a right PIN or when a shown vault is parked; Lock, or choosing the same account again, while it is already parked does **not** restart it (a regression test pins this: it was the one way to get unlimited tries).
* **Storage.** The PIN is an Argon2id hash in the account's entry in `profiles.json` (`pin`), so the wipe that removes the registry removes it. No command returns it. No other file holds it (a test scans the data folder for the PIN text).
* **Error codes added:** `wrong_pin`, `pin_locked`, `master_required`. Messages are fixed sentences; the typed PIN is never echoed.
* The gate's allow-list does not include any of the new commands: with the app lock closed they answer `locked`, like every vault command.

### 5.2 Archive and clone

* **Archive** = a flag inside the sealed payload (`archived`), not a state change in the trash lifecycle. Archived items are fully readable and editable, keep their history, are hidden from the default list, search of "All items" and the Codes view, and have their own list. Archiving and unarchiving are versioned events (`archived` / `unarchived`). Trashing an archived item is allowed; restoring it returns it to the archive.
* **Clone** copies an item into a brand-new item (version 1, title `<title> (copy)`, not archived, same type, tags, folder and custom fields; attachments are not copied). It is password-gated because it duplicates secrets, and it happens entirely in Rust so the secrets never travel to the UI.

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `item_archive` | `id` | `ItemSummary` | `in_trash` refused; no password |
| `item_unarchive` | `id` | `ItemSummary` | no password |
| `item_clone` | `id`, `password?` | `ItemSummary` | same rule as `item_reveal` |

### 5.3 As built: where Phase 2b is more specific than §5 (backend, verified by tests)

None of these change a shape or a command name; they pin behaviour the text above left open.

* **Restoring an old version keeps the current archive flag** (`history_restore` restores content only; archived is a lifecycle flag like the trash state). `item_archive` / `item_unarchive` are idempotent: repeating one returns the item without a new version.
* **Argument checks come before key derivation.** An empty password or a malformed item id is `invalid_input`, answered without running Argon2id (same as every other gated command). A refused clone (wrong password) creates nothing.
* **`profile_select`** always locks whatever was unlocked, including the same account if it is re-selected. Selecting also updates `lastUsedAt`, as does a successful `vault_unlock`; failing to save that timestamp never blocks a login.
* **`profile_create`** checks the name first (an invalid or taken name is answered before anything is created or any key derived). A too-short password or any later failure (folder, key generation, saving the account list) removes everything it made again; the previously selected account stays selected and unlocked.
* **`profile_remove`**: the confirmation is compared first (`confirmation_mismatch`), then the account's own master password (`wrong_password`). If the account's vault file is already gone, no password is asked and the entry is dropped. The adopted single-vault file is deleted together with its `-wal` / `-shm`, and nothing else in the data folder is touched.
* **`profile_import_file`**: the name is validated (and `profile_name_taken` raised) *before* the dialog opens. A file inside the app's own data folder is `invalid_input`; every other bad file is `unsupported_file` with a specific message (folder, unreadable, not a database, a database but not a Hexis vault, saved by a newer version of Hexis, empty, damaged or truncated, no vault inside, damaged key data). The chosen file is only read. The new account gets a fresh device id.
* **`vault_export_file`**: the password is verified before the dialog opens; a destination inside the app's own data folder is `invalid_input`; a cancelled dialog is `null`, never an error. The file is written 0600 via a temp file and rename.
* **`vault_status`** with no account selected: `exists: false`, `unlocked: false`, `profile: null`. With one selected: `exists` is whether *its* vault file has a key bundle.
* The development shim `src-tauri/examples/e2e_server.rs` is not part of this contract or of any build: it serves the same commands over loopback HTTP for browser-driven tests (`POST /ipc` `{cmd, args}` answering `{ok}` or `{err: {code, message}}`), stands in for the native dialogs with the environment variables `HEXIS_E2E_OPEN_PATH` and `HEXIS_E2E_SAVE_PATH`, and adds a dev-only `POST /e2e/picker` that sets those two variables at run time.

### 5.4 Opening a link: `open_url` (Phase 4a)

The UI shows web addresses (an item's `url` field, the provider's help page). It must not open them itself and must not hold a plugin permission that could open *anything*, so a link goes through one command that checks it first.

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `open_url` | `url` | `void` | needs no account and no unlocked vault. Only **`http`** and **`https`** are opened. Rust validates, then hands the *canonical* form of the address (what `url::Url` serializes, so exactly what was checked is what is opened) to the system browser through `tauri-plugin-opener`, called **from Rust only** |

Refused with `invalid_input` and a fixed message, before anything is opened: an empty address, one longer than 2048 characters, any control character, any whitespace, any backslash (browsers read `\` as `/`), a scheme other than `http`/`https` (`javascript:`, `file:`, `data:`, `vbscript:`, `mailto:`, `ftp:`, `tauri:` and the rest), no host, and **credentials in the address** (`https://user:password@host/`). A relative address (`example.com`, `//host/x`) has no scheme and is refused too. `https:///path` is not an error: like a browser, the parser reads it as `https://path/`, and that canonical form is what gets opened. If the operating system cannot open the link the error is `storage` ("The link could not be opened.").

**Capability.** `src-tauri/capabilities/default.json` still grants the main window only `core:default`. Neither `opener:*` nor `dialog:*` is granted, so the plugin's own commands (`plugin:opener|open_url`, `reveal_item_in_dir`, `default_urls`) are closed to the webview, and the plugin's click-interception script is switched off (`open_js_links_on_click(false)`), so a plain `<a href>` does nothing special either. This was checked on the generated files, not assumed: after a build, `src-tauri/gen/schemas/capabilities.json` lists exactly two capabilities, `default` (`core:default`) and `mobile` (platforms `android` and `iOS` only; the five `barcode-scanner:*` permissions of §6.1), and no permission of `opener` or `dialog` appears in either. The `opener` and `dialog` entries of `acl-manifests.json` are only the list of what each plugin *offers*.

The development shim (`src-tauri/examples/e2e_server.rs`) opens nothing: it records the validated address instead (`GET /e2e/opened` returns `{ok: [urls]}`, `POST /e2e/opened` clears the list).

## 6. Backup targets and the master backup (Phase 4a)

A **backup target** is an S3-compatible bucket (AWS S3, Cloudflare R2, Backblaze B2, MinIO, Wasabi, Storj and others). Targets, their credentials and their queues live **inside that profile's vault file**, sealed with the vault's `meta` sub-key (credentials never touch disk in plaintext, never reach the bucket in plaintext, and never return to the UI except through a password-gated reveal).

* **What is replicated:** the already-encrypted objects of `STORAGE_AND_SYNC.md` §2 (key bundle, latest item states, immutable version snapshots, tombstones). The bucket sees only ciphertext and the metadata listed in §8 there.
* **Master backup:** at most one target is the *master*. Every change is replicated to the master first and then to the other enabled targets. Each target also has the checkbox **"Save in master backup"** (`shareInMaster`, default on): when set, that target's connection details travel inside the vault snapshot, so on a fresh device you only have to enter the **master** target once and unlock with your master password to get all shared targets back; when unset the details stay on this device only (they are stripped from exported files and from what is uploaded).
* Phase 4a is **single-writer replication plus restore**: push from the active device, restore into a fresh profile. A remote item that changed since this device last saw it is *not overwritten* by a push. Since R3a (§6.2) the pull at unlock and at "Back up now" brings such changes in (fast-forward, or an "other device" copy in the history when both sides changed), so a target shows `failing` for this only when the other device wrote between the pull and the push; a conflict screen with side-by-side merge is not built.
* Capability probe at setup (conditional writes) as in `STORAGE_AND_SYNC.md` §6.

```ts
interface BackupTargetInput {
  name: string;                        // label shown in the UI, unique per vault
  endpoint: string | null;             // null / empty = AWS; https URL for other providers (http only for localhost)
  region: string; bucket: string; prefix: string;
  accessKeyId: string; secretAccessKey: string;
  pathStyle: boolean;                  // true for MinIO and most self-hosted
  enabled: boolean; master: boolean; shareInMaster: boolean;
}
type HealthState = "untested" | "healthy" | "lagging" | "failing" | "disabled";
interface TargetHealth {
  state: HealthState; queued: number; lastSuccessAt: number | null; lastCheckedAt: number | null;
  lastError: string | null; conditionalWrites: "strong" | "weak" | "unknown";
}
interface BackupTargetView {           // never contains the secret key
  id: string; name: string; endpoint: string | null; region: string; bucket: string; prefix: string;
  accessKeyId: string; hasSecret: boolean; pathStyle: boolean;
  enabled: boolean; master: boolean; shareInMaster: boolean; health: TargetHealth;
}
```

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `backup_list` | – | `BackupTargetView[]` | master first |
| `backup_test` | `input` | `{ ok: boolean; message: string; conditionalWrites: "strong" \| "weak" \| "unknown" }` | does not save: writes, reads and deletes a probe object, lists the prefix; `network` / `remote` errors carry a human message |
| `backup_add` | `input` | `BackupTargetView` | saves sealed; enqueues a full first upload when enabled; a new master demotes the old one |
| `backup_update` | `id`, `input` (secret optional) | `BackupTargetView` | an empty `secretAccessKey` keeps the stored one |
| `backup_remove` | `id` | `void` | removes the target and its queue only; nothing is deleted in the bucket |
| `backup_set_master` | `id` | `void` | |
| `backup_run` | `id \| null` | `BackupTargetView[]` | "Back up now": pull and reconcile from the source target (§6.2), then drain the queue for one target or all; returns the fresh health |
| `backup_reveal_secret` | `id`, `password` | `string` | password-gated (spec: exporting S3 configs) |
| `sync_flush` | – | `{ complete: boolean }` | R3a, trigger 3 for phones: sends what is queued for at most 8 seconds in total, then answers; `complete` is true when nothing is left queued (also with the vault locked or no targets: a no-op). Call it from `visibilitychange` (hidden). Never an error for network problems |
| `profile_restore_from_backup` | `displayName`, `input` | `ProfileInfo` | fresh-device flow: connects to the target, downloads the key bundle, items and versions into a new profile folder (authenticated only after unlock, since the bundle is public by design), selects it (locked). `shareInMaster` targets reappear after the user unlocks |

Automatic runs (R3a, no timers): a change enqueues work, and a background worker drains each target's queue at exactly four triggers, **unlock** (after a pull), **a committed local change**, **app exit or backgrounding** (`sync_flush`, the window-close hook) and, since 0.4.0, **the device coming back online** (`sync_reconnect`, §13). A failed job stays queued and is retried at the next trigger, never on a schedule. `lagging` is a failed attempt since the last success or an oldest job older than 5 minutes (read when asked), `failing` is 5 consecutive failed triggers. The UI polls `backup_list` while the backup view or the status chip is on screen (that is a read of stored state, it contacts nobody).

### 6.1 As built: where Phase 4a is more specific than §6 (backend, verified by tests)

Shapes and command names are exactly those above (camelCase on the wire; a snake_case argument or field name is rejected by Tauri, and a mock-runtime test pins it). What the text above left open:

* **Where it lives.** Settings, credentials, queue and health are rows of the vault file (schema v2: `targets`, `outbox`, `remote_objects`). The whole of a target's settings, **including the secret access key**, is one envelope sealed with the `meta` sub-key (`target-<uuid>`); the columns beside it hold only flags, counters and the last error text. A test scans the vault file, its `-wal` and the registry after targets were added, edited and replicated: no secret key, key id, bucket, prefix, endpoint, region or target name is in any of them. The secret exists in memory only while the vault is unlocked (it is zeroized after use) and returns to the UI only through `backup_reveal_secret`.
* **Validation comes first, with no network.** Every `input` is checked before any request leaves the device: name (NFKC, 1-60 characters, unique per vault ignoring case), `endpoint` (`null`/empty is AWS; otherwise `https`, or `http` for loopback only: `localhost`, `*.localhost`, `127.0.0.0/8`, `::1`; no credentials, query or fragment), region, bucket (3-63 characters), prefix (no `.`/`..` parts), key id and secret (no whitespace; a pasted trailing newline is trimmed). Failures are `invalid_input` with a message that never repeats the secret. At most **12 targets** per vault.
* **`backup_test`** needs no account, no unlocked vault and does not save anything. It writes a random object under `vault/probe/`, reads it back, sends a second `If-None-Match: *` write and an `If-Match` write with a wrong ETag (both must be refused with 412 for `conditionalWrites: "strong"`; any accepted one is `weak`; a service that refuses the headers outright, or a probe that cannot finish, is `unknown`), lists `vault/`, and deletes the probe object on every path (a failed delete is a note in the message, not a failure). A target that cannot be reached or refuses the keys is an *answer* (`ok: false`, a fixed human message, `conditionalWrites: "unknown"`), not an error; only invalid input is an error. Messages never contain the endpoint, bucket, key names or any part of a request.
* **`backup_add` / `backup_update`** probe outside the vault lock and **save even when the probe fails**: the person typed the settings once and should not have to type them again because the service is down. The failure shows as `health.state: "failing"` (keys, bucket and signature errors count as failing at once) with `health.lastError`. `conditionalWrites` is whatever the probe learned. A new master (`master: true`, on add or update) demotes the old one in the same transaction; **the first target is not promoted automatically**, a vault has no master until one is asked for, and removing the master leaves none. A **disabled target cannot be the master** (`invalid_input`). Disabling empties its queue; re-enabling, or changing endpoint, region, bucket, prefix or path style, schedules a fresh full upload (everything is verified against the new location, not assumed to be there).
* **`backup_list`** is ordered master first, then in the order the targets were added. `lastCheckedAt` is the last time the target was contacted for any reason (success or failure), `lastSuccessAt` the last time a request succeeded.
* **Health.** `untested` (never contacted), `healthy`, `lagging` (any failed attempt since the last success, or the oldest waiting job is older than 5 minutes), `failing` (5 consecutive failed attempts, **or** a failure that retrying cannot fix such as a wrong key, a missing bucket or a rejected signature, **or** a blocked job, see below), `disabled`. `queued` counts waiting jobs. `lastError` is a fixed sentence written for the person (it names what to check, never the request or the response).
* **`backup_run`** first pulls and reconciles from the source (§6.2), then drains one target (`id`) or all (`id` omitted or `null`; an unknown `id` is `not_found`), and returns the fresh `BackupTargetView[]`. A target with nothing queued that was failing (or has never succeeded) gets a cheap connection check instead, so fixing the cause and pressing "retry" turns it `healthy` without waiting for the next change. Everything about the outcome is in the returned health, not in an error; only `locked` and `not_found` are errors.
* **What is sent and in what order.** Per target: the key bundle, then tombstones, version snapshots, latest item states, purges, then the shared-targets snapshot (`vault/meta/targets.enc`, master only). The master is served first on every pass. Every storage change (create, edit, trash, restore, archive, history restore, purge, password change) enqueues its jobs **in the same database transaction** as the change itself, so nothing is lost if the app is closed between the two. A permanent delete uploads the tombstone first and then removes the item and all its versions from every enabled target.
* **The single-writer rule (no overwriting of newer remote data).** Before an existing remote item is replaced, its header is read: a remote copy with an equal version and the same bytes is adopted as already there; an **older** copy is overwritten (guarded by `If-Match` where the target enforces conditional writes, by read-then-write where it does not); a copy with a **newer** version, a different copy of the **same** version, an object that is not a Hexis object, or a key bundle of **another vault** is never overwritten: that job stays queued as *blocked* with a fixed reason (`health.state` is `failing`, `lastError` explains, e.g. "This backup already holds a newer version of an item than this device has, so it was not overwritten..."), and is looked at again at the next trigger. Since R3a the pull of that trigger has normally merged the other copy by then; a newer or same-version remote copy this device has **already merged** (it is in the history as an other-device copy) is not an error either: the job completes without writing (see `STORAGE_AND_SYNC.md` §10). A changed key bundle of the *same* vault (after a password change) is replaced.
* **The background worker** belongs to the unlocked vault. It sleeps until one of the four triggers wakes it (no timer, no polling, no backoff: R3a), and **stops on lock, profile switch and app exit** (it is joined, so no request outlives the keys). Network calls never happen while the vault's state lock is held, so a slow bucket cannot freeze the UI.
* **`backup_reveal_secret`**: the argument check (`not_found` for an unknown id) comes before key derivation; then Argon2id runs on every call (`wrong_password`); the password is never cached. Returns the bare secret string.
* **`profile_restore_from_backup(displayName, input)`** needs no account and no unlocked vault. Order: display-name rules and the unique-name check (`profile_name_taken`), then the target settings (`invalid_input`), **only then** the first request. It downloads the key bundle (it must exist and validate: `remote` otherwise), lists everything under `vault/`, then writes tombstones, items, version snapshots and the shared-targets snapshot into the new vault file in **one transaction**, using the same private-file helpers as `profile_import_file` (the vault file is owner-only from its first byte). Any failure (network, a damaged or misplaced object, more than 100 000 objects, a repeated listing token) removes the new folder completely, leaves the registry unchanged and leaves the currently open account open. Keys that do not look like Hexis objects are ignored; keys that do but do not match their header (an item stored under another id, a version with a wrong number) stop the restore with a fixed message. Authentication of the downloaded content happens at the first unlock, as the key bundle is public by design. The new account is selected, **locked**, with a fresh device id; `network` is returned when the service cannot be reached, `remote` when it answers but cannot serve a restore. There is no progress callback yet; the call returns when the download is done (the lock of the account list is held meanwhile, so no other account change can run).
* **`shareInMaster`.** Targets marked shared are sealed together into one snapshot that is replicated to the **master only**; a target that is not shared is never in it, and `strip_device_only_targets` removes it (with the queue, remote bookkeeping and health) from the copy written by `vault_export_file`, which keeps the shared ones. After `profile_restore_from_backup` and the first unlock, the shared targets reappear **with their credentials** and the restored master starts with a full verification pass (it re-checks every object once, because the restored device cannot know which of them the bucket already holds). A restored vault that has no snapshot simply has no targets.
* **Not a contract change, but good to know:** a master that is demoted keeps the (now stale) snapshot object in its bucket until the next full upload overwrites it; a restore from a bucket reads whatever snapshot is there. The e2e shim uses the real S3 client like the app, so browser-driven tests point a target at `scripts/mock_s3.py` on loopback; its `--fast-backup` flag makes a queue read as `lagging` after one second instead of five minutes (there are no retry timers to shorten any more).

### 6.2 Pull, reconcile and other-device copies (R3a, backend, verified by tests)

Algorithm and storage: `STORAGE_AND_SYNC.md` §6 and §10. What the page sees:

* **When it pulls.** On the worker right after `vault_unlock` (which never waits for it) and at the start of `backup_run`. The source is the master target, or the first enabled one when there is no master; other targets are written to and never read. While a pull is queued or running `VaultStatus.reconciling` is `true`.
* **`VaultStatus.syncRevision`.** Increases by one for every pull that changed something stored. After an unlock the page should poll `vault_status` (for example every 2 seconds while `reconciling`, and on each focus) and re-read `item_list` / `trash_list` / an open item's `item_get` and `history_list` when `syncRevision` changed. An item the pull deletes (another device purged it) disappears from the next `item_list`; an open item then answers `not_found`.
* **`VaultStatus.remoteDamaged`.** Objects the last pull could not authenticate (a bucket that was tampered with, or damaged storage). Nothing of them is used. Show a quiet warning when it is above 0.
* **`ItemView.changedElsewhere` and `VersionEntry.otherDevice`.** When another device and this one changed the same item and neither line contains the other, the local state stays the head, and the other device's state is added to the history as an entry with `otherDevice: true` (its `version` is a local number above all own entries, so history numbers can have gaps and an other-device entry can be above the head; the latest own state is the first entry with `otherDevice: false`). The item shows `changedElsewhere: true` until the person acts. Viewing such an entry needs `history_get` (password re-prompt) like any other. **Restore this version** on it (`history_restore`) makes the merged state the new head, a descendant of both lines, which the other device then fast-forwards to; `item_review_other_device(id)` just dismisses the flag.
* **Nothing is merged field by field and nothing is overwritten.** Both lines stay in the history on both devices.
* **Errors.** A pull's problems are never errors of a command: they appear in the source target's `health` (a fixed sentence in `lastError`, never an endpoint, bucket or key name) and, for objects that failed authentication, in `remoteDamaged`.
* **`sync_flush`** (table above) is the page's half of "app exit or backgrounding". On a phone the page calls it from `visibilitychange` when `document.visibilityState` becomes `hidden`; `BUILD_TARGETS.md` lists what Android does and does not guarantee. On the desktop the window-close hook does the same flush without the page.

## 7. Disguise: app icon and name (Q2) [impl]

Both commands are **app-level**: they need no account, no unlocked vault, and they never read or write vault state (a test snapshots every vault file before and after). The Stage 1 gate may therefore allow them while it is closed.

```ts
interface DisguiseInput { presetId: string | null; customName: string | null; lowercase: boolean }  // presetId null = the original icon
interface DisguiseView extends DisguiseInput { appliedName: string; note: string | null }           // note: what the launcher could and could not do

// disguise_get() -> DisguiseView
// disguise_set(input: DisguiseInput) -> DisguiseView
```

* Wire format: camelCase. The JS call is `invoke("disguise_set", { input: { presetId, customName, lowercase } })`.
* `presetId` must be one of the 11 ids in `assets/disguise/presets.json` (the core embeds the same file with `include_str!`): `tally, vellum, drizzle, trim, plinth, kerf, basalt, cairn, kovren, tivlo, orvyn`. Ids are matched exactly (no case folding, no trimming).
* `customName`: control characters (newline, tab, escape, NUL, ...) are turned into one space each, then the name is trimmed; a name that is empty after that means "no custom name" (`customName: null` in the answer); 1 to 30 **characters** (not bytes) are allowed, counted after that clean-up. Zero-width, bidirectional-override, soft-hyphen and private-use characters are refused. Path separators, `%`, `;`, `=` and brackets are accepted (they are escaped wherever the name is written). The answer carries the cleaned name, so the UI should show the returned `customName`.
* `appliedName` = the custom name when set, else the preset's name, else `Hexis`; lower-cased when `lowercase` is true (so `lowercase` alone renames the original look to `hexis`).
* `disguise_get` before anything was saved returns `{ presetId: null, customName: null, lowercase: false, appliedName: "Hexis", note: null }`.
* Errors: `invalid_input` with one of three fixed sentences (no input echoed back): `That icon is not one of the choices.` (unknown preset id), `Use 1 to 30 characters for the name.` (over 30), `The name contains characters that cannot be shown.` (invisible formatting characters). The first problem found wins, in that order. Nothing is saved or applied when the input is refused. A storage failure while saving is `storage` with a fixed sentence.
* `note` is a fixed sentence about what the launcher did, or `null`. Linux: `The window and your launcher entry now use this name and icon. Some desktops show the change only after a moment or a new login.` after a change; `The launcher entry was removed and the window shows the original name and icon.` after going back to the original look; a launcher problem (for example a foreign `.desktop` file in the way) is a sentence in the same field and **never** an error: the choice is saved first, then applied, and a failed apply leaves the choice saved. Other desktops: `Only the window title and icon change on this system; there is no launcher entry to rewrite.` Android: `On Android the launcher shows the icons and names from the list. A name of your own changes only the title inside the app and in the recent-apps list, and the lower-case option applies inside the app only. Some launchers need a moment to refresh.` The UI should show `note` as is (it may also keep its own static Android text). `disguise_get` returns the note of the last apply in this run, or the platform's standing note (Android) when nothing was applied yet.
* Behaviour: the choice is saved to `<data dir>/disguise.json` (see STORAGE_AND_SYNC) **before** the launcher is asked to apply it. On start the saved choice is re-applied (window title and icon, Linux launcher entry) from the `setup` hook; a problem there is only a note.
* What "apply" does: the main window's title and icon are set **from Rust** (`capabilities/default.json` still grants only `core:default`; a test keeps it that way, so the webview has no window permission). Linux: writes `<XDG_DATA_HOME or ~/.local/share>/applications/<app identifier>.desktop` and the icon `<data home>/<app identifier>/disguise/<id>.png`, then runs `update-desktop-database` on the applications folder (best effort). Android: enables one `<activity-alias>` and disables the others through a local Kotlin plugin (needs `scripts/android_patch_manifest.py`, see BUILD_TARGETS).
* Hooks for the Q1 wipe: `AppCore::disguise_wipe()` overwrites (zeros) and unlinks `disguise.json`, then asks the launcher for the original look (removes the `.desktop` entry and the icon files on Linux; resets the window title and icon). Call it **before** the data directory is removed. A free function `disguise::remove_launcher_entry(data_home, identifier)` removes only our own entry and icons without a launcher object.
* Frontend: no change needed against `src/backend/types.ts` as it stands (`DisguiseInput`, `DisguiseView`, `disguiseGet`, `disguiseSet`).

## 8. Stage 1 gate, inactivity lock and wipe (Q1) [impl]

**Implemented (Q1).** All names, arguments and shapes are exactly those of `src/backend/types.ts` / `tauri.ts`
(camelCase arguments, snake_case Rust parameters).

| Command | Arguments | Result | Notes |
|---|---|---|---|
| `gate_status` | none | `GateStatus` | `{enabled, open, mode, biometricAvailable, biometric, panicButton, equation, attemptsLeft, lockedForMs, finalAttempt, streak}`. Not configured means `enabled:false, open:true`. `equation` is `null` when open. `attemptsLeft` is `null` while the counter is 0, otherwise `16 - attempts`. `lockedForMs` is the time left of a wait after wrong passcodes (`null` when none runs; never an absolute time). `finalAttempt` is true when the next wrong passcode erases the device (attempt 16). |
| `gate_submit` | `{ input: string }` | `GateOutcome` | 1 to 3 characters (after NFKC): decoy maths, `{kind:"math", correct, equation, streak}`, never a credential, never the counter. 4 or more: a security attempt, `{kind:"opened"}`, `{kind:"failed", attemptsLeft, equation, lockedForMs}` (`lockedForMs` is set when this attempt started a wait), `{kind:"locked", retryAfterMs}` (a wait is running: the input was neither counted nor looked at, not even the right passcode or a panic code) or `{kind:"wiped"}`. With no gate configured (or already open) every input answers `{kind:"opened"}` and costs nothing. |
| `gate_biometric` | none | `GateOutcome` | `{kind:"opened"}` on Android after a recognised fingerprint; `invalid_input` on desktop, when the switch is off, or when the sensor did not recognise the person. Never counts or resets an attempt. Opens Stage 1 only. |
| `gate_panic` | none | `{kind:"wiped"}` | Only while "panic mark" is on and the gate is closed, else `invalid_input`. Same wipe as a panic credential. |
| `gate_close` | none | `null` | Closes the gate (when on) **and locks the open vault** (keys zeroized, backup worker joined). With the gate off it is just a lock. |
| `gate_configure` | `{ input: GateConfigInput }` | `GateStatus` | Needs an unlocked vault (`locked` otherwise) and that vault's master password (`wrong_password`; the re-prompt guard). `credential:null` keeps the stored one (error when none exists or the mode changes), `panicCode:null` keeps, `""` removes; `enabled:false` deletes `gate.json`. Returns `open:true` for this run. Rule failures are `invalid_input` with the messages of `src/lib/gateRules.ts`; they are reported before any Argon2id work. |
| `app_reset_challenge` | none | `string` | The long sum for the last reset step (two ordinary sums joined by `+`, `-` or `*`, answer 100 to 99,999). The core keeps only this one sum; the answer never leaves it. Allowed with the gate closed. |
| `app_reset` | `{ confirmedSteps: 3, answer: string }` | `null` | `confirmedSteps` other than the number 3 is `invalid_input` (also a string, `-3`, `3.5`, `null`; a missing argument is a decoding error). `answer` is checked against the sum from `app_reset_challenge`, which is **used up by any answer**: a wrong answer, or no sum asked for first, is `wrong_answer` and erases nothing; ask again for a new sum. A right answer does the same wipe as panic, reported normally; a second call is `wrong_answer`. Allowed with the gate closed. |
| `app_prepare_uninstall` | `{ confirmedSteps: 3, answer: string }` | `null` | "Remove Hexis from this device" (Settings > Security). Same three confirmations and the same sum as `app_reset` (`app_reset_challenge` first; the sum is used up by any answer; `confirmedSteps` other than 3 is `invalid_input`; a wrong or missing sum is `wrong_answer` and removes nothing). A right answer does the wipe of a reset (every vault, the app lock and its counter, the settings, the launcher entry; files are overwritten with zeros, then unlinked), and then also removes what a reset keeps on purpose: `device.json`, the web view's leftovers, and the data folder when nothing else is in it. **Nothing is made again afterwards** (no demo vault, no default file). On desktop the command then asks the app to close (about 0.9 s later, so the answer reaches the screen); on a phone the app stays on its last page ("Everything is removed", with the next step: uninstall it), because an Android app does not close itself. S3 backups are never touched. **Not on the closed-gate allow-list** (`locked` with the app lock closed): it needs the app lock open. |
| `app_settings_get` | none | `{lockAfterMinutes, lockAfterLeaveSeconds}` | Defaults 15 minutes and 300 seconds. |
| `app_settings_set` | `{ input: {lockAfterMinutes, lockAfterLeaveSeconds?} }` | same | `lockAfterMinutes`: 1, 5, 15, 30, 60 or 240 (no "never"). `lockAfterLeaveSeconds`: 0 (right away), 15, 30, 60, 300, 900, 1800, 3600 or 14400 (default 300 when missing). Anything else is `invalid_input`. Stored in `app-settings.json`. |
| `activity_ping` | none | `null` | Restarts the inactivity timer. |
| `app_leave` | none | `null` | The app went to the background (phones). Stops the inactivity timer and starts the leave timer for `lockAfterLeaveSeconds`; at 0 it locks now. When it runs out the core closes the gate (when on) and locks the vault, whether or not the webview is still running. A native picker or the camera on top of the app is not "leaving" (the UI does not call this then). Not activity. Allowed with the gate closed. |
| `app_return` | none | `null` | The app is back. If the leave time ran out the core locks now; otherwise the leave timer stops and the inactivity timer restarts. Not activity. Allowed with the gate closed. |

**The gate is enforced in one place.** `commands::invoke_handler()` wraps the whole handler list in
`commands::gate::guarded`, which asks `AppCore::before_command(name)` first. While the gate is enabled and
closed, every command that is not on the allow-list answers `{code:"locked", message:"The vault is locked."}`
before its code runs. The guard is deny-by-default: a command added later, and even a name that does not exist,
is `locked` until it is added to `gate::ALLOWED_WHILE_CLOSED`. Allow-list while closed:
`crypto_suite_info`, `suite_info`, `vault_status`, `gate_status`, `gate_submit`, `gate_biometric`, `gate_panic`,
`app_reset_challenge`, `app_reset`, `app_settings_get`, `activity_ping`, `app_leave`, `app_return`, `sync_flush`, `disguise_get`, `screen_secure`
(the last four are listed for the slices that add them; a listed name that is not registered never arrives).
`gate_close`, `gate_configure` and `app_settings_set` need the gate open (they change what Stage 1 protects).

**`vault_status` while closed** answers the neutral `{exists:false, unlocked:false, profile:null, damagedItems:0}`
(no account name). The UI polls it (`syncAfterCoreLock`). `profile_list` is refused while closed; after Stage 1
passes the real values return (a vault is still locked: **Stage 1 never unlocks anything**).

**Removed (strict lock separation).** `VaultStatus.capabilities` (`quickUnlock`, `biometric`) and the Rust type
`Capabilities` are gone: nothing in Rust wraps, caches or derives a `VaultRootKey` outside RAM, and there is no
`quick_unlock_*` / session-blob / keystore code. Every vault open, and every re-open after the idle timer, needs
the master password. The old "Planned for Phase 3: quick_unlock_*" paragraph of this document is obsolete.

**Error codes.** `locked` is the existing code. Two were added: `password_required` (a flagged item was asked for without a
password; see §1) and `wrong_answer` (the answer to the reset sum was wrong or no sum was asked for). `invalid_input`,
`wrong_password` and `storage` cover the rest. Messages are fixed sentences: nothing typed (credential, master password, input) is
ever echoed or logged.

**Waits after wrong passcodes.** 16 security attempts in all; the 16th wrong one erases the device. After the 3rd, 6th, 9th,
12th and 15th wrong attempt a wait starts (1.5 minutes, 5 minutes, 45 minutes, 8 hours, 3 days). The wait is stored as a wall-clock
end time under the same MAC as the counter and written ahead together with it, so force-closing or restarting the app dodges neither the count nor the wait. While a wait runs, `gate_submit` with 4 or more
characters answers `{kind:"locked", retryAfterMs}` and nothing else happens. Changing the device clock or editing the files can
shorten a wait at the price of one more counted attempt; the 16-attempt wipe still applies. Fingerprint is not an attempt and still
opens Stage 1 during a wait.

**Leave timer.** A second timer thread, driven by `app_leave` / `app_return`, with the same wall-clock safeguards as the
inactivity timer.

**Inactivity lock.** One watcher thread per `AppCore` waits on a condition variable until the deadline (no tick
loop), holds no session lock while it sleeps, and runs the same lock the commands use (so it can never run inside
a command's critical section). When the deadline passes it closes the gate (when on) and locks the vault (keys
zeroized, backup worker joined). Every command except `vault_status` and `gate_status` restarts the deadline.
The deadline is kept on the monotonic **and** the wall clock, and every command also checks it, so a laptop
resumed from suspend is locked by the first command that arrives (the monotonic clock stops during suspend).
There is no timer while nothing is open: `gate_close` disarms it.

---

## 9. Encrypted drive: files and attachments (Q3) [impl]

```ts
interface FileEntry { id: string; name: string; size: number; mime: string | null; itemId: string | null; addedAt: number }
// ItemView.attachments: { id, name, mime, size }[]  (mime falls back to "application/octet-stream")
```

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `file_list` | `itemId?: string \| null` | `FileEntry[]` | `null`/absent = the standalone Files section; an item id = that item's attachments (the item must be active: `in_trash`, `not_found`). Newest first. A file whose metadata fails authentication is left out |
| `file_add` | `itemId?: string \| null` | `FileEntry[] \| null` | Rust opens the **native multi-file picker** (the webview never sends a path); `null` when cancelled. A request that cannot succeed (locked, item in the trash or missing) fails **before** the dialog opens. All sizes are checked before the first byte is written: one file over 52,428,800 bytes rejects the whole call with `invalid_input` `"<name> is larger than 50 MB."`. All or nothing: if a later file fails (vanished, disk full), the files this call already added are removed again. At most 100 files per call. Picked paths inside the app's own data folder are refused. Names come from the picked path: control, bidi and path characters become `_`, cut to 200 characters keeping a short extension |
| `file_rename` | `id`, `name` | `FileEntry` | `invalid_input`: `"Use 1 to 200 characters for the file name."` / `"A file name cannot contain slashes or control characters."`; the name is trimmed; `not_found`; `in_trash` when the owner is in the trash. Optimistic: stale metadata is `version_conflict` |
| `file_delete` | `id`, `password` | `void` | re-prompt (`wrong_password`); removes the row, the blob and queues the remote deletion |
| `file_read` | `id` | **raw bytes** (`tauri::ipc::Response`, no JSON, no base64) | for the preview; the plaintext as an `ArrayBuffer`. Errors keep the normal `{code, message}` shape. Decryption runs without the vault lock held |
| `file_save_copy` | `id`, `password` | `{ path: string } \| null` | re-prompt first (a wrong password never opens a dialog), then the native save dialog (pre-filled with the file name), then a streaming decryption to `<dest>.part` (0600) renamed over the destination only when the whole file authenticated; `null` when cancelled |
| `screen_secure` | `on: boolean` | `void` | a no-op everywhere except Android, where it sets or clears `FLAG_SECURE` on the activity window (**UNVERIFIED**, see BUILD_TARGETS) |

* Errors: `locked`, `not_found` ("That item no longer exists." for a missing file or owner), `in_trash`, `wrong_password`, `invalid_input` (limit, names, too many files, a file that cannot be read: `"That file could not be read. Check that it still exists."`, changed while encrypting: `"The file changed while it was being encrypted. Try again."`), `storage` (`"The encrypted file could not be written. Check the free disk space."`, `"The encrypted data of this file is missing on this device."`, and the one integrity sentence `"A stored file failed its integrity check: the data is damaged or was modified."` for any blob or metadata that fails authentication or structure checks; never a panic, never an internal detail).
* Locking (or switching account) while a file is being added, read or saved cancels the transfer and removes the partial output; plaintext buffers are `Zeroizing`; saved copies and temp files are mode 0600.
* Frontend: no change against `src/backend/types.ts` / `tauri.ts` for `file_*`. **Needed:** the real adapter must read `file_read` as binary (Tauri returns an `ArrayBuffer` for a raw response; the dev adapter must read the body as bytes when the reply carries `x-hexis-binary: 1`, which the shim sets), and `screen_secure` has no wrapper yet: add `screenSecure(on: boolean)` calling `invoke("screen_secure", { on })` (a no-op on desktop). The Files section should call it with `true` while a decrypted preview is open and `false` when it closes.

## 10. Importer and plaintext export (Q4) [impl]

All seven commands need an **unlocked vault** (a locked or missing vault answers `locked`, except `import_cancel`, which is always harmless). They are not app-level commands, so the Stage 1 gate must keep them closed while the app is gated. The webview never names a path: the core opens the native dialog itself and hands back a random handle.

```ts
type ImportFormat = "keepass-kdbx" | "keepass-xml" | "bitwarden-json" | "bitwarden-csv" | "proton-json" | "proton-zip" | "proton-csv"
  | "1password-1pux" | "1password-csv" | "lastpass-csv" | "dashlane-csv" | "dashlane-json" | "nordpass-csv" | "enpass-csv" | "enpass-json"
  | "chrome-csv" | "firefox-csv" | "edge-csv" | "brave-csv" | "safari-csv" | "aegis-json" | "2fas-json" | "otpauth-list" | "google-migration" | "custom-csv";
interface ImportPick { handle: string; fileName: string; size: number; format: ImportFormat | "unknown"; needsPassword: boolean; needsKeyfile: boolean }
interface CsvMapping { title?: string; username?: string; password?: string; url?: string; totp?: string; notes?: string; folder?: string }  // header names
type ImportMode = "skip_duplicates" | "append_version" | "import_all" | "merge";   // "merge": 0.4.0, see "Merge" below
interface ImportOptions { handle: string; format: ImportFormat; mode: ImportMode; password?: string; keyfileHandle?: string; mapping?: CsvMapping }
interface ImportPreview { entries: number; totpSeeds: number; attachments: number; duplicates: number; skipped: number; warnings: string[]; titles: string[] }
interface ImportOutcome { added: number; appended: number; skipped: number; failed: number; warnings: string[] }
// import_pick() -> ImportPick | null                           native picker; null = cancelled
// import_pick_keyfile() -> { handle: string; fileName: string } | null
// inspect_csv_headers({ handle }) -> { delimiter: string; headers: string[]; sample: string[][] }
// import_preview({ options }) -> ImportPreview                 changes nothing in the vault
// import_commit({ options }) -> ImportOutcome                  one SQLite transaction
// import_cancel({ handle }) -> void
// export_plain({ format: "bitwarden-json" | "csv", password: string }) -> { path: string; warnings?: string[] } | null   warnings is absent when empty
```

* **`merge` (0.4.0, the default of the page).** The same account arriving from two apps becomes **one** item, with every note kept. An entry matches an item you already have when they share a one-time-code secret, or have the same account (case-insensitive) and alike names (equal, or one contains the other when the shorter is at least 4 characters; a website's name counts as a name). A match is merged into the existing item as a **new version** (nothing is overwritten): a value that differs goes into a custom field `<Label> (imported)` (a field of the same name and value is not added twice), notes are joined (one under the other with a blank line between; a note that already holds the other is simply the longer one; nothing is dropped), websites, tags and custom fields are united, and a second title becomes `Also named`. A Totp-only item (Ente Auth) and a Login of the same account (Bitwarden) become one **Login** in place (same id; the type changes; every earlier version stays as it was; Ente's note, which arrives as a custom field `Notes`, joins the login's notes). `import_preview` counts matches in `duplicates`; `ImportOutcome.appended` counts items merged. Importing the same file twice changes nothing the second time. `skip_duplicates`, `append_version` and `import_all` never match across kinds, as before.
* **Wire format:** camelCase arguments exactly as `src/backend/tauri.ts` already sends them (`invoke("import_preview", { options })`). `ImportFormat` and `ImportMode` are the strings above (`mode` is snake_case). `ImportOptions.password` is the file's password (KeePass, never the vault password). The ids were checked against `src/backend/types.ts` by a test.
* **Handles.** 128 random bits as 32 hex characters. A handle lives 30 minutes after it was picked (not after last use), is wiped by **every lock, every account switch and quit** (the table sits inside the unlocked session), and by `import_cancel`. At most 16 picked files and 2 parsed previews are kept (the oldest goes first). A handle from `import_pick_keyfile` is only accepted as `keyfileHandle`, and a file handle never as one (a test pins the mix-up).
* **Preview, then commit.** `import_preview` reads the file (no lock held, so a KeePass key derivation or a 200 MB file does not stall other commands), parses it and keeps the entries in core memory under the handle, zeroized on drop. `import_commit` imports **exactly what the preview showed**: a stored preview is reused only when handle, format, mapping, key file **and file password** are identical (any change re-parses). A commit works without a preview (it parses first). A commit consumes the stored entries; the file handle stays valid, so a second commit re-parses (and with `skip_duplicates` adds nothing).
* **`ImportPreview`.** `entries` = records that will be imported; `skipped` = records of the file Hexis cannot hold (an unknown item type, a passkey-only record, a trashed record, a counter-based code ...), each with a reason in `warnings`; `duplicates` = entries that match an item already in the vault (rule in STORAGE_AND_SYNC.md); `titles` = up to 50, no secrets; `warnings` = at most 100 fixed-format sentences (a record is named by its title, cut to 60 characters, never by a value).
* **`ImportOutcome`.** `added` new items, `appended` new versions of existing items, `skipped` duplicates left alone (and, in `append_version`, identical records), `failed` records that did not validate (a field too long, a bad code; each with a sentence in `warnings`; the others are unaffected). **The parser-skipped records of the preview are not counted again in `outcome.skipped`.** `warnings` carries the file's own notes first, then the batch's notes, then the attachment note: `N attachments were not imported (the encrypted drive is not available yet).` (`1 attachment was ...`), or `N attachments were not saved.` when a sink failed. The frontend should show `warnings` after a commit (the UI shows `preview.warnings` today).
* **Formats and sniffing.** `import_pick` guesses the format **from the content, never the file name**: KDBX magic; ZIP (then `export.data` = 1pux, a `data.json` member = Proton); JSON top-level keys (Bitwarden, Proton, Aegis, 2FAS, Dashlane, Enpass; the encrypted shapes are recognised too, so they get the refusal below); a `KeePassFile` XML root; `otpauth://` / `otpauth-migration://` lines; CSV by each vendor's header set. Any other table is `custom-csv`; bytes with control characters, an empty file and everything else are `"unknown"`. Chrome, Edge and Brave write the same file and always come back as `chrome-csv`; the UI may re-label by what the person picked. Parsing a file as another format than it is gives `That file is not in the format you chose...`. `needsPassword` / `needsKeyfile` are true for `keepass-kdbx` only.
* **Encrypted exports are refused** (Bitwarden encrypted JSON, Aegis and 2FAS encrypted backups, Proton's PGP export) with `unsupported_file` and exactly: `This export is encrypted. Export it unencrypted from the app, import it, then delete the file.`
* **Errors.** `locked` (no unlocked vault), `invalid_input` (bad mapping, unknown export format, a destination inside the app's data folder, a wrong KeePass password/key file is `invalid_input` too), `unsupported_file` (damaged, too large, too deep, not in this format, encrypted export; **fixed sentences, never any file content**), `wrong_password` (`export_plain` only, and the dialog does not open), `not_found` for an unknown, expired or locked-away handle. **Frontend note:** the `not_found` message text is the generic `That item no longer exists.`; for a file handle the UI should show its own sentence ("That file is no longer open. Choose it again.") when it sees `not_found` from these commands.
* **Limits** (each is a test): file 256 MiB, key file 64 MiB; a zip member 128 MiB, all members 512 MiB, and a member over 1 MiB may not be compressed more than 200:1; at most 200,000 records; JSON and XML nesting 128; KeePass key derivation is read from the header first and refused above 1 GiB of memory or 64 GiB-passes of work (the `keepass` crate does not bound it); XML with a DOCTYPE is refused (no entity is ever defined or expanded); a zip member is read in memory by name and never written to disk or used as a path; one value is cut at 1 MiB (Hexis itself accepts 64 KiB, so a longer value is reported as not importable instead of losing its tail silently). A parser panic inside a third-party crate is caught and becomes `That file is damaged or incomplete...`.
* **`inspect_csv_headers`** takes the picker `handle`, not a path (deviation from the spec text kept from FEATURE_QUEUE.md: a compromised webview cannot make the core read an arbitrary file). The sample is the first 5 rows with every cell cut to 80 characters; it can contain secrets (it is the person's own file), so it is never logged.
* **`export_plain` writes PLAINTEXT.** Anyone who gets the file can read every password, card number and one-time-code secret in it. Order: format check (`invalid_input`) -> master-password re-prompt (`wrong_password`, no dialog yet) -> native save dialog (`<account>-<date>-plaintext.json|csv`; `null` when cancelled) -> refusal of a destination inside the app's own data folder -> decrypt every **non-trashed** item (archived items are included) in RAM -> write to a new file with mode **0600** through a temp file, fsync, rename, fsync of the folder. The buffers wipe themselves. An item that cannot be decrypted is left out and `warnings` says `N items could not be read and are not in the file.` The native encrypted archive `vault_export_file` is untouched.
* **Bitwarden JSON export shape.** `encrypted: false`, `folders`, `items` of type 1 login / 2 secure note / 3 card / 4 identity / 5 SSH key; the TOTP is `login.totp`; custom fields keep their `hidden` flag (field type 1). Hexis has things Bitwarden has no place for: tags and standalone `totp` items. They are written as custom fields named `Hexis tags` and `Hexis item type` (`totp`), and Hexis's importer turns them back, so **the JSON export read by Hexis again is lossless for logins, cards, identities, notes, SSH keys and standalone codes** (round-trip tests, field by field); another app shows those two as ordinary extra fields. **One lossy case:** a `media` item is written as a secure note (its description as the note) and comes back as a note. **The CSV is lossier** (it is Bitwarden's column set): logins and notes round-trip, but cards, identities and SSH keys are flattened into a note that lists every value, and the hidden flag of custom fields is lost. Say so in the export screen if it offers both.
* **UI text mismatch to fix in `ImportView.tsx`:** the `skip_duplicates` card says "An entry with the same name, username and password is left alone." The rule does not compare passwords. Suggested text: "An entry with the same name, username and website is left alone." (The `append_version` card, "Same name and username ...", is right; it also compares the website.) The mock backend (`mock.ts`) compares passwords; it should follow the Rust rule.

## 11. Device settings, trusted time and the demo vault (Q5) [impl]

Shapes are exactly those of `src/backend/types.ts` / `tauri.ts` (camelCase arguments, snake_case Rust parameters). The decisions are D27 to D30 and D32 to D37 in `ARCHITECTURE.md`.

| Command | Arguments | Result | Notes |
|---|---|---|---|
| `device_get` | none | `DeviceInfo` | `{timeZone: string \| null, setupDone: boolean, timeCheck: boolean, theme: "system" \| "light" \| "dark", clipboardClear: 15 \| 30 \| 60, openInSearch: boolean, listSort: ListSort}` (the last two are 0.4.0, §13). `timeZone` is `null` until the first page is answered. `theme` is what Settings > Theme chose (default `system`: follow the device); `clipboardClear` is how many seconds a copied secret stays on the clipboard (default 30). Never carries the demo vault's id. **Allowed while the gate is closed** (the gate screen writes times in the zone and is drawn in the chosen theme). |
| `device_set_time_zone` | `{ zone: string }` | `DeviceInfo` | An IANA name such as `Asia/Dhaka`. The core checks the **shape** only (1 to 64 characters, letters, digits, `_ + - /`, no empty, dotted or leading-dash part); `invalid_input` otherwise. The webview's own time library decides whether the name is a real zone before it sends it. |
| `device_finish_setup` | none | `DeviceInfo` | The first-run welcome was seen: `setupDone` becomes `true`. |
| `device_set_time_check` | `{ on: boolean }` | `DeviceInfo` | Switches the online time check. Off, the core makes no request at all. On, a check starts in the background. |
| `device_set_theme` | `{ theme: "system" \| "light" \| "dark" }` | `DeviceInfo` | Saves the colour theme in `device.json` (the web view stores nothing) and, on desktop, tells the native window so that the title bar and system dialogs follow. Any other word is a decoding error. |
| `device_set_open_in_search` | `{ on: boolean }` | `DeviceInfo` | 0.4.0. After the vault opens, the screen puts the cursor in the search box (a phone shows the keyboard). Default off. Saved in `device.json`. |
| `device_set_list_sort` | `{ order: string }` | `DeviceInfo` | 0.4.0. The order the item list starts in: `updated-desc` (default), `updated-asc`, `created-desc`, `created-asc`, `title-asc`, `title-desc`, `type`, `versions-desc`, `versions-asc`; anything else is `invalid_input`. Saved in `device.json`. |
| `device_set_clipboard_clear` | `{ seconds: number }` | `DeviceInfo` | 15, 30 or 60; anything else is `invalid_input`. Saved in `device.json`. (Before 0.3.0 this lived in the web view's local storage; the value is not carried over, so it starts at 30 once.) |
| `clock_status` | none | `ClockStatus` | `{verified, enabled, skewMs, checkedAgoMs, sources, problem, nowMs}`. `skewMs` is checked UTC minus this device's clock (positive: the device is behind). Starts a background check when one is due. A polling command: it does not count as activity. **Allowed while the gate is closed.** |
| `clock_sync` | none | `ClockStatus` | Checks now and waits for the answer (about ten seconds at most). `invalid_input` when the check is switched off. A failed check is **not** an error: `problem` says what went wrong. **Allowed while the gate is closed.** |
| `demo_spawn` | none | `ProfileInfo` | Makes the vault "Hexis123" (15 to 25 items) **only when the app holds no vault** (`invalid_input` otherwise); selects and unlocks it. `demo: true` in the result. |
| `demo_open` | none | `void` | Selects and unlocks the demo vault without a typed password. `not_found` when there is no demo vault. Never works for another vault. |

`device_set_time_zone`, `device_finish_setup`, `device_set_time_check`, `device_set_theme`, `device_set_clipboard_clear`, `device_set_open_in_search`, `device_set_list_sort`, `demo_spawn` and `demo_open` are **closed** while the gate is closed (deny by default, D19). Only `device_get` is open: the lock screens read the theme from it.

Changes to earlier shapes:

* `ProfileInfo` gains `demo?: boolean` (present and `true` only for the demo vault; set by `profile_list`, `vault_status` and `profile_rename`).
* `GateStatus` gains `lockedUntilMs: number | null` (the end of a running wait as checked UTC milliseconds, so the screen can say "try again at 14:35" in the chosen zone) and `clockPending: boolean` (a wait is running but its end cannot be judged yet: the device restarted and the time was not checked online since; nothing is counted or checked meanwhile).
* `GateOutcome` `locked` gains `clockPending: boolean` (then `retryAfterMs` is 0).
* Every stored time is UTC milliseconds since 1970. Nothing in the contract carries a zone except `DeviceInfo.timeZone`.

**Network.** `clock_*` and the background check are the only requests the core makes besides S3: three `HEAD` requests (about 20 s apart at most, at most once in 6 hours after a good check) to `google.com/generate_204`, `cloudflare.com/cdn-cgi/trace` and `aws.amazon.com`, reading only the `Date` header. No vault data, identifier or cookie is sent. They are skipped while the check is switched off.


## 12. Vault preferences: the generator's last settings [impl]

Stored sealed in the vault, copied to every backup target and picked up by the person's other devices (`STORAGE_AND_SYNC.md` §10, "Preferences that follow the vault"; decision D31). Both commands need an open vault (`locked` otherwise) and are closed while the gate is closed.

| Command | Arguments | Result | Notes |
|---|---|---|---|
| `prefs_get` | none | `PrefsView` | `{generator: GeneratorPrefs \| null, folders: string[]}` (`folders`: 0.4.0, §13). `null` until something was saved (the page then offers its defaults: 20 characters, every type on, look-alikes avoided). A stored setting this build cannot use is shown as `null`. |
| `prefs_set_generator` | `{ generator: GeneratorPrefs }` | `PrefsView` | `GeneratorPrefs` is `{length, lower, upper, digits, symbols, avoidAmbiguous}`. `invalid_input` (fixed sentences, no input echoed) when `length` is not 4 to 128 ("The password length must be between 4 and 128.") or no character type is on ("Select at least one character type."); nothing changes then. Saving the settings that are already saved writes and sends nothing. Otherwise the change is saved and queued for every enabled target. |

Front end: the slider offers 8 to 64; a saved length outside it is shown at the nearest end. The Generator page and the item editor share one set of settings (`app.generator` in `state/app.ts`): a change shows in both at once, is saved about 0.6 s after the last change (and when the generator closes or the vault locks), and is re-read after every unlock and whenever a pull from the backup changed something (`syncRevision`). With every character type switched off the page says so and saves nothing. A failed read or save never blocks the vault: the settings on screen stay and the next change saves again.


## 13. Folders, search, merging between devices and coming back online (0.4.0) [impl]

Shapes are those of `src/backend/types.ts` / `tauri.ts`. Folders are kept with the vault's synced preferences (§12); the search runs in the core because only it can read an item; the merge runs in the core's pull (`STORAGE_AND_SYNC.md` §6.3).

**Folders.** A folder is a path of names joined with `/` (`Work/Clients` is Clients inside Work, the way Bitwarden writes nested folders, so an import keeps its structure). At most 6 levels, 64 characters per name, 128 in all; empty names and control characters are refused (`invalid_input`, a fixed sentence); at most 1000 folders made on purpose; a name that matches an existing folder without regard to capitals reuses that folder's spelling. An item carries its folder path in `ItemDraft.folder` (cleaned on every save: split, trimmed, empty parts dropped). `PrefsView.folders` lists the folders made **on purpose**, so an empty one can exist; the screen shows those plus every folder an item sits in plus their parents. Every command here needs an open vault and is closed while the gate is closed.

```ts
interface FolderChange { path: string | null; moved: number; prefs: PrefsView }   // the folder as spelled, items changed, the list now
interface SearchHit { id: string; matched: string[]; oldVersion: number | null }
interface PurgePlan { deletedAt: number; recent: boolean; olderAt: number; sum: string | null }   // §3, item_purge_plan
```

| Command | Arguments | Result | Notes |
|---|---|---|---|
| `folder_create` | `path` | `FolderChange` | makes the folder and the ones above it; making one that exists changes nothing |
| `folder_rename` | `from`, `to` | `FolderChange` | renames or moves a folder; everything inside comes along (`moved` items get a new version each). `to` inside `from` is `invalid_input` ("A folder cannot be moved inside itself."); an unknown `from` is `not_found`; a `to` that already exists (compared without regard to capitals) merges the two, spelled the way the existing one is |
| `folder_remove` | `path` | `FolderChange` | removes the folder only: its items and subfolders move up one level (`moved` counts the items); nothing is deleted |
| `item_move` | `ids`, `folder?` | `number` | puts the items in `folder` (`null`: out of every folder). Each changed item gets a new version (`changedFields: ["folder"]`); nothing is revealed, so it works for items that ask for the master password. Answers with how many changed |
| `item_search` | `query`, `oldVersions` | `SearchHit[]` | the deep search. Words are matched one by one, in any order and in any field (`github work` finds the item called GitHub in the folder Work), lower-cased. **Searched:** titles, folders, tags, the type's name, every field that is not secret, the names of custom fields and the values of those that are not hidden, and the free-text fields people write notes in (`notes`, `body`, `description`), except for an item that asks for the master password again (its notes stay sealed, search included). **Never:** a password, card number, security code, PIN, identity number, one-time-code secret, private key or passphrase, nor the value of a hidden custom field (their *labels* are searched). `oldVersions` also reads the versions an item had before and reports the newest one that matches (`oldVersion`); an item whose current text matches is reported once, as a current match. The answer is ids and the **names** of the fields that matched (at most three, for example `["Website", "Notes"]`), never a value. At most 1000 hits; a query over 200 characters is `invalid_input`; at most 8 words are used. Trashed items are never searched |
| `sync_reconnect` | – | `void` | trigger 4 of the sync (§6): the page calls it from the window's `online` event. The background worker pulls what other devices changed, merges it with this device's own changes, and sends what was queued while offline. Answers at once, never an error for network problems; with the vault locked (or no backup target) it does nothing. Asking while a pull is already running changes nothing |

**Offline.** Nothing in the app needs a connection. Items are made, read, edited, searched and deleted locally; each change is written to the vault and, when a backup target exists, to a durable queue. A failed send stays queued and is tried again at the next trigger: unlock, a committed change, leaving the app, and now the device coming back online. There is still no timer.

## 14. The scanner, passkeys and the Android passkey provider (2026-11-25, D47 to D51) [impl]

Design: `PASSKEYS.md`. Everything here is core-side; the screen never sees a passkey's private key.

### 14.1 Passkey fields in items

* An item draft or view of a login may hold the fields `passkey`, `passkey#2` … `passkey#10` (`FieldKind` `passkey`, secret). In a **view** (`ItemView.fields[]`) such a field has `value: null`, `hasValue: true` and a `passkey: PasskeyInfo`
  (`credentialId`, `rpId`, `rpName`, `userName`, `displayName`, `alg`, `createdAt`). There is no reveal and no copy for a passkey (it has nothing a person could type).
* In an **editable draft** (`item_begin_edit`, `item_reveal`, `history_get`) the value is a marker `{"keep":true,"credentialId":…,…info}`. `item_update` replaces a marker by the stored record with the same `credentialId`;
  a marker for a credential the item does not hold, or a field that is neither a marker nor absent, answers `invalid_input`. Removing the field from the draft removes the passkey.
* `item_create` answers `invalid_input` for any passkey field (a passkey is made by a website's request or by an import, never by a form). A clone or a copy to another account carries none.
* `ItemSummary.hasPasskey: bool`. `item_search` also matches the site and the user name of a passkey.
* Error codes: `passkey_unsupported` (the request offered no algorithm this version can make, only ES256), `passkey_excluded` (the website listed a credential the vault already holds).

### 14.2 The scanner

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `scan_classify` | `text`, `bareKey?` | `ScanReply` | sorts a scanned or pasted text. `bareKey` is true only for typed or pasted text: a camera never reads a bare key. Needs an unlocked vault (`locked`). Nothing in the reply is secret: the logins named are the ones the list shows |
| `scan_apply` | `text`, `choice`, `password?` | `ItemSummary` | `choice` is `{"existing": "<id>"}` or `{"new": {"title": string \| null}}`. Adds the code of `text` as the next free `totp#n` of that login, or makes a login for it (user name from the account, title from `title` or the issuer, site from the issuer when it looks like a domain). A re-prompt login needs `password` (`password_required`); a login with no free slot or a text that is not a one-time code answers `invalid_input`; the same code twice answers `invalid_input` |

`ScanReply` (`kind` decides): `totp` `{issuer, account, digits, period, suggestions: LoginSuggestion[]}` · `migration` · `fido` · `link` `{host}` · `text` · `invalid` `{message}`.
`LoginSuggestion`: `{id, title, subtitle, reasons: ("account"|"site"|"name")[], score, codes, passkeys, hasThisCode, hasSitePasskey, full, reprompt}`; at most eight, strongest first (account 100, site 60, name 40).
The screen lists these first and then every other login, and always offers "Create a new login".

### 14.3 The passkey provider (Settings > Passkeys)

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `passkey_provider_status` | – | `{supported, enabled}` | `supported`: Android 14 or later with the Credential Manager; everywhere else `{false,false}` |
| `passkey_provider_set` | `on` | `{supported, enabled}` | needs the vault fully open (`require_full`: the master password, not a PIN-limited vault); switches the service component on or off. Off Android: `storage` "Passkeys can be switched on only on an Android phone." |
| `passkey_provider_open_settings` | – | `void` | opens Android's own passkey settings, where the person still has to choose Hexis |
| `device_set_passkey_app_check` | `on` | `DeviceInfo` | `DeviceInfo.passkeyAppCheck` (default true): whether the provider may fetch a website's `assetlinks.json` for a native app. Off: apps get no passkey (`app_check_off`), no request is made, browsers are unaffected |

### 14.4 The provider's own entry (Kotlin to Rust, not a Tauri command)

`Provider::call(op, argsJson) -> {"ok":true,"data":…} | {"ok":false,"code","message"}` through the two JNI functions
`Java_dev_habibullah_hexis_provider_ProviderNative_{init,call}`. Ops: `state`, `unlock` (master password), `lock`, `getOptions` (a request: the matching logins and the others), `sign`, `createOptions`, `create`.
Callers are `{"kind":"browser","origin","clientDataHash"}` or `{"kind":"app","package","certSha256"}`; `userVerified` is reported by Kotlin after `BiometricPrompt`. Codes: every code of the list at the top, plus `app_locked`
(Stage 1 is closed: every op), `app_check_off` and `password_required` (a re-prompt login, or a vault opened with the PIN: the sheet asks for the master password and sends the same call again). The e2e shim exposes it as `POST /e2e/provider {"op","args"}`
(dev only), so the scripts can make and use a passkey for a made-up website without a phone.

### 14.5 Bitwarden import and export

`import_preview` / `import_commit` read `login.fido2Credentials` of a Bitwarden JSON file (ECDSA P-256 only; others are skipped with a warning naming the site); `ImportPreview` counts a passkey-only record as a login.
`export_plain` writes `fido2Credentials` into Bitwarden JSON (master password, as before); CSV skips passkeys and says so in `warnings`. Details: `PASSKEYS.md` §7.
