//! End-to-end tests through Tauri's mock runtime: real IPC dispatch, real argument
//! deserialization (camelCase from JavaScript), real serialization of results and errors.
//! This is what the frontend actually sees.

use super::*;
use crate::crypto::KdfParams;
use crate::storage::backup::remote::mem::MemFactory;
use crate::storage::files::TestPicker;
use crate::storage::{BackupRuntime, RecordingOpener, Timing};
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
    dir: tempfile::TempDir,
    picker: Arc<TestPicker>,
    /// In-memory buckets (one per bucket name) standing in for S3.
    mem: Arc<MemFactory>,
    /// Records every address `open_url` was allowed to open.
    opener: Arc<RecordingOpener>,
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
    let mem = MemFactory::new();
    let opener = Arc::new(RecordingOpener::new());
    let runtime = BackupRuntime::new(
        mem.clone(),
        Timing {
            background: false,
            ..Timing::default()
        },
    );
    let core = AppCore::open(dir.path(), cfg, picker.clone())
        .unwrap()
        .with_backup_runtime(runtime)
        .with_opener(opener.clone());
    let state = AppState::new(core);
    let app = mock_builder()
        .manage(state)
        .invoke_handler(invoke_handler())
        .build(mock_context(noop_assets()))
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    Harness {
        dir,
        picker,
        mem,
        opener,
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
}

fn draft(title: &str) -> Value {
    json!({
        "type": "login",
        "title": title,
        "favorite": false,
        "tags": ["work"],
        "folder": null,
        "fields": {
            "username": "ana@example.com",
            "password": "pw-first-111",
            "url": "https://github.com/login",
            "totp": "JBSWY3DPEHPK3PXP"
        },
        "customFields": [{"name": "Plan", "value": "Team", "hidden": false}]
    })
}

#[test]
fn the_whole_contract_works_over_real_ipc() {
    let h = harness();

    // --- status and creation
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["exists"], false);
    assert_eq!(st["unlocked"], false);
    assert!(
        st.get("capabilities").is_none(),
        "the quick-unlock design is gone: no capabilities field"
    );
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    assert_eq!(
        h.err_code("vault_unlock", json!({"password": PW})),
        "no_vault"
    );
    assert!(
        h.call("vault_create", json!({"password": PW})).is_err(),
        "vault_create no longer exists: accounts are created with profile_create"
    );
    assert_eq!(
        h.err_code(
            "profile_create",
            json!({"displayName": "Personal", "password": "short"})
        ),
        "invalid_input"
    );
    let me = h.ok(
        "profile_create",
        json!({"displayName": "Personal", "password": PW}),
    );
    assert_eq!(me["displayName"], "Personal");
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["unlocked"], true);
    assert_eq!(st["exists"], true);
    assert_eq!(st["profile"]["id"], me["id"]);
    assert_eq!(
        h.err_code(
            "profile_create",
            json!({"displayName": "personal", "password": PW})
        ),
        "profile_name_taken"
    );

    // --- create and list
    let created = h.ok("item_create", json!({"draft": draft("GitHub")}));
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["type"], "login");
    assert_eq!(created["subtitle"], "ana@example.com");
    assert_eq!(created["hasTotp"], true);
    assert_eq!(created["version"], 1);
    assert!(created["updatedAt"].as_u64().unwrap() > 1_700_000_000_000);
    let list = h.ok("item_list", json!({}));
    assert_eq!(list.as_array().unwrap().len(), 1);

    // --- masked view never carries secrets
    let view = h.ok("item_get", json!({"id": id}));
    let text = view.to_string();
    for secret in ["pw-first-111", "JBSWY3DPEHPK3PXP"] {
        assert!(!text.contains(secret), "{secret} leaked through item_get");
    }
    let pw_field = view["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["key"] == "password")
        .unwrap();
    assert_eq!(pw_field["secret"], true);
    assert!(pw_field["value"].is_null());
    assert_eq!(pw_field["hasValue"], true);
    assert_eq!(pw_field["kind"], "password");

    // --- one-time codes
    let code = h.ok("totp_now", json!({"id": id}));
    for k in [
        "code",
        "previous",
        "next",
        "period",
        "digits",
        "secondsRemaining",
        "validUntilMs",
    ] {
        assert!(code.get(k).is_some(), "totp_now lacks {k}");
    }
    assert!(!code.to_string().contains("JBSWY3DPEHPK3PXP"));

    // --- the re-prompt guard, with the exact JS-style argument names
    assert_eq!(
        h.err_code(
            "item_reveal",
            json!({"id": id, "password": "wrong password!"})
        ),
        "wrong_password"
    );
    let revealed = h.ok("item_reveal", json!({"id": id, "password": PW}));
    assert_eq!(revealed["fields"]["password"], "pw-first-111");

    let begin = h.ok("item_begin_edit", json!({"id": id, "password": PW}));
    let token = begin["token"].as_str().unwrap().to_string();
    assert_eq!(begin["version"], 1);
    let mut edited = begin["draft"].clone();
    edited["fields"]["password"] = json!("pw-second-222");
    // snake_case argument names are NOT accepted: the UI must send camelCase
    assert!(h
        .call(
            "item_update",
            json!({"id": id, "token": token, "expected_version": 1, "draft": edited})
        )
        .is_err());
    let updated = h.ok(
        "item_update",
        json!({"id": id, "token": token, "expectedVersion": 1, "draft": edited}),
    );
    assert_eq!(updated["version"], 2);
    assert_eq!(
        h.err_code(
            "item_update",
            json!({"id": id, "token": token, "expectedVersion": 1, "draft": edited})
        ),
        "token_invalid"
    );

    // --- history
    let hist = h.ok("history_list", json!({"id": id}));
    let hist = hist.as_array().unwrap();
    assert_eq!(hist.len(), 2);
    assert_eq!(hist[0]["version"], 2);
    assert_eq!(hist[0]["event"], "edited");
    assert_eq!(hist[0]["changedFields"], json!(["password"]));
    assert_eq!(hist[1]["event"], "created");
    assert!(hist[0].get("restoredFrom").is_some());
    assert_eq!(
        h.err_code(
            "history_get",
            json!({"id": id, "version": 1, "password": "nope nope nope"})
        ),
        "wrong_password"
    );
    let old = h.ok(
        "history_get",
        json!({"id": id, "version": 1, "password": PW}),
    );
    assert_eq!(old["fields"]["password"], "pw-first-111");
    let restored = h.ok(
        "history_restore",
        json!({"id": id, "version": 1, "password": PW}),
    );
    assert_eq!(restored["version"], 3);

    // --- trash: restricted view, restore, purge with the password (the sum for a recent one is in folder_search_tests)
    h.ok("item_trash", json!({"id": id}));
    assert_eq!(h.err_code("item_get", json!({"id": id})), "in_trash");
    assert_eq!(
        h.err_code("item_reveal", json!({"id": id, "password": PW})),
        "in_trash"
    );
    let trash = h.ok("trash_list", json!({}));
    assert_eq!(trash.as_array().unwrap().len(), 1);
    let entry = trash[0].as_object().unwrap();
    assert_eq!(entry.len(), 3);
    assert_eq!(entry["title"], "GitHub");
    assert!(entry["deletedAt"].as_u64().unwrap() > 0);
    assert!(!trash.to_string().contains("ana@example.com"));
    let back = h.ok("item_restore", json!({"id": id}));
    assert_eq!(back["version"], 5);
    h.ok("item_trash", json!({"id": id}));
    assert_eq!(
        h.err_code(
            "item_purge",
            json!({"id": id, "password": "wrong wrong", "answer": null})
        ),
        "wrong_password"
    );
    // the answer is optional: an old item needs the password alone
    h.ok("item_purge", json!({"id": id, "password": PW}));
    assert_eq!(h.ok("trash_list", json!({})).as_array().unwrap().len(), 0);
    assert_eq!(h.err_code("item_get", json!({"id": id})), "not_found");

    // --- password change, lock, unlock
    assert_eq!(
        h.err_code(
            "vault_change_password",
            json!({"oldPassword": "wrong wrong", "newPassword": "a new master password"})
        ),
        "wrong_password"
    );
    h.ok(
        "vault_change_password",
        json!({"oldPassword": PW, "newPassword": "a new master password"}),
    );
    h.ok("vault_lock", json!({}));
    assert_eq!(h.ok("vault_status", json!({}))["unlocked"], false);
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    assert_eq!(
        h.err_code("vault_unlock", json!({"password": PW})),
        "wrong_password"
    );
    h.ok("vault_unlock", json!({"password": "a new master password"}));
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 0);

    // --- suite info still works while unlocked or locked
    assert_eq!(
        h.ok("crypto_suite_info", json!({}))["kem"],
        "ML-KEM-1024+X25519"
    );
}

