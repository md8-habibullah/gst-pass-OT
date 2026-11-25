//! Folders: making, renaming and removing them, and moving items into them.
//!
//! A folder is a path of names joined by `/` (`Work/Clients` is "Clients" inside "Work", the way Bitwarden writes nested
//! folders, so an import keeps its structure). An item carries its folder path in its payload. The folders made on
//! purpose are also listed in the synced preferences (`backup/prefs.rs`), which is what lets an empty folder exist and
//! follow the person to every device. The folders that exist are the union of both lists plus every parent, so an
//! item's folder is never lost even if two devices wrote the list at the same time.
//!
//! Moving an item to another folder is an ordinary edit: it writes a new version (`changedFields: ["folder"]`), keeps
//! the old one in the history and is queued for sync. It needs no password: it shows and copies nothing.
//!
//! This file is a child module of `vault.rs` (`#[path]` declaration there), so it can use the vault's private index.

use super::{IndexEntry, Vault};
use crate::crypto::now_millis;
use crate::storage::error::{Result, StorageError};
use crate::storage::model::{ItemEvent, MAX_FOLDER_CHARS};
use std::collections::BTreeSet;

/// Folder parts are joined with this.
pub const SEP: char = '/';
/// One part of a path ("Clients"), in characters.
pub const MAX_PART_CHARS: usize = 64;
/// How many levels a path may nest.
pub const MAX_DEPTH: usize = 6;
/// How many folders may be made on purpose.
pub const MAX_FOLDERS: usize = 1000;

/// Trim every part and drop the empty ones: `" Work // Clients/ "` is `Work/Clients`. `None` when nothing is left.
pub fn clean(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw
        .split(SEP)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// A path that may be used for a new or renamed folder.
pub fn check(path: &str) -> Result<()> {
    if path.chars().any(char::is_control) {
        return Err(StorageError::invalid(
            "A folder name cannot contain control characters.",
        ));
    }
    let parts: Vec<&str> = path.split(SEP).collect();
    if parts.len() > MAX_DEPTH {
        return Err(StorageError::invalid(format!(
            "Folders can be nested {MAX_DEPTH} levels deep at most."
        )));
    }
    if parts.iter().any(|p| p.chars().count() > MAX_PART_CHARS) {
        return Err(StorageError::invalid(format!(
            "A folder name can have {MAX_PART_CHARS} characters at most."
        )));
    }
    if path.chars().count() > MAX_FOLDER_CHARS {
        return Err(StorageError::invalid(format!(
            "A folder path can have {MAX_FOLDER_CHARS} characters at most."
        )));
    }
    Ok(())
}

/// Is `path` the folder `ancestor`, or something inside it?
pub fn is_inside(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.starts_with(SEP))
}

/// Put `to` where the leading `from` of `path` was (`None`: the top level). `None` back means "no folder".
pub fn replace_prefix(path: &str, from: &str, to: Option<&str>) -> Option<String> {
    let rest = if path == from {
        ""
    } else {
        path.strip_prefix(from)?.strip_prefix(SEP)?
    };
    match (to, rest.is_empty()) {
        (Some(t), true) => Some(t.to_string()),
        (Some(t), false) => Some(format!("{t}/{rest}")),
        (None, true) => None,
        (None, false) => Some(rest.to_string()),
    }
}

/// The parent of a path: `Work/Clients` is `Work`, `Work` is the top level.
pub fn parent(path: &str) -> Option<&str> {
    path.rsplit_once(SEP).map(|(p, _)| p)
}

/// `a/b/c` puts `a`, `a/b` and `a/b/c` in the set.
fn with_parents(set: &mut BTreeSet<String>, path: &str) {
    for (i, c) in path.char_indices() {
        if c == SEP {
            set.insert(path[..i].to_string());
        }
    }
    set.insert(path.to_string());
}

