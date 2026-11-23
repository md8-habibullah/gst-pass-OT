//! Hybrid KEM: ML-KEM-1024 (FIPS 203) + X25519, combined with HKDF-SHA512
//! (`ai-docs/CRYPTO_SPEC.md` §6). The result stays secret if either component holds.

use super::error::{CryptoError, Result};
use super::rng;
use super::secret::{SecretKey32, SecretVec};
use hkdf::Hkdf;
use ml_kem::kem::{Decapsulate, Encapsulate, Key, KeyExport};
use ml_kem::{Ciphertext, DecapsulationKey1024, EncapsulationKey1024, MlKem1024, Seed};
use sha2::Sha512;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

pub const MLKEM_SEED_LEN: usize = 64;
pub const MLKEM_EK_LEN: usize = 1568;
pub const MLKEM_CT_LEN: usize = 1568;
pub const X25519_LEN: usize = 32;

const COMBINER_SALT: &[u8] = b"passman/v1/hybrid-kem-salt";
const COMBINER_INFO_PREFIX: &[u8] = b"passman/v1/hybrid-kem\0";

/// Recipient public keys (safe to store and sync).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HybridPublicKey {
    pub mlkem_ek: Vec<u8>,
    pub x25519: [u8; X25519_LEN],
}

/// What the encapsulator sends to the recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HybridCiphertext {
    pub mlkem_ct: Vec<u8>,
    pub x25519_eph: [u8; X25519_LEN],
}

/// Recipient secret keys: the 64-byte ML-KEM seed and the 32-byte X25519 secret.
/// Both are held in wiping containers; the library key objects only exist transiently inside
/// the methods below (and wipe themselves on drop via the `zeroize` features).
pub struct HybridSecretKey {
    mlkem_seed: SecretVec,
    x25519: SecretKey32,
}

impl HybridSecretKey {
    pub fn generate() -> Result<Self> {
        let mut seed = Zeroizing::new(vec![0u8; MLKEM_SEED_LEN]);
        rng::fill(&mut seed)?;
        Ok(Self {
            mlkem_seed: seed,
            x25519: SecretKey32::generate()?,
        })
    }

    pub fn from_bytes(mlkem_seed: &[u8], x25519: &[u8]) -> Result<Self> {
        if mlkem_seed.len() != MLKEM_SEED_LEN {
            return Err(CryptoError::InvalidInput("ML-KEM seed must be 64 bytes"));
        }
        Ok(Self {
            mlkem_seed: Zeroizing::new(mlkem_seed.to_vec()),
            x25519: SecretKey32::from_slice(x25519)?,
        })
    }

    pub fn mlkem_seed(&self) -> &[u8] {
        &self.mlkem_seed
    }

    pub fn x25519_secret(&self) -> &SecretKey32 {
        &self.x25519
    }

    fn mlkem_key(&self) -> Result<DecapsulationKey1024> {
        let seed = Seed::try_from(&self.mlkem_seed[..])
            .map_err(|_| CryptoError::InvalidInput("ML-KEM seed length"))?;
        Ok(DecapsulationKey1024::from_seed(seed))
    }

    fn x25519_key(&self) -> StaticSecret {
        let mut raw = *self.x25519.as_bytes();
        let key = StaticSecret::from(raw);
        raw.zeroize();
        key
    }

    pub fn public_key(&self) -> Result<HybridPublicKey> {
        let dk = self.mlkem_key()?;
        let ek = dk.encapsulation_key().to_bytes();
        let x = PublicKey::from(&self.x25519_key());
        Ok(HybridPublicKey {
            mlkem_ek: ek.as_slice().to_vec(),
            x25519: x.to_bytes(),
        })
    }

    /// Recover the wrap key for `ct`. A corrupted ML-KEM ciphertext does not error here (implicit
    /// rejection yields a pseudorandom key); the failure surfaces as an AEAD authentication error.
    pub fn decapsulate(&self, ct: &HybridCiphertext) -> Result<SecretKey32> {
        let ct_m: &Ciphertext<MlKem1024> = ct
            .mlkem_ct
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::Authentication)?;
        let dk = self.mlkem_key()?;
        let mut ss_m = dk.decapsulate(ct_m);

        let sk = self.x25519_key();
        let pk_x = PublicKey::from(&sk);
        let ss_x = sk.diffie_hellman(&PublicKey::from(ct.x25519_eph));
        let result = if ss_x.was_contributory() {
            combine(
                ss_m.as_slice(),
                ss_x.as_bytes(),
                &ct.mlkem_ct,
                &ct.x25519_eph,
                pk_x.as_bytes(),
            )
        } else {
            Err(CryptoError::Authentication)
        };
        ss_m.as_mut_slice().zeroize();
        result
    }
}

