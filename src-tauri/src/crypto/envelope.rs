//! Authenticated object container (`ai-docs/CRYPTO_SPEC.md` §7).
//!
//! ```text
//! "PMV1" | header_len u32 BE | header JSON (= AAD) | nonce[24] | ciphertext ‖ tag[16]
//! ```

use super::aead::{self, NONCE_LEN, TAG_LEN};
use super::error::{CryptoError, Result};
use super::ids;
use super::rng;
use super::secret::{SecretKey32, SecretVec};
use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"PMV1";
pub const MAX_HEADER_LEN: usize = 1024;
/// One-shot AEAD limit; larger blobs need the chunked envelope planned for attachments.
pub const MAX_PLAINTEXT_LEN: usize = 64 * 1024 * 1024;

pub const SUITE_V: u32 = 1;
pub const SUITE_KEM: &str = "ML-KEM-1024+X25519";
pub const SUITE_KDF: &str = "Argon2id";
pub const SUITE_AEAD: &str = "XChaCha20-Poly1305";

const PREFIX_LEN: usize = 8; // magic + header_len

/// Unencrypted but authenticated header. Field order is the wire order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub v: u32,
    pub kem: String,
    pub kdf: String,
    pub aead: String,
    #[serde(rename = "itemId")]
    pub item_id: String,
    pub version: u64,
    pub timestamp: u64,
}

impl Header {
    /// Header for the current suite. `version` starts at 1.
    pub fn new(item_id: &str, version: u64, timestamp: u64) -> Result<Self> {
        ids::validate_id(item_id)?;
        if version == 0 {
            return Err(CryptoError::InvalidInput("version must be at least 1"));
        }
        Ok(Self {
            v: SUITE_V,
            kem: SUITE_KEM.into(),
            kdf: SUITE_KDF.into(),
            aead: SUITE_AEAD.into(),
            item_id: item_id.into(),
            version,
            timestamp,
        })
    }

    /// Exact bytes that are stored and authenticated.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)
            .map_err(|_| CryptoError::MalformedEnvelope("header serialization"))?;
        if bytes.len() > MAX_HEADER_LEN {
            return Err(CryptoError::MalformedEnvelope("header too long"));
        }
        Ok(bytes)
    }

    fn validate(&self) -> Result<()> {
        if self.v != SUITE_V {
            return Err(CryptoError::Unsupported("envelope suite version"));
        }
        if self.kem != SUITE_KEM || self.kdf != SUITE_KDF || self.aead != SUITE_AEAD {
            return Err(CryptoError::Unsupported("envelope algorithm identifiers"));
        }
        ids::validate_id(&self.item_id)
            .map_err(|_| CryptoError::MalformedEnvelope("invalid itemId"))?;
        if self.version == 0 {
            return Err(CryptoError::MalformedEnvelope("invalid version"));
        }
        Ok(())
    }
}

/// What the caller requires of an envelope before its contents are trusted.
#[derive(Debug, Clone, Copy)]
pub struct Expect<'a> {
    pub item_id: &'a str,
    /// `Some(n)` pins the exact version (e.g. opening snapshot `n`); `None` accepts any.
    pub version: Option<u64>,
}

/// An authenticated and decrypted envelope.
pub struct Opened {
    pub header: Header,
    pub plaintext: SecretVec,
}

struct Parsed<'a> {
    header_bytes: &'a [u8],
    header: Header,
    nonce: [u8; NONCE_LEN],
    ciphertext: &'a [u8],
}

fn parse(bytes: &[u8]) -> Result<Parsed<'_>> {
    if bytes.len() < PREFIX_LEN {
        return Err(CryptoError::MalformedEnvelope("too short"));
    }
    if bytes[..4] != MAGIC {
        return Err(CryptoError::MalformedEnvelope("bad magic"));
    }
    let header_len = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if header_len == 0 || header_len > MAX_HEADER_LEN {
        return Err(CryptoError::MalformedEnvelope("bad header length"));
    }
    let nonce_start = PREFIX_LEN
        .checked_add(header_len)
        .ok_or(CryptoError::MalformedEnvelope("bad header length"))?;
    let ct_start = nonce_start + NONCE_LEN;
    if bytes.len() < ct_start + TAG_LEN {
        return Err(CryptoError::MalformedEnvelope("truncated"));
    }
    let header_bytes = &bytes[PREFIX_LEN..nonce_start];
    let header: Header = serde_json::from_slice(header_bytes)
        .map_err(|_| CryptoError::MalformedEnvelope("header is not valid"))?;
    header.validate()?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[nonce_start..ct_start]);
    Ok(Parsed {
        header_bytes,
        header,
        nonce,
        ciphertext: &bytes[ct_start..],
    })
}

