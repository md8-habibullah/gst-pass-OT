//! File-system helpers for the profile layer: atomic writes, private files, the native file
//! picker seam, and the structural checks done on a vault file before it is adopted.
//!
//! Nothing here touches secrets. A vault file is already fully encrypted; these helpers only
//! move bytes around safely (owner-only permissions, temp file + rename, folder fsync).

use super::db::{APPLICATION_ID, SCHEMA_VERSION};
use super::error::{Result, StorageError};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The two native dialogs the app needs. Production uses the dialog plugin (the webview never
/// supplies a path); tests and the end-to-end server inject fixed paths.
///
/// Both return `Ok(None)` when the person cancels. They may block (a modal dialog), so callers
/// run them on the blocking pool and never while holding a lock.
pub trait FilePicker: Send + Sync {
    /// Ask for an existing vault file to import.
    fn open_vault_file(&self) -> Result<Option<PathBuf>>;
    /// Ask where to save an exported vault file (`suggested_name` pre-fills the dialog).
    fn save_vault_file(&self, suggested_name: &str) -> Result<Option<PathBuf>>;
    /// Ask for one or more files to add to the encrypted drive. Cancelling (or a build without
    /// dialogs) answers `None`.
    fn open_files(&self) -> Result<Option<Vec<PathBuf>>> {
        Ok(None)
    }
    /// Ask where to save a decrypted copy of a drive file (`suggested_name` pre-fills the
    /// dialog). Cancelling (or a build without dialogs) answers `None`.
    fn save_file(&self, _suggested_name: &str) -> Result<Option<PathBuf>> {
        Ok(None)
    }

    // The three dialogs of the importer / exporter (slice Q4). Default methods, so every
    // existing implementation keeps compiling; a picker that does not override them cancels.

    /// Ask for a file to import from another app (any file type; the importer sniffs the
    /// content, never the name).
    fn open_import_file(&self) -> Result<Option<PathBuf>> {
        Ok(None)
    }
    /// Ask for a KeePass key file (any file type).
    fn open_keyfile(&self) -> Result<Option<PathBuf>> {
        Ok(None)
    }
    /// Ask where to save a plaintext export (`suggested_name` pre-fills the dialog).
    fn save_export_file(&self, suggested_name: &str) -> Result<Option<PathBuf>> {
        let _ = suggested_name;
        Ok(None)
    }
}

/// A picker for platforms and builds without dialogs: every request is cancelled.
pub struct NoPicker;

impl FilePicker for NoPicker {
    fn open_vault_file(&self) -> Result<Option<PathBuf>> {
        Ok(None)
    }
    fn save_vault_file(&self, _suggested_name: &str) -> Result<Option<PathBuf>> {
        Ok(None)
    }
}

/// Create a brand-new file that only the owner can read and write (fails if it exists).
pub fn create_private_file(path: &Path) -> std::io::Result<File> {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// Create one folder that only the owner can enter (Unix mode 0700, set at creation, so there is
/// no window with wider access). An existing folder is left as it is; anything else in the way
/// (a file, a link) is an error.
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    match b.create(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(e) => Err(e),
    }
}

/// Flush a folder's metadata so a rename inside it survives a power cut (Unix; no-op elsewhere).
pub fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Replace `path` with `bytes`: write a private temp file next to it, fsync, rename over the
/// target, fsync the folder. A crash leaves either the old file or the new one, never half.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent folder"))?;
    let mut tmp_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?
        .to_os_string();
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        let mut f = create_private_file(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        fsync_dir(dir)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Is `path` inside (or equal to) `dir`? Compared after resolving links where the path exists;
/// for a file that does not exist yet its folder is resolved instead.
pub fn is_inside(dir: &Path, path: &Path) -> bool {
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let resolved = std::fs::canonicalize(path).or_else(|_| {
        let parent = path.parent().unwrap_or(Path::new("."));
        let name = path.file_name().unwrap_or_default();
        std::fs::canonicalize(if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        })
        .map(|p| p.join(name))
    });
    match resolved {
        Ok(p) => p.starts_with(&dir),
        Err(_) => false,
    }
}

fn unsupported(message: &str) -> StorageError {
    StorageError::UnsupportedFile(message.into())
}

/// Largest file the import accepts (a vault is text-sized; this only stops a mistaken pick of
/// something huge from being copied into the data folder).
pub const MAX_IMPORT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Structural check of a file somebody offers as a vault, **without any password** and without
/// letting SQLite parse it yet: it must be a regular file, start with the SQLite magic, and carry
/// Hexis's `application_id` and a schema version this build understands. A file written by a
/// newer Hexis says so instead of calling itself foreign.
pub fn check_vault_header(path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path)
        .map_err(|_| unsupported("That file could not be read. Check that it still exists."))?;
    if !meta.is_file() {
        return Err(unsupported("Choose a vault file, not a folder."));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(unsupported("That file is too large to be a vault."));
    }
    let mut head = [0u8; 100];
    let mut f = File::open(path)
        .map_err(|_| unsupported("That file could not be read. Check that it still exists."))?;
    if f.read_exact(&mut head).is_err() || &head[..16] != b"SQLite format 3\0" {
        return Err(unsupported(
            "That file is not a vault file (it is not a database at all).",
        ));
    }
    let user_version = i64::from(i32::from_be_bytes([head[60], head[61], head[62], head[63]]));
    let app_id = i64::from(i32::from_be_bytes([head[68], head[69], head[70], head[71]]));
    if app_id != APPLICATION_ID {
        return Err(unsupported(
            "That file is a database, but not a vault this app can open.",
        ));
    }
    if user_version > SCHEMA_VERSION {
        return Err(unsupported(
            "That vault was saved by a newer version of this app. Update the app to import it.",
        ));
    }
    if user_version < 1 {
        return Err(unsupported("That vault file is empty or incomplete."));
    }
    Ok(())
}