#[test]
fn errors_cross_the_boundary_as_code_and_message() {
    let h = harness();
    let e = h.call("item_list", json!({})).unwrap_err();
    assert_eq!(e["code"], "locked");
    assert!(e["message"].as_str().unwrap().contains("locked"));
    // unknown commands and bad arguments are rejected by Tauri itself
    assert!(h.call("no_such_command", json!({})).is_err());
    h.ok(
        "profile_create",
        json!({"displayName": "Personal", "password": PW}),
    );
    assert!(h.call("item_get", json!({})).is_err(), "missing id");
    assert!(h
        .call(
            "item_create",
            json!({"draft": {"type": "login", "title": "x", "bogus": 1}})
        )
        .is_err());
    assert_eq!(
        h.err_code(
            "item_create",
            json!({"draft": {"type": "login", "title": "   "}})
        ),
        "invalid_input"
    );
}

// ============================================================ Phase 2b: types, archive, clone

fn card_draft(title: &str) -> Value {
    json!({
        "type": "card",
        "title": title,
        "favorite": false,
        "tags": [],
        "folder": null,
        "fields": {
            "cardholder": "Ana Example",
            "brand": "Visa",
            "number": "4111 1111 1111 1111",
            "expMonth": "09",
            "expYear": "2029",
            "code": "737"
        },
        "customFields": []
    })
}

fn signed_in(h: &Harness) -> Value {
    h.ok(
        "profile_create",
        json!({"displayName": "Personal", "password": PW}),
    )
}