impl Vault {
    /// The folders items of this vault sit in (active items only).
    fn folders_in_use(&self) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        for e in self.index.values() {
            if let IndexEntry::Active(s) = e {
                if let Some(f) = s.folder.as_deref().and_then(clean) {
                    with_parents(&mut set, &f);
                }
            }
        }
        set
    }

    /// Every folder that exists: made on purpose, or used by an item, with every parent.
    pub fn folders_known(&self) -> Vec<String> {
        let mut set = self.folders_in_use();
        for f in &self.prefs().folders {
            if let Some(f) = clean(f) {
                with_parents(&mut set, &f);
            }
        }
        let mut all: Vec<String> = set.into_iter().collect();
        all.sort_by_key(|f| f.to_lowercase());
        all
    }

    /// The spelling of a folder that already exists under another capitalisation, or the path as given.
    fn folder_spelling(&self, path: &str) -> String {
        self.folders_known()
            .into_iter()
            .find(|k| k.to_lowercase() == path.to_lowercase())
            .unwrap_or_else(|| path.to_string())
    }

    /// Make a folder (and nothing else: the parents come with the path). A folder that exists already, in any
    /// capitalisation, is left as it is. Returns the path as it is spelled in the vault.
    pub fn folder_create(&mut self, raw: &str) -> Result<String> {
        let path = clean(raw).ok_or_else(|| StorageError::invalid("Give the folder a name."))?;
        check(&path)?;
        let spelled = self.folder_spelling(&path);
        let mut list = self.prefs().folders;
        if list.len() >= MAX_FOLDERS && !self.folders_known().contains(&spelled) {
            return Err(StorageError::invalid(format!(
                "Use at most {MAX_FOLDERS} folders."
            )));
        }
        if !list.contains(&spelled) {
            list.push(spelled.clone());
            self.set_folder_list(list)?;
        }
        Ok(spelled)
    }

    /// Rename a folder, or move it inside another one (`to` is the whole new path). Everything inside it comes along:
    /// the folders made on purpose and every active item. A name that exists already merges the two. Returns how
    /// many items were changed.
    pub fn folder_rename(&mut self, from: &str, to: &str) -> Result<usize> {
        let from = clean(from).ok_or_else(|| StorageError::invalid("Choose a folder."))?;
        let to = clean(to).ok_or_else(|| StorageError::invalid("Give the folder a name."))?;
        check(&to)?;
        if !self.folders_known().contains(&from) {
            return Err(StorageError::NotFound);
        }
        if is_inside(&to, &from) && to != from {
            return Err(StorageError::invalid(
                "A folder cannot be moved inside itself.",
            ));
        }
        let to = if to.to_lowercase() == from.to_lowercase() {
            to // changing only the capitalisation of its own name
        } else {
            self.folder_spelling(&to)
        };
        if to == from {
            return Ok(0);
        }
        self.remap_folders(&from, Some(&to))
    }

    /// Remove a folder. What was inside moves up one level (the items and the folders alike), so nothing is lost and
    /// nothing is left in a folder that is gone. Returns how many items were changed.
    pub fn folder_dissolve(&mut self, path: &str) -> Result<usize> {
        let path = clean(path).ok_or_else(|| StorageError::invalid("Choose a folder."))?;
        if !self.folders_known().contains(&path) {
            return Err(StorageError::NotFound);
        }
        let up = parent(&path).map(str::to_string);
        self.remap_folders(&path, up.as_deref())
    }

    /// Everything at or inside `from` goes to `to` (`None`: the top level), in the list and on the items.
    fn remap_folders(&mut self, from: &str, to: Option<&str>) -> Result<usize> {
        let ids: Vec<String> = self
            .index
            .iter()
            .filter_map(|(id, e)| match e {
                IndexEntry::Active(s)
                    if s.folder.as_deref().is_some_and(|f| is_inside(f, from)) =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .collect();
        let mut changed = 0;
        for id in ids {
            let (old, _) = self.load_latest(&id)?;
            let target = old
                .folder
                .as_deref()
                .and_then(|f| replace_prefix(f, from, to));
            if self.set_item_folder(&id, target)? {
                changed += 1;
            }
        }
        let list: Vec<String> = self
            .prefs()
            .folders
            .iter()
            .filter_map(|f| clean(f))
            .filter_map(|f| {
                if is_inside(&f, from) {
                    replace_prefix(&f, from, to)
                } else {
                    Some(f)
                }
            })
            .collect();
        self.set_folder_list(list)?;
        Ok(changed)
    }

    /// Move items into a folder (`None`: out of every folder). The folder is made if it is new. Returns how many
    /// items changed (one that is already there is left alone).
    pub fn items_move(&mut self, ids: &[String], folder: Option<&str>) -> Result<usize> {
        if ids.len() > 5_000 {
            return Err(StorageError::invalid("Move at most 5,000 items at once."));
        }
        let folder = match folder.map(str::trim).filter(|f| !f.is_empty()) {
            Some(raw) => {
                let path =
                    clean(raw).ok_or_else(|| StorageError::invalid("Give the folder a name."))?;
                check(&path)?;
                Some(self.folder_spelling(&path))
            }
            None => None,
        };
        for id in ids {
            self.require_active(id)?;
        }
        let mut changed = 0;
        for id in ids {
            if self.set_item_folder(id, folder.clone())? {
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// One item's folder, as a new version. `false` when it is there already.
    fn set_item_folder(&mut self, id: &str, folder: Option<String>) -> Result<bool> {
        let (old, version) = self.load_latest(id)?;
        let current = old.folder.as_deref().and_then(clean);
        if current == folder {
            return Ok(false);
        }
        let mut next = old.clone();
        next.folder = folder;
        next.updated_at = now_millis().max(old.updated_at);
        next.event = ItemEvent::Edited;
        next.restored_from = None;
        next.changed_fields = vec!["folder".to_string()];
        self.commit(next, Some(version), self.next_number(id, version)?)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_cleaned_part_by_part() {
        assert_eq!(clean(" Work // Clients/ ").as_deref(), Some("Work/Clients"));
        assert_eq!(clean("   "), None);
        assert_eq!(clean("/"), None);
        assert_eq!(clean("Plain").as_deref(), Some("Plain"));
    }

    #[test]
    fn limits_are_enforced() {
        assert!(check("Work/Clients").is_ok());
        assert!(check("a/b/c/d/e/f").is_ok());
        assert!(check("a/b/c/d/e/f/g").is_err());
        assert!(check(&"x".repeat(64)).is_ok());
        assert!(check(&"x".repeat(65)).is_err());
        assert!(check(&["a".repeat(60), "b".repeat(60), "c".repeat(60)].join("/")).is_err());
        assert!(check("a\u{7}b").is_err());
        assert!(check("tab\there").is_err());
    }

    #[test]
    fn inside_means_the_folder_or_something_in_it_not_a_shared_prefix() {
        assert!(is_inside("Work", "Work"));
        assert!(is_inside("Work/Clients", "Work"));
        assert!(!is_inside("Workshop", "Work"));
        assert!(!is_inside("Wor", "Work"));
        assert!(!is_inside("Home/Work", "Work"));
    }

    #[test]
    fn a_prefix_is_replaced_whole_parts_only() {
        assert_eq!(
            replace_prefix("Work", "Work", Some("Job")).as_deref(),
            Some("Job")
        );
        assert_eq!(
            replace_prefix("Work/Clients/Acme", "Work", Some("Job")).as_deref(),
            Some("Job/Clients/Acme")
        );
        assert_eq!(replace_prefix("Work", "Work", None), None);
        assert_eq!(
            replace_prefix("Work/Clients", "Work", None).as_deref(),
            Some("Clients")
        );
        assert_eq!(replace_prefix("Workshop", "Work", Some("Job")), None);
    }

    #[test]
    fn the_parent_of_a_path() {
        assert_eq!(parent("Work/Clients"), Some("Work"));
        assert_eq!(parent("Work"), None);
        assert_eq!(parent("a/b/c"), Some("a/b"));
    }

    #[test]
    fn every_parent_of_a_path_exists_too() {
        let mut set = BTreeSet::new();
        with_parents(&mut set, "a/b/c");
        assert_eq!(set.into_iter().collect::<Vec<_>>(), ["a", "a/b", "a/b/c"]);
    }
}
