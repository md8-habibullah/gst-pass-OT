//! The gate on top of real accounts ([`AppCore`]): strict lock separation (the open session is gone
//! after every way of leaving it), the neutral status while the gate is closed, `gate_configure`
//! behind the master password, the idle lock with a real vault, and the wipe on a realistic
//! two-account folder. Everything runs in `tempfile` folders; the real data folder is never
//! passed to the wipe.

use super::wipe::{WipeObserver, WipeStep};
use super::*;
use crate::storage::db::Store;
use crate::storage::files::TestPicker;
use crate::storage::model::{ItemDraft, ItemType};
use crate::storage::session::{Session, SessionConfig, EDIT_TOKEN_TTL};
use crate::storage::AppCore;
use std::collections::BTreeMap;
use zeroize::Zeroizing;

const PW: &str = "correct horse battery staple";
const PW2: &str = "a second long master password";
/// Plaintext that is put in files the app itself writes unencrypted, to prove the wipe removes it.
const MARKER: &[u8] = b"MARKER-4f2a9c71-plaintext-bytes";
/// A PIN to search the files for. Sealed files are random bytes, and a short PIN such as `2468` shows up
/// in a megabyte of them by chance every so often, which a test must not mistake for a leak. Thirteen
/// digits cannot appear by chance (about 1 in 10^13 per position).
const FINDABLE_PIN: &str = "7194052863107";

fn cfg() -> SessionConfig {
    SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: EDIT_TOKEN_TTL,
    }
}

fn open_app(dir: &Path) -> AppCore {
    AppCore::open(dir, cfg(), Arc::new(TestPicker::default())).unwrap()
}

fn code<T: std::fmt::Debug>(r: Result<T, StorageError>) -> &'static str {
    r.expect_err("expected an error").code()
}

/// `app_reset` the way the screen does it: ask for the long sum, then answer it (the test reads the answer from the core).
fn erase(app: &AppCore, steps: u64) -> Result<(), StorageError> {
    app.app_reset_challenge();
    let answer = app
        .gate()
        .reset_answer()
        .expect("a sum is waiting")
        .to_string();
    app.app_reset(steps, &answer)
}

fn login(title: &str) -> ItemDraft {
    ItemDraft {
        item_type: ItemType::Login,
        title: title.into(),
        favorite: false,
        reprompt: false,
        tags: vec![],
        folder: None,
        fields: [("username", "ana@example.com"), ("password", "pw-111")]
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect::<BTreeMap<_, _>>(),
        custom_fields: vec![],
    }
}

fn pin(credential: &str, master: &str) -> GateConfigInput {
    GateConfigInput {
        enabled: true,
        mode: GateMode::Pin,
        credential: Some(credential.into()),
        reverse_panic: true,
        panic_code: None,
        biometric: false,
        panic_button: true,
        master_password: master.into(),
    }
}

fn pin_with_panic_code(credential: &str, master: &str, panic_code: &str) -> GateConfigInput {
    let mut input = pin(credential, master);
    input.panic_code = Some(panic_code.into());
    input
}

struct Fx {
    dir: tempfile::TempDir,
    app: AppCore,
}

impl Fx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let app = open_app(dir.path());
        Self { dir, app }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// An account that is created, selected and unlocked, with one item in it.
    fn account(&self, name: &str, password: &str) -> String {
        let info = self.app.profile_create(name, password.into()).unwrap();
        self.app
            .item_create(login(&format!("{name}'s login")))
            .unwrap();
        info.id
    }

    /// Everything in the data folder, relative, sorted (files and folders).
    fn tree(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                out.push(
                    e.path()
                        .strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), base, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self.root(), self.root(), &mut out);
        out.sort();
        out
    }

    fn all_bytes(&self) -> Vec<u8> {
        fn walk(dir: &Path, out: &mut Vec<u8>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let kind = e.file_type().unwrap();
                if kind.is_dir() {
                    walk(&e.path(), out);
                } else if kind.is_file() {
                    out.extend(std::fs::read(e.path()).unwrap());
                }
            }
        }
        let mut out = Vec::new();
        walk(self.root(), &mut out);
        out
    }
}