#[test]
fn card_identity_and_ssh_items_work_over_real_ipc() {
    let h = harness();
    signed_in(&h);

    let card = h.ok("item_create", json!({"draft": card_draft("Everyday card")}));
    assert_eq!(card["type"], "card");
    assert_eq!(card["subtitle"], "Visa •••• 1111");
    assert_eq!(card["archived"], false);
    let id = card["id"].as_str().unwrap().to_string();

    // the card number and code are secret: the plain read shows only that they exist
    let view = h.ok("item_get", json!({"id": id}));
    let field = |k: &str| {
        view["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["key"] == k)
            .unwrap_or_else(|| panic!("no field {k}"))
            .clone()
    };
    assert_eq!(field("number")["secret"], true);
    assert_eq!(field("number")["value"], Value::Null);
    assert_eq!(field("number")["hasValue"], true);
    assert_eq!(field("code")["value"], Value::Null);
    assert_eq!(field("cardholder")["value"], "Ana Example");
    assert_eq!(field("expMonth")["value"], "09");

    // the re-prompt reveals them (camelCase keys survive the round trip)
    let revealed = h.ok("item_reveal", json!({"id": id, "password": PW}));
    assert_eq!(revealed["fields"]["number"], "4111 1111 1111 1111");
    assert_eq!(revealed["fields"]["expYear"], "2029");

    // light validation
    let mut bad = card_draft("Bad");
    bad["fields"]["number"] = json!("12ab");
    assert_eq!(
        h.err_code("item_create", json!({"draft": bad})),
        "invalid_input"
    );
    let mut bad = card_draft("Bad");
    bad["fields"]["expMonth"] = json!("13");
    assert_eq!(
        h.err_code("item_create", json!({"draft": bad})),
        "invalid_input"
    );

    let identity = h.ok(
        "item_create",
        json!({"draft": {
            "type": "identity", "title": "Me", "favorite": false, "tags": [], "folder": null,
            "fields": {"firstName": "Ana", "lastName": "Example", "email": "ana@example.com",
                       "nationalId": "123-45-6789"},
            "customFields": []
        }}),
    );
    assert_eq!(identity["subtitle"], "Ana Example");
    assert_eq!(identity["type"], "identity");

    let ssh = h.ok(
        "item_create",
        json!({"draft": {
            "type": "sshKey", "title": "Laptop key", "favorite": false, "tags": [], "folder": null,
            "fields": {"publicKey": "ssh-ed25519 AAAAC3Nza ana@laptop",
                       "privateKey": "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----"},
            "customFields": []
        }}),
    );
    assert_eq!(ssh["type"], "sshKey");
    assert_eq!(ssh["subtitle"], "ana@laptop");
    // the public key is not secret, the private key is
    let ssh_view = h.ok("item_get", json!({"id": ssh["id"]}));
    let ssh_field = |k: &str| {
        ssh_view["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["key"] == k)
            .unwrap()
            .clone()
    };
    assert_eq!(ssh_field("publicKey")["secret"], false);
    assert_eq!(ssh_field("publicKey")["kind"], "paragraph");
    assert_eq!(ssh_field("privateKey")["secret"], true);
    assert_eq!(ssh_field("privateKey")["value"], Value::Null);

    // a field of another type is refused
    let mut wrong = card_draft("Wrong");
    wrong["fields"]["username"] = json!("x");
    assert_eq!(
        h.err_code("item_create", json!({"draft": wrong})),
        "invalid_input"
    );
}

#[test]
fn archive_and_clone_work_over_real_ipc() {
    let h = harness();
    signed_in(&h);
    let a = h.ok("item_create", json!({"draft": draft("GitHub")}));
    let id = a["id"].as_str().unwrap().to_string();

    // archive: no password, still listed (flagged), still readable and editable
    let arch = h.ok("item_archive", json!({"id": id}));
    assert_eq!(arch["archived"], true);
    assert_eq!(arch["version"], 2);
    let list = h.ok("item_list", json!({}));
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["archived"], true);
    assert_eq!(h.ok("item_get", json!({"id": id}))["title"], "GitHub");
    // archiving twice is harmless and does not add a version
    assert_eq!(h.ok("item_archive", json!({"id": id}))["version"], 2);

    let history = h.ok("history_list", json!({"id": id}));
    assert_eq!(history[0]["event"], "archived");
    assert_eq!(history[0]["changedFields"], json!([]));
    assert_eq!(history[1]["event"], "created");

    let un = h.ok("item_unarchive", json!({"id": id}));
    assert_eq!(un["archived"], false);
    assert_eq!(un["version"], 3);
    assert_eq!(
        h.ok("history_list", json!({"id": id}))[0]["event"],
        "unarchived"
    );

    // clone: password-gated, new id, version 1, title with the suffix, archived reset
    h.ok("item_archive", json!({"id": id}));
    assert_eq!(
        h.err_code(
            "item_clone",
            json!({"id": id, "password": "wrong password!"})
        ),
        "wrong_password"
    );
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 1);
    let copy = h.ok("item_clone", json!({"id": id, "password": PW}));
    assert_ne!(copy["id"], json!(id));
    assert_eq!(copy["title"], "GitHub (copy)");
    assert_eq!(copy["version"], 1);
    assert_eq!(copy["archived"], false);
    assert_eq!(copy["hasTotp"], true);
    let copy_id = copy["id"].as_str().unwrap();
    let revealed = h.ok("item_reveal", json!({"id": copy_id, "password": PW}));
    assert_eq!(revealed["fields"]["password"], "pw-first-111");
    assert_eq!(revealed["tags"], json!(["work"]));
    assert_eq!(revealed["customFields"][0]["name"], "Plan");

    // trashed items can be neither archived nor cloned
    h.ok("item_trash", json!({"id": copy_id}));
    assert_eq!(
        h.err_code("item_archive", json!({"id": copy_id})),
        "in_trash"
    );
    assert_eq!(
        h.err_code("item_clone", json!({"id": copy_id, "password": PW})),
        "in_trash"
    );
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 1);
    // restoring puts it back, and the archive flag survives trash and restore
    h.ok("item_trash", json!({"id": id}));
    assert_eq!(h.ok("item_restore", json!({"id": id}))["archived"], true);
}

