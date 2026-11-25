//! Plaintext exports in Bitwarden's formats (JSON with `encrypted: false`, and CSV).
//!
//! **These files are not encrypted.** Everything in them (passwords, card numbers, one-time
//! code secrets) is readable by anyone who gets the file. The commands behind this module ask for
//! the master password first, write the file privately (mode 0600, temp file + rename), and say
//! so in their messages. The buffers built here are wiped on drop.
//!
//! Hexis keeps a few things Bitwarden has no place for. They are written as custom fields with
//! fixed names ([`TAGS_FIELD`], [`TYPE_FIELD`]) and the importer turns them back into tags and
//! item types, so an export read by Hexis again is lossless; another app simply shows them as
//! ordinary extra fields.

use super::jsonutil::format_iso;
use crate::storage::model::{urls_of, CustomField, ItemDraft, ItemType};
use crate::storage::{uuid, Result, StorageError};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use zeroize::Zeroize;

/// Custom field that carries Hexis's tags (a JSON list of names).
pub const TAGS_FIELD: &str = "Hexis tags";
/// Custom field that marks an item Bitwarden cannot represent (`totp`).
pub const TYPE_FIELD: &str = "Hexis item type";
/// The same two fields as exports written by builds made before the final name call them; the importer still reads them,
/// so an old export brings its tags and code items back.
pub const PREVIOUS_TAGS_FIELD: &str = "Passman tags";
pub const PREVIOUS_TYPE_FIELD: &str = "Passman item type";

/// The two plaintext export formats of `export_plain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlainFormat {
    BitwardenJson,
    Csv,
}

impl PlainFormat {
    /// The wire ids: `"bitwarden-json"` and `"csv"`.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "bitwarden-json" => Some(Self::BitwardenJson),
            "csv" => Some(Self::Csv),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::BitwardenJson => "json",
            Self::Csv => "csv",
        }
    }

    pub fn render(self, records: &[ExportRecord]) -> Result<SecretBuf> {
        match self {
            Self::BitwardenJson => bitwarden_json(records),
            Self::Csv => bitwarden_csv(records),
        }
    }
}

/// One item to export: its content (secrets included) and times.
pub struct ExportRecord {
    pub id: String,
    pub draft: ItemDraft,
    pub created_at: u64,
    pub updated_at: u64,
}

/// A growing byte buffer that wipes every block it ever held.
#[derive(Default)]
pub struct SecretBuf(Vec<u8>);

impl SecretBuf {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Write for SecretBuf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.0.capacity() - self.0.len() < data.len() {
            let want = (self.0.capacity() * 2)
                .max(self.0.len() + data.len())
                .max(4096);
            let mut bigger = Vec::with_capacity(want);
            bigger.extend_from_slice(&self.0);
            self.0.zeroize();
            self.0 = bigger;
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for SecretBuf {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

fn opt(s: &str) -> Option<&str> {
    (!s.trim().is_empty()).then_some(s)
}

fn f<'a>(d: &'a ItemDraft, key: &str) -> &'a str {
    d.fields.get(key).map_or("", String::as_str)
}

fn tags_value(tags: &[String]) -> String {
    serde_json::to_string(tags).unwrap_or_default()
}

// ------------------------------------------------------------------ JSON

#[derive(Serialize)]
struct BwFolder<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct BwField<'a> {
    name: &'a str,
    value: &'a str,
    #[serde(rename = "type")]
    kind: u8,
    #[serde(rename = "linkedId")]
    linked_id: Option<()>,
}

#[derive(Serialize)]
struct BwUri<'a> {
    #[serde(rename = "match")]
    matching: Option<()>,
    uri: &'a str,
}

#[derive(Serialize)]
struct BwLogin<'a> {
    uris: Vec<BwUri<'a>>,
    username: Option<&'a str>,
    password: Option<&'a str>,
    totp: Option<&'a str>,
}

#[derive(Serialize)]
struct BwNote {
    #[serde(rename = "type")]
    kind: u8,
}

#[derive(Serialize)]
struct BwCard<'a> {
    #[serde(rename = "cardholderName")]
    holder: Option<&'a str>,
    brand: Option<&'a str>,
    number: Option<&'a str>,
    #[serde(rename = "expMonth")]
    exp_month: Option<&'a str>,
    #[serde(rename = "expYear")]
    exp_year: Option<&'a str>,
    code: Option<&'a str>,
}

