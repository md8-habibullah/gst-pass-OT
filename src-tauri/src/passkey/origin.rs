//! Who may ask for a website's passkey.
//!
//! Three kinds of caller reach a passkey provider on Android:
//! * a **browser** that the system trusts to name the page's origin (the platform checks it against the allowlist of
//!   privileged apps, in Kotlin); the browser has already checked that the page may use the `rpId`, so only the
//!   `rpId` shape is checked here;
//! * a **native app**, which has no web origin: its origin is `android:apk-key-hash:<signing certificate hash>`, and the
//!   website must say the app may sign in for it, in its Digital Asset Links file ([`parse_asset_links`]);
//! * Hexis's **own window** (desktop), where the page's origin must belong to the `rpId` ([`rp_matches_origin`]).
//!
//! [`fetch_asset_links`] is the one network request the passkey code makes. It goes to the website's own domain, only
//! for a native app, only after the person chose a passkey for that app, with no redirects, a 2 second limit, a 64 KiB
//! limit and no cookies.

use super::{b64, PasskeyError};
use serde::Deserialize;
use std::time::Duration;

const MAX_LINKS_BYTES: u64 = 64 * 1024;

/// `https://login.example.com:8443` belongs to the `rpId` `example.com` (and `login.example.com`). The scheme must be
/// `https` (`http` only for `localhost`), without credentials in the address. This does not know the public suffix list:
/// a browser, which does, has already checked before it asks.
pub fn rp_matches_origin(rp_id: &str, origin: &str) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') || authority.is_empty() {
        return false;
    }
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let secure = scheme == "https" || (scheme == "http" && host == "localhost");
    secure
        && plain_host(&host)
        && super::record::check_rp_id(rp_id).is_ok()
        && (host == rp_id || host.ends_with(&format!(".{rp_id}")))
}

