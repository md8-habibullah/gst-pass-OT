//! `secure_wipe_all_local_data`: the panic, the 15th failure and the manual reset.
//!
//! What it does, in this order (`FEATURE_QUEUE.md` Q1, decision D16):
//!
//! 0. **Pre-flight**, with nothing touched: the folder must be absolute, must not be `/`, a drive
//!    root, a user's home or a parent of it, none of the known names may be a link or anything but
//!    a plain file or folder, and when the app believes it holds data the folder must look like
//!    a Passman data folder.
//! 1. **Cryptographic erase first.** For every vault file the key bundle row is overwritten with
//!    zero bytes, the WAL is checkpointed with `TRUNCATE` and the file is fsynced. From that moment
//!    nothing in the vault can be opened again (there is no key material to unlock), even if
//!    everything after this is interrupted.
//! 2. Every known file is **overwritten with zeros** in chunks, fsynced, truncated and unlinked.
//! 3. The emptied known folders are removed (a folder that still holds unknown files stays).
//!
//! It deletes **only known Passman paths** under the folder it is given; it never calls
//! `remove_dir_all`; files it does not know are left alone and listed in the report. It is
//! idempotent and keeps going after an error on one file. Remote S3 copies are never touched.
//!
//! Honest limit: on SSDs, flash and copy-on-write file systems an overwrite cannot be shown to
//! reach the old physical blocks. That is why step 1 exists: the vault is already unreadable.
//!
//! The function takes the folder from its caller (`AppCore`); it contains no path of its own.

use crate::storage::files;
use crate::storage::profiles::{REGISTRY_FILE, VAULTS_DIR, VAULT_FILE};
use crate::storage::uuid;
use rusqlite::{Connection, OpenFlags};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Files of the data folder itself that belong to Passman (exact names; each may also exist with
/// the temp suffix the atomic writer uses).
const TOP_FILES: [&str; 4] = [
    REGISTRY_FILE,
    "gate.json",
    "app-settings.json",
    "disguise.json",
];
/// `<name>.corrupt-<ms>` copies a damaged registry or gate file was moved to.
const QUARANTINE_BASES: [&str; 2] = [REGISTRY_FILE, "gate.json"];
const LEGACY_SUFFIXES: [&str; 4] = ["", "-wal", "-shm", "-journal"];
const ATTACHMENTS_DIR: &str = "attachments";
static ZEROS: [u8; 64 * 1024] = [0u8; 64 * 1024];

/// What the caller knows that the folder alone cannot say.
#[derive(Debug, Clone, Copy, Default)]
pub struct WipePlan {
    /// The registry lists the adopted single-vault-era `vault.db` in the data folder itself.
    pub legacy_registered: bool,
    /// The app believes it holds data (accounts, or the gate is on). Then a folder with no
    /// Passman marker at all is refused instead of treated as already clean.
    pub expect_data: bool,
}

/// One observable step, for the tests (production passes [`NoObserver`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WipeStep {
    KeyBundleDestroyed(PathBuf),
    Overwritten(PathBuf),
    Unlinked(PathBuf),
    DirRemoved(PathBuf),
}

/// Sees every step before the next one starts. Returning `false` stops the wipe right there (a
/// test's way of simulating a crash or a power cut between two steps).
pub trait WipeObserver {
    fn step(&mut self, step: &WipeStep) -> bool;
}

pub struct NoObserver;

impl WipeObserver for NoObserver {
    fn step(&mut self, _step: &WipeStep) -> bool {
        true
    }
}

/// Why nothing was touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    NotAbsolute,
    TooBroad,
    /// A known Passman name is a link, or not a plain file or folder.
    UnsafeName(PathBuf),
    /// The app holds data but the folder shows no sign of Passman.
    NotPassmanData,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WipeError {
    /// Relative to the data folder.
    pub path: PathBuf,
    pub what: &'static str,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WipeReport {
    pub files_removed: usize,
    pub dirs_removed: usize,
    /// Unknown files and folders that were left alone (relative to the data folder).
    pub unknown: Vec<PathBuf>,
    pub errors: Vec<WipeError>,
    /// An observer stopped the wipe early.
    pub aborted: bool,
    /// There was nothing of Passman's in the folder.
    pub nothing_to_do: bool,
}

