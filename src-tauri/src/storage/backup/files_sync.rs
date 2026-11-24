//! Replication of the encrypted drive: two objects per file on every backup target.
//!
//! ```text
//! vault/attachments/<uuid>.bin    the sealed blob (PMDF, see crypto/stream.rs). Immutable: written
//!                                 once with `If-None-Match: *`; a 412 means it is already there.
//! vault/attachments/<uuid>.meta   the sealed metadata envelope (`file-<uuid>`, the `meta` sub-key:
//!                                 name, size, type, owner, times). Mutable (a rename): the
//!                                 single-writer rule of `replicator::push_envelope` applies.
//! ```
//!
//! The bucket therefore learns what the disk learns (`STORAGE_AND_SYNC.md`): how many files exist,
//! how large each ciphertext is, when it was uploaded; never a name, a type or content.
//!
//! Jobs (`JobKind::FileBlob`, `FileMeta`, `FilePurge`) are queued in the same transaction as the
//! change, for every enabled target (`drive.rs`), and are executed here from
//! `replicator::execute`. The blob job runs before the metadata job (rank), and a delete removes
//! the metadata first, so a bucket never holds a name that points at nothing for longer than one
//! interrupted job.
//!
//! **Restore** ([`restore_files`], called from `restore.rs`) downloads every complete pair into
//! the new vault folder and inserts its row; owners become known at the first unlock, when the
//! metadata can be opened (`Vault::files_startup`).
//!
//! **Limit, by design:** files added, renamed or deleted on *another* device reach this one only
//! through a restore, not through the unlock-time pull (which covers items). The pull of files is
//! not built.

use super::dbops::{Job, JobKind};
use super::remote::{Condition, ObjectStore, RemoteError, MAX_META_BYTES};
use super::replicator::{blocked, push_envelope, JobResult, RuntimeTarget};
use super::restore::RestoreCounts;
use super::types::ConditionalWrites;
use crate::crypto::envelope;
use crate::crypto::stream::{self, HEADER_LEN, MAX_BLOB_BYTES};
use crate::storage::db::Store;
use crate::storage::drive::{self, FileRow};
use crate::storage::error::{Result, StorageError};
use crate::storage::uuid;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The object keys of the drive, in one place.
pub mod keys {
    pub const PREFIX: &str = "vault/attachments/";

    pub fn blob(id: &str) -> String {
        format!("{PREFIX}{id}.bin")
    }

    pub fn meta(id: &str) -> String {
        format!("{PREFIX}{id}.meta")
    }

    /// `vault/attachments/<uuid>.bin` -> `<uuid>` (and nothing for any other shape).
    pub fn parse_blob(key: &str) -> Option<&str> {
        let id = key.strip_prefix(PREFIX)?.strip_suffix(".bin")?;
        crate::storage::uuid::is_valid(id).then_some(id)
    }

    pub fn parse_meta(key: &str) -> Option<&str> {
        let id = key.strip_prefix(PREFIX)?.strip_suffix(".meta")?;
        crate::storage::uuid::is_valid(id).then_some(id)
    }
}

const MSG_LOCAL: &str = "The vault database could not be read for the backup.";
const MSG_BLOB_MISSING: &str =
    "The encrypted data of a file is missing on this device, so it could not be backed up. Delete the file to clear this message.";
const MSG_BLOB_DIFFERS: &str =
    "This backup already holds a different object under the name of one of your files, so it was not overwritten.";
const MSG_META_WAITS: &str =
    "The name of a file is held back until its encrypted data has been backed up.";
const MSG_TOO_MANY: &str =
    "This location holds far more files than a Passman vault, so they were not restored.";
const MSG_LISTING: &str = "The storage service's object listing did not make progress.";
/// Objects of one kind a restore will look at (the whole bucket is capped at the same number).
const MAX_LISTED: usize = 100_000;

fn blob_file(db: &Store, id: &str) -> Option<PathBuf> {
    let dir = db.path().parent()?.join(drive::FILES_DIR);
    Some(drive::blob_path(&dir, id))
}

