//! The stored form of one passkey: a small JSON text kept as the value of a login's `passkey` field (`passkey#2` …).
//!
//! The private key is inside it, so the field is a secret like a password: it is never shown, never searched, never
//! returned to the screen (the screen gets [`PasskeyInfo`], which has no key) and the text is wiped when dropped.
//! Extra keys written by a newer version are accepted when reading and are kept as they are, because the stored text
//! is never rewritten after it was made.

use super::{b64, PasskeyError, ALG_ES256};
use p256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

pub const RECORD_VERSION: u32 = 1;
pub const MAX_RP_ID: usize = 253;
pub const MAX_NAME: usize = 256;
pub const MAX_USER_HANDLE: usize = 64;
pub const MIN_CREDENTIAL_ID: usize = 16;
pub const MAX_CREDENTIAL_ID: usize = 1023;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyRecord {
    pub v: u32,
    pub rp_id: String,
    pub rp_name: String,
    /// base64url, no padding
    pub credential_id: String,
    pub user_handle: String,
    pub user_name: String,
    pub display_name: String,
    pub alg: i64,
    /// The P-256 private scalar, 32 bytes, base64url.
    pub private_key: String,
    pub created_at: u64,
}

impl Drop for PasskeyRecord {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

/// What the screen may know about a passkey: everything but the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyInfo {
    pub credential_id: String,
    pub rp_id: String,
    pub rp_name: String,
    pub user_name: String,
    pub display_name: String,
    pub alg: i64,
    pub created_at: u64,
}

/// What stands in an item's editable copy where a passkey is: the public facts and `keep: true`. On save the core swaps
/// the stored passkey back in (found by its `credentialId`); a passkey that is not in this list is gone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepMarker {
    pub keep: bool,
    #[serde(flatten)]
    pub info: PasskeyInfo,
}

impl PasskeyRecord {
    /// Read and check a stored record. Everything a later step relies on is checked here, once.
    pub fn parse(text: &str) -> Result<Self, PasskeyError> {
        if text.len() > 8 * 1024 {
            return Err(PasskeyError::invalid("A passkey is too large."));
        }
        let rec: PasskeyRecord = serde_json::from_str(text)
            .map_err(|_| PasskeyError::invalid("A passkey could not be read."))?;
        rec.check()?;
        Ok(rec)
    }

    fn check(&self) -> Result<(), PasskeyError> {
        if self.v != RECORD_VERSION {
            return Err(PasskeyError::invalid(
                "This passkey was saved by a newer version of the app.",
            ));
        }
        if self.alg != ALG_ES256 {
            return Err(PasskeyError::unsupported(
                "This passkey uses a signature type this version cannot use.",
            ));
        }
        check_rp_id(&self.rp_id)?;
        if self.rp_name.chars().count() > MAX_NAME
            || self.user_name.chars().count() > MAX_NAME
            || self.display_name.chars().count() > MAX_NAME
        {
            return Err(PasskeyError::invalid("A passkey name is too long."));
        }
        let id = b64::decode(&self.credential_id)?;
        if !(MIN_CREDENTIAL_ID..=MAX_CREDENTIAL_ID).contains(&id.len()) {
            return Err(PasskeyError::invalid("A passkey has an invalid id."));
        }
        let handle = b64::decode(&self.user_handle)?;
        if handle.is_empty() || handle.len() > MAX_USER_HANDLE {
            return Err(PasskeyError::invalid("A passkey has an invalid user id."));
        }
        self.signing_key()?;
        Ok(())
    }

    pub fn info(&self) -> PasskeyInfo {
        PasskeyInfo {
            credential_id: self.credential_id.clone(),
            rp_id: self.rp_id.clone(),
            rp_name: self.rp_name.clone(),
            user_name: self.user_name.clone(),
            display_name: self.display_name.clone(),
            alg: self.alg,
            created_at: self.created_at,
        }
    }

    pub fn to_json(&self) -> Zeroizing<String> {
        Zeroizing::new(serde_json::to_string(self).expect("a passkey record serializes"))
    }

    pub fn credential_id_bytes(&self) -> Vec<u8> {
        b64::decode(&self.credential_id).unwrap_or_default()
    }

    pub fn user_handle_bytes(&self) -> Vec<u8> {
        b64::decode(&self.user_handle).unwrap_or_default()
    }

