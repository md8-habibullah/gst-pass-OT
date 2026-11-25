//! The importer / exporter commands of [`AppCore`] (slice Q4).
//!
//! This file is a child module of `app.rs` (`#[path]` declaration there), so it can reach the
//! picker, the registry and the selected session like the other `AppCore` methods.
//!
//! * The webview never names a path: `import_pick` opens the native dialog itself and hands back
//!   a random handle; the path stays in the session (`session_import.rs`) and dies with it.
//! * Every command needs an unlocked vault. Reading and parsing a file run with **no lock held**
//!   (a KeePass key derivation or a 200 MB file must not stall the other commands); results are
//!   stored in the session under the handle.
//! * `import_preview` reads the vault (to count duplicates) and changes nothing. `import_commit`
//!   imports exactly what the preview showed, in one transaction (`Vault::import_batch`).
//! * `export_plain` writes **plaintext**. It is behind the master-password re-prompt, a native
//!   save dialog and a private (0600) file, and it says so everywhere it can.

use super::AppCore;
use crate::crypto::now_millis;
use crate::importer::export::PlainFormat;
use crate::importer::limits::{
    unsupported, MAX_FILE_BYTES, MAX_KEYFILE_BYTES, MSG_DAMAGED, MSG_KEYFILE_TOO_LARGE,
    MSG_NOT_A_FILE, MSG_TOO_LARGE, MSG_UNREADABLE,
};
use crate::importer::sniff::sniff;
use crate::importer::text;
use crate::importer::{
    parse, AttachmentSink, CsvInspect, ExportedPath, ImportFormat, ImportOptions, ImportOutcome,
    ImportPick, ImportPreview, KeyfilePick, ParseOptions,
};
use crate::storage::error::{Result, StorageError};
use crate::storage::files;
use crate::storage::session::import::{FileRef, ImportKey};
use crate::storage::session::Session;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use zeroize::Zeroizing;

#[cfg(test)]
#[path = "app_import_tests.rs"]
mod tests;

/// The base name of a picked file and its size, after checking that it is a regular file that
/// is not larger than `max`.
fn describe(path: &Path, max: u64, too_large: &'static str) -> Result<(String, u64)> {
    let meta = std::fs::metadata(path).map_err(|_| unsupported(MSG_UNREADABLE))?;
    if !meta.is_file() {
        return Err(unsupported(MSG_NOT_A_FILE));
    }
    if meta.len() > max {
        return Err(unsupported(too_large));
    }
    let name: String = path
        .file_name()
        .map(|n| n.to_string_lossy().chars().take(255).collect())
        .unwrap_or_default();
    Ok((name, meta.len()))
}

/// Read a whole file into memory that is wiped on drop. The buffer is sized up front so it never
/// reallocates (a reallocation would leave an unwiped copy behind).
fn read_limited(path: &Path, max: u64, too_large: &'static str) -> Result<Zeroizing<Vec<u8>>> {
    let (_, size) = describe(path, max, too_large)?;
    let file = std::fs::File::open(path).map_err(|_| unsupported(MSG_UNREADABLE))?;
    let capacity = usize::try_from(size.saturating_add(1)).map_err(|_| unsupported(too_large))?;
    let mut buf = Zeroizing::new(Vec::with_capacity(capacity));
    file.take(max.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|_| unsupported(MSG_UNREADABLE))?;
    if buf.len() as u64 > max {
        return Err(unsupported(too_large));
    }
    Ok(buf)
}

/// "1 attachment was" / "3 attachments were".
fn attachments_were(n: usize) -> String {
    if n == 1 {
        "1 attachment was".to_string()
    } else {
        format!("{n} attachments were")
    }
}

impl AppCore {
    /// Ask for a file to import and remember it under a handle. The format is guessed from the
    /// content, never from the name. `None` when the dialog is cancelled.
    pub fn import_pick(&self) -> Result<Option<ImportPick>> {
        let session = self.session()?;
        session.require_unlocked()?;
        let Some(path) = self.picker().open_import_file()? else {
            return Ok(None);
        };
        let (name, size) = describe(&path, MAX_FILE_BYTES, MSG_TOO_LARGE)?;
        let bytes = read_limited(&path, MAX_FILE_BYTES, MSG_TOO_LARGE)?;
        // A parser inside a library must not be able to take the app down.
        let format = catch_unwind(AssertUnwindSafe(|| sniff(&bytes))).unwrap_or(None);
        drop(bytes);
        let handle = session.import_register(
            FileRef {
                path,
                name: name.clone(),
                size,
            },
            false,
        )?;
        Ok(Some(ImportPick {
            handle,
            file_name: name,
            size,
            format: format.map_or("unknown", ImportFormat::id).to_string(),
            needs_password: format.is_some_and(ImportFormat::needs_password),
            needs_keyfile: format.is_some_and(ImportFormat::needs_keyfile),
        }))
    }

