//! OS CSPRNG access. Randomness always comes from the operating system (`getrandom`).

use super::error::{CryptoError, Result};
use getrandom::SysRng;
use rand_core::UnwrapErr;

/// Fill `buf` with OS randomness.
pub fn fill(buf: &mut [u8]) -> Result<()> {
    getrandom::fill(buf).map_err(|_| CryptoError::Rng)
}

/// Random array for **non-secret** values (salts, nonces). For secrets use `SecretKey32::generate`.
pub fn array<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    fill(&mut out)?;
    Ok(out)
}

/// Infallible RNG handle for libraries that want a `CryptoRng` (ML-KEM, X25519).
///
/// This panics if the OS RNG fails, which is acceptable: there is no safe fallback, and the
/// failure mode of the OS CSPRNG on Linux/Android after boot is "does not happen".
pub fn os_rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_different_output() {
        let a = array::<32>().unwrap();
        let b = array::<32>().unwrap();
        assert_ne!(a, b);
        assert_ne!(a, [0u8; 32]);
    }
}
