//! Error type for the crypto core.
//!
//! Messages are static strings: they never contain secrets or user-supplied data, and
//! authentication failures are deliberately opaque (see `ai-docs/CRYPTO_SPEC.md` §7.3).

use serde::ser::{Serialize, SerializeStruct, Serializer};

pub type Result<T> = core::result::Result<T, CryptoError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("the operating system random number generator failed")]
    Rng,
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("invalid key derivation parameters: {0}")]
    InvalidKdfParams(&'static str),
    #[error("not enough memory for the requested key derivation cost")]
    KdfOutOfMemory,
    #[error("key derivation failed")]
    Kdf,
    #[error("authentication failed: wrong key, or the data is corrupted or was tampered with")]
    Authentication,
    #[error("malformed envelope: {0}")]
    MalformedEnvelope(&'static str),
    #[error("malformed vault key bundle: {0}")]
    MalformedBundle(&'static str),
    #[error("unsupported format or algorithm: {0}")]
    Unsupported(&'static str),
    #[error("could not unlock: wrong master password, or the vault keys are damaged")]
    Unlock,
    #[error("vault key check failed: the stored key material was modified")]
    KeyMismatch,
    #[error("data is too large for a single envelope")]
    TooLarge,
    #[error("invalid TOTP data: {0}")]
    Totp(&'static str),
}

impl CryptoError {
    /// Stable machine-readable code for the UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Rng => "rng",
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidKdfParams(_) => "invalid_kdf_params",
            Self::KdfOutOfMemory => "kdf_out_of_memory",
            Self::Kdf => "kdf",
            Self::Authentication => "authentication",
            Self::MalformedEnvelope(_) => "malformed_envelope",
            Self::MalformedBundle(_) => "malformed_bundle",
            Self::Unsupported(_) => "unsupported",
            Self::Unlock => "unlock",
            Self::KeyMismatch => "key_mismatch",
            Self::TooLarge => "too_large",
            Self::Totp(_) => "totp",
        }
    }
}

/// Serialized as `{ "code": "...", "message": "..." }` so it can be returned from Tauri commands.
impl Serialize for CryptoError {
    fn serialize<S: Serializer>(&self, serializer: S) -> core::result::Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("CryptoError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_code_and_message() {
        let json = serde_json::to_string(&CryptoError::Unlock).unwrap();
        assert!(json.contains("\"code\":\"unlock\""));
        assert!(json.contains("\"message\":"));
    }
}
