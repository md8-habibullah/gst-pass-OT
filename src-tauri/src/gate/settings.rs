//! `app-settings.json`: the one app-level setting that exists today, the inactivity timeout.
//!
//! Plain JSON next to the registry (owner-only, written atomically). It holds one number and
//! nothing else, so it leaks only "this device locks after N minutes". A missing, unreadable or
//! out-of-range file is the default (15); it is never an error and never blocks anything.

use crate::storage::{files, StorageError};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SETTINGS_FILE: &str = "app-settings.json";
/// The only values the timer accepts: there is no "never".
pub const ALLOWED_MINUTES: [u32; 4] = [1, 5, 15, 30];
pub const DEFAULT_MINUTES: u32 = 15;
const MAX_FILE_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub lock_after_minutes: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            lock_after_minutes: DEFAULT_MINUTES,
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), StorageError> {
        if ALLOWED_MINUTES.contains(&self.lock_after_minutes) {
            Ok(())
        } else {
            Err(StorageError::invalid(
                "Choose 1, 5, 15 or 30 minutes for the lock timer.",
            ))
        }
    }
}

/// Read the file; anything unusual is the default.
pub fn load(dir: &Path) -> AppSettings {
    let path = dir.join(SETTINGS_FILE);
    let Ok(meta) = std::fs::metadata(&path) else {
        return AppSettings::default();
    };
    if meta.len() > MAX_FILE_BYTES {
        return AppSettings::default();
    }
    std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<AppSettings>(&b).ok())
        .filter(|s| s.validate().is_ok())
        .unwrap_or_default()
}

pub fn store(dir: &Path, settings: &AppSettings) -> Result<(), StorageError> {
    let bytes = serde_json::to_vec_pretty(settings)?;
    files::write_atomic(&dir.join(SETTINGS_FILE), &bytes)
        .map_err(|_| StorageError::Storage("The setting could not be saved.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validation_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).lock_after_minutes, 15);
        for m in ALLOWED_MINUTES {
            let s = AppSettings {
                lock_after_minutes: m,
            };
            s.validate().unwrap();
            store(dir.path(), &s).unwrap();
            assert_eq!(load(dir.path()), s);
        }
        for bad in [0, 2, 10, 16, 60, 1440, u32::MAX] {
            let e = AppSettings {
                lock_after_minutes: bad,
            }
            .validate()
            .unwrap_err();
            assert_eq!(e.code(), "invalid_input");
        }
    }

    #[test]
    fn the_file_is_small_private_and_only_holds_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        store(
            dir.path(),
            &AppSettings {
                lock_after_minutes: 5,
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join(SETTINGS_FILE)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 1);
        assert_eq!(v["lockAfterMinutes"], 5);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(SETTINGS_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_damaged_or_out_of_range_file_is_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        for junk in [
            "".to_string(),
            "not json".to_string(),
            r#"{"lockAfterMinutes": 7}"#.to_string(),
            r#"{"lockAfterMinutes": "5"}"#.to_string(),
            r#"{"lockAfterMinutes": -5}"#.to_string(),
            r#"{"lockAfterMinutes": 0}"#.to_string(),
            " ".repeat(10_000),
        ] {
            std::fs::write(&path, junk).unwrap();
            assert_eq!(load(dir.path()).lock_after_minutes, 15);
        }
    }
}
