//! 1Password `.1pux` exports (1Password 8 "Export > 1PUX").
//!
//! A `.1pux` is a ZIP with `export.attributes`, `export.data` (JSON) and a `files/` folder that
//! holds attachments as `files/<documentId>__<fileName>`. Written from 1Password's public
//! description of the format. Field values are single-key objects (`{"concealed": "..."}`,
//! `{"totp": "otpauth://..."}`, `{"monthYear": 202909}`, ...) and are read by [`Val`].

use super::entry::{EntryKind, ImportedAttachment, ImportedEntry};
use super::jsonutil::{self, epoch_millis, lax, lax_bool, lax_i64, nullable, s};
use super::limits::{
    own_capped, unsupported, warn_title, MAX_ATTACHMENT_BYTES, MSG_NO_ROWS, MSG_WRONG_FORMAT,
};
use super::zipsafe::SafeZip;
use super::ParsedImport;
use crate::storage::Result;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A field value. `kind` is the single key of the source object (`string`, `concealed`,
/// `totp`, `monthYear`, `address`, ...); `text` its value as text; `parts` the named parts of an
/// object value (an address).
#[derive(Zeroize, ZeroizeOnDrop, Default)]
struct Val {
    kind: String,
    text: String,
    parts: Vec<(String, String)>,
}

struct Inner;

impl<'de> Visitor<'de> for Inner {
    type Value = (String, Vec<(String, String)>);

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a value")
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
        Ok((v.to_string(), Vec::new()))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
        Ok((v.to_string(), Vec::new()))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
        Ok((v.to_string(), Vec::new()))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
        Ok((v.to_string(), Vec::new()))
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> {
        Ok((v.to_string(), Vec::new()))
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
        Ok((String::new(), Vec::new()))
    }
    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
        Ok((String::new(), Vec::new()))
    }
    fn visit_map<A: MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut parts = Vec::new();
        let mut n = 0usize;
        while let Some(k) = map.next_key::<String>()? {
            // Only scalar members are kept; nested objects are skipped.
            let v: Option<String> = map.next_value_seed(LaxSeed)?;
            n += 1;
            if let (Some(v), true) = (v, n <= 32) {
                parts.push((k, v));
            }
        }
        let joined = parts
            .iter()
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        Ok((joined, parts))
    }
}

struct LaxSeed;

impl<'de> serde::de::DeserializeSeed<'de> for LaxSeed {
    type Value = Option<String>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        lax(d)
    }
}

