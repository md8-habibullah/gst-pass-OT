//! Behavioural tests of the gate state machine ([`GateCore`]) without any account: the equal-cost
//! rule, the 3-character rule, the write-ahead counter and its tamper handling, the panic paths,
//! the allow-list and the inactivity timer. Everything runs in temp folders.

use super::biometric::BiometricAuth;
use super::counter::{self, CounterRead, Loaded, GATE_FILE};
use super::*;
use crate::crypto::KdfParams;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

fn kdf() -> KdfParams {
    KdfParams {
        m_cost_kib: 19_456,
        t_cost: 2,
        p_cost: 1,
    }
}

fn code(e: StorageError) -> &'static str {
    e.code()
}

struct Fx {
    dir: tempfile::TempDir,
    /// How many times the idle timer (or a close) locked the vault.
    locks: Arc<AtomicUsize>,
    /// How many times the wipe closure ran.
    wipes: Arc<AtomicUsize>,
    /// The device's boot clock, its date setting and the time servers: tests move them instead of waiting.
    fakes: Arc<crate::clock::testing::Fakes>,
    /// The clock the gate measures waits on (shared across simulated restarts: the device stays up).
    time: Arc<crate::clock::TrustedClock>,
    gate: GateCore,
}

impl Fx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let locks = Arc::new(AtomicUsize::new(0));
        let real = crate::crypto::device_millis();
        let fakes = crate::clock::testing::Fakes::new(1_000_000, real, real);
        let time = Arc::new(fakes.clock());
        let gate = Self::open_in(dir.path(), &locks, &time);
        Self {
            dir,
            locks,
            wipes: Arc::new(AtomicUsize::new(0)),
            fakes,
            time,
            gate,
        }
    }

    fn open_in(
        dir: &Path,
        locks: &Arc<AtomicUsize>,
        time: &Arc<crate::clock::TrustedClock>,
    ) -> GateCore {
        let locks = Arc::clone(locks);
        let lock: LockVault = Arc::new(move || {
            locks.fetch_add(1, Ordering::SeqCst);
        });
        let gate = GateCore::open(dir, &kdf(), lock).unwrap();
        // The waits after wrong credentials are tested on their own (see the lock tests); everywhere else a wrong
        // attempt must not slow the next one down.
        gate.set_lock_unit(Duration::ZERO);
        gate.set_time(Arc::clone(time));
        gate
    }

    /// Simulate quitting and starting the app again.
    fn restart(&mut self) {
        self.gate = Self::open_in(self.dir.path(), &self.locks, &self.time);
    }

    /// Production waits (one lock "minute" is 60 s) on the test clock.
    fn real_waits(&self) {
        self.gate.set_lock_unit(Duration::from_secs(60));
    }

    /// Time passes: the boot clock and the date setting both move on.
    fn advance(&self, ms: u64) {
        self.fakes.advance(ms);
        self.fakes.wall_ms.fetch_add(ms, Ordering::SeqCst);
    }

    /// The device checks the time online (the fake servers know the real time).
    fn check_time(&self) {
        self.time.sync().expect("the fake servers answer");
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join(GATE_FILE)
    }

    fn submit(&self, text: &str) -> GateOutcome {
        let wipes = Arc::clone(&self.wipes);
        self.gate
            .submit(text, &move || {
                wipes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap()
    }

    fn submit_err(&self, text: &str) -> StorageError {
        self.gate
            .submit(text, &|| Ok(()))
            .expect_err("expected an error")
    }

    fn wipes(&self) -> usize {
        self.wipes.load(Ordering::SeqCst)
    }

    fn configure(&self, input: GateConfigInput) -> Result<GateStatus, StorageError> {
        self.gate.configure(input, &|_| Ok(()))
    }

    /// Configure a PIN gate (reverse panic on, panic mark on) and close it, like a restart does.
    fn pin_gate(&self, credential: &str) {
        self.configure(pin(credential)).unwrap();
        self.gate.close();
        assert!(self.gate.is_closed());
    }

    fn counter_on_disk(&self) -> CounterRead {
        match counter::load(self.dir.path()) {
            Loaded::Ready(_, c) => c,
            Loaded::Absent => panic!("no gate file"),
            Loaded::Corrupt => panic!("corrupt gate file"),
        }
    }

    fn attempts_on_disk(&self) -> u32 {
        match self.counter_on_disk() {
            CounterRead::Valid { attempts, .. } => attempts,
            CounterRead::Damaged { .. } => panic!("the counter on disk is damaged"),
        }
    }

    fn edit_file(&self, f: impl FnOnce(&mut Value)) {
        let mut v: Value = serde_json::from_slice(&std::fs::read(self.path()).unwrap()).unwrap();
        f(&mut v);
        std::fs::write(self.path(), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }

    fn answer(&self) -> u32 {
        self.gate.current_answer().expect("an equation")
    }

    /// Submit text that is not the current answer (so it cannot be right by accident).
    fn submit_not_the_answer(&self, text: &str) -> GateOutcome {
        if text == self.answer().to_string() {
            return self.submit("0");
        }
        self.submit(text)
    }
}

/// A PIN gate configuration: reverse panic on, panic mark on, nothing else.
fn pin(credential: &str) -> GateConfigInput {
    GateConfigInput {
        enabled: true,
        mode: GateMode::Pin,
        credential: Some(credential.into()),
        reverse_panic: true,
        panic_code: None,
        biometric: false,
        panic_button: true,
        master_password: "the master password".into(),
    }
}

/// `pin(credential)` with changes (the input type wipes itself on drop, so no struct update).
fn pin_with(credential: &str, change: impl FnOnce(&mut GateConfigInput)) -> GateConfigInput {
    let mut input = pin(credential);
    change(&mut input);
    input
}

fn is_failed(o: &GateOutcome, left: u32) -> bool {
    matches!(o, GateOutcome::Failed { attempts_left, .. } if *attempts_left == left)
}

// ------------------------------------------------------------------ not configured

#[test]
fn a_gate_that_is_not_configured_refuses_nothing() {
    let fx = Fx::new();
    let st = fx.gate.status();
    assert!(!st.enabled && st.open && st.equation.is_none() && st.attempts_left.is_none());
    assert!(!fx.gate.is_closed());
    for cmd in [
        "profile_list",
        "item_list",
        "open_url",
        "a_command_from_the_future",
    ] {
        assert!(fx.gate.before_command(cmd).is_ok(), "{cmd}");
    }
    // there is nothing to submit to: any text just reports "opened", costs nothing, writes nothing
    assert_eq!(fx.submit("1234"), GateOutcome::Opened);
    assert_eq!(fx.submit("7"), GateOutcome::Opened);
    assert_eq!(fx.gate.verifications(), 0);
    assert!(!fx.path().exists());
    assert_eq!(
        code(fx.gate.panic(&|| Ok(())).unwrap_err()),
        "invalid_input"
    );
}

// ------------------------------------------------------------------ configure

#[test]
fn configuring_stores_only_hashes_and_the_next_start_is_closed() {
    let mut fx = Fx::new();
    let st = fx.configure(pin("2468")).unwrap();
    assert!(st.enabled && st.open && st.equation.is_none());
    assert_eq!(st.mode, GateMode::Pin);
    assert!(st.panic_button && !st.biometric);

    let text = std::fs::read_to_string(fx.path()).unwrap();
    assert!(!text.contains("2468") && !text.contains("8642"));
    assert!(!text.to_lowercase().contains("the master password"));
    assert_eq!(
        text.matches("$argon2id$").count(),
        2,
        "credential and reversed hash"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(fx.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    fx.restart();
    let st = fx.gate.status();
    assert!(st.enabled && !st.open, "a started app is behind the gate");
    assert!(st.equation.is_some());
    assert_eq!(st.attempts_left, None);
    assert!(fx.gate.is_closed());
}

#[test]
fn the_rules_are_enforced_in_rust_with_the_front_end_messages() {
    let fx = Fx::new();
    let cases: Vec<(GateConfigInput, &str)> = vec![
        (pin("123"), "Use at least 4 digits."),
        (pin("12a4"), "A PIN uses digits only."),
        (pin(&"7".repeat(65)), "Use at most 64 digits."),
        (
            pin("1221"),
            "This reads the same backwards, so the reversed panic code would be the same as your passcode. Choose one that does not.",
        ),
        (
            pin_with("abc", |c| c.mode = GateMode::Password),
            "Use at least 4 characters.",
        ),
        (
            pin_with("1231", |c| c.mode = GateMode::Grid),
            "A pattern cannot pass through the same dot twice.",
        ),
        (
            pin_with("1230", |c| c.mode = GateMode::Grid),
            "A pattern uses the dots 1 to 9.",
        ),
        (
            pin_with("2468", |c| c.panic_code = Some("12".into())),
            "The panic code needs at least 4 digits.",
        ),
        (
            pin_with("2468", |c| c.panic_code = Some("2468".into())),
            "The panic code must be different from your passcode.",
        ),
        (
            pin_with("2468", |c| c.panic_code = Some("8642".into())),
            "That is your passcode reversed, which already triggers panic.",
        ),
    ];
    for (input, message) in cases {
        let e = fx.configure(input).unwrap_err();
        assert_eq!(e.code(), "invalid_input", "{message}");
        assert_eq!(e.to_string(), message);
        assert!(
            !fx.path().exists(),
            "a refused configuration writes nothing"
        );
        assert!(!fx.gate.is_enabled());
    }
    // a reversed code that is allowed when reverse panic is off
    fx.configure(pin_with("2468", |c| {
        c.reverse_panic = false;
        c.panic_code = Some("8642".into());
    }))
    .unwrap();
    // no credential at all, and an empty one
    let fx = Fx::new();
    for credential in [None, Some(String::new())] {
        let e = fx
            .configure(pin_with("x", |c| c.credential = credential))
            .unwrap_err();
        assert_eq!(
            (e.code(), e.to_string().as_str()),
            ("invalid_input", "Choose a passcode.")
        );
    }
}

#[test]
fn a_wrong_master_password_or_a_locked_vault_changes_nothing() {
    let fx = Fx::new();
    let e = fx
        .gate
        .configure(pin("2468"), &|_| Err(StorageError::WrongPassword))
        .unwrap_err();
    assert_eq!(e.code(), "wrong_password");
    assert!(!fx.path().exists() && !fx.gate.is_enabled());
    let e = fx
        .gate
        .configure(pin("2468"), &|_| Err(StorageError::Locked))
        .unwrap_err();
    assert_eq!(e.code(), "locked");
    let empty = pin_with("2468", |c| c.master_password = String::new());
    assert_eq!(code(fx.configure(empty).unwrap_err()), "invalid_input");
    assert!(!fx.path().exists());
    // the master password is never stored anywhere
    fx.configure(pin("2468")).unwrap();
    assert!(!std::fs::read_to_string(fx.path())
        .unwrap()
        .contains("master"));
}

#[test]
fn a_cheap_rule_failure_costs_no_hashing_and_no_master_check() {
    let fx = Fx::new();
    let asked = AtomicUsize::new(0);
    let e = fx
        .gate
        .configure(pin("12"), &|_| {
            asked.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert_eq!(fx.gate.verifications(), 0);
}

#[test]
fn keeping_replacing_and_removing_what_is_stored() {
    let mut fx = Fx::new();
    // no stored credential: null cannot keep anything
    let e = fx
        .configure(pin_with("x", |c| c.credential = None))
        .unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    fx.configure(pin_with("2468", |c| c.panic_code = Some("5151".into())))
        .unwrap();
    // keep the credential and the custom panic code (null), only flip the switches
    fx.configure(pin_with("x", |c| {
        c.credential = None;
        c.reverse_panic = false;
        c.panic_button = false;
    }))
    .unwrap();
    fx.restart();
    assert!(!fx.gate.status().panic_button);
    assert_eq!(
        fx.submit("2468"),
        GateOutcome::Opened,
        "the stored credential was kept"
    );
    fx.gate.close();
    // reversed panic is off now, the custom code is still there
    assert!(is_failed(&fx.submit("8642"), 15));
    assert_eq!(fx.submit("5151"), GateOutcome::Wiped);
    assert_eq!(fx.wipes(), 1);

    // "" removes the custom code
    let mut fx = Fx::new();
    fx.configure(pin_with("2468", |c| c.panic_code = Some("5151".into())))
        .unwrap();
    fx.configure(pin_with("x", |c| {
        c.credential = None;
        c.panic_code = Some(String::new());
    }))
    .unwrap();
    fx.restart();
    assert!(
        is_failed(&fx.submit("5151"), 15),
        "the removed code is an ordinary wrong guess"
    );
    assert_eq!(fx.wipes(), 0);
    fx.gate.close();

    // a changed mode needs a new credential
    fx.submit("2468");
    let e = fx
        .configure(pin_with("x", |c| {
            c.mode = GateMode::Password;
            c.credential = None;
        }))
        .unwrap_err();
    assert_eq!(e.to_string(), "Choose a passcode.");
    // reversed panic cannot be switched on from a kept credential: its text is not stored
    fx.configure(pin_with("2468", |c| c.reverse_panic = false))
        .unwrap();
    let e = fx
        .configure(pin_with("x", |c| {
            c.credential = None;
            c.reverse_panic = true;
        }))
        .unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    assert!(e.to_string().contains("Enter your passcode again"));
    // a kept credential still has to differ from a new custom code
    let e = fx
        .configure(pin_with("x", |c| {
            c.credential = None;
            c.reverse_panic = false;
            c.panic_code = Some("2468".into());
        }))
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "The panic code must be different from your passcode."
    );
}

#[test]
fn a_new_credential_must_differ_from_the_kept_custom_panic_code() {
    let fx = Fx::new();
    fx.configure(pin_with("2468", |c| c.panic_code = Some("5151".into())))
        .unwrap();
    let e = fx.configure(pin("5151")).unwrap_err();
    assert_eq!(
        e.to_string(),
        "The panic code must be different from your passcode."
    );
    let e = fx.configure(pin("1515")).unwrap_err();
    assert_eq!(
        e.to_string(),
        "That is your passcode reversed, which already triggers panic."
    );
    // the stored gate is unchanged by the refused attempts
    assert!(fx.gate.is_enabled());
}

#[test]
fn turning_the_gate_off_removes_the_hashes_and_the_counter_file() {
    let mut fx = Fx::new();
    fx.pin_gate("2468");
    assert!(is_failed(&fx.submit("1111"), 15));
    assert!(fx.path().exists());
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    let st = fx.configure(pin_with("x", |c| c.enabled = false)).unwrap();
    assert!(!st.enabled && st.open);
    assert!(!fx.path().exists(), "gate.json is gone");
    fx.restart();
    assert!(!fx.gate.is_closed() && !fx.gate.is_enabled());
    assert_eq!(fx.gate.status().attempts_left, None);
    // turning it off still needs the master password
    fx.configure(pin("2468")).unwrap();
    let e = fx
        .gate
        .configure(pin_with("x", |c| c.enabled = false), &|_| {
            Err(StorageError::WrongPassword)
        })
        .unwrap_err();
    assert_eq!(e.code(), "wrong_password");
    assert!(fx.path().exists());
}

#[test]
fn a_unicode_passcode_is_normalised_so_any_keyboard_works() {
    let mut fx = Fx::new();
    fx.configure(pin_with("cafe\u{301}!!", |c| {
        c.mode = GateMode::Password;
        c.reverse_panic = false;
    }))
    .unwrap();
    fx.restart();
    assert!(is_failed(&fx.submit("cafe!!!"), 15));
    assert_eq!(
        fx.submit("caf\u{e9}!!"),
        GateOutcome::Opened,
        "the composed form of the same text"
    );
}

// ------------------------------------------------------------------ the 3-character rule

#[test]
fn one_to_three_characters_are_maths_and_never_touch_a_credential_or_the_counter() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    let before = std::fs::read(fx.path()).unwrap();
    for text in ["", "1", "12", "246", "abc", "\u{1F600}\u{1F600}", "9 9"] {
        let out = fx.submit_not_the_answer(text);
        assert!(
            matches!(out, GateOutcome::Math { correct: false, .. }),
            "{text:?}: {out:?}"
        );
    }
    assert_eq!(fx.gate.verifications(), 0, "no Argon2id run for maths");
    assert_eq!(
        std::fs::read(fx.path()).unwrap(),
        before,
        "the counter file was not written"
    );
    assert_eq!(fx.gate.status().attempts_left, None);
    assert!(fx.gate.is_closed());
    assert_eq!(fx.wipes(), 0);
}

#[test]
fn the_right_answer_counts_a_streak_and_brings_a_new_equation() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    let answer = fx.answer().to_string();
    let GateOutcome::Math {
        correct,
        equation,
        streak,
    } = fx.submit(&answer)
    else {
        panic!("not maths")
    };
    assert!(correct);
    assert_eq!(streak, 1);
    assert_eq!(
        fx.gate.status().equation.as_deref(),
        Some(equation.as_str())
    );
    assert_eq!(fx.gate.status().streak, 1);

    // a wrong answer keeps the streak and still brings a new equation
    let wrong = if fx.answer() == 999 {
        1
    } else {
        fx.answer() + 1
    };
    let before = fx.gate.status().equation;
    let GateOutcome::Math {
        correct,
        equation,
        streak,
    } = fx.submit(&wrong.to_string())
    else {
        panic!("not maths")
    };
    assert!(!correct);
    assert_eq!(streak, 1);
    assert_ne!(Some(equation), before);

    // the typed text is measured first (at most 3 characters), then spaces around the number are
    // trimmed, and only an ASCII integer counts
    let mut checked_padded = false;
    for _ in 0..400 {
        let a = fx.answer();
        if a < 100 {
            let padded = format!("{a} ");
            assert!(matches!(
                fx.submit(&padded),
                GateOutcome::Math { correct: true, .. }
            ));
            checked_padded = true;
            break;
        }
        fx.submit(&a.to_string());
    }
    assert!(checked_padded, "an answer below 100 turned up");
    for junk in ["+7", "7.", "0x", "\u{661}\u{662}"] {
        let out = fx.submit_not_the_answer(junk);
        assert!(
            matches!(out, GateOutcome::Math { correct: false, .. }),
            "{junk:?}"
        );
    }
    assert_eq!(fx.gate.verifications(), 0);
}

#[test]
fn solving_the_maths_never_opens_anything() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    for _ in 0..20 {
        let a = fx.answer().to_string();
        fx.submit(&a);
    }
    assert!(fx.gate.is_closed(), "the equation is only a decoy");
    assert_eq!(fx.gate.status().streak, 20);
}

// ------------------------------------------------------------------ equal cost

#[test]
fn every_security_attempt_runs_exactly_three_verifications() {
    let mut fx = Fx::new();
    // all three hashes present
    fx.configure(pin_with("2468", |c| c.panic_code = Some("5151".into())))
        .unwrap();
    fx.restart();
    let cost = |fx: &Fx, text: &str| -> u64 {
        let before = fx.gate.verifications();
        let out = fx.submit(text);
        let spent = fx.gate.verifications() - before;
        if matches!(out, GateOutcome::Opened) {
            fx.gate.close();
        }
        spent
    };
    assert_eq!(cost(&fx, "1111"), 3, "wrong");
    assert_eq!(cost(&fx, "2468"), 3, "right");
    assert_eq!(cost(&fx, "8642"), 3, "reversed panic");
    assert_eq!(cost(&fx, "5151"), 3, "custom panic");
    assert_eq!(fx.wipes(), 2);

    // only the credential stored: the missing hashes are verified against a dummy
    let mut fx = Fx::new();
    fx.configure(pin_with("2468", |c| c.reverse_panic = false))
        .unwrap();
    fx.restart();
    for (text, opens) in [("1111", false), ("8642", false), ("2468", true)] {
        let before = fx.gate.verifications();
        let out = fx.submit(text);
        assert_eq!(fx.gate.verifications() - before, 3, "{text}");
        assert_eq!(matches!(out, GateOutcome::Opened), opens, "{text}");
    }
}

#[test]
fn an_endless_input_is_cut_and_still_costs_exactly_three() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    let before = fx.gate.verifications();
    assert!(matches!(
        fx.submit(&"9".repeat(100_000)),
        GateOutcome::Failed { .. }
    ));
    assert_eq!(fx.gate.verifications() - before, 3);
}

// ------------------------------------------------------------------ the waits after wrong credentials

fn wrong(fx: &Fx, n: u32) -> GateOutcome {
    fx.submit(&format!("{:04}", 5000 + n))
}

#[test]
fn the_wait_table_and_the_limit_are_what_the_product_promises() {
    assert_eq!(MAX_ATTEMPTS, 16);
    assert_eq!(
        LOCK_STEPS,
        [
            (3, 90),
            (6, 5 * 60),
            (9, 45 * 60),
            (12, 8 * 60 * 60),
            (15, 3 * 24 * 60 * 60)
        ]
    );
    for n in 0..=MAX_ATTEMPTS {
        let expected = LOCK_STEPS.iter().find(|(k, _)| *k == n).map(|(_, s)| *s);
        assert_eq!(lock_seconds_after(n), expected, "{n}");
    }
    // the wipe is the first thing after the last wait: no step lies at or beyond the limit
    assert!(LOCK_STEPS.iter().all(|(n, _)| *n < MAX_ATTEMPTS));
}

#[test]
fn three_wrong_credentials_start_a_wait_and_nothing_is_checked_or_counted_during_it() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.real_waits();
    assert!(is_failed(&wrong(&fx, 1), 15));
    assert!(is_failed(&wrong(&fx, 2), 14));
    assert!(fx.gate.status().locked_for_ms.is_none());
    // the third one starts the wait and says so
    let GateOutcome::Failed {
        attempts_left,
        locked_for_ms,
        ..
    } = wrong(&fx, 3)
    else {
        panic!("expected a failure")
    };
    assert_eq!((attempts_left, locked_for_ms), (13, Some(90_000)));
    assert_eq!(fx.attempts_on_disk(), 3);
    assert_eq!(fx.gate.status().locked_for_ms, Some(90_000));

    // during the wait even the RIGHT passcode is not looked at, and nothing is counted
    let verifications = fx.gate.verifications();
    fx.advance(30_000);
    assert_eq!(
        fx.submit("2468"),
        GateOutcome::Locked {
            retry_after_ms: 60_000,
            clock_pending: false,
        }
    );
    assert_eq!(
        fx.submit("2468"),
        GateOutcome::Locked {
            retry_after_ms: 60_000,
            clock_pending: false,
        }
    );
    assert_eq!(fx.gate.verifications(), verifications, "no Argon2id run");
    assert_eq!(fx.attempts_on_disk(), 3, "no attempt was used");
    assert!(fx.gate.is_closed());
    assert_eq!(fx.gate.status().locked_for_ms, Some(60_000));
    assert_eq!(fx.wipes(), 0);

    // a sum is still just a sum: the decoy keeps working while the real input is locked
    assert!(matches!(fx.submit("12"), GateOutcome::Math { .. }));

    // when the wait is over the right passcode opens it
    fx.advance(60_000);
    assert!(fx.gate.status().locked_for_ms.is_none());
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    assert_eq!(fx.attempts_on_disk(), 0, "and the counter starts again");
}

