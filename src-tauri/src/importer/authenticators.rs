//! Authenticator-app exports: Aegis (plain JSON), 2FAS (plain JSON), a list of `otpauth://`
//! links, and Google Authenticator's `otpauth-migration://` export links.
//!
//! Every account becomes a `totp` item. Counter-based (HOTP), Steam and Yandex codes cannot be
//! computed by Passman and are counted as skipped with a reason. Encrypted Aegis and 2FAS
//! exports are refused with the standard sentence.

use super::entry::{EntryKind, ImportedEntry};
use super::jsonutil::{self, lax, lax_i64, nullable, s};
use super::limits::{
    unsupported, MAX_ENTRIES, MSG_ENCRYPTED_EXPORT, MSG_NOT_TEXT, MSG_NO_ROWS, MSG_WRONG_FORMAT,
};
use super::otp;
use super::text::decode;
use super::ParsedImport;
use crate::storage::Result;
use serde::Deserialize;
use std::collections::HashMap;
use zeroize::{Zeroize, ZeroizeOnDrop};

const SKIP_KIND: &str = "it is a counter-based, Steam or Yandex code, which Passman cannot compute";
const SKIP_SECRET: &str = "its secret key could not be read";

fn title_of(issuer: &str, account: &str) -> String {
    match (issuer.trim().is_empty(), account.trim().is_empty()) {
        (false, false) => format!("{} ({})", issuer.trim(), account.trim()),
        (false, true) => issuer.trim().to_string(),
        (true, false) => account.trim().to_string(),
        (true, true) => String::new(),
    }
}

fn algo_of(raw: &str) -> &'static str {
    match raw
        .trim()
        .to_ascii_uppercase()
        .replace(['-', '_'], "")
        .as_str()
    {
        "SHA256" => "SHA256",
        "SHA512" => "SHA512",
        _ => "SHA1",
    }
}

/// One code account as an item of type `totp`.
fn account_entry(
    issuer: &str,
    account: &str,
    value: &str,
    note: &str,
    favorite: bool,
) -> ImportedEntry {
    let mut e = ImportedEntry::new(EntryKind::Totp, title_of(issuer, account));
    e.put("issuer", issuer);
    e.put("account", account);
    e.add_totp(value);
    e.set_notes(note);
    e.favorite = favorite;
    e
}

