//! Tauri commands: the IPC surface of `ai-docs/IPC_CONTRACT.md`.
//!
//! This layer is deliberately thin: it moves arguments into [`AppCore`] and results back out.
//! Every command is `async` and runs on the blocking pool, so an Argon2id check (the re-prompt
//! guard, ~0.3-1 s by design) never freezes the UI thread. Arguments arrive camelCase from
//! JavaScript (`expectedVersion`) and are snake_case here (`expected_version`).
//!
//! Rules enforced one level down, in `storage::session` / `storage::vault`: nothing returns a
//! key; reveal/edit/history/purge/password-change verify the master password on every call;
//! trashed items expose only id, title and deletion time.

use crate::crypto;
use crate::storage::{
    AppCore, BackupTargetInput, BackupTargetView, BackupTestResult, BeginEdit, ExportedFile,
    FilePicker, ItemDraft, ItemSummary, ItemView, ProfileInfo, ProfileList, SessionConfig,
    StorageError, SyncFlush, TotpNow, TrashEntry, UrlOpener, VaultStatus, VersionEntry, EXIT_FLUSH,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::State;

pub mod disguise;
pub mod gate;
// The importer / exporter commands (slice Q4).
mod importer;

/// Managed state: the account registry and the one selected profile's vault session.
pub struct AppState {
    core: Arc<AppCore>,
    gate: Arc<ExitGate>,
}

impl AppState {
    pub fn new(core: AppCore) -> Self {
        Self {
            core: Arc::new(core),
            gate: Arc::new(ExitGate::default()),
        }
    }

    /// The shared core (for the window hooks in `lib.rs`).
    pub fn core(&self) -> Arc<AppCore> {
        Arc::clone(&self.core)
    }

    pub fn gate(&self) -> Arc<ExitGate> {
        Arc::clone(&self.gate)
    }
}

/// What a window-close request does (sync trigger 3 on desktop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Let the window close now: nothing is queued, or a flush is already running.
    Close,
    /// Hold the close back, send the queue (bounded), then close.
    Flush,
}

/// Decides what a `CloseRequested` event does and remembers that a flush is running, so a
/// second request during the flush closes at once. Tauri-free, so it is unit-tested.
#[derive(Default)]
pub struct ExitGate {
    flushing: AtomicBool,
}

impl ExitGate {
    pub fn on_close_request(&self, core: &AppCore) -> CloseDecision {
        // A second request while the flush runs: the person insists, close now (the queue is
        // durable, the rest goes at the next trigger).
        if self.flushing.load(Ordering::SeqCst) || !core.has_pending_sync() {
            return CloseDecision::Close;
        }
        if self.flushing.swap(true, Ordering::SeqCst) {
            return CloseDecision::Close;
        }
        CloseDecision::Flush
    }

    pub fn is_flushing(&self) -> bool {
        self.flushing.load(Ordering::SeqCst)
    }
}

/// After a `Flush` decision: on a separate thread (the event loop is never blocked) send the queue
/// for at most [`EXIT_FLUSH`], then lock the vault (the keys are zeroized and the worker is
/// stopped), then run `then` (which closes the window). If no thread can be started the window
/// is closed at once: the queue is durable and the rest goes at the next trigger.
pub fn flush_then(
    core: Arc<AppCore>,
    gate: Arc<ExitGate>,
    then: impl Fn() + Send + Sync + 'static,
) {
    let then = Arc::new(then);
    let (then_in_thread, gate_in_thread) = (Arc::clone(&then), Arc::clone(&gate));
    let spawned = std::thread::Builder::new()
        .name("hexis-exit-flush".into())
        .spawn(move || {
            core.flush_for_exit(EXIT_FLUSH);
            core.lock();
            gate_in_thread.flushing.store(false, Ordering::SeqCst);
            then_in_thread();
        });
    if spawned.is_err() {
        gate.flushing.store(false, Ordering::SeqCst);
        then();
    }
}

/// Phones: the app went to the background. Send the queue (bounded) on a separate thread; the
/// vault stays unlocked. Nothing is lost if the system freezes the process first: the queue is
/// durable and is retried at the next trigger.
pub fn flush_in_background(core: Arc<AppCore>) {
    let _ = std::thread::Builder::new()
        .name("hexis-background-flush".into())
        .spawn(move || {
            core.flush_for_exit(EXIT_FLUSH);
        });
}

/// Open the app's data folder (owner-only on Unix): read the account registry, adopt a vault
/// from the single-vault era if there is one. Nothing is selected or unlocked yet.
pub fn prepare_state(
    data_dir: &Path,
    picker: Arc<dyn FilePicker>,
    opener: Arc<dyn UrlOpener>,
) -> Result<AppState, StorageError> {
    Ok(AppState::new(
        AppCore::open(data_dir, SessionConfig::default(), picker)?.with_opener(opener),
    ))
}

