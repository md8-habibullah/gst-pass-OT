//! SQL of the backup tables (`targets`, `outbox`, `remote_objects`) and the few meta rows and
//! tombstone columns that go with them. Schema in `storage/db.rs` (`MIGRATION_V2`).
//!
//! The functions that change several tables take a plain `&Connection` so they can run inside
//! one transaction with the storage change that causes them (`Store::commit_state` and friends);
//! the readers the worker and the vault use are methods of [`Store`].

use super::types::ConditionalWrites;
use crate::storage::db::Store;
use crate::storage::error::{Result, StorageError};
use rusqlite::{params, Connection, OptionalExtension};

fn to_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

fn from_i64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// What a queued job does. The rank orders a drain: the key bundle first, then proofs of
/// deletion, then immutable snapshots, then latest states, then the deletion of an erased
/// item's objects, last the shared-targets snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Bundle,
    Tombstone,
    Version,
    Item,
    Purge,
    Targets,
    /// Encrypted drive: upload a file's blob (write-once).
    FileBlob,
    /// Encrypted drive: upload or replace a file's sealed metadata.
    FileMeta,
    /// Encrypted drive: delete both objects of a file.
    FilePurge,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bundle => "bundle",
            Self::Tombstone => "tombstone",
            Self::Version => "version",
            Self::Item => "item",
            Self::Purge => "purge",
            Self::Targets => "targets",
            Self::FileBlob => "file",
            Self::FileMeta => "file_meta",
            Self::FilePurge => "file_purge",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "bundle" => Self::Bundle,
            "tombstone" => Self::Tombstone,
            "version" => Self::Version,
            "item" => Self::Item,
            "purge" => Self::Purge,
            "targets" => Self::Targets,
            "file" => Self::FileBlob,
            "file_meta" => Self::FileMeta,
            "file_purge" => Self::FilePurge,
            _ => return None,
        })
    }

    pub fn rank(self) -> i64 {
        match self {
            Self::Bundle => 0,
            Self::Tombstone => 1,
            Self::Version => 2,
            Self::Item => 3,
            Self::Purge => 4,
            Self::Targets => 5,
            // the metadata goes after the blob, so a restore never meets a name without its data
            Self::FileBlob => 6,
            Self::FileMeta => 7,
            Self::FilePurge => 4,
        }
    }
}

/// One queued job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub seq: i64,
    pub gen: i64,
    pub kind: JobKind,
    /// Item id (empty for `bundle` and for the shared-targets snapshot of kind `targets`; the
    /// vault preferences are the other `targets` job and use the ref `prefs`).
    pub target_ref: String,
    pub version: u64,
    pub ts: u64,
    pub rank: i64,
    pub enqueued_at: u64,
    pub attempts: u32,
    pub blocked: bool,
    pub last_error: Option<String>,
}

/// One row of `targets`. `config` is the sealed envelope; the rest is not secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRow {
    pub id: String,
    pub position: i64,
    pub master: bool,
    pub enabled: bool,
    pub share_in_master: bool,
    pub config: Vec<u8>,
    pub seeded: bool,
    pub conditional: ConditionalWrites,
    pub last_success_at: Option<u64>,
    pub last_checked_at: Option<u64>,
    pub last_error: Option<String>,
    pub consecutive_errors: u32,
    pub hard_error: bool,
    pub created_at: u64,
}

/// What the queue of one target looks like right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboxStats {
    pub queued: u64,
    /// Enqueue time of the oldest job that is not blocked.
    pub oldest_enqueued_at: Option<u64>,
    pub blocked: u64,
    pub first_blocked_error: Option<String>,
}

const TARGET_COLS: &str = "id, position, is_master, enabled, share_in_master, config, seeded, \
    conditional, last_success_at, last_checked_at, last_error, consecutive_errors, hard_error, created_at";

fn row_to_target(r: &rusqlite::Row<'_>) -> rusqlite::Result<TargetRow> {
    let conditional: String = r.get(7)?;
    Ok(TargetRow {
        id: r.get(0)?,
        position: r.get(1)?,
        master: r.get::<_, i64>(2)? == 1,
        enabled: r.get::<_, i64>(3)? == 1,
        share_in_master: r.get::<_, i64>(4)? == 1,
        config: r.get(5)?,
        seeded: r.get::<_, i64>(6)? == 1,
        conditional: ConditionalWrites::parse(&conditional),
        last_success_at: r.get::<_, Option<i64>>(8)?.map(from_i64),
        last_checked_at: r.get::<_, Option<i64>>(9)?.map(from_i64),
        last_error: r.get(10)?,
        consecutive_errors: u32::try_from(r.get::<_, i64>(11)?).unwrap_or(u32::MAX),
        hard_error: r.get::<_, i64>(12)? == 1,
        created_at: from_i64(r.get(13)?),
    })
}