#[test]
fn every_step_of_the_ladder_waits_as_long_as_promised_and_the_sixteenth_wipes() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.real_waits();
    let mut waits = Vec::new();
    for n in 1..=15u32 {
        let out = wrong(&fx, n);
        let GateOutcome::Failed {
            attempts_left,
            locked_for_ms,
            ..
        } = out
        else {
            panic!("attempt {n}: {out:?}")
        };
        assert_eq!(attempts_left, MAX_ATTEMPTS - n, "attempt {n}");
        if let Some(ms) = locked_for_ms {
            waits.push((n, ms));
            // a second too early is still locked, the exact end is not
            fx.advance(ms - 1000);
            assert!(
                matches!(
                    wrong(&fx, 99),
                    GateOutcome::Locked {
                        retry_after_ms: 1000,
                        clock_pending: false,
                    }
                ),
                "attempt {n}"
            );
            fx.advance(1000);
        }
        assert_eq!(fx.gate.status().final_attempt, n == 15, "attempt {n}");
    }
    assert_eq!(
        waits,
        vec![
            (3, 90_000),
            (6, 300_000),
            (9, 2_700_000),
            (12, 28_800_000),
            (15, 259_200_000)
        ]
    );
    // the last chance: one attempt left, the screen warns, and a wrong one erases
    let st = fx.gate.status();
    assert_eq!(st.attempts_left, Some(1));
    assert!(st.final_attempt);
    assert_eq!(fx.wipes(), 0);
    assert_eq!(wrong(&fx, 16), GateOutcome::Wiped);
    assert_eq!(fx.wipes(), 1);
}

