//! The vault side of a pull (`ai-docs/STORAGE_AND_SYNC.md` §6): everything that needs the vault
//! key, and everything that writes to the vault's tables.
//!
//! The replicator thread ([`super::pull`]) does the network work. Each time it needs the key it
//! makes one short visit through a [`VaultLink`](super::pull::VaultLink), which runs one of the
//! methods below on the unlocked [`Vault`] under the session lock. So:
//!
//! * the key never leaves the vault (there is no copy of it on the replicator thread);
//! * **every remote object is authenticated here** before anything of it is used: the envelope
//!   header's item id must equal the id in the object's key, the AEAD tag must verify, the
//!   payload must parse and carry the same id. An object that fails is [`Verdict::Damaged`] and is
//!   never applied;
//! * deciding and writing happen in one visit ([`Vault::pull_apply`]), inside one transaction that
//!   re-checks what the decision was based on, so an edit made by the person in between makes the
//!   pull step back (`Applied::Raced`) instead of overwriting it.

use super::dbops::{self, JobKind};
use super::vault_ext::tombstone_envelope_id;
use crate::crypto::{envelope, now_millis, KeyPurpose};
use crate::storage::db::{Store, VersionRow};
use crate::storage::drive;
use crate::storage::error::{Result, StorageError};
use crate::storage::model::{ItemPayload, ParentRef};
use crate::storage::vault::Vault;
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

/// An item's state as far as lineage is concerned: its header and the parent it records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    pub version: u64,
    pub ts: u64,
    pub parent: Option<ParentRef>,
}

/// One step of a chain of states (a snapshot): its version number and the parent it records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub version: u64,
    pub parent: Option<ParentRef>,
}

/// What the vault makes of a downloaded latest state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Failed authentication, or does not belong to this key (wrong id, wrong shape).
    Damaged,
    /// Nothing to do: the item was permanently deleted here, or the local copy cannot be read.
    Skip,
    /// This device already holds exactly this state.
    Same,
    /// This device has no such item.
    New(Head),
    /// Both sides have the item and the states differ.
    Differs { remote: Head, local: Head },
}

/// A downloaded snapshot, not yet authenticated.
pub struct Snap {
    pub version: u64,
    pub ts: u64,
    pub bytes: Vec<u8>,
}

/// What the replicator decided; the vault authenticates again and writes it.
pub enum Plan {
    /// A proof of deletion: erase the item here (without telling the source target, which is
    /// where it came from; the other targets are told).
    Tombstone { id: String, blob: Vec<u8> },
    /// An item this device does not have, with the history that was found for it.
    New {
        id: String,
        head: Vec<u8>,
        snapshots: Vec<Snap>,
    },
    /// The other device's state grew out of this device's head: take it over.
    FastForward {
        id: String,
        /// The local head this decision was made against (version, timestamp).
        expect: (u64, u64),
        head: Vec<u8>,
        /// The states between the old head and the new one that were downloaded on the way.
        snapshots: Vec<Snap>,
    },
    /// Both devices changed the item: the local head stays, the remote state is kept as an extra
    /// immutable version ("from another device") for the person to look at.
    OtherDevice {
        id: String,
        expect: (u64, u64),
        head: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// Something in the vault changed.
    Changed,
    /// Nothing needed to change (already there, or deleted here).
    Unchanged,
    /// The local item changed while the pull was deciding; this round leaves it alone.
    Raced,
    /// The object did not authenticate; nothing was written.
    Damaged,
}

/// Result of one [`Vault::pull_apply`]: what happened, and how many snapshots that came with the
/// item were skipped because they did not authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyResult {
    pub applied: Applied,
    pub skipped_snapshots: usize,
}

impl ApplyResult {
    fn of(applied: Applied) -> Self {
        Self {
            applied,
            skipped_snapshots: 0,
        }
    }
}

fn to_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

fn from_i64(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

// ------------------------------------------------------------------ the tables

/// Outcome of one of the pull's write transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wrote {
    Yes,
    /// The item is deleted here (or the copy already exists): nothing to write.
    Skipped,
    /// The local head is no longer the one the decision was based on.
    Raced,
}

type SnapBlob = (u64, u64, Vec<u8>);

