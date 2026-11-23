//! UUIDv4 identifiers from the OS CSPRNG (no `uuid` crate needed for a single use).
//!
//! Item ids are lowercase, hyphenated UUIDv4 strings, which also satisfy
//! `crypto::ids::validate_id` (36 characters of `[0-9a-f-]`).

use crate::crypto::{rng, CryptoError};

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Fresh random UUIDv4 (`xxxxxxxx-xxxx-4xxx-[89ab]xxx-xxxxxxxxxxxx`).
pub fn new_v4() -> Result<String, CryptoError> {
    let mut b = rng::array::<16>()?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let mut out = String::with_capacity(36);
    for (i, byte) in b.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(out)
}

/// Strict check for the canonical form produced by [`new_v4`].
pub fn is_valid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for (i, &c) in b.iter().enumerate() {
        let ok = match i {
            8 | 13 | 18 | 23 => c == b'-',
            14 => c == b'4',
            19 => matches!(c, b'8' | b'9' | b'a' | b'b'),
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(&c),
        };
        if !ok {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn has_the_v4_shape() {
        for _ in 0..200 {
            let id = new_v4().unwrap();
            assert!(is_valid(&id), "{id}");
            assert_eq!(id.len(), 36);
            assert_eq!(&id[14..15], "4");
            assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
            crate::crypto::ids::validate_id(&id).unwrap();
        }
    }

    #[test]
    fn ids_do_not_repeat() {
        let set: HashSet<String> = (0..2000).map(|_| new_v4().unwrap()).collect();
        assert_eq!(set.len(), 2000);
    }

    #[test]
    fn rejects_everything_that_is_not_canonical() {
        let good = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";
        assert!(is_valid(good));
        assert!(!is_valid(&good.to_uppercase()));
        assert!(!is_valid(&good.replace('-', "")));
        assert!(!is_valid("3f2b8c1e-5d4a-1e7b-9c1d-0a1b2c3d4e5f")); // version 1
        assert!(!is_valid("3f2b8c1e-5d4a-4e7b-cc1d-0a1b2c3d4e5f")); // bad variant
        assert!(!is_valid("3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5g"));
        assert!(!is_valid(""));
        assert!(!is_valid("_keyring"));
        assert!(!is_valid(&format!("{good}x")));
    }
}
