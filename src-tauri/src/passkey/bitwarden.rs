//! Bitwarden's `fido2Credentials` entry of a login <-> a stored passkey ([`PasskeyRecord`]).
//!
//! Bitwarden keeps the private key as a PKCS#8 document in base64 and the ids as text: a credential id is a GUID when the
//! raw id is 16 bytes, base64url otherwise. Only ECDSA over P-256 can be kept here (the one kind Hexis makes and uses); a
//! credential of another kind (RSA, Ed25519) is reported by the importer and left out.

use super::record::PasskeyRecord;
use super::{b64, PasskeyError, ALG_ES256};
use p256::ecdsa::SigningKey;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use zeroize::Zeroizing;

/// The fields of Bitwarden's passkey entry that matter, all as text (Bitwarden writes numbers and booleans as text too).
pub struct BwCredential<'a> {
    pub credential_id: &'a str,
    pub key_type: &'a str,
    pub key_algorithm: &'a str,
    pub key_curve: &'a str,
    pub key_value: &'a str,
    pub rp_id: &'a str,
    pub rp_name: &'a str,
    pub user_handle: &'a str,
    pub user_name: &'a str,
    pub user_display_name: &'a str,
    /// Unix milliseconds, when Bitwarden's `creationDate` could be read.
    pub created_at: Option<u64>,
}

