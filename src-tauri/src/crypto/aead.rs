//! XChaCha20-Poly1305 primitive. Crate-private: all callers go through `envelope`, which owns
//! the nonce policy (fresh random 24-byte nonce per seal).

use super::error::{CryptoError, Result};
use super::secret::{SecretKey32, SecretVec};
use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

fn cipher(key: &SecretKey32) -> Result<XChaCha20Poly1305> {
    XChaCha20Poly1305::new_from_slice(key.as_bytes())
        .map_err(|_| CryptoError::InvalidInput("bad AEAD key length"))
}

fn nonce(bytes: &[u8; NONCE_LEN]) -> Result<XNonce> {
    XNonce::try_from(&bytes[..]).map_err(|_| CryptoError::InvalidInput("bad nonce length"))
}

/// Returns `ciphertext ‖ tag`.
pub(crate) fn encrypt(
    key: &SecretKey32,
    nonce_bytes: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = cipher(key)?;
    let nonce = nonce(nonce_bytes)?;
    // Capacity includes the tag, so encryption never reallocates (a reallocation would
    // leave a copy of the plaintext in freed memory).
    let mut buf = Vec::with_capacity(plaintext.len() + TAG_LEN);
    buf.extend_from_slice(plaintext);
    cipher
        .encrypt_in_place(&nonce, aad, &mut buf)
        .map_err(|_| CryptoError::TooLarge)?;
    Ok(buf)
}

/// Verifies the tag and returns the plaintext in a zeroizing buffer. Any failure is the single
/// opaque `Authentication` error.
pub(crate) fn decrypt(
    key: &SecretKey32,
    nonce_bytes: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<SecretVec> {
    if ciphertext_and_tag.len() < TAG_LEN {
        return Err(CryptoError::Authentication);
    }
    let cipher = cipher(key)?;
    let nonce = nonce(nonce_bytes)?;
    let mut buf: SecretVec = Zeroizing::new(Vec::with_capacity(ciphertext_and_tag.len()));
    buf.extend_from_slice(ciphertext_and_tag);
    cipher
        .decrypt_in_place(&nonce, aad, &mut *buf)
        .map_err(|_| CryptoError::Authentication)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SecretKey32 {
        SecretKey32::from_slice(&[0x42; 32]).unwrap()
    }

    #[test]
    fn round_trip_and_overhead() {
        let n = [9u8; NONCE_LEN];
        let ct = encrypt(&key(), &n, b"aad", b"hello world").unwrap();
        assert_eq!(ct.len(), 11 + TAG_LEN);
        let pt = decrypt(&key(), &n, b"aad", &ct).unwrap();
        assert_eq!(&pt[..], b"hello world");
    }

    #[test]
    fn empty_plaintext_works() {
        let n = [1u8; NONCE_LEN];
        let ct = encrypt(&key(), &n, b"", b"").unwrap();
        assert_eq!(ct.len(), TAG_LEN);
        assert!(decrypt(&key(), &n, b"", &ct).unwrap().is_empty());
    }

    #[test]
    fn any_change_fails_authentication() {
        let n = [3u8; NONCE_LEN];
        let ct = encrypt(&key(), &n, b"aad", b"secret data").unwrap();

        // wrong key / nonce / aad
        let other = SecretKey32::from_slice(&[0x43; 32]).unwrap();
        assert_eq!(
            decrypt(&other, &n, b"aad", &ct).err(),
            Some(CryptoError::Authentication)
        );
        assert!(decrypt(&key(), &[4u8; NONCE_LEN], b"aad", &ct).is_err());
        assert!(decrypt(&key(), &n, b"AAD", &ct).is_err());

        // every single-bit flip of ciphertext+tag
        for i in 0..ct.len() {
            let mut bad = ct.clone();
            bad[i] ^= 1;
            assert_eq!(
                decrypt(&key(), &n, b"aad", &bad).unwrap_err(),
                CryptoError::Authentication,
                "byte {i}"
            );
        }
        // truncation
        assert!(decrypt(&key(), &n, b"aad", &ct[..ct.len() - 1]).is_err());
        assert!(decrypt(&key(), &n, b"aad", &ct[..TAG_LEN - 1]).is_err());
        assert!(decrypt(&key(), &n, b"aad", &[]).is_err());
    }
}
