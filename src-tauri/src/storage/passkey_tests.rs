//! Passkeys and the scanner through [`Session`], the API the commands use (`ai-docs/PASSKEYS.md`).

use super::model::{ItemDraft, ItemType};
use super::session::passkeys::{AssertInput, Caller, RegisterInput, ScanChoice, ScanReply};
use super::session::{Session, SessionConfig, EDIT_TOKEN_TTL};
use super::vault::passkeys::Target;
use super::StorageError;
use crate::crypto::KdfParams;
use crate::passkey::b64;
use crate::passkey::engine::tests::rs_from_der;
use crate::passkey::PasskeyError;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const PW: &str = "correct horse battery staple";
const KEY: &str = "JBSWY3DPEHPK3PXP";
const KEY2: &str = "KRSXG5CTMVRXEZLU";

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
    s: Session,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let s = Session::new(dir.path().join("vault.db"), fast());
    s.create(PW.into()).unwrap();
    Fx { _dir: dir, s }
}

fn login(title: &str, fields: &[(&str, &str)]) -> ItemDraft {
    ItemDraft {
        item_type: ItemType::Login,
        title: title.into(),
        favorite: false,
        reprompt: false,
        tags: vec![],
        folder: None,
        fields: fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        custom_fields: vec![],
    }
}

fn create_json(rp: &str, user: &str, user_id: &str) -> String {
    format!(
        r#"{{"rp":{{"id":"{rp}","name":"{rp}"}},"user":{{"id":"{user_id}","name":"{user}","displayName":"{user}"}},
        "challenge":"Y2hhbGxlbmdl","pubKeyCredParams":[{{"type":"public-key","alg":-7}}]}}"#
    )
}

fn get_json(rp: &str) -> String {
    format!(r#"{{"challenge":"Y2hhbGxlbmdl","rpId":"{rp}","userVerification":"required"}}"#)
}

fn own(rp: &str) -> Caller {
    Caller::Own {
        origin: format!("https://{rp}"),
    }
}

fn no_network(_: &str) -> Result<String, PasskeyError> {
    panic!("no request to a website was expected")
}

fn register(
    f: &Fx,
    rp: &str,
    user: &str,
    uid: &str,
    target: Target,
) -> super::session::passkeys::Registered {
    f.s.passkey_register(
        &RegisterInput {
            request_json: create_json(rp, user, uid),
            caller: own(rp),
            user_verified: true,
            target,
        },
        None,
        &no_network,
    )
    .unwrap()
}

fn is_passkey_key(k: &str) -> bool {
    k == "passkey" || k.starts_with("passkey#")
}

// ------------------------------------------------------------------ the scanner

#[test]
fn a_code_for_an_email_that_has_a_login_suggests_that_login_first() {
    let f = fx();
    let ana =
        f.s.item_create(login(
            "GitHub",
            &[
                ("username", "ana@example.com"),
                ("url", "https://github.com"),
            ],
        ))
        .unwrap()
        .id;
    f.s.item_create(login("Other", &[("username", "bob@example.com")]))
        .unwrap();
    let uri = format!("otpauth://totp/GitHub:ana%40Example.com?secret={KEY}&issuer=GitHub");
    let ScanReply::Totp {
        issuer,
        account,
        suggestions,
        ..
    } = f.s.scan_classify(&uri, false).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (issuer.as_deref(), account.as_str()),
        (Some("GitHub"), "ana@Example.com")
    );
    assert_eq!(
        suggestions.len(),
        1,
        "only the login that shares something is suggested"
    );
    assert_eq!(suggestions[0].id, ana);
    assert_eq!(
        suggestions[0].reasons,
        ["account", "site", "name"],
        "email (any case), the address github.com, the title"
    );
    assert_eq!(suggestions[0].score, 200);
    assert!(!suggestions[0].has_this_code);
}