// ------------------------------------------------------------------ pre-flight

/// Too broad to ever be an app data folder: the root, a drive root, a user's home or a parent of
/// it, or anything with fewer than two path components (`/tmp`, `/home`, `/etc`).
pub fn too_broad(canon: &Path, home: Option<&Path>) -> bool {
    use std::path::Component;
    if canon.parent().is_none() {
        return true;
    }
    let normals = canon
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if normals < 2 {
        return true;
    }
    home.is_some_and(|h| h == canon || h.starts_with(canon))
}

fn home_dirs() -> Vec<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|v| !v.is_empty())
        .map(|v| std::fs::canonicalize(&v).unwrap_or_else(|_| PathBuf::from(v)))
        .collect()
}

fn is_top_file(name: &str) -> bool {
    let base = name.strip_suffix(".tmp").unwrap_or(name);
    if TOP_FILES.contains(&base) {
        return true;
    }
    QUARANTINE_BASES.iter().any(|b| {
        name.strip_prefix(b)
            .and_then(|rest| rest.strip_prefix(".corrupt-"))
            .is_some_and(|ms| !ms.is_empty() && ms.bytes().all(|c| c.is_ascii_digit()))
    })
}

fn is_vault_file_name(name: &str) -> bool {
    [
        VAULT_FILE.to_string(),
        format!("{VAULT_FILE}-wal"),
        format!("{VAULT_FILE}-shm"),
        format!("{VAULT_FILE}-journal"),
        format!("{VAULT_FILE}.part"),
        format!("{VAULT_FILE}.tmp"),
    ]
    .iter()
    .any(|k| k == name)
}

enum Kind {
    File,
    Dir,
    Other,
}

fn kind_of(path: &Path) -> Result<Kind, Refused> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| Refused::Unreadable)?;
    let t = meta.file_type();
    if t.is_symlink() {
        return Err(Refused::UnsafeName(path.to_path_buf()));
    }
    Ok(if t.is_file() {
        Kind::File
    } else if t.is_dir() {
        Kind::Dir
    } else {
        Kind::Other
    })
}

/// A known name that must be a plain file.
fn plain_file(path: &Path) -> Result<(), Refused> {
    match kind_of(path)? {
        Kind::File => Ok(()),
        _ => Err(Refused::UnsafeName(path.to_path_buf())),
    }
}

#[derive(Default)]
struct Layout {
    /// Vault databases (their key bundle is destroyed first).
    vault_dbs: Vec<PathBuf>,
    /// Every known file, in the order they will be wiped: vault files first.
    files: Vec<PathBuf>,
    /// Known folders, deepest first.
    dirs: Vec<PathBuf>,
    unknown: Vec<PathBuf>,
    marker: bool,
}

fn rel(root: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(root).unwrap_or(p).to_path_buf()
}

fn read_names(dir: &Path) -> Result<Vec<(String, PathBuf)>, Refused> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|_| Refused::Unreadable)? {
        let entry = entry.map_err(|_| Refused::Unreadable)?;
        // A name that is not UTF-8 cannot be one of ours.
        let name = entry.file_name().to_string_lossy().into_owned();
        out.push((name, entry.path()));
    }
    out.sort();
    Ok(out)
}