/// The production link opener: `tauri-plugin-opener`, called from Rust only. The webview holds no
/// `opener:*` permission (see `capabilities/default.json`), so the plugin's own commands are
/// closed to it; the only way to open a link is `open_url`, which validates the address first.
pub struct PluginOpener<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> PluginOpener<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> UrlOpener for PluginOpener<R> {
    fn open(&self, url: &str) -> Result<(), StorageError> {
        use tauri_plugin_opener::OpenerExt;
        self.app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|_| StorageError::Storage("The link could not be opened.".into()))
    }
}

/// The production file picker: native open/save dialogs from `tauri-plugin-dialog`, called from
/// Rust only. The webview has no permission for the plugin's own commands (see
/// `capabilities/default.json`), so it can neither open dialogs nor name a path itself.
pub struct DialogPicker<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> DialogPicker<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

fn picked(path: Option<tauri_plugin_dialog::FilePath>) -> Result<Option<PathBuf>, StorageError> {
    path.map(|p| {
        p.into_path().map_err(|_| {
            StorageError::invalid("That location cannot be used. Choose a file on this device.")
        })
    })
    .transpose()
}

impl<R: tauri::Runtime> FilePicker for DialogPicker<R> {
    fn open_vault_file(&self) -> Result<Option<PathBuf>, StorageError> {
        use tauri_plugin_dialog::DialogExt;
        picked(
            self.app
                .dialog()
                .file()
                .set_title("Choose a vault file")
                .add_filter(
                    "Vault file",
                    &["hexis", crate::storage::legacy::PREVIOUS_EXTENSION, "db"],
                )
                .blocking_pick_file(),
        )
    }

    fn save_vault_file(&self, suggested_name: &str) -> Result<Option<PathBuf>, StorageError> {
        use tauri_plugin_dialog::DialogExt;
        picked(
            self.app
                .dialog()
                .file()
                .set_title("Save a copy of your vault")
                .set_file_name(suggested_name)
                .add_filter("Vault file", &["hexis"])
                .blocking_save_file(),
        )
    }