/// Whether the bytes hold `needle`. Sealed files are random bytes, so a short needle turns up in them
/// by chance and makes a test fail now and then for no reason; this refuses needles under 8 bytes.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    assert!(
        needle.len() >= 8,
        "a search for {} bytes can match random sealed bytes by chance; use a longer, distinctive value",
        needle.len()
    );
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The vault is closed for good: not "unlocked", no item and no password check answers (the keys,
/// the index and every edit token live inside the state that was dropped).
fn assert_session_gone(session: &Arc<Session>, how: &str) {
    assert!(!session.status().unwrap().unlocked, "{how}: still unlocked");
    assert_eq!(code(session.item_list()), "locked", "{how}");
    assert_eq!(code(session.trash_list()), "locked", "{how}");
    assert_eq!(code(session.confirm_password(PW.into())), "locked", "{how}");
}

// ------------------------------------------------------------------ lock separation

#[test]
fn the_session_is_gone_after_lock_switch_gate_close_wipe_remove_and_exit() {
    // lock
    let fx = Fx::new();
    fx.account("Ana", PW);
    let s = fx.app.test_session();
    assert!(s.status().unwrap().unlocked);
    fx.app.lock();
    assert_session_gone(&s, "lock");

    // creating another account selects it and wipes the old session; switching back wipes that
    let fx = Fx::new();
    let ana = fx.account("Ana", PW);
    let s_ana = fx.app.test_session();
    fx.account("Bob", PW2);
    assert_session_gone(&s_ana, "creating another account");
    let s_bob = fx.app.test_session();
    assert!(s_bob.status().unwrap().unlocked);
    fx.app.profile_select(&ana).unwrap();
    assert_session_gone(&s_bob, "profile switch");

    // gate_close, with the gate on and with the gate off
    for enabled in [true, false] {
        let fx = Fx::new();
        fx.account("Ana", PW);
        if enabled {
            fx.app.gate_configure(pin("2468", PW)).unwrap();
        }
        let s = fx.app.test_session();
        assert!(s.status().unwrap().unlocked);
        fx.app.gate_close();
        assert_session_gone(
            &s,
            if enabled {
                "gate_close (on)"
            } else {
                "gate_close (off)"
            },
        );
        assert_eq!(fx.app.gate().is_closed(), enabled);
    }

    // removing the open account
    let fx = Fx::new();
    let id = fx.account("Ana", PW);
    let s = fx.app.test_session();
    fx.app.profile_remove(&id, PW.into(), "CONFIRM").unwrap();
    assert_session_gone(&s, "profile_remove");

    // the wipe
    let fx = Fx::new();
    fx.account("Ana", PW);
    let s = fx.app.test_session();
    erase(&fx.app, 3).unwrap();
    assert_session_gone(&s, "wipe");

    // quitting the app
    let fx = Fx::new();
    fx.account("Ana", PW);
    let s = fx.app.test_session();
    let Fx { dir, app } = fx;
    drop(app);
    assert_session_gone(&s, "app exit");
    drop(dir);
}

#[test]
fn the_idle_timer_drops_the_session_and_a_command_in_flight_never_races_it() {
    let fx = Fx::new();
    fx.account("Ana", PW);
    let s = fx.app.test_session();
    fx.app.gate().set_idle_unit(Duration::from_millis(400));
    fx.app
        .app_settings_set(AppSettings {
            lock_after_minutes: 1,
            ..AppSettings::default()
        })
        .unwrap();
    fx.app.before_command("item_list").unwrap();
    let app = Arc::new(fx.app);
    // four threads hammer the vault while the timer runs out: every answer is an item list or
    // `locked`, never anything else, and no thread hangs
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let app = Arc::clone(&app);
            std::thread::spawn(move || {
                let start = std::time::Instant::now();
                loop {
                    match app.item_list() {
                        Ok(items) => assert_eq!(items.len(), 1),
                        Err(e) => {
                            assert_eq!(e.code(), "locked");
                            return;
                        }
                    }
                    assert!(start.elapsed() < Duration::from_secs(10), "never locked");
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    assert_session_gone(&s, "idle lock");
    assert_eq!(code(app.item_list()), "locked");
    // the next unlock works as always
    app.unlock(PW.into()).unwrap();
    assert_eq!(app.item_list().unwrap().len(), 1);
}

#[test]
fn with_the_gate_on_the_idle_lock_also_closes_the_gate_and_activity_postpones_it() {
    let fx = Fx::new();
    fx.account("Ana", PW);
    fx.app.gate_configure(pin("2468", PW)).unwrap();
    fx.app.gate().set_idle_unit(Duration::from_millis(1000));
    fx.app
        .app_settings_set(AppSettings {
            lock_after_minutes: 1,
            ..AppSettings::default()
        })
        .unwrap();
    // pings keep both the vault and the gate open well past one period
    for _ in 0..14 {
        std::thread::sleep(Duration::from_millis(100));
        fx.app.activity_ping();
        assert!(fx.app.status().unwrap().unlocked);
        assert!(!fx.app.gate().is_closed());
    }
    // polling alone does not
    let start = std::time::Instant::now();
    while !fx.app.gate().is_closed() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
        let _ = fx.app.before_command("vault_status");
        let _ = fx.app.before_command("gate_status");
    }
    assert!(fx.app.gate().is_closed());
    assert!(start.elapsed() < Duration::from_millis(4000));
    // the next call is refused with `locked`, and the status reveals nothing
    assert_eq!(code(fx.app.before_command("item_list")), "locked");
    assert_eq!(code(fx.app.before_command("profile_list")), "locked");
    let st = fx.app.status().unwrap();
    assert!(!st.exists && !st.unlocked && st.profile.is_none());
    // ... and the account behind it is locked
    assert!(!fx.app.test_session().status().unwrap().unlocked);
    let g = fx.app.gate_status();
    assert!(g.enabled && !g.open && g.equation.is_some());
}

// ------------------------------------------------------------------ configure, status

#[test]
fn gate_configure_needs_an_unlocked_vault_and_its_master_password() {
    let fx = Fx::new();
    // no account at all
    assert_eq!(code(fx.app.gate_configure(pin("2468", PW))), "locked");
    fx.account("Ana", PW);
    assert_eq!(
        code(
            fx.app
                .gate_configure(pin("2468", "not the master password"))
        ),
        "wrong_password"
    );
    assert!(!fx.app.gate().is_enabled());
    assert!(!fx.root().join("gate.json").exists());
    // a rule failure is reported before any Argon2id work on the master password
    assert_eq!(code(fx.app.gate_configure(pin("12", PW))), "invalid_input");
    let st = fx.app.gate_configure(pin(FINDABLE_PIN, PW)).unwrap();
    assert!(st.enabled && st.open);
    // a locked vault cannot reconfigure it
    fx.app.lock();
    assert_eq!(code(fx.app.gate_configure(pin("1357", PW))), "locked");
    // the master password is not in the gate file
    let file = std::fs::read(fx.root().join("gate.json")).unwrap();
    assert!(!contains(&file, PW.as_bytes()) && !contains(&file, FINDABLE_PIN.as_bytes()));
}

#[test]
fn a_closed_gate_answers_the_status_neutrally_and_reveals_no_account_name() {
    let fx = Fx::new();
    fx.account("Ana Secret Name", PW);
    let before = fx.app.status().unwrap();
    assert!(before.unlocked && before.profile.is_some());
    fx.app.gate_configure(pin("2468", PW)).unwrap();
    fx.app.gate_close();
    let st = serde_json::to_value(fx.app.status().unwrap()).unwrap();
    assert_eq!(st["exists"], false);
    assert_eq!(st["unlocked"], false);
    assert_eq!(st["profile"], serde_json::Value::Null);
    assert!(!st.to_string().contains("Ana"));
    assert!(st.get("capabilities").is_none());
    // the right PIN opens Stage 1: the account shows again, still locked
    let out = fx.app.gate_submit(Zeroizing::new("2468".into())).unwrap();
    assert_eq!(out, GateOutcome::Opened);
    let st = fx.app.status().unwrap();
    assert!(st.exists && !st.unlocked);
    assert_eq!(st.profile.unwrap().display_name, "Ana Secret Name");
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(fx.app.item_list().unwrap().len(), 1);
}

#[test]
fn opening_the_gate_never_unlocks_a_vault() {
    let fx = Fx::new();
    fx.account("Ana", PW);
    fx.app.gate_configure(pin("2468", PW)).unwrap();
    fx.app.gate_close();
    assert_eq!(
        fx.app.gate_submit(Zeroizing::new("2468".into())).unwrap(),
        GateOutcome::Opened
    );
    // Stage 1 passed, the vault is still locked: only the master password opens it
    assert!(!fx.app.status().unwrap().unlocked);
    assert_eq!(code(fx.app.item_list()), "locked");
    assert_eq!(
        code(fx.app.unlock("2468 is not a master".into())),
        "wrong_password"
    );
    assert_eq!(code(fx.app.unlock("2468".into())), "wrong_password");
    fx.app.unlock(PW.into()).unwrap();
}

#[test]
fn nothing_that_could_unlock_a_vault_is_ever_written_to_disk() {
    let fx = Fx::new();
    fx.account("Ana", PW);
    fx.app.gate_configure(pin(FINDABLE_PIN, PW)).unwrap();
    fx.app
        .app_settings_set(AppSettings {
            lock_after_minutes: 5,
            ..AppSettings::default()
        })
        .unwrap();
    fx.app.gate_close();
    fx.app
        .gate_submit(Zeroizing::new(FINDABLE_PIN.into()))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    fx.app.gate_close();
    let known = ["app-settings.json", "gate.json", "profiles.json", "vaults"];
    for entry in std::fs::read_dir(fx.root()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(known.contains(&name.as_str()), "unexpected file {name}");
    }
    let bytes = fx.all_bytes();
    assert!(!contains(&bytes, PW.as_bytes()));
    assert!(!contains(&bytes, FINDABLE_PIN.as_bytes()));
}

// ------------------------------------------------------------------ the wipe

/// Lay out a realistic folder: two accounts (real vault files, WAL when SQLite leaves one),
/// attachment-like files, plaintext files that carry a marker, and one file that is not Hexis's.
fn realistic(fx: &Fx) -> (String, String) {
    let ana = fx.account("Ana", PW);
    let bob = fx.account("Bob", PW2);
    fx.app.gate_configure(pin("2468", PW2)).unwrap();
    for (id, tag) in [(&ana, "a"), (&bob, "b")] {
        let attachments = fx.root().join("vaults").join(id).join("attachments");
        std::fs::create_dir_all(&attachments).unwrap();
        for n in 0..3u8 {
            let mut data = MARKER.to_vec();
            data.extend(vec![n; 70_000]);
            std::fs::write(attachments.join(format!("{tag}{n}.bin")), data).unwrap();
        }
    }
    std::fs::write(fx.root().join("disguise.json"), MARKER).unwrap();
    std::fs::write(fx.root().join("notes.txt"), b"not Hexis's, keep me").unwrap();
    (ana, bob)
}

#[test]
fn erase_everything_needs_the_long_sum_and_a_wrong_answer_erases_nothing() {
    let fx = Fx::new();
    realistic(&fx);
    // no sum was asked for: nothing is accepted
    assert_eq!(code(fx.app.app_reset(3, "100")), "wrong_answer");
    // with the gate closed the sum can still be asked for and answered (closing locks the vault, which tidies its
    // journal files: the folder is compared from here on)
    fx.app.gate_close();
    let before = fx.tree();
    fx.app.before_command("app_reset_challenge").unwrap();
    let text = fx.app.app_reset_challenge();
    assert!(text.len() > 12, "{text}");
    let answer = fx.app.gate().reset_answer().unwrap();
    assert_eq!(
        code(fx.app.app_reset(3, &(answer + 1).to_string())),
        "wrong_answer"
    );
    assert_eq!(fx.tree(), before, "a wrong answer erases nothing");
    assert_eq!(
        code(fx.app.app_reset(3, &answer.to_string())),
        "wrong_answer",
        "the sum was used up by the wrong answer"
    );
    assert_eq!(fx.tree(), before);
    assert!(fx.app.gate().is_enabled());
    // steps are checked before the sum, so a wrong step count never even uses the sum up
    fx.app.app_reset_challenge();
    assert_eq!(code(fx.app.app_reset(2, "1")), "invalid_input");
    assert!(fx.app.gate().reset_answer().is_some());
    // the right answer to a fresh sum erases
    erase(&fx.app, 3).unwrap();
    assert_eq!(fx.tree(), vec!["notes.txt".to_string()]);
}

#[test]
fn leaving_the_app_locks_the_vault_after_the_chosen_time_and_not_before() {
    // 0 seconds: the vault is locked the moment the person leaves
    let fx = Fx::new();
    fx.account("Ana", PW);
    let s = fx.app.test_session();
    fx.app
        .app_settings_set(AppSettings {
            lock_after_leave_seconds: 0,
            ..AppSettings::default()
        })
        .unwrap();
    fx.app.app_leave();
    assert_session_gone(&s, "leaving the app");
    assert_eq!(code(fx.app.item_list()), "locked");
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(fx.app.item_list().unwrap().len(), 1);

    // 5 minutes: leaving and coming straight back keeps the vault open and every command works without a prompt
    fx.app
        .app_settings_set(AppSettings {
            lock_after_leave_seconds: 300,
            ..AppSettings::default()
        })
        .unwrap();
    fx.app.app_leave();
    fx.app.app_return();
    assert!(fx.app.status().unwrap().unlocked);
    assert_eq!(fx.app.item_list().unwrap().len(), 1);

    // with the gate on, staying away longer than allowed locks the vault and closes the gate
    let fx = Fx::new();
    fx.account("Ana", PW);
    fx.app.gate_configure(pin("2468", PW)).unwrap();
    fx.app.gate().set_idle_unit(Duration::from_millis(200));
    fx.app
        .app_settings_set(AppSettings {
            lock_after_minutes: 240,
            lock_after_leave_seconds: 15,
        })
        .unwrap();
    fx.app.app_leave();
    let start = std::time::Instant::now();
    while !fx.app.gate().is_closed() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(fx.app.gate().is_closed());
    assert!(!fx.app.test_session().status().unwrap().unlocked);
}

#[test]
fn the_manual_reset_wipes_a_realistic_tree_and_leaves_the_app_at_first_run() {
    let fx = Fx::new();
    let (ana, bob) = realistic(&fx);
    // a precious neighbour outside the data folder is never touched
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("precious.txt"), b"keep").unwrap();
    assert!(fx.tree().contains(&format!("vaults/{ana}/vault.db")));
    assert!(contains(&fx.all_bytes(), MARKER));
    let old_session = fx.app.test_session();

    // wrong confirmations touch nothing
    for steps in [0, 1, 2, 4, 99] {
        assert_eq!(code(erase(&fx.app, steps)), "invalid_input", "{steps}");
    }
    assert!(fx.tree().contains(&format!("vaults/{bob}/vault.db")));
    assert!(fx.app.gate().is_enabled());

    erase(&fx.app, 3).unwrap();
    assert_eq!(
        fx.tree(),
        vec!["notes.txt".to_string()],
        "only the unknown file is left"
    );
    assert!(
        !contains(&fx.all_bytes(), MARKER),
        "no byte of the marker survives"
    );
    assert_eq!(
        std::fs::read(fx.root().join("notes.txt")).unwrap(),
        b"not Hexis's, keep me"
    );
    assert_eq!(
        std::fs::read(outside.path().join("precious.txt")).unwrap(),
        b"keep"
    );
    assert_session_gone(&old_session, "reset");

    // first-run state: no accounts, nothing selected, the gate off with defaults, settings default
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());
    let st = fx.app.status().unwrap();
    assert!(!st.exists && !st.unlocked && st.profile.is_none());
    let g = fx.app.gate_status();
    assert!(!g.enabled && g.open && g.equation.is_none() && g.attempts_left.is_none());
    assert_eq!(fx.app.app_settings_get().lock_after_minutes, 15);
    assert!(!fx.app.gate().is_closed());
    assert!(fx.app.before_command("profile_list").is_ok());

    // a second wipe is a no-op, not an error
    erase(&fx.app, 3).unwrap();
    assert_eq!(fx.tree(), vec!["notes.txt".to_string()]);

    // and the app is usable again from scratch
    fx.account("Cleo", PW);
    assert_eq!(fx.app.profile_list().unwrap().profiles.len(), 1);
    fx.app.lock();
    let again = open_app(fx.root());
    let names: Vec<String> = again
        .profile_list()
        .unwrap()
        .profiles
        .into_iter()
        .map(|p| p.display_name)
        .collect();
    assert_eq!(names, vec!["Cleo".to_string()]);
}

#[test]
fn a_panic_pin_wipes_silently_with_exactly_the_wiped_outcome() {
    let fx = Fx::new();
    realistic(&fx);
    fx.app.gate_close();
    assert!(fx.app.gate().is_closed());
    // the reversed PIN looks like a failed attempt on the screen, but it is the wipe
    let out = fx.app.gate_submit(Zeroizing::new("8642".into())).unwrap();
    assert_eq!(serde_json::to_string(&out).unwrap(), r#"{"kind":"wiped"}"#);
    assert_eq!(fx.tree(), vec!["notes.txt".to_string()]);
    assert!(!contains(&fx.all_bytes(), MARKER));
    // the UI finds a first-run state with the gate disabled and the counter file gone
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());
    assert!(!fx.app.gate_status().enabled);
    assert!(!fx.root().join("gate.json").exists());
}

#[test]
fn the_panic_mark_and_the_sixteenth_failure_run_the_same_wipe() {
    // the mark
    let fx = Fx::new();
    realistic(&fx);
    fx.app.gate_close();
    let out = fx.app.gate_panic().unwrap();
    assert_eq!(serde_json::to_string(&out).unwrap(), r#"{"kind":"wiped"}"#);
    assert_eq!(fx.tree(), vec!["notes.txt".to_string()]);
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());

    // the 16th failure
    let fx = Fx::new();
    realistic(&fx);
    fx.app.gate_close();
    fx.app.gate().set_lock_unit(Duration::ZERO);
    let mut last = None;
    for i in 0..16 {
        last = Some(
            fx.app
                .gate_submit(Zeroizing::new(format!("{:04}", 3000 + i)))
                .unwrap(),
        );
    }
    assert_eq!(
        serde_json::to_string(&last.unwrap()).unwrap(),
        r#"{"kind":"wiped"}"#
    );
    assert_eq!(fx.tree(), vec!["notes.txt".to_string()]);
    assert!(!fx.root().join("gate.json").exists());
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());
}

