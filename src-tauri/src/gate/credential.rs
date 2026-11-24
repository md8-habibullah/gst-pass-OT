//! The Stage 1 credential: rules, Argon2id hashing, and the equal-cost check.
//!
//! * Only **Argon2id PHC strings** are ever stored (`$argon2id$v=19$m=..,t=..,p=..$salt$hash`,
//!   16-byte random salt from the OS CSPRNG, 32-byte output). No length, no hint, nothing else is
//!   derived from the plaintext.
//! * Text is **NFKC-normalised** before it is hashed or measured, in every mode, so the same
//!   passcode works with any keyboard (`CRYPTO_SPEC.md` §3 does the same for the master password).
//! * Every security attempt runs **all three** verifications (the real credential, its reverse,
//!   the custom panic code). A missing hash is verified against a fixed dummy of the same cost, so
//!   a right guess, a panic guess and a wrong guess cost the same time and the same work.
//! * The rules and their messages mirror `src/lib/gateRules.ts`; Rust stays the authority.

use super::GateMode;
use crate::crypto::{rng, CryptoError, KdfParams};
use crate::storage::StorageError;
use argon2::{Algorithm, Argon2, Block, Params, Version};
use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use base64::Engine;
use std::sync::atomic::{AtomicU64, Ordering};
use subtle::{Choice, ConstantTimeEq};
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, Zeroizing};

pub const MIN_CREDENTIAL: usize = 4;
pub const MAX_CREDENTIAL: usize = 64;
/// Typed text longer than this is cut before hashing (no credential can be this long, so the
/// result can never match, and the cost of one attempt stays bounded).
pub const MAX_INPUT_CHARS: usize = 256;

pub const SALT_LEN: usize = 16;
pub const HASH_LEN: usize = 32;
/// Cost ceiling for a hash read from disk: a tampered file must not force a huge allocation.
pub const MAX_STORED_M_COST_KIB: u32 = 262_144;
const MAX_PHC_LEN: usize = 256;

/// Production cost of one gate hash: 32 MiB and 3 passes on one lane. One attempt runs three of
/// them one after the other, so the peak memory is 32 MiB (fine for a phone with 4 GiB) and an
/// attempt takes a fraction of a second on a desktop. A gate credential protects a *privacy*
/// screen, not the keys, so it is deliberately cheaper than the vault's own KDF.
pub const GATE_M_COST_KIB: u32 = 32_768;
pub const GATE_T_COST: u32 = 3;

/// The gate's Argon2id cost, taken from the vault's KDF parameters but never above the gate's
/// own production cost. The shim's `--fast-kdf` (19 MiB, 2 passes) therefore makes the gate fast
/// too, while the production default (64 MiB, 4 passes, 4 lanes) becomes 32 MiB, 3 passes, 1 lane.
pub fn gate_params(vault: &KdfParams) -> KdfParams {
    KdfParams {
        m_cost_kib: vault.m_cost_kib.min(GATE_M_COST_KIB),
        t_cost: vault.t_cost.min(GATE_T_COST),
        p_cost: 1,
    }
}

// ------------------------------------------------------------------ text

/// NFKC-normalised text that is wiped on drop. Longer than [`MAX_INPUT_CHARS`] is cut.
pub struct Normalised(Zeroizing<String>);

impl Normalised {
    pub fn new(raw: &str) -> Self {
        // Exact size first, so the buffer never reallocates (a reallocation would leave an
        // unwiped copy of the text behind).
        let bytes: usize = raw.nfkc().take(MAX_INPUT_CHARS).map(char::len_utf8).sum();
        let mut s = Zeroizing::new(String::with_capacity(bytes));
        for c in raw.nfkc().take(MAX_INPUT_CHARS) {
            s.push(c);
        }
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn chars(&self) -> usize {
        self.0.chars().count()
    }

    /// The characters in reverse order, normalised again.
    pub fn reversed(&self) -> Self {
        let rev: Zeroizing<String> = Zeroizing::new(self.0.chars().rev().collect());
        Self::new(&rev)
    }
}

/// How many characters `raw` has after normalisation (what the gate measures, uncut).
pub fn char_count(raw: &str) -> usize {
    raw.nfkc().count()
}

// ------------------------------------------------------------------ rules (gateRules.ts)

fn noun(mode: GateMode) -> &'static str {
    match mode {
        GateMode::Pin => "digits",
        GateMode::Password => "characters",
        GateMode::Grid => "dots",
    }
}

