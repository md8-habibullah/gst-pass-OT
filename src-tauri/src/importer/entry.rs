//! One parsed record of an import file, and its conversion into an [`ItemDraft`].
//!
//! [`ImportedEntry`] is what every parser produces. It wipes all of its text when dropped and
//! never prints a value (`Debug` shows only the kind and counts). The conversion to a draft lives
//! in [`ImportedEntry::to_draft`] and is the single place that decides which Passman field a
//! source value lands in.

use super::limits::{cap_chars, warn_title};
use super::otp;
use crate::storage::model::{
    host_of, CustomField, ItemDraft, ItemType, MAX_CUSTOM_FIELDS, MAX_CUSTOM_NAME_CHARS,
    MAX_FOLDER_CHARS, MAX_TAGS, MAX_TAG_CHARS, MAX_TITLE_CHARS,
};
use std::collections::BTreeMap;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The Passman item type a record becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryKind {
    #[default]
    Login,
    Card,
    Identity,
    SshKey,
    Note,
    /// A one-time code on its own (authenticator apps).
    Totp,
}

/// A named extra value (source "custom field").
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ImportedField {
    pub name: String,
    pub value: String,
    #[zeroize(skip)]
    pub hidden: bool,
}

/// A file attached to a record. Parsed and counted; stored once the encrypted drive exists.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ImportedAttachment {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// One record read from an import file.
#[derive(Zeroize, ZeroizeOnDrop, Default)]
pub struct ImportedEntry {
    pub title: String,
    #[zeroize(skip)]
    pub kind: EntryKind,
    pub username: String,
    pub password: String,
    /// The first address is the main one; the others become custom fields.
    pub urls: Vec<String>,
    pub notes: String,
    /// Top-level folder (nested folder names go to `tags`).
    pub folder: Option<String>,
    pub tags: Vec<String>,
    /// One-time code values as found in the source (otpauth link or Base32 key).
    pub totp: Vec<String>,
    pub custom_fields: Vec<ImportedField>,
    /// Values for cards, identities, SSH keys and codes, keyed by the Passman field key
    /// (`number`, `firstName`, `privateKey`, `issuer`, ...). A card may also carry a pseudo key
    /// `expiry` ("09/2029") that is split into month and year.
    pub typed: Vec<(String, String)>,
    pub attachments: Vec<ImportedAttachment>,
    #[zeroize(skip)]
    pub favorite: bool,
    pub source_id: Option<String>,
    /// Creation and last-change time in the source (Unix milliseconds).
    #[zeroize(skip)]
    pub created: Option<u64>,
    #[zeroize(skip)]
    pub modified: Option<u64>,
}