/// Records every step and looks at the vault files when their key bundle is reported destroyed.
struct Recorder {
    steps: Vec<WipeStep>,
    /// Pieces of each vault's sealed key material.
    sealed: Vec<Vec<u8>>,
    /// Vault files that still held key material when their bundle was "destroyed".
    leaks: Vec<PathBuf>,
    /// Stop (simulating a power cut) once this many steps were seen.
    stop_after: usize,
}

impl WipeObserver for Recorder {
    fn step(&mut self, step: &WipeStep) -> bool {
        if let WipeStep::KeyBundleDestroyed(db) = step {
            assert!(
                !self
                    .steps
                    .iter()
                    .any(|s| matches!(s, WipeStep::Overwritten(_) | WipeStep::Unlinked(_))),
                "a file was already overwritten or unlinked before the key bundles were destroyed"
            );
            let mut bytes = std::fs::read(db).unwrap_or_default();
            let mut wal = db.clone().into_os_string();
            wal.push("-wal");
            bytes.extend(std::fs::read(PathBuf::from(wal)).unwrap_or_default());
            if self.sealed.iter().any(|s| contains(&bytes, s)) {
                self.leaks.push(db.clone());
            }
        }
        self.steps.push(step.clone());
        self.steps.len() < self.stop_after
    }
}

