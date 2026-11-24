//! Data written by builds made before the app had its final name.
//!
//! The first builds used a working name and the bundle identifier `app.passman.vault`, so their data lived in
//! `~/.local/share/app.passman.vault`. The bundle identifier is now `dev.habibullah.hexis`, which moves the data
//! folder. Nothing may be orphaned by a rename, so on start the old folder is adopted: if the new folder does not
//! exist yet and the old one does, the old one is renamed into its place (the same file system, one atomic step;
//! nothing is copied, deleted or rewritten). Vault files keep the same format: the stored format is frozen
//! (`ai-docs/CRYPTO_SPEC.md`), so an adopted vault opens as before.
//!
//! Desktop only in practice: an Android package id is the app's identity, so a build under a new id is a new app.

use std::path::Path;

/// The data folder of the earliest builds, next to the current one.
pub const PREVIOUS_FOLDER_NAME: &str = "app.passman.vault";
/// The extension those builds gave exported vault files; the open dialog still lists it.
pub const PREVIOUS_EXTENSION: &str = "passman";

/// Move the previous data folder to `dir` when `dir` does not exist yet. `identifier` is the current bundle
/// identifier: the move happens only when `dir` is really named after it, so a test or tool that points the app at
/// some other folder never touches anything. Returns whether a folder was adopted. A failure leaves both folders as
/// they were (the app then simply starts with an empty folder and the old data is still there).
pub fn adopt_previous_folder(dir: &Path, identifier: &str) -> bool {
    if dir.file_name().and_then(|n| n.to_str()) != Some(identifier) {
        return false;
    }
    let Some(parent) = dir.parent() else {
        return false;
    };
    let old = parent.join(PREVIOUS_FOLDER_NAME);
    // `symlink_metadata`: never follow a link someone put there, and never replace an existing new folder
    let is_real_dir = std::fs::symlink_metadata(&old).is_ok_and(|m| m.is_dir());
    if !is_real_dir || std::fs::symlink_metadata(dir).is_ok() {
        return false;
    }
    std::fs::rename(&old, dir).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "dev.habibullah.hexis";

    fn old_folder(root: &Path) -> std::path::PathBuf {
        let old = root.join(PREVIOUS_FOLDER_NAME);
        std::fs::create_dir_all(old.join("vaults")).unwrap();
        std::fs::write(old.join("vault.db"), b"sealed bytes").unwrap();
        old
    }

    #[test]
    fn the_previous_folder_moves_into_place_with_its_contents() {
        let root = tempfile::tempdir().unwrap();
        let old = old_folder(root.path());
        let dir = root.path().join(ID);
        assert!(adopt_previous_folder(&dir, ID));
        assert!(!old.exists());
        assert_eq!(
            std::fs::read(dir.join("vault.db")).unwrap(),
            b"sealed bytes"
        );
        assert!(dir.join("vaults").is_dir());
    }

    #[test]
    fn an_existing_new_folder_is_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let old = old_folder(root.path());
        let dir = root.path().join(ID);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vault.db"), b"newer").unwrap();
        assert!(!adopt_previous_folder(&dir, ID));
        assert!(
            old.join("vault.db").exists(),
            "the old folder stays where it was"
        );
        assert_eq!(std::fs::read(dir.join("vault.db")).unwrap(), b"newer");
    }

    #[test]
    fn nothing_happens_without_an_old_folder() {
        let root = tempfile::tempdir().unwrap();
        assert!(!adopt_previous_folder(&root.path().join(ID), ID));
        assert!(!root.path().join(ID).exists());
    }

    #[test]
    fn a_folder_that_is_not_named_after_the_identifier_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let old = old_folder(root.path());
        // a test harness, a --data-dir override or a differently named profile: not our business
        assert!(!adopt_previous_folder(&root.path().join("e2e-data"), ID));
        assert!(old.join("vault.db").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_in_the_old_place_is_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = root.path().join("somewhere-else");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.path().join(PREVIOUS_FOLDER_NAME)).unwrap();
        assert!(!adopt_previous_folder(&root.path().join(ID), ID));
        assert!(elsewhere.exists());
    }
}
