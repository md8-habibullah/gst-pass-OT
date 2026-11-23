//! Passman core library: post-quantum, zero-knowledge vault.
//!
//! Phase 1 contains the cryptographic core only. See `ai-docs/` for the design.

pub mod crypto;

/// Non-secret description of the active crypto suite, for the UI status page.
#[tauri::command]
fn crypto_suite_info() -> crypto::SuiteInfo {
    crypto::SuiteInfo::current()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![crypto_suite_info])
        .run(tauri::generate_context!())
        .expect("error while running Passman");
}
