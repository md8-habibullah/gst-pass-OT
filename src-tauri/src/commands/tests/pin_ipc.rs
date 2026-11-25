//! The account PIN over real IPC dispatch: the argument and result shapes `src/backend/tauri.ts` uses, the `limited`
//! vault that follows a PIN unlock, and the inactivity lock wiping a vault that waits behind a PIN.

use super::*;
use std::time::Duration;
use tauri::Manager;

const PIN: &str = "4829-pin";

fn core(h: &Harness) -> Arc<AppCore> {
    h._app.state::<AppState>().inner().core()
}

fn pinned(h: &Harness) -> String {
    let info = h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok("item_create", json!({"draft": draft("A login")}));
    let got = h.ok("account_pin_set", json!({"password": PW, "pin": PIN}));
    assert_eq!(got["hasPin"], true);
    info["id"].as_str().unwrap().to_string()
}

#[test]
fn the_pin_commands_work_over_ipc_with_the_shapes_the_front_end_sends() {
    let h = harness();
    pinned(&h);
    // a status without a PIN in play carries neither flag
    let st = h.ok("vault_status", json!({}));
    assert!(st.get("resumable").is_none() && st.get("limited").is_none());

    // the Lock button parks the vault: locked for every command, `resumable` for the lock screen
    h.ok("vault_lock", json!({}));
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["unlocked"], false);
    assert_eq!(st["resumable"], true);
    assert_eq!(st["profile"]["hasPin"], true);
    assert_eq!(st["profile"]["resumable"], true);
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    assert_eq!(h.err_code("backup_list", json!({})), "locked");
    let list = h.ok("profile_list", json!({}));
    assert_eq!(list["profiles"][0]["resumable"], true);

    // a wrong PIN: its own code, and the message says how many tries are left (and never echoes the PIN)
    let e = h
        .call("vault_unlock_pin", json!({"pin": "9999-no"}))
        .unwrap_err();
    assert_eq!(e["code"], "wrong_pin");
    assert!(e["message"].as_str().unwrap().contains("Tries left: 4"));
    assert!(!e.to_string().contains("9999-no"));

    h.ok("vault_unlock_pin", json!({"pin": PIN}));
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["unlocked"], true);
    assert_eq!(st["limited"], true);
    assert!(st.get("resumable").is_none());
    let items = h.ok("item_list", json!({}));
    let id = items[0]["id"].as_str().unwrap().to_string();

    // limited: secrets and settings need the master password, enforced in the core
    assert_eq!(
        h.err_code("item_reveal", json!({"id": id, "password": null})),
        "password_required"
    );
    assert_eq!(
        h.err_code("item_begin_edit", json!({"id": id, "password": null})),
        "password_required"
    );
    assert_eq!(
        h.err_code("totp_now", json!({"id": id, "password": null})),
        "password_required"
    );
    assert_eq!(
        h.err_code("item_clone", json!({"id": id, "password": null})),
        "password_required"
    );
    // where the vault is copied to, and how the app locks itself, are protections
    assert_eq!(
        h.err_code("backup_add", json!({"input": target_args("Home", "bkt")})),
        "master_required"
    );
    assert_eq!(
        h.err_code("backup_remove", json!({"id": "nope"})),
        "master_required"
    );
    assert_eq!(
        h.err_code("backup_set_master", json!({"id": "nope"})),
        "master_required"
    );
    assert_eq!(
        h.err_code(
            "app_settings_set",
            json!({"input": {"lockAfterMinutes": 30, "lockAfterLeaveSeconds": 300}})
        ),
        "master_required"
    );
    // a secret is only ever shown with the master password in the call itself
    assert_eq!(
        h.err_code(
            "item_reveal",
            json!({"id": id, "password": "wrong password!"})
        ),
        "wrong_password"
    );
    assert!(
        h.ok("item_reveal", json!({"id": id, "password": PW}))["fields"]["password"].is_string()
    );

    // typing the master password lifts the limit for this open vault
    assert_eq!(
        h.err_code("vault_elevate", json!({"password": "wrong password!"})),
        "wrong_password"
    );
    assert_eq!(h.ok("vault_status", json!({}))["limited"], true);
    h.ok("vault_elevate", json!({"password": PW}));
    assert!(h.ok("vault_status", json!({})).get("limited").is_none());
    assert!(
        h.ok("item_reveal", json!({"id": id, "password": null}))["fields"]["password"].is_string()
    );
    h.ok(
        "app_settings_set",
        json!({"input": {"lockAfterMinutes": 30, "lockAfterLeaveSeconds": 300}}),
    );
}