/// Read the header **without authenticating it**. Only for routing (e.g. choosing which key to
/// try); never trust its contents until `open` has succeeded.
pub fn peek_header(bytes: &[u8]) -> Result<Header> {
    Ok(parse(bytes)?.header)
}

fn assemble(header_bytes: &[u8], nonce: &[u8; NONCE_LEN], ciphertext: &[u8]) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(PREFIX_LEN + header_bytes.len() + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(header_bytes);
    out.extend_from_slice(nonce);
    out.extend_from_slice(ciphertext);
    out
}

/// Deterministic variant for known-answer tests only; production code must call [`seal`].
pub(crate) fn seal_with_nonce(
    key: &SecretKey32,
    header: &Header,
    plaintext: &[u8],
    nonce: &[u8; NONCE_LEN],
) -> Result<Vec<u8>> {
    if plaintext.len() > MAX_PLAINTEXT_LEN {
        return Err(CryptoError::TooLarge);
    }
    header.validate()?;
    let header_bytes = header.to_bytes()?;
    let ciphertext = aead::encrypt(key, nonce, &header_bytes, plaintext)?;
    Ok(assemble(&header_bytes, nonce, &ciphertext))
}

/// Encrypt `plaintext` under `key`, binding `header` as AAD. A fresh random nonce is drawn for
/// every call.
pub fn seal(key: &SecretKey32, header: &Header, plaintext: &[u8]) -> Result<Vec<u8>> {
    let nonce = rng::array::<NONCE_LEN>()?;
    seal_with_nonce(key, header, plaintext, &nonce)
}

