//! `app-settings.json`: the two app-level lock settings.
//!
//! * `lockAfterMinutes`: how long the app may sit unused while it is on screen.
//! * `lockAfterLeaveSeconds`: how long the app may stay open after the person leaves it (phones and
//!   tablets: the app goes to the background). 0 = lock the moment they leave.
//!
//! Plain JSON next to the registry (owner-only, written atomically). It holds those two numbers and
//! nothing else, so it leaks only "this device locks after N". A missing, unreadable or out-of-range
//! file is the defaults; it is never an error and never blocks anything. A file written before the
//! second setting existed reads with the default for it.

use crate::storage::{files, StorageError};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SETTINGS_FILE: &str = "app-settings.json";
/// The only values the inactivity timer accepts: there is no "never".
pub const ALLOWED_MINUTES: [u32; 6] = [1, 5, 15, 30, 60, 240];
pub const DEFAULT_MINUTES: u32 = 15;
/// The only values the "after leaving the app" timer accepts: 0 (at once), 15 s, 30 s, 1, 5, 15, 30 min, 1 h, 4 h.
/// There is no "never" here either.
pub const ALLOWED_LEAVE_SECONDS: [u32; 9] = [0, 15, 30, 60, 300, 900, 1800, 3600, 14400];
/// Five minutes: long enough for a quick look at a message or a permission screen, short enough to matter.
pub const DEFAULT_LEAVE_SECONDS: u32 = 300;
const MAX_FILE_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub lock_after_minutes: u32,
    #[serde(default = "default_leave_seconds")]
    pub lock_after_leave_seconds: u32,
}

fn default_leave_seconds() -> u32 {
    DEFAULT_LEAVE_SECONDS
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            lock_after_minutes: DEFAULT_MINUTES,
            lock_after_leave_seconds: DEFAULT_LEAVE_SECONDS,
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), StorageError> {
        if !ALLOWED_MINUTES.contains(&self.lock_after_minutes) {
            return Err(StorageError::invalid(
                "Choose 1, 5, 15, 30 or 60 minutes, or 4 hours, for the lock timer.",
            ));
        }
        if !ALLOWED_LEAVE_SECONDS.contains(&self.lock_after_leave_seconds) {
            return Err(StorageError::invalid(
                "Choose one of the listed times for locking after you leave the app.",
            ));
        }
        Ok(())
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
                ..AppSettings::default()
            };
            s.validate().unwrap();
            store(dir.path(), &s).unwrap();
            assert_eq!(load(dir.path()), s);
        }
        for secs in ALLOWED_LEAVE_SECONDS {
            let s = AppSettings {
                lock_after_leave_seconds: secs,
                ..AppSettings::default()
            };
            s.validate().unwrap();
            store(dir.path(), &s).unwrap();
            assert_eq!(load(dir.path()), s);
        }
        for bad in [1, 10, 59, 61, 301, 86_400, u32::MAX] {
            let e = AppSettings {
                lock_after_leave_seconds: bad,
                ..AppSettings::default()
            }
            .validate()
            .unwrap_err();
            assert_eq!(e.code(), "invalid_input");
        }
        for bad in [0, 2, 10, 16, 45, 1440, u32::MAX] {
            let e = AppSettings {
                lock_after_minutes: bad,
                ..AppSettings::default()
            }
            .validate()
            .unwrap_err();
            assert_eq!(e.code(), "invalid_input");
        }
    }

    #[test]
    fn the_file_is_small_private_and_only_holds_the_timeouts() {
        let dir = tempfile::tempdir().unwrap();
        store(
            dir.path(),
            &AppSettings {
                lock_after_minutes: 5,
                lock_after_leave_seconds: 60,
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join(SETTINGS_FILE)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 2);
        assert_eq!(v["lockAfterMinutes"], 5);
        assert_eq!(v["lockAfterLeaveSeconds"], 60);
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
            r#"{"lockAfterMinutes": 5, "lockAfterLeaveSeconds": 7}"#.to_string(),
            r#"{"lockAfterMinutes": "5"}"#.to_string(),
            r#"{"lockAfterMinutes": -5}"#.to_string(),
            r#"{"lockAfterMinutes": 0}"#.to_string(),
            " ".repeat(10_000),
        ] {
            std::fs::write(&path, junk).unwrap();
            assert_eq!(load(dir.path()).lock_after_minutes, 15);
        }
    }

    #[test]
    fn a_file_from_before_the_second_setting_reads_with_its_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE), r#"{"lockAfterMinutes": 5}"#).unwrap();
        let s = load(dir.path());
        assert_eq!(s.lock_after_minutes, 5);
        assert_eq!(s.lock_after_leave_seconds, DEFAULT_LEAVE_SECONDS);
    }
}
