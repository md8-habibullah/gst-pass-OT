//! Q2 disguise commands (`disguise_get`, `disguise_set`) and the production [`TauriLauncher`].
//!
//! Both commands are app-level: they need no account and no unlocked vault, and they never read
//! or write vault state (`storage::app_disguise`). That is also what lets the Stage 1 gate allow
//! them while it is closed.
//!
//! The launcher changes what the app looks like **from Rust**: the main window's title and icon
//! through the Tauri window API, the Linux launcher entry through `disguise::desktop`, the Android
//! launcher alias through a local Kotlin plugin. The webview gets no window permission for any of
//! it: `capabilities/default.json` still grants only `core:default` (a test keeps it that way).

use super::{run, AppState, CmdResult};
use crate::disguise::{DisguiseInput, DisguiseView, Launcher, PresetAssets};
use crate::storage::error::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime, State};

#[cfg(test)]
mod tests;

// ------------------------------------------------------------------ commands

#[tauri::command]
pub async fn disguise_get(state: State<'_, AppState>) -> CmdResult<DisguiseView> {
    run(&state, |c| c.disguise_get()).await
}

#[tauri::command]
pub async fn disguise_set(
    state: State<'_, AppState>,
    input: DisguiseInput,
) -> CmdResult<DisguiseView> {
    run(&state, move |c| c.disguise_set(input)).await
}

/// Install the production launcher and re-apply the saved disguise (called once from the setup
/// hook, after the state was managed). Never fails: a problem is only a note in Settings.
///
/// Not done on Android: the alias that was switched on stays switched on (the system keeps that
/// state), and a blocking call into the Kotlin plugin from the setup hook is not worth the risk.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    state
        .core
        .set_launcher(Arc::new(TauriLauncher::new(app.clone())));
    #[cfg(not(target_os = "android"))]
    {
        let _ = state.core.disguise_apply_saved();
    }
}

// ------------------------------------------------------------------ what the window shows

/// What the main window should show for a look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPlan {
    pub title: String,
    /// The preset whose 512 px icon the window gets; `None`: the bundled default icon.
    pub icon_preset: Option<String>,
}

pub fn window_plan(view: &DisguiseView, preset: Option<&PresetAssets>) -> WindowPlan {
    WindowPlan {
        title: view.applied_name.clone(),
        icon_preset: preset.map(|p| p.preset.id.clone()),
    }
}

// ------------------------------------------------------------------ notes (fixed sentences)

#[cfg(target_os = "linux")]
const NOTE_LINUX: &str = "The window and your launcher entry now use this name and icon. \
Some desktops show the change only after a moment or a new login.";
#[cfg(target_os = "linux")]
const NOTE_LINUX_RESET: &str =
    "The launcher entry was removed and the window shows the original name and icon.";
#[cfg(all(desktop, not(target_os = "linux")))]
const NOTE_OTHER_DESKTOP: &str =
    "Only the window title and icon change on this system; there is no launcher entry to rewrite.";
#[cfg(desktop)]
const NOTE_WINDOW_FAILED: &str = "The window title or icon could not be changed.";
#[cfg(target_os = "android")]
const NOTE_ANDROID: &str = "On Android the launcher shows the icons and names from the list. \
A name of your own changes only the title inside the app and in the recent-apps list, and the \
lower-case option applies inside the app only. Some launchers need a moment to refresh.";

/// The name of the Android `<activity-alias>` for a preset (`None`: the original look). The same
/// rule as `scripts/android_patch_manifest.py`: `Alias` + the id with a capital first letter.
pub fn android_alias_name(preset_id: Option<&str>) -> String {
    match preset_id {
        None => "AliasOriginal".to_string(),
        Some(id) => {
            let mut chars = id.chars();
            let head = chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string())
                .unwrap_or_default();
            format!("Alias{head}{}", chars.as_str())
        }
    }
}

// ------------------------------------------------------------------ the production launcher