// ============================================================ Phase 2b: accounts

#[test]
fn accounts_work_over_real_ipc() {
    let h = harness();
    let empty = h.ok("profile_list", json!({}));
    assert_eq!(empty["profiles"], json!([]));
    assert_eq!(empty["selectedId"], Value::Null);
    assert_eq!(empty["lastUsedId"], Value::Null);
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["exists"], false);
    assert_eq!(st["profile"], Value::Null);

    let work = signed_in(&h);
    h.ok("item_create", json!({"draft": draft("Work item")}));
    let home = h.ok(
        "profile_create",
        json!({"displayName": "Home", "password": "another long password"}),
    );
    // creating selects the new account and wipes the old one
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 0);
    h.ok("item_create", json!({"draft": draft("Home item")}));

    let list = h.ok("profile_list", json!({}));
    assert_eq!(list["profiles"].as_array().unwrap().len(), 2);
    assert_eq!(list["selectedId"], home["id"]);
    assert_eq!(list["lastUsedId"], home["id"]);

    // switching: the other account's items are not visible, and its password is its own
    h.ok("profile_select", json!({"id": work["id"]}));
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["exists"], true);
    assert_eq!(st["unlocked"], false);
    assert_eq!(st["profile"]["displayName"], "Personal");
    assert_eq!(
        h.err_code("vault_unlock", json!({"password": "another long password"})),
        "wrong_password"
    );
    h.ok("vault_unlock", json!({"password": PW}));
    let items = h.ok("item_list", json!({}));
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["title"], "Work item");

    // rename: unique, normalized, selected + unlocked only
    let renamed = h.ok(
        "profile_rename",
        json!({"id": work["id"], "displayName": "  Work   Stuff "}),
    );
    assert_eq!(renamed["displayName"], "Work Stuff");
    assert_eq!(
        h.err_code(
            "profile_rename",
            json!({"id": work["id"], "displayName": "HOME"})
        ),
        "profile_name_taken"
    );
    assert_eq!(
        h.err_code(
            "profile_rename",
            json!({"id": home["id"], "displayName": "Elsewhere"})
        ),
        "locked"
    );
    assert_eq!(
        h.err_code("profile_select", json!({"id": "nope"})),
        "not_found"
    );

    // remove: password and confirmation both required; only that account goes
    assert_eq!(
        h.err_code(
            "profile_remove",
            json!({"id": home["id"], "password": PW, "confirmation": "Home"})
        ),
        "wrong_password"
    );
    assert_eq!(
        h.err_code(
            "profile_remove",
            json!({"id": home["id"], "password": "another long password", "confirmation": "no"})
        ),
        "confirmation_mismatch"
    );
    h.ok(
        "profile_remove",
        json!({"id": home["id"], "password": "another long password", "confirmation": "Home"}),
    );
    let list = h.ok("profile_list", json!({}));
    assert_eq!(list["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(list["profiles"][0]["displayName"], "Work Stuff");
    // the surviving account is untouched and still unlocked
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 1);
}

#[test]
fn export_and_import_work_over_real_ipc() {
    let h = harness();
    signed_in(&h);
    h.ok("item_create", json!({"draft": draft("Keep me")}));
    h.ok("item_create", json!({"draft": card_draft("Card")}));
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("backup.hexis");

    // cancelled dialog: nothing is written
    h.picker.set_save(None);
    assert_eq!(
        h.ok("vault_export_file", json!({"password": PW})),
        Value::Null
    );
    // the password is checked before any dialog
    h.picker.set_save(Some(file.clone()));
    assert_eq!(
        h.err_code("vault_export_file", json!({"password": "wrong password!"})),
        "wrong_password"
    );
    assert!(!file.exists());

    let done = h.ok("vault_export_file", json!({"password": PW}));
    assert_eq!(done["path"], json!(file.to_string_lossy()));
    assert!(file.exists());
    let names = h.picker.suggested_names();
    assert!(names.last().unwrap().starts_with("Personal-"), "{names:?}");
    assert!(names.last().unwrap().ends_with(".hexis"));

    // import it as a second account: it opens with its original master password
    h.picker.set_open(None);
    assert_eq!(
        h.ok("profile_import_file", json!({"displayName": "Restored"})),
        Value::Null,
        "a cancelled dialog is not an error"
    );
    assert_eq!(
        h.err_code("profile_import_file", json!({"displayName": "personal"})),
        "profile_name_taken"
    );
    h.picker.set_open(Some(file.clone()));
    let imported = h.ok("profile_import_file", json!({"displayName": "Restored"}));
    assert_eq!(imported["displayName"], "Restored");
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["profile"]["id"], imported["id"]);
    assert_eq!(st["unlocked"], false);
    h.ok("vault_unlock", json!({"password": PW}));
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 2);

    // a file that is not a vault is refused with a specific code
    let junk = out.path().join("junk.hexis");
    std::fs::write(&junk, b"this is not a database").unwrap();
    h.picker.set_open(Some(junk));
    assert_eq!(
        h.err_code("profile_import_file", json!({"displayName": "Junk"})),
        "unsupported_file"
    );
    let list = h.ok("profile_list", json!({}));
    assert_eq!(
        list["profiles"].as_array().unwrap().len(),
        2,
        "no half-made account"
    );
    assert!(h.dir.path().join("profiles.json").exists());
}

// ---------------------------------------------------------------------------- Phase 4a

