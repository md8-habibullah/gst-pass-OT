//! The importer / exporter commands through Tauri's mock runtime: real IPC dispatch, camelCase
//! arguments as the webview sends them, and the exact JSON shapes of `types.ts`.

use super::super::*;
use crate::crypto::KdfParams;
use crate::storage::files::TestPicker;
use crate::storage::{BackupRuntime, Timing};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

const PW: &str = "correct horse battery staple";

struct Harness {
    /// The app's own data folder (exports may not be written here).
    _dir: tempfile::TempDir,
    /// Where the files to import and the exports live.
    other: tempfile::TempDir,
    picker: Arc<TestPicker>,
    _app: tauri::App<MockRuntime>,
    window: WebviewWindow<MockRuntime>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let picker = Arc::new(TestPicker::default());
    let cfg = SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: Duration::from_secs(600),
        purge_grace: std::time::Duration::ZERO,
    };
    let runtime = BackupRuntime::new(
        crate::storage::backup::remote::mem::MemFactory::new(),
        Timing {
            background: false,
            ..Timing::default()
        },
    );
    let core = AppCore::open(dir.path(), cfg, picker.clone())
        .unwrap()
        .with_backup_runtime(runtime);
    let app = mock_builder()
        .manage(AppState::new(core))
        .invoke_handler(invoke_handler())
        .build(mock_context(noop_assets()))
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    Harness {
        _dir: dir,
        other: tempfile::tempdir().unwrap(),
        picker,
        _app: app,
        window,
    }
}

