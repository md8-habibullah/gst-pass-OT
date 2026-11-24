//! `device.json`: facts about this install that are not secret and that the erase keeps.
//!
//! * `timeZone`: the IANA name the person chose on the first screen ("Asia/Dhaka"). It only decides how
//!   UTC times are *shown*; everything the app stores is UTC.
//! * `setupDone`: the first-run welcome was seen. After an erase this lets the app open its demo vault
//!   straight away, with no setup screens, so a reset does not announce itself.
//! * `timeCheck`: ask a few HTTPS servers for the real time (`crate::clock`). On by default.
//! * `demoProfile`: the id of the demo vault while one exists (`storage/demo.rs`).
//! * `theme`: `system` (follow the device's light or dark setting), `light` or `dark`.
//! * `clipboardClear`: how many seconds a copied secret stays on the clipboard (15, 30 or 60).
//!
//! The last two used to live in the web view's own storage. They are here so the web view can run without
//! writing anything to disk (`storage/webview_data.rs`), and so the lock screen, which is shown before any
//! vault is open, already has the right colours.
//!
//! The file leaks nothing a vault holds: a place name, a few switches and a random id. A missing,
//! unreadable or oversized file is the defaults and never blocks anything. A value that is not one of the
//! known choices (a hand edit) is the default for that one setting only.

use crate::storage::{files, StorageError};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

pub const DEVICE_FILE: &str = "device.json";
const MAX_FILE_BYTES: u64 = 4096;
const MAX_ZONE_CHARS: usize = 64;

/// The colours of the screen. `System` follows the device's own light or dark setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// The seconds a copied secret may stay on the clipboard.
pub const CLIPBOARD_CHOICES: [u32; 3] = [15, 30, 60];
pub const DEFAULT_CLIPBOARD_CLEAR: u32 = 30;

fn lenient_theme<'de, D: Deserializer<'de>>(d: D) -> Result<Theme, D::Error> {
    // an unknown word is "system", not a reason to forget the zone and the other settings
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<Theme>(v).unwrap_or_default())
}

fn thirty() -> u32 {
    DEFAULT_CLIPBOARD_CLEAR
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSettings {
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub setup_done: bool,
    #[serde(default = "yes")]
    pub time_check: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demo_profile: Option<String>,
    #[serde(default, deserialize_with = "lenient_theme")]
    pub theme: Theme,
    #[serde(default = "thirty")]
    pub clipboard_clear: u32,
}

fn yes() -> bool {
    true
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            time_zone: None,
            setup_done: false,
            time_check: true,
            demo_profile: None,
            theme: Theme::System,
            clipboard_clear: DEFAULT_CLIPBOARD_CLEAR,
        }
    }
}

/// What the screen is told. The demo id stays inside the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    /// `None` until the person has chosen one on the first screen.
    pub time_zone: Option<String>,
    pub setup_done: bool,
    pub time_check: bool,
    pub theme: Theme,
    /// Seconds a copied secret stays on the clipboard: 15, 30 or 60.
    pub clipboard_clear: u32,
}

impl From<&DeviceSettings> for DeviceInfo {
    fn from(s: &DeviceSettings) -> Self {
        Self {
            time_zone: s.time_zone.clone(),
            setup_done: s.setup_done,
            time_check: s.time_check,
            theme: s.theme,
            clipboard_clear: s.clipboard_clear,
        }
    }
}

/// An IANA zone name looks like `Area/Location` (`Asia/Dhaka`, `America/Argentina/Buenos_Aires`, `Etc/GMT+5`) or
/// is `UTC`. The core cannot know every name (the webview knows them), so it checks the shape only: letters, digits,
/// `_`, `+`, `-` and `/`, no empty or dotted part. The screen only sends names its own time library accepted.
pub fn validate_zone(zone: &str) -> Result<String, StorageError> {
    let zone = zone.trim();
    let shape_ok = !zone.is_empty()
        && zone.chars().count() <= MAX_ZONE_CHARS
        && zone
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'))
        && zone
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('-') && part != "." && part != "..");
    if shape_ok {
        Ok(zone.to_string())
    } else {
        Err(StorageError::invalid(
            "Choose a time zone from the list, for example Asia/Dhaka or Europe/Berlin.",
        ))
    }
}

/// Read the file; anything unusual is the defaults.
pub fn load(dir: &Path) -> DeviceSettings {
    let path = dir.join(DEVICE_FILE);
    let Ok(meta) = std::fs::metadata(&path) else {
        return DeviceSettings::default();
    };
    if meta.len() > MAX_FILE_BYTES {
        return DeviceSettings::default();
    }
    let mut s = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<DeviceSettings>(&b).ok())
        .unwrap_or_default();
    s.time_zone = s.time_zone.as_deref().and_then(|z| validate_zone(z).ok());
    if !CLIPBOARD_CHOICES.contains(&s.clipboard_clear) {
        s.clipboard_clear = DEFAULT_CLIPBOARD_CLEAR;
    }
    s
}

