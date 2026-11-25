//! The app's single vault session: locked or unlocked, with the master-password re-prompt guard.
//!
//! Tauri-free and fully testable. The command layer (`commands/`) is a thin wrapper.
//!
//! **Re-prompt rule** (`ai-docs/IPC_CONTRACT.md` §1): permanent delete, password change, exports, attachments
//! and backup secrets verify the master password with Argon2id on *every* call. Revealing, copying, editing,
//! cloning and the history of ONE item need it only when that item has the "ask for the master password
//! again" flag (`reprompt`, like Bitwarden's): the vault is already open, and the person typed the master
//! password to open it. An item without the flag works directly; with the flag, a call without a password
//! answers `password_required` and one with a wrong password `wrong_password`. A password that is sent is
//! always verified, flagged item or not. The password is never stored. The Argon2id run happens **without** the
//! session lock held, so other commands (and the UI) are not blocked behind it.

use super::backup::prefs::{GeneratorPrefs, PrefsView};
use super::backup::{probe_config, BackupRuntime, BackupTargetInput, BackupTargetView, VaultLink};
use super::db::Store;
use super::error::{Result, StorageError};
use super::model::{
    BeginEdit, ItemDraft, ItemSummary, ItemView, TotpNow, TrashEntry, VaultStatus, VersionEntry,
};
use super::uuid;
use super::vault::search::SearchHit;
use super::vault::Vault;
use crate::crypto::{self, now_millis, rng, KdfParams, MasterPassword};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

// Import state and the plaintext export (slice Q4). A child module, so it can reach `Unlocked`.
#[path = "session_import.rs"]
pub mod import;
use self::import::ImportStash;

pub const MIN_PASSWORD_CHARS: usize = 8;
/// How long an edit token stays valid (`IPC_CONTRACT.md` §1).
pub const EDIT_TOKEN_TTL: Duration = Duration::from_secs(600);
const MAX_OUTSTANDING_TOKENS: usize = 16;
/// An item that has been in the trash for less than this (about six months) cannot be erased for good without solving
/// a sum as well as typing the master password. Older ones need the password alone.
pub const PURGE_GRACE: Duration = Duration::from_secs(183 * 24 * 60 * 60);
/// How long the sum for a permanent delete stays valid once it has been shown.
const PURGE_SUM_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    /// Argon2id parameters used when creating a vault or changing its password.
    pub kdf: KdfParams,
    pub edit_token_ttl: Duration,
    /// See [`PURGE_GRACE`]. Tests that purge at once set it to zero.
    pub purge_grace: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            kdf: KdfParams::default(),
            edit_token_ttl: EDIT_TOKEN_TTL,
            purge_grace: PURGE_GRACE,
        }
    }
}

/// What the screen needs to show before an item is erased for good.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgePlan {
    /// When the item went to the trash (milliseconds since 1970, UTC).
    pub deleted_at: u64,
    /// It was trashed less than six months ago: the sum must be solved as well as the password typed.
    pub recent: bool,
    /// When the six months are over (milliseconds since 1970, UTC).
    pub older_at: u64,
    /// The sum, as text, when `recent`. The answer stays in the core.
    pub sum: Option<String>,
}

/// The sum that guards the permanent delete of one recently trashed item.
struct PurgeSum {
    item_id: String,
    answer: u32,
    expires: Instant,
}

/// Single-use permission to save one edit of one item at one version.
struct EditToken {
    secret: Zeroizing<[u8; 32]>,
    item_id: String,
    version: u64,
    expires: Instant,
}

struct Unlocked {
    vault: Vault,
    tokens: Vec<EditToken>,
    /// Picked import files and parsed previews; wiped with everything else at lock.
    imports: ImportStash,
    /// The sum made for the last permanent delete that asked for one (one at a time; a new one replaces it).
    purge_sum: Option<PurgeSum>,
    /// Parked: the vault is still open in memory but out of reach. Every call answers `locked` until the account PIN
    /// brings it back (`quick.rs`). The keys are still held, which is exactly what the idle timer, a logout and five
    /// wrong PINs end.
    parked: bool,
    /// Opened again with the PIN, not the master password: everything sensitive still asks for the master password.
    limited: bool,
    /// Wrong PINs since it was parked.
    wrong_pins: u8,
}

