# STORAGE_AND_SYNC — item storage, version history, trash, multi-S3 sync

Design for Phases 2 and 4. Status legend: **[impl]** implemented and tested (Phase 2: local storage, history, trash) · **[plan]** designed, not yet implemented (Phase 4: S3 sync, mirrors, conflicts, tombstone upload). All blobs are envelopes from `CRYPTO_SPEC.md` §7. The IPC surface over this layer is `IPC_CONTRACT.md`.

## 1. Object model [impl]

Every item (login, TOTP, secure note, media entry) is identified by a random UUIDv4 (lowercase, hyphenated). Each item has:

* a **latest state** — the current decrypted payload plus lifecycle flags;
* an append-only list of **version snapshots** — never modified, never deleted except by permanent purge;
* zero or more **attachments** — separate encrypted blobs referenced from the payload.

### Decrypted item payload (inside `items/<id>.enc`, schema 1)

```json
{
  "schema": 1,
  "id": "<uuid>",
  "type": "login | totp | secureNote | media",
  "title": "…", "favorite": false, "tags": [], "folder": null,
  "fields": { "...": "type-specific: username, password, urls[], totp{uri}, noteBody, …" },
  "customFields": [{ "name": "…", "value": "…", "hidden": true }],
  "attachments": [{ "id": "<uuid>", "name": "…", "mime": "…", "size": 0, "sha256": "…" }],
  "createdAt": 1732360000000, "updatedAt": 1732360000000,
  "deleted": false, "deletedAt": null,
  "event": "created | edited | trashed | restored | restoredFromVersion",
  "restoredFrom": null,
  "changedFields": ["password"]
}
```

`deleted/deletedAt` live **inside** the ciphertext, so the bucket cannot tell trash from active items.

## 2. Object layout (S3 keys and local mirror of them) [impl locally, S3 upload is plan]

| Key | Content | Mutability |
|-----|---------|------------|
| `vault/meta/keybundle.json` | public key bundle (`CRYPTO_SPEC.md` §8) | replaced only by change-password |
| `vault/items/<uuid>.enc` | latest item state (envelope `itemId=<uuid>`, `version=n`) | overwritten with conditional write (§5) |
| `vault/versions/<uuid>/<version>_<timestamp_ms>.enc` | immutable snapshot of version `n` | write-once |
| `vault/attachments/<attachment-uuid>.bin` | encrypted file/image | write-once |
| `vault/conflicts/<uuid>_<timestamp_ms>.enc` | losing side of a concurrent edit | write-once |
| `vault/tombstones/<uuid>.enc` | proof of permanent deletion (addition to the spec, see §6) | write-once |

Additions beyond the original spec: `meta/keybundle.json` (needed so a new device can unlock after the first pull) and `tombstones/`.

## 3. Version history rules [impl]

* An update **never** overwrites previous data. Saving produces: (1) a new snapshot `versions/<id>/<n+1>_<ts>.enc`, then (2) the new latest state with `version = n+1`.
* Every state change bumps `version`, including trash/restore, so sync can compare a single integer. Those events appear in the timeline as lifecycle entries with unchanged content.
* UI: latest version on top; collapsible "Version History" timeline `v1, v2, v3…` with exact timestamp and `changedFields`. Viewing a historical secret requires the Master-Password Re-Prompt.
* **Restore this version** = create a new snapshot `n+1` whose content equals the old one (`event = restoredFromVersion`, `restoredFrom = k`). History stays linear; nothing is rewritten.
* Two devices creating the same `n+1` offline produce two snapshot files (different timestamps). Both are kept; the conflict flow (§5) decides what becomes latest.

## 4. Trash lifecycle [impl locally; deleting on S3 is plan]

1. **Delete** → new version with `deleted = true, deletedAt = <ms>`, `event = trashed`. No data removed.
2. **No auto-purge.** Items stay in Trash until the user acts.
3. **Trash view** returns only `{ id, title, deletedAt }`. Passwords, TOTP, notes and attachments cannot be opened, copied or revealed (the Rust commands refuse for `deleted` items — policy is not UI-only). Title is the only payload field the trash listing command is allowed to return.
4. **Restore** → new version with `deleted = false`, `event = restored`; full access returns.
5. **Permanent delete** is hidden behind a double confirmation: step 1 warning modal; step 2 re-enter Master Password **and** type the item name or `CONFIRM`. Then, in order: write `tombstones/<id>.enc` to the primary (and mirrors), delete `items/`, `versions/<id>/`, referenced `attachments/`, `conflicts/` for that id on primary and every mirror, then purge local rows. Tombstones exist because otherwise another device that still holds the item would re-upload it ("resurrection").

