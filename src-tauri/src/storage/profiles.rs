//! Accounts ("profiles"): several independent vaults in one app.
//!
//! Each profile is a separate vault with its own master password, key bundle and SQLite file.
//! Profiles share nothing. The registry (`profiles.json` in the app data folder) is only a list
//! of display names and folder names; it is **not secret** (the login screen has to show it
//! before any unlock), so anyone who can read the folder can read the names. Vault contents are
//! only ever inside each profile's own ciphertext.
//!
//! Layout: `<data dir>/profiles.json` and `<data dir>/vaults/<id>/vault.db`, where `<id>` is a
//! random UUIDv4 (never the display name: no path tricks, no unicode surprises). A vault from
//! the single-vault era, `<data dir>/vault.db`, is **adopted in place** (registry `dir` is
//! `"."`): it is never moved or rewritten.

use super::error::{Result, StorageError};
use super::files::write_atomic;
use super::uuid;
use crate::crypto::now_millis;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

pub const REGISTRY_FILE: &str = "profiles.json";
pub const VAULTS_DIR: &str = "vaults";
pub const VAULT_FILE: &str = "vault.db";
const REGISTRY_VERSION: u64 = 1;
/// Registry `dir` of the adopted single-vault-era file (`<data dir>/vault.db`).
pub const LEGACY_DIR: &str = ".";
pub const MAX_NAME_CHARS: usize = 40;
pub const LEGACY_NAME: &str = "My vault";

// ------------------------------------------------------------------ wire types

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    pub id: String,
    pub display_name: String,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileList {
    pub profiles: Vec<ProfileInfo>,
    pub selected_id: Option<String>,
    pub last_used_id: Option<String>,
}

// ------------------------------------------------------------------ display names

/// Characters that render as nothing or reorder text (format characters). They would let two
/// names look identical or let a name hide itself, so they are refused.
fn is_invisible_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}' | '\u{E0001}' | '\u{E0020}'..='\u{E007F}'
        | '\u{E000}'..='\u{F8FF}' // private use
    )
}

/// Normalize a display name: Unicode NFKC, trimmed, whitespace runs collapsed to one space,
/// 1-40 characters, no control or invisible formatting characters.
pub fn normalize_display_name(raw: &str) -> Result<String> {
    let nfkc: String = raw.nfkc().collect();
    let trimmed = nfkc.trim();
    if trimmed.is_empty() {
        return Err(StorageError::invalid("Enter a name for this account."));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || is_invisible_format(c))
    {
        return Err(StorageError::invalid(
            "The name contains characters that cannot be shown. Use letters, numbers and spaces.",
        ));
    }
    let collapsed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_NAME_CHARS {
        return Err(StorageError::invalid(
            "Use at most 40 characters for the account name.",
        ));
    }
    Ok(collapsed)
}

/// What uniqueness compares: the normalized name, lower-cased, normalized again (so the result
/// is stable under NFKC even for characters whose lower-case form decomposes).
pub fn name_key(normalized: &str) -> String {
    normalized.to_lowercase().nfkc().collect()
}

// ------------------------------------------------------------------ registry file

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub display_name: String,
    pub created_at: u64,
    #[serde(default)]
    pub last_used_at: Option<u64>,
    /// `"."` for the adopted legacy file, otherwise exactly `vaults/<id>`.
    pub dir: String,
}

impl Entry {
    pub fn info(&self) -> ProfileInfo {
        ProfileInfo {
            id: self.id.clone(),
            display_name: self.display_name.clone(),
            created_at: self.created_at,
            last_used_at: self.last_used_at,
        }
    }

    pub fn is_legacy(&self) -> bool {
        self.dir == LEGACY_DIR
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryFile {
    version: u64,
    profiles: Vec<Entry>,
}

enum Parsed {
    Ok(Vec<Entry>),
    /// Written by a newer Passman: never touched, the app refuses to start instead.
    Newer,
    Corrupt,
}

fn parse_registry(bytes: &[u8]) -> Parsed {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Parsed::Corrupt;
    };
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v > REGISTRY_VERSION => return Parsed::Newer,
        Some(REGISTRY_VERSION) => {}
        _ => return Parsed::Corrupt,
    }
    let Ok(file) = serde_json::from_value::<RegistryFile>(value) else {
        return Parsed::Corrupt;
    };
    if entries_are_valid(&file.profiles) {
        Parsed::Ok(file.profiles)
    } else {
        Parsed::Corrupt
    }
}

