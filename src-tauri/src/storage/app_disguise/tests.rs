//! Behaviour of the disguise on [`AppCore`]: round trip, persistence, validation, save-before-apply,
//! a failing launcher, startup re-apply, the wipe, and that it never touches vault state. Temp
//! folders only.

use super::*;
use crate::crypto::KdfParams;
use crate::disguise::{self, Launcher, PresetAssets, RecordingLauncher, FILE_NAME};
use crate::storage::files::TestPicker;
use crate::storage::session::{SessionConfig, EDIT_TOKEN_TTL};
use crate::storage::StorageError;
use std::path::Path;

const PW: &str = "correct horse battery staple";

fn fast() -> SessionConfig {
    SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: EDIT_TOKEN_TTL,
    }
}

fn input(preset: Option<&str>, name: Option<&str>, lowercase: bool) -> DisguiseInput {
    DisguiseInput {
        preset_id: preset.map(str::to_string),
        custom_name: name.map(str::to_string),
        lowercase,
    }
}

fn core(dir: &Path, launcher: Arc<dyn Launcher>) -> AppCore {
    AppCore::open(dir, fast(), Arc::new(TestPicker::default()))
        .unwrap()
        .with_launcher(launcher)
}

struct Fx {
    dir: tempfile::TempDir,
    rec: Arc<RecordingLauncher>,
    app: AppCore,
}

impl Fx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let rec = Arc::new(RecordingLauncher::new());
        let app = core(dir.path(), rec.clone());
        Self { dir, rec, app }
    }

    fn file(&self) -> std::path::PathBuf {
        self.dir.path().join(FILE_NAME)
    }
}

#[test]
fn before_anything_was_chosen_it_is_the_original_look() {
    let fx = Fx::new();
    assert_eq!(fx.app.disguise_get().unwrap(), DisguiseView::original());
    assert!(!fx.file().exists(), "reading writes nothing");
    assert_eq!(fx.rec.count(), 0, "reading applies nothing");
}

#[test]
fn round_trip() {
    let fx = Fx::new();
    let set = fx
        .app
        .disguise_set(input(Some("kerf"), None, true))
        .unwrap();
    assert_eq!(set.preset_id.as_deref(), Some("kerf"));
    assert_eq!(set.custom_name, None);
    assert!(set.lowercase);
    assert_eq!(set.applied_name, "kerf");
    assert_eq!(set.note, None);
    assert_eq!(fx.app.disguise_get().unwrap(), set);

    let again = fx
        .app
        .disguise_set(input(Some("orvyn"), Some("  Ledger "), false))
        .unwrap();
    assert_eq!(again.custom_name.as_deref(), Some("Ledger"), "trimmed");
    assert_eq!(again.applied_name, "Ledger");
    assert_eq!(fx.app.disguise_get().unwrap(), again);

    // back to the original look
    let orig = fx.app.disguise_set(input(None, None, false)).unwrap();
    assert_eq!(orig, DisguiseView::original());
    assert_eq!(fx.app.disguise_get().unwrap(), DisguiseView::original());
}

#[test]
fn the_choice_survives_a_restart() {
    let mut fx = Fx::new();
    fx.app
        .disguise_set(input(Some("cairn"), Some("Notes"), true))
        .unwrap();
    // simulate quitting and starting the app again
    let rec2 = Arc::new(RecordingLauncher::new());
    fx.app = core(fx.dir.path(), rec2.clone());
    let v = fx.app.disguise_get().unwrap();
    assert_eq!(v.preset_id.as_deref(), Some("cairn"));
    assert_eq!(v.custom_name.as_deref(), Some("Notes"));
    assert!(v.lowercase);
    assert_eq!(v.applied_name, "notes");
    assert_eq!(rec2.count(), 0, "get never drives the launcher");
}

#[test]
fn a_damaged_file_means_the_defaults_and_is_never_an_error() {
    let fx = Fx::new();
    for junk in [
        &b""[..],
        b"\xff\xfe\x00",
        b"{\"version\":1,\"presetId\":\"evil\"}",
        b"[]",
    ] {
        std::fs::write(fx.file(), junk).unwrap();
        assert_eq!(fx.app.disguise_get().unwrap(), DisguiseView::original());
        // and a fresh start with that file in place still opens the app
        let app = AppCore::open(fx.dir.path(), fast(), Arc::new(TestPicker::default()));
        assert!(app.is_ok());
    }
    // a good choice can be made over the damage
    fx.app
        .disguise_set(input(Some("tally"), None, false))
        .unwrap();
    assert_eq!(
        fx.app.disguise_get().unwrap().preset_id.as_deref(),
        Some("tally")
    );
}

