//! HKDF-SHA512 sub-key derivation with domain-separation labels (`ai-docs/CRYPTO_SPEC.md` §5).

use super::error::{CryptoError, Result};
use super::ids;
use super::secret::SecretKey32;
use hkdf::Hkdf;
use sha2::Sha512;
use zeroize::Zeroizing;

pub(crate) const SUBKEY_SALT: &[u8] = b"passman/v1/hkdf";
const INFO_PREFIX: &[u8] = b"passman/v1";

/// What a derived key is for. The label is part of the HKDF `info`, so keys for different
/// purposes are independent even when derived from the same root and context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPurpose {
    /// Seals the secret keyring. Root: the Argon2id password key.
    Keyring,
    /// Commitment to the VaultRootKey stored inside the keyring.
    VrkCommit,
    /// Item latest-state objects.
    Item,
    /// Immutable version snapshots.
    Version,
    /// Attachment blobs.
    Attachment,
    /// Local metadata/settings (S3 target configs, ...). Context = setting name.
    Meta,
}

impl KeyPurpose {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Keyring => "keyring",
            Self::VrkCommit => "vrk-commit",
            Self::Item => "item",
            Self::Version => "version",
            Self::Attachment => "attachment",
            Self::Meta => "meta",
        }
    }
}

/// `HKDF-SHA512(salt = "passman/v1/hkdf", ikm = root, info = "passman/v1" 0x00 label 0x00 context)` → 32 bytes.
///
/// `context` is empty or a valid object id (`ids::validate_id`).
pub fn derive_subkey(
    root: &SecretKey32,
    purpose: KeyPurpose,
    context: &str,
) -> Result<SecretKey32> {
    if !context.is_empty() {
        ids::validate_id(context)?;
    }
    let mut info = Zeroizing::new(Vec::with_capacity(
        INFO_PREFIX.len() + 2 + 16 + context.len(),
    ));
    info.extend_from_slice(INFO_PREFIX);
    info.push(0);
    info.extend_from_slice(purpose.label().as_bytes());
    info.push(0);
    info.extend_from_slice(context.as_bytes());

    let hk = Hkdf::<Sha512>::new(Some(SUBKEY_SALT), root.as_bytes());
    let mut out = SecretKey32::zeroed();
    hk.expand(&info, out.as_mut_bytes())
        .map_err(|_| CryptoError::Kdf)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> SecretKey32 {
        SecretKey32::from_slice(&[7u8; 32]).unwrap()
    }

    #[test]
    fn deterministic_and_domain_separated() {
        let r = root();
        let a1 = derive_subkey(&r, KeyPurpose::Item, "item-1").unwrap();
        let a2 = derive_subkey(&r, KeyPurpose::Item, "item-1").unwrap();
        assert_eq!(a1, a2);
        assert_ne!(a1, derive_subkey(&r, KeyPurpose::Item, "item-2").unwrap());
        assert_ne!(
            a1,
            derive_subkey(&r, KeyPurpose::Version, "item-1").unwrap()
        );
        assert_ne!(
            a1,
            derive_subkey(&r, KeyPurpose::Attachment, "item-1").unwrap()
        );
        assert_ne!(a1, derive_subkey(&r, KeyPurpose::Meta, "item-1").unwrap());
        let other = SecretKey32::from_slice(&[8u8; 32]).unwrap();
        assert_ne!(
            a1,
            derive_subkey(&other, KeyPurpose::Item, "item-1").unwrap()
        );
    }

    #[test]
    fn rejects_invalid_context() {
        let r = root();
        assert!(derive_subkey(&r, KeyPurpose::Item, "bad context").is_err());
        assert!(derive_subkey(&r, KeyPurpose::Item, "nul\0").is_err());
        assert!(derive_subkey(&r, KeyPurpose::Keyring, "").is_ok());
    }
}
