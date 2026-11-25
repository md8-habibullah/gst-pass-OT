//! Make a passkey and sign with it (WebAuthn level 3 §6, CTAP 2.1 §6). ES256 only, no attestation.
//!
//! Layout of the authenticator data (§6.1):
//! `rpIdHash (32) | flags (1) | signCount (4, big-endian) | attestedCredentialData (only when AT is set)`,
//! and the attested credential data is `aaguid (16) | credentialIdLength (2, big-endian) | credentialId | COSE_Key`.
//!
//! Flags written: UP (the person chose this passkey), UV when the caller verified the person (fingerprint, screen lock
//! or master password), BE and BS (the passkey is backed up: it lives in the vault, which syncs), AT on creation.
//! The signature counter stays 0 because a passkey that lives on several devices has no single counter; the spec lets
//! an authenticator do that. The AAGUID is all zeros: the provider does not tell sites what it is.

use super::cbor;
use super::record::{PasskeyRecord, RECORD_VERSION};
use super::{b64, PasskeyError, ALG_ES256};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const FLAG_UP: u8 = 0x01;
pub const FLAG_UV: u8 = 0x04;
pub const FLAG_BE: u8 = 0x08;
pub const FLAG_BS: u8 = 0x10;
pub const FLAG_AT: u8 = 0x40;

/// 91 bytes: the fixed start of a SubjectPublicKeyInfo for a P-256 key, then `04 | x | y`.
const SPKI_P256_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

pub struct MakeParams<'a> {
    pub rp_id: &'a str,
    pub rp_name: &'a str,
    pub user_handle: &'a [u8],
    pub user_name: &'a str,
    pub display_name: &'a str,
    /// COSE algorithm numbers the site accepts, best first (`pubKeyCredParams`); empty means the spec's default list.
    pub algorithms: &'a [i64],
    /// The person was verified (fingerprint, screen lock, master password).
    pub user_verified: bool,
    pub now_ms: u64,
}

pub struct Made {
    pub record: PasskeyRecord,
    pub auth_data: Vec<u8>,
    pub attestation_object: Vec<u8>,
    /// SubjectPublicKeyInfo DER of the new public key (`response.publicKey`).
    pub public_key: Vec<u8>,
}

/// The first algorithm of the site's list that this version can make. An empty list is the default one (ES256, RS256).
pub fn pick_algorithm(requested: &[i64]) -> Result<i64, PasskeyError> {
    if requested.is_empty() || requested.contains(&ALG_ES256) {
        return Ok(ALG_ES256);
    }
    Err(PasskeyError::unsupported(
        "This website asks for a kind of passkey that Hexis cannot make yet (it needs ES256).",
    ))
}

fn flags(user_verified: bool, attested: bool) -> u8 {
    let mut f = FLAG_UP | FLAG_BE | FLAG_BS;
    if user_verified {
        f |= FLAG_UV;
    }
    if attested {
        f |= FLAG_AT;
    }
    f
}

fn random_key() -> Result<SigningKey, PasskeyError> {
    // Retry only matters for the one-in-2^128 chance of zero or a value above the curve order.
    for _ in 0..8 {
        let mut b = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut b[..]).map_err(|_| PasskeyError::Rng)?;
        if let Ok(k) = SigningKey::from_slice(&b[..]) {
            return Ok(k);
        }
    }
    Err(PasskeyError::Rng)
}

pub fn spki(x: &[u8; 32], y: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(91);
    out.extend_from_slice(&SPKI_P256_PREFIX);
    out.push(0x04);
    out.extend_from_slice(x);
    out.extend_from_slice(y);
    out
}

/// The `x` and `y` of the public key of `key`.
fn xy(key: &SigningKey) -> ([u8; 32], [u8; 32]) {
    let point = key.verifying_key().to_sec1_point(false);
    let bytes = point.as_bytes();
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    x.copy_from_slice(&bytes[1..33]);
    y.copy_from_slice(&bytes[33..65]);
    (x, y)
}

