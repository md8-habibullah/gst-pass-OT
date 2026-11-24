//! The encrypted drive on an unlocked [`Vault`]: add, list, rename, delete, read, save a copy.
//! Storage half and file formats: `drive.rs`; the streaming crypto: `crypto/stream.rs`.
//!
//! Keys: a file's blob is sealed under `HKDF(root, "attachment", <file uuid>)` (one key per file,
//! `Vault::file_key`); its metadata envelope under `HKDF(root, "meta", "file-<uuid>")`. Neither
//! key ever leaves this module's tickets.
//!
//! Adding a file is three steps so that the (slow) streaming never runs under the session lock:
//! [`Vault::file_add_prepare`] (lock held: checks, new id, key) -> `drive::stage` (no lock:
//! read, encrypt, write a temp file) -> [`Vault::file_add_commit`] (lock held: rename, row,
//! outbox, all-or-nothing). `Vault::file_add_from_path` runs the three in a row for callers that
//! hold the vault anyway (tests, white-box code).

use super::drive::{
    self, check_new_name, damaged, mime_for, AddTicket, FileEntry, FileMeta, FileRow, ReadTicket,
    Staged,
};
use super::error::{Result, StorageError};
use super::model::AttachmentView;
use super::uuid;
use super::vault::Vault;
use crate::crypto::stream;
use crate::crypto::{now_millis, KeyPurpose, SecretKey32};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use zeroize::Zeroizing;

/// The envelope id of a file's metadata (a valid object id: 41 characters).
pub fn meta_object_id(file_id: &str) -> String {
    format!("file-{file_id}")
}

fn check_file_id(id: &str) -> Result<()> {
    if uuid::is_valid(id) {
        Ok(())
    } else {
        Err(StorageError::invalid("That is not a valid file id."))
    }
}

fn next_meta_version(v: u64) -> Result<u64> {
    v.checked_add(1)
        .ok_or_else(|| StorageError::Storage("This file has been renamed too many times.".into()))
}

fn no_such_file() -> StorageError {
    StorageError::NotFound
}

impl Vault {
    // ------------------------------------------------------------------ internals

    fn file_key(&self, id: &str) -> Result<SecretKey32> {
        Ok(self.root.object_key(KeyPurpose::Attachment, id)?)
    }

    fn files_dir(&self) -> Result<PathBuf> {
        drive::files_dir_of(&self.store)
    }

    /// Seal metadata as `version` of `file-<id>`.
    fn seal_file_meta(&self, id: &str, version: u64, meta: &FileMeta, now: u64) -> Result<Vec<u8>> {
        let json = Zeroizing::new(serde_json::to_vec(meta)?);
        Ok(self
            .root
            .seal_object(KeyPurpose::Meta, &meta_object_id(id), version, now, &json)?)
    }

    /// Authenticate and parse a row's metadata. Everything that is not exactly this file's own,
    /// well-formed metadata is `damaged`.
    pub(super) fn open_file_meta(&self, row: &FileRow) -> Result<FileMeta> {
        let opened = self.root.open_object(
            KeyPurpose::Meta,
            &meta_object_id(&row.id),
            Some(row.meta_version),
            &row.meta,
        )?;
        let meta: FileMeta = serde_json::from_slice(&opened.plaintext)?;
        meta.check()?;
        if row.indexed && meta.item_id != row.item_id {
            return Err(damaged());
        }
        if stream::blob_len(meta.size) != row.blob_len {
            return Err(damaged());
        }
        Ok(meta)
    }

    /// The row and its metadata, for an operation on one file. A file whose owner is in the
    /// trash (or gone) is not reachable until the owner is restored.
    fn file_open(&self, id: &str) -> Result<(FileRow, FileMeta)> {
        check_file_id(id)?;
        let row = self.store.file_row(id)?.ok_or_else(no_such_file)?;
        if !row.indexed {
            // restored from a backup and not yet opened: only `files_startup` resolves it
            return Err(damaged());
        }
        if let Some(owner) = &row.item_id {
            self.require_active(owner)?;
        }
        let meta = self.open_file_meta(&row)?;
        Ok((row, meta))
    }

