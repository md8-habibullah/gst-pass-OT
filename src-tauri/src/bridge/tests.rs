//! The browser bridge against a real core: requests as the socket delivers them, the window's side as the commands drive it.

use super::*;
use crate::crypto::KdfParams;
use crate::passkey::b64;
use crate::passkey::engine::tests::rs_from_der;
use crate::storage::files::NoPicker;
use crate::storage::session::{SessionConfig, EDIT_TOKEN_TTL};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

const PW: &str = "correct horse battery staple";

fn fast() -> SessionConfig {
    SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: EDIT_TOKEN_TTL,
        purge_grace: Duration::ZERO,
    }
}

struct Fx {
    dir: tempfile::TempDir,
    core: Arc<AppCore>,
    bridge: Arc<Bridge>,
}

/// A core with one account, and the bridge switched on in the settings (the socket itself is not opened).
fn fx() -> Fx {
    let f = bare();
    f.core.profile_create("Ana", PW.into()).unwrap();
    f
}

fn bare() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let core =
        Arc::new(AppCore::open(&dir.path().join("data"), fast(), Arc::new(NoPicker)).unwrap());
    core.device_set_browser_bridge(true).unwrap();
    let bridge = Arc::new(Bridge::new(Arc::clone(&core)));
    Fx { dir, core, bridge }
}

fn create_request(rp: &str, user: &str, uv: &str) -> String {
    json!({
        "rp": {"id": rp, "name": rp},
        "user": {"id": "dXNlci0x", "name": user, "displayName": user},
        "challenge": "Y2hhbGxlbmdl",
        "pubKeyCredParams": [{"type": "public-key", "alg": -7}],
        "authenticatorSelection": {"residentKey": "required", "userVerification": uv},
    })
    .to_string()
}

fn get_request(rp: &str, uv: &str) -> String {
    json!({"challenge": "Y2hhbGxlbmdl", "rpId": rp, "userVerification": uv}).to_string()
}

fn line(id: &str, op: &str, origin: &str, request: &str) -> String {
    json!({"v": 1, "id": id, "op": op, "origin": origin, "request": request, "timeoutMs": 20000})
        .to_string()
}

fn submit(bridge: &Arc<Bridge>, line: String) -> JoinHandle<String> {
    let b = Arc::clone(bridge);
    std::thread::spawn(move || b.submit_line(&line, &|| true))
}

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn answer_of(h: JoinHandle<String>) -> Value {
    parsed(&h.join().unwrap())
}

/// The request the window would show, once one is there.
fn front_of(bridge: &Bridge) -> View {
    let limit = Instant::now() + Duration::from_secs(5);
    let mut after = 0;
    loop {
        let w = bridge.wait(after, 500);
        if let Some(v) = w.front {
            return v;
        }
        after = w.version;
        assert!(Instant::now() < limit, "no request reached the window");
    }
}

fn new_login() -> Value {
    json!({"new": {"title": null}})
}

/// Make a passkey for `example.com` through the bridge, the way a page does. Returns the registration.
fn make_passkey(f: &Fx) -> Value {
    let h = submit(
        &f.bridge,
        line(
            "make-0001",
            "create",
            "https://example.com",
            &create_request("example.com", "ana@example.com", "preferred"),
        ),
    );
    let v = front_of(&f.bridge);
    f.bridge
        .answer(
            &v.id,
            Action::Create {
                target: new_login(),
                password: None,
            },
        )
        .unwrap();
    let a = answer_of(h);
    assert_eq!(a["ok"], true, "{a}");
    parsed(a["responseJson"].as_str().unwrap())
}

fn client_data(response: &Value) -> Value {
    serde_json::from_slice(
        &b64::decode(response["response"]["clientDataJSON"].as_str().unwrap()).unwrap(),
    )
    .unwrap()
}

// ------------------------------------------------------------------ what a request needs before the window sees it