#[test]
fn the_right_passcode_on_the_last_attempt_still_opens() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.real_waits();
    for n in 1..=15u32 {
        if let GateOutcome::Failed {
            locked_for_ms: Some(ms),
            ..
        } = wrong(&fx, n)
        {
            fx.advance(ms);
        }
    }
    assert!(fx.gate.status().final_attempt);
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    assert_eq!(fx.wipes(), 0);
    assert_eq!(fx.attempts_on_disk(), 0);
}

#[test]
fn a_wait_survives_closing_the_app_and_a_force_close_in_the_middle_of_the_attempt() {
    let mut fx = Fx::new();
    fx.pin_gate("2468");
    fx.real_waits();
    wrong(&fx, 1);
    wrong(&fx, 2);
    // the third attempt is force-closed right after the write-ahead: it still costs the attempt AND starts the wait
    fx.gate.set_failpoint_after_write_ahead(true);
    assert_eq!(fx.submit_err("2468").code(), "storage");
    fx.restart();
    fx.real_waits();
    assert_eq!(fx.gate.status().attempts_left, Some(13));
    assert_eq!(fx.gate.status().locked_for_ms, Some(90_000));
    assert_eq!(
        fx.submit("2468"),
        GateOutcome::Locked {
            retry_after_ms: 90_000,
            clock_pending: false,
        }
    );
    // a restart does not shorten it, time passing does
    fx.advance(89_999);
    fx.restart();
    fx.real_waits();
    assert_eq!(
        fx.submit("2468"),
        GateOutcome::Locked {
            retry_after_ms: 1,
            clock_pending: false,
        }
    );
    fx.advance(1);
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
}

