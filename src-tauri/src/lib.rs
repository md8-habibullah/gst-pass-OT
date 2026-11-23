//! Passman core library: post-quantum, zero-knowledge vault.
//!
//! * [`crypto`]: Phase 1, the cryptographic core (verified in isolation, see `ai-docs/CRYPTO_SPEC.md`)
//! * [`storage`]: Phase 2, local SQLite storage, version history, trash (`ai-docs/STORAGE_AND_SYNC.md`)
//! * [`commands`]: the Tauri IPC surface (`ai-docs/IPC_CONTRACT.md`)

pub mod commands;
pub mod crypto;
pub mod storage;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Linux: ~/.local/share/<identifier>/vault.db; Android: the app-private data dir.
            let dir = app.path().app_data_dir()?;
            app.manage(commands::prepare_state(&dir)?);
            Ok(())
        })
        .invoke_handler(commands::invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running Passman");
}
