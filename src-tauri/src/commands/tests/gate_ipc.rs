//! The Stage 1 gate over real IPC dispatch: the argument and result shapes `src/backend/tauri.ts`
//! uses (camelCase), the `locked` refusals while the gate is closed (every registered command is
//! either on the allow-list or refused), the idle lock, the wipe, and the error sentences.

use super::*;
use crate::gate::ALLOWED_WHILE_CLOSED;
use std::collections::BTreeSet;
use tauri::Manager;

const PIN: &str = "2468";

fn core(h: &Harness) -> Arc<AppCore> {
    h._app.state::<AppState>().inner().core()
}

/// The shape `gateConfigure` sends.
fn config(master: &str) -> Value {
    json!({"input": {
        "enabled": true, "mode": "pin", "credential": PIN, "reversePanic": true,
        "panicCode": null, "biometric": false, "panicButton": true, "masterPassword": master
    }})
}

/// An account is created and unlocked through IPC, then the gate is configured and closed.
fn gated(h: &Harness) {
    h.ok(
        "profile_create",
        json!({"displayName": "Ana Secret Name", "password": PW}),
    );
    h.ok("item_create", json!({"draft": draft("A login")}));
    let st = h.ok("gate_configure", config(PW));
    assert_eq!(st["enabled"], true);
    assert_eq!(
        st["open"], true,
        "you are inside the app while configuring it"
    );
    h.ok("gate_close", json!({}));
}