#[test]
fn a_wait_is_stored_under_the_same_tag_as_the_counter() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.real_waits();
    for n in 1..=3 {
        wrong(&fx, n);
    }
    let CounterRead::Valid { attempts, lock, .. } = fx.counter_on_disk() else {
        panic!("valid")
    };
    assert_eq!(attempts, 3);
    // the deadline is on the boot clock; the time was not checked, so there is no UTC deadline yet
    assert_eq!(
        lock.boot_ms,
        fx.fakes.boot_ms.load(Ordering::SeqCst) + 90_000
    );
    assert_eq!(lock.until_utc, 0);
    // shortening the stored wait by hand breaks the tag: the record is damaged, not trusted
    fx.edit_file(|v| v["counter"]["lockBoot"] = json!(0));
    assert!(matches!(fx.counter_on_disk(), CounterRead::Damaged { .. }));
}

/// Fail three times (the third starts the 90 s wait). `check` = ask the time servers first.
fn start_wait(fx: &Fx, check: bool) {
    fx.pin_gate("2468");
    fx.real_waits();
    if check {
        fx.check_time();
    }
    for n in 1..=3 {
        wrong(fx, n);
    }
    assert_eq!(fx.gate.status().locked_for_ms, Some(90_000));
}

fn locked(retry_after_ms: u64) -> GateOutcome {
    GateOutcome::Locked {
        retry_after_ms,
        clock_pending: false,
    }
}

const THREE_DAYS: u64 = 3 * 24 * 3600 * 1000;

#[test]
fn moving_the_device_date_never_shortens_a_wait() {
    for checked in [false, true] {
        let fx = Fx::new();
        start_wait(&fx, checked);
        // forward by three days, as someone would to skip the wait
        fx.fakes.wall_ms.fetch_add(THREE_DAYS, Ordering::SeqCst);
        assert_eq!(fx.submit("2468"), locked(90_000), "checked: {checked}");
        assert_eq!(fx.gate.status().locked_for_ms, Some(90_000));
        // back by a day: nothing gets longer either
        fx.fakes
            .wall_ms
            .fetch_sub(THREE_DAYS + 86_400_000, Ordering::SeqCst);
        assert_eq!(fx.gate.status().locked_for_ms, Some(90_000));
        // only the boot clock counts
        fx.fakes.advance(40_000);
        assert_eq!(fx.submit("2468"), locked(50_000));
        // and the same after the app is closed and started again
        let mut fx = fx;
        fx.restart();
        fx.real_waits();
        assert_eq!(fx.gate.status().locked_for_ms, Some(50_000));
        fx.fakes.advance(50_000);
        assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    }
}

#[test]
fn a_status_while_checked_gives_the_end_of_the_wait_in_utc() {
    let fx = Fx::new();
    start_wait(&fx, true);
    let st = fx.gate.status();
    let now = fx.time.verified_utc_ms().unwrap();
    assert_eq!(st.locked_until_ms, Some(now + 90_000));
    assert!(!st.clock_pending);
    // the stored record carries both deadlines
    let CounterRead::Valid { lock, .. } = fx.counter_on_disk() else {
        panic!("valid")
    };
    assert_eq!(lock.until_utc, now + 90_000);
}

#[test]
fn after_the_device_restarts_a_wait_ends_only_by_time_checked_online() {
    let mut fx = Fx::new();
    start_wait(&fx, true);
    let start = fx.time.verified_utc_ms().unwrap();
    // the device restarts; the person also sets the date three days ahead, offline
    fx.fakes.reboot();
    fx.fakes.wall_ms.fetch_add(THREE_DAYS, Ordering::SeqCst);
    *fx.fakes.servers.lock().unwrap() = Err(crate::clock::SyncError::Offline);
    fx.restart();
    fx.real_waits();
    let st = fx.gate.status();
    assert!(st.clock_pending, "cannot be judged without a checked time");
    assert_eq!((st.locked_for_ms, st.locked_until_ms), (None, None));
    // nothing is checked or counted meanwhile, not even the right passcode
    let before = fx.gate.verifications();
    assert_eq!(
        fx.submit("2468"),
        GateOutcome::Locked {
            retry_after_ms: 0,
            clock_pending: true
        }
    );
    assert_eq!(fx.gate.verifications(), before);
    assert_eq!(fx.attempts_on_disk(), 3);
    // the servers come back: thirty real seconds have passed, sixty are left
    *fx.fakes.servers.lock().unwrap() = Ok((start + 30_000) as i64);
    fx.check_time();
    assert!(!fx.gate.status().clock_pending);
    assert_eq!(fx.gate.status().locked_for_ms, Some(60_000));
    assert_eq!(fx.submit("2468"), locked(60_000));
    // sixty seconds later, on the boot clock, it opens
    fx.fakes.advance(60_000);
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
}

#[test]
fn a_restart_after_the_wait_really_ended_lets_the_passcode_in() {
    let mut fx = Fx::new();
    start_wait(&fx, true);
    let start = fx.time.verified_utc_ms().unwrap();
    fx.fakes.reboot();
    *fx.fakes.servers.lock().unwrap() = Ok((start + 5 * 60_000) as i64);
    fx.restart();
    fx.real_waits();
    fx.check_time();
    assert!(fx.gate.status().locked_for_ms.is_none());
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
}

#[test]
fn a_wait_that_began_before_any_check_starts_over_at_the_first_check_after_a_restart() {
    let mut fx = Fx::new();
    start_wait(&fx, false);
    fx.fakes.reboot();
    fx.restart();
    fx.real_waits();
    assert!(fx.gate.status().clock_pending);
    let real = crate::crypto::device_millis();
    *fx.fakes.servers.lock().unwrap() = Ok(real as i64);
    fx.check_time();
    // the full wait, from the first moment the time is known (the safe direction), written down once
    assert_eq!(fx.gate.status().locked_for_ms, Some(90_000));
    let epoch = || match fx.counter_on_disk() {
        CounterRead::Valid { epoch, .. } => epoch,
        CounterRead::Damaged { .. } => panic!("damaged"),
    };
    let written = epoch();
    fx.fakes.advance(30_000);
    assert_eq!(
        fx.gate.status().locked_for_ms,
        Some(60_000),
        "it does not start over again"
    );
    assert_eq!(epoch(), written);
    fx.fakes.advance(60_000);
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
}

#[test]
fn with_the_online_check_switched_off_a_restart_cannot_lock_the_person_out_for_good() {
    let mut fx = Fx::new();
    fx.time.set_enabled(false);
    start_wait(&fx, false);
    // the device restarts; no time is ever checked (the person switched that off)
    fx.fakes.reboot();
    fx.fakes.wall_ms.fetch_add(30_000, Ordering::SeqCst);
    fx.restart();
    fx.real_waits();
    let st = fx.gate.status();
    assert!(
        !st.clock_pending,
        "off means the device clock is the judge, not 'wait for ever'"
    );
    assert_eq!(st.locked_for_ms, Some(60_000));
    assert_eq!(fx.submit("2468"), locked(60_000));
    fx.fakes.wall_ms.fetch_add(60_000, Ordering::SeqCst);
    fx.fakes.advance(60_000);
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
}

#[test]
fn the_online_check_being_on_but_unreachable_still_fails_closed() {
    let mut fx = Fx::new();
    start_wait(&fx, true);
    fx.fakes.reboot();
    *fx.fakes.servers.lock().unwrap() = Err(crate::clock::SyncError::Offline);
    fx.restart();
    fx.real_waits();
    assert!(fx.time.is_enabled());
    assert!(fx.gate.status().clock_pending);
}

