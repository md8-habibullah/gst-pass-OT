//! Stage 1: the optional app gate, the decoy maths, the attempt counter, the panic wipe and the
//! inactivity timer (`ai-docs/FEATURE_QUEUE.md` Q1, `ARCHITECTURE.md` §5 and D13 to D16).
//!
//! **Stage 1 holds no key and unlocks no secret.** It decides whether the account list may be
//! shown. Opening a vault always needs the master password; nothing here wraps, caches or derives
//! the `VaultRootKey`, and nothing here can reach one.
//!
//! * [`credential`]: the rules, Argon2id PHC hashes, the equal-cost three-way verification
//! * [`counter`]: `gate.json`, the write-ahead tamper-evident attempt counter
//! * [`equation`]: the decoy maths, built from exact integer arithmetic
//! * [`wipe`](mod@wipe): `secure_wipe_all_local_data` (cryptographic erase first, known paths only)
//! * [`idle`]: the inactivity timer thread; [`settings`]: `app-settings.json`
//! * [`biometric`]: the fingerprint seam (Android only)
//!
//! [`GateCore`] is Tauri-free and owned by `AppCore`; `storage/app/gate_api.rs` connects it to the
//! account registry and the open session, and `commands/gate.rs` is the IPC layer.

pub mod biometric;
pub mod counter;
pub mod credential;
pub mod equation;
pub mod idle;
pub mod settings;
pub mod wipe;

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod tests;

use self::biometric::{BiometricAuth, NoBiometric};
use self::counter::{CounterRead, Loaded, Persist};
use self::credential::{Decision, Normalised, VerifyCounter};
use self::equation::{Equation, OsRand, Rand};
use self::idle::IdleTimer;
use self::settings::AppSettings;
use crate::crypto::KdfParams;
use crate::storage::StorageError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The 15th failed security attempt erases the device.
pub const MAX_ATTEMPTS: u32 = 15;
/// Typed text of this many characters or fewer is an answer to the maths, never a credential.
pub const MATH_MAX_CHARS: usize = 3;

/// Commands that still work while the gate is closed. Everything else (every `profile_*`,
/// `vault_*` that reveals or changes data, `item_*`, `trash_*`, `history_*`, `totp_*`, `backup_*`,
/// `open_url`, `file_*`, `import_*`, `export_*`, ...) answers `locked`: the guard is deny-by-default,
/// so a command added later is closed unless it is listed here.
///
/// `vault_status` answers a neutral "no account, locked" while closed (the UI polls it).
/// `sync_flush`, `disguise_get`, `screen_secure` and `suite_info` are listed for the slices that
/// add them (the front end's boot calls `disguise_get` before anything else): a name that is
/// listed but not registered in this build simply never arrives.
pub const ALLOWED_WHILE_CLOSED: &[&str] = &[
    "crypto_suite_info",
    "suite_info",
    "vault_status",
    "gate_status",
    "gate_submit",
    "gate_biometric",
    "gate_panic",
    "app_reset",
    "app_settings_get",
    "activity_ping",
    "sync_flush",
    "disguise_get",
    "screen_secure",
];

/// The polling commands: they must not count as activity, or an open window would never time out.
pub const POLLING_COMMANDS: &[&str] = &["vault_status", "gate_status"];

pub fn allowed_while_closed(cmd: &str) -> bool {
    ALLOWED_WHILE_CLOSED.contains(&cmd)
}

pub fn is_polling(cmd: &str) -> bool {
    POLLING_COMMANDS.contains(&cmd)
}

