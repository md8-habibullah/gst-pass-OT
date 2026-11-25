//! The passkey provider's switch over real IPC dispatch (`src/backend/tauri.ts`): on every system but an Android 14 phone
//! it is "not supported", and the switch needs a vault that the master password opened.

use super::*;

const PIN: &str = "4821-xy";

#[test]
fn off_android_the_provider_is_not_supported_and_cannot_be_switched_on() {
    let h = harness();
    // the answer does not need an open vault: Settings asks before anyone unlocks anything
    assert_eq!(
        h.ok("passkey_provider_status", json!({})),
        json!({"supported": false, "enabled": false})
    );
    assert_eq!(
        h.ok("passkey_provider_open_settings", json!({})),
        json!(null)
    );

    // locked or without a vault: the switch is refused before it can say anything about the platform
    assert_eq!(
        h.err_code("passkey_provider_set", json!({"on": true})),
        "locked"
    );

    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    // opened with the master password: the platform is what refuses, in plain words
    let e = h
        .call("passkey_provider_set", json!({"on": true}))
        .unwrap_err();
    assert!(e["message"].as_str().unwrap().contains("Android"), "{e}");
    assert_eq!(h.ok("passkey_provider_status", json!({}))["enabled"], false);
}

#[test]
fn a_vault_opened_with_the_pin_may_not_change_what_the_system_can_ask() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok("account_pin_set", json!({"password": PW, "pin": PIN}));
    h.ok("vault_lock", json!({}));
    h.ok("vault_unlock_pin", json!({"pin": PIN}));
    assert_eq!(h.ok("vault_status", json!({}))["limited"], true);
    assert_eq!(
        h.err_code("passkey_provider_set", json!({"on": true})),
        "master_required"
    );
    assert_eq!(
        h.err_code("passkey_provider_set", json!({"on": false})),
        "master_required"
    );
    // typing the master password lifts the limit; the platform answer is then the only thing left to refuse
    h.ok("vault_elevate", json!({"password": PW}));
    let e = h
        .call("passkey_provider_set", json!({"on": true}))
        .unwrap_err();
    assert_ne!(e["code"], "master_required", "{e}");
}

#[test]
fn the_website_check_switch_is_a_device_setting_the_core_keeps() {
    let h = harness();
    // needs no vault: it is about this install, like the theme
    assert_eq!(h.ok("device_get", json!({}))["passkeyAppCheck"], true);
    assert_eq!(
        h.ok("device_set_passkey_app_check", json!({"on": false}))["passkeyAppCheck"],
        false
    );
    assert_eq!(h.ok("device_get", json!({}))["passkeyAppCheck"], false);
    assert_eq!(
        h.ok("device_set_passkey_app_check", json!({"on": true}))["passkeyAppCheck"],
        true
    );
}