impl Harness {
    fn call(&self, cmd: &str, args: Value) -> Result<Value, Value> {
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

    fn err_code(&self, cmd: &str, args: Value) -> String {
        let e = self.call(cmd, args).expect_err("expected an error");
        e["code"]
            .as_str()
            .unwrap_or_else(|| panic!("{cmd}: error without a code: {e}"))
            .to_string()
    }

    fn file(&self, name: &str, body: &str) -> std::path::PathBuf {
        let p = self.other.path().join(name);
        std::fs::write(&p, body).unwrap();
        p
    }
}

const BITWARDEN_CSV: &str = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
Work,1,login,GitHub,,,0,https://github.com/login,ana@example.com,pw-github-111,\n\
,0,login,Mail,,,0,https://mail.example.com,ana,pw-mail-222,\n";

#[test]
fn every_importer_command_is_registered_and_needs_an_unlocked_vault() {
    let h = harness();
    // Locked (no account yet): each one is refused with the contract's `locked`, which also
    // proves the command exists and its arguments were accepted.
    assert_eq!(h.err_code("import_pick", json!({})), "locked");
    assert_eq!(h.err_code("import_pick_keyfile", json!({})), "locked");
    assert_eq!(
        h.err_code("inspect_csv_headers", json!({"handle": "00"})),
        "locked"
    );
    let options =
        json!({"options": {"handle": "00", "format": "bitwarden-csv", "mode": "skip_duplicates"}});
    assert_eq!(h.err_code("import_preview", options.clone()), "locked");
    assert_eq!(h.err_code("import_commit", options), "locked");
    assert_eq!(
        h.err_code("export_plain", json!({"format": "csv", "password": PW})),
        "locked"
    );
    // Cancelling is harmless whatever the state.
    assert_eq!(h.ok("import_cancel", json!({"handle": "00"})), Value::Null);
    // Missing arguments are rejected by Tauri itself.
    assert!(h.call("inspect_csv_headers", json!({})).is_err());
    assert!(h.call("import_preview", json!({})).is_err());
    assert!(h.call("export_plain", json!({"format": "csv"})).is_err());
}

#[test]
fn import_and_export_work_over_real_ipc() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Personal", "password": PW}),
    );

    // A cancelled dialog is null, not an error.
    assert_eq!(h.ok("import_pick", json!({})), Value::Null);
    assert_eq!(h.ok("import_pick_keyfile", json!({})), Value::Null);

    // pick: camelCase result, no path anywhere.
    let path = h.file("export.csv", BITWARDEN_CSV);
    h.picker.queue_import(path.clone());
    let pick = h.ok("import_pick", json!({}));
    assert_eq!(pick["format"], "bitwarden-csv");
    assert_eq!(pick["fileName"], "export.csv");
    assert_eq!(pick["needsPassword"], false);
    assert_eq!(pick["needsKeyfile"], false);
    assert_eq!(pick["size"], BITWARDEN_CSV.len());
    assert!(
        !pick.to_string().contains(h.other.path().to_str().unwrap()),
        "path leaked: {pick}"
    );
    let handle = pick["handle"].as_str().unwrap().to_string();
    assert_eq!(handle.len(), 32);

    // inspect (camelCase `handle` argument)
    let csv = h.ok("inspect_csv_headers", json!({"handle": handle}));
    assert_eq!(csv["delimiter"], ",");
    assert_eq!(csv["headers"][3], "name");
    assert_eq!(csv["sample"].as_array().unwrap().len(), 2);

    // preview: options as the webview builds them (snake_case mode, camelCase fields)
    let options = json!({"options": {
        "handle": handle, "format": "bitwarden-csv", "mode": "skip_duplicates"
    }});
    let preview = h.ok("import_preview", options.clone());
    assert_eq!(preview["entries"], 2);
    assert_eq!(preview["totpSeeds"], 0);
    assert_eq!(preview["attachments"], 0);
    assert_eq!(preview["duplicates"], 0);
    assert_eq!(preview["skipped"], 0);
    assert_eq!(preview["titles"], json!(["GitHub", "Mail"]));
    assert!(preview["warnings"].as_array().unwrap().is_empty());
    assert!(
        !preview.to_string().contains("pw-github-111"),
        "preview carries no secret"
    );
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 0);

    // commit
    let outcome = h.ok("import_commit", options.clone());
    assert_eq!(outcome["added"], 2);
    assert_eq!(outcome["appended"], 0);
    assert_eq!(outcome["skipped"], 0);
    assert_eq!(outcome["failed"], 0);
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 2);

    // a second commit skips both as duplicates; the options are validated like any other input
    let again = h.ok("import_commit", options);
    assert_eq!(
        (again["added"].as_u64(), again["skipped"].as_u64()),
        (Some(0), Some(2))
    );
    assert!(h
        .call(
            "import_preview",
            json!({"options": {"handle": handle, "format": "no-such-format", "mode": "skip_duplicates"}})
        )
        .is_err());
    assert!(h
        .call(
            "import_preview",
            json!({"options": {"handle": handle, "format": "bitwarden-csv", "mode": "overwrite"}})
        )
        .is_err());

    // cancel forgets the file: the handle is gone afterwards
    h.ok("import_cancel", json!({"handle": handle}));
    assert_eq!(
        h.err_code("inspect_csv_headers", json!({"handle": handle})),
        "not_found"
    );

    // export: the master password is required again, then the destination dialog opens
    assert_eq!(
        h.err_code(
            "export_plain",
            json!({"format": "csv", "password": "wrong password!!"})
        ),
        "wrong_password"
    );
    assert!(
        h.picker.export_names().is_empty(),
        "no dialog for a wrong password"
    );
    assert_eq!(
        h.err_code("export_plain", json!({"format": "xml", "password": PW})),
        "invalid_input"
    );
    assert_eq!(
        h.ok("export_plain", json!({"format": "csv", "password": PW})),
        Value::Null,
        "a cancelled save dialog is null"
    );
    let dest = h.other.path().join("out.json");
    h.picker.set_export(Some(dest.clone()));
    let done = h.ok(
        "export_plain",
        json!({"format": "bitwarden-json", "password": PW}),
    );
    assert_eq!(done["path"], dest.to_string_lossy().as_ref());
    assert!(
        done.get("warnings").is_none(),
        "warnings are absent when empty"
    );
    let written = std::fs::read_to_string(&dest).unwrap();
    assert!(written.contains("pw-github-111"), "the export is plaintext");
    let names = h.picker.export_names();
    assert_eq!(names.len(), 2, "one name per dialog that opened: {names:?}");
    assert!(names[0].ends_with("-plaintext.csv"), "{names:?}");
    assert!(names[1].ends_with("-plaintext.json"), "{names:?}");
}

#[test]
fn locking_clears_import_handles_over_ipc() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Personal", "password": PW}),
    );
    h.picker.queue_import(h.file("a.csv", BITWARDEN_CSV));
    let handle = h.ok("import_pick", json!({}))["handle"]
        .as_str()
        .unwrap()
        .to_string();
    h.ok("vault_lock", json!({}));
    assert_eq!(
        h.err_code("inspect_csv_headers", json!({"handle": handle})),
        "locked"
    );
    h.ok("vault_unlock", json!({"password": PW}));
    assert_eq!(
        h.err_code("inspect_csv_headers", json!({"handle": handle})),
        "not_found",
        "handles do not survive a lock"
    );
}