/// Three short pieces of a vault's sealed keyring (a long string can straddle a database page, so
/// one piece alone may be split by SQLite's overflow pointer).
fn sealed_pieces(db: &Path) -> Vec<Vec<u8>> {
    let json = Store::open(db).unwrap().bundle_json().unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let key = v
        .as_object()
        .unwrap()
        .iter()
        .find(|(k, _)| k.to_lowercase().contains("sealed"))
        .map(|(_, v)| v.as_str().unwrap().as_bytes().to_vec())
        .expect("a sealed keyring field");
    assert!(key.len() > 200);
    let mid = key.len() / 2;
    vec![
        key[..40].to_vec(),
        key[mid..mid + 40].to_vec(),
        key[key.len() - 40..].to_vec(),
    ]
}

fn holds_any(bytes: &[u8], pieces: &[Vec<u8>]) -> bool {
    pieces.iter().any(|p| contains(bytes, p))
}

#[test]
fn every_key_bundle_is_destroyed_before_any_file_is_overwritten_or_unlinked() {
    let fx = Fx::new();
    let (ana, bob) = realistic(&fx);
    fx.app.lock();
    let dbs: Vec<PathBuf> = [&ana, &bob]
        .iter()
        .map(|id| fx.root().join("vaults").join(id).join("vault.db"))
        .collect();
    let mut sealed = Vec::new();
    for db in &dbs {
        let pieces = sealed_pieces(db);
        assert!(
            holds_any(&std::fs::read(db).unwrap(), &pieces),
            "the key material is in the file before the wipe"
        );
        sealed.extend(pieces);
    }
    let mut rec = Recorder {
        steps: Vec::new(),
        sealed,
        leaks: Vec::new(),
        stop_after: usize::MAX,
    };
    let report = fx.app.wipe_with(&mut rec).unwrap();
    assert!(
        rec.leaks.is_empty(),
        "key bundle still readable: {:?}",
        rec.leaks
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.unknown, vec![PathBuf::from("notes.txt")]);
    // the first steps are the two cryptographic erases, in front of every other step
    assert!(rec
        .steps
        .iter()
        .take(2)
        .all(|s| matches!(s, WipeStep::KeyBundleDestroyed(_))));
    let last_erase = rec
        .steps
        .iter()
        .rposition(|s| matches!(s, WipeStep::KeyBundleDestroyed(_)))
        .unwrap();
    let first_unlink = rec
        .steps
        .iter()
        .position(|s| matches!(s, WipeStep::Unlinked(_)))
        .unwrap();
    assert!(last_erase < first_unlink);
}

