//! Passkeys and the scanner on an unlocked session (`ai-docs/PASSKEYS.md`).
//!
//! This file is a child module of `session.rs` (`#[path]` declaration there), so it can reach the private [`Unlocked`]
//! state. The rules it keeps, which the tests state:
//!
//! * a passkey's private key never leaves the core: what goes out is the signed answer of a website's request;
//! * using a passkey (signing) or adding to an existing login needs what any change to that login needs: nothing
//!   extra for an ordinary login in a vault opened with the master password, the master password for a login that asks
//!   for it and for every login while the vault was opened with the PIN (`password_required`);
//! * a new passkey is saved before its answer is handed back, so a registration can never succeed without a stored key;
//! * a website that lists a passkey the vault already holds in `excludeCredentials` gets `passkey_excluded`.

use super::Session;
use crate::passkey::json::{self, CreateRequest, GetRequest};
use crate::passkey::origin::{self, parse_asset_links};
use crate::passkey::{b64, PasskeyError};
use crate::scan::{classify, Scanned};
use crate::storage::error::{Result, StorageError};
use crate::storage::model::ItemSummary;
use crate::storage::vault::passkeys::{CodeFacts, Hints, PasskeyMatch, Suggestion, Target};
use serde::{Deserialize, Serialize};

/// How the website's own app list is fetched. Tests pass a fake; the app passes [`origin::fetch_asset_links`].
pub type LinksFn<'a> = &'a dyn Fn(&str) -> std::result::Result<String, PasskeyError>;

/// Who is asking for the passkey.
#[derive(Debug, Clone)]
pub enum Caller {
    /// A browser the system trusts to name the page's origin, with the hash of the client data it made.
    Browser {
        origin: String,
        client_data_hash: [u8; 32],
    },
    /// A native app: no web origin, so the website must list the app (package and signing certificate).
    App {
        package: String,
        cert_sha256: [u8; 32],
    },
    /// The app's own window: the page's origin must belong to the website named.
    Own { origin: String },
}

impl Caller {
    /// The host of the origin, for a request that names no website of its own.
    pub fn origin_host(&self) -> Option<String> {
        match self {
            Caller::Browser { origin, .. } | Caller::Own { origin } => {
                origin::host_of_origin(origin)
            }
            Caller::App { .. } => None,
        }
    }

    fn client_data(&self, rp_id: &str, links: LinksFn<'_>) -> Result<json::ClientData> {
        match self {
            Caller::Browser {
                origin,
                client_data_hash,
            } => {
                if origin::host_of_origin(origin).is_none() {
                    return Err(StorageError::invalid(
                        "The page that asked has no valid address.",
                    ));
                }
                Ok(json::ClientData::Hash(*client_data_hash))
            }
            Caller::App {
                package,
                cert_sha256,
            } => {
                let text = links(rp_id).map_err(StorageError::from)?;
                if !parse_asset_links(&text, package, cert_sha256) {
                    return Err(StorageError::invalid(
                        "The website does not say that this app may use its passkeys.",
                    ));
                }
                Ok(json::ClientData::Built {
                    origin: origin::app_origin(cert_sha256),
                    cross_origin: false,
                    android_package: Some(package.clone()),
                })
            }
            Caller::Own { origin } => {
                if !origin::rp_matches_origin(rp_id, origin) {
                    return Err(StorageError::invalid(
                        "This page may not use passkeys of that website.",
                    ));
                }
                Ok(json::ClientData::Built {
                    origin: origin.clone(),
                    cross_origin: false,
                    android_package: None,
                })
            }
        }
    }
}

/// What the screen sends to put a scanned code on a login or make a login for it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanChoice {
    Existing(String),
    New { title: Option<String> },
}

impl From<ScanChoice> for Target {
    fn from(c: ScanChoice) -> Self {
        match c {
            ScanChoice::Existing(id) => Target::Existing(id),
            ScanChoice::New { title } => Target::New { title },
        }
    }
}

