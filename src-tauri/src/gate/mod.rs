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
use self::counter::{CounterRead, Loaded, Lock, Persist};
use self::credential::{Decision, Normalised, VerifyCounter};
use self::equation::{Equation, OsRand, Rand};
use self::idle::IdleTimer;
use self::settings::AppSettings;
use crate::clock::TrustedClock;
use crate::crypto::KdfParams;
use crate::storage::StorageError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The 16th failed security attempt erases the device. Fifteen wrong ones in a row are survivable, but only with long
/// waits in between ([`LOCK_STEPS`]), and the 16th is the last chance: the screen warns before it is sent.
pub const MAX_ATTEMPTS: u32 = 16;

/// Waits after wrong credentials: `(wrong attempts in a row, seconds until the next attempt is allowed)`.
/// 3 wrong: 1 min 30 s. 6: 5 min. 9: 45 min. 12: 8 h. 15: 3 days. Attempt 16 is the last (a wrong one erases).
/// Maths answers never count. A wait survives closing the app and restarting the device (it is stored in `gate.json`,
/// under the same MAC as the counter). It is measured on the boot clock and on UTC time checked online
/// (`crate::clock`), never on the device date, so setting the date forward shortens nothing. Honest limit: someone
/// who edits the files can forge them (the counter's MAC detects casual edits only); the wipe at attempt 16 and the
/// master password still stand.
pub const LOCK_STEPS: [(u32, u64); 5] = [
    (3, 90),
    (6, 5 * 60),
    (9, 45 * 60),
    (12, 8 * 3600),
    (15, 3 * 24 * 3600),
];

/// The wait (seconds) that starts when the counter reaches `attempts`, if any.
pub fn lock_seconds_after(attempts: u32) -> Option<u64> {
    LOCK_STEPS
        .iter()
        .find(|(n, _)| *n == attempts)
        .map(|(_, secs)| *secs)
}
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
    "app_reset_challenge",
    "app_settings_get",
    "device_get",
    "clock_status",
    "clock_sync",
    "activity_ping",
    "app_leave",
    "app_return",
    "sync_flush",
    "disguise_get",
    "screen_secure",
];

/// The polling commands: they must not count as activity, or an open window would never time out.
pub const POLLING_COMMANDS: &[&str] = &[
    "vault_status",
    "gate_status",
    "app_leave",
    "app_return",
    "clock_status",
];

/// Commands the window runs by itself in the background (the browsers' long poll): like the polling commands they must not count
/// as activity, but unlike them they are **not** allowed while the app lock is closed (they name the website that is waiting).
pub const PASSIVE_COMMANDS: &[&str] = &["bridge_wait"];

pub fn allowed_while_closed(cmd: &str) -> bool {
    ALLOWED_WHILE_CLOSED.contains(&cmd)
}

pub fn is_polling(cmd: &str) -> bool {
    POLLING_COMMANDS.contains(&cmd)
}

pub fn is_passive(cmd: &str) -> bool {
    PASSIVE_COMMANDS.contains(&cmd)
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
    /// Milliseconds until credential attempts are allowed again, while a wait is running.
    pub locked_for_ms: Option<u64>,
    /// The same moment as checked UTC milliseconds, so the screen can say "try again at 14:35" in the chosen zone
    /// without trusting the device date. Absent while the time is not checked.
    pub locked_until_ms: Option<u64>,
    /// A wait is running but its end cannot be judged yet: the device restarted and the time was not checked online
    /// since. The core asks the time servers on its own; nothing is counted or checked meanwhile.
    pub clock_pending: bool,
    /// The next wrong credential erases the device (the screen warns before it is sent).
    pub final_attempt: bool,
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
        /// Set when this failure started a wait: the milliseconds until the next attempt is allowed.
        locked_for_ms: Option<u64>,
    },
    /// A wait is running: the attempt was neither counted nor checked.
    Locked {
        retry_after_ms: u64,
        /// The end of the wait cannot be judged yet (see [`GateStatus::clock_pending`]); `retry_after_ms` is 0.
        clock_pending: bool,
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

#[derive(Clone)]
struct Known {
    attempts: u32,
    epoch: u64,
    /// The wait after wrong credentials, on the boot clock and in checked UTC (see [`Lock`]).
    lock: Lock,
}

/// What is left of a wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    Over,
    Left(u64),
    /// Cannot be judged yet: another boot, and no checked UTC time.
    Unknown,
}