/// Verify and decrypt. The header must match `expect`; authentication failure of any kind is the
/// single opaque `Authentication` error.
pub fn open(key: &SecretKey32, bytes: &[u8], expect: &Expect<'_>) -> Result<Opened> {
    let parsed = parse(bytes)?;
    if parsed.header.item_id != expect.item_id {
        return Err(CryptoError::MalformedEnvelope(
            "envelope is for a different object",
        ));
    }
    if let Some(v) = expect.version {
        if parsed.header.version != v {
            return Err(CryptoError::MalformedEnvelope(
                "envelope has an unexpected version",
            ));
        }
    }
    let plaintext = aead::decrypt(key, &parsed.nonce, parsed.header_bytes, parsed.ciphertext)?;
    Ok(Opened {
        header: parsed.header,
        plaintext,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";

    fn key() -> SecretKey32 {
        SecretKey32::from_slice(&[0x11; 32]).unwrap()
    }
    fn hdr(version: u64) -> Header {
        Header::new(ID, version, 1_732_360_000_000).unwrap()
    }
    fn expect(version: Option<u64>) -> Expect<'static> {
        Expect {
            item_id: ID,
            version,
        }
    }

    #[test]
    fn header_json_matches_spec_exactly() {
        let bytes = hdr(1).to_bytes().unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            "{\"v\":1,\"kem\":\"ML-KEM-1024+X25519\",\"kdf\":\"Argon2id\",\"aead\":\"XChaCha20-Poly1305\",\
             \"itemId\":\"3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f\",\"version\":1,\"timestamp\":1732360000000}"
        );
    }

    #[test]
    fn round_trip_with_layout() {
        let env = seal(&key(), &hdr(3), b"payload").unwrap();
        assert_eq!(&env[..4], b"PMV1");
        let hl = u32::from_be_bytes(env[4..8].try_into().unwrap()) as usize;
        assert_eq!(env.len(), 8 + hl + NONCE_LEN + 7 + TAG_LEN);
        let opened = open(&key(), &env, &expect(Some(3))).unwrap();
        assert_eq!(&opened.plaintext[..], b"payload");
        assert_eq!(opened.header, hdr(3));
        assert_eq!(peek_header(&env).unwrap(), hdr(3));
    }

    #[test]
    fn nonce_is_fresh_every_time() {
        let a = seal(&key(), &hdr(1), b"same").unwrap();
        let b = seal(&key(), &hdr(1), b"same").unwrap();
        assert_ne!(a, b);
        let hl = u32::from_be_bytes(a[4..8].try_into().unwrap()) as usize;
        assert_ne!(
            &a[8 + hl..8 + hl + NONCE_LEN],
            &b[8 + hl..8 + hl + NONCE_LEN]
        );
    }

    #[test]
    fn wrong_key_is_opaque_authentication_error() {
        let env = seal(&key(), &hdr(1), b"x").unwrap();
        let other = SecretKey32::from_slice(&[0x12; 32]).unwrap();
        assert_eq!(
            open(&other, &env, &expect(None)).err(),
            Some(CryptoError::Authentication)
        );
    }

    #[test]
    fn binds_object_id_and_version() {
        let env = seal(&key(), &hdr(2), b"x").unwrap();
        let wrong_id = Expect {
            item_id: "another-object",
            version: None,
        };
        assert!(matches!(
            open(&key(), &env, &wrong_id),
            Err(CryptoError::MalformedEnvelope(_))
        ));
        assert!(matches!(
            open(&key(), &env, &expect(Some(3))),
            Err(CryptoError::MalformedEnvelope(_))
        ));
        assert!(open(&key(), &env, &expect(Some(2))).is_ok());
    }

    #[test]
    fn every_header_byte_is_authenticated() {
        let env = seal(&key(), &hdr(1), b"x").unwrap();
        let hl = u32::from_be_bytes(env[4..8].try_into().unwrap()) as usize;
        for i in 8..8 + hl {
            let mut bad = env.clone();
            bad[i] ^= 0x01;
            let err = open(&key(), &bad, &expect(None))
                .err()
                .expect("tampered header must not open");
            assert!(
                matches!(
                    err,
                    CryptoError::Authentication
                        | CryptoError::MalformedEnvelope(_)
                        | CryptoError::Unsupported(_)
                ),
                "byte {i}: {err:?}"
            );
        }
    }

    #[test]
    fn every_ciphertext_nonce_and_tag_byte_is_authenticated() {
        let env = seal(&key(), &hdr(1), b"some secret bytes").unwrap();
        let hl = u32::from_be_bytes(env[4..8].try_into().unwrap()) as usize;
        for i in 8 + hl..env.len() {
            let mut bad = env.clone();
            bad[i] ^= 0x80;
            assert_eq!(
                open(&key(), &bad, &expect(None)).err(),
                Some(CryptoError::Authentication),
                "byte {i}"
            );
        }
    }

    #[test]
    fn rejects_malformed_framing() {
        let env = seal(&key(), &hdr(1), b"x").unwrap();
        assert!(peek_header(&[]).is_err());
        assert!(peek_header(&env[..7]).is_err());
        let mut bad_magic = env.clone();
        bad_magic[0] = b'X';
        assert!(matches!(
            open(&key(), &bad_magic, &expect(None)),
            Err(CryptoError::MalformedEnvelope("bad magic"))
        ));
        let mut zero_len = env.clone();
        zero_len[4..8].copy_from_slice(&0u32.to_be_bytes());
        assert!(open(&key(), &zero_len, &expect(None)).is_err());
        let mut huge_len = env.clone();
        huge_len[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(open(&key(), &huge_len, &expect(None)).is_err());
        for cut in 0..env.len() {
            assert!(
                open(&key(), &env[..cut], &expect(None)).is_err(),
                "truncated at {cut}"
            );
        }
    }

    #[test]
    fn rejects_unknown_suite_and_extra_fields() {
        // Build a syntactically valid envelope by hand with a bad suite id.
        let mut h = hdr(1);
        h.kem = "ML-KEM-512".into();
        assert!(seal(&key(), &h, b"x").is_err());

        let bad_json = br#"{"v":1,"kem":"ML-KEM-1024+X25519","kdf":"Argon2id","aead":"XChaCha20-Poly1305","itemId":"a","version":1,"timestamp":1,"extra":true}"#;
        let env = assemble(bad_json, &[0u8; NONCE_LEN], &[0u8; TAG_LEN]);
        assert!(matches!(
            peek_header(&env),
            Err(CryptoError::MalformedEnvelope(_))
        ));

        let dup = br#"{"v":1,"v":1,"kem":"ML-KEM-1024+X25519","kdf":"Argon2id","aead":"XChaCha20-Poly1305","itemId":"a","version":1,"timestamp":1}"#;
        let env = assemble(dup, &[0u8; NONCE_LEN], &[0u8; TAG_LEN]);
        assert!(peek_header(&env).is_err());

        let v2 = br#"{"v":2,"kem":"ML-KEM-1024+X25519","kdf":"Argon2id","aead":"XChaCha20-Poly1305","itemId":"a","version":1,"timestamp":1}"#;
        let env = assemble(v2, &[0u8; NONCE_LEN], &[0u8; TAG_LEN]);
        assert!(matches!(
            peek_header(&env),
            Err(CryptoError::Unsupported(_))
        ));
    }

    #[test]
    fn header_constructor_validates() {
        assert!(Header::new("bad id", 1, 0).is_err());
        assert!(Header::new(ID, 0, 0).is_err());
    }

    #[test]
    fn size_limit_enforced() {
        let big = vec![0u8; MAX_PLAINTEXT_LEN + 1];
        assert_eq!(
            seal(&key(), &hdr(1), &big).err(),
            Some(CryptoError::TooLarge)
        );
    }

    #[test]
    fn empty_plaintext_round_trips() {
        let env = seal(&key(), &hdr(1), b"").unwrap();
        assert!(open(&key(), &env, &expect(None))
            .unwrap()
            .plaintext
            .is_empty());
    }
}
