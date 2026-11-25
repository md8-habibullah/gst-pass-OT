//! The encrypted-drive commands over Tauri's mock runtime: real IPC dispatch, the argument and
//! result shapes the frontend uses (`src/backend/tauri.ts`), and the raw-bytes answer of
//! `file_read`.

use super::*;
use crate::crypto::KdfParams;
use crate::storage::files::TestPicker;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
use tauri::test::{
    get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

const PW: &str = "correct horse battery staple";

struct Harness {
    _dir: tempfile::TempDir,
    src: tempfile::TempDir,
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
    let core = AppCore::open(dir.path(), cfg, picker.clone()).unwrap();
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
        src: tempfile::tempdir().unwrap(),
        picker,
        _app: app,
        window,
    }
}

impl Harness {
    fn raw(&self, cmd: &str, args: Value) -> Result<InvokeResponseBody, Value> {
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
    }

    fn call(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        self.raw(cmd, args)
            .map(|body| body.deserialize::<Value>().unwrap())
    }

    fn ok(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args)
            .unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
    }

    fn err(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args).expect_err("expected an error")
    }

    fn file(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = self.src.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// Account created and unlocked.
    fn start(&self) {
        self.ok(
            "profile_create",
            json!({"displayName": "Personal", "password": PW}),
        );
    }
}

fn note() -> Value {
    json!({
        "type": "secureNote", "title": "Passport", "favorite": false, "tags": [], "folder": null,
        "fields": {"body": "text"}, "customFields": []
    })
}

