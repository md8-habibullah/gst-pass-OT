//! Restore: download a backup into a brand-new vault file. Needs no unlocked vault and no
//! password: everything downloaded is already encrypted and is authenticated at the first unlock
//! (the key bundle is public by design).
//!
//! What is checked here, without any key:
//!
//! * the key bundle must exist and parse;
//! * every object whose key has the shape of an item, snapshot or tombstone must carry an
//!   envelope header whose item id (and version, and timestamp for snapshots) matches the key.
//!   A mismatch is an error, not a skip: a damaged backup must not silently restore less;
//! * objects with any other key are ignored (other software may share the folder);
//! * tombstoned items are not restored (their data is stale by definition);
//! * a bucket with more than [`MAX_OBJECTS`] objects is refused (it is not a Passman backup).
//!
//! Downloads are sequential and the inserts run in one transaction on the new, private file, so
//! a failure leaves nothing usable behind (the caller deletes the folder).

use super::dbops;
use super::remote::{keys, ObjectStore, MAX_META_BYTES, MAX_OBJECT_BYTES};
use super::replicator::BackupRuntime;
use super::types::TargetConfig;
use super::vault_ext::tombstone_envelope_id;
use super::PENDING_TARGETS_META;
use crate::crypto::{envelope, now_millis, VaultKeyBundle};
use crate::storage::db::Store;
use crate::storage::error::{Result, StorageError};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::collections::HashSet;

pub const MAX_OBJECTS: usize = 100_000;

const MSG_NO_VAULT: &str =
    "No Passman vault was found at this location (the key bundle is missing). Check the bucket, the folder (prefix) and the keys.";
const MSG_BAD_BUNDLE: &str = "The key bundle at this location is damaged or is not a Passman key bundle, so it cannot be restored.";
const MSG_BAD_OBJECT: &str =
    "An object in this backup is damaged or does not belong where it was found, so the restore was stopped. Nothing was changed on this device.";
const MSG_TOO_MANY: &str =
    "This location holds far more objects than a Passman vault, so it was not restored.";
const MSG_LISTING: &str = "The storage service's object listing did not make progress.";

/// What a restore brought in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreCounts {
    pub items: usize,
    pub versions: usize,
    pub tombstones: usize,
    /// A shared-targets snapshot was found and kept for the first unlock.
    pub shared_targets: bool,
    /// Encrypted-drive files downloaded (blob and metadata), and objects that were skipped
    /// because a part was missing or failed the structural check.
    pub files: usize,
    pub files_skipped: usize,
}

fn damaged() -> StorageError {
    StorageError::Remote(MSG_BAD_OBJECT.into())
}

struct Listing {
    tombstones: Vec<String>,
    items: Vec<String>,
    versions: Vec<(String, u64, u64)>,
    has_targets: bool,
}

fn list_all(remote: &dyn ObjectStore) -> Result<Listing> {
    let mut out = Listing {
        tombstones: Vec::new(),
        items: Vec::new(),
        versions: Vec::new(),
        has_targets: false,
    };
    let mut seen = 0usize;
    let mut token: Option<String> = None;
    loop {
        let page = remote.list("vault/", token.as_deref(), 1000)?;
        for e in page.entries {
            seen += 1;
            if seen > MAX_OBJECTS {
                return Err(StorageError::Remote(MSG_TOO_MANY.into()));
            }
            if let Some(id) = keys::parse_tombstone(&e.key) {
                out.tombstones.push(id.to_string());
            } else if let Some(id) = keys::parse_item(&e.key) {
                out.items.push(id.to_string());
            } else if let Some((id, n, ts)) = keys::parse_version(&e.key) {
                out.versions.push((id.to_string(), n, ts));
            } else if e.key == keys::TARGETS {
                out.has_targets = true;
            }
        }
        match page.next_token {
            Some(next) => {
                if token.as_deref() == Some(next.as_str()) {
                    return Err(StorageError::Remote(MSG_LISTING.into()));
                }
                token = Some(next);
            }
            None => break,
        }
    }
    out.tombstones.sort();
    out.items.sort();
    out.versions.sort();
    Ok(out)
}

/// Download the backup at `cfg` into `store` (a fresh, empty vault database).
pub fn restore_into(
    store: &mut Store,
    cfg: &TargetConfig,
    runtime: &BackupRuntime,
    device_id: &str,
) -> Result<RestoreCounts> {
    let remote = runtime.factory.open(cfg).map_err(StorageError::from)?;
    let remote = remote.as_ref();

    let bundle_obj = remote
        .get(keys::BUNDLE, MAX_META_BYTES)?
        .ok_or_else(|| StorageError::Remote(MSG_NO_VAULT.into()))?;
    let bundle_json = String::from_utf8(bundle_obj.bytes)
        .map_err(|_| StorageError::Remote(MSG_BAD_BUNDLE.into()))?;
    let bundle = VaultKeyBundle::from_json(&bundle_json)
        .map_err(|_| StorageError::Remote(MSG_BAD_BUNDLE.into()))?;
    bundle
        .validate()
        .map_err(|_| StorageError::Remote(MSG_BAD_BUNDLE.into()))?;

    let listing = list_all(remote)?;
    store.create_vault(&bundle_json, device_id, now_millis())?;
    let mut counts = store.restore_batch(|db| {
        let mut counts = RestoreCounts::default();
        let mut dead: HashSet<&str> = HashSet::new();

        for id in &listing.tombstones {
            let Some(obj) = remote.get(&keys::tombstone(id), MAX_META_BYTES)? else {
                continue;
            };
            let h = envelope::peek_header(&obj.bytes).map_err(|_| damaged())?;
            if h.item_id != tombstone_envelope_id(id) {
                return Err(damaged());
            }
            db.restore_tombstone(id, h.timestamp, &obj.bytes)?;
            dead.insert(id.as_str());
            counts.tombstones += 1;
        }

        for id in listing.items.iter().filter(|i| !dead.contains(i.as_str())) {
            let Some(obj) = remote.get(&keys::item(id), MAX_OBJECT_BYTES)? else {
                continue;
            };
            let h = envelope::peek_header(&obj.bytes).map_err(|_| damaged())?;
            if h.item_id != *id || h.version == 0 {
                return Err(damaged());
            }
            db.restore_item(id, h.version, h.timestamp, &obj.bytes)?;
            counts.items += 1;
        }

        for (id, n, ts) in listing
            .versions
            .iter()
            .filter(|(i, _, _)| !dead.contains(i.as_str()))
        {
            let Some(obj) = remote.get(&keys::version(id, *n, *ts), MAX_OBJECT_BYTES)? else {
                continue;
            };
            let h = envelope::peek_header(&obj.bytes).map_err(|_| damaged())?;
            if h.item_id != *id || h.version != *n || h.timestamp != *ts {
                return Err(damaged());
            }
            if db.restore_version(id, *n, *ts, &obj.bytes)? {
                counts.versions += 1;
            }
        }

        if listing.has_targets {
            if let Some(obj) = remote.get(keys::TARGETS, MAX_META_BYTES)? {
                // Opened (and imported) at the first unlock, when the root key exists.
                if envelope::peek_header(&obj.bytes).is_ok_and(|h| h.item_id == "shared-targets") {
                    dbops::set_meta(&db.conn, PENDING_TARGETS_META, &B64.encode(&obj.bytes))?;
                    counts.shared_targets = true;
                }
            }
        }
        Ok(counts)
    })?;
    super::files_sync::restore_files(store, remote, &mut counts)?;
    Ok(counts)
}
