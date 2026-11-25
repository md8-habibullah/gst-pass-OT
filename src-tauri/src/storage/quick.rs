//! The account PIN ("quick unlock"), the middle of three layers (`ai-docs/ARCHITECTURE.md` §5, amended 2026-11-25).
//!
//! 1. **The app lock** (Stage 1, `crate::gate`): hides everything; holds no key.
//! 2. **The account PIN** (this module): shows an account again while its vault is still open in memory.
//! 3. **The master password**: opens a vault from nothing, and is needed for everything sensitive.
//!
//! The PIN **never wraps, caches or derives a key**. It is only a check that lets a vault that is *already unlocked
//! in memory* (parked there by "Lock" or by switching account) be shown again. Nothing on disk can open a vault
//! with it: whoever copies the files and cracks the PIN hash has learned a short word and nothing else. When the
//! idle timer ends, the app is left for too long, the app closes, the person logs out, or the PIN is wrong five times
//! in a row, the key is wiped and only the master password opens the vault again.
//!
//! A vault shown again with the PIN is *limited* (`Session::limited`): everything sensitive (showing or copying a
//! secret, editing, changing a backup target, the app's lock settings) still asks for the master password, and the core
//! enforces that, not the screens.

use super::error::{Result, StorageError};
use crate::crypto::KdfParams;
use crate::gate::credential::{self, Normalised, Phc, VerifyCounter};

/// Fewest characters in a PIN or password (the same floor as the app lock's credential).
pub const MIN_PIN_CHARS: usize = credential::MIN_CREDENTIAL;
pub const MAX_PIN_CHARS: usize = credential::MAX_CREDENTIAL;
/// Wrong PINs in a row before the open vault is closed and only the master password works.
pub const MAX_WRONG_PINS: u8 = 5;

/// Why a PIN cannot be used, in the words the person sees; `None` when it is fine.
pub fn problem(pin: &str) -> Option<String> {
    let n = credential::char_count(pin);
    if n < MIN_PIN_CHARS {
        return Some(format!(
            "Use at least {MIN_PIN_CHARS} characters for the PIN."
        ));
    }
    if n > MAX_PIN_CHARS {
        return Some(format!(
            "Use at most {MAX_PIN_CHARS} characters for the PIN."
        ));
    }
    None
}

/// Hash a new PIN. The cost is the app lock's (never above the vault's own).
pub fn hash(pin: &str, vault_kdf: &KdfParams) -> Result<String> {
    if let Some(p) = problem(pin) {
        return Err(StorageError::invalid(p));
    }
    let text = Normalised::new(pin);
    let phc = credential::hash_new(&text, &credential::gate_params(vault_kdf))?;
    Ok(phc.to_phc_string())
}

/// Is `pin` the one behind `stored`? A stored value that cannot be read never matches.
pub fn matches(pin: &str, stored: &str) -> Result<bool> {
    let Some(phc) = Phc::parse(stored) else {
        return Ok(false);
    };
    let text = Normalised::new(pin);
    credential::matches(&text, &phc, &VerifyCounter::default()).map_err(StorageError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> KdfParams {
        KdfParams {
            m_cost_kib: 19 * 1024,
            t_cost: 2,
            p_cost: 1,
        }
    }

    #[test]
    fn a_pin_is_four_to_sixty_four_characters() {
        assert!(problem("123").is_some());
        assert!(problem("1234").is_none());
        assert!(problem(&"x".repeat(64)).is_none());
        assert!(problem(&"x".repeat(65)).is_some());
        assert_eq!(hash("12", &fast()).unwrap_err().code(), "invalid_input");
    }

    #[test]
    fn only_the_right_pin_matches_and_the_hash_is_not_the_pin() {
        let stored = hash("4821 pin", &fast()).unwrap();
        assert!(stored.starts_with("$argon2id$"));
        assert!(!stored.contains("4821"));
        assert!(matches("4821 pin", &stored).unwrap());
        assert!(!matches("4821 pim", &stored).unwrap());
        assert!(!matches("", &stored).unwrap());
        // two hashes of the same PIN differ (a fresh salt each time) and both match
        let again = hash("4821 pin", &fast()).unwrap();
        assert_ne!(stored, again);
        assert!(matches("4821 pin", &again).unwrap());
        // text that is not a hash never matches
        assert!(!matches("4821 pin", "nonsense").unwrap());
        assert!(!matches("4821 pin", "").unwrap());
    }

    #[test]
    fn the_same_text_typed_another_way_still_matches() {
        // full-width digits normalise to plain ones, as for the app lock and the master password
        let stored = hash("１２３４５", &fast()).unwrap();
        assert!(matches("12345", &stored).unwrap());
    }
}
