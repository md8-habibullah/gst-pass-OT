//! Guess the format of an import file from its **content** (never from its name).
//!
//! * KDBX: the 8 magic bytes.
//! * ZIP: `export.data` inside means 1Password; a `data.json` member means Proton Pass.
//! * JSON: the keys of the top-level object (Bitwarden, Proton, Aegis, 2FAS, Dashlane, Enpass).
//! * XML: a `KeePassFile` root.
//! * Text with `otpauth://` or `otpauth-migration://` lines.
//! * CSV: the set of header names of each vendor; any other table is "another CSV".
//!
//! Chrome, Edge and Brave write the same file, so all three are reported as `chrome-csv`; the
//! screen may show the name the person picked.

use super::kdbx;
use super::text::{decode, header_row, norm};
use super::zipsafe::{looks_like_zip, SafeZip};
use super::ImportFormat;

/// How much of the start of a file the sniffer looks at (a ZIP is looked at in full).
pub const HEAD_BYTES: usize = 1024 * 1024;

/// Names of the keys of the top-level JSON object found in `text` (which may be cut off).
fn top_keys(text: &str, max: usize) -> Vec<String> {
    let mut keys = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut cur = String::new();
    let mut last: Option<String> = None;
    for c in text.chars() {
        if in_str {
            if esc {
                esc = false;
                if depth == 1 && cur.len() < 128 {
                    cur.push(c);
                }
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
                if depth == 1 {
                    last = Some(std::mem::take(&mut cur));
                }
            } else if depth == 1 && cur.len() < 128 {
                cur.push(c);
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                cur.clear();
            }
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth <= 0 {
                    break;
                }
            }
            ':' if depth == 1 => {
                if let Some(k) = last.take() {
                    keys.push(k);
                    if keys.len() >= max {
                        break;
                    }
                }
            }
            ',' => last = None,
            _ => {}
        }
    }
    keys
}

fn json_format(text: &str) -> Option<ImportFormat> {
    let keys = top_keys(text, 64);
    let has = |k: &str| keys.iter().any(|x| x == k);
    // Bitwarden: the plain export has `items`; the password-protected one has `encrypted`
    // together with `data` (or the key check `encKeyValidation_DO_NOT_EDIT`). The parser then
    // says that it is encrypted.
    if (has("items") && has("encrypted"))
        || has("encKeyValidation_DO_NOT_EDIT")
        || (has("encrypted") && has("data"))
    {
        return Some(ImportFormat::BitwardenJson);
    }
    if has("items") && (has("folders") || has("collections")) {
        return Some(if text.contains("\"template_type\"") {
            ImportFormat::EnpassJson
        } else {
            ImportFormat::BitwardenJson
        });
    }
    if has("vaults") {
        return Some(ImportFormat::ProtonJson);
    }
    if has("db") && (has("header") || has("version")) {
        return Some(ImportFormat::AegisJson);
    }
    if has("services") || has("servicesEncrypted") {
        return Some(ImportFormat::TwofasJson);
    }
    if keys.iter().any(|k| {
        let n = norm(k);
        n.contains("authentifiant") || n.contains("securenote") || n.contains("paymentmean")
    }) {
        return Some(ImportFormat::DashlaneJson);
    }
    if has("items") {
        return Some(if text.contains("\"template_type\"") {
            ImportFormat::EnpassJson
        } else {
            ImportFormat::BitwardenJson
        });
    }
    None
}

fn csv_format(headers: &[String]) -> ImportFormat {
    use ImportFormat as F;
    let h: Vec<String> = headers.iter().map(|x| norm(x)).collect();
    let has = |k: &str| h.iter().any(|x| x == k);
    if h == ["title", "url", "username", "password", "notes", "otpauth"] {
        return F::SafariCsv;
    }
    if has("loginuri") && has("loginusername") && has("loginpassword") {
        return F::BitwardenCsv;
    }
    if has("vault") && has("totp") && has("note") {
        return F::ProtonCsv;
    }
    if has("otpauth") && (has("favorite") || has("archived") || has("tags")) {
        return F::OnepasswordCsv;
    }
    if has("extra") && has("grouping") {
        return F::LastpassCsv;
    }
    if has("username2") || has("otpsecret") {
        return F::DashlaneCsv;
    }
    if has("cardholdername") && has("cvc") {
        return F::NordpassCsv;
    }
    if has("httprealm") && has("formactionorigin") {
        return F::FirefoxCsv;
    }
    if has("name") && has("url") && has("username") && has("password") {
        return F::ChromeCsv;
    }
    let middle_ok = h.len() >= 4
        && h.get(1..h.len() - 1).is_some_and(|mid| {
            mid.iter()
                .all(|x| x.starts_with("field") || x.starts_with("value") || x.starts_with("label"))
        });
    if h.first().is_some_and(|x| x == "title") && h.last().is_some_and(|x| x == "note") && middle_ok
    {
        return F::EnpassCsv;
    }
    F::CustomCsv
}

