//! What the vault does for passkeys and for the scanner: find the logins a new code or passkey probably belongs to,
//! save a passkey (on an existing login or on a new one), find the passkeys for a website, add a one-time code.
//!
//! This file is a child module of `vault.rs` (`#[path]` declaration there), so it can use the vault's private index and
//! its sealing helpers. Everything here works on an unlocked vault; the password rules (a login that asks for the master
//! password again, a vault opened with the PIN) are applied by the session before any of it runs.
//!
//! **How a login is matched** (the one rule, so a test can state it). A login scores points for each of these it shares
//! with what was scanned: the same account (an email address or user name equal to one of the login's user names, 100),
//! the same website (the login's address is the website, a part of it, or a site it belongs to, or the issuer's name is
//! a word of that address: `GitHub` and `github.com`; 60), the same name (the issuer or site name is in the login's
//! title or the title is in it, 40). Only logins with points are suggested, best
//! first. A login is never matched by anything secret.

use super::{IndexEntry, Vault};
use crate::crypto::now_millis;
use crate::passkey::{PasskeyInfo, PasskeyRecord};
use crate::storage::error::{Result, StorageError};
use crate::storage::model::{
    host_of, instance_key, instances, totp_account, urls_of, FieldKind, ItemDraft, ItemPayload,
    ItemSummary, ItemType, MAX_INSTANCES, MAX_TITLE_CHARS,
};
use crate::storage::uuid;
use serde::Serialize;

/// How many logins one suggestion list holds.
pub const MAX_SUGGESTIONS: usize = 8;

const POINTS_ACCOUNT: u32 = 100;
const POINTS_SITE: u32 = 60;
const POINTS_NAME: u32 = 40;

/// What was scanned, as far as it helps to find the login it belongs to. All text is compared lower-cased.
#[derive(Debug, Clone, Default)]
pub struct Hints {
    /// Email addresses or user names (the account of a code, the user name of a passkey).
    pub accounts: Vec<String>,
    /// Website names: `example.com`.
    pub sites: Vec<String>,
    /// Names: the issuer of a code, the name of a website.
    pub names: Vec<String>,
    /// The secret of a scanned one-time code (as `account_key_secret` spells it), to tell a login that has it already.
    pub secret: Option<String>,
}

/// One login that may be the place a scanned code or passkey belongs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// Why: `account`, `site`, `name` (strongest first).
    pub reasons: Vec<String>,
    pub score: u32,
    /// How many one-time codes and passkeys it holds now.
    pub codes: usize,
    pub passkeys: usize,
    /// This exact one-time code is on it already.
    pub has_this_code: bool,
    /// It already holds a passkey for the website asked about.
    pub has_site_passkey: bool,
    /// No slot is left for one more code or passkey.
    pub full: bool,
    pub reprompt: bool,
}

/// A passkey the vault holds, with where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyMatch {
    pub item_id: String,
    /// The field it is in (`passkey`, `passkey#2` …).
    pub key: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub reprompt: bool,
    #[serde(flatten)]
    pub info: PasskeyInfo,
}

/// Where a new passkey or code goes.
#[derive(Debug, Clone)]
pub enum Target {
    Existing(String),
    New { title: Option<String> },
}

/// `a.example.com` and `example.com` are the same site for this purpose (either may be the parent of the other).
fn related_sites(a: &str, b: &str) -> bool {
    a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
}

fn lower(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The first of `a_keys` free in a numbered field, or `None` when all `MAX_INSTANCES` are used.
pub(in crate::storage) fn free_slot(
    fields: &std::collections::BTreeMap<String, String>,
    base: &str,
) -> Option<String> {
    (1..=MAX_INSTANCES)
        .map(|n| instance_key(base, n))
        .find(|k| !fields.contains_key(k))
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_TITLE_CHARS).collect()
}

impl Vault {
    /// Every active login, opened. A damaged one is skipped (the vault reports it elsewhere).
    fn each_login(&self, mut f: impl FnMut(&ItemPayload, &ItemSummary)) -> Result<()> {
        for row in self.store.all_item_rows()? {
            let Some(IndexEntry::Active(summary)) = self.index.get(&row.id) else {
                continue;
            };
            if summary.archived {
                continue;
            }
            let Ok(payload) = self.open_row(&row) else {
                continue;
            };
            if payload.item_type == ItemType::Login {
                f(&payload, summary);
            }
        }
        Ok(())
    }

