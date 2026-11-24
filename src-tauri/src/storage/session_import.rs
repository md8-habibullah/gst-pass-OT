//! Import and plaintext-export state of an unlocked session (slice Q4).
//!
//! This file is a child module of `session.rs` (`#[path]` declaration there), so it can reach the
//! private [`Unlocked`] state. Everything here lives **inside** `Unlocked`:
//!
//! * the table of picked files, `handle -> path` (the webview only ever sees the random handle);
//! * the parsed entries of a preview, kept under their handle so that the commit imports exactly
//!   what was previewed.
//!
//! Both vanish with `Unlocked`: locking, switching accounts and quitting all drop it, and the
//! parsed entries wipe their text on drop. A handle also expires 30 minutes after it was made.
//!
//! Parsing and reading files never happen with the session lock held (a key-derivation or a
//! 200 MB file would stall every other command); the lock is taken only to look a handle up,
//! to store a result, and to read or write the vault.

use super::Session;
use crate::crypto::rng;
use crate::importer::entry::{dup_key, ImportedAttachment, ImportedEntry};
use crate::importer::export::{PlainFormat, SecretBuf};
use crate::importer::limits::{cap_chars, warn_title, MAX_WARNINGS};
use crate::importer::{ImportMode, ImportPreview, ParsedImport};
use crate::storage::error::{Result, StorageError};
use crate::storage::files;
use crate::storage::vault::import::{BatchItem, BatchOutcome};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// How long a picked file (and a parsed preview) stays usable.
pub const HANDLE_TTL: Duration = Duration::from_secs(30 * 60);
/// Picked files kept at once (the oldest is forgotten first).
const MAX_FILE_HANDLES: usize = 16;
/// Parsed previews kept at once: they hold plaintext, so only the latest two.
const MAX_PARSED: usize = 2;
/// Titles shown in a preview.
const MAX_PREVIEW_TITLES: usize = 50;
/// Handles are 32 hex characters; anything much longer cannot be one.
const MAX_HANDLE_LEN: usize = 64;

/// A picked file: where it is, and what to call it. The path never leaves the core.
#[derive(Debug, Clone)]
pub struct FileRef {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
}

struct Registered {
    file: FileRef,
    keyfile: bool,
    created: Instant,
}

/// Identifies one way of reading a file: the handle's file, format, column mapping, key file and
/// password, hashed with a per-session salt. A stored preview is reused only for an identical key.
#[derive(Clone)]
pub struct ImportKey(Zeroizing<[u8; 32]>);

impl PartialEq for ImportKey {
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.0[..].ct_eq(&other.0[..]))
    }
}

impl Eq for ImportKey {}

struct Stashed {
    handle: String,
    key: ImportKey,
    parsed: ParsedImport,
    created: Instant,
}

/// The import state inside [`Unlocked`].
#[derive(Default)]
pub struct ImportStash {
    files: HashMap<String, Registered>,
    stashed: Vec<Stashed>,
    salt: Option<Zeroizing<[u8; 16]>>,
    ttl_override: Option<Duration>,
}

