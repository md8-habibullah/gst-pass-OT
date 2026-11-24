//! The encrypted drive on the session: which steps run under the state lock and which do not.
//!
//! Reading, encrypting and decrypting files takes seconds for a large file, so the state lock is
//! held only for the short steps (checks, keys, the commit). The master-password check of
//! delete and save-copy runs with the lock released, like every other re-prompt.
//!
//! This file is a child module of `session.rs` (`#[path]`), which is how it can use the session's
//! private state.

use super::{Session, StorageError};
use crate::storage::drive::{self, FileEntry, MAX_FILES_PER_ADD};
use crate::storage::error::Result;
use std::path::PathBuf;
use zeroize::Zeroizing;

impl Session {
    /// Is a file add for `item_id` possible at all (unlocked, owner active)? Run before a dialog
    /// is shown, so a request that cannot succeed never asks the person to pick files.
    pub fn file_add_check(&self, item_id: Option<&str>) -> Result<()> {
        self.with_unlocked(|u| u.vault.file_add_prepare(item_id).map(drop))
    }

    pub fn file_list(&self, item_id: Option<&str>) -> Result<Vec<FileEntry>> {
        self.with_unlocked(|u| u.vault.file_list(item_id))
    }

    /// Encrypt the picked files into the drive, all or nothing.
    ///
    /// Every size is checked before the first byte is written: one file over the limit rejects
    /// the whole call with `"<name> is larger than 50 MB."`. If a later file then fails for another
    /// reason (a vanished file, a full disk), the files this call already added are removed again
    /// and the error is returned.
    pub fn file_add(&self, item_id: Option<&str>, paths: &[PathBuf]) -> Result<Vec<FileEntry>> {
        if paths.len() > MAX_FILES_PER_ADD {
            return Err(StorageError::invalid(format!(
                "Add at most {MAX_FILES_PER_ADD} files at once."
            )));
        }
        self.file_add_check(item_id)?;
        for path in paths {
            drive::check_source(path)?;
        }
        let mut added: Vec<FileEntry> = Vec::new();
        for path in paths {
            let step = (|| {
                let ticket = self.with_unlocked(|u| u.vault.file_add_prepare(item_id))?;
                // The slow part: no lock is held while the file is read and encrypted.
                let staged = drive::stage(ticket, path)?;
                self.with_unlocked(|u| u.vault.file_add_commit(staged))
            })();
            match step {
                Ok(entry) => added.push(entry),
                Err(e) => {
                    let ids: Vec<String> = added.iter().map(|f| f.id.clone()).collect();
                    // best effort: a locked vault cannot roll back, the files stay and are listed
                    let _ = self.with_unlocked(|u| {
                        u.vault.file_rollback(&ids);
                        Ok(())
                    });
                    return Err(e);
                }
            }
        }
        Ok(added)
    }

    /// Encrypt bytes that are already in memory into the drive (an attachment of an imported
    /// file). Same three phases as [`file_add`](Self::file_add): the vault lock is not held while
    /// the data is encrypted, and nothing is written to disk in the clear.
    pub fn file_add_bytes(
        &self,
        item_id: Option<&str>,
        name: &str,
        bytes: &[u8],
    ) -> Result<FileEntry> {
        self.file_add_check(item_id)?;
        let ticket = self.with_unlocked(|u| u.vault.file_add_prepare(item_id))?;
        let staged = drive::stage_bytes(ticket, name, bytes)?;
        self.with_unlocked(|u| u.vault.file_add_commit(staged))
    }

    pub fn file_rename(&self, id: &str, name: &str) -> Result<FileEntry> {
        self.with_unlocked(|u| u.vault.file_rename(id, name))
    }

    /// Delete a file for good. Master password required.
    pub fn file_delete(&self, id: &str, password: String) -> Result<()> {
        self.gated(
            password,
            |u| u.vault.file_delete_check(id),
            |u| u.vault.file_delete(id),
        )
    }

    /// The decrypted bytes for the preview. The decryption runs without the state lock.
    pub fn file_read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let ticket = self.with_unlocked(|u| u.vault.file_read_ticket(id))?;
        drive::read_to_memory(&ticket)
    }

    /// Save a decrypted copy: master password first (a wrong one never opens a dialog), then
    /// `choose` (the native dialog, no lock held) with the file's name, then the streaming
    /// decryption to the chosen place. `Ok(None)` when the person cancels.
    pub fn file_save_copy(
        &self,
        id: &str,
        password: String,
        choose: impl FnOnce(&str) -> Result<Option<PathBuf>>,
    ) -> Result<Option<PathBuf>> {
        let first = self.with_unlocked(|u| u.vault.file_read_ticket(id))?;
        let name = first.name.clone();
        drop(first);
        self.gated(password, |_| Ok(()), |_| Ok(()))?;
        let Some(dest) = choose(&name)? else {
            return Ok(None);
        };
        // Looked up again: the file may have changed or gone while the dialog was open.
        let ticket = self.with_unlocked(|u| u.vault.file_read_ticket(id))?;
        drive::save_to_path(&ticket, &dest)?;
        Ok(Some(dest))
    }
}
