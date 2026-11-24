//! Proton Pass exports: the JSON file and the ZIP that holds it (plain exports only; the
//! PGP-encrypted export is refused with the standard sentence).
//!
//! Written from Proton's public description of the format: a `vaults` object whose values hold a
//! name and a list of items; each item has `data` (`metadata`, `extraFields`, `type`, `content`),
//! a `state` (1 active, 2 trashed) and Unix times in seconds.

use super::entry::{EntryKind, ImportedEntry};
use super::jsonutil::{self, epoch_millis, lax, lax_bool, lax_i64, nullable, s, Pairs, Scalar};
use super::limits::{
    own_capped, unsupported, warn_title, MSG_DAMAGED, MSG_ENCRYPTED_EXPORT, MSG_NO_ROWS,
    MSG_WRONG_FORMAT,
};
use super::zipsafe::SafeZip;
use super::ParsedImport;
use crate::storage::Result;
use serde::Deserialize;
use std::collections::BTreeMap;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Meta {
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    note: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct ExtraData {
    #[serde(default, deserialize_with = "lax")]
    content: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Extra {
    #[serde(default, rename = "fieldName", deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "lax")]
    kind: Option<String>,
    data: Option<ExtraData>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Content {
    #[serde(default, rename = "itemEmail", deserialize_with = "lax")]
    email: Option<String>,
    #[serde(default, rename = "itemUsername", deserialize_with = "lax")]
    username: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    password: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    urls: Vec<Scalar>,
    #[serde(default, rename = "totpUri", deserialize_with = "lax")]
    totp: Option<String>,
    // cards
    #[serde(default, rename = "cardholderName", deserialize_with = "lax")]
    holder: Option<String>,
    #[serde(default, rename = "cardType", deserialize_with = "lax")]
    card_type: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    number: Option<String>,
    #[serde(default, rename = "verificationNumber", deserialize_with = "lax")]
    cvv: Option<String>,
    #[serde(default, rename = "expirationDate", deserialize_with = "lax")]
    expiry: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    pin: Option<String>,
    // every other key (identity fields and newer additions)
    #[serde(flatten)]
    rest: Pairs,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Data {
    metadata: Option<Meta>,
    #[serde(default, rename = "extraFields", deserialize_with = "nullable")]
    extra: Vec<Extra>,
    #[serde(default, rename = "type", deserialize_with = "lax")]
    kind: Option<String>,
    content: Option<Content>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Item {
    data: Option<Data>,
    #[serde(default, deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    state: Option<i64>,
    #[serde(default, rename = "aliasEmail", deserialize_with = "lax")]
    alias: Option<String>,
    #[serde(default, rename = "createTime", deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    created: Option<i64>,
    #[serde(default, rename = "modifyTime", deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    modified: Option<i64>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    pinned: Option<bool>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Vault {
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Doc {
    #[serde(default, deserialize_with = "lax_bool")]
    encrypted: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    vaults: BTreeMap<String, Vault>,
}

/// `birthdate` -> `Birthdate`, `socialSecurityNumber` -> `Social security number`.
fn humanize(key: &str) -> String {
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn convert(vault: &Vault, item: &Item, out: &mut ParsedImport) -> Result<()> {
    if item.state == Some(2) {
        out.skip("it was in the Proton Pass trash");
        return Ok(());
    }
    let Some(data) = &item.data else {
        out.skip("it has no data");
        return Ok(());
    };
    let name = data.metadata.as_ref().map_or("", |m| s(&m.name));
    let kind_text = s(&data.kind).to_ascii_lowercase();
    let kind = match kind_text.as_str() {
        "login" | "alias" | "" => EntryKind::Login,
        "creditcard" => EntryKind::Card,
        "identity" => EntryKind::Identity,
        "note" => EntryKind::Note,
        _ => EntryKind::Note,
    };
    let mut e = ImportedEntry::new(kind, name);
    e.favorite = item.pinned.unwrap_or(false);
    e.created = item.created.and_then(epoch_millis);
    e.modified = item.modified.and_then(epoch_millis);
    e.folder = vault
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    e.set_notes(data.metadata.as_ref().map_or("", |m| s(&m.note)));

    if let Some(c) = &data.content {
        match kind {
            EntryKind::Login => {
                let user = s(&c.username).trim();
                let mail = s(&c.email).trim();
                let alias = s(&item.alias).trim();
                e.username = if !user.is_empty() {
                    user.to_string()
                } else if !mail.is_empty() {
                    mail.to_string()
                } else {
                    alias.to_string()
                };
                if !user.is_empty() && !mail.is_empty() && user != mail {
                    e.add_custom("Email", mail, false);
                }
                e.password = own_capped(s(&c.password));
                for u in &c.urls {
                    e.add_url(s(&u.0));
                }
                e.add_totp(s(&c.totp));
            }
            EntryKind::Card => {
                e.put("cardholder", s(&c.holder));
                e.put("brand", s(&c.card_type));
                e.put("number", s(&c.number));
                e.put("code", s(&c.cvv));
                e.put("expiry", s(&c.expiry));
                e.put("pin", s(&c.pin));
            }
            EntryKind::Identity => {
                e.put("firstName", s(&c.rest_value("firstName")));
                e.put("middleName", s(&c.rest_value("middleName")));
                e.put("lastName", s(&c.rest_value("lastName")));
                if e.typed
                    .iter()
                    .all(|(k, _)| k != "firstName" && k != "lastName")
                {
                    let full = c.rest_value("fullName");
                    let mut parts = s(&full).split_whitespace();
                    if let Some(first) = parts.next() {
                        e.put("firstName", first);
                        let rest: Vec<&str> = parts.collect();
                        if let Some((last, mid)) = rest.split_last() {
                            e.put("lastName", last);
                            e.put("middleName", &mid.join(" "));
                        }
                    }
                }
                e.put("email", s(&c.rest_value("email")));
                e.put("phone", s(&c.rest_value("phoneNumber")));
                e.put("company", s(&c.rest_value("organization")));
                e.put("address1", s(&c.rest_value("streetAddress")));
                e.put("postalCode", s(&c.rest_value("zipOrPostalCode")));
                e.put("city", s(&c.rest_value("city")));
                e.put("state", s(&c.rest_value("stateOrProvince")));
                e.put("country", s(&c.rest_value("countryOrRegion")));
                e.put("nationalId", s(&c.rest_value("socialSecurityNumber")));
                e.put("passport", s(&c.rest_value("passportNumber")));
                e.put("license", s(&c.rest_value("licenseNumber")));
                const MAPPED: [&str; 14] = [
                    "firstName",
                    "middleName",
                    "lastName",
                    "fullName",
                    "email",
                    "phoneNumber",
                    "organization",
                    "streetAddress",
                    "zipOrPostalCode",
                    "city",
                    "stateOrProvince",
                    "countryOrRegion",
                    "socialSecurityNumber",
                    "passportNumber",
                ];
                for (k, v) in &c.rest.0 {
                    if MAPPED.contains(&k.as_str())
                        || k == "licenseNumber"
                        || s(&v.0).trim().is_empty()
                    {
                        continue;
                    }
                    e.add_custom(&humanize(k), s(&v.0), false);
                }
            }
            _ => {}
        }
    }
    for x in &data.extra {
        let value = x.data.as_ref().map_or("", |d| s(&d.content));
        let label = s(&x.name);
        match s(&x.kind).to_ascii_lowercase().as_str() {
            "totp" => e.add_totp(value),
            "hidden" => e.add_custom(label, value, true),
            _ => e.add_custom(label, value, false),
        }
    }
    if e.is_blank() {
        out.skip("it is empty");
        return Ok(());
    }
    if kind_text == "custom" {
        out.warn(format!(
            "\u{201c}{}\u{201d} is a custom item; it was imported as a note.",
            warn_title(name)
        ));
    }
    out.push(e)
}

impl Content {
    fn rest_value(&self, key: &str) -> Option<String> {
        self.rest.get(key).map(str::to_string)
    }
}

pub fn parse_json(bytes: &[u8]) -> Result<ParsedImport> {
    // An encrypted export stores `data` as one armoured text; it would fail the typed read, so the
    // flag is checked on a light first pass.
    #[derive(Deserialize)]
    struct Flag {
        #[serde(default, deserialize_with = "lax_bool")]
        encrypted: Option<bool>,
        #[serde(default, deserialize_with = "lax")]
        data: Option<String>,
    }
    let flag: Flag = jsonutil::parse(bytes)?;
    if flag.encrypted == Some(true)
        || flag
            .data
            .as_deref()
            .is_some_and(|d| d.contains("BEGIN PGP"))
    {
        return Err(unsupported(MSG_ENCRYPTED_EXPORT));
    }
    let doc: Doc = jsonutil::parse(bytes)?;
    if doc.encrypted == Some(true) {
        return Err(unsupported(MSG_ENCRYPTED_EXPORT));
    }
    let mut out = ParsedImport::default();
    for vault in doc.vaults.values() {
        for item in &vault.items {
            convert(vault, item, &mut out)?;
        }
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

/// The ZIP export: `Proton Pass/data.json` plus a `files/` folder for attachments.
pub fn parse_zip(bytes: &[u8]) -> Result<ParsedImport> {
    let mut zip = SafeZip::open(bytes)?;
    let idx = zip
        .find(|n| n.to_ascii_lowercase().ends_with("data.json"))
        .ok_or_else(|| unsupported(MSG_WRONG_FORMAT))?;
    let json = zip.read(idx)?;
    let extras = zip
        .names()
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            *i != idx && !n.ends_with('/') && !n.to_ascii_lowercase().ends_with(".json")
        })
        .count();
    let mut out = parse_json(&json).map_err(|e| match e.code() {
        "unsupported_file" => e,
        _ => unsupported(MSG_DAMAGED),
    })?;
    if extras > 0 {
        out.warn(format!(
            "The archive holds {extras} attached files, which are not imported yet."
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::zipsafe::test_support::make_zip;
    use crate::storage::model::ItemType;

    const SAMPLE: &str = r#"{
      "version": "1.20.0", "userId": "u", "encrypted": false,
      "vaults": {
        "v1": {"name": "Personal", "description": "", "items": [
          {"itemId":"1","shareId":"v1","state":1,"pinned":true,"createTime":1700000000,"modifyTime":1700000100,"aliasEmail":null,
           "data":{"metadata":{"name":"Site","note":"hello","itemUuid":"x"},
             "extraFields":[{"fieldName":"pin","type":"hidden","data":{"content":"1234"}},{"fieldName":"label","type":"text","data":{"content":"v"}},{"fieldName":"2fa","type":"totp","data":{"content":"JBSWY3DPEHPK3PXP"}}],
             "type":"login","content":{"itemEmail":"me@example.com","itemUsername":"me","password":"pw","urls":["https://a.example","https://b.example"],"totpUri":"otpauth://totp/S:me?secret=JBSWY3DPEHPK3PXP&issuer=S","passkeys":[]}}},
          {"itemId":"2","state":1,"data":{"metadata":{"name":"Card","note":""},"type":"creditCard","content":{"cardholderName":"A B","cardType":"0","number":"4111111111111111","verificationNumber":"123","expirationDate":"2029-09","pin":"1111"}}},
          {"itemId":"3","state":1,"data":{"metadata":{"name":"Me","note":""},"type":"identity","content":{"fullName":"Ana Maria Lopez","email":"a@example.com","phoneNumber":"555","organization":"ACME","streetAddress":"1 Road","city":"Town","countryOrRegion":"ES","birthdate":"1990-01-01","floor":"3"}}},
          {"itemId":"4","state":1,"data":{"metadata":{"name":"Memo","note":"remember"},"type":"note","content":{}}},
          {"itemId":"5","state":1,"aliasEmail":"x@alias.example","data":{"metadata":{"name":"Alias","note":""},"type":"alias","content":{}}},
          {"itemId":"6","state":2,"data":{"metadata":{"name":"Trashed","note":""},"type":"login","content":{"password":"x"}}}
        ]},
        "v2": {"name": "Work", "items": [
          {"itemId":"7","state":1,"data":{"metadata":{"name":"Work site","note":""},"type":"login","content":{"itemUsername":"w","password":"p","urls":[]}}}
        ]}
      }
    }"#;

    #[test]
    fn parses_every_type_vaults_and_trash() {
        let p = parse_json(SAMPLE.as_bytes()).unwrap();
        assert_eq!(p.entries.len(), 6);
        assert_eq!(p.skipped, 1);
        let a = &p.entries[0];
        assert_eq!(
            (a.title.as_str(), a.username.as_str(), a.password.as_str()),
            ("Site", "me", "pw")
        );
        assert_eq!(a.folder.as_deref(), Some("Personal"));
        assert!(a.favorite);
        assert_eq!(a.created, Some(1_700_000_000_000));
        assert_eq!(a.urls.len(), 2);
        assert_eq!(a.totp.len(), 2);
        assert_eq!(
            a.custom_fields.iter().filter(|f| f.name == "Email").count(),
            1
        );
        assert!(a.custom_fields.iter().any(|f| f.name == "pin" && f.hidden));
        let card = p.entries[1].to_draft().draft;
        assert_eq!(card.item_type, ItemType::Card);
        assert_eq!(card.fields["expYear"], "2029");
        assert_eq!(card.fields["expMonth"], "09");
        let id = p.entries[2].to_draft().draft;
        assert_eq!(id.fields["firstName"], "Ana");
        assert_eq!(id.fields["middleName"], "Maria");
        assert_eq!(id.fields["lastName"], "Lopez");
        assert!(id.custom_fields.iter().any(|f| f.name == "Birthdate"));
        assert_eq!(p.entries[3].kind, EntryKind::Note);
        assert_eq!(p.entries[4].username, "x@alias.example");
        assert_eq!(p.entries[5].folder.as_deref(), Some("Work"));
        for e in &p.entries {
            let mut d = e.to_draft().draft;
            d.normalize().unwrap();
        }
    }

    #[test]
    fn encrypted_export_is_refused() {
        let e = parse_json(br#"{"encrypted": true, "data": "-----BEGIN PGP MESSAGE-----"}"#)
            .err()
            .unwrap();
        assert!(e.to_string().starts_with("This export is encrypted."));
    }

    #[test]
    fn zip_with_data_json() {
        let z = make_zip(
            &[
                ("Proton Pass/data.json", SAMPLE.as_bytes()),
                ("Proton Pass/files/abc", b"binary"),
            ],
            false,
        );
        let p = parse_zip(&z).unwrap();
        assert_eq!(p.entries.len(), 6);
        assert!(p.warnings.iter().any(|w| w.contains("attached files")));
        let none = make_zip(&[("readme.txt", b"x")], false);
        assert_eq!(parse_zip(&none).err().unwrap().code(), "unsupported_file");
        assert!(parse_zip(b"PK\x03\x04junk").is_err());
    }

    #[test]
    fn humanized_names() {
        assert_eq!(humanize("birthdate"), "Birthdate");
        assert_eq!(humanize("socialSecurityNumber"), "Social security number");
    }
}
