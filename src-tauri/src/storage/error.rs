//! Error type of the storage layer and of every Tauri command (`ai-docs/IPC_CONTRACT.md`).
//!
//! Serialized as `{ "code": "...", "message": "..." }`. Messages are written for the person
//! using the app and never contain secrets, file paths or SQL.

use crate::crypto::CryptoError;
use serde::ser::{Serialize, SerializeStruct, Serializer};

pub type Result<T> = core::result::Result<T, StorageError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    #[error("The vault is locked.")]
    Locked,
    #[error("There is no vault on this device yet.")]
    NoVault,
    #[error("A vault already exists on this device.")]
    VaultExists,
    #[error("Wrong master password.")]
    WrongPassword,
    #[error("That item no longer exists.")]
    NotFound,
    #[error("This item is in the trash. Restore it first.")]
    InTrash,
    #[error("This item is not in the trash.")]
    NotInTrash,
    #[error("This item changed since you opened it. Reopen it and try again.")]
    VersionConflict,
    #[error("{0}")]
    InvalidInput(String),
    #[error("The confirmation text does not match.")]
    ConfirmationMismatch,
    #[error("This edit session expired. Enter the master password again.")]
    TokenInvalid,
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Crypto(String),
}

impl StorageError {
    /// Stable machine-readable code (the list in `IPC_CONTRACT.md`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::NoVault => "no_vault",
            Self::VaultExists => "vault_exists",
            Self::WrongPassword => "wrong_password",
            Self::NotFound => "not_found",
            Self::InTrash => "in_trash",
            Self::NotInTrash => "not_in_trash",
            Self::VersionConflict => "version_conflict",
            Self::InvalidInput(_) => "invalid_input",
            Self::ConfirmationMismatch => "confirmation_mismatch",
            Self::TokenInvalid => "token_invalid",
            Self::Storage(_) => "storage",
            Self::Crypto(_) => "crypto",
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }
}

impl Serialize for StorageError {
    fn serialize<S: Serializer>(&self, serializer: S) -> core::result::Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("StorageError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<CryptoError> for StorageError {
    fn from(e: CryptoError) -> Self {
        match e {
            // The crypto layer deliberately does not tell "wrong password" from "damaged keyring".
            CryptoError::Unlock => Self::WrongPassword,
            CryptoError::InvalidInput(m) | CryptoError::Totp(m) => Self::InvalidInput(m.into()),
            CryptoError::TooLarge => Self::invalid("That content is too large."),
            CryptoError::Authentication
            | CryptoError::MalformedEnvelope(_)
            | CryptoError::Unsupported(_) => Self::Crypto(
                "A stored item failed its integrity check: the data is damaged or was modified."
                    .into(),
            ),
            CryptoError::KeyMismatch | CryptoError::MalformedBundle(_) => Self::Crypto(
                "The vault's key material failed its integrity check.".into(),
            ),
            CryptoError::KdfOutOfMemory => Self::Crypto(
                "Not enough memory to check the master password on this device.".into(),
            ),
            CryptoError::Rng | CryptoError::Kdf | CryptoError::InvalidKdfParams(_) => {
                Self::Crypto("A cryptographic operation failed.".into())
            }
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode as C;
        let message = match &e {
            rusqlite::Error::SqliteFailure(f, _) => match f.code {
                C::DatabaseBusy | C::DatabaseLocked => {
                    "The vault database is busy. Try again in a moment."
                }
                C::DiskFull => "The disk is full, so the vault could not be saved.",
                C::ReadOnly => "The vault database is read-only.",
                C::NotADatabase | C::DatabaseCorrupt => {
                    "The vault database file is damaged, or it is not a Passman vault."
                }
                C::CannotOpen => "The vault database file could not be opened.",
                _ => "The vault database reported an error.",
            },
            _ => "The vault database reported an error.",
        };
        Self::Storage(message.into())
    }
}

impl From<std::io::Error> for StorageError {
    fn from(_: std::io::Error) -> Self {
        Self::Storage("The vault folder could not be prepared.".into())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(_: serde_json::Error) -> Self {
        Self::Storage("A stored item has an unexpected format.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_code_and_message() {
        let json = serde_json::to_value(StorageError::InTrash).unwrap();
        assert_eq!(json["code"], "in_trash");
        assert!(json["message"].as_str().unwrap().contains("trash"));
    }

    #[test]
    fn every_code_matches_the_contract_list() {
        let allowed = [
            "locked",
            "no_vault",
            "vault_exists",
            "wrong_password",
            "not_found",
            "in_trash",
            "not_in_trash",
            "version_conflict",
            "invalid_input",
            "confirmation_mismatch",
            "token_invalid",
            "storage",
            "crypto",
        ];
        let all = [
            StorageError::Locked,
            StorageError::NoVault,
            StorageError::VaultExists,
            StorageError::WrongPassword,
            StorageError::NotFound,
            StorageError::InTrash,
            StorageError::NotInTrash,
            StorageError::VersionConflict,
            StorageError::invalid("x"),
            StorageError::ConfirmationMismatch,
            StorageError::TokenInvalid,
            StorageError::Storage("x".into()),
            StorageError::Crypto("x".into()),
        ];
        for e in all {
            assert!(allowed.contains(&e.code()), "{e:?}");
        }
    }

    #[test]
    fn crypto_errors_map_without_leaking_internals() {
        assert_eq!(
            StorageError::from(CryptoError::Unlock),
            StorageError::WrongPassword
        );
        assert_eq!(
            StorageError::from(CryptoError::Authentication).code(),
            "crypto"
        );
        assert_eq!(
            StorageError::from(CryptoError::Totp("secret is not valid Base32")).code(),
            "invalid_input"
        );
        let msg = StorageError::from(CryptoError::KeyMismatch).to_string();
        assert!(!msg.contains("KeyMismatch"));
    }

    #[test]
    fn sqlite_errors_are_generic() {
        let e = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            Some("secret path /home/x".into()),
        );
        let m = StorageError::from(e).to_string();
        assert!(m.contains("disk is full"));
        assert!(!m.contains("/home"));
    }
}
