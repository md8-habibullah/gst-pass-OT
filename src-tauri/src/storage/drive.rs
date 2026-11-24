//! The encrypted drive (Files section and item attachments), storage half: the `files` table,
//! the sealed per-file metadata, file names and types, the staging of an encrypted blob.
//!
//! * A file is one **blob** `<profile folder>/attachments/<uuid>.bin` (chunked XChaCha20-Poly1305,
//!   `crypto::stream`) plus one row of `files` whose `meta` column is a sealed envelope
//!   (`file-<uuid>`, `meta` sub-key) holding the name, plaintext size, type, owner and times.
//! * The row, and the replication jobs it causes, commit in **one transaction**; the blob is
//!   written first (temp file, fsync, rename, folder fsync), so a crash leaves at worst an
//!   orphan blob, which the next unlock deletes.
//! * What the database and the disk learn in the clear: how many files exist, the ciphertext
//!   size of each, which item each belongs to (`item_id`), and their creation times. Never a
//!   name, a type or any content.
//!
//! The `Vault` side (keys, access rules, the operations) is `vault_drive.rs`.

use super::backup::dbops::{self, JobKind};
use super::db::Store;
use super::error::{Result, StorageError};
use super::files::{create_private_dir, create_private_file, fsync_dir};
use crate::crypto::stream::{self, StreamError};
use crate::crypto::{CryptoError, SecretKey32};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Folder (inside the profile folder) that holds the encrypted blobs.
pub const FILES_DIR: &str = "attachments";
/// The per-file limit, 52,428,800 bytes (50 MB).
pub const MAX_FILE_BYTES: u64 = stream::MAX_FILE_BYTES;
pub const MAX_FILE_LABEL: &str = "50 MB";
pub const MAX_NAME_CHARS: usize = 200;
/// How many files one `file_add` call accepts.
pub const MAX_FILES_PER_ADD: usize = 100;
/// Sealed metadata format version.
const META_V: u32 = 1;

/// Schema version this migration brings a vault to.
const FILES_SCHEMA: i64 = 4;

// ------------------------------------------------------------------ wire types

/// One file as the webview sees it (`IPC_CONTRACT.md` "Files").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: Option<String>,
    pub item_id: Option<String>,
    pub added_at: u64,
}

/// What is sealed inside a file's metadata envelope. Wiped on drop.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileMeta {
    pub v: u32,
    pub name: String,
    pub size: u64,
    pub mime: Option<String>,
    pub item_id: Option<String>,
    pub added_at: u64,
    pub updated_at: u64,
}

impl FileMeta {
    pub fn new(name: String, size: u64, item_id: Option<String>, now: u64) -> Self {
        Self {
            v: META_V,
            mime: mime_for(&name).map(str::to_string),
            name,
            size,
            item_id,
            added_at: now,
            updated_at: now,
        }
    }

    /// Structural rules that must hold for anything read back from disk or from a backup.
    pub fn check(&self) -> Result<()> {
        let ok = self.v == META_V
            && self.size <= MAX_FILE_BYTES
            && check_new_name(&self.name).is_ok()
            && self.item_id.as_deref().is_none_or(super::uuid::is_valid);
        if ok {
            Ok(())
        } else {
            Err(damaged())
        }
    }

    pub fn entry(&self, id: &str) -> FileEntry {
        FileEntry {
            id: id.to_string(),
            name: self.name.clone(),
            size: self.size,
            mime: self.mime.clone(),
            item_id: self.item_id.clone(),
            added_at: self.added_at,
        }
    }
}

/// One row of `files`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: String,
    /// The owning item, `None` for the standalone Files section. For a row restored from a
    /// backup this stays `None` and `indexed` stays `false` until the first unlock opens the
    /// sealed metadata and reads the owner from it.
    pub item_id: Option<String>,
    /// The sealed metadata envelope (also the body of `vault/attachments/<uuid>.meta`).
    pub meta: Vec<u8>,
    pub meta_version: u64,
    /// Length of the blob on disk.
    pub blob_len: u64,
    pub created_at: u64,
    pub indexed: bool,
}

// ------------------------------------------------------------------ messages

pub const MSG_DAMAGED_FILE: &str =
    "A stored file failed its integrity check: the data is damaged or was modified.";
pub const MSG_MISSING_BLOB: &str = "The encrypted data of this file is missing on this device.";
pub const MSG_READ_SOURCE: &str = "That file could not be read. Check that it still exists.";
pub const MSG_WRITE_FAILED: &str =
    "The encrypted file could not be written. Check the free disk space.";
pub const MSG_SOURCE_CHANGED: &str = "The file changed while it was being encrypted. Try again.";
pub const MSG_NAME_RULE: &str = "Use 1 to 200 characters for the file name.";
pub const MSG_NAME_CHARS: &str = "A file name cannot contain slashes or control characters.";

/// The one error for any stored file that fails authentication or structure checks.
pub fn damaged() -> StorageError {
    StorageError::Crypto(MSG_DAMAGED_FILE.into())
}

