//! The backup side of an unlocked [`Vault`]: target settings sealed with the vault key, the
//! shared-targets snapshot, the replication engine's handle, and the operations behind the
//! `backup_*` commands.
//!
//! * A target's connection details (name, endpoint, region, bucket, folder, access key id and
//!   **secret key**) are one JSON document sealed with the vault's `meta` sub-key
//!   (`target-<id>`), so nothing that identifies a bucket is readable in the vault file.
//! * Targets marked "Save in master backup" are also collected into one sealed snapshot
//!   (`shared-targets`) that is replicated to the **master** target only; a fresh device that
//!   restores from the master gets them back at its first unlock.
//! * No network call is made here: the session probes a target (or drains a queue) *between*
//!   two short visits to this module, never while the vault state is locked.

use super::dbops::{self, JobKind, TargetRow};
use super::probe::ProbeReport;
use super::pull::VaultLink;
use super::replicator::{compute_health, BackupRuntime, Replicator, RuntimeTarget};
use super::types::{
    name_key, BackupTargetInput, BackupTargetView, ConditionalWrites, HealthState, TargetConfig,
};
use super::{PENDING_TARGETS_META, SHARED_TARGETS_META};
use crate::crypto::{envelope, now_millis, KeyPurpose, VaultRootKey};
use crate::storage::error::{Result, StorageError};
use crate::storage::uuid;
use crate::storage::vault::Vault;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use zeroize::Zeroizing;

/// More than this many targets is almost certainly a mistake (and a long drain).
pub const MAX_TARGETS: usize = 12;
const SNAPSHOT_ID: &str = "shared-targets";

/// The engine of one unlocked vault. Dropping it (lock, profile switch, quit) stops the worker.
pub struct BackupHandle {
    pub runtime: Arc<BackupRuntime>,
    pub replicator: Arc<Replicator>,
}

impl Drop for BackupHandle {
    fn drop(&mut self) {
        self.replicator.stop();
    }
}

/// The envelope id of a sealed tombstone.
pub fn tombstone_envelope_id(item_id: &str) -> String {
    format!("tombstone-{item_id}")
}

fn config_envelope_id(target_id: &str) -> String {
    format!("target-{target_id}")
}

/// What the person asked for, validated and ready to commit (after the probe).
pub struct Prepared {
    /// `Some` when this edits an existing target.
    pub existing: Option<String>,
    pub config: TargetConfig,
    pub enabled: bool,
    pub master: bool,
    pub share_in_master: bool,
}

// ------------------------------------------------------------------ sealing

fn seal_config(root: &VaultRootKey, id: &str, cfg: &TargetConfig, now: u64) -> Result<Vec<u8>> {
    let json = Zeroizing::new(serde_json::to_vec(cfg)?);
    Ok(root.seal_object(KeyPurpose::Meta, &config_envelope_id(id), 1, now, &json)?)
}

fn open_config(root: &VaultRootKey, id: &str, blob: &[u8]) -> Result<TargetConfig> {
    let opened = root.open_object(KeyPurpose::Meta, &config_envelope_id(id), None, blob)?;
    Ok(serde_json::from_slice(&opened.plaintext)?)
}

/// What the shared snapshot carries for one target.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotTarget {
    id: String,
    enabled: bool,
    master: bool,
    config: TargetConfig,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    v: u32,
    targets: Vec<SnapshotTarget>,
}

/// Rebuild the sealed shared-targets snapshot from the current rows (inside the caller's
/// transaction) and queue it for the master.
fn refresh_snapshot(root: &VaultRootKey, conn: &Connection, now: u64) -> Result<()> {
    let rows = dbops::read_targets(conn)?;
    let mut targets = Vec::new();
    for row in rows.iter().filter(|r| r.share_in_master) {
        // An unreadable stored config is left out rather than failing every target change.
        if let Ok(config) = open_config(root, &row.id, &row.config) {
            targets.push(SnapshotTarget {
                id: row.id.clone(),
                enabled: row.enabled,
                master: row.master,
                config,
            });
        }
    }
    let json = Zeroizing::new(serde_json::to_vec(&Snapshot { v: 1, targets })?);
    // Strictly increasing, so a newer snapshot is always recognisably newer than an older one.
    let previous = dbops::get_meta(conn, SHARED_TARGETS_META)?
        .and_then(|b64| B64.decode(b64).ok())
        .and_then(|blob| envelope::peek_header(&blob).ok())
        .map_or(0, |h| h.version);
    let version = now.max(previous.saturating_add(1));
    let blob = root.seal_object(KeyPurpose::Meta, SNAPSHOT_ID, version, now, &json)?;
    dbops::set_meta(conn, SHARED_TARGETS_META, &B64.encode(&blob))?;
    if let Some(master) = rows.iter().find(|r| r.master && r.enabled) {
        dbops::enqueue_for_target(conn, &master.id, JobKind::Targets, "", 0, 0, now)?;
    }
    Ok(())
}

