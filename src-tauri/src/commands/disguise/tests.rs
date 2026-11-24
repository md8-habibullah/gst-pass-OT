//! The disguise commands over Tauri's mock runtime (real IPC dispatch, the exact JSON the
//! frontend sends), the production launcher against a temp folder, and the rule that the webview
//! gets no window permission. The user's real launcher folders are never touched: the launcher is
//! always built with an injected data home.

use super::*;
use crate::crypto::KdfParams;
use crate::disguise::RecordingLauncher;
use crate::storage::files::TestPicker;
use crate::storage::{AppCore, SessionConfig};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

struct Harness {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    window: WebviewWindow<MockRuntime>,
}

fn harness(with_window: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let cfg = SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: Duration::from_secs(600),
    };
    let core = AppCore::open(dir.path(), cfg, Arc::new(TestPicker::default())).unwrap();
    let app = mock_builder()
        .manage(AppState::new(core))
        .invoke_handler(super::super::invoke_handler())
        .build(mock_context(noop_assets()))
        .unwrap();
    // IPC needs a window to dispatch from; the launcher looks for the one labelled `main`
    let label = if with_window { "main" } else { "other" };
    let window = WebviewWindowBuilder::new(&app, label, Default::default())
        .build()
        .unwrap();
    Harness { dir, app, window }
}

impl Harness {
    fn core(&self) -> Arc<AppCore> {
        Arc::clone(&self.app.state::<AppState>().core)
    }

    fn call(&self, cmd: &str, args: Value) -> std::result::Result<Value, Value> {
        get_ipc_response(
            &self.window,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().unwrap())
    }

    fn ok(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args)
            .unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
    }

    fn err(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args).expect_err("expected an error")
    }
}

fn set_args(preset: Value, name: Value, lowercase: bool) -> Value {
    json!({"input": {"presetId": preset, "customName": name, "lowercase": lowercase}})
}

// ------------------------------------------------------------------ over real IPC

#[test]
fn get_and_set_speak_exactly_the_frontends_json() {
    let h = harness(true);
    let rec = Arc::new(RecordingLauncher::new());
    h.core().set_launcher(rec.clone());

    assert_eq!(
        h.ok("disguise_get", json!({})),
        json!({"presetId": null, "customName": null, "lowercase": false, "appliedName": "Passman", "note": null})
    );
    assert_eq!(rec.count(), 0, "reading drives nothing");

    assert_eq!(
        h.ok("disguise_set", set_args(json!("kerf"), json!(null), true)),
        json!({"presetId": "kerf", "customName": null, "lowercase": true, "appliedName": "kerf", "note": null})
    );
    assert_eq!(
        h.ok("disguise_get", json!({})),
        json!({"presetId": "kerf", "customName": null, "lowercase": true, "appliedName": "kerf", "note": null})
    );
    assert_eq!(
        h.ok(
            "disguise_set",
            set_args(json!("tally"), json!("  Ledger  "), false)
        ),
        json!({"presetId": "tally", "customName": "Ledger", "lowercase": false, "appliedName": "Ledger", "note": null})
    );
    assert_eq!(
        h.ok("disguise_set", set_args(json!(null), json!(null), false)),
        json!({"presetId": null, "customName": null, "lowercase": false, "appliedName": "Passman", "note": null})
    );
    assert_eq!(rec.count(), 3);
    assert_eq!(rec.last().unwrap().preset_id, None);
}

#[test]
fn a_note_from_the_launcher_comes_back_as_note() {
    let h = harness(true);
    h.core().set_launcher(Arc::new(
        RecordingLauncher::new().with_note("Preview only."),
    ));
    let v = h.ok("disguise_set", set_args(json!("cairn"), json!(null), false));
    assert_eq!(v["note"], "Preview only.");
    assert_eq!(h.ok("disguise_get", json!({}))["note"], "Preview only.");
    // a failing launcher is also only a note: the call succeeds and the choice is kept
    h.core().set_launcher(Arc::new(
        RecordingLauncher::new().failing("It did not work."),
    ));
    let v = h.ok("disguise_set", set_args(json!("orvyn"), json!(null), true));
    assert_eq!(v["note"], "It did not work.");
    assert_eq!(v["appliedName"], "orvyn");
    assert_eq!(h.ok("disguise_get", json!({}))["presetId"], "orvyn");
}