fn files(h: &Harness) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(h.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Every command name inside `generate_handler![ ... ]` of `commands/mod.rs`.
fn registered_commands() -> Vec<String> {
    let src = include_str!("../mod.rs");
    let start =
        src.find("generate_handler![").expect("the handler list") + "generate_handler![".len();
    let end = start + src[start..].find(']').expect("the end of the list");
    src[start..end]
        .split(',')
        .map(|n| n.trim().rsplit("::").next().unwrap_or("").to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

#[test]
fn the_gate_commands_exist_with_the_names_the_front_end_calls() {
    let names: BTreeSet<String> = registered_commands().into_iter().collect();
    for cmd in [
        "gate_status",
        "gate_submit",
        "gate_biometric",
        "gate_panic",
        "gate_close",
        "gate_configure",
        "app_reset",
        "app_settings_get",
        "app_settings_set",
        "activity_ping",
    ] {
        assert!(names.contains(cmd), "{cmd} is not registered");
    }
    assert!(names.len() > 40);
}

#[test]
fn with_no_gate_everything_works_exactly_as_before() {
    let h = harness();
    let st = h.ok("gate_status", json!({}));
    assert_eq!(
        st,
        json!({
            "enabled": false, "open": true, "mode": "pin", "biometricAvailable": false,
            "biometric": false, "panicButton": false, "equation": null, "attemptsLeft": null,
            "streak": 0
        })
    );
    assert_eq!(
        h.ok("app_settings_get", json!({})),
        json!({"lockAfterMinutes": 15})
    );
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    assert_eq!(h.ok("item_list", json!({})), json!([]));
    assert_eq!(h.ok("vault_status", json!({}))["unlocked"], true);
    // nothing refused, nothing to submit to
    assert_eq!(
        h.ok("gate_submit", json!({"input": "anything"})),
        json!({"kind": "opened"})
    );
    assert_eq!(h.err_code("gate_panic", json!({})), "invalid_input");
    assert_eq!(h.err_code("gate_biometric", json!({})), "invalid_input");
    h.ok("activity_ping", json!({}));
}

#[test]
fn configure_over_ipc_validates_in_rust_and_needs_the_master_password() {
    let h = harness();
    // no account yet
    assert_eq!(h.err_code("gate_configure", config(PW)), "locked");
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    assert_eq!(
        h.err_code("gate_configure", config("wrong one")),
        "wrong_password"
    );
    assert!(!files(&h).contains(&"gate.json".to_string()));
    let bad = |mutate: &dyn Fn(&mut Value)| {
        let mut v = config(PW);
        mutate(&mut v["input"]);
        h.call("gate_configure", v).expect_err("refused")
    };
    let e = bad(&|i| i["credential"] = json!("12"));
    assert_eq!(e["code"], "invalid_input");
    assert_eq!(e["message"], "Use at least 4 digits.");
    let e = bad(&|i| i["credential"] = json!("12ab"));
    assert_eq!(e["message"], "A PIN uses digits only.");
    let e = bad(&|i| i["credential"] = json!("1221"));
    assert_eq!(e["code"], "invalid_input");
    let e = bad(&|i| i["panicCode"] = json!("2468"));
    assert_eq!(
        e["message"],
        "The panic code must be different from your passcode."
    );
    // a malformed argument is an error, never a half-applied change
    for mutate in [
        Box::new(|i: &mut Value| i["mode"] = json!("fingerprint")) as Box<dyn Fn(&mut Value)>,
        Box::new(|i| i["enabled"] = json!("yes")),
        Box::new(|i| {
            i.as_object_mut().unwrap().remove("masterPassword");
        }),
        Box::new(|i| i["credential"] = json!(2468)),
    ] {
        let mut v = config(PW);
        mutate(&mut v["input"]);
        assert!(h.call("gate_configure", v).is_err());
    }
    assert!(h.call("gate_configure", json!({"input": null})).is_err());
    assert!(h.call("gate_configure", json!({})).is_err());
    assert!(
        !files(&h).contains(&"gate.json".to_string()),
        "nothing was written"
    );
    // the good one
    let st = h.ok("gate_configure", config(PW));
    assert_eq!(st["enabled"], true);
    assert_eq!(st["mode"], "pin");
    assert_eq!(st["panicButton"], true);
    assert_eq!(st["equation"], Value::Null, "open: no equation");
    // changing it again needs the master password again
    assert_eq!(
        h.err_code("gate_configure", config("wrong one")),
        "wrong_password"
    );
    // keeping the stored credential: credential null, panic code "" removes
    let keep = json!({"input": {
        "enabled": true, "mode": "pin", "credential": null, "reversePanic": false,
        "panicCode": "", "biometric": false, "panicButton": false, "masterPassword": PW
    }});
    let st = h.ok("gate_configure", keep);
    assert_eq!(st["panicButton"], false);
    // turning it off removes the file
    let off = json!({"input": {
        "enabled": false, "mode": "pin", "credential": null, "reversePanic": false,
        "panicCode": null, "biometric": false, "panicButton": false, "masterPassword": PW
    }});
    let st = h.ok("gate_configure", off);
    assert_eq!(st["enabled"], false);
    assert!(!files(&h).contains(&"gate.json".to_string()));
    h.ok("vault_lock", json!({}));
    assert_eq!(h.err_code("gate_configure", config(PW)), "locked");
}

#[test]
fn a_closed_gate_answers_locked_to_every_command_that_is_not_on_the_allow_list() {
    let h = harness();
    gated(&h);
    let allowed: BTreeSet<&str> = ALLOWED_WHILE_CLOSED.iter().copied().collect();
    let mut refused = 0;
    let mut passed = Vec::new();
    for cmd in registered_commands() {
        if allowed.contains(cmd.as_str()) {
            passed.push(cmd);
            continue;
        }
        // arguments do not matter: the guard answers before the command looks at them
        let e = h.call(&cmd, json!({})).expect_err(&cmd);
        assert_eq!(e["code"], "locked", "{cmd}: {e}");
        assert_eq!(e["message"], "The vault is locked.", "{cmd}");
        refused += 1;
    }
    assert!(refused >= 35, "{refused}");
    passed.sort();
    assert_eq!(
        passed,
        [
            "activity_ping",
            "app_reset",
            "app_settings_get",
            "crypto_suite_info",
            "disguise_get",
            "gate_biometric",
            "gate_panic",
            "gate_status",
            "gate_submit",
            "screen_secure",
            "sync_flush",
            "vault_status"
        ]
    );
    // a name that does not exist is refused the same way while closed (deny by default)
    assert_eq!(h.err_code("a_command_added_later", json!({})), "locked");
    // the allowed ones answer
    assert!(h.call("crypto_suite_info", json!({})).is_ok());
    h.ok("app_settings_get", json!({}));
    h.ok("activity_ping", json!({}));
    // while open, an unknown command is just unknown (and not `locked`)
    h.ok("gate_submit", json!({"input": PIN}));
    let unknown = h
        .call("a_command_added_later", json!({}))
        .expect_err("unknown");
    assert_ne!(unknown["code"], "locked");
}

#[test]
fn the_boot_sequence_works_in_both_states_and_reveals_no_account_name() {
    let h = harness();
    gated(&h);
    // boot(): suiteInfo, gateStatus, disguiseGet while closed
    h.ok("crypto_suite_info", json!({}));
    h.ok("disguise_get", json!({}));
    let st = h.ok("gate_status", json!({}));
    assert_eq!(st["enabled"], true);
    assert_eq!(st["open"], false);
    assert!(st["equation"].as_str().is_some_and(|e| !e.is_empty()));
    assert_eq!(st["attemptsLeft"], Value::Null);
    // the UI's poll answers a neutral value
    let vs = h.ok("vault_status", json!({}));
    assert_eq!(
        vs,
        json!({"exists": false, "unlocked": false, "profile": null, "damagedItems": 0,
               "syncRevision": 0, "reconciling": false, "remoteDamaged": 0})
    );
    assert!(!vs.to_string().contains("Ana"));
    // pass the gate: enterAccounts() works now
    assert_eq!(
        h.ok("gate_submit", json!({"input": PIN})),
        json!({"kind": "opened"})
    );
    let list = h.ok("profile_list", json!({}));
    assert_eq!(list["profiles"][0]["displayName"], "Ana Secret Name");
    let vs = h.ok("vault_status", json!({}));
    assert_eq!(vs["exists"], true);
    assert_eq!(vs["unlocked"], false, "Stage 1 never unlocks a vault");
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    assert_eq!(
        h.err_code("vault_unlock", json!({"password": PIN})),
        "wrong_password"
    );
    h.ok("vault_unlock", json!({"password": PW}));
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 1);
    // gate_close: back behind the gate, vault locked, keys gone
    h.ok("gate_close", json!({}));
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    assert_eq!(h.ok("gate_status", json!({}))["open"], false);
}

#[test]
fn submit_over_ipc_follows_the_three_character_rule_and_counts_attempts() {
    let h = harness();
    gated(&h);
    // 1 to 3 characters: maths, never a credential, never counted
    for text in ["", "1", "24", "abc"] {
        let out = h.ok("gate_submit", json!({"input": text}));
        assert_eq!(out["kind"], "math", "{text:?}");
        assert!(out["correct"].is_boolean());
        assert!(out["equation"].is_string() && out["streak"].is_number());
    }
    assert_eq!(h.ok("gate_status", json!({}))["attemptsLeft"], Value::Null);
    // 4 or more: a security attempt
    let out = h.ok("gate_submit", json!({"input": "1111"}));
    assert_eq!(out["kind"], "failed");
    assert_eq!(out["attemptsLeft"], 14);
    assert!(out["equation"].is_string());
    assert_eq!(out.as_object().unwrap().len(), 3);
    assert_eq!(h.ok("gate_status", json!({}))["attemptsLeft"], 14);
    // nothing typed is ever echoed back
    assert!(!out.to_string().contains("1111"));
    let out = h.ok("gate_submit", json!({"input": PIN}));
    assert_eq!(out, json!({"kind": "opened"}));
    assert_eq!(h.ok("gate_status", json!({}))["attemptsLeft"], Value::Null);
    // a wrong argument name or type is an error, not an attempt
    h.ok("gate_close", json!({}));
    assert!(h.call("gate_submit", json!({"text": "1111"})).is_err());
    assert!(h.call("gate_submit", json!({"input": 1111})).is_err());
    assert!(h.call("gate_submit", json!({})).is_err());
    assert_eq!(h.ok("gate_status", json!({}))["attemptsLeft"], Value::Null);
}

#[test]
fn a_fingerprint_is_invalid_input_on_a_desktop_and_counts_nothing() {
    let h = harness();
    gated(&h);
    h.ok("gate_submit", json!({"input": "1111"}));
    assert_eq!(h.err_code("gate_biometric", json!({})), "invalid_input");
    let st = h.ok("gate_status", json!({}));
    assert_eq!(st["biometricAvailable"], false);
    assert_eq!(st["attemptsLeft"], 14, "neither counted nor reset");
    assert_eq!(st["open"], false);
}

#[test]
fn the_reversed_pin_wipes_over_ipc_with_exactly_the_wiped_outcome() {
    let h = harness();
    gated(&h);
    std::fs::write(h.dir.path().join("notes.txt"), b"not Hexis's").unwrap();
    let out = h.ok("gate_submit", json!({"input": "8642"}));
    assert_eq!(out, json!({"kind": "wiped"}));
    assert_eq!(files(&h), ["notes.txt"], "only the unknown file is left");
    // the UI finds a first-run state
    assert_eq!(h.ok("profile_list", json!({}))["profiles"], json!([]));
    let st = h.ok("gate_status", json!({}));
    assert_eq!(st["enabled"], false);
    assert_eq!(st["open"], true);
    assert_eq!(st["attemptsLeft"], Value::Null);
    assert_eq!(h.ok("vault_status", json!({}))["exists"], false);
    assert_eq!(
        h.ok("app_settings_get", json!({})),
        json!({"lockAfterMinutes": 15})
    );
    // and the app starts over
    h.ok(
        "profile_create",
        json!({"displayName": "New", "password": PW}),
    );
}

#[test]
fn the_panic_mark_works_only_while_it_is_on_and_the_gate_is_closed() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    let mut cfg = config(PW);
    cfg["input"]["panicButton"] = json!(false);
    h.ok("gate_configure", cfg);
    h.ok("gate_close", json!({}));
    assert_eq!(
        h.err_code("gate_panic", json!({})),
        "invalid_input",
        "switched off"
    );
    h.ok("gate_submit", json!({"input": PIN}));
    h.ok("vault_unlock", json!({"password": PW}));
    h.ok("gate_configure", config(PW));
    assert_eq!(
        h.err_code("gate_panic", json!({})),
        "invalid_input",
        "the gate is open"
    );
    h.ok("gate_close", json!({}));
    assert_eq!(h.ok("gate_panic", json!({})), json!({"kind": "wiped"}));
    assert_eq!(h.ok("profile_list", json!({}))["profiles"], json!([]));
    assert!(files(&h).is_empty());
}

#[test]
fn the_fifteenth_failure_wipes_over_ipc() {
    let h = harness();
    gated(&h);
    for left in (1..=14).rev() {
        let out = h.ok(
            "gate_submit",
            json!({"input": format!("{:04}", 5000 + left)}),
        );
        assert_eq!(out["kind"], "failed");
        assert_eq!(out["attemptsLeft"], left);
    }
    assert!(files(&h).contains(&"gate.json".to_string()));
    let out = h.ok("gate_submit", json!({"input": "5000"}));
    assert_eq!(out, json!({"kind": "wiped"}));
    assert!(files(&h).is_empty());
    assert!(!h.ok("gate_status", json!({}))["enabled"].as_bool().unwrap());
}

#[test]
fn app_reset_needs_exactly_three_confirmed_steps_and_works_with_the_gate_closed() {
    let h = harness();
    gated(&h);
    for bad in [
        json!(0),
        json!(1),
        json!(2),
        json!(4),
        json!(-3),
        json!(3.5),
        json!("3"),
        json!(null),
        json!(true),
    ] {
        assert_eq!(
            h.err_code("app_reset", json!({"confirmedSteps": bad})),
            "invalid_input",
            "{bad}"
        );
    }
    // a missing argument is an error too, and nothing was wiped by any of them
    assert!(h.call("app_reset", json!({})).is_err());
    assert!(files(&h).contains(&"profiles.json".to_string()));
    assert!(h.ok("gate_status", json!({}))["enabled"].as_bool().unwrap());
    // the gate is closed and `app_reset` is allowed anyway (the UI's "erase this device")
    assert_eq!(h.ok("app_reset", json!({"confirmedSteps": 3})), Value::Null);
    assert!(files(&h).is_empty());
    assert_eq!(h.ok("profile_list", json!({}))["profiles"], json!([]));
    // a second reset is not an error
    h.ok("app_reset", json!({"confirmedSteps": 3}));
}

#[test]
fn the_timeout_setting_has_four_values_and_a_default_over_ipc() {
    let h = harness();
    assert_eq!(
        h.ok("app_settings_get", json!({})),
        json!({"lockAfterMinutes": 15})
    );
    for m in [1, 5, 15, 30] {
        let out = h.ok(
            "app_settings_set",
            json!({"input": {"lockAfterMinutes": m}}),
        );
        assert_eq!(out, json!({"lockAfterMinutes": m}));
        assert_eq!(
            h.ok("app_settings_get", json!({})),
            json!({"lockAfterMinutes": m})
        );
    }
    for bad in [
        json!(0),
        json!(2),
        json!(10),
        json!(45),
        json!(60),
        json!(100000),
    ] {
        assert_eq!(
            h.err_code(
                "app_settings_set",
                json!({"input": {"lockAfterMinutes": bad}})
            ),
            "invalid_input",
            "{bad}"
        );
    }
    // wrong types are errors and change nothing
    for bad in [json!("5"), json!(null), json!(5.5), json!(-5)] {
        assert!(h
            .call(
                "app_settings_set",
                json!({"input": {"lockAfterMinutes": bad}})
            )
            .is_err());
    }
    assert!(h.call("app_settings_set", json!({"input": {}})).is_err());
    assert_eq!(
        h.ok("app_settings_get", json!({})),
        json!({"lockAfterMinutes": 30})
    );
    let text = std::fs::read_to_string(h.dir.path().join("app-settings.json")).unwrap();
    assert!(text.contains("30"));
}

fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_idle_lock_over_ipc_with_and_without_a_gate() {
    // no gate: the vault is locked after the scaled minute, `item_list` says `locked`
    let h = harness();
    core(&h).gate().set_idle_unit(Duration::from_millis(400));
    h.ok(
        "app_settings_set",
        json!({"input": {"lockAfterMinutes": 1}}),
    );
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok("item_create", json!({"draft": draft("A login")}));
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(100));
        h.ok("activity_ping", json!({}));
        assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 1);
    }
    // polling does not count as activity
    wait_until("the idle lock", || {
        let _ = h.call("vault_status", json!({}));
        let _ = h.call("gate_status", json!({}));
        !h.ok("vault_status", json!({}))["unlocked"]
            .as_bool()
            .unwrap()
    });
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    assert!(h.ok("gate_status", json!({}))["open"].as_bool().unwrap());
    h.ok("vault_unlock", json!({"password": PW}));
    assert_eq!(h.ok("item_list", json!({})).as_array().unwrap().len(), 1);

    // with the gate: the gate closes too, the next command answers `locked`, the status is neutral
    let h = harness();
    core(&h).gate().set_idle_unit(Duration::from_millis(400));
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok("gate_configure", config(PW));
    h.ok(
        "app_settings_set",
        json!({"input": {"lockAfterMinutes": 1}}),
    );
    h.ok("item_create", json!({"draft": draft("A login")}));
    wait_until("the gate to close", || {
        !h.ok("gate_status", json!({}))["open"].as_bool().unwrap()
    });
    assert_eq!(h.err_code("item_list", json!({})), "locked");
    assert_eq!(h.err_code("profile_list", json!({})), "locked");
    assert_eq!(h.ok("vault_status", json!({}))["exists"], false);
    // the PIN opens Stage 1 only: the vault is still locked
    assert_eq!(
        h.ok("gate_submit", json!({"input": PIN})),
        json!({"kind": "opened"})
    );
    assert_eq!(h.ok("vault_status", json!({}))["unlocked"], false);
    assert_eq!(h.err_code("item_list", json!({})), "locked");
}

#[test]
fn errors_use_fixed_sentences_and_never_echo_what_was_typed() {
    let h = harness();
    gated(&h);
    let secret = "9876-very-secret-typed-text";
    let out = h.ok("gate_submit", json!({"input": secret}));
    assert!(!out.to_string().contains("secret"));
    h.ok("gate_submit", json!({"input": PIN}));
    h.ok("vault_unlock", json!({"password": PW}));
    let e = h
        .call("gate_configure", config(secret))
        .expect_err("wrong master password");
    assert_eq!(e["code"], "wrong_password");
    assert!(!e.to_string().contains("secret"));
    let mut cfg = config(PW);
    cfg["input"]["credential"] = json!("ab");
    cfg["input"]["mode"] = json!("password");
    let e = h.call("gate_configure", cfg).expect_err("too short");
    assert_eq!(e["message"], "Use at least 4 characters.");
    // the gate file holds no plaintext of anything typed
    let file = std::fs::read_to_string(h.dir.path().join("gate.json")).unwrap();
    assert!(!file.contains(secret) && !file.contains(PW) && !file.contains(PIN));
}
