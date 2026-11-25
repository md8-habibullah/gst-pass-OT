//! Device settings, the trusted clock and the demo vault over IPC (`ai-docs/IPC_CONTRACT.md`, "Device and time").
//!
//! `device_get`, `clock_status` and `clock_sync` also work while Stage 1 is closed (the gate screen shows
//! a countdown and may need a time check). The setters and the demo commands do not: the guard in
//! `commands/gate.rs` answers `locked` for everything that is not on `gate::ALLOWED_WHILE_CLOSED`.

use super::{run, AppState, CmdResult};
use crate::clock::ClockStatus;
use crate::storage::device::{DeviceInfo, Theme};
use crate::storage::ProfileInfo;
use tauri::State;

#[tauri::command]
pub async fn device_get(state: State<'_, AppState>) -> CmdResult<DeviceInfo> {
    run(&state, |c| Ok(c.device_get())).await
}

#[tauri::command]
pub async fn device_set_time_zone(
    state: State<'_, AppState>,
    zone: String,
) -> CmdResult<DeviceInfo> {
    run(&state, move |c| c.device_set_time_zone(&zone)).await
}

#[tauri::command]
pub async fn device_finish_setup(state: State<'_, AppState>) -> CmdResult<DeviceInfo> {
    run(&state, |c| c.device_finish_setup()).await
}

#[tauri::command]
pub async fn device_set_time_check(state: State<'_, AppState>, on: bool) -> CmdResult<DeviceInfo> {
    run(&state, move |c| c.device_set_time_check(on)).await
}

/// `system`, `light` or `dark`. Also tells the native window, so the title bar and the system dialogs follow.
#[tauri::command]
pub async fn device_set_theme(state: State<'_, AppState>, theme: Theme) -> CmdResult<DeviceInfo> {
    let info = run(&state, move |c| c.device_set_theme(theme)).await?;
    state.apply_theme(theme);
    Ok(info)
}

/// How long a copied secret stays on the clipboard: 15, 30 or 60 seconds.
#[tauri::command]
pub async fn device_set_clipboard_clear(
    state: State<'_, AppState>,
    seconds: u32,
) -> CmdResult<DeviceInfo> {
    run(&state, move |c| c.device_set_clipboard_clear(seconds)).await
}

/// Open the vault with the cursor in the search box (and the keyboard up on a phone).
#[tauri::command]
pub async fn device_set_open_in_search(
    state: State<'_, AppState>,
    on: bool,
) -> CmdResult<DeviceInfo> {
    run(&state, move |c| c.device_set_open_in_search(on)).await
}

/// May Hexis ask an app's website whether that app may use its passkeys (Settings > Passkeys)? On by default.
#[tauri::command]
pub async fn device_set_passkey_app_check(
    state: State<'_, AppState>,
    on: bool,
) -> CmdResult<DeviceInfo> {
    run(&state, move |c| c.device_set_passkey_app_check(on)).await
}

/// The order the item list starts in (one of the names the core knows).
#[tauri::command]
pub async fn device_set_list_sort(
    state: State<'_, AppState>,
    order: String,
) -> CmdResult<DeviceInfo> {
    run(&state, move |c| c.device_set_list_sort(&order)).await
}

#[tauri::command]
pub async fn clock_status(state: State<'_, AppState>) -> CmdResult<ClockStatus> {
    run(&state, |c| Ok(c.clock_status())).await
}

#[tauri::command]
pub async fn clock_sync(state: State<'_, AppState>) -> CmdResult<ClockStatus> {
    run(&state, |c| c.clock_sync()).await
}

/// Make the demo vault when the app holds no vault. Leaves it selected and unlocked.
#[tauri::command]
pub async fn demo_spawn(state: State<'_, AppState>) -> CmdResult<ProfileInfo> {
    run(&state, |c| c.demo_spawn()).await
}

/// Select and unlock the demo vault without a typed password.
#[tauri::command]
pub async fn demo_open(state: State<'_, AppState>) -> CmdResult<()> {
    run(&state, |c| c.demo_open()).await
}
