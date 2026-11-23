//! The vault key bundle: create / unlock / verify / change password
//! (`ai-docs/CRYPTO_SPEC.md` §8–§10).
//!
//! ```text
//! master password ─NFKC─▶ Argon2id ─▶ PasswordKey ─HKDF("keyring")─▶ seals ─▶ keyring {mlkem_seed, x25519_sk, vrk_commit}
//! random VaultRootKey ─hybrid KEM wrap (to the public keys)─▶ wrappedRootKey
//! VaultRootKey ─HKDF(label, id)─▶ per-object keys (items, versions, attachments, meta)
//! ```

use super::envelope::{self, Expect, Header, Opened};
use super::error::{CryptoError, Result};
use super::ids::{KEYRING_ID, ROOT_KEY_ID};
use super::kdf::{self, KdfParams, SALT_LEN};
use super::kem::{
    self, HybridCiphertext, HybridSecretKey, MLKEM_CT_LEN, MLKEM_EK_LEN, MLKEM_SEED_LEN, X25519_LEN,
};
use super::secret::{MasterPassword, SecretKey32, SecretVec};
use super::subkey::{derive_subkey, KeyPurpose};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub const BUNDLE_FORMAT: u32 = 1;
const ARGON2_VERSION_0X13: u32 = 19;
const KEYRING_LEN: usize = MLKEM_SEED_LEN + X25519_LEN + 32;
/// Upper bound on a bundle JSON document (the real size is about 5 KiB).
const MAX_BUNDLE_JSON_BYTES: usize = 16 * 1024;
/// Upper bound on a sealed small object inside the bundle (the real size is about 250 bytes).
const MAX_SEALED_BYTES: usize = 2048;