/// The SubjectPublicKeyInfo of a stored passkey's public key.
pub fn public_key_of(record: &PasskeyRecord) -> Result<Vec<u8>, PasskeyError> {
    let (x, y) = xy(&record.signing_key()?);
    Ok(spki(&x, &y))
}

pub fn rp_id_hash(rp_id: &str) -> [u8; 32] {
    Sha256::digest(rp_id.as_bytes()).into()
}

/// authenticatorMakeCredential: a new key pair, a new credential id, and the "none" attestation.
pub fn make_credential(p: &MakeParams<'_>) -> Result<Made, PasskeyError> {
    let alg = pick_algorithm(p.algorithms)?;
    super::record::check_rp_id(p.rp_id)?;
    let key = random_key()?;
    let mut credential_id = [0u8; 32];
    getrandom::fill(&mut credential_id).map_err(|_| PasskeyError::Rng)?;
    let (x, y) = xy(&key);

    let mut auth_data = Vec::with_capacity(37 + 18 + 32 + 77);
    auth_data.extend_from_slice(&rp_id_hash(p.rp_id));
    auth_data.push(flags(p.user_verified, true));
    auth_data.extend_from_slice(&0u32.to_be_bytes());
    auth_data.extend_from_slice(&[0u8; 16]); // AAGUID
    auth_data.extend_from_slice(&(credential_id.len() as u16).to_be_bytes());
    auth_data.extend_from_slice(&credential_id);
    auth_data.extend_from_slice(&cbor::cose_es256_key(&x, &y));

    let record = PasskeyRecord {
        v: RECORD_VERSION,
        rp_id: p.rp_id.to_string(),
        rp_name: p.rp_name.chars().take(super::record::MAX_NAME).collect(),
        credential_id: b64::encode(&credential_id),
        user_handle: b64::encode(p.user_handle),
        user_name: p.user_name.chars().take(super::record::MAX_NAME).collect(),
        display_name: p
            .display_name
            .chars()
            .take(super::record::MAX_NAME)
            .collect(),
        alg,
        private_key: b64::encode(&key.to_bytes()),
        created_at: p.now_ms,
    };
    // `parse` is the one place that decides what a good record is; a made one has to pass it too.
    PasskeyRecord::parse(&record.to_json())?;
    Ok(Made {
        attestation_object: cbor::attestation_none(&auth_data),
        public_key: spki(&x, &y),
        auth_data,
        record,
    })
}

pub struct Assertion {
    pub auth_data: Vec<u8>,
    /// ASN.1 DER `ECDSA-Sig-Value`, as WebAuthn wants for ES256.
    pub signature: Vec<u8>,
}

/// authenticatorGetAssertion for one stored passkey: sign `authenticatorData || clientDataHash`.
pub fn get_assertion(
    record: &PasskeyRecord,
    client_data_hash: &[u8; 32],
    user_verified: bool,
) -> Result<Assertion, PasskeyError> {
    let key = record.signing_key()?;
    let mut auth_data = Vec::with_capacity(37);
    auth_data.extend_from_slice(&rp_id_hash(&record.rp_id));
    auth_data.push(flags(user_verified, false));
    auth_data.extend_from_slice(&0u32.to_be_bytes());

    let mut message = auth_data.clone();
    message.extend_from_slice(client_data_hash);
    let sig: Signature = key.sign(&message);
    Ok(Assertion {
        signature: der_signature(&sig.to_bytes()),
        auth_data,
    })
}