#[derive(Serialize)]
struct BwIdentity<'a> {
    title: Option<&'a str>,
    #[serde(rename = "firstName")]
    first: Option<&'a str>,
    #[serde(rename = "middleName")]
    middle: Option<&'a str>,
    #[serde(rename = "lastName")]
    last: Option<&'a str>,
    address1: Option<&'a str>,
    address2: Option<&'a str>,
    address3: Option<&'a str>,
    city: Option<&'a str>,
    state: Option<&'a str>,
    #[serde(rename = "postalCode")]
    postal: Option<&'a str>,
    country: Option<&'a str>,
    company: Option<&'a str>,
    email: Option<&'a str>,
    phone: Option<&'a str>,
    ssn: Option<&'a str>,
    username: Option<&'a str>,
    #[serde(rename = "passportNumber")]
    passport: Option<&'a str>,
    #[serde(rename = "licenseNumber")]
    license: Option<&'a str>,
}

#[derive(Serialize)]
struct BwSsh<'a> {
    #[serde(rename = "privateKey")]
    private: Option<&'a str>,
    #[serde(rename = "publicKey")]
    public: Option<&'a str>,
    #[serde(rename = "keyFingerprint")]
    fingerprint: Option<&'a str>,
}

#[derive(Serialize)]
struct BwItem<'a> {
    id: &'a str,
    #[serde(rename = "organizationId")]
    organization_id: Option<()>,
    #[serde(rename = "folderId")]
    folder_id: Option<&'a str>,
    #[serde(rename = "type")]
    kind: u8,
    reprompt: u8,
    name: &'a str,
    notes: Option<&'a str>,
    favorite: bool,
    fields: Vec<BwField<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<BwLogin<'a>>,
    #[serde(rename = "secureNote", skip_serializing_if = "Option::is_none")]
    secure_note: Option<BwNote>,
    #[serde(skip_serializing_if = "Option::is_none")]
    card: Option<BwCard<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<BwIdentity<'a>>,
    #[serde(rename = "sshKey", skip_serializing_if = "Option::is_none")]
    ssh_key: Option<BwSsh<'a>>,
    #[serde(rename = "collectionIds")]
    collection_ids: Option<()>,
    #[serde(rename = "revisionDate")]
    revision_date: String,
    #[serde(rename = "creationDate")]
    creation_date: String,
    #[serde(rename = "deletedDate")]
    deleted_date: Option<()>,
}

#[derive(Serialize)]
struct BwDoc<'a> {
    encrypted: bool,
    folders: Vec<BwFolder<'a>>,
    items: Vec<BwItem<'a>>,
}

/// Extra custom fields Hexis adds (kept alive until the document is written).
struct Extras {
    tags: String,
}

fn map_error(_: serde_json::Error) -> StorageError {
    StorageError::Storage("The export file could not be built.".into())
}

/// Bitwarden JSON (`encrypted: false`) with folders, logins, notes, cards, identities and SSH keys.
pub fn bitwarden_json(records: &[ExportRecord]) -> Result<SecretBuf> {
    // folder name -> generated id
    let mut folder_ids: BTreeMap<&str, String> = BTreeMap::new();
    for r in records {
        if let Some(name) = r.draft.folder.as_deref().filter(|n| !n.trim().is_empty()) {
            if !folder_ids.contains_key(name) {
                folder_ids.insert(name, uuid::new_v4()?);
            }
        }
    }
    let extras: Vec<Extras> = records
        .iter()
        .map(|r| Extras {
            tags: if r.draft.tags.is_empty() {
                String::new()
            } else {
                tags_value(&r.draft.tags)
            },
        })
        .collect();
    let mut items = Vec::with_capacity(records.len());
    for (r, extra) in records.iter().zip(&extras) {
        items.push(bw_item(r, extra, &folder_ids));
    }
    let doc = BwDoc {
        encrypted: false,
        folders: folder_ids
            .iter()
            .map(|(name, id)| BwFolder { id, name })
            .collect(),
        items,
    };
    let mut buf = SecretBuf::default();
    serde_json::to_writer_pretty(&mut buf, &doc).map_err(map_error)?;
    buf.write_all(b"\n")
        .map_err(|_| StorageError::Storage("The export file could not be built.".into()))?;
    Ok(buf)
}

