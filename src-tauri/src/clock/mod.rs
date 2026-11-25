//! Trusted time: UTC that does not come from the device's own date setting.
//!
//! The device date is whatever the person sets it to, so nothing that guards a secret may rely on
//! it. Two clocks work together:
//!
//! * the **boot clock** ([`boot`]): milliseconds since the device started, asleep time included. It
//!   cannot be moved from the settings. Waits are measured on it.
//! * **checked UTC** ([`net`]): real time read from a few well-known HTTPS servers. Each reading
//!   links the boot clock to UTC (`utc = boot + offset`), which gives UTC time that keeps counting
//!   correctly with no network until the device restarts, whatever the date setting says.
//!
//! Everything stored is UTC milliseconds since 1970, taken from [`TrustedClock::now_ms`]: the checked
//! UTC time when there is one, the device's own clock otherwise (a vault on a phone that was never
//! online still works; it just cannot know the device date is wrong).
//!
//! What this protects: the lock after wrong passcodes (`gate`). A wait is stored as a boot-clock
//! deadline, plus a UTC deadline once the time was checked. After a restart the UTC deadline is judged
//! against checked UTC time; with no internet and no checked time the wait is not over yet. Setting the
//! date forward therefore shortens nothing. Honest limits: someone who can edit the app's files can still
//! forge them (the gate counter only detects casual edits), and the 16-attempt erase applies regardless.
//!
//! The time check is switchable (Settings, Date and time). Off, no request is ever made: a wait is judged on
//! the boot clock within a boot, and after a restart on the device's own clock (`deadline_utc_ms`), because
//! failing closed there would lock the person out for good. Whoever switches the check off accepts that
//! changing the date can then shorten a wait after a restart. With the check on but no network, such a wait
//! stays unjudged and the gate refuses every attempt until the time has been checked.

pub mod boot;
pub mod net;

use self::boot::{BootClock, BootStamp, SystemBoot};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// One answer of one time server, tied to the boot clock at the moment it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub utc_ms: u64,
    pub boot: BootStamp,
}

/// Why a check did not give a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncError {
    /// The person switched the online check off.
    Disabled,
    /// No server answered.
    Offline,
    /// Servers answered, but not with the same time.
    Disagree,
    /// The answers were not usable.
    Unusable,
}

impl SyncError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Disabled => "The online time check is switched off.",
            Self::Offline => "No time server could be reached. Check the internet connection.",
            Self::Disagree => "The time servers did not agree, so none of them was trusted.",
            Self::Unusable => "The time servers gave answers that could not be used.",
        }
    }
}

pub trait TimeFetcher: Send + Sync {
    fn fetch(&self, boot: &dyn BootClock) -> Result<Vec<Sample>, SyncError>;
}

/// Servers may differ by this much (their own rounding plus the network) before none is trusted.
const MAX_SPREAD_MS: i64 = 15_000;
/// A fresh check is wanted when the last one is older than this.
const REFRESH_AFTER_MS: u64 = 6 * 3600 * 1000;
/// ...but never more often than this.
const MIN_RETRY_MS: u64 = 20_000;
/// 2024-01-01 in milliseconds; a "checked" time before it is nonsense.
const FLOOR_MS: i64 = 1_704_067_200_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Anchor {
    boot_id: String,
    /// `utc_ms - boot_ms` at the moment of the check.
    offset_ms: i64,
    /// Boot clock when it was made (to tell how old the check is).
    at_boot_ms: u64,
    /// Servers that agreed.
    sources: usize,
}

struct State {
    anchor: Option<Anchor>,
    enabled: bool,
    last_error: Option<SyncError>,
    last_try: Option<BootStamp>,
}

/// What the screen may show about the clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockStatus {
    /// The time was checked online since this device started.
    pub verified: bool,
    /// The online check is switched on.
    pub enabled: bool,
    /// Checked UTC time minus this device's own clock, in milliseconds (positive: the device is behind).
    pub skew_ms: Option<i64>,
    /// How long ago the time was last checked, in milliseconds.
    pub checked_ago_ms: Option<u64>,
    /// How many servers agreed.
    pub sources: usize,
    /// Why the last check failed, in words (None after a good one).
    pub problem: Option<String>,
    /// UTC milliseconds the core uses as "now".
    pub now_ms: u64,
}