#[test]
fn a_record_from_before_the_boot_clock_is_judged_on_checked_time_only() {
    let mut fx = Fx::new();
    fx.pin_gate("2468");
    fx.real_waits();
    fx.check_time();
    let now = fx.time.verified_utc_ms().unwrap();
    // what the build before wrote: a UTC deadline and nothing else, signed as v2
    let key = match counter::load(fx.dir.path()) {
        Loaded::Ready(p, _) => *p.install_key,
        _ => panic!("ready"),
    };
    let until = now + 45_000;
    fx.edit_file(|v| {
        let c = v["counter"].as_object_mut().unwrap();
        c.remove("lockBoot");
        c.remove("lockBootId");
        c.insert("attempts".into(), json!(3));
        c.insert("lockUntil".into(), json!(until));
        c.insert(
            "tag".into(),
            json!(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD_NO_PAD,
                counter::tag_locked(&key, 3, 1, until)
            )),
        );
        c.insert("epoch".into(), json!(1));
    });
    fx.restart();
    fx.real_waits();
    assert_eq!(fx.gate.status().locked_for_ms, Some(45_000));
    fx.fakes.advance(45_000);
    assert!(fx.gate.status().locked_for_ms.is_none());
    // the same record on a device that has not checked the time since it started
    fx.fakes.reboot();
    fx.restart();
    fx.real_waits();
    *fx.fakes.servers.lock().unwrap() = Err(crate::clock::SyncError::Offline);
    let _ = fx.time.sync();
    assert!(
        fx.gate.status().clock_pending,
        "an old UTC deadline is never trusted without a check since the restart"
    );
}

#[test]
fn a_wrong_credential_after_a_wait_counts_from_where_it_stood() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.real_waits();
    for n in 1..=3 {
        wrong(&fx, n);
    }
    fx.advance(90_000);
    assert!(is_failed(&wrong(&fx, 4), 12));
    assert!(is_failed(&wrong(&fx, 5), 11));
    let out = wrong(&fx, 6);
    assert!(
        matches!(
            out,
            GateOutcome::Failed {
                attempts_left: 10,
                locked_for_ms: Some(300_000),
                ..
            }
        ),
        "{out:?}"
    );
}

#[test]
fn the_status_shows_the_countdown_and_the_final_warning() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.real_waits();
    let st = fx.gate.status();
    assert!(st.locked_for_ms.is_none() && !st.final_attempt && st.attempts_left.is_none());
    for n in 1..=3 {
        wrong(&fx, n);
    }
    assert_eq!(fx.gate.status().locked_for_ms, Some(90_000));
    fx.advance(45_000);
    assert_eq!(fx.gate.status().locked_for_ms, Some(45_000));
    fx.advance(45_000);
    assert_eq!(fx.gate.status().locked_for_ms, None);
    // an open gate shows neither
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    let st = fx.gate.status();
    assert!(st.locked_for_ms.is_none() && !st.final_attempt);
}

// ------------------------------------------------------------------ the counter

#[test]
fn failures_count_down_and_the_right_credential_resets_them() {
    let mut fx = Fx::new();
    fx.pin_gate("2468");
    assert!(is_failed(&fx.submit("0000"), 15));
    assert!(is_failed(&fx.submit("0001"), 14));
    assert_eq!(fx.gate.status().attempts_left, Some(14));
    assert_eq!(fx.attempts_on_disk(), 2);
    // the counter survives a restart
    fx.restart();
    assert_eq!(fx.gate.status().attempts_left, Some(14));
    assert!(is_failed(&fx.submit("0002"), 13));
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    assert_eq!(fx.attempts_on_disk(), 0, "success writes zero");
    assert_eq!(fx.gate.status().attempts_left, None);
    assert!(fx.gate.status().open && fx.gate.status().equation.is_none());
    assert_eq!(fx.wipes(), 0);
}

#[test]
fn the_sixteenth_failure_wipes_and_the_fifteenth_does_not() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    for left in (1..=15).rev() {
        let out = fx.submit(&format!("{:04}", 9000 + left));
        assert!(is_failed(&out, left), "{out:?}");
    }
    assert_eq!(fx.wipes(), 0);
    assert_eq!(fx.submit("9999"), GateOutcome::Wiped);
    assert_eq!(fx.wipes(), 1);
}

#[test]
fn the_wipe_outcome_is_exactly_the_same_for_every_trigger() {
    let mut seen = Vec::new();
    let json = |o: &GateOutcome| serde_json::to_string(o).unwrap();
    for input in ["8642", "5151"] {
        let fx = Fx::new();
        fx.configure(pin_with("2468", |c| c.panic_code = Some("5151".into())))
            .unwrap();
        fx.gate.close();
        seen.push(json(&fx.submit(input)));
    }
    let fx = Fx::new();
    fx.pin_gate("2468");
    seen.push(json(&fx.gate.panic(&|| Ok(())).unwrap()));
    let fx = Fx::new();
    fx.pin_gate("2468");
    let mut last = None;
    for i in 0..16 {
        last = Some(fx.submit(&format!("{:04}", 7000 + i)));
    }
    seen.push(json(&last.unwrap()));
    assert_eq!(seen, vec![r#"{"kind":"wiped"}"#; 4]);
}

#[test]
fn a_force_close_right_after_the_write_ahead_still_costs_one_attempt() {
    let mut fx = Fx::new();
    fx.pin_gate("2468");
    fx.gate.set_failpoint_after_write_ahead(true);
    // the person types the RIGHT passcode and the process dies right after the counter was written
    let e = fx.submit_err("2468");
    assert_eq!(e.code(), "storage");
    assert_eq!(
        fx.attempts_on_disk(),
        1,
        "attempts + 1 is on disk before the input is checked"
    );
    assert_eq!(
        fx.gate.verifications(),
        0,
        "the input was not looked at yet"
    );
    assert!(fx.gate.is_closed());
    // a real restart sees it
    fx.restart();
    assert_eq!(fx.gate.status().attempts_left, Some(15));
    assert!(fx.gate.is_closed(), "the attempt did not open the gate");
}

#[test]
fn if_the_attempt_cannot_be_recorded_the_input_is_never_checked() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    // make the replace fail: the temp name is taken by a folder
    std::fs::create_dir(fx.dir.path().join("gate.json.tmp")).unwrap();
    let e = fx.submit_err("2468");
    assert_eq!(e.code(), "storage");
    assert_eq!(
        e.to_string(),
        "The attempt could not be recorded, so it was not checked."
    );
    assert_eq!(fx.gate.verifications(), 0);
    assert!(fx.gate.is_closed());
    assert_eq!(fx.wipes(), 0);
}

#[test]
fn editing_the_counter_by_hand_costs_one_extra_attempt_and_is_repaired() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    for i in 0..3 {
        fx.submit(&format!("000{i}"));
    }
    assert_eq!(fx.attempts_on_disk(), 3);
    // someone sets the counter back to 0 in a text editor
    fx.edit_file(|v| v["counter"]["attempts"] = json!(0));
    assert!(matches!(
        fx.counter_on_disk(),
        CounterRead::Damaged { hint: Some(0) }
    ));
    let out = fx.submit("1111");
    // best known 3, one extra for the tamper, one for this attempt
    assert!(is_failed(&out, 11), "{out:?}");
    assert_eq!(fx.attempts_on_disk(), 5, "rewritten with a valid tag");
    assert_eq!(fx.wipes(), 0);
}

#[test]
fn a_missing_unparsable_or_forged_counter_is_never_a_wipe_by_itself() {
    type Patch = Box<dyn Fn(&mut Value)>;
    let patches: Vec<(&str, Patch)> = vec![
        (
            "removed",
            Box::new(|v| {
                v.as_object_mut().unwrap().remove("counter");
            }),
        ),
        ("junk", Box::new(|v| v["counter"] = json!("junk"))),
        ("array", Box::new(|v| v["counter"] = json!([1, 2, 3]))),
        ("bad tag", Box::new(|v| v["counter"]["tag"] = json!("AAAA"))),
        (
            "tag removed",
            Box::new(|v| {
                v["counter"].as_object_mut().unwrap().remove("tag");
            }),
        ),
        // a forged "exhausted" record must not wipe the device
        (
            "forged 16",
            Box::new(|v| {
                v["counter"]["attempts"] = json!(16);
                v["counter"]["tag"] = json!("AAAA");
            }),
        ),
        (
            "forged huge",
            Box::new(|v| v["counter"]["attempts"] = json!(4_000_000_000u64)),
        ),
    ];
    for (name, patch) in patches {
        let fx = Fx::new();
        fx.pin_gate("2468");
        fx.submit("0000");
        fx.edit_file(|v| patch(v));
        let out = fx.submit("1111");
        assert!(matches!(out, GateOutcome::Failed { .. }), "{name}: {out:?}");
        assert_eq!(fx.wipes(), 0, "{name}");
        assert!(
            fx.attempts_on_disk() >= 3,
            "{name}: at least one extra attempt"
        );
        // the repaired file works: the right credential still opens
        assert_eq!(fx.submit("2468"), GateOutcome::Opened, "{name}");
    }
}

#[test]
fn tampering_near_the_limit_never_crosses_it() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    for i in 0..12 {
        fx.submit(&format!("{:04}", 500 + i));
    }
    assert_eq!(fx.attempts_on_disk(), 12);
    fx.edit_file(|v| v["counter"]["attempts"] = json!(0));
    let out = fx.submit("1111");
    assert!(matches!(out, GateOutcome::Failed { .. }), "{out:?}");
    assert_eq!(fx.wipes(), 0);
    assert!(fx.attempts_on_disk() <= MAX_ATTEMPTS - 2);
}