impl std::fmt::Debug for ImportedEntry {
    /// Never prints a value, not even the title.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportedEntry")
            .field("kind", &self.kind)
            .field("urls", &self.urls.len())
            .field("totp", &self.totp.len())
            .field("custom_fields", &self.custom_fields.len())
            .field("typed", &self.typed.len())
            .field("attachments", &self.attachments.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ImportedField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportedField")
            .field("hidden", &self.hidden)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ImportedAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportedAttachment")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// The result of converting one record.
pub struct Converted {
    pub draft: ItemDraft,
    pub created: Option<u64>,
    pub warnings: Vec<String>,
}

fn non_empty(s: &str) -> Option<&str> {
    let t = s.trim();
    (!t.is_empty()).then_some(s)
}

impl ImportedEntry {
    pub fn new(kind: EntryKind, title: impl Into<String>) -> Self {
        let mut e = Self::default();
        e.kind = kind;
        e.title = title.into();
        e
    }

    /// Add a typed value (ignored when empty).
    pub fn put(&mut self, key: &str, value: &str) {
        if !value.trim().is_empty() {
            self.typed
                .push((key.to_string(), super::limits::own_capped(value)));
        }
    }

    pub fn add_custom(&mut self, name: &str, value: &str, hidden: bool) {
        if self.custom_fields.len() < super::limits::MAX_PARSED_FIELDS {
            self.custom_fields.push(ImportedField {
                name: cap_chars(name.trim(), MAX_CUSTOM_NAME_CHARS),
                value: super::limits::own_capped(value),
                hidden,
            });
        }
    }

    pub fn add_url(&mut self, url: &str) {
        let u = url.trim();
        if !u.is_empty() && !self.urls.iter().any(|x| x == u) {
            self.urls.push(super::limits::own_capped(u));
        }
    }

    pub fn add_tag(&mut self, tag: &str) {
        let t = tag.trim();
        if !t.is_empty() && !self.tags.iter().any(|x| x == t) && self.tags.len() < 4 * MAX_TAGS {
            self.tags.push(cap_chars(t, MAX_TAG_CHARS));
        }
    }

    pub fn add_totp(&mut self, value: &str) {
        let v = value.trim();
        if !v.is_empty() {
            self.totp.push(super::limits::own_capped(v));
        }
    }

    /// Set the folder from a path: the first component is the folder, the deeper ones become tags
    /// so the location is not lost.
    pub fn set_folder_path(&mut self, parts: &[&str]) {
        let mut parts = parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty());
        if let Some(first) = parts.next() {
            self.folder = Some(cap_chars(first, MAX_FOLDER_CHARS));
        }
        for p in parts {
            self.add_tag(p);
        }
    }

    pub fn set_notes(&mut self, notes: &str) {
        self.notes = super::limits::own_capped(notes);
    }

    /// Has this record anything a person would call content?
    pub fn is_blank(&self) -> bool {
        self.title.trim().is_empty()
            && self.username.is_empty()
            && self.password.is_empty()
            && self.urls.is_empty()
            && self.notes.trim().is_empty()
            && self.totp.is_empty()
            && self.typed.iter().all(|(_, v)| v.trim().is_empty())
            && self.custom_fields.is_empty()
    }

    /// Number of one-time code secrets this record carries.
    pub fn seed_count(&self) -> usize {
        self.totp.len()
    }

    /// Number of one-time codes that can really make codes (a key that is not Base32, a Steam
    /// code or a counter-based one is kept as a hidden note instead and is not counted).
    pub fn usable_seed_count(&self) -> usize {
        self.totp
            .iter()
            .filter(|raw| otp::stored_value(raw).is_ok())
            .count()
    }

    fn display_title(&self) -> String {
        let t = self.title.trim();
        if !t.is_empty() {
            return t.to_string();
        }
        if let Some(h) = self.urls.first().and_then(|u| host_of(u)) {
            return h;
        }
        if !self.username.trim().is_empty() {
            return self.username.trim().to_string();
        }
        "(no name)".to_string()
    }

    /// Convert into an item draft (same field keys as the app's editor). Never fails: anything
    /// that cannot be stored as a proper field is kept in a custom field and reported.
    pub fn to_draft(&self) -> Converted {
        let mut warnings = Vec::new();
        let title: String = cap_chars(&self.display_title(), MAX_TITLE_CHARS);
        let label = warn_title(&title);
        let mut extra: Vec<CustomField> = Vec::new();
        let mut fields: BTreeMap<String, String> = BTreeMap::new();

        // One-time codes: the first readable one is the item's code, the others are kept.
        let mut primary: Option<String> = None;
        let mut extra_seeds = 0usize;
        for raw in &self.totp {
            match otp::stored_value(raw) {
                Ok(v) => {
                    if primary.is_none() {
                        primary = Some(v.to_string());
                    } else {
                        extra_seeds += 1;
                        extra.push(CustomField {
                            name: format!("One-time code {}", extra_seeds + 1),
                            value: v.to_string(),
                            hidden: true,
                        });
                    }
                }
                Err(why) => {
                    extra.push(CustomField {
                        name: "One-time code (not imported)".into(),
                        value: raw.clone(),
                        hidden: true,
                    });
                    warnings.push(format!(
                        "\u{201c}{label}\u{201d}: its one-time code could not be read ({why}). It was kept in a hidden custom field."
                    ));
                }
            }
        }

        let mut kind = self.kind;
        if kind == EntryKind::Login
            && primary.is_some()
            && self.username.trim().is_empty()
            && self.password.is_empty()
            && self.urls.is_empty()
            && self.notes.trim().is_empty()
            && self.custom_fields.is_empty()
        {
            kind = EntryKind::Totp;
        }
        if kind == EntryKind::Totp && primary.is_none() {
            // A code that cannot be read is no code item: keep what there is as a note.
            kind = EntryKind::Note;
        }

        let item_type = match kind {
            EntryKind::Login => ItemType::Login,
            EntryKind::Card => ItemType::Card,
            EntryKind::Identity => ItemType::Identity,
            EntryKind::SshKey => ItemType::SshKey,
            EntryKind::Note => ItemType::SecureNote,
            EntryKind::Totp => ItemType::Totp,
        };

        match kind {
            EntryKind::Login => {
                put(&mut fields, "username", &self.username);
                put(&mut fields, "password", &self.password);
                if let Some(first) = self.urls.first() {
                    put(&mut fields, "url", first);
                }
                for (i, u) in self.urls.iter().enumerate().skip(1) {
                    extra.push(CustomField {
                        name: format!("Website {}", i + 1),
                        value: u.clone(),
                        hidden: false,
                    });
                }
                put(&mut fields, "notes", &self.notes);
            }
            EntryKind::Note => {
                put(&mut fields, "body", &self.notes);
                for (i, u) in self.urls.iter().enumerate() {
                    extra.push(CustomField {
                        name: if i == 0 {
                            "Website".into()
                        } else {
                            format!("Website {}", i + 1)
                        },
                        value: u.clone(),
                        hidden: false,
                    });
                }
                if !self.username.is_empty() {
                    extra.push(CustomField {
                        name: "Username".into(),
                        value: self.username.clone(),
                        hidden: false,
                    });
                }
                if !self.password.is_empty() {
                    extra.push(CustomField {
                        name: "Password".into(),
                        value: self.password.clone(),
                        hidden: true,
                    });
                }
            }
            EntryKind::Card | EntryKind::Identity | EntryKind::SshKey | EntryKind::Totp => {
                if kind == EntryKind::Totp {
                    // The code schema has no notes or website field.
                    if !self.notes.trim().is_empty() {
                        extra.push(CustomField {
                            name: "Notes".into(),
                            value: self.notes.clone(),
                            hidden: false,
                        });
                    }
                    for (i, u) in self.urls.iter().enumerate() {
                        extra.push(CustomField {
                            name: if i == 0 {
                                "Website".into()
                            } else {
                                format!("Website {}", i + 1)
                            },
                            value: u.clone(),
                            hidden: false,
                        });
                    }
                } else {
                    put(&mut fields, "notes", &self.notes);
                }
                self.apply_typed(item_type, &mut fields, &mut extra, &mut warnings, &label);
            }
        }
        if let Some(p) = &primary {
            if item_type.spec("totp").is_some() {
                fields.insert("totp".into(), p.clone());
            } else {
                // Cards, identities, keys and notes have no code field: keep it, hidden.
                extra.push(CustomField {
                    name: "One-time code".into(),
                    value: p.clone(),
                    hidden: true,
                });
            }
        }

        for f in &self.custom_fields {
            extra.push(CustomField {
                name: f.name.clone(),
                value: f.value.clone(),
                hidden: f.hidden,
            });
        }
        let mut custom_fields: Vec<CustomField> = Vec::new();
        for (i, mut c) in extra.into_iter().enumerate() {
            c.name = cap_chars(c.name.trim(), MAX_CUSTOM_NAME_CHARS);
            if c.name.is_empty() {
                c.name = format!("Field {}", i + 1);
            }
            if c.value.is_empty() {
                continue;
            }
            custom_fields.push(c);
        }
        if custom_fields.len() > MAX_CUSTOM_FIELDS {
            warnings.push(format!(
                "\u{201c}{label}\u{201d} has more than {MAX_CUSTOM_FIELDS} custom fields; the extra ones were left out."
            ));
            custom_fields.truncate(MAX_CUSTOM_FIELDS);
        }

        let mut tags: Vec<String> = Vec::new();
        for t in &self.tags {
            let t = cap_chars(t.trim(), MAX_TAG_CHARS);
            if !t.is_empty() && !tags.contains(&t) {
                tags.push(t);
            }
        }
        if tags.len() > MAX_TAGS {
            warnings.push(format!(
                "\u{201c}{label}\u{201d} has more than {MAX_TAGS} tags; the extra ones were left out."
            ));
            tags.truncate(MAX_TAGS);
        }

        let folder = self
            .folder
            .as_deref()
            .map(|f| cap_chars(f.trim(), MAX_FOLDER_CHARS))
            .filter(|f| !f.is_empty());

        Converted {
            draft: ItemDraft {
                item_type,
                title,
                favorite: self.favorite,
                tags,
                folder,
                fields,
                custom_fields,
            },
            created: self.created,
            warnings,
        }
    }

    fn apply_typed(
        &self,
        item_type: ItemType,
        fields: &mut BTreeMap<String, String>,
        extra: &mut Vec<CustomField>,
        warnings: &mut Vec<String>,
        label: &str,
    ) {
        for (key, value) in &self.typed {
            if value.trim().is_empty() {
                continue;
            }
            if item_type == ItemType::Card && key == "expiry" {
                match parse_expiry(value) {
                    Some((m, y)) => {
                        fields.entry("expMonth".into()).or_insert(format!("{m:02}"));
                        fields.entry("expYear".into()).or_insert(format!("{y:04}"));
                    }
                    None => extra.push(CustomField {
                        name: "Expiry (as in the source)".into(),
                        value: value.clone(),
                        hidden: false,
                    }),
                }
                continue;
            }
            if item_type.spec(key).is_none() {
                extra.push(CustomField {
                    name: key.clone(),
                    value: value.clone(),
                    hidden: false,
                });
                continue;
            }
            fields.insert(key.clone(), value.clone());
        }
        if item_type == ItemType::Card {
            if let Some(n) = fields.get("number") {
                if !card_number_ok(n) {
                    if let Some(bad) = fields.remove("number") {
                        extra.push(CustomField {
                            name: "Card number (as in the source)".into(),
                            value: bad,
                            hidden: true,
                        });
                        warnings.push(format!(
                            "\u{201c}{label}\u{201d}: the card number does not look like a card number. It was kept in a hidden custom field."
                        ));
                    }
                }
            }
            if let Some(m) = fields.get("expMonth") {
                let ok = m.trim().parse::<u8>().is_ok_and(|n| (1..=12).contains(&n));
                if !ok {
                    if let Some(bad) = fields.remove("expMonth") {
                        extra.push(CustomField {
                            name: "Expiry month (as in the source)".into(),
                            value: bad,
                            hidden: false,
                        });
                    }
                } else if let Some(v) = fields.get_mut("expMonth") {
                    *v = format!("{:02}", v.trim().parse::<u8>().unwrap_or(1));
                }
            }
            if let Some(y) = fields.get("expYear") {
                let t = y.trim().to_string();
                let fixed = match t.len() {
                    4 if t.bytes().all(|b| b.is_ascii_digit()) => Some(t),
                    2 if t.bytes().all(|b| b.is_ascii_digit()) => Some(format!("20{t}")),
                    _ => None,
                };
                match fixed {
                    Some(v) => {
                        fields.insert("expYear".into(), v);
                    }
                    None => {
                        if let Some(bad) = fields.remove("expYear") {
                            extra.push(CustomField {
                                name: "Expiry year (as in the source)".into(),
                                value: bad,
                                hidden: false,
                            });
                        }
                    }
                }
            }
        }
    }
}

fn put(fields: &mut BTreeMap<String, String>, key: &str, value: &str) {
    if let Some(v) = non_empty(value) {
        fields.insert(key.to_string(), v.to_string());
    }
}

/// Same shape rule the vault applies to a card number (digits, spaces and dashes; 8 to 23 characters).
fn card_number_ok(n: &str) -> bool {
    let n = n.trim();
    let len = n.chars().count();
    let shape = n
        .chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-');
    let digits = n.chars().filter(char::is_ascii_digit).count();
    shape && (8..=23).contains(&len) && digits >= 8
}

fn month_from_name(s: &str) -> Option<u8> {
    const NAMES: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower = s.trim().to_ascii_lowercase();
    let p = lower.get(..3)?;
    NAMES
        .iter()
        .position(|n| *n == p)
        .and_then(|i| u8::try_from(i + 1).ok())
}

/// Month and four-digit year from the shapes cards are exported in: `09/29`, `9/2029`,
/// `2029-09`, `092029`, `202909`, `September,2029`.
pub fn parse_expiry(s: &str) -> Option<(u8, u16)> {
    let s = s.trim();
    let tokens: Vec<&str> = s
        .split(['/', '-', ',', ' ', '.'])
        .filter(|t| !t.is_empty())
        .collect();
    let year_of = |y: &str| -> Option<u16> {
        if !y.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: u16 = y.parse().ok()?;
        match y.len() {
            2 => Some(2000 + n),
            4 if (1900..=2200).contains(&n) => Some(n),
            _ => None,
        }
    };
    let month_of = |m: &str| -> Option<u8> {
        if m.bytes().all(|b| b.is_ascii_digit()) && (1..=2).contains(&m.len()) {
            m.parse::<u8>().ok().filter(|n| (1..=12).contains(n))
        } else {
            month_from_name(m)
        }
    };
    match tokens.as_slice() {
        [a, b] => {
            if a.len() == 4 && a.bytes().all(|c| c.is_ascii_digit()) {
                Some((month_of(b)?, year_of(a)?))
            } else {
                Some((month_of(a)?, year_of(b)?))
            }
        }
        [one] if one.bytes().all(|b| b.is_ascii_digit()) => match one.len() {
            4 => Some((month_of(one.get(..2)?)?, year_of(one.get(2..)?)?)),
            6 if one.starts_with("19") || one.starts_with("20") || one.starts_with("21") => {
                Some((month_of(one.get(4..)?)?, year_of(one.get(..4)?)?))
            }
            6 => Some((month_of(one.get(..2)?)?, year_of(one.get(2..)?)?)),
            _ => None,
        },
        _ => None,
    }
}

// ------------------------------------------------------------------ duplicate rule

/// What makes two items "the same" for the duplicate rule: the item type and the title
/// (case-insensitive), and for logins also the username (case-insensitive) and the primary host
/// (the first address's host, without `www.`, lower case).
#[derive(Clone, PartialEq, Eq, Hash, Zeroize, ZeroizeOnDrop)]
pub struct DupKey {
    #[zeroize(skip)]
    pub item_type: ItemType,
    pub title: String,
    pub username: String,
    pub host: String,
}

impl std::fmt::Debug for DupKey {
    /// Shows nothing but the type: a key holds a username.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DupKey")
            .field("item_type", &self.item_type)
            .finish_non_exhaustive()
    }
}