    fn open_files(&self) -> Result<Option<Vec<PathBuf>>, StorageError> {
        use tauri_plugin_dialog::DialogExt;
        self.app
            .dialog()
            .file()
            .set_title("Choose files to add")
            .blocking_pick_files()
            .map(|list| {
                list.into_iter()
                    .map(|p| {
                        p.into_path().map_err(|_| {
                            StorageError::invalid(
                                "That location cannot be used. Choose files on this device.",
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
    }

    fn save_file(&self, suggested_name: &str) -> Result<Option<PathBuf>, StorageError> {
        use tauri_plugin_dialog::DialogExt;
        picked(
            self.app
                .dialog()
                .file()
                .set_title("Save a decrypted copy of this file")
                .set_file_name(suggested_name)
                .blocking_save_file(),
        )
    }

    // Slice Q4: any file type (the importer reads the content, not the name).
    fn open_import_file(&self) -> Result<Option<PathBuf>, StorageError> {
        use tauri_plugin_dialog::DialogExt;
        picked(
            self.app
                .dialog()
                .file()
                .set_title("Choose a file exported from another password manager")
                .blocking_pick_file(),
        )
    }

    fn open_keyfile(&self) -> Result<Option<PathBuf>, StorageError> {
        use tauri_plugin_dialog::DialogExt;
        picked(
            self.app
                .dialog()
                .file()
                .set_title("Choose the KeePass key file")
                .blocking_pick_file(),
        )
    }

    fn save_export_file(&self, suggested_name: &str) -> Result<Option<PathBuf>, StorageError> {
        use tauri_plugin_dialog::DialogExt;
        picked(
            self.app
                .dialog()
                .file()
                .set_title("Save a plaintext export (this file is not encrypted)")
                .set_file_name(suggested_name)
                .blocking_save_file(),
        )
    }
}

pub mod drive;

type CmdResult<T> = Result<T, StorageError>;

/// Run `f` on the blocking pool with the shared app core (Argon2id checks and native dialogs
/// both block, so neither may run on the UI thread or hold a lock).
async fn run<T, F>(state: &State<'_, AppState>, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&AppCore) -> CmdResult<T> + Send + 'static,
{
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || f(&core))
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
    run(&state, |c| c.status()).await
}

// ------------------------------------------------------------------ accounts (profiles)

#[tauri::command]
pub async fn profile_list(state: State<'_, AppState>) -> CmdResult<ProfileList> {
    run(&state, |c| c.profile_list()).await
}

#[tauri::command]
pub async fn profile_create(
    state: State<'_, AppState>,
    display_name: String,
    password: String,
) -> CmdResult<ProfileInfo> {
    run(&state, move |c| c.profile_create(&display_name, password)).await
}

#[tauri::command]
pub async fn profile_select(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    run(&state, move |c| c.profile_select(&id)).await
}

#[tauri::command]
pub async fn profile_rename(
    state: State<'_, AppState>,
    id: String,
    display_name: String,
) -> CmdResult<ProfileInfo> {
    run(&state, move |c| c.profile_rename(&id, &display_name)).await
}

#[tauri::command]
pub async fn profile_remove(
    state: State<'_, AppState>,
    id: String,
    password: String,
    confirmation: String,
) -> CmdResult<()> {
    run(&state, move |c| {
        c.profile_remove(&id, password, &confirmation)
    })
    .await
}

#[tauri::command]
pub async fn profile_import_file(
    state: State<'_, AppState>,
    display_name: String,
) -> CmdResult<Option<ProfileInfo>> {
    run(&state, move |c| c.profile_import_file(&display_name)).await
}

#[tauri::command]
pub async fn vault_export_file(
    state: State<'_, AppState>,
    password: String,
) -> CmdResult<Option<ExportedFile>> {
    run(&state, move |c| c.vault_export_file(password)).await
}

#[tauri::command]
pub async fn vault_unlock(state: State<'_, AppState>, password: String) -> CmdResult<()> {
    run(&state, move |c| c.unlock(password)).await
}

#[tauri::command]
pub async fn vault_lock(state: State<'_, AppState>) -> CmdResult<()> {
    run(&state, |c| {
        c.lock();
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
    run(&state, move |c| {
        c.change_password(old_password, new_password)
    })
    .await
}

// ------------------------------------------------------------------ items

#[tauri::command]
pub async fn item_list(state: State<'_, AppState>) -> CmdResult<Vec<ItemSummary>> {
    run(&state, |c| c.item_list()).await
}

#[tauri::command]
pub async fn item_get(state: State<'_, AppState>, id: String) -> CmdResult<ItemView> {
    run(&state, move |c| c.item_get(&id)).await
}

#[tauri::command]
pub async fn item_reveal(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CmdResult<ItemDraft> {
    run(&state, move |c| c.item_reveal(&id, password)).await
}

#[tauri::command]
pub async fn item_begin_edit(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CmdResult<BeginEdit> {
    run(&state, move |c| c.item_begin_edit(&id, password)).await
}

#[tauri::command]
pub async fn item_create(state: State<'_, AppState>, draft: ItemDraft) -> CmdResult<ItemSummary> {
    run(&state, move |c| c.item_create(draft)).await
}

#[tauri::command]
pub async fn item_update(
    state: State<'_, AppState>,
    id: String,
    token: String,
    expected_version: u64,
    draft: ItemDraft,
) -> CmdResult<ItemSummary> {
    run(&state, move |c| {
        c.item_update(&id, &token, expected_version, draft)
    })
    .await
}

#[tauri::command]
pub async fn item_trash(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    run(&state, move |c| c.item_trash(&id)).await
}

#[tauri::command]
pub async fn item_restore(state: State<'_, AppState>, id: String) -> CmdResult<ItemSummary> {
    run(&state, move |c| c.item_restore(&id)).await
}

/// "Got it" for the copy another device sent: clears `changedElsewhere` on the item. (Restoring
/// such a copy from the history clears it too.)
#[tauri::command]
pub async fn item_review_other_device(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    run(&state, move |c| c.item_review_other_device(&id)).await
}

#[tauri::command]
pub async fn item_archive(state: State<'_, AppState>, id: String) -> CmdResult<ItemSummary> {
    run(&state, move |c| c.item_archive(&id)).await
}

#[tauri::command]
pub async fn item_unarchive(state: State<'_, AppState>, id: String) -> CmdResult<ItemSummary> {
    run(&state, move |c| c.item_unarchive(&id)).await
}

#[tauri::command]
pub async fn item_clone(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CmdResult<ItemSummary> {
    run(&state, move |c| c.item_clone(&id, password)).await
}

#[tauri::command]
pub async fn trash_list(state: State<'_, AppState>) -> CmdResult<Vec<TrashEntry>> {
    run(&state, |c| c.trash_list()).await
}

#[tauri::command]
pub async fn item_purge(
    state: State<'_, AppState>,
    id: String,
    password: String,
    confirmation: String,
) -> CmdResult<()> {
    run(&state, move |c| c.item_purge(&id, password, &confirmation)).await
}

// ------------------------------------------------------------------ history

#[tauri::command]
pub async fn history_list(state: State<'_, AppState>, id: String) -> CmdResult<Vec<VersionEntry>> {
    run(&state, move |c| c.history_list(&id)).await
}

#[tauri::command]
pub async fn history_get(
    state: State<'_, AppState>,
    id: String,
    version: u64,
    password: String,
) -> CmdResult<ItemDraft> {
    run(&state, move |c| c.history_get(&id, version, password)).await
}

#[tauri::command]
pub async fn history_restore(
    state: State<'_, AppState>,
    id: String,
    version: u64,
    password: String,
) -> CmdResult<ItemSummary> {
    run(&state, move |c| c.history_restore(&id, version, password)).await
}

// ------------------------------------------------------------------ one-time codes

#[tauri::command]
pub async fn totp_now(state: State<'_, AppState>, id: String) -> CmdResult<TotpNow> {
    run(&state, move |c| c.totp_now(&id)).await
}

// ------------------------------------------------------------------ backup targets (Phase 4a)

#[tauri::command]
pub async fn backup_list(state: State<'_, AppState>) -> CmdResult<Vec<BackupTargetView>> {
    run(&state, |c| c.backup_list()).await
}

#[tauri::command]
pub async fn backup_test(
    state: State<'_, AppState>,
    input: BackupTargetInput,
) -> CmdResult<BackupTestResult> {
    run(&state, move |c| c.backup_test(&input)).await
}

#[tauri::command]
pub async fn backup_add(
    state: State<'_, AppState>,
    input: BackupTargetInput,
) -> CmdResult<BackupTargetView> {
    run(&state, move |c| c.backup_add(input)).await
}

#[tauri::command]
pub async fn backup_update(
    state: State<'_, AppState>,
    id: String,
    input: BackupTargetInput,
) -> CmdResult<BackupTargetView> {
    run(&state, move |c| c.backup_update(&id, input)).await
}

#[tauri::command]
pub async fn backup_remove(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    run(&state, move |c| c.backup_remove(&id)).await
}

#[tauri::command]
pub async fn backup_set_master(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    run(&state, move |c| c.backup_set_master(&id)).await
}

#[tauri::command]
pub async fn backup_run(
    state: State<'_, AppState>,
    id: Option<String>,
) -> CmdResult<Vec<BackupTargetView>> {
    run(&state, move |c| c.backup_run(id.as_deref())).await
}

#[tauri::command]
pub async fn backup_reveal_secret(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CmdResult<String> {
    run(&state, move |c| c.backup_reveal_secret(&id, password)).await
}

#[tauri::command]
pub async fn profile_restore_from_backup(
    state: State<'_, AppState>,
    display_name: String,
    input: BackupTargetInput,
) -> CmdResult<ProfileInfo> {
    run(&state, move |c| {
        c.profile_restore_from_backup(&display_name, input)
    })
    .await
}

/// Trigger 3 for phones: the webview calls this when the page goes to the background
/// (`visibilitychange` hidden). Sends the queue for at most 8 seconds in total; with the vault
/// locked it does nothing and answers `{ complete: true }`.
#[tauri::command]
pub async fn sync_flush(state: State<'_, AppState>) -> CmdResult<SyncFlush> {
    run(&state, |c| Ok(c.sync_flush())).await
}

// ------------------------------------------------------------------ links

#[tauri::command]
pub async fn open_url(state: State<'_, AppState>, url: String) -> CmdResult<()> {
    run(&state, move |c| c.open_url(&url)).await
}

/// Every command of the contract, for `Builder::invoke_handler` (and for the mock-runtime tests).
///
/// The whole list runs behind [`gate::guarded`]: while the Stage 1 gate is closed only its
/// allow-list answers, everything else (this list and any command added later) is `locked`.
pub fn invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    gate::guarded(tauri::generate_handler![
        crypto_suite_info,
        vault_status,
        profile_list,
        profile_create,
        profile_select,
        profile_rename,
        profile_remove,
        profile_import_file,
        vault_export_file,
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
        item_review_other_device,
        item_archive,
        item_unarchive,
        item_clone,
        trash_list,
        item_purge,
        history_list,
        history_get,
        history_restore,
        totp_now,
        backup_list,
        backup_test,
        backup_add,
        backup_update,
        backup_remove,
        backup_set_master,
        backup_run,
        backup_reveal_secret,
        profile_restore_from_backup,
        sync_flush,
        open_url,
        disguise::disguise_get,
        disguise::disguise_set,
        gate::gate_status,
        gate::gate_submit,
        gate::gate_biometric,
        gate::gate_panic,
        gate::gate_close,
        gate::gate_configure,
        gate::app_reset,
        gate::app_settings_get,
        gate::app_settings_set,
        gate::activity_ping,
        drive::file_list,
        drive::file_add,
        drive::file_rename,
        drive::file_delete,
        drive::file_read,
        drive::file_save_copy,
        drive::screen_secure,
        importer::import_pick,
        importer::import_pick_keyfile,
        importer::inspect_csv_headers,
        importer::import_preview,
        importer::import_commit,
        importer::import_cancel,
        importer::export_plain,
    ])
}

#[cfg(test)]
mod drive_tests;
#[cfg(test)]
mod tests;