#[test]
fn a_gate_file_deleted_while_running_is_rewritten_with_the_best_known_count() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.submit("0000");
    fx.submit("0001");
    std::fs::remove_file(fx.path()).unwrap();
    let out = fx.submit("0002");
    assert!(matches!(out, GateOutcome::Failed { .. }));
    assert!(fx.path().exists(), "the file is back");
    assert!(fx.attempts_on_disk() >= 4);
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
}

#[test]
fn restoring_an_older_copy_of_the_file_does_not_hand_back_attempts_in_the_same_run() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.submit("0000");
    let old = std::fs::read(fx.path()).unwrap();
    for i in 1..5 {
        fx.submit(&format!("000{i}"));
    }
    assert_eq!(fx.attempts_on_disk(), 5);
    std::fs::write(fx.path(), old).unwrap();
    let out = fx.submit("1111");
    assert!(matches!(out, GateOutcome::Failed { .. }));
    assert!(
        fx.attempts_on_disk() >= 6,
        "rolled back, but never below what this run knows"
    );
}

#[test]
fn an_exhausted_counter_finishes_the_wipe_that_was_owed_even_for_the_right_credential() {
    let mut fx = Fx::new();
    fx.pin_gate("2468");
    let Loaded::Ready(p, c) = counter::load(fx.dir.path()) else {
        panic!("no file")
    };
    let CounterRead::Valid { epoch, .. } = c else {
        panic!("damaged")
    };
    counter::store(fx.dir.path(), &p, MAX_ATTEMPTS, epoch + 1).unwrap();
    fx.restart();
    assert_eq!(fx.gate.status().attempts_left, Some(0));
    assert_eq!(fx.submit("2468"), GateOutcome::Wiped);
    assert_eq!(fx.wipes(), 1);
}

#[test]
fn a_refused_wipe_keeps_the_gate_closed_and_reports_a_fixed_sentence() {
    let fx = Fx::new();
    fx.configure(pin_with("2468", |c| c.panic_code = Some("5151".into())))
        .unwrap();
    fx.gate.close();
    let refuse = || {
        Err(StorageError::Storage(
            "This device could not be erased.".into(),
        ))
    };
    let e = fx.gate.submit("5151", &refuse).unwrap_err();
    assert_eq!(e.code(), "storage");
    assert!(fx.gate.is_closed(), "nothing opened");
    assert!(fx.gate.status().enabled);
    let e = fx.gate.panic(&refuse).unwrap_err();
    assert_eq!(e.code(), "storage");
    assert!(fx.gate.is_closed());
}

#[test]
fn forgetting_everything_zeroes_the_gate_and_the_settings() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.gate
        .set_settings(AppSettings {
            lock_after_minutes: 5,
            ..AppSettings::default()
        })
        .unwrap();
    fx.gate.forget_everything();
    let st = fx.gate.status();
    assert!(!st.enabled && st.open && st.equation.is_none() && st.attempts_left.is_none());
    assert_eq!(fx.gate.settings().lock_after_minutes, 15);
    assert!(!fx.gate.is_closed());
    assert!(fx.gate.current_answer().is_none());
}

#[test]
fn a_corrupt_gate_file_is_moved_aside_and_does_not_lock_anyone_out() {
    let mut fx = Fx::new();
    fx.pin_gate("2468");
    std::fs::write(fx.path(), b"{ this is not json").unwrap();
    fx.restart();
    assert!(!fx.gate.is_enabled() && !fx.gate.is_closed());
    assert!(!fx.path().exists());
    let aside: Vec<String> = std::fs::read_dir(fx.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("gate.json.corrupt-"))
        .collect();
    assert_eq!(
        aside.len(),
        1,
        "the damaged file is kept for the wipe, not deleted"
    );
}

// ------------------------------------------------------------------ biometric

struct FakeBio {
    available: bool,
    result: Result<bool, ()>,
    asked: AtomicUsize,
}

impl BiometricAuth for FakeBio {
    fn available(&self) -> bool {
        self.available
    }
    fn authenticate(&self) -> Result<bool, StorageError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.result
            .map_err(|()| StorageError::Storage("The fingerprint reader failed.".into()))
    }
}

fn bio(available: bool, result: Result<bool, ()>) -> Arc<FakeBio> {
    Arc::new(FakeBio {
        available,
        result,
        asked: AtomicUsize::new(0),
    })
}

#[test]
fn a_fingerprint_is_not_available_on_a_desktop() {
    let fx = Fx::new();
    fx.configure(pin_with("2468", |c| c.biometric = true))
        .unwrap();
    fx.gate.close();
    let st = fx.gate.status();
    assert!(
        !st.biometric_available && !st.biometric,
        "the real default provider has none"
    );
    assert_eq!(code(fx.gate.biometric().unwrap_err()), "invalid_input");
    assert!(fx.gate.is_closed());
    assert_eq!(fx.gate.status().attempts_left, None);
}

#[test]
fn a_fingerprint_opens_stage_one_only_and_never_counts_an_attempt() {
    let fx = Fx::new();
    fx.configure(pin_with("2468", |c| c.biometric = true))
        .unwrap();
    fx.gate.close();
    fx.submit("0000");
    fx.submit("0001");
    let file_before = std::fs::read(fx.path()).unwrap();
    let verifications = fx.gate.verifications();

    // a sensor that does not recognise the person: refused, nothing counted
    let reader = bio(true, Ok(false));
    fx.gate.set_biometric(reader.clone());
    let st = fx.gate.status();
    assert!(st.biometric_available && st.biometric);
    assert_eq!(code(fx.gate.biometric().unwrap_err()), "invalid_input");
    assert_eq!(reader.asked.load(Ordering::SeqCst), 1);
    // a reader that fails
    fx.gate.set_biometric(bio(true, Err(())));
    assert_eq!(code(fx.gate.biometric().unwrap_err()), "storage");
    assert!(fx.gate.is_closed());
    assert_eq!(
        std::fs::read(fx.path()).unwrap(),
        file_before,
        "not counted"
    );

    // recognised: Stage 1 opens, the counter is untouched (it is not reset either)
    fx.gate.set_biometric(bio(true, Ok(true)));
    assert_eq!(fx.gate.biometric().unwrap(), GateOutcome::Opened);
    assert!(!fx.gate.is_closed());
    assert_eq!(std::fs::read(fx.path()).unwrap(), file_before);
    assert_eq!(fx.gate.verifications(), verifications);
    assert_eq!(fx.gate.status().attempts_left, Some(14), "no reset either");
    // the gate is already open: no prompt is shown again
    let silent = bio(true, Ok(true));
    fx.gate.set_biometric(silent.clone());
    assert_eq!(fx.gate.biometric().unwrap(), GateOutcome::Opened);
    assert_eq!(silent.asked.load(Ordering::SeqCst), 0);
}

#[test]
fn a_fingerprint_that_is_switched_off_in_settings_does_nothing() {
    let fx = Fx::new();
    fx.configure(pin_with("2468", |c| c.biometric = false))
        .unwrap();
    fx.gate.close();
    let reader = bio(true, Ok(true));
    fx.gate.set_biometric(reader.clone());
    assert!(!fx.gate.status().biometric);
    assert_eq!(code(fx.gate.biometric().unwrap_err()), "invalid_input");
    assert_eq!(
        reader.asked.load(Ordering::SeqCst),
        0,
        "the prompt is never shown"
    );
    assert!(fx.gate.is_closed());
}

// ------------------------------------------------------------------ the panic mark