// ------------------------------------------------------------------ enqueue (in a transaction)

const UPSERT_TAIL: &str = "ON CONFLICT (target_id, kind, ref, version, ts) DO UPDATE SET \
    gen = gen + 1, attempts = 0, blocked = 0, last_error = NULL";

/// Queue `kind` for every enabled target. A job that is already queued is not duplicated: its
/// `gen` is bumped (so one that is in flight right now is run again) and its failure record
/// (attempts, last error, blocked) is reset, because the thing it sends has just changed.
///
/// (`outbox.next_try_at` still exists in the schema but nothing reads or writes it any more:
/// there are no retry timers, a queued job waits for the next trigger.)
pub fn enqueue_for_enabled(
    conn: &Connection,
    kind: JobKind,
    target_ref: &str,
    version: u64,
    ts: u64,
    now: u64,
) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO outbox (target_id, kind, ref, version, ts, rank, enqueued_at)
             SELECT id, ?1, ?2, ?3, ?4, ?5, ?6 FROM targets WHERE enabled = 1 {UPSERT_TAIL}"
        ),
        params![
            kind.as_str(),
            target_ref,
            to_i64(version)?,
            to_i64(ts)?,
            kind.rank(),
            to_i64(now)?
        ],
    )?;
    Ok(())
}

/// Queue `kind` for every enabled target except `except` (the target a pull just read the object
/// from: it already holds it). Same rules as [`enqueue_for_enabled`].
pub fn enqueue_for_enabled_except(
    conn: &Connection,
    kind: JobKind,
    target_ref: &str,
    version: u64,
    ts: u64,
    now: u64,
    except: &str,
) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO outbox (target_id, kind, ref, version, ts, rank, enqueued_at)
             SELECT id, ?1, ?2, ?3, ?4, ?5, ?6 FROM targets WHERE enabled = 1 AND id <> ?7 {UPSERT_TAIL}"
        ),
        params![
            kind.as_str(),
            target_ref,
            to_i64(version)?,
            to_i64(ts)?,
            kind.rank(),
            to_i64(now)?,
            except
        ],
    )?;
    Ok(())
}

/// Queue `kind` for one target (which must exist).
pub fn enqueue_for_target(
    conn: &Connection,
    target_id: &str,
    kind: JobKind,
    target_ref: &str,
    version: u64,
    ts: u64,
    now: u64,
) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO outbox (target_id, kind, ref, version, ts, rank, enqueued_at)
             SELECT id, ?2, ?3, ?4, ?5, ?6, ?7 FROM targets WHERE id = ?1 {UPSERT_TAIL}"
        ),
        params![
            target_id,
            kind.as_str(),
            target_ref,
            to_i64(version)?,
            to_i64(ts)?,
            kind.rank(),
            to_i64(now)?
        ],
    )?;
    Ok(())
}

/// Queue everything this vault holds for one target (its first upload): the key bundle, every
/// latest state, every snapshot and every tombstone. Already-queued jobs are kept as they are.
pub fn seed_target(conn: &Connection, target_id: &str, now: u64) -> Result<()> {
    let now = to_i64(now)?;
    let head =
        "INSERT OR IGNORE INTO outbox (target_id, kind, ref, version, ts, rank, enqueued_at)";
    conn.execute(
        &format!("{head} VALUES (?1, 'bundle', '', 0, 0, 0, ?2)"),
        params![target_id, now],
    )?;
    conn.execute(
        &format!(
            "{head} SELECT ?1, 'tombstone', id, 0, 0, 1, ?2 FROM tombstones WHERE blob IS NOT NULL"
        ),
        params![target_id, now],
    )?;
    conn.execute(
        // Copies that came from another device are not part of this device's history; the
        // device that wrote them has already put the originals on its targets.
        &format!(
            "{head} SELECT ?1, 'version', item_id, version, ts, 2, ?2 FROM versions WHERE origin = 'local'"
        ),
        params![target_id, now],
    )?;
    conn.execute(
        &format!("{head} SELECT ?1, 'item', id, 0, 0, 3, ?2 FROM items"),
        params![target_id, now],
    )?;
    // The vault preferences (when the person has saved any) go to a new target like everything else.
    conn.execute(
        &format!(
            "{head} SELECT ?1, 'targets', 'prefs', 0, 0, {}, ?2 WHERE EXISTS (SELECT 1 FROM meta WHERE key = 'prefs')",
            JobKind::Targets.rank()
        ),
        params![target_id, now],
    )?;
    crate::storage::drive::seed_files(conn, target_id, now)?;
    Ok(())
}

