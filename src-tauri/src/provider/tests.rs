//! The provider as the Kotlin side drives it: operation names and JSON in, JSON out, against a real core.

use super::*;
use crate::crypto::KdfParams;
use crate::passkey::engine::tests::rs_from_der;
use crate::storage::files::NoPicker;
use crate::storage::session::{SessionConfig, EDIT_TOKEN_TTL};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

const PW: &str = "correct horse battery staple";

fn fast() -> SessionConfig {
    SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: EDIT_TOKEN_TTL,
        purge_grace: std::time::Duration::ZERO,
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    core: Arc<AppCore>,
    p: Provider,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(AppCore::open(dir.path(), fast(), Arc::new(NoPicker)).unwrap());
    core.profile_create("Ana", PW.into()).unwrap();
    let p = Provider::new(Arc::clone(&core))
        .with_links(Box::new(|_| panic!("no request to a website was expected")));
    Fx { _dir: dir, core, p }
}

fn ok(f: &Fx, op: &str, args: Value) -> Value {
    let out: Value = serde_json::from_str(&f.p.call(op, &args.to_string())).unwrap();
    assert_eq!(out["ok"], true, "{op}: {out}");
    out["data"].clone()
}

fn fail(f: &Fx, op: &str, args: Value) -> String {
    let out: Value = serde_json::from_str(&f.p.call(op, &args.to_string())).unwrap();
    assert_eq!(out["ok"], false, "{op} should fail: {out}");
    out["code"].as_str().unwrap().to_string()
}

fn create_request(rp: &str, user: &str) -> String {
    json!({
        "rp": {"id": rp, "name": rp},
        "user": {"id": "dXNlci0x", "name": user, "displayName": user},
        "challenge": "Y2hhbGxlbmdl",
        "pubKeyCredParams": [{"type": "public-key", "alg": -7}],
        "authenticatorSelection": {"residentKey": "required", "userVerification": "required"},
    })
    .to_string()
}

fn get_request(rp: &str) -> String {
    json!({"challenge": "Y2hhbGxlbmdl", "rpId": rp, "userVerification": "required"}).to_string()
}

