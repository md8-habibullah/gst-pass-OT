//! The encrypted drive on the app core: the native dialogs (through the picker seam) plus the
//! session's file operations. The webview never sees or sends a path.
//!
//! This file is a child module of `app.rs` (`#[path]`), which is how it can use the core's
//! private fields.

use super::{files, AppCore, ExportedFile, StorageError};
use crate::storage::drive::FileEntry;
use crate::storage::error::Result;
use zeroize::Zeroizing;

const MSG_OUTSIDE: &str = "Choose a place outside the app's own data folder.";

impl AppCore {
    pub fn file_list(&self, item_id: Option<&str>) -> Result<Vec<FileEntry>> {
        self.session()?.file_list(item_id)
    }

    /// Open the native multi-file picker and encrypt the chosen files into the drive. `None` when
    /// the dialog is cancelled (or nothing was chosen).
    pub fn file_add(&self, item_id: Option<&str>) -> Result<Option<Vec<FileEntry>>> {
        let session = self.session()?;
        // A request that cannot succeed must not open a dialog.
        session.file_add_check(item_id)?;
        let Some(paths) = self.picker().open_files()? else {
            return Ok(None);
        };
        if paths.is_empty() {
            return Ok(None);
        }
        if paths.iter().any(|p| files::is_inside(&self.dir, p)) {
            return Err(StorageError::invalid(
                "Choose files outside the app's own data folder.",
            ));
        }
        session.file_add(item_id, &paths).map(Some)
    }

    pub fn file_rename(&self, id: &str, name: &str) -> Result<FileEntry> {
        self.session()?.file_rename(id, name)
    }

    pub fn file_delete(&self, id: &str, password: String) -> Result<()> {
        self.session()?.file_delete(id, password)
    }

    pub fn file_read(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        self.session()?.file_read(id)
    }

    /// Decrypt a file to a place chosen in the native save dialog. Password-gated. `None` when
    /// the dialog is cancelled.
    pub fn file_save_copy(&self, id: &str, password: String) -> Result<Option<ExportedFile>> {
        let session = self.session()?;
        let picker = self.picker();
        let data_dir = self.dir.clone();
        let saved = session.file_save_copy(id, password, move |suggested| {
            let chosen = picker.save_file(suggested)?;
            if let Some(p) = &chosen {
                if files::is_inside(&data_dir, p) {
                    return Err(StorageError::invalid(MSG_OUTSIDE));
                }
            }
            Ok(chosen)
        })?;
        Ok(saved.map(|p| ExportedFile {
            path: p.to_string_lossy().into_owned(),
        }))
    }
}