fn location_of(c: &TargetConfig) -> (Option<&str>, &str, &str, &str) {
    (
        c.endpoint.as_deref(),
        c.region.as_str(),
        c.bucket.as_str(),
        c.prefix.as_str(),
    )
}

fn unreadable() -> StorageError {
    StorageError::Storage(
        "The settings of this backup target could not be read. Remove it and add it again.".into(),
    )
}

const MSG_UNREADABLE: &str =
    "The saved settings of this target could not be decrypted. Remove it and add it again.";

impl Vault {
    // ------------------------------------------------------------------ the engine

    /// Create the replication engine for this unlocked vault and do the per-unlock housekeeping.
    /// Every step is best effort: a problem here must never stop the person from opening their
    /// vault (an unfinished step is simply tried again at the next unlock).
    ///
    /// The worker is **not** started here: the session installs the vault first and then calls
    /// [`start_backup`](Self::start_backup), so a pull that starts at once finds the vault.
    pub fn attach_backup(&mut self, runtime: Arc<BackupRuntime>, link: Arc<dyn VaultLink>) {
        let replicator = Arc::new(Replicator::new(
            self.store.path().to_path_buf(),
            Arc::clone(&runtime),
        ));
        replicator.set_link(link);
        self.backup = Some(BackupHandle {
            runtime,
            replicator,
        });
        let _ = self.backfill_tombstones();
        let _ = self.import_pending_targets();
        let _ = self.seed_unseeded_targets();
        let _ = self.queue_snapshot_for_master();
    }

    /// Trigger 1 (unlock): hand the worker its targets and ask it to pull, reconcile and drain.
    /// Returns at once; nothing here touches the network.
    pub fn start_backup(&self) {
        self.sync_backup();
        if let Some(b) = &self.backup {
            b.replicator.request_reconcile();
        }
    }

    /// Trigger 4 (the device is back online): the same pull, reconcile and drain as at unlock, for work that could not
    /// be sent, or changes that could not be fetched, while there was no connection. Returns at once; nothing here
    /// touches the network. Asking while a pull is already queued or running changes nothing.
    pub fn request_sync(&self) {
        if let Some(b) = &self.backup {
            b.replicator.request_reconcile();
        }
    }

    /// Is a pull queued or running?
    pub fn backup_reconciling(&self) -> bool {
        self.backup
            .as_ref()
            .is_some_and(|b| b.replicator.is_reconciling())
    }

    pub fn backup_runtime(&self) -> Result<Arc<BackupRuntime>> {
        self.backup
            .as_ref()
            .map(|b| Arc::clone(&b.runtime))
            .ok_or_else(|| StorageError::Storage("Backups are not available right now.".into()))
    }

    pub fn backup_replicator(&self) -> Option<Arc<Replicator>> {
        self.backup.as_ref().map(|b| Arc::clone(&b.replicator))
    }

    /// Jobs waiting for enabled targets (what a flush would send). `0` when the database cannot
    /// be read: a flush then has nothing it could do either.
    pub fn backup_pending(&self) -> u64 {
        self.store.outbox_pending_enabled().unwrap_or(0)
    }

    /// New work was committed: let the worker know.
    pub(in crate::storage) fn wake_backup(&self) {
        if let Some(b) = &self.backup {
            b.replicator.wake();
        }
    }

    /// Hand the worker the current set of enabled targets (decrypted, wiped when it stops).
    fn sync_backup(&self) {
        let Some(b) = &self.backup else { return };
        let mut out = Vec::new();
        if let Ok(rows) = self.store.targets_all() {
            for row in rows.into_iter().filter(|r| r.enabled) {
                if let Ok(config) = open_config(&self.root, &row.id, &row.config) {
                    out.push(RuntimeTarget {
                        id: row.id,
                        position: row.position,
                        master: row.master,
                        conditional: row.conditional,
                        config,
                    });
                }
            }
        }
        b.replicator.set_targets(out);
    }

    // ------------------------------------------------------------------ tombstones

