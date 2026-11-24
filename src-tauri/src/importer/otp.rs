//! One-time code values found in import files: `otpauth://` links, bare Base32 keys, and Google
//! Authenticator's `otpauth-migration://` export links (a Protocol Buffers message, decoded by
//! hand with bounds checks on every read).

use crate::storage::model::totp_account;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use zeroize::{Zeroize, Zeroizing};

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
/// A bare key shorter than this is almost certainly not a key ("yes", "none", ...).
const MIN_BARE_KEY: usize = 8;
const MAX_LINK_CHARS: usize = 8192;

/// Upper-case and strip the separators people copy along; `None` if anything else is left over.
pub fn clean_base32(raw: &str) -> Option<Zeroizing<String>> {
    let mut out = Zeroizing::new(String::with_capacity(raw.len()));
    for c in raw.chars() {
        match c {
            ' ' | '\t' | '\r' | '\n' | '-' | '=' => {}
            'A'..='Z' | '2'..='7' => out.push(c),
            'a'..='z' => out.push(c.to_ascii_uppercase()),
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

pub fn to_base32(bytes: &[u8]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(bytes.len() * 8 / 5 + 1));
    let mut bits = 0u32;
    let mut acc = 0u32;
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            let idx = ((acc >> (bits - 5)) & 31) as usize;
            out.push(char::from(B32.get(idx).copied().unwrap_or(b'A')));
            bits -= 5;
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        let idx = ((acc << (5 - bits)) & 31) as usize;
        out.push(char::from(B32.get(idx).copied().unwrap_or(b'A')));
    }
    out
}

pub(super) fn pct_decode(s: &str, plus_is_space: bool) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b.get(i) {
            Some(b'%') => {
                let hex = b
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok());
                if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            Some(b'+') if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            Some(&c) => {
                out.push(c);
                i += 1;
            }
            None => break,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A time-based code read from a link, with what the link says about it.
pub struct OtpLink {
    pub issuer: String,
    pub account: String,
    pub secret: Zeroizing<String>,
    pub algorithm: &'static str,
    pub digits: u8,
    pub period: u64,
}

impl Drop for OtpLink {
    fn drop(&mut self) {
        self.issuer.zeroize();
        self.account.zeroize();
    }
}

/// Parse an `otpauth://totp/...` link. The error is a fixed phrase fit for a warning.
pub fn parse_link(link: &str) -> Result<OtpLink, &'static str> {
    let link = link.trim();
    if link.len() > MAX_LINK_CHARS {
        return Err("the link is far too long");
    }
    let lower = link.to_ascii_lowercase();
    if lower.starts_with("otpauth://hotp") {
        return Err("counter-based (HOTP) codes are not supported");
    }
    let Some(rest) = lower
        .starts_with("otpauth://totp")
        .then(|| link.get("otpauth://totp".len()..))
        .flatten()
    else {
        return Err("it is not a time-based otpauth link");
    };
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let query = query.split('#').next().unwrap_or("");
    let label = pct_decode(path.trim_start_matches('/'), false);
    let (label_issuer, label_account) = match label.split_once(':') {
        Some((i, a)) => (i.trim().to_string(), a.trim().to_string()),
        None => (String::new(), label.trim().to_string()),
    };
    let mut secret: Option<Zeroizing<String>> = None;
    let mut issuer = String::new();
    let mut algorithm = "SHA1";
    let mut digits = 6u8;
    let mut period = 30u64;
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let v = pct_decode(v, true);
        match k.to_ascii_lowercase().as_str() {
            "secret" => secret = clean_base32(&v),
            "issuer" => issuer = v.trim().to_string(),
            "algorithm" => {
                algorithm = match v.trim().to_ascii_uppercase().replace('-', "").as_str() {
                    "SHA1" => "SHA1",
                    "SHA256" => "SHA256",
                    "SHA512" => "SHA512",
                    _ => return Err("it uses an unsupported algorithm"),
                }
            }
            "digits" => {
                digits = v
                    .trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|d| (6..=8).contains(d))
                    .ok_or("the digit count must be 6, 7 or 8")?;
            }
            "period" => {
                period = v
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|p| (1..=86_400).contains(p))
                    .ok_or("the period must be 1 to 86400 seconds")?;
            }
            "encoder" if v.trim().eq_ignore_ascii_case("steam") => {
                return Err("Steam Guard codes are not supported");
            }
            _ => {}
        }
    }
    let secret = secret.ok_or("it has no readable secret")?;
    if issuer.is_empty() {
        issuer = label_issuer;
    }
    Ok(OtpLink {
        issuer,
        account: label_account,
        secret,
        algorithm,
        digits,
        period,
    })
}