fn scan_vault_dir(root: &Path, dir: &Path, l: &mut Layout) -> Result<(), Refused> {
    let mut vault_files = Vec::new();
    let mut attachments = None;
    for (name, path) in read_names(dir)? {
        if name == ATTACHMENTS_DIR {
            match kind_of(&path)? {
                Kind::Dir => attachments = Some(path),
                _ => l.unknown.push(rel(root, &path)),
            }
        } else if is_vault_file_name(&name) {
            plain_file(&path)?;
            if name == VAULT_FILE {
                l.vault_dbs.push(path.clone());
            }
            vault_files.push(path);
        } else {
            l.unknown.push(rel(root, &path));
        }
    }
    // the database first, then its journal files
    vault_files.sort_by_key(|p| p.file_name() != Some(std::ffi::OsStr::new(VAULT_FILE)));
    l.files.extend(vault_files);
    if let Some(att) = attachments {
        for (_, path) in read_names(&att)? {
            match kind_of(&path)? {
                Kind::File => l.files.push(path),
                _ => l.unknown.push(rel(root, &path)),
            }
        }
        l.dirs.push(att);
    }
    l.dirs.push(dir.to_path_buf());
    Ok(())
}

fn scan(root: &Path, plan: &WipePlan) -> Result<Layout, Refused> {
    let mut l = Layout::default();
    let mut others = Vec::new();
    let legacy_db = root.join(VAULT_FILE);
    let legacy_known = plan.legacy_registered
        || (std::fs::symlink_metadata(&legacy_db).is_ok_and(|m| m.file_type().is_file())
            && files::check_vault_header(&legacy_db).is_ok());
    let mut legacy_files = Vec::new();
    for (name, path) in read_names(root)? {
        if name == VAULTS_DIR {
            match kind_of(&path)? {
                Kind::Dir => {
                    l.marker = true;
                    let mut vault_dirs = Vec::new();
                    for (vname, vpath) in read_names(&path)? {
                        if uuid::is_valid(&vname) {
                            match kind_of(&vpath)? {
                                Kind::Dir => vault_dirs.push(vpath),
                                _ => l.unknown.push(rel(root, &vpath)),
                            }
                        } else {
                            l.unknown.push(rel(root, &vpath));
                        }
                    }
                    for v in vault_dirs {
                        scan_vault_dir(root, &v, &mut l)?;
                    }
                    l.dirs.push(path);
                }
                _ => l.unknown.push(rel(root, &path)),
            }
        } else if is_top_file(&name) {
            plain_file(&path)?;
            l.marker = true;
            others.push(path);
        } else if LEGACY_SUFFIXES
            .iter()
            .any(|s| name == format!("{VAULT_FILE}{s}"))
        {
            if legacy_known {
                plain_file(&path)?;
                if name == VAULT_FILE {
                    l.vault_dbs.push(path.clone());
                    l.marker = true;
                }
                legacy_files.push(path);
            } else {
                l.unknown.push(rel(root, &path));
            }
        } else {
            l.unknown.push(rel(root, &path));
        }
    }
    legacy_files.sort_by_key(|p| p.file_name() != Some(std::ffi::OsStr::new(VAULT_FILE)));
    // vault files (legacy and per-account) before the small state files
    l.files.extend(legacy_files);
    l.files.extend(others);
    l.unknown.sort();
    Ok(l)
}

// ------------------------------------------------------------------ the steps

/// Overwrite the key bundle row with zero bytes, fold the WAL away and fsync.
fn destroy_key_bundle(path: &Path) -> bool {
    let go = || -> rusqlite::Result<()> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(Duration::from_secs(3))?;
        conn.pragma_update(None, "secure_delete", "ON")?;
        conn.execute(
            "UPDATE meta SET value = zeroblob(length(CAST(value AS BLOB))) WHERE key = 'bundle'",
            [],
        )?;
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    };
    go().is_ok() && File::open(path).and_then(|f| f.sync_all()).is_ok()
}

/// Overwrite with zeros (chunked, fsynced), truncate, unlink. A failed overwrite still tries the
/// unlink. Returns `Err` with a fixed description of the first thing that went wrong.
pub fn shred_file(path: &Path) -> Result<(), &'static str> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        // already gone (SQLite removes `-wal` and `-shm` itself when the last connection of the
        // cryptographic erase closes): nothing left to do, which is the point
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("a file could not be inspected"),
    };
    if !meta.file_type().is_file() {
        return Err("a file changed into something else");
    }
    let mut problem = None;
    let overwritten = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new().write(true).open(path)?;
        zero_fill(&mut f, meta.len())?;
        f.set_len(0)?;
        f.sync_all()
    })();
    if overwritten.is_err() {
        problem = Some("a file could not be overwritten");
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => problem = problem.or(Some("a file could not be removed")),
    }
    match problem {
        None => Ok(()),
        Some(p) => Err(p),
    }
}

