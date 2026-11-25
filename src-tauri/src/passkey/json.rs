//! The WebAuthn JSON form of a request and its answer: what Android's Credential Manager hands to a provider
//! (`CreatePublicKeyCredentialRequest.requestJson`, `GetPublicKeyCredentialOption.requestJson`) and takes back.
//!
//! Only what the authenticator needs is read; every other key of the request is ignored. All binary values are
//! base64url without padding.

use super::engine::{Assertion, Made};
use super::record::{check_rp_id, PasskeyRecord, MAX_USER_HANDLE};
use super::{b64, PasskeyError};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Largest request JSON read (browsers send well under 4 KiB; this only stops absurd input).
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_LIST: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    Required,
    Preferred,
    Discouraged,
}

impl Verification {
    fn of(text: Option<&str>) -> Self {
        match text {
            Some("required") => Self::Required,
            Some("discouraged") => Self::Discouraged,
            _ => Self::Preferred,
        }
    }
}

#[derive(Deserialize)]
struct RpIn {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserIn {
    id: String,
    name: String,
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct ParamIn {
    alg: i64,
}

#[derive(Deserialize)]
struct CredIn {
    id: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SelectionIn {
    user_verification: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreationIn {
    rp: RpIn,
    user: UserIn,
    challenge: String,
    #[serde(default)]
    pub_key_cred_params: Vec<ParamIn>,
    #[serde(default)]
    exclude_credentials: Vec<CredIn>,
    #[serde(default)]
    authenticator_selection: SelectionIn,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestIn {
    challenge: String,
    rp_id: Option<String>,
    #[serde(default)]
    allow_credentials: Vec<CredIn>,
    user_verification: Option<String>,
}

/// A request to make a passkey, read and checked.
#[derive(Debug, Clone)]
pub struct CreateRequest {
    pub rp_id: String,
    pub rp_name: String,
    pub user_handle: Vec<u8>,
    pub user_name: String,
    pub display_name: String,
    pub algorithms: Vec<i64>,
    pub exclude: Vec<Vec<u8>>,
    /// The challenge exactly as the site spelled it (it goes back inside the client data).
    pub challenge: String,
    pub verification: Verification,
}

/// A request to sign in with a passkey, read and checked.
#[derive(Debug, Clone)]
pub struct GetRequest {
    pub rp_id: String,
    pub allow: Vec<Vec<u8>>,
    pub challenge: String,
    pub verification: Verification,
}

fn too_big(json: &str) -> Result<(), PasskeyError> {
    if json.len() > MAX_REQUEST_BYTES {
        return Err(PasskeyError::invalid("The passkey request is too large."));
    }
    Ok(())
}

fn ids(list: &[CredIn]) -> Result<Vec<Vec<u8>>, PasskeyError> {
    if list.len() > MAX_LIST {
        return Err(PasskeyError::invalid(
            "The passkey request lists too many passkeys.",
        ));
    }
    list.iter().map(|c| b64::decode(&c.id)).collect()
}

fn check_challenge(c: &str) -> Result<(), PasskeyError> {
    let bytes = b64::decode(c)?;
    if bytes.is_empty() || bytes.len() > 1024 {
        return Err(PasskeyError::invalid(
            "The passkey request has an invalid challenge.",
        ));
    }
    Ok(())
}

/// Read a creation request. `origin_host` is the host of the caller's origin (a browser tab); a request that names no
/// `rp.id` falls back to it, as the web platform does.
pub fn parse_create(json: &str, origin_host: Option<&str>) -> Result<CreateRequest, PasskeyError> {
    too_big(json)?;
    let r: CreationIn = serde_json::from_str(json)
        .map_err(|_| PasskeyError::invalid("The passkey request could not be read."))?;
    let rp_id = match (&r.rp.id, origin_host) {
        (Some(id), _) => id.to_ascii_lowercase(),
        (None, Some(host)) => host.to_ascii_lowercase(),
        (None, None) => {
            return Err(PasskeyError::invalid(
                "The passkey request names no website.",
            ))
        }
    };
    check_rp_id(&rp_id)?;
    check_challenge(&r.challenge)?;
    let user_handle = b64::decode(&r.user.id)?;
    if user_handle.is_empty() || user_handle.len() > MAX_USER_HANDLE {
        return Err(PasskeyError::invalid(
            "The passkey request has an invalid user id.",
        ));
    }
    if r.pub_key_cred_params.len() > MAX_LIST {
        return Err(PasskeyError::invalid(
            "The passkey request lists too many algorithms.",
        ));
    }
    let user_name = r.user.name;
    let display_name = r
        .user
        .display_name
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| user_name.clone());
    Ok(CreateRequest {
        rp_name: r
            .rp
            .name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| rp_id.clone()),
        rp_id,
        user_handle,
        user_name,
        display_name,
        algorithms: r.pub_key_cred_params.iter().map(|p| p.alg).collect(),
        exclude: ids(&r.exclude_credentials)?,
        challenge: r.challenge,
        verification: Verification::of(r.authenticator_selection.user_verification.as_deref()),
    })
}

/// Read a sign-in request.
pub fn parse_get(json: &str, origin_host: Option<&str>) -> Result<GetRequest, PasskeyError> {
    too_big(json)?;
    let r: RequestIn = serde_json::from_str(json)
        .map_err(|_| PasskeyError::invalid("The passkey request could not be read."))?;
    let rp_id = match (&r.rp_id, origin_host) {
        (Some(id), _) => id.to_ascii_lowercase(),
        (None, Some(host)) => host.to_ascii_lowercase(),
        (None, None) => {
            return Err(PasskeyError::invalid(
                "The passkey request names no website.",
            ))
        }
    };
    check_rp_id(&rp_id)?;
    check_challenge(&r.challenge)?;
    Ok(GetRequest {
        rp_id,
        allow: ids(&r.allow_credentials)?,
        challenge: r.challenge,
        verification: Verification::of(r.user_verification.as_deref()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ceremony {
    Create,
    Get,
}

/// What the client data is, or how it is known.
#[derive(Debug, Clone)]
pub enum ClientData {
    /// Built here from the origin and the challenge (an app, or the app's own window).
    Built {
        origin: String,
        cross_origin: bool,
        android_package: Option<String>,
    },
    /// The system already hashed it (a browser asked on behalf of a tab). The answer then carries a placeholder text.
    Hash([u8; 32]),
}

/// The client data JSON a client writes (WebAuthn §5.8.1).
pub fn client_data_json(
    ceremony: Ceremony,
    challenge: &str,
    origin: &str,
    cross_origin: bool,
    android_package: Option<&str>,
) -> String {
    let mut v = json!({
        "type": match ceremony { Ceremony::Create => "webauthn.create", Ceremony::Get => "webauthn.get" },
        "challenge": challenge,
        "origin": origin,
        "crossOrigin": cross_origin,
    });
    if let Some(p) = android_package {
        v["androidPackageName"] = json!(p);
    }
    v.to_string()
}

/// The pair the authenticator needs and the answer carries: the hash to sign over, and the client data text to return.
pub fn resolve_client_data(
    ceremony: Ceremony,
    challenge: &str,
    data: &ClientData,
) -> ([u8; 32], String) {
    match data {
        ClientData::Built {
            origin,
            cross_origin,
            android_package,
        } => {
            let text = client_data_json(
                ceremony,
                challenge,
                origin,
                *cross_origin,
                android_package.as_deref(),
            );
            (
                Sha256::digest(text.as_bytes()).into(),
                b64::encode(text.as_bytes()),
            )
        }
        // A privileged caller replaces this text itself; it only has to be there.
        ClientData::Hash(h) => (*h, b64::encode(b"placeholder")),
    }
}

/// The answer to a creation request (`RegistrationResponseJSON`).
pub fn registration_response(made: &Made, client_data_b64: &str) -> String {
    let id = made.record.credential_id.clone();
    json!({
        "id": id,
        "rawId": id,
        "type": "public-key",
        "authenticatorAttachment": "platform",
        "response": {
            "clientDataJSON": client_data_b64,
            "attestationObject": b64::encode(&made.attestation_object),
            "authenticatorData": b64::encode(&made.auth_data),
            "transports": ["internal", "hybrid"],
            "publicKeyAlgorithm": made.record.alg,
            "publicKey": b64::encode(&made.public_key),
        },
        "clientExtensionResults": { "credProps": { "rk": true } },
    })
    .to_string()
}

/// The answer to a sign-in request (`AuthenticationResponseJSON`).
pub fn assertion_response(record: &PasskeyRecord, a: &Assertion, client_data_b64: &str) -> String {
    let id = record.credential_id.clone();
    json!({
        "id": id,
        "rawId": id,
        "type": "public-key",
        "authenticatorAttachment": "platform",
        "response": {
            "clientDataJSON": client_data_b64,
            "authenticatorData": b64::encode(&a.auth_data),
            "signature": b64::encode(&a.signature),
            "userHandle": record.user_handle,
        },
        "clientExtensionResults": {},
    })
    .to_string()
}

/// Make the whole answer for a creation request in one step: choose, make, answer. The caller stores `made.record`
/// before it hands the answer back.
pub fn make_for(
    req: &CreateRequest,
    data: &ClientData,
    user_verified: bool,
    now_ms: u64,
) -> Result<(Made, String), PasskeyError> {
    if req.verification == Verification::Required && !user_verified {
        return Err(PasskeyError::invalid(
            "This website needs you to verify it is you.",
        ));
    }
    // "none" attestation signs nothing, so only the client data text goes into the answer
    let (_hash, text) = resolve_client_data(Ceremony::Create, &req.challenge, data);
    let made = super::engine::make_credential(&super::engine::MakeParams {
        rp_id: &req.rp_id,
        rp_name: &req.rp_name,
        user_handle: &req.user_handle,
        user_name: &req.user_name,
        display_name: &req.display_name,
        algorithms: &req.algorithms,
        user_verified,
        now_ms,
    })?;
    let answer = registration_response(&made, &text);
    Ok((made, answer))
}

/// Sign in with `record` for `req` and write the answer.
pub fn sign_for(
    record: &PasskeyRecord,
    req: &GetRequest,
    data: &ClientData,
    user_verified: bool,
) -> Result<String, PasskeyError> {
    if record.rp_id != req.rp_id {
        return Err(PasskeyError::invalid(
            "That passkey belongs to another website.",
        ));
    }
    if req.verification == Verification::Required && !user_verified {
        return Err(PasskeyError::invalid(
            "This website needs you to verify it is you.",
        ));
    }
    let (hash, text) = resolve_client_data(Ceremony::Get, &req.challenge, data);
    let a = super::engine::get_assertion(record, &hash, user_verified)?;
    Ok(assertion_response(record, &a, &text))
}

/// Used by tests and by the matching code: is this a value of the JSON shape this module writes?
pub fn is_json_object(text: &str) -> bool {
    matches!(serde_json::from_str::<Value>(text), Ok(Value::Object(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passkey::cbor::read::parse;
    use crate::passkey::engine::tests::{rp_check_registration, rs_from_der};
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::Signature;

    const CREATE: &str = r#"{
        "rp": {"id": "example.com", "name": "Example"},
        "user": {"id": "dXNlci0x", "name": "ana@example.com", "displayName": "Ana"},
        "challenge": "Y2hhbGxlbmdl",
        "pubKeyCredParams": [{"type": "public-key", "alg": -8}, {"type": "public-key", "alg": -7}],
        "timeout": 1800000,
        "excludeCredentials": [{"type": "public-key", "id": "AQIDBA"}],
        "authenticatorSelection": {"residentKey": "required", "userVerification": "required"},
        "attestation": "none",
        "extensions": {"credProps": true, "prf": {}}
    }"#;
    const GET: &str = r#"{"challenge": "Y2hhbGxlbmdl", "rpId": "example.com", "timeout": 1800000,
        "userVerification": "preferred", "allowCredentials": []}"#;

    fn origin() -> ClientData {
        ClientData::Built {
            origin: "https://example.com".into(),
            cross_origin: false,
            android_package: None,
        }
    }

    #[test]
    fn a_creation_request_is_read() {
        let r = parse_create(CREATE, None).unwrap();
        assert_eq!(r.rp_id, "example.com");
        assert_eq!(r.rp_name, "Example");
        assert_eq!(r.user_handle, b"user-1");
        assert_eq!(r.algorithms, vec![-8, -7]);
        assert_eq!(r.exclude, vec![vec![1, 2, 3, 4]]);
        assert_eq!(r.verification, Verification::Required);
        assert_eq!(r.challenge, "Y2hhbGxlbmdl");
    }

    #[test]
    fn missing_or_bad_parts_are_refused() {
        for bad in [
            "{}",
            "[]",
            r#"{"rp":{"id":"example.com"},"user":{"id":"","name":"a"},"challenge":"Y2hh"}"#,
            r#"{"rp":{"id":"example.com"},"user":{"id":"AQ","name":"a"},"challenge":""}"#,
            r#"{"rp":{"id":"EXAMPLE COM"},"user":{"id":"AQ","name":"a"},"challenge":"Y2hh"}"#,
            r#"{"rp":{"id":"example.com"},"user":{"id":"A+/=","name":"a"},"challenge":"Y2hh"}"#,
        ] {
            assert!(parse_create(bad, None).is_err(), "{bad}");
        }
        assert!(parse_create(&"x".repeat(MAX_REQUEST_BYTES + 1), None).is_err());
        // no rp id and no origin host: nothing to bind the passkey to
        let no_rp = r#"{"rp":{"name":"x"},"user":{"id":"AQ","name":"a"},"challenge":"Y2hh"}"#;
        assert!(parse_create(no_rp, None).is_err());
        assert_eq!(
            parse_create(no_rp, Some("Login.Example.com"))
                .unwrap()
                .rp_id,
            "login.example.com"
        );
    }

    #[test]
    fn a_full_round_trip_verifies_like_a_website_would() {
        let req = parse_create(CREATE, None).unwrap();
        let (made, answer) = make_for(&req, &origin(), true, 1_700_000_000_000).unwrap();
        // the website reads the answer
        let v: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(v["type"], "public-key");
        assert_eq!(v["id"], v["rawId"]);
        assert_eq!(v["authenticatorAttachment"], "platform");
        assert_eq!(v["clientExtensionResults"]["credProps"]["rk"], true);
        assert_eq!(v["response"]["publicKeyAlgorithm"], -7);
        let cd = b64::decode(v["response"]["clientDataJSON"].as_str().unwrap()).unwrap();
        let cd_v: Value = serde_json::from_slice(&cd).unwrap();
        assert_eq!(cd_v["type"], "webauthn.create");
        assert_eq!(cd_v["challenge"], "Y2hhbGxlbmdl");
        assert_eq!(cd_v["origin"], "https://example.com");
        let att = b64::decode(v["response"]["attestationObject"].as_str().unwrap()).unwrap();
        parse(&att); // well-formed CBOR, exactly consumed
        let (id, vk, _) = rp_check_registration(&made, "example.com");
        assert_eq!(b64::encode(&id), v["id"].as_str().unwrap());

        // sign in with it
        let get = parse_get(GET, None).unwrap();
        let out = sign_for(&made.record, &get, &origin(), true).unwrap();
        let a: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(a["id"], v["id"]);
        assert_eq!(a["response"]["userHandle"], "dXNlci0x");
        let auth = b64::decode(a["response"]["authenticatorData"].as_str().unwrap()).unwrap();
        let sig = b64::decode(a["response"]["signature"].as_str().unwrap()).unwrap();
        let cd = b64::decode(a["response"]["clientDataJSON"].as_str().unwrap()).unwrap();
        let cd_v: Value = serde_json::from_slice(&cd).unwrap();
        assert_eq!(cd_v["type"], "webauthn.get");
        let mut signed = auth.clone();
        signed.extend_from_slice(&Sha256::digest(&cd));
        let sig = Signature::from_slice(&rs_from_der(&sig)).unwrap();
        assert!(vk.verify(&signed, &sig).is_ok());
    }

    #[test]
    fn a_hash_from_the_system_is_signed_as_given() {
        let req = parse_create(CREATE, None).unwrap();
        let hash = [0xabu8; 32];
        let (made, answer) = make_for(&req, &ClientData::Hash(hash), true, 1).unwrap();
        let v: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(
            b64::decode(v["response"]["clientDataJSON"].as_str().unwrap()).unwrap(),
            b"placeholder"
        );
        let get = parse_get(GET, None).unwrap();
        let out = sign_for(&made.record, &get, &ClientData::Hash(hash), true).unwrap();
        let a: Value = serde_json::from_str(&out).unwrap();
        let (_, vk, _) = rp_check_registration(&made, "example.com");
        let auth = b64::decode(a["response"]["authenticatorData"].as_str().unwrap()).unwrap();
        let sig = b64::decode(a["response"]["signature"].as_str().unwrap()).unwrap();
        let mut signed = auth;
        signed.extend_from_slice(&hash);
        assert!(vk
            .verify(&signed, &Signature::from_slice(&rs_from_der(&sig)).unwrap())
            .is_ok());
    }

    #[test]
    fn android_apps_get_their_package_in_the_client_data() {
        let t = client_data_json(
            Ceremony::Create,
            "Y2hh",
            "android:apk-key-hash:abc",
            false,
            Some("com.example.app"),
        );
        let v: Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["androidPackageName"], "com.example.app");
        assert_eq!(v["origin"], "android:apk-key-hash:abc");
        assert_eq!(v["crossOrigin"], false);
    }

    #[test]
    fn verification_and_other_sites_are_enforced() {
        let req = parse_create(CREATE, None).unwrap();
        assert!(
            make_for(&req, &origin(), false, 1).is_err(),
            "required but not verified"
        );
        let (made, _) = make_for(&req, &origin(), true, 1).unwrap();
        let mut other = parse_get(GET, None).unwrap();
        other.rp_id = "evil.example".into();
        assert!(sign_for(&made.record, &other, &origin(), true).is_err());
        let mut get = parse_get(GET, None).unwrap();
        get.verification = Verification::Required;
        assert!(sign_for(&made.record, &get, &origin(), false).is_err());
        assert!(sign_for(&made.record, &get, &origin(), true).is_ok());
    }

    #[test]
    fn a_site_that_wants_only_other_algorithms_is_told_so() {
        let only_rsa = CREATE.replace(
            r#"{"type": "public-key", "alg": -8}, {"type": "public-key", "alg": -7}"#,
            r#"{"type": "public-key", "alg": -257}"#,
        );
        let req = parse_create(&only_rsa, None).unwrap();
        assert!(matches!(
            make_for(&req, &origin(), true, 1),
            Err(PasskeyError::Unsupported(_))
        ));
    }
}