## 5. Local database (Phase 2) [impl]

SQLite via `rusqlite` (bundled). It stores **only opaque envelopes** plus sync bookkeeping, so the file needs no SQLCipher. Titles/tags are searched from an in-memory index built after unlock.

| Table | Columns (sketch) |
|-------|------------------|
| `meta` | `key`, `value` (schema version, device id, high-water marks) |
| `items` | `id` PK, `version`, `ts`, `blob`, `remote_etag`, `base_version`, `dirty` |
| `versions` | `item_id`, `version`, `ts`, `blob`, `uploaded` — PK `(item_id, version, ts)` |
| `attachments` | `id` PK, `item_id`, `blob`/path, `size`, `uploaded` |
| `conflicts` | `id`, `item_id`, `ts`, `blob`, `resolved` |
| `tombstones` | `id` PK, `ts`, `uploaded` |
| `targets` | `id`, `role` (primary/mirror), `config_blob` (S3 endpoint/keys sealed with `meta` sub-key), `status` |
| `outbox` | `seq`, `target_id`, `op` (put/delete), `key`, `attempts`, `next_try_at`, `last_error` |

Local plaintext leakage (accepted, documented): item ids, versions, timestamps, blob sizes, dirty flags, S3 target ids/status. Titles, URLs, usernames, trash state and S3 credentials are inside ciphertext.
Writes are transactional: snapshot + latest + outbox rows commit together or not at all. The app is fully usable offline; the outbox drains when a network is available.

## 6. Sync protocol [plan]

**First-time setup requires internet:** fetch `meta/keybundle.json`, unlock, then pull everything. After that, the app runs offline.

**Conditional writes.** `PUT items/<id>.enc` with `If-Match: <remote_etag>` (new items: `If-None-Match: *`). Immutable objects (versions, attachments, conflicts, tombstones) use `If-None-Match: *`.
Support differs between providers (AWS S3 added conditional writes in 2024; B2, R2, MinIO, Storj, Scaleway behave differently). At target setup run a **capability probe** (two writes to a probe key) and store the result per target. Targets without conditional writes degrade to `HEAD`-then-`PUT` optimistic checking and are flagged "weak-conditional" in the UI. ETags are treated as opaque strings.

**Push (primary).** For each dirty item: upload new snapshot → conditional PUT of latest. `412 Precondition Failed` ⇒ conflict.

**Pull.** `ListObjectsV2(prefix=vault/items/)`; for each object whose ETag differs from `remote_etag`: GET, authenticate, check header `itemId` equals the key, compare `version`.
* never apply a remote `version` lower than the local high-water mark (rollback guard);
* remote newer and local clean ⇒ fast-forward;
* local dirty and remote ETag changed ⇒ **conflict**.

**Conflict handling (never silently overwrite).**
1. Upload the local losing state to `vault/conflicts/<id>_<ts>.enc` and keep a `conflicts` row.
2. Local latest becomes the remote state (so the device converges), the conflict is flagged in the UI.
3. Side-by-side merge UI: pick per field or edit; saving writes version `max(remote, local)+1` on top of the remote ETag, `event = edited`, and marks the conflict resolved. Both original versions remain in history. Conflict objects are never auto-purged.
4. Ties and ordering use UTC milliseconds, then `version`, then object ETag as a stable tiebreak; clock skew never decides data loss because both sides are preserved.

**Tombstones** win over any later non-tombstoned state for the same id.

## 7. Mirrors (primary + N) [plan]

* The **primary** is the source of truth for conflicts. Mirrors only replicate; they never merge.
* Each mirror has its own outbox cursor. Every successful primary write enqueues a copy job per mirror (same key, same bytes). Jobs are idempotent (plain PUT; immutable keys use `If-None-Match: *` and treat 412 as success).
* Background worker, bounded concurrency, exponential backoff with jitter, per-target pause/resume.
* **Health indicator per bucket:** `healthy` (last success < 5 min, empty queue) · `lagging` (queue age above threshold) · `failing` (≥ N consecutive errors, shows last error) · `disabled`. Also shows queue length and last success time.
* Recovery: "promote mirror to primary" re-verifies every object by authentication before switching. A periodic reconcile job lists mirror keys and re-enqueues anything missing.
* Cost note: aws-sdk-s3 is large to compile (the dev machine has ~7 GB RAM); evaluate `rust-s3` vs `aws-sdk-s3` with a spike at the start of Phase 4, including conditional-header support.

## 8. Security notes for sync [plan]

