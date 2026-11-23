//! RFC 6238 TOTP engine on top of `totp-rs` 6.x (`ai-docs/CRYPTO_SPEC.md` §11).
//!
//! The secret never leaves Rust: the UI receives [`TotpCode`] (code, seconds remaining, period)
//! and runs its own 1 Hz countdown.

use super::error::{CryptoError, Result};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use totp_rs::{Algorithm, Builder, Secret, Totp};
use zeroize::Zeroizing;

pub const DEFAULT_DIGITS: u8 = 6;
pub const DEFAULT_PERIOD: u64 = 30;
pub const MAX_PERIOD: u64 = 86_400;
const MAX_URI_LEN: usize = 2048;
const MAX_SECRET_BYTES: usize = 1024;
/// Accepted clock skew (in steps) for [`TotpAccount::verify`], per RFC 6238 §5.2.
const VERIFY_SKEW: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TotpAlgorithm {
    #[serde(rename = "SHA1")]
    Sha1,
    #[serde(rename = "SHA256")]
    Sha256,
    #[serde(rename = "SHA512")]
    Sha512,
}

impl From<TotpAlgorithm> for Algorithm {
    fn from(a: TotpAlgorithm) -> Self {
        match a {
            TotpAlgorithm::Sha1 => Algorithm::SHA1,
            TotpAlgorithm::Sha256 => Algorithm::SHA256,
            TotpAlgorithm::Sha512 => Algorithm::SHA512,
        }
    }
}

/// What the UI gets: never the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpCode {
    pub code: String,
    pub seconds_remaining: u64,
    pub period: u64,
    pub digits: u8,
}

/// Parameters for building an account from a pasted Base32 secret.
#[derive(Debug, Clone, Copy)]
pub struct TotpParams<'a> {
    pub secret_base32: &'a str,
    pub algorithm: TotpAlgorithm,
    pub digits: u8,
    pub period: u64,
    pub issuer: Option<&'a str>,
    pub account: &'a str,
}

impl<'a> TotpParams<'a> {
    /// SHA-1, 6 digits, 30 s: what almost every service uses.
    pub fn defaults(secret_base32: &'a str) -> Self {
        Self {
            secret_base32,
            algorithm: TotpAlgorithm::Sha1,
            digits: DEFAULT_DIGITS,
            period: DEFAULT_PERIOD,
            issuer: None,
            account: "",
        }
    }
}

/// A TOTP secret plus its parameters. The secret is wiped on drop (`totp-rs` `zeroize` feature).
pub struct TotpAccount {
    inner: Totp,
}

/// Upper-case, strip whitespace/hyphens/padding, and check the RFC 4648 Base32 alphabet.
fn normalize_base32(input: &str) -> Result<Zeroizing<String>> {
    let mut out = Zeroizing::new(String::with_capacity(input.len()));
    for c in input.chars() {
        match c {
            ' ' | '\t' | '\r' | '\n' | '-' | '=' => {}
            'A'..='Z' | '2'..='7' => out.push(c),
            'a'..='z' => out.push(c.to_ascii_uppercase()),
            _ => return Err(CryptoError::Totp("secret is not valid Base32")),
        }
    }
    if out.is_empty() {
        return Err(CryptoError::Totp("secret is empty"));
    }
    Ok(out)
}

fn unix_now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| CryptoError::Totp("system clock is set before 1970"))
}

impl TotpAccount {
    pub fn new(p: &TotpParams<'_>) -> Result<Self> {
        let normalized = normalize_base32(p.secret_base32)?;
        let secret = Secret::try_from_base32(normalized.as_str())
            .map_err(|_| CryptoError::Totp("secret is not valid Base32"))?;
        if secret.as_bytes().is_empty() || secret.as_bytes().len() > MAX_SECRET_BYTES {
            return Err(CryptoError::Totp("secret length is not supported"));
        }
        // Validate everything the library's non-compliant builder would let through. It accepts
        // short secrets (needed: real services issue 80-bit ones) but would panic on these.
        if !(6..=8).contains(&p.digits) {
            return Err(CryptoError::Totp("digits must be 6, 7 or 8"));
        }
        if p.period == 0 || p.period > MAX_PERIOD {
            return Err(CryptoError::Totp("period must be 1 to 86400 seconds"));
        }
        let inner = Builder::new()
            .with_algorithm(p.algorithm.into())
            .with_digits(p.digits)
            .with_step_duration(p.period)
            .with_skew(VERIFY_SKEW)
            .with_secret(secret)
            .with_account_name(p.account)
            .with_issuer(p.issuer)
            .build_noncompliant();
        Ok(Self { inner })
    }