/// Drop the queued uploads of an item that was just erased (its tombstone and purge jobs are
/// queued next).
pub fn forget_pending(conn: &Connection, item_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM outbox WHERE ref = ?1 AND kind IN ('item', 'version')",
        [item_id],
    )?;
    Ok(())
}

// ------------------------------------------------------------------ targets (in a transaction)

pub fn next_position(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM targets",
        [],
        |r| r.get(0),
    )?)
}

pub fn insert_target(conn: &Connection, t: &TargetRow) -> Result<()> {
    conn.execute(
        "INSERT INTO targets (id, position, is_master, enabled, share_in_master, config, seeded,
                              conditional, last_success_at, last_checked_at, last_error,
                              consecutive_errors, hard_error, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            t.id,
            t.position,
            i64::from(t.master),
            i64::from(t.enabled),
            i64::from(t.share_in_master),
            t.config,
            i64::from(t.seeded),
            t.conditional.as_str(),
            t.last_success_at.map(to_i64).transpose()?,
            t.last_checked_at.map(to_i64).transpose()?,
            t.last_error,
            i64::from(t.consecutive_errors),
            i64::from(t.hard_error),
            to_i64(t.created_at)?,
        ],
    )?;
    Ok(())
}

pub fn update_target_settings(
    conn: &Connection,
    id: &str,
    config: &[u8],
    enabled: bool,
    share_in_master: bool,
    conditional: ConditionalWrites,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE targets SET config = ?2, enabled = ?3, share_in_master = ?4, conditional = ?5
         WHERE id = ?1",
        params![
            id,
            config,
            i64::from(enabled),
            i64::from(share_in_master),
            conditional.as_str()
        ],
    )?;
    if changed != 1 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub fn set_seeded(conn: &Connection, id: &str, seeded: bool) -> Result<()> {
    conn.execute(
        "UPDATE targets SET seeded = ?2 WHERE id = ?1",
        params![id, i64::from(seeded)],
    )?;
    Ok(())
}

/// Remove a target with its queue and remote bookkeeping (nothing is touched in the bucket).
pub fn delete_target(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM outbox WHERE target_id = ?1", [id])?;
    conn.execute("DELETE FROM remote_objects WHERE target_id = ?1", [id])?;
    let gone = conn.execute("DELETE FROM targets WHERE id = ?1", [id])?;
    if gone != 1 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub fn clear_queue(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM outbox WHERE target_id = ?1", [id])?;
    Ok(())
}

pub fn clear_remote_objects(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM remote_objects WHERE target_id = ?1", [id])?;
    Ok(())
}

/// Make `id` the only master (the previous one is demoted first: a unique index allows one).
pub fn set_master(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("UPDATE targets SET is_master = 0 WHERE is_master = 1", [])?;
    let changed = conn.execute("UPDATE targets SET is_master = 1 WHERE id = ?1", [id])?;
    if changed != 1 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub fn clear_master(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE targets SET is_master = 0 WHERE is_master = 1", [])?;
    Ok(())
}

/// Forget what went wrong before: the person changed the settings, so old errors are stale.
pub fn reset_health(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE targets SET last_error = NULL, consecutive_errors = 0, hard_error = 0 WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn delete_meta(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM meta WHERE key = ?1", [key])?;
    Ok(())
}

/// Every target, the master first, then in creation order.
pub fn read_targets(conn: &Connection) -> Result<Vec<TargetRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TARGET_COLS} FROM targets ORDER BY is_master DESC, position ASC"
    ))?;
    let rows = stmt.query_map([], row_to_target)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn read_target(conn: &Connection, id: &str) -> Result<Option<TargetRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {TARGET_COLS} FROM targets WHERE id = ?1"),
            [id],
            row_to_target,
        )
        .optional()?)
}

