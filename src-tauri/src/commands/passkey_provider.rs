//! Settings > Passkeys: can this phone use Hexis as a passkey provider, and the switch (`ai-docs/PASSKEYS.md`).
//!
//! The provider itself (`crate::provider`) is the service Android's Credential Manager asks. It is switched off in the
//! manifest; these commands switch the component on or off through the Kotlin `ProviderPlugin` and open the system's own
//! passkey settings, where the person still has to choose Hexis. Everywhere but Android the answer is "not supported".

use super::{run, AppState, CmdResult};
#[cfg(not(target_os = "android"))]
use crate::storage::StorageError;
use serde::{Deserialize, Serialize};
use tauri::{Runtime, State};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    /// The phone can use a passkey provider (Android 14 or later with the system's Credential Manager).
    pub supported: bool,
    /// The provider component is switched on. The person must still choose it in the system's passkey settings.
    pub enabled: bool,
}

const NOT_ANDROID: ProviderStatus = ProviderStatus {
    supported: false,
    enabled: false,
};

#[tauri::command]
pub async fn passkey_provider_status<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> CmdResult<ProviderStatus> {
    #[cfg(target_os = "android")]
    {
        android::status(&app, "status", serde_json::json!({}))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = &app;
        Ok(NOT_ANDROID)
    }
}

/// Switch the provider component on or off. Needs the vault open with the master password: it changes what the system may
/// ask this app, so it is not something the PIN alone may do.
#[tauri::command]
pub async fn passkey_provider_set<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    on: bool,
) -> CmdResult<ProviderStatus> {
    run(&state, |c| c.require_full()).await?;
    #[cfg(target_os = "android")]
    {
        android::status(&app, "setEnabled", serde_json::json!({ "on": on }))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (&app, on);
        Err(StorageError::Storage(
            "Passkeys can be switched on only on an Android phone.".into(),
        ))
    }
}

/// Open the system's own passkey settings, where Hexis is chosen as a provider.
#[tauri::command]
pub async fn passkey_provider_open_settings<R: Runtime>(app: tauri::AppHandle<R>) -> CmdResult<()> {
    #[cfg(target_os = "android")]
    {
        android::open_settings(&app)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = &app;
        Ok(())
    }
}

/// The handle to the Kotlin `ProviderPlugin` (`scripts/android/ProviderPlugin.kt`).
#[cfg(target_os = "android")]
pub struct AndroidProvider<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Registers the Kotlin plugin that `scripts/android_patch_manifest.py` writes into the generated project. A missing class
/// (the script was not run) is not fatal: the app starts and Settings reports that passkeys are not supported.
#[cfg(target_os = "android")]
pub fn android_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri::Manager;
    tauri::plugin::Builder::new("passkey-provider")
        .setup(|app, api| {
            if let Ok(handle) =
                api.register_android_plugin("dev.habibullah.hexis.provider", "ProviderPlugin")
            {
                app.manage(AndroidProvider(handle));
            }
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
mod android {
    use super::{AndroidProvider, ProviderStatus};
    use crate::storage::StorageError;
    use tauri::{AppHandle, Manager, Runtime};

    fn plugin_error() -> StorageError {
        StorageError::Storage("The phone did not accept that. Try again.".into())
    }

    /// One command of the Kotlin plugin. Without the plugin (the patch script was not run) the phone is "not supported".
    fn run_command<R: Runtime>(
        app: &AppHandle<R>,
        command: &str,
        args: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        let Some(plugin) = app.try_state::<AndroidProvider<R>>() else {
            return Ok(None);
        };
        plugin
            .0
            .run_mobile_plugin::<serde_json::Value>(command, args)
            .map(Some)
            .map_err(|_| plugin_error())
    }

    pub fn status<R: Runtime>(
        app: &AppHandle<R>,
        command: &str,
        args: serde_json::Value,
    ) -> Result<ProviderStatus, StorageError> {
        match run_command(app, command, args)? {
            Some(v) => serde_json::from_value(v).map_err(|_| plugin_error()),
            None => Ok(super::NOT_ANDROID),
        }
    }

    pub fn open_settings<R: Runtime>(app: &AppHandle<R>) -> Result<(), StorageError> {
        run_command(app, "openSettings", serde_json::json!({})).map(|_| ())
    }
}