fn custom<'a>(c: &'a CustomField) -> BwField<'a> {
    BwField {
        name: &c.name,
        value: &c.value,
        kind: u8::from(c.hidden),
        linked_id: None,
    }
}

fn bw_item<'a>(
    r: &'a ExportRecord,
    extra: &'a Extras,
    folders: &'a BTreeMap<&str, String>,
) -> BwItem<'a> {
    let d = &r.draft;
    let mut fields: Vec<BwField<'a>> = Vec::new();
    let mut item = BwItem {
        id: &r.id,
        organization_id: None,
        folder_id: d
            .folder
            .as_deref()
            .and_then(|n| folders.get(n))
            .map(String::as_str),
        kind: 1,
        reprompt: u8::from(d.reprompt),
        name: &d.title,
        notes: None,
        favorite: d.favorite,
        fields: Vec::new(),
        login: None,
        secure_note: None,
        card: None,
        identity: None,
        ssh_key: None,
        collection_ids: None,
        revision_date: format_iso(r.updated_at),
        creation_date: format_iso(r.created_at.min(r.updated_at)),
        deleted_date: None,
    };
    match d.item_type {
        ItemType::Login => {
            item.notes = opt(f(d, "notes"));
            item.login = Some(BwLogin {
                uris: urls_of(f(d, "url"))
                    .map(|u| BwUri {
                        matching: None,
                        uri: u,
                    })
                    .collect(),
                username: opt(f(d, "username")),
                password: opt(f(d, "password")),
                totp: opt(f(d, "totp")),
            });
        }
        ItemType::Totp => {
            item.login = Some(BwLogin {
                uris: Vec::new(),
                username: opt(f(d, "account")),
                password: None,
                totp: opt(f(d, "totp")),
            });
            fields.push(BwField {
                name: TYPE_FIELD,
                value: "totp",
                kind: 0,
                linked_id: None,
            });
            if let Some(i) = opt(f(d, "issuer")) {
                fields.push(BwField {
                    name: "Issuer",
                    value: i,
                    kind: 0,
                    linked_id: None,
                });
            }
        }
        ItemType::SecureNote | ItemType::Media => {
            item.kind = 2;
            item.secure_note = Some(BwNote { kind: 0 });
            item.notes = opt(f(
                d,
                if d.item_type == ItemType::Media {
                    "description"
                } else {
                    "body"
                },
            ));
        }
        ItemType::Card => {
            item.kind = 3;
            item.notes = opt(f(d, "notes"));
            item.card = Some(BwCard {
                holder: opt(f(d, "cardholder")),
                brand: opt(f(d, "brand")),
                number: opt(f(d, "number")),
                exp_month: opt(f(d, "expMonth")),
                exp_year: opt(f(d, "expYear")),
                code: opt(f(d, "code")),
            });
            if let Some(p) = opt(f(d, "pin")) {
                fields.push(BwField {
                    name: "PIN",
                    value: p,
                    kind: 1,
                    linked_id: None,
                });
            }
        }
        ItemType::Identity => {
            item.kind = 4;
            item.notes = opt(f(d, "notes"));
            item.identity = Some(BwIdentity {
                title: opt(f(d, "salutation")),
                first: opt(f(d, "firstName")),
                middle: opt(f(d, "middleName")),
                last: opt(f(d, "lastName")),
                address1: opt(f(d, "address1")),
                address2: opt(f(d, "address2")),
                address3: None,
                city: opt(f(d, "city")),
                state: opt(f(d, "state")),
                postal: opt(f(d, "postalCode")),
                country: opt(f(d, "country")),
                company: opt(f(d, "company")),
                email: opt(f(d, "email")),
                phone: opt(f(d, "phone")),
                ssn: opt(f(d, "nationalId")),
                username: None,
                passport: opt(f(d, "passport")),
                license: opt(f(d, "license")),
            });
        }
        ItemType::SshKey => {
            item.kind = 5;
            item.notes = opt(f(d, "notes"));
            item.ssh_key = Some(BwSsh {
                private: opt(f(d, "privateKey")),
                public: opt(f(d, "publicKey")),
                fingerprint: opt(f(d, "fingerprint")),
            });
            if let Some(p) = opt(f(d, "passphrase")) {
                fields.push(BwField {
                    name: "Passphrase",
                    value: p,
                    kind: 1,
                    linked_id: None,
                });
            }
        }
    }
    fields.extend(d.custom_fields.iter().map(custom));
    if !extra.tags.is_empty() {
        fields.push(BwField {
            name: TAGS_FIELD,
            value: &extra.tags,
            kind: 0,
            linked_id: None,
        });
    }
    item.fields = fields;
    item
}