#[test]
fn a_wipe_cut_off_after_the_key_bundles_leaves_vaults_that_cannot_be_opened() {
    let fx = Fx::new();
    let (ana, bob) = realistic(&fx);
    fx.app.lock();
    let db = fx.root().join("vaults").join(&ana).join("vault.db");
    let mut sealed = sealed_pieces(&db);
    sealed.extend(sealed_pieces(
        &fx.root().join("vaults").join(&bob).join("vault.db"),
    ));
    // a power cut right after the second key bundle was destroyed
    let mut rec = Recorder {
        steps: Vec::new(),
        sealed,
        leaks: Vec::new(),
        stop_after: 2,
    };
    let report = fx.app.wipe_with(&mut rec).unwrap();
    assert!(report.aborted);
    assert!(rec.leaks.is_empty());
    assert!(db.exists(), "nothing was unlinked yet");
    // the files are still there, and no password opens the vault any more
    let restarted = open_app(fx.root());
    for (name, pw) in [("Ana", PW), ("Bob", PW2)] {
        let id = restarted
            .profile_list()
            .unwrap()
            .profiles
            .into_iter()
            .find(|p| p.display_name == name)
            .map(|p| p.id)
            .expect("the account is still listed");
        restarted.profile_select(&id).unwrap();
        assert!(
            restarted.unlock(pw.into()).is_err(),
            "{name}'s vault is unreadable"
        );
        assert!(restarted.unlock("anything else at all".into()).is_err());
    }
    // finishing the job later removes the rest
    erase(&restarted, 3).unwrap();
    assert_eq!(fx.tree(), vec!["notes.txt".to_string()]);
}