#[test]
fn with_the_switch_off_every_request_goes_to_the_browsers_own_flow() {
    let f = fx();
    f.core.device_set_browser_bridge(false).unwrap();
    for (op, request) in [
        ("create", create_request("example.com", "ana", "preferred")),
        ("get", get_request("example.com", "preferred")),
    ] {
        let a = parsed(&f.bridge.submit_line(
            &line("off-00001", op, "https://example.com", &request),
            &|| true,
        ));
        assert_eq!(a["code"], "fallback", "{a}");
        assert_eq!(a["reason"], "disabled");
        assert_eq!(a["id"], "off-00001");
    }
    let a = parsed(
        &f.bridge
            .submit_line(r#"{"v":1,"id":"status-01","op":"status"}"#, &|| true),
    );
    assert_eq!(a["reason"], "disabled");
    assert!(f.bridge.front().front.is_none(), "nothing was queued");
}

#[test]
fn status_says_the_bridge_is_on_and_nothing_about_the_vault() {
    let f = fx();
    let ask = || {
        parsed(
            &f.bridge
                .submit_line(r#"{"v":1,"id":"status-01","op":"status"}"#, &|| true),
        )
    };
    let a = ask();
    assert_eq!(
        (a["ok"].clone(), a["enabled"].clone()),
        (json!(true), json!(true))
    );
    // Any page may ask, so the answer must not tell a page whether the app lock is closed.
    f.core.lock();
    assert_eq!(ask(), a);
    assert!(a.get("locked").is_none(), "{a}");
}

#[test]
fn a_request_nobody_could_have_sent_is_refused_with_an_error() {
    let f = fx();
    for text in [
        "not json",
        r#"{"v":1,"id":"x","op":"get"}"#,
        r#"{"v":2,"id":"abcdefgh","op":"get"}"#,
        r#"{"v":1,"id":"abcdefgh","op":"delete"}"#,
        r#"{"v":1,"id":"abcdefgh","op":"get","origin":"https://example.com/path","request":"{}"}"#,
        r#"{"v":1,"id":"abcdefgh","op":"get","origin":"http://example.com","request":"{}"}"#,
        r#"{"v":1,"id":"abcdefgh","op":"get","origin":"https://example.com","request":""}"#,
    ] {
        let a = parsed(&f.bridge.submit_line(text, &|| true));
        assert_eq!(a["code"], "error", "{text}: {a}");
        assert_eq!(a["errorCode"], "invalid_input");
    }
    assert!(f.bridge.front().front.is_none());
}

#[test]
fn with_no_account_the_request_goes_to_the_browser() {
    let f = bare();
    let a = parsed(&f.bridge.submit_line(
        &line(
            "none-0001",
            "create",
            "https://example.com",
            &create_request("example.com", "a", "preferred"),
        ),
        &|| true,
    ));
    assert_eq!(
        (a["code"].clone(), a["reason"].clone()),
        (json!("fallback"), json!("no_vault"))
    );
}

#[test]
fn with_the_app_lock_closed_the_request_goes_to_the_browser_and_nothing_is_told() {
    let f = fx();
    f.core
        .gate_configure(
            serde_json::from_value(json!({"enabled": true, "mode": "pin", "credential": "24680", "masterPassword": PW}))
                .unwrap(),
        )
        .unwrap();
    f.core.gate_close();
    let a = f.bridge.submit_line(
        &line(
            "gate-0001",
            "get",
            "https://example.com",
            &get_request("example.com", "preferred"),
        ),
        &|| true,
    );
    let v = parsed(&a);
    assert_eq!(
        (v["code"].clone(), v["reason"].clone()),
        (json!("fallback"), json!("app_locked"))
    );
    assert!(!a.contains("Ana") && !a.contains("example"), "{a}");
}

#[test]
fn a_sign_in_with_no_passkey_for_the_website_goes_to_the_browser_without_a_prompt() {
    let f = fx();
    make_passkey(&f);
    let a = parsed(&f.bridge.submit_line(
        &line(
            "none-0002",
            "get",
            "https://other.example",
            &get_request("other.example", "preferred"),
        ),
        &|| true,
    ));
    assert_eq!(
        (a["code"].clone(), a["reason"].clone()),
        (json!("fallback"), json!("no_match"))
    );
    assert!(f.bridge.front().front.is_none(), "no prompt was queued");
}

#[test]
fn a_page_cannot_make_the_window_list_another_websites_passkeys() {
    let f = fx();
    make_passkey(&f);
    // the page is on evil.test but names example.com; the name `com` is shared by every .com site; plain http is no secure page
    for (origin, rp) in [
        ("https://evil.test", "example.com"),
        ("https://example.com", "com"),
        ("https://sub.github.io", "github.io"),
    ] {
        for (op, request) in [
            ("get", get_request(rp, "preferred")),
            ("create", create_request(rp, "ana", "preferred")),
        ] {
            let a = parsed(
                &f.bridge
                    .submit_line(&line("evil-0001", op, origin, &request), &|| true),
            );
            assert_eq!(
                (a["code"].clone(), a["reason"].clone()),
                (json!("fallback"), json!("invalid_request")),
                "{op} {origin} {rp}: {a}"
            );
        }
    }
    assert!(f.bridge.front().front.is_none());
}

// ------------------------------------------------------------------ the window's side

#[test]
fn a_new_passkey_waits_for_the_person_and_is_saved_only_when_they_allow_it() {
    let f = fx();
    let h = submit(
        &f.bridge,
        line(
            "create-001",
            "create",
            "https://example.com",
            &create_request("example.com", "ana@example.com", "required"),
        ),
    );
    let v = front_of(&f.bridge);
    assert_eq!(
        (v.kind, v.origin.as_str()),
        ("create", "https://example.com")
    );
    assert!(v.remaining_ms > 0 && v.remaining_ms <= 20_000);
    assert_eq!(
        f.core.item_list().unwrap().len(),
        0,
        "nothing is saved while the window has not answered"
    );

    let o = f.bridge.options(&v.id).unwrap();
    assert_eq!(
        (
            o["kind"].clone(),
            o["rpId"].clone(),
            o["verification"].clone()
        ),
        (json!("create"), json!("example.com"), json!("required"))
    );
    assert_eq!(o["origin"], "https://example.com");

    // the website requires verification: a click is not enough, the master password is
    let e = f
        .bridge
        .answer(
            &v.id,
            Action::Create {
                target: new_login(),
                password: None,
            },
        )
        .unwrap_err();
    assert_eq!(e.code, "password_required");
    assert!(
        f.bridge.front().front.is_some(),
        "the request keeps waiting"
    );
    assert_eq!(f.core.item_list().unwrap().len(), 0);

    f.bridge
        .answer(
            &v.id,
            Action::Create {
                target: new_login(),
                password: Some(PW.into()),
            },
        )
        .unwrap();
    let a = answer_of(h);
    assert_eq!(
        (a["ok"].clone(), a["id"].clone()),
        (json!(true), json!("create-001"))
    );
    let reg = parsed(a["responseJson"].as_str().unwrap());
    // the client data was built here from the address the extension saw, and carries the verification
    let cd = client_data(&reg);
    assert_eq!(
        (cd["type"].clone(), cd["origin"].clone()),
        (json!("webauthn.create"), json!("https://example.com"))
    );
    let auth = b64::decode(reg["response"]["authenticatorData"].as_str().unwrap()).unwrap();
    assert_eq!(auth[32] & 0x04, 0x04, "user verified");
    assert!(!a.to_string().contains("privateKey"));
    assert_eq!(f.core.item_list().unwrap().len(), 1);
    assert!(f.bridge.front().front.is_none(), "the request is gone");
}

#[test]
fn a_wrong_master_password_on_a_new_login_saves_nothing_and_the_request_keeps_waiting() {
    let f = fx();
    let h = submit(
        &f.bridge,
        line(
            "wrongpw-01",
            "create",
            "https://example.com",
            &create_request("example.com", "ana@example.com", "required"),
        ),
    );
    let v = front_of(&f.bridge);
    for password in ["not the password", "Not The Password"] {
        let e = f
            .bridge
            .answer(
                &v.id,
                Action::Create {
                    target: new_login(),
                    password: Some(password.into()),
                },
            )
            .unwrap_err();
        assert!(
            ["wrong_password", "password_required"].contains(&e.code.as_str()),
            "{password:?}: {}",
            e.code
        );
        assert_eq!(f.core.item_list().unwrap().len(), 0, "nothing was saved");
        assert_eq!(
            f.bridge.front().front.map(|x| x.id),
            Some(v.id.clone()),
            "the request keeps waiting"
        );
    }
    let e = f
        .bridge
        .answer(
            &v.id,
            Action::Create {
                target: new_login(),
                password: Some("not the password".into()),
            },
        )
        .unwrap_err();
    assert_eq!(e.code, "wrong_password");
    // and the right one still works after all that
    f.bridge
        .answer(
            &v.id,
            Action::Create {
                target: new_login(),
                password: Some(PW.into()),
            },
        )
        .unwrap();
    let a = answer_of(h);
    assert_eq!(a["ok"], true, "{a}");
    let reg = parsed(a["responseJson"].as_str().unwrap());
    let auth = b64::decode(reg["response"]["authenticatorData"].as_str().unwrap()).unwrap();
    assert_eq!(auth[32] & 0x04, 0x04, "verified by the typed password");
    assert_eq!(f.core.item_list().unwrap().len(), 1);
}

#[test]
fn a_request_id_is_used_once_even_after_it_has_been_answered() {
    let f = fx();
    let ask = |id: &str| {
        submit(
            &f.bridge,
            line(
                id,
                "create",
                "https://example.com",
                &create_request("example.com", "ana", "preferred"),
            ),
        )
    };
    let h = ask("once-0001");
    let v = front_of(&f.bridge);
    f.bridge.answer(&v.id, Action::Decline).unwrap();
    assert_eq!(answer_of(h)["reason"], "declined");
    // the same id again, from a connection that is not the first one, gets nothing in front of the person
    let a = parsed(&f.bridge.submit_line(
        &line(
            "once-0001",
            "create",
            "https://example.com",
            &create_request("example.com", "ana", "preferred"),
        ),
        &|| true,
    ));
    assert_eq!(
        (a["code"].clone(), a["reason"].clone()),
        (json!("fallback"), json!("busy"))
    );
    assert!(f.bridge.front().front.is_none());
}

#[test]
fn signing_in_uses_the_passkey_the_person_picked_and_the_signature_checks() {
    let f = fx();
    let reg = make_passkey(&f);
    let spki = b64::decode(reg["response"]["publicKey"].as_str().unwrap()).unwrap();
    let key = VerifyingKey::from_sec1_bytes(&spki[26..]).unwrap();

    let h = submit(
        &f.bridge,
        line(
            "get-00001",
            "get",
            "https://example.com",
            &get_request("example.com", "preferred"),
        ),
    );
    let v = front_of(&f.bridge);
    assert_eq!(v.kind, "get");
    let o = f.bridge.options(&v.id).unwrap();
    let m = &o["matches"][0];
    assert_eq!(m["userName"], "ana@example.com");
    assert!(!o.to_string().contains("privateKey"));
    f.bridge
        .answer(
            &v.id,
            Action::Use {
                item_id: m["itemId"].as_str().unwrap().into(),
                key: m["key"].as_str().unwrap().into(),
                password: None,
            },
        )
        .unwrap();
    let a = answer_of(h);
    assert_eq!(a["ok"], true, "{a}");
    let asserted = parsed(a["responseJson"].as_str().unwrap());
    assert_eq!(asserted["id"], reg["id"]);
    let cd = client_data(&asserted);
    assert_eq!(
        (cd["type"].clone(), cd["origin"].clone()),
        (json!("webauthn.get"), json!("https://example.com"))
    );
    let mut signed =
        b64::decode(asserted["response"]["authenticatorData"].as_str().unwrap()).unwrap();
    assert_eq!(
        signed[32] & 0x04,
        0,
        "no password was typed, so the answer does not claim verification"
    );
    let client_bytes =
        b64::decode(asserted["response"]["clientDataJSON"].as_str().unwrap()).unwrap();
    signed.extend_from_slice(&Sha256::digest(&client_bytes));
    let sig = b64::decode(asserted["response"]["signature"].as_str().unwrap()).unwrap();
    assert!(key
        .verify(&signed, &Signature::from_slice(&rs_from_der(&sig)).unwrap())
        .is_ok());
}

#[test]
fn a_wrong_master_password_leaves_the_request_waiting_for_another_try() {
    let f = fx();
    make_passkey(&f);
    let h = submit(
        &f.bridge,
        line(
            "retry-001",
            "get",
            "https://example.com",
            &get_request("example.com", "required"),
        ),
    );
    let v = front_of(&f.bridge);
    let m = f.bridge.options(&v.id).unwrap()["matches"][0].clone();
    let use_with = |password: Option<&str>| Action::Use {
        item_id: m["itemId"].as_str().unwrap().into(),
        key: m["key"].as_str().unwrap().into(),
        password: password.map(Into::into),
    };
    assert_eq!(
        f.bridge.answer(&v.id, use_with(None)).unwrap_err().code,
        "password_required"
    );
    assert_eq!(
        f.bridge
            .answer(&v.id, use_with(Some("wrong")))
            .unwrap_err()
            .code,
        "wrong_password"
    );
    assert_eq!(
        f.bridge.front().front.map(|v| v.id),
        Some(v.id.clone()),
        "still there"
    );
    f.bridge.answer(&v.id, use_with(Some(PW))).unwrap();
    let a = answer_of(h);
    assert_eq!(a["ok"], true, "{a}");
    let auth = b64::decode(
        parsed(a["responseJson"].as_str().unwrap())["response"]["authenticatorData"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        auth[32] & 0x04,
        0x04,
        "the typed password verified the person"
    );
}

#[test]
fn a_login_that_asks_for_the_master_password_asks_for_it_here_too() {
    let f = fx();
    let draft = serde_json::from_value(json!({
        "type": "login", "title": "Bank", "reprompt": true,
        "fields": {"username": "ana@example.com", "url": "https://bank.example"},
    }))
    .unwrap();
    let item = f.core.item_create(draft).unwrap().id;
    let h = submit(
        &f.bridge,
        line(
            "bank-0001",
            "create",
            "https://bank.example",
            &create_request("bank.example", "ana@example.com", "preferred"),
        ),
    );
    let v = front_of(&f.bridge);
    let target = json!({"existing": item});
    let e = f
        .bridge
        .answer(
            &v.id,
            Action::Create {
                target: target.clone(),
                password: None,
            },
        )
        .unwrap_err();
    assert_eq!(e.code, "password_required");
    f.bridge
        .answer(
            &v.id,
            Action::Create {
                target,
                password: Some(PW.into()),
            },
        )
        .unwrap();
    assert_eq!(answer_of(h)["ok"], true);
}

#[test]
fn a_passkey_that_already_exists_is_an_answer_for_the_page_not_a_retry() {
    let f = fx();
    make_passkey(&f);
    // the page lists the credential it already knows, so a second one for the same account is refused
    let reg = {
        let h = submit(
            &f.bridge,
            line(
                "again-001",
                "get",
                "https://example.com",
                &get_request("example.com", "preferred"),
            ),
        );
        let v = front_of(&f.bridge);
        let m = f.bridge.options(&v.id).unwrap()["matches"][0].clone();
        f.bridge
            .answer(
                &v.id,
                Action::Use {
                    item_id: m["itemId"].as_str().unwrap().into(),
                    key: m["key"].as_str().unwrap().into(),
                    password: None,
                },
            )
            .unwrap();
        parsed(answer_of(h)["responseJson"].as_str().unwrap())
    };
    let mut request: Value = serde_json::from_str(&create_request(
        "example.com",
        "ana@example.com",
        "preferred",
    ))
    .unwrap();
    request["excludeCredentials"] = json!([{"type": "public-key", "id": reg["rawId"]}]);
    let h = submit(
        &f.bridge,
        line(
            "again-002",
            "create",
            "https://example.com",
            &request.to_string(),
        ),
    );
    let v = front_of(&f.bridge);
    f.bridge
        .answer(
            &v.id,
            Action::Create {
                target: new_login(),
                password: None,
            },
        )
        .unwrap();
    let a = answer_of(h);
    assert_eq!(
        (a["code"].clone(), a["errorCode"].clone()),
        (json!("error"), json!("passkey_excluded")),
        "{a}"
    );
}

#[test]
fn declining_and_cancelling_end_the_request_in_different_ways() {
    let f = fx();
    for (n, (action, code, reason)) in [
        (Action::Decline, "fallback", Some("declined")),
        (Action::Cancel, "cancelled", None),
    ]
    .into_iter()
    .enumerate()
    {
        let h = submit(
            &f.bridge,
            line(
                &format!("choice-{n:03}"),
                "create",
                "https://example.com",
                &create_request("example.com", "ana", "preferred"),
            ),
        );
        let v = front_of(&f.bridge);
        f.bridge.answer(&v.id, action).unwrap();
        let a = answer_of(h);
        assert_eq!(a["code"], code, "{a}");
        assert_eq!(a["reason"].as_str(), reason);
        assert!(f.bridge.front().front.is_none());
    }
    assert_eq!(f.core.item_list().unwrap().len(), 0, "nothing was saved");
}

#[test]
fn a_choice_for_a_request_that_is_gone_is_refused() {
    let f = fx();
    let e = f.bridge.answer("nothing-1", Action::Cancel).unwrap_err();
    assert_eq!(e.code, "not_found");
    let e = f.bridge.options("nothing-1").unwrap_err();
    assert_eq!(e.code, "not_found");
}

#[test]
fn a_sign_in_cannot_be_answered_as_a_save_and_the_other_way_round() {
    let f = fx();
    make_passkey(&f);
    let h = submit(
        &f.bridge,
        line(
            "kind-0001",
            "get",
            "https://example.com",
            &get_request("example.com", "preferred"),
        ),
    );
    let v = front_of(&f.bridge);
    let e = f
        .bridge
        .answer(
            &v.id,
            Action::Create {
                target: new_login(),
                password: None,
            },
        )
        .unwrap_err();
    assert_eq!(e.code, "invalid_input");
    f.bridge.answer(&v.id, Action::Cancel).unwrap();
    assert_eq!(answer_of(h)["code"], "cancelled");
}

#[test]
fn a_locked_vault_offers_nothing_until_the_lock_screen_has_opened_it() {
    let f = fx();
    f.core.lock();
    let h = submit(
        &f.bridge,
        line(
            "lock-0001",
            "create",
            "https://example.com",
            &create_request("example.com", "ana", "preferred"),
        ),
    );
    let v = front_of(&f.bridge);
    assert!(
        f.bridge.front().locked,
        "the window shows its lock screen, not the request"
    );
    assert_eq!(f.bridge.options(&v.id).unwrap_err().code, "locked");
    f.core.unlock(PW.into()).unwrap(); // the person types the master password on the lock screen
    assert!(!f.bridge.front().locked);
    assert_eq!(f.bridge.options(&v.id).unwrap()["rpId"], "example.com");
    f.bridge.answer(&v.id, Action::Cancel).unwrap();
    assert_eq!(answer_of(h)["code"], "cancelled");
}

#[test]
fn a_request_this_page_may_not_make_is_dropped_when_it_is_seen_after_unlocking() {
    let f = fx();
    f.core.lock();
    let h = submit(
        &f.bridge,
        line(
            "late-0001",
            "create",
            "https://evil.test",
            &create_request("example.com", "ana", "preferred"),
        ),
    );
    let v = front_of(&f.bridge);
    f.core.unlock(PW.into()).unwrap();
    assert_eq!(f.bridge.options(&v.id).unwrap_err().code, "invalid_input");
    let a = answer_of(h);
    assert_eq!(
        (a["code"].clone(), a["reason"].clone()),
        (json!("fallback"), json!("invalid_request"))
    );
}

// ------------------------------------------------------------------ the end of a request

#[test]
fn a_request_nobody_answers_times_out_and_leaves_the_queue() {
    let f = fx();
    let a = parsed(&f.bridge.ask(
        "slow-0001".into(),
        Kind::Create,
        "https://example.com".into(),
        create_request("example.com", "ana", "preferred"),
        120,
        &|| true,
    ));
    assert_eq!(a["code"], "timeout");
    assert!(f.bridge.front().front.is_none());
}

#[test]
fn a_browser_that_hung_up_takes_the_prompt_with_it() {
    let f = fx();
    let gone = AtomicBool::new(false);
    let started = Instant::now();
    let b = Arc::clone(&f.bridge);
    let h = std::thread::scope(|s| {
        let h = s.spawn(|| {
            b.ask(
                "gone-0001".into(),
                Kind::Create,
                "https://example.com".into(),
                create_request("example.com", "ana", "preferred"),
                30_000,
                &|| !gone.load(Ordering::SeqCst),
            )
        });
        front_of(&f.bridge);
        gone.store(true, Ordering::SeqCst);
        h.join().unwrap()
    });
    assert_eq!(parsed(&h)["code"], "cancelled");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        f.bridge.front().front.is_none(),
        "the prompt is gone from the window"
    );
}

#[test]
fn at_most_eight_requests_wait_and_one_id_is_one_request() {
    let f = fx();
    let mut waiting = Vec::new();
    for n in 0..MAX_QUEUE {
        waiting.push(submit(
            &f.bridge,
            line(
                &format!("queue-{n:03}"),
                "create",
                "https://example.com",
                &create_request("example.com", "ana", "preferred"),
            ),
        ));
    }
    let limit = Instant::now() + Duration::from_secs(5);
    while f.bridge.guard().queue.len() < MAX_QUEUE {
        assert!(Instant::now() < limit, "the queue did not fill");
        std::thread::sleep(Duration::from_millis(10));
    }
    for id in ["queue-extra", "queue-000"] {
        let a = parsed(&f.bridge.submit_line(
            &line(
                id,
                "create",
                "https://example.com",
                &create_request("example.com", "ana", "preferred"),
            ),
            &|| true,
        ));
        assert_eq!(
            (a["code"].clone(), a["reason"].clone()),
            (json!("fallback"), json!("busy")),
            "{id}"
        );
    }
    f.bridge.end_all("disabled");
    for h in waiting {
        assert_eq!(answer_of(h)["reason"], "disabled");
    }
    assert!(f.bridge.front().front.is_none());
}

#[test]
fn the_window_hears_of_a_change_at_once_and_a_quiet_wait_ends_on_time() {
    let f = fx();
    let quiet = Instant::now();
    let w = f.bridge.wait(f.bridge.front().version, 150);
    assert!(w.front.is_none());
    assert!(
        quiet.elapsed() >= Duration::from_millis(100) && quiet.elapsed() < Duration::from_secs(3)
    );

    let after = f.bridge.front().version;
    let b = Arc::clone(&f.bridge);
    let waiter = std::thread::spawn(move || {
        let t = Instant::now();
        let w = b.wait(after, 10_000);
        (w, t.elapsed())
    });
    std::thread::sleep(Duration::from_millis(80));
    let h = submit(
        &f.bridge,
        line(
            "hear-0001",
            "create",
            "https://example.com",
            &create_request("example.com", "ana", "preferred"),
        ),
    );
    let (w, took) = waiter.join().unwrap();
    assert!(w.front.is_some());
    assert!(took < Duration::from_secs(3), "{took:?}");
    f.bridge.end_all("disabled");
    h.join().unwrap();
}

// ------------------------------------------------------------------ the socket and the native host (Unix)

#[cfg(unix)]
mod over_the_socket {
    use super::*;
    use crate::bridge::{host, socket};
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::Path;

    fn framed(text: &str) -> Vec<u8> {
        let mut v = u32::try_from(text.len()).unwrap().to_ne_bytes().to_vec();
        v.extend_from_slice(text.as_bytes());
        v
    }

    fn unframed(bytes: &[u8]) -> Value {
        let n = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 4 + n, "exactly one message");
        parsed(std::str::from_utf8(&bytes[4..]).unwrap())
    }

    /// Where `$XDG_RUNTIME_DIR` would be: the folder above Hexis's own socket folder exists before Hexis starts.
    fn run_folder(f: &Fx) -> std::path::PathBuf {
        let dir = f.dir.path().join("run");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn listening(f: &Fx) -> std::path::PathBuf {
        let path = run_folder(f).join("hexis").join("bridge.sock");
        f.bridge.start_at(&path).unwrap();
        path
    }

    #[test]
    fn a_request_travels_the_socket_and_the_answer_comes_back() {
        let f = fx();
        let path = listening(&f);
        assert!(f.bridge.listening());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let status =
            parsed(&socket::ask(&path, r#"{"v":1,"id":"sock-0001","op":"status"}"#).unwrap());
        assert_eq!(
            (status["ok"].clone(), status["enabled"].clone()),
            (json!(true), json!(true))
        );

        let request = line(
            "sock-0002",
            "create",
            "https://example.com",
            &create_request("example.com", "ana", "preferred"),
        );
        let p = path.clone();
        let asked = std::thread::spawn(move || socket::ask(&p, &request).unwrap());
        let v = front_of(&f.bridge);
        f.bridge
            .answer(
                &v.id,
                Action::Create {
                    target: new_login(),
                    password: None,
                },
            )
            .unwrap();
        let a = parsed(&asked.join().unwrap());
        assert_eq!(
            (a["ok"].clone(), a["id"].clone()),
            (json!(true), json!("sock-0002"))
        );
        f.bridge.stop();
        assert!(!path.exists(), "the socket is removed when listening stops");
        assert!(!f.bridge.listening());
    }

    #[test]
    fn a_hung_up_connection_drops_the_prompt() {
        let f = fx();
        let path = listening(&f);
        let request = line(
            "hang-0001",
            "create",
            "https://example.com",
            &create_request("example.com", "ana", "preferred"),
        );
        {
            use std::io::Write;
            let mut stream = std::os::unix::net::UnixStream::connect(&path).unwrap();
            stream.write_all(format!("{request}\n").as_bytes()).unwrap();
            front_of(&f.bridge);
        } // the stream is closed here, as a browser that closed the tab closes the host's pipe
        let limit = Instant::now() + Duration::from_secs(5);
        while f.bridge.front().front.is_some() {
            assert!(Instant::now() < limit, "the prompt stayed");
            std::thread::sleep(Duration::from_millis(50));
        }
        f.bridge.stop();
    }

    #[test]
    fn garbage_on_the_socket_gets_an_error_and_nothing_else() {
        let f = fx();
        let path = listening(&f);
        let a = parsed(&socket::ask(&path, "this is not json").unwrap());
        assert_eq!(
            (a["code"].clone(), a["errorCode"].clone()),
            (json!("error"), json!("invalid_input"))
        );
        let big = format!(
            r#"{{"v":1,"id":"big-00001","op":"get","origin":"https://example.com","request":"{}"}}"#,
            "x".repeat(wire::MAX_REQUEST_BYTES + 1)
        );
        let a = parsed(&socket::ask(&path, &big).unwrap());
        assert_eq!(a["code"], "error");
        assert!(f.bridge.front().front.is_none());
        f.bridge.stop();
    }

    #[test]
    fn a_folder_other_people_can_enter_is_refused_and_a_second_hexis_is_told_so() {
        let f = fx();
        let folder = f.dir.path().join("open");
        std::fs::create_dir(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = f.bridge.start_at(&folder.join("bridge.sock")).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);

        let path = listening(&f);
        let other = Arc::new(Bridge::new(Arc::clone(&f.core)));
        let e = other.start_at(&path).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse);
        f.bridge.stop();
    }

    #[test]
    fn the_host_will_not_talk_to_a_socket_in_a_folder_that_is_not_private_or_is_a_link() {
        let f = fx();
        let status = r#"{"v":1,"id":"status-01","op":"status"}"#;
        let base = f.dir.path().join("host-checks");
        std::fs::create_dir_all(&base).unwrap();
        // somebody answers in each of these places, as an attacker's program would
        let open = base.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _a = UnixListener::bind(open.join("bridge.sock")).unwrap();
        let e = socket::ask(&open.join("bridge.sock"), status).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");

        let real = base.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _b = UnixListener::bind(real.join("bridge.sock")).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let e = socket::ask(&link.join("bridge.sock"), status).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");

        // a plain file with the socket's name, and a link to a socket, are not the socket either
        let fake = base.join("fake");
        std::fs::create_dir(&fake).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(fake.join("bridge.sock"), "x").unwrap();
        assert!(socket::ask(&fake.join("bridge.sock"), status).is_err());
        std::fs::remove_file(fake.join("bridge.sock")).unwrap();
        std::os::unix::fs::symlink(real.join("bridge.sock"), fake.join("bridge.sock")).unwrap();
        let e = socket::ask(&fake.join("bridge.sock"), status).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");

        // and Hexis's own, in its own folder, is still reached
        let path = listening(&f);
        let a: Value = parsed(&socket::ask(&path, status).unwrap());
        assert_eq!(a["ok"], true);
        f.bridge.stop();
    }

    #[test]
    fn a_socket_left_by_a_crash_is_replaced_and_something_else_in_the_way_is_not() {
        let f = fx();
        let path = f.dir.path().join("run").join("bridge.sock");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::set_permissions(
            path.parent().unwrap(),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        drop(UnixListener::bind(&path).unwrap()); // leaves the file behind, nobody answering
        assert!(path.exists());
        f.bridge.start_at(&path).unwrap();
        assert!(f.bridge.listening());
        f.bridge.stop();

        std::fs::write(&path, "not a socket").unwrap();
        let e = f.bridge.start_at(&path).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "not a socket",
            "left alone"
        );
    }

    #[test]
    fn the_native_host_passes_one_message_and_prints_one_answer() {
        let f = fx();
        let path = listening(&f);
        let mut out = Vec::new();
        host::relay(
            &mut Cursor::new(framed(r#"{"v":1,"id":"host-0001","op":"status"}"#)),
            &mut out,
            &path,
        )
        .unwrap();
        let a = unframed(&out);
        assert_eq!(
            (a["id"].clone(), a["enabled"].clone()),
            (json!("host-0001"), json!(true))
        );
        f.bridge.stop();
    }

    #[test]
    fn with_no_hexis_running_the_host_says_so_in_the_requests_own_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        host::relay(
            &mut Cursor::new(framed(r#"{"v":1,"id":"host-0002","op":"get"}"#)),
            &mut out,
            &dir.path().join("none.sock"),
        )
        .unwrap();
        let a = unframed(&out);
        assert_eq!(
            (a["id"].clone(), a["code"].clone(), a["reason"].clone()),
            (json!("host-0002"), json!("fallback"), json!("not_running"))
        );
    }

    #[test]
    fn the_native_host_refuses_what_the_extension_would_never_send() {
        let dir = tempfile::tempdir().unwrap();
        let path: &Path = &dir.path().join("none.sock");
        // line breaks inside a message
        let mut out = Vec::new();
        host::relay(
            &mut Cursor::new(framed("{\"v\":1,\n\"id\":\"host-0003\"}")),
            &mut out,
            path,
        )
        .unwrap();
        assert_eq!(unframed(&out)["errorCode"], "invalid_input");
        // a message longer than any request
        let long = (u32::try_from(wire::MAX_LINE_BYTES).unwrap() + 1)
            .to_ne_bytes()
            .to_vec();
        assert!(host::read_message(&mut Cursor::new(long)).is_err());
        // an empty pipe is not an error and prints nothing
        let mut out = Vec::new();
        host::relay(&mut Cursor::new(Vec::new()), &mut out, path).unwrap();
        assert!(out.is_empty());
    }
}

// ------------------------------------------------------------------ the extension's own files

#[test]
fn the_extension_is_written_whole_kept_in_step_and_removed_without_a_trace() {
    use crate::bridge::extension;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    assert!(!extension::present(&data));
    let folders = extension::write(&data).unwrap();
    assert!(extension::present(&data));
    let manifest = |dir: &std::path::Path| {
        parsed(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
    };
    // each family gets its own manifest and the same scripts
    assert!(manifest(&folders.chromium)["key"].is_string());
    assert!(manifest(&folders.firefox)["browser_specific_settings"]["gecko"]["id"].is_string());
    for name in [
        "page.js",
        "content.js",
        "background.js",
        "icon-32.png",
        "icon-64.png",
        "icon-128.png",
    ] {
        assert_eq!(
            std::fs::read(folders.chromium.join(name)).unwrap(),
            std::fs::read(folders.firefox.join(name)).unwrap(),
            "{name}"
        );
    }
    // the manifests name only files that are there
    for dir in [&folders.chromium, &folders.firefox] {
        let m = manifest(dir);
        let mut named: Vec<String> = m["content_scripts"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| {
                c["js"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|j| j.as_str().unwrap().to_string())
            })
            .collect();
        named.extend(
            m["icons"]
                .as_object()
                .unwrap()
                .values()
                .map(|v| v.as_str().unwrap().to_string()),
        );
        named.push(
            m["background"]["service_worker"]
                .as_str()
                .or_else(|| m["background"]["scripts"][0].as_str())
                .unwrap()
                .to_string(),
        );
        for file in named {
            assert!(dir.join(&file).is_file(), "{file} is named but missing");
        }
    }
    // a changed or deleted file is made right again
    std::fs::write(folders.chromium.join("page.js"), "tampered").unwrap();
    assert!(!extension::present(&data));
    extension::write(&data).unwrap();
    assert!(extension::present(&data));
    extension::remove(&data);
    assert!(!data.join("extension").exists(), "nothing is left behind");
    assert!(data.exists());
}

// ------------------------------------------------------------------ the switch and the registration (Linux)

#[cfg(target_os = "linux")]
mod on_linux {
    use super::*;
    use crate::bridge::install::{self, BROWSERS};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn home_with(browsers: &[&str]) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        for b in BROWSERS.iter().filter(|b| browsers.contains(&b.id)) {
            std::fs::create_dir_all(home.join(b.config)).unwrap();
        }
        (dir, home, data)
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    fn manifest_of(home: &Path, id: &str) -> Value {
        let b = BROWSERS.iter().find(|b| b.id == id).unwrap();
        parsed(
            &std::fs::read_to_string(
                home.join(b.hosts)
                    .join(format!("{}.json", install::NATIVE_HOST_NAME)),
            )
            .unwrap(),
        )
    }

    #[test]
    fn registering_writes_the_manifests_and_the_launcher_and_removing_takes_them_away() {
        let (_dir, home, data) = home_with(&["firefox", "chrome"]);
        let program = Path::new("/opt/Hexis app/it's.AppImage");
        let done = install::install(
            &home,
            &data,
            program,
            &ids(&["chrome", "firefox", "brave", "nonsense"]),
        )
        .unwrap();
        assert_eq!(
            done,
            vec!["firefox", "chrome"],
            "brave was never started, nonsense is nothing"
        );

        let launcher = data.join(install::LAUNCHER_NAME);
        assert_eq!(
            std::fs::metadata(&launcher).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let text = std::fs::read_to_string(&launcher).unwrap();
        assert_eq!(
            text,
            "#!/bin/sh\nexec '/opt/Hexis app/it'\\''s.AppImage' --native-host \"$@\"\n"
        );

        let chrome = manifest_of(&home, "chrome");
        assert_eq!(chrome["name"], install::NATIVE_HOST_NAME);
        assert_eq!(chrome["path"], launcher.to_string_lossy().as_ref());
        assert_eq!(chrome["type"], "stdio");
        assert_eq!(
            chrome["allowed_origins"],
            json!([format!(
                "chrome-extension://{}/",
                install::CHROMIUM_EXTENSION_ID
            )])
        );
        assert!(chrome.get("allowed_extensions").is_none());
        let firefox = manifest_of(&home, "firefox");
        assert_eq!(
            firefox["allowed_extensions"],
            json!([install::FIREFOX_EXTENSION_ID])
        );
        assert!(firefox.get("allowed_origins").is_none());

        let s = install::status(&home, &data);
        let get = |id: &str| s.browsers.iter().find(|b| b.id == id).unwrap().clone();
        assert!(get("chrome").registered && get("firefox").registered);
        assert!(!get("brave").found && !get("brave").registered);

        install::uninstall(&home, &data, &ids(&["chrome"])).unwrap();
        assert!(
            !install::status(&home, &data)
                .browsers
                .iter()
                .find(|b| b.id == "chrome")
                .unwrap()
                .registered
        );
        assert!(launcher.exists(), "Firefox still uses it");
        install::uninstall(&home, &data, &[]).unwrap();
        assert!(
            !launcher.exists(),
            "the last registration took the launcher with it"
        );
        assert!(install::status(&home, &data)
            .browsers
            .iter()
            .all(|b| !b.registered && !b.stale));
    }

    #[test]
    fn a_registration_that_does_not_match_is_stale_until_it_is_made_again() {
        let (_dir, home, data) = home_with(&["chromium"]);
        let program = Path::new("/opt/hexis");
        install::install(&home, &data, program, &ids(&["chromium"])).unwrap();
        let b = BROWSERS.iter().find(|b| b.id == "chromium").unwrap();
        let file = home
            .join(b.hosts)
            .join(format!("{}.json", install::NATIVE_HOST_NAME));
        std::fs::write(
            &file,
            r#"{"name":"dev.habibullah.hexis.passkeys","path":"/old/place"}"#,
        )
        .unwrap();
        let s = install::status(&home, &data);
        let c = s.browsers.iter().find(|b| b.id == "chromium").unwrap();
        assert!(c.stale && !c.registered);
        install::install(&home, &data, program, &ids(&["chromium"])).unwrap();
        let s = install::status(&home, &data);
        let c = s.browsers.iter().find(|b| b.id == "chromium").unwrap();
        assert!(c.registered && !c.stale);
        // a launcher that went missing is stale too
        std::fs::remove_file(data.join(install::LAUNCHER_NAME)).unwrap();
        assert!(
            install::status(&home, &data)
                .browsers
                .iter()
                .find(|b| b.id == "chromium")
                .unwrap()
                .stale
        );
    }

    #[test]
    fn what_is_not_ours_in_a_browsers_folder_is_never_touched() {
        let (_dir, home, data) = home_with(&["chrome"]);
        let b = BROWSERS.iter().find(|b| b.id == "chrome").unwrap();
        let folder = home.join(b.hosts);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("com.other.host.json"), "{}").unwrap();
        install::install(&home, &data, Path::new("/opt/hexis"), &ids(&["chrome"])).unwrap();
        install::uninstall(&home, &data, &[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(folder.join("com.other.host.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn browsers_in_a_sandbox_are_named_because_they_cannot_start_the_host() {
        let (_dir, home, data) = home_with(&[]);
        std::fs::create_dir_all(home.join("snap/firefox")).unwrap();
        std::fs::create_dir_all(home.join(".var/app/com.google.Chrome")).unwrap();
        let s = install::status(&home, &data);
        assert_eq!(
            s.sandboxed,
            vec!["Firefox (Snap)".to_string(), "Chrome (Flatpak)".to_string()]
        );
    }

    #[test]
    fn the_switch_opens_the_socket_and_is_kept_only_when_that_worked() {
        let f = fx();
        f.core.device_set_browser_bridge(false).unwrap();
        // without a place to live the bridge is not supported
        assert!(!f.bridge.status().supported);
        assert_eq!(f.bridge.set_enabled(true).unwrap_err().code, "unsupported");
        assert!(!f.core.browser_bridge());

        let (_dir, home, data) = home_with(&["chrome"]);
        std::fs::create_dir_all(f.dir.path().join("run")).unwrap();
        let socket = f.dir.path().join("run").join("hexis").join("bridge.sock");
        f.bridge.set_env(Env {
            home: home.clone(),
            data_dir: data.clone(),
            socket: socket.clone(),
        });
        let s = f.bridge.status();
        assert!(s.supported && !s.enabled && !s.listening);
        assert_eq!(s.browsers.len(), BROWSERS.len());

        assert!(
            s.extension.is_none(),
            "no folder is named before the files are written"
        );
        let s = f.bridge.set_enabled(true).unwrap();
        assert!(s.enabled && s.listening && s.error.is_none());
        assert!(f.core.browser_bridge() && socket.exists());
        let folders = s.extension.expect("switching on writes the extension");
        assert!(Path::new(&folders.chromium).join("manifest.json").is_file());
        assert!(Path::new(&folders.firefox).join("manifest.json").is_file());

        // a request waits; switching off sends it to the browser's own flow and closes the socket
        let h = submit(
            &f.bridge,
            line(
                "swit-0001",
                "create",
                "https://example.com",
                &create_request("example.com", "ana", "preferred"),
            ),
        );
        front_of(&f.bridge);
        let s = f.bridge.set_enabled(false).unwrap();
        assert!(!s.enabled && !s.listening);
        assert!(!socket.exists() && !f.core.browser_bridge());
        assert_eq!(answer_of(h)["reason"], "disabled");

        // registering and removing go through the same places
        let s = f.bridge.register(&ids(&["chrome"])).unwrap();
        assert!(s
            .browsers
            .iter()
            .any(|b| b["id"] == "chrome" && b["registered"] == true));
        let s = f.bridge.unregister(&[]).unwrap();
        assert!(s.browsers.iter().all(|b| b["registered"] == false));
    }

    #[test]
    fn removing_hexis_from_the_device_leaves_nothing_in_any_browser() {
        let f = fx();
        let (_dir, home, data) = home_with(&["firefox", "vivaldi"]);
        std::fs::create_dir_all(f.dir.path().join("run")).unwrap();
        let socket = f.dir.path().join("run").join("hexis").join("bridge.sock");
        f.bridge.set_env(Env {
            home: home.clone(),
            data_dir: data.clone(),
            socket: socket.clone(),
        });
        f.bridge.set_enabled(true).unwrap();
        f.bridge.register(&ids(&["firefox", "vivaldi"])).unwrap();
        assert!(data.join("extension").is_dir());
        f.bridge.forget_browsers();
        assert!(!socket.exists() && !f.bridge.listening());
        assert!(install::status(&home, &data)
            .browsers
            .iter()
            .all(|b| !b.registered));
        assert!(!data.join(install::LAUNCHER_NAME).exists());
        assert!(
            !data.join("extension").exists(),
            "the extension's files go too"
        );
        assert!(!data.exists(), "and the folder, when nothing else is in it");
    }

    #[test]
    fn the_extension_files_and_the_app_agree_on_the_addresses() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("extension");
        let read = |name: &str| {
            std::fs::read_to_string(root.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
        };

        // the Chromium address is derived from the public key in the manifest
        let chromium = parsed(&read("manifest.chromium.json"));
        let key = STANDARD.decode(chromium["key"].as_str().unwrap()).unwrap();
        let id: String = Sha256::digest(&key)[..16]
            .iter()
            .flat_map(|b| [b >> 4, b & 15])
            .map(|n| char::from(b'a' + n))
            .collect();
        assert_eq!(id, install::CHROMIUM_EXTENSION_ID);

        let firefox = parsed(&read("manifest.firefox.json"));
        assert_eq!(
            firefox["browser_specific_settings"]["gecko"]["id"],
            install::FIREFOX_EXTENSION_ID
        );

        // the name the background script connects to is the one the manifests carry, and a valid one
        let background = read("src/background.js");
        assert!(background.contains(&format!("const HOST = \"{}\";", install::NATIVE_HOST_NAME)));
        assert!(install::NATIVE_HOST_NAME
            .split('.')
            .all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')));

        // both manifests ask for the same thing: the native messaging permission, and pages only on https (or localhost)
        for m in [&chromium, &firefox] {
            assert_eq!(m["permissions"], json!(["nativeMessaging"]));
            for script in m["content_scripts"].as_array().unwrap() {
                assert_eq!(
                    script["matches"],
                    json!(["https://*/*", "http://localhost/*"])
                );
                assert_eq!(script["all_frames"], false);
            }
        }
    }
}