// ------------------------------------------------------------------ Aegis

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct AegisInfo {
    #[serde(default, deserialize_with = "lax")]
    secret: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    algo: Option<String>,
    #[serde(default, deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    digits: Option<i64>,
    #[serde(default, deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    period: Option<i64>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct AegisEntry {
    #[serde(default, rename = "type", deserialize_with = "lax")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    issuer: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    note: Option<String>,
    #[serde(default, deserialize_with = "super::jsonutil::lax_bool")]
    #[zeroize(skip)]
    favorite: Option<bool>,
    info: Option<AegisInfo>,
    #[serde(default, deserialize_with = "nullable")]
    groups: Vec<Option<String>>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct AegisGroup {
    #[serde(default, deserialize_with = "lax")]
    uuid: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
}

#[derive(Deserialize)]
struct AegisDb {
    #[serde(default, deserialize_with = "nullable")]
    entries: Vec<AegisEntry>,
    #[serde(default, deserialize_with = "nullable")]
    groups: Vec<AegisGroup>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AegisBody {
    Plain(AegisDb),
    Encrypted(#[allow(dead_code)] String),
}

#[derive(Deserialize)]
struct AegisFile {
    db: Option<AegisBody>,
}

pub fn parse_aegis(bytes: &[u8]) -> Result<ParsedImport> {
    let file: AegisFile = jsonutil::parse(bytes)?;
    let db = match file.db {
        Some(AegisBody::Plain(db)) => db,
        Some(AegisBody::Encrypted(_)) => return Err(unsupported(MSG_ENCRYPTED_EXPORT)),
        None => return Err(unsupported(MSG_WRONG_FORMAT)),
    };
    let groups: HashMap<&str, &str> = db
        .groups
        .iter()
        .filter_map(|g| Some((g.uuid.as_deref()?, g.name.as_deref()?)))
        .collect();
    let mut out = ParsedImport::default();
    for item in &db.entries {
        if !s(&item.kind).eq_ignore_ascii_case("totp") {
            out.skip(SKIP_KIND);
            continue;
        }
        let Some(info) = &item.info else {
            out.skip(SKIP_SECRET);
            continue;
        };
        let Some(secret) = otp::clean_base32(s(&info.secret)) else {
            out.skip(SKIP_SECRET);
            continue;
        };
        let digits = info.digits.and_then(|d| u8::try_from(d).ok()).unwrap_or(6);
        let period = info
            .period
            .and_then(|p| u64::try_from(p).ok())
            .unwrap_or(30);
        let value = otp::build_value(
            &secret,
            s(&item.issuer),
            s(&item.name),
            algo_of(s(&info.algo)),
            digits,
            period,
        );
        let mut e = account_entry(
            s(&item.issuer),
            s(&item.name),
            &value,
            s(&item.note),
            item.favorite.unwrap_or(false),
        );
        if let Some(name) = item
            .groups
            .iter()
            .flatten()
            .find_map(|g| groups.get(g.as_str()))
        {
            e.folder = Some((*name).to_string());
        }
        out.push(e)?;
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

// ------------------------------------------------------------------ 2FAS

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct TwofasOtp {
    #[serde(default, deserialize_with = "lax")]
    account: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    issuer: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    label: Option<String>,
    #[serde(default, deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    digits: Option<i64>,
    #[serde(default, deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    period: Option<i64>,
    #[serde(default, deserialize_with = "lax")]
    algorithm: Option<String>,
    #[serde(default, rename = "tokenType", deserialize_with = "lax")]
    token_type: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct TwofasService {
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    secret: Option<String>,
    otp: Option<TwofasOtp>,
    #[serde(default, rename = "groupId", deserialize_with = "lax")]
    group: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct TwofasGroup {
    #[serde(default, deserialize_with = "lax")]
    id: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
}

#[derive(Deserialize)]
struct TwofasFile {
    #[serde(default, deserialize_with = "nullable")]
    services: Vec<TwofasService>,
    #[serde(default, rename = "servicesEncrypted", deserialize_with = "lax")]
    encrypted: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    groups: Vec<TwofasGroup>,
}

pub fn parse_2fas(bytes: &[u8]) -> Result<ParsedImport> {
    let file: TwofasFile = jsonutil::parse(bytes)?;
    if file.encrypted.is_some() {
        return Err(unsupported(MSG_ENCRYPTED_EXPORT));
    }
    let groups: HashMap<&str, &str> = file
        .groups
        .iter()
        .filter_map(|g| Some((g.id.as_deref()?, g.name.as_deref()?)))
        .collect();
    let mut out = ParsedImport::default();
    for svc in &file.services {
        let otp_info = svc.otp.as_ref();
        let kind = otp_info
            .map_or("", |o| s(&o.token_type))
            .to_ascii_uppercase();
        if !kind.is_empty() && kind != "TOTP" {
            out.skip(SKIP_KIND);
            continue;
        }
        let Some(secret) = otp::clean_base32(s(&svc.secret)) else {
            out.skip(SKIP_SECRET);
            continue;
        };
        let issuer = otp_info
            .and_then(|o| o.issuer.as_deref())
            .filter(|i| !i.trim().is_empty())
            .unwrap_or_else(|| s(&svc.name));
        let account = otp_info
            .and_then(|o| o.account.as_deref().or(o.label.as_deref()))
            .unwrap_or("");
        let digits = otp_info
            .and_then(|o| o.digits)
            .and_then(|d| u8::try_from(d).ok())
            .unwrap_or(6);
        let period = otp_info
            .and_then(|o| o.period)
            .and_then(|p| u64::try_from(p).ok())
            .unwrap_or(30);
        let algo = algo_of(otp_info.map_or("", |o| s(&o.algorithm)));
        let value = otp::build_value(&secret, issuer, account, algo, digits, period);
        let mut e = account_entry(issuer, account, &value, "", false);
        if let Some(name) = svc.group.as_deref().and_then(|g| groups.get(g)) {
            e.folder = Some((*name).to_string());
        }
        out.push(e)?;
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

// ------------------------------------------------------------------ lists of links

/// What Ente Auth adds to every link of its plain-text export: `codeDisplay=<JSON>` with the
/// app's own settings for the code (pinned, trashed, tags, note). It is not part of the
/// `otpauth` standard, so it is read here and left out of the stored link.
#[derive(Default)]
struct EnteDisplay {
    pinned: bool,
    trashed: bool,
    tags: Vec<String>,
    note: String,
}

/// Split the `codeDisplay` parameter off a link. Returns the link without it (everything else
/// exactly as written) and, when it was there and readable, what it said.
fn split_code_display(link: &str) -> (String, Option<EnteDisplay>) {
    let link = link.trim();
    let Some((base, query)) = link.split_once('?') else {
        return (link.to_string(), None);
    };
    let mut kept: Vec<&str> = Vec::new();
    let mut display: Option<EnteDisplay> = None;
    for pair in query.split('&') {
        let is_display = pair
            .split_once('=')
            .is_some_and(|(k, _)| k.eq_ignore_ascii_case("codeDisplay"));
        if !is_display {
            kept.push(pair);
            continue;
        }
        let raw = otp::pct_decode(pair.split_once('=').map_or("", |(_, v)| v), false);
        if let Ok(serde_json::Value::Object(o)) = serde_json::from_str::<serde_json::Value>(&raw) {
            let flag = |k: &str| {
                o.get(k)
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            };
            display = Some(EnteDisplay {
                pinned: flag("pinned"),
                trashed: flag("trashed"),
                tags: o
                    .get("tags")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|t| t.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                note: o
                    .get("note")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    let clean = if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    (clean, display)
}

const SKIP_TRASHED: &str = "it is in the authenticator app's trash";

fn link_entry(link: &str, out: &mut ParsedImport) -> Result<()> {
    let (link, display) = split_code_display(link);
    let display = display.unwrap_or_default();
    match otp::parse_link(&link) {
        Ok(_) if display.trashed => {
            out.skip(SKIP_TRASHED);
            Ok(())
        }
        Ok(l) => {
            let mut e = account_entry(&l.issuer, &l.account, &link, &display.note, display.pinned);
            for t in &display.tags {
                e.add_tag(t);
            }
            if e.title.is_empty() {
                e.title = "One-time code".into();
            }
            out.push(e)
        }
        Err(why) if why.contains("HOTP") || why.contains("Steam") => {
            out.skip(SKIP_KIND);
            Ok(())
        }
        Err(_) => {
            out.skip(SKIP_SECRET);
            Ok(())
        }
    }
}

fn migration_entries(link: &str, out: &mut ParsedImport) -> Result<()> {
    match otp::parse_migration(link) {
        Ok(m) => {
            for _ in 0..m.skipped {
                out.skip(SKIP_KIND);
            }
            for a in &m.accounts {
                let mut e = account_entry(&a.issuer, &a.account, &a.value, "", false);
                if e.title.is_empty() {
                    e.title = "One-time code".into();
                }
                out.push(e)?;
            }
            Ok(())
        }
        Err(_) => {
            out.skip("its export link is damaged");
            Ok(())
        }
    }
}

/// Text with one `otpauth://` link per line; export links are expanded too.
pub fn parse_list(bytes: &[u8]) -> Result<ParsedImport> {
    let text = decode(bytes)?;
    let mut out = ParsedImport::default();
    let mut seen = 0usize;
    for line in text.as_str().lines() {
        let line = line.trim();
        let lower = line
            .get(..line.len().min(20))
            .unwrap_or("")
            .to_ascii_lowercase();
        if lower.starts_with("otpauth://") {
            seen += 1;
            link_entry(line, &mut out)?;
        } else if lower.starts_with("otpauth-migration://") {
            seen += 1;
            migration_entries(line, &mut out)?;
        } else if !line.is_empty() && !line.starts_with('#') {
            out.skip("the line is not an otpauth link");
        }
        if seen > MAX_ENTRIES {
            return Err(unsupported(super::limits::MSG_TOO_MANY));
        }
    }
    if seen == 0 {
        return Err(unsupported(MSG_NOT_TEXT));
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

/// A file with Google Authenticator export links (one or more).
pub fn parse_migration(bytes: &[u8]) -> Result<ParsedImport> {
    let text = decode(bytes)?;
    let mut out = ParsedImport::default();
    let mut seen = 0usize;
    for line in text.as_str().lines() {
        let line = line.trim();
        let lower = line
            .get(..line.len().min(20))
            .unwrap_or("")
            .to_ascii_lowercase();
        if lower.starts_with("otpauth-migration://") {
            seen += 1;
            migration_entries(line, &mut out)?;
        } else if lower.starts_with("otpauth://") {
            seen += 1;
            link_entry(line, &mut out)?;
        }
    }
    if seen == 0 {
        return Err(unsupported(MSG_WRONG_FORMAT));
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::model::ItemType;

    const AEGIS: &str = r#"{"version":1,"header":{"slots":null,"params":null},"db":{"version":2,
      "entries":[
        {"type":"totp","uuid":"1","name":"me@example.com","issuer":"GitHub","note":"work","favorite":true,"icon":null,
         "info":{"secret":"JBSWY3DPEHPK3PXP","algo":"SHA1","digits":6,"period":30},"groups":["g1"]},
        {"type":"totp","uuid":"2","name":"b","issuer":"Bank","info":{"secret":"jbsw y3dp ehpk 3pxp","algo":"SHA256","digits":8,"period":60}},
        {"type":"hotp","uuid":"3","name":"c","issuer":"X","info":{"secret":"JBSWY3DPEHPK3PXP","algo":"SHA1","digits":6,"counter":1}},
        {"type":"steam","uuid":"4","name":"d","issuer":"Steam","info":{"secret":"JBSWY3DPEHPK3PXP","algo":"SHA1","digits":5,"period":30}},
        {"type":"totp","uuid":"5","name":"bad","issuer":"Bad","info":{"secret":"!!!","algo":"SHA1","digits":6,"period":30}}
      ],"groups":[{"uuid":"g1","name":"Work"}]}}"#;

    #[test]
    fn aegis_plain() {
        let mut p = parse_aegis(AEGIS.as_bytes()).unwrap();
        p.close();
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.skipped, 3);
        let a = &p.entries[0];
        assert_eq!(a.title, "GitHub (me@example.com)");
        assert!(a.favorite);
        assert_eq!(a.folder.as_deref(), Some("Work"));
        let d = a.to_draft().draft;
        assert_eq!(d.item_type, ItemType::Totp);
        assert_eq!(d.fields["totp"], "JBSWY3DPEHPK3PXP");
        assert_eq!(d.fields["issuer"], "GitHub");
        assert_eq!(d.fields["account"], "me@example.com");
        let b = p.entries[1].to_draft().draft;
        assert!(b.fields["totp"].starts_with("otpauth://totp/Bank:b?secret=JBSWY3DPEHPK3PXP"));
        assert!(
            b.fields["totp"].contains("algorithm=SHA256")
                && b.fields["totp"].contains("digits=8")
                && b.fields["totp"].contains("period=60")
        );
        let mut copy = b.clone();
        copy.normalize().unwrap();
        assert!(p.warnings.iter().any(|w| w.contains("Steam")));
    }

    #[test]
    fn aegis_encrypted_is_refused() {
        let e =
            parse_aegis(br#"{"version":1,"header":{"slots":[{}],"params":{}},"db":"c2VjcmV0"}"#)
                .err()
                .unwrap();
        assert!(e.to_string().starts_with("This export is encrypted."));
        assert!(parse_aegis(br#"{"x":1}"#).is_err());
    }

    #[test]
    fn twofas_plain_and_encrypted() {
        let plain = r#"{"services":[
          {"name":"GitHub","secret":"JBSWY3DPEHPK3PXP","otp":{"account":"me","issuer":"GitHub","digits":6,"period":30,"algorithm":"SHA1","tokenType":"TOTP"},"groupId":"g"},
          {"name":"Old","secret":"JBSWY3DPEHPK3PXP","otp":{"tokenType":"HOTP","counter":3}},
          {"name":"Plain","secret":"JBSWY3DPEHPK3PXP"}],
          "groups":[{"id":"g","name":"Dev"}],"schemaVersion":4}"#;
        let p = parse_2fas(plain.as_bytes()).unwrap();
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.skipped, 1);
        assert_eq!(p.entries[0].folder.as_deref(), Some("Dev"));
        assert_eq!(p.entries[0].title, "GitHub (me)");
        assert_eq!(p.entries[1].title, "Plain");
        let enc =
            parse_2fas(br#"{"services":[],"servicesEncrypted":"abc:def:ghi","schemaVersion":4}"#)
                .err()
                .unwrap();
        assert!(enc.to_string().starts_with("This export is encrypted."));
    }

    #[test]
    fn otpauth_lists() {
        let text = "# my codes\notpauth://totp/Example:me?secret=JBSWY3DPEHPK3PXP&issuer=Example\n\notpauth://hotp/X:y?secret=JBSWY3DPEHPK3PXP&counter=1\nsome other line\notpauth://totp/NoSecret?issuer=Z\n";
        let p = parse_list(text.as_bytes()).unwrap();
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.skipped, 3);
        assert_eq!(p.entries[0].title, "Example (me)");
        assert!(parse_list(b"nothing here\n").is_err());
        assert!(parse_list(b"").is_err());
    }

    #[test]
    fn ente_auth_export_keeps_its_settings_and_not_its_extra_parameter() {
        // The shape Ente Auth writes: the standard parameters, then `codeDisplay=<percent-encoded JSON>`.
        let display = |json: &str| {
            let mut s = String::new();
            for b in json.bytes() {
                if b.is_ascii_alphanumeric() {
                    s.push(char::from(b));
                } else {
                    s.push_str(&format!("%{b:02X}"));
                }
            }
            s
        };
        let plain = display(
            r#"{"pinned":false,"trashed":false,"lastUsedAt":0,"tapCount":0,"tags":[],"note":"","position":0,"iconSrc":"","iconID":""}"#,
        );
        let rich = display(
            r#"{"pinned":true,"trashed":false,"tags":["Work","Cloud"],"note":"root account","iconSrc":"customIcon","iconID":"aws"}"#,
        );
        let gone = display(r#"{"pinned":false,"trashed":true,"tags":[],"note":""}"#);
        let text = format!(
            "otpauth://totp/Atlassian:me%40example.org?secret=JBSWY3DPEHPK3PXP&issuer=Atlassian&algorithm=SHA1&digits=6&period=30&codeDisplay={plain}\n\
             otpauth://totp/Amazon%20Web%20Services:root?algorithm=SHA1&digits=6&issuer=Amazon+Web+Services&period=30&secret=JBSWY3DPEHPK3PXP&codeDisplay={rich}\n\
             otpauth://totp/Old:x?secret=JBSWY3DPEHPK3PXP&issuer=Old&codeDisplay={gone}\n\
             otpauth://totp/Odd:me%20%20K7Q2M9X4TESTTOKEN?secret=JBSWY3DPEHPK3PXP&issuer=Odd&codeDisplay=%7Bnot-json\n"
        );
        let p = parse_list(text.as_bytes()).unwrap();
        assert_eq!(
            (p.entries.len(), p.skipped),
            (3, 1),
            "the trashed code is not imported"
        );
        assert_eq!(p.entries[0].title, "Atlassian (me@example.org)");
        assert!(
            !p.entries[0].favorite && p.entries[0].tags.is_empty() && p.entries[0].notes.is_empty()
        );
        assert_eq!(p.entries[1].title, "Amazon Web Services (root)");
        assert!(
            p.entries[1].favorite,
            "pinned in the app is a favourite here"
        );
        assert_eq!(p.entries[1].tags, vec!["Work", "Cloud"]);
        assert_eq!(p.entries[1].notes, "root account");
        for e in &p.entries {
            let link = &e.totp[0];
            assert!(!link.contains("codeDisplay"), "{link}");
            assert!(link.contains("secret=JBSWY3DPEHPK3PXP"));
            // Still a valid code after the extra parameter is gone.
            assert!(otp::stored_value(link).is_ok());
        }
        // The unreadable settings are ignored, the account label is kept as written.
        assert_eq!(p.entries[2].title, "Odd (me  K7Q2M9X4TESTTOKEN)");
        assert!(!p.entries[2].totp[0].contains("codeDisplay"));
    }

    #[test]
    fn google_migration_links() {
        let link =
            otp::test_support::migration_link(&[("GitHub", "me", b"Hello!\xde\xad\xbe\xef")]);
        let p = parse_migration(link.as_bytes()).unwrap();
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].title, "GitHub (me)");
        assert!(p.entries[0].totp[0].starts_with("JBSWY3DPEHPK3PXP"));
        let damaged = parse_migration(b"otpauth-migration://offline?data=%%%").unwrap();
        assert!(damaged.entries.is_empty() && damaged.skipped == 1);
        assert!(parse_migration(b"hello").is_err());
        let list = parse_list(link.as_bytes()).unwrap();
        assert_eq!(list.entries.len(), 1);
    }
}