/// Base64 in either alphabet, with or without padding, as the url-safe spelling without padding.
fn url_form(text: &str) -> String {
    text.trim()
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect()
}

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` (hex digits, dashes in the usual places) read as its 16 bytes.
fn guid_bytes(text: &str) -> Option<[u8; 16]> {
    let t = text.trim();
    let b = t.as_bytes();
    if b.len() != 36 || ![8, 13, 18, 23].iter().all(|i| b[*i] == b'-') {
        return None;
    }
    let hex: Vec<u8> = t.bytes().filter(|c| *c != b'-').collect();
    if hex.len() != 32 || !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, pair) in hex.chunks(2).enumerate() {
        let s = std::str::from_utf8(pair).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

/// The credential id as base64url: a GUID is its 16 bytes, anything else is base64 already.
fn credential_id(text: &str) -> String {
    match guid_bytes(text) {
        Some(bytes) => b64::encode(&bytes),
        None => url_form(text),
    }
}

/// A stored passkey from Bitwarden's entry. The text of every refusal is written for the person.
pub fn record_from(cred: &BwCredential<'_>, now: u64) -> Result<PasskeyRecord, PasskeyError> {
    if !cred.key_type.is_empty() && !cred.key_type.eq_ignore_ascii_case("public-key") {
        return Err(PasskeyError::unsupported("it is not a public-key passkey"));
    }
    if !cred.key_algorithm.eq_ignore_ascii_case("ECDSA")
        || !cred.key_curve.eq_ignore_ascii_case("P-256")
    {
        return Err(PasskeyError::unsupported(
            "its key is not an ES256 (P-256) key, the only kind this app keeps",
        ));
    }
    let der = Zeroizing::new(
        b64::decode(&url_form(cred.key_value))
            .map_err(|_| PasskeyError::invalid("its key could not be read"))?,
    );
    let key = SigningKey::from_pkcs8_der(&der)
        .map_err(|_| PasskeyError::invalid("its key could not be read"))?;
    let rp_id = cred.rp_id.trim().to_ascii_lowercase();
    let record = PasskeyRecord {
        v: super::record::RECORD_VERSION,
        rp_id: rp_id.clone(),
        rp_name: if cred.rp_name.trim().is_empty() {
            rp_id
        } else {
            cred.rp_name.trim().to_string()
        },
        credential_id: credential_id(cred.credential_id),
        user_handle: url_form(cred.user_handle),
        user_name: cred.user_name.to_string(),
        display_name: cred.user_display_name.to_string(),
        alg: ALG_ES256,
        private_key: b64::encode(&key.to_bytes()),
        created_at: cred
            .created_at
            .filter(|t| *t > 0 && *t <= now)
            .unwrap_or(now),
    };
    // the one place that checks everything a later step relies on (website name, id and handle sizes, the key itself)
    let text = record.to_json();
    PasskeyRecord::parse(&text).map_err(|e| PasskeyError::invalid(describe(&e)))?;
    Ok(record)
}

/// The refusal of `PasskeyRecord::parse` as the end of a sentence that starts with the title of the item.
fn describe(e: &PasskeyError) -> String {
    let text = e.to_string();
    let mut chars = text.trim_end_matches('.').chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => "it could not be read".to_string(),
    }
}

/// Bitwarden's entry for a stored passkey, every value as text the way Bitwarden writes it.
pub struct BwOut {
    pub credential_id: String,
    pub key_value: Zeroizing<String>,
    pub rp_id: String,
    pub rp_name: String,
    pub user_handle: String,
    pub user_name: String,
    pub user_display_name: String,
    pub creation_ms: u64,
}

pub fn to_bitwarden(record: &PasskeyRecord) -> Result<BwOut, PasskeyError> {
    let key = record.signing_key()?;
    let der = key
        .to_pkcs8_der()
        .map_err(|_| PasskeyError::invalid("A passkey has an invalid key."))?;
    Ok(BwOut {
        credential_id: record.credential_id.clone(),
        key_value: Zeroizing::new(b64::encode(der.as_bytes())),
        rp_id: record.rp_id.clone(),
        rp_name: record.rp_name.clone(),
        user_handle: record.user_handle.clone(),
        user_name: record.user_name.clone(),
        user_display_name: record.display_name.clone(),
        creation_ms: record.created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passkey::engine;

    const NOW: u64 = 1_800_000_000_000;

    fn cred<'a>(id: &'a str, key: &'a str) -> BwCredential<'a> {
        BwCredential {
            credential_id: id,
            key_type: "public-key",
            key_algorithm: "ECDSA",
            key_curve: "P-256",
            key_value: key,
            rp_id: "Example.COM",
            rp_name: "Example",
            user_handle: "dXNlci0x",
            user_name: "ana@example.com",
            user_display_name: "Ana",
            created_at: Some(1_700_000_000_000),
        }
    }

    fn some_key() -> (SigningKey, String) {
        let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let der = key.to_pkcs8_der().unwrap();
        (key, b64::encode(der.as_bytes()))
    }

    #[test]
    fn a_guid_is_its_sixteen_bytes_and_other_ids_stay_what_they_are() {
        let guid = "00112233-4455-6677-8899-aabbccddeeff";
        assert_eq!(
            b64::decode(&credential_id(guid)).unwrap(),
            vec![
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff
            ]
        );
        assert_eq!(
            credential_id("AAECAwQFBgcICQoLDA0ODw=="),
            "AAECAwQFBgcICQoLDA0ODw"
        );
        assert_eq!(credential_id("a+b/c"), "a-b_c");
        // not a GUID: the dashes are in the wrong places
        assert!(guid_bytes("0011223-34455-6677-8899-aabbccddeeff").is_none());
        assert!(guid_bytes("zz112233-4455-6677-8899-aabbccddeeff").is_none());
    }

    #[test]
    fn a_bitwarden_passkey_becomes_a_record_that_signs_for_the_same_key() {
        let (key, der) = some_key();
        let rec = record_from(&cred("00112233-4455-6677-8899-aabbccddeeff", &der), NOW).unwrap();
        assert_eq!(rec.rp_id, "example.com", "the website name is lower-cased");
        assert_eq!(rec.created_at, 1_700_000_000_000);
        assert_eq!(rec.signing_key().unwrap().to_bytes(), key.to_bytes());
        // it works as a passkey: a login signature verifies against the public key of the key it came with
        use p256::ecdsa::signature::Verifier;
        use p256::ecdsa::{Signature, VerifyingKey};
        let hash = [1u8; 32];
        let assertion = engine::get_assertion(&rec, &hash, true).unwrap();
        let mut message = assertion.auth_data.clone();
        message.extend_from_slice(&hash);
        let sig = Signature::from_der(&assertion.signature).unwrap();
        VerifyingKey::from(&key).verify(&message, &sig).unwrap();
    }

    #[test]
    fn the_key_may_be_written_in_either_base64_alphabet_with_or_without_padding() {
        let (key, der) = some_key();
        let std_padded = {
            use base64::engine::general_purpose::STANDARD;
            use base64::Engine;
            STANDARD.encode(key.to_pkcs8_der().unwrap().as_bytes())
        };
        for text in [der.clone(), std_padded] {
            let rec = record_from(&cred("AAECAwQFBgcICQoLDA0ODw", &text), NOW).unwrap();
            assert_eq!(rec.signing_key().unwrap().to_bytes(), key.to_bytes());
        }
    }

    #[test]
    fn a_missing_date_or_one_in_the_future_becomes_now() {
        let (_, der) = some_key();
        let mut c = cred("AAECAwQFBgcICQoLDA0ODw", &der);
        c.created_at = None;
        assert_eq!(record_from(&c, NOW).unwrap().created_at, NOW);
        c.created_at = Some(NOW + 5);
        assert_eq!(record_from(&c, NOW).unwrap().created_at, NOW);
    }

    #[test]
    fn what_cannot_be_kept_is_refused_with_a_reason() {
        let (_, der) = some_key();
        let id = "AAECAwQFBgcICQoLDA0ODw";
        let mut rsa = cred(id, &der);
        rsa.key_algorithm = "RSA";
        assert!(record_from(&rsa, NOW)
            .unwrap_err()
            .to_string()
            .contains("ES256"));
        let mut curve = cred(id, &der);
        curve.key_curve = "P-384";
        assert!(record_from(&curve, NOW).is_err());
        let mut kind = cred(id, &der);
        kind.key_type = "password";
        assert!(record_from(&kind, NOW).is_err());
        assert!(record_from(&cred(id, "not a key!"), NOW)
            .unwrap_err()
            .to_string()
            .contains("key"));
        assert!(
            record_from(&cred(id, &b64::encode(&[1, 2, 3])), NOW).is_err(),
            "bytes that are no PKCS#8 document"
        );
        let mut ip = cred(id, &der);
        ip.rp_id = "192.168.0.1";
        assert!(
            record_from(&ip, NOW).is_err(),
            "a website name is a domain, not an address"
        );
        let mut short = cred("AAEC", &der);
        short.rp_id = "example.com";
        assert!(record_from(&short, NOW).is_err(), "an id under 16 bytes");
        let mut nohandle = cred(id, &der);
        nohandle.user_handle = "";
        assert!(
            record_from(&nohandle, NOW).is_err(),
            "a passkey needs a user id"
        );
    }

    #[test]
    fn a_missing_site_name_is_the_website_name() {
        let (_, der) = some_key();
        let mut c = cred("AAECAwQFBgcICQoLDA0ODw", &der);
        c.rp_name = "  ";
        assert_eq!(record_from(&c, NOW).unwrap().rp_name, "example.com");
    }

    #[test]
    fn exporting_and_reading_back_gives_the_same_passkey() {
        let (_, der) = some_key();
        let rec = record_from(&cred("AAECAwQFBgcICQoLDA0ODw", &der), NOW).unwrap();
        let out = to_bitwarden(&rec).unwrap();
        assert_eq!(out.key_value.as_str(), der);
        let back = record_from(
            &BwCredential {
                credential_id: &out.credential_id,
                key_type: "public-key",
                key_algorithm: "ECDSA",
                key_curve: "P-256",
                key_value: &out.key_value,
                rp_id: &out.rp_id,
                rp_name: &out.rp_name,
                user_handle: &out.user_handle,
                user_name: &out.user_name,
                user_display_name: &out.user_display_name,
                created_at: Some(out.creation_ms),
            },
            NOW,
        )
        .unwrap();
        assert_eq!(back.to_json().as_str(), rec.to_json().as_str());
    }
}