#[test]
fn log_out_wipes_the_vault_behind_the_pin_and_the_pin_alone_opens_nothing_after_it() {
    let h = harness();
    pinned(&h);
    h.ok("vault_lock", json!({}));
    assert_eq!(h.ok("vault_status", json!({}))["resumable"], true);
    h.ok("vault_lock_all", json!({}));
    let st = h.ok("vault_status", json!({}));
    assert_eq!(st["unlocked"], false);
    assert!(
        st.get("resumable").is_none(),
        "nothing waits behind a PIN any more"
    );
    assert_eq!(
        h.err_code("vault_unlock_pin", json!({"pin": PIN})),
        "locked"
    );
    assert_eq!(
        h.err_code("vault_unlock", json!({"password": "wrong password!"})),
        "wrong_password"
    );
    h.ok("vault_unlock", json!({"password": PW}));
    assert!(h.ok("vault_status", json!({})).get("limited").is_none());
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 1);
    // the saved account is still listed and keeps its PIN
    let list = h.ok("profile_list", json!({}));
    assert_eq!(list["profiles"][0]["hasPin"], true);
    assert!(list["profiles"][0].get("resumable").is_none());
}

#[test]
fn the_pin_never_reaches_a_file_and_pin_changes_need_the_master_password() {
    let h = harness();
    pinned(&h);
    assert_eq!(
        h.err_code(
            "account_pin_set",
            json!({"password": "wrong password!", "pin": "7777-new"})
        ),
        "wrong_password"
    );
    assert_eq!(
        h.err_code("account_pin_set", json!({"password": PW, "pin": "abc"})),
        "invalid_input"
    );
    assert_eq!(
        h.err_code("account_pin_remove", json!({"password": "wrong password!"})),
        "wrong_password"
    );
    for entry in walk(h.dir.path()) {
        let bytes = std::fs::read(&entry).unwrap();
        assert!(
            !bytes.windows(PIN.len()).any(|w| w == PIN.as_bytes()),
            "{entry:?} holds the PIN"
        );
    }
    let got = h.ok("account_pin_remove", json!({"password": PW}));
    assert!(got.get("hasPin").is_none());
    h.ok("vault_lock", json!({}));
    assert!(
        h.ok("vault_status", json!({})).get("resumable").is_none(),
        "without a PIN, Lock wipes"
    );
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn the_inactivity_lock_wipes_a_vault_that_waits_behind_a_pin() {
    let h = harness();
    core(&h).gate().set_idle_unit(Duration::from_millis(400));
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok("account_pin_set", json!({"password": PW, "pin": PIN}));
    h.ok(
        "app_settings_set",
        json!({"input": {"lockAfterMinutes": 1, "lockAfterLeaveSeconds": 300}}),
    );
    h.ok("vault_lock", json!({}));
    assert_eq!(h.ok("vault_status", json!({}))["resumable"], true);
    // polling does not count as activity: the timer that Lock started runs out and takes the parked vault with it
    let start = std::time::Instant::now();
    while h.ok("vault_status", json!({})).get("resumable").is_some() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the parked vault was never wiped"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        h.err_code("vault_unlock_pin", json!({"pin": PIN})),
        "locked"
    );
    h.ok("vault_unlock", json!({"password": PW}));
}
