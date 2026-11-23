//! Secret-holding types. All of them wipe their memory on drop and never print their contents.

use super::error::{CryptoError, Result};
use super::rng;
use subtle::ConstantTimeEq;
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Variable-length secret bytes (decrypted plaintext, keyring contents, ...).
pub type SecretVec = Zeroizing<Vec<u8>>;

/// A 256-bit key. Heap-boxed so that moving it around does not leave copies on the stack.
/// No `Clone`, no `Serialize`, redacted `Debug`, constant-time equality.
pub struct SecretKey32(Box<[u8; 32]>);

impl SecretKey32 {
    pub const LEN: usize = 32;

    /// All-zero key; callers must overwrite it before use.
    pub(crate) fn zeroed() -> Self {
        Self(Box::new([0u8; 32]))
    }

    /// Fresh uniformly random key from the OS CSPRNG.
    pub fn generate() -> Result<Self> {
        let mut key = Self::zeroed();
        rng::fill(&mut *key.0)?;
        Ok(key)
    }

    /// Copy 32 bytes into a new key. The caller is responsible for wiping `bytes` if it is secret.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != Self::LEN {
            return Err(CryptoError::InvalidInput("key must be exactly 32 bytes"));
        }
        let mut key = Self::zeroed();
        key.0.copy_from_slice(bytes);
        Ok(key)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn as_mut_bytes(&mut self) -> &mut [u8; 32] {
        &mut self.0
    }
}

impl Zeroize for SecretKey32 {
    fn zeroize(&mut self) {
        self.0.as_mut_slice().zeroize();
    }
}

impl Drop for SecretKey32 {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for SecretKey32 {}

impl ConstantTimeEq for SecretKey32 {
    fn ct_eq(&self, other: &Self) -> subtle::Choice {
        self.0.as_slice().ct_eq(other.0.as_slice())
    }
}

impl PartialEq for SecretKey32 {
    fn eq(&self, other: &Self) -> bool {
        self.ct_eq(other).into()
    }
}

impl Eq for SecretKey32 {}

impl core::fmt::Debug for SecretKey32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SecretKey32(<redacted>)")
    }
}

/// Upper bound on the normalized master password length (DoS guard).
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// The master password, already Unicode-NFKC-normalized and UTF-8 encoded.
/// No `Clone`, no `Serialize`, redacted `Debug`.
pub struct MasterPassword(SecretVec);

impl MasterPassword {
    /// Normalize (NFKC) and store. Rejects empty and over-long passwords.
    pub fn new(password: &str) -> Result<Self> {
        // First pass: exact normalized length, so the buffer never reallocates (a reallocation
        // would leave an unwiped copy of the password behind).
        let len: usize = password.nfkc().map(char::len_utf8).sum();
        if len == 0 {
            return Err(CryptoError::InvalidInput(
                "master password must not be empty",
            ));
        }
        if len > MAX_PASSWORD_BYTES {
            return Err(CryptoError::InvalidInput("master password is too long"));
        }
        let mut out = Zeroizing::new(Vec::with_capacity(len));
        let mut buf = [0u8; 4];
        for c in password.nfkc() {
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        buf.zeroize();
        debug_assert_eq!(out.len(), len);
        Ok(Self(out))
    }

    /// Like [`new`](Self::new) but consumes and wipes the input string.
    pub fn from_string(mut password: String) -> Result<Self> {
        let result = Self::new(&password);
        password.zeroize();
        result
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl core::fmt::Debug for MasterPassword {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MasterPassword(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_differ_and_compare_constant_time() {
        let a = SecretKey32::generate().unwrap();
        let b = SecretKey32::generate().unwrap();
        assert_ne!(a, b);
        let c = SecretKey32::from_slice(a.as_bytes()).unwrap();
        assert_eq!(a, c);
    }

    #[test]
    fn from_slice_rejects_wrong_length() {
        assert!(SecretKey32::from_slice(&[0u8; 31]).is_err());
        assert!(SecretKey32::from_slice(&[0u8; 33]).is_err());
    }

    #[test]
    fn debug_never_leaks() {
        let k = SecretKey32::from_slice(&[0xAB; 32]).unwrap();
        let s = format!("{k:?}");
        assert!(!s.to_lowercase().contains("ab"), "{s}");
        let p = MasterPassword::new("hunter2hunter2").unwrap();
        assert!(!format!("{p:?}").contains("hunter"));
    }

    #[test]
    fn explicit_zeroize_clears_key() {
        let mut k = SecretKey32::from_slice(&[0x5A; 32]).unwrap();
        k.zeroize();
        assert_eq!(k.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn password_is_nfkc_normalized() {
        // "é" precomposed vs "e" + combining acute must be identical after NFKC.
        let a = MasterPassword::new("caf\u{00e9}-pass").unwrap();
        let b = MasterPassword::new("cafe\u{0301}-pass").unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
        // Compatibility folding: full-width digits and the "fi" ligature.
        let c = MasterPassword::new("\u{ff11}\u{ff12}\u{fb01}").unwrap();
        assert_eq!(c.as_bytes(), "12fi".as_bytes());
    }

    #[test]
    fn password_length_rules() {
        assert!(MasterPassword::new("").is_err());
        assert!(MasterPassword::new(&"a".repeat(MAX_PASSWORD_BYTES)).is_ok());
        assert!(MasterPassword::new(&"a".repeat(MAX_PASSWORD_BYTES + 1)).is_err());
        // Length limit applies to the normalized UTF-8 size, not the character count.
        assert!(MasterPassword::new(&"\u{20ac}".repeat(400)).is_err()); // 3 bytes each = 1200
    }

    #[test]
    fn from_string_wipes_the_source() {
        let source = String::from("correct horse battery staple");
        let p = MasterPassword::from_string(source).unwrap();
        assert_eq!(p.as_bytes(), b"correct horse battery staple");
    }
}