#[test]
fn a_bad_choice_is_refused_before_anything_is_saved_or_applied() {
    let fx = Fx::new();
    let bad: Vec<(DisguiseInput, &str)> = vec![
        (
            input(Some("unknown"), None, false),
            "That icon is not one of the choices.",
        ),
        (
            input(Some(""), None, false),
            "That icon is not one of the choices.",
        ),
        (
            input(Some("Kerf"), None, false),
            "That icon is not one of the choices.",
        ),
        (
            input(None, Some(&"a".repeat(31)), false),
            "Use 1 to 30 characters for the name.",
        ),
        (
            input(None, Some("a\u{200B}Exec=evil"), false),
            "The name contains characters that cannot be shown.",
        ),
    ];
    for (choice, message) in &bad {
        let e = fx.app.disguise_set(choice.clone()).expect_err("refused");
        assert_eq!(e.code(), "invalid_input");
        assert_eq!(e.to_string(), *message);
        assert!(!fx.file().exists(), "nothing saved for {choice:?}");
        assert_eq!(fx.rec.count(), 0, "nothing applied for {choice:?}");
    }
    // after a good choice, a bad one leaves it exactly as it was
    fx.app
        .disguise_set(input(Some("kerf"), None, false))
        .unwrap();
    let saved = std::fs::read(fx.file()).unwrap();
    let applied = fx.rec.count();
    for (choice, _) in &bad {
        assert!(fx.app.disguise_set(choice.clone()).is_err());
    }
    assert_eq!(std::fs::read(fx.file()).unwrap(), saved);
    assert_eq!(fx.rec.count(), applied);
    assert_eq!(
        fx.app.disguise_get().unwrap().preset_id.as_deref(),
        Some("kerf")
    );
}

#[test]
fn the_launcher_gets_the_resolved_view_and_the_preset_assets() {
    let fx = Fx::new();
    fx.app
        .disguise_set(input(Some("kovren"), None, true))
        .unwrap();
    let last = fx.rec.last().unwrap();
    assert_eq!(last.view.applied_name, "kovren");
    assert_eq!(last.view.note, None);
    assert_eq!(last.preset_id.as_deref(), Some("kovren"));

    fx.app
        .disguise_set(input(None, Some("Ledger"), false))
        .unwrap();
    let last = fx.rec.last().unwrap();
    assert_eq!(last.view.applied_name, "Ledger");
    assert_eq!(last.preset_id, None, "no preset: the original icon");

    fx.app.disguise_set(input(None, None, false)).unwrap();
    assert_eq!(fx.rec.last().unwrap().view, DisguiseView::original());
    assert_eq!(fx.rec.count(), 3);
}

/// Looks at the saved file at the moment it is asked to apply.
struct ProbeLauncher {
    dir: std::path::PathBuf,
    saw: std::sync::Mutex<Vec<DisguiseInput>>,
}

impl Launcher for ProbeLauncher {
    fn apply(
        &self,
        _view: &DisguiseView,
        _preset: Option<&PresetAssets>,
    ) -> crate::storage::Result<Option<String>> {
        self.saw.lock().unwrap().push(disguise::load(&self.dir));
        Ok(None)
    }
}

#[test]
fn the_choice_is_saved_before_the_launcher_runs() {
    let dir = tempfile::tempdir().unwrap();
    let probe = Arc::new(ProbeLauncher {
        dir: dir.path().to_path_buf(),
        saw: Default::default(),
    });
    let app = core(dir.path(), probe.clone());
    app.disguise_set(input(Some("basalt"), Some("Quiet"), true))
        .unwrap();
    app.disguise_set(input(Some("trim"), None, false)).unwrap();
    let saw = probe.saw.lock().unwrap().clone();
    assert_eq!(
        saw,
        [
            input(Some("basalt"), Some("Quiet"), true),
            input(Some("trim"), None, false)
        ]
    );
}

