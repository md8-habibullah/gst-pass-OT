//! SQLite persistence (`ai-docs/STORAGE_AND_SYNC.md` §5).
//!
//! The database holds only opaque envelopes plus sync bookkeeping, so it needs no encryption of
//! its own. Every state change (snapshot + latest state) commits in one transaction.

use super::error::{Result, StorageError};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;
use std::time::Duration;

/// `PRAGMA application_id` of every Passman vault file ("PMV1").
pub const APPLICATION_ID: i64 = 0x504D_5631;
/// Schema version this build writes.
pub const SCHEMA_VERSION: i64 = 1;

const KEY_BUNDLE: &str = "bundle";
const KEY_DEVICE_ID: &str = "device_id";
const KEY_CREATED_AT: &str = "created_at";

/// Schema v1. Only the small `meta` / `tombstones` tables are `WITHOUT ROWID`; the blob-heavy
/// tables are ordinary rowid tables. `targets` and `outbox` arrive with the sync engine (Phase 4, schema v2); the
/// `dirty` / `uploaded` / `remote_etag` columns they will use exist from the start.
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
}

pub struct Store {
    conn: Connection,
}

fn to_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

fn from_i64(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

impl Store {
    /// Open (creating if needed) and migrate. Refuses files that are not Passman vaults and
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
        let mut store = Self { conn };
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
                "This vault was created by a newer version of Passman. Update the app to open it."
                    .into(),
            ));
        }
        if version == 0 {
            // A fresh file, or somebody else's database: never adopt a database with tables.
            let tables: i64 =
                tx.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
            if tables > 0 || app_id != 0 {
                return Err(StorageError::Storage(
                    "That file is not a Passman vault.".into(),
                ));
            }
            tx.execute_batch(MIGRATION_V1)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if app_id != APPLICATION_ID {
            return Err(StorageError::Storage(
                "That file is not a Passman vault.".into(),
            ));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    // ---------------------------------------------------------------- meta

    fn meta(&self, key: &str) -> Result<Option<String>> {
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
            "SELECT version, ts, blob FROM versions WHERE item_id = ?1
             ORDER BY version DESC, ts DESC",
        )?;
        let rows = stmt.query_map([id], row_to_version)?;
        rows.map(|r| r?).collect()
    }

    /// The snapshot of `version` (if two devices wrote the same number, the newest timestamp).
    pub fn version_row(&self, id: &str, version: u64) -> Result<Option<VersionRow>> {
        self.conn
            .query_row(
                "SELECT version, ts, blob FROM versions WHERE item_id = ?1 AND version = ?2
                 ORDER BY ts DESC LIMIT 1",
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
        tx.commit()?;
        Ok(())
    }

    /// Permanently erase an item: tombstone first, then every row that belongs to it, all in
    /// one transaction. Afterwards the WAL is folded back and truncated so no old page copy
    /// of the erased rows lingers in the `-wal` file.
    pub fn purge(&mut self, id: &str, now: u64) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO tombstones (id, ts) VALUES (?1, ?2)",
            params![id, to_i64(now)?],
        )?;
        // Explicit, although ON DELETE CASCADE would also do it.
        tx.execute("DELETE FROM conflicts WHERE item_id = ?1", [id])?;
        tx.execute("DELETE FROM attachments WHERE item_id = ?1", [id])?;
        tx.execute("DELETE FROM versions WHERE item_id = ?1", [id])?;
        let gone = tx.execute("DELETE FROM items WHERE id = ?1", [id])?;
        if gone != 1 {
            return Err(StorageError::NotFound);
        }
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

fn row_to_version(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<VersionRow>> {
    let version: i64 = r.get(0)?;
    let ts: i64 = r.get(1)?;
    let blob: Vec<u8> = r.get(2)?;
    Ok((|| {
        Ok(VersionRow {
            version: from_i64(version)?,
            ts: from_i64(ts)?,
            blob,
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
    fn fresh_file_gets_schema_v1_and_hardened_pragmas() {
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
            assert_eq!(s.schema_version().unwrap(), 1);
            assert_eq!(s.bundle_json().unwrap().as_deref(), Some("{\"x\":1}"));
            assert_eq!(s.device_id().unwrap().as_deref(), Some("dev"));
            assert_eq!(s.created_at().unwrap(), Some(5));
        }
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
        s.purge(a, 99).unwrap();
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
        assert_eq!(s.purge(a, 100).unwrap_err(), StorageError::NotFound);
        // tombstoned ids can never come back
        let e = s.commit_state(a, None, 1, 1, b"i", b"s").unwrap_err();
        assert_eq!(e.code(), "invalid_input");
    }
}