// ------------------------------------------------------------------ wire types

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateMode {
    #[default]
    Pin,
    Password,
    Grid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateStatus {
    pub enabled: bool,
    pub open: bool,
    pub mode: GateMode,
    pub biometric_available: bool,
    pub biometric: bool,
    pub panic_button: bool,
    pub equation: Option<String>,
    pub attempts_left: Option<u32>,
    pub streak: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum GateOutcome {
    Math {
        correct: bool,
        equation: String,
        streak: u32,
    },
    Opened,
    Failed {
        attempts_left: u32,
        equation: String,
    },
    Wiped,
}

/// `gate_configure`'s argument. Wiped when dropped; never printed.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct GateConfigInput {
    #[zeroize(skip)]
    pub enabled: bool,
    #[zeroize(skip)]
    #[serde(default)]
    pub mode: GateMode,
    /// A new credential; `None` keeps the stored one.
    #[serde(default)]
    pub credential: Option<String>,
    #[zeroize(skip)]
    #[serde(default)]
    pub reverse_panic: bool,
    /// `None` keeps the stored custom panic code, `Some("")` removes it.
    #[serde(default)]
    pub panic_code: Option<String>,
    #[zeroize(skip)]
    #[serde(default)]
    pub biometric: bool,
    #[zeroize(skip)]
    #[serde(default)]
    pub panic_button: bool,
    pub master_password: String,
}

impl core::fmt::Debug for GateConfigInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("GateConfigInput(<redacted>)")
    }
}

// ------------------------------------------------------------------ the core

pub type WipeHook = Box<dyn Fn() + Send + Sync>;
/// What the gate calls to erase the device (supplied by `AppCore`). `Err` only when nothing was
/// touched (the pre-flight refused); the gate then stays exactly as it was, closed.
pub type WipeFn<'a> = &'a dyn Fn() -> Result<(), StorageError>;
/// Locks the open vault (keys zeroized, backup worker joined). Supplied by `AppCore`.
pub type LockVault = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy)]
struct Known {
    attempts: u32,
    epoch: u64,
}

struct State {
    /// `None`: the gate is not configured (everything behaves as if there were no gate).
    cfg: Option<Persist>,
    /// Stage 1 passed in this run.
    open: bool,
    known: Known,
    equation: Option<Equation>,
    streak: u32,
}

struct Inner {
    dir: PathBuf,
    /// Argon2id cost of every gate hash, from the vault's `SessionConfig`.
    kdf: KdfParams,
    state: Mutex<State>,
    /// Serialises security attempts, configuration and the wipe.
    attempt: Mutex<()>,
    idle: Arc<IdleTimer>,
    settings: Mutex<AppSettings>,
    unit: Mutex<Duration>,
    biometric: Mutex<Arc<dyn BiometricAuth>>,
    rng: Mutex<Box<dyn Rand + Send>>,
    verifications: VerifyCounter,
    lock_vault: LockVault,
    hooks: Mutex<Vec<WipeHook>>,
    /// Test failpoint: abort a security attempt right after the write-ahead (a force-close).
    #[cfg(test)]
    stop_after_write_ahead: std::sync::atomic::AtomicBool,
}

pub struct GateCore {
    inner: Arc<Inner>,
    watcher: Mutex<Option<JoinHandle<()>>>,
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn invalid(msg: &str) -> StorageError {
    StorageError::invalid(msg)
}

/// A counter that is damaged (missing, unparsable, bad MAC, or older than what this process
/// already wrote) costs one extra attempt on top of the best value known, but never reaches the
/// limit by itself: a corrupt file must not wipe a device. `trusted` is a value this process wrote
/// or a record whose MAC checked out; `hint` is whatever number could be read from a record that
/// failed its MAC. A hint is capped two below the limit, so a forged "15" cannot trigger a wipe.
fn penalised(trusted: u32, hint: Option<u32>) -> u32 {
    let cap = MAX_ATTEMPTS - 2;
    let best = trusted.max(hint.unwrap_or(0).min(cap));
    best.max((best + 1).min(cap))
}

impl Inner {
    fn st(&self) -> MutexGuard<'_, State> {
        relock(&self.state)
    }

    fn next_equation(&self) -> Equation {
        equation::generate(&mut **relock(&self.rng))
    }