/// Wipe the known Passman files and folders under `dir`. See the module documentation.
pub fn secure_wipe_all_local_data(
    dir: &Path,
    plan: &WipePlan,
    observer: &mut dyn WipeObserver,
) -> Result<WipeReport, Refused> {
    if !dir.is_absolute() {
        return Err(Refused::NotAbsolute);
    }
    let canon = match std::fs::canonicalize(dir) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WipeReport {
                nothing_to_do: true,
                ..WipeReport::default()
            })
        }
        Err(_) => return Err(Refused::Unreadable),
    };
    if home_dirs().iter().any(|h| too_broad(&canon, Some(h))) || too_broad(&canon, None) {
        return Err(Refused::TooBroad);
    }
    // Nothing is touched until the whole tree has been looked at.
    let layout = scan(&canon, plan)?;
    let nothing = layout.files.is_empty() && layout.dirs.is_empty();
    if !layout.marker {
        if plan.expect_data {
            return Err(Refused::NotPassmanData);
        }
        if nothing {
            return Ok(WipeReport {
                nothing_to_do: true,
                unknown: layout.unknown,
                ..WipeReport::default()
            });
        }
    }
    let mut report = WipeReport {
        unknown: layout.unknown.clone(),
        ..WipeReport::default()
    };

    // 1. cryptographic erase of every vault, before any file goes away
    for db in &layout.vault_dbs {
        if !destroy_key_bundle(db) {
            // not a usable database: zero the whole file now instead
            if shred_in_place(db).is_err() {
                report.errors.push(WipeError {
                    path: rel(&canon, db),
                    what: "a vault file could not be made unreadable",
                });
            }
        }
        if !observer.step(&WipeStep::KeyBundleDestroyed(db.clone())) {
            report.aborted = true;
            return Ok(report);
        }
    }

    // 2. overwrite and unlink every known file
    let mut touched_dirs: Vec<PathBuf> = Vec::new();
    for file in &layout.files {
        let result = shred_file(file);
        if let Some(parent) = file.parent() {
            if !touched_dirs.iter().any(|d| d == parent) {
                touched_dirs.push(parent.to_path_buf());
            }
        }
        match result {
            Ok(()) => report.files_removed += 1,
            Err(what) => report.errors.push(WipeError {
                path: rel(&canon, file),
                what,
            }),
        }
        let alive = observer.step(&WipeStep::Overwritten(file.clone()))
            && observer.step(&WipeStep::Unlinked(file.clone()));
        if !alive {
            report.aborted = true;
            return Ok(report);
        }
    }
    for d in &touched_dirs {
        let _ = files::fsync_dir(d);
    }

    // 3. the emptied known folders (deepest first); one that still holds unknown files stays
    for d in &layout.dirs {
        match std::fs::remove_dir(d) {
            Ok(()) => {
                report.dirs_removed += 1;
                if !observer.step(&WipeStep::DirRemoved(d.clone())) {
                    report.aborted = true;
                    return Ok(report);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                // Folders that still hold unknown files are expected; they are in `unknown`.
                if std::fs::read_dir(d).is_ok_and(|mut r| r.next().is_none()) {
                    report.errors.push(WipeError {
                        path: rel(&canon, d),
                        what: "a folder could not be removed",
                    });
                }
            }
        }
    }
    let _ = files::fsync_dir(&canon);
    Ok(report)
}

/// Write `len` zero bytes from the start of `f` in 64 KiB chunks, then fsync.
fn zero_fill(f: &mut File, len: u64) -> std::io::Result<()> {
    let mut left = len;
    while left > 0 {
        let n = left.min(ZEROS.len() as u64) as usize;
        f.write_all(&ZEROS[..n])?;
        left -= n as u64;
    }
    f.sync_all()
}