#[test]
fn the_drive_contract_works_over_real_ipc() {
    let h = harness();
    // locked before anything exists
    assert_eq!(
        h.err("file_list", json!({"itemId": null}))["code"],
        "locked"
    );
    assert_eq!(h.err("file_add", json!({"itemId": null}))["code"], "locked");
    h.start();

    // --- add to the standalone section; the picker is native, no path crosses IPC
    let a = h.file("Scan.PDF", b"%PDF-fake");
    let b = h.file("notes", b"plain");
    h.picker.queue_files(vec![a, b]);
    let added = h.ok("file_add", json!({"itemId": null}));
    let added = added.as_array().unwrap();
    assert_eq!(added.len(), 2);
    let first = &added[0];
    let mut keys: Vec<_> = first.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["addedAt", "id", "itemId", "mime", "name", "size"]);
    assert_eq!(first["name"], "Scan.PDF");
    assert_eq!(first["size"], 9);
    assert_eq!(first["mime"], "application/pdf");
    assert_eq!(first["itemId"], Value::Null);
    assert!(first["addedAt"].as_u64().unwrap() > 1_700_000_000_000);
    assert_eq!(added[1]["mime"], Value::Null, "an unknown type is null");
    let id = first["id"].as_str().unwrap().to_string();

    // --- list, with the argument spelled the way the frontend sends it
    let listed = h.ok("file_list", json!({"itemId": null}));
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(
        h.ok("file_list", json!({})).as_array().unwrap().len(),
        2,
        "a missing itemId means the standalone section"
    );

    // --- attachments of an item show in item_get with the AttachmentView shape
    let item = h.ok("item_create", json!({"draft": note()}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.picker.queue_files(vec![h.file("pic.png", &[1, 2, 3])]);
    let att = h.ok("file_add", json!({"itemId": item}));
    assert_eq!(att[0]["itemId"], json!(item));
    let view = h.ok("item_get", json!({"id": item}));
    assert_eq!(
        view["attachments"],
        json!([{"id": att[0]["id"], "name": "pic.png", "mime": "image/png", "size": 3}])
    );
    assert_eq!(
        h.ok("file_list", json!({"itemId": item}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.ok("file_list", json!({"itemId": null}))
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // --- rename
    let renamed = h.ok("file_rename", json!({"id": id, "name": "Scan renamed.txt"}));
    assert_eq!(renamed["name"], "Scan renamed.txt");
    assert_eq!(renamed["mime"], "text/plain");
    assert_eq!(renamed["id"], json!(id));
    assert_eq!(
        h.err("file_rename", json!({"id": id, "name": "a/b"}))["code"],
        "invalid_input"
    );
    assert_eq!(
        h.err("file_rename", json!({"id": id, "name": " "}))["message"],
        "Use 1 to 200 characters for the file name."
    );
    assert!(
        h.call("file_rename", json!({"id": id})).is_err(),
        "name is required"
    );

    // --- save a copy: re-prompt, native dialog, { path } or null
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("copy.txt");
    h.picker.set_save(Some(dest.clone()));
    assert_eq!(
        h.err(
            "file_save_copy",
            json!({"id": id, "password": "wrong wrong wrong"})
        )["code"],
        "wrong_password"
    );
    assert!(!dest.exists());
    let saved = h.ok("file_save_copy", json!({"id": id, "password": PW}));
    assert_eq!(saved, json!({"path": dest.to_string_lossy()}));
    assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-fake");
    h.picker.set_save(None);
    assert_eq!(
        h.ok("file_save_copy", json!({"id": id, "password": PW})),
        Value::Null
    );

    // --- delete: re-prompt
    assert_eq!(
        h.err(
            "file_delete",
            json!({"id": id, "password": "nope nope nope"})
        )["code"],
        "wrong_password"
    );
    assert_eq!(
        h.ok("file_delete", json!({"id": id, "password": PW})),
        Value::Null
    );
    assert_eq!(
        h.ok("file_list", json!({"itemId": null}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.err("file_delete", json!({"id": id, "password": PW}))["code"],
        "not_found"
    );

    // --- cancelled picker
    assert_eq!(h.ok("file_add", json!({"itemId": null})), Value::Null);

    // --- lock again: every file command answers locked
    h.ok("vault_lock", json!({}));
    for (cmd, args) in [
        ("file_list", json!({"itemId": null})),
        ("file_add", json!({"itemId": null})),
        ("file_rename", json!({"id": id, "name": "x"})),
        ("file_delete", json!({"id": id, "password": PW})),
        ("file_read", json!({"id": id})),
        ("file_save_copy", json!({"id": id, "password": PW})),
    ] {
        assert_eq!(h.err(cmd, args)["code"], "locked", "{cmd}");
    }
}

#[test]
fn file_read_answers_with_raw_bytes_not_json() {
    let h = harness();
    h.start();
    // 2.5 MiB with every byte value: no base64, no JSON array, exactly the file's bytes
    let content: Vec<u8> = (0..2_621_440u32).map(|i| (i % 251) as u8).collect();
    h.picker.queue_files(vec![h.file("blob.bin", &content)]);
    let id = h.ok("file_add", json!({"itemId": null}))[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    match h.raw("file_read", json!({"id": id})).unwrap() {
        InvokeResponseBody::Raw(bytes) => assert!(bytes == content, "the plaintext, byte for byte"),
        InvokeResponseBody::Json(_) => panic!("file_read must not answer with JSON"),
    }
    // an empty file is an empty raw body
    h.picker.queue_files(vec![h.file("empty", b"")]);
    let empty = h.ok("file_add", json!({"itemId": null}))[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    match h.raw("file_read", json!({"id": empty})).unwrap() {
        InvokeResponseBody::Raw(bytes) => assert!(bytes.is_empty()),
        InvokeResponseBody::Json(_) => panic!("file_read must not answer with JSON"),
    }
    // errors stay the normal JSON { code, message }
    let e = h.raw(
        "file_read",
        json!({"id": crate::storage::uuid::new_v4().unwrap()}),
    );
    assert_eq!(e.unwrap_err()["code"], "not_found");
    assert_eq!(
        h.err("file_read", json!({"id": "nope"}))["code"],
        "invalid_input"
    );
}

#[test]
fn the_size_limit_message_reaches_the_webview_unchanged() {
    let h = harness();
    h.start();
    let big = h.src.path().join("Huge Video.mp4");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(52_428_801)
        .unwrap();
    h.picker.queue_files(vec![h.file("ok.txt", b"ok"), big]);
    let e = h.err("file_add", json!({"itemId": null}));
    assert_eq!(e["code"], "invalid_input");
    assert_eq!(e["message"], "Huge Video.mp4 is larger than 50 MB.");
    assert_eq!(h.ok("file_list", json!({"itemId": null})), json!([]));
}

#[test]
fn damaged_data_gives_a_fixed_sentence_over_ipc() {
    let h = harness();
    h.start();
    h.picker.queue_files(vec![h.file("a.bin", &[7u8; 5000])]);
    let id = h.ok("file_add", json!({"itemId": null}))[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // flip a byte in the stored blob
    let blob = std::fs::read_dir(h._dir.path().join("vaults"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("attachments").join(format!("{id}.bin")))
        .find(|p| p.is_file())
        .unwrap();
    let mut bytes = std::fs::read(&blob).unwrap();
    let last = bytes.len() - 3;
    bytes[last] ^= 0x40;
    std::fs::write(&blob, bytes).unwrap();
    let e = h.err("file_read", json!({"id": id}));
    assert_eq!(e["code"], "crypto");
    assert_eq!(
        e["message"],
        "A stored file failed its integrity check: the data is damaged or was modified."
    );
}

#[test]
fn screen_secure_is_accepted_everywhere_and_does_nothing_off_android() {
    let h = harness();
    // needs no vault and no account
    assert_eq!(h.ok("screen_secure", json!({"on": true})), Value::Null);
    assert_eq!(h.ok("screen_secure", json!({"on": false})), Value::Null);
    assert!(
        h.call("screen_secure", json!({})).is_err(),
        "`on` is required"
    );
}