impl<'de> Deserialize<'de> for Val {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Outer;
        impl<'de> Visitor<'de> for Outer {
            type Value = Val;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a field value")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Val, E> {
                Ok(Val {
                    kind: "string".into(),
                    text: v.to_string(),
                    parts: Vec::new(),
                })
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Val, E> {
                self.visit_str(&v.to_string())
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Val, E> {
                self.visit_str(&v.to_string())
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Val, E> {
                self.visit_str(&v.to_string())
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Val, E> {
                self.visit_str(&v.to_string())
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Val, E> {
                Ok(Val::default())
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Val, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(Val::default())
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Val, A::Error> {
                let mut val = Val::default();
                let mut first = true;
                while let Some(k) = map.next_key::<String>()? {
                    if first {
                        let (text, parts) = map.next_value_seed(InnerSeed)?;
                        val = Val {
                            kind: k,
                            text,
                            parts,
                        };
                        first = false;
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                Ok(val)
            }
        }
        d.deserialize_any(Outer)
    }
}

struct InnerSeed;

impl<'de> serde::de::DeserializeSeed<'de> for InnerSeed {
    type Value = (String, Vec<(String, String)>);

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        d.deserialize_any(Inner)
    }
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct LoginField {
    #[serde(default, deserialize_with = "lax")]
    value: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    designation: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct SectionField {
    #[serde(default, deserialize_with = "lax")]
    title: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    id: Option<String>,
    value: Option<Val>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    guarded: Option<bool>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Section {
    #[serde(default, deserialize_with = "lax")]
    title: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    fields: Vec<SectionField>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct DocAttr {
    #[serde(default, rename = "fileName", deserialize_with = "lax")]
    file_name: Option<String>,
    #[serde(default, rename = "documentId", deserialize_with = "lax")]
    document_id: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Details {
    #[serde(default, rename = "loginFields", deserialize_with = "nullable")]
    login_fields: Vec<LoginField>,
    #[serde(default, rename = "notesPlain", deserialize_with = "lax")]
    notes: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    sections: Vec<Section>,
    #[serde(default, rename = "documentAttributes")]
    document: Option<DocAttr>,
    #[serde(default, deserialize_with = "lax")]
    password: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct UrlEntry {
    #[serde(default, deserialize_with = "lax")]
    url: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Overview {
    #[serde(default, deserialize_with = "lax")]
    title: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    url: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    urls: Vec<UrlEntry>,
    #[serde(default, deserialize_with = "nullable")]
    tags: Vec<Option<String>>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Item {
    #[serde(default, deserialize_with = "lax")]
    uuid: Option<String>,
    #[serde(default, rename = "favIndex", deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    fav: Option<i64>,
    #[serde(default, rename = "createdAt", deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    created: Option<i64>,
    #[serde(default, rename = "updatedAt", deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    updated: Option<i64>,
    #[serde(default, deserialize_with = "lax")]
    state: Option<String>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    trashed: Option<bool>,
    #[serde(default, rename = "categoryUuid", deserialize_with = "lax")]
    category: Option<String>,
    details: Option<Details>,
    overview: Option<Overview>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct VaultAttrs {
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Vault {
    attrs: Option<VaultAttrs>,
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<Item>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Account {
    #[serde(default, deserialize_with = "nullable")]
    vaults: Vec<Vault>,
}

#[derive(Deserialize)]
struct Doc {
    #[serde(default, deserialize_with = "nullable")]
    accounts: Vec<Account>,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The text a field shows: dates and expiry months are written the way people read them.
fn shown(v: &Val) -> String {
    match v.kind.as_str() {
        "monthYear" => {
            let n: u32 = v.text.trim().parse().unwrap_or(0);
            if n >= 190_001 {
                format!("{:02}/{:04}", n % 100, n / 100)
            } else {
                v.text.clone()
            }
        }
        "date" => v
            .text
            .trim()
            .parse::<i64>()
            .ok()
            .and_then(epoch_millis)
            .map(|ms| iso_date(ms / 1000))
            .unwrap_or_else(|| v.text.clone()),
        "email" => v
            .parts
            .iter()
            .find(|(k, _)| k == "email_address")
            .map_or_else(|| v.text.clone(), |(_, e)| e.clone()),
        _ => v.text.clone(),
    }
}

/// `YYYY-MM-DD` of a Unix time in seconds.
fn iso_date(secs: u64) -> String {
    jsonutil::format_iso(secs.saturating_mul(1000))
        .get(..10)
        .unwrap_or("")
        .to_string()
}

fn is_secret_kind(v: &Val, guarded: bool) -> bool {
    guarded || matches!(v.kind.as_str(), "concealed" | "totp" | "creditCardNumber")
}

fn category_kind(cat: &str) -> EntryKind {
    match cat {
        "001" | "005" => EntryKind::Login,
        "002" => EntryKind::Card,
        "003" | "006" => EntryKind::Note,
        "004" => EntryKind::Identity,
        "114" => EntryKind::SshKey,
        _ => EntryKind::Note,
    }
}

fn convert(
    vault: &Vault,
    item: &Item,
    zip: &mut SafeZip<'_>,
    out: &mut ParsedImport,
) -> Result<()> {
    if item.trashed == Some(true)
        || item
            .state
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("trashed"))
    {
        out.skip("it was in the 1Password trash");
        return Ok(());
    }
    let title = item.overview.as_ref().map_or("", |o| s(&o.title));
    let cat = s(&item.category);
    let mut kind = category_kind(cat);
    let mut e = ImportedEntry::new(kind, title);
    e.source_id = item.uuid.clone();
    e.favorite = item.fav == Some(1);
    e.created = item.created.and_then(epoch_millis);
    e.modified = item.updated.and_then(epoch_millis);
    if let Some(name) = vault.attrs.as_ref().and_then(|a| a.name.as_deref()) {
        let n = name.trim();
        if !n.is_empty() {
            e.folder = Some(n.to_string());
        }
    }
    if let Some(o) = &item.overview {
        e.add_url(s(&o.url));
        for u in &o.urls {
            e.add_url(s(&u.url));
        }
        for t in o.tags.iter().flatten() {
            e.add_tag(t);
        }
    }
    let details = item.details.as_ref();
    e.set_notes(details.map_or("", |d| s(&d.notes)));

    if let Some(d) = details {
        for f in &d.login_fields {
            let value = s(&f.value);
            if value.is_empty() {
                continue;
            }
            match s(&f.designation) {
                "username" if e.username.is_empty() => e.username = own_capped(value),
                "password" if e.password.is_empty() => e.password = own_capped(value),
                _ => {}
            }
        }
        if cat == "005" && e.password.is_empty() {
            e.password = own_capped(s(&d.password));
        }
        for sec in &d.sections {
            for f in &sec.fields {
                let Some(v) = &f.value else { continue };
                let text = shown(v);
                if text.trim().is_empty() && v.parts.is_empty() {
                    continue;
                }
                let id = norm(s(&f.id));
                let label = if s(&f.title).trim().is_empty() {
                    s(&f.id)
                } else {
                    s(&f.title)
                };
                let secret = is_secret_kind(v, f.guarded == Some(true));
                match (kind, v.kind.as_str()) {
                    (_, "totp") => e.add_totp(&text),
                    (EntryKind::Card, _) => match (id.as_str(), v.kind.as_str()) {
                        (_, "creditCardNumber") | ("ccnum", _) => e.put("number", &text),
                        (_, "creditCardType") | ("type", _) => e.put("brand", &text),
                        ("cardholder", _) => e.put("cardholder", &text),
                        ("cvv" | "verificationnumber", _) => e.put("code", &text),
                        ("pin", _) => e.put("pin", &text),
                        ("expiry", _) | (_, "monthYear") if id != "validfrom" => {
                            e.put("expiry", &text)
                        }
                        _ => e.add_custom(label, &text, secret),
                    },
                    (EntryKind::Identity, "address") => {
                        let part = |k: &str| {
                            v.parts
                                .iter()
                                .find(|(n, _)| n == k)
                                .map_or("", |(_, x)| x.as_str())
                        };
                        e.put("address1", part("street"));
                        e.put("city", part("city"));
                        e.put("state", part("state"));
                        e.put("postalCode", part("zip"));
                        e.put("country", part("country"));
                    }
                    (EntryKind::Identity, _) => match id.as_str() {
                        "firstname" => e.put("firstName", &text),
                        "initial" => e.put("middleName", &text),
                        "lastname" => e.put("lastName", &text),
                        "company" => e.put("company", &text),
                        "email" => e.put("email", &text),
                        "defphone" | "cellphone" | "homephone"
                            if !e.typed.iter().any(|(k, _)| k == "phone") =>
                        {
                            e.put("phone", &text);
                        }
                        _ => e.add_custom(label, &text, secret),
                    },
                    (EntryKind::SshKey, "sshKey") => {
                        let part = |k: &str| {
                            v.parts
                                .iter()
                                .find(|(n, _)| n == k)
                                .map_or("", |(_, x)| x.as_str())
                        };
                        e.put("privateKey", part("privateKey"));
                    }
                    (_, "file") => {
                        // attachments inside sections are listed by name only
                        e.add_custom(label, &text, false);
                    }
                    _ => {
                        // Logins and everything else: user name and password slots, the rest as fields.
                        let key = id.as_str();
                        if kind == EntryKind::Note
                            && (key == "username" || key == "password")
                            && secret == (key == "password")
                        {
                            kind = EntryKind::Login;
                            e.kind = EntryKind::Login;
                        }
                        match (key, v.kind.as_str()) {
                            ("username", _)
                                if e.kind == EntryKind::Login && e.username.is_empty() =>
                            {
                                e.username = own_capped(&text);
                            }
                            ("password", "concealed")
                                if e.kind == EntryKind::Login && e.password.is_empty() =>
                            {
                                e.password = own_capped(&text);
                            }
                            (_, "url") if e.kind == EntryKind::Login => e.add_url(&text),
                            _ => e.add_custom(label, &text, secret),
                        }
                    }
                }
            }
        }
        // Attachments: the document itself, then files listed by name in sections are not read.
        if let Some(doc) = &d.document {
            let name = s(&doc.file_name);
            let id = s(&doc.document_id);
            if !name.is_empty() {
                let want = format!("files/{id}__{name}");
                let found = zip.find(|n| {
                    n == want || (!id.is_empty() && n.starts_with(&format!("files/{id}")))
                });
                match found {
                    Some(i) => {
                        let bytes = zip.read(i)?;
                        if bytes.len() > MAX_ATTACHMENT_BYTES {
                            out.warn(format!(
                                "The file attached to \u{201c}{}\u{201d} is too large and was not read.",
                                warn_title(title)
                            ));
                        } else {
                            e.attachments.push(ImportedAttachment {
                                name: name.to_string(),
                                bytes: bytes.to_vec(),
                            });
                        }
                    }
                    None => out.warn(format!(
                        "The file attached to \u{201c}{}\u{201d} is not in the archive.",
                        warn_title(title)
                    )),
                }
            }
        }
    }
    let _ = kind;
    if e.is_blank() && e.attachments.is_empty() {
        out.skip("it is empty");
        return Ok(());
    }
    out.push(e)
}

pub fn parse(bytes: &[u8]) -> Result<ParsedImport> {
    let mut zip = SafeZip::open(bytes)?;
    let idx = zip
        .find(|n| n == "export.data")
        .ok_or_else(|| unsupported(MSG_WRONG_FORMAT))?;
    let json = zip.read(idx)?;
    let doc: Doc = jsonutil::parse(&json)?;
    drop(json);
    let mut out = ParsedImport::default();
    for account in &doc.accounts {
        for vault in &account.vaults {
            for item in &vault.items {
                convert(vault, item, &mut zip, &mut out)?;
            }
        }
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::zipsafe::test_support::make_zip;
    use crate::storage::model::ItemType;

    const DATA: &str = r#"{"accounts":[{"attrs":{"accountName":"x"},"vaults":[{"attrs":{"uuid":"v","name":"Personal"},"items":[
      {"uuid":"i1","favIndex":1,"createdAt":1700000000,"updatedAt":1700000100,"state":"active","categoryUuid":"001",
       "details":{"loginFields":[{"value":"me","name":"username","fieldType":"T","designation":"username"},{"value":"pw","name":"password","fieldType":"P","designation":"password"}],
         "notesPlain":"a note",
         "sections":[{"title":"","fields":[{"title":"one-time password","id":"TOTP_x","value":{"totp":"otpauth://totp/S:me?secret=JBSWY3DPEHPK3PXP&issuer=S"}},{"title":"recovery","id":"r","value":{"concealed":"abc"}},{"title":"since","id":"d","value":{"date":1700000000}}]}]},
       "overview":{"title":"Site","url":"https://a.example","urls":[{"label":"w","url":"https://a.example"},{"label":"x","url":"https://b.example"}],"tags":["work","vip"]}},
      {"uuid":"i2","categoryUuid":"002","details":{"sections":[{"fields":[
          {"title":"cardholder name","id":"cardholder","value":{"string":"A B"}},
          {"title":"type","id":"type","value":{"creditCardType":"visa"}},
          {"title":"number","id":"ccnum","value":{"creditCardNumber":"4111111111111111"}},
          {"title":"verification number","id":"cvv","value":{"concealed":"123"}},
          {"title":"expiry date","id":"expiry","value":{"monthYear":202909}},
          {"title":"valid from","id":"validFrom","value":{"monthYear":202001}}]}]},
       "overview":{"title":"Visa"}},
      {"uuid":"i3","categoryUuid":"003","details":{"notesPlain":"secret memo"},"overview":{"title":"Memo"}},
      {"uuid":"i4","categoryUuid":"004","details":{"sections":[{"fields":[
          {"title":"first name","id":"firstname","value":{"string":"Ana"}},
          {"title":"last name","id":"lastname","value":{"string":"Lopez"}},
          {"title":"address","id":"address","value":{"address":{"street":"1 Road","city":"Town","country":"ES","zip":"12345","state":"M"}}},
          {"title":"email","id":"email","value":{"email":{"email_address":"a@example.com","provider":"x"}}}]}]},
       "overview":{"title":"Me"}},
      {"uuid":"i5","categoryUuid":"110","details":{"sections":[{"fields":[
          {"title":"username","id":"username","value":{"string":"root"}},
          {"title":"password","id":"password","value":{"concealed":"srv"}},
          {"title":"URL","id":"url","value":{"url":"https://srv.example"}}]}]},"overview":{"title":"Server"}},
      {"uuid":"i6","categoryUuid":"006","details":{"documentAttributes":{"fileName":"doc.txt","documentId":"D1"}},"overview":{"title":"Doc"}},
      {"uuid":"i7","trashed":true,"categoryUuid":"001","details":{"loginFields":[{"value":"x","designation":"password"}]},"overview":{"title":"Gone"}}
    ]}]}]}"#;

    fn zip() -> Vec<u8> {
        make_zip(
            &[
                ("export.attributes", br#"{"version":3}"#),
                ("export.data", DATA.as_bytes()),
                ("files/D1__doc.txt", b"file body"),
            ],
            false,
        )
    }

    #[test]
    fn parses_the_categories() {
        let p = parse(&zip()).unwrap();
        assert_eq!(p.entries.len(), 6);
        assert_eq!(p.skipped, 1);
        let a = &p.entries[0];
        assert_eq!(
            (a.title.as_str(), a.username.as_str(), a.password.as_str()),
            ("Site", "me", "pw")
        );
        assert!(a.favorite);
        assert_eq!(a.folder.as_deref(), Some("Personal"));
        assert_eq!(a.tags, vec!["work", "vip"]);
        assert_eq!(a.urls, vec!["https://a.example", "https://b.example"]);
        assert_eq!(a.totp.len(), 1);
        assert_eq!(a.notes, "a note");
        assert!(a
            .custom_fields
            .iter()
            .any(|f| f.name == "recovery" && f.hidden));
        assert!(a
            .custom_fields
            .iter()
            .any(|f| f.name == "since" && f.value == "2023-11-14"));
        let card = p.entries[1].to_draft().draft;
        assert_eq!(card.item_type, ItemType::Card);
        assert_eq!(card.fields["number"], "4111111111111111");
        assert_eq!(card.fields["expMonth"], "09");
        assert_eq!(card.fields["expYear"], "2029");
        assert_eq!(card.fields["code"], "123");
        assert_eq!(card.fields["brand"], "visa");
        assert!(card.custom_fields.iter().any(|f| f.name == "valid from"));
        assert_eq!(p.entries[2].notes, "secret memo");
        let id = p.entries[3].to_draft().draft;
        assert_eq!(id.fields["firstName"], "Ana");
        assert_eq!(id.fields["city"], "Town");
        assert_eq!(id.fields["email"], "a@example.com");
        let srv = &p.entries[4];
        assert_eq!(srv.kind, EntryKind::Login);
        assert_eq!(
            (srv.username.as_str(), srv.password.as_str()),
            ("root", "srv")
        );
        assert_eq!(srv.urls, vec!["https://srv.example"]);
        let doc = &p.entries[5];
        assert_eq!(doc.attachments.len(), 1);
        assert_eq!(doc.attachments[0].bytes, b"file body");
        for e in &p.entries {
            let mut d = e.to_draft().draft;
            d.normalize().unwrap();
        }
    }

    #[test]
    fn wrong_archives_and_garbage_are_refused() {
        let other = make_zip(&[("a.txt", b"x")], false);
        assert_eq!(parse(&other).err().unwrap().code(), "unsupported_file");
        assert!(parse(b"PK\x03\x04garbage").is_err());
        let bad = make_zip(&[("export.data", b"{not json")], false);
        assert!(parse(&bad).is_err());
        let empty = make_zip(&[("export.data", br#"{"accounts":[]}"#)], false);
        assert!(parse(&empty).is_err());
    }

    #[test]
    fn dates() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_700_000_000), "2023-11-14");
        assert_eq!(iso_date(951_782_400), "2000-02-29");
    }
}
