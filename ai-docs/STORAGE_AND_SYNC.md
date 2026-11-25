# STORAGE_AND_SYNC — item storage, version history, trash, multi-S3 sync

Design for Phases 2 and 4. Status legend: **[impl]** implemented and tested (Phase 2: local storage, history, trash; **Phase 4a: backup targets, single-writer push, restore into a fresh profile, see §10; R3a: event-driven sync without timers, pull and reconcile, other-device copies, see §6 and §10**) · **[plan]** designed, not yet implemented (a conflict-object merge screen; attachments; mirror promotion). All blobs are envelopes from `CRYPTO_SPEC.md` §7. The IPC surface over this layer is `IPC_CONTRACT.md`.

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
  "type": "login | card | identity | sshKey | secureNote | totp | media",
  "title": "…", "favorite": false, "tags": [], "folder": null,
  "fields": { "...": "type-specific keys, see IPC_CONTRACT.md field schemas (username, password, number, publicKey, …)" },
  "customFields": [{ "name": "…", "value": "…", "hidden": true }],
  "attachments": [{ "id": "<uuid>", "name": "…", "mime": "…", "size": 0, "sha256": "…" }],
  "createdAt": 1732360000000, "updatedAt": 1732360000000,
  "deleted": false, "deletedAt": null,
  "archived": false,
  "event": "created | edited | trashed | restored | restoredFromVersion | archived | unarchived",
  "restoredFrom": null,
  "changedFields": ["password"]
}
```

`deleted/deletedAt` and `archived` live **inside** the ciphertext, so the bucket cannot tell trash or archive from active items. `archived` was added after payload schema 1 shipped without a schema bump: it is `#[serde(default)]` on read, so every payload written before it simply means "not archived", and a regression test plants such a payload and opens it.

## 2. Object layout (S3 keys and local mirror of them) [impl; uploaded since Phase 4a, except attachments and conflicts]

| Key | Content | Mutability |
|-----|---------|------------|
| `vault/meta/keybundle.json` | public key bundle (`CRYPTO_SPEC.md` §8) | replaced only by change-password |
| `vault/items/<uuid>.enc` | latest item state (envelope `itemId=<uuid>`, `version=n`) | overwritten with conditional write (§5) |
| `vault/versions/<uuid>/<version>_<timestamp_ms>.enc` | immutable snapshot of version `n` | write-once |
| `vault/attachments/<attachment-uuid>.bin` | encrypted file/image | write-once |
| `vault/conflicts/<uuid>_<timestamp_ms>.enc` | losing side of a concurrent edit | write-once |
| `vault/tombstones/<uuid>.enc` | proof of permanent deletion (addition to the spec, see §6) | write-once |
| `vault/meta/prefs.enc` | the vault's preferences (the password generator's last settings), sealed, envelope `itemId=prefs` (§10 "Preferences that follow the vault") | overwritten; the newest version wins |

Additions beyond the original spec: `meta/keybundle.json` (needed so a new device can unlock after the first pull) and `tombstones/`.

## 3. Version history rules [impl]

* An update **never** overwrites previous data. Saving produces: (1) a new snapshot `versions/<id>/<n+1>_<ts>.enc`, then (2) the new latest state with `version = n+1`.
* Every state change bumps `version`, including trash/restore, so sync can compare a single integer. Those events appear in the timeline as lifecycle entries with unchanged content.
* UI: latest version on top; collapsible "Version History" timeline `v1, v2, v3…` with exact timestamp and `changedFields`. Viewing a historical secret requires the Master-Password Re-Prompt.
* **Restore this version** = create a new snapshot `n+1` whose content equals the old one (`event = restoredFromVersion`, `restoredFrom = k`). History stays linear; nothing is rewritten.
* Two devices creating the same `n+1` offline produce two snapshot files (different timestamps). Both are kept. Since R3a (§6) the pull keeps the copy it did not choose as a snapshot flagged "from another device", and the person decides what becomes latest by restoring it or not. A new state's number is `max(every snapshot number of the item, the head) + 1`, so numbers can have gaps: an other-device copy is stored under a local number above everything the item has and records the number it had on the other device (`remote_version`).

## 4. Trash lifecycle [impl; tombstone upload and remote deletion since Phase 4a]

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

## 6. Sync protocol [impl: push since Phase 4a, pull and reconcile since R3a (queue item Q0); conflict objects and a merge screen are not built]

**First-time setup requires internet:** fetch `meta/keybundle.json`, unlock, then pull everything (`profile_restore_from_backup` downloads the objects, §10; the first unlock authenticates them). After that, the app runs offline.

**Conditional writes.** `PUT items/<id>.enc` with `If-Match: <remote_etag>` (new items: `If-None-Match: *`). Immutable objects (versions, attachments, conflicts, tombstones) use `If-None-Match: *`.
Support differs between providers (AWS S3 added conditional writes in 2024; B2, R2, MinIO, Storj, Scaleway behave differently). At target setup run a **capability probe** (two writes to a probe key) and store the result per target. Targets without conditional writes degrade to `HEAD`-then-`PUT` optimistic checking and are flagged "weak-conditional" in the UI. ETags are treated as opaque strings.

**Push.** For each queued item: upload the new snapshot, then conditionally PUT the latest state (§10, "The single-writer rule").