    pub fn signing_key(&self) -> Result<SigningKey, PasskeyError> {
        let bytes = Zeroizing::new(b64::decode(&self.private_key)?);
        // `from_slice` pads a short value with zeros; a stored key is exactly the 32 bytes of the scalar
        if bytes.len() != 32 {
            return Err(PasskeyError::invalid("A passkey has an invalid key."));
        }
        SigningKey::from_slice(&bytes)
            .map_err(|_| PasskeyError::invalid("A passkey has an invalid key."))
    }
}

/// A relying party id is a domain name: lower-case letters, digits, `-` and `.`, between 1 and 253 characters, no empty
/// label, no leading or trailing dash in a label, and not an IP address. (`localhost` is allowed for development.)
pub fn check_rp_id(rp_id: &str) -> Result<(), PasskeyError> {
    let ok = !rp_id.is_empty()
        && rp_id.len() <= MAX_RP_ID
        && rp_id.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
    let is_ip = rp_id
        .split('.')
        .all(|l| l.bytes().all(|b| b.is_ascii_digit()))
        && rp_id.contains('.');
    if ok && !is_ip {
        Ok(())
    } else {
        Err(PasskeyError::invalid(
            "That website name is not valid for a passkey.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> PasskeyRecord {
        PasskeyRecord {
            v: 1,
            rp_id: "example.com".into(),
            rp_name: "Example".into(),
            credential_id: b64::encode(&[5u8; 32]),
            user_handle: b64::encode(&[8u8; 16]),
            user_name: "ana@example.com".into(),
            display_name: "Ana".into(),
            alg: ALG_ES256,
            private_key: b64::encode(&[3u8; 32]),
            created_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn a_record_round_trips_and_the_screen_copy_has_no_key() {
        let rec = sample();
        let text = rec.to_json();
        let back = PasskeyRecord::parse(&text).unwrap();
        assert_eq!(back.rp_id, "example.com");
        let info = serde_json::to_string(&back.info()).unwrap();
        assert!(!info.contains("privateKey") && !info.contains(&rec.private_key));
        assert!(info.contains("\"rpId\":\"example.com\""));
    }

    #[test]
    fn bad_records_are_refused() {
        let good = sample();
        let mut cases: Vec<(&str, PasskeyRecord)> = Vec::new();
        let mut r = sample();
        r.v = 2;
        cases.push(("newer version", r));
        let mut r = sample();
        r.alg = -257;
        cases.push(("other algorithm", r));
        let mut r = sample();
        r.rp_id = "Example.com".into();
        cases.push(("upper case rp id", r));
        let mut r = sample();
        r.rp_id = "192.168.0.1".into();
        cases.push(("ip address", r));
        let mut r = sample();
        r.credential_id = b64::encode(&[1u8; 4]);
        cases.push(("short credential id", r));
        let mut r = sample();
        r.user_handle = b64::encode(&[1u8; 65]);
        cases.push(("long user handle", r));
        let mut r = sample();
        r.private_key = b64::encode(&[0u8; 32]);
        cases.push(("zero key", r));
        let mut r = sample();
        r.private_key = b64::encode(&[1u8; 31]);
        cases.push(("short key", r));
        let _ = good;
        for (what, rec) in cases {
            let text = serde_json::to_string(&rec).unwrap();
            assert!(PasskeyRecord::parse(&text).is_err(), "{what} was accepted");
        }
        assert!(PasskeyRecord::parse("{}").is_err());
        assert!(PasskeyRecord::parse("not json").is_err());
    }

    #[test]
    fn a_newer_record_with_extra_keys_still_reads() {
        let mut v: serde_json::Value = serde_json::from_str(&sample().to_json()).unwrap();
        v["somethingNew"] = serde_json::json!(true);
        assert!(PasskeyRecord::parse(&v.to_string()).is_ok());
    }

    #[test]
    fn rp_ids_are_domain_names() {
        for ok in [
            "example.com",
            "a.b.example.co.uk",
            "localhost",
            "xn--bcher-kva.example",
        ] {
            assert!(check_rp_id(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "a..b", "-a.com", "a-.com", "A.com", "a b.com", "a/b.com", "1.2.3.4", "a_b.com",
        ] {
            assert!(check_rp_id(bad).is_err(), "{bad}");
        }
    }
}