/// The value to store for a code with explicit parameters: the bare key when everything is the
/// default (SHA-1, 6 digits, 30 s), otherwise a full link.
pub fn build_value(
    secret_b32: &str,
    issuer: &str,
    account: &str,
    algorithm: &str,
    digits: u8,
    period: u64,
) -> Zeroizing<String> {
    if algorithm == "SHA1" && digits == 6 && period == 30 {
        return Zeroizing::new(secret_b32.to_string());
    }
    let label = match (issuer.is_empty(), account.is_empty()) {
        (false, false) => format!("{}:{}", pct_encode(issuer), pct_encode(account)),
        (false, true) => pct_encode(issuer),
        (true, false) => pct_encode(account),
        (true, true) => "code".to_string(),
    };
    let mut uri = format!("otpauth://totp/{label}?secret={secret_b32}");
    if !issuer.is_empty() {
        uri.push_str(&format!("&issuer={}", pct_encode(issuer)));
    }
    if algorithm != "SHA1" {
        uri.push_str(&format!("&algorithm={algorithm}"));
    }
    if digits != 6 {
        uri.push_str(&format!("&digits={digits}"));
    }
    if period != 30 {
        uri.push_str(&format!("&period={period}"));
    }
    Zeroizing::new(uri)
}

/// Validate a code value from an import file and return the form to store: a valid `otpauth://`
/// link as written, or a clean Base32 key. Steam, HOTP and export links are refused with a phrase.
pub fn stored_value(raw: &str) -> Result<Zeroizing<String>, &'static str> {
    let v = raw.trim();
    if v.is_empty() {
        return Err("it is empty");
    }
    let lower = v.get(..v.len().min(24)).unwrap_or("").to_ascii_lowercase();
    if lower.starts_with("otpauth-migration://") {
        return Err("it is an export link, not a single code");
    }
    if lower.starts_with("steam://") {
        return Err("Steam Guard codes are not supported");
    }
    if lower.starts_with("otpauth://") {
        let _ = parse_link(v)?;
        return match totp_account(v) {
            Ok(_) => Ok(Zeroizing::new(v.to_string())),
            Err(_) => Err("it is not a valid time-based code"),
        };
    }
    let key = v
        .split_once('=')
        .filter(|(k, _)| k.trim().eq_ignore_ascii_case("secret"))
        .map_or(v, |(_, rest)| rest);
    let clean = clean_base32(key).ok_or("it is not a Base32 key")?;
    if clean.len() < MIN_BARE_KEY {
        return Err("the key is too short");
    }
    match totp_account(&clean) {
        Ok(_) => Ok(clean),
        Err(_) => Err("it is not a valid key"),
    }
}

// ------------------------------------------------------------------ Google Authenticator export

/// One account of an export, with its code value ready to store.
pub struct MigrationAccount {
    pub issuer: String,
    pub account: String,
    pub value: Zeroizing<String>,
}

impl Drop for MigrationAccount {
    fn drop(&mut self) {
        self.issuer.zeroize();
        self.account.zeroize();
    }
}

/// What an export link held: the usable accounts and how many it had to leave out (counter-based
/// or unsupported algorithms).
pub struct Migration {
    pub accounts: Vec<MigrationAccount>,
    pub skipped: usize,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn varint(&mut self) -> Option<u64> {
        let mut result = 0u64;
        let mut shift = 0u32;
        loop {
            let b = *self.buf.get(self.pos)?;
            self.pos += 1;
            if shift > 63 {
                return None;
            }
            result |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
        }
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = usize::try_from(self.varint()?).ok()?;
        let end = self.pos.checked_add(n)?;
        let out = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }

    fn skip(&mut self, wire: u64) -> Option<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => self.pos = self.pos.checked_add(8).filter(|p| *p <= self.buf.len())?,
            2 => {
                self.bytes()?;
            }
            5 => self.pos = self.pos.checked_add(4).filter(|p| *p <= self.buf.len())?,
            _ => return None,
        }
        Some(())
    }
}

fn b64_lenient(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .collect();
    if cleaned.contains(['-', '_']) {
        URL_SAFE_NO_PAD.decode(cleaned.as_bytes()).ok()
    } else {
        STANDARD_NO_PAD.decode(cleaned.as_bytes()).ok()
    }
}