**When it runs: four triggers, no timers (decision D18, extended by D42).** Unlock, a committed local change, app exit or backgrounding, and the device coming back online (added in 0.4.0 at the owner's request: work done offline syncs on reconnect); plus the person's own "Back up now". Nothing retries, polls or wakes up by itself (§10, "Worker, triggers, health").

### Pull and reconcile (R3a)

Runs on the replicator thread right after unlock (the unlock never waits for it) and at the start of every "Back up now"; the push drain follows it. One pass:

1. **Source.** The master target, or the first enabled target (by position) when there is no master. Every other target is a replica: it is written to and never read for content. If the source cannot be reached, the failure is recorded in its health (a fixed sentence) and the push side carries on.
2. **List, compare ETags.** `vault/tombstones/` and `vault/items/` are listed (paged; more than 100 000 objects, or a repeated page token, is an error). An object whose ETag equals the one stored in `remote_objects` is already known and is **not downloaded**: a pass over an unchanged bucket costs the listings and no GET (a test counts them). ETags are compared as opaque strings after dropping quotes and a weak `W/` marker.
3. **Tombstones first, and they win.** A listed tombstone is authenticated (`tombstone-<id>`, `meta` sub-key, header id equals key id) and then erases the item here exactly like a local permanent delete, except that the source is not told what it already holds; the other targets are, through the normal queue. A tombstone that does not authenticate is counted and ignored (it hides nothing). A latest state listed beside an authenticated tombstone is skipped.
4. **Authenticate every object.** Each changed latest state is downloaded and opened by the vault: key id equals header id, tag verifies, payload parses and carries that id. A failure is **skipped and counted** (`VaultStatus.remoteDamaged`); nothing of an object that did not authenticate is ever applied.
5. **Rollback guard.** `remote_objects` remembers the highest version ever seen at each key. A state with a lower version is ignored (a bucket that was rolled back, or a stale copy); it is recorded so that the next push replaces it.
6. **Classify by version and lineage.** Every state's sealed payload records the state it replaced as `parent: {version, ts}` (missing only on a first version). Following parents from one state shows whether it descends from another. Versions strictly decrease along a chain, so a walk stops as soon as it has gone past its goal, and it never takes more than 5 000 steps. A chain that ends at version 1 is a proper beginning (the goal is not an ancestor); a chain that ends above version 1 (a state written before parents were recorded, or a missing snapshot) cannot be proven and is treated as **diverged**, the safe answer. Against the local head:

| Case | What the pull does |
|------|--------------------|
| identical state | nothing |
| no such item here | take it over **with its history**: the snapshots under `vault/versions/<id>/` (at most 2 000, each authenticated, a failing one counted) |
| the remote state descends from the local head (fast-forward) | take over the remote state, with the snapshots missing on the way (fetched one step at a time along the parent pointers) |
| the local head descends from the remote state (local is ahead) | nothing; the push side replaces the remote copy |
| neither (diverged) | **merged automatically** (0.4.0): the remote state and the states the other device went through since the two last agreed are kept as snapshots "from another device", and the two heads are merged field by field into one new local version (below). When no merge can be made, the local head stays and the remote state is kept as one snapshot for the person to restore (below) |

7. **Apply in one transaction, race-safe.** Deciding and writing happen in a single short visit under the session lock, and the write re-checks that the local head is still what the decision was based on. If the person edited the item in between, the pull steps back (`Raced`) and leaves it to the next trigger. Afterwards the vault refreshes its search index and `syncRevision` goes up by one, so the page knows to re-read its lists.

**The replicator never holds a key.** It reaches the unlocked vault only through `VaultLink::with_vault`, which runs a closure on the vault of *this* unlock under the session lock; if the vault was locked (or locked and unlocked again) the pass stops quietly. Network calls never happen under that lock.

**Diverged states are merged (0.4.0; before that the other device's state was only kept for the person to restore).** Code: `storage/merge.rs` (the rules, no I/O), `backup/pull_apply.rs` (`auto_merge`, `store_copy`, `merge_base`) and `backup/pull.rs` (`remote_chain`).

1. **Keep the other device's side.** The remote head, and the states the other device went through since the two sides last agreed (walked back along the sealed `parent` pointers, downloading each snapshot under `vault/versions/<id>/`, at most 200 states and 2 000 ancestors), are re-sealed locally as snapshots under **unique local numbers above everything the item has** (so version numbers can have gaps, §3), with `origin = 'remote'`, `remote_version` (the number on the other device) and `reviewed = 0`. They are ordinary immutable snapshots: they appear in `history_list` with `otherDevice: true`, can be viewed (re-prompt applies) and restored, and are never uploaded under their local numbers (they already exist in the bucket under the other device's key).
2. **Find the base**: the newest state both sides have in their lineage. Without one (a state written before parents were recorded) the merge runs with no base.
3. **Merge** the local head and the remote head against the base (rules below). The result is written as a new local version (event `edited`, `restored_from` = the local number of the remote head, which is how the history labels it "Combined with the other device's version N") whose `parent` is the **remote** head, so it descends from both lines. The other device fast-forwards to it at its next pull. Both earlier states stay in the history.
4. **Bookkeeping.** When the merge was made the remote state is recorded under its own key (`record_main`), so the next push replaces it with `If-Match`; when no merge could be made it is recorded under `<key>#merged` as before.
5. **No merge** is made (the local head stays, the remote state is one other-device snapshot as before) when the two states are of different item types, when the merged content does not pass the normal validation, or when the walk back is too long. The item then shows `changedElsewhere` and the person restores the copy by hand (`history_restore` of an other-device copy makes the new head a descendant of both lines and marks the copies reviewed).

*The merge rules.* Each slot (a field, the title, the folder, a flag) is a three-way choice against the base: only one side changed it, take that side; both changed it to the same value, keep it; both changed it differently, the side with the **newer `updated_at`** wins and the loser stays in the history (an exact tie takes the greater value, so two devices merging the same pair agree). A value one side removed and the other edited keeps the edit (`keep_data`: a merge never throws away typed data). Tags, web addresses (a line each, at most 20) and attachments are sets: what either side added is kept, what one side removed and the other did not touch is removed. Custom fields merge by name plus occurrence. The **notes** (`notes`, `body`, `description`, any note-kind field) are merged line by line with a three-way diff: changes in different places both go in; when both sides changed the same lines both versions are written one after the other between the markers `[Changed on two devices. Keep what you want and delete the rest.]`, `[The version from the other device]` and `[End of the two versions]`, so no line is dropped. A note over 4 000 000 diff cells, or a merged note over 64 KiB, falls back to the newer side's text. The trash flag follows the newer decision. `reprompt` stays on when there is no base and the sides differ. **Never added to the payload:** a merge uses existing fields only (`ItemPayload` rejects unknown keys, and an older build must still open every state).

*Left as before.* An other-device copy that was already stored unreviewed by an earlier build is not merged after the fact. Two devices that each create a different **empty** folder offline can lose the earlier one (the folder list is one value, newest write wins, §10); folders that hold items are never lost.

**Offline and coming back online.** Offline is a normal state. The vault, the search and every edit work with no connection; a send that fails stays in the durable queue. The page calls `sync_reconnect` from the window's `online` event (trigger 4, below): the worker pulls what other devices changed, merges it with the changes made here, and sends the queue.

**Original design not built.** `vault/conflicts/<id>_<ts>.enc`, a `conflicts` table and a side-by-side merge screen were planned. R3a keeps the losing line as an other-device snapshot in the history instead (nothing extra in the bucket); the key prefix `conflicts/` stays reserved. Ties and ordering never decide data loss because both sides are preserved.

**Tombstones** win over any later non-tombstoned state for the same id.

## 7. Mirrors (primary + N) [impl as "master first, then the others" (Phase 4a, §10); the master is also the pull source (R3a)]

* The **primary** is the source of truth for conflicts. Mirrors only replicate; they never merge. Since R3a the pull reads only the master (or the first enabled target when there is no master); every other target is written to and never read for content.
* Each mirror has its own outbox cursor. Every successful primary write enqueues a copy job per mirror (same key, same bytes). Jobs are idempotent (plain PUT; immutable keys use `If-None-Match: *` and treat 412 as success).
* Background worker: event-driven, **no timers and no backoff**; a failed job stays queued and is retried at the next trigger (§10). Per-target pause/resume is the `enabled` flag.
* **Health indicator per bucket:** `healthy` (last success < 5 min, empty queue) · `lagging` (queue age above threshold) · `failing` (≥ N consecutive errors, shows last error) · `disabled`. Also shows queue length and last success time.
* Recovery: "promote mirror to primary" re-verifies every object by authentication before switching. A periodic reconcile job that lists mirror keys and re-enqueues anything missing is **not built** (the engine has no timers); a replica is filled from the queue and by the full seed when it is added or re-enabled.
* Cost note: aws-sdk-s3 is large to compile (the dev machine has ~7 GB RAM); evaluate `rust-s3` vs `aws-sdk-s3` with a spike at the start of Phase 4, including conditional-header support.

## 8. Security notes for sync [impl for what Phase 4a sends; the rest is plan]

* The bucket sees: object keys (UUIDs), sizes, timestamps, update frequency, number of items/versions. It does not see content, titles, trash state, or credentials.
* S3 target credentials are stored only locally, sealed with a `meta` sub-key; exporting them requires the Master-Password Re-Prompt. They are never written to the bucket.
* Every downloaded object is authenticated before use; a malicious bucket can delete or roll back data but cannot forge or silently alter it. Keep at least one mirror on a different provider for resilience against deletion.
* Attachments: one-shot envelope up to 64 MiB in v1; larger files need the chunked envelope (Phase 5).

## 9. Phase 2 implementation notes [impl]

What shipped in `src-tauri/src/storage/` and `src-tauri/src/commands/` (sections 1-5 above; sections 6-8 were still plan then; Phase 4a is §10).

### Schema as built (`PRAGMA user_version = 1`, `application_id = 0x504D5631`)

- `meta(key, value)` holds `bundle` (the key-bundle JSON, so the vault is self-contained locally), `device_id`, `created_at`.
- `items(id PK, version, ts, blob, remote_etag, base_version, dirty)` is the latest sealed state (`KeyPurpose::Item`, header itemId = id, version = n).
- `versions(item_id, version, ts, blob, uploaded)`, PK `(item_id, version, ts)`, are immutable snapshots (`KeyPurpose::Version`). Rows are only ever inserted.
- `attachments`, `conflicts`, `tombstones` exist with the §5 columns. They are `ON DELETE CASCADE` from `items`, except `tombstones`, which is standalone. Attachments and conflicts are not written yet (Phase 3+).
- `targets` and `outbox` were **not created in Phase 2**. Migration v2 (Phase 4a, §10) adds them.
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
2. `targets` and `outbox` are not created in Phase 2, because nothing writes them yet (they exist since schema v2, §10).
3. UUIDv4 ids are generated from `crypto::rng` with a strict canonical-form validator, and no `uuid` crate is used.
4. The vault bundle is stored in `meta.bundle` so the vault file is self-contained.

Not a deviation, but worth knowing: command arguments are camelCase exactly as in `IPC_CONTRACT.md`. Tauri rejects snake_case names, and a mock-runtime test pins this.

### Phase 2b: accounts, archive, clone, vault files [impl]

Code: `storage/profiles.rs` (registry and display names), `storage/files.rs` (atomic writes, private files, the `FilePicker` seam, vault-file header checks), `storage/app.rs` (`AppCore`: registry + the one selected profile + its `Session`), `commands/mod.rs` (thin IPC layer and the production `DialogPicker`). The crypto module is unchanged.

**Several accounts, one app.** Each account is a complete, separate vault: its own master password, key bundle, SQLite file and ids. Nothing is shared between accounts, so one account's password can never open another, and unlocking one reveals nothing about the others.

- Layout: `<data dir>/profiles.json` and `<data dir>/vaults/<id>/vault.db`. `<id>` is a random UUIDv4, never the display name, so a name can never form a path. A vault from the single-vault era (`<data dir>/vault.db`) is **adopted in place** as "My vault": registry `dir` is `"."`, the file is not moved, rewritten or even opened until the person selects it. A test pins that it stays byte-identical across adoption, select and a wrong password.
- `profiles.json` is **plain JSON and not secret**: the login screen must list accounts before any unlock. It holds `id`, `displayName`, `createdAt`, `lastUsedAt` and `dir`, nothing about the vault's contents. The display names are therefore visible to anyone who can read the data folder. This is a documented trade-off. The folder and file are owner-only (0700 / 0600 on Unix).
- Writes are atomic (temp file, fsync, rename, fsync of the folder). The in-memory list is rolled back if saving fails. A registry written by a newer Hexis makes the app refuse to start and is never touched. A registry that does not parse, or that fails validation (duplicate ids or names, a `dir` other than `.` or exactly `vaults/<id>`), is renamed to `profiles.json.corrupt-<ms>` (never deleted) and rebuilt from the folders on disk (`Recovered vault 1..n`). Validating `dir` is also what makes a tampered registry harmless: nothing outside `vaults/<uuid>` can ever reach a delete.
- Display names: Unicode NFKC, trimmed, whitespace collapsed, 1-40 characters, no control, invisible-format, bidi or private-use characters. Uniqueness compares `nfkc(lowercase(nfkc(name)))`, so `Zoë` composed, decomposed, upper-case and fullwidth variants are the same name.
- Exactly one account is selected and at most one is unlocked. Selecting, creating or removing locks the previous one first, which drops its keys, its index and its edit tokens. After a restart nothing is selected; `lastUsedAt` (set on select and unlock) only decides which account the login screen offers first.
- `profile_create` makes the folder, the vault and the registry entry as one unit: on any failure its folder is removed again, and an existing file named `vaults` is left alone. Create, import and remove are serialized, so two of them can never trip over each other's half-made folders; two creates with the same name cannot both win.
- `profile_remove` checks the account's own master password with Argon2id (without unlocking it, even when another account is the selected one) and a typed confirmation, then deletes exactly that account's folder (or, for the adopted file, only `vault.db`, `-wal` and `-shm`). It refuses a folder that is a link or not directly under `vaults/`. If the vault file is already gone there is nothing left to protect, so no password is needed to drop the entry.

**Archive and clone.** Archive is the sealed `archived` flag plus a versioned event (`archived` / `unarchived`, empty `changedFields`), written through the same one-transaction state change as every other edit, and idempotent (archiving an archived item adds no version). `item_list` returns active and archived items, never trashed ones; the UI decides what to show. Trashing keeps the flag, so restoring from the trash returns the item to the archive. Restoring an old *version* restores content only; the current archive flag is kept. Clone runs entirely in Rust after the password check: it reveals the item internally and creates a new item (version 1, title plus ` (copy)`, cut to fit the 256-character limit, not archived); only an `ItemSummary` crosses back, so the secrets never reach the UI. A refused clone creates nothing.

**Vault files in and out.** The native dialogs are opened from Rust only, through the `FilePicker` trait (`tauri-plugin-dialog` in the app, fixed paths in tests and in the e2e shim). The capability file grants the webview no `dialog:*` permission, so the UI can neither open a dialog nor name a path.

- *Export* (`vault_export_file`): the password is verified first, so a wrong one never opens a dialog; then the dialog runs on the blocking pool with no lock held; then SQLite `VACUUM INTO` writes a consistent, compacted copy into a private (0600) `<name>.part` file next to the chosen name, which is renamed into place and the folder fsynced. It never overwrites the live vault and refuses any destination inside the app's own data folder. The copy is fully encrypted already, so no further protection is added. The hook `strip_device_only_targets` runs on the copy; in Phase 2b it was a no-op; since Phase 4a (§10) it removes the targets that are not marked "save in master backup", the queue and all device-only state.
- *Import* (`profile_import_file`): the name is validated before the dialog opens. The chosen file is checked **before SQLite parses it**, by reading the first 100 bytes: SQLite magic, `application_id`, a `user_version` this build understands (a newer one says "Update the app to import it"), and a size limit. The file is then copied (never moved, never modified) into a new private account folder and checked more deeply: the key-bundle JSON must parse and validate, the vault must have a device id, `PRAGMA quick_check` must pass and the item rows must be readable. The copy gets a **new device id**, because it is a new replica. Any failure removes the new folder and changes nothing else. It stays locked and opens with the master password it was created with.

**What is in the database file, and what is not.** All item content, titles, tags, folders, field names, custom-field names, archive state and trash state live only inside sealed envelopes. A test creates logins, a card, an SSH key and a trashed item with distinctive markers, then scans `vault.db`, `vault.db-wal` and `vault.db-shm` (while unlocked and after locking) and the registry for every title, username, password, URL, note, tag, folder, custom-field name, card number, private key and the master password, and for the display name outside the registry: none is found. What a reader of the file does learn: that it is a Hexis vault, the public key bundle (public by design), a random device id, the number of items and versions, their random ids, their version numbers and timestamps, and the sizes of the sealed blobs. There is no SQLCipher: the rows are opaque envelopes already, and encrypting them twice would add nothing but a second key to manage.

**File permissions (fixed in this phase).** SQLite gives the `-wal` and `-shm` files the mode of the main file at the moment it creates them. Phase 2 created the vault file with the default mode and changed it to 0600 afterwards, which left the other two readable by group and others (the 0700 data folder kept other users out, and the contents are encrypted either way). The vault file is now created owner-only *before* SQLite opens it, and the `vaults/` and account folders are created 0700, so all three files are 0600 from the first byte. A test checks the modes of a new, an imported and a single-vault-era vault.

Further deviations: the `ProfileList` / `VaultStatus.profile` shapes, the error codes `profile_name_taken` and `unsupported_file`, and everything else in `IPC_CONTRACT.md` §5 are as written there. Where behaviour is stricter or more specific than the contract, it is listed in `IPC_CONTRACT.md` §5.3.

## 10. Phase 4a: backup targets, single-writer replication, restore [impl]

Code: `src-tauri/src/storage/backup/` (`types`, `remote`, `s3`, `dbops`, `probe`, `replicator`, `vault_ext`, `restore`, and since R3a `pull` and `pull_apply`), wired through `session.rs`, `app.rs` and `commands/mod.rs`. Wire shapes and per-command behaviour: `IPC_CONTRACT.md` §6 and §6.1. This section is the storage side.

### Schema v2 (`PRAGMA user_version = 2`)

Migration v2 runs in one transaction and upgrades a v1 vault **in place**: every existing row stays byte-identical (a test builds a v1 file by hand with a key bundle, an item, two versions and a tombstone, migrates it, checks every value and that the old tombstone is flagged for a sealed proof, and reopens it; another test checks that a v1 file with a foreign `application_id` is still refused). A file with a newer version or another `application_id` is still refused.

| Table | Purpose |
|-------|---------|
| `targets` | one row per bucket: `config` (**sealed** envelope `target-<uuid>`, `meta` sub-key, holds name, endpoint, region, bucket, prefix, key id, **secret**, path style), flags (`is_master` with a partial unique index so exactly one row can be the master, `enabled`, `share_in_master`, `position`), `seeded`, `conditional` (strong/weak/unknown), and health counters (`last_success_at`, `last_checked_at`, `last_error`, `consecutive_errors`, `hard_error`) |
| `outbox` | the queue: `kind` (bundle, tombstone, version, item, purge, targets), `ref`, `version`, `ts`, `rank`, `gen`, `attempts`, `next_try_at` (still in the schema, **no longer written** since R3a: nothing is scheduled), `blocked`, `last_error`. Unique per `(target, kind, ref, version, ts)`, so enqueueing the same thing again coalesces (an item edited five times while offline is **one** `item` job that uploads the latest state, plus one `version` job per snapshot) |
| `remote_objects` | what this device last saw at each key of each target (ETag, item version). The single-writer rule needs it |
| `tombstones.blob` | the sealed tombstone proof (`tombstone-<uuid>`, `meta` sub-key), so it can be uploaded and restored; tombstones written by an older build get theirs at the next unlock |
| `meta.shared_targets` / `meta.pending_targets` | the sealed shared-targets snapshot this device holds / one downloaded by a restore, waiting for the first unlock (base64) |

**Local plaintext leakage added by this phase:** the number of targets, which one is the master, whether a target is enabled or shared, its health counters, its last error text (fixed sentences only, never a bucket or key name), and the queue (item ids, versions, timestamps). Names, endpoints, buckets, prefixes, key ids and secrets are inside the sealed envelope. A regression test (`app_tests.rs`, built on R1's scanner) adds targets, edits one, replicates, and finds none of them in `vault.db`, `-wal`, `-shm` or the registry.

### Schema v3 (`PRAGMA user_version = 3`, R3a)

One migration step for the `versions` table, applied (v1 then v2 then v3 as needed) inside **one IMMEDIATE transaction**, so a failure leaves the file at its old version (a test checks it): `origin TEXT NOT NULL DEFAULT 'local'` (`'local'` or `'remote'`), `remote_version INTEGER` (the number the state had on the other device) and `reviewed INTEGER NOT NULL DEFAULT 1`. Every existing row becomes `origin = 'local'`, `reviewed = 1`, byte-identical otherwise (tests migrate hand-built v1 and v2 files and compare every value). The sealed item payload gained `parent: {version, ts}` (optional, skipped when absent, so older payloads still open). **Local plaintext leakage added:** which snapshots came from another device, the other device's version number, and whether the person has looked at them; no content.

### Writing to the queue

Every storage mutation enqueues its jobs for **every enabled target in the same transaction** as the change (`db.rs`: create/update state, purge, change of the key bundle). Enqueue order is the upload order: key bundle, tombstones, version snapshots, latest item states, purges, then the shared-targets snapshot (master only). Adding or re-enabling a target, or changing where it points, runs a **seed**: a full upload job set built from the vault (bundle, tombstones that have a proof, every version, every item). Disabling a target empties its queue and forgets what it knew. A seed uploads only the device's own snapshots (`origin = 'local'`); an other-device copy exists in the bucket already under the other device's key.

### The client (`s3.rs`)

`rusty-s3` 0.12.0 builds SigV4 presigned URLs (UNSIGNED-PAYLOAD, no request body hashing); blocking `ureq` 3.5.2 with rustls does the transfer. `ObjectStore` (put/get/head/delete/list) is the seam; `S3Store` is the real one, `MemStore` (tests only) an in-memory bucket with fault injection and the same conditional semantics. Rules enforced in `S3Store`: https required (http only for loopback), no credentials in the URL, **no redirects are followed** (a 3xx is an error, so a signed request is never replayed elsewhere), no system proxy for loopback, timeouts (connect 10 s, first byte 30 s, body 60 s, total 120 s), size caps (72 MiB per object, 1 MiB for metadata objects, 8 MiB per listing page), listing pagination by continuation token, and a key prefix per target. Errors are an enum with **fixed wording**: a message never contains the endpoint, the bucket, a key, the signature or a presigned URL. Mapping: 404 on the bucket → "bucket not found"; `InvalidAccessKeyId` → unknown key id; `SignatureDoesNotMatch` and a region mismatch → "secret key or region"; `RequestTimeTooSkewed` and an expired presigned URL (`Request has expired`) → "this device's clock"; other 403 → access refused; 412 → precondition failed; 429/5xx/409 → retryable; 3xx → redirect refused.

### Conditional writes and the probe

`backup_test` and every add/update run the probe of §6: write `vault/probe/<random>.tmp` with `If-None-Match: *`, read it back, repeat the write (must fail with 412) and write with a wrong `If-Match` (must fail with 412), list `vault/`, delete. Outcomes: **strong** (both refused), **weak** (a write whose precondition was false was accepted: the service ignores the headers), **unknown** (the headers were refused with an error, or the probe could not finish). The result is stored per target and decides the push mode:

* **strong:** a mutable object is written with `If-Match <etag we last saw>` (new objects `If-None-Match: *`); a 412 or 404 means someone else changed it, so the device reads it, compares the header version and decides (below).
* **weak / unknown:** read first, then write unconditionally. Best effort, **not atomic**: two devices writing at the same moment can still lose one write. The UI shows this per target (§6 "weak-conditional").
* Immutable objects (version snapshots, tombstones): `If-None-Match: *`, and a 412 counts as success (the object is already there); on weak targets a HEAD comes first.

### The single-writer rule

The push **never overwrites remote data this device has not seen**. For a mutable object (latest item state, `targets.enc`) the device reads the remote header (`peek_header`, unauthenticated, only to compare) and asks whether it already knows that state (`snapshot_known`: a snapshot with that timestamp and that number, either its own or the `remote_version` of an other-device copy). Same version and same bytes: adopt the ETag, nothing to send. Then:

| Remote copy | Known to this device | Result |
|-------------|----------------------|--------|
| older | yes | overwrite (guarded by `If-Match` on strong targets) |
| older | no | **blocked**: `MSG_UNSEEN_REMOTE`, the job stays queued |
| newer, or a different copy of the same version | yes (already merged into the history) | **parked**: the job completes without writing; the next edit supersedes it |
| newer, or a different copy of the same version | no | **blocked**: `MSG_NEWER_REMOTE` / `MSG_DIVERGED` |
| not a Hexis object | n/a | **blocked**: `MSG_FOREIGN_OBJECT` |

A blocked job stays in the queue with a fixed reason, the target shows `failing`, and the job is looked at again at the **next trigger** (normally the pull of that trigger has merged the other copy by then, which turns the case into one of the "known" rows). Objects that are not an item's latest state (`targets.enc`) have no lineage: an older copy is simply replaced. The key bundle: a different `public_keys` at `vault/meta/keybundle.json` means **another vault lives there** and is a hard failure ("choose an empty folder"); the same vault with different bytes (a changed password re-wrapped it) is replaced. A permanent delete uploads the tombstone first, then deletes `vault/items/<id>.enc` and every key under `vault/versions/<id>/` (listing it, so versions another run uploaded are found too).

**Bookkeeping (`remote_objects`).** A remote state that **grows into this device's own** (identical, taken over, in this device's lineage, new, or a stale rollback) is stored under its own key, so the next push replaces it with `If-Match`. A state that was **kept apart** (diverged, or unreadable here) is stored under `<key>#merged`: the next pass does not download it again, but a push cannot replace it blindly (a strong target answers 412 and the conflict check above parks it). The rollback guard compares with the highest version over both rows.

### Worker, triggers, health (no timers, R3a / D18)

One thread per unlocked vault (`Replicator`), started by the first trigger once an enabled target exists (`attach_backup` at unlock only attaches the engine and its link) and stopped (joined) by `Vault`'s drop, which is what lock, profile switch and app exit all run, so no request outlives the keys. It owns its own SQLite connection to the vault file (WAL), kept under a mutex that also serialises "a drain" so a pull, a push and a flush never overlap; it takes jobs in `rank` order per target (master first) and never holds the session lock during network I/O.

**The worker sleeps on a condition variable and has no timeout.** It wakes only for one of four triggers (the person's "Back up now" is one more way in, with the same effect):

| Trigger | Where it comes from | What runs |
|---------|---------------------|-----------|
| **Unlock** | `Session::install` -> `Vault::start_backup` -> `Replicator::request_reconcile` | on the worker: pull and reconcile from the source (§6), then drain every target |
| **A committed local change** | `Vault::wake_backup`, called after every mutation (create, edit, trash, restore, archive, history restore, purge, password change) and after a change of the target list | on the worker: drain every target (no pull) |
| **App exit or backgrounding** | `AppCore::flush_for_exit` (8 s, `EXIT_FLUSH`), the Linux `CloseRequested` hook, `WindowEvent::Suspended` under `cfg(mobile)`, and the IPC `sync_flush` for the page's `visibilitychange` | `Replicator::flush(deadline)`: drain with a deadline, on its own thread, no pull; returns whether the queue is empty |
| **The device is back online** (0.4.0) | the page's `online` event -> IPC `sync_reconnect` -> `Session::sync_request` -> `Vault::request_sync` -> `Replicator::request_reconcile` | on the worker: pull, reconcile and merge from the source (§6), then drain every target. Asking while a pull is running changes nothing; a locked vault does nothing. Nothing is polled: the system tells the page when the connection returns |
| "Back up now" (`backup_run`) | `Replicator::run_now` from the calling thread | pull and reconcile, then drain (forced: an empty queue still checks the connection) |

A job that fails stays queued with its error text and attempt count and is tried again **only at the next trigger**: no retry schedule, no exponential backoff, no polling, no heartbeat, `lagging` is read from the clock only when health is asked for. A drained worker makes **no store calls at all** (a test records the `MemStore` log over a settle period). `Timing` shrank to `lagging_after`, `failing_after` and `background` (`false`: no thread, for tests and scripted runs); `Timing::default()` is 5 minutes and 5. One trigger counts as **one failed attempt** for a target even when both its pull and its push fail (`target_record_failure` once, then `target_note_failure`, which updates the text without counting). `scripts/check_no_notifications.py` checks that no notification or tray plugin, permission or Android manifest entry exists (the app never produces an OS notification).

**Exit flush behaviour.** Linux `CloseRequested`: if the queue is empty the window closes at once; otherwise the close is prevented, the flush runs on its own thread (never on the UI thread) and the window is destroyed when it returns or after 8 s; a **second** close request while that is running closes immediately. Nothing is lost on a timeout: the queue is durable and is drained at the next unlock. See `BUILD_TARGETS.md` for what Android can and cannot guarantee.

Health is **computed**, not stored: `untested` (no success and no check yet) -> `healthy` -> `lagging` (a failed attempt since the last success, or the oldest unblocked job older than 5 minutes) -> `failing` (5 consecutive failed triggers, or a hard failure, or a blocked job) / `disabled`. Every failure text shown is one of the fixed sentences.

### Restore (`profile_restore_from_backup`)

Needs no unlocked vault. After the name and settings checks it: GETs `vault/meta/keybundle.json` (must parse and validate), lists everything under `vault/` (paged; at most 100 000 objects; a repeated token is an error), creates the vault file with the bundle, and in **one transaction** inserts tombstones (header id must be `tombstone-<id>`), then items (header `itemId` must equal the key's id), then version snapshots (id, version and timestamp must equal the key; skipped when the item is erased), then the downloaded `targets.enc` into `meta.pending_targets`. Objects whose keys do not have the Hexis shape are ignored; shaped keys with a wrong header abort the whole restore and the new folder is removed. Everything downloaded is sealed and is authenticated when it is first opened after unlock; a damaged item then counts in `damagedItems` like any other. At the first unlock `attach_backup` imports `pending_targets` (decrypting it with the vault's `meta` key: same ids, `seeded = 0`, a master only if none exists, names that collide are skipped), then seeds. **A restored master therefore re-verifies every object once** (it downloads the item states again to compare headers, because the restored device cannot know what the bucket already holds), which is correct but not free on a large vault.

### Preferences that follow the vault (the generator's last settings)

Code: `storage/backup/prefs.rs`, plus small parts in `replicator.rs`, `pull.rs`, `restore.rs` and `dbops.rs`. Contract: `IPC_CONTRACT.md` §12. Decision: D31.

* **What is kept.** One JSON document `{v, generator: {length, lower, upper, digits, symbols, avoidAmbiguous}, folders: [...], ...}` (`folders`: 0.4.0, the folders made on purpose, `IPC_CONTRACT.md` §13; at most 1000, each path 128 characters at most), sealed as the object `prefs` with the vault's `meta` sub-key, in `meta` row `prefs` (base64). The generated passwords are never stored; only the chosen settings. A saved setting must be usable (length 4 to 128, at least one character type) or the core answers `invalid_input` and changes nothing. Saving what is already saved writes and sends nothing.
* **Newest write wins.** The envelope version is a millisecond stamp that is always higher than the version this device holds (`max(now, held + 1)`), so a later change on any device replaces an earlier one, and a device whose clock is wrong cannot lock the others out: it adopts the higher version it reads and its next change goes above it. There is no merge: the whole document is one value (two devices that change it at nearly the same moment keep the later one, on both).
* **Fields this build does not know are kept** when it saves (`#[serde(flatten)]`), so an older build never erases a setting a newer one added.
* **Push.** `set_generator_prefs` writes the row and enqueues one `targets` job with the ref `prefs` (the shared-targets snapshot has an empty ref) for every enabled target in the same transaction, so no schema migration was needed. The job sends `vault/meta/prefs.enc` under the single-writer rule. A blocked write (the bucket holds a newer copy this device has not pulled) completes as obsolete and raises no alarm; the next pull takes the newer copy. A seed (new target, re-enabled target, moved bucket) adds the job when a `prefs` row exists.
* **Pull.** Each pass on the source target makes **one `HEAD`** for the object and downloads it only when its ETag differs from the one stored in `remote_objects`; an unchanged bucket costs one request and no download. The object is authenticated and parsed before anything is stored, then: newer than this device's copy, adopted (and queued for the other targets, `syncRevision` goes up so the page re-reads); same, nothing; older, this device's copy replaces it at the next push. A copy that **does not authenticate** (a flipped bit, a planted file) is never applied and is counted in `remoteDamaged`; this device's own copy is queued to replace it. A copy that authenticates but is in a **shape this build cannot read** (a newer app wrote it) is left exactly as it is and is not counted.
* **Restore.** A restore downloads `vault/meta/prefs.enc` when it is listed and its header says `prefs` with a version above 0; it is kept sealed and is authenticated the first time the vault reads it. A copy that does not open means "nothing saved", never an error.
* **Local plaintext leakage added:** a `prefs` row exists once something was saved (its size, not its content, is visible), and the outbox may hold a `targets` job with ref `prefs`. Neither names a setting or a value.
* **Tests.** `backup/tests/prefs_tests.rs` (20): nothing saved or sent by default; an open vault is needed; sealed (no setting name in the stored or the uploaded bytes); refusal of unusable settings; no new version for an unchanged save; strictly rising versions; push to every enabled target and to a target added later; a settled vault makes one `HEAD` and no `PUT` or `GET`; restore; a change on one device reaching the other; both devices changed it (the later wins on both); a blocked write not shown as an error; a damaged copy ignored, counted and replaced; a non-envelope ignored; a newer app's extra fields surviving; a copy in an unreadable shape not overwritten; a stored copy that does not open meaning defaults; services that ignore or refuse preconditions never overwriting a newer copy. `e2e/real-sync.mjs` runs the two-device case through the browser against the real core and the mock S3 service.

### Deviations from the design text above

1. **`remote_objects`** and **`tombstones.blob`** are additions to the §5 table sketch (the sketch has `items.remote_etag`/`dirty` and `outbox.op/key`; the real queue is per-kind with coalescing, and ETags are per target, not per item).
2. §7 speaks of *primary* and *mirrors*; Phase 4a has a **master** (replicated first, holds the shared-targets snapshot) and *other targets*. Since R3a the master (or, without one, the first enabled target) is the pull source (§6); no other target merges or resolves anything. Promotion ("verify every object, then switch") is not built, but a master can be changed at any time with `backup_set_master`, which only changes the order and where the snapshot goes.
3. **Pull, the rollback guard and the reconcile are built (R3a, §6).** **Not built yet:** conflict objects under `vault/conflicts/` and the side-by-side merge screen (a diverged copy is kept as an other-device snapshot in the history instead), a periodic reconcile job (there are no timers), attachments, promotion of a mirror with verification, reading from more than one target (only the source is pulled; a replica is only written), and a pull of a replica that has objects the source lacks.
4. **Add and update save even when the probe fails** (the failure is shown as the target's health). The contract text implies a test first; saving anyway avoids retyping credentials when a service is briefly down.
5. **The first target is not made master automatically.** A vault has no master until one is chosen (`master: true` or `backup_set_master`); until then targets are served in the order they were added.
6. **Stricter than the contract:** endpoint rules (https only, loopback http, no credentials/query/fragment), field alphabets and lengths, at most 12 targets, a disabled target cannot be master.
7. **Restore is sequential and has no progress report**; it holds the account list's lock while it downloads. Tombstones are checked structurally at restore (envelope header), not authenticated, because no key exists yet and `vault_unlock` does not re-verify them in this phase; a hostile bucket could therefore make a restored device hide an item it has a forged tombstone for. A hostile bucket can already delete or roll back data (§8), so no new power. Since R3a the **pull** authenticates every tombstone before using it (a forged one is counted and ignored); the restore path and `vault_unlock` still do not, so verifying tombstones at first unlock stays open.
8. **A demoted master keeps its stale snapshot object** in its bucket until the next full upload there.
9. `tauri-plugin-opener` and (phones only) `tauri-plugin-barcode-scanner` were added for the UI's needs: `IPC_CONTRACT.md` §5.4 and `BUILD_TARGETS.md` §7.
10. **R3a: lineage by sealed parent pointers.** The original §6 text decided "fast-forward or conflict" by whether the local head exists as a snapshot of the remote chain; that is unsound as soon as a branch snapshot sits in the bucket (an other-device copy or a restore), because existence says nothing about ancestry. Each state therefore records the state it replaced (`ItemPayload.parent`), and states written before R3a, which have none, count as diverged unless they are a first version. A build older than R3a that reads a payload carrying `parent` and rejects unknown fields would refuse it; this is pre-release, so no compatibility shim was added.
11. **R3a: version numbers can have gaps** (`max(all rows) + 1`, an other-device copy sits above everything). The history list is ordered by the number, so an other-device entry can sit above the head; the page should treat the first entry with `otherDevice: false` as the latest own state.
12. **R3a: a state that was merged and is newer is parked, not blocked.** The original text blocks every newer remote object; that would leave a target `failing` for good after the person has merged the other copy and chosen to keep their own. Known (merged) states are left alone and the job completes.
13. **R3a: `VaultStatus.remoteDamaged`** is an addition beyond the brief: a pull that skipped unauthenticated objects must be visible somewhere, and `damagedItems` is the unlock-time count of local rows.
14. **R3a: single-writer tests.** The old tests that expected a target to turn `failing` after one `backup_run` now use a push-only drain helper, because `backup_run` reconciles first and the pull merges the cause away; the behaviour they pin (never overwrite unseen newer data) is unchanged.

### How it was tested

Unit and integration tests in `storage/backup/` (queue coalescing and order, the health matrix, sealing, v1 migration, strip hook, worker start/stop/recover, restore round trip and cleanup, name and URL checks, single-writer cases against two simulated devices) run against `MemStore`. `storage/backup/mock_tests.rs` drives the **real** `S3Store` (presigning, ureq, XML parsing) against `scripts/mock_s3.py`, a stdlib-only server that verifies every SigV4 query signature on its own and supports conditional writes, paged listings, fault injection and clock-skew rejection. **R3a:** `storage/backup/tests/pull_tests.rs` simulates two devices on one `MemStore` (fast-forward with history, the state recording its parent, local ahead, diverged copy with flag, idempotence and review, restore of the other-device copy followed by the other device's fast-forward, both ends keeping both, the rollback guard, a tombstone reaching a replica but not going back to the source, a tombstone beating a listed item, damaged objects counted and never applied, a new remote item with history and a replica, an unreachable source, master versus first-enabled source, the unlock pull not blocking `vault_unlock`, and the race with a local edit); `mock_tests.rs::a_pull_works_over_real_signed_requests` runs a pull through the real `S3Store` against `scripts/mock_s3.py`; `tests.rs` has the "no timers" section (a settled worker makes no store calls, a failed job waits for the next trigger, unlock and flush are triggers, a flush returns by its deadline, every mutation path wakes the worker); `commands/tests.rs` drives `sync_flush`, `item_review_other_device` and the exit gate over the real IPC dispatch. **Not tested:** any real provider, real public-CA TLS, real clock skew, the Android build of `ring`/`ureq`, the Android lifecycle hook, and the real window `CloseRequested` path (only the Tauri-free gate logic is tested).

## 11. App-level disguise state: `disguise.json` (Q2) [impl]

`<data dir>/disguise.json` holds the chosen icon and name: `{"version":1,"presetId":"kerf"|null,"customName":"..."|null,"lowercase":false}`. It lives next to `profiles.json`, outside every vault.

* **Plaintext by design.** It leaks the chosen preset and custom name to anyone who can read the data folder, because the launcher entry (Linux), the Android alias state and the window title show the same thing in the clear anyway, and the app must apply the choice at start before any password is typed. It contains no key, password or vault data. Anyone with the data folder can see which disguise is active; the disguise hides the app from a casual look, not from forensic access.
* Written atomically with owner-only permissions (0600, fsynced, renamed). Read leniently: a missing, damaged, oversized (more than 4 KiB), wrong-version or invalid file means the defaults (original look); it never produces an error and never blocks start. The values are re-validated on load, so a hand-edited file cannot inject anything into the launcher entry.
* Not synced, not part of a backup or an archive, not per account.
* Removed by the Q1 wipe (`AppCore::disguise_wipe`: zero-overwrite, fsync, unlink, then the launcher entry and icon files are removed and the window is reset). The Linux launcher entry lives **outside** the data dir (`$XDG_DATA_HOME/applications/<identifier>.desktop`), and the icon files live in `$XDG_DATA_HOME/<identifier>/disguise/`, which in the app is a subfolder of the data dir (`app_data_dir()` is `$XDG_DATA_HOME/<identifier>`). That is why the wipe goes through the launcher (`disguise_wipe`) and not only through the data dir: the entry must be removed too, and the icon folder is removed only when nothing but our own files was in it, so Q1's "known paths" list should name `disguise.json` and `disguise/` but delegate their removal to `disguise_wipe()`.

## 12. Plaintext app-level files of the Stage 1 gate (Q1) [impl]

**`gate.json`** (data folder, owner-only 0600, atomic replace, file and folder fsynced). Plaintext JSON:
`version`, `mode` (`pin|password|grid`), `reversePanic`, `biometric`, `panicButton`, `credential` (Argon2id PHC
string), `reversed` (PHC string or `null`), `panicCode` (PHC string or `null`), `installKey` (32 random bytes,
base64, the MAC key seed), `counter` = `{attempts, epoch, tag}`. It never holds a length, a hint or any plaintext.
It leaks: that a gate exists, which mode it uses, which switches are on, whether a custom panic code is set,
how many failed attempts have happened (and, from the PHC strings, the hash cost), and the hashes themselves,
which are offline-guessable for short secrets (see the limits above). Removed when the gate is turned off.

**`app-settings.json`** (same folder, 0600, atomic): `{"lockAfterMinutes": 1|5|15|30|60|240, "lockAfterLeaveSeconds": 0|15|30|60|300|900|1800|3600|14400}`. It leaks only the lock
timeouts. A missing, unreadable or out-of-range file is the default (15 minutes; 300 seconds after leaving the app) and never an error.
**`gate.json`** also carries the wait after wrong passcodes under the same MAC as the attempt counter: `lockUntil` (checked UTC milliseconds, 0 = not known), `lockBoot` (a deadline on the boot clock, milliseconds) and `lockBootId` (which boot it belongs to). Both deadlines must pass; changing the device date moves neither (D27). The record's MAC version is `v3` with the boot fields, `v2` with only `lockUntil`; records written before the waits existed read as "no wait".

**Wipe** (`secure_wipe_all_local_data`, also `app_reset`, panic credential, panic mark and the 16th failure).
Order: (1) the open session is dropped (keys zeroized, backup worker joined); (2) **cryptographic erase first**:
for every vault file the `meta.bundle` row is overwritten with `zeroblob`, the WAL is checkpointed with
`TRUNCATE`, the file is fsynced (from here the vault cannot be opened with any password even if the process
dies); (3) every known file is overwritten with zeros in 64 KiB chunks, fsynced, truncated, unlinked, the
folder fsynced; (4) emptied known folders are removed; (5) in memory: the gate, the settings and the account
list are reset (`profile_list` is empty, the next screen is the welcome screen). Known paths only:
`profiles.json`, `gate.json`, `app-settings.json`, `disguise.json` (each with its temp name and
`.corrupt-<ms>` copies), a legacy `vault.db` (+ `-wal`, `-shm`, `-journal`) only when registered or a real Hexis
file, and `vaults/<uuid>/{vault.db,-wal,-shm,-journal,attachments/*}`. Anything else is left alone and reported in
the Rust return value (never to the webview). **Never** `remove_dir_all` on the data folder (the folder itself
stays). Pre-flight (nothing touched when it refuses): absolute path; not `/`, a drive root, fewer than two path
components, a user's home or a parent of it; no known name is a symlink or anything but a plain file/folder; when
the app believes it holds data, the folder must show a Hexis marker. Idempotent; continues after an error on
one file. Remote S3 copies are never touched.
Honest limits (D16): an overwrite cannot be proven on SSD/flash/copy-on-write file systems, which is why
step 2 exists. A copy of the key bundle from **before a master-password change** may sit in a free database
page or an old WAL frame until step 3 overwrites the file; if the process dies between 2 and 3, that old copy
is protected only by the old master password. Thumbnails, clipboard, swap and backups outside the folder are
not covered. A per-file failure leaves that file (reported in the return value) and the wipe still resets the
in-memory state; the vault key bundles were already destroyed.

**Wipe and the disguise (merge note, 2026-11-24).** `AppCore::wipe_with` calls `AppCore::disguise_wipe()` (Q2) after the known files are gone and before `GateCore::run_wipe_hooks()`: the Linux launcher entry, its icons and the window title go back to the original look. A launcher problem cannot fail the wipe. The generic hook list (`GateCore::add_wipe_hook`) stays empty.

## 13. The encrypted drive (Q3) [impl; schema v4]

**Local layout.** `<vault dir>/attachments/<uuid>.bin` (directory 0700, blobs 0600). A blob is written as `<uuid>.bin.tmp`, fsynced, renamed, and the directory fsynced; the row is committed only after the rename (a crash leaves at worst an orphan temp/blob file, which the next unlock sweeps: any `*.bin` no row accounts for and any `*.bin.tmp`).

**Schema v3 (merge note: with R3a this becomes v4, see the orchestrator notes).** Migration is a separate function `storage::drive::migrate_files_table(conn, from_version)`, called inside `Store::migrate`'s transaction:

```
files(id TEXT PK, item_id TEXT NULL, meta BLOB NOT NULL, meta_version INTEGER >= 1, blob_len INTEGER >= 0,
      created_at INTEGER, indexed INTEGER DEFAULT 1)     + index files_by_item(item_id)
outbox: rebuilt with the CHECK list extended by 'file', 'file_meta', 'file_purge' (rows, seq numbers and both indexes preserved)
```

`indexed = 0` marks rows a restore inserted without a key: the owner is not known until the sealed metadata is opened, which happens at the next unlock (`Vault::files_startup`). Such rows are invisible until then. `item_id` is **not** a foreign key: purging an item deletes its files explicitly, in the same transaction (`Store::purge` -> `drive::purge_files_of_item`), and the blobs right after.

**Plaintext in the local database / on disk (added by Q3):** the number of files, `blob_len`, `created_at`, `meta_version`, and which item each file belongs to (`item_id`). Names, types, sizes and content are only inside the sealed `meta` blob and the blob file. A test scans `vault.db`, `-wal`, `-shm` and every `attachments/*` file for a plaintext marker after adds, renames and deletes.

**Rules on the owner.** Files of an item in the trash are not reachable (`in_trash`) and not listed in Files; they come back with the item. Purging an item erases its files (rows, blobs, remote objects). Cloning an item does not copy files. `vault_export_file` strips the `files` table (the export carries no files; exported copies contain no blobs). Rows whose owner is missing or whose metadata fails authentication are left alone (hidden, not deleted).

**Remote layout (addition to section 2).**

```
vault/attachments/<uuid>.bin    sealed blob (PMDF). Immutable: written once with If-None-Match: *, a 412 counts as "already there"
vault/attachments/<uuid>.meta   sealed metadata envelope "file-<uuid>" (name, size, type, owner, times). Mutable: single-writer rule
```

The `.meta` object is an addition so a restore can learn names and owners without opening 50 MB blobs.

**Outbox kinds** (enqueued in the same transaction as the change, for every enabled target, like items; seeding a new target also queues all existing files):

| kind | rank | when | what runs |
|------|------|------|-----------|
| `file` | 6 | file added | upload the blob. Strong preconditions: `put_file(If-None-Match: *)`, 412 -> HEAD, same length = done, different length = blocked ("This backup already holds a different object under the name of one of your files, so it was not overwritten."). Weak targets: HEAD first, PUT only if absent |
| `file_meta` | 7 | file added, renamed | `replicator::push_envelope` (the existing single-writer rule: `If-Match` with the remembered ETag, 412 -> read and compare versions: newer remote = blocked "newer on the service", same version but different bytes = diverged) |
| `file_purge` | 4 | file or item deleted | delete `.meta` first, then `.bin` (deletes of missing objects succeed), then forget the bookkeeping |

* **A name never reaches a target before its data.** Rank 6 runs before rank 7; and if the blob job of the same file is still queued for that target (blocked or deferred), the metadata job is blocked behind it ("The name of a file is held back until its encrypted data has been backed up.").
* A file deleted before its first upload simply has its pending jobs dropped (the purge job issues two harmless deletes).
* A blob whose local file is missing or has the wrong length is `blocked` with a fixed message; the other jobs continue. Deleting the file clears it.
* The S3 client streams: uploads send the file as the request body with `Content-Length` (no whole-blob buffer), downloads write straight to a temp file; per-request timeout for these two operations is 30 minutes instead of the normal timeout. The in-memory bucket of the tests uses default methods that buffer. A mid-upload cannot be cancelled: lock does not wait for it, the transfer ends at its own timeout.
* New `RemoteError::LocalIo` ("A file on this device could not be read or written during the transfer."): retried at the next trigger, not the service's fault.

**Restore** (`files_sync::restore_files`, one additive call at the end of `restore::restore_into`): lists `vault/attachments/`, then for every `.meta` that has a `.bin`: downloads the metadata (its envelope header must name this file), downloads the blob to a temp file, checks its structure **without any key** (header parses, belongs to this uuid, length is possible for its chunk count) and renames it into place, inserts the row with `indexed = 0`. A pair that does not fit together (missing half, wrong header, impossible length, metadata for another file) is skipped and counted (`RestoreCounts.files`, `.files_skipped`); a network or disk error stops the restore and the half-made vault folder is removed (as for items). A blob that is structurally possible but whose length does not match the sealed size is restored but stays hidden (damaged) after unlock. Names that are not `<uuid>.bin` / `<uuid>.meta` are ignored.

**Honest limit (by design, not built): files added, renamed or deleted on another device reach this one only through a restore, not through the unlock-time pull** (the pull covers items). A test pins this.

## 14. Batch import and plaintext export (Q4) [impl]

**Where.** `storage/vault_import.rs` (`impl Vault`, a `#[path]` child module of `vault.rs`, so it reaches the private index and sealing helpers without widening any visibility), `storage/session_import.rs` (the handle table and previews inside `Unlocked`), `storage/app_import.rs` (`impl AppCore`).

**One transaction.** `Vault::import_batch(items, mode)` validates each draft with the same `normalize()` as `create_item` / `update_item`, then seals **each item twice** (the item head and its immutable version snapshot: two keys, two nonces, exactly as a normal create or update) and writes **all of them in a single `Store::in_tx`**: `INSERT items` (new) or `UPDATE items ... WHERE version = prev` (appended; `dirty = 1`), `INSERT versions`, and, for every enabled backup target, the `Version` and `Item` outbox jobs (`dbops::enqueue_for_enabled`, the same helper the normal writes use). The in-memory index is updated and the backup worker woken (`wake_backup`, once) **only after the commit**. A database error rolls everything back and leaves the index untouched (test: a dropped `versions` table). An entry that fails validation is counted in `failed` with a fixed sentence labelled by its title and does not stop the batch. A 2-item import with one enabled target adds 2 `versions` rows and 4 outbox jobs (a `Version` and an `Item` job per item; test).

**Duplicate rule** (one place: `importer::entry::dup_key_parts`): same item type, same title (trimmed, case-insensitive) and, **for logins only**, the same username (case-insensitive) and the same primary host (`model::host_of` of the first URL, so `https://www.Example.com/login` and `example.com` match). Passwords are never compared. Matching is against the **non-trashed items that were in the vault before the import** (archived items count, trash does not), never against the file's own earlier records: a file with two identical records imports both, and importing the same file twice with `skip_duplicates` adds nothing the second time. The key holds titles and usernames, so it is `ZeroizeOnDrop` with a redacted `Debug`.

**Modes.** `skip_duplicates`: a match is left alone (`skipped`). `import_all`: every entry becomes a new item (`added`), matches included. `append_version`: a match gets a **new version** through the normal immutable-history path (`appended`, event `Edited`) **only when something changed**; an identical record counts as `skipped`. The merge (`vault_import::merge_drafts`): a non-empty imported field wins, an empty one keeps the stored value; custom fields merge by name (the file wins); tags are unioned; the stored title and folder stay (the file's folder only fills an empty one); `favorite` is OR-ed. The stored password is overwritten only by a non-empty imported password.

**Attachments.** Parsed (name + bytes, zeroized, each at most 50 MiB), counted in the preview, and handed to an `AttachmentSink` after the items are saved. The trait lives in `importer/mod.rs`: `available() -> bool` and `add(item_id, name, bytes) -> Result<()>`. **`import_commit` uses the encrypted drive** (`DriveSink` in `storage/app_import.rs`): each attachment goes through `Session::file_add_bytes(Some(item_id), name, bytes)` (the same three phases as `file_add`: prepare under the vault lock, encrypt without it, commit under it, nothing readable on disk), so it is attached to the saved item (`file_list(Some(item_id))`; `file_list(None)` is only the standalone Files section). `import_commit_with_sink(options, &dyn AttachmentSink)` stays for tests and other sinks; `NoSink` says it is unavailable and the commit adds the "N attachments were not imported (the encrypted drive is not available)." warning instead of failing. The sink is called after the transaction, with no lock held. A failing sink gives the `N attachments were not saved.` warning and never undoes the import.

**Memory hygiene.** Everything parsed is `ImportedEntry` (`Zeroize + ZeroizeOnDrop`, hand-written `Debug` that prints kinds and counts only). Files are read into pre-sized `Zeroizing` buffers (no reallocation, so no unwiped copy), parsed with no lock held, and dropped before the commit. Stored previews and handles die with the session on lock, account switch and quit.

**Tests:** `app_import_tests.rs` runs the commands against a real temp vault, with an in-memory backup target to count outbox jobs.

## 13. Device settings and the demo vault (Q5) [impl]

**`device.json`** (data folder, owner-only 0600, atomic replace; read at start, never an error: a missing, unreadable, oversized (over 4 KiB) or damaged file is the defaults). Plaintext JSON:

```json
{"timeZone": "Asia/Dhaka", "setupDone": true, "timeCheck": true, "theme": "system", "clipboardClear": 30, "demoProfile": "<uuid>"}
```

* `timeZone`: the IANA name from the first page, only used to *show* UTC times and to read times the person types. Absent until chosen.
* `setupDone`: the first-run welcome was seen.
* `timeCheck`: the online time check (on by default).
* `theme`: `system` (follow the device, the default), `light` or `dark`. Any other value reads as `system`.
* `clipboardClear`: seconds a copied secret stays on the clipboard, 15, 30 (default) or 60. Added 2026-11-25; before that it was web view storage and is not carried over.
* `demoProfile`: the id of the demo vault while one exists. It stays inside the core; `DeviceInfo` (what the screen reads) has no such field.

It leaks a place name, a colour preference, a few switches and a random id; no name, item or key. **It survives the erase on purpose** (D30): the wipe only knows its fixed list of files, and `forget_demo_if_any` clears just the demo id. A reset therefore opens straight into a fresh demo vault with no zone page.

**The demo vault** is an ordinary vault in `vaults/<id>/vault.db` named "Hexis123" with the master password "Hexis123" (public on purpose: it holds nothing real). `AppCore::demo_spawn` runs only when the registry is empty: `profile_create`, then `Session::seed_items` with the 15 to 25 items from `storage/demo.rs` (one invented person, Hexis Alden: logins for Google, Microsoft, GitHub, Amazon and others, with random passwords and one-time-code secrets made at spawn time; cards that are the published test numbers; an identity, a secure note, an SSH key; created at back-dated times so the list shows groups from "Last 7 days" to "Older"), then the id is written to `device.json`. A failure after the vault exists removes it again. `demo_open` selects it and unlocks it with the public password, and only for the id in `device.json`. Deleting it (Settings, Accounts, with its password and `CONFIRM`) forgets the id; the app then makes a new one if no vault is left.

**Time.** Every time written to a vault, a backup object, a file's metadata or `profiles.json` is UTC milliseconds since 1970 (`crypto::now_millis`, which reads `TrustedClock::now_ms`: checked UTC when there is one, the device clock otherwise). Nothing stored depends on the zone.

### Web view data and "remove from this device" (2026-11-25)

**The web view keeps nothing** (D32). The window is incognito, and the screen stores nothing itself. Builds up to 0.2.0 let WebKitGTK write into the data folder; `storage/webview_data.rs` lists the names it used (`localstorage`, `indexeddb`, `databases`, `storage`, `CacheStorage`, `WebKitCache`, `mediakeys`, `deviceidhashsalts`, `itp`, `ResourceLoadStatistics`, `serviceworkers`, `applicationcache`, `offlineapplicationcache`, `cookies`, `hsts-storage.sqlite` and its `-journal`, `-wal`, `-shm`). `remove(dir)` deletes exactly those, directly inside the data folder, never following a link, and runs at every start (non-mobile) and inside the wipe (`WipeReport.web_view_removed`). They are not secret, so they are unlinked, not overwritten. The wipe's scan no longer counts these names as "unknown files", so a folder that holds only them is removed.

**`app_prepare_uninstall`** (D34) runs the wipe and then `forget_device_files`: `device.json` goes, and the data folder itself when it is empty. A folder that still holds a file Hexis does not know is left as it is, with that file. Nothing is created again: the demo vault is made on the next start of an *installed* app, which is what a fresh install does anyway.