    /// Ask for a KeePass key file. `None` when the dialog is cancelled.
    pub fn import_pick_keyfile(&self) -> Result<Option<KeyfilePick>> {
        let session = self.session()?;
        session.require_unlocked()?;
        let Some(path) = self.picker().open_keyfile()? else {
            return Ok(None);
        };
        let (name, size) = describe(&path, MAX_KEYFILE_BYTES, MSG_KEYFILE_TOO_LARGE)?;
        let handle = session.import_register(
            FileRef {
                path,
                name: name.clone(),
                size,
            },
            true,
        )?;
        Ok(Some(KeyfilePick {
            handle,
            file_name: name,
        }))
    }

    /// Delimiter, headers and the first rows of a CSV file, for the column mapper.
    pub fn inspect_csv_headers(&self, handle: &str) -> Result<CsvInspect> {
        let session = self.session()?;
        let file = session.import_file(handle)?;
        let bytes = read_limited(&file.path, MAX_FILE_BYTES, MSG_TOO_LARGE)?;
        let sample = catch_unwind(AssertUnwindSafe(|| text::inspect(&bytes)))
            .map_err(|_| unsupported(MSG_DAMAGED))??;
        Ok(CsvInspect {
            delimiter: sample.delimiter.to_string(),
            headers: sample.headers,
            sample: sample.sample,
        })
    }

    /// What identifies one way of reading a file; a stored preview is reused only for the same.
    fn import_key(&self, session: &Session, o: &ImportOptions) -> Result<ImportKey> {
        let mapping = serde_json::to_vec(&o.mapping)?;
        session.import_key(&[
            o.handle.as_bytes(),
            o.format.id().as_bytes(),
            &mapping,
            o.keyfile_handle.as_deref().unwrap_or("").as_bytes(),
            o.password.as_deref().unwrap_or("").as_bytes(),
        ])
    }

    /// Make sure the parsed entries of `o` are in the session: read and parse the file (no lock
    /// held) unless an identical preview is already there.
    fn import_ensure_parsed(
        &self,
        session: &Session,
        o: &ImportOptions,
        key: &ImportKey,
    ) -> Result<()> {
        let lookup = session.import_lookup(&o.handle, o.keyfile_handle.as_deref(), key)?;
        if lookup.cached {
            return Ok(());
        }
        let bytes = read_limited(&lookup.file.path, MAX_FILE_BYTES, MSG_TOO_LARGE)?;
        let keyfile = lookup
            .keyfile
            .as_ref()
            .map(|k| read_limited(&k.path, MAX_KEYFILE_BYTES, MSG_KEYFILE_TOO_LARGE))
            .transpose()?;
        let opts = ParseOptions {
            password: o.password.as_deref().filter(|p| !p.is_empty()),
            keyfile: keyfile.as_ref().map(|k| k.as_slice()),
            mapping: o.mapping.as_ref(),
        };
        let parsed = parse(o.format, &bytes, &opts)?;
        drop(keyfile);
        drop(bytes);
        session.import_stash(&o.handle, key.clone(), parsed)
    }

    /// Parse the file and say what an import would do. Changes nothing in the vault.
    pub fn import_preview(&self, options: ImportOptions) -> Result<ImportPreview> {
        let session = self.session()?;
        let key = self.import_key(&session, &options)?;
        self.import_ensure_parsed(&session, &options, &key)?;
        session.import_preview(&options.handle, &key, options.mode)
    }

    /// Import the file. Attachments found in it go into the encrypted drive, encrypted from memory.
    pub fn import_commit(&self, options: ImportOptions) -> Result<ImportOutcome> {
        let session = self.session()?;
        self.import_commit_with_sink(options, &DriveSink { session: &session })
    }

