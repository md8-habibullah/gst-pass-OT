//! The passkey provider behind Android's Credential Manager (`ai-docs/PASSKEYS.md`), as plain functions on JSON text.
//!
//! The system asks a provider two things: "which passkeys do you have for this website" and "make a passkey for this
//! website". The Kotlin side (`scripts/android/HexisCredentialProviderService.kt` and `PasskeyActivity.kt`) draws the system
//! sheet and the fingerprint prompt; everything else is decided here, in the same core the app uses:
//!
//! * [`Provider::call`] is the only entry. It takes an operation name and a JSON text and answers a JSON text
//!   (`{"ok":true,"data":…}` or `{"ok":false,"code":…,"message":…}`), so the JNI layer (`jni.rs`) has one function to export
//!   and a panic can never cross into Java.
//! * The private key of a passkey never leaves the core: what goes out is the signed answer for a website's request.
//! * While the app lock (Stage 1) is closed **every** operation refuses with `app_locked`: the provider is no way around it.
//! * A locked vault is opened with its master password only (a fingerprint never opens a vault, see D13). With the vault
//!   open, each sign-in or new passkey still needs a fresh fingerprint or screen lock, which Kotlin reports as
//!   `userVerified`; Rust writes that flag into the answer and refuses a request that requires it when it is missing.
//! * Using or adding to a login that asks for the master password (or any login while the vault was opened with the PIN)
//!   answers `password_required`, exactly like the same change inside the app.

use crate::passkey::{b64, origin, PasskeyError};
use crate::storage::app::AppCore;
use crate::storage::error::StorageError;
use crate::storage::model::ItemSummary;
use crate::storage::session::passkeys::{AssertInput, Caller, RegisterInput};
use crate::storage::vault::passkeys::Target;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

#[cfg(target_os = "android")]
pub mod jni;
#[cfg(target_os = "android")]
pub mod shared;

#[cfg(test)]
mod tests;

/// How a website's list of allowed apps is fetched (`https://<site>/.well-known/assetlinks.json`). Tests pass a fake.
pub type LinksSource = Box<dyn Fn(&str) -> Result<String, PasskeyError> + Send + Sync>;

/// The app's identifier (`tauri.conf.json`), which is also the name of the folder inside Android's per-app data directory
/// that the window keeps its data in (`app_data_dir`). The provider finds the same folder from the directory the system gives it.
pub fn app_identifier() -> &'static str {
    use std::sync::OnceLock;
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        serde_json::from_str::<Value>(include_str!("../../tauri.conf.json"))
            .ok()
            .and_then(|v| v["identifier"].as_str().map(str::to_string))
            .expect("tauri.conf.json names the app identifier")
    })
}

/// The most logins the sheet offers besides the matches (the sheet is small; the app has the full list).
const MAX_OTHER_LOGINS: usize = 40;

pub struct Provider {
    core: Arc<AppCore>,
    links: LinksSource,
}

/// An answer to Kotlin, before it becomes text.
type Answer = Result<Value, Failure>;

enum Failure {
    Storage(StorageError),
    /// The app lock is closed.
    AppLocked,
    /// An app asked and the person switched the website check off (Settings > Passkeys): apps get no passkeys.
    AppCheckOff,
    /// The request from Kotlin was not what it should be (a bug on the Kotlin side, or a changed contract).
    Bad(String),
}