    /// Parse an `otpauth://totp/...` URI (as found in QR codes). HOTP, Steam and Google
    /// Authenticator migration links are rejected with a specific message.
    pub fn from_uri(uri: &str) -> Result<Self> {
        let uri = uri.trim();
        if uri.len() > MAX_URI_LEN {
            return Err(CryptoError::Totp("URI is too long"));
        }
        let lower = uri.to_ascii_lowercase();
        if lower.starts_with("otpauth-migration://") {
            return Err(CryptoError::Totp(
                "Google Authenticator migration links are not supported yet",
            ));
        }
        if lower.starts_with("otpauth://hotp") {
            return Err(CryptoError::Totp(
                "counter-based (HOTP) codes are not supported",
            ));
        }
        if !lower.starts_with("otpauth://totp/") {
            return Err(CryptoError::Totp("not an otpauth://totp/ URI"));
        }
        let normalized = normalize_uri_secret(uri)?;
        let parsed = Totp::from_url_unchecked(normalized.as_str())
            .map_err(|_| CryptoError::Totp("not a valid otpauth URI"))?;

        // Same guard as `new`: `from_url_unchecked` performs no validation.
        if !(6..=8).contains(&parsed.digits()) {
            return Err(CryptoError::Totp("digits must be 6, 7 or 8"));
        }
        if parsed.step() == 0 || parsed.step() > MAX_PERIOD {
            return Err(CryptoError::Totp("period must be 1 to 86400 seconds"));
        }
        let len = parsed.secret().as_bytes().len();
        if len == 0 || len > MAX_SECRET_BYTES {
            return Err(CryptoError::Totp("secret length is not supported"));
        }
        // Re-apply our skew policy (the URI cannot carry one).
        let inner = Builder::new()
            .with_algorithm(parsed.algorithm())
            .with_digits(parsed.digits())
            .with_step_duration(parsed.step())
            .with_skew(VERIFY_SKEW)
            .with_secret(parsed.secret().as_bytes())
            .with_account_name(parsed.account_name())
            .with_issuer(parsed.issuer())
            .build_noncompliant();
        Ok(Self { inner })
    }

    pub fn digits(&self) -> u8 {
        self.inner.digits()
    }

    pub fn period(&self) -> u64 {
        self.inner.step()
    }

    pub fn issuer(&self) -> Option<&str> {
        self.inner.issuer()
    }

    pub fn account(&self) -> &str {
        self.inner.account_name()
    }

    /// Code for an explicit Unix time (seconds). Deterministic, used by tests and by the UI refresh.
    pub fn code_at(&self, unix_secs: u64) -> TotpCode {
        let period = self.inner.step();
        TotpCode {
            code: self.inner.generate(unix_secs).to_string(),
            seconds_remaining: period - (unix_secs % period),
            period,
            digits: self.inner.digits(),
        }
    }

    pub fn code_now(&self) -> Result<TotpCode> {
        Ok(self.code_at(unix_now()?))
    }

    /// Check a user-entered code at `unix_secs`, allowing ±1 step of clock skew.
    pub fn verify(&self, code: &str, unix_secs: u64) -> bool {
        self.inner.check(code.trim(), unix_secs).is_some()
    }

    /// `otpauth://` URI **including the secret**. Only for export, behind the re-prompt guard.
    pub fn to_uri(&self) -> Result<Zeroizing<String>> {
        if self.inner.account_name().is_empty() {
            return Err(CryptoError::Totp(
                "an account name is required to export a URI",
            ));
        }
        self.inner
            .to_url()
            .map(Zeroizing::new)
            .map_err(|_| CryptoError::Totp("could not build the URI"))
    }
}

impl core::fmt::Debug for TotpAccount {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TotpAccount(<redacted>)")
    }
}