/// The problem with a credential (or panic code) for a mode, or `None`. Empty is `None` here, like
/// the front end; callers decide whether empty is allowed.
pub fn credential_problem(mode: GateMode, value: &str) -> Option<String> {
    let text = Normalised::new(value);
    let n = char_count(value);
    if n == 0 {
        return None;
    }
    let s = text.as_str();
    if mode == GateMode::Pin && !s.chars().all(|c| c.is_ascii_digit()) {
        return Some("A PIN uses digits only.".into());
    }
    if mode == GateMode::Grid {
        if !s.chars().all(|c| ('1'..='9').contains(&c)) {
            return Some("A pattern uses the dots 1 to 9.".into());
        }
        let mut seen = [false; 10];
        for c in s.chars() {
            let i = c as usize - '0' as usize;
            if seen[i] {
                return Some("A pattern cannot pass through the same dot twice.".into());
            }
            seen[i] = true;
        }
    }
    if n < MIN_CREDENTIAL {
        return Some(format!("Use at least {MIN_CREDENTIAL} {}.", noun(mode)));
    }
    if n > MAX_CREDENTIAL {
        return Some(format!("Use at most {MAX_CREDENTIAL} {}.", noun(mode)));
    }
    None
}

/// Problems with the reverse-panic and custom panic settings for a given credential.
pub fn panic_problems(
    mode: GateMode,
    credential: &str,
    reverse_panic: bool,
    panic_code: &str,
) -> (Option<String>, Option<String>) {
    let cred = Normalised::new(credential);
    let rev = cred.reversed();
    let palindrome = cred.chars() > 0 && cred.as_str() == rev.as_str();
    let reverse = (reverse_panic && palindrome).then(|| {
        "This reads the same backwards, so the reversed panic code would be the same as your \
         passcode. Choose one that does not."
            .to_string()
    });
    let mut panic = None;
    if char_count(panic_code) > 0 {
        let code = Normalised::new(panic_code);
        if let Some(own) = credential_problem(mode, panic_code) {
            panic = Some(
                own.replace("Use at least", "The panic code needs at least")
                    .replace("Use at most", "The panic code can have at most"),
            );
        } else if cred.chars() > 0 && code.as_str() == cred.as_str() {
            panic = Some("The panic code must be different from your passcode.".into());
        } else if cred.chars() > 0 && reverse_panic && code.as_str() == rev.as_str() {
            panic = Some("That is your passcode reversed, which already triggers panic.".into());
        }
    }
    (reverse, panic)
}

// ------------------------------------------------------------------ PHC strings

/// A parsed Argon2id hash: its own cost, its salt and the 32-byte result.
#[derive(Clone)]
pub struct Phc {
    pub params: KdfParams,
    pub salt: [u8; SALT_LEN],
    hash: Zeroizing<[u8; HASH_LEN]>,
}

impl Phc {
    pub fn to_phc_string(&self) -> String {
        format!(
            "$argon2id$v=19$m={},t={},p={}${}${}",
            self.params.m_cost_kib,
            self.params.t_cost,
            self.params.p_cost,
            B64.encode(self.salt),
            B64.encode(&self.hash[..]),
        )
    }

    /// Strict parse: exactly the form [`to_phc_string`](Self::to_phc_string) writes, with the
    /// cost inside the allowed range. Anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() > MAX_PHC_LEN {
            return None;
        }
        let mut parts = text.split('$');
        if !parts.next()?.is_empty() || parts.next()? != "argon2id" || parts.next()? != "v=19" {
            return None;
        }
        let mut cost = parts.next()?.split(',');
        let m = cost.next()?.strip_prefix("m=")?.parse::<u32>().ok()?;
        let t = cost.next()?.strip_prefix("t=")?.parse::<u32>().ok()?;
        let p = cost.next()?.strip_prefix("p=")?.parse::<u32>().ok()?;
        if cost.next().is_some() {
            return None;
        }
        let salt_text = parts.next()?;
        let hash_text = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        let params = KdfParams {
            m_cost_kib: m,
            t_cost: t,
            p_cost: p,
        };
        if params.validate().is_err() || m > MAX_STORED_M_COST_KIB {
            return None;
        }
        let salt: [u8; SALT_LEN] = B64.decode(salt_text).ok()?.try_into().ok()?;
        let mut raw = B64.decode(hash_text).ok()?;
        let hash: Option<[u8; HASH_LEN]> = raw.as_slice().try_into().ok();
        raw.zeroize();
        Some(Self {
            params,
            salt,
            hash: Zeroizing::new(hash?),
        })
    }
}

impl core::fmt::Debug for Phc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Phc(<redacted>)")
    }
}