impl core::fmt::Debug for HybridSecretKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("HybridSecretKey(<redacted>)")
    }
}

/// Encapsulate a fresh wrap key to `pk`. Returns `(ciphertext to store, wrap key)`.
pub fn encapsulate(pk: &HybridPublicKey) -> Result<(HybridCiphertext, SecretKey32)> {
    let ek_bytes: &Key<EncapsulationKey1024> = pk
        .mlkem_ek
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::InvalidInput("ML-KEM public key must be 1568 bytes"))?;
    let ek = EncapsulationKey1024::new(ek_bytes)
        .map_err(|_| CryptoError::InvalidInput("invalid ML-KEM public key"))?;
    let mut rng = rng::os_rng();
    let (ct_m, mut ss_m) = ek.encapsulate_with_rng(&mut rng);

    let eph = EphemeralSecret::random_from_rng(&mut rng);
    let pk_e = PublicKey::from(&eph);
    let ss_x = eph.diffie_hellman(&PublicKey::from(pk.x25519));
    if !ss_x.was_contributory() {
        ss_m.as_mut_slice().zeroize();
        return Err(CryptoError::InvalidInput("invalid X25519 public key"));
    }
    let wrap = combine(
        ss_m.as_slice(),
        ss_x.as_bytes(),
        ct_m.as_slice(),
        pk_e.as_bytes(),
        &pk.x25519,
    );
    ss_m.as_mut_slice().zeroize();
    Ok((
        HybridCiphertext {
            mlkem_ct: ct_m.as_slice().to_vec(),
            x25519_eph: pk_e.to_bytes(),
        },
        wrap?,
    ))
}