// ------------------------------------------------------------------ readers and small writers

impl Store {
    pub fn targets_all(&self) -> Result<Vec<TargetRow>> {
        read_targets(&self.conn)
    }

    pub fn target_row(&self, id: &str) -> Result<Option<TargetRow>> {
        read_target(&self.conn, id)
    }

    pub fn outbox_stats(&self, target_id: &str) -> Result<OutboxStats> {
        let (queued, oldest, blocked): (i64, Option<i64>, Option<i64>) = self.conn.query_row(
            "SELECT count(*), min(CASE WHEN blocked = 0 THEN enqueued_at END), sum(blocked)
             FROM outbox WHERE target_id = ?1",
            [target_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let first_blocked_error: Option<String> = self
            .conn
            .query_row(
                "SELECT last_error FROM outbox WHERE target_id = ?1 AND blocked = 1
                 ORDER BY seq LIMIT 1",
                [target_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(OutboxStats {
            queued: from_i64(queued),
            oldest_enqueued_at: oldest.map(from_i64),
            blocked: from_i64(blocked.unwrap_or(0)),
            first_blocked_error,
        })
    }

    /// How many jobs wait for enabled targets (what a flush has to send).
    pub fn outbox_pending_enabled(&self) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM outbox o JOIN targets t ON t.id = o.target_id WHERE t.enabled = 1",
            [],
            |r| r.get(0),
        )?;
        Ok(from_i64(n))
    }

    /// Up to `limit` jobs of one target in drain order, after the `(rank, seq)` cursor. Every
    /// queued job is returned: a trigger (unlock, a committed change, the flush, "Back up now")
    /// retries everything that is still waiting, whatever happened to it before.
    pub fn outbox_batch(
        &self,
        target_id: &str,
        after: (i64, i64),
        limit: usize,
    ) -> Result<Vec<Job>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, gen, kind, ref, version, ts, rank, enqueued_at, attempts, blocked, last_error
             FROM outbox
             WHERE target_id = ?1 AND (rank > ?2 OR (rank = ?2 AND seq > ?3))
             ORDER BY rank, seq LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![
                target_id,
                after.0,
                after.1,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            |r| {
                let kind: String = r.get(2)?;
                Ok((
                    JobKind::parse(&kind),
                    Job {
                        seq: r.get(0)?,
                        gen: r.get(1)?,
                        kind: JobKind::Bundle,
                        target_ref: r.get(3)?,
                        version: from_i64(r.get(4)?),
                        ts: from_i64(r.get(5)?),
                        rank: r.get(6)?,
                        enqueued_at: from_i64(r.get(7)?),
                        attempts: u32::try_from(r.get::<_, i64>(8)?).unwrap_or(u32::MAX),
                        blocked: r.get::<_, i64>(9)? == 1,
                        last_error: r.get(10)?,
                    },
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (kind, mut job) = row?;
            // The CHECK constraint makes an unknown kind impossible; skip it rather than guess.
            if let Some(kind) = kind {
                job.kind = kind;
                out.push(job);
            }
        }
        Ok(out)
    }

    /// Remove a finished job, but only if nothing bumped it while it ran. `false` = it was
    /// bumped and stays queued.
    pub fn outbox_complete(&self, seq: i64, gen: i64) -> Result<bool> {
        let gone = self.conn.execute(
            "DELETE FROM outbox WHERE seq = ?1 AND gen = ?2",
            params![seq, gen],
        )?;
        Ok(gone == 1)
    }

    /// Remember a failed attempt on a job: one more attempt, the reason, and whether the job is
    /// blocked (a conflict that waits for a person, not just for the network). The job stays in
    /// the queue and is tried again at the next trigger; no retry time is stored.
    pub fn outbox_defer(&self, seq: i64, error: &str, blocked: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET attempts = attempts + 1, last_error = ?2, blocked = ?3
             WHERE seq = ?1",
            params![seq, error, i64::from(blocked)],
        )?;
        Ok(())
    }

    /// A job that was blocked and has now gone through is no longer blocked (it is deleted by
    /// `outbox_complete`); this clears the flag on one that is retried and fails differently.
    pub fn outbox_unblock(&self, seq: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET blocked = 0, last_error = NULL WHERE seq = ?1",
            [seq],
        )?;
        Ok(())
    }

    // ---- remote bookkeeping (single-writer rule)

    pub fn remote_object(
        &self,
        target_id: &str,
        key: &str,
    ) -> Result<Option<(Option<String>, u64)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT etag, version FROM remote_objects WHERE target_id = ?1 AND key = ?2",
                params![target_id, key],
                |r| Ok((r.get::<_, Option<String>>(0)?, from_i64(r.get(1)?))),
            )
            .optional()?)
    }

    pub fn set_remote_object(
        &self,
        target_id: &str,
        key: &str,
        etag: Option<&str>,
        version: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO remote_objects (target_id, key, etag, version) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (target_id, key) DO UPDATE SET etag = excluded.etag, version = excluded.version",
            params![target_id, key, etag, to_i64(version)?],
        )?;
        Ok(())
    }

    /// Forget the bookkeeping of a key and of everything below `prefix` (after a purge).
    pub fn forget_remote_objects(&self, target_id: &str, key: &str, prefix: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM remote_objects WHERE target_id = ?1
             AND (key = ?2 OR substr(key, 1, length(?3)) = ?3)",
            params![target_id, key, prefix],
        )?;
        Ok(())
    }

    // ---- health columns

    /// The target answered: clear every error and remember when.
    pub fn target_record_success(&self, id: &str, now: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET last_success_at = ?2, last_checked_at = ?2, last_error = NULL,
                    consecutive_errors = 0, hard_error = 0 WHERE id = ?1",
            params![id, to_i64(now)?],
        )?;
        Ok(())
    }

    pub fn target_record_failure(&self, id: &str, error: &str, hard: bool, now: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET last_checked_at = ?2, last_error = ?3,
                    consecutive_errors = MIN(consecutive_errors + 1, 1000000), hard_error = ?4
             WHERE id = ?1",
            params![id, to_i64(now)?, error, i64::from(hard)],
        )?;
        Ok(())
    }

    /// Update what the target shows as its last problem without counting another failed attempt
    /// (one trigger is one attempt, even when both its pull and its push hit the same problem).
    pub fn target_note_failure(&self, id: &str, error: &str, hard: bool, now: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET last_checked_at = ?2, last_error = ?3, hard_error = ?4,
                    consecutive_errors = MAX(consecutive_errors, 1)
             WHERE id = ?1",
            params![id, to_i64(now)?, error, i64::from(hard)],
        )?;
        Ok(())
    }

    pub fn target_set_conditional(&self, id: &str, c: ConditionalWrites) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET conditional = ?2 WHERE id = ?1",
            params![id, c.as_str()],
        )?;
        Ok(())
    }

    // ---- tombstones

    pub fn tombstone_blob(&self, id: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT blob FROM tombstones WHERE id = ?1", [id], |r| {
                r.get::<_, Option<Vec<u8>>>(0)
            })
            .optional()?
            .flatten())
    }

    /// Tombstones written before schema v2 have no sealed blob yet: `(id, ts)`.
    pub fn tombstones_without_blob(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, ts FROM tombstones WHERE blob IS NULL ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, from_i64(r.get(1)?))))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_tombstone_blob(&self, id: &str, blob: &[u8]) -> Result<()> {
        self.conn.execute(
            "UPDATE tombstones SET blob = ?2 WHERE id = ?1 AND blob IS NULL",
            params![id, blob],
        )?;
        Ok(())
    }

    /// The exact snapshot a `version` job refers to.
    pub fn version_blob(&self, id: &str, version: u64, ts: u64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT blob FROM versions WHERE item_id = ?1 AND version = ?2 AND ts = ?3",
                params![id, to_i64(version)?, to_i64(ts)?],
                |r| r.get(0),
            )
            .optional()?)
    }

    // ---- restore from a backup (a brand-new database, no targets yet)

    /// Insert a downloaded latest state. The envelope is authenticated at the first unlock.
    pub fn restore_item(&self, id: &str, version: u64, ts: u64, blob: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO items (id, version, ts, blob, dirty) VALUES (?1, ?2, ?3, ?4, 0)",
            params![id, to_i64(version)?, to_i64(ts)?, blob],
        )?;
        Ok(())
    }

    /// Insert a downloaded snapshot; `false` if its item was not restored (no foreign key target).
    pub fn restore_version(&self, id: &str, version: u64, ts: u64, blob: &[u8]) -> Result<bool> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO versions (item_id, version, ts, blob, uploaded)
             SELECT ?1, ?2, ?3, ?4, 1 WHERE EXISTS (SELECT 1 FROM items WHERE id = ?1)",
            params![id, to_i64(version)?, to_i64(ts)?, blob],
        )?;
        Ok(n == 1)
    }

    pub fn restore_tombstone(&self, id: &str, ts: u64, blob: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO tombstones (id, ts, uploaded, blob) VALUES (?1, ?2, 1, ?3)",
            params![id, to_i64(ts)?, blob],
        )?;
        Ok(())
    }

    // ---- meta rows owned by the backup code

    pub fn put_meta(&mut self, key: &str, value: &str) -> Result<()> {
        self.in_tx(|c| set_meta(c, key, value))
    }

    pub fn remove_meta(&mut self, key: &str) -> Result<()> {
        self.in_tx(|c| delete_meta(c, key))
    }

    /// Run the restore inserts in one transaction.
    pub fn restore_batch<T>(&mut self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("vault.db")).unwrap();
        (dir, s)
    }

    fn target(id: &str, position: i64, master: bool, enabled: bool) -> TargetRow {
        TargetRow {
            id: id.into(),
            position,
            master,
            enabled,
            share_in_master: true,
            config: b"sealed".to_vec(),
            seeded: false,
            conditional: ConditionalWrites::Unknown,
            last_success_at: None,
            last_checked_at: None,
            last_error: None,
            consecutive_errors: 0,
            hard_error: false,
            created_at: 1,
        }
    }

    fn add(s: &mut Store, t: &TargetRow) {
        s.in_tx(|c| insert_target(c, t)).unwrap();
    }

    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    const ID: &str = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";

    #[test]
    fn exactly_one_master_is_enforced_by_the_database() {
        let (_d, mut s) = store();
        add(&mut s, &target(A, 1, true, true));
        let second_master = s.in_tx(|c| insert_target(c, &target(B, 2, true, true)));
        assert!(second_master.is_err(), "a unique index allows one master");
        add(&mut s, &target(B, 2, false, true));
        s.in_tx(|c| set_master(c, B)).unwrap();
        let all = s.targets_all().unwrap();
        assert_eq!(all.iter().filter(|t| t.master).count(), 1);
        assert_eq!(all[0].id, B, "the master is listed first");
        assert_eq!(all[1].id, A);
        assert_eq!(
            s.in_tx(|c| set_master(c, "nope")).unwrap_err(),
            StorageError::NotFound
        );
        // a failed promotion leaves the old master in place
        assert_eq!(s.targets_all().unwrap()[0].id, B);
    }

    #[test]
    fn enqueue_goes_to_enabled_targets_only_and_collapses_repeats() {
        let (_d, mut s) = store();
        add(&mut s, &target(A, 1, true, true));
        add(&mut s, &target(B, 2, false, false));
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Item, ID, 0, 0, 100))
            .unwrap();
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Version, ID, 2, 50, 101))
            .unwrap();
        assert_eq!(s.outbox_stats(A).unwrap().queued, 2);
        assert_eq!(
            s.outbox_stats(B).unwrap().queued,
            0,
            "disabled targets get nothing"
        );
        // the same item edited again: still one job, but bumped
        let before = s.outbox_batch(A, (-1, 0), 10).unwrap();
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Item, ID, 0, 0, 500))
            .unwrap();
        let after = s.outbox_batch(A, (-1, 0), 10).unwrap();
        assert_eq!(after.len(), 2);
        let item = |jobs: &[Job]| {
            jobs.iter()
                .find(|j| j.kind == JobKind::Item)
                .unwrap()
                .clone()
        };
        assert_eq!(item(&after).gen, item(&before).gen + 1);
        assert_eq!(item(&after).seq, item(&before).seq);
        assert_eq!(
            item(&after).enqueued_at,
            100,
            "age counts from the oldest unpushed change"
        );
        // drain order: snapshot before latest state
        assert_eq!(
            after.iter().map(|j| j.kind).collect::<Vec<_>>(),
            [JobKind::Version, JobKind::Item]
        );
    }

    #[test]
    fn a_job_bumped_while_it_runs_is_not_lost() {
        let (_d, mut s) = store();
        add(&mut s, &target(A, 1, true, true));
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Item, ID, 0, 0, 1))
            .unwrap();
        let running = s.outbox_batch(A, (-1, 0), 10).unwrap().remove(0);
        // a new edit lands while the worker is pushing
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Item, ID, 0, 0, 2))
            .unwrap();
        assert!(!s.outbox_complete(running.seq, running.gen).unwrap());
        assert_eq!(s.outbox_stats(A).unwrap().queued, 1, "still queued");
        let again = s.outbox_batch(A, (-1, 0), 10).unwrap().remove(0);
        assert!(s.outbox_complete(again.seq, again.gen).unwrap());
        assert_eq!(s.outbox_stats(A).unwrap().queued, 0);
    }

    #[test]
    fn a_failed_job_stays_queued_and_every_trigger_sees_it_again() {
        let (_d, mut s) = store();
        add(&mut s, &target(A, 1, true, true));
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Item, ID, 0, 0, 10))
            .unwrap();
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Bundle, "", 0, 0, 20))
            .unwrap();
        let jobs = s.outbox_batch(A, (-1, 0), 10).unwrap();
        assert_eq!(jobs[0].kind, JobKind::Bundle);
        // a conflict that waits for a person, and a plain network failure
        s.outbox_defer(jobs[1].seq, "newer remote", true).unwrap();
        s.outbox_defer(jobs[0].seq, "timeout", false).unwrap();
        s.outbox_defer(jobs[0].seq, "timeout again", false).unwrap();
        // there is no retry time: both are returned by the very next batch
        let again = s.outbox_batch(A, (-1, 0), 10).unwrap();
        assert_eq!(again.len(), 2);
        assert_eq!(again[0].attempts, 2, "attempts are counted");
        assert_eq!(again[0].last_error.as_deref(), Some("timeout again"));
        assert!(!again[0].blocked && again[1].blocked);
        // the column still exists for schema compatibility, but nothing writes it
        let nonzero: i64 = s
            .conn()
            .query_row(
                "SELECT count(*) FROM outbox WHERE next_try_at <> 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nonzero, 0);
        let st = s.outbox_stats(A).unwrap();
        assert_eq!(st.blocked, 1);
        assert_eq!(st.first_blocked_error.as_deref(), Some("newer remote"));
        assert_eq!(
            st.oldest_enqueued_at,
            Some(20),
            "blocked jobs do not make a target lag"
        );
        // changing the thing a job sends resets its failure record
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Item, ID, 0, 0, 30))
            .unwrap();
        let bumped = s.outbox_batch(A, (-1, 0), 10).unwrap();
        let item = bumped.iter().find(|j| j.kind == JobKind::Item).unwrap();
        assert_eq!(
            (item.attempts, item.blocked, item.last_error.clone()),
            (0, false, None)
        );
        // cursor paging by (rank, seq)
        let first = s.outbox_batch(A, (-1, 0), 1).unwrap();
        let next = s.outbox_batch(A, (first[0].rank, first[0].seq), 1).unwrap();
        assert_eq!(first[0].kind, JobKind::Bundle);
        assert_eq!(next[0].kind, JobKind::Item);
        // what a flush has to send: jobs of enabled targets only
        assert_eq!(s.outbox_pending_enabled().unwrap(), 2);
        add(&mut s, &target(B, 2, false, false));
        s.in_tx(|c| enqueue_for_target(c, B, JobKind::Bundle, "", 0, 0, 1))
            .unwrap();
        assert_eq!(
            s.outbox_pending_enabled().unwrap(),
            2,
            "a disabled target is not flushed"
        );
    }

    #[test]
    fn seeding_queues_everything_the_vault_holds() {
        let (_d, mut s) = store();
        s.commit_state(ID, None, 1, 10, b"i1", b"s1").unwrap();
        s.commit_state(ID, Some(1), 2, 20, b"i2", b"s2").unwrap();
        s.commit_state(A, None, 1, 30, b"i", b"s").unwrap();
        s.purge(A, 99, b"tomb").unwrap();
        add(&mut s, &target(B, 1, true, true));
        s.in_tx(|c| seed_target(c, B, 5)).unwrap();
        let kinds: Vec<(JobKind, String)> = s
            .outbox_batch(B, (-1, 0), 100)
            .unwrap()
            .into_iter()
            .map(|j| (j.kind, j.target_ref))
            .collect();
        assert_eq!(kinds[0], (JobKind::Bundle, String::new()));
        assert!(kinds.contains(&(JobKind::Tombstone, A.to_string())));
        assert_eq!(
            kinds.iter().filter(|(k, _)| *k == JobKind::Version).count(),
            2
        );
        assert!(kinds.contains(&(JobKind::Item, ID.to_string())));
        // seeding twice adds nothing
        s.in_tx(|c| seed_target(c, B, 6)).unwrap();
        assert_eq!(s.outbox_stats(B).unwrap().queued as usize, kinds.len());
    }

    #[test]
    fn commit_and_purge_enqueue_in_the_same_transaction() {
        let (_d, mut s) = store();
        add(&mut s, &target(A, 1, true, true));
        s.commit_state(ID, None, 1, 10, b"i1", b"s1").unwrap();
        s.commit_state(ID, Some(1), 2, 20, b"i2", b"s2").unwrap();
        let jobs = s.outbox_batch(A, (-1, 0), 100).unwrap();
        assert_eq!(
            jobs.iter().map(|j| (j.kind, j.version)).collect::<Vec<_>>(),
            [
                (JobKind::Version, 1),
                (JobKind::Version, 2),
                (JobKind::Item, 0)
            ]
        );
        // a failed commit enqueues nothing
        assert!(s.commit_state(ID, Some(7), 8, 80, b"x", b"y").is_err());
        assert_eq!(s.outbox_stats(A).unwrap().queued, 3);
        // purge drops the pending uploads of the item and queues the proof and the deletion
        s.purge(ID, 99, b"tomb").unwrap();
        let kinds: Vec<JobKind> = s
            .outbox_batch(A, (-1, 0), 100)
            .unwrap()
            .into_iter()
            .map(|j| j.kind)
            .collect();
        assert_eq!(kinds, [JobKind::Tombstone, JobKind::Purge]);
        assert_eq!(s.tombstone_blob(ID).unwrap().as_deref(), Some(&b"tomb"[..]));
    }

    #[test]
    fn deleting_a_target_removes_its_queue_and_bookkeeping_only() {
        let (_d, mut s) = store();
        add(&mut s, &target(A, 1, true, true));
        add(&mut s, &target(B, 2, false, true));
        s.in_tx(|c| enqueue_for_enabled(c, JobKind::Bundle, "", 0, 0, 1))
            .unwrap();
        s.set_remote_object(A, "k", Some("\"e\""), 3).unwrap();
        s.set_remote_object(B, "k", Some("\"f\""), 4).unwrap();
        s.in_tx(|c| delete_target(c, A)).unwrap();
        assert_eq!(s.outbox_stats(A).unwrap().queued, 0);
        assert_eq!(s.outbox_stats(B).unwrap().queued, 1);
        assert_eq!(s.remote_object(A, "k").unwrap(), None);
        assert_eq!(
            s.remote_object(B, "k").unwrap(),
            Some((Some("\"f\"".into()), 4))
        );
        assert_eq!(
            s.in_tx(|c| delete_target(c, A)).unwrap_err(),
            StorageError::NotFound
        );
    }

    #[test]
    fn health_columns_and_remote_bookkeeping() {
        let (_d, mut s) = store();
        add(&mut s, &target(A, 1, true, true));
        s.target_record_failure(A, "boom", false, 10).unwrap();
        s.target_record_failure(A, "boom", true, 20).unwrap();
        let t = s.target_row(A).unwrap().unwrap();
        assert_eq!(
            (t.consecutive_errors, t.hard_error, t.last_checked_at),
            (2, true, Some(20))
        );
        assert_eq!(t.last_error.as_deref(), Some("boom"));
        s.target_record_success(A, 30).unwrap();
        let t = s.target_row(A).unwrap().unwrap();
        assert_eq!(
            (t.consecutive_errors, t.hard_error, t.last_success_at),
            (0, false, Some(30))
        );
        assert_eq!(t.last_error, None);
        // forgetting a prefix
        s.set_remote_object(A, "vault/items/x.enc", None, 1)
            .unwrap();
        s.set_remote_object(A, "vault/versions/x/1_2.enc", None, 1)
            .unwrap();
        s.set_remote_object(A, "vault/versions/y/1_2.enc", None, 1)
            .unwrap();
        s.forget_remote_objects(A, "vault/items/x.enc", "vault/versions/x/")
            .unwrap();
        assert_eq!(s.remote_object(A, "vault/items/x.enc").unwrap(), None);
        assert_eq!(
            s.remote_object(A, "vault/versions/x/1_2.enc").unwrap(),
            None
        );
        assert!(s
            .remote_object(A, "vault/versions/y/1_2.enc")
            .unwrap()
            .is_some());
    }
}
