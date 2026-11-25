//! What a scanned QR code, or a pasted text, is. Pure text work: no vault, no network, nothing is stored.
//!
//! The camera button and the "paste or choose an image" path both end here, so one scanner can take a one-time code,
//! a passkey sign-in code from a computer, or a link, and say which it was.

use crate::storage::model::totp_account;

/// A scanned text is at most this long (a QR code holds under 3 KB).
pub const MAX_SCAN_CHARS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scanned {
    /// A one-time-code setup (`otpauth://totp/…`, or a bare key when the person typed or pasted it).
    Totp(TotpFacts),
    /// Several accounts exported from an authenticator app (`otpauth-migration://`). Not importable yet.
    Migration,
    /// The code a computer shows for "sign in with a passkey on your phone" (`FIDO:/…`).
    Fido,
    /// A web link. Nothing to save from it.
    Link { host: String },
    /// Any other text.
    Text,
    /// It looks like a one-time-code link but cannot be used. The message is for the person.
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotpFacts {
    pub issuer: Option<String>,
    /// The account the code is for, usually an email address or a user name. Empty for a bare key.
    pub account: String,
    pub digits: u8,
    pub period: u64,
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// A bare key is Base32 letters and digits (2-7) in groups, at least 16 of them (80 bits).
fn looks_like_bare_key(text: &str) -> bool {
    let chars: Vec<char> = text.chars().filter(|c| *c != ' ' && *c != '-').collect();
    chars.len() >= 16
        && chars
            .iter()
            .all(|c| c.is_ascii_alphabetic() || ('2'..='7').contains(c))
}

/// Say what `text` is. `allow_bare_key` is for typed or pasted text: a camera never reads a bare key.
pub fn classify(text: &str, allow_bare_key: bool) -> Scanned {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > MAX_SCAN_CHARS {
        return Scanned::Text;
    }
    if starts_with_ignore_case(t, "otpauth-migration://") {
        return Scanned::Migration;
    }
    if starts_with_ignore_case(t, "otpauth://") {
        return match totp_account(t) {
            Ok(a) => Scanned::Totp(TotpFacts {
                issuer: a.issuer().map(str::to_string).filter(|s| !s.is_empty()),
                account: a.account().to_string(),
                digits: a.digits(),
                period: a.period(),
            }),
            Err(_) => Scanned::Invalid(
                "This is a one-time code link, but it has no valid secret or is not a time-based code.".into(),
            ),
        };
    }
    if starts_with_ignore_case(t, "fido:/") {
        return Scanned::Fido;
    }
    if starts_with_ignore_case(t, "https://") || starts_with_ignore_case(t, "http://") {
        let host = crate::storage::model::host_of(t).unwrap_or_default();
        return Scanned::Link { host };
    }
    if allow_bare_key && looks_like_bare_key(t) {
        if let Ok(a) = totp_account(t) {
            return Scanned::Totp(TotpFacts {
                issuer: None,
                account: String::new(),
                digits: a.digits(),
                period: a.period(),
            });
        }
    }
    Scanned::Text
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "JBSWY3DPEHPK3PXP";

    #[test]
    fn a_code_link_gives_its_issuer_and_account() {
        let uri = format!("otpauth://totp/GitHub:ana%40example.com?secret={KEY}&issuer=GitHub");
        match classify(&uri, false) {
            Scanned::Totp(f) => {
                assert_eq!(f.issuer.as_deref(), Some("GitHub"));
                assert_eq!(f.account, "ana@example.com");
                assert_eq!((f.digits, f.period), (6, 30));
            }
            other => panic!("{other:?}"),
        }
        // no issuer, an account only
        match classify(&format!("otpauth://totp/ana?secret={KEY}"), false) {
            Scanned::Totp(f) => assert_eq!((f.issuer, f.account.as_str()), (None, "ana")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_kind_is_found_from_the_start_of_the_text() {
        assert_eq!(
            classify("OTPAUTH-MIGRATION://offline?data=abc", false),
            Scanned::Migration
        );
        assert_eq!(classify("FIDO:/12345678901234567890", false), Scanned::Fido);
        assert_eq!(classify("fido:/1", false), Scanned::Fido);
        assert_eq!(
            classify("https://www.example.com/path?x=1", false),
            Scanned::Link {
                host: "example.com".into()
            }
        );
        assert_eq!(classify("hello there", false), Scanned::Text);
        assert_eq!(classify("   ", false), Scanned::Text);
        assert_eq!(
            classify(&"a".repeat(MAX_SCAN_CHARS + 1), true),
            Scanned::Text
        );
    }

    #[test]
    fn a_broken_code_link_is_called_invalid_not_text() {
        assert!(matches!(
            classify("otpauth://totp/x?secret=!!!", false),
            Scanned::Invalid(_)
        ));
        assert!(matches!(
            classify(&format!("otpauth://hotp/x?secret={KEY}&counter=1"), false),
            Scanned::Invalid(_)
        ));
        assert!(matches!(
            classify("otpauth://totp/x", false),
            Scanned::Invalid(_)
        ));
    }

    #[test]
    fn a_bare_key_counts_only_when_typed_or_pasted() {
        assert!(matches!(classify(KEY, true), Scanned::Totp(_)));
        assert!(matches!(
            classify("jbsw y3dp ehpk 3pxp", true),
            Scanned::Totp(_)
        ));
        assert_eq!(
            classify(KEY, false),
            Scanned::Text,
            "a camera does not read bare keys"
        );
        // too short, or not Base32
        assert_eq!(classify("JBSWY3DP", true), Scanned::Text);
        assert_eq!(classify("hello world, this is text!", true), Scanned::Text);
    }
}