/// Everything a trusted registry guarantees; anything else is treated as corruption, which
/// also stops a tampered `dir` (like `../..`) from ever reaching a delete.
fn entries_are_valid(entries: &[Entry]) -> bool {
    let mut ids = std::collections::HashSet::new();
    let mut keys = std::collections::HashSet::new();
    let mut legacy = 0;
    for e in entries {
        if !uuid::is_valid(&e.id) || !ids.insert(e.id.clone()) {
            return false;
        }
        match normalize_display_name(&e.display_name) {
            Ok(n) if n == e.display_name && keys.insert(name_key(&n)) => {}
            _ => return false,
        }
        if e.is_legacy() {
            legacy += 1;
        } else if e.dir != format!("{VAULTS_DIR}/{}", e.id) {
            return false;
        }
    }
    legacy <= 1
}

fn mtime_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(now_millis, |d| d.as_millis() as u64)
}

/// Build a registry from what is on disk: the adopted legacy file plus any `vaults/<uuid>/vault.db`.
fn rebuild(root: &Path) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let legacy = root.join(VAULT_FILE);
    if legacy.is_file() {
        out.push(Entry {
            id: uuid::new_v4()?,
            display_name: LEGACY_NAME.into(),
            created_at: mtime_ms(&legacy),
            last_used_at: None,
            dir: LEGACY_DIR.into(),
        });
    }
    let mut found: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root.join(VAULTS_DIR)) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && uuid::is_valid(&name) && e.path().join(VAULT_FILE).is_file() {
                found.push(name);
            }
        }
    }
    found.sort();
    for (i, id) in found.into_iter().enumerate() {
        let created = mtime_ms(&root.join(VAULTS_DIR).join(&id).join(VAULT_FILE));
        out.push(Entry {
            dir: format!("{VAULTS_DIR}/{id}"),
            id,
            display_name: format!("Recovered vault {}", i + 1),
            created_at: created,
            last_used_at: None,
        });
    }
    Ok(out)
}

pub struct Registry {
    root: PathBuf,
    entries: Vec<Entry>,
}

impl Registry {
    /// Read the registry. A missing file adopts what is on disk; an unreadable or invalid one is
    /// moved aside as `profiles.json.corrupt-<ms>` (never deleted) and rebuilt from the folders.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(REGISTRY_FILE);
        let (entries, must_save) =
            match std::fs::read(&path) {
                Ok(bytes) => match parse_registry(&bytes) {
                    Parsed::Ok(entries) => (entries, false),
                    Parsed::Newer => return Err(StorageError::Storage(
                        "The account list was saved by a newer version of Passman. Update the app."
                            .into(),
                    )),
                    Parsed::Corrupt => {
                        let aside = root.join(format!("{REGISTRY_FILE}.corrupt-{}", now_millis()));
                        std::fs::rename(&path, aside)?;
                        (rebuild(root)?, true)
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let entries = rebuild(root)?;
                    let save = !entries.is_empty();
                    (entries, save)
                }
                Err(_) => {
                    return Err(StorageError::Storage(
                        "The account list could not be read.".into(),
                    ))
                }
            };
        let registry = Self {
            root: root.to_path_buf(),
            entries,
        };
        if must_save {
            registry.save()?;
        }
        Ok(registry)
    }