    // ------------------------------------------------------------------ unlock housekeeping

    /// At unlock: open the metadata of rows that a restore inserted without a key (to learn their
    /// owners), and delete blobs no row accounts for. Best effort: nothing here may stop an
    /// unlock, and an unfinished step is simply tried again next time.
    pub(super) fn files_startup(&mut self) {
        if let Ok(rows) = self.store.file_rows_unindexed() {
            for row in rows {
                let opened = self.root.open_object(
                    KeyPurpose::Meta,
                    &meta_object_id(&row.id),
                    Some(row.meta_version),
                    &row.meta,
                );
                let Ok(opened) = opened else { continue };
                let Ok(meta) = serde_json::from_slice::<FileMeta>(&opened.plaintext) else {
                    continue;
                };
                if meta.check().is_err() || stream::blob_len(meta.size) != row.blob_len {
                    continue;
                }
                let _ = self.store.file_set_index(&row.id, meta.item_id.as_deref());
            }
        }
        if let (Ok(dir), Ok(known)) = (self.files_dir(), self.store.file_all_ids()) {
            drive::sweep_orphans(&dir, &known);
        }
    }

    /// After an item's permanent deletion: its blobs go (the rows went with the item).
    pub(super) fn remove_file_blobs(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        if let Ok(dir) = self.files_dir() {
            drive::remove_blobs(&dir, ids);
        }
    }

    // ------------------------------------------------------------------ reading

    /// The attachments of an item as `ItemView` shows them. A file that cannot be opened is left
    /// out (the item stays usable).
    pub(super) fn attachments_of(&self, item_id: &str) -> Vec<AttachmentView> {
        let Ok(rows) = self.store.file_rows(Some(item_id)) else {
            return Vec::new();
        };
        rows.iter()
            .filter_map(|row| {
                let meta = self.open_file_meta(row).ok()?;
                Some(AttachmentView {
                    id: row.id.clone(),
                    mime: meta
                        .mime
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                    name: meta.name.clone(),
                    size: meta.size,
                })
            })
            .collect()
    }

    /// `None` = the standalone Files section; `Some(item)` = that item's attachments (the item
    /// must be active). Newest first. A file whose metadata fails authentication is left out.
    pub fn file_list(&self, item_id: Option<&str>) -> Result<Vec<FileEntry>> {
        if let Some(item) = item_id {
            self.require_active(item)?;
        }
        let mut out = Vec::new();
        for row in self.store.file_rows(item_id)? {
            if let Ok(meta) = self.open_file_meta(&row) {
                out.push(meta.entry(&row.id));
            }
        }
        out.sort_by(|a, b| b.added_at.cmp(&a.added_at).then_with(|| a.id.cmp(&b.id)));
        Ok(out)
    }

    /// Everything needed to decrypt a file outside the vault lock.
    pub fn file_read_ticket(&self, id: &str) -> Result<ReadTicket> {
        let (row, meta) = self.file_open(id)?;
        Ok(ReadTicket {
            key: self.file_key(&row.id)?,
            blob: drive::blob_path(&self.files_dir()?, &row.id),
            blob_len: row.blob_len,
            cancel: Arc::clone(&self.drive.cancel),
            name: meta.name.clone(),
            size: meta.size,
            id: row.id,
        })
    }

    /// The plaintext of a file, for the in-app preview (refused above the size limit).
    pub fn file_read_all(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let ticket = self.file_read_ticket(id)?;
        drive::read_to_memory(&ticket)
    }

    /// Stream-decrypt a file into a new owner-only file at `dest` (replacing what is there once
    /// the whole file has authenticated).
    pub fn file_save_copy_to(&self, id: &str, dest: &Path) -> Result<()> {
        let ticket = self.file_read_ticket(id)?;
        drive::save_to_path(&ticket, dest)
    }

    // ------------------------------------------------------------------ adding

    /// Step 1 of an add (lock held): the owner must be an active item.
    pub fn file_add_prepare(&self, item_id: Option<&str>) -> Result<AddTicket> {
        if let Some(item) = item_id {
            self.require_active(item)?;
        }
        let id = uuid::new_v4()?;
        Ok(AddTicket {
            key: self.file_key(&id)?,
            id,
            item_id: item_id.map(str::to_string),
            dir: self.files_dir()?,
            cancel: Arc::clone(&self.drive.cancel),
        })
    }