#[cfg(unix)]
#[test]
fn a_refused_wipe_touches_nothing_locks_the_vault_and_leaves_the_gate_closed() {
    let fx = Fx::new();
    fx.account("Ana", PW);
    fx.app
        .gate_configure(pin_with_panic_code("2468", PW, "5151"))
        .unwrap();
    fx.app.gate_close();
    // a known Hexis name is a link out of the folder: the pre-flight refuses the whole wipe
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("precious.txt");
    std::fs::write(&target, b"keep").unwrap();
    std::os::unix::fs::symlink(&target, fx.root().join("disguise.json")).unwrap();
    let before = fx.tree();

    let e = fx
        .app
        .gate_submit(Zeroizing::new("5151".into()))
        .expect_err("the wipe is refused");
    assert_eq!(e.code(), "storage");
    assert_eq!(e.to_string(), "This device could not be erased.");
    assert_eq!(fx.tree(), before, "nothing was touched");
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    assert!(
        fx.app.gate().is_closed(),
        "a refused wipe never opens the gate"
    );
    assert_eq!(code(fx.app.before_command("profile_list")), "locked");
    assert_eq!(code(erase(&fx.app, 3)), "storage");
    assert_eq!(fx.tree(), before);

    // once the link is gone the same request works
    std::fs::remove_file(fx.root().join("disguise.json")).unwrap();
    erase(&fx.app, 3).unwrap();
    assert!(fx.tree().is_empty());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
}

