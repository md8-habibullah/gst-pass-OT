//! Search inside the items, in Rust, because only here can they be read.
//!
//! The screen already filters by what an item summary holds (title, subtitle, folder, tags). This goes further: every
//! field an item has, the names of its custom fields, and (when asked) the versions it had before. The answer is a list
//! of ids and the *names* of the fields that matched, never a value, so a match inside a note shows "Notes" and not the
//! note.
//!
//! **What is searched, and what never is** (the one rule, kept as data so a test can state it):
//! * Titles, folders, tags, the type's name, every field that is not secret (username, website, brand, name, address),
//!   the names of custom fields and the values of custom fields that are not hidden.
//! * The free-text fields people write notes in (`notes`, `body`, `description`), because finding the note about the
//!   Wi-Fi at the cabin is what a note search is for. Not for an item that asks for the master password again: its
//!   notes stay sealed until the password is typed, search included.
//! * Never a password, card number, security code or PIN, identity number, one-time code secret, private key or
//!   passphrase, and never the value of a hidden custom field. Their *labels* count as words ("passport" finds an
//!   identity that has a passport number) but their values are never compared.
//!
//! Words are matched one by one, in any order and in any field: `github work` finds the item called GitHub in the folder
//! Work. Text is compared lower-cased. Nothing that was read is kept: payloads wipe themselves when dropped, and the
//! lower-cased copies used for comparing are wiped too.
//!
//! Old versions: with `old_versions` the earlier states of every active item are read as well, under the same rules, so
//! "the password manager I used to call X" can be found by a name an edit has since replaced. An item whose current
//! text matches is reported once, as a current match.
//!
//! This file is a child module of `vault.rs` (`#[path]` declaration there), so it can use the vault's private index.

use super::{IndexEntry, Vault};
use crate::storage::error::{Result, StorageError};
use crate::storage::model::{FieldKind, ItemPayload, ItemType};
use serde::Serialize;
use zeroize::Zeroizing;

/// The longest query that is looked at, in characters.
pub const MAX_QUERY_CHARS: usize = 200;
/// How many words of a query are used.
pub const MAX_WORDS: usize = 8;
/// The most items one search reports.
pub const MAX_HITS: usize = 1_000;
/// How many field names a hit lists.
const MAX_MATCHED: usize = 3;

/// The secret-kind fields whose text is a note a person wrote and wants to find again.
const NOTE_KEYS: [&str; 3] = ["notes", "body", "description"];

/// One item the search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: String,
    /// The names of the fields the words were found in (at most three), for example `["Website", "Notes"]`.
    pub matched: Vec<String>,
    /// `Some(n)`: the item as it is now does not match, but its version `n` did.
    pub old_version: Option<u64>,
}

/// What a person may type or the core may compare, for one field.
struct Piece {
    /// What the hit shows when this field matched.
    label: String,
    /// The label and (when the field may be searched) its value, lower-cased and wiped on drop.
    hay: Zeroizing<String>,
}

fn type_name(t: ItemType) -> &'static str {
    match t {
        ItemType::Login => "Login",
        ItemType::Card => "Card",
        ItemType::Identity => "Identity",
        ItemType::SshKey => "SSH key",
        ItemType::SecureNote => "Secure note",
        ItemType::Totp => "Code only",
        ItemType::Media => "Media",
    }
}

fn piece(label: &str, value: Option<&str>) -> Piece {
    let mut hay = Zeroizing::new(label.to_lowercase());
    if let Some(v) = value {
        hay.push('\n');
        hay.push_str(&v.to_lowercase());
    }
    Piece {
        label: label.to_string(),
        hay,
    }
}

/// May the *value* of this field be compared? (The label always may.)
fn value_searchable(kind: FieldKind, key: &str, reprompt: bool) -> bool {
    match kind {
        FieldKind::Text | FieldKind::Url | FieldKind::Paragraph => true,
        FieldKind::Note => NOTE_KEYS.contains(&key) && !reprompt,
        FieldKind::Password | FieldKind::Totp => false,
    }
}

/// Everything about one state of an item that a search may look at.
fn pieces_of(p: &ItemPayload, reprompt: bool) -> Vec<Piece> {
    let mut out = vec![
        piece("Title", Some(&p.title)),
        piece("Type", Some(type_name(p.item_type))),
    ];
    if let Some(f) = &p.folder {
        out.push(piece("Folder", Some(f)));
    }
    for t in &p.tags {
        out.push(piece("Tag", Some(t)));
    }
    for (key, value) in &p.fields {
        let Some(spec) = p.item_type.spec(key) else {
            continue;
        };
        let value = value_searchable(spec.kind, key, reprompt).then_some(value.as_str());
        out.push(piece(spec.label, value));
    }
    for c in &p.custom_fields {
        let label = format!("Custom field: {}", c.name);
        out.push(piece(&label, (!c.hidden).then_some(c.value.as_str())));
    }
    out
}