#[test]
fn the_panic_mark_works_only_when_on_and_only_while_the_gate_is_closed() {
    let fx = Fx::new();
    fx.configure(pin_with("2468", |c| c.panic_button = false))
        .unwrap();
    fx.gate.close();
    assert_eq!(
        code(fx.gate.panic(&|| Ok(())).unwrap_err()),
        "invalid_input",
        "switched off"
    );
    fx.configure(pin("2468")).unwrap();
    assert_eq!(
        code(fx.gate.panic(&|| Ok(())).unwrap_err()),
        "invalid_input",
        "the gate is open (you are inside the app)"
    );
    fx.gate.close();
    assert_eq!(fx.wipes(), 0);
    let wipes = Arc::clone(&fx.wipes);
    let out = fx
        .gate
        .panic(&move || {
            wipes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    assert_eq!(out, GateOutcome::Wiped);
    assert_eq!(fx.wipes(), 1);
}

// ------------------------------------------------------------------ the allow-list

#[test]
fn while_the_gate_is_closed_only_the_allow_list_answers() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    let allowed = [
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
    for cmd in allowed {
        assert!(
            fx.gate.before_command(cmd).is_ok(),
            "{cmd} must work while closed"
        );
    }
    let refused = [
        "profile_list",
        "profile_create",
        "profile_select",
        "profile_rename",
        "profile_remove",
        "profile_import_file",
        "profile_restore_from_backup",
        "vault_unlock",
        "vault_lock",
        "vault_export_file",
        "vault_change_password",
        "item_list",
        "item_get",
        "item_reveal",
        "item_create",
        "trash_list",
        "history_list",
        "totp_now",
        "backup_list",
        "backup_run",
        "open_url",
        "file_pick",
        "import_preview",
        "export_csv",
        "gate_configure",
        "gate_close",
        "app_settings_set",
        "disguise_set",
        "",
        "GATE_STATUS",
        "gate_status ",
        "a_command_from_the_future",
    ];
    for cmd in refused {
        let e = fx.gate.before_command(cmd).expect_err(cmd);
        assert_eq!(e.code(), "locked", "{cmd:?}");
    }
    // once opened, everything passes the guard again
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    for cmd in refused {
        assert!(
            fx.gate.before_command(cmd).is_ok(),
            "{cmd:?} after the gate opened"
        );
    }
}

// ------------------------------------------------------------------ the inactivity timer

fn fast_idle(fx: &Fx, unit_ms: u64) {
    fx.gate.set_idle_unit(Duration::from_millis(unit_ms));
    fx.gate
        .set_settings(AppSettings {
            lock_after_minutes: 1,
            ..AppSettings::default()
        })
        .unwrap();
}

fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_timer_locks_the_vault_and_closes_the_gate_after_the_scaled_minute() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    fast_idle(&fx, 300);
    fx.gate.before_command("item_list").unwrap();
    assert!(!fx.gate.is_closed());
    assert_eq!(fx.locks.load(Ordering::SeqCst), 0);
    wait_for("the idle lock", || fx.locks.load(Ordering::SeqCst) == 1);
    assert!(fx.gate.is_closed(), "the gate is closed again");
    assert!(fx.gate.status().equation.is_some());
    assert_eq!(
        code(fx.gate.before_command("item_list").unwrap_err()),
        "locked"
    );
    // and it fires only once for one period of inactivity
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(fx.locks.load(Ordering::SeqCst), 1);
}

#[test]
fn activity_postpones_the_lock_and_polling_does_not() {
    let fx = Fx::new();
    // gate off: the timer still locks the vault
    fast_idle(&fx, 1000);
    fx.gate.before_command("item_list").unwrap();
    // a ping every 100 ms for 1.2 s keeps it open (more than one period)
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(100));
        fx.gate.ping();
        assert_eq!(
            fx.locks.load(Ordering::SeqCst),
            0,
            "a ping postpones the lock"
        );
    }
    // any other command counts as activity too
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(100));
        fx.gate.before_command("item_get").unwrap();
    }
    assert_eq!(fx.locks.load(Ordering::SeqCst), 0);
    // polling does not: it is the UI asking whether anything changed
    let start = std::time::Instant::now();
    while fx.locks.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
        fx.gate.before_command("vault_status").unwrap();
        fx.gate.before_command("gate_status").unwrap();
    }
    assert_eq!(fx.locks.load(Ordering::SeqCst), 1);
    assert!(
        start.elapsed() < Duration::from_millis(4000),
        "polling did not postpone it: {:?}",
        start.elapsed()
    );
    // with the gate off there is nothing to close
    assert!(!fx.gate.is_closed());
}

#[test]
fn a_deadline_that_passed_while_the_machine_slept_is_enforced_by_the_next_command() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.submit("2468");
    fx.gate.before_command("item_list").unwrap();
    assert!(fx.gate.idle_armed());
    assert_eq!(fx.locks.load(Ordering::SeqCst), 0);
    // the wall clock jumped an hour ahead (a suspend) while the monotonic clock stood still
    fx.gate.enforce_idle_at(
        std::time::Instant::now(),
        std::time::SystemTime::now() + Duration::from_secs(3600),
    );
    assert_eq!(fx.locks.load(Ordering::SeqCst), 1);
    assert!(fx.gate.is_closed());
    // the deadline was consumed: a second look does nothing more
    fx.gate.enforce_idle_at(
        std::time::Instant::now(),
        std::time::SystemTime::now() + Duration::from_secs(7200),
    );
    assert_eq!(fx.locks.load(Ordering::SeqCst), 1);
}

#[test]
fn closing_the_gate_stops_the_timer_and_dropping_the_core_stops_the_watcher() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.submit("2468");
    fx.gate.before_command("item_list").unwrap();
    assert!(fx.gate.idle_armed());
    fx.gate.close();
    assert!(
        !fx.gate.idle_armed(),
        "nothing is open, so there is nothing to time out"
    );
    // dropping joins the watcher thread (the test would hang here if it did not stop)
    let dir = tempfile::tempdir().unwrap();
    let g = Fx::open_in(dir.path(), &fx.locks, &fx.time);
    g.before_command("item_list").unwrap();
    drop(g);
}

#[test]
fn the_timeout_setting_accepts_only_the_listed_values_and_is_stored() {
    let mut fx = Fx::new();
    assert_eq!(fx.gate.settings().lock_after_minutes, 15, "the default");
    for m in [1, 5, 15, 30, 60, 240] {
        let s = fx
            .gate
            .set_settings(AppSettings {
                lock_after_minutes: m,
                ..AppSettings::default()
            })
            .unwrap();
        assert_eq!(s.lock_after_minutes, m);
    }
    for bad in [0, 2, 10, 45, 1440, u32::MAX] {
        let e = fx
            .gate
            .set_settings(AppSettings {
                lock_after_minutes: bad,
                ..AppSettings::default()
            })
            .unwrap_err();
        assert_eq!(e.code(), "invalid_input", "{bad}");
    }
    assert_eq!(
        fx.gate.settings().lock_after_minutes,
        240,
        "a refused value changes nothing"
    );
    fx.restart();
    assert_eq!(fx.gate.settings().lock_after_minutes, 240, "it is stored");
}

// ------------------------------------------------------------------ leaving the app (phones and tablets)

fn leave_settings(fx: &Fx, unit_ms: u64, idle_minutes: u32, leave_seconds: u32) {
    fx.gate.set_idle_unit(Duration::from_millis(unit_ms));
    fx.gate
        .set_settings(AppSettings {
            lock_after_minutes: idle_minutes,
            lock_after_leave_seconds: leave_seconds,
        })
        .unwrap();
}

#[test]
fn leaving_the_app_with_zero_seconds_locks_at_once_and_with_more_it_waits() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    leave_settings(&fx, 60_000, 240, 0);
    fx.gate.app_leave();
    assert_eq!(
        fx.locks.load(Ordering::SeqCst),
        1,
        "locked the moment the person left"
    );
    assert!(fx.gate.is_closed());

    // 5 minutes (the default): nothing happens right away, and coming back in time changes nothing
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    leave_settings(&fx, 60_000, 240, 300);
    fx.gate.app_leave();
    assert_eq!(fx.locks.load(Ordering::SeqCst), 1);
    assert!(!fx.gate.is_closed());
    fx.gate.app_return();
    assert_eq!(fx.locks.load(Ordering::SeqCst), 1);
    assert!(!fx.gate.is_closed());
    assert!(fx.gate.idle_armed(), "the inactivity timer runs again");
}

#[test]
fn staying_away_longer_than_allowed_locks_the_vault_and_the_gate_once() {
    let fx = Fx::new();
    fx.pin_gate("2468");
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    // one "minute" is 200 ms: 15 seconds away = 50 ms, the inactivity limit (240 minutes) is far away
    leave_settings(&fx, 200, 240, 15);
    fx.gate.app_leave();
    wait_for("the lock after leaving", || {
        fx.locks.load(Ordering::SeqCst) == 1
    });
    assert!(fx.gate.is_closed());
    assert!(fx.gate.status().equation.is_some());
    // coming back afterwards locks nothing twice
    fx.gate.app_return();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fx.locks.load(Ordering::SeqCst), 1);
    assert!(fx.gate.is_closed());
}

#[test]
fn coming_back_in_time_cancels_the_lock_and_the_inactivity_timer_pauses_while_away() {
    let fx = Fx::new();
    // gate off: the vault alone is locked by both timers
    // 1 "minute" = 200 ms: inactivity after 1 minute (200 ms), leaving allowed for 300 s (1 s)
    leave_settings(&fx, 200, 1, 300);
    fx.gate.before_command("item_list").unwrap();
    fx.gate.app_leave();
    // the inactivity timer would have fired by now, but it does not run while the app is in the background
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(fx.locks.load(Ordering::SeqCst), 0);
    fx.gate.app_return();
    assert_eq!(
        fx.locks.load(Ordering::SeqCst),
        0,
        "back in time: nothing locked"
    );
    // the leave timer is stopped: it never fires later
    std::thread::sleep(Duration::from_millis(1200));
    // ... but the inactivity timer runs again from the moment the person came back, and fires once
    assert_eq!(fx.locks.load(Ordering::SeqCst), 1);
}