/// What a scanned text is, and the logins it probably belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ScanReply {
    Totp {
        issuer: Option<String>,
        account: String,
        digits: u8,
        period: u64,
        suggestions: Vec<Suggestion>,
    },
    Migration,
    Fido,
    Link {
        host: String,
    },
    Text,
    Invalid {
        message: String,
    },
}

pub struct RegisterInput {
    pub request_json: String,
    pub caller: Caller,
    pub user_verified: bool,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Registered {
    /// The answer to hand to the website or app (`RegistrationResponseJSON`).
    pub response_json: String,
    pub item: ItemSummary,
}

pub struct AssertInput {
    pub request_json: String,
    pub caller: Caller,
    pub user_verified: bool,
    pub item_id: String,
    pub key: String,
}

/// What to look for when a website asks to make a passkey.
fn hints_of(req: &CreateRequest) -> Hints {
    let mut accounts = vec![req.user_name.clone()];
    if req.display_name.contains('@') {
        accounts.push(req.display_name.clone());
    }
    // `login.github.com` -> `github`: the name before the last dot is what a title usually holds
    let labels: Vec<&str> = req.rp_id.split('.').collect();
    let mut names = vec![req.rp_name.clone()];
    if labels.len() >= 2 {
        names.push(labels[labels.len() - 2].to_string());
    }
    Hints {
        accounts,
        sites: vec![req.rp_id.clone()],
        names,
        secret: None,
    }
}

impl Session {
    // ------------------------------------------------------------------ the scanner

    /// Say what a scanned or pasted text is and which logins it may belong to. Nothing is stored and nothing secret is
    /// returned: the answer holds the names a login already shows in the list.
    pub fn scan_classify(&self, text: &str, allow_bare_key: bool) -> Result<ScanReply> {
        match classify(text, allow_bare_key) {
            Scanned::Totp(f) => {
                let secret = crate::storage::vault::import::account_key_secret(text.trim());
                let hints = Hints {
                    accounts: vec![f.account.clone()],
                    sites: f
                        .issuer
                        .iter()
                        .filter(|i| i.contains('.'))
                        .cloned()
                        .collect(),
                    names: f.issuer.iter().cloned().collect(),
                    secret: Some(secret).filter(|s| !s.is_empty()),
                };
                let suggestions = self.with_unlocked(|u| u.vault.suggest_logins(&hints))?;
                Ok(ScanReply::Totp {
                    issuer: f.issuer,
                    account: f.account,
                    digits: f.digits,
                    period: f.period,
                    suggestions,
                })
            }
            Scanned::Migration => {
                self.with_unlocked(|_| Ok(()))?;
                Ok(ScanReply::Migration)
            }
            Scanned::Fido => {
                self.with_unlocked(|_| Ok(()))?;
                Ok(ScanReply::Fido)
            }
            Scanned::Link { host } => {
                self.with_unlocked(|_| Ok(()))?;
                Ok(ScanReply::Link { host })
            }
            Scanned::Text => {
                self.with_unlocked(|_| Ok(()))?;
                Ok(ScanReply::Text)
            }
            Scanned::Invalid(message) => {
                self.with_unlocked(|_| Ok(()))?;
                Ok(ScanReply::Invalid { message })
            }
        }
    }

    /// Put a scanned one-time code on an existing login (the master password is asked for exactly as for any change to
    /// that login) or make a new login for it.
    pub fn scan_apply(
        &self,
        text: &str,
        choice: ScanChoice,
        password: Option<String>,
    ) -> Result<ItemSummary> {
        let facts = match classify(text, true) {
            Scanned::Totp(f) => CodeFacts {
                issuer: f.issuer,
                account: f.account,
            },
            _ => return Err(StorageError::invalid("That is not a one-time code.")),
        };
        let value = text.trim().to_string();
        let target: Target = choice.into();
        match &target {
            Target::Existing(id) => {
                let id = id.clone();
                self.gated_item(
                    &id,
                    password,
                    |u| u.vault.require_active(&id),
                    |u| u.vault.save_code(&value, &target, &facts),
                )
            }
            Target::New { .. } => {
                self.with_unlocked(|u| u.vault.save_code(&value, &target, &facts))
            }
        }
    }