#[test]
fn a_code_with_no_matching_login_suggests_nothing_and_makes_a_login() {
    let f = fx();
    f.s.item_create(login("Bank", &[("username", "someone")]))
        .unwrap();
    let uri = format!("otpauth://totp/Acme:zoe%40example.com?secret={KEY}&issuer=Acme");
    let ScanReply::Totp { suggestions, .. } = f.s.scan_classify(&uri, false).unwrap() else {
        panic!()
    };
    assert!(suggestions.is_empty());
    let made =
        f.s.scan_apply(&uri, ScanChoice::New { title: None }, None)
            .unwrap();
    assert_eq!(made.title, "Acme");
    assert_eq!(made.subtitle.as_deref(), Some("zoe@example.com"));
    assert!(made.has_totp);
    // it can make a code, so the secret was kept
    assert_eq!(f.s.totp_now(&made.id, None).unwrap().code.len(), 6);
    // and now the same code is recognised as already saved
    let ScanReply::Totp { suggestions, .. } = f.s.scan_classify(&uri, false).unwrap() else {
        panic!()
    };
    assert!(suggestions[0].has_this_code && suggestions[0].id == made.id);
}

#[test]
fn a_code_is_added_to_the_chosen_login_in_the_next_free_slot_and_never_twice() {
    let f = fx();
    let id =
        f.s.item_create(login("GitHub", &[("username", "ana"), ("totp", KEY)]))
            .unwrap()
            .id;
    let uri = format!("otpauth://totp/GitHub:ana?secret={KEY2}&issuer=GitHub");
    let item =
        f.s.scan_apply(&uri, ScanChoice::Existing(id.clone()), None)
            .unwrap();
    assert_eq!(item.code_keys, ["totp", "totp#2"]);
    let again = f.s.scan_apply(&uri, ScanChoice::Existing(id.clone()), None);
    assert!(matches!(again, Err(StorageError::InvalidInput(m)) if m.contains("already has")));
    // a bare key typed in works too, and a link that is not a code does not
    assert!(f
        .s
        .scan_apply("https://example.com", ScanChoice::New { title: None }, None)
        .is_err());
    assert!(f
        .s
        .scan_apply("not a code", ScanChoice::Existing(id), None)
        .is_err());
}

#[test]
fn other_scans_are_named_and_nothing_is_saved() {
    let f = fx();
    assert_eq!(
        f.s.scan_classify("FIDO:/123", false).unwrap(),
        ScanReply::Fido
    );
    assert_eq!(
        f.s.scan_classify("otpauth-migration://offline?data=x", false)
            .unwrap(),
        ScanReply::Migration
    );
    assert_eq!(f.s.scan_classify("hello", false).unwrap(), ScanReply::Text);
    assert!(matches!(
        f.s.scan_classify("otpauth://totp/x?secret=!", false)
            .unwrap(),
        ScanReply::Invalid { .. }
    ));
    assert!(f.s.item_list().unwrap().is_empty());
}

#[test]
fn a_locked_vault_answers_locked_to_every_scan_and_passkey_call() {
    let dir = tempfile::tempdir().unwrap();
    let s = Session::new(dir.path().join("vault.db"), fast());
    assert!(matches!(
        s.scan_classify("hello", false),
        Err(StorageError::Locked)
    ));
    assert!(matches!(
        s.passkey_candidates(&get_json("example.com"), None),
        Err(StorageError::Locked)
    ));
}

// ------------------------------------------------------------------ passkeys

#[test]
fn a_new_passkey_makes_a_login_that_a_website_can_verify_a_sign_in_against() {
    let f = fx();
    let r = register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::New { title: None },
    );
    // the answer is a real registration
    let reg: Value = serde_json::from_str(&r.response_json).unwrap();
    let spki = b64::decode(reg["response"]["publicKey"].as_str().unwrap()).unwrap();
    let vk = VerifyingKey::from_sec1_bytes(&spki[26..]).unwrap();
    // the login holds it
    let view = f.s.item_get(&r.item.id).unwrap();
    assert_eq!(view.title, "example.com");
    assert!(r.item.has_passkey);
    let url = view.fields.iter().find(|x| x.key == "url").unwrap();
    assert_eq!(url.value.as_deref(), Some("https://example.com"));
    let user = view.fields.iter().find(|x| x.key == "username").unwrap();
    assert_eq!(user.value.as_deref(), Some("ana@example.com"));
    let pk = view.fields.iter().find(|x| x.key == "passkey").unwrap();
    assert!(pk.secret && pk.value.is_none() && pk.has_value);
    let info = pk.passkey.as_ref().unwrap();
    assert_eq!(
        (info.rp_id.as_str(), info.user_name.as_str()),
        ("example.com", "ana@example.com")
    );
    assert_eq!(info.credential_id, reg["id"].as_str().unwrap());

    // sign in
    let (_, found) =
        f.s.passkey_candidates(&get_json("example.com"), None)
            .unwrap();
    assert_eq!(found.len(), 1);
    let out =
        f.s.passkey_assert(
            &AssertInput {
                request_json: get_json("example.com"),
                caller: own("example.com"),
                user_verified: true,
                item_id: found[0].item_id.clone(),
                key: found[0].key.clone(),
            },
            None,
            &no_network,
        )
        .unwrap();
    let a: Value = serde_json::from_str(&out).unwrap();
    let auth = b64::decode(a["response"]["authenticatorData"].as_str().unwrap()).unwrap();
    let cd = b64::decode(a["response"]["clientDataJSON"].as_str().unwrap()).unwrap();
    let sig = b64::decode(a["response"]["signature"].as_str().unwrap()).unwrap();
    let mut signed = auth;
    signed.extend_from_slice(&Sha256::digest(&cd));
    assert!(vk
        .verify(&signed, &Signature::from_slice(&rs_from_der(&sig)).unwrap())
        .is_ok());
    assert_eq!(a["id"], reg["id"]);
}

