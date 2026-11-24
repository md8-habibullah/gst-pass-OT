//! The web view's own files inside the data folder.
//!
//! The screen of the app runs in a system web view. Builds up to 0.2.0 let it keep its normal browser
//! storage in the data folder, next to the vaults (WebKitGTK names seen on a real install:
//! `localstorage`, `WebKitCache`, `CacheStorage`, `storage`, `mediakeys`, `hsts-storage.sqlite`). It held
//! two small settings and cached the app's own pages, nothing secret, but it was "something left behind"
//! after an erase or an uninstall.
//!
//! Since 0.3.0 the window is created in incognito mode (`tauri.conf.json`): on Linux WebKit then keeps
//! everything in memory and writes no cookies, cache or local storage at all. The two settings that lived
//! there (how long the clipboard keeps a copied secret, the colour theme) are in `device.json` now.
//!
//! What this file does is remove what an older build left. [`remove`] runs at every start (a no-op once
//! the folders are gone), and the wipe calls it as well, so "erase everything" leaves no web view data.
//!
//! Safety rules: only the fixed names below are looked at, only directly inside the folder it is given;
//! a link is removed as a link and never followed; nothing outside the folder is touched.

use std::path::{Path, PathBuf};

/// Names WebKit (and the web view library around it) create in a data folder. None of them is a name Hexis
/// uses itself (`profiles.json`, `vaults`, `gate.json`, `device.json`, `app-settings.json`, `disguise.json`,
/// `disguise`, `attachments`, a legacy `vault.db`).
pub const NAMES: &[&str] = &[
    "localstorage",
    "indexeddb",
    "databases",
    "storage",
    "CacheStorage",
    "WebKitCache",
    "mediakeys",
    "deviceidhashsalts",
    "itp",
    "ResourceLoadStatistics",
    "serviceworkers",
    "applicationcache",
    "offlineapplicationcache",
    "cookies",
    "hsts-storage.sqlite",
    "hsts-storage.sqlite-journal",
    "hsts-storage.sqlite-wal",
    "hsts-storage.sqlite-shm",
];

/// Is `name` one of the web view's?
pub fn is_webview_name(name: &str) -> bool {
    NAMES.contains(&name)
}

/// Everything the web view left under `dir`, as `(files, folders)`, folders deepest first so that removing
/// them in order works. A link is listed as a file (it is removed as a link). Missing names are skipped.
pub fn collect(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for name in NAMES {
        walk(&dir.join(name), &mut files, &mut dirs);
    }
    (files, dirs)
}

fn walk(path: &Path, files: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_dir() {
        if let Ok(read) = std::fs::read_dir(path) {
            let mut children: Vec<PathBuf> =
                read.filter_map(|e| e.ok().map(|e| e.path())).collect();
            children.sort();
            for child in children {
                walk(&child, files, dirs);
            }
        }
        dirs.push(path.to_path_buf());
    } else {
        // a file, a link or anything else: removed with `remove_file`, never followed
        files.push(path.to_path_buf());
    }
}

/// Remove everything [`collect`] finds. Returns how many files and folders went. Nothing here can fail the
/// caller: a leftover that cannot be removed is simply still there.
pub fn remove(dir: &Path) -> usize {
    let (files, dirs) = collect(dir);
    let mut gone = 0;
    for f in files {
        if std::fs::remove_file(&f).is_ok() {
            gone += 1;
        }
    }
    for d in dirs {
        if std::fs::remove_dir(&d).is_ok() {
            gone += 1;
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay_out(dir: &Path) {
        for (path, body) in [
            ("localstorage/http_localhost_1420.localstorage", "x"),
            ("localstorage/http_localhost_1420.localstorage-wal", "x"),
            ("WebKitCache/Version 17/Records/aa/bb", "cached"),
            ("WebKitCache/Version 17/salt", "salt"),
            ("CacheStorage/salt", "salt"),
            ("storage/salt", "salt"),
            ("mediakeys/v1/salt", "salt"),
            ("hsts-storage.sqlite", "db"),
            ("cookies", "jar"),
        ] {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
    }

    #[test]
    fn everything_the_web_view_left_is_removed_and_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        lay_out(dir);
        // Hexis's own files, and a stranger's, stay
        for keep in ["profiles.json", "gate.json", "device.json", "notes.txt"] {
            std::fs::write(dir.join(keep), "mine").unwrap();
        }
        std::fs::create_dir_all(dir.join("vaults/abc")).unwrap();
        std::fs::write(dir.join("vaults/abc/vault.db"), "vault").unwrap();
        std::fs::create_dir_all(dir.join("disguise")).unwrap();

        let gone = remove(dir);
        assert!(gone >= 15, "{gone}");
        for name in NAMES {
            assert!(!dir.join(name).exists(), "{name} is still there");
        }
        for keep in [
            "profiles.json",
            "gate.json",
            "device.json",
            "notes.txt",
            "vaults/abc/vault.db",
            "disguise",
        ] {
            assert!(dir.join(keep).exists(), "{keep} must stay");
        }
        assert_eq!(remove(dir), 0, "a second run finds nothing");
    }

    #[test]
    fn a_link_is_removed_as_a_link_and_never_followed() {
        #[cfg(unix)]
        {
            let tmp = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("precious"), "do not touch").unwrap();
            std::os::unix::fs::symlink(outside.path(), tmp.path().join("storage")).unwrap();
            std::fs::create_dir_all(tmp.path().join("localstorage")).unwrap();
            std::os::unix::fs::symlink(
                outside.path().join("precious"),
                tmp.path().join("localstorage/link"),
            )
            .unwrap();
            remove(tmp.path());
            assert!(!tmp.path().join("storage").exists(), "the link itself went");
            assert!(!tmp.path().join("localstorage").exists());
            assert_eq!(
                std::fs::read_to_string(outside.path().join("precious")).unwrap(),
                "do not touch",
                "what the links pointed at is untouched"
            );
        }
    }

    #[test]
    fn a_missing_folder_is_fine() {
        assert_eq!(remove(Path::new("/nonexistent/hexis/test/folder")), 0);
    }

    #[test]
    fn no_web_view_name_collides_with_a_hexis_file() {
        for ours in [
            "profiles.json",
            "vaults",
            "gate.json",
            "device.json",
            "app-settings.json",
            "disguise.json",
            "disguise",
            "attachments",
            "vault.db",
        ] {
            assert!(!is_webview_name(ours), "{ours}");
        }
    }
}
