//! Encrypted-drive commands (`file_*`) and `screen_secure` (`ai-docs/IPC_CONTRACT.md`, Files).
//!
//! The webview never sees or sends a path: `file_add` and `file_save_copy` open the native
//! dialogs from Rust, and `file_read` answers with the raw bytes (no JSON, no base64), which is
//! what makes the preview of a 50 MB file practical.

use super::{run, AppState, CmdResult};
use crate::storage::{ExportedFile, FileEntry, StorageError};
use tauri::State;

#[tauri::command]
pub async fn file_list(
    state: State<'_, AppState>,
    item_id: Option<String>,
) -> CmdResult<Vec<FileEntry>> {
    run(&state, move |c| c.file_list(item_id.as_deref())).await
}

/// Opens the native multi-file picker. `null` when it is cancelled.
#[tauri::command]
pub async fn file_add(
    state: State<'_, AppState>,
    item_id: Option<String>,
) -> CmdResult<Option<Vec<FileEntry>>> {
    run(&state, move |c| c.file_add(item_id.as_deref())).await
}

#[tauri::command]
pub async fn file_rename(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> CmdResult<FileEntry> {
    run(&state, move |c| c.file_rename(&id, &name)).await
}

#[tauri::command]
pub async fn file_delete(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CmdResult<()> {
    run(&state, move |c| c.file_delete(&id, password)).await
}

/// The decrypted bytes, as a raw binary IPC response.
#[tauri::command]
pub async fn file_read(
    state: State<'_, AppState>,
    id: String,
) -> Result<tauri::ipc::Response, StorageError> {
    let bytes = run(&state, move |c| c.file_read(&id)).await?;
    // The copy handed to the IPC layer cannot be wiped by us; ours is wiped when `bytes` drops.
    Ok(tauri::ipc::Response::new(bytes.to_vec()))
}

/// Native save dialog, then a streaming decryption to the chosen place. `null` when cancelled.
#[tauri::command]
pub async fn file_save_copy(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CmdResult<Option<ExportedFile>> {
    run(&state, move |c| c.file_save_copy(&id, password)).await
}

/// Keep the window out of screenshots, screen recordings and the recent-apps preview while a
/// sensitive screen is open. A no-op everywhere except Android (`FLAG_SECURE`).
#[tauri::command]
pub async fn screen_secure<R: tauri::Runtime>(app: tauri::AppHandle<R>, on: bool) -> CmdResult<()> {
    #[cfg(target_os = "android")]
    {
        secure_screen::set(&app, on)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (&app, on);
        Ok(())
    }
}

/// Android only (UNVERIFIED: written without an Android SDK; see `scripts/android/`). The Kotlin
/// side is `scripts/android/SecureScreenPlugin.kt`.
#[cfg(target_os = "android")]
pub mod secure_screen {
    use crate::storage::StorageError;
    use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
    use tauri::{AppHandle, Manager, Runtime};

    pub struct SecureScreen<R: Runtime>(PluginHandle<R>);

    pub fn init<R: Runtime>() -> TauriPlugin<R> {
        Builder::new("secure-screen")
            .setup(|app, api| {
                let handle =
                    api.register_android_plugin("app.passman.securescreen", "SecureScreenPlugin")?;
                app.manage(SecureScreen(handle));
                Ok(())
            })
            .build()
    }

    pub fn set<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<(), StorageError> {
        app.state::<SecureScreen<R>>()
            .0
            .run_mobile_plugin::<()>("setSecure", serde_json::json!({ "on": on }))
            .map_err(|_| {
                StorageError::Storage("The screen protection could not be changed.".into())
            })
    }
}