#[test]
fn the_private_key_is_in_no_view_no_editable_copy_no_list_no_history_and_no_search_result() {
    let f = fx();
    let r = register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::New { title: None },
    );
    let key = f.s.test_vault(|v| {
        let rec = v.passkey_record(&r.item.id, "passkey").unwrap();
        rec.private_key.clone()
    });
    assert_eq!(key.len(), 43);
    let seen = [
        serde_json::to_string(&f.s.item_get(&r.item.id).unwrap()).unwrap(),
        serde_json::to_string(&f.s.item_list().unwrap()).unwrap(),
        serde_json::to_string(&f.s.item_reveal(&r.item.id, None).unwrap()).unwrap(),
        serde_json::to_string(&f.s.item_begin_edit(&r.item.id, None).unwrap().draft).unwrap(),
        serde_json::to_string(&f.s.history_list(&r.item.id).unwrap()).unwrap(),
        serde_json::to_string(&f.s.history_get(&r.item.id, 1, None).unwrap()).unwrap(),
        serde_json::to_string(&f.s.item_search("passkey example", true).unwrap()).unwrap(),
        r.response_json.clone(),
    ];
    for (i, text) in seen.iter().enumerate() {
        assert!(!text.contains(&key), "the key showed in view {i}");
    }
    // what the screen's editable copy holds instead
    let draft = f.s.item_begin_edit(&r.item.id, None).unwrap().draft;
    let marker: Value = serde_json::from_str(&draft.fields["passkey"]).unwrap();
    assert_eq!(marker["keep"], true);
    assert_eq!(marker["rpId"], "example.com");
    assert!(marker.get("privateKey").is_none());
}

#[test]
fn a_passkey_is_found_by_its_website_and_account_name() {
    let f = fx();
    register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::New {
            title: Some("My bank".into()),
        },
    );
    for q in ["example.com", "ana@example.com", "passkey"] {
        assert_eq!(f.s.item_search(q, false).unwrap().len(), 1, "{q}");
    }
}

#[test]
fn editing_a_login_keeps_its_passkey_and_the_screen_can_only_keep_or_drop_it() {
    let f = fx();
    let r = register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::New { title: None },
    );
    let id = r.item.id;
    // keep: the title changes, the passkey stays and still signs
    let b = f.s.item_begin_edit(&id, None).unwrap();
    let mut d = b.draft.clone();
    d.title = "Example account".into();
    f.s.item_update(&id, &b.token, b.version, d).unwrap();
    assert!(f
        .s
        .item_get(&id)
        .unwrap()
        .fields
        .iter()
        .any(|x| x.key == "passkey"));
    // the history names the field and nothing else
    let h = f.s.history_list(&id).unwrap();
    assert_eq!(h[0].changed_fields, ["title"]);

    // forge: a made-up passkey, or a marker of a passkey the item does not hold, is refused
    let b = f.s.item_begin_edit(&id, None).unwrap();
    for forged in [
        r#"{"keep":false,"credentialId":"AAAA"}"#.to_string(),
        r#"{"keep":true,"credentialId":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","rpId":"example.com","rpName":"x","userName":"u","displayName":"u","alg":-7,"createdAt":1}"#.to_string(),
        "plain text".to_string(),
    ] {
        let mut d = b.draft.clone();
        d.fields.insert("passkey".into(), forged);
        assert!(f.s.item_update(&id, &b.token, b.version, d).is_err());
    }
    // a second passkey slot with a real marker copied from the first is the same credential twice: refused
    let mut d = b.draft.clone();
    let m = d.fields["passkey"].clone();
    d.fields.insert("passkey#2".into(), m);
    assert!(f.s.item_update(&id, &b.token, b.version, d).is_err());

    // drop: the marker removed removes the passkey, and the history says so
    let mut d = b.draft.clone();
    d.fields.remove("passkey");
    let item = f.s.item_update(&id, &b.token, b.version, d).unwrap();
    assert!(!item.has_passkey);
    assert_eq!(
        f.s.history_list(&id).unwrap()[0].changed_fields,
        ["passkey"]
    );
    assert!(f
        .s
        .passkey_candidates(&get_json("example.com"), None)
        .unwrap()
        .1
        .is_empty());
}

