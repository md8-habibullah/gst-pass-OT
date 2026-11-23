# IPC_CONTRACT — Tauri commands between the UI and the Rust core

Single source of truth for the boundary in `ARCHITECTURE.md` §4. The React UI (`src/backend/`) and the Rust command layer (`src-tauri/src/commands/`) both implement exactly this. Change it here first.

Conventions: JSON over Tauri IPC; **camelCase** on the wire (`#[serde(rename_all = "camelCase")]`); JS passes camelCase arguments (Tauri maps them to snake_case Rust parameters); timestamps are UTC milliseconds; ids are lowercase UUIDv4. Every error is `{ "code": string, "message": string }` (the message is safe to show; it never contains secrets).

Error codes: `locked` · `no_vault` · `vault_exists` · `wrong_password` · `not_found` · `in_trash` · `not_in_trash` · `version_conflict` · `invalid_input` · `confirmation_mismatch` · `token_invalid` · `storage` · `crypto`.

## 1. Re-prompt rule (from the spec, enforced in Rust)

Editing any entry, revealing a password/secret, viewing or restoring history snapshots, permanent deletion and exporting **always** require the master password, even while unlocked. Rust verifies it with Argon2id (`verify_password`) on every such call and **never caches the password**.

* One-shot operations take `password` in the same command (`item_reveal`, `history_get`, `history_restore`, `item_purge`, `vault_change_password`).
* Editing spans two calls, so `item_begin_edit(id, password)` returns a random single-use **edit token** (256-bit, bound to item id + version, expires after 10 minutes, invalidated by lock). `item_update` must present it. The token carries no secret.
* Creating, trashing and restoring-from-trash do **not** require the password (spec lists neither); history *listing* shows only event, time and changed field names, so it does not either.
* TOTP codes are shown without a prompt (they rotate every period); the TOTP **secret** is never returned by any command except inside `item_reveal` / `item_begin_edit` / `history_get` (all password-gated).

## 2. Types

```ts
type ItemType = "login" | "totp" | "secureNote" | "media";

interface SuiteInfo { /* unchanged, see src-tauri/src/crypto/mod.rs */ }

interface VaultStatus {
  exists: boolean;                    // a vault file with a key bundle exists
  unlocked: boolean;
  capabilities: { quickUnlock: boolean; biometric: boolean };  // false until Phase 3
  damagedItems: number;               // additive (Phase 2): items that failed authentication at unlock and were
                                      // skipped instead of locking the whole vault; 0 normally. The UI shows a
                                      // warning toast after unlock when this is > 0. Optional in the TS type
                                      // because the preview backend omits it.
}

// Planned for Phase 3 (not implemented; the UI hides PIN / biometric entry points because the real backend
// lacks the optional methods): quick_unlock_set_pin, quick_unlock_pin(pin), quick_unlock_biometric,
// quick_unlock_clear, lock-timer settings. The TypeScript `Backend` interface already declares the two unlock
// calls as optional (`quickUnlockPin?`, `quickUnlockBiometric?`).

interface ItemSummary {               // list rows; no secrets
  id: string; type: ItemType; title: string;
  subtitle: string | null;            // username or url host, for the row's second line
  favorite: boolean; tags: string[]; folder: string | null;
  hasTotp: boolean; updatedAt: number; version: number;
}

type FieldKind = "text" | "url" | "password" | "note" | "totp";
interface FieldView {                 // masked view of one field
  key: string; label: string; kind: FieldKind;
  secret: boolean;                    // password / note / totp are secret
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
  event: "created" | "edited" | "trashed" | "restored" | "restoredFromVersion";
  changedFields: string[];            // field keys / "title" / "tags" ...; never values
  restoredFrom: number | null;
}

interface TotpNow { code: string; period: number; digits: number; secondsRemaining: number; validUntilMs: number }
```

### Field schemas (keys inside `ItemDraft.fields`)

| Type | Keys (in display order) |
|------|--------------------------|
| `login` | `username` text · `password` password · `url` url · `totp` totp · `notes` note |
| `totp` | `issuer` text · `account` text · `totp` totp |
| `secureNote` | `body` note |
| `media` | `description` note (attachments arrive in Phase 5) |

`totp` accepts an `otpauth://totp/...` URI **or** a bare base32 secret; Rust validates with `TotpAccount::from_uri` / `new` and rejects with `invalid_input`. Unknown keys are rejected. Empty values are dropped.

## 3. Commands

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `crypto_suite_info` | – | `SuiteInfo` | exists today |
| `vault_status` | – | `VaultStatus` | works while locked |
| `vault_create` | `password` | `void` | `vault_exists` if one exists; leaves the vault **unlocked**; enforces min length 8 (`invalid_input`); Argon2 default params |
| `vault_unlock` | `password` | `void` | `wrong_password` / `no_vault`; builds the in-memory index |
| `vault_lock` | – | `void` | drops the VaultRootKey, index and edit tokens (zeroized) |
| `vault_change_password` | `oldPassword`, `newPassword` | `void` | re-seals the keyring only (`change_password`) |
| `item_list` | – | `ItemSummary[]` | active items only, newest first, from the in-memory index |
| `item_get` | `id` | `ItemView` | `in_trash` for trashed items (trash policy lives here, not in the UI) |
| `item_reveal` | `id`, `password` | `ItemDraft` | password-gated; `in_trash` refused |
| `item_begin_edit` | `id`, `password` | `{ token: string; draft: ItemDraft; version: number }` | password-gated |
| `item_create` | `draft` | `ItemSummary` | version 1 |
| `item_update` | `id`, `token`, `expectedVersion`, `draft` | `ItemSummary` | consumes the token; `version_conflict` if `expectedVersion` ≠ stored; `changedFields` computed in Rust |
| `item_trash` | `id` | `void` | new version, `deleted=true` |
| `item_restore` | `id` | `ItemSummary` | from trash; new version |
| `trash_list` | – | `TrashEntry[]` | **only** id, title, deletedAt |
| `item_purge` | `id`, `password`, `confirmation` | `void` | must be in trash (`not_in_trash`); `confirmation` must equal the item title (trimmed) or the literal `CONFIRM` (`confirmation_mismatch`); writes tombstone, removes items/versions/attachments/conflicts rows |
| `history_list` | `id` | `VersionEntry[]` | newest first; refused for trashed items |
| `history_get` | `id`, `version`, `password` | `ItemDraft` | password-gated |
| `history_restore` | `id`, `version`, `password` | `ItemSummary` | new snapshot `n+1`, `event = restoredFromVersion` |
| `totp_now` | `id` | `TotpNow` | active items with a `totp` field only |

Nothing returns keys, seeds, hashes or the master password. S3 and export commands arrive with Phases 4–5.

## 4. Behavioural guarantees the tests must cover

* Every state change (create, update, trash, restore, restore-from-version) writes an immutable snapshot **and** the new latest state atomically, with `version = previous + 1`.
* Trashed items: `item_get`, `item_reveal`, `item_begin_edit`, `history_*`, `totp_now` all fail with `in_trash`; `trash_list` exposes only id/title/deletedAt.
* Nothing is auto-purged. `item_purge` is the only deletion path and requires the password, the in-trash state and the confirmation text.
* A tombstoned id can never be re-created or re-imported.
* After `vault_lock`, every command except `vault_status`, `vault_unlock`, `vault_create` and `crypto_suite_info` returns `locked`.
