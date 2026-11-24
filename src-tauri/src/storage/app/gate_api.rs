//! `AppCore` and the Stage 1 gate (`crate::gate`).
//!
//! This is a child module of `app` so it can reach the core's private state, and it is the only
//! place that connects the Tauri-free gate to the account registry and the open session:
//!
//! * the **wipe**: drop the session (keys zeroized, backup worker joined), erase every known file
//!   (`gate::wipe`, cryptographic erase first), then forget the gate and the account list in
//!   memory so the next `profile_list` is empty;
//! * **`gate_configure`**: needs an unlocked vault and its master password (the re-prompt guard);
//! * **`gate_close`** and the **idle lock**: back to Stage 1 *and* the vault locked;
//! * the neutral `vault_status` answer while the gate is closed (see `AppCore::status`).
//!
//! Lock order: the gate's `attempt` lock, then `ops`, `registry`, `active` (`app.rs`).

use super::{relock, Active, AppCore};
use crate::gate::settings::AppSettings;
use crate::gate::wipe::{self, NoObserver, WipeObserver, WipePlan, WipeReport};
use crate::gate::{GateConfigInput, GateCore, GateOutcome, GateStatus, LockVault};
use crate::storage::error::{Result, StorageError};
use crate::storage::profiles::{Entry, Registry};
use crate::storage::session::SessionConfig;
use std::path::Path;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

/// Build the gate for a data folder. The idle timer's action locks whatever is open: it holds the
/// same `active` cell the core uses, so it can never lock a session the core no longer owns.
pub(super) fn open_gate(
    dir: &Path,
    cfg: &SessionConfig,
    active: &Arc<Mutex<Option<Active>>>,
) -> Result<GateCore> {
    let active = Arc::clone(active);
    let lock_vault: LockVault = Arc::new(move || {
        let session = relock(&active).as_ref().map(|a| Arc::clone(&a.session));
        if let Some(session) = session {
            session.lock();
        }
    });
    GateCore::open(dir, &cfg.kdf, lock_vault)
}

impl AppCore {
    /// The gate, for the command layer and tests.
    pub fn gate(&self) -> &GateCore {
        &self.gate
    }

    /// The gate is on and has not been opened in this run: only the allow-listed commands work.
    pub(super) fn gate_is_closed(&self) -> bool {
        self.gate.is_closed()
    }

    /// Runs before every IPC command (see `commands::gate::guarded`).
    pub fn before_command(&self, cmd: &str) -> Result<()> {
        self.gate.before_command(cmd)
    }

    pub fn gate_status(&self) -> GateStatus {
        self.gate.status()
    }

    /// Typed text on the gate screen. The text is wiped when this returns.
    pub fn gate_submit(&self, input: Zeroizing<String>) -> Result<GateOutcome> {
        self.gate.submit(&input, &|| self.wipe_unserialised())
    }

    pub fn gate_biometric(&self) -> Result<GateOutcome> {
        self.gate.biometric()
    }

    /// The discreet lock-screen mark: the same silent wipe as a panic credential.
    pub fn gate_panic(&self) -> Result<GateOutcome> {
        self.gate.panic(&|| self.wipe_unserialised())
    }

    /// Back to Stage 1 (when it is on) and lock the open vault: keys zeroized, backup worker
    /// stopped. Works with the gate off too (then it is just a lock).
    pub fn gate_close(&self) {
        self.gate.close();
        self.lock();
    }

    /// `gate_configure`: needs the open vault and its master password.
    pub fn gate_configure(&self, input: GateConfigInput) -> Result<GateStatus> {
        let session = self.session()?;
        if !session.status()?.unlocked {
            return Err(StorageError::Locked);
        }
        let verify = |password: String| session.confirm_password(password);
        let status = self.gate.configure(input, &verify)?;
        self.gate.ping();
        Ok(status)
    }

    pub fn app_settings_get(&self) -> AppSettings {
        self.gate.settings()
    }

    pub fn app_settings_set(&self, settings: AppSettings) -> Result<AppSettings> {
        self.gate.set_settings(settings)
    }

    /// The person is active; restart the inactivity timer.
    pub fn activity_ping(&self) {
        self.gate.ping();
    }