#[test]
fn a_new_item_cannot_be_made_with_a_passkey_and_a_copy_does_not_share_one() {
    let f = fx();
    let r = register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::New { title: None },
    );
    let rec = f.s.test_vault(|v| {
        v.passkey_record(&r.item.id, "passkey")
            .unwrap()
            .to_json()
            .to_string()
    });
    let made_up = login("Forged", &[("passkey", rec.as_str())]);
    assert!(f.s.item_create(made_up).is_err());
    let copy = f.s.item_clone(&r.item.id, None).unwrap();
    assert!(
        !copy.has_passkey,
        "a copy would hold the same credential twice"
    );
    assert!(f
        .s
        .item_get(&r.item.id)
        .unwrap()
        .fields
        .iter()
        .any(|x| x.key == "passkey"));
}

#[test]
fn a_passkey_goes_on_an_existing_login_and_a_second_registration_of_the_same_account_replaces_it() {
    let f = fx();
    let id =
        f.s.item_create(login(
            "Example",
            &[
                ("username", "ana@example.com"),
                ("url", "https://www.example.com/login"),
            ],
        ))
        .unwrap()
        .id;
    // suggested because the account and the site match
    let (_, list) =
        f.s.passkey_suggest(
            &create_json("example.com", "ana@example.com", "dXNlci0x"),
            None,
        )
        .unwrap();
    assert_eq!(list[0].id, id);
    assert_eq!(list[0].reasons, ["account", "site", "name"]);
    let first = register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::Existing(id.clone()),
    );
    assert_eq!(first.item.id, id);
    let view = f.s.item_get(&id).unwrap();
    let url = view.fields.iter().find(|x| x.key == "url").unwrap();
    assert_eq!(
        url.value.as_deref(),
        Some("https://www.example.com/login"),
        "the address it had is kept as it was"
    );
    // the same account registers again (the site was asked to "replace"): same slot, new credential
    let second = register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::Existing(id.clone()),
    );
    let view = f.s.item_get(&id).unwrap();
    let keys: Vec<&str> = view
        .fields
        .iter()
        .filter(|x| is_passkey_key(&x.key))
        .map(|x| x.key.as_str())
        .collect();
    assert_eq!(keys, ["passkey"]);
    let a: Value = serde_json::from_str(&first.response_json).unwrap();
    let b: Value = serde_json::from_str(&second.response_json).unwrap();
    assert_ne!(a["id"], b["id"]);
    assert_eq!(
        view.fields
            .iter()
            .find(|x| x.key == "passkey")
            .unwrap()
            .passkey
            .as_ref()
            .unwrap()
            .credential_id,
        b["id"].as_str().unwrap()
    );
    // another account on the same site takes the next slot
    register(
        &f,
        "example.com",
        "work@example.com",
        "d29yay0x",
        Target::Existing(id.clone()),
    );
    assert_eq!(
        f.s.item_get(&id)
            .unwrap()
            .fields
            .iter()
            .filter(|x| is_passkey_key(&x.key))
            .count(),
        2
    );
    assert_eq!(
        f.s.passkey_candidates(&get_json("example.com"), None)
            .unwrap()
            .1
            .len(),
        2
    );
}