#[test]
fn a_failing_launcher_is_a_note_and_the_choice_stays_saved() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Arc::new(RecordingLauncher::new().failing("The launcher is read-only."));
    let app = core(dir.path(), rec.clone());
    let v = app
        .disguise_set(input(Some("plinth"), None, false))
        .expect("not an error");
    assert_eq!(v.preset_id.as_deref(), Some("plinth"));
    assert_eq!(v.applied_name, "Plinth");
    assert_eq!(v.note.as_deref(), Some("The launcher is read-only."));
    assert_eq!(
        disguise::load(dir.path()),
        input(Some("plinth"), None, false)
    );
    // Settings opened again in this run sees the same note
    assert_eq!(
        app.disguise_get().unwrap().note.as_deref(),
        Some("The launcher is read-only.")
    );
    // and after a restart the choice is still there
    let app2 = core(dir.path(), Arc::new(RecordingLauncher::new()));
    assert_eq!(
        app2.disguise_get().unwrap().preset_id.as_deref(),
        Some("plinth")
    );
    assert_eq!(app2.disguise_get().unwrap().note, None);
}

#[test]
fn notes_and_the_standing_note() {
    let dir = tempfile::tempdir().unwrap();
    let app = core(
        dir.path(),
        Arc::new(RecordingLauncher::new().with_note("Done.")),
    );
    assert_eq!(app.disguise_get().unwrap().note, None);
    assert_eq!(
        app.disguise_set(input(Some("vellum"), None, false))
            .unwrap()
            .note
            .as_deref(),
        Some("Done.")
    );
    assert_eq!(app.disguise_get().unwrap().note.as_deref(), Some("Done."));

    struct Standing;
    impl Launcher for Standing {
        fn apply(
            &self,
            _v: &DisguiseView,
            _p: Option<&PresetAssets>,
        ) -> crate::storage::Result<Option<String>> {
            Ok(None)
        }
        fn standing_note(&self) -> Option<String> {
            Some("Always true here.".into())
        }
    }
    let app = core(dir.path(), Arc::new(Standing));
    assert_eq!(
        app.disguise_get().unwrap().note.as_deref(),
        Some("Always true here.")
    );
    assert_eq!(
        app.disguise_set(input(None, Some("X"), false))
            .unwrap()
            .note,
        None
    );
    assert_eq!(
        app.disguise_get().unwrap().note.as_deref(),
        Some("Always true here.")
    );
}

#[test]
fn a_launcher_can_be_installed_after_the_core_was_built() {
    let dir = tempfile::tempdir().unwrap();
    let app = AppCore::open(dir.path(), fast(), Arc::new(TestPicker::default())).unwrap();
    // the default launcher does nothing and says nothing
    assert_eq!(
        app.disguise_set(input(Some("tally"), None, false))
            .unwrap()
            .note,
        None
    );
    let rec = Arc::new(RecordingLauncher::new());
    app.set_launcher(rec.clone());
    app.disguise_set(input(Some("kerf"), None, false)).unwrap();
    assert_eq!(rec.count(), 1);
}

#[test]
fn startup_applies_the_saved_choice_again() {
    let dir = tempfile::tempdir().unwrap();
    core(dir.path(), Arc::new(RecordingLauncher::new()))
        .disguise_set(input(Some("drizzle"), Some("Weather"), true))
        .unwrap();

    let rec = Arc::new(RecordingLauncher::new());
    let app = core(dir.path(), rec.clone());
    let v = app.disguise_apply_saved();
    assert_eq!(v.applied_name, "weather");
    let last = rec.last().unwrap();
    assert_eq!(last.view.applied_name, "weather");
    assert_eq!(last.preset_id.as_deref(), Some("drizzle"));

    // nothing saved: the original look is applied (this is what resets a stale launcher entry)
    let dir2 = tempfile::tempdir().unwrap();
    let rec2 = Arc::new(RecordingLauncher::new());
    let v = core(dir2.path(), rec2.clone()).disguise_apply_saved();
    assert_eq!(v, DisguiseView::original());
    assert_eq!(rec2.last().unwrap().view, DisguiseView::original());
    assert_eq!(rec2.last().unwrap().preset_id, None);
    assert!(
        !dir2.path().join(FILE_NAME).exists(),
        "applying writes no file"
    );

    // damaged file: the original look, no error
    std::fs::write(dir2.path().join(FILE_NAME), b"\x00garbage").unwrap();
    assert_eq!(
        core(dir2.path(), rec2.clone()).disguise_apply_saved(),
        DisguiseView::original()
    );
}

#[test]
fn startup_apply_survives_a_failing_launcher() {
    let dir = tempfile::tempdir().unwrap();
    let app = core(
        dir.path(),
        Arc::new(RecordingLauncher::new().failing("no window")),
    );
    app.disguise_set(input(Some("tivlo"), None, false)).unwrap();
    let v = app.disguise_apply_saved();
    assert_eq!(v.preset_id.as_deref(), Some("tivlo"));
    assert_eq!(v.note.as_deref(), Some("no window"));
}