/// Is `seconds` one of the clipboard choices?
pub fn validate_clipboard_clear(seconds: u32) -> Result<u32, StorageError> {
    if CLIPBOARD_CHOICES.contains(&seconds) {
        Ok(seconds)
    } else {
        Err(StorageError::invalid("Choose 15, 30 or 60 seconds."))
    }
}

pub fn store(dir: &Path, settings: &DeviceSettings) -> Result<(), StorageError> {
    let bytes = serde_json::to_vec_pretty(settings)?;
    files::write_atomic(&dir.join(DEVICE_FILE), &bytes)
        .map_err(|_| StorageError::Storage("The setting could not be saved.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_names_are_checked_for_shape() {
        for ok in [
            "UTC",
            "Asia/Dhaka",
            "America/Argentina/Buenos_Aires",
            "Etc/GMT+5",
            "Europe/Isle_of_Man",
            " Asia/Kolkata ",
        ] {
            assert!(validate_zone(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "   ",
            "../etc/passwd",
            "Asia//Dhaka",
            "/Asia/Dhaka",
            "Asia/Dhaka/",
            "Asia/Dh aka",
            "Asia\\Dhaka",
            "Zone/\u{202e}evil",
            "Asia/-Dhaka",
            &"A".repeat(65),
        ] {
            assert!(validate_zone(bad).is_err(), "{bad:?}");
        }
        assert_eq!(validate_zone(" Asia/Kolkata ").unwrap(), "Asia/Kolkata");
    }

    #[test]
    fn defaults_round_trip_and_damage_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let d = load(dir.path());
        assert_eq!(d, DeviceSettings::default());
        assert!(d.time_check && d.time_zone.is_none() && !d.setup_done);
        let s = DeviceSettings {
            time_zone: Some("Asia/Dhaka".into()),
            setup_done: true,
            time_check: false,
            demo_profile: Some("abc".into()),
            theme: Theme::Dark,
            clipboard_clear: 60,
        };
        store(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()), s);
        for junk in ["", "nope", "[]", &" ".repeat(9000), r#"{"timeZone": 5}"#] {
            std::fs::write(dir.path().join(DEVICE_FILE), junk).unwrap();
            assert_eq!(load(dir.path()), DeviceSettings::default(), "{junk:?}");
        }
        // a hand-edited bad zone is dropped, the rest is kept
        std::fs::write(
            dir.path().join(DEVICE_FILE),
            r#"{"timeZone":"../x","setupDone":true,"timeCheck":false}"#,
        )
        .unwrap();
        let d = load(dir.path());
        assert_eq!(d.time_zone, None);
        assert!(d.setup_done && !d.time_check);
    }

    #[test]
    fn an_unknown_theme_or_clipboard_time_only_costs_that_one_setting() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(DEVICE_FILE),
            r#"{"timeZone":"Asia/Dhaka","setupDone":true,"timeCheck":false,"theme":"pink","clipboardClear":7}"#,
        )
        .unwrap();
        let d = load(dir.path());
        assert_eq!(d.theme, Theme::System);
        assert_eq!(d.clipboard_clear, DEFAULT_CLIPBOARD_CLEAR);
        assert_eq!(
            d.time_zone.as_deref(),
            Some("Asia/Dhaka"),
            "the rest is kept"
        );
        assert!(d.setup_done && !d.time_check);
        // a theme of the wrong kind is the same
        std::fs::write(
            dir.path().join(DEVICE_FILE),
            r#"{"setupDone":true,"theme":5}"#,
        )
        .unwrap();
        let d = load(dir.path());
        assert_eq!((d.theme, d.setup_done), (Theme::System, true));
        // a file written before these settings existed reads as the defaults for them
        std::fs::write(
            dir.path().join(DEVICE_FILE),
            r#"{"timeZone":"UTC","setupDone":true}"#,
        )
        .unwrap();
        let d = load(dir.path());
        assert_eq!((d.theme, d.clipboard_clear), (Theme::System, 30));
    }

    #[test]
    fn the_wire_names_are_plain_words() {
        for (t, word) in [
            (Theme::System, "system"),
            (Theme::Light, "light"),
            (Theme::Dark, "dark"),
        ] {
            assert_eq!(serde_json::to_string(&t).unwrap(), format!("\"{word}\""));
            assert_eq!(
                serde_json::from_str::<Theme>(&format!("\"{word}\"")).unwrap(),
                t
            );
        }
        let json = serde_json::to_string(&DeviceInfo::from(&DeviceSettings::default())).unwrap();
        assert!(
            json.contains(r#""theme":"system""#) && json.contains(r#""clipboardClear":30"#),
            "{json}"
        );
        assert!(validate_clipboard_clear(15).is_ok() && validate_clipboard_clear(60).is_ok());
        for bad in [0, 14, 31, 61, 300] {
            assert!(validate_clipboard_clear(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_screen_never_sees_the_demo_id() {
        let s = DeviceSettings {
            demo_profile: Some("secret-id".into()),
            ..DeviceSettings::default()
        };
        let json = serde_json::to_string(&DeviceInfo::from(&s)).unwrap();
        assert!(!json.contains("secret-id") && !json.contains("demo"));
    }
}
