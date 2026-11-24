//! Device settings, the trusted clock and the demo vault over IPC (`ai-docs/IPC_CONTRACT.md`, "Device and time").
//!
//! `device_get`, `clock_status` and `clock_sync` also work while Stage 1 is closed (the gate screen shows
//! a countdown and may need a time check). The setters and the demo commands do not: the guard in
//! `commands/gate.rs` answers `locked` for everything that is not on `gate::ALLOWED_WHILE_CLOSED`.

use super::{run, AppState, CmdResult};
use crate::clock::ClockStatus;
use crate::storage::device::DeviceInfo;
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