impl From<StreamError> for StorageError {
    fn from(e: StreamError) -> Self {
        match e {
            StreamError::Crypto(
                CryptoError::Authentication
                | CryptoError::MalformedEnvelope(_)
                | CryptoError::Unsupported(_),
            ) => damaged(),
            StreamError::Crypto(other) => Self::from(other),
            StreamError::Io(_) => Self::Storage(MSG_WRITE_FAILED.into()),
            StreamError::TooLarge => {
                Self::invalid(format!("That file is larger than {MAX_FILE_LABEL}."))
            }
            StreamError::SourceChanged => Self::Storage(MSG_SOURCE_CHANGED.into()),
            StreamError::Cancelled => Self::Locked,
        }
    }
}

// ------------------------------------------------------------------ names and types

/// The file types the Files section knows by extension (the same table as `src/lib/files.ts`
/// `mimeFor`); anything else has no type. No content sniffing.
pub fn mime_for(name: &str) -> Option<&'static str> {
    let (_, ext) = name.rsplit_once('.')?;
    Some(match ext.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "zip" => "application/zip",
        _ => return None,
    })
}

fn unsafe_name_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '/' | '\\')
        // bidirectional controls and line separators can make a name look like another name
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The rename rule: trimmed, 1 to 200 characters, no path separators or control characters.
pub fn check_new_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_NAME_CHARS || name == "." || name == ".." {
        return Err(StorageError::invalid(MSG_NAME_RULE));
    }
    if name.chars().any(unsafe_name_char) {
        return Err(StorageError::invalid(MSG_NAME_CHARS));
    }
    Ok(name.to_string())
}

/// A file name for a picked path: the last component, with anything unsafe replaced and the
/// length cut to the limit (keeping the extension), so that a valid file on disk is never
/// refused only because of its name.
pub fn name_from_path(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned: String = raw
        .chars()
        .map(|c| if unsafe_name_char(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return "file".to_string();
    }
    if cleaned.chars().count() <= MAX_NAME_CHARS {
        return cleaned.to_string();
    }
    // too long: cut the stem, keep a short extension
    match cleaned.rsplit_once('.') {
        Some((stem, ext)) if ext.chars().count() <= 12 => {
            let keep = MAX_NAME_CHARS - ext.chars().count() - 1;
            let stem: String = stem.chars().take(keep).collect();
            format!("{stem}.{ext}")
        }
        _ => cleaned.chars().take(MAX_NAME_CHARS).collect(),
    }
}

/// Look at a picked file before anything is written: readable, a regular file, within the
/// limit. (`stage` checks again on the open handle; this one lets a whole selection be refused
/// up front.)
pub fn check_source(path: &Path) -> Result<()> {
    let name = name_from_path(path);
    let meta = std::fs::metadata(path).map_err(|_| {
        StorageError::invalid(format!(
            "{name} could not be read. Check that it still exists."
        ))
    })?;
    if !meta.is_file() {
        return Err(StorageError::invalid(format!("{name} is not a file.")));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(too_large_error(&name));
    }
    Ok(())
}

/// `"<name> is larger than 50 MB."` (the wording of the preview backend).
pub fn too_large_error(name: &str) -> StorageError {
    StorageError::invalid(format!("{name} is larger than {MAX_FILE_LABEL}."))
}

// ------------------------------------------------------------------ paths

pub fn blob_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.bin"))
}

pub fn temp_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.bin.tmp"))
}

/// The vault's `attachments` folder: next to the database file.
pub fn files_dir_of(store: &Store) -> Result<PathBuf> {
    store
        .path()
        .parent()
        .map(|p| p.join(FILES_DIR))
        .ok_or_else(|| StorageError::Storage("The vault folder could not be found.".into()))
}

// ------------------------------------------------------------------ cancellation

/// One per unlocked vault. Dropping the vault (lock, profile switch, quit) raises the flag, so a
/// stream that is running without the vault lock stops within a chunk.
pub struct DriveState {
    pub(super) cancel: Arc<AtomicBool>,
}

