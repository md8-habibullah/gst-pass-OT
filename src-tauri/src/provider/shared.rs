//! One core per process on Android.
//!
//! The system may start this app's process only to answer a passkey request: then no window exists, and the provider opens
//! the core itself (`provider::jni`). When the person opens the app later in the same process, the window must use that same
//! core, or there would be two of them, two lock states and two copies of the same open vault. The window therefore asks for
//! the core here too (`commands::prepare_state`) and puts its own file dialogs and link opener on it.

use crate::storage::app::AppCore;
use crate::storage::error::StorageError;
use crate::storage::files::NoPicker;
use crate::storage::session::SessionConfig;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

static CORE: OnceLock<Arc<AppCore>> = OnceLock::new();
static OPENING: Mutex<()> = Mutex::new(());

/// The process's core for the data folder `dir`: the one already open, or a new one that is kept for the next caller.
/// A second, different folder is an error (there is one data folder per app), never a second core.
pub fn get_or_open(dir: &Path) -> Result<Arc<AppCore>, StorageError> {
    let _serial = OPENING.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(core) = CORE.get() {
        return if core.data_dir() == dir {
            Ok(Arc::clone(core))
        } else {
            Err(StorageError::Storage(
                "The app's data folder is not the one the passkey service uses.".into(),
            ))
        };
    }
    let core = Arc::new(AppCore::open(
        dir,
        SessionConfig::default(),
        Arc::new(NoPicker),
    )?);
    let _ = CORE.set(Arc::clone(&core));
    Ok(core)
}