/// `HKDF-SHA512(salt, ikm = ss_M ‖ ss_X, info = prefix ‖ ct_M ‖ pk_E ‖ pk_X) → 32 bytes`.
pub(crate) fn combine(
    ss_m: &[u8],
    ss_x: &[u8],
    ct_m: &[u8],
    pk_e: &[u8],
    pk_x: &[u8],
) -> Result<SecretKey32> {
    if ss_m.len() != 32
        || ss_x.len() != 32
        || ct_m.len() != MLKEM_CT_LEN
        || pk_e.len() != X25519_LEN
        || pk_x.len() != X25519_LEN
    {
        return Err(CryptoError::InvalidInput(
            "hybrid KEM component has the wrong length",
        ));
    }
    let mut ikm = Zeroizing::new(Vec::with_capacity(64));
    ikm.extend_from_slice(ss_m);
    ikm.extend_from_slice(ss_x);
    let mut info = Vec::with_capacity(COMBINER_INFO_PREFIX.len() + MLKEM_CT_LEN + 2 * X25519_LEN);
    info.extend_from_slice(COMBINER_INFO_PREFIX);
    info.extend_from_slice(ct_m);
    info.extend_from_slice(pk_e);
    info.extend_from_slice(pk_x);

    let hk = Hkdf::<Sha512>::new(Some(COMBINER_SALT), &ikm);
    let mut out = SecretKey32::zeroed();
    hk.expand(&info, out.as_mut_bytes())
        .map_err(|_| CryptoError::Kdf)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (HybridSecretKey, HybridPublicKey) {
        let sk = HybridSecretKey::generate().unwrap();
        let pk = sk.public_key().unwrap();
        (sk, pk)
    }

    #[test]
    fn sizes_match_fips_203_and_rfc_7748() {
        let (_, pk) = fixture();
        assert_eq!(pk.mlkem_ek.len(), MLKEM_EK_LEN);
        assert_eq!(pk.x25519.len(), 32);
        let (ct, _) = encapsulate(&pk).unwrap();
        assert_eq!(ct.mlkem_ct.len(), MLKEM_CT_LEN);
    }

    #[test]
    fn encapsulate_decapsulate_agree() {
        let (sk, pk) = fixture();
        let (ct, wrap_a) = encapsulate(&pk).unwrap();
        let wrap_b = sk.decapsulate(&ct).unwrap();
        assert_eq!(wrap_a, wrap_b);
    }

    #[test]
    fn every_encapsulation_is_fresh() {
        let (_, pk) = fixture();
        let (ct1, k1) = encapsulate(&pk).unwrap();
        let (ct2, k2) = encapsulate(&pk).unwrap();
        assert_ne!(ct1, ct2);
        assert_ne!(k1, k2);
    }

    #[test]
    fn same_seed_gives_same_public_key() {
        let (sk, pk) = fixture();
        let again =
            HybridSecretKey::from_bytes(sk.mlkem_seed(), sk.x25519_secret().as_bytes()).unwrap();
        assert_eq!(again.public_key().unwrap(), pk);
        let other = HybridSecretKey::generate().unwrap();
        assert_ne!(other.public_key().unwrap(), pk);
    }

    #[test]
    fn wrong_recipient_gets_a_different_key() {
        let (_, pk) = fixture();
        let (ct, wrap) = encapsulate(&pk).unwrap();
        let intruder = HybridSecretKey::generate().unwrap();
        assert_ne!(intruder.decapsulate(&ct).unwrap(), wrap);
    }

    #[test]
    fn corrupted_mlkem_ciphertext_yields_a_different_key_not_an_error() {
        let (sk, pk) = fixture();
        let (mut ct, wrap) = encapsulate(&pk).unwrap();
        ct.mlkem_ct[100] ^= 1;
        assert_ne!(sk.decapsulate(&ct).unwrap(), wrap);
    }

    #[test]
    fn corrupted_ephemeral_key_yields_a_different_key() {
        let (sk, pk) = fixture();
        let (mut ct, wrap) = encapsulate(&pk).unwrap();
        ct.x25519_eph[0] ^= 1;
        assert_ne!(sk.decapsulate(&ct).unwrap(), wrap);
    }

    #[test]
    fn wrong_length_ciphertext_is_rejected() {
        let (sk, pk) = fixture();
        let (mut ct, _) = encapsulate(&pk).unwrap();
        ct.mlkem_ct.pop();
        assert_eq!(sk.decapsulate(&ct).err(), Some(CryptoError::Authentication));
    }

    #[test]
    fn low_order_x25519_points_are_rejected() {
        let (sk, pk) = fixture();
        // Decapsulation side: all-zero ephemeral key gives a non-contributory secret.
        let (mut ct, _) = encapsulate(&pk).unwrap();
        ct.x25519_eph = [0u8; 32];
        assert_eq!(sk.decapsulate(&ct).err(), Some(CryptoError::Authentication));
        // Encapsulation side: all-zero recipient key.
        let bad = HybridPublicKey {
            x25519: [0u8; 32],
            ..pk
        };
        assert!(matches!(
            encapsulate(&bad),
            Err(CryptoError::InvalidInput(_))
        ));
    }

    #[test]
    fn rejects_malformed_public_keys() {
        let (_, pk) = fixture();
        let short = HybridPublicKey {
            mlkem_ek: pk.mlkem_ek[..1567].to_vec(),
            ..pk.clone()
        };
        assert!(encapsulate(&short).is_err());
        // Coefficients >= q (3329) are not a valid ML-KEM encapsulation key (FIPS 203 modulus check).
        let invalid = HybridPublicKey {
            mlkem_ek: vec![0xFF; MLKEM_EK_LEN],
            ..pk
        };
        assert!(encapsulate(&invalid).is_err());
    }

    #[test]
    fn combiner_depends_on_every_input() {
        let (m, x, ct, e, p) = (
            [1u8; 32],
            [2u8; 32],
            vec![3u8; MLKEM_CT_LEN],
            [4u8; 32],
            [5u8; 32],
        );
        let base = combine(&m, &x, &ct, &e, &p).unwrap();
        assert_eq!(base, combine(&m, &x, &ct, &e, &p).unwrap());
        assert_ne!(base, combine(&[9u8; 32], &x, &ct, &e, &p).unwrap());
        assert_ne!(base, combine(&m, &[9u8; 32], &ct, &e, &p).unwrap());
        let mut ct2 = ct.clone();
        ct2[1567] ^= 1;
        assert_ne!(base, combine(&m, &x, &ct2, &e, &p).unwrap());
        assert_ne!(base, combine(&m, &x, &ct, &[9u8; 32], &p).unwrap());
        assert_ne!(base, combine(&m, &x, &ct, &e, &[9u8; 32]).unwrap());
        // ss_M and ss_X are not interchangeable.
        assert_ne!(base, combine(&x, &m, &ct, &e, &p).unwrap());
        assert!(combine(&m[..31], &x, &ct, &e, &p).is_err());
    }

    #[test]
    fn from_bytes_validates_lengths() {
        assert!(HybridSecretKey::from_bytes(&[0u8; 63], &[0u8; 32]).is_err());
        assert!(HybridSecretKey::from_bytes(&[0u8; 64], &[0u8; 31]).is_err());
        assert!(HybridSecretKey::from_bytes(&[0u8; 64], &[0u8; 32]).is_ok());
    }

    #[test]
    fn debug_is_redacted() {
        let (sk, _) = fixture();
        assert_eq!(format!("{sk:?}"), "HybridSecretKey(<redacted>)");
    }
}
