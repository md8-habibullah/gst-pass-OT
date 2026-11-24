//! Bitwarden JSON (plain exports only; encrypted exports are refused with a clear sentence).
//!
//! The same reader also takes the files the exporter writes ([`super::export`]), which is what
//! the round-trip test relies on.

use super::entry::{EntryKind, ImportedEntry};
use super::jsonutil::{self, lax, lax_bool, lax_i64, nullable, parse_iso, s};
use super::limits::{unsupported, MSG_ENCRYPTED_EXPORT};
use super::ParsedImport;
use crate::storage::Result;
use serde::Deserialize;
use std::collections::HashMap;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Named {
    #[serde(default, deserialize_with = "lax")]
    id: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Uri {
    #[serde(default, deserialize_with = "lax")]
    uri: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Login {
    #[serde(default, deserialize_with = "nullable")]
    uris: Vec<Uri>,
    #[serde(default, deserialize_with = "lax")]
    username: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    password: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    totp: Option<String>,
    #[serde(default, rename = "fido2Credentials", deserialize_with = "nullable")]
    #[zeroize(skip)]
    passkeys: Vec<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Card {
    #[serde(default, rename = "cardholderName", deserialize_with = "lax")]
    holder: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    brand: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    number: Option<String>,
    #[serde(default, rename = "expMonth", deserialize_with = "lax")]
    exp_month: Option<String>,
    #[serde(default, rename = "expYear", deserialize_with = "lax")]
    exp_year: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    code: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Identity {
    #[serde(default, deserialize_with = "lax")]
    title: Option<String>,
    #[serde(default, rename = "firstName", deserialize_with = "lax")]
    first: Option<String>,
    #[serde(default, rename = "middleName", deserialize_with = "lax")]
    middle: Option<String>,
    #[serde(default, rename = "lastName", deserialize_with = "lax")]
    last: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    address1: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    address2: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    address3: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    city: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    state: Option<String>,
    #[serde(default, rename = "postalCode", deserialize_with = "lax")]
    postal: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    country: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    company: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    email: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    phone: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    ssn: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    username: Option<String>,
    #[serde(default, rename = "passportNumber", deserialize_with = "lax")]
    passport: Option<String>,
    #[serde(default, rename = "licenseNumber", deserialize_with = "lax")]
    license: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct SshKey {
    #[serde(default, rename = "privateKey", deserialize_with = "lax")]
    private: Option<String>,
    #[serde(default, rename = "publicKey", deserialize_with = "lax")]
    public: Option<String>,
    #[serde(default, rename = "keyFingerprint", deserialize_with = "lax")]
    fingerprint: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Field {
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    value: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    kind: Option<i64>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct Item {
    #[serde(default, deserialize_with = "lax")]
    id: Option<String>,
    #[serde(default, rename = "folderId", deserialize_with = "lax")]
    folder_id: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "lax_i64")]
    #[zeroize(skip)]
    kind: Option<i64>,
    #[serde(default, deserialize_with = "lax")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lax")]
    notes: Option<String>,
    #[serde(default, deserialize_with = "lax_bool")]
    #[zeroize(skip)]
    favorite: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    fields: Vec<Field>,
    login: Option<Login>,
    card: Option<Card>,
    identity: Option<Identity>,
    #[serde(default, rename = "sshKey")]
    ssh_key: Option<SshKey>,
    #[serde(default, rename = "collectionIds", deserialize_with = "nullable")]
    collection_ids: Vec<Option<String>>,
    #[serde(default, rename = "creationDate", deserialize_with = "lax")]
    created: Option<String>,
    #[serde(default, rename = "revisionDate", deserialize_with = "lax")]
    revised: Option<String>,
    #[serde(default, rename = "deletedDate", deserialize_with = "lax")]
    deleted: Option<String>,
}

#[derive(Deserialize)]
struct Doc {
    #[serde(default, deserialize_with = "lax_bool")]
    encrypted: Option<bool>,
    #[serde(
        default,
        rename = "encKeyValidation_DO_NOT_EDIT",
        deserialize_with = "lax"
    )]
    enc_key: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    folders: Vec<Named>,
    #[serde(default, deserialize_with = "nullable")]
    collections: Vec<Named>,
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<Item>,
}

fn time(v: &Option<String>) -> Option<u64> {
    parse_iso(s(v))
}

/// Bitwarden nests folders with `/` inside one name.
fn set_folder(e: &mut ImportedEntry, name: &str) {
    let parts: Vec<&str> = name.split('/').collect();
    e.set_folder_path(&parts);
}

/// Turn the custom fields Passman's own exporter adds back into what they stand for: tags, the
/// code item type, a card PIN and an SSH key passphrase. Everything else is left alone.
pub(super) fn apply_passman_extras(e: &mut ImportedEntry) {
    let mut keep = Vec::new();
    let mut code_item = false;
    for f in std::mem::take(&mut e.custom_fields) {
        match (f.name.as_str(), e.kind) {
            (super::export::TAGS_FIELD, _) => match serde_json::from_str::<Vec<String>>(&f.value) {
                Ok(tags) => {
                    for t in tags {
                        e.add_tag(&t);
                    }
                }
                Err(_) => keep.push(f),
            },
            (super::export::TYPE_FIELD, EntryKind::Login) if f.value == "totp" => code_item = true,
            ("PIN", EntryKind::Card) if f.hidden => e.put("pin", &f.value),
            ("Passphrase", EntryKind::SshKey) if f.hidden => e.put("passphrase", &f.value),
            _ => keep.push(f),
        }
    }
    e.custom_fields = keep;
    if code_item {
        e.kind = EntryKind::Totp;
        if !e.username.is_empty() {
            let account = std::mem::take(&mut e.username);
            e.put("account", &account);
        }
        if let Some(i) = e.custom_fields.iter().position(|f| f.name == "Issuer") {
            let f = e.custom_fields.remove(i);
            e.put("issuer", &f.value);
        }
    }
}

fn convert(
    item: &Item,
    folders: &HashMap<&str, &str>,
    collections: &HashMap<&str, &str>,
    out: &mut ParsedImport,
) -> Result<()> {
    if item.deleted.is_some() {
        out.skip("it was in the Bitwarden trash");
        return Ok(());
    }
    let kind = match item.kind {
        Some(1) => EntryKind::Login,
        Some(2) => EntryKind::Note,
        Some(3) => EntryKind::Card,
        Some(4) => EntryKind::Identity,
        Some(5) => EntryKind::SshKey,
        None if item.login.is_some() => EntryKind::Login,
        None if item.card.is_some() => EntryKind::Card,
        None if item.identity.is_some() => EntryKind::Identity,
        None if item.ssh_key.is_some() => EntryKind::SshKey,
        None => EntryKind::Note,
        Some(_) => {
            out.skip("it has an item type Passman does not know");
            return Ok(());
        }
    };
    let mut e = ImportedEntry::new(kind, s(&item.name));
    e.favorite = item.favorite.unwrap_or(false);
    e.source_id = item.id.clone();
    e.created = time(&item.created);
    e.modified = time(&item.revised);
    if let Some(name) = item.folder_id.as_deref().and_then(|id| folders.get(id)) {
        set_folder(&mut e, name);
    }
    for id in item.collection_ids.iter().flatten() {
        if let Some(name) = collections.get(id.as_str()) {
            e.add_tag(name);
        }
    }
    e.set_notes(s(&item.notes));

    match kind {
        EntryKind::Login => {
            if let Some(l) = &item.login {
                e.username = super::limits::own_capped(s(&l.username));
                e.password = super::limits::own_capped(s(&l.password));
                for u in &l.uris {
                    e.add_url(s(&u.uri));
                }
                e.add_totp(s(&l.totp));
                if !l.passkeys.is_empty() {
                    out.warn(format!(
                        "\u{201c}{}\u{201d} has a passkey, which Passman cannot keep. Its other data was imported.",
                        super::limits::warn_title(s(&item.name))
                    ));
                }
            }
        }
        EntryKind::Card => {
            if let Some(c) = &item.card {
                e.put("cardholder", s(&c.holder));
                e.put("brand", s(&c.brand));
                e.put("number", s(&c.number));
                e.put("expMonth", s(&c.exp_month));
                e.put("expYear", s(&c.exp_year));
                e.put("code", s(&c.code));
            }
        }
        EntryKind::Identity => {
            if let Some(i) = &item.identity {
                e.put("salutation", s(&i.title));
                e.put("firstName", s(&i.first));
                e.put("middleName", s(&i.middle));
                e.put("lastName", s(&i.last));
                e.put("email", s(&i.email));
                e.put("phone", s(&i.phone));
                e.put("company", s(&i.company));
                e.put("address1", s(&i.address1));
                e.put("address2", s(&i.address2));
                e.put("city", s(&i.city));
                e.put("state", s(&i.state));
                e.put("postalCode", s(&i.postal));
                e.put("country", s(&i.country));
                e.put("nationalId", s(&i.ssn));
                e.put("passport", s(&i.passport));
                e.put("license", s(&i.license));
                if !s(&i.address3).trim().is_empty() {
                    e.add_custom("Address line 3", s(&i.address3), false);
                }
                if !s(&i.username).trim().is_empty() {
                    e.add_custom("Username", s(&i.username), false);
                }
            }
        }
        EntryKind::SshKey => {
            if let Some(k) = &item.ssh_key {
                e.put("privateKey", s(&k.private));
                e.put("publicKey", s(&k.public));
                e.put("fingerprint", s(&k.fingerprint));
            }
        }
        EntryKind::Note | EntryKind::Totp => {}
    }
    for f in &item.fields {
        match f.kind {
            // 3 = linked to another field of the item: nothing to keep.
            Some(3) => {}
            kind => e.add_custom(s(&f.name), s(&f.value), kind == Some(1)),
        }
    }
    apply_passman_extras(&mut e);
    if e.is_blank() {
        out.skip("it is empty");
        return Ok(());
    }
    out.push(e)
}

pub fn parse_json(bytes: &[u8]) -> Result<ParsedImport> {
    let doc: Doc = jsonutil::parse(bytes)?;
    if doc.encrypted == Some(true) || doc.enc_key.is_some() {
        return Err(unsupported(MSG_ENCRYPTED_EXPORT));
    }
    let folders: HashMap<&str, &str> = doc
        .folders
        .iter()
        .filter_map(|f| Some((f.id.as_deref()?, f.name.as_deref()?)))
        .collect();
    let collections: HashMap<&str, &str> = doc
        .collections
        .iter()
        .filter_map(|f| Some((f.id.as_deref()?, f.name.as_deref()?)))
        .collect();
    let mut out = ParsedImport::default();
    for item in &doc.items {
        convert(item, &folders, &collections, &mut out)?;
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(super::limits::MSG_NO_ROWS));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::model::ItemType;

    const SAMPLE: &str = r#"{
      "encrypted": false,
      "folders": [{"id": "f1", "name": "Work/Servers"}, {"id": "f2", "name": "Home"}],
      "collections": [{"id": "c1", "name": "Shared"}],
      "items": [
        {"id":"i1","folderId":"f1","type":1,"name":"Box","notes":"n","favorite":true,
         "fields":[{"name":"pin","value":"1234","type":1},{"name":"label","value":"x","type":0},{"name":"link","value":null,"type":3}],
         "login":{"uris":[{"match":null,"uri":"https://a.example"},{"uri":"https://b.example"}],"username":"u","password":"p","totp":"JBSWY3DPEHPK3PXP","fido2Credentials":[{"x":1}]},
         "collectionIds":["c1"],"creationDate":"2024-03-05T10:20:30.000Z","revisionDate":"2024-03-06T00:00:00Z"},
        {"id":"i2","folderId":null,"type":2,"name":"Memo","notes":"remember","secureNote":{"type":0}},
        {"id":"i3","type":3,"name":"Visa","card":{"cardholderName":"A B","brand":"Visa","number":"4111111111111111","expMonth":"9","expYear":"2029","code":"123"}},
        {"id":"i4","type":4,"name":"Me","identity":{"title":"Dr","firstName":"Ana","lastName":"Lopez","email":"a@example.com","ssn":"123","address3":"c/o","username":"ana"}},
        {"id":"i5","type":5,"name":"Key","sshKey":{"privateKey":"-----BEGIN-----","publicKey":"ssh-ed25519 AAAA","keyFingerprint":"SHA256:xyz"}},
        {"id":"i6","type":1,"name":"Gone","login":{"username":"x","password":"y"},"deletedDate":"2024-01-01T00:00:00Z"},
        {"id":"i7","type":9,"name":"Odd"}
      ]
    }"#;

    #[test]
    fn parses_every_item_type() {
        let mut p = parse_json(SAMPLE.as_bytes()).unwrap();
        p.close();
        assert_eq!(p.entries.len(), 5);
        assert_eq!(p.skipped, 2);
        let a = &p.entries[0];
        assert_eq!(
            (a.title.as_str(), a.username.as_str(), a.password.as_str()),
            ("Box", "u", "p")
        );
        assert_eq!(a.folder.as_deref(), Some("Work"));
        assert_eq!(a.tags, vec!["Servers", "Shared"]);
        assert!(a.favorite);
        assert_eq!(a.urls.len(), 2);
        assert_eq!(a.custom_fields.len(), 2);
        assert!(a.custom_fields[0].hidden && !a.custom_fields[1].hidden);
        assert_eq!(a.created, Some(1_709_634_030_000));
        assert_eq!(p.entries[1].kind, EntryKind::Note);
        let card = p.entries[2].to_draft().draft;
        assert_eq!(card.item_type, ItemType::Card);
        assert_eq!(card.fields["expMonth"], "09");
        assert_eq!(card.fields["number"], "4111111111111111");
        let id = p.entries[3].to_draft().draft;
        assert_eq!(id.fields["salutation"], "Dr");
        assert_eq!(id.fields["nationalId"], "123");
        assert!(id.custom_fields.iter().any(|f| f.name == "Address line 3"));
        let ssh = p.entries[4].to_draft().draft;
        assert_eq!(ssh.item_type, ItemType::SshKey);
        assert_eq!(ssh.fields["fingerprint"], "SHA256:xyz");
        assert!(p.warnings.iter().any(|w| w.contains("passkey")));
        assert!(p.warnings.iter().any(|w| w.contains("trash")));
        for e in &p.entries {
            let mut d = e.to_draft().draft;
            d.normalize().unwrap();
        }
    }

    /// The shape the Bitwarden web vault writes (values made up): `reprompt`, empty
    /// `passwordHistory`, `collectionIds: null`, a folder with no items, a code key that is not Base32.
    #[test]
    fn web_vault_export_with_a_card_a_login_and_a_note() {
        let json = r#"{
          "encrypted": false,
          "folders": [{"name": "_Folder 1", "id": "f0"}, {"name": "Keys", "id": "f1"}],
          "items": [
            {"type":3,"favorite":false,"reprompt":1,"id":"a","collectionIds":null,"folderId":"f1","name":"card_1",
             "notes":"card note","fields":[],"card":{"cardholderName":"Jo Doe","brand":"Visa","number":"4111111111111111","expMonth":"1","expYear":"2030","code":"123"},
             "passwordHistory":[],"creationDate":"2026-11-24T01:58:57.288Z","revisionDate":"2026-11-24T01:58:57.288Z"},
            {"type":1,"favorite":false,"reprompt":0,"id":"b","collectionIds":null,"folderId":"f1","name":"Site","notes":"login note","fields":[],
             "login":{"uris":[{"uri":"https://www.example.com/"}],"fido2Credentials":[],"username":"demo@example.org","password":"pw-1","totp":"not.a@base32-key1"},
             "passwordHistory":[],"creationDate":"2026-11-24T01:57:45.770Z","revisionDate":"2026-11-24T01:57:45.770Z"},
            {"type":2,"favorite":false,"reprompt":1,"id":"c","collectionIds":null,"name":"Memo","notes":"memo body","fields":[],
             "secureNote":{"type":0},"passwordHistory":[],"creationDate":"2026-11-24T01:59:23.073Z","revisionDate":"2026-11-24T01:59:23.073Z"}
          ]
        }"#;
        let p = parse_json(json.as_bytes()).unwrap();
        assert_eq!((p.entries.len(), p.skipped), (3, 0));
        let card = p.entries[0].to_draft().draft;
        assert_eq!(card.item_type, ItemType::Card);
        assert_eq!(card.folder.as_deref(), Some("Keys"));
        assert_eq!(card.fields["expMonth"], "01");
        assert_eq!(card.fields["cardholder"], "Jo Doe");
        assert_eq!(card.fields["notes"], "card note");
        let login = p.entries[1].to_draft();
        assert_eq!(login.draft.item_type, ItemType::Login);
        assert_eq!(login.draft.fields["url"], "https://www.example.com/");
        assert_eq!(login.draft.fields["username"], "demo@example.org");
        // A key that cannot make codes is not dropped: it stays in a hidden field, with a note.
        assert!(!login.draft.fields.contains_key("totp"));
        assert!(login
            .draft
            .custom_fields
            .iter()
            .any(|f| f.hidden && f.value == "not.a@base32-key1"));
        assert!(login.warnings.iter().any(|w| w.contains("one-time code")));
        let note = p.entries[2].to_draft().draft;
        assert_eq!(note.item_type, ItemType::SecureNote);
        assert_eq!(note.fields["body"], "memo body");
        assert_eq!(p.entries[0].created, Some(1_795_485_537_288));
    }

    #[test]
    fn encrypted_exports_are_refused_with_the_fixed_sentence() {
        for body in [
            r#"{"encrypted": true, "items": []}"#,
            r#"{"encrypted": false, "encKeyValidation_DO_NOT_EDIT": "2.x|y", "items": []}"#,
        ] {
            let e = parse_json(body.as_bytes()).err().unwrap();
            assert_eq!(e.code(), "unsupported_file");
            assert!(e.to_string().starts_with("This export is encrypted."));
        }
    }

    #[test]
    fn tolerates_missing_and_null_parts() {
        let p = parse_json(
            br#"{"items":[{"name":"x","login":null,"fields":null,"collectionIds":null}]}"#,
        )
        .unwrap();
        assert_eq!(p.entries.len(), 1);
        assert!(parse_json(br#"{"items":[]}"#).is_err());
        assert!(parse_json(b"[]").is_err());
        assert!(parse_json(b"not json").is_err());
    }
}