/// Decode an `otpauth-migration://offline?data=...` link.
pub fn parse_migration(link: &str) -> Result<Migration, &'static str> {
    let link = link.trim();
    if link.len() > 1024 * 1024 {
        return Err("the export link is far too long");
    }
    let query = link.split_once('?').map(|(_, q)| q).unwrap_or("");
    let data = query
        .split('&')
        .find_map(|p| p.strip_prefix("data="))
        .ok_or("the export link has no data")?;
    let decoded = pct_decode(data, false);
    let bytes = Zeroizing::new(b64_lenient(&decoded).ok_or("the export link is damaged")?);
    if bytes.is_empty() {
        return Err("the export link is empty");
    }
    let mut top = Reader {
        buf: &bytes,
        pos: 0,
    };
    let mut accounts = Vec::new();
    let mut skipped = 0usize;
    while !top.done() {
        let tag = top.varint().ok_or("the export link is damaged")?;
        if tag >> 3 == 1 && tag & 7 == 2 {
            let inner = top.bytes().ok_or("the export link is damaged")?;
            let mut r = Reader { buf: inner, pos: 0 };
            let mut secret: &[u8] = &[];
            let mut name = String::new();
            let mut issuer = String::new();
            let (mut alg, mut digits_enum, mut kind) = (1u64, 1u64, 2u64);
            while !r.done() {
                let t = r.varint().ok_or("the export link is damaged")?;
                let (field, wire) = (t >> 3, t & 7);
                match (field, wire) {
                    (1, 2) => secret = r.bytes().ok_or("the export link is damaged")?,
                    (2, 2) => {
                        name =
                            String::from_utf8_lossy(r.bytes().ok_or("the export link is damaged")?)
                                .into_owned();
                    }
                    (3, 2) => {
                        issuer =
                            String::from_utf8_lossy(r.bytes().ok_or("the export link is damaged")?)
                                .into_owned();
                    }
                    (4, 0) => alg = r.varint().ok_or("the export link is damaged")?,
                    (5, 0) => digits_enum = r.varint().ok_or("the export link is damaged")?,
                    (6, 0) => kind = r.varint().ok_or("the export link is damaged")?,
                    _ => r.skip(wire).ok_or("the export link is damaged")?,
                }
            }
            let algorithm = match alg {
                0 | 1 => Some("SHA1"),
                2 => Some("SHA256"),
                3 => Some("SHA512"),
                _ => None,
            };
            let digits = match digits_enum {
                2 => 8u8,
                3 => 7,
                _ => 6,
            };
            let (Some(algorithm), true) =
                (algorithm, (kind == 2 || kind == 0) && !secret.is_empty())
            else {
                skipped += 1;
                continue;
            };
            // Google stores "Issuer:account" in the name when it also has an issuer.
            let account = match name.strip_prefix(&format!("{issuer}:")) {
                Some(a) if !issuer.is_empty() => a.trim().to_string(),
                _ => name.trim().to_string(),
            };
            let b32 = to_base32(secret);
            let value = build_value(&b32, &issuer, &account, algorithm, digits, 30);
            accounts.push(MigrationAccount {
                issuer: issuer.trim().to_string(),
                account,
                value,
            });
            name.zeroize();
            issuer.zeroize();
        } else {
            top.skip(tag & 7).ok_or("the export link is damaged")?;
        }
        if accounts.len() > super::limits::MAX_ENTRIES {
            return Err("the export link has too many accounts");
        }
    }
    Ok(Migration { accounts, skipped })
}

#[cfg(test)]
pub(crate) mod test_support {
    use base64::Engine;