    /// The app went to the background: start the "after leaving" timer (see `GateCore::app_leave`).
    pub fn app_leave(&self) {
        self.gate.app_leave();
    }

    /// The app is back on screen: lock now if the allowed time away has passed (see `GateCore::app_return`).
    pub fn app_return(&self) {
        self.gate.app_return();
    }

    // ------------------------------------------------------------------ the wipe

    /// `app_reset_challenge`: a long sum for the last confirmation of "erase everything". Allowed with the gate
    /// closed (the reset itself is). The answer stays here; only the text goes to the screen.
    pub fn app_reset_challenge(&self) -> String {
        self.gate.reset_challenge()
    }

    /// `app_reset`: the manual "erase this device" (three confirmations in the UI, the last one a long sum).
    /// Allowed with the gate closed. Waits for a security attempt in flight instead of racing it.
    /// `wrong_answer` when the sum was answered wrongly (it is used up: ask for a new one).
    pub fn app_reset(&self, confirmed_steps: u64, answer: &str) -> Result<()> {
        if confirmed_steps != 3 {
            return Err(StorageError::invalid(
                "Confirm all three steps to erase this device.",
            ));
        }
        if !self.gate.check_reset_answer(answer) {
            return Err(StorageError::WrongAnswer);
        }
        self.gate.exclusive(|| self.wipe_unserialised())
    }

    /// `app_prepare_uninstall`: "remove everything Hexis keeps on this device, so that uninstalling leaves
    /// nothing behind". The same three confirmations and the same sum as `app_reset`, then the same wipe, and
    /// after it the things the reset keeps on purpose so that it does not announce itself: `device.json`
    /// (time zone, theme, the first-run flag), the web view's leftovers, and the data folder itself when it
    /// is empty. Unlike a reset, nothing new is made afterwards: no demo vault, no default files. The caller
    /// closes the app. Remote S3 copies are never touched. Needs the gate open (it is not on the allow-list).
    pub fn app_prepare_uninstall(&self, confirmed_steps: u64, answer: &str) -> Result<()> {
        if confirmed_steps != 3 {
            return Err(StorageError::invalid(
                "Confirm all three steps to remove Hexis from this device.",
            ));
        }
        if !self.gate.check_reset_answer(answer) {
            return Err(StorageError::WrongAnswer);
        }
        self.gate.exclusive(|| {
            self.wipe_unserialised()?;
            self.forget_device_files();
            Ok(())
        })
    }

    /// The wipe as the gate's closure uses it: the gate already holds its own serial lock.
    fn wipe_unserialised(&self) -> Result<()> {
        self.wipe_with(&mut NoObserver).map(|_| ())
    }

    /// Erase everything Hexis keeps in the data folder. See `gate::wipe` for the order of the
    /// file steps. In memory, **first** the open session goes (the keys are zeroized and the
    /// backup worker is joined, whatever happens next); **after** the files are gone the gate and
    /// the account list are reset, so a refused wipe never leaves the gate open.
    ///
    /// `Err` means nothing on disk was touched (the pre-flight refused); the vault is locked and
    /// the gate unchanged.
    pub(crate) fn wipe_with(&self, observer: &mut dyn WipeObserver) -> Result<WipeReport> {
        let _ops = relock(&self.ops);
        let open = relock(&self.active).take();
        if let Some(a) = open {
            a.session.lock();
        }
        let plan = {
            let reg = self.reg();
            WipePlan {
                legacy_registered: reg.entries().iter().any(Entry::is_legacy),
                expect_data: !reg.entries().is_empty() || self.gate.is_enabled(),
            }
        };
        let report = wipe::secure_wipe_all_local_data(&self.dir, &plan, observer)
            .map_err(|_| StorageError::Storage("This device could not be erased.".into()))?;
        self.gate.forget_everything();
        *self.reg() = Registry::empty(&self.dir);
        // `device.json` stays (time zone, "setup was done", the time-check switch); only the demo vault's id goes.
        self.forget_demo_if_any();
        // The launcher entry, its icons and the window title live outside the vaults (Q2): put the
        // original look back. It cannot fail the wipe; a launcher problem only leaves a stale entry.
        let _ = self.disguise_wipe();
        self.gate.run_wipe_hooks();
        Ok(report)
    }
}