/// Run one drive job against one target. Called from `replicator::execute`.
pub(super) fn execute(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &RuntimeTarget,
    job: &Job,
) -> JobResult {
    let id = job.target_ref.as_str();
    if !uuid::is_valid(id) {
        return JobResult::Obsolete;
    }
    match job.kind {
        JobKind::FileBlob => push_blob(db, remote, t, id),
        JobKind::FileMeta => match db.file_row(id) {
            // A name never reaches a target before the data it points at: while this file's blob
            // job is still queued for this target (blocked), the metadata waits too.
            Ok(Some(_)) if db.file_blob_job_queued(&t.id, id).unwrap_or(false) => {
                blocked(MSG_META_WAITS)
            }
            Ok(Some(row)) => {
                push_envelope(db, remote, t, &keys::meta(id), &row.meta, row.meta_version)
            }
            Ok(None) => JobResult::Obsolete,
            Err(_) => JobResult::Fatal(MSG_LOCAL.to_string()),
        },
        JobKind::FilePurge => purge_remote_file(db, remote, t, id),
        _ => JobResult::Obsolete,
    }
}

/// Upload the blob once. Targets with reliable preconditions send `If-None-Match: *` and treat a
/// 412 as "already there"; the others look with a HEAD first.
fn push_blob(db: &Store, remote: &dyn ObjectStore, t: &RuntimeTarget, id: &str) -> JobResult {
    let row = match db.file_row(id) {
        Ok(Some(row)) => row,
        Ok(None) => return JobResult::Obsolete,
        Err(_) => return JobResult::Fatal(MSG_LOCAL.to_string()),
    };
    let Some(path) = blob_file(db, id) else {
        return JobResult::Fatal(MSG_LOCAL.to_string());
    };
    match std::fs::metadata(&path) {
        Ok(m) if m.is_file() && m.len() == row.blob_len => {}
        _ => return blocked(MSG_BLOB_MISSING),
    }
    let key = keys::blob(id);
    if t.conditional == ConditionalWrites::Strong {
        return match remote.put_file(&key, &path, &Condition::IfNoneMatch) {
            Ok(_) => JobResult::Done,
            Err(RemoteError::PreconditionFailed) => same_object(remote, &key, row.blob_len),
            Err(e) => JobResult::Transient(e),
        };
    }
    match remote.head(&key) {
        Ok(Some(_)) => same_object(remote, &key, row.blob_len),
        Ok(None) => match remote.put_file(&key, &path, &Condition::None) {
            Ok(_) => JobResult::Done,
            Err(e) => JobResult::Transient(e),
        },
        Err(e) => JobResult::Transient(e),
    }
}

/// An object of this name is already in the bucket: it is ours when it has our length.
fn same_object(remote: &dyn ObjectStore, key: &str, len: u64) -> JobResult {
    match remote.head(key) {
        Ok(Some(m)) if m.size == len => JobResult::Done,
        Ok(Some(_)) => blocked(MSG_BLOB_DIFFERS),
        // It vanished between the refusal and now: the next trigger writes it again.
        Ok(None) => JobResult::Transient(RemoteError::PreconditionFailed),
        Err(e) => JobResult::Transient(e),
    }
}

/// After a deletion: the metadata first, then the blob (both deletes accept "not there").
fn purge_remote_file(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &RuntimeTarget,
    id: &str,
) -> JobResult {
    for key in [keys::meta(id), keys::blob(id)] {
        if let Err(e) = remote.delete(&key) {
            return JobResult::Transient(e);
        }
    }
    let _ = db.forget_remote_objects(&t.id, &keys::meta(id), &keys::blob(id));
    JobResult::Done
}

// ------------------------------------------------------------------ restore

fn damaged_listing() -> StorageError {
    StorageError::Remote(MSG_LISTING.into())
}

struct Listing {
    metas: BTreeSet<String>,
    blobs: BTreeSet<(String, u64)>,
}

fn list_files(remote: &dyn ObjectStore) -> Result<Listing> {
    let mut out = Listing {
        metas: BTreeSet::new(),
        blobs: BTreeSet::new(),
    };
    let mut seen = 0usize;
    let mut token: Option<String> = None;
    loop {
        let page = remote.list(keys::PREFIX, token.as_deref(), 1000)?;
        for e in page.entries {
            seen += 1;
            if seen > MAX_LISTED {
                return Err(StorageError::Remote(MSG_TOO_MANY.into()));
            }
            if let Some(id) = keys::parse_meta(&e.key) {
                out.metas.insert(id.to_string());
            } else if let Some(id) = keys::parse_blob(&e.key) {
                out.blobs.insert((id.to_string(), e.size));
            }
        }
        match page.next_token {
            Some(next) => {
                if token.as_deref() == Some(next.as_str()) {
                    return Err(damaged_listing());
                }
                token = Some(next);
            }
            None => break,
        }
    }
    Ok(out)
}