    /// The logins a scanned code or passkey probably belongs to, best first (see the module notes).
    pub fn suggest_logins(&self, hints: &Hints) -> Result<Vec<Suggestion>> {
        let accounts: Vec<String> = hints
            .accounts
            .iter()
            .map(|a| lower(a))
            .filter(|a| !a.is_empty())
            .collect();
        let sites: Vec<String> = hints
            .sites
            .iter()
            .map(|a| lower(a))
            .filter(|a| !a.is_empty())
            .collect();
        let names: Vec<String> = hints
            .names
            .iter()
            .map(|a| lower(a))
            .filter(|a| a.chars().count() >= 3)
            .collect();
        let mut out: Vec<Suggestion> = Vec::new();
        self.each_login(|p, summary| {
            let usernames: Vec<String> = instances(&p.fields, "username")
                .iter()
                .map(|(_, _, v)| lower(v))
                .collect();
            let hosts: Vec<String> = p
                .fields
                .get("url")
                .map(|u| {
                    urls_of(u)
                        .filter_map(host_of)
                        .map(|h| h.to_lowercase())
                        .collect()
                })
                .unwrap_or_default();
            let title = lower(&p.title);
            let mut reasons: Vec<String> = Vec::new();
            let mut score = 0;
            if accounts.iter().any(|a| usernames.contains(a)) {
                reasons.push("account".into());
                score += POINTS_ACCOUNT;
            }
            // an issuer such as "GitHub" names the site a login's address points to: `github.com` has the label `github`
            let host_labels: Vec<&str> = hosts
                .iter()
                .flat_map(|h| {
                    let parts: Vec<&str> = h.split('.').collect();
                    parts[..parts.len().saturating_sub(1)].to_vec()
                })
                .filter(|l| l.chars().count() >= 3)
                .collect();
            let by_name = names
                .iter()
                .any(|n| host_labels.contains(&n.replace(' ', "").as_str()));
            if by_name
                || sites
                    .iter()
                    .any(|s| hosts.iter().any(|h| related_sites(h, s)))
            {
                reasons.push("site".into());
                score += POINTS_SITE;
            }
            if title.chars().count() >= 3
                && names
                    .iter()
                    .any(|n| title.contains(n.as_str()) || n.contains(title.as_str()))
            {
                reasons.push("name".into());
                score += POINTS_NAME;
            }
            let has_this_code = hints.secret.as_deref().is_some_and(|secret| {
                !secret.is_empty()
                    && instances(&p.fields, "totp")
                        .iter()
                        .any(|(_, _, v)| super::import::account_key_secret(v) == secret)
            });
            if score == 0 && !has_this_code {
                return;
            }
            let passkeys: Vec<PasskeyInfo> = instances(&p.fields, "passkey")
                .iter()
                .filter_map(|(_, _, v)| PasskeyRecord::parse(v).ok().map(|r| r.info()))
                .collect();
            let codes = instances(&p.fields, "totp").len();
            out.push(Suggestion {
                id: p.id.clone(),
                title: p.title.clone(),
                subtitle: summary.subtitle.clone(),
                reasons,
                score,
                codes,
                passkeys: passkeys.len(),
                has_this_code,
                has_site_passkey: passkeys.iter().any(|k| sites.contains(&k.rp_id)),
                full: codes >= MAX_INSTANCES as usize && passkeys.len() >= MAX_INSTANCES as usize,
                reprompt: p.reprompt,
            });
        })?;
        out.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        out.truncate(MAX_SUGGESTIONS);
        Ok(out)
    }

    /// The passkeys for a website, newest first. With a non-empty `allow` only those credential ids.
    pub fn passkeys_for(&self, rp_id: &str, allow: &[Vec<u8>]) -> Result<Vec<PasskeyMatch>> {
        let mut out: Vec<PasskeyMatch> = Vec::new();
        self.each_login(|p, summary| {
            for (_, key, value) in instances(&p.fields, "passkey") {
                let Ok(record) = PasskeyRecord::parse(value) else {
                    continue;
                };
                if record.rp_id != rp_id {
                    continue;
                }
                if !allow.is_empty() && !allow.iter().any(|id| *id == record.credential_id_bytes())
                {
                    continue;
                }
                out.push(PasskeyMatch {
                    item_id: p.id.clone(),
                    key: key.to_string(),
                    title: p.title.clone(),
                    subtitle: summary.subtitle.clone(),
                    reprompt: p.reprompt,
                    info: record.info(),
                });
            }
        })?;
        out.sort_by(|a, b| {
            b.info
                .created_at
                .cmp(&a.info.created_at)
                .then_with(|| a.title.cmp(&b.title))
        });
        Ok(out)
    }

    /// Is this credential id already held, for this website? (`excludeCredentials` of a registration.)
    pub fn holds_passkey(&self, rp_id: &str, ids: &[Vec<u8>]) -> Result<bool> {
        if ids.is_empty() {
            return Ok(false);
        }
        Ok(!self.passkeys_for(rp_id, ids)?.is_empty())
    }

    /// The stored passkey in field `key` of item `id`.
    pub fn passkey_record(&self, id: &str, key: &str) -> Result<PasskeyRecord> {
        self.require_active(id)?;
        let (payload, _) = self.load_latest(id)?;
        let is_passkey = payload
            .item_type
            .spec(key)
            .is_some_and(|s| s.kind == FieldKind::Passkey);
        let value = payload
            .fields
            .get(key)
            .filter(|_| is_passkey)
            .ok_or(StorageError::NotFound)?;
        PasskeyRecord::parse(value).map_err(|e| StorageError::invalid(e.to_string()))
    }

