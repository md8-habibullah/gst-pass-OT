//! Fingerprint for Stage 1 (Android; iOS and desktop have none here).
//!
//! A fingerprint can only open the **app shell** (the account list). It never touches a vault key:
//! this module has no access to one, and opening a vault always needs the master password.
//!
//! The core talks to a [`BiometricAuth`] seam, like the file picker and the link opener, so the
//! whole Stage 1 flow is tested on a desktop with a fake. The production implementation on phones
//! is [`PluginBiometric`] (`tauri-plugin-biometric`, called from Rust only).
//!
//! **Not compiled or run here: the machine has no Android toolchain.** Only the thin adapter
//! below is unverified; everything it feeds is covered by tests with a fake provider.

use crate::storage::StorageError;

pub trait BiometricAuth: Send + Sync {
    /// The device has an enrolled biometric that can be used right now.
    fn available(&self) -> bool;
    /// Show the system prompt. `Ok(true)` when the person was recognised, `Ok(false)` when they
    /// cancelled or the sensor did not recognise them.
    fn authenticate(&self) -> Result<bool, StorageError>;
}

/// Desktop: no biometric of any kind.
pub struct NoBiometric;

impl BiometricAuth for NoBiometric {
    fn available(&self) -> bool {
        false
    }
    fn authenticate(&self) -> Result<bool, StorageError> {
        Ok(false)
    }
}

/// `tauri-plugin-biometric` behind the seam (phones only). UNVERIFIED: not built here.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub struct PluginBiometric<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
impl<R: tauri::Runtime> PluginBiometric<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
impl<R: tauri::Runtime> BiometricAuth for PluginBiometric<R> {
    fn available(&self) -> bool {
        use tauri_plugin_biometric::BiometricExt;
        self.app
            .biometric()
            .status()
            .map(|s| s.is_available)
            .unwrap_or(false)
    }

    fn authenticate(&self) -> Result<bool, StorageError> {
        use tauri_plugin_biometric::{AuthOptions, BiometricExt};
        // Biometrics only: the device PIN or pattern is not accepted as a stand-in.
        let options = AuthOptions {
            allow_device_credential: false,
            cancel_title: Some("Cancel".into()),
            ..AuthOptions::default()
        };
        Ok(self
            .app
            .biometric()
            .authenticate("Open the account list".into(), options)
            .is_ok())
    }
}