#[test]
fn the_wipe_forgets_the_choice_everywhere() {
    let fx = Fx::new();
    fx.app
        .disguise_set(input(Some("kerf"), Some("Ledger"), true))
        .unwrap();
    assert!(fx.file().exists());
    fx.app.disguise_wipe().unwrap();
    assert!(!fx.file().exists());
    assert_eq!(fx.app.disguise_get().unwrap(), DisguiseView::original());
    let last = fx.rec.last().unwrap();
    assert_eq!(
        last.view,
        DisguiseView::original(),
        "the launcher was reset to the original"
    );
    assert_eq!(last.preset_id, None);
}

/// Every file under `dir` except the disguise file: path and content.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else if p.file_name().is_some_and(|n| n != FILE_NAME) {
                let rel = p.strip_prefix(root).unwrap().display().to_string();
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn it_works_with_no_vault_and_never_touches_vault_state() {
    // no account at all
    let fx = Fx::new();
    assert!(!fx.app.status().unwrap().exists);
    fx.app
        .disguise_set(input(Some("kerf"), None, false))
        .unwrap();
    assert!(
        !fx.app.status().unwrap().exists,
        "setting a look creates no vault"
    );
    assert!(
        snapshot(fx.dir.path()).is_empty(),
        "only disguise.json exists"
    );

    // an account, unlocked
    fx.app.profile_create("Ana", PW.to_string()).unwrap();
    assert!(fx.app.status().unwrap().unlocked);
    let before = snapshot(fx.dir.path());
    assert!(!before.is_empty());
    let profiles_before = fx.app.profile_list().unwrap();
    fx.app
        .disguise_set(input(Some("orvyn"), Some("Ledger"), true))
        .unwrap();
    fx.app.disguise_set(input(None, None, false)).unwrap();
    let _ = fx.app.disguise_get().unwrap();
    assert!(fx.app.status().unwrap().unlocked, "still unlocked");
    assert_eq!(fx.app.profile_list().unwrap(), profiles_before);
    assert_eq!(
        snapshot(fx.dir.path()),
        before,
        "no vault or registry byte changed"
    );

    // locked
    fx.app.lock();
    assert!(!fx.app.status().unwrap().unlocked);
    // (locking itself may checkpoint the database, so the baseline is taken after it)
    let before = snapshot(fx.dir.path());
    fx.app
        .disguise_set(input(Some("tally"), None, false))
        .unwrap();
    assert!(
        !fx.app.status().unwrap().unlocked,
        "setting a look does not unlock"
    );
    assert_eq!(snapshot(fx.dir.path()), before);
    assert_eq!(
        fx.app.disguise_get().unwrap().preset_id.as_deref(),
        Some("tally")
    );
}

#[test]
fn a_data_folder_that_cannot_be_written_is_a_clear_error_and_nothing_is_applied() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Arc::new(RecordingLauncher::new());
    let app = core(dir.path(), rec.clone());
    // a folder where the file must go makes the save fail
    std::fs::create_dir(dir.path().join(FILE_NAME)).unwrap();
    let e = app
        .disguise_set(input(Some("kerf"), None, false))
        .expect_err("cannot save");
    assert!(matches!(e, StorageError::Storage(_)));
    assert!(
        !e.to_string().contains(&dir.path().display().to_string()),
        "no path in the message"
    );
    assert_eq!(rec.count(), 0, "not applied when it could not be saved");
}

#[test]
fn quick_changes_from_several_threads_end_in_one_consistent_state() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Arc::new(RecordingLauncher::new());
    let app = Arc::new(core(dir.path(), rec.clone()));
    let ids: Vec<String> = disguise::presets()
        .iter()
        .map(|p| p.id().to_string())
        .collect();
    let handles: Vec<_> = ids
        .iter()
        .cloned()
        .map(|id| {
            let app = Arc::clone(&app);
            std::thread::spawn(move || {
                for _ in 0..5 {
                    app.disguise_set(input(Some(&id), None, false)).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(rec.count(), ids.len() * 5);
    // the saved choice is the one the launcher applied last, never a mix of the two orders
    let saved = disguise::load(dir.path());
    assert_eq!(rec.last().unwrap().view.preset_id, saved.preset_id);
    assert_eq!(app.disguise_get().unwrap().preset_id, saved.preset_id);
}