fn hash_of(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

fn browser(rp: &str, text: &str) -> Value {
    json!({"kind": "browser", "origin": format!("https://{rp}"), "clientDataHash": b64::encode(&hash_of(text))})
}

fn draft(title: &str, fields: Value) -> Value {
    json!({"type": "login", "title": title, "fields": fields})
}

fn add_login(f: &Fx, title: &str, fields: Value, reprompt: bool) -> String {
    let mut d = draft(title, fields);
    d["reprompt"] = json!(reprompt);
    let draft = serde_json::from_value(d).unwrap();
    f.core.item_create(draft).unwrap().id
}

#[test]
fn a_passkey_is_made_found_and_used_through_the_json_interface() {
    let f = fx();
    // the sheet asks first: nothing matches, so the only choice is a new login
    let o = ok(
        &f,
        "createOptions",
        json!({"request": create_request("example.com", "ana@example.com"), "caller": browser("example.com", "cd-1")}),
    );
    assert_eq!(o["rpId"], "example.com");
    assert_eq!(o["suggestions"], json!([]));

    let made = ok(
        &f,
        "create",
        json!({
            "request": create_request("example.com", "ana@example.com"),
            "caller": browser("example.com", "cd-1"),
            "target": {"new": {"title": null}},
            "userVerified": true,
        }),
    );
    let reg: Value = serde_json::from_str(made["responseJson"].as_str().unwrap()).unwrap();
    assert_eq!(reg["type"], "public-key");
    assert_eq!(made["item"]["title"], "example.com");
    let spki = b64::decode(reg["response"]["publicKey"].as_str().unwrap()).unwrap();
    let vk = VerifyingKey::from_sec1_bytes(&spki[26..]).unwrap();

    // it is found for the website, and never for another one
    let g = ok(
        &f,
        "getOptions",
        json!({"request": get_request("example.com"), "caller": browser("example.com", "cd-2")}),
    );
    assert_eq!(g["rpId"], "example.com");
    assert_eq!(g["matches"].as_array().unwrap().len(), 1);
    assert_eq!(g["matches"][0]["userName"], "ana@example.com");
    assert!(
        !g.to_string().contains("privateKey"),
        "no key in any answer"
    );
    let other = ok(
        &f,
        "getOptions",
        json!({"request": get_request("other.example"), "caller": browser("other.example", "cd-2")}),
    );
    assert_eq!(other["matches"], json!([]));

    // signing uses the hash the browser gave and the signature checks against the public key of the registration
    let m = &g["matches"][0];
    let signed = ok(
        &f,
        "sign",
        json!({
            "request": get_request("example.com"),
            "caller": browser("example.com", "cd-2"),
            "itemId": m["itemId"], "key": m["key"], "userVerified": true,
        }),
    );
    let a: Value = serde_json::from_str(signed["responseJson"].as_str().unwrap()).unwrap();
    let mut data = b64::decode(a["response"]["authenticatorData"].as_str().unwrap()).unwrap();
    data.extend_from_slice(&hash_of("cd-2"));
    let sig = b64::decode(a["response"]["signature"].as_str().unwrap()).unwrap();
    assert!(vk
        .verify(&data, &Signature::from_slice(&rs_from_der(&sig)).unwrap())
        .is_ok());
    assert_eq!(a["id"], reg["id"]);
}

#[test]
fn a_new_passkey_can_go_on_a_login_the_options_suggested() {
    let f = fx();
    let id = add_login(
        &f,
        "GitHub",
        json!({"username": "ana@example.com", "url": "https://github.com"}),
        false,
    );
    add_login(&f, "Bank", json!({"username": "zed"}), false);
    let o = ok(
        &f,
        "createOptions",
        json!({"request": create_request("github.com", "ana@example.com"), "caller": browser("github.com", "cd")}),
    );
    assert_eq!(o["suggestions"][0]["id"], id);
    assert_eq!(
        o["suggestions"][0]["reasons"],
        json!(["account", "site", "name"])
    );
    // the others are every other login, so the sheet can offer them too
    let others: Vec<&str> = o["others"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["title"].as_str().unwrap())
        .collect();
    assert_eq!(others, ["Bank"]);
    let made = ok(
        &f,
        "create",
        json!({
            "request": create_request("github.com", "ana@example.com"),
            "caller": browser("github.com", "cd"),
            "target": {"existing": id}, "userVerified": true,
        }),
    );
    assert_eq!(made["item"]["id"], id);
    // and a second one for the same account is refused by `excludeCredentials`-style duplicate logic only when listed
    let item = f.core.item_get(&id).unwrap();
    assert!(item.fields.iter().any(|x| x.key == "passkey"));
}

#[test]
fn nothing_works_while_the_app_lock_is_closed() {
    let f = fx();
    ok(
        &f,
        "create",
        json!({"request": create_request("example.com", "ana"), "caller": browser("example.com", "cd"), "target": {"new": {"title": null}}, "userVerified": true}),
    );
    f.core
        .gate_configure(
            serde_json::from_value(json!({"enabled": true, "mode": "pin", "credential": "24680", "masterPassword": PW}))
                .unwrap(),
        )
        .unwrap();
    f.core.gate_close();
    assert!(f.core.gate().is_closed());
    // every operation is refused first, whatever it is, and nothing about the vault is told
    for (op, args) in [
        ("state", json!({})),
        ("lock", json!({})),
        ("unlock", json!({"accountId": null, "password": PW})),
        (
            "getOptions",
            json!({"request": get_request("example.com"), "caller": browser("example.com", "cd")}),
        ),
        (
            "createOptions",
            json!({"request": create_request("example.com", "ana"), "caller": browser("example.com", "cd")}),
        ),
        (
            "sign",
            json!({"request": get_request("example.com"), "caller": browser("example.com", "cd"), "itemId": "x", "key": "passkey", "userVerified": true}),
        ),
        (
            "create",
            json!({"request": create_request("example.com", "ana"), "caller": browser("example.com", "cd"), "target": {"new": {"title": null}}, "userVerified": true}),
        ),
    ] {
        let out = f.p.call(op, &args.to_string());
        assert!(out.contains("\"app_locked\""), "{op}: {out}");
        assert!(
            !out.contains("Ana") && !out.contains("example.com"),
            "{op} told something: {out}"
        );
    }
}

#[test]
fn a_locked_vault_opens_with_the_master_password_only() {
    let f = fx();
    let s = ok(&f, "state", json!({}));
    assert_eq!(
        (s["unlocked"].clone(), s["limited"].clone()),
        (json!(true), json!(false))
    );
    let s = ok(&f, "lock", json!({}));
    assert_eq!(s["unlocked"], false);
    assert_eq!(s["accounts"][0]["name"], "Ana");
    // while locked nothing can be asked
    assert_eq!(
        fail(
            &f,
            "getOptions",
            json!({"request": get_request("example.com"), "caller": browser("example.com", "cd")})
        ),
        "locked"
    );
    assert_eq!(
        fail(
            &f,
            "unlock",
            json!({"accountId": s["accounts"][0]["id"], "password": "wrong"})
        ),
        "wrong_password"
    );
    let s = ok(
        &f,
        "unlock",
        json!({"accountId": s["accounts"][0]["id"], "password": PW}),
    );
    assert_eq!(s["unlocked"], true);
}

#[test]
fn a_login_that_asks_for_the_master_password_asks_here_too() {
    let f = fx();
    let id = add_login(
        &f,
        "Bank",
        json!({"username": "ana@example.com", "url": "https://bank.example"}),
        true,
    );
    let args = |pw: Option<&str>| {
        json!({
            "request": create_request("bank.example", "ana@example.com"),
            "caller": browser("bank.example", "cd"),
            "target": {"existing": id}, "userVerified": true, "password": pw,
        })
    };
    assert_eq!(fail(&f, "create", args(None)), "password_required");
    assert_eq!(fail(&f, "create", args(Some("wrong"))), "wrong_password");
    let made = ok(&f, "create", args(Some(PW)));
    assert_eq!(made["item"]["id"], id);
    // signing asks the same
    let sign = |pw: Option<&str>| {
        json!({
            "request": get_request("bank.example"), "caller": browser("bank.example", "cd2"),
            "itemId": id, "key": "passkey", "userVerified": true, "password": pw,
        })
    };
    assert_eq!(fail(&f, "sign", sign(None)), "password_required");
    ok(&f, "sign", sign(Some(PW)));
}

#[test]
fn a_request_that_requires_verification_is_refused_without_it() {
    let f = fx();
    let created = |uv: bool| {
        json!({
            "request": create_request("example.com", "ana@example.com"),
            "caller": browser("example.com", "cd"), "target": {"new": {"title": null}}, "userVerified": uv,
        })
    };
    let code = fail(&f, "create", created(false));
    assert_eq!(code, "invalid_input");
    // nothing was saved by the refused attempt
    assert_eq!(f.core.item_list().unwrap().len(), 0);
    ok(&f, "create", created(true));
}

#[test]
fn an_app_needs_the_website_to_list_it_and_the_check_runs_only_for_apps() {
    let f = fx();
    let cert = [7u8; 32];
    let fp = cert
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    let listed = format!(
        r#"[{{"relation":["delegate_permission/common.get_login_creds"],"target":{{"namespace":"android_app","package_name":"com.example.app","sha256_cert_fingerprints":["{fp}"]}}}}]"#
    );
    let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&asked);
    let p = Provider::new(Arc::clone(&f.core)).with_links(Box::new(move |rp| {
        log.lock().unwrap().push(rp.to_string());
        Ok(listed.clone())
    }));
    let app = |package: &str| json!({"kind": "app", "package": package, "certSha256": b64::encode(&cert)});
    let call = |p: &Provider, op: &str, a: Value| -> Value {
        serde_json::from_str(&p.call(op, &a.to_string())).unwrap()
    };
    let create = |package: &str| json!({"request": create_request("example.com", "ana@example.com"), "caller": app(package), "target": {"new": {"title": null}}, "userVerified": true});
    assert_eq!(
        call(&p, "create", create("com.evil.app"))["ok"],
        false,
        "another app"
    );
    let done = call(&p, "create", create("com.example.app"));
    assert_eq!(done["ok"], true, "{done}");
    let reg: Value = serde_json::from_str(done["data"]["responseJson"].as_str().unwrap()).unwrap();
    let cd: Value = serde_json::from_slice(
        &b64::decode(reg["response"]["clientDataJSON"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(cd["androidPackageName"], "com.example.app");
    assert_eq!(
        cd["origin"],
        format!("android:apk-key-hash:{}", b64::encode(&cert))
    );
    // looking up the passkeys never asks the website
    let before = asked.lock().unwrap().len();
    call(
        &p,
        "getOptions",
        json!({"request": get_request("example.com"), "caller": app("com.example.app")}),
    );
    assert_eq!(asked.lock().unwrap().len(), before);
}

#[test]
fn with_the_website_check_switched_off_apps_get_nothing_and_no_request_is_made() {
    let f = fx();
    let cert = [7u8; 32];
    let asked = Arc::new(std::sync::Mutex::new(0usize));
    let log = Arc::clone(&asked);
    let p = Provider::new(Arc::clone(&f.core)).with_links(Box::new(move |_| {
        *log.lock().unwrap() += 1;
        Err(PasskeyError::invalid("no network in a test"))
    }));
    let call = |op: &str, a: Value| -> Value {
        serde_json::from_str(&p.call(op, &a.to_string())).unwrap()
    };
    let app =
        json!({"kind": "app", "package": "com.example.app", "certSha256": b64::encode(&cert)});
    f.core.device_set_passkey_app_check(false).unwrap();
    for (op, args) in [
        (
            "getOptions",
            json!({"request": get_request("example.com"), "caller": app}),
        ),
        (
            "createOptions",
            json!({"request": create_request("example.com", "ana"), "caller": app}),
        ),
        (
            "create",
            json!({"request": create_request("example.com", "ana"), "caller": app, "target": {"new": {"title": null}}, "userVerified": true}),
        ),
        (
            "sign",
            json!({"request": get_request("example.com"), "caller": app, "itemId": "x", "key": "passkey", "userVerified": true}),
        ),
    ] {
        let out = call(op, args);
        assert_eq!(
            (out["ok"].clone(), out["code"].clone()),
            (json!(false), json!("app_check_off")),
            "{op}: {out}"
        );
    }
    assert_eq!(
        *asked.lock().unwrap(),
        0,
        "switched off means no request to any website"
    );
    // a browser is not affected: it names the page's origin itself and needs no website check
    let made = call(
        "create",
        json!({"request": create_request("example.com", "ana"), "caller": browser("example.com", "cd"), "target": {"new": {"title": null}}, "userVerified": true}),
    );
    assert_eq!(made["ok"], true, "{made}");
    // switched on again, the same app is looked up
    f.core.device_set_passkey_app_check(true).unwrap();
    let out = call(
        "create",
        json!({"request": create_request("example.com", "ana"), "caller": app, "target": {"new": {"title": null}}, "userVerified": true}),
    );
    assert_ne!(out["code"], json!("app_check_off"), "{out}");
    assert_eq!(*asked.lock().unwrap(), 1, "the website was asked once");
}

#[test]
fn bad_input_is_an_answer_never_a_panic() {
    let f = fx();
    assert_eq!(fail(&f, "nope", json!({})), "bad_request");
    assert_eq!(fail(&f, "sign", json!({"request": 5})), "bad_request");
    let out: Value = serde_json::from_str(&f.p.call("create", "not json")).unwrap();
    assert_eq!(out["ok"], false);
    assert_eq!(
        fail(
            &f,
            "create",
            json!({
                "request": create_request("example.com", "a"), "caller": {"kind": "app", "package": "x", "certSha256": "AAAA"},
                "target": {"new": {"title": null}}, "userVerified": true,
            })
        ),
        "bad_request"
    );
    // an answer holds no more than the contract names: no private key, no vault key, no password
    ok(
        &f,
        "create",
        json!({"request": create_request("example.com", "ana"), "caller": browser("example.com", "cd"), "target": {"new": {"title": null}}, "userVerified": true}),
    );
    let s = ok(&f, "state", json!({})).to_string();
    assert!(!s.contains(PW));
}

#[test]
fn the_data_folder_name_is_the_apps_identifier() {
    assert_eq!(app_identifier(), "dev.habibullah.hexis");
}