#[test]
fn a_site_that_lists_a_passkey_we_hold_is_told_so_and_nothing_is_saved() {
    let f = fx();
    let r = register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::New { title: None },
    );
    let held =
        f.s.passkey_candidates(&get_json("example.com"), None)
            .unwrap()
            .1[0]
            .info
            .credential_id
            .clone();
    let req = format!(
        r#"{{"rp":{{"id":"example.com","name":"x"}},"user":{{"id":"dXNlci0y","name":"b"}},"challenge":"Y2hh",
        "excludeCredentials":[{{"type":"public-key","id":"{held}"}}]}}"#
    );
    let err =
        f.s.passkey_register(
            &RegisterInput {
                request_json: req,
                caller: own("example.com"),
                user_verified: true,
                target: Target::New { title: None },
            },
            None,
            &no_network,
        )
        .unwrap_err();
    assert_eq!(err, StorageError::PasskeyExcluded);
    assert_eq!(err.code(), "passkey_excluded");
    assert_eq!(f.s.item_list().unwrap().len(), 1);
    let _ = r;
}

#[test]
fn a_login_that_asks_for_the_master_password_asks_before_it_signs_or_takes_a_passkey() {
    let f = fx();
    let id =
        f.s.item_create(login("Example", &[("username", "ana@example.com")]))
            .unwrap()
            .id;
    let b = f.s.item_begin_edit(&id, None).unwrap();
    let mut d = b.draft.clone();
    d.reprompt = true;
    f.s.item_update(&id, &b.token, b.version, d).unwrap();
    let input = RegisterInput {
        request_json: create_json("example.com", "ana@example.com", "dXNlci0x"),
        caller: own("example.com"),
        user_verified: true,
        target: Target::Existing(id.clone()),
    };
    assert_eq!(
        f.s.passkey_register(&input, None, &no_network).unwrap_err(),
        StorageError::PasswordRequired
    );
    assert_eq!(
        f.s.passkey_register(&input, Some("wrong password!".into()), &no_network)
            .unwrap_err(),
        StorageError::WrongPassword
    );
    f.s.passkey_register(&input, Some(PW.into()), &no_network)
        .unwrap();
    let sign = AssertInput {
        request_json: get_json("example.com"),
        caller: own("example.com"),
        user_verified: true,
        item_id: id,
        key: "passkey".into(),
    };
    assert_eq!(
        f.s.passkey_assert(&sign, None, &no_network).unwrap_err(),
        StorageError::PasswordRequired
    );
    assert!(f
        .s
        .passkey_assert(&sign, Some(PW.into()), &no_network)
        .is_ok());
}