    /// The idle deadline passed: back to Stage 1 (when it is on), and lock the vault.
    fn on_idle_expired(&self) {
        {
            let mut s = self.st();
            if s.cfg.is_some() {
                s.open = false;
                s.equation = Some(self.next_equation());
            }
        }
        (self.lock_vault)();
    }

    fn period(&self) -> Duration {
        let minutes = relock(&self.settings).lock_after_minutes;
        *relock(&self.unit) * minutes
    }
}

impl GateCore {
    /// Read `app-settings.json` and `gate.json` from `dir` and start the idle watcher.
    ///
    /// A missing gate file means "no gate". A gate file that cannot be used at all is moved aside
    /// (`gate.json.corrupt-<ms>`) and the gate is off: file damage never wipes or locks anyone out
    /// of the app (the vaults stay behind their master passwords). A damaged *counter* inside a
    /// usable file is handled at the next attempt (see [`penalised`]).
    pub fn open(
        dir: &Path,
        vault_kdf: &KdfParams,
        lock_vault: LockVault,
    ) -> Result<Self, StorageError> {
        let settings = settings::load(dir);
        let (cfg, known) = match counter::load(dir) {
            Loaded::Absent => (
                None,
                Known {
                    attempts: 0,
                    epoch: 0,
                },
            ),
            Loaded::Corrupt => {
                counter::quarantine(dir);
                (
                    None,
                    Known {
                        attempts: 0,
                        epoch: 0,
                    },
                )
            }
            Loaded::Ready(p, c) => {
                let known = match c {
                    CounterRead::Valid { attempts, epoch } => Known { attempts, epoch },
                    // a record that failed its MAC: its number is only a hint (and capped)
                    CounterRead::Damaged { hint } => Known {
                        attempts: hint.unwrap_or(0).min(MAX_ATTEMPTS - 2),
                        epoch: 0,
                    },
                };
                (Some(*p), known)
            }
        };
        let enabled = cfg.is_some();
        let unit = idle::MINUTE;
        let inner = Arc::new(Inner {
            dir: dir.to_path_buf(),
            kdf: credential::gate_params(vault_kdf),
            state: Mutex::new(State {
                cfg,
                open: !enabled,
                known,
                equation: None,
                streak: 0,
            }),
            attempt: Mutex::new(()),
            idle: IdleTimer::new(unit * settings.lock_after_minutes),
            settings: Mutex::new(settings),
            unit: Mutex::new(unit),
            biometric: Mutex::new(Arc::new(NoBiometric)),
            rng: Mutex::new(Box::new(OsRand)),
            verifications: VerifyCounter::default(),
            lock_vault,
            hooks: Mutex::new(Vec::new()),
            #[cfg(test)]
            stop_after_write_ahead: std::sync::atomic::AtomicBool::new(false),
        });
        if enabled {
            let eq = inner.next_equation();
            inner.st().equation = Some(eq);
        }
        let watcher = {
            let inner = Arc::clone(&inner);
            std::thread::Builder::new()
                .name("passman-idle".into())
                .spawn(move || {
                    let idle = Arc::clone(&inner.idle);
                    idle.watch(|| inner.on_idle_expired());
                })
                .map_err(|_| StorageError::Storage("A background timer could not start.".into()))?
        };
        Ok(Self {
            inner,
            watcher: Mutex::new(Some(watcher)),
        })
    }

    // ------------------------------------------------------------ state

    /// Enabled and not yet opened in this run.
    pub fn is_closed(&self) -> bool {
        let s = self.inner.st();
        s.cfg.is_some() && !s.open
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.st().cfg.is_some()
    }