/// `Argon2id(v0x13, text, salt, params) -> 32 bytes`, with the work memory wiped afterwards.
fn derive(
    text: &[u8],
    salt: &[u8; SALT_LEN],
    params: &KdfParams,
) -> Result<Zeroizing<[u8; HASH_LEN]>, CryptoError> {
    params.validate()?;
    let argon_params = Params::new(
        params.m_cost_kib,
        params.t_cost,
        params.p_cost,
        Some(HASH_LEN),
    )
    .map_err(|_| CryptoError::InvalidKdfParams("rejected by Argon2"))?;
    let blocks_len = argon_params.block_count();
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);
    let mut blocks: Zeroizing<Vec<Block>> = Zeroizing::new(Vec::new());
    blocks
        .try_reserve_exact(blocks_len)
        .map_err(|_| CryptoError::KdfOutOfMemory)?;
    blocks.resize(blocks_len, Block::default());
    let mut out = Zeroizing::new([0u8; HASH_LEN]);
    argon
        .hash_password_into_with_memory(text, salt, &mut out[..], blocks.as_mut_slice())
        .map_err(|_| CryptoError::Kdf)?;
    Ok(out)
}

/// Hash `text` with a fresh random salt.
pub fn hash_new(text: &Normalised, params: &KdfParams) -> Result<Phc, StorageError> {
    let salt = rng::array::<SALT_LEN>().map_err(StorageError::from)?;
    let hash = derive(text.as_bytes(), &salt, params).map_err(StorageError::from)?;
    Ok(Phc {
        params: *params,
        salt,
        hash,
    })
}

// ------------------------------------------------------------------ equal-cost verification

/// A fixed salt for the dummy verification of a hash that does not exist.
const DUMMY_SALT: [u8; SALT_LEN] = *b"hexis-gate-dummy";

/// The outcome of one security attempt's three verifications, still unresolved.
pub struct Verdict {
    pub real: Choice,
    pub reversed: Choice,
    pub custom: Choice,
}

/// Counts every Argon2id run the gate makes. The equal-cost rule is tested through it: a
/// security attempt adds exactly three, a maths attempt none.
#[derive(Default)]
pub struct VerifyCounter(AtomicU64);