impl Wait {
    /// The longer of two waits; an unknown one wins (nothing is let through on a guess).
    fn longest(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::Left(a), Self::Left(b)) => Self::Left(a.max(b)),
            (Self::Left(a), Self::Over) | (Self::Over, Self::Left(a)) => Self::Left(a),
            (Self::Over, Self::Over) => Self::Over,
        }
    }
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
    /// Armed while the app is in the background: when it runs out, the same lock as the inactivity timer.
    leave: Arc<IdleTimer>,
    settings: Mutex<AppSettings>,
    unit: Mutex<Duration>,
    /// How long one "lock minute" lasts (production 60 s). Tests set it to zero (no waits) or use the clock below.
    lock_unit: Mutex<Duration>,
    /// Time: the boot clock and UTC checked online. Replaceable so tests can move time.
    time: Mutex<Arc<TrustedClock>>,
    biometric: Mutex<Arc<dyn BiometricAuth>>,
    /// The long sum that guards "erase everything" (the answer never leaves Rust). One at a time.
    reset_sum: Mutex<Option<Equation>>,
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
    leave_watcher: Mutex<Option<JoinHandle<()>>>,
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

    fn time(&self) -> Arc<TrustedClock> {
        Arc::clone(&relock(&self.time))
    }

    /// What is left of `lock`. On the same boot only the boot clock counts (the device date is never looked at);
    /// after a restart the UTC deadline counts, but only against UTC time that was checked online.
    fn wait_left(&self, lock: &Lock) -> Wait {
        if lock.is_none() {
            return Wait::Over;
        }
        let time = self.time();
        let boot = time.boot_now();
        if lock.boot_ms > 0 && lock.boot_id == boot.id {
            return if boot.ms >= lock.boot_ms {
                Wait::Over
            } else {
                Wait::Left(lock.boot_ms - boot.ms)
            };
        }
        match (time.deadline_utc_ms(), lock.until_utc) {
            (Some(now), until) if until > 0 => {
                if now >= until {
                    Wait::Over
                } else {
                    Wait::Left(until - now)
                }
            }
            _ => Wait::Unknown,
        }
    }

    /// A lock of `ms` that starts now.
    fn start_lock(&self, ms: u64) -> Lock {
        let time = self.time();
        let boot = time.boot_now();
        Lock {
            until_utc: time.deadline_utc_ms().map_or(0, |now| now + ms),
            boot_id: boot.id,
            boot_ms: boot.ms + ms,
        }
    }

    /// The wait that is running, as far as the stored lock goes. A wait that started while the time was not checked
    /// and then survived a restart has no deadline in UTC; the first time the time is checked it starts over in
    /// full (the safe direction), and that is written down so it does not start over again.
    fn current_wait(&self) -> Wait {
        let (cfg, known) = {
            let s = self.st();
            (s.cfg.clone(), s.known.clone())
        };
        let wait = self.wait_left(&known.lock);
        let time = self.time();
        let needs_clock = wait == Wait::Unknown;
        if needs_clock && known.lock.until_utc == 0 {
            if let (Some(cfg), Some(now)) = (cfg.as_ref(), time.deadline_utc_ms()) {
                let full =
                    lock_seconds_after(known.attempts).map_or(90_000, |secs| self.lock_ms(secs));
                let boot = time.boot_now();
                let lock = Lock {
                    until_utc: now + full,
                    boot_id: boot.id,
                    boot_ms: boot.ms + full,
                };
                let epoch = known.epoch + 1;
                if counter::store_locked(&self.dir, cfg, known.attempts, epoch, &lock).is_ok() {
                    let mut s = self.st();
                    s.known.epoch = epoch;
                    s.known.lock = lock;
                    return Wait::Left(full);
                }
            }
        }
        if needs_clock {
            time.refresh_in_background();
        }
        wait
    }

    /// Milliseconds of a wait of `secs` seconds, scaled by the lock unit.
    fn lock_ms(&self, secs: u64) -> u64 {
        let unit_ms = relock(&self.lock_unit).as_millis() as u64;
        secs.saturating_mul(unit_ms) / 60
    }

    fn period(&self) -> Duration {
        let minutes = relock(&self.settings).lock_after_minutes;
        *relock(&self.unit) * minutes
    }

    /// How long the app may stay open in the background (one "second" is a sixtieth of the unit).
    fn leave_period(&self) -> Duration {
        let secs = relock(&self.settings).lock_after_leave_seconds;
        *relock(&self.unit) * secs / 60
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
                    lock: Lock::none(),
                },
            ),
            Loaded::Corrupt => {
                counter::quarantine(dir);
                (
                    None,
                    Known {
                        attempts: 0,
                        epoch: 0,
                        lock: Lock::none(),
                    },
                )
            }
            Loaded::Ready(p, c) => {
                let known = match c {
                    CounterRead::Valid {
                        attempts,
                        epoch,
                        lock,
                    } => Known {
                        attempts,
                        epoch,
                        lock,
                    },
                    // a record that failed its MAC: its number is only a hint (and capped)
                    CounterRead::Damaged { hint } => Known {
                        attempts: hint.unwrap_or(0).min(MAX_ATTEMPTS - 2),
                        epoch: 0,
                        lock: Lock::none(),
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
            leave: IdleTimer::new(unit * settings.lock_after_leave_seconds / 60),
            settings: Mutex::new(settings),
            unit: Mutex::new(unit),
            lock_unit: Mutex::new(Duration::from_secs(60)),
            time: Mutex::new(Arc::clone(TrustedClock::global())),
            biometric: Mutex::new(Arc::new(NoBiometric)),
            reset_sum: Mutex::new(None),
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
                .name("hexis-idle".into())
                .spawn(move || {
                    let idle = Arc::clone(&inner.idle);
                    idle.watch(|| inner.on_idle_expired());
                })
                .map_err(|_| StorageError::Storage("A background timer could not start.".into()))?
        };
        let leave_watcher = {
            let inner = Arc::clone(&inner);
            std::thread::Builder::new()
                .name("hexis-leave".into())
                .spawn(move || {
                    let leave = Arc::clone(&inner.leave);
                    leave.watch(|| inner.on_idle_expired());
                })
                .map_err(|_| StorageError::Storage("A background timer could not start.".into()))?
        };
        Ok(Self {
            inner,
            watcher: Mutex::new(Some(watcher)),
            leave_watcher: Mutex::new(Some(leave_watcher)),
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
        let closed = {
            let s = self.inner.st();
            s.cfg.is_some() && !s.open
        };
        let wait = if closed {
            self.inner.current_wait()
        } else {
            Wait::Over
        };
        let utc_now = self.inner.time().deadline_utc_ms();
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
            locked_for_ms: match wait {
                Wait::Left(ms) => Some(ms),
                _ => None,
            },
            locked_until_ms: match (wait, utc_now) {
                (Wait::Left(ms), Some(now)) => Some(now + ms),
                _ => None,
            },
            clock_pending: wait == Wait::Unknown,
            final_attempt: enabled && !open && s.known.attempts + 1 >= MAX_ATTEMPTS,
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
    /// wipe's pre-flight: a folder with no sign of Hexis is refused only when this is true.)
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
        if !is_polling(cmd) && !is_passive(cmd) {
            self.inner.idle.touch();
        }
        Ok(())
    }

    /// Run the expiry action now if a deadline has passed (the inactivity one, or the one that runs while the app
    /// is in the background; the phone may have slept through either).
    pub fn enforce_idle(&self) {
        let idle = self.inner.idle.take_expired();
        let left = self.inner.leave.take_expired();
        if idle || left {
            self.inner.on_idle_expired();
        }
    }

    /// The app went to the background (a phone or tablet: the person switched away). The inactivity timer stops
    /// and the "after leaving" timer starts; with 0 seconds the lock happens now. Native screens that sit on top of
    /// the app (the camera, a file picker, a permission prompt) are not "leaving": the front end does not call this
    /// for them.
    pub fn app_leave(&self) {
        self.inner.idle.disarm();
        let period = self.inner.leave_period();
        if period.is_zero() {
            self.inner.leave.disarm();
            self.inner.on_idle_expired();
        } else {
            self.inner.leave.touch();
        }
    }

    /// The app is back on screen. If the time allowed away has run out the app locks now (the vault and, when it is
    /// on, the gate); otherwise the timer stops and the inactivity timer starts again.
    pub fn app_return(&self) {
        let expired = self.inner.leave.take_expired();
        self.inner.leave.disarm();
        if expired {
            self.inner.on_idle_expired();
        } else if !self.is_closed() {
            self.inner.idle.touch();
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
        self.inner.leave.set_period(self.inner.leave_period());
        // Changing the timeout is activity too.
        self.ping();
        Ok(new)
    }

    /// Make one minute of every wait after wrong credentials last `unit` (production: 60 s; zero: no waits at all).
    pub fn set_lock_unit(&self, unit: Duration) {
        *relock(&self.inner.lock_unit) = unit;
    }

    /// Replace the clock the waits are measured on. For tests and the shim.
    pub fn set_time(&self, time: Arc<TrustedClock>) {
        *relock(&self.inner.time) = time;
    }

    /// The clock the waits are measured on.
    pub fn time(&self) -> Arc<TrustedClock> {
        self.inner.time()
    }

    /// Make one "minute" last `unit` (production: 60 s). For the shim's `--idle-unit-ms` and tests.
    pub fn set_idle_unit(&self, unit: Duration) {
        *relock(&self.inner.unit) = unit;
        self.inner.idle.set_period(self.inner.period());
        self.inner.leave.set_period(self.inner.leave_period());
    }

    // ------------------------------------------------------------ the gate

    /// Typed text: 1 to 3 characters answer the decoy maths, 4 or more are a security attempt.
    /// `wipe` erases the device (the panic and the 16th failure).
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
                (Some(c), false) => (c.clone(), s.known.clone()),
                _ => return Ok(GateOutcome::Opened),
            }
        };

        // (1) read
        let (base, file_epoch, file_lock) =
            match counter::read_counter(&inner.dir, &cfg.install_key) {
                CounterRead::Valid {
                    attempts,
                    epoch,
                    lock,
                } if epoch >= known.epoch => (attempts, epoch, lock),
                // an older record than this process already wrote: a restored or replayed copy
                CounterRead::Valid {
                    attempts, epoch, ..
                } => (
                    penalised(attempts.max(known.attempts), None),
                    epoch,
                    known.lock.clone(),
                ),
                CounterRead::Damaged { hint } => {
                    (penalised(known.attempts, hint), 0, known.lock.clone())
                }
            };
        let base = base.min(MAX_ATTEMPTS);
        if base >= MAX_ATTEMPTS {
            // A record that says "16 failures" and a wipe that never finished: finish it.
            wipe()?;
            return Ok(GateOutcome::Wiped);
        }
        // (1b) a wait is running: the attempt is neither counted nor looked at
        let (w_file, w_known) = (inner.wait_left(&file_lock), inner.wait_left(&known.lock));
        let (wait, keep) = match (w_file, w_known) {
            (Wait::Over, Wait::Over) => (Wait::Over, None),
            (Wait::Over, w) => (w, Some(known.lock.clone())),
            (w, Wait::Over) => (w, Some(file_lock.clone())),
            (a, b) => {
                let known_is_longer = matches!((a, b), (Wait::Left(x), Wait::Left(y)) if y > x);
                let lock = if known_is_longer {
                    known.lock.clone()
                } else {
                    file_lock.clone()
                };
                (a.longest(b), Some(lock))
            }
        };
        if wait != Wait::Over {
            if let Some(lock) = keep {
                inner.st().known.lock = lock;
            }
            return Ok(match wait {
                Wait::Left(ms) => GateOutcome::Locked {
                    retry_after_ms: ms,
                    clock_pending: false,
                },
                _ => {
                    inner.time().refresh_in_background();
                    GateOutcome::Locked {
                        retry_after_ms: 0,
                        clock_pending: true,
                    }
                }
            });
        }
        let next = base + 1;
        let epoch = known.epoch.max(file_epoch) + 1;
        // The wait that a wrong answer now would start is written together with the counter, so
        // force-closing the app mid-attempt dodges neither the count nor the wait.
        let wait_ms = lock_seconds_after(next)
            .map(|secs| inner.lock_ms(secs))
            .filter(|ms| *ms > 0);
        let lock = wait_ms.map_or_else(Lock::none, |ms| inner.start_lock(ms));

        // (2) write-ahead: if it cannot be recorded, the input is not even looked at
        if counter::store_locked(&inner.dir, &cfg, next, epoch, &lock).is_err() {
            return Err(StorageError::Storage(
                "The attempt could not be recorded, so it was not checked.".into(),
            ));
        }
        inner.st().known = Known {
            attempts: next,
            epoch,
            lock,
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
                        lock: Lock::none(),
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
                    locked_for_ms: wait_ms,
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

    /// A new long sum for the last step of "erase everything". Replaces any earlier one, so an answer worked out for
    /// an old sum is worthless. Returns the text only.
    pub fn reset_challenge(&self) -> String {
        let eq = equation::generate_long(&mut **relock(&self.inner.rng));
        let text = eq.text.clone();
        *relock(&self.inner.reset_sum) = Some(eq);
        text
    }

    /// Checks the typed answer to the current sum. The sum is used up either way: a wrong answer needs a new one.
    pub fn check_reset_answer(&self, typed: &str) -> bool {
        let Some(eq) = relock(&self.inner.reset_sum).take() else {
            return false;
        };
        let typed = typed.trim();
        !typed.is_empty()
            && typed.len() <= 9
            && typed.bytes().all(|b| b.is_ascii_digit())
            && typed.parse::<u32>().is_ok_and(|n| n == eq.answer)
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
                lock: Lock::none(),
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
            s.known = Known {
                attempts: 0,
                epoch,
                lock: Lock::none(),
            };
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
                lock: Lock::none(),
            };
        }
        *relock(&self.inner.settings) = AppSettings::default();
        self.inner.idle.set_period(self.inner.period());
        self.inner.idle.disarm();
        self.inner.leave.set_period(self.inner.leave_period());
        self.inner.leave.disarm();
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

    /// The answer of the sum that "erase everything" is waiting on (tests only).
    #[cfg(test)]
    pub(crate) fn reset_answer(&self) -> Option<u32> {
        relock(&self.inner.reset_sum).as_ref().map(|e| e.answer)
    }

    #[cfg(test)]
    pub(crate) fn idle_armed(&self) -> bool {
        self.inner.idle.is_armed()
    }
}

impl Drop for GateCore {
    fn drop(&mut self) {
        self.inner.idle.shutdown();
        self.inner.leave.shutdown();
        if let Some(handle) = relock(&self.watcher).take() {
            let _ = handle.join();
        }
        if let Some(handle) = relock(&self.leave_watcher).take() {
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