/// The best guess for `bytes` (the start of the file is enough, except for ZIP archives), or
/// `None` when the file is not recognised.
pub fn sniff(bytes: &[u8]) -> Option<ImportFormat> {
    if kdbx::is_kdbx(bytes) {
        return Some(ImportFormat::KeepassKdbx);
    }
    if looks_like_zip(bytes) {
        let zip = SafeZip::open(bytes).ok()?;
        if zip.find(|n| n == "export.data").is_some() {
            return Some(ImportFormat::Onepassword1pux);
        }
        if zip
            .find(|n| n.to_ascii_lowercase().ends_with("data.json"))
            .is_some()
        {
            return Some(ImportFormat::ProtonZip);
        }
        return None;
    }
    let head = bytes.get(..HEAD_BYTES).unwrap_or(bytes);
    let text = decode(head).ok()?;
    let text = text.as_str();
    // Text that is full of control characters is a binary file, not a table of any kind.
    let binary = text
        .chars()
        .take(65_536)
        .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\u{0c}'));
    if binary {
        return None;
    }
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return json_format(trimmed);
    }
    if trimmed.starts_with('<') {
        let top = trimmed.get(..trimmed.len().min(8192)).unwrap_or(trimmed);
        return top
            .contains("<KeePassFile")
            .then_some(ImportFormat::KeepassXml);
    }
    let first_line = trimmed
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let lower: String = first_line
        .chars()
        .take(24)
        .collect::<String>()
        .to_ascii_lowercase();
    if lower.starts_with("otpauth-migration://") {
        return Some(ImportFormat::GoogleMigration);
    }
    if lower.starts_with("otpauth://") {
        return Some(ImportFormat::OtpauthList);
    }
    let (_, headers) = header_row(trimmed)?;
    Some(csv_format(&headers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guess(text: &str) -> Option<ImportFormat> {
        sniff(text.as_bytes())
    }

    #[test]
    fn csv_vendors() {
        use ImportFormat as F;
        assert_eq!(
            guess("name,url,username,password,note\n"),
            Some(F::ChromeCsv)
        );
        assert_eq!(guess("name,url,username,password\n"), Some(F::ChromeCsv));
        assert_eq!(
            guess("url,username,password,httpRealm,formActionOrigin,guid,timeCreated\n"),
            Some(F::FirefoxCsv)
        );
        assert_eq!(
            guess("Title,URL,Username,Password,Notes,OTPAuth\n"),
            Some(F::SafariCsv)
        );
        assert_eq!(
            guess("Title,Url,Username,Password,OTPAuth,Favorite,Archived,Tags,Notes\n"),
            Some(F::OnepasswordCsv)
        );
        assert_eq!(guess("folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n"), Some(F::BitwardenCsv));
        assert_eq!(
            guess("type,name,url,email,username,password,note,totp,createTime,modifyTime,vault\n"),
            Some(F::ProtonCsv)
        );
        assert_eq!(
            guess("url,username,password,totp,extra,name,grouping,fav\n"),
            Some(F::LastpassCsv)
        );
        assert_eq!(
            guess("username,username2,username3,title,password,note,url,category,otpSecret\n"),
            Some(F::DashlaneCsv)
        );
        assert_eq!(guess("name,url,username,password,note,cardholdername,cardnumber,cvc,pin,expirydate,zipcode,folder,full_name\n"), Some(F::NordpassCsv));
        assert_eq!(
            guess("Title,Field,Value,Field,Value,Note\n"),
            Some(F::EnpassCsv)
        );
        assert_eq!(guess("Site;Login;Secret\nx;y;z\n"), Some(F::CustomCsv));
        assert_eq!(guess("one column only\nvalue\n"), None);
        assert_eq!(guess("\n\n"), None);
        assert_eq!(guess(""), None);
    }

    #[test]
    fn json_shapes() {
        use ImportFormat as F;
        assert_eq!(
            guess(r#"{"encrypted":false,"folders":[],"items":[]}"#),
            Some(F::BitwardenJson)
        );
        assert_eq!(
            guess(r#"{"encrypted":true,"encKeyValidation_DO_NOT_EDIT":"x","items":[]}"#),
            Some(F::BitwardenJson)
        );
        assert_eq!(
            guess(r#"{"version":"1","userId":"u","encrypted":false,"vaults":{}}"#),
            Some(F::ProtonJson)
        );
        assert_eq!(
            guess(r#"{"version":1,"header":{},"db":{"entries":[]}}"#),
            Some(F::AegisJson)
        );
        assert_eq!(
            guess(r#"{"services":[],"schemaVersion":4}"#),
            Some(F::TwofasJson)
        );
        assert_eq!(
            guess(r#"{"AUTHENTIFIANT":[],"SECURENOTE":[]}"#),
            Some(F::DashlaneJson)
        );
        assert_eq!(
            guess(r#"{"folders":[],"items":[{"template_type":"login.default"}]}"#),
            Some(F::EnpassJson)
        );
        assert_eq!(guess(r#"{"hello":"world"}"#), None);
        assert_eq!(guess("[1,2,3]"), None);
        // keys nested deeper do not count, and a cut-off file still sniffs
        assert_eq!(guess(r#"{"x":{"vaults":1}}"#), None);
        assert_eq!(
            guess(r#"{"encrypted":false,"folders":[{"id":"a","name":"x"}],"items":[{"na"#),
            Some(F::BitwardenJson)
        );
        // escaped quotes and colons inside strings do not confuse the scanner
        assert_eq!(
            guess(r#"{"a\":\"vaults":"x","services":[]}"#),
            Some(F::TwofasJson)
        );
    }

    #[test]
    fn xml_otpauth_and_binary() {
        use ImportFormat as F;
        assert_eq!(
            guess("<?xml version=\"1.0\"?><KeePassFile><Meta/></KeePassFile>"),
            Some(F::KeepassXml)
        );
        assert_eq!(guess("<html></html>"), None);
        assert_eq!(
            guess("otpauth://totp/a?secret=JBSWY3DPEHPK3PXP\n"),
            Some(F::OtpauthList)
        );
        assert_eq!(
            guess("# comment\n\notpauth-migration://offline?data=AAAA\n"),
            Some(F::GoogleMigration)
        );
        assert_eq!(
            sniff(&[0x03, 0xD9, 0xA2, 0x9A, 0x67, 0xFB, 0x4B, 0xB5, 0, 0]),
            Some(F::KeepassKdbx)
        );
        assert_eq!(sniff(&[0u8; 100]), None);
        assert_eq!(sniff(&[0xff; 100]), None);
        assert_eq!(
            sniff(b"\xEF\xBB\xBFname,url,username,password\n"),
            Some(F::ChromeCsv)
        );
    }

    #[test]
    fn zip_archives_are_looked_into() {
        use crate::importer::zipsafe::test_support::make_zip;
        let onepux = make_zip(
            &[("export.attributes", b"{}"), ("export.data", b"{}")],
            false,
        );
        assert_eq!(sniff(&onepux), Some(ImportFormat::Onepassword1pux));
        let proton = make_zip(&[("Proton Pass/data.json", b"{}")], false);
        assert_eq!(sniff(&proton), Some(ImportFormat::ProtonZip));
        let other = make_zip(&[("a.txt", b"x")], false);
        assert_eq!(sniff(&other), None);
        assert_eq!(sniff(b"PK\x03\x04broken"), None);
    }

    #[test]
    fn the_name_of_the_file_plays_no_part() {
        // the sniffer only ever sees bytes
        assert_eq!(
            guess("name,url,username,password\nA,https://a,u,p\n"),
            Some(ImportFormat::ChromeCsv)
        );
    }

    #[test]
    fn never_panics_on_mutated_input() {
        let seeds: [&[u8]; 4] = [
            br#"{"encrypted":false,"folders":[],"items":[{"name":"x"}]}"#,
            b"name,url,username,password\nA,https://a,u,p\n",
            b"<KeePassFile><Root/></KeePassFile>",
            b"otpauth://totp/a?secret=JBSWY3DPEHPK3PXP\n",
        ];
        for seed in seeds {
            for i in 0..seed.len() {
                let mut m = seed.to_vec();
                m[i] ^= 0xA5;
                let _ = sniff(&m);
                let _ = sniff(&seed[..i]);
            }
        }
    }
}