/// Only what a host name is made of. A parser that reads the address differently from this function (`evil.com\.example.com`
/// is the host `evil.com` to a browser) must not get an origin accepted here, so anything else is refused outright.
fn plain_host(host: &str) -> bool {
    !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

/// The host of an `https` origin, for requests that do not name an `rpId`.
pub fn host_of_origin(origin: &str) -> Option<String> {
    let rest = origin.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let host = authority.split(':').next()?.to_ascii_lowercase();
    plain_host(&host).then_some(host)
}

/// The origin Android gives a native app: `android:apk-key-hash:` and the base64url (no padding) SHA-256 of its
/// signing certificate.
pub fn app_origin(cert_sha256: &[u8; 32]) -> String {
    format!("android:apk-key-hash:{}", b64::encode(cert_sha256))
}

#[derive(Deserialize)]
struct Statement {
    #[serde(default)]
    relation: Vec<String>,
    target: Option<Target>,
}

#[derive(Deserialize)]
struct Target {
    namespace: Option<String>,
    package_name: Option<String>,
    #[serde(default)]
    sha256_cert_fingerprints: Vec<String>,
}

/// `AA:BB:…` (any case, with or without colons) as 32 bytes.
fn fingerprint_bytes(text: &str) -> Option<[u8; 32]> {
    let hex: String = text.chars().filter(|c| *c != ':').collect();
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Does this `assetlinks.json` let the app (`package`, signed with `cert_sha256`) sign in for the website?
/// Needs a statement with the relation `delegate_permission/common.get_login_creds` whose target is that package
/// with that certificate.
pub fn parse_asset_links(json: &str, package: &str, cert_sha256: &[u8; 32]) -> bool {
    let Ok(statements) = serde_json::from_str::<Vec<Statement>>(json) else {
        return false;
    };
    statements.iter().any(|s| {
        s.relation
            .iter()
            .any(|r| r == "delegate_permission/common.get_login_creds")
            && s.target.as_ref().is_some_and(|t| {
                t.namespace.as_deref() == Some("android_app")
                    && t.package_name.as_deref() == Some(package)
                    && t.sha256_cert_fingerprints
                        .iter()
                        .any(|f| fingerprint_bytes(f).as_ref() == Some(cert_sha256))
            })
    })
}

/// A name that can be a website on the public internet: at least two labels, a last label of letters only (so not a number
/// in disguise) and not one of the names reserved for local use.
fn looks_public(rp_id: &str) -> bool {
    const LOCAL: [&str; 8] = [
        "local",
        "localhost",
        "internal",
        "intranet",
        "lan",
        "home",
        "corp",
        "arpa",
    ];
    let mut labels = rp_id.rsplit('.');
    let last = labels.next().unwrap_or("");
    labels.next().is_some()
        && !last.is_empty()
        && last.bytes().all(|b| b.is_ascii_lowercase())
        && !LOCAL.contains(&last)
}

/// Download `https://<rp_id>/.well-known/assetlinks.json`. See the module notes for its limits.
pub fn fetch_asset_links(rp_id: &str) -> Result<String, PasskeyError> {
    super::record::check_rp_id(rp_id)?;
    if !looks_public(rp_id) {
        // An app may name any `rpId`; one that points at this phone or its network would make Hexis knock on a private door.
        return Err(PasskeyError::invalid(
            "That website name is not an address on the public internet.",
        ));
    }
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .https_only(true)
            .timeout_resolve(Some(Duration::from_secs(2)))
            .timeout_connect(Some(Duration::from_secs(2)))
            .timeout_global(Some(Duration::from_secs(2)))
            .max_response_header_size(16 * 1024)
            .build(),
    );
    let url = format!("https://{rp_id}/.well-known/assetlinks.json");
    let mut res = agent.get(&url).call().map_err(|_| {
        PasskeyError::invalid("The website could not be reached to check this app.")
    })?;
    if res.status().as_u16() != 200 {
        return Err(PasskeyError::invalid(
            "The website does not say that this app may use its passkeys.",
        ));
    }
    res.body_mut()
        .with_config()
        .limit(MAX_LINKS_BYTES)
        .read_to_string()
        .map_err(|_| PasskeyError::invalid("The website's app list could not be read."))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "14:6D:E9:83:C5:73:06:50:D8:EE:B9:95:2F:34:FC:64:16:A0:83:42:E6:1D:BE:A8:8A:04:96:B2:3F:CF:44:E5";

    fn cert() -> [u8; 32] {
        fingerprint_bytes(FP).unwrap()
    }

    #[test]
    fn an_origin_only_a_different_parser_would_read_as_this_site_is_refused() {
        for evil in [
            "https://evil.com\\.example.com",
            "https://evil.com\\@example.com",
            "https://exa mple.com",
            "https://example.com\t.example.com",
            "https://ex\u{e4}mple.com",
            "https://[::1]",
        ] {
            assert!(!rp_matches_origin("example.com", evil), "{evil:?}");
            assert_eq!(host_of_origin(evil), None, "{evil:?}");
        }
        assert!(rp_matches_origin(
            "example.com",
            "https://login.example.com"
        ));
        assert_eq!(
            host_of_origin("https://Login.Example.com:8443/x").as_deref(),
            Some("login.example.com")
        );
    }

    #[test]
    fn only_a_public_looking_name_is_ever_asked_for_its_app_list() {
        for no in [
            "localhost",
            "router",
            "nas.local",
            "printer.lan",
            "x.internal",
            "intranet.corp",
            "1.2.3.4",
            "a.b.123",
            "foo.arpa",
        ] {
            assert!(!looks_public(no), "{no}");
            // refused before any request is made
            if super::super::record::check_rp_id(no).is_ok() {
                let err = fetch_asset_links(no).unwrap_err();
                assert!(err.to_string().contains("public internet"), "{no}: {err:?}");
            }
        }
        for yes in ["example.com", "login.example.co.uk", "a-b.example.dev"] {
            assert!(looks_public(yes), "{yes}");
        }
    }

    #[test]
    fn origins_belong_to_their_own_site_and_its_subdomains_only() {
        assert!(rp_matches_origin("example.com", "https://example.com"));
        assert!(rp_matches_origin(
            "example.com",
            "https://login.example.com:8443/a?b#c"
        ));
        assert!(rp_matches_origin(
            "login.example.com",
            "https://login.example.com"
        ));
        assert!(rp_matches_origin("localhost", "http://localhost:1420"));
        for no in [
            ("example.com", "https://notexample.com"),
            ("example.com", "https://example.com.evil.net"),
            ("example.com", "http://example.com"),
            ("example.com", "https://user@example.com"),
            ("example.com", "https://evil.net/example.com"),
            ("login.example.com", "https://example.com"),
            ("example.com", "example.com"),
            ("example.com", "https://"),
            ("example.com", "android:apk-key-hash:abc"),
            ("Example.com", "https://example.com"),
        ] {
            assert!(!rp_matches_origin(no.0, no.1), "{no:?}");
        }
    }

    #[test]
    fn the_host_of_an_origin() {
        assert_eq!(
            host_of_origin("https://Login.Example.com:8443/x").as_deref(),
            Some("login.example.com")
        );
        assert_eq!(host_of_origin("http://example.com"), None);
        assert_eq!(host_of_origin("https://a@example.com"), None);
    }

    #[test]
    fn an_app_origin_is_the_certificate_hash_in_base64url() {
        let o = app_origin(&cert());
        assert!(o.starts_with("android:apk-key-hash:"));
        assert_eq!(
            b64::decode(&o["android:apk-key-hash:".len()..]).unwrap(),
            cert()
        );
    }

    fn links(relation: &str, package: &str, fp: &str) -> String {
        format!(
            r#"[{{"relation":["{relation}"],"target":{{"namespace":"android_app","package_name":"{package}","sha256_cert_fingerprints":["{fp}"]}}}}]"#
        )
    }

    #[test]
    fn asset_links_must_name_this_app_and_this_certificate_for_sign_in() {
        let rel = "delegate_permission/common.get_login_creds";
        assert!(parse_asset_links(
            &links(rel, "com.example.app", FP),
            "com.example.app",
            &cert()
        ));
        assert!(parse_asset_links(
            &links(rel, "com.example.app", &FP.to_lowercase()),
            "com.example.app",
            &cert()
        ));
        // another app, another certificate, another relation, a broken file
        assert!(!parse_asset_links(
            &links(rel, "com.example.app", FP),
            "com.evil.app",
            &cert()
        ));
        assert!(!parse_asset_links(
            &links(rel, "com.example.app", FP),
            "com.example.app",
            &[1u8; 32]
        ));
        assert!(!parse_asset_links(
            &links(
                "delegate_permission/common.handle_all_urls",
                "com.example.app",
                FP
            ),
            "com.example.app",
            &cert()
        ));
        assert!(!parse_asset_links(
            &links(rel, "com.example.app", "AB:CD"),
            "com.example.app",
            &cert()
        ));
        assert!(!parse_asset_links("not json", "com.example.app", &cert()));
        assert!(!parse_asset_links("[]", "com.example.app", &cert()));
        assert!(!parse_asset_links(
            r#"[{"relation":[],"target":null}]"#,
            "com.example.app",
            &cert()
        ));
    }
}
