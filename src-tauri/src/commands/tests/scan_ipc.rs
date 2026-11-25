//! The scanner over real IPC dispatch: the argument and result shapes `src/backend/tauri.ts` uses.

use super::*;

const URI: &str = "otpauth://totp/GitHub:ana%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=GitHub";

#[test]
fn the_scanner_commands_work_over_ipc_with_the_shapes_the_front_end_sends() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    let mut d = draft("GitHub");
    d["fields"] = json!({"username": "ana@example.com", "url": "https://github.com"});
    let id = h.ok("item_create", json!({"draft": d}))["id"]
        .as_str()
        .unwrap()
        .to_string();

    // what is it, and where does it go: the login with the same email comes first
    let r = h.ok("scan_classify", json!({"text": URI}));
    assert_eq!(r["kind"], "totp");
    assert_eq!(r["issuer"], "GitHub");
    assert_eq!(r["account"], "ana@example.com");
    assert_eq!(r["suggestions"][0]["id"], id);
    assert_eq!(
        r["suggestions"][0]["reasons"],
        json!(["account", "site", "name"])
    );
    assert!(
        !r.to_string().contains("JBSWY3DPEHPK3PXP"),
        "the secret never comes back"
    );

    // other kinds
    assert_eq!(
        h.ok("scan_classify", json!({"text": "FIDO:/123"}))["kind"],
        "fido"
    );
    assert_eq!(
        h.ok("scan_classify", json!({"text": "https://a.example/x"}))["host"],
        "a.example"
    );
    assert_eq!(
        h.ok("scan_classify", json!({"text": "hello"}))["kind"],
        "text"
    );
    // a bare key counts only when the screen says it was typed or pasted
    assert_eq!(
        h.ok("scan_classify", json!({"text": "JBSWY3DPEHPK3PXP"}))["kind"],
        "text"
    );
    assert_eq!(
        h.ok(
            "scan_classify",
            json!({"text": "JBSWY3DPEHPK3PXP", "bareKey": true})
        )["kind"],
        "totp"
    );

    // apply: to the chosen login, then as a new login
    let item = h.ok(
        "scan_apply",
        json!({"text": URI, "choice": {"existing": id}}),
    );
    assert_eq!(item["hasTotp"], true);
    assert_eq!(item["codeKeys"], json!(["totp"]));
    let new = h.ok("scan_apply", json!({"text": URI.replace("JBSWY3DPEHPK3PXP", "KRSXG5CTMVRXEZLU"), "choice": {"new": {"title": null}}}));
    assert_eq!(new["title"], "GitHub");
    assert_eq!(
        h.err_code(
            "scan_apply",
            json!({"text": "hello", "choice": {"new": {"title": null}}})
        ),
        "invalid_input"
    );

    // a locked vault answers locked
    h.ok("vault_lock", json!({}));
    assert_eq!(h.err_code("scan_classify", json!({"text": URI})), "locked");
}