    /// Save a new passkey on an existing login or on a new one. A passkey for the same website and the same account
    /// (user id) that the login already holds is replaced, as a new registration replaces the old one in the spec.
    pub fn save_passkey(&mut self, record: &PasskeyRecord, target: &Target) -> Result<ItemSummary> {
        let value = record.to_json().to_string();
        match target {
            Target::Existing(id) => {
                self.require_active(id)?;
                let (old, version) = self.load_latest(id)?;
                if old.item_type != ItemType::Login {
                    return Err(StorageError::invalid(
                        "A passkey can only be saved on a login.",
                    ));
                }
                let mut draft = old.to_draft();
                let same_account = instances(&draft.fields, "passkey")
                    .iter()
                    .find(|(_, _, v)| {
                        PasskeyRecord::parse(v).is_ok_and(|r| {
                            r.rp_id == record.rp_id && r.user_handle == record.user_handle
                        })
                    })
                    .map(|(_, k, _)| k.to_string());
                let key = match same_account.or_else(|| free_slot(&draft.fields, "passkey")) {
                    Some(k) => k,
                    None => {
                        return Err(StorageError::invalid(format!(
                            "This login already holds {MAX_INSTANCES} passkeys."
                        )))
                    }
                };
                draft.fields.insert(key, value);
                // the site the passkey is for helps to find this login again
                let known = draft.fields.get("url").is_some_and(|u| {
                    urls_of(u)
                        .filter_map(host_of)
                        .any(|h| related_sites(&h.to_lowercase(), &record.rp_id))
                });
                if !known {
                    let url = format!("https://{}", record.rp_id);
                    let merged = match draft.fields.get("url") {
                        Some(u) if !u.is_empty() => format!("{u}\n{url}"),
                        _ => url,
                    };
                    draft.fields.insert("url".into(), merged);
                }
                self.write_update(old, version, draft)
            }
            Target::New { title } => {
                let mut draft =
                    new_login(title.as_deref().unwrap_or(&record.rp_name), &record.rp_id);
                if !record.user_name.is_empty() {
                    draft
                        .fields
                        .insert("username".into(), record.user_name.clone());
                }
                draft.fields.insert("passkey".into(), value);
                self.create_with_passkeys(draft)
            }
        }
    }

    /// `create_item` for a draft the core made itself, so it may carry a passkey.
    fn create_with_passkeys(&mut self, mut draft: ItemDraft) -> Result<ItemSummary> {
        let id = uuid::new_v4()?;
        draft.normalize()?;
        let payload = ItemPayload::new(id, &draft, now_millis());
        self.commit(payload, None, 1)
    }

    /// Add a one-time code (an `otpauth://` link or a bare key) to an existing login, or make a login for it.
    pub fn save_code(
        &mut self,
        value: &str,
        target: &Target,
        facts: &CodeFacts,
    ) -> Result<ItemSummary> {
        totp_account(value)?; // a code that does not work is never saved
        match target {
            Target::Existing(id) => {
                self.require_active(id)?;
                let (old, version) = self.load_latest(id)?;
                if old.item_type != ItemType::Login {
                    return Err(StorageError::invalid(
                        "A one-time code can only be added to a login.",
                    ));
                }
                let mut draft = old.to_draft();
                let secret = super::import::account_key_secret(value);
                if instances(&draft.fields, "totp")
                    .iter()
                    .any(|(_, _, v)| super::import::account_key_secret(v) == secret)
                {
                    return Err(StorageError::invalid(
                        "This login already has this one-time code.",
                    ));
                }
                let key = free_slot(&draft.fields, "totp").ok_or_else(|| {
                    StorageError::invalid(format!(
                        "This login already holds {MAX_INSTANCES} one-time codes."
                    ))
                })?;
                draft.fields.insert(key, value.to_string());
                self.write_update(old, version, draft)
            }
            Target::New { title } => {
                let name = title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .or_else(|| facts.issuer.clone())
                    .or_else(|| (!facts.account.is_empty()).then(|| facts.account.clone()))
                    .unwrap_or_else(|| "New login".into());
                let mut draft = new_login(&name, "");
                if !facts.account.is_empty() {
                    draft
                        .fields
                        .insert("username".into(), facts.account.clone());
                }
                draft.fields.insert("totp".into(), value.to_string());
                self.create_item(draft)
            }
        }
    }
}

/// What a scanned code says about itself, for naming a new login.
#[derive(Debug, Clone, Default)]
pub struct CodeFacts {
    pub issuer: Option<String>,
    pub account: String,
}

/// A login with a title and, when there is one, its website (`https://<site>`).
fn new_login(title: &str, site: &str) -> ItemDraft {
    let mut fields = std::collections::BTreeMap::new();
    if !site.is_empty() {
        fields.insert("url".to_string(), format!("https://{site}"));
    }
    ItemDraft {
        item_type: ItemType::Login,
        title: clip(title.trim()),
        favorite: false,
        reprompt: false,
        tags: Vec::new(),
        folder: None,
        fields,
        custom_fields: Vec::new(),
    }
}