    /// [`import_commit`](Self::import_commit) with a place for attachments (the encrypted drive).
    pub fn import_commit_with_sink(
        &self,
        options: ImportOptions,
        sink: &dyn AttachmentSink,
    ) -> Result<ImportOutcome> {
        let session = self.session()?;
        let key = self.import_key(&session, &options)?;
        self.import_ensure_parsed(&session, &options, &key)?;
        let result =
            session.import_commit(&options.handle, &key, options.mode, sink.available())?;
        let mut outcome = ImportOutcome {
            added: result.outcome.added,
            appended: result.outcome.appended,
            skipped: result.outcome.skipped,
            failed: result.outcome.failed,
            warnings: result.outcome.warnings,
        };
        // The items are saved; attachments go to the sink with no lock held.
        let mut unsaved = 0usize;
        for (item_id, attachments) in result.attachments {
            for a in &attachments {
                if sink.add(&item_id, &a.name, &a.bytes).is_err() {
                    unsaved += 1;
                }
            }
        }
        if unsaved > 0 {
            outcome
                .warnings
                .push(format!("{} not saved.", attachments_were(unsaved)));
        }
        if result.attachments_dropped > 0 {
            outcome.warnings.push(format!(
                "{} not imported (the encrypted drive is not available).",
                attachments_were(result.attachments_dropped)
            ));
        }
        Ok(outcome)
    }

    /// Forget a picked file (and any preview made from it). Harmless for an unknown handle.
    pub fn import_cancel(&self, handle: &str) -> Result<()> {
        if let Ok(session) = self.session() {
            session.import_cancel(handle);
        }
        Ok(())
    }

    /// Export every non-trashed item as a **plaintext** Bitwarden-compatible file.
    ///
    /// `format` is `"bitwarden-json"` or `"csv"`. Needs the master password again (a wrong one is
    /// `wrong_password` and no dialog opens), then asks where to save. The file is new, private
    /// (mode 0600) and written through a temp file, fsync and rename. `None` when the dialog is
    /// cancelled. Anyone who gets the file can read everything in it.
    pub fn export_plain(&self, format: &str, password: String) -> Result<Option<ExportedPath>> {
        let format = PlainFormat::from_wire(format)
            .ok_or_else(|| StorageError::invalid("Choose Bitwarden JSON or CSV."))?;
        let (id, session) = self.active_session().ok_or(StorageError::Locked)?;
        let name = self
            .reg()
            .get(&id)
            .map_or_else(|| "vault".to_string(), |e| e.display_name.clone());
        let base = files::suggested_export_name(&name, now_millis());
        let base = base.strip_suffix(".hexis").unwrap_or(&base);
        let suggested = format!("{base}-plaintext.{}", format.extension());
        let picker = self.picker();
        let data_dir = self.dir.clone();
        let written = session.export_plain_with(password, format, move || {
            let chosen = picker.save_export_file(&suggested)?;
            if let Some(p) = &chosen {
                if files::is_inside(&data_dir, p) {
                    return Err(StorageError::invalid(
                        "Choose a place outside the app's own data folder.",
                    ));
                }
            }
            Ok(chosen)
        })?;
        Ok(written.map(|w| ExportedPath {
            path: w.path.to_string_lossy().into_owned(),
            warnings: {
                let mut out = Vec::new();
                if w.unreadable > 0 {
                    out.push(format!(
                        "{} could not be read and is not in the file.",
                        if w.unreadable == 1 {
                            "1 item".to_string()
                        } else {
                            format!("{} items", w.unreadable)
                        }
                    ));
                }
                if w.passkeys_left_out > 0 {
                    out.push(format!(
                        "{} not in the file. Bitwarden JSON can carry passkeys; CSV cannot.",
                        if w.passkeys_left_out == 1 {
                            "1 passkey is".to_string()
                        } else {
                            format!("{} passkeys are", w.passkeys_left_out)
                        }
                    ));
                }
                out
            },
        }))
    }
}

/// The importer's attachments go to the encrypted drive of the open vault.
struct DriveSink<'a> {
    session: &'a Session,
}

impl AttachmentSink for DriveSink<'_> {
    fn add(&self, item_id: &str, name: &str, bytes: &[u8]) -> Result<()> {
        self.session
            .file_add_bytes(Some(item_id), name, bytes)
            .map(drop)
    }
}
