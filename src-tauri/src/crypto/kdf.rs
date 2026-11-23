//! Argon2id password key derivation (`ai-docs/CRYPTO_SPEC.md` §4).

use super::error::{CryptoError, Result};
use super::rng;
use super::secret::{MasterPassword, SecretKey32};
use argon2::{Algorithm, Argon2, Block, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const SALT_LEN: usize = 32;

pub const MIN_M_COST_KIB: u32 = 19_456; // 19 MiB (OWASP minimum)
pub const MAX_M_COST_KIB: u32 = 1_048_576; // 1 GiB
pub const MIN_T_COST: u32 = 2;
pub const MAX_T_COST: u32 = 64;
pub const MIN_P_COST: u32 = 1;
pub const MAX_P_COST: u32 = 16;

/// Argon2id cost parameters. Defaults: 64 MiB, 4 iterations, 4 lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KdfParams {
    #[serde(rename = "mCostKiB")]
    pub m_cost_kib: u32,
    #[serde(rename = "tCost")]
    pub t_cost: u32,
    #[serde(rename = "pCost")]
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost_kib: 65_536,
            t_cost: 4,
            p_cost: 4,
        }
    }
}

impl KdfParams {
    /// Enforced on creation and on every parse of stored parameters: the lower bounds keep the
    /// KDF meaningful, the upper bounds stop a tampered bundle from forcing huge allocations.
    pub fn validate(&self) -> Result<()> {
        if !(MIN_M_COST_KIB..=MAX_M_COST_KIB).contains(&self.m_cost_kib) {
            return Err(CryptoError::InvalidKdfParams(
                "memory cost must be 19456..=1048576 KiB",
            ));
        }
        if !(MIN_T_COST..=MAX_T_COST).contains(&self.t_cost) {
            return Err(CryptoError::InvalidKdfParams("iterations must be 2..=64"));
        }
        if !(MIN_P_COST..=MAX_P_COST).contains(&self.p_cost) {
            return Err(CryptoError::InvalidKdfParams("parallelism must be 1..=16"));
        }
        Ok(())
    }
}

pub fn generate_salt() -> Result<[u8; SALT_LEN]> {
    rng::array::<SALT_LEN>()
}

/// `Argon2id(v0x13, password, salt, m, t, p) → 32 bytes`.
///
/// The 64 MiB+ work memory is allocated here and wiped afterwards (the crate's own buffer would
/// be freed without wiping, and it is derived from the password).
pub fn derive_password_key(
    password: &MasterPassword,
    salt: &[u8],
    params: &KdfParams,
) -> Result<SecretKey32> {
    params.validate()?;
    if salt.len() != SALT_LEN {
        return Err(CryptoError::InvalidInput("salt must be exactly 32 bytes"));
    }
    let argon_params = Params::new(params.m_cost_kib, params.t_cost, params.p_cost, Some(32))
        .map_err(|_| CryptoError::InvalidKdfParams("rejected by Argon2"))?;
    let blocks_len = argon_params.block_count();
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

    let mut blocks: Zeroizing<Vec<Block>> = Zeroizing::new(Vec::new());
    blocks
        .try_reserve_exact(blocks_len)
        .map_err(|_| CryptoError::KdfOutOfMemory)?;
    blocks.resize(blocks_len, Block::default());

    let mut out = SecretKey32::zeroed();
    argon
        .hash_password_into_with_memory(
            password.as_bytes(),
            salt,
            out.as_mut_bytes(),
            blocks.as_mut_slice(),
        )
        .map_err(|_| CryptoError::Kdf)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pw(s: &str) -> MasterPassword {
        MasterPassword::new(s).unwrap()
    }

    fn fast() -> KdfParams {
        KdfParams {
            m_cost_kib: MIN_M_COST_KIB,
            t_cost: MIN_T_COST,
            p_cost: 1,
        }
    }

    #[test]
    fn defaults_match_spec() {
        let d = KdfParams::default();
        assert_eq!((d.m_cost_kib, d.t_cost, d.p_cost), (65_536, 4, 4));
        assert!(d.validate().is_ok());
    }

    #[test]
    fn validate_enforces_both_bounds() {
        let ok = fast();
        assert!(ok.validate().is_ok());
        assert!(KdfParams {
            m_cost_kib: MIN_M_COST_KIB - 1,
            ..ok
        }
        .validate()
        .is_err());
        assert!(KdfParams {
            m_cost_kib: MAX_M_COST_KIB + 1,
            ..ok
        }
        .validate()
        .is_err());
        assert!(KdfParams { t_cost: 1, ..ok }.validate().is_err());
        assert!(KdfParams {
            t_cost: MAX_T_COST + 1,
            ..ok
        }
        .validate()
        .is_err());
        assert!(KdfParams { p_cost: 0, ..ok }.validate().is_err());
        assert!(KdfParams {
            p_cost: MAX_P_COST + 1,
            ..ok
        }
        .validate()
        .is_err());
    }

    #[test]
    fn deterministic_and_input_sensitive() {
        let salt = [1u8; SALT_LEN];
        let a = derive_password_key(&pw("password one"), &salt, &fast()).unwrap();
        let b = derive_password_key(&pw("password one"), &salt, &fast()).unwrap();
        assert_eq!(a, b);
        assert_ne!(
            a,
            derive_password_key(&pw("password two"), &salt, &fast()).unwrap()
        );
        assert_ne!(
            a,
            derive_password_key(&pw("password one"), &[2u8; SALT_LEN], &fast()).unwrap()
        );
        let more = KdfParams {
            t_cost: 3,
            ..fast()
        };
        assert_ne!(
            a,
            derive_password_key(&pw("password one"), &salt, &more).unwrap()
        );
    }

    #[test]
    fn rejects_bad_salt_and_params() {
        assert!(derive_password_key(&pw("x"), &[0u8; 16], &fast()).is_err());
        assert!(derive_password_key(&pw("x"), &[0u8; 33], &fast()).is_err());
        let bad = KdfParams {
            m_cost_kib: 8,
            ..fast()
        };
        assert!(derive_password_key(&pw("x"), &[0u8; SALT_LEN], &bad).is_err());
    }

    #[test]
    fn salts_are_random() {
        assert_ne!(generate_salt().unwrap(), generate_salt().unwrap());
    }

    #[test]
    fn params_json_shape() {
        let json = serde_json::to_string(&KdfParams::default()).unwrap();
        assert_eq!(json, r#"{"mCostKiB":65536,"tCost":4,"pCost":4}"#);
        assert!(
            serde_json::from_str::<KdfParams>(r#"{"mCostKiB":1,"tCost":1,"pCost":1,"x":1}"#)
                .is_err()
        );
    }
}