fn exists(c: &Connection, sql: &str, id: &str) -> Result<bool> {
    Ok(c.query_row(sql, [id], |_| Ok(())).optional()?.is_some())
}

fn head_of(c: &Connection, id: &str) -> Result<Option<(u64, u64)>> {
    let row: Option<(i64, i64)> = c
        .query_row("SELECT version, ts FROM items WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    row.map(|(v, t)| Ok((from_i64(v)?, from_i64(t)?)))
        .transpose()
}

fn write_snapshots(c: &Connection, id: &str, snaps: &[SnapBlob]) -> Result<()> {
    for (version, ts, blob) in snaps {
        c.execute(
            "INSERT OR IGNORE INTO versions (item_id, version, ts, blob, uploaded)
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![id, to_i64(*version)?, to_i64(*ts)?, blob],
        )?;
    }
    Ok(())
}

/// The other targets have to learn what a pull brought in; the source already holds it.
fn queue_for_replicas(c: &Connection, id: &str, snaps: &[SnapBlob], source: &str) -> Result<()> {
    let now = now_millis();
    for (version, ts, _) in snaps {
        dbops::enqueue_for_enabled_except(c, JobKind::Version, id, *version, *ts, now, source)?;
    }
    dbops::enqueue_for_enabled_except(c, JobKind::Item, id, 0, 0, now, source)
}

impl Store {
    /// Highest version number in use for an item, whatever the snapshot's origin.
    pub fn max_version(&self, id: &str) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM versions WHERE item_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        from_i64(n)
    }

    /// Does an item hold a copy from another device that nobody has looked at yet?
    pub fn has_unreviewed_other_device(&self, id: &str) -> Result<bool> {
        exists(
            &self.conn,
            "SELECT 1 FROM versions WHERE item_id = ?1 AND origin = 'remote' AND reviewed = 0 LIMIT 1",
            id,
        )
    }

    pub fn review_other_device(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE versions SET reviewed = 1 WHERE item_id = ?1 AND origin = 'remote'",
            [id],
        )?;
        Ok(())
    }

    /// The snapshot of the state `(version, ts)`, whether this device wrote it or holds it as a
    /// copy of another device's (whose own number is `remote_version`).
    pub fn snapshot_row(&self, id: &str, version: u64, ts: u64) -> Result<Option<VersionRow>> {
        self.conn
            .query_row(
                "SELECT version, ts, blob, origin, remote_version, reviewed FROM versions
                 WHERE item_id = ?1 AND ts = ?3 AND (version = ?2 OR remote_version = ?2)
                 ORDER BY origin = 'local' DESC LIMIT 1",
                params![id, to_i64(version)?, to_i64(ts)?],
                crate::storage::db::row_to_version,
            )
            .optional()?
            .transpose()
    }

    /// Has this device already seen the state `(version, ts)` of an item?
    pub fn snapshot_known(&self, id: &str, version: u64, ts: u64) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM versions
                 WHERE item_id = ?1 AND ts = ?3 AND (version = ?2 OR remote_version = ?2) LIMIT 1",
                params![id, to_i64(version)?, to_i64(ts)?],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn pull_insert_item(
        &mut self,
        id: &str,
        head: (u64, u64, &[u8]),
        snaps: &[SnapBlob],
        source: &str,
    ) -> Result<Wrote> {
        self.in_tx(|c| {
            if exists(c, "SELECT 1 FROM tombstones WHERE id = ?1", id)? {
                return Ok(Wrote::Skipped);
            }
            if exists(c, "SELECT 1 FROM items WHERE id = ?1", id)? {
                return Ok(Wrote::Raced);
            }
            c.execute(
                "INSERT INTO items (id, version, ts, blob, dirty) VALUES (?1, ?2, ?3, ?4, 0)",
                params![id, to_i64(head.0)?, to_i64(head.1)?, head.2],
            )?;
            write_snapshots(c, id, snaps)?;
            queue_for_replicas(c, id, snaps, source)?;
            Ok(Wrote::Yes)
        })
    }

    fn pull_fast_forward(
        &mut self,
        id: &str,
        expect: (u64, u64),
        head: (u64, u64, &[u8]),
        snaps: &[SnapBlob],
        source: &str,
    ) -> Result<Wrote> {
        self.in_tx(|c| {
            if exists(c, "SELECT 1 FROM tombstones WHERE id = ?1", id)? {
                return Ok(Wrote::Skipped);
            }
            if head_of(c, id)? != Some(expect) {
                return Ok(Wrote::Raced);
            }
            c.execute(
                "UPDATE items SET version = ?2, ts = ?3, blob = ?4, dirty = 0 WHERE id = ?1",
                params![id, to_i64(head.0)?, to_i64(head.1)?, head.2],
            )?;
            write_snapshots(c, id, snaps)?;
            queue_for_replicas(c, id, snaps, source)?;
            Ok(Wrote::Yes)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn pull_add_other_device(
        &mut self,
        id: &str,
        expect: (u64, u64),
        number: u64,
        remote_version: u64,
        ts: u64,
        blob: &[u8],
    ) -> Result<Wrote> {
        self.in_tx(|c| {
            if head_of(c, id)? != Some(expect) {
                return Ok(Wrote::Raced);
            }
            let top: i64 = c.query_row(
                "SELECT COALESCE(MAX(version), 0) FROM versions WHERE item_id = ?1",
                [id],
                |r| r.get(0),
            )?;
            if from_i64(top)? >= number {
                return Ok(Wrote::Raced);
            }
            c.execute(
                "INSERT INTO versions (item_id, version, ts, blob, uploaded, origin, remote_version, reviewed)
                 VALUES (?1, ?2, ?3, ?4, 1, 'remote', ?5, 0)",
                params![id, to_i64(number)?, to_i64(ts)?, blob, to_i64(remote_version)?],
            )?;
            Ok(Wrote::Yes)
        })
    }

    /// Apply a proof of deletion that came from a target: the same erasure as a local permanent
    /// delete (tombstone first, then every row of the item, WAL folded back), except that the
    /// source target is not told again. `true` when something changed.
    fn pull_apply_tombstone(
        &mut self,
        id: &str,
        ts: u64,
        blob: &[u8],
        source: &str,
    ) -> Result<bool> {
        let had_item = self.in_tx(|c| {
            let had_item = exists(c, "SELECT 1 FROM items WHERE id = ?1", id)?;
            let inserted = c.execute(
                "INSERT OR IGNORE INTO tombstones (id, ts, uploaded, blob) VALUES (?1, ?2, 1, ?3)",
                params![id, to_i64(ts)?, blob],
            )?;
            c.execute(
                "UPDATE tombstones SET blob = ?2 WHERE id = ?1 AND blob IS NULL",
                params![id, blob],
            )?;
            if inserted == 0 && !had_item {
                return Ok(None);
            }
            c.execute("DELETE FROM conflicts WHERE item_id = ?1", [id])?;
            c.execute("DELETE FROM attachments WHERE item_id = ?1", [id])?;
            c.execute("DELETE FROM versions WHERE item_id = ?1", [id])?;
            c.execute("DELETE FROM items WHERE id = ?1", [id])?;
            dbops::forget_pending(c, id)?;
            let now = now_millis();
            // The drive files of the erased item go too: their rows here, their objects on every target
            // (the source's copy is deleted again, which is harmless). The local blobs go after the commit.
            drive::purge_files_of_item(c, id, now)?;
            dbops::enqueue_for_enabled_except(c, JobKind::Tombstone, id, 0, 0, now, source)?;
            dbops::enqueue_for_enabled_except(c, JobKind::Purge, id, 0, 0, now, source)?;
            Ok(Some(had_item))
        })?;
        let Some(had_item) = had_item else {
            return Ok(false);
        };
        if had_item {
            // Best effort, as for a local permanent delete: no old copy of the erased rows
            // lingers in the `-wal` file.
            let _ = self
                .conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        }
        Ok(true)
    }
}

// ------------------------------------------------------------------ the vault

impl Vault {
    /// Authenticate a downloaded latest state. `None` for anything that is not this item sealed
    /// with this vault's key.
    fn open_remote_head(&self, id: &str, bytes: &[u8]) -> Option<(ItemPayload, envelope::Header)> {
        let h = envelope::peek_header(bytes).ok()?;
        if h.item_id != id || h.version == 0 {
            return None;
        }
        let opened = self
            .root
            .open_object(KeyPurpose::Item, id, Some(h.version), bytes)
            .ok()?;
        let payload: ItemPayload = serde_json::from_slice(&opened.plaintext).ok()?;
        payload.check(id).ok()?;
        Some((payload, opened.header))
    }

    /// Authenticate a downloaded snapshot of `(version, ts)`.
    fn open_remote_snapshot(
        &self,
        id: &str,
        version: u64,
        ts: u64,
        bytes: &[u8],
    ) -> Option<ItemPayload> {
        let opened = self
            .root
            .open_object(KeyPurpose::Version, id, Some(version), bytes)
            .ok()?;
        if opened.header.timestamp != ts {
            return None;
        }
        let payload: ItemPayload = serde_json::from_slice(&opened.plaintext).ok()?;
        payload.check(id).ok()?;
        Some(payload)
    }

    /// Look at a downloaded latest state against what this device holds. Changes nothing.
    pub fn pull_classify(&self, id: &str, bytes: &[u8]) -> Verdict {
        let Some((remote_payload, h)) = self.open_remote_head(id, bytes) else {
            return Verdict::Damaged;
        };
        if self.store.is_tombstoned(id).unwrap_or(true) {
            return Verdict::Skip;
        }
        let remote = Head {
            version: h.version,
            ts: h.timestamp,
            parent: remote_payload.parent,
        };
        let row = match self.store.item_row(id) {
            Ok(Some(row)) => row,
            Ok(None) => return Verdict::New(remote),
            Err(_) => return Verdict::Skip,
        };
        if row.blob == bytes {
            return Verdict::Same;
        }
        // A damaged local copy is left alone: the person decides (the pull never "heals" by
        // overwriting something it cannot read).
        let Ok(local_payload) = self.open_row(&row) else {
            return Verdict::Skip;
        };
        if row.version == remote.version && row.ts == remote.ts && local_payload == remote_payload {
            return Verdict::Same;
        }
        Verdict::Differs {
            remote,
            local: Head {
                version: row.version,
                ts: row.ts,
                parent: local_payload.parent,
            },
        }
    }

    /// The step of a chain that this device holds as a snapshot (its own history, or a copy of
    /// another device's state). `None` when it has no such snapshot.
    pub fn pull_node_local(&self, id: &str, version: u64, ts: u64) -> Option<Node> {
        let row = self.store.snapshot_row(id, version, ts).ok().flatten()?;
        let (payload, _) = self.open_snapshot(id, row.version, &row.blob).ok()?;
        Some(Node {
            version: row.remote_version.unwrap_or(row.version),
            parent: payload.parent,
        })
    }

    /// The step of a chain that a downloaded snapshot describes (authenticated first).
    pub fn pull_node_remote(&self, id: &str, version: u64, ts: u64, bytes: &[u8]) -> Option<Node> {
        let payload = self.open_remote_snapshot(id, version, ts, bytes)?;
        Some(Node {
            version,
            parent: payload.parent,
        })
    }

    /// Authenticate the snapshots that came with an item; the ones that fail are dropped and
    /// counted. The state's own snapshot is made from the state itself when the backup has none.
    fn trusted_snapshots(
        &self,
        id: &str,
        snaps: Vec<Snap>,
        head: &(ItemPayload, envelope::Header),
    ) -> Result<(Vec<SnapBlob>, usize)> {
        let mut kept: Vec<SnapBlob> = Vec::new();
        let mut skipped = 0;
        for s in snaps {
            if self
                .open_remote_snapshot(id, s.version, s.ts, &s.bytes)
                .is_some()
            {
                kept.push((s.version, s.ts, s.bytes));
            } else {
                skipped += 1;
            }
        }
        let (payload, h) = head;
        if !kept
            .iter()
            .any(|(v, t, _)| *v == h.version && *t == h.timestamp)
        {
            let json = Zeroizing::new(serde_json::to_vec(payload)?);
            let blob =
                self.root
                    .seal_object(KeyPurpose::Version, id, h.version, h.timestamp, &json)?;
            kept.push((h.version, h.timestamp, blob));
        }
        Ok((kept, skipped))
    }

    fn pulled(&mut self, id: &str) {
        self.reindex(id);
        self.sync_revision = self.sync_revision.saturating_add(1);
    }

    /// Decide-and-write for one object, in one visit under the session lock.
    pub fn pull_apply(&mut self, source: &str, plan: Plan) -> Result<ApplyResult> {
        match plan {
            Plan::Tombstone { id, blob } => {
                let Ok(h) = envelope::peek_header(&blob) else {
                    return Ok(ApplyResult::of(Applied::Damaged));
                };
                let env = tombstone_envelope_id(&id);
                if h.item_id != env
                    || self
                        .root
                        .open_object(KeyPurpose::Meta, &env, Some(h.version), &blob)
                        .is_err()
                {
                    return Ok(ApplyResult::of(Applied::Damaged));
                }
                // Read before the erase: afterwards the rows are gone and the blobs would be orphans.
                let file_ids = self.store.file_ids_of_item(&id).unwrap_or_default();
                if !self
                    .store
                    .pull_apply_tombstone(&id, h.timestamp, &blob, source)?
                {
                    return Ok(ApplyResult::of(Applied::Unchanged));
                }
                self.remove_file_blobs(&file_ids);
                self.pulled(&id);
                Ok(ApplyResult::of(Applied::Changed))
            }
            Plan::New {
                id,
                head,
                snapshots,
            } => {
                let Some(opened) = self.open_remote_head(&id, &head) else {
                    return Ok(ApplyResult::of(Applied::Damaged));
                };
                let (snaps, skipped) = self.trusted_snapshots(&id, snapshots, &opened)?;
                let h = &opened.1;
                let wrote = self.store.pull_insert_item(
                    &id,
                    (h.version, h.timestamp, &head),
                    &snaps,
                    source,
                )?;
                self.finish(&id, wrote, skipped)
            }
            Plan::FastForward {
                id,
                expect,
                head,
                snapshots,
            } => {
                let Some(opened) = self.open_remote_head(&id, &head) else {
                    return Ok(ApplyResult::of(Applied::Damaged));
                };
                let (snaps, skipped) = self.trusted_snapshots(&id, snapshots, &opened)?;
                let h = &opened.1;
                let wrote = self.store.pull_fast_forward(
                    &id,
                    expect,
                    (h.version, h.timestamp, &head),
                    &snaps,
                    source,
                )?;
                self.finish(&id, wrote, skipped)
            }
            Plan::OtherDevice { id, expect, head } => {
                let Some((payload, h)) = self.open_remote_head(&id, &head) else {
                    return Ok(ApplyResult::of(Applied::Damaged));
                };
                if self.store.snapshot_known(&id, h.version, h.timestamp)? {
                    return Ok(ApplyResult::of(Applied::Unchanged));
                }
                // A number above every one in use here: the copy never shares a number with this
                // device's own states (see `Vault::next_number`).
                let number = self
                    .store
                    .max_version(&id)?
                    .max(h.version)
                    .checked_add(1)
                    .ok_or_else(|| {
                        StorageError::Storage("This item has too many versions.".into())
                    })?;
                let json = Zeroizing::new(serde_json::to_vec(&payload)?);
                let blob =
                    self.root
                        .seal_object(KeyPurpose::Version, &id, number, h.timestamp, &json)?;
                let wrote = self.store.pull_add_other_device(
                    &id,
                    expect,
                    number,
                    h.version,
                    h.timestamp,
                    &blob,
                )?;
                self.finish(&id, wrote, 0)
            }
        }
    }

    fn finish(&mut self, id: &str, wrote: Wrote, skipped: usize) -> Result<ApplyResult> {
        let applied = match wrote {
            Wrote::Yes => {
                self.pulled(id);
                Applied::Changed
            }
            Wrote::Skipped => Applied::Unchanged,
            Wrote::Raced => Applied::Raced,
        };
        Ok(ApplyResult {
            applied,
            skipped_snapshots: skipped,
        })
    }

    /// The number the pull passes on to `VaultStatus` (and the tests).
    pub fn sync_revision(&self) -> u64 {
        self.sync_revision
    }

    /// Record how many remote objects failed authentication in the last pull.
    pub fn set_remote_damaged(&mut self, n: usize) {
        self.remote_damaged = n;
    }

    pub fn remote_damaged(&self) -> usize {
        self.remote_damaged
    }
}