    /// A registry with no accounts and nothing on disk (the state after the wipe).
    pub fn empty(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            entries: Vec::new(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let file = RegistryFile {
            version: REGISTRY_VERSION,
            profiles: self.entries.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file)?;
        write_atomic(&self.root.join(REGISTRY_FILE), &bytes)
            .map_err(|_| StorageError::Storage("The account list could not be saved.".into()))
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// `vault.db` of a profile.
    pub fn vault_path(&self, e: &Entry) -> PathBuf {
        self.root.join(&e.dir).join(VAULT_FILE)
    }

    /// The profile's own folder; `None` for the adopted legacy file (it lives in the data root).
    pub fn profile_dir(&self, e: &Entry) -> Option<PathBuf> {
        (!e.is_legacy()).then(|| self.root.join(&e.dir))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Newest-used first; never-used profiles by creation time, newest first.
    pub fn list(&self, selected: Option<&str>) -> ProfileList {
        let mut profiles: Vec<ProfileInfo> = self.entries.iter().map(Entry::info).collect();
        profiles.sort_by(|a, b| {
            b.last_used_at
                .cmp(&a.last_used_at)
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        let last_used_id = profiles
            .iter()
            .find(|p| p.last_used_at.is_some())
            .map(|p| p.id.clone());
        ProfileList {
            profiles,
            selected_id: selected.map(str::to_string),
            last_used_id,
        }
    }

    /// Is this name key used by a profile other than `except`?
    pub fn key_taken(&self, key: &str, except: Option<&str>) -> bool {
        self.entries
            .iter()
            .any(|e| Some(e.id.as_str()) != except && name_key(&e.display_name) == key)
    }

    /// Run `change` on the entry list and save; the in-memory list is restored if saving fails.
    fn commit(&mut self, change: impl FnOnce(&mut Vec<Entry>) -> Result<()>) -> Result<()> {
        let before = self.entries.clone();
        let result = change(&mut self.entries).and_then(|()| self.save());
        if result.is_err() {
            self.entries = before;
        }
        result
    }

    /// Register a new profile (the caller already created its folder). Name must be unique.
    pub fn add(&mut self, entry: Entry) -> Result<()> {
        let key = name_key(&entry.display_name);
        self.commit(|entries| {
            if entries.iter().any(|e| name_key(&e.display_name) == key) {
                return Err(StorageError::ProfileNameTaken);
            }
            entries.push(entry);
            Ok(())
        })
    }

    pub fn rename(&mut self, id: &str, display_name: &str) -> Result<ProfileInfo> {
        let key = name_key(display_name);
        let mut info = None;
        self.commit(|entries| {
            if entries
                .iter()
                .any(|e| e.id != id && name_key(&e.display_name) == key)
            {
                return Err(StorageError::ProfileNameTaken);
            }
            let e = entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(StorageError::NotFound)?;
            e.display_name = display_name.to_string();
            info = Some(e.info());
            Ok(())
        })?;
        info.ok_or(StorageError::NotFound)
    }

    pub fn remove(&mut self, id: &str) -> Result<Entry> {
        let mut removed = None;
        self.commit(|entries| {
            let i = entries
                .iter()
                .position(|e| e.id == id)
                .ok_or(StorageError::NotFound)?;
            removed = Some(entries.remove(i));
            Ok(())
        })?;
        removed.ok_or(StorageError::NotFound)
    }

    /// Record that a profile was just used (best effort for the caller; see `AppCore`).
    pub fn touch(&mut self, id: &str) -> Result<()> {
        let now = now_millis();
        self.commit(|entries| {
            let e = entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(StorageError::NotFound)?;
            e.last_used_at = Some(now);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_normalized_trimmed_and_collapsed() {
        let n = |s: &str| normalize_display_name(s).unwrap();
        assert_eq!(n("  Work   Account \u{3000} "), "Work Account");
        assert_eq!(
            n("Ｗｏｒｋ"),
            "Work",
            "fullwidth letters become ASCII under NFKC"
        );
        assert_eq!(
            n("Zoe\u{0308}"),
            "Zoë",
            "decomposed and composed forms agree"
        );
        assert_eq!(n("Zoë"), "Zoë");
        assert_eq!(n("ﬁnance"), "finance", "ligatures fold");
        assert_eq!(n(&"a".repeat(40)).chars().count(), 40);
    }

    #[test]
    fn bad_names_are_refused() {
        for bad in [
            "",
            "   ",
            "a\tb",
            "a\nb",
            "a\u{0000}b",
            "x\u{200B}y",
            "x\u{202E}y",
            "x\u{FEFF}y",
            "x\u{E000}y",
        ] {
            let e = normalize_display_name(bad).unwrap_err();
            assert_eq!(e.code(), "invalid_input", "{bad:?}");
        }
        assert_eq!(
            normalize_display_name(&"a".repeat(41)).unwrap_err().code(),
            "invalid_input"
        );
        // 40 *characters*, not bytes
        assert!(normalize_display_name(&"é".repeat(40)).is_ok());
    }

    #[test]
    fn keys_ignore_case_width_and_unicode_form() {
        let k = |s: &str| name_key(&normalize_display_name(s).unwrap());
        assert_eq!(k("Zoë"), k("ZOË"));
        assert_eq!(k("Zoë"), k("Zoe\u{0308}"));
        assert_eq!(k("Work"), k("ｗｏｒｋ"));
        assert_ne!(k("Work"), k("Work 2"));
    }

    #[test]
    fn registry_entries_reject_path_tricks() {
        let id = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";
        let good = Entry {
            id: id.into(),
            display_name: "Work".into(),
            created_at: 1,
            last_used_at: None,
            dir: format!("vaults/{id}"),
        };
        assert!(entries_are_valid(std::slice::from_ref(&good)));
        for dir in [
            "../..",
            "/etc",
            "vaults/other",
            "vaults/../x",
            "vaults",
            "..",
        ] {
            let mut bad = good.clone();
            bad.dir = dir.into();
            assert!(!entries_are_valid(&[bad]), "{dir}");
        }
    }
}