#[test]
fn who_may_ask_is_checked_per_kind_of_caller() {
    let f = fx();
    // the app's own window must be on the website
    let bad = RegisterInput {
        request_json: create_json("example.com", "a", "dXNlci0x"),
        caller: Caller::Own {
            origin: "https://evil.example".into(),
        },
        user_verified: true,
        target: Target::New { title: None },
    };
    assert!(f.s.passkey_register(&bad, None, &no_network).is_err());
    assert!(f.s.item_list().unwrap().is_empty());

    // a browser brings its own hash, which is signed as given
    let hash = [7u8; 32];
    let browser = RegisterInput {
        request_json: create_json("example.com", "a", "dXNlci0x"),
        caller: Caller::Browser {
            origin: "https://example.com".into(),
            client_data_hash: hash,
        },
        user_verified: true,
        target: Target::New { title: None },
    };
    let r = f.s.passkey_register(&browser, None, &no_network).unwrap();
    let reg: Value = serde_json::from_str(&r.response_json).unwrap();
    let vk = VerifyingKey::from_sec1_bytes(
        &b64::decode(reg["response"]["publicKey"].as_str().unwrap()).unwrap()[26..],
    )
    .unwrap();
    let out =
        f.s.passkey_assert(
            &AssertInput {
                request_json: get_json("example.com"),
                caller: Caller::Browser {
                    origin: "https://example.com".into(),
                    client_data_hash: hash,
                },
                user_verified: true,
                item_id: r.item.id.clone(),
                key: "passkey".into(),
            },
            None,
            &no_network,
        )
        .unwrap();
    let a: Value = serde_json::from_str(&out).unwrap();
    let mut signed = b64::decode(a["response"]["authenticatorData"].as_str().unwrap()).unwrap();
    signed.extend_from_slice(&hash);
    let sig = b64::decode(a["response"]["signature"].as_str().unwrap()).unwrap();
    assert!(vk
        .verify(&signed, &Signature::from_slice(&rs_from_der(&sig)).unwrap())
        .is_ok());

    // a native app needs the website to list it (package and certificate)
    let cert = [9u8; 32];
    let fp = cert
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    let links_ok = move |_: &str| -> Result<String, PasskeyError> {
        Ok(format!(
            r#"[{{"relation":["delegate_permission/common.get_login_creds"],"target":{{"namespace":"android_app","package_name":"com.example.app","sha256_cert_fingerprints":["{fp}"]}}}}]"#
        ))
    };
    let app = |package: &str| RegisterInput {
        request_json: create_json("example.com", "app@example.com", "dXNlci0x"),
        caller: Caller::App {
            package: package.into(),
            cert_sha256: cert,
        },
        user_verified: true,
        target: Target::New { title: None },
    };
    assert!(
        f.s.passkey_register(&app("com.evil.app"), None, &links_ok)
            .is_err(),
        "another app"
    );
    let unreachable =
        |_: &str| -> Result<String, PasskeyError> { Err(PasskeyError::invalid("offline")) };
    assert!(
        f.s.passkey_register(&app("com.example.app"), None, &unreachable)
            .is_err(),
        "no answer from the website means no passkey"
    );
    let r =
        f.s.passkey_register(&app("com.example.app"), None, &links_ok)
            .unwrap();
    let reg: Value = serde_json::from_str(&r.response_json).unwrap();
    let cd: Value = serde_json::from_slice(
        &b64::decode(reg["response"]["clientDataJSON"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(cd["androidPackageName"], "com.example.app");
    assert_eq!(cd["origin"], crate::passkey::origin::app_origin(&cert));
}

#[test]
fn the_allow_list_and_the_website_are_enforced_when_signing() {
    let f = fx();
    let r = register(
        &f,
        "example.com",
        "ana@example.com",
        "dXNlci0x",
        Target::New { title: None },
    );
    let other = register(
        &f,
        "other.example",
        "ana@example.com",
        "dXNlci0x",
        Target::New { title: None },
    );
    // a passkey of another website never signs for this one
    let wrong_site = AssertInput {
        request_json: get_json("example.com"),
        caller: own("example.com"),
        user_verified: true,
        item_id: other.item.id.clone(),
        key: "passkey".into(),
    };
    assert!(f.s.passkey_assert(&wrong_site, None, &no_network).is_err());
    // an allow list that does not name it
    let req = r#"{"challenge":"Y2hh","rpId":"example.com","allowCredentials":[{"type":"public-key","id":"AAAAAAAAAAAAAAAAAAAAAA"}]}"#.to_string();
    let not_listed = AssertInput {
        request_json: req.clone(),
        caller: own("example.com"),
        user_verified: true,
        item_id: r.item.id.clone(),
        key: "passkey".into(),
    };
    assert!(f.s.passkey_assert(&not_listed, None, &no_network).is_err());
    assert!(f.s.passkey_candidates(&req, None).unwrap().1.is_empty());
    // a field that is not a passkey is not one
    let not_a_passkey = AssertInput {
        request_json: get_json("example.com"),
        caller: own("example.com"),
        user_verified: true,
        item_id: r.item.id,
        key: "username".into(),
    };
    assert!(f
        .s
        .passkey_assert(&not_a_passkey, None, &no_network)
        .is_err());
}

#[test]
fn a_login_holds_ten_passkeys_and_no_more() {
    let f = fx();
    let id =
        f.s.item_create(login("Example", &[("username", "ana")]))
            .unwrap()
            .id;
    for n in 0..10 {
        register(
            &f,
            "example.com",
            &format!("user{n}@example.com"),
            &b64::encode(format!("uid-{n}").as_bytes()),
            Target::Existing(id.clone()),
        );
    }
    let input = RegisterInput {
        request_json: create_json("example.com", "user10@example.com", &b64::encode(b"uid-10")),
        caller: own("example.com"),
        user_verified: true,
        target: Target::Existing(id.clone()),
    };
    let err = f.s.passkey_register(&input, None, &no_network).unwrap_err();
    assert!(matches!(err, StorageError::InvalidInput(m) if m.contains("10 passkeys")));
    assert_eq!(
        f.s.item_get(&id)
            .unwrap()
            .fields
            .iter()
            .filter(|x| is_passkey_key(&x.key))
            .count(),
        10
    );
}