/// Copy `src` to a new private file `dest` (which must not exist).
pub fn copy_private(src: &Path, dest: &Path) -> Result<()> {
    let mut input = File::open(src)
        .map_err(|_| unsupported("That file could not be read. Check that it still exists."))?;
    let mut out = create_private_file(dest)?;
    std::io::copy(&mut input, &mut out)?;
    out.sync_all()?;
    Ok(())
}

// ------------------------------------------------------------------ export file naming

/// UTC calendar date of a Unix time in milliseconds (civil-from-days, proleptic Gregorian).
pub fn ymd_utc(ms: u64) -> (i64, u32, u32) {
    let days = (ms / 86_400_000) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `<display name>-<yyyy-mm-dd>.hexis`, with characters that file systems dislike replaced.
pub fn suggested_export_name(display_name: &str, now_ms: u64) -> String {
    let cleaned: String = display_name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.');
    let base = if cleaned.is_empty() { "vault" } else { cleaned };
    let (y, m, d) = ymd_utc(now_ms);
    format!("{base}-{y:04}-{m:02}-{d:02}.hexis")
}

/// Test double for the native dialogs: hands back whatever path a test set (or "cancelled"), and
/// remembers the file names the app suggested.
#[cfg(test)]
#[derive(Default)]
pub struct TestPicker {
    open: std::sync::Mutex<Option<PathBuf>>,
    save: std::sync::Mutex<Option<PathBuf>>,
    suggested: std::sync::Mutex<Vec<String>>,
    /// Canned answers of `open_files`, one per call (an empty queue means "cancelled").
    files: std::sync::Mutex<std::collections::VecDeque<Vec<PathBuf>>>,
    imports: std::sync::Mutex<std::collections::VecDeque<PathBuf>>,
    keyfiles: std::sync::Mutex<std::collections::VecDeque<PathBuf>>,
    export_save: std::sync::Mutex<Option<PathBuf>>,
    export_names: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl TestPicker {
    pub fn set_open(&self, p: Option<PathBuf>) {
        *self.open.lock().unwrap() = p;
    }
    pub fn set_save(&self, p: Option<PathBuf>) {
        *self.save.lock().unwrap() = p;
    }
    pub fn suggested_names(&self) -> Vec<String> {
        self.suggested.lock().unwrap().clone()
    }
    /// The next `open_files` call picks exactly these files.
    pub fn queue_files(&self, paths: Vec<PathBuf>) {
        self.files.lock().unwrap().push_back(paths);
    }
    /// The next `open_import_file` returns this path (queued; an empty queue means "cancelled").
    pub fn queue_import(&self, p: PathBuf) {
        self.imports.lock().unwrap().push_back(p);
    }
    /// The next `open_keyfile` returns this path.
    pub fn queue_keyfile(&self, p: PathBuf) {
        self.keyfiles.lock().unwrap().push_back(p);
    }
    /// Where `save_export_file` saves (`None`: the person cancels).
    pub fn set_export(&self, p: Option<PathBuf>) {
        *self.export_save.lock().unwrap() = p;
    }
    pub fn export_names(&self) -> Vec<String> {
        self.export_names.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl FilePicker for TestPicker {
    fn open_vault_file(&self) -> Result<Option<PathBuf>> {
        Ok(self.open.lock().unwrap().clone())
    }
    fn save_vault_file(&self, suggested_name: &str) -> Result<Option<PathBuf>> {
        self.suggested
            .lock()
            .unwrap()
            .push(suggested_name.to_string());
        Ok(self.save.lock().unwrap().clone())
    }
    fn open_files(&self) -> Result<Option<Vec<PathBuf>>> {
        Ok(self.files.lock().unwrap().pop_front())
    }
    /// Shares the destination (`set_save`) and the list of suggested names with the vault export.
    fn save_file(&self, suggested_name: &str) -> Result<Option<PathBuf>> {
        self.save_vault_file(suggested_name)
    }
    fn open_import_file(&self) -> Result<Option<PathBuf>> {
        Ok(self.imports.lock().unwrap().pop_front())
    }
    fn open_keyfile(&self) -> Result<Option<PathBuf>> {
        Ok(self.keyfiles.lock().unwrap().pop_front())
    }
    fn save_export_file(&self, suggested_name: &str) -> Result<Option<PathBuf>> {
        self.export_names
            .lock()
            .unwrap()
            .push(suggested_name.to_string());
        Ok(self.export_save.lock().unwrap().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_correct_around_known_instants() {
        assert_eq!(ymd_utc(0), (1970, 1, 1));
        assert_eq!(ymd_utc(951_782_400_000), (2000, 2, 29)); // leap day
        assert_eq!(ymd_utc(1_732_320_000_000), (2024, 11, 23));
        assert_eq!(ymd_utc(1_735_689_599_999), (2024, 12, 31));
        assert_eq!(ymd_utc(1_735_689_600_000), (2025, 1, 1));
    }

    #[test]
    fn export_names_are_safe() {
        assert_eq!(
            suggested_export_name("Work", 1_732_320_000_000),
            "Work-2024-11-23.hexis"
        );
        assert_eq!(
            suggested_export_name("a/b\\c:d*e", 0),
            "a_b_c_d_e-1970-01-01.hexis"
        );
        assert_eq!(suggested_export_name("  ..  ", 0), "vault-1970-01-01.hexis");
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.json"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn inside_check_resolves_links_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        assert!(is_inside(dir.path(), &dir.path().join("x.hexis")));
        assert!(!is_inside(dir.path(), &other.path().join("x.hexis")));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path(), other.path().join("link")).unwrap();
            assert!(is_inside(dir.path(), &other.path().join("link/z.hexis")));
        }
    }

    #[test]
    fn header_checks_give_specific_messages() {
        let dir = tempfile::tempdir().unwrap();
        let msg = |p: &Path| check_vault_header(p).unwrap_err().to_string();
        assert!(msg(&dir.path().join("missing")).contains("could not be read"));
        assert!(msg(dir.path()).contains("not a folder"));
        let garbage = dir.path().join("g");
        std::fs::write(&garbage, b"hello").unwrap();
        assert!(msg(&garbage).contains("not a database"));
        let mut head = [0u8; 100];
        head[..16].copy_from_slice(b"SQLite format 3\0");
        let foreign = dir.path().join("f");
        std::fs::write(&foreign, head).unwrap();
        assert!(msg(&foreign).contains("not a vault"));
        let mut newer = head;
        newer[68..72].copy_from_slice(&(APPLICATION_ID as i32).to_be_bytes());
        newer[60..64].copy_from_slice(&((SCHEMA_VERSION + 1) as i32).to_be_bytes());
        let p = dir.path().join("n");
        std::fs::write(&p, newer).unwrap();
        assert!(msg(&p).contains("newer version"));
        let mut ok = newer;
        ok[60..64].copy_from_slice(&(SCHEMA_VERSION as i32).to_be_bytes());
        let p = dir.path().join("ok");
        std::fs::write(&p, ok).unwrap();
        assert!(check_vault_header(&p).is_ok());
    }
}