    pub fn status(&self) -> GateStatus {
        let available = self.biometric_provider().available();
        let s = self.inner.st();
        let enabled = s.cfg.is_some();
        let open = !enabled || s.open;
        GateStatus {
            enabled,
            open,
            mode: s.cfg.as_ref().map_or(GateMode::Pin, |c| c.mode),
            biometric_available: available,
            biometric: enabled && available && s.cfg.as_ref().is_some_and(|c| c.biometric),
            panic_button: enabled && s.cfg.as_ref().is_some_and(|c| c.panic_button),
            equation: if enabled && !open {
                s.equation.as_ref().map(|e| e.text.clone())
            } else {
                None
            },
            attempts_left: (s.known.attempts > 0)
                .then(|| MAX_ATTEMPTS.saturating_sub(s.known.attempts)),
            streak: s.streak,
        }
    }

    fn biometric_provider(&self) -> Arc<dyn BiometricAuth> {
        Arc::clone(&relock(&self.inner.biometric))
    }

    pub fn set_biometric(&self, provider: Arc<dyn BiometricAuth>) {
        *relock(&self.inner.biometric) = provider;
    }

    /// Is the gate in a state that needs the panic wipe to believe data exists? (Used by the
    /// wipe's pre-flight: a folder with no sign of Passman is refused only when this is true.)
    pub fn holds_state(&self) -> bool {
        self.is_enabled()
    }

    // ------------------------------------------------------------ the command guard and the timer

    /// Runs before every IPC command. Returns `locked` when the gate is closed and the command is
    /// not on the allow-list; otherwise counts the command as activity (unless it is a polling
    /// one). A deadline that passed while the machine slept is enforced here.
    pub fn before_command(&self, cmd: &str) -> Result<(), StorageError> {
        self.enforce_idle();
        if self.is_closed() {
            return if allowed_while_closed(cmd) {
                Ok(())
            } else {
                Err(StorageError::Locked)
            };
        }
        if !is_polling(cmd) {
            self.inner.idle.touch();
        }
        Ok(())
    }

    /// Run the expiry action now if the deadline has passed.
    pub fn enforce_idle(&self) {
        if self.inner.idle.take_expired() {
            self.inner.on_idle_expired();
        }
    }

    /// The person is active: restart the timer.
    pub fn ping(&self) {
        if !self.is_closed() {
            self.inner.idle.touch();
        }
    }

    pub fn settings(&self) -> AppSettings {
        *relock(&self.inner.settings)
    }

    pub fn set_settings(&self, new: AppSettings) -> Result<AppSettings, StorageError> {
        new.validate()?;
        settings::store(&self.inner.dir, &new)?;
        *relock(&self.inner.settings) = new;
        self.inner.idle.set_period(self.inner.period());
        // Changing the timeout is activity too.
        self.ping();
        Ok(new)
    }

    /// Make one "minute" last `unit` (production: 60 s). For the shim's `--idle-unit-ms` and tests.
    pub fn set_idle_unit(&self, unit: Duration) {
        *relock(&self.inner.unit) = unit;
        self.inner.idle.set_period(self.inner.period());
    }

    // ------------------------------------------------------------ the gate

    /// Typed text: 1 to 3 characters answer the decoy maths, 4 or more are a security attempt.
    /// `wipe` erases the device (the panic and the 15th failure).
    pub fn submit(&self, input: &str, wipe: WipeFn<'_>) -> Result<GateOutcome, StorageError> {
        {
            let s = self.inner.st();
            if s.cfg.is_none() || s.open {
                return Ok(GateOutcome::Opened);
            }
        }
        let text = Normalised::new(input);
        if text.chars() <= MATH_MAX_CHARS {
            return Ok(self.math_attempt(&text));
        }
        self.security_attempt(&text, wipe)
    }