#[test]
fn leaving_and_returning_are_polling_commands_and_work_with_the_gate_closed() {
    assert!(is_polling("app_leave") && is_polling("app_return"));
    assert!(allowed_while_closed("app_leave") && allowed_while_closed("app_return"));
    let fx = Fx::new();
    fx.pin_gate("2468");
    // closed gate: the calls are allowed and change nothing visible
    fx.gate.before_command("app_leave").unwrap();
    fx.gate.before_command("app_return").unwrap();
    assert!(fx.gate.is_closed());
    // they must not restart the inactivity timer (the open app would never time out)
    assert_eq!(fx.submit("2468"), GateOutcome::Opened);
    fx.gate.close();
}

#[test]
fn the_leave_setting_accepts_only_the_listed_times_and_is_stored() {
    let mut fx = Fx::new();
    assert_eq!(
        fx.gate.settings().lock_after_leave_seconds,
        300,
        "the default"
    );
    for secs in [0, 15, 30, 60, 300, 900, 1800, 3600, 14_400] {
        let s = fx
            .gate
            .set_settings(AppSettings {
                lock_after_leave_seconds: secs,
                ..AppSettings::default()
            })
            .unwrap();
        assert_eq!(s.lock_after_leave_seconds, secs);
    }
    for bad in [1, 10, 45, 120, 86_400, u32::MAX] {
        let e = fx
            .gate
            .set_settings(AppSettings {
                lock_after_leave_seconds: bad,
                ..AppSettings::default()
            })
            .unwrap_err();
        assert_eq!(e.code(), "invalid_input", "{bad}");
    }
    assert_eq!(fx.gate.settings().lock_after_leave_seconds, 14_400);
    fx.restart();
    assert_eq!(
        fx.gate.settings().lock_after_leave_seconds,
        14_400,
        "it is stored"
    );
}

#[test]
fn a_settings_file_from_before_the_leave_setting_reads_with_the_default() {
    let fx = Fx::new();
    std::fs::write(
        fx.dir.path().join(settings::SETTINGS_FILE),
        br#"{"lockAfterMinutes": 30}"#,
    )
    .unwrap();
    let loaded = settings::load(fx.dir.path());
    assert_eq!(loaded.lock_after_minutes, 30);
    assert_eq!(loaded.lock_after_leave_seconds, 300);
}

// ------------------------------------------------------------------ the long sum that guards "erase everything"

#[test]
fn the_reset_sum_is_answered_once_and_a_wrong_answer_uses_it_up() {
    let fx = Fx::new();
    // no sum asked for: nothing is accepted
    assert!(!fx.gate.check_reset_answer("1"));
    let text = fx.gate.reset_challenge();
    assert!(text.len() > 12 && text.contains(' '), "{text}");
    let answer = fx.gate.reset_answer().unwrap();
    assert!(answer >= 100);
    // wrong, then even the right one is refused: the sum was used up
    assert!(!fx.gate.check_reset_answer(&(answer + 1).to_string()));
    assert!(!fx.gate.check_reset_answer(&answer.to_string()));
    // a new sum, answered right (spaces around it do not matter), is accepted exactly once
    fx.gate.reset_challenge();
    let answer = fx.gate.reset_answer().unwrap();
    assert!(fx.gate.check_reset_answer(&format!("  {answer} ")));
    assert!(!fx.gate.check_reset_answer(&answer.to_string()));
}

#[test]
fn a_new_reset_sum_replaces_the_old_one_and_odd_input_never_matches() {
    let fx = Fx::new();
    for bad in [
        "",
        " ",
        "abc",
        "-5",
        "1e3",
        "０１２",
        "99999999999999999999",
    ] {
        fx.gate.reset_challenge();
        assert!(!fx.gate.check_reset_answer(bad), "{bad:?}");
    }
    fx.gate.reset_challenge();
    let first = fx.gate.reset_answer().unwrap();
    let mut second = first;
    for _ in 0..50 {
        fx.gate.reset_challenge();
        second = fx.gate.reset_answer().unwrap();
        if second != first {
            break;
        }
    }
    assert!(second != first, "sums vary");
    assert!(
        !fx.gate.check_reset_answer(&first.to_string()),
        "an answer to an old sum is worthless"
    );
}

// ------------------------------------------------------------------ wire shapes

#[test]
fn outcomes_and_status_have_the_shapes_the_front_end_expects() {
    let v = |o: GateOutcome| serde_json::to_value(o).unwrap();
    assert_eq!(v(GateOutcome::Opened), json!({"kind": "opened"}));
    assert_eq!(v(GateOutcome::Wiped), json!({"kind": "wiped"}));
    assert_eq!(
        v(GateOutcome::Math {
            correct: true,
            equation: "1 + 2".into(),
            streak: 3
        }),
        json!({"kind": "math", "correct": true, "equation": "1 + 2", "streak": 3})
    );
    assert_eq!(
        v(GateOutcome::Failed {
            attempts_left: 14,
            equation: "4 * 5".into(),
            locked_for_ms: None
        }),
        json!({"kind": "failed", "attemptsLeft": 14, "equation": "4 * 5", "lockedForMs": null})
    );
    assert_eq!(
        v(GateOutcome::Failed {
            attempts_left: 13,
            equation: "4 * 5".into(),
            locked_for_ms: Some(90_000)
        }),
        json!({"kind": "failed", "attemptsLeft": 13, "equation": "4 * 5", "lockedForMs": 90000})
    );
    assert_eq!(
        v(GateOutcome::Locked {
            retry_after_ms: 5000,
            clock_pending: false,
        }),
        json!({"kind": "locked", "retryAfterMs": 5000, "clockPending": false})
    );
    let fx = Fx::new();
    let st = serde_json::to_value(fx.gate.status()).unwrap();
    let mut keys: Vec<&str> = st.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "attemptsLeft",
            "biometric",
            "biometricAvailable",
            "clockPending",
            "enabled",
            "equation",
            "finalAttempt",
            "lockedForMs",
            "lockedUntilMs",
            "mode",
            "open",
            "panicButton",
            "streak"
        ]
    );
    assert_eq!(st["mode"], "pin");
    assert_eq!(st["equation"], Value::Null);
    assert_eq!(st["attemptsLeft"], Value::Null);
}

#[test]
fn the_configure_input_reads_the_front_end_json_and_never_prints_a_secret() {
    let input: GateConfigInput = serde_json::from_value(json!({
        "enabled": true, "mode": "grid", "credential": "12589", "reversePanic": true,
        "panicCode": null, "biometric": true, "panicButton": false, "masterPassword": "pw pw pw pw"
    }))
    .unwrap();
    assert_eq!(input.mode, GateMode::Grid);
    assert!(input.panic_code.is_none() && input.reverse_panic && input.biometric);
    let printed = format!("{input:?}");
    assert!(!printed.contains("12589") && !printed.contains("pw pw"));
    for bad in [
        json!({"enabled": true, "mode": "fingerprint", "credential": "1234", "masterPassword": "x"}),
        json!({"mode": "pin", "masterPassword": "x"}),
        json!({"enabled": true, "mode": "pin", "credential": 1234, "masterPassword": "x"}),
    ] {
        assert!(serde_json::from_value::<GateConfigInput>(bad).is_err());
    }
}

#[test]
fn the_allow_list_has_no_duplicates_and_names_the_polling_commands() {
    let mut names = ALLOWED_WHILE_CLOSED.to_vec();
    names.sort_unstable();
    let n = names.len();
    names.dedup();
    assert_eq!(names.len(), n);
    for p in POLLING_COMMANDS {
        assert!(allowed_while_closed(p), "{p}");
    }
    assert!(!allowed_while_closed("item_list"));
}

#[test]
fn the_generator_can_be_seeded_for_tests_and_the_core_uses_it() {
    use super::equation::testing::SeededRand;
    let fx = Fx::new();
    fx.pin_gate("2468");
    fx.gate.set_rng(Box::new(SeededRand(77)));
    fx.gate.close();
    let expected = equation::generate(&mut SeededRand(77));
    assert_eq!(
        fx.gate.status().equation.as_deref(),
        Some(expected.text.as_str())
    );
    assert_eq!(fx.answer(), expected.answer);
    // the webview never sees the answer: it is not part of the status
    assert!(!serde_json::to_string(&fx.gate.status())
        .unwrap()
        .contains("answer"));
}