impl VerifyCounter {
    pub fn count(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn verify_one(
    text: &Normalised,
    stored: Option<&Phc>,
    fallback: &KdfParams,
    counter: &VerifyCounter,
) -> Result<Choice, CryptoError> {
    counter.0.fetch_add(1, Ordering::SeqCst);
    match stored {
        Some(phc) => {
            let got = derive(text.as_bytes(), &phc.salt, &phc.params)?;
            Ok(got[..].ct_eq(&phc.hash[..]))
        }
        None => {
            // Same work with the same cost, against a hash that can never match.
            let got = derive(text.as_bytes(), &DUMMY_SALT, fallback)?;
            let never = [0xA5u8; HASH_LEN];
            let _ = got[..].ct_eq(&never[..]);
            Ok(Choice::from(0))
        }
    }
}

/// One Argon2id check of `text` against a stored hash (counted like every other run).
pub fn matches(
    text: &Normalised,
    stored: &Phc,
    counter: &VerifyCounter,
) -> Result<bool, CryptoError> {
    let choice = verify_one(text, Some(stored), &stored.params, counter)?;
    Ok(bool::from(choice))
}

/// Always runs three Argon2id verifications, in this order: the real credential, its reverse, the
/// custom panic code. A missing hash costs the same as a present one.
pub fn verify_all(
    text: &Normalised,
    real: &Phc,
    reversed: Option<&Phc>,
    custom: Option<&Phc>,
    counter: &VerifyCounter,
) -> Result<Verdict, CryptoError> {
    let fallback = real.params;
    let real_ok = verify_one(text, Some(real), &fallback, counter)?;
    let reversed_ok = verify_one(text, reversed, &fallback, counter)?;
    let custom_ok = verify_one(text, custom, &fallback, counter)?;
    Ok(Verdict {
        real: real_ok,
        reversed: reversed_ok,
        custom: custom_ok,
    })
}

/// What a verdict means. The decision is made with constant-time logic and only after all three
/// verifications ran: the real credential wins over a panic match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Open,
    Panic,
    Wrong,
}

impl Verdict {
    pub fn decide(&self) -> Decision {
        let open = self.real;
        let panic = !self.real & (self.reversed | self.custom);
        // Both flags are computed before either is looked at.
        let open_flag = bool::from(open);
        let panic_flag = bool::from(panic);
        if open_flag {
            Decision::Open
        } else if panic_flag {
            Decision::Panic
        } else {
            Decision::Wrong
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> KdfParams {
        KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        }
    }

    fn hash(s: &str) -> Phc {
        hash_new(&Normalised::new(s), &fast()).unwrap()
    }

    #[test]
    fn production_cost_is_capped_and_fast_kdf_stays_fast() {
        let prod = gate_params(&KdfParams::default());
        assert_eq!(
            (prod.m_cost_kib, prod.t_cost, prod.p_cost),
            (32_768, 3, 1),
            "the vault default (64 MiB, 4, 4) becomes the gate default"
        );
        assert!(prod.validate().is_ok());
        let quick = gate_params(&fast());
        assert_eq!(quick, fast(), "--fast-kdf makes the gate fast too");
        let heavy = KdfParams {
            m_cost_kib: 1_048_576,
            t_cost: 64,
            p_cost: 16,
        };
        assert_eq!(gate_params(&heavy).m_cost_kib, 32_768);
    }

    #[test]
    fn phc_strings_round_trip_and_are_standard_shaped() {
        let h = hash("2468");
        let s = h.to_phc_string();
        assert!(s.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{s}");
        assert_eq!(s.split('$').count(), 6);
        let back = Phc::parse(&s).unwrap();
        assert_eq!(back.to_phc_string(), s);
        assert_eq!(back.params, fast());
        assert!(!s.contains("2468"));
        assert_eq!(format!("{h:?}"), "Phc(<redacted>)");
    }

    #[test]
    fn phc_parsing_rejects_everything_unusual() {
        let good = hash("2468").to_phc_string();
        assert!(Phc::parse(&good).is_some());
        let parts: Vec<&str> = good.split('$').collect();
        let (salt, hash_text) = (parts[4], parts[5]);
        let with = |cost: &str| format!("$argon2id$v=19${cost}${salt}${hash_text}");
        let bad = [
            String::new(),
            "plain".to_string(),
            good.replace("argon2id", "argon2i"),
            good.replace("v=19", "v=16"),
            // cost below the floor, above the gate ceiling, with extras, in the wrong order or
            // not numbers at all
            with("m=8,t=2,p=1"),
            with("m=524288,t=2,p=1"),
            with("m=19456,t=1,p=1"),
            with("m=19456,t=2,p=0"),
            with("t=2,m=19456,p=1"),
            with("m=19456,t=2,p=1,x=1"),
            with("m=19456,t=2"),
            with("m=x,t=2,p=1"),
            // wrong salt or hash length, not base64, extra segments
            format!("$argon2id$v=19$m=19456,t=2,p=1$AAAA${hash_text}"),
            format!("$argon2id$v=19$m=19456,t=2,p=1${salt}$AAAA"),
            format!("$argon2id$v=19$m=19456,t=2,p=1${salt}$!!!!"),
            format!("{good}$extra"),
            "x".repeat(1000),
        ];
        for b in bad {
            assert!(Phc::parse(&b).is_none(), "{b}");
        }
    }

    #[test]
    fn text_is_normalised_measured_and_reversed_as_characters() {
        // composed and decomposed forms are the same passcode; full-width digits become ASCII
        assert_eq!(Normalised::new("caf\u{e9}").as_str(), "caf\u{e9}");
        assert_eq!(Normalised::new("cafe\u{301}").as_str(), "caf\u{e9}");
        assert_eq!(
            Normalised::new("\u{ff11}\u{ff12}\u{ff13}\u{ff14}").as_str(),
            "1234"
        );
        assert_eq!(Normalised::new("12\u{fb01}").as_str(), "12fi");
        assert_eq!(Normalised::new("12345").reversed().as_str(), "54321");
        assert_eq!(
            Normalised::new("p\u{e9}ra").reversed().as_str(),
            "ar\u{e9}p"
        );
        assert_eq!(char_count("\u{1F600}\u{1F600}"), 2, "characters, not bytes");
        assert_eq!(Normalised::new(&"a".repeat(1000)).chars(), MAX_INPUT_CHARS);
    }

    #[test]
    fn the_rules_and_their_messages_match_the_front_end() {
        use GateMode::*;
        let p = credential_problem;
        assert_eq!(p(Pin, ""), None);
        assert_eq!(p(Pin, "12a4").as_deref(), Some("A PIN uses digits only."));
        assert_eq!(p(Pin, "123").as_deref(), Some("Use at least 4 digits."));
        assert_eq!(p(Pin, "1234"), None);
        assert_eq!(
            p(Pin, &"1".repeat(65)).as_deref(),
            Some("Use at most 64 digits.")
        );
        assert_eq!(p(Pin, &"1".repeat(64)), None);
        assert_eq!(
            p(Password, "abc").as_deref(),
            Some("Use at least 4 characters.")
        );
        assert_eq!(p(Password, "abcd"), None);
        assert_eq!(
            p(Grid, "1230").as_deref(),
            Some("A pattern uses the dots 1 to 9.")
        );
        assert_eq!(
            p(Grid, "1231").as_deref(),
            Some("A pattern cannot pass through the same dot twice.")
        );
        assert_eq!(p(Grid, "123").as_deref(), Some("Use at least 4 dots."));
        assert_eq!(p(Grid, "12589"), None);
        assert_eq!(p(Grid, "987654321"), None);
    }

    #[test]
    fn panic_rules_match_the_front_end() {
        use GateMode::*;
        let (rev, pan) = panic_problems(Pin, "1221", true, "");
        assert!(rev.unwrap().starts_with("This reads the same backwards"));
        assert!(pan.is_none());
        assert_eq!(panic_problems(Pin, "1221", false, ""), (None, None));
        assert_eq!(panic_problems(Pin, "1234", true, ""), (None, None));
        let pan = |cred: &str, rp: bool, code: &str| panic_problems(Pin, cred, rp, code).1;
        assert_eq!(
            pan("1234", true, "99").as_deref(),
            Some("The panic code needs at least 4 digits.")
        );
        assert_eq!(
            pan("1234", true, "1234").as_deref(),
            Some("The panic code must be different from your passcode.")
        );
        assert_eq!(
            pan("1234", true, "4321").as_deref(),
            Some("That is your passcode reversed, which already triggers panic.")
        );
        assert_eq!(
            pan("1234", false, "4321"),
            None,
            "allowed when reverse panic is off"
        );
        assert_eq!(pan("1234", true, "5555"), None);
        assert_eq!(
            panic_problems(Pin, "1234", true, "ab12").1.as_deref(),
            Some("A PIN uses digits only."),
            "the panic code follows the mode's own rules"
        );
        assert_eq!(
            panic_problems(Password, "abcd", true, &"x".repeat(65))
                .1
                .as_deref(),
            Some("The panic code can have at most 64 characters.")
        );
    }

    #[test]
    fn verify_all_always_runs_three_hashes_and_decides_afterwards() {
        let real = hash("2468");
        let rev = hash("8642");
        let custom = hash("1111");
        let counter = VerifyCounter::default();
        let t = |s: &str| Normalised::new(s);
        let run = |s: &str, r: Option<&Phc>, c: Option<&Phc>| {
            let before = counter.count();
            let v = verify_all(&t(s), &real, r, c, &counter).unwrap();
            assert_eq!(counter.count() - before, 3, "three verifications for {s:?}");
            v.decide()
        };
        assert_eq!(run("2468", Some(&rev), Some(&custom)), Decision::Open);
        assert_eq!(run("8642", Some(&rev), Some(&custom)), Decision::Panic);
        assert_eq!(run("1111", Some(&rev), Some(&custom)), Decision::Panic);
        assert_eq!(run("0000", Some(&rev), Some(&custom)), Decision::Wrong);
        // missing hashes still cost three, and can never match
        assert_eq!(run("2468", None, None), Decision::Open);
        assert_eq!(run("8642", None, None), Decision::Wrong);
        assert_eq!(run("1111", None, None), Decision::Wrong);
        assert_eq!(run("0000", None, Some(&custom)), Decision::Wrong);
    }

    #[test]
    fn the_real_credential_wins_when_it_also_matches_a_panic_hash() {
        // not reachable through configure (it forbids it), but the decision must still be defined
        let real = hash("2468");
        let same = hash("2468");
        let counter = VerifyCounter::default();
        let v = verify_all(
            &Normalised::new("2468"),
            &real,
            Some(&same),
            Some(&same),
            &counter,
        )
        .unwrap();
        assert_eq!(v.decide(), Decision::Open);
    }

    #[test]
    fn a_hash_made_with_other_cost_still_verifies_with_its_own_cost() {
        let other = KdfParams {
            m_cost_kib: 19_456,
            t_cost: 3,
            p_cost: 1,
        };
        let real = hash_new(&Normalised::new("2468"), &other).unwrap();
        let reparsed = Phc::parse(&real.to_phc_string()).unwrap();
        let counter = VerifyCounter::default();
        let v = verify_all(&Normalised::new("2468"), &reparsed, None, None, &counter).unwrap();
        assert_eq!(v.decide(), Decision::Open);
    }

    #[test]
    fn salts_are_random_so_equal_credentials_hash_differently() {
        assert_ne!(hash("2468").to_phc_string(), hash("2468").to_phc_string());
    }
}