#[test]
fn bad_choices_come_back_as_invalid_input_with_fixed_sentences() {
    let h = harness(true);
    let long = "x".repeat(31);
    for (args, message) in [
        (
            set_args(json!("unknown"), json!(null), false),
            "That icon is not one of the choices.",
        ),
        (
            set_args(json!("Kerf"), json!(null), false),
            "That icon is not one of the choices.",
        ),
        (
            set_args(json!(""), json!(null), false),
            "That icon is not one of the choices.",
        ),
        (
            set_args(json!(null), json!(long), false),
            "Use 1 to 30 characters for the name.",
        ),
        (
            set_args(json!(null), json!("a\u{200B}Exec=evil"), false),
            "The name contains characters that cannot be shown.",
        ),
        (
            set_args(json!(null), json!("\u{202E}b"), false),
            "The name contains characters that cannot be shown.",
        ),
    ] {
        let e = h.err("disguise_set", args.clone());
        assert_eq!(e["code"], "invalid_input", "{args}");
        assert_eq!(e["message"], message, "{args}");
    }
    // nothing was saved
    assert!(!h.dir.path().join(crate::disguise::FILE_NAME).exists());
    assert_eq!(h.ok("disguise_get", json!({}))["appliedName"], "Passman");
    // 30 characters are fine
    let thirty = "x".repeat(30);
    assert_eq!(
        h.ok("disguise_set", set_args(json!(null), json!(thirty), false))["customName"],
        json!(thirty)
    );
}

#[test]
fn malformed_arguments_are_refused_by_the_ipc_layer() {
    let h = harness(true);
    for args in [
        json!({}),
        json!({"input": null}),
        json!({"input": {"presetId": 5, "customName": null, "lowercase": false}}),
        json!({"input": {"presetId": null, "customName": 7, "lowercase": false}}),
        json!({"input": {"presetId": null, "customName": null, "lowercase": "yes"}}),
        json!({"presetId": "kerf", "customName": null, "lowercase": false}),
    ] {
        assert!(h.call("disguise_set", args.clone()).is_err(), "{args}");
    }
    assert!(!h.dir.path().join(crate::disguise::FILE_NAME).exists());
}

#[test]
fn the_commands_need_no_account_and_leave_vault_state_alone() {
    let h = harness(true);
    // no vault at all: both work
    h.ok(
        "disguise_set",
        set_args(json!("basalt"), json!(null), false),
    );
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["exists"], false);
    assert_eq!(st["unlocked"], false);
    // item commands are still locked
    assert_eq!(h.err("item_list", json!({}))["code"], "locked");
    h.ok("disguise_get", json!({}));
    assert_eq!(h.err("item_list", json!({}))["code"], "locked");
}

// ------------------------------------------------------------------ the webview's permissions

#[test]
fn the_webview_gets_no_window_permission() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/capabilities/default.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        v["permissions"],
        json!(["core:default"]),
        "the window is changed from Rust only"
    );
    assert_eq!(v["windows"], json!(["main"]));
    // `core:default` itself holds only read-only window permissions (no set-title, set-icon)
    let text = std::fs::read_to_string(path).unwrap();
    assert!(
        !text.contains("set-title") && !text.contains("set-icon") && !text.contains("window:allow")
    );
}

// ------------------------------------------------------------------ plans and names

#[test]
fn the_window_follows_the_applied_name_and_the_preset_icon() {
    let kerf = crate::disguise::DisguiseInput {
        preset_id: Some("kerf".into()),
        custom_name: None,
        lowercase: true,
    };
    let plan = window_plan(&kerf.view(None), crate::disguise::preset("kerf"));
    assert_eq!(
        plan,
        WindowPlan {
            title: "kerf".into(),
            icon_preset: Some("kerf".into())
        }
    );
    let plan = window_plan(&DisguiseView::original(), None);
    assert_eq!(
        plan,
        WindowPlan {
            title: "Passman".into(),
            icon_preset: None
        }
    );
}

#[test]
fn android_alias_names_follow_the_patch_script() {
    // the same table is asserted by scripts/tests/test_android_patch.py
    assert_eq!(android_alias_name(None), "AliasOriginal");
    assert_eq!(android_alias_name(Some("kerf")), "AliasKerf");
    assert_eq!(android_alias_name(Some("tally")), "AliasTally");
    assert_eq!(android_alias_name(Some("orvyn")), "AliasOrvyn");
    assert_eq!(android_alias_name(Some("")), "Alias");
}

// ------------------------------------------------------------------ the production launcher

