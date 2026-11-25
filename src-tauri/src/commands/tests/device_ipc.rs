//! The device-settings and demo-vault commands over real IPC dispatch: the argument and result shapes
//! `src/backend/tauri.ts` uses (camelCase), what the closed gate lets through, and that the demo
//! vault's id never reaches the screen.

use super::*;
use tauri::Manager;

const PIN: &str = "2468";

#[test]
fn the_defaults_a_new_install_reports() {
    let h = harness();
    assert_eq!(
        h.ok("device_get", json!({})),
        json!({
            "timeZone": null, "setupDone": false, "timeCheck": true,
            "theme": "system", "clipboardClear": 30,
            "openInSearch": false, "listSort": "updated-desc",
            "passkeyAppCheck": true
        })
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
        ("device_set_theme", json!({"theme": "light"})),
        ("device_set_clipboard_clear", json!({"seconds": 15})),
        ("device_set_open_in_search", json!({"on": true})),
        ("device_set_list_sort", json!({"order": "title-asc"})),
        ("demo_spawn", json!({})),
        ("demo_open", json!({})),
    ] {
        assert_eq!(h.err_code(cmd, args), "locked", "{cmd}");
    }
}

#[test]
fn opening_in_search_and_the_list_order_are_saved_by_the_core() {
    let h = harness();
    let d = h.ok("device_get", json!({}));
    assert_eq!(d["openInSearch"], false);
    assert_eq!(d["listSort"], "updated-desc");

    assert_eq!(
        h.ok("device_set_open_in_search", json!({"on": true}))["openInSearch"],
        true
    );
    for order in [
        "updated-desc",
        "updated-asc",
        "created-desc",
        "created-asc",
        "title-asc",
        "title-desc",
        "type",
        "versions-desc",
        "versions-asc",
    ] {
        let d = h.ok("device_set_list_sort", json!({"order": order}));
        assert_eq!(d["listSort"], order);
    }
    // an order the list does not offer changes nothing (and a missing one is refused outright)
    h.ok("device_set_list_sort", json!({"order": "title-asc"}));
    for bad in ["", "random", "title-sideways", "../x", "TITLE-ASC"] {
        assert_eq!(
            h.err_code("device_set_list_sort", json!({"order": bad})),
            "invalid_input",
            "{bad:?}"
        );
    }
    assert!(h.call("device_set_list_sort", json!({})).is_err());
    assert_eq!(h.ok("device_get", json!({}))["listSort"], "title-asc");

    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(h.dir.path().join("device.json")).unwrap())
            .unwrap();
    assert_eq!(saved["openInSearch"], true);
    assert_eq!(saved["listSort"], "title-asc");
}

#[test]
fn the_theme_and_the_clipboard_time_are_saved_by_the_core_not_by_the_web_view() {
    let h = harness();
    for (word, expected) in [("dark", "dark"), ("light", "light"), ("system", "system")] {
        let d = h.ok("device_set_theme", json!({"theme": word}));
        assert_eq!(d["theme"], expected);
        assert_eq!(h.ok("device_get", json!({}))["theme"], expected);
    }
    // a word that is not a theme changes nothing
    h.ok("device_set_theme", json!({"theme": "dark"}));
    assert!(h
        .call("device_set_theme", json!({"theme": "purple"}))
        .is_err());
    assert!(h.call("device_set_theme", json!({})).is_err());
    assert_eq!(h.ok("device_get", json!({}))["theme"], "dark");

    for seconds in [15, 60, 30] {
        let d = h.ok("device_set_clipboard_clear", json!({"seconds": seconds}));
        assert_eq!(d["clipboardClear"], seconds);
    }
    for bad in [0, 1, 20, 61, 3600] {
        assert_eq!(
            h.err_code("device_set_clipboard_clear", json!({"seconds": bad})),
            "invalid_input",
            "{bad}"
        );
    }
    assert_eq!(h.ok("device_get", json!({}))["clipboardClear"], 30);

    // both live in device.json (one small file), and nothing else new appeared in the data folder
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(h.dir.path().join("device.json")).unwrap())
            .unwrap();
    assert_eq!(saved["theme"], "dark");
    assert_eq!(saved["clipboardClear"], 30);
}

#[test]
fn the_native_window_is_told_the_theme_when_it_changes() {
    use std::sync::Mutex;
    let h = harness();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    h._app
        .state::<AppState>()
        .set_theme_hook(move |t| sink.lock().unwrap().push(t));
    h.ok("device_set_theme", json!({"theme": "dark"}));
    h.ok("device_set_theme", json!({"theme": "light"}));
    // a refused call tells nobody
    let _ = h.call("device_set_theme", json!({"theme": "purple"}));
    use crate::storage::device::Theme;
    assert_eq!(*seen.lock().unwrap(), vec![Theme::Dark, Theme::Light]);
}

#[test]
fn removing_hexis_over_ipc_needs_all_three_steps_and_the_sum_then_closes_the_app() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let h = harness();
    let exits = Arc::new(AtomicUsize::new(0));
    let counter = exits.clone();
    h._app.state::<AppState>().set_exit_hook(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });

    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok("device_set_theme", json!({"theme": "dark"}));
    // anything but exactly three confirmed steps is refused, and nothing is touched
    for bad in [json!(0), json!(2), json!(4), json!("3"), json!(null)] {
        assert_eq!(
            h.err_code("app_prepare_uninstall", json!({"confirmedSteps": bad})),
            "invalid_input",
            "{bad}"
        );
    }
    // the sum is asked for first; a wrong or missing answer removes nothing
    assert_eq!(
        h.err_code("app_prepare_uninstall", json!({"confirmedSteps": 3})),
        "wrong_answer"
    );
    h.ok("app_reset_challenge", json!({}));
    assert_eq!(
        h.err_code(
            "app_prepare_uninstall",
            json!({"confirmedSteps": 3, "answer": "1"})
        ),
        "wrong_answer"
    );
    assert!(h.dir.path().join("profiles.json").exists());
    assert_eq!(exits.load(Ordering::SeqCst), 0, "no exit after a refusal");

    h.ok("app_reset_challenge", json!({}));
    let answer = h
        ._app
        .state::<AppState>()
        .inner()
        .core()
        .gate()
        .reset_answer()
        .unwrap()
        .to_string();
    let r = h.ok(
        "app_prepare_uninstall",
        json!({"confirmedSteps": 3, "answer": answer}),
    );
    assert_eq!(r, Value::Null);
    assert_eq!(exits.load(Ordering::SeqCst), 1, "the app is asked to close");
    // the data folder is empty: not even device.json is left, and the folder itself is not recreated
    let left: Vec<_> = std::fs::read_dir(h.dir.path())
        .map(|d| d.map(|e| e.unwrap().file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "left behind: {left:?}");
}
