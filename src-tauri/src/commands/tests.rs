//! End-to-end tests through Tauri's mock runtime: real IPC dispatch, real argument
//! deserialization (camelCase from JavaScript), real serialization of results and errors.
//! This is what the frontend actually sees.

use super::*;
use crate::crypto::KdfParams;
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
    _dir: tempfile::TempDir,
    _app: tauri::App<MockRuntime>,
    window: WebviewWindow<MockRuntime>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let cfg = SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: Duration::from_secs(600),
    };
    let state = AppState::new(Session::new(dir.path().join("vault.db"), cfg));
    let app = mock_builder()
        .manage(state)
        .invoke_handler(invoke_handler())
        .build(mock_context(noop_assets()))
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    Harness {
        _dir: dir,
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
    assert_eq!(
        st["capabilities"],
        json!({"quickUnlock": false, "biometric": false})
    );
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    assert_eq!(
        h.err_code("vault_unlock", json!({"password": PW})),
        "no_vault"
    );
    assert_eq!(
        h.err_code("vault_create", json!({"password": "short"})),
        "invalid_input"
    );
    h.ok("vault_create", json!({"password": PW}));
    assert_eq!(h.ok("vault_status", json!({}))["unlocked"], true);
    assert_eq!(
        h.err_code("vault_create", json!({"password": PW})),
        "vault_exists"
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

    // --- trash: restricted view, restore, purge with double confirmation
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
            json!({"id": id, "password": PW, "confirmation": "nope"})
        ),
        "confirmation_mismatch"
    );
    assert_eq!(
        h.err_code(
            "item_purge",
            json!({"id": id, "password": "wrong wrong", "confirmation": "CONFIRM"})
        ),
        "wrong_password"
    );
    h.ok(
        "item_purge",
        json!({"id": id, "password": PW, "confirmation": "CONFIRM"}),
    );
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
    h.ok("vault_create", json!({"password": PW}));
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
