//! Stage 1 and the inactivity lock over IPC (`ai-docs/IPC_CONTRACT.md`, "Stage 1 gate").
//!
//! Besides the commands, this file holds [`guarded`], the one place where the gate is enforced:
//! it wraps the whole invoke handler, so **every** command, including one added later, passes
//! through `AppCore::before_command` first. While the gate is closed anything that is not on the
//! allow-list (`gate::ALLOWED_WHILE_CLOSED`) is answered `locked` before its code runs. Any other
//! command also counts as activity for the inactivity timer.

use super::{run, AppState, CmdResult};
use crate::gate::settings::AppSettings;
use crate::gate::{GateConfigInput, GateOutcome, GateStatus};
use crate::storage::StorageError;
use serde_json::Value;
use tauri::ipc::Invoke;
use tauri::{Manager, Runtime, State};
use zeroize::Zeroizing;

/// Wrap an invoke handler with the gate's deny-by-default check.
pub(super) fn guarded<R, H>(inner: H) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static
where
    R: Runtime,
    H: Fn(Invoke<R>) -> bool + Send + Sync + 'static,
{
    move |invoke: Invoke<R>| {
        let name = invoke.message.command().to_string();
        let verdict = invoke
            .message
            .webview_ref()
            .try_state::<AppState>()
            .map(|state| state.core.before_command(&name));
        match verdict {
            Some(Err(refusal)) => {
                invoke.resolver.reject(refusal);
                true
            }
            _ => inner(invoke),
        }
    }
}

#[tauri::command]
pub async fn gate_status(state: State<'_, AppState>) -> CmdResult<GateStatus> {
    run(&state, |c| Ok(c.gate_status())).await
}

/// `input` is what the person typed on the gate screen: 1 to 3 characters answer the decoy
/// maths, 4 or more are a security attempt. It is wiped as soon as the call returns.
#[tauri::command]
pub async fn gate_submit(state: State<'_, AppState>, input: String) -> CmdResult<GateOutcome> {
    let input = Zeroizing::new(input);
    run(&state, move |c| c.gate_submit(input)).await
}

#[tauri::command]
pub async fn gate_biometric(state: State<'_, AppState>) -> CmdResult<GateOutcome> {
    run(&state, |c| c.gate_biometric()).await
}

#[tauri::command]
pub async fn gate_panic(state: State<'_, AppState>) -> CmdResult<GateOutcome> {
    run(&state, |c| c.gate_panic()).await
}

#[tauri::command]
pub async fn gate_close(state: State<'_, AppState>) -> CmdResult<()> {
    run(&state, |c| {
        c.gate_close();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn gate_configure(
    state: State<'_, AppState>,
    input: GateConfigInput,
) -> CmdResult<GateStatus> {
    run(&state, move |c| c.gate_configure(input)).await
}

/// A long sum for the last confirmation of "erase everything". The answer stays in the core.
#[tauri::command]
pub async fn app_reset_challenge(state: State<'_, AppState>) -> CmdResult<String> {
    run(&state, |c| Ok(c.app_reset_challenge())).await
}

/// The manual "erase this device". `confirmed_steps` is the number of confirmations the person
/// went through; anything but exactly 3 is refused. Taken as a loose JSON value so that every
/// wrong value (a string, a negative number, a fraction) is `invalid_input`, not a decoding error.
/// `answer` is the answer to the sum from `app_reset_challenge`; a wrong one is `wrong_answer`.
#[tauri::command]
pub async fn app_reset(
    state: State<'_, AppState>,
    confirmed_steps: Value,
    answer: Option<String>,
) -> CmdResult<()> {
    run(&state, move |c| match confirmed_steps.as_u64() {
        Some(n) => c.app_reset(n, answer.as_deref().unwrap_or("")),
        None => Err(StorageError::invalid(
            "Confirm all three steps to erase this device.",
        )),
    })
    .await
}

/// "Remove Hexis from this device": the same confirmations and sum as `app_reset`, then everything Hexis keeps
/// here is removed (vaults, settings, the web view's leftovers, the launcher entry) and the app closes, so that
/// uninstalling it leaves nothing behind. S3 backups are not touched. Not allowed with the app lock closed.
#[tauri::command]
pub async fn app_prepare_uninstall(
    state: State<'_, AppState>,
    confirmed_steps: Value,
    answer: Option<String>,
) -> CmdResult<()> {
    run(&state, move |c| match confirmed_steps.as_u64() {
        Some(n) => c.app_prepare_uninstall(n, answer.as_deref().unwrap_or("")),
        None => Err(StorageError::invalid(
            "Confirm all three steps to remove Hexis from this device.",
        )),
    })
    .await?;
    state.request_exit();
    Ok(())
}

#[tauri::command]
pub async fn app_settings_get(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    run(&state, |c| Ok(c.app_settings_get())).await
}

#[tauri::command]
pub async fn app_settings_set(
    state: State<'_, AppState>,
    input: AppSettings,
) -> CmdResult<AppSettings> {
    run(&state, move |c| c.app_settings_set(input)).await
}

/// The person is active (the UI sends this on input; see `src/state/activity.ts`).
#[tauri::command]
pub async fn activity_ping(state: State<'_, AppState>) -> CmdResult<()> {
    run(&state, |c| {
        c.activity_ping();
        Ok(())
    })
    .await
}

/// The app went to the background (phones and tablets). See `GateCore::app_leave`.
#[tauri::command]
pub async fn app_leave(state: State<'_, AppState>) -> CmdResult<()> {
    run(&state, |c| {
        c.app_leave();
        Ok(())
    })
    .await
}

/// The app is back on screen. See `GateCore::app_return`.
#[tauri::command]
pub async fn app_return(state: State<'_, AppState>) -> CmdResult<()> {
    run(&state, |c| {
        c.app_return();
        Ok(())
    })
    .await
}