/// Settings exactly as the frontend sends them (camelCase keys).
fn target_args(name: &str, bucket: &str) -> Value {
    json!({
        "name": name,
        "endpoint": null,
        "region": "eu-west-1",
        "bucket": bucket,
        "prefix": "vaults/ana",
        "accessKeyId": "AKIDEXAMPLE0042",
        "secretAccessKey": "SECRETKEY-9d41c7-do-not-leak",
        "pathStyle": false,
        "enabled": true,
        "master": false,
        "shareInMaster": true
    })
}

const SECRET: &str = "SECRETKEY-9d41c7-do-not-leak";

#[test]
fn backup_targets_work_over_real_ipc() {
    let h = harness();

    // the probe needs no account at all
    let t = h.ok("backup_test", json!({"input": target_args("probe", "bkt")}));
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["conditionalWrites"], "strong");
    assert!(t["message"].as_str().unwrap().starts_with("Connected"));
    // ... but the list of saved targets needs an unlocked account
    assert_eq!(h.err_code("backup_list", json!({})), "locked");

    signed_in(&h);
    assert_eq!(h.ok("backup_list", json!({})), json!([]));

    // camelCase in, camelCase out; the secret never comes back
    let mut master = target_args("Home bucket", "bkt");
    master["master"] = json!(true);
    let first = h.ok("backup_add", json!({"input": master}));
    assert_eq!(first["name"], "Home bucket");
    assert_eq!(first["accessKeyId"], "AKIDEXAMPLE0042");
    assert_eq!(first["hasSecret"], true);
    assert_eq!(first["master"], true);
    assert_eq!(first["shareInMaster"], true);
    assert_eq!(first["pathStyle"], false);
    assert_eq!(first["endpoint"], Value::Null);
    assert_eq!(first["health"]["state"], "healthy");
    assert_eq!(first["health"]["conditionalWrites"], "strong");
    // the first upload is queued (no worker thread runs in this harness), not yet sent
    assert!(first["health"]["queued"].as_u64().unwrap() > 0, "{first}");
    assert!(first["health"]["lastSuccessAt"].is_number());
    assert!(first.get("secretAccessKey").is_none() && first.get("secret").is_none());
    assert!(!first.to_string().contains(SECRET));
    let first_id = first["id"].as_str().unwrap().to_string();

    // snake_case keys are not part of the wire contract
    let mut snake = json!({});
    for (k, v) in target_args("Snake", "bkt2").as_object().unwrap() {
        let k = match k.as_str() {
            "accessKeyId" => "access_key_id",
            "secretAccessKey" => "secret_access_key",
            other => other,
        };
        snake[k] = v.clone();
    }
    assert!(h.call("backup_add", json!({"input": snake})).is_err());
    // as is a snake_case argument name
    assert!(h
        .call("backup_add", json!({"target": target_args("X", "b")}))
        .is_err());

    let second = h.ok(
        "backup_add",
        json!({"input": target_args("Second", "bkt2")}),
    );
    let second_id = second["id"].as_str().unwrap().to_string();
    assert_eq!(second["master"], false);
    let order = |h: &Harness| -> Vec<String> {
        h.ok("backup_list", json!({}))
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        order(&h),
        ["Home bucket", "Second"],
        "the master is listed first"
    );
    h.ok("backup_set_master", json!({"id": second_id}));
    assert_eq!(order(&h), ["Second", "Home bucket"]);
    let list = h.ok("backup_list", json!({}));
    assert_eq!(list[0]["master"], true);
    assert_eq!(list[1]["master"], false, "a new master demotes the old one");

    // an item reaches both buckets through backup_run (null, absent and a concrete id)
    h.ok("item_create", json!({"draft": draft("Bank")}));
    for args in [json!({}), json!({"id": null}), json!({"id": first_id})] {
        let ran = h.ok("backup_run", args);
        assert_eq!(ran.as_array().unwrap().len(), 2);
    }
    let ran = h.ok("backup_run", json!({"id": null}));
    for t in ran.as_array().unwrap() {
        assert_eq!(t["health"]["state"], "healthy", "{t}");
        assert_eq!(t["health"]["queued"], 0);
    }
    for bucket in ["bkt", "bkt2"] {
        let keys = h.mem.bucket(bucket).keys();
        assert!(
            keys.contains(&"vault/meta/keybundle.json".to_string()),
            "{bucket}: {keys:?}"
        );
        assert!(
            keys.iter().any(|k| k.contains("vault/items/")),
            "{bucket}: {keys:?}"
        );
    }

    // an empty secret keeps the stored one; reveal needs the master password
    let mut change = target_args("Second (renamed)", "bkt2");
    change["secretAccessKey"] = json!("");
    let updated = h.ok("backup_update", json!({"id": second_id, "input": change}));
    assert_eq!(updated["name"], "Second (renamed)");
    assert_eq!(updated["hasSecret"], true);
    assert_eq!(
        h.ok(
            "backup_reveal_secret",
            json!({"id": second_id, "password": PW})
        ),
        json!(SECRET)
    );
    assert_eq!(
        h.err_code(
            "backup_reveal_secret",
            json!({"id": second_id, "password": "not the password"})
        ),
        "wrong_password"
    );

    // errors keep the {code, message} shape and never contain the secret
    let mut insecure = target_args("Insecure", "bkt3");
    insecure["endpoint"] = json!("http://storage.example.com");
    let e = h
        .call("backup_add", json!({"input": insecure}))
        .unwrap_err();
    assert_eq!(e["code"], "invalid_input");
    assert!(!e.to_string().contains(SECRET), "{e}");
    assert_eq!(
        h.err_code(
            "backup_remove",
            json!({"id": "00000000-0000-4000-8000-000000000000"})
        ),
        "not_found"
    );
    h.ok("backup_remove", json!({"id": first_id}));
    assert_eq!(h.ok("backup_list", json!({})).as_array().unwrap().len(), 1);

    // locked: the vault-bound commands refuse, the probe still works
    h.ok("vault_lock", json!({}));
    for (cmd, args) in [
        ("backup_list", json!({})),
        ("backup_add", json!({"input": target_args("Late", "b")})),
        ("backup_run", json!({})),
        (
            "backup_reveal_secret",
            json!({"id": second_id, "password": PW}),
        ),
    ] {
        assert_eq!(h.err_code(cmd, args), "locked", "{cmd}");
    }
    assert_eq!(
        h.ok("backup_test", json!({"input": target_args("probe", "bkt")}))["ok"],
        true
    );
}