pub struct TrustedClock {
    boot: Arc<dyn BootClock>,
    fetcher: Arc<dyn TimeFetcher>,
    /// The device's own clock (milliseconds since the epoch). Replaceable for tests.
    wall: Arc<dyn Fn() -> u64 + Send + Sync>,
    state: Mutex<State>,
    busy: AtomicBool,
    /// The background checks started so far, so a test can wait for them (see `finish_background`).
    #[cfg(test)]
    background: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What the device's own clock says, in milliseconds since the epoch.
pub fn device_now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl TrustedClock {
    pub fn new(
        boot: Arc<dyn BootClock>,
        fetcher: Arc<dyn TimeFetcher>,
        wall: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            boot,
            fetcher,
            wall,
            state: Mutex::new(State {
                anchor: None,
                enabled: true,
                last_error: None,
                last_try: None,
            }),
            busy: AtomicBool::new(false),
            #[cfg(test)]
            background: Mutex::new(Vec::new()),
        }
    }

    /// The real clock: the system boot clock, the HTTPS servers, the device date as the fallback.
    pub fn system() -> Self {
        Self::new(
            Arc::new(SystemBoot),
            Arc::new(net::HttpsFetcher),
            Arc::new(device_now_ms),
        )
    }

    /// The one clock of this process. Nothing asks the network until [`sync`](Self::sync) or
    /// [`refresh_in_background`](Self::refresh_in_background) is called.
    pub fn global() -> &'static Arc<TrustedClock> {
        static CLOCK: OnceLock<Arc<TrustedClock>> = OnceLock::new();
        CLOCK.get_or_init(|| Arc::new(Self::system()))
    }

    fn st(&self) -> MutexGuard<'_, State> {
        relock(&self.state)
    }

    pub fn boot_now(&self) -> BootStamp {
        self.boot.now()
    }

    /// Checked UTC time in milliseconds, or `None` when the time was not checked since this device started.
    pub fn verified_utc_ms(&self) -> Option<u64> {
        let b = self.boot.now();
        let st = self.st();
        let a = st.anchor.as_ref()?;
        if !st.enabled || a.boot_id != b.id || b.ms < a.at_boot_ms {
            return None;
        }
        let utc = b.ms as i64 + a.offset_ms;
        (utc > FLOOR_MS).then_some(utc as u64)
    }

    /// The UTC that stored deadlines are judged against after a restart: the checked time, or, only when the person
    /// switched the online check off, the device's own clock. Off is their choice (it is what every app did before this
    /// check existed) and without it a wait that survives a restart could never end. Within one run waits still rest
    /// on the boot clock, which changing the date cannot move.
    pub fn deadline_utc_ms(&self) -> Option<u64> {
        self.verified_utc_ms()
            .or_else(|| (!self.is_enabled()).then(|| (self.wall)()))
    }

    /// UTC milliseconds: checked time when there is one, the device's own clock otherwise.
    pub fn now_ms(&self) -> u64 {
        self.verified_utc_ms().unwrap_or_else(|| (self.wall)())
    }

    pub fn is_enabled(&self) -> bool {
        self.st().enabled
    }

    /// Switch the online check on or off. Off forgets the checked time, so nothing stale is trusted.
    pub fn set_enabled(&self, on: bool) {
        let mut st = self.st();
        st.enabled = on;
        if !on {
            st.anchor = None;
            st.last_error = Some(SyncError::Disabled);
        } else if st.last_error == Some(SyncError::Disabled) {
            st.last_error = None;
        }
    }

    pub fn status(&self) -> ClockStatus {
        let verified = self.verified_utc_ms();
        let wall = (self.wall)();
        let b = self.boot.now();
        let st = self.st();
        let anchor = st.anchor.as_ref().filter(|a| a.boot_id == b.id);
        ClockStatus {
            verified: verified.is_some(),
            enabled: st.enabled,
            skew_ms: verified.map(|v| v as i64 - wall as i64),
            checked_ago_ms: anchor.map(|a| b.ms.saturating_sub(a.at_boot_ms)),
            sources: anchor.map_or(0, |a| a.sources),
            problem: st.last_error.map(|e| e.message().to_string()),
            now_ms: verified.unwrap_or(wall),
        }
    }

    /// Ask the servers now and wait for the answer (up to about ten seconds).
    pub fn sync(&self) -> Result<ClockStatus, SyncError> {
        if !self.is_enabled() {
            return Err(SyncError::Disabled);
        }
        if self.busy.swap(true, Ordering::SeqCst) {
            // another check is running; its result will be visible in the status
            return Ok(self.status());
        }
        let result = self.fetcher.fetch(&*self.boot).and_then(|s| self.apply(s));
        self.busy.store(false, Ordering::SeqCst);
        let mut st = self.st();
        st.last_try = Some(self.boot.now());
        match result {
            Ok(anchor) => {
                if st.enabled {
                    st.anchor = Some(anchor);
                    st.last_error = None;
                }
                drop(st);
                Ok(self.status())
            }
            Err(e) => {
                st.last_error = Some(e);
                Err(e)
            }
        }
    }

    /// Turn the servers' answers into one link between the boot clock and UTC.
    fn apply(&self, samples: Vec<Sample>) -> Result<Anchor, SyncError> {
        // Only answers given on the same boot as the first one can be compared.
        let id = samples.first().ok_or(SyncError::Unusable)?.boot.id.clone();
        let mut points: Vec<(i64, u64)> = samples
            .iter()
            .filter(|s| s.boot.id == id)
            .map(|s| (s.utc_ms as i64 - s.boot.ms as i64, s.boot.ms))
            .collect();
        if points.is_empty() {
            return Err(SyncError::Unusable);
        }
        points.sort_unstable();
        let (lowest, highest) = (points[0].0, points[points.len() - 1].0);
        if highest - lowest > MAX_SPREAD_MS {
            return Err(SyncError::Disagree);
        }
        let (offset_ms, at_boot_ms) = points[points.len() / 2];
        if at_boot_ms as i64 + offset_ms < FLOOR_MS {
            return Err(SyncError::Unusable);
        }
        Ok(Anchor {
            boot_id: id,
            offset_ms,
            at_boot_ms,
            sources: points.len(),
        })
    }

    /// Check the time on a background thread when it is due: nothing checked yet, or the check is old,
    /// and the last try was not just now. Cheap to call often (the screens poll).
    pub fn refresh_in_background(self: &Arc<Self>) {
        if self.busy.load(Ordering::SeqCst) || !self.due() {
            return;
        }
        let me = Arc::clone(self);
        let started = std::thread::Builder::new()
            .name("hexis-clock".into())
            .spawn(move || {
                let _ = me.sync();
            });
        #[cfg(test)]
        if let Ok(handle) = started {
            relock(&self.background).push(handle);
        }
        #[cfg(not(test))]
        let _ = started;
    }

    /// Test only: wait until every check the gate started on its own has ended. A check that is still on its way when a test
    /// moves the servers or the clock would land afterwards and anchor the time at the wrong moment.
    #[cfg(test)]
    pub(crate) fn finish_background(&self) {
        let handles: Vec<_> = relock(&self.background).drain(..).collect();
        for handle in handles {
            let _ = handle.join();
        }
    }

    fn due(&self) -> bool {
        let b = self.boot.now();
        let st = self.st();
        if !st.enabled {
            return false;
        }
        if let Some(t) = &st.last_try {
            if t.id == b.id && b.ms.saturating_sub(t.ms) < MIN_RETRY_MS {
                return false;
            }
        }
        match &st.anchor {
            Some(a) if a.boot_id == b.id => b.ms.saturating_sub(a.at_boot_ms) >= REFRESH_AFTER_MS,
            _ => true,
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A clock the tests can move, and servers they can script.
    use super::*;
    use std::sync::atomic::AtomicU64;

    /// Boot clock and device date, both set by hand.
    pub struct Fakes {
        pub boot_id: Mutex<String>,
        pub boot_ms: AtomicU64,
        pub wall_ms: AtomicU64,
        pub servers: Mutex<Result<i64, SyncError>>,
        pub asked: AtomicU64,
    }

    impl Fakes {
        /// A device up for `boot_ms`, whose own date says `wall_ms`, with servers that know the real time `real_ms`.
        pub fn new(boot_ms: u64, wall_ms: u64, real_ms: u64) -> Arc<Self> {
            Arc::new(Self {
                boot_id: Mutex::new("boot-1".into()),
                boot_ms: AtomicU64::new(boot_ms),
                wall_ms: AtomicU64::new(wall_ms),
                servers: Mutex::new(Ok(real_ms as i64)),
                asked: AtomicU64::new(0),
            })
        }

        pub fn advance(&self, ms: u64) {
            self.boot_ms.fetch_add(ms, Ordering::SeqCst);
        }

        pub fn reboot(&self) {
            *relock(&self.boot_id) = format!("boot-{}", self.asked.load(Ordering::SeqCst) + 100);
            self.boot_ms.store(1_000, Ordering::SeqCst);
        }

        pub fn clock(self: &Arc<Self>) -> TrustedClock {
            struct B(Arc<Fakes>);
            impl BootClock for B {
                fn now(&self) -> BootStamp {
                    BootStamp {
                        id: relock(&self.0.boot_id).clone(),
                        ms: self.0.boot_ms.load(Ordering::SeqCst),
                    }
                }
            }
            struct F(Arc<Fakes>);
            impl TimeFetcher for F {
                fn fetch(&self, boot: &dyn BootClock) -> Result<Vec<Sample>, SyncError> {
                    self.0.asked.fetch_add(1, Ordering::SeqCst);
                    match *relock(&self.0.servers) {
                        Ok(real) => {
                            let b = boot.now();
                            Ok(vec![Sample {
                                utc_ms: real as u64,
                                boot: b,
                            }])
                        }
                        Err(e) => Err(e),
                    }
                }
            }
            let wall = Arc::clone(self);
            TrustedClock::new(
                Arc::new(B(Arc::clone(self))),
                Arc::new(F(Arc::clone(self))),
                Arc::new(move || wall.wall_ms.load(Ordering::SeqCst)),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Fakes;
    use super::*;

    const REAL: u64 = 1_800_000_000_000;

    #[test]
    fn before_a_check_the_device_clock_is_used() {
        let f = Fakes::new(5_000, REAL + 7_000, REAL);
        let c = f.clock();
        assert_eq!(c.now_ms(), REAL + 7_000);
        assert_eq!(c.verified_utc_ms(), None);
        let s = c.status();
        assert!(!s.verified && s.enabled && s.skew_ms.is_none());
    }

    #[test]
    fn a_check_replaces_a_wrong_device_date_and_keeps_counting_offline() {
        // the person set the date a week forward
        let week = 7 * 24 * 3600 * 1000;
        let f = Fakes::new(5_000, REAL + week, REAL);
        let c = f.clock();
        let s = c.sync().unwrap();
        assert!(s.verified);
        assert_eq!(c.now_ms(), REAL);
        assert_eq!(s.skew_ms, Some(-(week as i64)));
        // time passes, the servers go away, and the date is moved again: the answer follows the boot clock only
        *relock(&f.servers) = Err(SyncError::Offline);
        f.advance(90_000);
        f.wall_ms.store(REAL - 10_000_000, Ordering::SeqCst);
        assert_eq!(c.now_ms(), REAL + 90_000);
        assert!(c.sync().is_err());
        assert_eq!(
            c.now_ms(),
            REAL + 90_000,
            "a failed check keeps the old link"
        );
        assert!(c.status().problem.is_some());
    }

    #[test]
    fn a_restart_forgets_the_check_until_the_next_one() {
        let f = Fakes::new(5_000, REAL + 123, REAL);
        let c = f.clock();
        c.sync().unwrap();
        assert!(c.verified_utc_ms().is_some());
        f.reboot();
        assert_eq!(
            c.verified_utc_ms(),
            None,
            "another boot id: the old link means nothing"
        );
        // the device date is all there is now
        assert_eq!(c.now_ms(), REAL + 123);
        c.sync().unwrap();
        assert_eq!(c.verified_utc_ms(), Some(REAL));
    }

    #[test]
    fn switching_the_check_off_forgets_it_and_blocks_requests() {
        let f = Fakes::new(5_000, REAL + 99, REAL);
        let c = f.clock();
        c.sync().unwrap();
        c.set_enabled(false);
        assert_eq!(c.verified_utc_ms(), None);
        assert_eq!(c.sync(), Err(SyncError::Disabled));
        assert_eq!(f.asked.load(Ordering::SeqCst), 1, "no request while off");
        c.set_enabled(true);
        c.sync().unwrap();
        assert!(c.status().verified);
    }

    #[test]
    fn servers_that_disagree_or_say_nonsense_are_not_trusted() {
        struct Two(i64);
        impl TimeFetcher for Two {
            fn fetch(&self, boot: &dyn BootClock) -> Result<Vec<Sample>, SyncError> {
                let b = boot.now();
                Ok(vec![
                    Sample {
                        utc_ms: REAL,
                        boot: b.clone(),
                    },
                    Sample {
                        utc_ms: (REAL as i64 + self.0) as u64,
                        boot: b,
                    },
                ])
            }
        }
        let f = Fakes::new(5_000, REAL, REAL);
        let boot = Arc::clone(&f.clock().boot);
        let close = TrustedClock::new(Arc::clone(&boot), Arc::new(Two(2_000)), Arc::new(|| REAL));
        assert_eq!(close.sync().unwrap().sources, 2);
        let far = TrustedClock::new(Arc::clone(&boot), Arc::new(Two(60_000)), Arc::new(|| REAL));
        assert_eq!(far.sync(), Err(SyncError::Disagree));
        assert!(!far.status().verified);
        // a server claiming 1970 is garbage
        let old = Fakes::new(5_000, REAL, 1_000_000);
        assert_eq!(old.clock().sync(), Err(SyncError::Unusable));
    }

    #[test]
    fn background_refresh_asks_once_then_waits() {
        let f = Fakes::new(5_000, REAL, REAL);
        let c = Arc::new(f.clock());
        assert!(c.due());
        c.sync().unwrap();
        assert!(!c.due(), "just checked");
        f.advance(REFRESH_AFTER_MS + 1);
        assert!(c.due(), "an old check is renewed");
        // a failed try is not repeated at once
        *relock(&f.servers) = Err(SyncError::Offline);
        let _ = c.sync();
        assert!(!c.due());
        f.advance(MIN_RETRY_MS + 1);
        assert!(c.due());
    }
}