impl From<StorageError> for Failure {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<PasskeyError> for Failure {
    fn from(e: PasskeyError) -> Self {
        Self::Storage(e.into())
    }
}

impl Failure {
    fn to_json(&self) -> String {
        let (code, message) = match self {
            Self::Storage(e) => (e.code().to_string(), e.to_string()),
            Self::AppLocked => (
                "app_locked".to_string(),
                "Open the app and unlock it first.".to_string(),
            ),
            Self::AppCheckOff => (
                "app_check_off".to_string(),
                "Passkeys for apps are switched off. Turn on \u{201c}Check an app with its website\u{201d} in Hexis, Settings, Passkeys.".to_string(),
            ),
            Self::Bad(m) => ("bad_request".to_string(), m.clone()),
        };
        json!({"ok": false, "code": code, "message": message}).to_string()
    }
}

// ------------------------------------------------------------------ what Kotlin sends

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CallerIn {
    /// A browser the system trusts: the origin of the page and the hash of the client data it built.
    #[serde(rename_all = "camelCase")]
    Browser {
        origin: String,
        client_data_hash: String,
    },
    /// Any other app: its package and the SHA-256 of its signing certificate (base64url).
    #[serde(rename_all = "camelCase")]
    App {
        package: String,
        cert_sha256: String,
    },
}

impl CallerIn {
    /// `app_check`: may an app be checked against its website. A browser needs no such check, so it is never affected.
    fn into_caller(self, app_check: bool) -> Result<Caller, Failure> {
        if matches!(self, Self::App { .. }) && !app_check {
            return Err(Failure::AppCheckOff);
        }
        Ok(match self {
            Self::Browser {
                origin,
                client_data_hash,
            } => Caller::Browser {
                origin,
                client_data_hash: crate::storage::session::passkeys::hash_from_b64(
                    &client_data_hash,
                )?,
            },
            Self::App {
                package,
                cert_sha256,
            } => {
                let bytes = b64::decode(&cert_sha256).map_err(StorageError::from)?;
                let cert_sha256: [u8; 32] = bytes.try_into().map_err(|_| {
                    Failure::Bad("The signing certificate hash must be 32 bytes.".into())
                })?;
                Caller::App {
                    package,
                    cert_sha256,
                }
            }
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetIn {
    existing: Option<String>,
    new: Option<NewIn>,
}

#[derive(Deserialize)]
struct NewIn {
    title: Option<String>,
}

impl TargetIn {
    fn into_target(self) -> Result<Target, Failure> {
        match (self.existing, self.new) {
            (Some(id), None) => Ok(Target::Existing(id)),
            (None, Some(n)) => Ok(Target::New { title: n.title }),
            _ => Err(Failure::Bad("Choose one login, or a new one.".into())),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnlockIn {
    account_id: Option<String>,
    password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestIn {
    request: String,
    caller: CallerIn,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignIn {
    request: String,
    caller: CallerIn,
    item_id: String,
    key: String,
    #[serde(default)]
    user_verified: bool,
    password: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateIn {
    request: String,
    caller: CallerIn,
    target: TargetIn,
    #[serde(default)]
    user_verified: bool,
    password: Option<String>,
}

// ------------------------------------------------------------------ what Kotlin gets

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountOut {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateOut {
    /// The accounts a locked vault can be chosen from. Empty while the app lock is closed.
    accounts: Vec<AccountOut>,
    selected: Option<String>,
    unlocked: bool,
    /// The vault was opened with the account PIN: secrets, and so passkeys, ask for the master password.
    limited: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginOut {
    id: String,
    title: String,
    subtitle: Option<String>,
    reprompt: bool,
}

impl Provider {
    pub fn new(core: Arc<AppCore>) -> Self {
        Self {
            core,
            links: Box::new(origin::fetch_asset_links),
        }
    }

    /// Use another way of reading a website's app list (tests; the app never needs this).
    #[must_use]
    pub fn with_links(mut self, links: LinksSource) -> Self {
        self.links = links;
        self
    }

    pub fn core(&self) -> &Arc<AppCore> {
        &self.core
    }

    /// Run one operation. Never panics and never returns anything but a JSON object.
    pub fn call(&self, op: &str, args: &str) -> String {
        match catch_unwind(AssertUnwindSafe(|| self.dispatch(op, args))) {
            Ok(Ok(data)) => json!({"ok": true, "data": data}).to_string(),
            Ok(Err(f)) => f.to_json(),
            Err(_) => json!({"ok": false, "code": "internal", "message": "The passkey service hit an unexpected error."}).to_string(),
        }
    }

    fn dispatch(&self, op: &str, args: &str) -> Answer {
        if self.core.gate().is_closed() {
            return Err(Failure::AppLocked);
        }
        match op {
            "state" => self.state(),
            "unlock" => self.unlock(parse(args)?),
            "lock" => {
                self.core.lock();
                self.state()
            }
            "getOptions" => self.get_options(parse(args)?),
            "sign" => self.sign(parse(args)?),
            "createOptions" => self.create_options(parse(args)?),
            "create" => self.create(parse(args)?),
            other => Err(Failure::Bad(format!("Unknown operation {other}."))),
        }
    }

    fn state(&self) -> Answer {
        let list = self.core.profile_list()?;
        let status = self.core.status()?;
        let out = StateOut {
            accounts: list
                .profiles
                .iter()
                .filter(|p| !p.demo)
                .map(|p| AccountOut {
                    id: p.id.clone(),
                    name: p.display_name.clone(),
                })
                .collect(),
            selected: list.selected_id,
            unlocked: status.unlocked && !status.resumable,
            limited: status.limited,
        };
        Ok(serde_json::to_value(out).expect("state serializes"))
    }

    /// Open a vault with its master password. A different account than the one open is selected first, which wipes the open
    /// one (or parks it behind its PIN), the same as choosing it in the app.
    fn unlock(&self, input: UnlockIn) -> Answer {
        let current = self.core.profile_list()?.selected_id;
        if let Some(id) = input.account_id.as_deref() {
            if current.as_deref() != Some(id) {
                self.core.profile_select(id)?;
            }
        } else if current.is_none() {
            return Err(StorageError::NoVault.into());
        }
        self.core.unlock(input.password)?;
        self.state()
    }

    fn get_options(&self, input: RequestIn) -> Answer {
        let caller = input.caller.into_caller(self.core.passkey_app_check())?;
        let host = caller.origin_host();
        let (req, matches) = self
            .core
            .passkey_candidates(&input.request, host.as_deref())?;
        Ok(json!({"rpId": req.rp_id, "matches": matches}))
    }

    fn sign(&self, input: SignIn) -> Answer {
        let caller = input.caller.into_caller(self.core.passkey_app_check())?;
        let assert = AssertInput {
            request_json: input.request,
            caller,
            user_verified: input.user_verified,
            item_id: input.item_id,
            key: input.key,
        };
        let response = self
            .core
            .passkey_assert(&assert, input.password, &*self.links)?;
        Ok(json!({"responseJson": response}))
    }

    /// What the "save this passkey" sheet shows: the request's names, the logins it fits (by account, site and name) and the
    /// other logins, so the person picks one or makes a new one.
    fn create_options(&self, input: RequestIn) -> Answer {
        let caller = input.caller.into_caller(self.core.passkey_app_check())?;
        let host = caller.origin_host();
        let (req, suggestions) = self.core.passkey_suggest(&input.request, host.as_deref())?;
        let taken: std::collections::HashSet<&str> =
            suggestions.iter().map(|s| s.id.as_str()).collect();
        let mut others: Vec<LoginOut> = self
            .core
            .item_list()?
            .into_iter()
            .filter(|i: &ItemSummary| {
                i.item_type == crate::storage::model::ItemType::Login
                    && !i.archived
                    && !taken.contains(i.id.as_str())
            })
            .map(|i| LoginOut {
                id: i.id,
                title: i.title,
                subtitle: i.subtitle,
                reprompt: i.reprompt,
            })
            .collect();
        others.sort_by_key(|l| l.title.to_lowercase());
        others.truncate(MAX_OTHER_LOGINS);
        Ok(json!({
            "rpId": req.rp_id,
            "rpName": req.rp_name,
            "userName": req.user_name,
            "displayName": req.display_name,
            "suggestions": suggestions,
            "others": others,
        }))
    }

    fn create(&self, input: CreateIn) -> Answer {
        let caller = input.caller.into_caller(self.core.passkey_app_check())?;
        let register = RegisterInput {
            request_json: input.request,
            caller,
            user_verified: input.user_verified,
            target: input.target.into_target()?,
        };
        let done = self
            .core
            .passkey_register(&register, input.password, &*self.links)?;
        Ok(json!({
            "responseJson": done.response_json,
            "item": {"id": done.item.id, "title": done.item.title},
        }))
    }
}

fn parse<T: for<'de> Deserialize<'de>>(args: &str) -> Result<T, Failure> {
    serde_json::from_str(args)
        .map_err(|e| Failure::Bad(format!("The request was not understood: {e}")))
}