impl Unlocked {
    fn new(vault: Vault) -> Self {
        Self {
            vault,
            tokens: Vec::new(),
            imports: ImportStash::default(),
            purge_sum: None,
            parked: false,
            limited: false,
            wrong_pins: 0,
        }
    }

    /// Out of reach: nothing that was half done survives (edit tokens, picked import files, the delete sum).
    /// The count of wrong PINs restarts only when the vault goes from shown to parked, never while it is already parked:
    /// otherwise Lock, or choosing the account again, would give four more tries and the fifth would never come.
    fn park(&mut self) {
        if !self.parked {
            self.wrong_pins = 0;
        }
        self.parked = true;
        self.tokens.clear();
        self.imports = ImportStash::default();
        self.purge_sum = None;
    }

    /// Use up the sum for `id`: `Ok` only for the right answer, and the sum is gone either way.
    fn take_purge_sum(&mut self, id: &str, typed: &str) -> Result<()> {
        let Some(sum) = self.purge_sum.take() else {
            return Err(StorageError::WrongAnswer);
        };
        let typed = typed.trim();
        let right = sum.item_id == id
            && sum.expires > Instant::now()
            && !typed.is_empty()
            && typed.len() <= 9
            && typed.bytes().all(|b| b.is_ascii_digit())
            && typed.parse::<u32>().is_ok_and(|n| n == sum.answer);
        if right {
            Ok(())
        } else {
            Err(StorageError::WrongAnswer)
        }
    }

    fn issue_token(&mut self, item_id: &str, version: u64, ttl: Duration) -> Result<String> {
        let now = Instant::now();
        self.tokens.retain(|t| t.expires > now);
        while self.tokens.len() >= MAX_OUTSTANDING_TOKENS {
            self.tokens.remove(0);
        }
        let mut secret = Zeroizing::new([0u8; 32]);
        rng::fill(&mut secret[..])?;
        let text = URL_SAFE_NO_PAD.encode(&secret[..]);
        self.tokens.push(EditToken {
            secret,
            item_id: item_id.to_string(),
            version,
            expires: now + ttl,
        });
        Ok(text)
    }

    /// Index of the presented token, compared in constant time against every outstanding one.
    fn find_token(&mut self, presented: &str) -> Result<usize> {
        let now = Instant::now();
        self.tokens.retain(|t| t.expires > now);
        let decoded = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(presented.trim())
                .map_err(|_| StorageError::TokenInvalid)?,
        );
        if decoded.len() != 32 {
            return Err(StorageError::TokenInvalid);
        }
        let mut found = None;
        for (i, t) in self.tokens.iter().enumerate() {
            if bool::from(t.secret[..].ct_eq(&decoded[..])) {
                found = Some(i);
            }
        }
        found.ok_or(StorageError::TokenInvalid)
    }
}

enum State {
    Locked,
    Unlocked(Box<Unlocked>),
}

pub struct Session {
    path: PathBuf,
    cfg: SessionConfig,
    /// How backup targets are reached (real S3 in the app, in-memory buckets in tests).
    runtime: Arc<BackupRuntime>,
    /// Shared (weakly) with the replicator's [`VaultLink`], so a pull can visit the unlocked
    /// vault without ever holding a copy of its key.
    state: Arc<Mutex<State>>,
}

/// The replicator's way into the unlocked vault: it runs a closure on the vault of *one* unlock
/// (the instance number) under the session lock, or reports that there is no such vault any more.
struct SessionLink {
    state: Weak<Mutex<State>>,
    instance: u64,
}

impl VaultLink for SessionLink {
    fn with_vault(&self, f: &mut dyn FnMut(&mut Vault)) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        // A poisoned lock means a panic happened inside the vault: it is treated as locked here
        // (the session itself resets it to locked the next time it is used).
        let Ok(mut guard) = state.lock() else {
            return false;
        };
        match &mut *guard {
            State::Unlocked(u) if u.vault.instance() == self.instance => {
                f(&mut u.vault);
                true
            }
            _ => false,
        }
    }
}

fn check_new_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(StorageError::invalid(
            "Use at least 8 characters for the master password.",
        ));
    }
    Ok(())
}

impl Session {
    pub fn new(path: PathBuf, cfg: SessionConfig) -> Self {
        Self::with_runtime(path, cfg, BackupRuntime::production())
    }