#[test]
fn the_wipe_is_idempotent_on_an_empty_folder_and_never_removes_the_folder_or_its_neighbours() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("data");
    let app = open_app(&data);
    erase(&app, 3).unwrap();
    erase(&app, 3).unwrap();
    assert!(data.is_dir(), "the data folder itself is not removed");
    std::fs::write(parent.path().join("precious.txt"), b"keep").unwrap();
    erase(&app, 3).unwrap();
    assert_eq!(
        std::fs::read(parent.path().join("precious.txt")).unwrap(),
        b"keep"
    );
}

// ------------------------------------------------------------------ remove Hexis from this device (before an uninstall)

/// `app_prepare_uninstall` the way the screen does it: ask for the sum, answer it.
fn prepare_uninstall(app: &AppCore, steps: u64) -> Result<(), StorageError> {
    app.app_reset_challenge();
    let answer = app
        .gate()
        .reset_answer()
        .expect("a sum is waiting")
        .to_string();
    app.app_prepare_uninstall(steps, &answer)
}

/// What an older build's web view left in the data folder.
fn leave_web_view_data(root: &Path) {
    for (path, body) in [
        ("localstorage/http_localhost_1420.localstorage", "prefs"),
        ("WebKitCache/Version 17/Records/aa/bb", "cached page"),
        ("WebKitCache/Version 17/salt", "salt"),
        ("CacheStorage/salt", "salt"),
        ("mediakeys/v1/salt", "salt"),
        ("hsts-storage.sqlite", "hsts"),
    ] {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
}

#[test]
fn an_erase_also_removes_the_web_views_leftovers_but_keeps_the_device_settings() {
    let fx = Fx::new();
    fx.account("Ana", PW);
    fx.app.device_set_time_zone("Asia/Dhaka").unwrap();
    fx.app
        .device_set_theme(crate::storage::device::Theme::Dark)
        .unwrap();
    leave_web_view_data(fx.root());
    erase(&fx.app, 3).unwrap();
    let left = fx.tree();
    assert_eq!(
        left,
        vec!["device.json".to_string()],
        "no vault, no cache, no local storage; only the settings an erase keeps on purpose"
    );
    let d = fx.app.device_get();
    assert_eq!(d.time_zone.as_deref(), Some("Asia/Dhaka"));
    assert_eq!(d.theme, crate::storage::device::Theme::Dark);
}

#[test]
fn removing_hexis_from_the_device_leaves_nothing_of_ours_behind() {
    let fx = Fx::new();
    let (ana, _bob) = realistic(&fx);
    fx.app.device_set_time_zone("Asia/Dhaka").unwrap();
    fx.app.device_finish_setup().unwrap();
    fx.app.device_set_clipboard_clear(60).unwrap();
    leave_web_view_data(fx.root());
    assert!(fx.tree().contains(&"device.json".to_string()));
    assert!(fx.tree().contains(&format!("vaults/{ana}/vault.db")));

    // wrong confirmations and a wrong sum remove nothing
    for steps in [0, 1, 2, 4, 99] {
        assert_eq!(
            code(prepare_uninstall(&fx.app, steps)),
            "invalid_input",
            "{steps}"
        );
    }
    fx.app.app_reset_challenge();
    assert_eq!(code(fx.app.app_prepare_uninstall(3, "1")), "wrong_answer");
    assert!(fx.tree().contains(&"device.json".to_string()));
    assert!(fx.tree().contains(&format!("vaults/{ana}/vault.db")));

    prepare_uninstall(&fx.app, 3).unwrap();
    assert_eq!(
        fx.tree(),
        vec!["notes.txt".to_string()],
        "vaults, settings and web view data are gone; a file that is not ours stays"
    );
    assert!(!contains(&fx.all_bytes(), MARKER));
    // the settings in memory are the defaults, and asking for them writes nothing
    let d = fx.app.device_get();
    assert_eq!(
        (d.time_zone, d.setup_done, d.clipboard_clear),
        (None, false, 30)
    );
    let _ = fx.app.profile_list();
    let _ = fx.app.status();
    assert_eq!(
        fx.tree(),
        vec!["notes.txt".to_string()],
        "nothing was made afterwards"
    );
    // unlike a reset it makes no demo vault and no default files
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());
}

#[test]
fn the_data_folder_itself_goes_when_nothing_else_is_in_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("dev.habibullah.hexis");
    std::fs::create_dir_all(&root).unwrap();
    let app = open_app(&root);
    app.profile_create("Ana", PW.into()).unwrap();
    leave_web_view_data(&root);
    prepare_uninstall(&app, 3).unwrap();
    assert!(!root.exists(), "the folder is removed, not just emptied");
    assert!(tmp.path().read_dir().unwrap().next().is_none());
}

#[test]
fn removing_hexis_is_closed_while_the_app_lock_is_closed() {
    let fx = Fx::new();
    fx.account("Ana", PW);
    fx.app.gate_configure(pin("2468", PW)).unwrap();
    fx.app.gate_close();
    assert_eq!(
        code(fx.app.before_command("app_prepare_uninstall")),
        "locked",
        "a closed lock screen cannot remove the app's data (a reset can: it is for a forgotten passcode)"
    );
    assert!(fx.app.before_command("app_reset").is_ok());
}