// ------------------------------------------------------------------ CSV

const CSV_HEADER: [&str; 11] = [
    "folder",
    "favorite",
    "type",
    "name",
    "notes",
    "fields",
    "reprompt",
    "login_uri",
    "login_username",
    "login_password",
    "login_totp",
];

/// The cell text for the `fields` column: one `name: value` per line.
fn fields_cell(extra: &[(String, String)]) -> String {
    extra
        .iter()
        .map(|(n, v)| format!("{n}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Bitwarden CSV. The format only has logins and notes, so cards, identities, SSH keys and the
/// like are written as notes with one `Label: value` per line.
pub fn bitwarden_csv(records: &[ExportRecord]) -> Result<SecretBuf> {
    let mut buf = SecretBuf::default();
    {
        let mut w = csv::WriterBuilder::new()
            .has_headers(false)
            .flexible(false)
            .from_writer(&mut buf);
        let fail = |_| StorageError::Storage("The export file could not be built.".into());
        w.write_record(CSV_HEADER).map_err(fail)?;
        for r in records {
            let d = &r.draft;
            let mut extra: Vec<(String, String)> = Vec::new();
            let (kind, notes, uri, user, pass, totp) = match d.item_type {
                ItemType::Login => (
                    "login",
                    f(d, "notes").to_string(),
                    f(d, "url").to_string(),
                    f(d, "username").to_string(),
                    f(d, "password").to_string(),
                    f(d, "totp").to_string(),
                ),
                ItemType::Totp => {
                    extra.push((TYPE_FIELD.into(), "totp".into()));
                    if !f(d, "issuer").is_empty() {
                        extra.push(("Issuer".into(), f(d, "issuer").to_string()));
                    }
                    (
                        "login",
                        String::new(),
                        String::new(),
                        f(d, "account").to_string(),
                        String::new(),
                        f(d, "totp").to_string(),
                    )
                }
                ItemType::SecureNote => (
                    "note",
                    f(d, "body").to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                ItemType::Media => (
                    "note",
                    f(d, "description").to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                other => {
                    // Card, identity, SSH key: a note listing every value with its label.
                    let mut lines = Vec::new();
                    for spec in other.schema() {
                        let v = f(d, spec.key);
                        if !v.is_empty() {
                            lines.push(format!("{}: {v}", spec.label));
                        }
                    }
                    (
                        "note",
                        lines.join("\n"),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                    )
                }
            };
            for c in &d.custom_fields {
                extra.push((c.name.clone(), c.value.clone()));
            }
            if !d.tags.is_empty() {
                extra.push((TAGS_FIELD.into(), tags_value(&d.tags)));
            }
            let cell = fields_cell(&extra);
            w.write_record([
                d.folder.as_deref().unwrap_or(""),
                if d.favorite { "1" } else { "" },
                kind,
                &d.title,
                &notes,
                &cell,
                if d.reprompt { "1" } else { "0" },
                &uri,
                &user,
                &pass,
                &totp,
            ])
            .map_err(fail)?;
        }
        w.flush()
            .map_err(|_| StorageError::Storage("The export file could not be built.".into()))?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::{bitwarden, csv_import, ImportFormat};

    fn draft(item_type: ItemType, title: &str, fields: &[(&str, &str)]) -> ItemDraft {
        ItemDraft {
            item_type,
            title: title.into(),
            favorite: false,
            reprompt: false,
            tags: Vec::new(),
            folder: None,
            fields: fields
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            custom_fields: Vec::new(),
        }
    }

    fn records() -> Vec<ExportRecord> {
        let mut login = draft(
            ItemType::Login,
            "GitHub",
            &[
                ("username", "ana"),
                ("password", "p\"w, with\nnewline"),
                ("url", "https://github.com/login"),
                (
                    "totp",
                    "otpauth://totp/GitHub:ana?secret=JBSWY3DPEHPK3PXP&issuer=GitHub",
                ),
                ("notes", "line one\nline two"),
            ],
        );
        login.favorite = true;
        login.folder = Some("Work".into());
        login.tags = vec!["dev".into(), "2fa".into()];
        login.custom_fields = vec![
            CustomField {
                name: "pin".into(),
                value: "1234".into(),
                hidden: true,
            },
            CustomField {
                name: "label".into(),
                value: "plain".into(),
                hidden: false,
            },
        ];
        let card = draft(
            ItemType::Card,
            "Visa",
            &[
                ("cardholder", "A B"),
                ("brand", "Visa"),
                ("number", "4111 1111 1111 1111"),
                ("expMonth", "09"),
                ("expYear", "2029"),
                ("code", "123"),
                ("pin", "4321"),
                ("notes", "card note"),
            ],
        );
        let identity = draft(
            ItemType::Identity,
            "Me",
            &[
                ("salutation", "Dr"),
                ("firstName", "Ana"),
                ("lastName", "Lopez"),
                ("email", "a@example.com"),
                ("address1", "1 Road"),
                ("city", "Town"),
                ("nationalId", "123-45"),
                ("notes", "n"),
            ],
        );
        let note = draft(ItemType::SecureNote, "Memo", &[("body", "remember this")]);
        let ssh = draft(
            ItemType::SshKey,
            "Key",
            &[
                ("publicKey", "ssh-ed25519 AAAA"),
                ("privateKey", "-----BEGIN-----\nabc\n-----END-----"),
                ("passphrase", "open sesame"),
                ("fingerprint", "SHA256:xyz"),
            ],
        );
        let mut code = draft(
            ItemType::Totp,
            "Bank 2FA",
            &[
                ("issuer", "Bank"),
                ("account", "me"),
                ("totp", "JBSWY3DPEHPK3PXP"),
            ],
        );
        code.folder = Some("Home".into());
        [login, card, identity, note, ssh, code]
            .into_iter()
            .enumerate()
            .map(|(i, draft)| ExportRecord {
                id: uuid::new_v4().unwrap(),
                draft,
                created_at: 1_700_000_000_000 + i as u64,
                updated_at: 1_700_000_100_000 + i as u64,
            })
            .collect()
    }

    #[test]
    fn json_is_plain_and_round_trips_through_the_importer() {
        let recs = records();
        let out = bitwarden_json(&recs).unwrap();
        let text = std::str::from_utf8(out.as_slice()).unwrap();
        assert!(text.contains("\"encrypted\": false"));
        let back = bitwarden::parse_json(out.as_slice()).unwrap();
        assert_eq!(back.entries.len(), recs.len());
        for (rec, e) in recs.iter().zip(&back.entries) {
            let c = e.to_draft();
            assert_eq!(c.draft, rec.draft, "round trip of {}", rec.draft.title);
            assert_eq!(e.created, Some(rec.created_at));
            assert_eq!(e.modified, Some(rec.updated_at));
            assert_eq!(e.source_id.as_deref(), Some(rec.id.as_str()));
            assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        }
    }

    #[test]
    fn an_export_from_before_the_rename_still_brings_its_tags_and_code_items_back() {
        let recs = records();
        let out = bitwarden_json(&recs).unwrap();
        let text = std::str::from_utf8(out.as_slice())
            .unwrap()
            .replace(TAGS_FIELD, PREVIOUS_TAGS_FIELD)
            .replace(TYPE_FIELD, PREVIOUS_TYPE_FIELD);
        assert!(text.contains(PREVIOUS_TAGS_FIELD) && !text.contains(TAGS_FIELD));
        let back = bitwarden::parse_json(text.as_bytes()).unwrap();
        for (rec, e) in recs.iter().zip(&back.entries) {
            assert_eq!(
                e.to_draft().draft,
                rec.draft,
                "round trip of {}",
                rec.draft.title
            );
        }
    }

    #[test]
    fn every_web_address_of_a_login_is_exported_and_comes_back() {
        let one = draft(
            ItemType::Login,
            "Many",
            &[("url", "https://a.example\nhttps://b.example")],
        );
        let recs = vec![ExportRecord {
            draft: one,
            ..records().remove(0)
        }];
        let out = bitwarden_json(&recs).unwrap();
        let v: serde_json::Value = serde_json::from_slice(out.as_slice()).unwrap();
        let uris = v["items"][0]["login"]["uris"].as_array().unwrap();
        assert_eq!(uris.len(), 2);
        assert_eq!(uris[1]["uri"], "https://b.example");
        // a file with both addresses reads back as both
        let back = bitwarden::parse_json(out.as_slice()).unwrap();
        let d = back.entries[0].to_draft().draft;
        assert_eq!(d.fields["url"], "https://a.example\nhttps://b.example");
    }

    #[test]
    fn json_uses_bitwarden_types_and_folders() {
        let out = bitwarden_json(&records()).unwrap();
        let v: serde_json::Value = serde_json::from_slice(out.as_slice()).unwrap();
        let items = v["items"].as_array().unwrap();
        let types: Vec<i64> = items.iter().map(|i| i["type"].as_i64().unwrap()).collect();
        assert_eq!(types, [1, 3, 4, 2, 5, 1]);
        assert_eq!(
            items[0]["login"]["totp"],
            "otpauth://totp/GitHub:ana?secret=JBSWY3DPEHPK3PXP&issuer=GitHub"
        );
        assert_eq!(
            items[0]["login"]["uris"][0]["uri"],
            "https://github.com/login"
        );
        let folders = v["folders"].as_array().unwrap();
        assert_eq!(folders.len(), 2);
        let work = folders.iter().find(|f| f["name"] == "Work").unwrap();
        assert_eq!(items[0]["folderId"], work["id"]);
        assert!(items[1]["folderId"].is_null());
        assert_eq!(items[0]["favorite"], true);
        assert_eq!(items[0]["creationDate"], "2023-11-14T22:13:20.000Z");
    }

    #[test]
    fn csv_round_trips_logins_and_notes_and_flattens_the_rest() {
        let recs = records();
        let out = bitwarden_csv(&recs).unwrap();
        let text = std::str::from_utf8(out.as_slice()).unwrap();
        assert!(text.starts_with("folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n"));
        let back = csv_import::parse(ImportFormat::BitwardenCsv, out.as_slice(), None).unwrap();
        assert_eq!(back.entries.len(), recs.len());
        let login = back.entries[0].to_draft().draft;
        // the CSV format cannot say which custom fields are hidden
        let mut expected = recs[0].draft.clone();
        for c in &mut expected.custom_fields {
            c.hidden = false;
        }
        assert_eq!(login, expected);
        // cards and identities become notes listing every value
        let card = back.entries[1].to_draft().draft;
        assert_eq!(card.item_type, ItemType::SecureNote);
        let body = &card.fields["body"];
        assert!(
            body.contains("Card number: 4111 1111 1111 1111")
                && body.contains("Security code: 123")
        );
        let note = back.entries[3].to_draft().draft;
        assert_eq!(note, recs[3].draft);
        let code = back.entries[5].to_draft().draft;
        assert_eq!(code.item_type, ItemType::Totp);
        assert_eq!(code.fields["totp"], "JBSWY3DPEHPK3PXP");
        assert_eq!(code.fields["account"], "me");
        assert_eq!(code.fields["issuer"], "Bank");
    }

    #[test]
    fn empty_vault_exports_valid_files() {
        let json = bitwarden_json(&[]).unwrap();
        let v: serde_json::Value = serde_json::from_slice(json.as_slice()).unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 0);
        let csv = bitwarden_csv(&[]).unwrap();
        assert_eq!(
            std::str::from_utf8(csv.as_slice()).unwrap().lines().count(),
            1
        );
    }

    #[test]
    fn secret_buffer_grows_and_is_correct() {
        let mut b = SecretBuf::default();
        for i in 0..5000u32 {
            b.write_all(format!("line {i}\n").as_bytes()).unwrap();
        }
        assert_eq!(b.as_slice().iter().filter(|c| **c == b'\n').count(), 5000);
        assert!(!b.is_empty());
    }
}
