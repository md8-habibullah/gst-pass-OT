//! Browsers on a Linux computer (`ai-docs/PASSKEYS.md`, "Browsers on Linux"): the switch, the registration with browsers, and
//! the window's side of a request.
//!
//! The window asks for what is waiting with `bridge_wait` (a long poll, so a request shows at once and nothing runs in
//! between), shows it, and answers with `bridge_answer`. Everything but `bridge_status` and the window's own calls needs the
//! master password to have opened the vault (`require_full`): what the browsers may ask of Hexis is not something the PIN alone
//! may change. On anything but Linux the answer is "not supported".

use super::AppState;
use crate::bridge::{Action, Failure, Status, Waited};
use serde_json::Value;
use tauri::State;

type Reply<T> = Result<T, Failure>;

async fn blocking<T, F>(f: F) -> Reply<T>
where
    T: Send + 'static,
    F: FnOnce() -> Reply<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| Failure::new("storage", "A background task failed."))?
}

#[tauri::command]
pub async fn bridge_status(state: State<'_, AppState>) -> Reply<Status> {
    let bridge = state.bridge().clone();
    blocking(move || Ok(bridge.status())).await
}

/// The switch. On opens the private socket; the setting is kept only if that worked.
#[tauri::command]
pub async fn bridge_set(state: State<'_, AppState>, on: bool) -> Reply<Status> {
    let (bridge, core) = (state.bridge().clone(), state.core());
    blocking(move || {
        core.require_full()?;
        #[cfg(unix)]
        {
            bridge.set_enabled(on)
        }
        #[cfg(not(unix))]
        {
            let _ = (&bridge, on);
            Err(unsupported())
        }
    })
    .await
}

/// Tell the chosen browsers (by id) where Hexis is. Writes the small files described in `bridge::install`.
#[tauri::command]
pub async fn bridge_register(state: State<'_, AppState>, ids: Vec<String>) -> Reply<Status> {
    let (bridge, core) = (state.bridge().clone(), state.core());
    blocking(move || {
        core.require_full()?;
        #[cfg(target_os = "linux")]
        {
            bridge.register(&ids)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (&bridge, &ids);
            Err(unsupported())
        }
    })
    .await
}

/// Take Hexis away from the chosen browsers; none given means all.
#[tauri::command]
pub async fn bridge_unregister(state: State<'_, AppState>, ids: Vec<String>) -> Reply<Status> {
    let (bridge, core) = (state.bridge().clone(), state.core());
    blocking(move || {
        core.require_full()?;
        #[cfg(target_os = "linux")]
        {
            bridge.unregister(&ids)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (&bridge, &ids);
            Err(unsupported())
        }
    })
    .await
}

#[cfg(not(target_os = "linux"))]
fn unsupported() -> Failure {
    Failure::new(
        "unsupported",
        "Browsers can use Hexis only on a Linux computer.",
    )
}

/// What waits for the window: answers when something changed since `after`, or after `ms` (at most 30 seconds).
#[tauri::command]
pub async fn bridge_wait(state: State<'_, AppState>, after: u64, ms: u64) -> Reply<Waited> {
    let bridge = state.bridge().clone();
    blocking(move || Ok(bridge.wait(after, ms))).await
}

/// What the window offers for a waiting request: the passkeys that fit, or the logins a new one can go on.
#[tauri::command]
pub async fn bridge_options(state: State<'_, AppState>, id: String) -> Reply<Value> {
    let bridge = state.bridge().clone();
    blocking(move || bridge.options(&id)).await
}

/// What the person chose for a waiting request.
#[tauri::command]
pub async fn bridge_answer(state: State<'_, AppState>, id: String, action: Action) -> Reply<()> {
    let bridge = state.bridge().clone();
    blocking(move || bridge.answer(&id, action)).await
}