#[test]
fn installing_without_state_does_nothing() {
    let app = mock_builder().build(mock_context(noop_assets())).unwrap();
    install(app.handle()); // must not panic; nothing is managed
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::disguise::desktop::{exec_value, MARKER_LINE};

    /// The mock runtime's configuration has an empty identifier, so the launcher is given one.
    const ID: &str = "app.passman.test";

    fn entry_of(home: &std::path::Path) -> std::path::PathBuf {
        home.join("applications").join(format!("{ID}.desktop"))
    }

    fn install_in(h: &Harness, home: &std::path::Path) {
        h.core().set_launcher(Arc::new(TauriLauncher::with_sandbox(
            h.app.handle().clone(),
            home.to_path_buf(),
            ID,
        )));
    }

    #[test]
    fn choosing_a_look_writes_the_launcher_entry_and_going_back_removes_it() {
        let h = harness(true);
        let home = tempfile::tempdir().unwrap();
        install_in(&h, home.path());
        let entry = entry_of(home.path());

        let v = h.ok("disguise_set", set_args(json!("kerf"), json!(null), true));
        assert_eq!(v["note"], NOTE_LINUX);
        let text = std::fs::read_to_string(&entry).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.contains(&"Name=kerf"), "{text}");
        assert!(lines.contains(&"Comment=Cursor and dot"), "{text}");
        assert!(lines.contains(&MARKER_LINE), "{text}");
        assert!(lines.contains(&"Terminal=false"), "{text}");
        assert!(lines.contains(&"Type=Application"), "{text}");
        assert_eq!(lines.iter().filter(|l| l.starts_with("Exec=")).count(), 1);
        let exe = std::env::current_exe().unwrap();
        let exec_line = format!("Exec={}", exec_value(exe.to_str().unwrap()));
        assert!(lines.contains(&exec_line.as_str()), "{text}");
        let class = exe.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            lines.contains(&format!("StartupWMClass={class}").as_str()),
            "{text}"
        );
        let icon_line = lines.iter().find(|l| l.starts_with("Icon=")).unwrap();
        let icon = std::path::Path::new(&icon_line["Icon=".len()..]);
        assert!(icon.is_absolute() && icon.starts_with(home.path()));
        assert_eq!(
            std::fs::read(icon).unwrap(),
            crate::disguise::preset("kerf").unwrap().png_512
        );

        // a custom name with a hostile spelling stays on one line
        h.ok(
            "disguise_set",
            set_args(json!("kerf"), json!("a%u\\nExec=evil"), false),
        );
        let text = std::fs::read_to_string(&entry).unwrap();
        assert_eq!(
            text.lines().filter(|l| l.starts_with("Exec=")).count(),
            1,
            "{text}"
        );
        assert!(text.contains("\nName=a%u\\\\nExec=evil\n"), "{text}");

        // ... and a real line break in the name becomes a space: still one Exec line
        let v = h.ok(
            "disguise_set",
            set_args(json!("kerf"), json!("a%u\nExec=evil"), false),
        );
        assert_eq!(v["customName"], "a%u Exec=evil");
        assert_eq!(v["appliedName"], "a%u Exec=evil");
        let text = std::fs::read_to_string(&entry).unwrap();
        assert_eq!(
            text.lines().filter(|l| l.starts_with("Exec=")).count(),
            1,
            "{text}"
        );
        assert!(text.contains("\nName=a%u Exec=evil\n"), "{text}");

        // back to the original look: nothing of ours is left
        let v = h.ok("disguise_set", set_args(json!(null), json!(null), false));
        assert_eq!(v["note"], NOTE_LINUX_RESET);
        assert!(!entry.exists());
        assert!(!home.path().join(ID).join("disguise").exists());
    }

    #[test]
    fn the_original_look_with_nothing_to_remove_says_nothing() {
        let h = harness(true);
        let home = tempfile::tempdir().unwrap();
        let launcher =
            TauriLauncher::with_sandbox(h.app.handle().clone(), home.path().to_path_buf(), ID);
        assert_eq!(
            launcher.apply(&DisguiseView::original(), None).unwrap(),
            None
        );
        assert_eq!(
            std::fs::read_dir(home.path()).unwrap().count(),
            0,
            "nothing was created"
        );
    }

    #[test]
    fn a_foreign_entry_in_the_way_is_a_note_and_the_choice_is_kept() {
        let h = harness(true);
        let home = tempfile::tempdir().unwrap();
        install_in(&h, home.path());
        let entry = entry_of(home.path());
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, "[Desktop Entry]\nName=Mine\nExec=mine\n").unwrap();

        let v = h.ok("disguise_set", set_args(json!("tally"), json!(null), false));
        assert_eq!(v["presetId"], "tally");
        assert!(
            v["note"].as_str().unwrap().contains("did not create"),
            "{v}"
        );
        assert_eq!(
            std::fs::read_to_string(&entry).unwrap(),
            "[Desktop Entry]\nName=Mine\nExec=mine\n"
        );
        assert_eq!(h.ok("disguise_get", json!({}))["presetId"], "tally");
    }

    #[test]
    fn it_works_without_a_main_window() {
        let h = harness(false);
        let home = tempfile::tempdir().unwrap();
        install_in(&h, home.path());
        let v = h.ok(
            "disguise_set",
            set_args(json!("vellum"), json!(null), false),
        );
        assert_eq!(v["note"], NOTE_LINUX);
        assert!(entry_of(home.path()).exists());
    }

    #[test]
    fn the_saved_look_is_applied_again_at_startup_and_repairs_the_entry() {
        let h = harness(true);
        let home = tempfile::tempdir().unwrap();
        install_in(&h, home.path());
        h.ok(
            "disguise_set",
            set_args(json!("drizzle"), json!("Weather"), false),
        );
        let entry = entry_of(home.path());
        // the entry went stale (the executable moved, somebody edited it): startup rewrites it
        let good = std::fs::read_to_string(&entry).unwrap();
        std::fs::write(
            &entry,
            format!("{good}Exec=stale\nX-Passman-Managed=true\n"),
        )
        .unwrap();
        let v = h.core().disguise_apply_saved();
        assert_eq!(v.applied_name, "Weather");
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), good);
    }
}
