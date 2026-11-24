//! The inactivity timer (`FEATURE_QUEUE.md` Q1: configurable 1, 5, 15 or 30 minutes, no "never",
//! enforced in Rust, not in the webview).
//!
//! One watcher thread per core waits on a condition variable **until the deadline** (no tick
//! loop). Any command except the polling ones refreshes the deadline; when it passes, the watcher
//! calls the expiry action (close the gate when it is on, lock the vault) and goes back to sleep
//! until the next activity. It holds no session lock while it waits, and the expiry action takes
//! the same locks a command takes, so it can never run inside a command's critical section.
//!
//! The deadline is kept on both the monotonic clock and the wall clock. The monotonic clock stops
//! while a laptop is suspended, the wall clock does not: every command (the polling ones too)
//! also asks [`IdleTimer::take_expired`], so a vault left open over a suspend is locked by the
//! first thing that touches the core afterwards, not minutes later.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

/// One "minute" in production. The shim's `--idle-unit-ms` shortens it for tests.
pub const MINUTE: Duration = Duration::from_secs(60);

struct State {
    deadline: Option<(Instant, SystemTime)>,
    /// Length of the timeout (minutes times the unit).
    period: Duration,
    shutdown: bool,
}

pub struct IdleTimer {
    state: Mutex<State>,
    wake: Condvar,
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl IdleTimer {
    pub fn new(period: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                deadline: None,
                period,
                shutdown: false,
            }),
            wake: Condvar::new(),
        })
    }

    /// Activity: the deadline moves to `now + period`.
    pub fn touch(&self) {
        let mut s = relock(&self.state);
        if s.shutdown {
            return;
        }
        s.deadline = Some((Instant::now() + s.period, SystemTime::now() + s.period));
        self.wake.notify_all();
    }

    /// Change the timeout. A running deadline restarts from now.
    pub fn set_period(&self, period: Duration) {
        let mut s = relock(&self.state);
        s.period = period;
        if !s.shutdown && s.deadline.is_some() {
            s.deadline = Some((Instant::now() + period, SystemTime::now() + period));
        }
        self.wake.notify_all();
    }

    /// Forget the deadline (nothing is open any more, so there is nothing to time out).
    pub fn disarm(&self) {
        relock(&self.state).deadline = None;
        self.wake.notify_all();
    }

    pub fn is_armed(&self) -> bool {
        relock(&self.state).deadline.is_some()
    }

    /// `true` exactly once per expired deadline: when either clock has passed it, the deadline is
    /// cleared and the caller must run the expiry action.
    pub fn take_expired(&self) -> bool {
        self.take_expired_at(Instant::now(), SystemTime::now())
    }

    pub(crate) fn take_expired_at(&self, mono: Instant, wall: SystemTime) -> bool {
        let mut s = relock(&self.state);
        match s.deadline {
            Some((m, w)) if mono >= m || wall >= w => {
                s.deadline = None;
                true
            }
            _ => false,
        }
    }

    /// Stop the watcher (the core is going away).
    pub fn shutdown(&self) {
        let mut s = relock(&self.state);
        s.shutdown = true;
        s.deadline = None;
        self.wake.notify_all();
    }

    /// The watcher loop. Runs on its own thread until [`shutdown`](Self::shutdown).
    pub fn watch(self: &Arc<Self>, on_expire: impl Fn()) {
        let mut guard = relock(&self.state);
        loop {
            if guard.shutdown {
                return;
            }
            match guard.deadline {
                None => {
                    guard = self.wake.wait(guard).unwrap_or_else(|p| p.into_inner());
                }
                Some((mono, wall)) => {
                    let now = Instant::now();
                    if now >= mono || SystemTime::now() >= wall {
                        guard.deadline = None;
                        // The action takes other locks: never run it inside this one.
                        drop(guard);
                        on_expire();
                        guard = relock(&self.state);
                    } else {
                        guard = self
                            .wake
                            .wait_timeout(guard, mono - now)
                            .unwrap_or_else(|p| p.into_inner())
                            .0;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn nothing_expires_until_it_is_touched() {
        let t = IdleTimer::new(ms(50));
        assert!(!t.is_armed());
        assert!(!t.take_expired_at(
            Instant::now() + Duration::from_secs(3600),
            SystemTime::now()
        ));
        t.touch();
        assert!(t.is_armed());
        assert!(!t.take_expired());
    }

    #[test]
    fn either_clock_can_expire_it_and_only_once() {
        let t = IdleTimer::new(Duration::from_secs(60));
        t.touch();
        let later = Duration::from_secs(61);
        // the monotonic clock did not advance (a suspend), the wall clock did
        assert!(t.take_expired_at(Instant::now(), SystemTime::now() + later));
        assert!(!t.take_expired_at(Instant::now() + later, SystemTime::now() + later));
        t.touch();
        // the wall clock was set back; the monotonic clock still counts
        assert!(t.take_expired_at(Instant::now() + later, SystemTime::now()));
    }

    #[test]
    fn touching_moves_the_deadline_and_changing_the_period_restarts_it() {
        let t = IdleTimer::new(Duration::from_secs(60));
        t.touch();
        let almost = Instant::now() + Duration::from_secs(59);
        assert!(!t.take_expired_at(almost, SystemTime::now()));
        t.set_period(Duration::from_secs(1000));
        assert!(!t.take_expired_at(Instant::now() + Duration::from_secs(61), SystemTime::now()));
        t.disarm();
        assert!(!t.is_armed());
        assert!(!t.take_expired_at(
            Instant::now() + Duration::from_secs(5000),
            SystemTime::now() + Duration::from_secs(5000)
        ));
    }

    #[test]
    fn the_watcher_fires_after_the_period_postpones_on_touch_and_stops_on_shutdown() {
        let t = IdleTimer::new(ms(600));
        let fired = Arc::new(AtomicUsize::new(0));
        let handle = {
            let t = Arc::clone(&t);
            let fired = Arc::clone(&fired);
            std::thread::spawn(move || {
                t.watch(|| {
                    fired.fetch_add(1, Ordering::SeqCst);
                })
            })
        };
        t.touch();
        // postponed by activity: touch every 100 ms for 800 ms (longer than one period), it must
        // not fire meanwhile
        for _ in 0..8 {
            std::thread::sleep(ms(100));
            t.touch();
        }
        assert_eq!(
            fired.load(Ordering::SeqCst),
            0,
            "activity postponed the timer"
        );
        // now stop touching: it fires once, not repeatedly
        let start = Instant::now();
        while fired.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(ms(20));
        }
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        assert!(
            start.elapsed() >= ms(300),
            "it did not fire early: {:?}",
            start.elapsed()
        );
        std::thread::sleep(ms(900));
        assert_eq!(fired.load(Ordering::SeqCst), 1, "one expiry per deadline");
        // woken again by the next activity
        t.touch();
        let start = Instant::now();
        while fired.load(Ordering::SeqCst) == 1 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(ms(20));
        }
        assert_eq!(fired.load(Ordering::SeqCst), 2);
        t.shutdown();
        handle.join().unwrap();
        t.touch();
        assert!(!t.is_armed(), "no timer after shutdown");
    }
}