mod b64 {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD
            .decode(text.as_bytes())
            .map_err(|_| serde::de::Error::custom("invalid base64"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleKdf {
    pub alg: String,
    pub version: u32,
    #[serde(rename = "mCostKiB")]
    pub m_cost_kib: u32,
    #[serde(rename = "tCost")]
    pub t_cost: u32,
    #[serde(rename = "pCost")]
    pub p_cost: u32,
    #[serde(with = "b64")]
    pub salt: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundlePublicKeys {
    #[serde(with = "b64")]
    pub ml_kem_1024: Vec<u8>,
    #[serde(with = "b64")]
    pub x25519: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WrappedRootKey {
    #[serde(with = "b64")]
    pub ml_kem_ct: Vec<u8>,
    #[serde(with = "b64")]
    pub x25519_eph: Vec<u8>,
    #[serde(with = "b64")]
    pub envelope: Vec<u8>,
}

/// Everything needed to unlock a vault from the master password. Contains no secrets in the
/// clear and is safe to store locally and sync to S3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VaultKeyBundle {
    pub format: u32,
    pub created_at: u64,
    pub kdf: BundleKdf,
    pub public_keys: BundlePublicKeys,
    #[serde(with = "b64")]
    pub sealed_keyring: Vec<u8>,
    pub wrapped_root_key: WrappedRootKey,
}

impl VaultKeyBundle {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|_| CryptoError::MalformedBundle("serialization failed"))
    }

    /// Parse **and validate** (format, algorithms, parameter bounds, every length).
    pub fn from_json(json: &str) -> Result<Self> {
        if json.len() > MAX_BUNDLE_JSON_BYTES {
            return Err(CryptoError::MalformedBundle("document too large"));
        }
        let bundle: Self = serde_json::from_str(json)
            .map_err(|_| CryptoError::MalformedBundle("not a valid bundle document"))?;
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn kdf_params(&self) -> KdfParams {
        KdfParams {
            m_cost_kib: self.kdf.m_cost_kib,
            t_cost: self.kdf.t_cost,
            p_cost: self.kdf.p_cost,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != BUNDLE_FORMAT {
            return Err(CryptoError::Unsupported("bundle format version"));
        }
        if self.kdf.alg != envelope::SUITE_KDF || self.kdf.version != ARGON2_VERSION_0X13 {
            return Err(CryptoError::Unsupported("key derivation algorithm"));
        }
        self.kdf_params().validate()?;
        if self.kdf.salt.len() != SALT_LEN {
            return Err(CryptoError::MalformedBundle("salt length"));
        }
        if self.public_keys.ml_kem_1024.len() != MLKEM_EK_LEN
            || self.public_keys.x25519.len() != X25519_LEN
        {
            return Err(CryptoError::MalformedBundle("public key length"));
        }
        let w = &self.wrapped_root_key;
        if w.ml_kem_ct.len() != MLKEM_CT_LEN || w.x25519_eph.len() != X25519_LEN {
            return Err(CryptoError::MalformedBundle("wrapped key length"));
        }
        for sealed in [&self.sealed_keyring, &w.envelope] {
            if sealed.is_empty() || sealed.len() > MAX_SEALED_BYTES {
                return Err(CryptoError::MalformedBundle("sealed object length"));
            }
        }
        Ok(())
    }
}

/// The 256-bit key that encrypts the vault. Exists only in memory while unlocked.
pub struct VaultRootKey(SecretKey32);

impl VaultRootKey {
    /// Per-object key (`HKDF(root, purpose, id)`).
    pub fn object_key(&self, purpose: KeyPurpose, id: &str) -> Result<SecretKey32> {
        derive_subkey(&self.0, purpose, id)
    }

    /// Encrypt an object (item state, version snapshot, attachment, setting) under its own key.
    pub fn seal_object(
        &self,
        purpose: KeyPurpose,
        id: &str,
        version: u64,
        timestamp: u64,
        plaintext: &[u8],
    ) -> Result<Vec<u8>> {
        let key = self.object_key(purpose, id)?;
        envelope::seal(&key, &Header::new(id, version, timestamp)?, plaintext)
    }

    /// Authenticate and decrypt an object. `version = Some(n)` pins the expected version.
    pub fn open_object(
        &self,
        purpose: KeyPurpose,
        id: &str,
        version: Option<u64>,
        bytes: &[u8],
    ) -> Result<Opened> {
        let key = self.object_key(purpose, id)?;
        envelope::open(
            &key,
            bytes,
            &Expect {
                item_id: id,
                version,
            },
        )
    }

    /// Raw key access for wrapping (quick unlock, Phase 3). Crate-private on purpose.
    #[allow(dead_code)]
    pub(crate) fn secret(&self) -> &SecretKey32 {
        &self.0
    }
}

impl core::fmt::Debug for VaultRootKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("VaultRootKey(<redacted>)")
    }
}

fn vrk_commitment(vrk: &SecretKey32) -> Result<SecretKey32> {
    derive_subkey(vrk, KeyPurpose::VrkCommit, "")
}

fn pack_keyring(secret: &HybridSecretKey, commit: &SecretKey32) -> SecretVec {
    let mut out = Zeroizing::new(Vec::with_capacity(KEYRING_LEN));
    out.extend_from_slice(secret.mlkem_seed());
    out.extend_from_slice(secret.x25519_secret().as_bytes());
    out.extend_from_slice(commit.as_bytes());
    out
}

fn unpack_keyring(plain: &[u8]) -> Result<(HybridSecretKey, SecretKey32)> {
    if plain.len() != KEYRING_LEN {
        return Err(CryptoError::MalformedBundle("keyring length"));
    }
    let (seed, rest) = plain.split_at(MLKEM_SEED_LEN);
    let (x25519, commit) = rest.split_at(X25519_LEN);
    Ok((
        HybridSecretKey::from_bytes(seed, x25519)?,
        SecretKey32::from_slice(commit)?,
    ))
}

/// Create a brand-new vault: returns the public bundle and the unlocked root key.
pub fn create(
    password: &MasterPassword,
    params: &KdfParams,
) -> Result<(VaultKeyBundle, VaultRootKey)> {
    params.validate()?;
    let salt = kdf::generate_salt()?;
    let password_key = kdf::derive_password_key(password, &salt, params)?;

    let secret = HybridSecretKey::generate()?;
    let public = secret.public_key()?;
    let vrk = SecretKey32::generate()?;
    let now = super::now_millis();

    let (ct, wrap_key) = kem::encapsulate(&public)?;
    let wrapped = envelope::seal(
        &wrap_key,
        &Header::new(ROOT_KEY_ID, 1, now)?,
        vrk.as_bytes(),
    )?;

    let commit = vrk_commitment(&vrk)?;
    let keyring_key = derive_subkey(&password_key, KeyPurpose::Keyring, "")?;
    let sealed_keyring = envelope::seal(
        &keyring_key,
        &Header::new(KEYRING_ID, 1, now)?,
        &pack_keyring(&secret, &commit),
    )?;

    let bundle = VaultKeyBundle {
        format: BUNDLE_FORMAT,
        created_at: now,
        kdf: BundleKdf {
            alg: envelope::SUITE_KDF.into(),
            version: ARGON2_VERSION_0X13,
            m_cost_kib: params.m_cost_kib,
            t_cost: params.t_cost,
            p_cost: params.p_cost,
            salt: salt.to_vec(),
        },
        public_keys: BundlePublicKeys {
            ml_kem_1024: public.mlkem_ek,
            x25519: public.x25519.to_vec(),
        },
        sealed_keyring,
        wrapped_root_key: WrappedRootKey {
            ml_kem_ct: ct.mlkem_ct,
            x25519_eph: ct.x25519_eph.to_vec(),
            envelope: wrapped,
        },
    };
    bundle.validate()?;
    Ok((bundle, VaultRootKey(vrk)))
}

/// Argon2id + open the sealed keyring. Wrong password and damaged file both map to `Unlock`.
fn open_keyring(bundle: &VaultKeyBundle, password: &MasterPassword) -> Result<(SecretVec, Header)> {
    bundle.validate()?;
    let password_key = kdf::derive_password_key(password, &bundle.kdf.salt, &bundle.kdf_params())?;
    let keyring_key = derive_subkey(&password_key, KeyPurpose::Keyring, "")?;
    let opened = envelope::open(
        &keyring_key,
        &bundle.sealed_keyring,
        &Expect {
            item_id: KEYRING_ID,
            version: None,
        },
    )
    .map_err(|e| match e {
        CryptoError::Authentication | CryptoError::MalformedEnvelope(_) => CryptoError::Unlock,
        other => other,
    })?;
    Ok((opened.plaintext, opened.header))
}

/// Check the master password without unlocking (Master-Password Re-Prompt Guard).
/// Costs one Argon2id run by design.
pub fn verify_password(bundle: &VaultKeyBundle, password: &MasterPassword) -> Result<()> {
    open_keyring(bundle, password).map(|_| ())
}

/// Unlock: master password → root key. Detects any tampering with the stored key material.
pub fn unlock(bundle: &VaultKeyBundle, password: &MasterPassword) -> Result<VaultRootKey> {
    let (plain, _) = open_keyring(bundle, password)?;
    let (secret, commit) = unpack_keyring(&plain)?;

    // The public keys in the bundle must be the ones belonging to the sealed secrets.
    let derived = secret.public_key()?;
    if derived.mlkem_ek != bundle.public_keys.ml_kem_1024
        || derived.x25519[..] != bundle.public_keys.x25519[..]
    {
        return Err(CryptoError::KeyMismatch);
    }

    let w = &bundle.wrapped_root_key;
    let ct = HybridCiphertext {
        mlkem_ct: w.ml_kem_ct.clone(),
        x25519_eph: w
            .x25519_eph
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::MalformedBundle("wrapped key length"))?,
    };
    let wrap_key = secret
        .decapsulate(&ct)
        .map_err(|_| CryptoError::KeyMismatch)?;
    drop(secret); // wipe the KEM secrets as early as possible

    let opened = envelope::open(
        &wrap_key,
        &w.envelope,
        &Expect {
            item_id: ROOT_KEY_ID,
            version: None,
        },
    )
    .map_err(|_| CryptoError::KeyMismatch)?;
    let vrk = SecretKey32::from_slice(&opened.plaintext).map_err(|_| CryptoError::KeyMismatch)?;

    // Defeats root-key substitution: public keys are public, so anyone could wrap a different key.
    if !bool::from(vrk_commitment(&vrk)?.ct_eq(&commit)) {
        return Err(CryptoError::KeyMismatch);
    }
    Ok(VaultRootKey(vrk))
}

/// Re-seal the same keyring under a new password (new salt, new parameters). The wrapped root
/// key and public keys are unchanged, so no vault data has to be re-encrypted.
pub fn change_password(
    bundle: &VaultKeyBundle,
    old_password: &MasterPassword,
    new_password: &MasterPassword,
    new_params: &KdfParams,
) -> Result<VaultKeyBundle> {
    new_params.validate()?;
    let (plain, old_header) = open_keyring(bundle, old_password)?;

    let salt = kdf::generate_salt()?;
    let password_key = kdf::derive_password_key(new_password, &salt, new_params)?;
    let keyring_key = derive_subkey(&password_key, KeyPurpose::Keyring, "")?;
    let next_version = old_header
        .version
        .checked_add(1)
        .ok_or(CryptoError::InvalidInput("keyring version overflow"))?;
    let sealed_keyring = envelope::seal(
        &keyring_key,
        &Header::new(KEYRING_ID, next_version, super::now_millis())?,
        &plain,
    )?;

    let mut next = bundle.clone();
    next.kdf = BundleKdf {
        alg: envelope::SUITE_KDF.into(),
        version: ARGON2_VERSION_0X13,
        m_cost_kib: new_params.m_cost_kib,
        t_cost: new_params.t_cost,
        p_cost: new_params.p_cost,
        salt: salt.to_vec(),
    };
    next.sealed_keyring = sealed_keyring;
    next.validate()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::kem::HybridPublicKey;
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    fn pw(s: &str) -> MasterPassword {
        MasterPassword::new(s).unwrap()
    }

    fn fast() -> KdfParams {
        KdfParams {
            m_cost_kib: kdf::MIN_M_COST_KIB,
            t_cost: kdf::MIN_T_COST,
            p_cost: 1,
        }
    }

    fn vault() -> (VaultKeyBundle, VaultRootKey) {
        create(&pw("correct horse battery staple"), &fast()).unwrap()
    }

    #[test]
    fn create_then_unlock_returns_the_same_root_key() {
        let (bundle, vrk) = vault();
        let again = unlock(&bundle, &pw("correct horse battery staple")).unwrap();
        assert_eq!(vrk.secret(), again.secret());
    }

    #[test]
    fn default_parameters_work_end_to_end() {
        let (bundle, vrk) = create(
            &pw("a long default-parameter passphrase"),
            &KdfParams::default(),
        )
        .unwrap();
        assert_eq!(
            (bundle.kdf.m_cost_kib, bundle.kdf.t_cost, bundle.kdf.p_cost),
            (65_536, 4, 4)
        );
        assert_eq!(bundle.kdf.salt.len(), 32);
        let again = unlock(&bundle, &pw("a long default-parameter passphrase")).unwrap();
        assert_eq!(vrk.secret(), again.secret());
    }

    #[test]
    fn wrong_password_is_rejected_opaquely() {
        let (bundle, _) = vault();
        assert_eq!(
            unlock(&bundle, &pw("wrong")).err(),
            Some(CryptoError::Unlock)
        );
        assert_eq!(
            verify_password(&bundle, &pw("wrong")),
            Err(CryptoError::Unlock)
        );
        assert!(verify_password(&bundle, &pw("correct horse battery staple")).is_ok());
    }

    #[test]
    fn nfkc_equivalent_passwords_unlock_the_same_vault() {
        let (bundle, vrk) = create(&pw("caf\u{00e9}-vault-pass"), &fast()).unwrap();
        let again = unlock(&bundle, &pw("cafe\u{0301}-vault-pass")).unwrap();
        assert_eq!(vrk.secret(), again.secret());
    }

    #[test]
    fn every_vault_gets_fresh_keys() {
        let (b1, v1) = vault();
        let (b2, v2) = vault();
        assert_ne!(v1.secret(), v2.secret());
        assert_ne!(b1.kdf.salt, b2.kdf.salt);
        assert_ne!(b1.public_keys, b2.public_keys);
    }

    #[test]
    fn json_round_trip_and_field_names() {
        let (bundle, vrk) = vault();
        let json = bundle.to_json().unwrap();
        for field in [
            "\"format\":1",
            "\"createdAt\"",
            "\"mCostKiB\"",
            "\"tCost\"",
            "\"pCost\"",
            "\"salt\"",
            "\"mlKem1024\"",
            "\"x25519\"",
            "\"sealedKeyring\"",
            "\"wrappedRootKey\"",
            "\"mlKemCt\"",
            "\"x25519Eph\"",
            "\"envelope\"",
            "\"alg\":\"Argon2id\"",
            "\"version\":19",
        ] {
            assert!(json.contains(field), "missing {field} in {json}");
        }
        let parsed = VaultKeyBundle::from_json(&json).unwrap();
        assert_eq!(parsed, bundle);
        let unlocked = unlock(&parsed, &pw("correct horse battery staple")).unwrap();
        assert_eq!(unlocked.secret(), vrk.secret());
        // The public document must not contain the root key.
        assert!(!json.contains(&STANDARD.encode(vrk.secret().as_bytes())));
    }

    #[test]
    fn parser_rejects_hostile_documents() {
        let (bundle, _) = vault();
        let json = bundle.to_json().unwrap();
        assert!(VaultKeyBundle::from_json("{}").is_err());
        assert!(VaultKeyBundle::from_json("not json").is_err());
        assert!(VaultKeyBundle::from_json(&"x".repeat(MAX_BUNDLE_JSON_BYTES + 1)).is_err());
        assert!(VaultKeyBundle::from_json(&json.replace("\"format\":1", "\"format\":2")).is_err());
        assert!(VaultKeyBundle::from_json(
            &json.replace("\"alg\":\"Argon2id\"", "\"alg\":\"Argon2i\"")
        )
        .is_err());
        assert!(
            VaultKeyBundle::from_json(&json.replace("\"version\":19", "\"version\":16")).is_err()
        );
        // Memory-exhaustion attempt through the parameters.
        assert!(matches!(
            VaultKeyBundle::from_json(&json.replace("\"mCostKiB\":19456", "\"mCostKiB\":4194304")),
            Err(CryptoError::InvalidKdfParams(_))
        ));
        assert!(matches!(
            VaultKeyBundle::from_json(&json.replace("\"tCost\":2", "\"tCost\":1000000")),
            Err(CryptoError::InvalidKdfParams(_))
        ));
        // Unknown field.
        assert!(VaultKeyBundle::from_json(&json.replacen('{', "{\"extra\":1,", 1)).is_err());
    }

    #[test]
    fn tampering_with_any_part_is_detected() {
        let (bundle, _) = vault();
        let good = pw("correct horse battery staple");
        let flip = |v: &mut Vec<u8>, i: usize| v[i] ^= 0x01;

        let mut b = bundle.clone();
        flip(&mut b.kdf.salt, 0);
        assert_eq!(unlock(&b, &good).err(), Some(CryptoError::Unlock), "salt");

        let mut b = bundle.clone();
        let last = b.sealed_keyring.len() - 1;
        flip(&mut b.sealed_keyring, last);
        assert_eq!(
            unlock(&b, &good).err(),
            Some(CryptoError::Unlock),
            "sealed keyring tag"
        );

        let mut b = bundle.clone();
        flip(&mut b.wrapped_root_key.ml_kem_ct, 7);
        assert_eq!(
            unlock(&b, &good).err(),
            Some(CryptoError::KeyMismatch),
            "ml-kem ciphertext"
        );

        let mut b = bundle.clone();
        flip(&mut b.wrapped_root_key.x25519_eph, 0);
        assert_eq!(
            unlock(&b, &good).err(),
            Some(CryptoError::KeyMismatch),
            "x25519 ephemeral"
        );

        let mut b = bundle.clone();
        let last = b.wrapped_root_key.envelope.len() - 1;
        flip(&mut b.wrapped_root_key.envelope, last);
        assert_eq!(
            unlock(&b, &good).err(),
            Some(CryptoError::KeyMismatch),
            "wrapped envelope"
        );

        let mut b = bundle.clone();
        flip(&mut b.public_keys.ml_kem_1024, 5);
        assert_eq!(
            unlock(&b, &good).err(),
            Some(CryptoError::KeyMismatch),
            "ml-kem public key"
        );

        let mut b = bundle.clone();
        flip(&mut b.public_keys.x25519, 5);
        assert_eq!(
            unlock(&b, &good).err(),
            Some(CryptoError::KeyMismatch),
            "x25519 public key"
        );
    }

    #[test]
    fn root_key_substitution_is_detected() {
        // Anyone can wrap a different root key to the (public) recipient keys. The commitment
        // inside the password-sealed keyring must expose that.
        let (bundle, genuine) = vault();
        let public = HybridPublicKey {
            mlkem_ek: bundle.public_keys.ml_kem_1024.clone(),
            x25519: bundle.public_keys.x25519.as_slice().try_into().unwrap(),
        };
        let evil_vrk = SecretKey32::generate().unwrap();
        assert_ne!(&evil_vrk, genuine.secret());
        let (ct, wrap_key) = kem::encapsulate(&public).unwrap();
        let forged = envelope::seal(
            &wrap_key,
            &Header::new(ROOT_KEY_ID, 1, 1).unwrap(),
            evil_vrk.as_bytes(),
        )
        .unwrap();

        let mut attacked = bundle.clone();
        attacked.wrapped_root_key = WrappedRootKey {
            ml_kem_ct: ct.mlkem_ct,
            x25519_eph: ct.x25519_eph.to_vec(),
            envelope: forged,
        };
        attacked.validate().unwrap();
        assert_eq!(
            unlock(&attacked, &pw("correct horse battery staple")).err(),
            Some(CryptoError::KeyMismatch)
        );
    }

    #[test]
    fn change_password_keeps_the_vault_and_replaces_the_credentials() {
        let (bundle, vrk) = vault();
        let new_params = KdfParams {
            t_cost: 3,
            ..fast()
        };
        let changed = change_password(
            &bundle,
            &pw("correct horse battery staple"),
            &pw("a brand new passphrase"),
            &new_params,
        )
        .unwrap();

        // Same data keys, nothing to re-encrypt.
        assert_eq!(changed.wrapped_root_key, bundle.wrapped_root_key);
        assert_eq!(changed.public_keys, bundle.public_keys);
        // New credentials.
        assert_ne!(changed.kdf.salt, bundle.kdf.salt);
        assert_eq!(changed.kdf.t_cost, 3);
        assert_ne!(changed.sealed_keyring, bundle.sealed_keyring);
        assert_eq!(
            envelope::peek_header(&changed.sealed_keyring)
                .unwrap()
                .version,
            2
        );

        let again = unlock(&changed, &pw("a brand new passphrase")).unwrap();
        assert_eq!(again.secret(), vrk.secret());
        assert_eq!(
            unlock(&changed, &pw("correct horse battery staple")).err(),
            Some(CryptoError::Unlock)
        );
        // The old bundle is untouched and still works with the old password.
        assert!(unlock(&bundle, &pw("correct horse battery staple")).is_ok());
    }

    #[test]
    fn change_password_requires_the_old_password_and_valid_params() {
        let (bundle, _) = vault();
        assert_eq!(
            change_password(&bundle, &pw("nope"), &pw("new"), &fast()).err(),
            Some(CryptoError::Unlock)
        );
        let weak = KdfParams {
            m_cost_kib: 1024,
            ..fast()
        };
        assert!(matches!(
            change_password(
                &bundle,
                &pw("correct horse battery staple"),
                &pw("new"),
                &weak
            ),
            Err(CryptoError::InvalidKdfParams(_))
        ));
        // Repeated changes keep bumping the keyring version.
        let c1 = change_password(
            &bundle,
            &pw("correct horse battery staple"),
            &pw("second"),
            &fast(),
        )
        .unwrap();
        let c2 = change_password(&c1, &pw("second"), &pw("third"), &fast()).unwrap();
        assert_eq!(
            envelope::peek_header(&c2.sealed_keyring).unwrap().version,
            3
        );
        assert!(unlock(&c2, &pw("third")).is_ok());
    }

    #[test]
    fn create_rejects_weak_parameters() {
        let weak = KdfParams {
            t_cost: 1,
            ..fast()
        };
        assert!(matches!(
            create(&pw("x"), &weak),
            Err(CryptoError::InvalidKdfParams(_))
        ));
    }

    #[test]
    fn objects_are_sealed_under_independent_keys() {
        let (_, vrk) = vault();
        let id = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";
        let sealed = vrk
            .seal_object(
                KeyPurpose::Item,
                id,
                1,
                1_732_360_000_000,
                b"{\"title\":\"x\"}",
            )
            .unwrap();
        let opened = vrk
            .open_object(KeyPurpose::Item, id, Some(1), &sealed)
            .unwrap();
        assert_eq!(&opened.plaintext[..], b"{\"title\":\"x\"}");
        assert_eq!(opened.header.timestamp, 1_732_360_000_000);

        // Another purpose or another id has another key: opening fails authentication.
        assert_eq!(
            vrk.open_object(KeyPurpose::Version, id, None, &sealed)
                .err(),
            Some(CryptoError::Authentication)
        );
        assert!(vrk
            .open_object(KeyPurpose::Item, "other-item", None, &sealed)
            .is_err());
        // Pinned version mismatch.
        assert!(vrk
            .open_object(KeyPurpose::Item, id, Some(2), &sealed)
            .is_err());
        // A different vault cannot open it.
        let (_, stranger) = vault();
        assert_eq!(
            stranger
                .open_object(KeyPurpose::Item, id, None, &sealed)
                .err(),
            Some(CryptoError::Authentication)
        );
    }

    #[test]
    fn keyring_layout_is_exactly_128_bytes() {
        let secret = HybridSecretKey::generate().unwrap();
        let commit = SecretKey32::generate().unwrap();
        let packed = pack_keyring(&secret, &commit);
        assert_eq!(packed.len(), 128);
        let (s2, c2) = unpack_keyring(&packed).unwrap();
        assert_eq!(s2.mlkem_seed(), secret.mlkem_seed());
        assert_eq!(c2, commit);
        assert!(unpack_keyring(&packed[..127]).is_err());
    }
}
