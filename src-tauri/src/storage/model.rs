//! Item model: the decrypted payload (schema 1, `ai-docs/STORAGE_AND_SYNC.md` §1), the editable
//! draft, and the wire types of `ai-docs/IPC_CONTRACT.md`.
//!
//! Secrets live in [`ItemPayload`] and [`ItemDraft`]; both wipe their values on drop.

use super::error::{Result, StorageError};
use super::profiles::ProfileInfo;
use crate::crypto::{CryptoError, TotpAccount, TotpParams};
use crate::passkey::{KeepMarker, PasskeyInfo, PasskeyRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroize;

pub const SCHEMA: u32 = 1;

pub const MAX_TITLE_CHARS: usize = 256;
pub const MAX_FIELD_BYTES: usize = 64 * 1024;
pub const MAX_TAGS: usize = 32;
pub const MAX_TAG_CHARS: usize = 64;
pub const MAX_FOLDER_CHARS: usize = 128;
pub const MAX_CUSTOM_FIELDS: usize = 50;
pub const MAX_CUSTOM_NAME_CHARS: usize = 64;
/// How many web addresses one login may hold (they are kept one per line in its `url` field).
pub const MAX_URLS: usize = 20;
/// How many sign-ins (username and password) and how many one-time codes one login may hold.
/// The first of each has the plain key (`username`); the others are `username#2` … `username#10`.
pub const MAX_INSTANCES: u32 = 10;
/// Sum of all text in one item (a guard far below the 64 MiB envelope limit).
pub const MAX_ITEM_BYTES: usize = 4 * 1024 * 1024;

// ------------------------------------------------------------------ item types and field schemas

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemType {
    Login,
    Card,
    Identity,
    SshKey,
    SecureNote,
    Totp,
    Media,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Text,
    Url,
    Password,
    /// Secret multi-line text.
    Note,
    Totp,
    /// Non-secret multi-line text (for example an SSH public key).
    Paragraph,
    /// A passkey: a small JSON record with the private key inside (`crate::passkey::record`). Only the core makes one.
    Passkey,
}

impl FieldKind {
    /// Secret fields are never returned by `item_get`; they need the re-prompt.
    pub fn is_secret(self) -> bool {
        matches!(
            self,
            Self::Password | Self::Note | Self::Totp | Self::Passkey
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    /// May be stored more than once (`key`, `key#2` …, at most [`MAX_INSTANCES`]).
    pub repeats: bool,
}

const fn spec(key: &'static str, label: &'static str, kind: FieldKind) -> FieldSpec {
    FieldSpec {
        key,
        label,
        kind,
        repeats: false,
    }
}

const fn repeating(key: &'static str, label: &'static str, kind: FieldKind) -> FieldSpec {
    FieldSpec {
        key,
        label,
        kind,
        repeats: true,
    }
}

/// Split a stored field key into its schema key and instance number: `totp` is `("totp", 1)`,
/// `totp#3` is `("totp", 3)`. Spellings that are not canonical (`totp#1`, `totp#03`, `totp#`)
/// are `None`, so one instance can never be stored under two keys.
pub fn parse_key(key: &str) -> Option<(&str, u32)> {
    let Some((base, n)) = key.split_once('#') else {
        return Some((key, 1));
    };
    if n.is_empty() || n.starts_with('0') || n.len() > 3 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = n.parse().ok()?;
    (n >= 2).then_some((base, n))
}

/// The stored key of instance `n` of `base`.
pub fn instance_key(base: &str, n: u32) -> String {
    if n <= 1 {
        base.to_string()
    } else {
        format!("{base}#{n}")
    }
}

/// Every stored instance of one field, lowest number first: `(n, key, value)`.
pub fn instances<'a>(
    fields: &'a BTreeMap<String, String>,
    base: &str,
) -> Vec<(u32, &'a str, &'a str)> {
    let mut out: Vec<(u32, &str, &str)> = fields
        .iter()
        .filter_map(|(k, v)| match parse_key(k) {
            Some((b, n)) if b == base => Some((n, k.as_str(), v.as_str())),
            _ => None,
        })
        .collect();
    out.sort_by_key(|(n, _, _)| *n);
    out
}

/// The fields that share instance numbers: a sign-in's username and password are one numbered
/// group, a one-time code is on its own.
fn slot_group(base: &str) -> &'static [&'static str] {
    match base {
        "username" | "password" => &["username", "password"],
        "totp" => &["totp"],
        "passkey" => &["passkey"],
        _ => &[],
    }
}

/// The instance numbers stored for any of `bases`, lowest first.
fn slots_in(fields: &BTreeMap<String, String>, bases: &[&str]) -> Vec<u32> {
    let mut out: Vec<u32> = fields
        .keys()
        .filter_map(|k| parse_key(k))
        .filter(|(b, _)| bases.contains(b))
        .map(|(_, n)| n)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The value of the first instance (the lowest number that is stored).
pub fn first_instance<'a>(fields: &'a BTreeMap<String, String>, base: &str) -> Option<&'a str> {
    instances(fields, base).first().map(|(_, _, v)| *v)
}

const LOGIN: &[FieldSpec] = &[
    repeating("username", "Username", FieldKind::Text),
    repeating("password", "Password", FieldKind::Password),
    spec("url", "Website", FieldKind::Url),
    repeating("totp", "One-time code", FieldKind::Totp),
    repeating("passkey", "Passkey", FieldKind::Passkey),
    spec("notes", "Notes", FieldKind::Note),
];
const CARD: &[FieldSpec] = &[
    spec("cardholder", "Cardholder name", FieldKind::Text),
    spec("brand", "Brand", FieldKind::Text),
    spec("number", "Card number", FieldKind::Password),
    spec("expMonth", "Expiry month", FieldKind::Text),
    spec("expYear", "Expiry year", FieldKind::Text),
    spec("code", "Security code", FieldKind::Password),
    spec("pin", "PIN", FieldKind::Password),
    spec("notes", "Notes", FieldKind::Note),
];
const IDENTITY: &[FieldSpec] = &[
    spec("salutation", "Title", FieldKind::Text),
    spec("firstName", "First name", FieldKind::Text),
    spec("middleName", "Middle name", FieldKind::Text),
    spec("lastName", "Last name", FieldKind::Text),
    spec("email", "Email", FieldKind::Text),
    spec("phone", "Phone", FieldKind::Text),
    spec("company", "Company", FieldKind::Text),
    spec("address1", "Address", FieldKind::Text),
    spec("address2", "Address line 2", FieldKind::Text),
    spec("city", "City", FieldKind::Text),
    spec("state", "State or region", FieldKind::Text),
    spec("postalCode", "Postal code", FieldKind::Text),
    spec("country", "Country", FieldKind::Text),
    spec("nationalId", "National ID", FieldKind::Password),
    spec("passport", "Passport number", FieldKind::Password),
    spec("license", "Driver's license", FieldKind::Password),
    spec("notes", "Notes", FieldKind::Note),
];
const SSH_KEY: &[FieldSpec] = &[
    spec("publicKey", "Public key", FieldKind::Paragraph),
    spec("privateKey", "Private key", FieldKind::Note),
    spec("passphrase", "Passphrase", FieldKind::Password),
    spec("fingerprint", "Fingerprint", FieldKind::Text),
    spec("notes", "Notes", FieldKind::Note),
];
const TOTP: &[FieldSpec] = &[
    spec("issuer", "Issuer", FieldKind::Text),
    spec("account", "Account", FieldKind::Text),
    spec("totp", "One-time code", FieldKind::Totp),
];
const SECURE_NOTE: &[FieldSpec] = &[spec("body", "Note", FieldKind::Note)];
const MEDIA: &[FieldSpec] = &[spec("description", "Description", FieldKind::Note)];

