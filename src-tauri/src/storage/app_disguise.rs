//! Q2: the disguise ("App icon and name") on [`AppCore`].
//!
//! App-level, like `open_url`: none of this needs a vault, a selected account or an unlocked
//! session, and none of it reads or writes vault state. The choice lives in `disguise.json` in
//! the data folder (`crate::disguise`); what happens outside the app (window title, launcher
//! entry, Android alias) is done by a [`Launcher`], which the app installs.

use super::app::AppCore;
use super::error::Result;
use crate::disguise::{self, DisguiseInput, DisguiseView, Launcher, NoLauncher};
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(test)]
mod tests;

/// The launcher, the last note and the lock that keeps two changes from interleaving. One field
/// of [`AppCore`].
pub struct DisguiseRuntime {
    launcher: Mutex<Arc<dyn Launcher>>,
    /// What the launcher said the last time it was applied in this run.
    note: Mutex<Option<String>>,
    /// Held across "save, then apply", so two quick changes cannot save in one order and apply
    /// in the other.
    ops: Mutex<()>,
}

impl Default for DisguiseRuntime {
    fn default() -> Self {
        Self {
            launcher: Mutex::new(Arc::new(NoLauncher)),
            note: Mutex::new(None),
            ops: Mutex::new(()),
        }
    }
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl AppCore {
    /// Use another launcher (the app installs the Tauri one, tests a recording one).
    #[must_use]
    pub fn with_launcher(self, launcher: Arc<dyn Launcher>) -> Self {
        self.set_launcher(launcher);
        self
    }

    /// Install a launcher after the core was built. The production launcher needs the Tauri
    /// `AppHandle`, which only exists once the builder has run, after the core was managed.
    pub fn set_launcher(&self, launcher: Arc<dyn Launcher>) {
        *relock(&self.disguise.launcher) = launcher;
    }

    fn launcher(&self) -> Arc<dyn Launcher> {
        Arc::clone(&relock(&self.disguise.launcher))
    }

    /// Run the launcher for `input` and remember what it said. A launcher that fails is not an
    /// error for the caller: the message becomes the note and the choice stays saved.
    fn disguise_apply(&self, input: &DisguiseInput) -> DisguiseView {
        let view = input.view(None);
        let preset = input.preset_id.as_deref().and_then(disguise::preset);
        let note = match self.launcher().apply(&view, preset) {
            Ok(note) => note,
            Err(e) => Some(e.to_string()),
        };
        *relock(&self.disguise.note) = note.clone();
        input.view(note)
    }

    /// The saved choice, resolved. A missing or damaged `disguise.json` means the original look.
    /// The note is what the launcher said last time in this run, or its standing note.
    pub fn disguise_get(&self) -> Result<DisguiseView> {
        let input = disguise::load(self.data_dir());
        let note = relock(&self.disguise.note)
            .clone()
            .or_else(|| self.launcher().standing_note());
        Ok(input.view(note))
    }

    /// Validate, **save first**, then apply. A bad choice is refused before anything is written;
    /// a launcher that cannot do its job (read-only launcher folder, no window) never loses the
    /// choice: it comes back as the `note`.
    pub fn disguise_set(&self, input: DisguiseInput) -> Result<DisguiseView> {
        let input = input.validated()?;
        let _guard = relock(&self.disguise.ops);
        disguise::save(self.data_dir(), &input)?;
        Ok(self.disguise_apply(&input))
    }

    /// At startup: apply the saved choice again (the window is new every run, and the launcher
    /// entry points at the executable's current location, which can change with an update or a
    /// moved AppImage). Never fails: whatever goes wrong is only a note.
    pub fn disguise_apply_saved(&self) -> DisguiseView {
        let _guard = relock(&self.disguise.ops);
        let input = disguise::load(self.data_dir());
        self.disguise_apply(&input)
    }

    /// For the secure wipe: erase `disguise.json`, remove the launcher entry and icons, and put
    /// the window title and icon back to the original ([`disguise::wipe_hook`]).
    pub fn disguise_wipe(&self) -> Result<()> {
        let _guard = relock(&self.disguise.ops);
        *relock(&self.disguise.note) = None;
        disguise::wipe_hook(self.data_dir(), self.launcher().as_ref())
    }
}