    pub fn with_runtime(path: PathBuf, cfg: SessionConfig, runtime: Arc<BackupRuntime>) -> Self {
        Self {
            path,
            cfg,
            runtime,
            state: Arc::new(Mutex::new(State::Locked)),
        }
    }

    /// Make `vault` the open one: attach its backup engine, install it, then start the engine
    /// (unlock trigger: pull, reconcile, drain on the worker thread; this call never waits for
    /// the network).
    fn install(&self, mut vault: Vault) {
        let link = SessionLink {
            state: Arc::downgrade(&self.state),
            instance: vault.instance(),
        };
        vault.attach_backup(Arc::clone(&self.runtime), Arc::new(link));
        let mut guard = self.guard();
        *guard = State::Unlocked(Box::new(Unlocked::new(vault)));
        if let State::Unlocked(u) = &*guard {
            u.vault.start_backup();
        }
    }

    /// Lock the state. A panic while holding the lock leaves the vault *locked* (keys wiped)
    /// instead of in an unknown condition.
    fn guard(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| {
            let mut g = poisoned.into_inner();
            *g = State::Locked;
            self.state.clear_poison();
            g
        })
    }

    fn with_unlocked<T>(&self, f: impl FnOnce(&mut Unlocked) -> Result<T>) -> Result<T> {
        match &mut *self.guard() {
            State::Unlocked(u) if !u.parked => f(u),
            // parked behind the PIN counts as locked for everything
            State::Unlocked(_) | State::Locked => Err(StorageError::Locked),
        }
    }

    /// The re-prompt guard. `pre` runs under the lock and rejects requests that cannot succeed
    /// (so a doomed request never costs an Argon2id run); then the password is verified with the
    /// lock released; then `op` runs under the lock again and re-validates everything itself.
    fn gated<T>(
        &self,
        password: String,
        pre: impl FnOnce(&Unlocked) -> Result<()>,
        op: impl FnOnce(&mut Unlocked) -> Result<T>,
    ) -> Result<T> {
        let bundle = self.with_unlocked(|u| {
            pre(u)?;
            Ok(u.vault.bundle().clone())
        })?;
        let password = MasterPassword::from_string(password)?;
        crypto::verify_password(&bundle, &password)?;
        drop(password);
        self.with_unlocked(op)
    }

    /// The re-prompt guard for ONE item. No password: the item must not have the `reprompt` flag, and the check and
    /// the operation happen in one visit to the lock (so the flag cannot change in between). A password: exactly
    /// [`Self::gated`].
    fn gated_item<T>(
        &self,
        id: &str,
        password: Option<String>,
        pre: impl FnOnce(&Unlocked) -> Result<()>,
        op: impl FnOnce(&mut Unlocked) -> Result<T>,
    ) -> Result<T> {
        match password {
            Some(password) => self.gated(password, pre, op),
            None => self.with_unlocked(|u| {
                pre(u)?;
                // a vault opened with the PIN asks for the master password for every item, flagged or not
                if u.limited || u.vault.requires_reprompt(id)? {
                    return Err(StorageError::PasswordRequired);
                }
                op(u)
            }),
        }
    }

    fn vault_file_has_bundle(&self) -> Result<bool> {
        if !self.path.exists() {
            return Ok(false);
        }
        Ok(Store::open(&self.path)?.bundle_json()?.is_some())
    }

    // ------------------------------------------------------------------ vault lifecycle

    pub fn status(&self) -> Result<VaultStatus> {
        let (unlocked, damaged, revision, reconciling, remote_damaged, parked, limited) =
            match &*self.guard() {
                State::Unlocked(u) if u.parked => (false, 0, 0, false, 0, true, false),
                State::Unlocked(u) => (
                    true,
                    u.vault.damaged_items(),
                    u.vault.sync_revision(),
                    u.vault.backup_reconciling(),
                    u.vault.remote_damaged(),
                    false,
                    u.limited,
                ),
                State::Locked => (false, 0, 0, false, 0, false, false),
            };
        let exists = unlocked || parked || self.vault_file_has_bundle()?;
        Ok(VaultStatus {
            exists,
            unlocked,
            profile: None, // filled in by the profile layer (`AppCore`)
            damaged_items: damaged,
            sync_revision: revision,
            reconciling,
            remote_damaged,
            resumable: parked,
            limited,
        })
    }

    /// Create the vault with a new master password and leave it unlocked.
    pub fn create(&self, password: String) -> Result<()> {
        check_new_password(&password)?;
        if matches!(&*self.guard(), State::Unlocked(_)) || self.vault_file_has_bundle()? {
            return Err(StorageError::VaultExists);
        }
        let password = MasterPassword::from_string(password)?;
        // Slow (Argon2id + ML-KEM key generation); no lock held.
        let (bundle, root) = crypto::create(&password, &self.cfg.kdf)?;
        drop(password);

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Create the file ourselves, owner-only, *before* SQLite does: SQLite gives the `-wal` and
        // `-shm` files the mode of the main file at the moment it makes them, so all three are
        // private from the first byte (a chmod afterwards would be too late for the other two).
        match super::files::create_private_file(&self.path) {
            Ok(file) => drop(file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let mut store = Store::open(&self.path)?;
        restrict_file_permissions(&self.path);
        store.create_vault(&bundle.to_json()?, &uuid::new_v4()?, now_millis())?;
        let vault = Vault::open(store, bundle, root)?;
        self.install(vault);
        Ok(())
    }

    pub fn unlock(&self, password: String) -> Result<()> {
        if !self.path.exists() {
            return Err(StorageError::NoVault);
        }
        let store = Store::open(&self.path)?;
        let json = store.bundle_json()?.ok_or(StorageError::NoVault)?;
        let bundle = crypto::VaultKeyBundle::from_json(&json)?;
        let password = MasterPassword::from_string(password)?;
        // Slow (Argon2id + hybrid decapsulation); no lock held.
        let root = crypto::unlock(&bundle, &password)?;
        drop(password);
        let vault = Vault::open(store, bundle, root)?;
        self.install(vault);
        Ok(())
    }

    /// Drop the root key, the index and every edit token (all wiped on drop).
    pub fn lock(&self) {
        *self.guard() = State::Locked;
    }

    /// Put the open vault out of reach but keep it open in memory, so the account PIN can show it again.
    /// `false` when nothing is open (a locked vault stays locked).
    pub fn park(&self) -> bool {
        match &mut *self.guard() {
            State::Unlocked(u) => {
                u.park();
                true
            }
            State::Locked => false,
        }
    }

    /// Is the vault open in memory but out of reach (waiting for the PIN)?
    pub fn is_parked(&self) -> bool {
        matches!(&*self.guard(), State::Unlocked(u) if u.parked)
    }

    /// The PIN was right: the parked vault is shown again, **limited** until the master password is typed.
    pub fn resume(&self) -> Result<()> {
        match &mut *self.guard() {
            State::Unlocked(u) if u.parked => {
                u.parked = false;
                u.limited = true;
                u.wrong_pins = 0;
                Ok(())
            }
            _ => Err(StorageError::Locked),
        }
    }

    /// The PIN was wrong. After [`crate::storage::quick::MAX_WRONG_PINS`] in a row the vault is closed for good (keys wiped).
    pub fn note_wrong_pin(&self) -> StorageError {
        let mut g = self.guard();
        let State::Unlocked(u) = &mut *g else {
            return StorageError::Locked;
        };
        if !u.parked {
            return StorageError::Locked;
        }
        u.wrong_pins = u.wrong_pins.saturating_add(1);
        if u.wrong_pins >= super::quick::MAX_WRONG_PINS {
            *g = State::Locked;
            return StorageError::PinLocked;
        }
        StorageError::WrongPin {
            left: super::quick::MAX_WRONG_PINS - u.wrong_pins,
        }
    }

    /// Opened with the PIN and not yet with the master password?
    pub fn is_limited(&self) -> bool {
        matches!(&*self.guard(), State::Unlocked(u) if !u.parked && u.limited)
    }

    /// Refuse a sensitive change unless the master password opened this vault (or was typed since).
    pub fn require_full(&self) -> Result<()> {
        self.with_unlocked(|u| {
            if u.limited {
                Err(StorageError::MasterRequired)
            } else {
                Ok(())
            }
        })
    }

    /// Type the master password in a vault that was opened with the PIN: it stops being limited.
    pub fn elevate(&self, password: String) -> Result<()> {
        self.gated(
            password,
            |_| Ok(()),
            |u| {
                u.limited = false;
                Ok(())
            },
        )
    }

    pub fn change_password(&self, old_password: String, new_password: String) -> Result<()> {
        check_new_password(&new_password)?;
        let bundle = self.with_unlocked(|u| Ok(u.vault.bundle().clone()))?;
        let old = MasterPassword::from_string(old_password)?;
        let new = MasterPassword::from_string(new_password)?;
        // Two Argon2id runs; no lock held. A wrong old password is `wrong_password`.
        let next = crypto::change_password(&bundle, &old, &new, &self.cfg.kdf)?;
        drop((old, new));
        self.with_unlocked(|u| u.vault.replace_bundle(&bundle, next))
    }

    /// Check a master password against the vault file on disk, without unlocking anything
    /// (Argon2id + the keyring check). `NoVault` if there is no vault file.
    pub fn check_password(&self, password: String) -> Result<()> {
        if !self.path.exists() {
            return Err(StorageError::NoVault);
        }
        let store = Store::open(&self.path)?;
        let json = store.bundle_json()?.ok_or(StorageError::NoVault)?;
        drop(store);
        let bundle = crypto::VaultKeyBundle::from_json(&json)?;
        let password = MasterPassword::from_string(password)?;
        crypto::verify_password(&bundle, &password)?;
        Ok(())
    }

    /// The re-prompt guard on its own: the vault must be unlocked (`locked` otherwise) and the
    /// master password must be right (`wrong_password`). Argon2id runs without the lock held.
    pub fn confirm_password(&self, password: String) -> Result<()> {
        self.gated(password, |_| Ok(()), |_| Ok(()))
    }

    /// Does this session's file hold a vault (a key bundle)?
    pub fn has_vault_file(&self) -> Result<bool> {
        self.vault_file_has_bundle()
    }

    /// Password-gated export of a consistent copy of the encrypted vault file.
    ///
    /// The password is verified first (so a wrong one never opens a dialog); then `choose` runs
    /// **without any lock held** (it shows a native dialog and may take minutes); then the copy
    /// is written next to the chosen name as a private temp file and renamed into place.
    /// `Ok(None)` when `choose` returns `None` (the person cancelled).
    pub fn export_with(
        &self,
        password: String,
        choose: impl FnOnce() -> Result<Option<PathBuf>>,
    ) -> Result<Option<PathBuf>> {
        self.gated(password, |_| Ok(()), |_| Ok(()))?;
        let Some(dest) = choose()? else {
            return Ok(None);
        };
        if super::files::is_inside(self.path.parent().unwrap_or(&self.path), &dest)
            || dest == self.path
        {
            return Err(StorageError::invalid(
                "Choose a place outside the app's own data folder.",
            ));
        }
        let dir = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let mut tmp_name = dest.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(".part");
        let tmp = dir.join(tmp_name);
        let _ = std::fs::remove_file(&tmp);
        let written = super::files::create_private_file(&tmp)
            .map_err(StorageError::from)
            .and_then(|f| {
                drop(f); // VACUUM INTO needs a path to an empty file
                self.with_unlocked(|u| u.vault.export_copy(&tmp))
            })
            .and_then(|()| {
                std::fs::rename(&tmp, &dest)?;
                super::files::fsync_dir(dir)?;
                Ok(())
            });
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(Some(dest))
    }

    // ------------------------------------------------------------------ reading (no password)

    pub fn item_list(&self) -> Result<Vec<ItemSummary>> {
        self.with_unlocked(|u| Ok(u.vault.list_active()))
    }

    /// The vault's saved preferences (the generator's last settings), the same on every device.
    pub fn prefs_get(&self) -> Result<PrefsView> {
        self.with_unlocked(|u| Ok(u.vault.prefs().into()))
    }

    /// Remember the generator's settings; they are sealed, stored in the vault database and sent
    /// to the backup targets like the vault's other data.
    pub fn prefs_set_generator(&self, generator: GeneratorPrefs) -> Result<PrefsView> {
        self.with_unlocked(|u| u.vault.set_generator_prefs(generator).map(Into::into))
    }

    pub fn item_get(&self, id: &str) -> Result<ItemView> {
        self.with_unlocked(|u| u.vault.get_view(id))
    }

    pub fn trash_list(&self) -> Result<Vec<TrashEntry>> {
        self.with_unlocked(|u| Ok(u.vault.list_trash()))
    }

    pub fn history_list(&self, id: &str) -> Result<Vec<VersionEntry>> {
        self.with_unlocked(|u| u.vault.history_list(id))
    }

    /// The current one-time code. An item with the re-prompt flag needs the master password for it too.
    pub fn totp_now(&self, id: &str, password: Option<String>) -> Result<TotpNow> {
        self.totp_now_of(id, None, password)
    }

    /// The same for one of several codes on the item (`totp#2` …).
    pub fn totp_now_of(
        &self,
        id: &str,
        key: Option<&str>,
        password: Option<String>,
    ) -> Result<TotpNow> {
        self.gated_item(
            id,
            password,
            |u| u.vault.require_active(id),
            |u| u.vault.totp_now_of(id, key),
        )
    }

    // ------------------------------------------------------------------ password-gated reads

    pub fn item_reveal(&self, id: &str, password: Option<String>) -> Result<ItemDraft> {
        self.gated_item(
            id,
            password,
            |u| u.vault.require_active(id),
            |u| u.vault.reveal(id).map(|(draft, _)| draft),
        )
    }

    pub fn history_get(
        &self,
        id: &str,
        version: u64,
        password: Option<String>,
    ) -> Result<ItemDraft> {
        self.gated_item(
            id,
            password,
            |u| u.vault.require_active(id),
            |u| u.vault.history_get(id, version),
        )
    }

    /// Verify the password, then hand out the editable content and a single-use edit token.
    pub fn item_begin_edit(&self, id: &str, password: Option<String>) -> Result<BeginEdit> {
        let ttl = self.cfg.edit_token_ttl;
        self.gated_item(
            id,
            password,
            |u| u.vault.require_active(id),
            |u| {
                let (draft, version) = u.vault.reveal(id)?;
                let token = u.issue_token(id, version, ttl)?;
                Ok(BeginEdit {
                    token,
                    draft,
                    version,
                })
            },
        )
    }

    // ------------------------------------------------------------------ writes

    /// Creating needs no password (the spec's guard list covers editing, not adding).
    pub fn item_create(&self, draft: ItemDraft) -> Result<ItemSummary> {
        self.with_unlocked(|u| u.vault.create_item(draft))
    }

    /// Save an edit. The token must come from `item_begin_edit` for the same item and version.
    /// It is consumed when the save succeeds (or the item/version is no longer valid); a
    /// rejected draft keeps it so the person can fix the input without re-entering the password.
    pub fn item_update(
        &self,
        id: &str,
        token: &str,
        expected_version: u64,
        draft: ItemDraft,
    ) -> Result<ItemSummary> {
        self.with_unlocked(|u| {
            let index = u.find_token(token)?;
            {
                let t = &u.tokens[index];
                if t.item_id != id || t.version != expected_version {
                    u.tokens.remove(index);
                    return Err(StorageError::TokenInvalid);
                }
            }
            match u.vault.update_item(id, expected_version, draft) {
                Ok(summary) => {
                    u.tokens.remove(index);
                    Ok(summary)
                }
                Err(
                    e @ (StorageError::VersionConflict
                    | StorageError::InTrash
                    | StorageError::NotFound),
                ) => {
                    u.tokens.remove(index);
                    Err(e)
                }
                Err(e) => Err(e),
            }
        })
    }

    pub fn item_archive(&self, id: &str) -> Result<ItemSummary> {
        self.with_unlocked(|u| u.vault.archive_item(id))
    }

    pub fn item_unarchive(&self, id: &str) -> Result<ItemSummary> {
        self.with_unlocked(|u| u.vault.unarchive_item(id))
    }

    /// Duplicate an item entirely inside Rust. Password-gated: it copies secrets.
    pub fn item_clone(&self, id: &str, password: Option<String>) -> Result<ItemSummary> {
        self.gated_item(
            id,
            password,
            |u| u.vault.require_active(id),
            |u| u.vault.clone_item(id),
        )
    }

    pub fn item_trash(&self, id: &str) -> Result<()> {
        self.with_unlocked(|u| u.vault.trash_item(id))
    }

    pub fn item_restore(&self, id: &str) -> Result<ItemSummary> {
        self.with_unlocked(|u| u.vault.restore_item(id))
    }

    /// What it takes to erase this trashed item for good: its age, and (for one trashed less than six months ago) a
    /// freshly made sum. Asking again replaces the sum, so an answer worked out for an old one is worthless.
    pub fn item_purge_plan(&self, id: &str) -> Result<PurgePlan> {
        let grace = self.cfg.purge_grace;
        self.with_unlocked(|u| {
            let deleted_at = u.vault.trashed_at(id)?;
            let recent = Self::is_recent(deleted_at, grace);
            let sum = if recent {
                let eq = crate::gate::equation::generate_long(&mut crate::gate::equation::OsRand);
                u.purge_sum = Some(PurgeSum {
                    item_id: id.to_string(),
                    answer: eq.answer,
                    expires: Instant::now() + PURGE_SUM_TTL,
                });
                Some(eq.text.clone())
            } else {
                u.purge_sum = None;
                None
            };
            Ok(PurgePlan {
                deleted_at,
                recent,
                older_at: deleted_at.saturating_add(grace.as_millis() as u64),
                sum,
            })
        })
    }

    /// "Recent" is judged on the core's clock (checked UTC when it was checked, the device's otherwise), and anything
    /// that cannot be placed in the past (a trash time ahead of the clock) counts as recent: the stricter rule.
    fn is_recent(deleted_at: u64, grace: Duration) -> bool {
        let now = crate::clock::TrustedClock::global().now_ms();
        now.saturating_sub(deleted_at) < grace.as_millis() as u64
    }

    /// Permanent deletion. Always the master password; an item trashed less than six months ago also needs the answer
    /// to its sum (`item_purge_plan` made it). The password is checked first, then the sum, and the sum is used up
    /// by a wrong answer, so another sum is needed after one. Then the item and its history are erased.
    pub fn item_purge(&self, id: &str, password: String, answer: &str) -> Result<()> {
        let grace = self.cfg.purge_grace;
        self.gated(
            password,
            |u| u.vault.require_trashed(id),
            |u| {
                let deleted_at = u.vault.trashed_at(id)?;
                if Self::is_recent(deleted_at, grace) {
                    u.take_purge_sum(id, answer)?;
                }
                u.vault.purge_item(id)
            },
        )
    }

    /// Search the items (see `vault_search.rs` for what is and is not looked at).
    pub fn item_search(&self, query: &str, old_versions: bool) -> Result<Vec<SearchHit>> {
        // a vault opened with the PIN treats every item as asking for the master password, so no note text is compared
        self.with_unlocked(|u| u.vault.search(query, old_versions, u.limited))
    }

    // ------------------------------------------------------------------ folders

    pub fn folder_create(&self, path: &str) -> Result<(String, PrefsView)> {
        self.with_unlocked(|u| {
            let spelled = u.vault.folder_create(path)?;
            Ok((spelled, u.vault.prefs().into()))
        })
    }

    pub fn folder_rename(&self, from: &str, to: &str) -> Result<(usize, PrefsView)> {
        self.with_unlocked(|u| {
            let n = u.vault.folder_rename(from, to)?;
            Ok((n, u.vault.prefs().into()))
        })
    }

    pub fn folder_remove(&self, path: &str) -> Result<(usize, PrefsView)> {
        self.with_unlocked(|u| {
            let n = u.vault.folder_dissolve(path)?;
            Ok((n, u.vault.prefs().into()))
        })
    }

    pub fn item_move(&self, ids: &[String], folder: Option<&str>) -> Result<usize> {
        self.with_unlocked(|u| u.vault.items_move(ids, folder))
    }

    /// "Got it": the person has looked at the copy another device sent for this item.
    pub fn item_review_other_device(&self, id: &str) -> Result<()> {
        self.with_unlocked(|u| u.vault.review_other_device(id))
    }

    pub fn history_restore(
        &self,
        id: &str,
        version: u64,
        password: Option<String>,
    ) -> Result<ItemSummary> {
        self.gated_item(
            id,
            password,
            |u| u.vault.require_active(id),
            |u| u.vault.history_restore(id, version),
        )
    }

    // ------------------------------------------------------------------ backup targets
    //
    // Network calls (the probe, a drain) never run while the state lock is held: each command
    // visits the lock briefly, talks to the network, then visits it again.

    pub fn backup_list(&self) -> Result<Vec<BackupTargetView>> {
        self.with_unlocked(|u| u.vault.backup_views())
    }

    pub fn backup_add(&self, input: BackupTargetInput) -> Result<BackupTargetView> {
        self.backup_save(None, input)
    }

    pub fn backup_update(&self, id: &str, input: BackupTargetInput) -> Result<BackupTargetView> {
        self.backup_save(Some(id), input)
    }

    fn backup_save(&self, id: Option<&str>, input: BackupTargetInput) -> Result<BackupTargetView> {
        // changing where the vault is copied to needs the master password, not just the PIN
        self.require_full()?;
        let (prepared, runtime) = self.with_unlocked(|u| {
            Ok((
                u.vault.backup_prepare(id, &input)?,
                u.vault.backup_runtime()?,
            ))
        })?;
        drop(input);
        let report = probe_config(&runtime, &prepared.config);
        self.with_unlocked(|u| u.vault.backup_commit(prepared, &report))
    }

    pub fn backup_remove(&self, id: &str) -> Result<()> {
        self.require_full()?;
        self.with_unlocked(|u| u.vault.backup_remove(id))
    }

    pub fn backup_set_master(&self, id: &str) -> Result<()> {
        self.require_full()?;
        self.with_unlocked(|u| u.vault.backup_set_master(id))
    }

    /// Drain the queue of one target (or all) now, then report the fresh health.
    pub fn backup_run(&self, id: Option<&str>) -> Result<Vec<BackupTargetView>> {
        let replicator = self.with_unlocked(|u| {
            if let Some(id) = id {
                u.vault.backup_exists(id)?;
            }
            Ok(u.vault.backup_replicator())
        })?;
        if let Some(r) = replicator {
            r.run_now(id);
        }
        self.with_unlocked(|u| u.vault.backup_views())
    }

    /// Is there anything a flush would send? `false` while locked (the settings and keys are not
    /// there), and when no enabled target has queued jobs.
    pub fn sync_pending(&self) -> bool {
        self.with_unlocked(|u| Ok(u.vault.backup_pending() > 0))
            .unwrap_or(false)
    }

    /// Trigger 3 (app exit or backgrounding): send the queue within `deadline`. `true` when
    /// nothing is left to send (also while the vault is locked or has no targets). The state
    /// lock is *not* held while the network is used.
    pub fn flush_sync(&self, deadline: Duration) -> bool {
        let replicator = self.with_unlocked(|u| Ok(u.vault.backup_replicator()));
        match replicator {
            Ok(Some(r)) => r.flush(deadline),
            _ => true,
        }
    }

    /// Trigger 4 (the device came back online): pull what other devices changed and send what is queued. Returns at
    /// once. A locked vault is a no-op.
    pub fn sync_request(&self) {
        let _ = self.with_unlocked(|u| {
            u.vault.request_sync();
            Ok(())
        });
    }

    /// The stored secret access key, after the master password is verified again.
    pub fn backup_reveal_secret(&self, id: &str, password: String) -> Result<String> {
        self.gated(
            password,
            |u| u.vault.backup_exists(id),
            |u| u.vault.backup_secret(id),
        )
    }
}

#[path = "session_drive.rs"]
mod drive_ops;

/// Owner-only access to the vault file (and, because SQLite copies the mode, its `-wal`/`-shm`).
#[cfg(unix)]
fn restrict_file_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &std::path::Path) {}

#[cfg(test)]
impl Session {
    /// Run `f` against the unlocked vault (panics if locked). Lets tests inspect and tamper
    /// with the database.
    pub(crate) fn test_vault<T>(&self, f: impl FnOnce(&mut Vault) -> T) -> T {
        self.with_unlocked(|u| Ok(f(&mut u.vault))).unwrap()
    }

    /// The answer to the sum made for the last permanent delete that asked for one.
    pub(crate) fn test_purge_answer(&self) -> Option<u32> {
        self.with_unlocked(|u| Ok(u.purge_sum.as_ref().map(|s| s.answer)))
            .unwrap()
    }

    pub(crate) fn test_token_count(&self) -> usize {
        self.with_unlocked(|u| Ok(u.tokens.len())).unwrap()
    }
}