impl ItemType {
    /// Allowed field keys in display order (`IPC_CONTRACT.md` field schemas).
    pub fn schema(self) -> &'static [FieldSpec] {
        match self {
            Self::Login => LOGIN,
            Self::Card => CARD,
            Self::Identity => IDENTITY,
            Self::SshKey => SSH_KEY,
            Self::Totp => TOTP,
            Self::SecureNote => SECURE_NOTE,
            Self::Media => MEDIA,
        }
    }

    /// The schema entry a stored key belongs to. `totp#2` belongs to `totp` where the type repeats
    /// that field; any other key must be exactly a schema key.
    pub fn spec(self, key: &str) -> Option<&'static FieldSpec> {
        let (base, n) = parse_key(key)?;
        let found = self.schema().iter().find(|s| s.key == base)?;
        (n == 1 || (found.repeats && n <= MAX_INSTANCES)).then_some(found)
    }
}

// ------------------------------------------------------------------ draft (editable content)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomField {
    pub name: String,
    pub value: String,
    pub hidden: bool,
}

/// Full decrypted, editable content of an item (secrets included).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemDraft {
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub title: String,
    #[serde(default)]
    pub favorite: bool,
    /// "Ask for the master password again": revealing, copying, editing or cloning this item (or any of its old
    /// versions) needs the master password every time, even while the vault is open. Off for ordinary items.
    #[serde(default)]
    pub reprompt: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub custom_fields: Vec<CustomField>,
}

impl Drop for ItemDraft {
    fn drop(&mut self) {
        wipe_values(&mut self.fields, &mut self.custom_fields);
    }
}

fn wipe_values(fields: &mut BTreeMap<String, String>, custom: &mut [CustomField]) {
    for v in fields.values_mut() {
        v.zeroize();
    }
    for c in custom {
        c.value.zeroize();
    }
}

/// Short, printable echo of a user-supplied key for an error message.
fn echo(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(40)
        .collect::<String>()
}

impl ItemDraft {
    /// Trim, de-duplicate, drop empty field values, and enforce every limit and schema rule.
    /// Called on every create/update, so nothing invalid ever reaches the database.
    pub fn normalize(&mut self) -> Result<()> {
        self.title = self.title.trim().to_string();
        if self.title.is_empty() {
            return Err(StorageError::invalid("A title is required."));
        }
        if self.title.chars().count() > MAX_TITLE_CHARS {
            return Err(StorageError::invalid(
                "The title is too long (256 characters at most).",
            ));
        }

        let mut tags: Vec<String> = Vec::new();
        for t in &self.tags {
            let t = t.trim();
            if t.is_empty() {
                continue;
            }
            if t.chars().count() > MAX_TAG_CHARS {
                return Err(StorageError::invalid(
                    "A tag is too long (64 characters at most).",
                ));
            }
            if !tags.iter().any(|x| x == t) {
                tags.push(t.to_string());
            }
        }
        if tags.len() > MAX_TAGS {
            return Err(StorageError::invalid("Use at most 32 tags."));
        }
        self.tags = tags;

        self.folder = match self.folder.take() {
            Some(f) => {
                // the same spelling a folder made in the app has: `" Work // Clients "` is `Work/Clients`
                let f = super::vault::folders::clean(&f);
                if f.as_ref()
                    .is_some_and(|f| f.chars().count() > MAX_FOLDER_CHARS)
                {
                    return Err(StorageError::invalid(
                        "The folder name is too long (128 characters at most).",
                    ));
                }
                f
            }
            None => None,
        };

        let mut total = self.title.len();
        for (key, value) in &self.fields {
            if self.item_type.spec(key).is_none() {
                return Err(StorageError::invalid(format!(
                    "Unknown field \"{}\" for this item type.",
                    echo(key)
                )));
            }
            if value.len() > MAX_FIELD_BYTES {
                return Err(StorageError::invalid(
                    "A field is too long (64 KiB at most).",
                ));
            }
            total += key.len() + value.len();
        }
        self.fields.retain(|_, v| !v.is_empty());

        // several web addresses live in the one `url` field, a line each: tidy them like tags
        if let Some(v) = self.fields.get("url") {
            let urls = clean_urls(v);
            if urls.len() > MAX_URLS {
                return Err(StorageError::invalid(format!(
                    "Use at most {MAX_URLS} web addresses."
                )));
            }
            if urls.is_empty() {
                self.fields.remove("url");
            } else {
                self.fields.insert("url".into(), urls.join("\n"));
            }
        }

        for (key, value) in &self.fields {
            if self
                .item_type
                .spec(key)
                .is_some_and(|s| s.kind == FieldKind::Totp)
            {
                validate_totp(value)?;
            }
        }
        self.check_passkeys()?;
        if self.item_type == ItemType::Totp && !self.fields.contains_key("totp") {
            return Err(StorageError::invalid(
                "Add the secret key or an otpauth:// link for this code.",
            ));
        }
        if self.item_type == ItemType::Card {
            self.normalize_card()?;
        }

        if self.custom_fields.len() > MAX_CUSTOM_FIELDS {
            return Err(StorageError::invalid("Use at most 50 custom fields."));
        }
        for c in &mut self.custom_fields {
            c.name = c.name.trim().to_string();
            if c.name.is_empty() {
                return Err(StorageError::invalid("Every custom field needs a name."));
            }
            if c.name.chars().count() > MAX_CUSTOM_NAME_CHARS {
                return Err(StorageError::invalid(
                    "A custom field name is too long (64 characters at most).",
                ));
            }
            if c.value.len() > MAX_FIELD_BYTES {
                return Err(StorageError::invalid(
                    "A custom field is too long (64 KiB at most).",
                ));
            }
            total += c.name.len() + c.value.len();
        }
        if total > MAX_ITEM_BYTES {
            return Err(StorageError::invalid("This item is too large."));
        }
        Ok(())
    }
}

