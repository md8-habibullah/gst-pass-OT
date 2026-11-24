//! Tauri commands of the importer / exporter (slice Q4). Thin wrappers over `AppCore`: the
//! webview sends handles and options, never a path, and gets back only counts and titles.
//! Every command needs an unlocked vault and runs on the blocking pool (file reads, a KeePass
//! key derivation and the native dialogs all block).

use super::{run, AppState, CmdResult};
use crate::importer::{
    CsvInspect, ExportedPath, ImportOptions, ImportOutcome, ImportPick, ImportPreview, KeyfilePick,
};
use tauri::State;

#[tauri::command]
pub async fn import_pick(state: State<'_, AppState>) -> CmdResult<Option<ImportPick>> {
    run(&state, |c| c.import_pick()).await
}

#[tauri::command]
pub async fn import_pick_keyfile(state: State<'_, AppState>) -> CmdResult<Option<KeyfilePick>> {
    run(&state, |c| c.import_pick_keyfile()).await
}

#[tauri::command]
pub async fn inspect_csv_headers(
    state: State<'_, AppState>,
    handle: String,
) -> CmdResult<CsvInspect> {
    run(&state, move |c| c.inspect_csv_headers(&handle)).await
}

#[tauri::command]
pub async fn import_preview(
    state: State<'_, AppState>,
    options: ImportOptions,
) -> CmdResult<ImportPreview> {
    run(&state, move |c| c.import_preview(options)).await
}

#[tauri::command]
pub async fn import_commit(
    state: State<'_, AppState>,
    options: ImportOptions,
) -> CmdResult<ImportOutcome> {
    run(&state, move |c| c.import_commit(options)).await
}

#[tauri::command]
pub async fn import_cancel(state: State<'_, AppState>, handle: String) -> CmdResult<()> {
    run(&state, move |c| c.import_cancel(&handle)).await
}

/// Writes a **plaintext** file; behind the master-password re-prompt.
#[tauri::command]
pub async fn export_plain(
    state: State<'_, AppState>,
    format: String,
    password: String,
) -> CmdResult<Option<ExportedPath>> {
    run(&state, move |c| c.export_plain(&format, password)).await
}

#[cfg(test)]
#[path = "importer_tests.rs"]
mod tests;