    /// The sealed proof of deletion every target receives.
    pub(in crate::storage) fn seal_tombstone(&self, item_id: &str, ts: u64) -> Result<Vec<u8>> {
        Ok(self.root.seal_object(
            KeyPurpose::Meta,
            &tombstone_envelope_id(item_id),
            1,
            ts,
            b"{}",
        )?)
    }

    /// Tombstones written before schema v2 carry no sealed blob yet.
    fn backfill_tombstones(&mut self) -> Result<()> {
        for (id, ts) in self.store.tombstones_without_blob()? {
            let blob = self.seal_tombstone(&id, ts)?;
            self.store.set_tombstone_blob(&id, &blob)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ restore + seeding

    /// A restored profile carries the downloaded shared-targets snapshot in `pending_targets`
    /// (it could not be opened without the root key). Import it now and drop the pending copy.
    fn import_pending_targets(&mut self) -> Result<()> {
        let Some(b64) = self.store.meta(PENDING_TARGETS_META)? else {
            return Ok(());
        };
        let blob = B64
            .decode(b64.trim())
            .map_err(|_| StorageError::Storage("Bad vault metadata.".into()))?;
        let opened = self
            .root
            .open_object(KeyPurpose::Meta, SNAPSHOT_ID, None, &blob)?;
        let snapshot: Snapshot = serde_json::from_slice(&opened.plaintext)?;
        drop(opened);
        let now = now_millis();
        let root = &self.root;
        self.store.in_tx(|c| {
            let existing = dbops::read_targets(c)?;
            let mut names: HashSet<String> = existing
                .iter()
                .filter_map(|r| open_config(root, &r.id, &r.config).ok())
                .map(|cfg| name_key(&cfg.name))
                .collect();
            let mut has_master = existing.iter().any(|r| r.master);
            let mut position = dbops::next_position(c)?;
            for t in &snapshot.targets {
                if existing.len() >= MAX_TARGETS
                    || !uuid::is_valid(&t.id)
                    || existing.iter().any(|r| r.id == t.id)
                    || !names.insert(name_key(&t.config.name))
                {
                    continue;
                }
                let master = t.master && t.enabled && !has_master;
                has_master |= master;
                dbops::insert_target(
                    c,
                    &TargetRow {
                        id: t.id.clone(),
                        position,
                        master,
                        enabled: t.enabled,
                        share_in_master: true,
                        config: seal_config(root, &t.id, &t.config, now)?,
                        seeded: false,
                        conditional: ConditionalWrites::Unknown,
                        last_success_at: None,
                        last_checked_at: None,
                        last_error: None,
                        consecutive_errors: 0,
                        hard_error: false,
                        created_at: now,
                    },
                )?;
                position += 1;
            }
            // This device now holds the snapshot the master holds.
            dbops::set_meta(c, SHARED_TARGETS_META, &B64.encode(&blob))?;
            dbops::delete_meta(c, PENDING_TARGETS_META)?;
            Ok(())
        })
    }

    /// First upload for every enabled target that has not had one (new, imported, re-enabled).
    fn seed_unseeded_targets(&mut self) -> Result<()> {
        let now = now_millis();
        self.store.in_tx(|c| {
            for t in dbops::read_targets(c)? {
                if t.enabled && !t.seeded {
                    dbops::seed_target(c, &t.id, now)?;
                    dbops::set_seeded(c, &t.id, true)?;
                }
            }
            Ok(())
        })
    }

    /// Make sure the master receives the shared-targets snapshot this device holds (a restored
    /// device found it there already; the push then only confirms it).
    fn queue_snapshot_for_master(&mut self) -> Result<()> {
        let now = now_millis();
        let root = &self.root;
        self.store.in_tx(|c| {
            let rows = dbops::read_targets(c)?;
            let Some(master) = rows.iter().find(|r| r.master && r.enabled) else {
                return Ok(());
            };
            if dbops::get_meta(c, SHARED_TARGETS_META)?.is_some() {
                dbops::enqueue_for_target(c, &master.id, JobKind::Targets, "", 0, 0, now)
            } else if rows.iter().any(|r| r.share_in_master) {
                refresh_snapshot(root, c, now)
            } else {
                Ok(())
            }
        })
    }

    // ------------------------------------------------------------------ reading

    fn config_of(&self, row: &TargetRow) -> Result<TargetConfig> {
        open_config(&self.root, &row.id, &row.config)
    }

    fn require_target(&self, id: &str) -> Result<TargetRow> {
        self.store.target_row(id)?.ok_or(StorageError::NotFound)
    }

    pub fn backup_exists(&self, id: &str) -> Result<()> {
        self.require_target(id).map(|_| ())
    }

    fn view_of(&self, row: &TargetRow) -> Result<BackupTargetView> {
        let timing = self.backup_runtime().map(|r| r.timing).unwrap_or_default();
        let stats = self.store.outbox_stats(&row.id)?;
        let mut health = compute_health(row, &stats, now_millis(), &timing);
        let cfg = match self.config_of(row) {
            Ok(c) => c,
            Err(_) => {
                health.state = if row.enabled {
                    HealthState::Failing
                } else {
                    HealthState::Disabled
                };
                health.last_error = Some(MSG_UNREADABLE.into());
                return Ok(BackupTargetView {
                    id: row.id.clone(),
                    name: "Unreadable target".into(),
                    endpoint: None,
                    region: String::new(),
                    bucket: String::new(),
                    prefix: String::new(),
                    access_key_id: String::new(),
                    has_secret: false,
                    path_style: false,
                    enabled: row.enabled,
                    master: row.master,
                    share_in_master: row.share_in_master,
                    health,
                });
            }
        };
        Ok(BackupTargetView {
            id: row.id.clone(),
            name: cfg.name.clone(),
            endpoint: cfg.endpoint.clone(),
            region: cfg.region.clone(),
            bucket: cfg.bucket.clone(),
            prefix: cfg.prefix.clone(),
            access_key_id: cfg.access_key_id.clone(),
            has_secret: cfg.has_secret(),
            path_style: cfg.path_style,
            enabled: row.enabled,
            master: row.master,
            share_in_master: row.share_in_master,
            health,
        })
    }

    /// Every target as the UI shows it: the master first, then in creation order.
    pub fn backup_views(&self) -> Result<Vec<BackupTargetView>> {
        self.store
            .targets_all()?
            .iter()
            .map(|row| self.view_of(row))
            .collect()
    }

    pub fn backup_view(&self, id: &str) -> Result<BackupTargetView> {
        self.view_of(&self.require_target(id)?)
    }

    /// The secret key of a target. Password-gated by the caller.
    pub fn backup_secret(&self, id: &str) -> Result<String> {
        let cfg = self.config_of(&self.require_target(id)?)?;
        if !cfg.has_secret() {
            return Err(unreadable());
        }
        Ok(cfg.secret_access_key.clone())
    }

    // ------------------------------------------------------------------ add / update

    fn taken_names(&self, except: Option<&str>) -> Result<HashSet<String>> {
        let mut names = HashSet::new();
        for row in self.store.targets_all()? {
            if except == Some(row.id.as_str()) {
                continue;
            }
            if let Ok(cfg) = self.config_of(&row) {
                names.insert(name_key(&cfg.name));
            }
        }
        Ok(names)
    }

    /// Validate an add (`existing = None`) or an edit and build what the probe will use. An
    /// empty secret on an edit keeps the stored one.
    pub fn backup_prepare(
        &self,
        existing: Option<&str>,
        input: &BackupTargetInput,
    ) -> Result<Prepared> {
        let config = if let Some(id) = existing {
            let row = self.require_target(id)?;
            let mut cfg = TargetConfig::from_input(input, false)?;
            if cfg.secret_access_key.is_empty() {
                cfg.secret_access_key = self.config_of(&row)?.secret_access_key.clone();
            }
            cfg
        } else {
            if self.store.targets_all()?.len() >= MAX_TARGETS {
                return Err(StorageError::invalid(
                    "That is the most backup targets one vault can have. Remove one first.",
                ));
            }
            TargetConfig::from_input(input, true)?
        };
        if self
            .taken_names(existing)?
            .contains(&name_key(&config.name))
        {
            return Err(StorageError::invalid(
                "Another backup target already has that name.",
            ));
        }
        if input.master && !input.enabled {
            return Err(StorageError::invalid(
                "A disabled target cannot be the master backup. Enable it first.",
            ));
        }
        Ok(Prepared {
            existing: existing.map(str::to_string),
            config,
            enabled: input.enabled,
            master: input.master,
            share_in_master: input.share_in_master,
        })
    }

    /// Save a prepared add or edit together with what the probe found. The target is saved even
    /// if the probe failed (the error is recorded and shown), so a person is never made to
    /// retype settings because the network was down.
    pub fn backup_commit(
        &mut self,
        prepared: Prepared,
        probe: &ProbeReport,
    ) -> Result<BackupTargetView> {
        let now = now_millis();
        let id = match &prepared.existing {
            Some(id) => id.clone(),
            None => uuid::new_v4()?,
        };
        // Names are checked again here: another window may have added one while the probe ran.
        if self
            .taken_names(prepared.existing.as_deref())?
            .contains(&name_key(&prepared.config.name))
        {
            return Err(StorageError::invalid(
                "Another backup target already has that name.",
            ));
        }
        let root = &self.root;
        let p = &prepared;
        let tx_id = id.clone();
        self.store.in_tx(move |c| {
            let id = tx_id;
            let sealed = seal_config(root, &id, &p.config, now)?;
            match &p.existing {
                None => {
                    if dbops::read_targets(c)?.len() >= MAX_TARGETS {
                        return Err(StorageError::invalid(
                            "That is the most backup targets one vault can have. Remove one first.",
                        ));
                    }
                    if p.master {
                        dbops::clear_master(c)?;
                    }
                    dbops::insert_target(
                        c,
                        &TargetRow {
                            id: id.clone(),
                            position: dbops::next_position(c)?,
                            master: p.master,
                            enabled: p.enabled,
                            share_in_master: p.share_in_master,
                            config: sealed,
                            seeded: false,
                            conditional: probe.conditional,
                            last_success_at: None,
                            last_checked_at: None,
                            last_error: None,
                            consecutive_errors: 0,
                            hard_error: false,
                            created_at: now,
                        },
                    )?;
                    if p.enabled {
                        dbops::seed_target(c, &id, now)?;
                        dbops::set_seeded(c, &id, true)?;
                    }
                }
                Some(_) => {
                    let old = dbops::read_target(c, &id)?.ok_or(StorageError::NotFound)?;
                    let old_cfg = open_config(root, &id, &old.config)?;
                    let moved = location_of(&old_cfg) != location_of(&p.config);
                    let conditional = if probe.ok {
                        probe.conditional
                    } else if moved {
                        ConditionalWrites::Unknown
                    } else {
                        old.conditional
                    };
                    dbops::update_target_settings(
                        c,
                        &id,
                        &sealed,
                        p.enabled,
                        p.share_in_master,
                        conditional,
                    )?;
                    dbops::reset_health(c, &id)?;
                    if p.master {
                        dbops::set_master(c, &id)?;
                    } else if old.master {
                        dbops::clear_master(c)?;
                    }
                    if !p.enabled {
                        // Nothing is queued for a disabled target; it is seeded afresh when re-enabled.
                        dbops::clear_queue(c, &id)?;
                        dbops::set_seeded(c, &id, false)?;
                    } else if moved || !old.enabled || !old.seeded {
                        if moved || !old.enabled {
                            dbops::clear_queue(c, &id)?;
                            dbops::clear_remote_objects(c, &id)?;
                        }
                        dbops::seed_target(c, &id, now)?;
                        dbops::set_seeded(c, &id, true)?;
                    }
                }
            }
            refresh_snapshot(root, c, now)
        })?;
        drop(prepared);
        // The probe is the target's first (or newest) contact: record what it found.
        let msg = probe.message.as_str();
        if probe.ok {
            self.store.target_record_success(&id, now)?;
        } else {
            self.store
                .target_record_failure(&id, msg, probe.hard, now)?;
        }
        self.sync_backup();
        self.wake_backup();
        self.backup_view(&id)
    }

    // ------------------------------------------------------------------ remove / master

    pub fn backup_remove(&mut self, id: &str) -> Result<()> {
        let now = now_millis();
        let root = &self.root;
        self.store.in_tx(|c| {
            dbops::delete_target(c, id)?;
            refresh_snapshot(root, c, now)
        })?;
        self.sync_backup();
        self.wake_backup();
        Ok(())
    }

    pub fn backup_set_master(&mut self, id: &str) -> Result<()> {
        let row = self.require_target(id)?;
        if !row.enabled {
            return Err(StorageError::invalid(
                "A disabled target cannot be the master backup. Enable it first.",
            ));
        }
        let now = now_millis();
        let root = &self.root;
        self.store.in_tx(|c| {
            dbops::set_master(c, id)?;
            refresh_snapshot(root, c, now)
        })?;
        self.sync_backup();
        self.wake_backup();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_ids_are_valid_and_distinct() {
        let id = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";
        for s in [
            config_envelope_id(id),
            tombstone_envelope_id(id),
            SNAPSHOT_ID.to_string(),
        ] {
            assert!(crate::crypto::ids::validate_id(&s).is_ok(), "{s}");
        }
        assert_ne!(config_envelope_id(id), tombstone_envelope_id(id));
    }
}