* The bucket sees: object keys (UUIDs), sizes, timestamps, update frequency, number of items/versions. It does not see content, titles, trash state, or credentials.
* S3 target credentials are stored only locally, sealed with a `meta` sub-key; exporting them requires the Master-Password Re-Prompt. They are never written to the bucket.
* Every downloaded object is authenticated before use; a malicious bucket can delete or roll back data but cannot forge or silently alter it. Keep at least one mirror on a different provider for resilience against deletion.
* Attachments: one-shot envelope up to 64 MiB in v1; larger files need the chunked envelope (Phase 5).

## 9. Phase 2 implementation notes [impl]

What shipped in `src-tauri/src/storage/` and `src-tauri/src/commands/` (sections 1-5 above; sections 6-8 are still plan).

### Schema as built (`PRAGMA user_version = 1`, `application_id = 0x504D5631`)

- `meta(key, value)` holds `bundle` (the key-bundle JSON, so the vault is self-contained locally), `device_id`, `created_at`.
- `items(id PK, version, ts, blob, remote_etag, base_version, dirty)` is the latest sealed state (`KeyPurpose::Item`, header itemId = id, version = n).
- `versions(item_id, version, ts, blob, uploaded)`, PK `(item_id, version, ts)`, are immutable snapshots (`KeyPurpose::Version`). Rows are only ever inserted.
- `attachments`, `conflicts`, `tombstones` exist with the §5 columns. They are `ON DELETE CASCADE` from `items`, except `tombstones`, which is standalone. Attachments and conflicts are not written yet (Phase 3+).
- `targets` and `outbox` are **not created yet**. They belong to the sync phase, and a migration v2 adds them.
- `items`, `versions`, `attachments` and `conflicts` are ordinary rowid tables, not `WITHOUT ROWID`, because blob-heavy rows are faster that way. `meta` and `tombstones` are small and keyed by text, so they are `WITHOUT ROWID`.
- Pragmas: `journal_mode=WAL` (verified), `synchronous=FULL`, `foreign_keys=ON`, `secure_delete=ON`, `trusted_schema=OFF`, `cell_size_check=ON`.
- Migrations run in one transaction and are idempotent. A file with another `application_id`, a newer `user_version`, or a non-SQLite file is refused.

### Writes

`commit_state` writes the snapshot and the latest row in **one transaction**, with `version = previous + 1` and a check that the previous version matches. A duplicate snapshot rolls everything back. Restore-from-version writes a **new** version (`event = restored`, `restoredFrom = n`), so history stays linear and old snapshots stay byte-identical. `purge` deletes every row for the id (items, cascade rows, versions), inserts the tombstone, then checkpoints the WAL with `TRUNCATE`.

### Index

Built at unlock by opening every `items` blob. Active items keep an `ItemSummary` (title, type, tags, folder, favorite, subtitle, hasTotp, version, updatedAt). Trashed items keep only `title` and `deletedAt`, so trash listing cannot leak other fields. An item that fails authentication is skipped and counted in `VaultStatus.damagedItems`. This is an additive field, and one tampered row does not lock the whole vault. The index lives only inside the unlocked session and is dropped on lock.

### Re-prompt guard and edit tokens

- Gated commands (`item_reveal`, `item_begin_edit`, `history_get`, `history_restore`, `item_purge`, `vault_change_password`) run Argon2id on the supplied password **every call**. The password is never cached. Verification runs **outside** the session lock, so a slow KDF cannot block the UI's other calls.
- Edit tokens are 32 random bytes, base64url. They are single-use, bound to `(item id, version at begin)`, have a TTL (default 600 s), and are wiped on lock. At most 16 are outstanding (the oldest is dropped first). The presented token is compared in constant time against every outstanding one. Unknown, expired, or locked-away tokens fail with `token_invalid` and change nothing. A real token presented for a different item or version is burned and fails with `token_invalid`. A real token is also consumed on success and on `version_conflict`, `in_trash` or `not_found`. A save rejected for `invalid_input` keeps the token, so the person can fix the form without re-entering the password.
- `item_update` also checks `expectedVersion` against the stored version (`version_conflict`).
- A poisoned session mutex resets the session to Locked.

### Deviations from the contract (all additive or internal)

1. `VaultStatus.damagedItems` is an extra field, described above.
2. `targets` and `outbox` are not created in Phase 2, because nothing writes them yet.
3. UUIDv4 ids are generated from `crypto::rng` with a strict canonical-form validator, and no `uuid` crate is used.
4. The vault bundle is stored in `meta.bundle` so the vault file is self-contained.

Not a deviation, but worth knowing: command arguments are camelCase exactly as in `IPC_CONTRACT.md`. Tauri rejects snake_case names, and a mock-runtime test pins this.