    /// Step 3 of an add (lock held): the staged blob becomes a file. Refused when the vault was
    /// locked since the ticket was made, or the owner left the active items meanwhile.
    pub fn file_add_commit(&mut self, mut staged: Staged) -> Result<FileEntry> {
        if !Arc::ptr_eq(&staged.cancel, &self.drive.cancel)
            || self.drive.cancel.load(Ordering::SeqCst)
        {
            return Err(StorageError::Locked);
        }
        if let Some(owner) = &staged.item_id {
            self.require_active(owner)?;
        }
        let now = now_millis();
        let meta = FileMeta::new(
            staged.name.clone(),
            staged.size,
            staged.item_id.clone(),
            now,
        );
        let sealed = self.seal_file_meta(&staged.id, 1, &meta, now)?;
        let blob = drive::publish_blob(&mut staged)?;
        let row = FileRow {
            id: staged.id.clone(),
            item_id: staged.item_id.clone(),
            meta: sealed,
            meta_version: 1,
            blob_len: staged.blob_len,
            created_at: now,
            indexed: true,
        };
        if let Err(e) = self.store.file_insert(&row, now) {
            let _ = std::fs::remove_file(&blob);
            return Err(e);
        }
        self.wake_backup();
        Ok(meta.entry(&row.id))
    }

    /// Encrypt the file at `path` into the drive (all three steps, vault held throughout).
    pub fn file_add_from_path(&mut self, item_id: Option<&str>, path: &Path) -> Result<FileEntry> {
        let ticket = self.file_add_prepare(item_id)?;
        let staged = drive::stage(ticket, path)?;
        self.file_add_commit(staged)
    }

    // ------------------------------------------------------------------ changing

    /// Rename (1 to 200 characters after trimming, no separators or control characters). The
    /// type follows the new extension. Writes the sealed metadata as its next version.
    pub fn file_rename(&mut self, id: &str, name: &str) -> Result<FileEntry> {
        let new_name = check_new_name(name)?;
        let (row, meta) = self.file_open(id)?;
        if new_name == meta.name {
            return Ok(meta.entry(&row.id));
        }
        let now = now_millis();
        let mut next = meta.clone();
        next.mime = mime_for(&new_name).map(str::to_string);
        next.name = new_name;
        next.updated_at = now.max(meta.updated_at);
        let version = next_meta_version(row.meta_version)?;
        let sealed = self.seal_file_meta(&row.id, version, &next, now)?;
        self.store
            .file_update_meta(&row.id, row.meta_version, &sealed, version, now)?;
        self.wake_backup();
        Ok(next.entry(&row.id))
    }

    /// Can this file be deleted at all (it exists, its owner is not in the trash)? Cheap, and it
    /// does not open the metadata, so a file whose metadata is damaged can still be deleted.
    pub fn file_delete_check(&self, id: &str) -> Result<()> {
        check_file_id(id)?;
        let row = self.store.file_row(id)?.ok_or_else(no_such_file)?;
        match &row.item_id {
            Some(owner) => self.require_active(owner),
            None => Ok(()),
        }
    }

    /// Delete a file: the row, the blob and (for every enabled target) the remote objects. Works
    /// on a file whose metadata is damaged too. The master password is checked by the caller.
    pub fn file_delete(&mut self, id: &str) -> Result<()> {
        check_file_id(id)?;
        let row = self.store.file_row(id)?.ok_or_else(no_such_file)?;
        if let Some(owner) = &row.item_id {
            self.require_active(owner)?;
        }
        self.store.file_delete(id, now_millis())?;
        self.remove_file_blobs(&[row.id]);
        self.wake_backup();
        Ok(())
    }

    /// Delete every file added by one call that failed half way (nothing is left of it).
    pub fn file_rollback(&mut self, ids: &[String]) {
        for id in ids {
            let _ = self.file_delete(id);
        }
    }
}