    /// An `otpauth-migration://` link for TOTP accounts (SHA-1, 6 digits). Test-sized: every
    /// field must be shorter than 128 bytes.
    pub fn migration_link(accounts: &[(&str, &str, &[u8])]) -> String {
        fn field(tag: u8, data: &[u8]) -> Vec<u8> {
            let mut v = vec![tag, data.len() as u8];
            v.extend_from_slice(data);
            v
        }
        let mut payload = Vec::new();
        for (issuer, account, secret) in accounts {
            let mut a = Vec::new();
            a.extend(field(0x0a, secret));
            a.extend(field(0x12, format!("{issuer}:{account}").as_bytes()));
            a.extend(field(0x1a, issuer.as_bytes()));
            a.extend([0x20, 1, 0x28, 1, 0x30, 2]);
            payload.extend(field(0x0a, &a));
        }
        payload.extend([0x10, 1, 0x18, 1, 0x20, 1]);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&payload);
        format!(
            "otpauth-migration://offline?data={}",
            b64.replace('+', "%2B")
                .replace('/', "%2F")
                .replace('=', "%3D")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_roundtrip_and_cleaning() {
        assert_eq!(
            to_base32(b"Hello!\xde\xad\xbe\xef").as_str(),
            "JBSWY3DPEHPK3PXP"
        );
        assert_eq!(
            clean_base32("jbsw y3dp-ehpk 3pxp==").unwrap().as_str(),
            "JBSWY3DPEHPK3PXP"
        );
        assert!(clean_base32("abc1").is_none());
        assert!(clean_base32("  ").is_none());
    }

    #[test]
    fn stored_values() {
        assert_eq!(
            stored_value("jbsw y3dp ehpk 3pxp").unwrap().as_str(),
            "JBSWY3DPEHPK3PXP"
        );
        assert_eq!(
            stored_value("secret=JBSWY3DPEHPK3PXP").unwrap().as_str(),
            "JBSWY3DPEHPK3PXP"
        );
        let uri = "otpauth://totp/GitHub:ana?secret=JBSWY3DPEHPK3PXP&issuer=GitHub";
        assert_eq!(stored_value(uri).unwrap().as_str(), uri);
        assert!(stored_value("steam://ABCDEFGH")
            .unwrap_err()
            .contains("Steam"));
        assert!(
            stored_value("otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP&counter=1")
                .unwrap_err()
                .contains("HOTP")
        );
        assert!(
            stored_value("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&encoder=steam")
                .unwrap_err()
                .contains("Steam")
        );
        assert!(stored_value("yes").is_err(), "short words are not keys");
        assert!(stored_value("true").is_err());
        assert!(stored_value("").is_err());
        assert!(stored_value("otpauth-migration://offline?data=AAAA").is_err());
        assert!(stored_value("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=9").is_err());
        assert!(stored_value("not a key!").is_err());
    }

    #[test]
    fn link_parts() {
        let l = parse_link("otpauth://totp/Example%3Aana%40example.com?secret=jbswy3dpehpk3pxp&issuer=Example&algorithm=sha256&digits=8&period=60").unwrap();
        assert_eq!(l.issuer, "Example");
        assert_eq!(l.account, "ana@example.com");
        assert_eq!(l.algorithm, "SHA256");
        assert_eq!((l.digits, l.period), (8, 60));
        assert_eq!(l.secret.as_str(), "JBSWY3DPEHPK3PXP");
        let l = parse_link("otpauth://totp/Acme:bob?secret=JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(l.issuer, "Acme");
        assert!(parse_link("otpauth://totp/x").is_err());
    }

    #[test]
    fn built_values() {
        assert_eq!(
            build_value("JBSWY3DPEHPK3PXP", "A", "b", "SHA1", 6, 30).as_str(),
            "JBSWY3DPEHPK3PXP"
        );
        let v = build_value("JBSWY3DPEHPK3PXP", "A B", "me@x", "SHA256", 8, 60);
        assert!(v.starts_with("otpauth://totp/A%20B:me%40x?secret=JBSWY3DPEHPK3PXP"));
        assert!(stored_value(&v).is_ok());
    }

    /// otpauth-migration payload with two accounts: a TOTP (Example:ana, SHA1, 6 digits) and an
    /// HOTP that must be skipped.
    fn sample_link() -> String {
        fn field(tag: u8, data: &[u8]) -> Vec<u8> {
            let mut v = vec![tag, data.len() as u8];
            v.extend_from_slice(data);
            v
        }
        let mut a = Vec::new();
        a.extend(field(0x0a, b"Hello!\xde\xad\xbe\xef"));
        a.extend(field(0x12, b"Example:ana"));
        a.extend(field(0x1a, b"Example"));
        a.extend([0x20, 1, 0x28, 1, 0x30, 2]);
        let mut h = Vec::new();
        h.extend(field(0x0a, b"counter!"));
        h.extend(field(0x12, b"hotp"));
        h.extend([0x20, 1, 0x28, 1, 0x30, 1]);
        let mut payload = Vec::new();
        payload.extend(field(0x0a, &a));
        payload.extend(field(0x0a, &h));
        payload.extend([0x10, 1, 0x18, 1, 0x20, 1]); // version, batch size, index: skipped
        let b64 = base64::engine::general_purpose::STANDARD.encode(&payload);
        format!(
            "otpauth-migration://offline?data={}",
            b64.replace('+', "%2B")
                .replace('/', "%2F")
                .replace('=', "%3D")
        )
    }

    #[test]
    fn google_migration_link() {
        let m = parse_migration(&sample_link()).unwrap();
        assert_eq!(m.accounts.len(), 1);
        assert_eq!(m.skipped, 1);
        let a = &m.accounts[0];
        assert_eq!((a.issuer.as_str(), a.account.as_str()), ("Example", "ana"));
        assert_eq!(a.value.as_str(), "JBSWY3DPEHPK3PXP");
        assert!(stored_value(&a.value).is_ok());
    }

    #[test]
    fn damaged_migration_links_are_errors_not_panics() {
        assert!(parse_migration("otpauth-migration://offline").is_err());
        assert!(parse_migration("otpauth-migration://offline?data=").is_err());
        assert!(parse_migration("otpauth-migration://offline?data=!!!").is_err());
        let good = sample_link();
        for cut in (0..good.len()).step_by(7) {
            let _ = parse_migration(&good[..cut]);
        }
        // a length that points past the end
        let bad = base64::engine::general_purpose::STANDARD.encode([0x0a, 0x7f, 0x01]);
        assert!(parse_migration(&format!("otpauth-migration://offline?data={bad}")).is_err());
    }
}