/// A write target for a downloaded object that cleans up after itself.
struct TempBlob {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempBlob {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Download the blob of `id` into `dir` and check its structure without any key: the header
/// belongs to this file id, its chunk count is possible and the length matches it exactly.
/// `Ok(None)` = skip (missing, too large, or not a blob of this file); remote errors propagate.
fn download_blob(remote: &dyn ObjectStore, dir: &Path, id: &str) -> Result<Option<u64>> {
    let Some(file_id) = stream::uuid_bytes(id) else {
        return Ok(None);
    };
    let dest = drive::blob_path(dir, id);
    let mut tmp = TempBlob {
        path: drive::temp_path(dir, id),
        armed: true,
    };
    let _ = std::fs::remove_file(&tmp.path);
    let out = crate::storage::files::create_private_file(&tmp.path)
        .map_err(|_| StorageError::Storage(drive::MSG_WRITE_FAILED.into()))?;
    let mut writer = std::io::BufWriter::with_capacity(64 * 1024, out);
    let got = match remote.get_to(&keys::blob(id), &mut writer, MAX_BLOB_BYTES) {
        Ok(Some(n)) => n,
        Ok(None) | Err(RemoteError::TooLarge) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let out = writer
        .into_inner()
        .map_err(|_| StorageError::Storage(drive::MSG_WRITE_FAILED.into()))?;
    out.sync_all()
        .map_err(|_| StorageError::Storage(drive::MSG_WRITE_FAILED.into()))?;
    drop(out);
    // structure, from the first bytes on disk
    let mut head = [0u8; HEADER_LEN];
    let readable = std::fs::File::open(&tmp.path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok();
    if !readable || stream::check_blob_structure(&head, &file_id, got).is_err() {
        return Ok(None);
    }
    std::fs::rename(&tmp.path, &dest)
        .map_err(|_| StorageError::Storage(drive::MSG_WRITE_FAILED.into()))?;
    tmp.armed = false;
    let _ = crate::storage::files::fsync_dir(dir);
    Ok(Some(got))
}

/// Download every complete (`.meta` + `.bin`) pair into the new vault `store`, one file at a time.
///
/// * a pair whose parts do not belong together, whose blob fails the structural check, or whose
///   metadata envelope names another file is **skipped and counted**; the rest still restores;
/// * a network or disk error stops the whole restore, like for items (the caller removes the
///   half-made vault folder);
/// * everything stays sealed: the rows are inserted without an owner (`indexed = 0`) and are
///   opened at the first unlock.
pub fn restore_files(
    store: &Store,
    remote: &dyn ObjectStore,
    counts: &mut RestoreCounts,
) -> Result<()> {
    let listing = list_files(remote)?;
    let blobs: std::collections::BTreeMap<&str, u64> = listing
        .blobs
        .iter()
        .map(|(id, size)| (id.as_str(), *size))
        .collect();
    // blobs without metadata and metadata without a blob cannot be shown: count and skip
    counts.files_skipped += listing
        .metas
        .iter()
        .filter(|id| !blobs.contains_key(id.as_str()))
        .count();
    counts.files_skipped += blobs
        .keys()
        .filter(|id| !listing.metas.contains(**id))
        .count();
    let dir = drive::files_dir_of(store)?;
    let mut dir_ready = false;
    for id in listing
        .metas
        .iter()
        .filter(|id| blobs.contains_key(id.as_str()))
    {
        let Some(meta) = remote.get(&keys::meta(id), MAX_META_BYTES)? else {
            counts.files_skipped += 1;
            continue;
        };
        let header = match envelope::peek_header(&meta.bytes) {
            Ok(h)
                if h.item_id == crate::storage::vault_drive::meta_object_id(id)
                    && h.version >= 1 =>
            {
                h
            }
            _ => {
                counts.files_skipped += 1;
                continue;
            }
        };
        if !dir_ready {
            crate::storage::files::create_private_dir(&dir)?;
            dir_ready = true;
        }
        let Some(blob_len) = download_blob(remote, &dir, id)? else {
            counts.files_skipped += 1;
            continue;
        };
        let row = FileRow {
            id: id.clone(),
            item_id: None,
            meta: meta.bytes,
            meta_version: header.version,
            blob_len,
            created_at: header.timestamp,
            indexed: false,
        };
        if let Err(e) = store.file_restore_row(&row) {
            let _ = std::fs::remove_file(drive::blob_path(&dir, id));
            return Err(e);
        }
        counts.files += 1;
    }
    Ok(())
}