#[test]
fn a_failing_service_is_an_answer_not_an_error() {
    let h = harness();
    signed_in(&h);
    h.mem.bucket("down").fail_all(
        None,
        crate::storage::backup::remote::RemoteError::AccessDenied,
    );
    let t = h.ok("backup_test", json!({"input": target_args("down", "down")}));
    assert_eq!(t["ok"], false);
    assert!(t["message"].as_str().unwrap().contains("403"), "{t}");
    assert_eq!(t["conditionalWrites"], "unknown");
    // adding it still saves the settings and shows why it does not work
    let v = h.ok("backup_add", json!({"input": target_args("down", "down")}));
    assert_eq!(v["health"]["state"], "failing");
    assert!(v["health"]["lastError"].as_str().unwrap().contains("403"));
    assert!(!v.to_string().contains(SECRET));
}

#[test]
fn restoring_from_a_backup_works_over_real_ipc() {
    let h = harness();
    signed_in(&h);
    let mut master = target_args("Home bucket", "bkt");
    master["master"] = json!(true);
    h.ok("backup_add", json!({"input": master}));
    h.ok("item_create", json!({"draft": draft("Keep me")}));
    h.ok("item_create", json!({"draft": card_draft("Card")}));
    h.ok("backup_run", json!({}));
    h.ok("vault_lock", json!({}));

    // the name and the settings are checked before any request leaves the device
    let calls_before = h.mem.opened().len();
    assert_eq!(
        h.err_code(
            "profile_restore_from_backup",
            json!({"displayName": "personal", "input": target_args("src", "bkt")})
        ),
        "profile_name_taken"
    );
    assert_eq!(
        h.err_code(
            "profile_restore_from_backup",
            json!({"displayName": "  ", "input": target_args("src", "bkt")})
        ),
        "invalid_input"
    );
    let mut bad = target_args("src", "bkt");
    bad["endpoint"] = json!("http://storage.example.com");
    assert_eq!(
        h.err_code(
            "profile_restore_from_backup",
            json!({"displayName": "Copy", "input": bad})
        ),
        "invalid_input"
    );
    assert_eq!(
        h.mem.opened().len(),
        calls_before,
        "no network call was made"
    );

    // a bucket without a vault is refused and leaves no half-made account
    assert_eq!(
        h.err_code(
            "profile_restore_from_backup",
            json!({"displayName": "Copy", "input": target_args("src", "empty")})
        ),
        "remote"
    );
    assert_eq!(
        h.ok("profile_list", json!({}))["profiles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // the real thing: no vault is unlocked here, and the new account is selected but locked
    let info = h.ok(
        "profile_restore_from_backup",
        json!({"displayName": "Copy", "input": target_args("src", "bkt")}),
    );
    assert_eq!(info["displayName"], "Copy");
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["profile"]["id"], info["id"]);
    assert_eq!(st["unlocked"], false);
    h.ok("vault_unlock", json!({"password": PW}));
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 2);
    // the target marked as shared came back with its credentials
    let targets = h.ok("backup_list", json!({}));
    assert_eq!(targets[0]["name"], "Home bucket");
    assert_eq!(targets[0]["master"], true);
    assert_eq!(targets[0]["hasSecret"], true);
    assert_eq!(
        h.ok(
            "backup_reveal_secret",
            json!({"id": targets[0]["id"], "password": PW})
        ),
        json!(SECRET)
    );
}

#[test]
fn open_url_validates_and_hands_only_safe_links_to_the_opener() {
    let h = harness();
    // no account needed
    h.ok(
        "open_url",
        json!({"url": "https://github.com/login?next=%2F"}),
    );
    h.ok("open_url", json!({"url": "http://localhost:8080/a%20b"}));
    assert_eq!(
        h.opener.urls(),
        [
            "https://github.com/login?next=%2F",
            "http://localhost:8080/a%20b"
        ]
    );
    h.opener.clear();
    for bad in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "file:///etc/passwd",
        "data:text/html,<script>alert(1)</script>",
        "ftp://example.com/",
        "mailto:a@b.c",
        "https://user:pass@example.com/",
        "https://user@example.com/",
        "https://exam\nple.com/",
        "https://example.com/\nSet-Cookie: x",
        "https://example.com/\tx",
        "https://exa mple.com/",
        "//example.com/",
        "example.com",
        "",
        "   ",
    ] {
        assert_eq!(
            h.err_code("open_url", json!({"url": bad})),
            "invalid_input",
            "{bad:?}"
        );
    }
    let too_long = format!("https://example.com/{}", "a".repeat(2100));
    assert_eq!(
        h.err_code("open_url", json!({"url": too_long})),
        "invalid_input"
    );
    assert!(
        h.opener.urls().is_empty(),
        "nothing rejected reached the opener"
    );
}