impl ItemDraft {
    /// Every passkey field holds a good record, and no passkey is on the item twice.
    fn check_passkeys(&self) -> Result<()> {
        let mut seen: Vec<String> = Vec::new();
        for (key, value) in &self.fields {
            if !self
                .item_type
                .spec(key)
                .is_some_and(|s| s.kind == FieldKind::Passkey)
            {
                continue;
            }
            let record =
                PasskeyRecord::parse(value).map_err(|e| StorageError::invalid(e.to_string()))?;
            if seen.contains(&record.credential_id) {
                return Err(StorageError::invalid(
                    "The same passkey is on this item twice.",
                ));
            }
            seen.push(record.credential_id.clone());
        }
        Ok(())
    }

    /// The editable copy that goes to the screen: each passkey is a marker with its public facts and no key.
    pub fn redact_passkeys(&mut self) {
        for (key, value) in self.fields.iter_mut() {
            if !self
                .item_type
                .spec(key)
                .is_some_and(|s| s.kind == FieldKind::Passkey)
            {
                continue;
            }
            *value = match PasskeyRecord::parse(value) {
                Ok(r) => serde_json::to_string(&KeepMarker {
                    keep: true,
                    info: r.info(),
                })
                .unwrap_or_default(),
                Err(_) => String::new(),
            };
        }
        self.fields.retain(|_, v| !v.is_empty());
    }

    /// Put the stored passkeys back where the screen's copy has a marker. The screen can keep or drop a passkey; it
    /// cannot make one or change one, so anything that is not a marker of a passkey the item holds is refused.
    pub fn restore_passkeys(&mut self, stored: &ItemPayload) -> Result<()> {
        let held: Vec<PasskeyRecord> = stored
            .fields
            .iter()
            .filter(|(k, _)| {
                stored
                    .item_type
                    .spec(k)
                    .is_some_and(|s| s.kind == FieldKind::Passkey)
            })
            .filter_map(|(_, v)| PasskeyRecord::parse(v).ok())
            .collect();
        let mut next: Vec<(String, String)> = Vec::new();
        for (key, value) in &self.fields {
            if !self
                .item_type
                .spec(key)
                .is_some_and(|s| s.kind == FieldKind::Passkey)
            {
                continue;
            }
            let marker: KeepMarker = serde_json::from_str(value)
                .ok()
                .filter(|m: &KeepMarker| m.keep)
                .ok_or_else(|| StorageError::invalid("Passkeys can only be made by the app."))?;
            let record = held
                .iter()
                .find(|r| r.credential_id == marker.info.credential_id)
                .ok_or_else(|| {
                    StorageError::invalid("A passkey on this item is no longer there.")
                })?;
            next.push((key.clone(), record.to_json().to_string()));
        }
        for (key, value) in next {
            self.fields.insert(key, value);
        }
        Ok(())
    }

    /// Take every passkey out (a copy of an item must not share a passkey with it).
    pub fn strip_passkeys(&mut self) {
        let item_type = self.item_type;
        self.fields.retain(|k, _| {
            !item_type
                .spec(k)
                .is_some_and(|s| s.kind == FieldKind::Passkey)
        });
    }

    /// The item as the person typed it must not carry a passkey they could have made up.
    pub fn forbid_passkeys(&self) -> Result<()> {
        if self.fields.keys().any(|k| {
            self.item_type
                .spec(k)
                .is_some_and(|s| s.kind == FieldKind::Passkey)
        }) {
            return Err(StorageError::invalid(
                "Passkeys can only be made by the app.",
            ));
        }
        Ok(())
    }

    /// Light, never-in-the-way checks for cards: the number looks like a number, the expiry like
    /// a date. Values are stored trimmed. Luhn is deliberately not enforced (gift and store cards).
    fn normalize_card(&mut self) -> Result<()> {
        for key in ["number", "expMonth", "expYear"] {
            if let Some(v) = self.fields.get_mut(key) {
                let trimmed = v.trim().to_string();
                v.zeroize();
                *v = trimmed;
            }
        }
        self.fields.retain(|_, v| !v.is_empty());
        if let Some(n) = self.fields.get("number") {
            let len = n.chars().count();
            let shape_ok = n
                .chars()
                .all(|c| c.is_ascii_digit() || c == ' ' || c == '-');
            let digits = n.chars().filter(char::is_ascii_digit).count();
            if !shape_ok || !(8..=23).contains(&len) || digits < 8 {
                return Err(StorageError::invalid(
                    "The card number may only contain digits, spaces and dashes (8 to 23 characters).",
                ));
            }
        }
        if let Some(m) = self.fields.get("expMonth") {
            let ok = m.len() <= 2
                && m.bytes().all(|b| b.is_ascii_digit())
                && m.parse::<u8>().is_ok_and(|n| (1..=12).contains(&n));
            if !ok {
                return Err(StorageError::invalid(
                    "The expiry month must be a number from 1 to 12.",
                ));
            }
        }
        if let Some(y) = self.fields.get("expYear") {
            if y.len() != 4 || !y.bytes().all(|b| b.is_ascii_digit()) {
                return Err(StorageError::invalid(
                    "The expiry year must have four digits, for example 2029.",
                ));
            }
        }
        Ok(())
    }
}

/// Names of what differs between two drafts of the same type, never values.
/// Order: title, favorite, reprompt, tags, folder, field keys in schema order, customFields.
pub fn changed_fields(old: &ItemDraft, new: &ItemDraft) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if old.title != new.title {
        out.push("title".into());
    }
    if old.favorite != new.favorite {
        out.push("favorite".into());
    }
    if old.reprompt != new.reprompt {
        out.push("reprompt".into());
    }
    if old.tags != new.tags {
        out.push("tags".into());
    }
    if old.folder != new.folder {
        out.push("folder".into());
    }
    for s in new.item_type.schema() {
        // `username`, then `username#2` …: the stored keys of both sides, lowest number first
        let mut keys: Vec<(u32, &str)> = instances(&old.fields, s.key)
            .into_iter()
            .chain(instances(&new.fields, s.key))
            .map(|(n, k, _)| (n, k))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        for (_, key) in keys {
            if old.fields.get(key) != new.fields.get(key) {
                out.push(key.into());
            }
        }
    }
    if old.custom_fields != new.custom_fields {
        out.push("customFields".into());
    }
    out
}