    // ------------------------------------------------------------------ passkeys

    /// The logins a new passkey for this request probably belongs on.
    pub fn passkey_suggest(
        &self,
        request_json: &str,
        origin_host: Option<&str>,
    ) -> Result<(CreateRequest, Vec<Suggestion>)> {
        let req = json::parse_create(request_json, origin_host)?;
        let hints = hints_of(&req);
        let list = self.with_unlocked(|u| u.vault.suggest_logins(&hints))?;
        Ok((req, list))
    }

    /// The passkeys this vault holds for a sign-in request.
    pub fn passkey_candidates(
        &self,
        request_json: &str,
        origin_host: Option<&str>,
    ) -> Result<(GetRequest, Vec<PasskeyMatch>)> {
        let req = json::parse_get(request_json, origin_host)?;
        let list = self.with_unlocked(|u| u.vault.passkeys_for(&req.rp_id, &req.allow))?;
        Ok((req, list))
    }

    /// Make a passkey for a website's request and save it. The answer is returned only after the passkey is stored.
    pub fn passkey_register(
        &self,
        input: &RegisterInput,
        password: Option<String>,
        links: LinksFn<'_>,
    ) -> Result<Registered> {
        let req = json::parse_create(&input.request_json, input.caller.origin_host().as_deref())?;
        let data = input.caller.client_data(&req.rp_id, links)?;
        let run = |u: &mut super::Unlocked| -> Result<Registered> {
            if u.vault.holds_passkey(&req.rp_id, &req.exclude)? {
                return Err(StorageError::PasskeyExcluded);
            }
            let (made, response_json) = json::make_for(
                &req,
                &data,
                input.user_verified,
                crate::crypto::now_millis(),
            )?;
            let item = u.vault.save_passkey(&made.record, &input.target)?;
            Ok(Registered {
                response_json,
                item,
            })
        };
        match &input.target {
            Target::Existing(id) => {
                self.gated_item(id, password, |u| u.vault.require_active(id), run)
            }
            // A typed password counts as user verification only after the core has checked it (the caller sets
            // `user_verified` from the presence of one), so a new login checks it first, like every other way in.
            Target::New { .. } => match password {
                Some(password) => self.gated(password, |_| Ok(()), run),
                None => self.with_unlocked(run),
            },
        }
    }

    /// Sign a website's request with the passkey in field `key` of item `item_id`.
    pub fn passkey_assert(
        &self,
        input: &AssertInput,
        password: Option<String>,
        links: LinksFn<'_>,
    ) -> Result<String> {
        let req = json::parse_get(&input.request_json, input.caller.origin_host().as_deref())?;
        let data = input.caller.client_data(&req.rp_id, links)?;
        self.gated_item(
            &input.item_id,
            password,
            |u| u.vault.require_active(&input.item_id),
            |u| {
                let record = u.vault.passkey_record(&input.item_id, &input.key)?;
                if !req.allow.is_empty()
                    && !req
                        .allow
                        .iter()
                        .any(|id| *id == record.credential_id_bytes())
                {
                    return Err(StorageError::invalid(
                        "That passkey is not one the website asked for.",
                    ));
                }
                Ok(json::sign_for(&record, &req, &data, input.user_verified)?)
            },
        )
    }
}

/// Spell a client data hash the way the provider's request carries it (base64url) into bytes.
pub fn hash_from_b64(text: &str) -> Result<[u8; 32]> {
    let bytes = b64::decode(text).map_err(StorageError::from)?;
    bytes
        .try_into()
        .map_err(|_| StorageError::invalid("The client data hash must be 32 bytes."))
}

impl From<PasskeyError> for StorageError {
    fn from(e: PasskeyError) -> Self {
        match e {
            PasskeyError::Invalid(m) => StorageError::InvalidInput(m),
            PasskeyError::Unsupported(m) => StorageError::PasskeyUnsupported(m),
            PasskeyError::Excluded => StorageError::PasskeyExcluded,
            PasskeyError::Rng => StorageError::Crypto("A cryptographic operation failed.".into()),
        }
    }
}