/// The lower-cased words of a query.
fn words_of(query: &str) -> Result<Vec<String>> {
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(StorageError::invalid("That search is too long."));
    }
    Ok(query
        .to_lowercase()
        .split_whitespace()
        .take(MAX_WORDS)
        .map(str::to_string)
        .collect())
}

/// The field names the words were found in, or `None` when some word was found nowhere.
fn matches(pieces: &[Piece], words: &[String]) -> Option<Vec<String>> {
    let mut labels: Vec<String> = Vec::new();
    for w in words {
        let found = pieces.iter().find(|p| p.hay.contains(w.as_str()))?;
        if labels.len() < MAX_MATCHED && !labels.contains(&found.label) {
            labels.push(found.label.clone());
        }
    }
    Some(labels)
}

impl Vault {
    /// Search the active items (see the module notes for what is and is not compared). `limited` (the vault was opened
    /// with the account PIN) searches as if every item asked for the master password again: no note text is compared.
    pub fn search(&self, query: &str, old_versions: bool, limited: bool) -> Result<Vec<SearchHit>> {
        let words = words_of(query)?;
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits: Vec<SearchHit> = Vec::new();
        for row in self.store.all_item_rows()? {
            let reprompt = match self.index.get(&row.id) {
                Some(IndexEntry::Active(s)) => s.reprompt || limited,
                _ => continue, // trashed, or unreadable: never searched
            };
            // A damaged item is skipped here exactly as at unlock; the vault reports it elsewhere.
            let Ok(payload) = self.open_row(&row) else {
                continue;
            };
            if let Some(matched) = matches(&pieces_of(&payload, reprompt), &words) {
                hits.push(SearchHit {
                    id: row.id.clone(),
                    matched,
                    old_version: None,
                });
            } else if old_versions {
                if let Some(hit) = self.search_history(&row.id, row.version, reprompt, &words) {
                    hits.push(hit);
                }
            }
            if hits.len() >= MAX_HITS {
                break;
            }
        }
        hits.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(hits)
    }