// ------------------------------------------------------------------ TOTP helpers

/// Build a TOTP account from an `otpauth://totp/...` URI or a bare Base32 secret.
pub fn totp_account(value: &str) -> Result<TotpAccount> {
    let v = value.trim();
    let is_uri = v
        .as_bytes()
        .get(..10)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"otpauth://"));
    let account = if is_uri {
        TotpAccount::from_uri(v)
    } else {
        TotpAccount::new(&TotpParams::defaults(v))
    };
    account.map_err(|e| match e {
        CryptoError::Totp(m) => {
            StorageError::invalid(format!("The one-time code secret is not valid: {m}."))
        }
        other => other.into(),
    })
}

fn validate_totp(value: &str) -> Result<()> {
    totp_account(value).map(|_| ())
}

// ------------------------------------------------------------------ stored payload (schema 1)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemEvent {
    Created,
    Edited,
    Trashed,
    Restored,
    RestoredFromVersion,
    Archived,
    Unarchived,
}

/// Reference to an encrypted attachment blob (the blobs and their UI arrive in Phase 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentRef {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub sha256: String,
}

/// Which state an item's state grew out of: the `(version, timestamp)` of its envelope header.
///
/// This is the lineage that sync needs. Two devices that both edit the same item write
/// different states with the same or overlapping version numbers; only the parent pointers tell
/// a plain fast-forward (the other device's state grew out of mine) from a real divergence (both
/// of us changed it). It is sealed inside the payload, so a bucket cannot forge or hide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentRef {
    pub version: u64,
    pub ts: u64,
}

/// What is sealed inside `items.blob` and `versions.blob`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemPayload {
    pub schema: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub title: String,
    pub favorite: bool,
    /// Added after schema 1 shipped. Written only when on, so an item without the flag is sealed byte-for-byte
    /// the way an older build seals it (an older build refuses a payload with a key it does not know).
    #[serde(default, skip_serializing_if = "is_false")]
    pub reprompt: bool,
    pub tags: Vec<String>,
    pub folder: Option<String>,
    pub fields: BTreeMap<String, String>,
    pub custom_fields: Vec<CustomField>,
    pub attachments: Vec<AttachmentRef>,
    pub created_at: u64,
    pub updated_at: u64,
    pub deleted: bool,
    pub deleted_at: Option<u64>,
    /// Added after schema 1 shipped: payloads written before it simply lack the key.
    #[serde(default)]
    pub archived: bool,
    pub event: ItemEvent,
    pub restored_from: Option<u64>,
    pub changed_fields: Vec<String>,
    /// The state this one replaced (absent on version 1 and on states written before sync
    /// existed, which sync treats as "lineage unknown").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentRef>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Drop for ItemPayload {
    fn drop(&mut self) {
        wipe_values(&mut self.fields, &mut self.custom_fields);
    }
}

impl ItemPayload {
    /// Version 1 of a new item. `draft` must already be normalized.
    pub fn new(id: String, draft: &ItemDraft, now: u64) -> Self {
        let mut p = Self {
            schema: SCHEMA,
            id,
            item_type: draft.item_type,
            title: String::new(),
            favorite: false,
            reprompt: false,
            tags: Vec::new(),
            folder: None,
            fields: BTreeMap::new(),
            custom_fields: Vec::new(),
            attachments: Vec::new(),
            created_at: now,
            updated_at: now,
            deleted: false,
            deleted_at: None,
            archived: false,
            event: ItemEvent::Created,
            restored_from: None,
            changed_fields: Vec::new(),
            parent: None,
        };
        p.apply_draft(draft);
        p
    }

    /// Replace the user-editable content with `draft`.
    pub fn apply_draft(&mut self, draft: &ItemDraft) {
        self.title = draft.title.clone();
        self.favorite = draft.favorite;
        self.reprompt = draft.reprompt;
        self.tags = draft.tags.clone();
        self.folder = draft.folder.clone();
        self.fields = draft.fields.clone();
        self.custom_fields = draft.custom_fields.clone();
    }

    pub fn to_draft(&self) -> ItemDraft {
        ItemDraft {
            item_type: self.item_type,
            title: self.title.clone(),
            favorite: self.favorite,
            reprompt: self.reprompt,
            tags: self.tags.clone(),
            folder: self.folder.clone(),
            fields: self.fields.clone(),
            custom_fields: self.custom_fields.clone(),
        }
    }

