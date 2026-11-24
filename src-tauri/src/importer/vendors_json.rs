//! Dashlane JSON and Enpass JSON exports.
//!
//! Both are written from the vendors' public descriptions and tested with synthetic files only.
//! Dashlane: one object with a list per kind of record (`AUTHENTIFIANT` for logins,
//! `SECURENOTE`, `PAYMENTMEAN_CREDITCARD`, ...). Enpass: `folders` and `items`, each item with
//! `template_type`, `fields` (`label`, `type`, `value`, `sensitive`) and base64 attachments.

use super::entry::{EntryKind, ImportedAttachment, ImportedEntry};
use super::jsonutil::{self, epoch_millis, lax, lax_bool, lax_i64, nullable, s, Pairs};
use super::limits::{
    own_capped, unsupported, warn_title, MAX_ATTACHMENT_BYTES, MSG_NO_ROWS, MSG_WRONG_FORMAT,
};
use super::text::norm;
use super::ParsedImport;
use crate::storage::Result;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::de::IgnoredAny;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The first non-empty value among `names` (compared on letters and digits only).
fn get<'a>(rec: &'a Pairs, names: &[&str]) -> &'a str {
    for n in names {
        for (k, v) in &rec.0 {
            if norm(k) == *n {
                if let Some(t) = v.0.as_deref().filter(|t| !t.is_empty()) {
                    return t;
                }
            }
        }
    }
    ""
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Section {
    List(Vec<Pairs>),
    Other(#[allow(dead_code)] IgnoredAny),
}

pub fn parse_dashlane(bytes: &[u8]) -> Result<ParsedImport> {
    let doc: BTreeMap<String, Section> = jsonutil::parse(bytes)?;
    let mut out = ParsedImport::default();
    let mut recognised = false;
    for (name, section) in &doc {
        let Section::List(records) = section else {
            continue;
        };
        let key = norm(name);
        let kind = if key.contains("authentifiant") || key.contains("credential") {
            EntryKind::Login
        } else if key.contains("securenote") || key == "notes" {
            EntryKind::Note
        } else if key.contains("creditcard") || key.contains("paymentcard") {
            EntryKind::Card
        } else {
            if !records.is_empty() {
                let n = records.len();
                out.warn(format!(
                    "{} of the kind \u{201c}{}\u{201d} {} not imported.",
                    if n == 1 {
                        "1 record".to_string()
                    } else {
                        format!("{n} records")
                    },
                    warn_title(name),
                    if n == 1 { "is" } else { "are" }
                ));
            }
            continue;
        };
        recognised = true;
        for r in records {
            let title = get(r, &["title", "name", "bank"]);
            let mut e = ImportedEntry::new(kind, title);
            let category = get(r, &["category"]);
            if !category.is_empty() {
                e.folder = Some(category.to_string());
            }
            match kind {
                EntryKind::Login => {
                    let login = get(r, &["login", "username", "user"]);
                    let email = get(r, &["email"]);
                    let second = get(r, &["secondarylogin", "username2"]);
                    e.username = own_capped(if login.is_empty() {
                        if email.is_empty() {
                            second
                        } else {
                            email
                        }
                    } else {
                        login
                    });
                    if !login.is_empty() && !email.is_empty() && login != email {
                        e.add_custom("Email", email, false);
                    }
                    if !second.is_empty() && second != e.username {
                        e.add_custom("Secondary username", second, false);
                    }
                    e.password = own_capped(get(r, &["password"]));
                    e.add_url(get(r, &["domain", "url", "website"]));
                    e.set_notes(get(r, &["note", "notes"]));
                    e.add_totp(get(r, &["otpsecret", "otp", "totp"]));
                }
                EntryKind::Note => e.set_notes(get(r, &["content", "note", "notes", "text"])),
                _ => {
                    e.put(
                        "cardholder",
                        get(r, &["owner", "cardholder", "cardholdername", "holder"]),
                    );
                    e.put("number", get(r, &["cardnumber", "number"]));
                    e.put("code", get(r, &["securitycode", "cvv", "cvc", "code"]));
                    let (m, y) = (
                        get(r, &["expiremonth", "expirymonth"]),
                        get(r, &["expireyear", "expiryyear"]),
                    );
                    e.put("expMonth", m);
                    e.put("expYear", y);
                    e.set_notes(get(r, &["note", "notes"]));
                }
            }
            if e.is_blank() {
                out.skip("it is empty");
                continue;
            }
            out.push(e)?;
        }
    }
    if !recognised {
        return Err(unsupported(MSG_WRONG_FORMAT));
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

// ------------------------------------------------------------------ Enpass

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct EField {
    #[serde(default, deserialize_with = "lax")]
    label: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "lax")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    value: Option<String>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    sensitive: Option<bool>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    deleted: Option<bool>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct EAttachment {
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    data: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct EItem {
    #[serde(default, deserialize_with = "lax")]
    uuid: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    title: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    category: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    template_type: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    note: Option<String>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    favorite: Option<bool>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    trashed: Option<bool>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    archived: Option<bool>,
    #[serde(default, deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    #[serde(rename = "createdAt", alias = "created_at")]
    created: Option<i64>,
    #[serde(default, deserialize_with = "lax_i64", alias = "updatedAt")]
    #[zeroize(skip)]
    updated_at: Option<i64>,
    #[serde(default, deserialize_with = "nullable")]
    folders: Vec<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    fields: Vec<EField>,
    #[serde(default, deserialize_with = "nullable")]
    attachments: Vec<EAttachment>,
}

#[derive(Deserialize)]
struct EFolder {
    #[serde(default, deserialize_with = "lax")]
    uuid: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    title: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    parent_uuid: Option<String>,
}

#[derive(Deserialize)]
struct EDoc {
    #[serde(default, deserialize_with = "nullable")]
    folders: Vec<EFolder>,
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<EItem>,
}

fn folder_path(id: &str, folders: &HashMap<&str, &EFolder>) -> Vec<String> {
    let mut path = Vec::new();
    let mut cur = Some(id);
    let mut guard = 0;
    while let Some(c) = cur {
        guard += 1;
        if guard > 32 {
            break;
        }
        match folders.get(c) {
            Some(f) => {
                path.push(s(&f.title).to_string());
                cur = f.parent_uuid.as_deref().filter(|p| !p.is_empty());
            }
            None => break,
        }
    }
    path.reverse();
    path
}

fn label_role(label: &str) -> &'static str {
    match norm(label).as_str() {
        "firstname" | "givenname" => "firstName",
        "lastname" | "surname" | "familyname" => "lastName",
        "middlename" | "initial" => "middleName",
        "email" | "emailaddress" => "email",
        "phone" | "phonenumber" | "mobile" | "mobilephone" => "phone",
        "company" | "organization" => "company",
        "address" | "street" | "address1" => "address1",
        "address2" => "address2",
        "city" | "town" => "city",
        "state" | "province" | "region" => "state",
        "zip" | "zipcode" | "postalcode" | "postcode" => "postalCode",
        "country" => "country",
        _ => "",
    }
}

pub fn parse_enpass(bytes: &[u8]) -> Result<ParsedImport> {
    let doc: EDoc = jsonutil::parse(bytes)?;
    let folders: HashMap<&str, &EFolder> = doc
        .folders
        .iter()
        .filter_map(|f| Some((f.uuid.as_deref()?, f)))
        .collect();
    let mut out = ParsedImport::default();
    for item in &doc.items {
        if item.trashed == Some(true) {
            out.skip("it was in the Enpass trash");
            continue;
        }
        let category = s(&item.category).to_ascii_lowercase();
        let template = s(&item.template_type).to_ascii_lowercase();
        let kind = if category.contains("creditcard") || template.starts_with("creditcard") {
            EntryKind::Card
        } else if category.contains("identity") || template.starts_with("identity") {
            EntryKind::Identity
        } else if category == "note" || template.starts_with("note") {
            EntryKind::Note
        } else {
            EntryKind::Login
        };
        let mut e = ImportedEntry::new(kind, s(&item.title));
        e.source_id = item.uuid.clone();
        e.favorite = item.favorite == Some(true);
        e.created = item.created.and_then(epoch_millis);
        e.modified = item.updated_at.and_then(epoch_millis);
        e.set_notes(s(&item.note));
        if let Some(first) = item.folders.iter().flatten().next() {
            let path = folder_path(first, &folders);
            let parts: Vec<&str> = path.iter().map(String::as_str).collect();
            e.set_folder_path(&parts);
        }
        for f in &item.fields {
            let value = s(&f.value);
            if f.deleted == Some(true) || value.trim().is_empty() {
                continue;
            }
            let label = s(&f.label);
            let kind_text = s(&f.kind).to_ascii_lowercase();
            let secret = f.sensitive == Some(true)
                || matches!(
                    kind_text.as_str(),
                    "password" | "pin" | "secret" | "ccpin" | "cccvc"
                );
            match (kind, kind_text.as_str()) {
                (_, "totp") => e.add_totp(value),
                (EntryKind::Login, "username" | "email") if e.username.is_empty() => {
                    e.username = own_capped(value);
                }
                (EntryKind::Login, "password") if e.password.is_empty() => {
                    e.password = own_capped(value);
                }
                (EntryKind::Login, "url") => e.add_url(value),
                (EntryKind::Card, "ccname") => e.put("cardholder", value),
                (EntryKind::Card, "ccnumber") => e.put("number", value),
                (EntryKind::Card, "cccvc") => e.put("code", value),
                (EntryKind::Card, "ccexpiry") => e.put("expiry", value),
                (EntryKind::Card, "cctype") => e.put("brand", value),
                (EntryKind::Card, "ccpin") => e.put("pin", value),
                (EntryKind::Identity, _) if !label_role(label).is_empty() => {
                    e.put(label_role(label), value);
                }
                _ => e.add_custom(label, value, secret),
            }
        }
        for a in &item.attachments {
            let Some(data) = a.data.as_deref() else {
                continue;
            };
            let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
            match STANDARD.decode(cleaned.as_bytes()) {
                Ok(bytes) if bytes.len() <= MAX_ATTACHMENT_BYTES => {
                    e.attachments.push(ImportedAttachment {
                        name: s(&a.name).to_string(),
                        bytes,
                    });
                }
                _ => out.warn(format!(
                    "An attached file of \u{201c}{}\u{201d} was too large or damaged and was not read.",
                    warn_title(s(&item.title))
                )),
            }
        }
        if e.is_blank() && e.attachments.is_empty() {
            out.skip("it is empty");
            continue;
        }
        if item.archived == Some(true) {
            e.add_tag("Archived");
        }
        out.push(e)?;
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

    #[test]
    fn dashlane_sections() {
        let json = r#"{
          "AUTHENTIFIANT": [
            {"title":"Shop","domain":"https://shop.example","login":"me","email":"me@example.com","secondaryLogin":"alt","password":"pw","note":"n","otpSecret":"JBSWY3DPEHPK3PXP","category":"Shopping"},
            {"title":"","domain":"https://empty.example"}
          ],
          "SECURENOTE": [{"title":"Memo","content":"remember","category":"Notes"}],
          "PAYMENTMEAN_CREDITCARD": [{"name":"Visa","owner":"A B","cardNumber":"4111111111111111","expireMonth":"09","expireYear":"2029","securityCode":"123"}],
          "IDENTITY": [{"firstName":"Ana"}],
          "version": "1"
        }"#;
        let mut p = parse_dashlane(json.as_bytes()).unwrap();
        p.close();
        assert_eq!(p.entries.len(), 4);
        let a = &p.entries[0];
        assert_eq!(
            (a.title.as_str(), a.username.as_str(), a.password.as_str()),
            ("Shop", "me", "pw")
        );
        assert_eq!(a.folder.as_deref(), Some("Shopping"));
        assert_eq!(a.urls, vec!["https://shop.example"]);
        assert_eq!(a.custom_fields.len(), 2);
        assert_eq!(a.totp.len(), 1);
        assert_eq!(p.entries[3].kind, EntryKind::Note); // sections come in alphabetical order
        let card = p.entries[2].to_draft().draft;
        assert_eq!(card.item_type, ItemType::Card);
        assert_eq!(card.fields["expYear"], "2029");
        assert!(p.warnings.iter().any(|w| w.contains("IDENTITY")));
        assert!(parse_dashlane(br#"{"x": 1}"#).is_err());
        assert!(parse_dashlane(b"[1,2]").is_err());
    }

    const ENPASS: &str = r#"{"folders":[{"uuid":"f1","title":"Work","parent_uuid":""},{"uuid":"f2","title":"Servers","parent_uuid":"f1"}],
      "items":[
        {"uuid":"i1","title":"Site","category":"login","template_type":"login.default","favorite":1,"folders":["f2"],"note":"n","createdAt":1700000000,"updated_at":1700000100,
         "fields":[{"label":"Username","type":"username","value":"me","sensitive":0},{"label":"Password","type":"password","value":"pw","sensitive":1},{"label":"Website","type":"url","value":"https://a.example"},{"label":"TOTP","type":"totp","value":"JBSWY3DPEHPK3PXP"},{"label":"Recovery","type":"text","value":"abc","sensitive":1},{"label":"Gone","type":"text","value":"x","deleted":1}],
         "attachments":[{"name":"a.txt","data":"SGVsbG8h"}]},
        {"uuid":"i2","title":"Visa","category":"creditcard","template_type":"creditcard.default","fields":[{"label":"Cardholder","type":"ccName","value":"A B"},{"label":"Number","type":"ccNumber","value":"4111111111111111"},{"label":"CVC","type":"ccCvc","value":"123"},{"label":"Expiry","type":"ccExpiry","value":"09/29"}]},
        {"uuid":"i3","title":"Memo","category":"note","template_type":"note.default","note":"text"},
        {"uuid":"i4","title":"Me","category":"identity","template_type":"identity.default","fields":[{"label":"First name","type":"text","value":"Ana"},{"label":"Last name","type":"text","value":"Lopez"},{"label":"Email","type":"email","value":"a@example.com"}]},
        {"uuid":"i5","title":"Old","category":"login","trashed":1,"fields":[{"label":"Password","type":"password","value":"x"}]}
      ]}"#;

    #[test]
    fn enpass_items() {
        let p = parse_enpass(ENPASS.as_bytes()).unwrap();
        assert_eq!(p.entries.len(), 4);
        assert_eq!(p.skipped, 1);
        let a = &p.entries[0];
        assert_eq!((a.username.as_str(), a.password.as_str()), ("me", "pw"));
        assert!(a.favorite);
        assert_eq!(a.folder.as_deref(), Some("Work"));
        assert_eq!(a.tags, vec!["Servers"]);
        assert_eq!(a.created, Some(1_700_000_000_000));
        assert_eq!(a.custom_fields.len(), 1);
        assert!(a.custom_fields[0].hidden);
        assert_eq!(a.attachments[0].bytes, b"Hello!");
        let card = p.entries[1].to_draft().draft;
        assert_eq!(card.fields["expMonth"], "09");
        assert_eq!(card.fields["cardholder"], "A B");
        assert_eq!(p.entries[2].kind, EntryKind::Note);
        let id = p.entries[3].to_draft().draft;
        assert_eq!(id.fields["firstName"], "Ana");
        assert_eq!(id.fields["email"], "a@example.com");
    }

    #[test]
    fn broken_input_is_a_clean_error() {
        assert!(parse_enpass(b"{").is_err());
        assert!(parse_enpass(br#"{"items":[]}"#).is_err());
        assert!(parse_enpass(b"").is_err());
    }
}