// ---------------------------------------------------------------------------- Q0: sync triggers

/// An account with one master target and queued work that nothing has sent yet (no worker thread
/// runs in these tests, so the queue only moves when a flush does it).
fn core_with_queued_work() -> (
    tempfile::TempDir,
    Arc<MemFactory>,
    Arc<AppCore>,
    String, // the id of the item whose upload is queued
) {
    let dir = tempfile::tempdir().unwrap();
    let mem = MemFactory::new();
    let cfg = SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: Duration::from_secs(600),
        purge_grace: std::time::Duration::ZERO,
    };
    let core = AppCore::open(dir.path(), cfg, Arc::new(TestPicker::default()))
        .unwrap()
        .with_backup_runtime(BackupRuntime::new(
            mem.clone(),
            Timing {
                background: false,
                ..Timing::default()
            },
        ));
    core.profile_create("Personal", PW.into()).unwrap();
    let mut input: BackupTargetInput = serde_json::from_value(target_args("Home", "bkt")).unwrap();
    input.master = true;
    core.backup_add(input).unwrap();
    let draft: ItemDraft = serde_json::from_value(draft("Bank")).unwrap();
    let item = core.item_create(draft).unwrap();
    (dir, mem, Arc::new(core), item.id)
}

#[test]
fn sync_reconnect_answers_at_once_with_or_without_an_open_vault() {
    let h = harness();
    assert_eq!(
        h.ok("sync_reconnect", json!({})),
        json!(null),
        "nothing open: a no-op"
    );
    signed_in(&h);
    assert_eq!(
        h.ok("sync_reconnect", json!({})),
        json!(null),
        "no backup target: nothing to do, no error"
    );
    h.ok("vault_lock", json!({}));
    assert_eq!(
        h.ok("sync_reconnect", json!({})),
        json!(null),
        "locked: a no-op"
    );
}

#[test]
fn sync_flush_works_over_real_ipc() {
    let h = harness();
    // nothing open: complete, and it needs no account
    assert_eq!(h.ok("sync_flush", json!({})), json!({"complete": true}));
    signed_in(&h);
    let mut master = target_args("Home", "bkt");
    master["master"] = json!(true);
    h.ok("backup_add", json!({"input": master}));
    let item = h.ok("item_create", json!({"draft": draft("Bank")}));
    let id = item["id"].as_str().unwrap();
    let bucket = h.mem.bucket("bkt");
    assert!(
        !bucket.keys().iter().any(|k| k.contains(id)),
        "queued, not yet sent"
    );
    // the flush sends it (the same bounded flush the exit hook uses) and says it is complete
    assert_eq!(h.ok("sync_flush", json!({})), json!({"complete": true}));
    assert!(bucket.keys().iter().any(|k| k.contains(id)));
    // a second flush with an empty queue
    let calls = bucket.total_calls();
    assert_eq!(h.ok("sync_flush", json!({})), json!({"complete": true}));
    assert_eq!(bucket.total_calls(), calls, "nothing queued, nothing sent");
    // an unreachable service: not complete, no error, and the work stays queued
    let second = h.ok("item_create", json!({"draft": draft("Mail")}));
    bucket.fail_all(
        None,
        crate::storage::backup::remote::RemoteError::Unreachable(
            crate::storage::backup::remote::NetKind::Timeout,
        ),
    );
    assert_eq!(h.ok("sync_flush", json!({})), json!({"complete": false}));
    bucket.clear_faults();
    assert_eq!(h.ok("sync_flush", json!({})), json!({"complete": true}));
    assert!(bucket
        .keys()
        .iter()
        .any(|k| k.contains(second["id"].as_str().unwrap())));
    // locked: a no-op
    h.ok("vault_lock", json!({}));
    assert_eq!(h.ok("sync_flush", json!({})), json!({"complete": true}));
}

#[test]
fn a_close_request_with_nothing_queued_is_not_delayed() {
    let gate = ExitGate::default();
    // no account, no vault
    let dir = tempfile::tempdir().unwrap();
    let bare = AppCore::open(
        dir.path(),
        SessionConfig::default(),
        Arc::new(TestPicker::default()),
    )
    .unwrap();
    let start = std::time::Instant::now();
    assert_eq!(gate.on_close_request(&bare), CloseDecision::Close);
    assert!(!gate.is_flushing());

    // an unlocked account whose queue is empty
    let (_dir, _mem, core, _id) = core_with_queued_work();
    assert!(core.flush_for_exit(Duration::from_secs(5)));
    assert_eq!(gate.on_close_request(&core), CloseDecision::Close);
    assert!(!gate.is_flushing());
    // a locked vault holds nothing a flush could send
    core.item_create(serde_json::from_value(draft("More")).unwrap())
        .unwrap();
    core.lock();
    assert_eq!(gate.on_close_request(&core), CloseDecision::Close);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "the decisions are instant; the time is the account set-up"
    );
}

