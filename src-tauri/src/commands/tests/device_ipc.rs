//! The device-settings and demo-vault commands over real IPC dispatch: the argument and result shapes
//! `src/backend/tauri.ts` uses (camelCase), what the closed gate lets through, and that the demo
//! vault's id never reaches the screen.

use super::*;

const PIN: &str = "2468";

#[test]
fn the_defaults_a_new_install_reports() {
    let h = harness();
    assert_eq!(
        h.ok("device_get", json!({})),
        json!({"timeZone": null, "setupDone": false, "timeCheck": true})
    );
}

#[test]
fn the_zone_is_saved_through_ipc_and_a_malformed_name_is_refused() {
    let h = harness();
    let d = h.ok("device_set_time_zone", json!({"zone": "Asia/Dhaka"}));
    assert_eq!(d["timeZone"], "Asia/Dhaka");
    assert_eq!(h.ok("device_get", json!({}))["timeZone"], "Asia/Dhaka");
    for bad in ["../etc", "", "Not A Zone!"] {
        assert_eq!(
            h.err_code("device_set_time_zone", json!({"zone": bad})),
            "invalid_input",
            "{bad:?}"
        );
    }
    assert_eq!(h.ok("device_get", json!({}))["timeZone"], "Asia/Dhaka");
    let d = h.ok("device_finish_setup", json!({}));
    assert_eq!(d["setupDone"], true);
}

#[test]
fn the_demo_is_made_once_for_an_empty_app_and_its_id_stays_in_the_core() {
    let h = harness();
    let p = h.ok("demo_spawn", json!({}));
    assert_eq!(p["displayName"], "Hexis123");
    assert_eq!(p["demo"], true);
    let id = p["id"].as_str().unwrap().to_string();

    // a vault exists now: a second demo is refused
    assert_eq!(h.err_code("demo_spawn", json!({})), "invalid_input");
    // the list marks it, and nothing the screen can read names its id as the demo's
    let list = h.ok("profile_list", json!({}));
    assert_eq!(list["profiles"][0]["demo"], true);
    let device = h.ok("device_get", json!({})).to_string();
    assert!(!device.contains(&id), "{device}");
    // the demo opens with no typed password, after a lock
    h.ok("vault_lock", json!({}));
    assert_eq!(h.ok("vault_status", json!({}))["unlocked"], false);
    h.ok("demo_open", json!({}));
    assert_eq!(h.ok("vault_status", json!({}))["unlocked"], true);
    let items = h.ok("item_list", json!({}));
    let n = items.as_array().unwrap().len();
    assert!((15..=25).contains(&n), "{n} items");
}

#[test]
fn another_vault_never_opens_without_its_password() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok("vault_lock", json!({}));
    // no demo exists, so there is nothing to open for free
    assert_eq!(h.err_code("demo_open", json!({})), "not_found");
    assert_eq!(h.ok("vault_status", json!({}))["unlocked"], false);
}

#[test]
fn the_clock_status_has_the_shape_the_screen_reads() {
    let h = harness();
    let c = h.ok("clock_status", json!({}));
    for key in [
        "verified",
        "enabled",
        "skewMs",
        "checkedAgoMs",
        "sources",
        "problem",
        "nowMs",
    ] {
        assert!(c.get(key).is_some(), "{key} missing from {c}");
    }
    assert!(c["nowMs"].as_u64().unwrap() > 1_700_000_000_000);
}

#[test]
fn a_closed_gate_lets_only_the_read_commands_through() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok(
        "gate_configure",
        json!({"input": {
            "enabled": true, "mode": "pin", "credential": PIN, "reversePanic": true,
            "panicCode": null, "biometric": false, "panicButton": true, "masterPassword": PW
        }}),
    );
    h.ok("gate_close", json!({}));

    // reading the zone and the clock's state works at the gate (the gate screen writes times in the zone)
    assert!(h.call("device_get", json!({})).is_ok());
    assert!(h.call("clock_status", json!({})).is_ok());
    // changing anything, or touching the demo vault, does not
    for (cmd, args) in [
        ("device_set_time_zone", json!({"zone": "Asia/Dhaka"})),
        ("device_finish_setup", json!({})),
        ("device_set_time_check", json!({"on": false})),
        ("demo_spawn", json!({})),
        ("demo_open", json!({})),
    ] {
        assert_eq!(h.err_code(cmd, args), "locked", "{cmd}");
    }
}