    /// A maths attempt never reads a credential and never touches the counter.
    fn math_attempt(&self, text: &Normalised) -> GateOutcome {
        let mut s = self.inner.st();
        if s.equation.is_none() {
            s.equation = Some(self.inner.next_equation());
        }
        let typed = text.as_str().trim_matches(' ');
        if text.chars() == 0 {
            let eq = s
                .equation
                .as_ref()
                .map_or_else(String::new, |e| e.text.clone());
            return GateOutcome::Math {
                correct: false,
                equation: eq,
                streak: s.streak,
            };
        }
        let correct = !typed.is_empty()
            && typed.bytes().all(|b| b.is_ascii_digit())
            && typed
                .parse::<u32>()
                .ok()
                .zip(s.equation.as_ref())
                .is_some_and(|(n, e)| n == e.answer);
        if correct {
            s.streak = s.streak.saturating_add(1);
        }
        let next = self.inner.next_equation();
        let shown = next.text.clone();
        s.equation = Some(next);
        GateOutcome::Math {
            correct,
            equation: shown,
            streak: s.streak,
        }
    }

    /// Order of operations (D15): read the counter, **write attempts + 1 and fsync**, only then
    /// look at the input, then act on the verdict. A force-close in between still costs one
    /// attempt.
    fn security_attempt(
        &self,
        text: &Normalised,
        wipe: WipeFn<'_>,
    ) -> Result<GateOutcome, StorageError> {
        let inner = &self.inner;
        let _serial = relock(&inner.attempt);
        let (cfg, known) = {
            let s = inner.st();
            match (&s.cfg, s.open) {
                (Some(c), false) => (c.clone(), s.known),
                _ => return Ok(GateOutcome::Opened),
            }
        };

        // (1) read
        let (base, file_epoch) = match counter::read_counter(&inner.dir, &cfg.install_key) {
            CounterRead::Valid { attempts, epoch } if epoch >= known.epoch => (attempts, epoch),
            // an older record than this process already wrote: a restored or replayed copy
            CounterRead::Valid { attempts, epoch } => {
                (penalised(attempts.max(known.attempts), None), epoch)
            }
            CounterRead::Damaged { hint } => (penalised(known.attempts, hint), 0),
        };
        let base = base.min(MAX_ATTEMPTS);
        if base >= MAX_ATTEMPTS {
            // A record that says "15 failures" and a wipe that never finished: finish it.
            wipe()?;
            return Ok(GateOutcome::Wiped);
        }
        let next = base + 1;
        let epoch = known.epoch.max(file_epoch) + 1;

        // (2) write-ahead: if it cannot be recorded, the input is not even looked at
        if counter::store(&inner.dir, &cfg, next, epoch).is_err() {
            return Err(StorageError::Storage(
                "The attempt could not be recorded, so it was not checked.".into(),
            ));
        }
        inner.st().known = Known {
            attempts: next,
            epoch,
        };
        #[cfg(test)]
        if inner
            .stop_after_write_ahead
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(StorageError::Storage("test failpoint".into()));
        }

        // (3) verify: always all three, decided only afterwards
        let verdict = credential::verify_all(
            text,
            &cfg.credential,
            cfg.reversed.as_ref(),
            cfg.panic_code.as_ref(),
            &inner.verifications,
        )?;