    /// The newest earlier state of `id` whose text matches.
    fn search_history(
        &self,
        id: &str,
        head: u64,
        reprompt: bool,
        words: &[String],
    ) -> Option<SearchHit> {
        let mut rows = self.store.version_rows(id).ok()?;
        rows.sort_by(|a, b| b.version.cmp(&a.version).then(b.ts.cmp(&a.ts)));
        for r in rows.iter().filter(|r| r.version != head) {
            let Ok((payload, _)) = self.open_snapshot(id, r.version, &r.blob) else {
                continue;
            };
            if let Some(matched) = matches(&pieces_of(&payload, reprompt), words) {
                return Some(SearchHit {
                    id: id.to_string(),
                    matched,
                    old_version: Some(r.version),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::model::{CustomField, ItemDraft};
    use std::collections::BTreeMap;

    fn payload(item_type: ItemType, fields: &[(&str, &str)]) -> ItemPayload {
        let draft = ItemDraft {
            item_type,
            title: "Cabin".into(),
            favorite: false,
            reprompt: false,
            tags: vec!["holiday".into()],
            folder: Some("Home/Trips".into()),
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
            custom_fields: vec![
                CustomField {
                    name: "Gate code".into(),
                    value: "4417".into(),
                    hidden: false,
                },
                CustomField {
                    name: "Alarm".into(),
                    value: "zebra-pass".into(),
                    hidden: true,
                },
            ],
        };
        ItemPayload::new("00000000-0000-4000-8000-000000000000".into(), &draft, 1)
    }

    fn found(p: &ItemPayload, reprompt: bool, q: &str) -> Option<Vec<String>> {
        matches(&pieces_of(p, reprompt), &words_of(q).unwrap())
    }

    #[test]
    fn words_match_in_any_order_and_any_field() {
        let p = payload(
            ItemType::Login,
            &[
                ("username", "me@example.com"),
                ("url", "https://git.example.org\nhttps://alt.example.net"),
            ],
        );
        assert!(found(&p, false, "cabin").is_some());
        assert!(found(&p, false, "EXAMPLE.org cabin").is_some());
        assert!(
            found(&p, false, "alt.example.net").is_some(),
            "the second website of a login is searched too"
        );
        assert!(found(&p, false, "holiday trips").is_some());
        assert!(found(&p, false, "cabin nowhere").is_none());
        assert!(
            found(&p, false, "   ").is_some(),
            "no words match everything; the caller turns that into no query"
        );
    }

    #[test]
    fn every_sign_in_is_searched_but_no_password_or_code() {
        let p = payload(
            ItemType::Login,
            &[
                ("username", "first@example.com"),
                ("username#2", "second@example.com"),
                ("password#2", "hunter2-second-horse"),
                ("totp#2", "ONSWG4TFOQYTEMZU"),
            ],
        );
        assert_eq!(found(&p, false, "second@example").unwrap(), ["Username"]);
        assert!(found(&p, false, "hunter2-second").is_none());
        assert!(found(&p, false, "ONSWG4TFOQYTEMZU").is_none());
    }

    #[test]
    fn the_hit_names_fields_never_values() {
        let p = payload(ItemType::Login, &[("username", "needle@example.com")]);
        let m = found(&p, false, "needle").unwrap();
        assert_eq!(m, ["Username"]);
        assert!(m.iter().all(|l| !l.contains("needle")));
    }

    #[test]
    fn notes_are_searched_but_secrets_never() {
        let p = payload(
            ItemType::Login,
            &[
                ("password", "hunter2-correct-horse"),
                ("totp", "JBSWY3DPEHPK3PXP"),
                ("notes", "the wifi is under the stairs"),
            ],
        );
        assert_eq!(found(&p, false, "under the stairs").unwrap(), ["Notes"]);
        assert!(
            found(&p, false, "hunter2").is_none(),
            "a password is never compared"
        );
        assert!(
            found(&p, false, "JBSWY3DPEHPK3PXP").is_none(),
            "a code secret is never compared"
        );

        let card = payload(
            ItemType::Card,
            &[
                ("number", "4111111111111111"),
                ("code", "123"),
                ("pin", "9876"),
                ("brand", "Visa"),
            ],
        );
        assert!(found(&card, false, "4111111111111111").is_none());
        assert!(found(&card, false, "9876").is_none());
        assert!(found(&card, false, "visa").is_some());

        let ssh = payload(
            ItemType::SshKey,
            &[
                (
                    "privateKey",
                    "-----BEGIN OPENSSH PRIVATE KEY----- b3BlbnNzaC1rZXk",
                ),
                ("passphrase", "sesame"),
            ],
        );
        assert!(
            found(&ssh, false, "b3BlbnNzaC1rZXk").is_none(),
            "a private key is never compared"
        );
        assert!(found(&ssh, false, "sesame").is_none());
    }

    #[test]
    fn a_label_counts_as_a_word_but_its_secret_value_does_not() {
        let id = payload(
            ItemType::Identity,
            &[("passport", "X1234567"), ("firstName", "Ada")],
        );
        assert_eq!(found(&id, false, "passport").unwrap(), ["Passport number"]);
        assert!(found(&id, false, "X1234567").is_none());
        let empty = payload(ItemType::Identity, &[("firstName", "Ada")]);
        assert!(
            found(&empty, false, "passport").is_none(),
            "no passport number, so nothing to find"
        );
    }

    #[test]
    fn an_item_that_asks_again_keeps_its_notes_sealed_from_search() {
        let p = payload(
            ItemType::SecureNote,
            &[("body", "the safe combination is thirty-one")],
        );
        assert!(found(&p, false, "combination").is_some());
        assert!(found(&p, true, "combination").is_none());
        // but its title, folder and tags are as findable as ever
        assert!(found(&p, true, "cabin").is_some());
        assert!(found(&p, true, "trips").is_some());
    }

    #[test]
    fn custom_fields_by_name_and_visible_value_only() {
        let p = payload(ItemType::Login, &[]);
        assert_eq!(
            found(&p, false, "gate").unwrap(),
            ["Custom field: Gate code"]
        );
        assert_eq!(
            found(&p, false, "4417").unwrap(),
            ["Custom field: Gate code"]
        );
        assert!(
            found(&p, false, "alarm").is_some(),
            "the name of a hidden field is searchable"
        );
        assert!(found(&p, false, "zebra").is_none(), "its value is not");
    }

    #[test]
    fn the_type_is_a_word_too() {
        let p = payload(ItemType::SshKey, &[]);
        assert!(found(&p, false, "ssh").is_some());
        assert!(found(&p, false, "login").is_none());
    }

    #[test]
    fn a_long_query_is_refused_and_the_words_are_capped() {
        assert!(words_of(&"a".repeat(MAX_QUERY_CHARS + 1)).is_err());
        assert!(words_of(&"a".repeat(MAX_QUERY_CHARS)).is_ok());
        let many = (0..20)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(words_of(&many).unwrap().len(), MAX_WORDS);
    }

    #[test]
    fn at_most_three_field_names_are_listed() {
        let p = payload(
            ItemType::Login,
            &[("username", "zz"), ("url", "zz.example"), ("notes", "zz")],
        );
        let m = found(&p, false, "zz cabin holiday trips login").unwrap();
        assert!(m.len() <= MAX_MATCHED);
    }
}