/// `r || s` (64 bytes) to `SEQUENCE { INTEGER r, INTEGER s }`.
pub fn der_signature(rs: &[u8]) -> Vec<u8> {
    fn int(out: &mut Vec<u8>, be: &[u8]) {
        let start = be.iter().position(|&b| b != 0).unwrap_or(be.len() - 1);
        let digits = &be[start..];
        out.push(0x02);
        if digits[0] & 0x80 != 0 {
            out.push(digits.len() as u8 + 1);
            out.push(0);
        } else {
            out.push(digits.len() as u8);
        }
        out.extend_from_slice(digits);
    }
    let mut body = Vec::with_capacity(72);
    int(&mut body, &rs[..32]);
    int(&mut body, &rs[32..64]);
    let mut out = Vec::with_capacity(body.len() + 2);
    out.push(0x30);
    out.push(body.len() as u8);
    out.extend(body);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::passkey::cbor::read::{parse, Item};
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::VerifyingKey;

    pub(crate) fn make(rp_id: &str, uv: bool) -> Made {
        make_credential(&MakeParams {
            rp_id,
            rp_name: "Example",
            user_handle: &[9u8; 16],
            user_name: "ana@example.com",
            display_name: "Ana",
            algorithms: &[-7, -257],
            user_verified: uv,
            now_ms: 1_700_000_000_000,
        })
        .unwrap()
    }

    /// Read a DER ECDSA signature the way a verifier does (strict: minimal integers, nothing left over).
    pub(crate) fn rs_from_der(der: &[u8]) -> [u8; 64] {
        assert_eq!(der[0], 0x30);
        assert_eq!(der[1] as usize, der.len() - 2, "sequence length");
        let mut rest = &der[2..];
        let mut out = [0u8; 64];
        for half in 0..2 {
            assert_eq!(rest[0], 0x02);
            let len = rest[1] as usize;
            let mut int = &rest[2..2 + len];
            assert!(int[0] & 0x80 == 0, "negative integer");
            if int.len() > 1 && int[0] == 0 {
                assert!(int[1] & 0x80 != 0, "integer not minimal");
            }
            while int.len() > 32 {
                assert_eq!(int[0], 0);
                int = &int[1..];
            }
            out[half * 32 + 32 - int.len()..(half + 1) * 32].copy_from_slice(int);
            rest = &rest[2 + len..];
        }
        assert!(rest.is_empty());
        out
    }

    /// What a relying party does with a registration: parse the attestation object, read the public key out of the
    /// authenticator data, and return (credential id, public key, flags).
    pub(crate) fn rp_check_registration(made: &Made, rp_id: &str) -> (Vec<u8>, VerifyingKey, u8) {
        let obj = parse(&made.attestation_object);
        assert_eq!(obj.text_key("fmt"), Some(&Item::Text("none".into())));
        assert_eq!(obj.text_key("attStmt"), Some(&Item::Map(vec![])));
        let auth = obj.text_key("authData").unwrap().as_bytes().to_vec();
        assert_eq!(auth, made.auth_data);
        assert_eq!(&auth[..32], &Sha256::digest(rp_id.as_bytes())[..]);
        let flags = auth[32];
        assert!(flags & FLAG_AT != 0 && flags & FLAG_UP != 0);
        assert_eq!(&auth[33..37], [0, 0, 0, 0], "sign count");
        assert_eq!(&auth[37..53], [0u8; 16], "aaguid");
        let id_len = u16::from_be_bytes([auth[53], auth[54]]) as usize;
        let id = auth[55..55 + id_len].to_vec();
        let cose = parse(&auth[55 + id_len..]);
        assert_eq!(cose.int_key(1), Some(&Item::Int(2)));
        assert_eq!(cose.int_key(3), Some(&Item::Int(-7)));
        assert_eq!(cose.int_key(-1), Some(&Item::Int(1)));
        let mut sec1 = vec![4u8];
        sec1.extend_from_slice(cose.int_key(-2).unwrap().as_bytes());
        sec1.extend_from_slice(cose.int_key(-3).unwrap().as_bytes());
        let vk = VerifyingKey::from_sec1_bytes(&sec1).unwrap();
        // the SPKI the site is given describes the same key
        assert_eq!(made.public_key[..26], SPKI_P256_PREFIX);
        assert_eq!(made.public_key[26..], sec1[..]);
        (id, vk, flags)
    }

    #[test]
    fn a_registration_reads_back_the_way_a_website_reads_it() {
        let made = make("example.com", true);
        let (id, _vk, flags) = rp_check_registration(&made, "example.com");
        assert_eq!(id, made.record.credential_id_bytes());
        assert_eq!(id.len(), 32);
        // UP, UV, BE, BS, AT
        assert_eq!(flags, 0x01 | 0x04 | 0x08 | 0x10 | 0x40);
        assert_eq!(made.record.alg, -7);
        assert_eq!(made.record.user_handle_bytes(), vec![9u8; 16]);
        assert_eq!(made.record.created_at, 1_700_000_000_000);
    }

    #[test]
    fn without_verification_the_uv_flag_is_off() {
        let made = make("example.com", false);
        let (_, _, flags) = rp_check_registration(&made, "example.com");
        assert_eq!(flags & FLAG_UV, 0);
        assert_eq!(flags & FLAG_BE, FLAG_BE);
    }

    #[test]
    fn an_assertion_verifies_against_the_registered_key() {
        let made = make("example.com", true);
        let (_, vk, reg_flags) = rp_check_registration(&made, "example.com");
        let cdh: [u8; 32] = Sha256::digest(b"{\"type\":\"webauthn.get\"}").into();
        let a = get_assertion(&made.record, &cdh, true).unwrap();

        assert_eq!(a.auth_data.len(), 37);
        assert_eq!(&a.auth_data[..32], &rp_id_hash("example.com")[..]);
        // BE must not change between registration and login (WebAuthn §6.3.3)
        assert_eq!(a.auth_data[32] & FLAG_BE, reg_flags & FLAG_BE);
        assert_eq!(a.auth_data[32] & FLAG_AT, 0);
        assert_eq!(&a.auth_data[33..], [0, 0, 0, 0]);

        let mut signed = a.auth_data.clone();
        signed.extend_from_slice(&cdh);
        let sig = Signature::from_slice(&rs_from_der(&a.signature)).unwrap();
        assert!(vk.verify(&signed, &sig).is_ok());
        // any other challenge, or any other rp, fails
        let mut other = a.auth_data.clone();
        other.extend_from_slice(&[0u8; 32]);
        assert!(vk.verify(&other, &sig).is_err());
    }

    #[test]
    fn der_signatures_handle_leading_zeros_and_the_high_bit() {
        // r starts with 0x00 bytes, s has the high bit set
        let mut rs = [0u8; 64];
        rs[2] = 0x7f;
        rs[32] = 0x80;
        rs[63] = 1;
        let der = der_signature(&rs);
        assert_eq!(rs_from_der(&der), rs);
        // all-zero halves still give a valid (if useless) encoding, one byte each
        assert_eq!(der_signature(&[0u8; 64]), [0x30, 6, 2, 1, 0, 2, 1, 0]);
    }

    #[test]
    fn credentials_are_unique_and_the_algorithm_list_is_honoured() {
        let a = make("example.com", true);
        let b = make("example.com", true);
        assert_ne!(a.record.credential_id, b.record.credential_id);
        assert_ne!(a.record.private_key, b.record.private_key);
        assert_eq!(pick_algorithm(&[]).unwrap(), -7);
        assert_eq!(pick_algorithm(&[-257, -7]).unwrap(), -7);
        assert!(matches!(
            pick_algorithm(&[-257, -8]),
            Err(PasskeyError::Unsupported(_))
        ));
    }

    #[test]
    fn a_bad_rp_id_makes_nothing() {
        let r = make_credential(&MakeParams {
            rp_id: "Not A Domain",
            rp_name: "x",
            user_handle: &[1],
            user_name: "u",
            display_name: "u",
            algorithms: &[],
            user_verified: true,
            now_ms: 0,
        });
        assert!(r.is_err());
    }
}
