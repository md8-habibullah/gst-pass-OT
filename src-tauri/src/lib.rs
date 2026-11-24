//! Hexis core library: post-quantum, zero-knowledge vault.
//!
//! * [`crypto`]: Phase 1, the cryptographic core (verified in isolation, see `ai-docs/CRYPTO_SPEC.md`)
//! * [`storage`]: Phase 2, local SQLite storage, version history, trash, accounts (`ai-docs/STORAGE_AND_SYNC.md`)
//! * [`commands`]: the Tauri IPC surface (`ai-docs/IPC_CONTRACT.md`)
//! * [`gate`]: Stage 1, the optional app gate, the wipe and the inactivity lock (`ai-docs/FEATURE_QUEUE.md` Q1)
//!
//! Backup sync runs on exactly three triggers (`ai-docs/FEATURE_QUEUE.md` Q0): unlock (pull and
//! reconcile), a committed local change (push) and app exit or backgrounding (flush). This file
//! hooks the third one: the window `CloseRequested` event on desktop, and the mobile
//! `Suspended` event (Android `onPause`). There are no timers, no tray icon and no notifications.

pub mod clock;
pub mod commands;
pub mod crypto;
pub mod disguise;
pub mod gate;
pub mod importer;
pub mod storage;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // QR-code scanning (adding a TOTP secret by camera) exists on phones only. The plugin is not
    // even compiled on desktop (`Cargo.toml` target section), and the capability that allows it
    // is limited to android and iOS (`capabilities/mobile.json`).
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    // Switching the launcher alias (the disguise) on Android goes through a small Kotlin plugin that
    // `scripts/android_patch_manifest.py` writes into the generated project. UNVERIFIED on a device.
    #[cfg(target_os = "android")]
    let builder = builder.plugin(commands::disguise::android_plugin());
    // Fingerprint for Stage 1 (the app gate) only, phones only, called from Rust only
    // (`gate/biometric.rs`). It never touches a vault key.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let builder = builder.plugin(tauri_plugin_biometric::init());
    // Android only (UNVERIFIED, no SDK here): `screen_secure` sets FLAG_SECURE through a tiny
    // local plugin, `scripts/android/SecureScreenPlugin.kt`.
    #[cfg(target_os = "android")]
    let builder = builder.plugin(commands::drive::secure_screen::init());
    builder
        // Native open/save dialogs, used from Rust only: the capability file grants the webview
        // no `dialog:*` permission, so it can never open a dialog or name a path itself.
        .plugin(tauri_plugin_dialog::init())
        // Opening web links in the system browser, used from Rust only (`open_url` validates the
        // address first). The injected click handler is switched off, and the capability file
        // grants the webview no `opener:*` permission, so the plugin's own commands stay closed.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .setup(|app| {
            // Linux: ~/.local/share/<identifier>/ holds profiles.json and vaults/<id>/vault.db
            // (plus a single-vault-era vault.db, adopted in place); Android: the app-private dir.
            let dir = app.path().app_data_dir()?;
            // A build made before the final name kept its data under the previous bundle identifier: adopt it.
            storage::legacy::adopt_previous_folder(&dir, &app.config().identifier);
            // Older builds let the web view keep its browser storage in this folder; the window is incognito now
            // (`tauri.conf.json`), so what they left is unused and is removed. Not needed on a phone: the web view
            // keeps its data outside this folder there, and uninstalling removes all of it.
            #[cfg(not(mobile))]
            storage::webview_data::remove(&dir);
            let picker = std::sync::Arc::new(commands::DialogPicker::new(app.handle().clone()));
            let opener = std::sync::Arc::new(commands::PluginOpener::new(app.handle().clone()));
            let state = commands::prepare_state(&dir, picker, opener)?;
            #[cfg(any(target_os = "android", target_os = "ios"))]
            state.core().gate().set_biometric(std::sync::Arc::new(
                gate::biometric::PluginBiometric::new(app.handle().clone()),
            ));
            // Start asking the time servers in the background (checked UTC time, see `clock`).
            state.core().start_clock();
            // The native window follows the chosen theme (title bar, system dialogs). A phone has no such setting.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                let apply = move |theme: storage::device::Theme| {
                    let theme = match theme {
                        storage::device::Theme::System => None,
                        storage::device::Theme::Light => Some(tauri::Theme::Light),
                        storage::device::Theme::Dark => Some(tauri::Theme::Dark),
                    };
                    if let Some(window) = handle.get_webview_window("main") {
                        let _ = window.set_theme(theme);
                    }
                };
                apply(state.core().device_get().theme);
                state.set_theme_hook(apply);
            }
            // "Remove Hexis from this device" ends by closing the app. A moment's wait lets the answer reach the screen first.
            // Not on a phone: Android would show a crash dialog when the process is cut short (see MainActivity.kt.tmpl), and
            // the screen tells the person to uninstall from the system settings instead.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                state.set_exit_hook(move || {
                    let handle = handle.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(900));
                        handle.exit(0);
                    });
                });
            }
            app.manage(state);
            // The disguise: install the launcher (window, launcher entry) and re-apply the saved look.
            commands::disguise::install(app.handle());
            Ok(())
        })
        .on_window_event(on_window_event)
        .invoke_handler(commands::invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building Hexis")
        .run(|app, event| {
            // However the app ends, the open vault is locked first: the keys are zeroized and the
            // sync worker is stopped (the event loop may exit the process without running the
            // destructors of managed state).
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<commands::AppState>() {
                    state.core().lock();
                }
            }
        });
}

/// Sync trigger 3 (`FEATURE_QUEUE.md` Q0): send what is still queued when the app is closed or
/// goes to the background. Never blocks the event loop: the work runs on its own thread.
fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    match event {
        // Desktop: closing the window. Nothing queued, or a flush already running: close at once.
        // Otherwise hold the close back, flush for at most 8 seconds, lock the vault, then close.
        tauri::WindowEvent::CloseRequested { api, .. } => {
            let state = window.state::<commands::AppState>();
            if state.gate().on_close_request(&state.core()) == commands::CloseDecision::Flush {
                api.prevent_close();
                let window = window.clone();
                commands::flush_then(state.core(), state.gate(), move || {
                    let _ = window.destroy();
                });
            }
        }
        // Android `onPause` (tao reports it as `WindowEvent::Suspended`; `onStop` is not forwarded
        // by tauri-runtime-wry 2.12.5). The page's `visibilitychange` calls `sync_flush` as well.
        // The vault stays unlocked: locking after inactivity is the lock timer's job, not this.
        #[cfg(mobile)]
        tauri::WindowEvent::Suspended => {
            let state = window.state::<commands::AppState>();
            commands::flush_in_background(state.core());
        }
        _ => {}
    }
}
