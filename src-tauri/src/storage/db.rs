//! SQLite persistence (`ai-docs/STORAGE_AND_SYNC.md` §5).
//!
//! The database holds only opaque envelopes plus sync bookkeeping, so it needs no encryption of
//! its own. Every state change (snapshot + latest state) commits in one transaction, together
//! with the replication work it causes (the `outbox` rows of every enabled backup target).

use super::backup::dbops::{self, JobKind};
use super::error::{Result, StorageError};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `PRAGMA application_id` of every Hexis vault file ("PMV1").
pub const APPLICATION_ID: i64 = 0x504D_5631;
/// Schema version this build writes (2 = backup targets, outbox, remote object bookkeeping;
/// 3 = copies of an item's state that arrived from another device,
/// 4 = the encrypted drive's `files` table, `storage/drive.rs`).
pub const SCHEMA_VERSION: i64 = 4;

const KEY_BUNDLE: &str = "bundle";
const KEY_DEVICE_ID: &str = "device_id";
const KEY_CREATED_AT: &str = "created_at";

/// Schema v1. Only the small `meta` / `tombstones` tables are `WITHOUT ROWID`; the blob-heavy
/// tables are ordinary rowid tables. The `dirty` / `uploaded` / `remote_etag` columns exist from
/// the start but replication does not use them (per-target state lives in schema v2's tables).
const MIGRATION_V1: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE items (
    id           TEXT PRIMARY KEY,
    version      INTEGER NOT NULL CHECK (version >= 1),
    ts           INTEGER NOT NULL,
    blob         BLOB NOT NULL,
    remote_etag  TEXT,
    base_version INTEGER NOT NULL DEFAULT 0,
    dirty        INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE versions (
    item_id  TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    version  INTEGER NOT NULL CHECK (version >= 1),
    ts       INTEGER NOT NULL,
    blob     BLOB NOT NULL,
    uploaded INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (item_id, version, ts)
);

CREATE TABLE attachments (
    id       TEXT PRIMARY KEY,
    item_id  TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    size     INTEGER NOT NULL,
    blob     BLOB,
    uploaded INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX attachments_by_item ON attachments(item_id);

CREATE TABLE conflicts (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id  TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    ts       INTEGER NOT NULL,
    blob     BLOB NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX conflicts_by_item ON conflicts(item_id);

CREATE TABLE tombstones (
    id       TEXT PRIMARY KEY,
    ts       INTEGER NOT NULL,
    uploaded INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;
";

/// Schema v2 (Phase 4a): backup targets and their replication queue.
///
/// * `targets` keeps only flags and health counters in the clear. Everything that identifies a
///   bucket (name, endpoint, region, bucket, prefix, access key id, **secret key**) is inside
///   `config`, sealed with the vault's `meta` sub-key, so none of it is readable in the file.
/// * `outbox` is the queue: one row per (target, object) that still has to be pushed. Content is
///   read from `items` / `versions` / `tombstones` when the job runs, so repeated edits of one
///   item collapse into one job (`gen` counts the bumps, so a job that is overtaken while it is
///   in flight is not lost). `rank` orders a drain: bundle, tombstones, snapshots, latest states.
/// * `remote_objects` remembers the ETag (opaque) and version this device last wrote or saw for
///   a key on a target: the basis of the single-writer rule.
/// * `tombstones.blob` is the sealed proof of deletion that is uploaded (`NULL` for tombstones
///   written by schema v1; the vault fills those in when it is next unlocked).
const MIGRATION_V2: &str = "
CREATE TABLE targets (
    id                 TEXT PRIMARY KEY,
    position           INTEGER NOT NULL,
    is_master          INTEGER NOT NULL DEFAULT 0 CHECK (is_master IN (0, 1)),
    enabled            INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    share_in_master    INTEGER NOT NULL DEFAULT 1 CHECK (share_in_master IN (0, 1)),
    config             BLOB NOT NULL,
    seeded             INTEGER NOT NULL DEFAULT 0 CHECK (seeded IN (0, 1)),
    conditional        TEXT NOT NULL DEFAULT 'unknown' CHECK (conditional IN ('strong', 'weak', 'unknown')),
    last_success_at    INTEGER,
    last_checked_at    INTEGER,
    last_error         TEXT,
    consecutive_errors INTEGER NOT NULL DEFAULT 0,
    hard_error         INTEGER NOT NULL DEFAULT 0 CHECK (hard_error IN (0, 1)),
    created_at         INTEGER NOT NULL
);
CREATE UNIQUE INDEX targets_one_master ON targets (is_master) WHERE is_master = 1;

CREATE TABLE outbox (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id    TEXT NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('bundle', 'tombstone', 'version', 'item', 'purge', 'targets')),
    ref          TEXT NOT NULL,
    version      INTEGER NOT NULL DEFAULT 0,
    ts           INTEGER NOT NULL DEFAULT 0,
    rank         INTEGER NOT NULL,
    gen          INTEGER NOT NULL DEFAULT 0,
    enqueued_at  INTEGER NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    next_try_at  INTEGER NOT NULL DEFAULT 0,
    blocked      INTEGER NOT NULL DEFAULT 0 CHECK (blocked IN (0, 1)),
    last_error   TEXT
);
CREATE UNIQUE INDEX outbox_unique ON outbox (target_id, kind, ref, version, ts);
CREATE INDEX outbox_order ON outbox (target_id, rank, seq);

CREATE TABLE remote_objects (
    target_id TEXT NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    key       TEXT NOT NULL,
    etag      TEXT,
    version   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (target_id, key)
) WITHOUT ROWID;

ALTER TABLE tombstones ADD COLUMN blob BLOB;
";

/// Schema v3 (event-driven sync, `FEATURE_QUEUE.md` Q0): a version snapshot can now be a copy of
/// another device's latest state that arrived while this device kept its own (a "diverged"
/// merge).
///
/// * `origin` is `'local'` for every snapshot that belongs to this device's own history (what it
///   wrote, or what it took over by a fast-forward) and `'remote'` for the extra copies.
/// * `remote_version` is the number the other device gave that state (the copy itself is
///   resealed under a number of this device's own, above every number in use, so that a version
///   number stays unambiguous inside this vault).
/// * `reviewed` is `0` until the person has looked at a `'remote'` copy (this is the "changed
///   on another device" flag); it is `1` for everything else.
///
/// The three `ALTER TABLE`s run inside the migration's transaction, so a failure leaves the file
/// at the version it had. Existing rows become `origin = 'local'`, `reviewed = 1`.
const MIGRATION_V3: &str = "
ALTER TABLE versions ADD COLUMN origin TEXT NOT NULL DEFAULT 'local' CHECK (origin IN ('local', 'remote'));
ALTER TABLE versions ADD COLUMN remote_version INTEGER;
ALTER TABLE versions ADD COLUMN reviewed INTEGER NOT NULL DEFAULT 1 CHECK (reviewed IN (0, 1));
";

/// One row of `items`: the latest sealed state.
#[derive(Debug, Clone)]
pub struct ItemRow {
    pub id: String,
    pub version: u64,
    pub ts: u64,
    pub blob: Vec<u8>,
}

/// One row of `versions`: an immutable sealed snapshot.
#[derive(Debug, Clone)]
pub struct VersionRow {
    pub version: u64,
    pub ts: u64,
    pub blob: Vec<u8>,
    /// A copy of another device's latest state (schema v3); `false` for this device's history.
    pub other_device: bool,
    /// The number the other device gave this state (`Some` only when `other_device`).
    pub remote_version: Option<u64>,
    /// `false` while the person has not looked at an other-device copy yet.
    pub reviewed: bool,
}

pub struct Store {
    /// Visible to `storage::backup`, which owns the SQL of targets, outbox and remote objects.
    pub(super) conn: Connection,
    path: PathBuf,
}

fn to_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

fn from_i64(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

impl Store {
    /// Open (creating if needed) and migrate. Refuses files that are not Hexis vaults and
    /// vaults written by a newer schema.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        // Defence in depth for a file that is opened from disk.
        conn.pragma_update(None, "trusted_schema", "OFF")?;
        conn.pragma_update(None, "cell_size_check", "ON")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Deleted pages are overwritten with zeros, not just unlinked.
        conn.pragma_update(None, "secure_delete", "ON")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(StorageError::Storage(
                "The vault database could not enable write-ahead logging.".into(),
            ));
        }
        let mut store = Self {
            conn,
            path: path.to_path_buf(),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Apply pending migrations. Running it again is a no-op.
    fn migrate(&mut self) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let app_id: i64 = tx.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StorageError::Storage(
                "This vault was created by a newer version of this app. Update the app to open it."
                    .into(),
            ));
        }
        if version == 0 {
            // A fresh file, or somebody else's database: never adopt a database with tables.
            let tables: i64 =
                tx.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
            if tables > 0 || app_id != 0 {
                return Err(StorageError::Storage(
                    "That file is not a vault this app can open.".into(),
                ));
            }
            tx.execute_batch(MIGRATION_V1)?;
            tx.execute_batch(MIGRATION_V2)?;
            tx.execute_batch(MIGRATION_V3)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if app_id != APPLICATION_ID {
            return Err(StorageError::Storage(
                "That file is not a vault this app can open.".into(),
            ));
        } else {
            // Older vaults are brought forward step by step, all in this one transaction: a
            // failure anywhere rolls the whole thing back and leaves the file as it was.
            if version == 1 {
                // From before Phase 4a: add the backup tables in place. Nothing in v1 is touched
                // except `tombstones` gaining a nullable column.
                tx.execute_batch(MIGRATION_V2)?;
            }
            if version < 3 {
                tx.execute_batch(MIGRATION_V3)?;
            }
            if version < SCHEMA_VERSION {
                tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
        }
        super::drive::migrate_files_table(&tx, version)?;
        tx.commit()?;
        Ok(())
    }

    /// Where this database lives (the replication worker opens its own connection to it).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run `f` in one IMMEDIATE transaction; it commits only if `f` succeeds.
    pub(super) fn in_tx<T>(&mut self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    // ---------------------------------------------------------------- meta

    pub(super) fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// The public key bundle JSON, if a vault was created in this file.
    pub fn bundle_json(&self) -> Result<Option<String>> {
        self.meta(KEY_BUNDLE)
    }

    pub fn device_id(&self) -> Result<Option<String>> {
        self.meta(KEY_DEVICE_ID)
    }

    pub fn created_at(&self) -> Result<Option<u64>> {
        self.meta(KEY_CREATED_AT)?
            .map(|s| {
                s.parse()
                    .map_err(|_| StorageError::Storage("Bad vault metadata.".into()))
            })
            .transpose()
    }

    /// SQLite's structural self-check (`PRAGMA quick_check`): `false` for a truncated or damaged
    /// file. Used before an imported file is adopted.
    pub fn is_structurally_sound(&self) -> Result<bool> {
        let verdict: String = self
            .conn
            .query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        Ok(verdict == "ok")
    }

    /// Give this copy of the vault a fresh device id (an imported vault is a new replica).
    pub fn set_device_id(&mut self, id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            params![id, KEY_DEVICE_ID],
        )?;
        if changed != 1 {
            return Err(StorageError::Storage("Bad vault metadata.".into()));
        }
        Ok(())
    }

    /// Write a consistent, compacted copy of the whole database (committed WAL content included)
    /// to `dest`, which must not exist or be an empty file. Everything in it is already
    /// encrypted, so no further protection is needed beyond the file's permissions.
    pub fn export_to(&self, dest: &Path) -> Result<()> {
        let dest_text = dest
            .to_str()
            .ok_or_else(|| StorageError::invalid("That file name cannot be used."))?;
        self.conn.execute("VACUUM INTO ?1", [dest_text])?;
        let copy = Connection::open(dest)?;
        strip_device_only_targets(&copy)?;
        // Fold the copy's own journal away so it is a single self-contained file.
        let _ = copy.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        Ok(())
    }

    /// Store the key bundle of a brand-new vault. `VaultExists` if one is already there.
    pub fn create_vault(&mut self, bundle_json: &str, device_id: &str, now: u64) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row("SELECT value FROM meta WHERE key = ?1", [KEY_BUNDLE], |r| {
                r.get(0)
            })
            .optional()?;
        if existing.is_some() {
            return Err(StorageError::VaultExists);
        }
        let mut put = tx.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
        put.execute(params![KEY_BUNDLE, bundle_json])?;
        put.execute(params![KEY_DEVICE_ID, device_id])?;
        put.execute(params![KEY_CREATED_AT, now.to_string()])?;
        drop(put);
        tx.commit()?;
        Ok(())
    }

    /// Replace the bundle after a password change, only if it is still the one the caller read.
    pub fn replace_bundle(&mut self, expected_old: &str, new_json: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2 AND value = ?3",
            params![new_json, KEY_BUNDLE, expected_old],
        )?;
        if changed != 1 {
            return Err(StorageError::VersionConflict);
        }
        // A new password changes the bundle: every enabled target has to learn the new one.
        dbops::enqueue_for_enabled(&tx, JobKind::Bundle, "", 0, 0, crate::crypto::now_millis())?;
        tx.commit()?;
        Ok(())
    }

    // ---------------------------------------------------------------- items and versions

    pub fn item_row(&self, id: &str) -> Result<Option<ItemRow>> {
        self.conn
            .query_row(
                "SELECT id, version, ts, blob FROM items WHERE id = ?1",
                [id],
                row_to_item,
            )
            .optional()?
            .transpose()
    }

    pub fn all_item_rows(&self) -> Result<Vec<ItemRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, version, ts, blob FROM items ORDER BY id")?;
        let rows = stmt.query_map([], row_to_item)?;
        rows.map(|r| r?).collect()
    }

    /// All snapshots of an item, newest version first (ties: newest timestamp first).
    pub fn version_rows(&self, id: &str) -> Result<Vec<VersionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT version, ts, blob, origin, remote_version, reviewed FROM versions
             WHERE item_id = ?1 ORDER BY version DESC, ts DESC",
        )?;
        let rows = stmt.query_map([id], row_to_version)?;
        rows.map(|r| r?).collect()
    }

    /// The snapshot of `version` (if two devices wrote the same number, the newest timestamp).
    pub fn version_row(&self, id: &str, version: u64) -> Result<Option<VersionRow>> {
        self.conn
            .query_row(
                "SELECT version, ts, blob, origin, remote_version, reviewed FROM versions
                 WHERE item_id = ?1 AND version = ?2 ORDER BY ts DESC LIMIT 1",
                params![id, to_i64(version)?],
                row_to_version,
            )
            .optional()?
            .transpose()
    }

    pub fn count_versions(&self, id: &str) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM versions WHERE item_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        from_i64(n)
    }

    pub fn is_tombstoned(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM tombstones WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    }

    /// Atomically write the snapshot and the new latest state.
    ///
    /// `prev_version = None` creates the item (fails if the id exists or was permanently
    /// deleted). `Some(n)` updates it and requires the stored version to still be `n`.
    pub fn commit_state(
        &mut self,
        id: &str,
        prev_version: Option<u64>,
        version: u64,
        ts: u64,
        item_blob: &[u8],
        snapshot_blob: &[u8],
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: Option<i64> = tx
            .query_row("SELECT version FROM items WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        match (prev_version, stored) {
            (None, None) => {
                let dead: Option<i64> = tx
                    .query_row("SELECT 1 FROM tombstones WHERE id = ?1", [id], |r| r.get(0))
                    .optional()?;
                if dead.is_some() {
                    return Err(StorageError::invalid(
                        "That item was permanently deleted and cannot be recreated.",
                    ));
                }
                tx.execute(
                    "INSERT INTO items (id, version, ts, blob, dirty) VALUES (?1, ?2, ?3, ?4, 1)",
                    params![id, to_i64(version)?, to_i64(ts)?, item_blob],
                )?;
            }
            (None, Some(_)) => {
                return Err(StorageError::invalid(
                    "An item with that id already exists.",
                ))
            }
            (Some(_), None) => return Err(StorageError::NotFound),
            (Some(prev), Some(stored)) => {
                if from_i64(stored)? != prev {
                    return Err(StorageError::VersionConflict);
                }
                tx.execute(
                    "UPDATE items SET version = ?2, ts = ?3, blob = ?4, dirty = 1 WHERE id = ?1",
                    params![id, to_i64(version)?, to_i64(ts)?, item_blob],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO versions (item_id, version, ts, blob) VALUES (?1, ?2, ?3, ?4)",
            params![id, to_i64(version)?, to_i64(ts)?, snapshot_blob],
        )?;
        // The replication work this change causes commits (or rolls back) with the change.
        let now = crate::crypto::now_millis();
        dbops::enqueue_for_enabled(&tx, JobKind::Version, id, version, ts, now)?;
        dbops::enqueue_for_enabled(&tx, JobKind::Item, id, 0, 0, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Permanently erase an item: tombstone first, then every row that belongs to it, all in
    /// one transaction. Afterwards the WAL is folded back and truncated so no old page copy
    /// of the erased rows lingers in the `-wal` file.
    ///
    /// `tombstone_blob` is the sealed proof of deletion that every backup target receives; the
    /// pending uploads of the erased item are dropped and replaced by "upload the tombstone, then
    /// delete the item's objects" jobs.
    pub fn purge(&mut self, id: &str, now: u64, tombstone_blob: &[u8]) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO tombstones (id, ts, blob) VALUES (?1, ?2, ?3)",
            params![id, to_i64(now)?, tombstone_blob],
        )?;
        // Explicit, although ON DELETE CASCADE would also do it.
        tx.execute("DELETE FROM conflicts WHERE item_id = ?1", [id])?;
        tx.execute("DELETE FROM attachments WHERE item_id = ?1", [id])?;
        tx.execute("DELETE FROM versions WHERE item_id = ?1", [id])?;
        let gone = tx.execute("DELETE FROM items WHERE id = ?1", [id])?;
        if gone != 1 {
            return Err(StorageError::NotFound);
        }
        super::drive::purge_files_of_item(&tx, id, now)?;
        dbops::forget_pending(&tx, id)?;
        dbops::enqueue_for_enabled(&tx, JobKind::Tombstone, id, 0, 0, now)?;
        dbops::enqueue_for_enabled(&tx, JobKind::Purge, id, 0, 0, now)?;
        tx.commit()?;
        // Best effort: a busy reader only delays the truncation.
        let _ = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        Ok(())
    }

    // ---------------------------------------------------------------- test support

    #[cfg(test)]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    #[cfg(test)]
    pub fn count(&self, table: &str, id_column: &str, id: &str) -> i64 {
        self.conn
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE {id_column} = ?1"),
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// Backup-target connection details that the person did not mark "Save in master backup" must
/// not travel inside an exported file. Run on the exported copy (never on the live database):
///
/// * targets without `share_in_master` are deleted, with secure-delete on and a `VACUUM`
///   afterwards, so their sealed bytes do not linger in free pages of the copy;
/// * the queue, the remote bookkeeping and health history are device state: they are cleared,
///   and the remaining targets are marked "not seeded" so the device that imports the file does
///   a full first upload to them;
/// * the cached shared-targets snapshot is dropped (it is rebuilt from the remaining targets).
fn strip_device_only_targets(copy: &Connection) -> Result<()> {
    copy.pragma_update(None, "secure_delete", "ON")?;
    copy.execute_batch(
        "DELETE FROM outbox;
         DELETE FROM remote_objects;
         DELETE FROM files;
         DELETE FROM targets WHERE share_in_master = 0;
         UPDATE targets SET seeded = 0, last_success_at = NULL, last_checked_at = NULL,
                            last_error = NULL, consecutive_errors = 0, hard_error = 0;
         DELETE FROM meta WHERE key IN ('shared_targets', 'pending_targets');",
    )?;
    copy.execute_batch("VACUUM")?;
    Ok(())
}

fn row_to_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<ItemRow>> {
    let id: String = r.get(0)?;
    let version: i64 = r.get(1)?;
    let ts: i64 = r.get(2)?;
    let blob: Vec<u8> = r.get(3)?;
    Ok((|| {
        Ok(ItemRow {
            id,
            version: from_i64(version)?,
            ts: from_i64(ts)?,
            blob,
        })
    })())
}

pub(super) fn row_to_version(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<VersionRow>> {
    let version: i64 = r.get(0)?;
    let ts: i64 = r.get(1)?;
    let blob: Vec<u8> = r.get(2)?;
    let origin: String = r.get(3)?;
    let remote_version: Option<i64> = r.get(4)?;
    let reviewed: i64 = r.get(5)?;
    Ok((|| {
        Ok(VersionRow {
            version: from_i64(version)?,
            ts: from_i64(ts)?,
            blob,
            other_device: origin == "remote",
            remote_version: remote_version.map(from_i64).transpose()?,
            reviewed: reviewed != 0,
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(dir: &tempfile::TempDir) -> Store {
        Store::open(&dir.path().join("vault.db")).unwrap()
    }

    #[test]
    fn fresh_file_gets_the_current_schema_and_hardened_pragmas() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        let c = s.conn();
        let one = |p: &str| -> i64 { c.pragma_query_value(None, p, |r| r.get(0)).unwrap() };
        assert_eq!(one("application_id"), APPLICATION_ID);
        assert_eq!(one("foreign_keys"), 1);
        assert_eq!(one("secure_delete"), 1);
        assert_eq!(one("synchronous"), 2, "FULL");
        let mode: String = c
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        for t in [
            "meta",
            "items",
            "versions",
            "attachments",
            "conflicts",
            "tombstones",
            "targets",
            "outbox",
            "remote_objects",
            "files",
        ] {
            let n: i64 = c
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing table {t}");
        }
    }

    #[test]
    fn migration_is_idempotent_and_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut s = open(&dir);
            s.create_vault("{\"x\":1}", "dev", 5).unwrap();
        }
        for _ in 0..3 {
            let s = open(&dir);
            assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
            assert_eq!(s.bundle_json().unwrap().as_deref(), Some("{\"x\":1}"));
            assert_eq!(s.device_id().unwrap().as_deref(), Some("dev"));
            assert_eq!(s.created_at().unwrap(), Some(5));
        }
    }

    fn all(s: &Store, table: &str) -> i64 {
        s.conn()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// A vault exactly as a Phase 2 build left it (schema v1: no backup tables, no tombstone
    /// blob column), with data in every v1 table.
    fn write_v1_fixture(path: &Path) {
        let c = Connection::open(path).unwrap();
        c.pragma_update(None, "journal_mode", "WAL").unwrap();
        c.execute_batch(MIGRATION_V1).unwrap();
        c.pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        c.execute_batch(
            "INSERT INTO meta (key, value) VALUES ('bundle', '{\"v1\":true}'), ('device_id', 'dev-1'), ('created_at', '42');
             INSERT INTO items (id, version, ts, blob) VALUES ('item-a', 2, 20, x'0102');
             INSERT INTO versions (item_id, version, ts, blob) VALUES ('item-a', 1, 10, x'0A'), ('item-a', 2, 20, x'0B');
             INSERT INTO tombstones (id, ts) VALUES ('item-gone', 99);",
        )
        .unwrap();
    }

    #[test]
    fn a_v1_vault_migrates_in_place_and_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        write_v1_fixture(&path);
        {
            let s = Store::open(&path).unwrap();
            assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
            // v1 data is untouched
            assert_eq!(s.bundle_json().unwrap().as_deref(), Some("{\"v1\":true}"));
            assert_eq!(s.device_id().unwrap().as_deref(), Some("dev-1"));
            assert_eq!(s.created_at().unwrap(), Some(42));
            let item = s.item_row("item-a").unwrap().unwrap();
            assert_eq!((item.version, item.ts, item.blob), (2, 20, vec![1, 2]));
            assert_eq!(s.version_rows("item-a").unwrap().len(), 2);
            assert!(s.is_tombstoned("item-gone").unwrap());
            // v2 pieces exist and are empty; the old tombstone has no sealed blob yet
            for t in ["targets", "outbox", "remote_objects"] {
                assert_eq!(all(&s, t), 0, "{t}");
            }
            assert_eq!(
                s.tombstones_without_blob().unwrap(),
                vec![("item-gone".to_string(), 99)]
            );
            // a second migration is a no-op
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(s.version_rows("item-a").unwrap().len(), 2);
    }

    /// A vault exactly as the Phase 4a build left it (schema v2), with data in the v2 tables.
    fn write_v2_fixture(path: &Path) {
        write_v1_fixture(path);
        let c = Connection::open(path).unwrap();
        c.execute_batch(MIGRATION_V2).unwrap();
        c.pragma_update(None, "user_version", 2).unwrap();
        c.execute_batch(
            "INSERT INTO targets (id, position, is_master, config, created_at) VALUES ('t1', 1, 1, x'AA', 5);
             INSERT INTO outbox (target_id, kind, ref, rank, enqueued_at) VALUES ('t1', 'item', 'item-a', 3, 6);
             INSERT INTO remote_objects (target_id, key, etag, version) VALUES ('t1', 'k', 'e', 2);",
        )
        .unwrap();
    }

    /// A vault exactly as the build before the encrypted drive left it (schema v2), with a
    /// target and a queue holding jobs of several kinds and some failure history.
    fn write_v2_queue_fixture(path: &Path) {
        let c = Connection::open(path).unwrap();
        c.pragma_update(None, "journal_mode", "WAL").unwrap();
        c.execute_batch(MIGRATION_V1).unwrap();
        c.execute_batch(MIGRATION_V2).unwrap();
        c.pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        c.pragma_update(None, "user_version", 2).unwrap();
        c.execute_batch(
            "INSERT INTO meta (key, value) VALUES ('bundle', '{\"v2\":true}'), ('device_id', 'dev-2'), ('created_at', '43');
             INSERT INTO items (id, version, ts, blob) VALUES ('item-a', 3, 30, x'0102');
             INSERT INTO targets (id, position, is_master, config, created_at) VALUES ('t1', 1, 1, x'AA', 1);
             INSERT INTO outbox (seq, target_id, kind, ref, version, ts, rank, gen, enqueued_at, attempts, next_try_at, blocked, last_error)
                VALUES (5, 't1', 'bundle', '', 0, 0, 0, 0, 100, 0, 0, 0, NULL),
                       (9, 't1', 'item', 'item-a', 0, 0, 3, 4, 101, 2, 0, 1, 'newer remote'),
                       (11, 't1', 'version', 'item-a', 3, 30, 2, 0, 102, 0, 0, 0, NULL);
             INSERT INTO remote_objects (target_id, key, etag, version) VALUES ('t1', 'vault/items/item-a.enc', 'e1', 3);",
        )
        .unwrap();
    }

    #[test]
    fn v1_and_v2_vaults_reach_the_current_schema_with_every_row_kept_and_every_snapshot_local_and_reviewed(
    ) {
        for from in [1, 2] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("vault.db");
            if from == 1 {
                write_v1_fixture(&path);
            } else {
                write_v2_fixture(&path);
            }
            for round in 0..2 {
                let s = Store::open(&path).unwrap();
                assert_eq!(
                    s.schema_version().unwrap(),
                    SCHEMA_VERSION,
                    "from v{from}, round {round}"
                );
                let rows = s.version_rows("item-a").unwrap();
                assert_eq!(rows.len(), 2);
                for r in &rows {
                    assert!(!r.other_device && r.reviewed && r.remote_version.is_none());
                }
                assert_eq!(s.item_row("item-a").unwrap().unwrap().blob, vec![1, 2]);
                assert!(s.is_tombstoned("item-gone").unwrap());
                if from == 2 {
                    assert_eq!(all(&s, "targets"), 1);
                    assert_eq!(all(&s, "outbox"), 1);
                    assert_eq!(
                        s.remote_object("t1", "k").unwrap(),
                        Some((Some("e".into()), 2))
                    );
                }
            }
        }
    }

    #[test]
    fn a_failed_migration_leaves_the_file_at_its_old_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        write_v2_fixture(&path);
        // Make the third ALTER fail halfway: a column of that name already exists.
        Connection::open(&path)
            .unwrap()
            .execute_batch("ALTER TABLE versions ADD COLUMN reviewed INTEGER NOT NULL DEFAULT 1;")
            .unwrap();
        assert!(Store::open(&path).is_err());
        let c = Connection::open(&path).unwrap();
        let version: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2, "the version number did not move");
        let has = |col: &str| -> bool {
            c.query_row(
                "SELECT count(*) FROM pragma_table_info('versions') WHERE name = ?1",
                [col],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        };
        assert!(
            !has("origin") && !has("remote_version"),
            "the first two ALTERs were rolled back"
        );
    }

    #[test]
    fn a_v2_vault_gets_the_files_table_and_keeps_its_queue_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        write_v2_queue_fixture(&path);
        for round in 0..2 {
            // the second round is a plain re-open: the migration must be a no-op
            let s = Store::open(&path).unwrap();
            assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION, "round {round}");
            assert_eq!(s.bundle_json().unwrap().as_deref(), Some("{\"v2\":true}"));
            assert_eq!(all(&s, "files"), 0);
            let jobs = s.outbox_batch("t1", (-1, 0), 10).unwrap();
            let seen: Vec<_> = jobs
                .iter()
                .map(|j| {
                    (
                        j.seq,
                        j.kind,
                        j.target_ref.clone(),
                        j.version,
                        j.gen,
                        j.attempts,
                        j.blocked,
                    )
                })
                .collect();
            assert_eq!(
                seen,
                vec![
                    (5, JobKind::Bundle, String::new(), 0, 0, 0, false),
                    (11, JobKind::Version, "item-a".into(), 3, 0, 0, false),
                    (9, JobKind::Item, "item-a".into(), 0, 4, 2, true),
                ],
                "rows, sequence numbers and failure history survive the queue rebuild"
            );
            assert_eq!(jobs[2].last_error.as_deref(), Some("newer remote"));
            assert_eq!(all(&s, "remote_objects"), 1);
            assert_eq!(all(&s, "targets"), 1);
        }
        // the rebuilt queue still refuses duplicates and accepts the drive's job kinds
        let mut s = Store::open(&path).unwrap();
        s.in_tx(|c| {
            dbops::enqueue_for_target(c, "t1", JobKind::Item, "item-a", 0, 0, 7)?;
            dbops::enqueue_for_target(c, "t1", JobKind::FileBlob, "f1", 0, 0, 7)?;
            dbops::enqueue_for_target(c, "t1", JobKind::FileMeta, "f1", 0, 0, 7)?;
            dbops::enqueue_for_target(c, "t1", JobKind::FilePurge, "f2", 0, 0, 7)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            all(&s, "outbox"),
            6,
            "the repeated item job was not duplicated"
        );
        let next: i64 = s
            .conn()
            .query_row("SELECT max(seq) FROM outbox", [], |r| r.get(0))
            .unwrap();
        assert!(next > 11, "sequence numbers continue after the old ones");
        drop(s);
        // a kind the schema does not know is still refused by the CHECK
        let s = Store::open(&path).unwrap();
        let bad = s.conn().execute(
            "INSERT INTO outbox (target_id, kind, ref, rank, enqueued_at) VALUES ('t1', 'bogus', 'x', 0, 0)",
            [],
        );
        assert!(bad.is_err());
    }

    #[test]
    fn a_v1_vault_reaches_the_drive_schema_in_one_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        write_v1_fixture(&path);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(all(&s, "files"), 0);
        assert_eq!(all(&s, "items"), 1);
    }

    #[test]
    fn a_v1_file_with_another_application_id_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        write_v1_fixture(&path);
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "application_id", 7)
            .unwrap();
        assert_eq!(Store::open(&path).err().unwrap().code(), "storage");
    }

    #[test]
    fn exported_copies_keep_only_targets_marked_shared_and_no_device_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(&dir);
        s.create_vault("{}", "dev", 1).unwrap();
        let now = 5;
        s.in_tx(|c| {
            for (id, share, master) in [("t-shared", 1, 1), ("t-private", 0, 0)] {
                c.execute(
                    "INSERT INTO targets (id, position, is_master, share_in_master, config, seeded,
                        last_success_at, last_error, consecutive_errors, hard_error, created_at)
                     VALUES (?1, 1, ?2, ?3, x'AA', 1, 7, 'boom', 3, 1, 1)",
                    params![id, master, share],
                )?;
                dbops::enqueue_for_target(c, id, JobKind::Bundle, "", 0, 0, now)?;
                c.execute(
                    "INSERT INTO remote_objects (target_id, key, etag, version) VALUES (?1, 'k', 'e', 1)",
                    [id],
                )?;
            }
            dbops::set_meta(c, "shared_targets", "c2hhcmVk")?;
            dbops::set_meta(c, "pending_targets", "cGVuZGluZw==")?;
            Ok(())
        })
        .unwrap();

        let out = dir.path().join("copy.hexis");
        s.export_to(&out).unwrap();
        // the live database is untouched
        assert_eq!(all(&s, "targets"), 2);
        assert_eq!(all(&s, "outbox"), 2);

        let copy = Store::open(&out).unwrap();
        assert_eq!(copy.count("targets", "id", "t-shared"), 1);
        assert_eq!(
            copy.count("targets", "id", "t-private"),
            0,
            "device-only target removed"
        );
        assert_eq!(all(&copy, "outbox"), 0);
        assert_eq!(all(&copy, "remote_objects"), 0);
        assert!(copy.meta("shared_targets").unwrap().is_none());
        assert!(copy.meta("pending_targets").unwrap().is_none());
        let row = copy.target_row("t-shared").unwrap().unwrap();
        assert!(
            !row.seeded,
            "the importing device starts with a first upload"
        );
        assert_eq!(
            (
                row.last_success_at,
                row.last_error,
                row.consecutive_errors,
                row.hard_error
            ),
            (None, None, 0, false)
        );
        assert_eq!(
            row.config,
            vec![0xAA],
            "the sealed settings travel unchanged"
        );
        // the vault itself came along
        assert_eq!(copy.device_id().unwrap().as_deref(), Some("dev"));
    }

    #[test]
    fn refuses_foreign_and_newer_databases() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = dir.path().join("foreign.db");
        Connection::open(&foreign)
            .unwrap()
            .execute_batch("CREATE TABLE notes (t TEXT);")
            .unwrap();
        let e = Store::open(&foreign).err().expect("must refuse");
        assert_eq!(e.code(), "storage");

        let newer = dir.path().join("newer.db");
        {
            let s = Store::open(&newer).unwrap();
            s.conn().pragma_update(None, "user_version", 99).unwrap();
        }
        let e = Store::open(&newer).err().expect("must refuse");
        assert!(e.to_string().contains("newer version"));

        let garbage = dir.path().join("garbage.db");
        std::fs::write(&garbage, vec![0x42u8; 4096]).unwrap();
        assert_eq!(
            Store::open(&garbage).err().expect("must refuse").code(),
            "storage"
        );
    }

    #[test]
    fn create_vault_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(&dir);
        assert!(s.bundle_json().unwrap().is_none());
        s.create_vault("{}", "d", 1).unwrap();
        assert_eq!(
            s.create_vault("{}", "d2", 2).unwrap_err(),
            StorageError::VaultExists
        );
        s.replace_bundle("{}", "{\"new\":true}").unwrap();
        assert_eq!(
            s.replace_bundle("{}", "{\"again\":true}").unwrap_err(),
            StorageError::VersionConflict
        );
    }

    #[test]
    fn commit_state_is_atomic_and_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(&dir);
        let id = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";
        s.commit_state(id, None, 1, 10, b"item1", b"snap1").unwrap();
        assert!(s.commit_state(id, None, 1, 10, b"x", b"y").is_err());
        s.commit_state(id, Some(1), 2, 20, b"item2", b"snap2")
            .unwrap();
        assert_eq!(
            s.commit_state(id, Some(1), 3, 30, b"x", b"y").unwrap_err(),
            StorageError::VersionConflict
        );
        assert_eq!(
            s.commit_state("other", Some(1), 2, 1, b"x", b"y")
                .unwrap_err(),
            StorageError::NotFound
        );
        let row = s.item_row(id).unwrap().unwrap();
        assert_eq!(
            (row.version, row.ts, row.blob.as_slice()),
            (2, 20, &b"item2"[..])
        );
        let v = s.version_rows(id).unwrap();
        assert_eq!(v.iter().map(|r| r.version).collect::<Vec<_>>(), [2, 1]);
        // a failed commit leaves nothing behind (the snapshot insert is rolled back)
        assert_eq!(s.count_versions(id).unwrap(), 2);
    }

    #[test]
    fn duplicate_snapshot_key_rolls_back_the_item_update() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(&dir);
        let id = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";
        s.commit_state(id, None, 1, 10, b"item1", b"snap1").unwrap();
        // same (item, version, ts) as an existing snapshot => PK violation after the UPDATE ran
        assert!(s
            .commit_state(id, Some(1), 1, 10, b"item-bad", b"snap-bad")
            .is_err());
        let row = s.item_row(id).unwrap().unwrap();
        assert_eq!(
            row.blob, b"item1",
            "item row must be rolled back with the snapshot"
        );
        assert_eq!(row.version, 1);
    }

    #[test]
    fn purge_removes_every_row_of_the_item_and_leaves_others() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(&dir);
        let (a, b) = (
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
        );
        for id in [a, b] {
            s.commit_state(id, None, 1, 1, b"i", b"s").unwrap();
            s.commit_state(id, Some(1), 2, 2, b"i2", b"s2").unwrap();
            s.conn()
                .execute(
                    "INSERT INTO attachments (id, item_id, size) VALUES (?1, ?2, 3)",
                    params![format!("att-{id}"), id],
                )
                .unwrap();
            s.conn()
                .execute(
                    "INSERT INTO conflicts (item_id, ts, blob) VALUES (?1, 5, x'00')",
                    [id],
                )
                .unwrap();
        }
        s.purge(a, 99, b"tombstone").unwrap();
        for (table, col) in [
            ("items", "id"),
            ("versions", "item_id"),
            ("attachments", "item_id"),
            ("conflicts", "item_id"),
        ] {
            assert_eq!(
                s.count(table, col, a),
                0,
                "{table} still has rows of the purged item"
            );
            assert!(
                s.count(table, col, b) > 0,
                "{table} lost the other item's rows"
            );
        }
        assert!(s.is_tombstoned(a).unwrap());
        assert!(!s.is_tombstoned(b).unwrap());
        assert_eq!(
            s.purge(a, 100, b"tombstone").unwrap_err(),
            StorageError::NotFound
        );
        // tombstoned ids can never come back
        let e = s.commit_state(a, None, 1, 1, b"i", b"s").unwrap_err();
        assert_eq!(e.code(), "invalid_input");
    }
}
