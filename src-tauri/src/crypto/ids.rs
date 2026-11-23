//! Object identifiers used in envelope headers and HKDF contexts.
//!
//! Allowed: `[A-Za-z0-9_.:-]{1,64}`. This excludes NUL and every separator used when
//! building HKDF `info`, so the encodings are unambiguous.

use super::error::{CryptoError, Result};

pub const MAX_ID_LEN: usize = 64;

/// Reserved system object ids (ids starting with `_` are never user items).
pub const KEYRING_ID: &str = "_keyring";
pub const ROOT_KEY_ID: &str = "_vrk";

pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return Err(CryptoError::InvalidInput(
            "object id must be 1 to 64 characters",
        ));
    }
    let ok = id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(CryptoError::InvalidInput(
            "object id contains a character outside [A-Za-z0-9_.:-]",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_uuids_and_reserved_ids() {
        assert!(validate_id("3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f").is_ok());
        assert!(validate_id(KEYRING_ID).is_ok());
        assert!(validate_id(ROOT_KEY_ID).is_ok());
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(validate_id("").is_err());
        assert!(validate_id(&"a".repeat(65)).is_err());
        assert!(validate_id("has space").is_err());
        assert!(validate_id("nul\0byte").is_err());
        assert!(validate_id("slash/path").is_err());
        assert!(validate_id("ünïcode").is_err());
        assert!(validate_id(&"a".repeat(64)).is_ok());
    }
}