/// The launcher the app installs: window title and icon (desktop), the launcher entry (Linux), the
/// launcher alias (Android).
pub struct TauriLauncher<R: Runtime> {
    app: AppHandle<R>,
    /// Tests only: where the Linux launcher folders are and which identifier names the files,
    /// instead of the user's real `$XDG_DATA_HOME` and the app's identifier (also: never run
    /// `update-desktop-database`).
    sandbox: Option<Sandbox>,
}

struct Sandbox {
    data_home: PathBuf,
    identifier: String,
}

impl<R: Runtime> TauriLauncher<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app, sandbox: None }
    }

    /// A launcher that writes its Linux entry under `data_home`, named after `identifier`, instead
    /// of the user's real `$XDG_DATA_HOME` (and never runs `update-desktop-database`). For tests.
    pub fn with_sandbox(app: AppHandle<R>, data_home: PathBuf, identifier: &str) -> Self {
        Self {
            app,
            sandbox: Some(Sandbox {
                data_home,
                identifier: identifier.to_string(),
            }),
        }
    }
}

#[cfg(desktop)]
impl<R: Runtime> TauriLauncher<R> {
    fn window_icon(&self, preset: Option<&PresetAssets>) -> Option<tauri::image::Image<'static>> {
        let decode = |bytes: &[u8]| {
            crate::disguise::decode_png_rgba(bytes)
                .ok()
                .map(|(w, h, px)| tauri::image::Image::new_owned(px, w, h))
        };
        match preset {
            Some(p) => decode(p.png_512),
            None => self
                .app
                .default_window_icon()
                .map(|i| i.clone().to_owned())
                .or_else(|| decode(crate::disguise::ORIGINAL_ICON_PNG)),
        }
    }

    /// Title and icon of the main window. `true` when it worked or there is no such window.
    fn apply_window(&self, view: &DisguiseView, preset: Option<&PresetAssets>) -> bool {
        let Some(window) = self.app.get_webview_window("main") else {
            return true;
        };
        let plan = window_plan(view, preset);
        let mut ok = window.set_title(&plan.title).is_ok();
        if let Some(icon) = self.window_icon(preset) {
            ok &= window.set_icon(icon).is_ok();
        }
        ok
    }

    /// Linux: write or remove the launcher entry. Other systems have nothing to do.
    #[cfg(target_os = "linux")]
    fn apply_menu_entry(
        &self,
        view: &DisguiseView,
        preset: Option<&PresetAssets>,
    ) -> Result<crate::disguise::desktop::Effect> {
        use crate::disguise::desktop::{exec_target, wm_class_of, LauncherFiles};
        let unavailable = || {
            crate::storage::StorageError::Storage(
                "The launcher entry could not be prepared.".into(),
            )
        };
        let (data_home, identifier) = match &self.sandbox {
            Some(s) => (s.data_home.clone(), s.identifier.clone()),
            None => (
                self.app.path().data_dir().map_err(|_| unavailable())?,
                self.app.config().identifier.clone(),
            ),
        };
        let exe = std::env::current_exe().map_err(|_| unavailable())?;
        let appimage = std::env::var_os("APPIMAGE");
        LauncherFiles::new(
            &data_home,
            &identifier,
            &exec_target(&exe, appimage.as_deref()),
            &wm_class_of(&exe),
        )?
        .with_refresh(self.sandbox.is_none())
        .sync(view, preset)
    }

    #[cfg(target_os = "linux")]
    fn apply_platform(
        &self,
        view: &DisguiseView,
        preset: Option<&PresetAssets>,
    ) -> Result<Option<String>> {
        use crate::disguise::desktop::Effect;
        let window_ok = self.apply_window(view, preset);
        // A launcher failure is an error whose message becomes the note; the window was changed
        // regardless.
        let effect = self.apply_menu_entry(view, preset)?;
        let note = if view.is_original() {
            (effect == Effect::Removed).then_some(NOTE_LINUX_RESET)
        } else {
            Some(NOTE_LINUX)
        };
        Ok(match (note, window_ok) {
            (note, true) => note.map(str::to_string),
            (Some(note), false) => Some(format!("{note} {NOTE_WINDOW_FAILED}")),
            (None, false) => Some(NOTE_WINDOW_FAILED.to_string()),
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn apply_platform(
        &self,
        view: &DisguiseView,
        preset: Option<&PresetAssets>,
    ) -> Result<Option<String>> {
        let _ = &self.sandbox;
        let window_ok = self.apply_window(view, preset);
        Ok(match (view.is_original(), window_ok) {
            (_, false) => Some(NOTE_WINDOW_FAILED.to_string()),
            (true, true) => None,
            (false, true) => Some(NOTE_OTHER_DESKTOP.to_string()),
        })
    }
}

/// **UNVERIFIED**: written against the `tauri` 2.12.7 mobile plugin API and never compiled or run
/// (no Android SDK on the build machine). Enables the chosen `<activity-alias>` and disables the
/// others through the local `DisguisePlugin` (`scripts/android_patch_manifest.py` writes it).
#[cfg(target_os = "android")]
impl<R: Runtime> TauriLauncher<R> {
    fn apply_platform(
        &self,
        view: &DisguiseView,
        preset: Option<&PresetAssets>,
    ) -> Result<Option<String>> {
        let _ = &self.sandbox;
        let Some(plugin) = self.app.try_state::<AndroidDisguise<R>>() else {
            return Err(crate::storage::StorageError::Storage(
                "The launcher icon could not be switched: the Android launcher patch is not part of this build."
                    .into(),
            ));
        };
        let alias = android_alias_name(preset.map(|p| p.preset.id.as_str()));
        plugin
            .0
            .run_mobile_plugin::<()>(
                "setAlias",
                serde_json::json!({ "alias": alias, "label": view.applied_name }),
            )
            .map_err(|_| {
                crate::storage::StorageError::Storage(
                    "The launcher icon could not be switched.".into(),
                )
            })?;
        Ok(Some(NOTE_ANDROID.to_string()))
    }
}

/// Neither desktop nor Android (iOS is not a target): nothing to change.
#[cfg(not(any(desktop, target_os = "android")))]
impl<R: Runtime> TauriLauncher<R> {
    fn apply_platform(
        &self,
        _view: &DisguiseView,
        _preset: Option<&PresetAssets>,
    ) -> Result<Option<String>> {
        let _ = (&self.app, &self.sandbox);
        Ok(None)
    }
}

impl<R: Runtime> Launcher for TauriLauncher<R> {
    fn apply(&self, view: &DisguiseView, preset: Option<&PresetAssets>) -> Result<Option<String>> {
        self.apply_platform(view, preset)
    }

    fn standing_note(&self) -> Option<String> {
        #[cfg(target_os = "android")]
        {
            Some(NOTE_ANDROID.to_string())
        }
        #[cfg(not(target_os = "android"))]
        {
            None
        }
    }
}

// ------------------------------------------------------------------ Android plugin registration

/// **UNVERIFIED** (see above). The handle to the Kotlin `DisguisePlugin`.
#[cfg(target_os = "android")]
pub struct AndroidDisguise<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// **UNVERIFIED** (see above). Registers the Kotlin `DisguisePlugin` that
/// `scripts/android_patch_manifest.py` writes into the generated Android project (package =
/// the app identifier). A missing class (the script was not run) is not fatal: the app starts,
/// and `apply` reports that the launcher icon could not be switched.
#[cfg(target_os = "android")]
pub fn android_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("disguise")
        .setup(|app, api| {
            if let Ok(handle) =
                api.register_android_plugin(&app.config().identifier, "DisguisePlugin")
            {
                app.manage(AndroidDisguise(handle));
            }
            Ok(())
        })
        .build()
}