/// Zero a file in place without unlinking it (the fallback of the cryptographic erase for a file
/// SQLite cannot open).
fn shred_in_place(path: &Path) -> std::io::Result<()> {
    let len = std::fs::metadata(path)?.len();
    let mut f = OpenOptions::new().write(true).open(path)?;
    zero_fill(&mut f, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recorder(Vec<WipeStep>);

    impl WipeObserver for Recorder {
        fn step(&mut self, step: &WipeStep) -> bool {
            self.0.push(step.clone());
            true
        }
    }

    /// Stops the wipe after `n` steps.
    struct StopAfter(usize, Vec<WipeStep>);

    impl WipeObserver for StopAfter {
        fn step(&mut self, step: &WipeStep) -> bool {
            self.1.push(step.clone());
            self.1.len() < self.0
        }
    }

    const ID1: &str = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";
    const ID2: &str = "7a9c0d2e-1b3f-4a5c-8d6e-9f0a1b2c3d4e";

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Plain files with the right names (the wipe does not care what is inside a non-database).
    fn fake_tree(root: &Path) {
        write(
            &root.join("profiles.json"),
            b"{\"names\":\"MARKER-REGISTRY\"}",
        );
        write(
            &root.join("profiles.json.corrupt-1700000000000"),
            b"MARKER-OLD",
        );
        write(&root.join("gate.json"), b"MARKER-GATE");
        write(&root.join("gate.json.tmp"), b"MARKER-GATE-TMP");
        write(&root.join("app-settings.json"), b"{}");
        write(&root.join("disguise.json"), b"{}");
        for id in [ID1, ID2] {
            let v = root.join("vaults").join(id);
            write(&v.join("vault.db"), &vec![0xAB; 150_000]);
            write(&v.join("vault.db-wal"), b"MARKER-WAL");
            write(&v.join("vault.db-shm"), b"MARKER-SHM");
            write(&v.join("attachments/one.bin"), b"MARKER-ATTACHMENT");
            write(&v.join("attachments/two.bin"), &vec![7u8; 70_000]);
        }
    }

    fn names(root: &Path) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                out.push(rel(base, &e.path()).to_string_lossy().into_owned());
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), base, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }

    fn all_bytes(root: &Path) -> Vec<u8> {
        fn walk(dir: &Path, out: &mut Vec<u8>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let kind = e.file_type().unwrap();
                if kind.is_symlink() {
                    // a link counts by where it points, never by what is behind it
                    let to = std::fs::read_link(e.path()).unwrap();
                    out.extend(to.to_string_lossy().as_bytes());
                } else if kind.is_dir() {
                    walk(&e.path(), out);
                } else {
                    out.extend(std::fs::read(e.path()).unwrap());
                }
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out
    }

    fn run(root: &Path, plan: &WipePlan) -> Result<WipeReport, Refused> {
        secure_wipe_all_local_data(root, plan, &mut NoObserver)
    }

    #[test]
    fn too_broad_catches_roots_homes_and_their_parents() {
        let p = Path::new;
        assert!(too_broad(p("/"), None));
        assert!(too_broad(p("/tmp"), None));
        assert!(too_broad(p("/home"), None));
        assert!(too_broad(p("/home/ana"), Some(p("/home/ana"))));
        assert!(too_broad(p("/home"), Some(p("/home/ana"))));
        assert!(!too_broad(
            p("/home/ana/.local/share/app"),
            Some(p("/home/ana"))
        ));
        assert!(!too_broad(p("/tmp/.tmp123"), Some(p("/home/ana"))));
        assert!(!too_broad(p("/data/user/0/app.passman.vault/files"), None));
    }

    #[test]
    fn a_relative_or_missing_folder_is_refused_or_a_no_op() {
        assert_eq!(
            run(Path::new("relative/dir"), &WipePlan::default()).unwrap_err(),
            Refused::NotAbsolute
        );
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("not-there");
        let r = run(&gone, &WipePlan::default()).unwrap();
        assert!(r.nothing_to_do);
    }

    #[test]
    fn a_folder_that_does_not_look_like_passman_data_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("holiday.jpg"), b"photo");
        write(&dir.path().join("notes/todo.txt"), b"todo");
        let before = names(dir.path());
        let plan = WipePlan {
            expect_data: true,
            ..WipePlan::default()
        };
        assert_eq!(run(dir.path(), &plan).unwrap_err(), Refused::NotPassmanData);
        assert_eq!(names(dir.path()), before, "nothing was touched");
        // Without the belief that data exists there is nothing of ours to delete: still nothing
        // is removed.
        let r = run(dir.path(), &WipePlan::default()).unwrap();
        assert!(r.nothing_to_do);
        assert_eq!(names(dir.path()), before);
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_known_name_refuses_everything() {
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("precious.txt"), b"keep me");
        for case in [
            "gate.json",
            "profiles.json",
            "vaults",
            "vault-dir",
            "attachments",
            "vault.db",
        ] {
            let dir = tempfile::tempdir().unwrap();
            fake_tree(dir.path());
            let link = match case {
                "vaults" => {
                    std::fs::remove_dir_all(dir.path().join("vaults")).unwrap();
                    dir.path().join("vaults")
                }
                "vault-dir" => {
                    std::fs::remove_dir_all(dir.path().join("vaults").join(ID2)).unwrap();
                    dir.path().join("vaults").join(ID2)
                }
                "attachments" => {
                    let a = dir.path().join("vaults").join(ID2).join("attachments");
                    std::fs::remove_dir_all(&a).unwrap();
                    a
                }
                "vault.db" => {
                    let p = dir.path().join("vaults").join(ID2).join("vault.db");
                    std::fs::remove_file(&p).unwrap();
                    p
                }
                name => {
                    std::fs::remove_file(dir.path().join(name)).unwrap();
                    dir.path().join(name)
                }
            };
            let target = if matches!(case, "vaults" | "vault-dir" | "attachments") {
                outside.path().to_path_buf()
            } else {
                outside.path().join("precious.txt")
            };
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let before = names(dir.path());
            let bytes = all_bytes(dir.path());
            let err = run(dir.path(), &WipePlan::default()).unwrap_err();
            assert!(matches!(err, Refused::UnsafeName(_)), "{case}: {err:?}");
            assert_eq!(names(dir.path()), before, "{case}: nothing was touched");
            assert_eq!(all_bytes(dir.path()), bytes, "{case}: no byte was changed");
            assert_eq!(
                std::fs::read(outside.path().join("precious.txt")).unwrap(),
                b"keep me",
                "{case}: the target of the link is untouched"
            );
        }
    }

    #[test]
    fn a_known_name_that_is_not_a_plain_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fake_tree(dir.path());
        std::fs::remove_file(dir.path().join("gate.json")).unwrap();
        std::fs::create_dir(dir.path().join("gate.json")).unwrap();
        assert!(matches!(
            run(dir.path(), &WipePlan::default()),
            Err(Refused::UnsafeName(_))
        ));
    }

    #[test]
    fn unknown_files_stay_and_are_reported_and_known_ones_go() {
        let dir = tempfile::tempdir().unwrap();
        fake_tree(dir.path());
        write(&dir.path().join("webview/cache.bin"), b"not ours");
        write(&dir.path().join("readme.txt"), b"not ours either");
        write(&dir.path().join("vaults/stray.txt"), b"stray");
        write(
            &dir.path().join("vaults").join(ID1).join("notes.txt"),
            b"mine, not yours",
        );
        write(
            &dir.path()
                .join("vaults")
                .join(ID1)
                .join("attachments/sub/deep.bin"),
            b"nested",
        );
        let r = run(dir.path(), &WipePlan::default()).unwrap();
        let left = names(dir.path());
        for gone in [
            "profiles.json",
            "gate.json",
            "gate.json.tmp",
            "app-settings.json",
            "disguise.json",
        ] {
            assert!(
                !left.contains(&gone.to_string()),
                "{gone} should be gone: {left:?}"
            );
        }
        assert!(!left.iter().any(|n| n.contains("corrupt")));
        assert!(!left.iter().any(|n| n.ends_with("vault.db")));
        assert!(!left.iter().any(|n| n.ends_with("one.bin")));
        // the second account's folder is fully gone, the first one stays because of its stranger
        assert!(!left.iter().any(|n| n.contains(ID2)), "{left:?}");
        for stays in [
            "webview/cache.bin",
            "readme.txt",
            "vaults/stray.txt",
            &format!("vaults/{ID1}/notes.txt"),
            &format!("vaults/{ID1}/attachments/sub/deep.bin"),
        ] {
            assert!(
                left.contains(&stays.to_string()),
                "{stays} must stay: {left:?}"
            );
        }
        assert_eq!(
            std::fs::read(dir.path().join("readme.txt")).unwrap(),
            b"not ours either"
        );
        for reported in [
            "webview",
            "readme.txt",
            "vaults/stray.txt",
            &format!("vaults/{ID1}/notes.txt"),
            &format!("vaults/{ID1}/attachments/sub"),
        ] {
            assert!(
                r.unknown.iter().any(|u| u == Path::new(reported)),
                "{reported} is reported: {:?}",
                r.unknown
            );
        }
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    #[test]
    fn a_realistic_tree_leaves_no_known_byte_behind_and_a_second_wipe_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        fake_tree(dir.path());
        let before = all_bytes(dir.path());
        for m in [
            "MARKER-REGISTRY",
            "MARKER-GATE",
            "MARKER-WAL",
            "MARKER-ATTACHMENT",
            "MARKER-OLD",
        ] {
            assert!(before.windows(m.len()).any(|w| w == m.as_bytes()), "{m}");
        }
        let plan = WipePlan {
            expect_data: true,
            ..WipePlan::default()
        };
        let r = run(dir.path(), &plan).unwrap();
        assert!(r.files_removed >= 15, "{r:?}");
        assert_eq!(names(dir.path()), Vec::<String>::new(), "{r:?}");
        assert!(all_bytes(dir.path()).is_empty());
        // again: nothing left, nothing refused, nothing changed
        let again = run(dir.path(), &WipePlan::default()).unwrap();
        assert!(again.nothing_to_do);
        assert_eq!(again.files_removed, 0);
        assert!(again.errors.is_empty());
        assert!(dir.path().exists(), "the data folder itself stays");
    }

    #[test]
    fn the_overwrite_writes_zeros_over_every_byte_in_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        // longer than two chunks and not a multiple of the chunk size
        std::fs::write(&p, vec![0xEE; 2 * ZEROS.len() + 123]).unwrap();
        let mut f = OpenOptions::new().write(true).open(&p).unwrap();
        zero_fill(&mut f, 2 * ZEROS.len() as u64 + 123).unwrap();
        drop(f);
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(bytes.len(), 2 * ZEROS.len() + 123);
        assert!(bytes.iter().all(|b| *b == 0));
    }

    #[test]
    fn shredding_removes_the_file_and_empties_every_other_name_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a");
        std::fs::write(&p, vec![0xEE; 200_000]).unwrap();
        #[cfg(unix)]
        std::fs::hard_link(&p, dir.path().join("alias")).unwrap();
        shred_file(&p).unwrap();
        assert!(!p.exists());
        #[cfg(unix)]
        {
            let left = std::fs::read(dir.path().join("alias")).unwrap();
            assert!(!left.contains(&0xEE), "the shared contents are gone");
        }
        shred_file(&p).unwrap(); // already gone: a repeated wipe is a no-op, not an error
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert!(
            shred_file(&sub).is_err(),
            "a folder is never shredded as a file"
        );
        assert!(sub.exists());
    }

    #[test]
    fn steps_come_in_the_promised_order() {
        let dir = tempfile::tempdir().unwrap();
        fake_tree(dir.path());
        let mut rec = Recorder(Vec::new());
        secure_wipe_all_local_data(dir.path(), &WipePlan::default(), &mut rec).unwrap();
        let steps = rec.0;
        let last_destroy = steps
            .iter()
            .rposition(|s| matches!(s, WipeStep::KeyBundleDestroyed(_)))
            .unwrap();
        let first_unlink = steps
            .iter()
            .position(|s| matches!(s, WipeStep::Unlinked(_)))
            .unwrap();
        let first_dir = steps
            .iter()
            .position(|s| matches!(s, WipeStep::DirRemoved(_)))
            .unwrap();
        let last_unlink = steps
            .iter()
            .rposition(|s| matches!(s, WipeStep::Unlinked(_)))
            .unwrap();
        assert!(
            last_destroy < first_unlink,
            "every key bundle goes first: {steps:?}"
        );
        assert!(last_unlink < first_dir, "folders go last: {steps:?}");
        assert_eq!(
            steps
                .iter()
                .filter(|s| matches!(s, WipeStep::KeyBundleDestroyed(_)))
                .count(),
            2,
            "one per vault"
        );
        for (i, s) in steps.iter().enumerate() {
            if let WipeStep::Unlinked(p) = s {
                assert_eq!(steps[i - 1], WipeStep::Overwritten(p.clone()));
            }
        }
    }

    #[test]
    fn an_interrupted_wipe_stops_where_told_and_the_next_one_finishes_it() {
        let dir = tempfile::tempdir().unwrap();
        fake_tree(dir.path());
        let mut stop = StopAfter(4, Vec::new());
        let r = secure_wipe_all_local_data(dir.path(), &WipePlan::default(), &mut stop).unwrap();
        assert!(r.aborted);
        assert!(!names(dir.path()).is_empty());
        let done = run(dir.path(), &WipePlan::default()).unwrap();
        assert!(!done.aborted);
        assert!(names(dir.path()).is_empty(), "{:?}", names(dir.path()));
    }

    #[test]
    fn a_file_that_is_not_a_database_is_zeroed_during_the_first_step() {
        let dir = tempfile::tempdir().unwrap();
        fake_tree(dir.path());
        // stop right after the first key-bundle step: that vault.db (150 KB of 0xAB, not SQLite)
        // must already be all zeros, while nothing was unlinked yet
        let mut stop = StopAfter(1, Vec::new());
        secure_wipe_all_local_data(dir.path(), &WipePlan::default(), &mut stop).unwrap();
        let WipeStep::KeyBundleDestroyed(db) = &stop.1[0] else {
            panic!("the first step is the key bundle: {:?}", stop.1);
        };
        let bytes = std::fs::read(db).unwrap();
        assert_eq!(bytes.len(), 150_000);
        assert!(bytes.iter().all(|b| *b == 0));
        assert!(dir.path().join("profiles.json").exists());
    }

    #[test]
    fn the_legacy_vault_in_the_data_folder_is_only_ours_when_registered_or_a_passman_file() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("vault.db"), b"someone else's database");
        write(&dir.path().join("vault.db-wal"), b"someone else's wal");
        write(&dir.path().join("gate.json"), b"{}");
        let r = run(dir.path(), &WipePlan::default()).unwrap();
        assert!(
            dir.path().join("vault.db").exists(),
            "a foreign vault.db is left alone"
        );
        assert!(dir.path().join("vault.db-wal").exists());
        assert!(!dir.path().join("gate.json").exists());
        assert!(r.unknown.contains(&PathBuf::from("vault.db")));
        // registered: it is ours
        let r = run(
            dir.path(),
            &WipePlan {
                legacy_registered: true,
                ..WipePlan::default()
            },
        )
        .unwrap();
        assert!(!dir.path().join("vault.db").exists());
        assert!(!dir.path().join("vault.db-wal").exists());
        assert!(r.files_removed >= 2);
    }
}
