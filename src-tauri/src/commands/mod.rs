//! Tauri commands: the IPC surface of `ai-docs/IPC_CONTRACT.md`.
//!
//! This layer is deliberately thin: it moves arguments into [`Session`] and results back out.
//! Every command is `async` and runs on the blocking pool, so an Argon2id check (the re-prompt
//! guard, ~0.3-1 s by design) never freezes the UI thread. Arguments arrive camelCase from
//! JavaScript (`expectedVersion`) and are snake_case here (`expected_version`).
//!
//! Rules enforced one level down, in `storage::session` / `storage::vault`: nothing returns a
//! key; reveal/edit/history/purge/password-change verify the master password on every call;
//! trashed items expose only id, title and deletion time.

use crate::crypto;
use crate::storage::{
    BeginEdit, ItemDraft, ItemSummary, ItemView, Session, SessionConfig, StorageError, TotpNow,
    TrashEntry, VaultStatus, VersionEntry,
};
use std::path::Path;
use std::sync::Arc;
use tauri::State;

/// Managed state: the one vault session of the app.
pub struct AppState {
    session: Arc<Session>,
}

impl AppState {
    pub fn new(session: Session) -> Self {
        Self {
            session: Arc::new(session),
        }
    }
}

/// Create the data folder (owner-only on Unix) and the session over `<dir>/vault.db`.
pub fn prepare_state(data_dir: &Path) -> std::io::Result<AppState> {
    std::fs::create_dir_all(data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(AppState::new(Session::new(
        data_dir.join("vault.db"),
        SessionConfig::default(),
    )))
}

type CmdResult<T> = Result<T, StorageError>;

/// Run `f` on the blocking pool with the shared session.
async fn run<T, F>(state: &State<'_, AppState>, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Session) -> CmdResult<T> + Send + 'static,
{
    let session = Arc::clone(&state.session);
    tauri::async_runtime::spawn_blocking(move || f(&session))
        .await
        .map_err(|_| StorageError::Storage("A background task failed.".into()))?
}

// ------------------------------------------------------------------ crypto info

#[tauri::command]
pub fn crypto_suite_info() -> crypto::SuiteInfo {
    crypto::SuiteInfo::current()
}

// ------------------------------------------------------------------ vault lifecycle

#[tauri::command]
pub async fn vault_status(state: State<'_, AppState>) -> CmdResult<VaultStatus> {
    run(&state, |s| s.status()).await
}

#[tauri::command]
pub async fn vault_create(state: State<'_, AppState>, password: String) -> CmdResult<()> {
    run(&state, move |s| s.create(password)).await
}

#[tauri::command]
pub async fn vault_unlock(state: State<'_, AppState>, password: String) -> CmdResult<()> {
    run(&state, move |s| s.unlock(password)).await
}

#[tauri::command]
pub async fn vault_lock(state: State<'_, AppState>) -> CmdResult<()> {
    run(&state, |s| {
        s.lock();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn vault_change_password(
    state: State<'_, AppState>,
    old_password: String,
    new_password: String,
) -> CmdResult<()> {
    run(&state, move |s| {
        s.change_password(old_password, new_password)
    })
    .await
}

// ------------------------------------------------------------------ items

#[tauri::command]
pub async fn item_list(state: State<'_, AppState>) -> CmdResult<Vec<ItemSummary>> {
    run(&state, |s| s.item_list()).await
}

#[tauri::command]
pub async fn item_get(state: State<'_, AppState>, id: String) -> CmdResult<ItemView> {
    run(&state, move |s| s.item_get(&id)).await
}

#[tauri::command]
pub async fn item_reveal(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CmdResult<ItemDraft> {
    run(&state, move |s| s.item_reveal(&id, password)).await
}

#[tauri::command]
pub async fn item_begin_edit(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CmdResult<BeginEdit> {
    run(&state, move |s| s.item_begin_edit(&id, password)).await
}

#[tauri::command]
pub async fn item_create(state: State<'_, AppState>, draft: ItemDraft) -> CmdResult<ItemSummary> {
    run(&state, move |s| s.item_create(draft)).await
}

#[tauri::command]
pub async fn item_update(
    state: State<'_, AppState>,
    id: String,
    token: String,
    expected_version: u64,
    draft: ItemDraft,
) -> CmdResult<ItemSummary> {
    run(&state, move |s| {
        s.item_update(&id, &token, expected_version, draft)
    })
    .await
}

#[tauri::command]
pub async fn item_trash(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    run(&state, move |s| s.item_trash(&id)).await
}

#[tauri::command]
pub async fn item_restore(state: State<'_, AppState>, id: String) -> CmdResult<ItemSummary> {
    run(&state, move |s| s.item_restore(&id)).await
}

#[tauri::command]
pub async fn trash_list(state: State<'_, AppState>) -> CmdResult<Vec<TrashEntry>> {
    run(&state, |s| s.trash_list()).await
}

#[tauri::command]
pub async fn item_purge(
    state: State<'_, AppState>,
    id: String,
    password: String,
    confirmation: String,
) -> CmdResult<()> {
    run(&state, move |s| s.item_purge(&id, password, &confirmation)).await
}

// ------------------------------------------------------------------ history

#[tauri::command]
pub async fn history_list(state: State<'_, AppState>, id: String) -> CmdResult<Vec<VersionEntry>> {
    run(&state, move |s| s.history_list(&id)).await
}

#[tauri::command]
pub async fn history_get(
    state: State<'_, AppState>,
    id: String,
    version: u64,
    password: String,
) -> CmdResult<ItemDraft> {
    run(&state, move |s| s.history_get(&id, version, password)).await
}

#[tauri::command]
pub async fn history_restore(
    state: State<'_, AppState>,
    id: String,
    version: u64,
    password: String,
) -> CmdResult<ItemSummary> {
    run(&state, move |s| s.history_restore(&id, version, password)).await
}

// ------------------------------------------------------------------ one-time codes

#[tauri::command]
pub async fn totp_now(state: State<'_, AppState>, id: String) -> CmdResult<TotpNow> {
    run(&state, move |s| s.totp_now(&id)).await
}

/// Every command of the contract, for `Builder::invoke_handler` (and for the mock-runtime tests).
pub fn invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        crypto_suite_info,
        vault_status,
        vault_create,
        vault_unlock,
        vault_lock,
        vault_change_password,
        item_list,
        item_get,
        item_reveal,
        item_begin_edit,
        item_create,
        item_update,
        item_trash,
        item_restore,
        trash_list,
        item_purge,
        history_list,
        history_get,
        history_restore,
        totp_now,
    ]
}

#[cfg(test)]
mod tests;