fn new_handle() -> Result<String> {
    let mut bytes = [0u8; 16];
    rng::fill(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The error for a handle that was never made, expired, or was cancelled.
fn gone() -> StorageError {
    StorageError::NotFound
}

impl ImportStash {
    fn ttl(&self) -> Duration {
        self.ttl_override.unwrap_or(HANDLE_TTL)
    }

    /// Forget everything past its time.
    fn purge(&mut self) {
        let ttl = self.ttl();
        self.files.retain(|_, r| r.created.elapsed() <= ttl);
        self.stashed.retain(|s| s.created.elapsed() <= ttl);
        let files = &self.files;
        self.stashed.retain(|s| files.contains_key(&s.handle));
    }

    fn register(&mut self, file: FileRef, keyfile: bool) -> Result<String> {
        self.purge();
        while self.files.len() >= MAX_FILE_HANDLES {
            let Some(oldest) = self
                .files
                .iter()
                .min_by_key(|(_, r)| r.created)
                .map(|(h, _)| h.clone())
            else {
                break;
            };
            self.forget(&oldest);
        }
        let handle = new_handle()?;
        self.files.insert(
            handle.clone(),
            Registered {
                file,
                keyfile,
                created: Instant::now(),
            },
        );
        Ok(handle)
    }

    fn forget(&mut self, handle: &str) {
        self.files.remove(handle);
        self.stashed.retain(|s| s.handle != handle);
    }

    fn file(&mut self, handle: &str, keyfile: bool) -> Result<FileRef> {
        self.purge();
        if handle.len() > MAX_HANDLE_LEN {
            return Err(gone());
        }
        match self.files.get(handle) {
            Some(r) if r.keyfile == keyfile => Ok(r.file.clone()),
            _ => Err(gone()),
        }
    }

    fn key(&mut self, parts: &[&[u8]]) -> Result<ImportKey> {
        if self.salt.is_none() {
            let mut salt = Zeroizing::new([0u8; 16]);
            rng::fill(&mut salt[..])?;
            self.salt = Some(salt);
        }
        let mut h = Sha256::new();
        if let Some(salt) = &self.salt {
            h.update(&salt[..]);
        }
        for p in parts {
            h.update((p.len() as u64).to_le_bytes());
            h.update(p);
        }
        let digest = h.finalize();
        let mut out = Zeroizing::new([0u8; 32]);
        out.copy_from_slice(&digest);
        Ok(ImportKey(out))
    }

    fn has(&self, handle: &str, key: &ImportKey) -> bool {
        self.stashed
            .iter()
            .any(|s| s.handle == handle && s.key == *key)
    }

    fn put(&mut self, handle: &str, key: ImportKey, parsed: ParsedImport) {
        self.stashed.retain(|s| s.handle != handle);
        while self.stashed.len() >= MAX_PARSED {
            self.stashed.remove(0);
        }
        self.stashed.push(Stashed {
            handle: handle.to_string(),
            key,
            parsed,
            created: Instant::now(),
        });
    }

    fn parsed(&self, handle: &str, key: &ImportKey) -> Result<&ParsedImport> {
        self.stashed
            .iter()
            .find(|s| s.handle == handle && s.key == *key)
            .map(|s| &s.parsed)
            .ok_or_else(gone)
    }

    fn take(&mut self, handle: &str, key: &ImportKey) -> Result<ParsedImport> {
        let i = self
            .stashed
            .iter()
            .position(|s| s.handle == handle && s.key == *key)
            .ok_or_else(gone)?;
        Ok(self.stashed.remove(i).parsed)
    }
}

/// What a lookup of a picked file found.
pub struct Lookup {
    pub file: FileRef,
    pub keyfile: Option<FileRef>,
    /// A preview of exactly this file, mapping, key file and password is already in memory.
    pub cached: bool,
}

/// What a commit did in the vault, and the attachments that still need a place to go.
pub struct CommitResult {
    pub outcome: BatchOutcome,
    /// `(item id, its attachments)` for items that were saved, when the sink can take them.
    pub attachments: Vec<(String, Vec<ImportedAttachment>)>,
    /// Attachments of saved items that were left out because there is no sink.
    pub attachments_dropped: usize,
}

/// A plaintext export that was written.
pub struct PlainWritten {
    pub path: PathBuf,
    /// Items that could not be decrypted and are not in the file.
    pub unreadable: usize,
}

/// Build the preview numbers for `parsed` against the vault's duplicate index.
fn build_preview(
    parsed: &ParsedImport,
    dups: &HashMap<crate::importer::entry::DupKey, String>,
) -> ImportPreview {
    let mut preview = ImportPreview {
        entries: parsed.entries.len(),
        totp_seeds: parsed.usable_seed_count(),
        attachments: parsed.attachment_count(),
        skipped: parsed.skipped,
        warnings: parsed.warnings.clone(),
        ..ImportPreview::default()
    };
    let mut dropped = 0usize;
    let mut note = |warnings: &mut Vec<String>, w: String| {
        if warnings.len() < MAX_WARNINGS {
            warnings.push(w);
        } else {
            dropped += 1;
        }
    };
    for entry in &parsed.entries {
        let converted = entry.to_draft();
        for w in converted.warnings {
            note(&mut preview.warnings, w);
        }
        let mut draft = converted.draft;
        if preview.titles.len() < MAX_PREVIEW_TITLES {
            preview.titles.push(cap_chars(&draft.title, 256));
        }
        if dups.contains_key(&dup_key(&draft)) {
            preview.duplicates += 1;
        }
        // What the commit would refuse, said now.
        if let Err(e) = draft.normalize() {
            note(
                &mut preview.warnings,
                format!(
                    "\u{201c}{}\u{201d} cannot be imported: {}",
                    warn_title(&draft.title),
                    e
                ),
            );
        }
    }
    if dropped > 0 {
        preview
            .warnings
            .push(format!("{dropped} more notes are not shown."));
    }
    preview
}

impl Session {
    /// The vault must be open: every import and export command needs it.
    pub fn require_unlocked(&self) -> Result<()> {
        self.with_unlocked(|_| Ok(()))
    }

    /// Make the key that identifies one way of reading a file (see [`ImportKey`]).
    pub fn import_key(&self, parts: &[&[u8]]) -> Result<ImportKey> {
        self.with_unlocked(|u| u.imports.key(parts))
    }

    /// Remember a picked file under a new random handle.
    pub fn import_register(&self, file: FileRef, keyfile: bool) -> Result<String> {
        self.with_unlocked(|u| u.imports.register(file, keyfile))
    }

    /// The file a handle stands for.
    pub fn import_file(&self, handle: &str) -> Result<FileRef> {
        self.with_unlocked(|u| u.imports.file(handle, false))
    }

    /// Resolve a file handle (and the optional key file handle), and say whether a preview made
    /// with exactly these options is still in memory.
    pub fn import_lookup(
        &self,
        handle: &str,
        keyfile_handle: Option<&str>,
        key: &ImportKey,
    ) -> Result<Lookup> {
        self.with_unlocked(|u| {
            let file = u.imports.file(handle, false)?;
            let keyfile = match keyfile_handle {
                Some(h) => Some(u.imports.file(h, true)?),
                None => None,
            };
            let cached = u.imports.has(handle, key);
            Ok(Lookup {
                file,
                keyfile,
                cached,
            })
        })
    }

    /// Keep a parsed file under its handle (replacing an earlier one for the same handle).
    pub fn import_stash(&self, handle: &str, key: ImportKey, parsed: ParsedImport) -> Result<()> {
        self.with_unlocked(|u| {
            // The handle may have been cancelled or expired while the file was being parsed.
            u.imports.file(handle, false)?;
            u.imports.put(handle, key, parsed);
            Ok(())
        })
    }

    /// The numbers of a stored preview. Reads the vault; changes nothing.
    pub fn import_preview(&self, handle: &str, key: &ImportKey) -> Result<ImportPreview> {
        self.with_unlocked(|u| {
            u.imports.purge();
            let dups = u.vault.import_dup_index()?;
            let parsed = u.imports.parsed(handle, key)?;
            Ok(build_preview(parsed, &dups))
        })
    }

    /// Save the stored entries in one transaction. The entries are consumed (and wiped) either
    /// way. With `keep_attachments`, the attachments of saved items are handed back for a sink;
    /// without it they are counted.
    pub fn import_commit(
        &self,
        handle: &str,
        key: &ImportKey,
        mode: ImportMode,
        keep_attachments: bool,
    ) -> Result<CommitResult> {
        self.with_unlocked(|u| {
            u.imports.purge();
            let mut parsed = u.imports.take(handle, key)?;
            let mut entries: Vec<ImportedEntry> = std::mem::take(&mut parsed.entries);
            let items = entries.iter().map(|e| {
                let c = e.to_draft();
                BatchItem {
                    draft: c.draft,
                    created: c.created,
                    warnings: c.warnings,
                }
            });
            let mut outcome = u.vault.import_batch(items, mode)?;
            let mut attachments = Vec::new();
            let mut attachments_dropped = 0usize;
            for (entry, id) in entries.iter_mut().zip(&outcome.ids) {
                let Some(id) = id else { continue };
                let taken = std::mem::take(&mut entry.attachments);
                if taken.is_empty() {
                    continue;
                }
                if keep_attachments {
                    attachments.push((id.clone(), taken));
                } else {
                    attachments_dropped += taken.len();
                }
            }
            drop(entries);
            // The notes of the file itself come first, then those of the batch.
            let mut warnings = std::mem::take(&mut parsed.warnings);
            warnings.append(&mut outcome.warnings);
            warnings.truncate(2 * MAX_WARNINGS + 2);
            outcome.warnings = warnings;
            Ok(CommitResult {
                outcome,
                attachments,
                attachments_dropped,
            })
        })
    }

    /// Forget a file handle and any preview made from it. Harmless for an unknown handle, and
    /// for a locked vault (everything is already gone).
    pub fn import_cancel(&self, handle: &str) {
        let _ = self.with_unlocked(|u| {
            u.imports.forget(handle);
            Ok(())
        });
    }

    /// Password-gated plaintext export of every non-trashed item.
    ///
    /// The password is verified first (a wrong one never opens a dialog); then `choose` runs with
    /// no lock held (it shows a native dialog); then the items are decrypted and serialized into
    /// a buffer that wipes itself, which is written to a new private file (mode 0600) through a
    /// temp file, fsync and rename. `Ok(None)` when `choose` returns `None`.
    pub fn export_plain_with(
        &self,
        password: String,
        format: PlainFormat,
        choose: impl FnOnce() -> Result<Option<PathBuf>>,
    ) -> Result<Option<PlainWritten>> {
        self.gated(password, |_| Ok(()), |_| Ok(()))?;
        let Some(dest) = choose()? else {
            return Ok(None);
        };
        let (buf, unreadable): (SecretBuf, usize) = self.with_unlocked(|u| {
            let set = u.vault.export_records()?;
            let buf = format.render(&set.records)?;
            Ok((buf, set.unreadable))
        })?;
        files::write_atomic(&dest, buf.as_slice()).map_err(|_| {
            StorageError::Storage(
                "The export file could not be saved. Check the folder and try again.".into(),
            )
        })?;
        Ok(Some(PlainWritten {
            path: dest,
            unreadable,
        }))
    }
}

#[cfg(test)]
impl Session {
    /// Make every handle (and preview) of this session expire after `ttl`.
    pub(crate) fn test_import_ttl(&self, ttl: Duration) {
        let _ = self.with_unlocked(|u| {
            u.imports.ttl_override = Some(ttl);
            Ok(())
        });
    }

    /// How many file handles and stored previews are alive.
    pub(crate) fn test_import_counts(&self) -> (usize, usize) {
        self.with_unlocked(|u| {
            u.imports.purge();
            Ok((u.imports.files.len(), u.imports.stashed.len()))
        })
        .unwrap_or((0, 0))
    }
}