#[test]
fn a_close_request_with_queued_work_flushes_once_then_locks_and_closes() {
    let (_dir, mem, core, id) = core_with_queued_work();
    let gate = Arc::new(ExitGate::default());
    assert!(core.has_pending_sync());
    assert_eq!(gate.on_close_request(&core), CloseDecision::Flush);
    assert!(gate.is_flushing());
    // the person presses close again while the flush runs: the window closes at once
    assert_eq!(gate.on_close_request(&core), CloseDecision::Close);

    let (tx, rx) = std::sync::mpsc::channel();
    flush_then(Arc::clone(&core), Arc::clone(&gate), move || {
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(20))
        .expect("the window is closed after the flush");
    assert!(
        mem.bucket("bkt").keys().iter().any(|k| k.contains(&id)),
        "the queued item was sent before closing"
    );
    assert!(!gate.is_flushing(), "the gate is free again");
    let status = core.status().unwrap();
    assert!(!status.unlocked, "the vault is locked, the keys are gone");
}

#[test]
fn a_close_request_during_an_unreachable_flush_still_closes_by_its_deadline() {
    let (_dir, mem, core, _id) = core_with_queued_work();
    mem.bucket("bkt").fail_all(
        None,
        crate::storage::backup::remote::RemoteError::Unreachable(
            crate::storage::backup::remote::NetKind::Timeout,
        ),
    );
    let gate = Arc::new(ExitGate::default());
    assert_eq!(gate.on_close_request(&core), CloseDecision::Flush);
    let (tx, rx) = std::sync::mpsc::channel();
    let start = std::time::Instant::now();
    flush_then(Arc::clone(&core), Arc::clone(&gate), move || {
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(20)).unwrap();
    assert!(start.elapsed() < EXIT_FLUSH + Duration::from_secs(2));
    assert!(!core.status().unwrap().unlocked);
    // the queue is durable: unlocking again still holds the work
    core.unlock(PW.into()).unwrap();
    assert!(core.has_pending_sync());
}

#[test]
fn another_devices_change_is_flagged_reviewed_and_counted_over_real_ipc() {
    let h = harness();
    // status fields exist before anything is open
    let st = h.ok("vault_status", json!({}));
    assert_eq!(
        (st["syncRevision"].as_u64(), st["reconciling"].as_bool()),
        (Some(0), Some(false))
    );
    assert_eq!(st["remoteDamaged"], 0);
    assert_eq!(
        h.err_code("item_review_other_device", json!({"id": "x"})),
        "locked"
    );

    let first = signed_in(&h);
    let mut master = target_args("Home bucket", "bkt");
    master["master"] = json!(true);
    h.ok("backup_add", json!({"input": master}));
    let item = h.ok("item_create", json!({"draft": draft("Shared")}));
    let id = item["id"].as_str().unwrap().to_string();
    assert_eq!(item_view(&h, &id)["changedElsewhere"], false);
    h.ok("backup_run", json!({}));
    h.ok("vault_lock", json!({}));

    // the other device: restored from the bucket, edits the item, uploads it
    h.ok(
        "profile_restore_from_backup",
        json!({"displayName": "Copy", "input": target_args("src", "bkt")}),
    );
    h.ok("vault_unlock", json!({"password": PW}));
    edit_password_over_ipc(&h, &id, "from-the-copy");
    h.ok("backup_run", json!({}));

    // this device edits the same item without having pulled, then backs up: its own state stays,
    // the other one is kept as a version "from another device"
    h.ok("profile_select", json!({"id": first["id"]}));
    h.ok("vault_unlock", json!({"password": PW}));
    edit_password_over_ipc(&h, &id, "from-the-original");
    h.ok("backup_run", json!({}));

    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["syncRevision"], 1);
    assert_eq!(st["reconciling"], false);
    assert_eq!(st["remoteDamaged"], 0);
    assert_eq!(item_view(&h, &id)["changedElsewhere"], true);
    let history = h.ok("history_list", json!({"id": id}));
    let entries = history.as_array().unwrap();
    assert!(
        entries.iter().all(|e| e["otherDevice"].is_boolean()),
        "{history}"
    );
    assert_eq!(
        entries.iter().filter(|e| e["otherDevice"] == true).count(),
        1,
        "{history}"
    );
    let copy = entries.iter().find(|e| e["otherDevice"] == true).unwrap();
    let copy_content = h.ok(
        "history_get",
        json!({"id": id, "version": copy["version"], "password": PW}),
    );
    assert_eq!(copy_content["fields"]["password"], "from-the-copy");

    // "Got it": the flag goes, the copy stays
    assert_eq!(
        h.ok("item_review_other_device", json!({"id": id})),
        Value::Null
    );
    assert_eq!(item_view(&h, &id)["changedElsewhere"], false);
    assert_eq!(
        h.ok("history_list", json!({"id": id}))
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["otherDevice"] == true)
            .count(),
        1
    );
    assert_eq!(
        h.err_code("item_review_other_device", json!({"id": "not-an-id"})),
        "invalid_input"
    );
    assert_eq!(
        h.err_code(
            "item_review_other_device",
            json!({"id": "00000000-0000-4000-8000-000000000000"})
        ),
        "not_found"
    );
    // snake_case argument names are not part of the contract
    assert!(h
        .call("item_review_other_device", json!({"item_id": id}))
        .is_err());
}

fn item_view(h: &Harness, id: &str) -> Value {
    h.ok("item_get", json!({"id": id}))
}

fn edit_password_over_ipc(h: &Harness, id: &str, password: &str) {
    let begin = h.ok("item_begin_edit", json!({"id": id, "password": PW}));
    let mut edited = begin["draft"].clone();
    edited["fields"]["password"] = json!(password);
    h.ok(
        "item_update",
        json!({
            "id": id,
            "token": begin["token"],
            "expectedVersion": begin["version"],
            "draft": edited
        }),
    );
}
mod device_ipc;
mod folder_ipc;
mod gate_ipc;
mod pin_ipc;