    /// Structural checks after decrypting: right schema, right id.
    pub fn check(&self, expected_id: &str) -> Result<()> {
        if self.schema != SCHEMA {
            return Err(StorageError::Storage(
                "This item was saved by a newer version of this app.".into(),
            ));
        }
        if self.id != expected_id {
            return Err(StorageError::Crypto(
                "A stored item failed its integrity check: the data is damaged or was modified."
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn summary(&self, version: u64) -> ItemSummary {
        ItemSummary {
            id: self.id.clone(),
            item_type: self.item_type,
            title: self.title.clone(),
            subtitle: subtitle_of(self.item_type, &self.fields),
            favorite: self.favorite,
            reprompt: self.reprompt,
            tags: self.tags.clone(),
            folder: self.folder.clone(),
            has_totp: !instances(&self.fields, "totp").is_empty(),
            code_keys: instances(&self.fields, "totp")
                .into_iter()
                .map(|(_, key, _)| key.to_string())
                .collect(),
            has_passkey: !instances(&self.fields, "passkey").is_empty(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            version,
            archived: self.archived,
        }
    }

    /// Masked view: secret values are replaced by `hasValue` flags.
    pub fn view(&self, version: u64) -> ItemView {
        let mut fields: Vec<FieldView> = Vec::new();
        for s in self.item_type.schema() {
            // a sign-in's username and password are numbered together, a code on its own
            let slots = slots_in(&self.fields, slot_group(s.key));
            for (_, key, value) in instances(&self.fields, s.key) {
                let secret = s.kind.is_secret();
                let n = parse_key(key).map_or(1, |(_, n)| n);
                fields.push(FieldView {
                    key: key.into(),
                    // "Password", or "Password 1", "Password 2" … once there are several; counted by place so a gap never shows
                    label: match slots.iter().position(|&x| x == n) {
                        Some(i) if slots.len() > 1 => format!("{} {}", s.label, i + 1),
                        _ => s.label.into(),
                    },
                    kind: s.kind,
                    secret,
                    value: (!secret).then(|| value.to_string()),
                    has_value: true,
                    passkey: (s.kind == FieldKind::Passkey)
                        .then(|| PasskeyRecord::parse(value).ok().map(|r| r.info()))
                        .flatten(),
                });
            }
        }
        ItemView {
            id: self.id.clone(),
            item_type: self.item_type,
            title: self.title.clone(),
            favorite: self.favorite,
            reprompt: self.reprompt,
            tags: self.tags.clone(),
            folder: self.folder.clone(),
            fields,
            custom_fields: self
                .custom_fields
                .iter()
                .map(|c| CustomFieldView {
                    name: c.name.clone(),
                    hidden: c.hidden,
                    value: (!c.hidden).then(|| c.value.clone()),
                })
                .collect(),
            attachments: self
                .attachments
                .iter()
                .map(|a| AttachmentView {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    mime: a.mime.clone(),
                    size: a.size,
                })
                .collect(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            version,
            changed_elsewhere: false,
        }
    }
}

/// Second line of a list row: username, else the website's host; codes show issuer/account.
pub fn subtitle_of(item_type: ItemType, fields: &BTreeMap<String, String>) -> Option<String> {
    let get = |k: &str| fields.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    match item_type {
        ItemType::Login => instances(fields, "username")
            .iter()
            .map(|(_, _, v)| v.trim())
            .find(|v| !v.is_empty())
            .map(str::to_string)
            .or_else(|| get("url").and_then(host_of)),
        ItemType::Totp => get("account").or_else(|| get("issuer")).map(str::to_string),
        ItemType::Card => {
            let digits: Vec<char> = get("number")
                .map(|n| n.chars().filter(char::is_ascii_digit).collect())
                .unwrap_or_default();
            if digits.len() >= 4 {
                let last4: String = digits[digits.len() - 4..].iter().collect();
                Some(match get("brand") {
                    Some(brand) => format!("{brand} •••• {last4}"),
                    None => format!("•••• {last4}"),
                })
            } else {
                get("cardholder").map(str::to_string)
            }
        }
        ItemType::Identity => {
            let name: Vec<&str> = ["firstName", "middleName", "lastName"]
                .iter()
                .filter_map(|k| get(k))
                .collect();
            if name.is_empty() {
                get("email").map(str::to_string)
            } else {
                Some(name.join(" "))
            }
        }
        // The fingerprint if the person typed one, else the comment of the public key line
        // ("ssh-ed25519 AAAA… ana@laptop" → "ana@laptop").
        ItemType::SshKey => get("fingerprint").map(str::to_string).or_else(|| {
            get("publicKey")
                .and_then(|k| k.lines().next())
                .and_then(|line| line.split_whitespace().nth(2))
                .map(str::to_string)
        }),
        ItemType::SecureNote | ItemType::Media => None,
    }
}

/// The web addresses in a login's `url` field: one per line, trimmed, none empty.
pub fn urls_of(value: &str) -> impl Iterator<Item = &str> {
    value.lines().map(str::trim).filter(|l| !l.is_empty())
}

/// The same list without repeats (the first spelling stays), in the order they were written.
pub fn clean_urls(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in urls_of(value) {
        if !out.iter().any(|seen| seen == u) {
            out.push(u.to_string());
        }
    }
    out
}

/// `https://user@www.example.com:8443/a?b#c` → `example.com`. With several addresses, the first one.
pub fn host_of(url: &str) -> Option<String> {
    let s = urls_of(url).next()?;
    let rest = s.split_once("://").map_or(s, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next()?;
    let host = if let Some(stripped) = hostport.strip_prefix('[') {
        stripped.split(']').next()?
    } else {
        hostport.split(':').next()?
    };
    let host = host.trim();
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then(|| host.to_string())
}

// ------------------------------------------------------------------ wire types (IPC_CONTRACT.md)

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemSummary {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub title: String,
    pub subtitle: Option<String>,
    pub favorite: bool,
    /// The item asks for the master password every time it is revealed, copied, edited or cloned.
    pub reprompt: bool,
    pub tags: Vec<String>,
    pub folder: Option<String>,
    pub has_totp: bool,
    /// Stored keys of the item's one-time codes in order (`totp`, `totp#2` …): what to ask `totp_now` for.
    pub code_keys: Vec<String>,
    /// The login holds at least one passkey.
    pub has_passkey: bool,
    pub created_at: u64,
    pub updated_at: u64,
    pub version: u64,
    /// Archived items are listed (they stay readable) but hidden from the default views by the UI.
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldView {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub secret: bool,
    pub value: Option<String>,
    pub has_value: bool,
    /// Only on a passkey: its public facts (the key stays in the core).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passkey: Option<PasskeyInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomFieldView {
    pub name: String,
    pub hidden: bool,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttachmentView {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub title: String,
    pub favorite: bool,
    pub reprompt: bool,
    pub tags: Vec<String>,
    pub folder: Option<String>,
    pub fields: Vec<FieldView>,
    pub custom_fields: Vec<CustomFieldView>,
    pub attachments: Vec<AttachmentView>,
    pub created_at: u64,
    pub updated_at: u64,
    pub version: u64,
    /// Another device changed this item too, and the copy it sent is in the history ("from
    /// another device") waiting to be looked at. Cleared by `item_review_other_device` or by
    /// restoring such a copy.
    pub changed_elsewhere: bool,
}

/// The trash listing exposes exactly these three values (spec: "ONLY title and timestamp").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    pub id: String,
    pub title: String,
    pub deleted_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionEntry {
    pub version: u64,
    pub timestamp: u64,
    pub event: ItemEvent,
    pub changed_fields: Vec<String>,
    pub restored_from: Option<u64>,
    /// A copy of another device's latest state that arrived while this device kept its own.
    pub other_device: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpNow {
    pub code: String,
    /// The code of the window before this one and the one of the window after it, so the person can see what is
    /// coming while the current code runs out. Withheld exactly like `code` (they come from the same call).
    pub previous: String,
    pub next: String,
    pub period: u64,
    pub digits: u8,
    pub seconds_remaining: u64,
    pub valid_until_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    /// The selected profile's vault file exists (false when no profile is selected).
    pub exists: bool,
    pub unlocked: bool,
    /// The selected profile; `None` when nothing is selected. Filled in by the profile layer.
    pub profile: Option<ProfileInfo>,
    /// Items whose stored data failed authentication when the vault was unlocked (0 normally).
    /// Additive field, not in the original contract.
    pub damaged_items: usize,
    /// Goes up every time a pull changed what is stored (a new item, a fast-forward, a deletion,
    /// an other-device copy). The page re-reads its lists when it differs from the last one it saw.
    /// Starts at 0 for every unlock; in memory only.
    pub sync_revision: u64,
    /// A pull is queued or running right now ("Checking your backup...").
    pub reconciling: bool,
    /// Objects in the backup that failed authentication during the last pull (never applied).
    pub remote_damaged: usize,
    /// Locked, but still open in memory: the account PIN shows it again. Absent when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resumable: bool,
    /// Opened with the PIN: everything sensitive still asks for the master password. Absent when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub limited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginEdit {
    pub token: String,
    pub draft: ItemDraft,
    pub version: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn login(title: &str) -> ItemDraft {
        ItemDraft {
            item_type: ItemType::Login,
            title: title.into(),
            favorite: false,
            reprompt: false,
            tags: vec![],
            folder: None,
            fields: BTreeMap::from([
                ("username".into(), "ana@example.com".into()),
                ("password".into(), "hunter2-hunter2".into()),
                ("url".into(), "https://www.example.com/login".into()),
            ]),
            custom_fields: vec![],
        }
    }

    #[test]
    fn several_web_addresses_are_kept_a_line_each_and_tidied() {
        let mut d = login("A");
        d.fields.insert(
            "url".into(),
            " https://a.example \n\nhttps://b.example\r\nhttps://a.example ".into(),
        );
        d.normalize().unwrap();
        assert_eq!(d.fields["url"], "https://a.example\nhttps://b.example");
        // the list row and the duplicate check use the first address
        assert_eq!(host_of(&d.fields["url"]).as_deref(), Some("a.example"));
        d.fields.remove("username");
        assert_eq!(
            subtitle_of(ItemType::Login, &d.fields).as_deref(),
            Some("a.example")
        );
        // blank lines alone are no address
        d.fields.insert("url".into(), " \n \n".into());
        d.normalize().unwrap();
        assert!(!d.fields.contains_key("url"));
        // and there is a limit
        let many: Vec<String> = (0..=MAX_URLS)
            .map(|i| format!("https://s{i}.example"))
            .collect();
        d.fields.insert("url".into(), many.join("\n"));
        assert!(d.normalize().is_err());
        d.fields.insert("url".into(), many[..MAX_URLS].join("\n"));
        d.normalize().unwrap();
    }

    #[test]
    fn normalize_trims_dedupes_and_drops_empty_values() {
        let mut d = login("  GitHub  ");
        d.tags = vec![" work ".into(), "work".into(), "".into(), "code".into()];
        d.folder = Some("   ".into());
        d.fields.insert("notes".into(), String::new());
        d.normalize().unwrap();
        assert_eq!(d.title, "GitHub");
        assert_eq!(d.tags, vec!["work", "code"]);
        assert_eq!(d.folder, None);
        d.folder = Some(" Work // Clients/ ".into());
        d.normalize().unwrap();
        assert_eq!(d.folder.as_deref(), Some("Work/Clients"));
        assert!(!d.fields.contains_key("notes"));
        assert!(d.fields.contains_key("username"));
    }

    #[test]
    fn normalize_enforces_every_limit() {
        let too_long = |n: usize| "a".repeat(n);
        type Mutation = Box<dyn Fn(&mut ItemDraft)>;
        let cases: Vec<(&str, Mutation)> = vec![
            ("empty title", Box::new(|d| d.title = "   ".into())),
            ("long title", Box::new(move |d| d.title = too_long(257))),
            ("long tag", Box::new(move |d| d.tags = vec![too_long(65)])),
            (
                "too many tags",
                Box::new(|d| d.tags = (0..33).map(|i| format!("t{i}")).collect()),
            ),
            (
                "long folder",
                Box::new(move |d| d.folder = Some(too_long(129))),
            ),
            (
                "huge field",
                Box::new(|d| {
                    d.fields
                        .insert("notes".into(), "x".repeat(MAX_FIELD_BYTES + 1));
                }),
            ),
            (
                "unknown key",
                Box::new(|d| {
                    d.fields.insert("cvv".into(), "123".into());
                }),
            ),
            (
                "too many custom fields",
                Box::new(|d| {
                    d.custom_fields = (0..51)
                        .map(|i| CustomField {
                            name: format!("f{i}"),
                            value: "v".into(),
                            hidden: false,
                        })
                        .collect()
                }),
            ),
            (
                "nameless custom field",
                Box::new(|d| {
                    d.custom_fields = vec![CustomField {
                        name: " ".into(),
                        value: "v".into(),
                        hidden: true,
                    }]
                }),
            ),
            (
                "huge custom field",
                Box::new(|d| {
                    d.custom_fields = vec![CustomField {
                        name: "n".into(),
                        value: "x".repeat(MAX_FIELD_BYTES + 1),
                        hidden: true,
                    }]
                }),
            ),
            (
                "bad totp",
                Box::new(|d| {
                    d.fields.insert("totp".into(), "not base32 !!".into());
                }),
            ),
        ];
        for (name, mutate) in cases {
            let mut d = login("ok");
            mutate(&mut d);
            let e = d.normalize().unwrap_err();
            assert_eq!(e.code(), "invalid_input", "{name}: {e:?}");
        }
        // boundaries that must pass
        let mut d = login(&"a".repeat(256));
        d.tags = (0..32)
            .map(|i| format!("{i:02}{}", "t".repeat(62)))
            .collect();
        d.fields.insert("notes".into(), "x".repeat(MAX_FIELD_BYTES));
        d.normalize().unwrap();
    }

    #[test]
    fn unknown_keys_are_rejected_per_type() {
        let mut note = ItemDraft {
            item_type: ItemType::SecureNote,
            title: "Wifi".into(),
            favorite: false,
            reprompt: false,
            tags: vec![],
            folder: None,
            fields: BTreeMap::from([("password".into(), "x".into())]),
            custom_fields: vec![],
        };
        assert_eq!(note.normalize().unwrap_err().code(), "invalid_input");
        note.fields = BTreeMap::from([("body".into(), "text".into())]);
        note.normalize().unwrap();
    }

    #[test]
    fn totp_items_need_a_valid_secret() {
        let mut d = ItemDraft {
            item_type: ItemType::Totp,
            title: "Acme".into(),
            favorite: false,
            reprompt: false,
            tags: vec![],
            folder: None,
            fields: BTreeMap::new(),
            custom_fields: vec![],
        };
        assert_eq!(d.normalize().unwrap_err().code(), "invalid_input");
        d.fields.insert("totp".into(), "JBSWY3DPEHPK3PXP".into());
        d.normalize().unwrap();
        d.fields.insert(
            "totp".into(),
            "otpauth://totp/Acme:ana@example.com?secret=JBSWY3DPEHPK3PXP&issuer=Acme".into(),
        );
        d.normalize().unwrap();
        d.fields.insert(
            "totp".into(),
            "otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP".into(),
        );
        assert_eq!(d.normalize().unwrap_err().code(), "invalid_input");
    }

    #[test]
    fn subtitle_prefers_username_then_host() {
        let mut f = BTreeMap::from([(
            "url".to_string(),
            "https://user:pw@www.Example.com:8443/a?b#c".to_string(),
        )]);
        assert_eq!(
            subtitle_of(ItemType::Login, &f).as_deref(),
            Some("Example.com")
        );
        f.insert("username".into(), "ana".into());
        assert_eq!(subtitle_of(ItemType::Login, &f).as_deref(), Some("ana"));
        f.clear();
        assert_eq!(subtitle_of(ItemType::Login, &f), None);
        let t = BTreeMap::from([("issuer".to_string(), "Acme".to_string())]);
        assert_eq!(subtitle_of(ItemType::Totp, &t).as_deref(), Some("Acme"));
        let n = BTreeMap::from([("body".to_string(), "secret".to_string())]);
        assert_eq!(subtitle_of(ItemType::SecureNote, &n), None);
    }

    #[test]
    fn host_extraction_edge_cases() {
        assert_eq!(host_of("example.com").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://[::1]:8080/x").as_deref(), Some("::1"));
        assert_eq!(
            host_of("https://a@b@host.test/").as_deref(),
            Some("host.test")
        );
        assert_eq!(host_of("   ").as_deref(), None);
        assert_eq!(host_of("https:///path").as_deref(), None);
    }

    #[test]
    fn view_hides_secret_values_but_shows_plain_ones() {
        let mut d = login("GitHub");
        d.fields.insert("totp".into(), "JBSWY3DPEHPK3PXP".into());
        d.fields.insert("notes".into(), "backup codes".into());
        d.custom_fields = vec![
            CustomField {
                name: "Plan".into(),
                value: "Team".into(),
                hidden: false,
            },
            CustomField {
                name: "PIN".into(),
                value: "4321".into(),
                hidden: true,
            },
        ];
        d.normalize().unwrap();
        let payload = ItemPayload::new("3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f".into(), &d, 1000);
        let view = payload.view(1);
        let json = serde_json::to_string(&view).unwrap();
        for secret in [
            "hunter2-hunter2",
            "JBSWY3DPEHPK3PXP",
            "backup codes",
            "4321",
        ] {
            assert!(!json.contains(secret), "{secret} leaked in the masked view");
        }
        assert!(json.contains("ana@example.com"));
        assert!(json.contains("Team"));
        let keys: Vec<&str> = view.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["username", "password", "url", "totp", "notes"]);
        let pw = &view.fields[1];
        assert!(pw.secret && pw.value.is_none() && pw.has_value);
        assert_eq!(view.custom_fields[1].value, None);
    }

    #[test]
    fn json_shapes_match_the_contract() {
        let mut d = login("GitHub");
        d.normalize().unwrap();
        let p = ItemPayload::new("3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f".into(), &d, 5);
        let s = serde_json::to_value(p.summary(1)).unwrap();
        for k in [
            "id",
            "type",
            "title",
            "subtitle",
            "favorite",
            "reprompt",
            "tags",
            "folder",
            "hasTotp",
            "codeKeys",
            "updatedAt",
            "version",
            "archived",
        ] {
            assert!(s.get(k).is_some(), "summary lacks {k}");
        }
        assert_eq!(s["type"], "login");
        let draft = serde_json::to_value(p.to_draft()).unwrap();
        assert_eq!(draft["type"], "login");
        assert_eq!(
            draft["reprompt"], false,
            "the editable view always carries the flag"
        );
        // The sealed payload carries the key only when it is on: an item that never asked for it is stored
        // exactly as an older build stores it (an older build refuses a payload with a key it does not know).
        let sealed = serde_json::to_value(&p).unwrap();
        assert!(sealed.get("reprompt").is_none());
        let mut flagged = d.clone();
        flagged.reprompt = true;
        let q = ItemPayload::new("3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f".into(), &flagged, 5);
        let sealed = serde_json::to_value(&q).unwrap();
        assert_eq!(sealed["reprompt"], true);
        let back: ItemPayload = serde_json::from_value(sealed).unwrap();
        assert!(back.reprompt && back.summary(1).reprompt && back.view(1).reprompt);
        assert!(draft["customFields"].is_array());
        let st = serde_json::to_value(VaultStatus {
            exists: true,
            unlocked: false,
            profile: None,
            damaged_items: 0,
            sync_revision: 7,
            reconciling: true,
            remote_damaged: 2,
            resumable: false,
            limited: false,
        })
        .unwrap();
        // The old quick-unlock design is gone: no `capabilities` field is sent any more.
        assert!(st.get("capabilities").is_none());
        // the account-PIN flags are sent only when they are on, so a status looks the same as before without a PIN
        assert!(st.get("resumable").is_none() && st.get("limited").is_none());
        let on = serde_json::to_value(VaultStatus {
            exists: true,
            unlocked: true,
            profile: None,
            damaged_items: 0,
            sync_revision: 0,
            reconciling: false,
            remote_damaged: 0,
            resumable: true,
            limited: true,
        })
        .unwrap();
        assert_eq!(on["resumable"], true);
        assert_eq!(on["limited"], true);
        assert_eq!(st["exists"], true);
        assert_eq!(st["syncRevision"], 7);
        assert_eq!(st["reconciling"], true);
        assert_eq!(st["remoteDamaged"], 2);
        assert_eq!(
            serde_json::to_value(ItemEvent::RestoredFromVersion).unwrap(),
            json!("restoredFromVersion")
        );
        assert_eq!(
            serde_json::to_value(ItemType::SecureNote).unwrap(),
            json!("secureNote")
        );
        let t = serde_json::to_value(TrashEntry {
            id: "x".into(),
            title: "t".into(),
            deleted_at: 9,
        })
        .unwrap();
        assert_eq!(t.as_object().unwrap().len(), 3);
    }

    #[test]
    fn drafts_reject_unknown_json_keys() {
        let bad = json!({"type":"login","title":"x","fields":{},"customFields":[],"extra":1});
        assert!(serde_json::from_value::<ItemDraft>(bad).is_err());
        let ok = json!({"type":"login","title":"x"});
        assert!(serde_json::from_value::<ItemDraft>(ok).is_ok());
    }

    #[test]
    fn changed_fields_names_only() {
        let a = login("GitHub");
        let mut b = a.clone();
        assert!(changed_fields(&a, &b).is_empty());
        b.title = "GitHub 2".into();
        b.fields
            .insert("password".into(), "new-secret-value".into());
        b.fields.remove("url");
        b.tags = vec!["work".into()];
        b.custom_fields.push(CustomField {
            name: "n".into(),
            value: "v".into(),
            hidden: false,
        });
        let c = changed_fields(&a, &b);
        assert_eq!(c, ["title", "tags", "password", "url", "customFields"]);
        assert!(!c.iter().any(|s| s.contains("secret")));
    }

    // ------------------------------------------------------------ several sign-ins and codes

    const SEED_A: &str = "JBSWY3DPEHPK3PXP";
    const SEED_B: &str = "ONSWG4TFOQYTEMZU";

    #[test]
    fn instance_keys_have_one_canonical_spelling() {
        assert_eq!(parse_key("totp"), Some(("totp", 1)));
        assert_eq!(parse_key("totp#2"), Some(("totp", 2)));
        assert_eq!(parse_key("totp#10"), Some(("totp", 10)));
        for bad in [
            "totp#",
            "totp#1",
            "totp#0",
            "totp#02",
            "totp#-2",
            "totp#2a",
            "totp#1000",
            "totp##2",
        ] {
            assert_eq!(parse_key(bad), None, "{bad}");
        }
        assert_eq!(instance_key("totp", 1), "totp");
        assert_eq!(instance_key("totp", 4), "totp#4");
    }

    #[test]
    fn only_sign_ins_and_codes_of_a_login_repeat() {
        let l = ItemType::Login;
        for ok in ["username#2", "password#10", "totp#3"] {
            assert!(l.spec(ok).is_some(), "{ok}");
        }
        // not repeatable, past the limit, or not even a field
        for bad in ["url#2", "notes#2", "totp#11", "username#1", "nope#2", "#2"] {
            assert!(l.spec(bad).is_none(), "{bad}");
        }
        // a code-only item keeps exactly one code; cards and notes never repeat
        assert!(ItemType::Totp.spec("totp#2").is_none());
        assert!(ItemType::Card.spec("number#2").is_none());
        assert_eq!(l.spec("password#4").unwrap().key, "password");
    }

    fn two_sign_ins() -> ItemDraft {
        let mut d = login("Amazon");
        d.fields
            .insert("username#2".into(), "work@example.com".into());
        d.fields
            .insert("password#2".into(), "work-pw-123456".into());
        d.fields.insert("totp".into(), SEED_A.into());
        d.fields.insert("totp#2".into(), SEED_B.into());
        d
    }

    #[test]
    fn a_login_keeps_several_sign_ins_and_codes() {
        let mut d = two_sign_ins();
        d.normalize().unwrap();
        assert_eq!(d.fields.len(), 7);
        // every code is checked, not just the first
        d.fields.insert("totp#3".into(), "not a code!".into());
        assert_eq!(d.normalize().unwrap_err().code(), "invalid_input");
        d.fields.remove("totp#3");
        // an unknown or non-canonical key is refused whatever its value
        for bad in ["url#2", "password#1", "password#11", "color"] {
            let mut e = two_sign_ins();
            e.fields.insert(bad.into(), "x".into());
            assert_eq!(e.normalize().unwrap_err().code(), "invalid_input", "{bad}");
        }
        // a blank second value is dropped like any blank field
        d.fields.insert("password#2".into(), String::new());
        d.normalize().unwrap();
        assert!(!d.fields.contains_key("password#2"));
    }

    #[test]
    fn the_view_lists_each_instance_in_order_and_hides_secrets() {
        let mut d = two_sign_ins();
        d.normalize().unwrap();
        let p = ItemPayload::new("id".into(), &d, 1);
        let v = p.view(1);
        let seen: Vec<(&str, &str)> = v
            .fields
            .iter()
            .map(|f| (f.key.as_str(), f.label.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                ("username", "Username 1"),
                ("username#2", "Username 2"),
                ("password", "Password 1"),
                ("password#2", "Password 2"),
                ("url", "Website"),
                ("totp", "One-time code 1"),
                ("totp#2", "One-time code 2"),
            ]
        );
        for f in &v.fields {
            assert_eq!(f.value.is_some(), !f.secret, "{}", f.key);
        }
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("work-pw-123456") && !json.contains(SEED_B));
        // a password alone in sign-in 2 is still called "Password 2": the number says which sign-in it belongs to
        let mut half = login("Half");
        half.fields.remove("password");
        half.fields
            .insert("username#2".into(), "work@example.com".into());
        half.fields
            .insert("password#2".into(), "work-pw-123456".into());
        let p = ItemPayload::new("id".into(), &half, 1);
        let seen: Vec<(String, String)> = p
            .view(1)
            .fields
            .into_iter()
            .map(|f| (f.key, f.label))
            .collect();
        assert_eq!(
            seen,
            [
                ("username".to_string(), "Username 1".to_string()),
                ("username#2".to_string(), "Username 2".to_string()),
                ("password#2".to_string(), "Password 2".to_string()),
                ("url".to_string(), "Website".to_string()),
            ]
        );
        // a lone `totp#2` is still the only code: its label has no number
        let mut lone = login("Lone");
        lone.fields.insert("totp#2".into(), SEED_B.into());
        let p = ItemPayload::new("id".into(), &lone, 1);
        let f = p
            .view(1)
            .fields
            .into_iter()
            .find(|f| f.key == "totp#2")
            .unwrap();
        assert_eq!(f.label, "One-time code");
        assert!(p.summary(1).has_totp);
    }

    #[test]
    fn the_first_stored_username_is_the_list_subtitle() {
        let mut d = login("A");
        d.fields.remove("username");
        d.fields
            .insert("username#2".into(), "second@example.com".into());
        assert_eq!(
            subtitle_of(ItemType::Login, &d.fields).as_deref(),
            Some("second@example.com")
        );
        d.fields
            .insert("username".into(), "first@example.com".into());
        assert_eq!(
            subtitle_of(ItemType::Login, &d.fields).as_deref(),
            Some("first@example.com")
        );
    }

    #[test]
    fn changed_fields_name_the_instance_that_changed() {
        let a = two_sign_ins();
        let mut b = a.clone();
        b.fields
            .insert("password#2".into(), "changed-pw-123456".into());
        b.fields.remove("totp#2");
        b.fields.insert("totp#3".into(), SEED_A.into());
        assert_eq!(changed_fields(&a, &b), ["password#2", "totp#2", "totp#3"]);
        // order is schema order, then instance number, so #10 comes after #9
        let mut c = login("C");
        c.fields.insert("totp#10".into(), SEED_A.into());
        c.fields.insert("totp#9".into(), SEED_B.into());
        assert_eq!(changed_fields(&login("C"), &c), ["totp#9", "totp#10"]);
    }
}