        // (4) act
        match verdict.decide() {
            Decision::Open => {
                let reset_epoch = epoch + 1;
                let mut s = inner.st();
                if counter::store(&inner.dir, &cfg, 0, reset_epoch).is_ok() {
                    s.known = Known {
                        attempts: 0,
                        epoch: reset_epoch,
                    };
                }
                s.open = true;
                s.equation = None;
                drop(s);
                inner.idle.touch();
                Ok(GateOutcome::Opened)
            }
            Decision::Panic => {
                wipe()?;
                Ok(GateOutcome::Wiped)
            }
            Decision::Wrong if next >= MAX_ATTEMPTS => {
                wipe()?;
                Ok(GateOutcome::Wiped)
            }
            Decision::Wrong => {
                let eq = inner.next_equation();
                let shown = eq.text.clone();
                inner.st().equation = Some(eq);
                Ok(GateOutcome::Failed {
                    attempts_left: MAX_ATTEMPTS - next,
                    equation: shown,
                })
            }
        }
    }

    /// A fingerprint opens Stage 1 and nothing else: it counts no attempt, resets no counter and
    /// has no access to any key. `invalid_input` where there is no fingerprint (desktop).
    pub fn biometric(&self) -> Result<GateOutcome, StorageError> {
        let provider = self.biometric_provider();
        if !provider.available() {
            return Err(invalid("Fingerprint is not available on this device."));
        }
        {
            let s = self.inner.st();
            match &s.cfg {
                None => return Ok(GateOutcome::Opened),
                Some(_) if s.open => return Ok(GateOutcome::Opened),
                Some(c) if !c.biometric => {
                    return Err(invalid("Fingerprint is not switched on."));
                }
                Some(_) => {}
            }
        }
        if !provider.authenticate()? {
            return Err(invalid("The fingerprint was not recognised."));
        }
        let mut s = self.inner.st();
        if s.cfg.is_some() {
            s.open = true;
            s.equation = None;
        }
        drop(s);
        self.inner.idle.touch();
        Ok(GateOutcome::Opened)
    }

    /// The discreet lock-screen trigger: the same wipe as a panic credential. Only while the mark
    /// is switched on and the gate is closed.
    pub fn panic(&self, wipe: WipeFn<'_>) -> Result<GateOutcome, StorageError> {
        let _serial = relock(&self.inner.attempt);
        {
            let s = self.inner.st();
            let on = s.cfg.as_ref().is_some_and(|c| c.panic_button) && !s.open;
            if !on {
                return Err(invalid("The panic mark is not switched on."));
            }
        }
        wipe()?;
        Ok(GateOutcome::Wiped)
    }

    /// Back to Stage 1 (when it is on). The caller locks the vault.
    pub fn close(&self) {
        let mut s = self.inner.st();
        if s.cfg.is_some() {
            s.open = false;
            s.equation = Some(self.inner.next_equation());
        }
        drop(s);
        self.inner.idle.disarm();
    }

    /// Run `f` while no security attempt, configuration or other wipe is in progress.
    pub fn exclusive<T>(&self, f: impl FnOnce() -> T) -> T {
        let _serial = relock(&self.inner.attempt);
        f()
    }

    // ------------------------------------------------------------ configure

    /// `gate_configure`. `verify_master` is the re-prompt guard (the open vault's master password,
    /// Argon2id, `wrong_password` when it is wrong); it runs after the cheap checks and before
    /// the hashing, so a wrong password costs one Argon2id run, not four.
    pub fn configure(
        &self,
        mut input: GateConfigInput,
        verify_master: &dyn Fn(String) -> Result<(), StorageError>,
    ) -> Result<GateStatus, StorageError> {
        let inner = &self.inner;
        let _serial = relock(&inner.attempt);
        if input.master_password.is_empty() {
            return Err(invalid("Enter the master password of this vault."));
        }
        let existing = inner.st().cfg.clone();
        let plan = if input.enabled {
            Some(plan_configure(&input, existing.as_ref())?)
        } else {
            None
        };
        verify_master(std::mem::take(&mut input.master_password))?;

        let Some(plan) = plan else {
            // Turning it off removes the hashes and the counter file.
            let path = inner.dir.join(counter::GATE_FILE);
            match wipe::shred_file(&path) {
                Ok(()) => {}
                Err(_) if !path.exists() => {}
                Err(_) => {
                    return Err(StorageError::Storage(
                        "The app lock could not be turned off.".into(),
                    ))
                }
            }
            let _ = std::fs::remove_file(inner.dir.join(format!("{}.tmp", counter::GATE_FILE)));
            let mut s = inner.st();
            s.cfg = None;
            s.open = true;
            s.equation = None;
            s.known = Known {
                attempts: 0,
                epoch: 0,
            };
            drop(s);
            return Ok(self.status());
        };

        let persist = self.build_persist(&plan, existing.as_ref())?;
        let epoch = inner.st().known.epoch + 1;
        counter::store(&inner.dir, &persist, 0, epoch)
            .map_err(|_| StorageError::Storage("The app lock could not be saved.".into()))?;
        {
            let mut s = inner.st();
            s.cfg = Some(persist);
            // you are inside the app while configuring it
            s.open = true;
            s.equation = None;
            s.known = Known { attempts: 0, epoch };
        }
        Ok(self.status())
    }

    /// The expensive half of `configure`: checks that need the stored hashes, and the hashing.
    fn build_persist(
        &self,
        plan: &Plan,
        existing: Option<&Persist>,
    ) -> Result<Persist, StorageError> {
        let inner = &self.inner;
        let params = inner.kdf;
        let counter = &inner.verifications;
        let same_mode = existing.is_some_and(|e| e.mode == plan.mode);

        // checks against hashes that were stored earlier
        if let (None, PanicChange::Set(code), Some(e)) = (&plan.credential, &plan.panic, existing) {
            if credential::matches(code, &e.credential, counter)? {
                return Err(invalid(
                    "The panic code must be different from your passcode.",
                ));
            }
            if plan.reverse_panic {
                if let Some(rev) = &e.reversed {
                    if credential::matches(code, rev, counter)? {
                        return Err(invalid(
                            "That is your passcode reversed, which already triggers panic.",
                        ));
                    }
                }
            }
        }
        if let (Some(cred), PanicChange::Keep, Some(e)) = (&plan.credential, &plan.panic, existing)
        {
            if let Some(old) = e.panic_code.as_ref().filter(|_| same_mode) {
                if credential::matches(cred, old, counter)? {
                    return Err(invalid(
                        "The panic code must be different from your passcode.",
                    ));
                }
                if plan.reverse_panic && credential::matches(&cred.reversed(), old, counter)? {
                    return Err(invalid(
                        "That is your passcode reversed, which already triggers panic.",
                    ));
                }
            }
        }

        let (credential_hash, reversed) = match (&plan.credential, existing) {
            (Some(c), _) => {
                let real = credential::hash_new(c, &params)?;
                let rev = if plan.reverse_panic {
                    Some(credential::hash_new(&c.reversed(), &params)?)
                } else {
                    None
                };
                (real, rev)
            }
            (None, Some(e)) => (
                e.credential.clone(),
                if plan.reverse_panic {
                    e.reversed.clone()
                } else {
                    None
                },
            ),
            (None, None) => return Err(invalid("Choose a passcode.")),
        };
        let panic_code = match &plan.panic {
            PanicChange::Keep => existing
                .filter(|_| same_mode)
                .and_then(|e| e.panic_code.clone()),
            PanicChange::Remove => None,
            PanicChange::Set(code) => Some(credential::hash_new(code, &params)?),
        };
        Ok(Persist {
            mode: plan.mode,
            reverse_panic: plan.reverse_panic,
            biometric: plan.biometric,
            panic_button: plan.panic_button,
            credential: credential_hash,
            reversed,
            panic_code,
            install_key: counter::new_install_key()?,
        })
    }

    // ------------------------------------------------------------ wipe support

    /// Zero everything the gate holds in memory and go back to "no gate, default settings". The
    /// files are the wipe's job (`wipe.rs`).
    pub fn forget_everything(&self) {
        {
            let mut s = self.inner.st();
            s.cfg = None;
            s.open = true;
            s.equation = None;
            s.streak = 0;
            s.known = Known {
                attempts: 0,
                epoch: 0,
            };
        }
        *relock(&self.inner.settings) = AppSettings::default();
        self.inner.idle.set_period(self.inner.period());
        self.inner.idle.disarm();
    }

    /// Something the disguise slice (and others) must undo when everything is wiped, for example
    /// removing the launcher entry. Hooks run after the files are gone; a hook cannot fail the wipe.
    pub fn add_wipe_hook(&self, hook: WipeHook) {
        relock(&self.inner.hooks).push(hook);
    }

    pub fn run_wipe_hooks(&self) {
        for hook in relock(&self.inner.hooks).iter() {
            hook();
        }
    }

    /// `enforce_idle` with the two clocks supplied by the test (a suspend moves only the wall clock).
    #[cfg(test)]
    pub(crate) fn enforce_idle_at(&self, mono: std::time::Instant, wall: std::time::SystemTime) {
        if self.inner.idle.take_expired_at(mono, wall) {
            self.inner.on_idle_expired();
        }
    }

    #[cfg(test)]
    pub(crate) fn verifications(&self) -> u64 {
        self.inner.verifications.count()
    }

    #[cfg(test)]
    pub(crate) fn set_rng(&self, rng: Box<dyn Rand + Send>) {
        *relock(&self.inner.rng) = rng;
    }

    #[cfg(test)]
    pub(crate) fn set_failpoint_after_write_ahead(&self, on: bool) {
        self.inner
            .stop_after_write_ahead
            .store(on, std::sync::atomic::Ordering::SeqCst);
    }

    /// The answer of the current equation (tests only: the webview never sees it).
    #[cfg(test)]
    pub(crate) fn current_answer(&self) -> Option<u32> {
        self.inner.st().equation.as_ref().map(|e| e.answer)
    }

    #[cfg(test)]
    pub(crate) fn idle_armed(&self) -> bool {
        self.inner.idle.is_armed()
    }
}