/// The duplicate key from its parts (`username` and `url` are only read for logins).
pub fn dup_key_parts(item_type: ItemType, title: &str, username: &str, url: &str) -> DupKey {
    let login = item_type == ItemType::Login;
    DupKey {
        item_type,
        title: title.trim().to_lowercase(),
        username: if login {
            username.trim().to_lowercase()
        } else {
            String::new()
        },
        host: if login {
            host_of(url.trim()).unwrap_or_default().to_lowercase()
        } else {
            String::new()
        },
    }
}

pub fn dup_key(d: &ItemDraft) -> DupKey {
    let get = |k: &str| d.fields.get(k).map_or("", |v| v.as_str());
    dup_key_parts(d.item_type, &d.title, get("username"), get("url"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login() -> ImportedEntry {
        let mut e = ImportedEntry::new(EntryKind::Login, "GitHub");
        e.username = "ana".into();
        e.password = "p w".into();
        e.add_url("https://github.com");
        e.add_url("https://gist.github.com");
        e.set_notes("n");
        e
    }

    #[test]
    fn debug_never_prints_values() {
        let mut e = login();
        e.add_totp("JBSWY3DPEHPK3PXP");
        e.add_custom("pin", "hunter2-secret", true);
        e.typed.push(("number".into(), "4111111111111111".into()));
        let shown = format!("{e:?} {:?}", e.custom_fields);
        for secret in [
            "GitHub",
            "ana",
            "p w",
            "hunter2-secret",
            "JBSWY3",
            "4111",
            "github.com",
        ] {
            assert!(!shown.contains(secret), "{secret} leaked into {shown}");
        }
    }

    #[test]
    fn login_conversion_uses_the_app_field_keys() {
        let mut e = login();
        e.add_totp("jbsw y3dp ehpk 3pxp");
        e.folder = Some("Work".into());
        e.favorite = true;
        let c = e.to_draft();
        let d = &c.draft;
        assert_eq!(d.item_type, ItemType::Login);
        assert_eq!(d.fields["username"], "ana");
        assert_eq!(d.fields["password"], "p w");
        assert_eq!(d.fields["url"], "https://github.com");
        assert_eq!(d.fields["totp"], "JBSWY3DPEHPK3PXP");
        assert_eq!(d.fields["notes"], "n");
        assert_eq!(d.folder.as_deref(), Some("Work"));
        assert!(d.favorite);
        assert_eq!(d.custom_fields.len(), 1);
        assert_eq!(d.custom_fields[0].name, "Website 2");
        assert!(c.warnings.is_empty());
        for k in d.fields.keys() {
            assert!(ItemType::Login.spec(k).is_some());
        }
    }

    #[test]
    fn passwords_keep_their_spaces() {
        let mut e = login();
        e.password = "  spaced out  ".into();
        assert_eq!(e.to_draft().draft.fields["password"], "  spaced out  ");
    }

    #[test]
    fn an_unreadable_code_is_kept_and_reported() {
        let mut e = login();
        e.add_totp("steam://ABCDEFGH");
        let c = e.to_draft();
        assert!(!c.draft.fields.contains_key("totp"));
        let kept = c
            .draft
            .custom_fields
            .iter()
            .find(|f| f.name == "One-time code (not imported)")
            .unwrap();
        assert!(kept.hidden);
        assert_eq!(kept.value, "steam://ABCDEFGH");
        assert_eq!(c.warnings.len(), 1);
        assert!(!c.warnings[0].contains("ABCDEFGH"));
    }

    #[test]
    fn extra_codes_are_kept_as_hidden_fields() {
        let mut e = login();
        e.add_totp("JBSWY3DPEHPK3PXP");
        e.add_totp("GEZDGNBVGY3TQOJQ");
        let d = e.to_draft().draft;
        assert_eq!(d.fields["totp"], "JBSWY3DPEHPK3PXP");
        let f = d
            .custom_fields
            .iter()
            .find(|f| f.name == "One-time code 2")
            .unwrap();
        assert!(f.hidden);
    }

    #[test]
    fn a_login_with_only_a_code_becomes_a_code_item() {
        let mut e = ImportedEntry::new(EntryKind::Login, "Bank 2FA");
        e.add_totp("JBSWY3DPEHPK3PXP");
        let d = e.to_draft().draft;
        assert_eq!(d.item_type, ItemType::Totp);
        assert_eq!(d.fields["totp"], "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn a_code_item_without_a_readable_code_falls_back_to_a_note() {
        let mut e = ImportedEntry::new(EntryKind::Totp, "X");
        e.add_totp("not base32!!");
        let c = e.to_draft();
        assert_eq!(c.draft.item_type, ItemType::SecureNote);
        assert_eq!(c.warnings.len(), 1);
    }

    #[test]
    fn card_expiry_and_number_are_checked() {
        let mut e = ImportedEntry::new(EntryKind::Card, "Visa");
        e.put("number", "4111 1111 1111 1111");
        e.put("expiry", "9/29");
        e.put("cardholder", "A B");
        let d = e.to_draft().draft;
        assert_eq!(d.fields["expMonth"], "09");
        assert_eq!(d.fields["expYear"], "2029");
        let mut ok = d.clone();
        ok.normalize().unwrap();

        let mut bad = ImportedEntry::new(EntryKind::Card, "Odd");
        bad.put("number", "XXXX-1234");
        bad.put("expiry", "soon");
        let c = bad.to_draft();
        assert!(!c.draft.fields.contains_key("number"));
        assert!(c
            .draft
            .custom_fields
            .iter()
            .any(|f| f.name.starts_with("Card number")));
        assert!(c
            .draft
            .custom_fields
            .iter()
            .any(|f| f.name.starts_with("Expiry")));
        let mut d = c.draft.clone();
        d.normalize().unwrap();
    }

    #[test]
    fn expiry_shapes() {
        assert_eq!(parse_expiry("09/29"), Some((9, 2029)));
        assert_eq!(parse_expiry("9/2029"), Some((9, 2029)));
        assert_eq!(parse_expiry("2029-09"), Some((9, 2029)));
        assert_eq!(parse_expiry("0929"), Some((9, 2029)));
        assert_eq!(parse_expiry("202909"), Some((9, 2029)));
        assert_eq!(parse_expiry("092029"), Some((9, 2029)));
        assert_eq!(parse_expiry("September,2029"), Some((9, 2029)));
        assert_eq!(parse_expiry("13/29"), None);
        assert_eq!(parse_expiry(""), None);
        assert_eq!(parse_expiry("abc"), None);
    }

    #[test]
    fn unknown_typed_keys_become_custom_fields_never_validation_errors() {
        let mut e = ImportedEntry::new(EntryKind::Identity, "Me");
        e.put("firstName", "A");
        e.put("bogusKey", "x");
        let mut d = e.to_draft().draft;
        assert!(d.custom_fields.iter().any(|f| f.name == "bogusKey"));
        d.normalize().unwrap();
    }

    #[test]
    fn nested_folders_keep_their_depth_as_tags() {
        let mut e = ImportedEntry::new(EntryKind::Login, "x");
        e.set_folder_path(&["Work", "Servers", "Prod"]);
        let d = e.to_draft().draft;
        assert_eq!(d.folder.as_deref(), Some("Work"));
        assert_eq!(d.tags, vec!["Servers", "Prod"]);
    }

    #[test]
    fn title_falls_back_to_host_then_username() {
        let mut e = ImportedEntry::new(EntryKind::Login, " ");
        e.add_url("https://www.example.com/login");
        assert_eq!(e.to_draft().draft.title, "example.com");
        let mut e = ImportedEntry::new(EntryKind::Login, "");
        e.username = "ana".into();
        assert_eq!(e.to_draft().draft.title, "ana");
        assert_eq!(
            ImportedEntry::new(EntryKind::Note, "")
                .to_draft()
                .draft
                .title,
            "(no name)"
        );
    }

    #[test]
    fn too_many_custom_fields_are_cut_with_a_warning() {
        let mut e = login();
        for i in 0..60 {
            e.add_custom(&format!("f{i}"), "v", false);
        }
        let c = e.to_draft();
        assert_eq!(c.draft.custom_fields.len(), MAX_CUSTOM_FIELDS);
        assert!(c.warnings.iter().any(|w| w.contains("custom fields")));
    }

    #[test]
    fn dup_key_rule() {
        let a = login().to_draft().draft;
        let mut b = login();
        b.title = "GITHUB".into();
        b.username = " ANA ".into();
        b.urls = vec!["http://www.github.com/login".into()];
        assert_eq!(dup_key(&a), dup_key(&b.to_draft().draft));
        let mut c = login();
        c.username = "bob".into();
        assert_ne!(dup_key(&a), dup_key(&c.to_draft().draft));
        let mut d = login();
        d.urls = vec!["https://gitlab.com".into()];
        assert_ne!(dup_key(&a), dup_key(&d.to_draft().draft));
        let mut note = ImportedEntry::new(EntryKind::Note, "GitHub");
        note.set_notes("x");
        assert_ne!(dup_key(&a), dup_key(&note.to_draft().draft));
    }
}