impl DriveState {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for DriveState {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DriveState {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

// ------------------------------------------------------------------ staging an encrypted blob

/// Everything needed to encrypt one new file without holding the vault: the new id, the per-file
/// key and where the blob goes. The key is wiped when this is dropped.
pub struct AddTicket {
    pub id: String,
    pub item_id: Option<String>,
    pub(super) key: SecretKey32,
    pub(super) dir: PathBuf,
    pub(super) cancel: Arc<AtomicBool>,
}

/// An encrypted blob written to a temporary file, not yet part of the vault. Dropping it deletes
/// the temporary file, so every failure path cleans up without extra code.
pub struct Staged {
    pub id: String,
    pub item_id: Option<String>,
    pub name: String,
    pub size: u64,
    pub blob_len: u64,
    pub(super) tmp: PathBuf,
    pub(super) dir: PathBuf,
    /// The flag of the vault that made the ticket: a commit into another (or a locked) vault is
    /// refused.
    pub(super) cancel: Arc<AtomicBool>,
    armed: bool,
}

impl Staged {
    /// The blob is in place (renamed away): nothing is left to delete.
    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

/// Encrypt the file at `src` into `<dir>/<id>.bin.tmp` (fsynced) with a fixed-size buffer.
///
/// The size limit is checked first (from the open file's own size), then enforced again while
/// streaming; a source that grows or shrinks while it is read is an error. Nothing but the
/// temporary file is created, and it is deleted again on every failure.
pub fn stage(ticket: AddTicket, src: &Path) -> Result<Staged> {
    let name = name_from_path(src);
    let file_id = stream::uuid_bytes(&ticket.id)
        .ok_or_else(|| StorageError::invalid("That is not a valid file id."))?;
    let mut input = std::fs::File::open(src).map_err(|_| {
        StorageError::invalid(format!(
            "{name} could not be read. Check that it still exists."
        ))
    })?;
    let meta = input.metadata().map_err(|_| {
        StorageError::invalid(format!(
            "{name} could not be read. Check that it still exists."
        ))
    })?;
    if !meta.is_file() {
        return Err(StorageError::invalid(format!("{name} is not a file.")));
    }
    let size = meta.len();
    if size > MAX_FILE_BYTES {
        return Err(too_large_error(&name));
    }
    create_private_dir(&ticket.dir)?;
    let tmp = temp_path(&ticket.dir, &ticket.id);
    let _ = std::fs::remove_file(&tmp);
    let out =
        create_private_file(&tmp).map_err(|_| StorageError::Storage(MSG_WRITE_FAILED.into()))?;
    // From here on, dropping `staged` removes the temporary file.
    let mut staged = Staged {
        id: ticket.id.clone(),
        item_id: ticket.item_id.clone(),
        name: name.clone(),
        size,
        blob_len: stream::blob_len(size),
        tmp: tmp.clone(),
        dir: ticket.dir.clone(),
        cancel: Arc::clone(&ticket.cancel),
        armed: true,
    };
    let mut writer = BufWriter::with_capacity(64 * 1024, out);
    let written = stream::encrypt_stream(
        &ticket.key,
        &file_id,
        size,
        &mut input,
        &mut writer,
        &ticket.cancel,
    )
    .map_err(|e| match e {
        StreamError::TooLarge => too_large_error(&name),
        other => StorageError::from(other),
    })?;
    // The file must not have changed size behind our back (a second look at the same handle).
    let after = input
        .metadata()
        .map_err(|_| StorageError::Storage(MSG_SOURCE_CHANGED.into()))?;
    if after.len() != size {
        return Err(StorageError::Storage(MSG_SOURCE_CHANGED.into()));
    }
    let out = writer
        .into_inner()
        .map_err(|_| StorageError::Storage(MSG_WRITE_FAILED.into()))?;
    out.sync_all()
        .map_err(|_| StorageError::Storage(MSG_WRITE_FAILED.into()))?;
    drop(out);
    debug_assert_eq!(written, staged.blob_len);
    staged.blob_len = written;
    Ok(staged)
}

/// [`stage`] for bytes that are already in memory (an attachment found inside an imported file):
/// the same encryption and the same temporary blob, but no source file, so the plaintext never
/// touches the disk. `name` is cleaned like a picked file's name.
pub fn stage_bytes(ticket: AddTicket, name: &str, bytes: &[u8]) -> Result<Staged> {
    let name = name_from_path(Path::new(name));
    let file_id = stream::uuid_bytes(&ticket.id)
        .ok_or_else(|| StorageError::invalid("That is not a valid file id."))?;
    let size = bytes.len() as u64;
    if size > MAX_FILE_BYTES {
        return Err(too_large_error(&name));
    }
    create_private_dir(&ticket.dir)?;
    let tmp = temp_path(&ticket.dir, &ticket.id);
    let _ = std::fs::remove_file(&tmp);
    let out =
        create_private_file(&tmp).map_err(|_| StorageError::Storage(MSG_WRITE_FAILED.into()))?;
    let mut staged = Staged {
        id: ticket.id.clone(),
        item_id: ticket.item_id.clone(),
        name: name.clone(),
        size,
        blob_len: stream::blob_len(size),
        tmp: tmp.clone(),
        dir: ticket.dir.clone(),
        cancel: Arc::clone(&ticket.cancel),
        armed: true,
    };
    let mut writer = BufWriter::with_capacity(64 * 1024, out);
    let mut input = bytes;
    let written = stream::encrypt_stream(
        &ticket.key,
        &file_id,
        size,
        &mut input,
        &mut writer,
        &ticket.cancel,
    )
    .map_err(|e| match e {
        StreamError::TooLarge => too_large_error(&name),
        other => StorageError::from(other),
    })?;
    let out = writer
        .into_inner()
        .map_err(|_| StorageError::Storage(MSG_WRITE_FAILED.into()))?;
    out.sync_all()
        .map_err(|_| StorageError::Storage(MSG_WRITE_FAILED.into()))?;
    drop(out);
    staged.blob_len = written;
    Ok(staged)
}

/// Move the staged blob to its final name and make the rename durable.
pub(super) fn publish_blob(staged: &mut Staged) -> Result<PathBuf> {
    let dest = blob_path(&staged.dir, &staged.id);
    std::fs::rename(&staged.tmp, &dest)
        .map_err(|_| StorageError::Storage(MSG_WRITE_FAILED.into()))?;
    staged.disarm();
    if fsync_dir(&staged.dir).is_err() {
        let _ = std::fs::remove_file(&dest);
        return Err(StorageError::Storage(MSG_WRITE_FAILED.into()));
    }
    Ok(dest)
}

// ------------------------------------------------------------------ reading a blob

/// Everything needed to decrypt one stored file without holding the vault: the per-file key, the
/// blob's place, what the sealed metadata said about it. The key is wiped when this is dropped.
pub struct ReadTicket {
    pub id: String,
    /// The file's name (for the suggested name of a save dialog).
    pub name: String,
    /// Plaintext size, from the authenticated metadata.
    pub size: u64,
    pub(super) key: SecretKey32,
    pub(super) blob: PathBuf,
    pub(super) blob_len: u64,
    pub(super) cancel: Arc<AtomicBool>,
}

/// A writer into a pre-sized, wiped-on-drop buffer that refuses to grow (so no un-wiped copy of
/// the plaintext is left behind by a reallocation).
struct SecretSink {
    buf: zeroize::Zeroizing<Vec<u8>>,
    cap: usize,
}

impl Write for SecretSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buf.len().saturating_add(data.len()) > self.cap {
            return Err(std::io::Error::other("more data than announced"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn open_blob(ticket: &ReadTicket) -> Result<std::fs::File> {
    let file = std::fs::File::open(&ticket.blob).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StorageError::Storage(MSG_MISSING_BLOB.into())
        } else {
            StorageError::Storage(MSG_READ_SOURCE.into())
        }
    })?;
    let len = file
        .metadata()
        .map_err(|_| StorageError::Storage(MSG_READ_SOURCE.into()))?
        .len();
    if len != ticket.blob_len {
        return Err(damaged());
    }
    Ok(file)
}

/// Decrypt the whole file into memory (the preview). Authenticated end to end before anything is
/// returned; the plaintext lives in a wiped-on-drop buffer.
pub fn read_to_memory(ticket: &ReadTicket) -> Result<zeroize::Zeroizing<Vec<u8>>> {
    let file_id = stream::uuid_bytes(&ticket.id).ok_or_else(damaged)?;
    if ticket.size > MAX_FILE_BYTES {
        return Err(StorageError::invalid(format!(
            "That file is larger than {MAX_FILE_LABEL}."
        )));
    }
    let cap = usize::try_from(ticket.size).map_err(|_| damaged())?;
    let mut sink = SecretSink {
        buf: zeroize::Zeroizing::new(Vec::with_capacity(cap)),
        cap,
    };
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, open_blob(ticket)?);
    let n = stream::decrypt_stream(
        &ticket.key,
        &file_id,
        &mut reader,
        &mut sink,
        &ticket.cancel,
    )?;
    if n != ticket.size || sink.buf.len() != cap {
        return Err(damaged());
    }
    Ok(std::mem::take(&mut sink.buf))
}

/// Decrypt straight into a new owner-only file next to `dest` and move it into place when the
/// whole blob has authenticated. On any failure the partial file is removed.
pub fn save_to_path(ticket: &ReadTicket, dest: &Path) -> Result<()> {
    let file_id = stream::uuid_bytes(&ticket.id).ok_or_else(damaged)?;
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut part_name = dest
        .file_name()
        .ok_or_else(|| StorageError::invalid("That is not a place a file can be saved to."))?
        .to_os_string();
    part_name.push(".part");
    let part = dir.join(part_name);
    let input = open_blob(ticket)?;
    let _ = std::fs::remove_file(&part);
    let out = create_private_file(&part)
        .map_err(|_| StorageError::Storage("The file could not be saved there.".into()))?;
    let result = (|| {
        let mut reader = std::io::BufReader::with_capacity(64 * 1024, input);
        let mut writer = BufWriter::with_capacity(64 * 1024, out);
        let n = stream::decrypt_stream(
            &ticket.key,
            &file_id,
            &mut reader,
            &mut writer,
            &ticket.cancel,
        )?;
        if n != ticket.size {
            return Err(damaged());
        }
        let out = writer
            .into_inner()
            .map_err(|_| StorageError::Storage("The file could not be saved there.".into()))?;
        out.sync_all()
            .map_err(|_| StorageError::Storage("The file could not be saved there.".into()))?;
        drop(out);
        std::fs::rename(&part, dest)
            .map_err(|_| StorageError::Storage("The file could not be saved there.".into()))?;
        // The rename is what matters; a folder fsync that fails does not undo the saved copy.
        let _ = fsync_dir(dir);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Delete blobs by file id (best effort; the sweep at the next unlock removes what is left).
pub(super) fn remove_blobs(dir: &Path, ids: &[String]) {
    for id in ids {
        let _ = std::fs::remove_file(blob_path(dir, id));
        let _ = std::fs::remove_file(temp_path(dir, id));
    }
}

/// Delete every blob (and leftover temp file) in `dir` that no row of `files` accounts for.
/// Only regular files with names this module writes are touched.
pub(super) fn sweep_orphans(dir: &Path, known: &HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let orphan = if let Some(stem) = name.strip_suffix(".bin.tmp") {
            super::uuid::is_valid(stem)
        } else if let Some(stem) = name.strip_suffix(".bin") {
            super::uuid::is_valid(stem) && !known.contains(stem)
        } else {
            false
        };
        if orphan {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

// ------------------------------------------------------------------ schema (version 4)

/// Schema v4 (Q3): the `files` table, and the `outbox` kind list extended with the three
/// replication jobs of files (`file` = upload the blob, `file_meta` = upload or replace the
/// sealed metadata, `file_purge` = delete both objects). SQLite cannot change a CHECK
/// constraint, so the queue table is rebuilt (rows, sequence numbers and indexes preserved).
const MIGRATION_FILES: &str = "
CREATE TABLE files (
    id           TEXT PRIMARY KEY,
    item_id      TEXT,
    meta         BLOB NOT NULL,
    meta_version INTEGER NOT NULL CHECK (meta_version >= 1),
    blob_len     INTEGER NOT NULL CHECK (blob_len >= 0),
    created_at   INTEGER NOT NULL,
    indexed      INTEGER NOT NULL DEFAULT 1 CHECK (indexed IN (0, 1))
);
CREATE INDEX files_by_item ON files (item_id);

CREATE TABLE outbox_rebuilt (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id    TEXT NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('bundle', 'tombstone', 'version', 'item', 'purge', 'targets', 'file', 'file_meta', 'file_purge')),
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
INSERT INTO outbox_rebuilt (seq, target_id, kind, ref, version, ts, rank, gen, enqueued_at, attempts, next_try_at, blocked, last_error)
    SELECT seq, target_id, kind, ref, version, ts, rank, gen, enqueued_at, attempts, next_try_at, blocked, last_error FROM outbox;
DROP TABLE outbox;
ALTER TABLE outbox_rebuilt RENAME TO outbox;
CREATE UNIQUE INDEX outbox_unique ON outbox (target_id, kind, ref, version, ts);
CREATE INDEX outbox_order ON outbox (target_id, rank, seq);
";

/// Bring a vault that was at schema `from_version` to the drive schema. Running it on a vault that
/// is already there is a no-op. Called from `Store::migrate` inside its transaction.
pub(super) fn migrate_files_table(conn: &Connection, from_version: i64) -> Result<()> {
    if from_version >= FILES_SCHEMA {
        return Ok(());
    }
    conn.execute_batch(MIGRATION_FILES)?;
    conn.pragma_update(None, "user_version", FILES_SCHEMA)?;
    Ok(())
}

// ------------------------------------------------------------------ SQL

fn to_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

fn from_i64(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

const FILE_COLS: &str = "id, item_id, meta, meta_version, blob_len, created_at, indexed";

fn row_to_file(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<FileRow>> {
    let id: String = r.get(0)?;
    let item_id: Option<String> = r.get(1)?;
    let meta: Vec<u8> = r.get(2)?;
    let meta_version: i64 = r.get(3)?;
    let blob_len: i64 = r.get(4)?;
    let created_at: i64 = r.get(5)?;
    let indexed: i64 = r.get(6)?;
    Ok((|| {
        Ok(FileRow {
            id,
            item_id,
            meta,
            meta_version: from_i64(meta_version)?,
            blob_len: from_i64(blob_len)?,
            created_at: from_i64(created_at)?,
            indexed: indexed == 1,
        })
    })())
}

/// Forget the queued uploads of a file (it is being deleted).
fn forget_pending_file(conn: &Connection, file_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM outbox WHERE ref = ?1 AND kind IN ('file', 'file_meta')",
        [file_id],
    )?;
    Ok(())
}

/// Inside a purge transaction: delete every file row of the erased item and queue the deletion
/// of their remote objects. (The local blobs are removed by the caller after the commit.)
pub(super) fn purge_files_of_item(conn: &Connection, item_id: &str, now: u64) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id FROM files WHERE item_id = ?1")?;
    let ids = stmt
        .query_map([item_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for id in ids {
        conn.execute("DELETE FROM files WHERE id = ?1", [&id])?;
        forget_pending_file(conn, &id)?;
        dbops::enqueue_for_enabled(conn, JobKind::FilePurge, &id, 0, 0, now)?;
    }
    Ok(())
}

/// Queue every file this vault holds for one target (its first upload): the blob and the
/// sealed metadata. Called from `dbops::seed_target`; already-queued jobs are kept.
pub(super) fn seed_files(conn: &Connection, target_id: &str, now: i64) -> Result<()> {
    for kind in [JobKind::FileBlob, JobKind::FileMeta] {
        conn.execute(
            "INSERT OR IGNORE INTO outbox (target_id, kind, ref, version, ts, rank, enqueued_at)
             SELECT ?1, ?2, id, 0, 0, ?3, ?4 FROM files",
            params![target_id, kind.as_str(), kind.rank(), now],
        )?;
    }
    Ok(())
}

impl Store {
    pub fn file_row(&self, id: &str) -> Result<Option<FileRow>> {
        self.conn
            .query_row(
                &format!("SELECT {FILE_COLS} FROM files WHERE id = ?1"),
                [id],
                row_to_file,
            )
            .optional()?
            .transpose()
    }

    /// The rows of one owner: `None` = the standalone section, `Some(item)` = that item's
    /// attachments. Rows whose owner is not known yet (restored, not yet indexed) are not part of
    /// any listing.
    pub fn file_rows(&self, item_id: Option<&str>) -> Result<Vec<FileRow>> {
        let sql = match item_id {
            None => format!(
                "SELECT {FILE_COLS} FROM files WHERE item_id IS NULL AND indexed = 1 ORDER BY created_at DESC, id"
            ),
            Some(_) => format!(
                "SELECT {FILE_COLS} FROM files WHERE item_id = ?1 AND indexed = 1 ORDER BY created_at DESC, id"
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = match item_id {
            None => stmt.query_map([], row_to_file)?.collect::<Vec<_>>(),
            Some(i) => stmt.query_map([i], row_to_file)?.collect::<Vec<_>>(),
        };
        rows.into_iter().map(|r| r?).collect()
    }

    pub fn file_rows_unindexed(&self) -> Result<Vec<FileRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {FILE_COLS} FROM files WHERE indexed = 0 ORDER BY id"
        ))?;
        let rows = stmt.query_map([], row_to_file)?.collect::<Vec<_>>();
        rows.into_iter().map(|r| r?).collect()
    }

    pub fn file_all_ids(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM files")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        Ok(ids)
    }

    pub fn file_ids_of_item(&self, item_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM files WHERE item_id = ?1 ORDER BY id")?;
        let ids = stmt
            .query_map([item_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// Is the upload job of file `id`'s blob still in `target_id`'s queue? (The metadata job of
    /// the same file waits for it, so a name never reaches a target before its data.)
    pub fn file_blob_job_queued(&self, target_id: &str, id: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE target_id = ?1 AND kind = ?2 AND ref = ?3)",
            params![target_id, JobKind::FileBlob.as_str(), id],
            |r| r.get(0),
        )?;
        Ok(n == 1)
    }

    pub fn file_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))?;
        from_i64(n)
    }

    /// Add a file: the row, and the upload of its blob and metadata for every enabled target,
    /// commit together.
    pub fn file_insert(&mut self, row: &FileRow, now: u64) -> Result<()> {
        self.in_tx(|c| {
            c.execute(
                "INSERT INTO files (id, item_id, meta, meta_version, blob_len, created_at, indexed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    row.id,
                    row.item_id,
                    row.meta,
                    to_i64(row.meta_version)?,
                    to_i64(row.blob_len)?,
                    to_i64(row.created_at)?,
                    i64::from(row.indexed)
                ],
            )?;
            dbops::enqueue_for_enabled(c, JobKind::FileBlob, &row.id, 0, 0, now)?;
            dbops::enqueue_for_enabled(c, JobKind::FileMeta, &row.id, 0, 0, now)?;
            Ok(())
        })
    }

    /// Replace the sealed metadata (a rename), only if the stored version is still
    /// `expected_version`; the metadata upload is queued in the same transaction.
    pub fn file_update_meta(
        &mut self,
        id: &str,
        expected_version: u64,
        meta: &[u8],
        new_version: u64,
        now: u64,
    ) -> Result<()> {
        self.in_tx(|c| {
            let changed = c.execute(
                "UPDATE files SET meta = ?2, meta_version = ?3 WHERE id = ?1 AND meta_version = ?4",
                params![id, meta, to_i64(new_version)?, to_i64(expected_version)?],
            )?;
            if changed != 1 {
                let exists = c
                    .query_row("SELECT 1 FROM files WHERE id = ?1", [id], |_| Ok(()))
                    .optional()?
                    .is_some();
                return Err(if exists {
                    StorageError::VersionConflict
                } else {
                    StorageError::NotFound
                });
            }
            dbops::enqueue_for_enabled(c, JobKind::FileMeta, id, 0, 0, now)?;
            Ok(())
        })
    }

    /// Record the owner of a restored row after its sealed metadata was opened.
    pub fn file_set_index(&self, id: &str, item_id: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET item_id = ?2, indexed = 1 WHERE id = ?1",
            params![id, item_id],
        )?;
        Ok(())
    }

    /// Delete a file row, drop its pending uploads and queue the deletion of its remote objects,
    /// in one transaction. `NotFound` if there is no such row.
    pub fn file_delete(&mut self, id: &str, now: u64) -> Result<()> {
        self.in_tx(|c| {
            let gone = c.execute("DELETE FROM files WHERE id = ?1", [id])?;
            if gone != 1 {
                return Err(StorageError::NotFound);
            }
            forget_pending_file(c, id)?;
            dbops::enqueue_for_enabled(c, JobKind::FilePurge, id, 0, 0, now)?;
            Ok(())
        })
    }

    /// Insert a row downloaded by a restore (no outbox work: the object is already remote).
    pub fn file_restore_row(&self, row: &FileRow) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (id, item_id, meta, meta_version, blob_len, created_at, indexed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.id,
                row.item_id,
                row.meta,
                to_i64(row.meta_version)?,
                to_i64(row.blob_len)?,
                to_i64(row.created_at)?,
                i64::from(row.indexed)
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_follow_the_frontend_table() {
        for (name, mime) in [
            ("a.png", "image/png"),
            ("a.JPG", "image/jpeg"),
            ("a.jpeg", "image/jpeg"),
            ("a.gif", "image/gif"),
            ("a.webp", "image/webp"),
            ("a.svg", "image/svg+xml"),
            ("report.PDF", "application/pdf"),
            ("a.txt", "text/plain"),
            ("a.md", "text/markdown"),
            ("a.json", "application/json"),
            ("a.csv", "text/csv"),
            ("a.zip", "application/zip"),
            ("two.dots.zip", "application/zip"),
        ] {
            assert_eq!(mime_for(name), Some(mime), "{name}");
        }
        for name in ["noext", "a.exe", "a.", ".png.bak", "png", ""] {
            assert_eq!(mime_for(name), None, "{name}");
        }
        // a dotfile that is only an extension has an empty stem but still that extension
        assert_eq!(mime_for(".png"), Some("image/png"));
    }

    #[test]
    fn rename_rules() {
        assert_eq!(check_new_name("  passport.pdf  ").unwrap(), "passport.pdf");
        assert_eq!(check_new_name(&"a".repeat(200)).unwrap().len(), 200);
        for bad in [
            "",
            "   ",
            ".",
            "..",
            &"a".repeat(201),
            "a/b",
            "a\\b",
            "tab\there",
            "nul\0",
            "new\nline",
            "bidi\u{202E}fdp.exe",
        ] {
            assert_eq!(
                check_new_name(bad).unwrap_err().code(),
                "invalid_input",
                "{bad:?}"
            );
        }
        // multi-byte characters count as characters, not bytes
        assert!(check_new_name(&"é".repeat(200)).is_ok());
        assert!(check_new_name(&"é".repeat(201)).is_err());
    }

    #[test]
    fn picked_names_are_made_safe_instead_of_refused() {
        assert_eq!(name_from_path(Path::new("/tmp/x/photo.png")), "photo.png");
        assert_eq!(name_from_path(Path::new("/")), "file");
        let long = format!("/tmp/{}.pdf", "n".repeat(300));
        let n = name_from_path(Path::new(&long));
        assert_eq!(n.chars().count(), 200);
        assert!(n.ends_with(".pdf"));
        assert!(check_new_name(&n).is_ok());
        let weird = name_from_path(Path::new("/tmp/we\u{202E}ird\nname"));
        assert!(check_new_name(&weird).is_ok());
    }

    #[test]
    fn stream_errors_map_to_fixed_sentences() {
        let damaged = StorageError::from(StreamError::Crypto(CryptoError::Authentication));
        assert_eq!(damaged.code(), "crypto");
        assert_eq!(damaged.to_string(), MSG_DAMAGED_FILE);
        assert_eq!(
            StorageError::from(StreamError::Cancelled),
            StorageError::Locked
        );
        assert_eq!(
            StorageError::from(StreamError::TooLarge).code(),
            "invalid_input"
        );
        assert_eq!(
            StorageError::from(StreamError::SourceChanged).code(),
            "storage"
        );
        let io = StorageError::from(StreamError::Io(std::io::Error::other("/secret/path")));
        assert!(!io.to_string().contains("secret"));
    }

    const F1: &str = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";
    const F2: &str = "4a2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e60";
    const ITEM: &str = "5b2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e61";

    /// A store with two enabled targets and one disabled.
    fn store_with_targets() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("vault.db")).unwrap();
        for (id, pos, enabled) in [("ta", 1, 1), ("tb", 2, 1), ("td", 3, 0)] {
            s.conn
                .execute(
                    "INSERT INTO targets (id, position, enabled, config, created_at)
                     VALUES (?1, ?2, ?3, x'AA', 1)",
                    params![id, pos, enabled],
                )
                .unwrap();
        }
        (dir, s)
    }

    fn row(id: &str, item: Option<&str>) -> FileRow {
        FileRow {
            id: id.into(),
            item_id: item.map(str::to_string),
            meta: vec![1, 2, 3],
            meta_version: 1,
            blob_len: 99,
            created_at: 7,
            indexed: true,
        }
    }

    /// `(target, kind, ref)` of every queued job, sorted.
    fn queue(s: &Store) -> Vec<(String, String, String)> {
        let mut stmt = s
            .conn
            .prepare("SELECT target_id, kind, ref FROM outbox ORDER BY target_id, kind, ref")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn jobs(targets: &[&str], kinds: &[(&str, &str)]) -> Vec<(String, String, String)> {
        let mut v = Vec::new();
        for t in targets {
            for (k, r) in kinds {
                v.push(((*t).to_string(), (*k).to_string(), (*r).to_string()));
            }
        }
        v.sort();
        v
    }

    #[test]
    fn adding_a_file_queues_blob_and_metadata_for_enabled_targets_in_the_same_commit() {
        let (_d, mut s) = store_with_targets();
        s.file_insert(&row(F1, None), 5).unwrap();
        assert_eq!(
            queue(&s),
            jobs(&["ta", "tb"], &[("file", F1), ("file_meta", F1)])
        );
        assert_eq!(s.file_count().unwrap(), 1);
        assert_eq!(s.file_row(F1).unwrap().unwrap(), row(F1, None));
        // a failed insert (duplicate id) rolls back its queue entries too
        s.file_insert(&row(F1, None), 6).unwrap_err();
        assert_eq!(queue(&s).len(), 4);
        // order of a drain: the blob before the metadata
        assert!(JobKind::FileBlob.rank() < JobKind::FileMeta.rank());
    }

    #[test]
    fn a_rename_is_a_compare_and_swap_that_queues_only_the_metadata() {
        let (_d, mut s) = store_with_targets();
        s.file_insert(&row(F1, Some(ITEM)), 5).unwrap();
        s.conn.execute("DELETE FROM outbox", []).unwrap();
        s.file_update_meta(F1, 1, &[9, 9], 2, 6).unwrap();
        let r = s.file_row(F1).unwrap().unwrap();
        assert_eq!((r.meta, r.meta_version), (vec![9, 9], 2));
        assert_eq!(queue(&s), jobs(&["ta", "tb"], &[("file_meta", F1)]));
        // a stale writer loses, an unknown file is not found
        assert_eq!(
            s.file_update_meta(F1, 1, &[7], 2, 7).unwrap_err(),
            StorageError::VersionConflict
        );
        assert_eq!(
            s.file_update_meta(F2, 1, &[7], 2, 7).unwrap_err(),
            StorageError::NotFound
        );
        assert_eq!(s.file_row(F1).unwrap().unwrap().meta, vec![9, 9]);
    }

    #[test]
    fn deleting_a_file_drops_its_pending_uploads_and_queues_the_remote_deletion() {
        let (_d, mut s) = store_with_targets();
        s.file_insert(&row(F1, None), 5).unwrap();
        s.file_insert(&row(F2, None), 5).unwrap();
        s.file_delete(F1, 6).unwrap();
        let mut want = jobs(&["ta", "tb"], &[("file", F2), ("file_meta", F2)]);
        want.extend(jobs(&["ta", "tb"], &[("file_purge", F1)]));
        want.sort();
        assert_eq!(queue(&s), want);
        assert!(s.file_row(F1).unwrap().is_none());
        assert_eq!(s.file_delete(F1, 7).unwrap_err(), StorageError::NotFound);
        assert_eq!(queue(&s), want, "a failed delete queues nothing");
    }

    #[test]
    fn erasing_an_item_deletes_its_file_rows_and_queues_their_remote_deletion() {
        let (_d, mut s) = store_with_targets();
        s.file_insert(&row(F1, Some(ITEM)), 5).unwrap();
        s.file_insert(&row(F2, None), 5).unwrap();
        s.in_tx(|c| purge_files_of_item(c, ITEM, 9)).unwrap();
        assert!(s.file_row(F1).unwrap().is_none());
        assert!(s.file_row(F2).unwrap().is_some(), "other files stay");
        let q = queue(&s);
        assert!(q.contains(&("ta".into(), "file_purge".into(), F1.into())));
        assert!(!q.iter().any(|(_, k, r)| r == F1 && k != "file_purge"));
        assert_eq!(s.file_ids_of_item(ITEM).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_new_target_is_seeded_with_every_existing_file() {
        let (_d, mut s) = store_with_targets();
        s.file_insert(&row(F1, None), 5).unwrap();
        s.file_insert(&row(F2, Some(ITEM)), 5).unwrap();
        s.conn
            .execute(
                "INSERT INTO targets (id, position, enabled, config, created_at) VALUES ('tn', 9, 1, x'AA', 1)",
                [],
            )
            .unwrap();
        s.in_tx(|c| dbops::seed_target(c, "tn", 10)).unwrap();
        let q: Vec<_> = queue(&s)
            .into_iter()
            .filter(|(t, _, _)| t == "tn")
            .collect();
        for f in [F1, F2] {
            assert!(q.contains(&("tn".into(), "file".into(), f.into())));
            assert!(q.contains(&("tn".into(), "file_meta".into(), f.into())));
        }
        // seeding twice adds nothing
        let n = queue(&s).len();
        s.in_tx(|c| dbops::seed_target(c, "tn", 11)).unwrap();
        assert_eq!(queue(&s).len(), n);
    }

    #[test]
    fn restored_rows_are_hidden_until_indexed() {
        let (_d, s) = store_with_targets();
        let mut r = row(F1, None);
        r.indexed = false;
        s.file_restore_row(&r).unwrap();
        assert!(s.file_rows(None).unwrap().is_empty());
        assert_eq!(s.file_rows_unindexed().unwrap().len(), 1);
        assert_eq!(queue(&s), vec![], "a restore queues no uploads");
        s.file_set_index(F1, Some(ITEM)).unwrap();
        assert!(s.file_rows(None).unwrap().is_empty());
        assert_eq!(s.file_rows(Some(ITEM)).unwrap().len(), 1);
        assert!(s.file_rows_unindexed().unwrap().is_empty());
    }

    #[test]
    fn the_blob_sweep_removes_only_unreferenced_blobs_and_temporaries() {
        let dir = tempfile::tempdir().unwrap();
        let known: HashSet<String> = [F1.to_string()].into();
        let w = |n: &str| std::fs::write(dir.path().join(n), b"x").unwrap();
        w(&format!("{F1}.bin"));
        w(&format!("{F2}.bin"));
        w(&format!("{F1}.bin.tmp"));
        w("readme.txt");
        w("short.bin");
        std::fs::create_dir(dir.path().join(format!("{ITEM}.bin"))).unwrap();
        sweep_orphans(dir.path(), &known);
        let mut left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let mut want = vec![
            format!("{F1}.bin"),
            "readme.txt".to_string(),
            "short.bin".to_string(),
            format!("{ITEM}.bin"),
        ];
        want.sort();
        assert_eq!(
            left, want,
            "a folder with a blob's name is never touched either"
        );
        sweep_orphans(&dir.path().join("missing"), &known); // no folder: nothing happens
    }

    #[test]
    fn meta_checks_reject_nonsense() {
        let good = FileMeta::new("a.png".into(), 5, None, 10);
        assert!(good.check().is_ok());
        assert_eq!(good.mime.as_deref(), Some("image/png"));
        let mut bad = good.clone();
        bad.size = MAX_FILE_BYTES + 1;
        assert!(bad.check().is_err());
        let mut bad = good.clone();
        bad.item_id = Some("not-a-uuid".into());
        assert!(bad.check().is_err());
        let mut bad = good.clone();
        bad.name = "a/b".into();
        assert!(bad.check().is_err());
        let mut bad = good;
        bad.v = 2;
        assert!(bad.check().is_err());
    }
}