/// Make the `secret=` query value strict Base32 so lenient real-world URIs (lowercase, spaces,
/// padding) parse. Everything else is passed through untouched.
fn normalize_uri_secret(uri: &str) -> Result<Zeroizing<String>> {
    let (no_fragment, _) = uri.split_once('#').unwrap_or((uri, ""));
    let Some((head, query)) = no_fragment.split_once('?') else {
        return Err(CryptoError::Totp("URI has no secret"));
    };
    let mut found = false;
    let mut rebuilt = Zeroizing::new(String::with_capacity(no_fragment.len()));
    rebuilt.push_str(head);
    rebuilt.push('?');
    for (i, pair) in query.split('&').enumerate() {
        if i > 0 {
            rebuilt.push('&');
        }
        match pair.split_once('=') {
            Some(("secret", value)) => {
                found = true;
                let decoded = value
                    .replace("%20", "")
                    .replace("%3D", "")
                    .replace("%3d", "")
                    .replace('+', "");
                rebuilt.push_str("secret=");
                rebuilt.push_str(normalize_base32(&decoded)?.as_str());
            }
            _ => rebuilt.push_str(pair),
        }
    }
    if !found {
        return Err(CryptoError::Totp("URI has no secret"));
    }
    Ok(rebuilt)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S1: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
    const S256: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA====";
    const S512: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNA=";
    const SHORT: &str = "JBSWY3DPEHPK3PXP"; // 80 bits

    fn acct(secret: &str, algorithm: TotpAlgorithm, digits: u8, period: u64) -> TotpAccount {
        TotpAccount::new(&TotpParams {
            secret_base32: secret,
            algorithm,
            digits,
            period,
            issuer: None,
            account: "",
        })
        .unwrap()
    }

    /// RFC 6238 Appendix B (8 digits, 30 s), independently recomputed with a stdlib HOTP.
    #[test]
    fn rfc6238_appendix_b_vectors() {
        let a1 = acct(S1, TotpAlgorithm::Sha1, 8, 30);
        let a256 = acct(S256, TotpAlgorithm::Sha256, 8, 30);
        let a512 = acct(S512, TotpAlgorithm::Sha512, 8, 30);
        let table: [(u64, &str, &str, &str); 6] = [
            (59, "94287082", "46119246", "90693936"),
            (1_111_111_109, "07081804", "68084774", "25091201"),
            (1_111_111_111, "14050471", "67062674", "99943326"),
            (1_234_567_890, "89005924", "91819424", "93441116"),
            (2_000_000_000, "69279037", "90698825", "38618901"),
            (20_000_000_000, "65353130", "77737706", "47863826"),
        ];
        for (t, c1, c256, c512) in table {
            assert_eq!(a1.code_at(t).code, c1, "SHA1 @{t}");
            assert_eq!(a256.code_at(t).code, c256, "SHA256 @{t}");
            assert_eq!(a512.code_at(t).code, c512, "SHA512 @{t}");
        }
    }

    #[test]
    fn short_secrets_are_accepted_and_match_reference() {
        // 80-bit secret, 6 digits SHA-1, and SHA-256/8 digits/60 s: values from a stdlib HOTP.
        let a = acct(SHORT, TotpAlgorithm::Sha1, 6, 30);
        assert_eq!(a.code_at(59).code, "996554");
        assert_eq!(a.code_at(1_700_000_000).code, "324550");
        let b = acct(SHORT, TotpAlgorithm::Sha256, 8, 60);
        assert_eq!(b.code_at(1_700_000_000).code, "71205722");
    }

    #[test]
    fn countdown_fields() {
        let a = acct(SHORT, TotpAlgorithm::Sha1, 6, 30);
        let at = |t| a.code_at(t);
        assert_eq!(
            (at(0).seconds_remaining, at(0).period, at(0).digits),
            (30, 30, 6)
        );
        assert_eq!(at(29).seconds_remaining, 1);
        assert_eq!(at(30).seconds_remaining, 30);
        assert_eq!(at(59).seconds_remaining, 1);
        // The code is stable inside a window and changes at the boundary.
        assert_eq!(at(30).code, at(59).code);
        assert_ne!(at(59).code, at(60).code);
    }

    #[test]
    fn json_shape_for_the_ui() {
        let json =
            serde_json::to_string(&acct(SHORT, TotpAlgorithm::Sha1, 6, 30).code_at(59)).unwrap();
        assert_eq!(
            json,
            r#"{"code":"996554","secondsRemaining":1,"period":30,"digits":6}"#
        );
    }

    #[test]
    fn code_now_uses_the_system_clock() {
        let c = acct(SHORT, TotpAlgorithm::Sha1, 6, 30).code_now().unwrap();
        assert_eq!(c.code.len(), 6);
        assert!((1..=30).contains(&c.seconds_remaining));
    }

    #[test]
    fn verify_allows_one_step_of_skew_only() {
        let a = acct(SHORT, TotpAlgorithm::Sha1, 6, 30);
        let t = 1_700_000_000;
        let code = a.code_at(t).code;
        assert!(a.verify(&code, t));
        assert!(a.verify(&code, t + 30));
        assert!(a.verify(&code, t - 30));
        assert!(!a.verify(&code, t + 90));
        assert!(!a.verify("000000", t) || code == "000000");
        assert!(!a.verify("12345", t));
        assert!(!a.verify("", t));
        assert!(a.verify(&format!("  {code}  "), t));
    }

    #[test]
    fn base32_input_is_normalized() {
        let reference = acct(SHORT, TotpAlgorithm::Sha1, 6, 30).code_at(59).code;
        for variant in [
            "jbswy3dpehpk3pxp",
            "JBSW Y3DP EHPK 3PXP",
            "jbsw-y3dp-ehpk-3pxp",
            "JBSWY3DPEHPK3PXP====",
            " JBSWY3DPEHPK3PXP\n",
        ] {
            assert_eq!(
                acct(variant, TotpAlgorithm::Sha1, 6, 30).code_at(59).code,
                reference,
                "{variant:?}"
            );
        }
    }

    #[test]
    fn invalid_parameters_are_rejected_without_panicking() {
        let ok = |s: &str, d: u8, p: u64| {
            TotpAccount::new(&TotpParams {
                secret_base32: s,
                algorithm: TotpAlgorithm::Sha1,
                digits: d,
                period: p,
                issuer: None,
                account: "",
            })
        };
        assert!(ok(SHORT, 6, 30).is_ok());
        assert!(ok(SHORT, 5, 30).is_err());
        assert!(ok(SHORT, 9, 30).is_err());
        assert!(ok(SHORT, 10, 30).is_err());
        assert!(ok(SHORT, 6, 0).is_err());
        assert!(ok(SHORT, 6, MAX_PERIOD + 1).is_err());
        assert!(ok("", 6, 30).is_err());
        assert!(ok("   ", 6, 30).is_err());
        assert!(ok("not base32!", 6, 30).is_err());
        assert!(ok("0189", 6, 30).is_err()); // 0, 1, 8, 9 are not in the alphabet
        assert!(ok(&"A".repeat(2000), 6, 30).is_err());
    }

    #[test]
    fn parses_a_standard_uri() {
        let a = TotpAccount::from_uri(
            "otpauth://totp/Example:alice@example.com?secret=JBSWY3DPEHPK3PXP&issuer=Example",
        )
        .unwrap();
        assert_eq!(a.issuer(), Some("Example"));
        assert_eq!(a.account(), "alice@example.com");
        assert_eq!((a.digits(), a.period()), (6, 30));
        assert_eq!(a.code_at(59).code, "996554");
    }

    #[test]
    fn parses_all_parameters() {
        let a = TotpAccount::from_uri("otpauth://totp/Corp%20VPN:bob%40corp.example?secret=JBSWY3DPEHPK3PXP&issuer=Corp%20VPN&algorithm=SHA256&digits=8&period=60").unwrap();
        assert_eq!(a.issuer(), Some("Corp VPN"));
        assert_eq!(a.account(), "bob@corp.example");
        assert_eq!((a.digits(), a.period()), (8, 60));
        assert_eq!(a.code_at(1_700_000_000).code, "71205722");
    }

    #[test]
    fn lenient_secrets_in_uris() {
        for secret in [
            "jbswy3dpehpk3pxp",
            "JBSW%20Y3DP%20EHPK%203PXP",
            "JBSWY3DPEHPK3PXP%3D%3D",
            "jbsw+y3dp+ehpk+3pxp",
        ] {
            let uri = format!("otpauth://totp/x?secret={secret}");
            let a = TotpAccount::from_uri(&uri).unwrap_or_else(|e| panic!("{uri}: {e}"));
            assert_eq!(a.code_at(59).code, "996554", "{uri}");
        }
    }

    #[test]
    fn rejects_bad_uris() {
        let bad = [
            "",
            "https://example.com",
            "otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP&counter=1",
            "otpauth-migration://offline?data=abc",
            "otpauth://totp/x",
            "otpauth://totp/x?issuer=a",
            "otpauth://totp/x?secret=",
            "otpauth://totp/x?secret=!!!!",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=9",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=5",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&period=0",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&period=99999",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&algorithm=MD5",
            "otpauth://totp/a:b?secret=JBSWY3DPEHPK3PXP&issuer=different",
            "otpauth://steam/x?secret=JBSWY3DPEHPK3PXP",
        ];
        for uri in bad {
            assert!(TotpAccount::from_uri(uri).is_err(), "should reject {uri:?}");
        }
        assert!(TotpAccount::from_uri(&format!(
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&pad={}",
            "a".repeat(3000)
        ))
        .is_err());
    }

    #[test]
    fn uri_round_trip() {
        let a = TotpAccount::new(&TotpParams {
            secret_base32: SHORT,
            algorithm: TotpAlgorithm::Sha512,
            digits: 8,
            period: 45,
            issuer: Some("My Service"),
            account: "carol@example.com",
        })
        .unwrap();
        let uri = a.to_uri().unwrap();
        let b = TotpAccount::from_uri(&uri).unwrap();
        assert_eq!(
            (b.issuer(), b.account(), b.digits(), b.period()),
            (Some("My Service"), "carol@example.com", 8, 45)
        );
        for t in [0, 59, 1_700_000_000] {
            assert_eq!(a.code_at(t), b.code_at(t));
        }
        // Export needs an account name.
        assert!(acct(SHORT, TotpAlgorithm::Sha1, 6, 30).to_uri().is_err());
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let a = acct(SHORT, TotpAlgorithm::Sha1, 6, 30);
        assert_eq!(format!("{a:?}"), "TotpAccount(<redacted>)");
        assert!(!format!("{:?}", CryptoError::Totp("x")).contains("JBSW"));
    }
}
