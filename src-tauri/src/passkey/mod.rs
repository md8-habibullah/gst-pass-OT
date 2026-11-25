//! Passkeys: the part of a WebAuthn authenticator Hexis needs to act as one (`ai-docs/PASSKEYS.md`).
//!
//! * [`engine`] makes a credential and signs a login, byte for byte as WebAuthn and CTAP2 describe (ES256, no
//!   attestation, synced credential flags). It knows nothing about the vault or about Android.
//! * [`json`] reads the request a browser or app hands to a passkey provider (the WebAuthn JSON form) and writes the
//!   answer. Android's Credential Manager speaks exactly this.
//! * [`origin`] decides whether a caller may use a website's name (`rpId`).
//! * [`record`] is the stored form of one passkey, the value of a login's `passkey` field.
//!
//! Nothing here touches the disk or the network except [`origin::fetch_asset_links`], and that one is only called after
//! the person chose a passkey for an app (see its notes).

pub mod bitwarden;
pub mod cbor;
pub mod engine;
pub mod json;
pub mod origin;
pub mod record;

pub use record::{KeepMarker, PasskeyInfo, PasskeyRecord};

/// ES256: ECDSA over P-256 with SHA-256, COSE algorithm -7. Every site that takes passkeys accepts it.
pub const ALG_ES256: i64 = -7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasskeyError {
    /// The request or a stored passkey is malformed. The text is written for the person.
    Invalid(String),
    /// Nothing in the request is something this version can do (for example only RS256 was offered).
    Unsupported(String),
    /// The vault already holds a passkey the website listed in `excludeCredentials`.
    Excluded,
    /// A random number could not be had.
    Rng,
}

impl PasskeyError {
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::Invalid(m.into())
    }
    pub fn unsupported(m: impl Into<String>) -> Self {
        Self::Unsupported(m.into())
    }
}

impl core::fmt::Display for PasskeyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Invalid(m) | Self::Unsupported(m) => f.write_str(m),
            Self::Excluded => f.write_str("A passkey for this account is already saved here."),
            Self::Rng => f.write_str("A random number could not be made on this device."),
        }
    }
}

impl std::error::Error for PasskeyError {}

/// base64url without padding, the spelling WebAuthn uses everywhere.
pub mod b64 {
    use super::PasskeyError;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    pub fn encode(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Strict: no padding, no `+` or `/`.
    pub fn decode(text: &str) -> Result<Vec<u8>, PasskeyError> {
        URL_SAFE_NO_PAD
            .decode(text)
            .map_err(|_| PasskeyError::invalid("A passkey value is not valid base64url."))
    }
}