impl Drop for GateCore {
    fn drop(&mut self) {
        self.inner.idle.shutdown();
        if let Some(handle) = relock(&self.watcher).take() {
            let _ = handle.join();
        }
    }
}

// ------------------------------------------------------------------ configure: the cheap half

enum PanicChange {
    Keep,
    Remove,
    Set(Normalised),
}

/// Everything `gate_configure` decides without running Argon2id.
struct Plan {
    mode: GateMode,
    /// A new credential, or `None` to keep the stored one.
    credential: Option<Normalised>,
    reverse_panic: bool,
    panic: PanicChange,
    biometric: bool,
    panic_button: bool,
}

fn plan_configure(
    input: &GateConfigInput,
    existing: Option<&Persist>,
) -> Result<Plan, StorageError> {
    let mode = input.mode;
    let mode_changed = existing.is_none_or(|e| e.mode != mode);
    let credential = match input.credential.as_deref() {
        Some(raw) => {
            if credential::char_count(raw) == 0 {
                return Err(invalid("Choose a passcode."));
            }
            if let Some(problem) = credential::credential_problem(mode, raw) {
                return Err(StorageError::InvalidInput(problem));
            }
            Some(Normalised::new(raw))
        }
        None => {
            if existing.is_none() || mode_changed {
                return Err(invalid("Choose a passcode."));
            }
            None
        }
    };
    let panic = match input.panic_code.as_deref() {
        None => PanicChange::Keep,
        Some(code) if credential::char_count(code) == 0 => PanicChange::Remove,
        Some(code) => PanicChange::Set(Normalised::new(code)),
    };
    // The rules that can be checked on the text alone (the same ones as the front end).
    let cred_text = input.credential.as_deref().unwrap_or("");
    let panic_text = input.panic_code.as_deref().unwrap_or("");
    let (reverse_problem, panic_problem) =
        credential::panic_problems(mode, cred_text, input.reverse_panic, panic_text);
    if let Some(p) = reverse_problem.or(panic_problem) {
        return Err(StorageError::InvalidInput(p));
    }
    // A kept credential cannot be reversed: its plaintext is not stored.
    if credential.is_none() && input.reverse_panic && existing.is_some_and(|e| e.reversed.is_none())
    {
        return Err(invalid(
            "Enter your passcode again to turn on the reversed panic code.",
        ));
    }
    Ok(Plan {
        mode,
        credential,
        reverse_panic: input.reverse_panic,
        panic,
        biometric: input.biometric,
        panic_button: input.panic_button,
    })
}
