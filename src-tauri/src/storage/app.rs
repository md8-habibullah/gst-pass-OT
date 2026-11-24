//! The whole app's state: the account registry, the one selected profile and its vault session.
//!
//! Tauri-free and fully testable; `crate::commands` is a thin wrapper over [`AppCore`].
//!
//! * Several profiles (accounts) exist on a device; each is a separate vault with its own master
//!   password (`profiles.rs` explains the on-disk layout).
//! * Exactly one profile is *selected* at a time and at most one is unlocked: selecting another
//!   profile first locks (wipes) the current one.
//! * Every item command acts on the selected profile's [`Session`]; with nothing selected they
//!   answer `locked` (and `vault_unlock` answers `no_vault`).

use super::backup::prefs::{GeneratorPrefs, PrefsView};
use super::backup::{
    probe_config, restore, BackupRuntime, BackupTargetInput, BackupTargetView, BackupTestResult,
    TargetConfig,
};
use super::db::Store;
use super::device::DeviceSettings;
use super::error::{Result, StorageError};
use super::files::{self, FilePicker};
use super::links::{self, NoOpener, UrlOpener};
use super::model::{
    BeginEdit, ItemDraft, ItemSummary, ItemView, TotpNow, TrashEntry, VaultStatus, VersionEntry,
};
use super::profiles::{
    name_key, normalize_display_name, Entry, ProfileInfo, ProfileList, Registry, LEGACY_DIR,
    VAULTS_DIR, VAULT_FILE,
};
use super::session::{Session, SessionConfig};
use super::uuid;
use crate::crypto::{now_millis, VaultKeyBundle};
use crate::gate::GateCore;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long a flush on exit or backgrounding may take in total, across all targets.
pub const EXIT_FLUSH: Duration = Duration::from_secs(8);

mod gate_api;

// The demo vault and the device settings (time zone, time check). A child module, like the two above.
#[path = "app_demo.rs"]
mod demo_commands;

// The importer / exporter commands (slice Q4). A child module, so it can reach the private
// helpers (`session`, `active_session`, `reg`) like the methods above.
#[path = "app_import.rs"]
mod import_commands;

struct Active {
    id: String,
    session: Arc<Session>,
}

/// Result of a vault export: where the file was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedFile {
    pub path: String,
}

/// Answer of `sync_flush`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFlush {
    /// Nothing is left in any enabled target's queue.
    pub complete: bool,
}

pub struct AppCore {
    dir: PathBuf,
    cfg: SessionConfig,
    picker: Arc<dyn FilePicker>,
    /// How backup targets are reached; every session this core makes shares it.
    runtime: Arc<BackupRuntime>,
    /// Shows a web link in the system browser (`open_url`).
    opener: Arc<dyn UrlOpener>,
    /// Q2 disguise: the launcher, its last note, the change lock (`app_disguise.rs`).
    pub(super) disguise: super::app_disguise::DisguiseRuntime,
    /// Serializes the operations that create or delete a profile's folder (create, import,
    /// remove), so two of them can never trip over each other's half-made folders.
    /// Lock order: `ops`, then `registry`, then `active`.
    ops: Mutex<()>,
    registry: Mutex<Registry>,
    /// Shared with the idle timer's thread (`gate_api::open_gate`), which locks the vault.
    active: Arc<Mutex<Option<Active>>>,
    /// Stage 1 (the optional app gate), the attempt counter, the idle timer and the wipe's
    /// in-memory half (`crate::gate`, glued in `app/gate_api.rs`).
    gate: GateCore,
    /// `device.json`: time zone, first-run flag, time check, demo vault id. Survives the erase (`device.rs`).
    device: Mutex<DeviceSettings>,
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Create the data folder (owner-only on Unix).
fn prepare_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Remove a half-made profile folder, and the `vaults/` folder if that leaves it empty.
fn discard_folder(root: &Path, folder: &Path) {
    let _ = std::fs::remove_dir_all(folder);
    let _ = std::fs::remove_dir(root.join(VAULTS_DIR));
}

impl AppCore {
    /// Open the app's data folder: read (or rebuild) the registry, adopt a legacy `vault.db`.
    /// No profile is selected and nothing is unlocked.
    pub fn open(dir: &Path, cfg: SessionConfig, picker: Arc<dyn FilePicker>) -> Result<Self> {
        prepare_dir(dir)?;
        let registry = Registry::load(dir)?;
        let active = Arc::new(Mutex::new(None));
        let gate = gate_api::open_gate(dir, &cfg, &active)?;
        // From here on every timestamp is checked UTC when there is such a time (see `crate::clock`).
        crate::crypto::install_time_source(|| crate::clock::TrustedClock::global().now_ms());
        let device = super::device::load(dir);
        crate::clock::TrustedClock::global().set_enabled(device.time_check);
        Ok(Self {
            dir: dir.to_path_buf(),
            cfg,
            picker,
            runtime: BackupRuntime::production(),
            opener: Arc::new(NoOpener),
            disguise: Default::default(),
            ops: Mutex::new(()),
            registry: Mutex::new(registry),
            active,
            gate,
            device: Mutex::new(device),
        })
    }

    /// Use another way of reaching backup targets (tests use in-memory buckets).
    #[must_use]
    pub fn with_backup_runtime(mut self, runtime: Arc<BackupRuntime>) -> Self {
        self.runtime = runtime;
        self
    }

    /// Use another way of showing links (the app installs the system opener).
    #[must_use]
    pub fn with_opener(mut self, opener: Arc<dyn UrlOpener>) -> Self {
        self.opener = opener;
        self
    }

    fn new_session(&self, path: PathBuf) -> Session {
        Session::with_runtime(path, self.cfg, Arc::clone(&self.runtime))
    }

    pub fn data_dir(&self) -> &Path {
        &self.dir
    }

    /// The selected profile's session, for white-box tests.
    #[cfg(test)]
    pub(crate) fn test_session(&self) -> Arc<Session> {
        self.session().expect("a profile is selected")
    }

    fn reg(&self) -> MutexGuard<'_, Registry> {
        relock(&self.registry)
    }

    fn active_session(&self) -> Option<(String, Arc<Session>)> {
        relock(&self.active)
            .as_ref()
            .map(|a| (a.id.clone(), Arc::clone(&a.session)))
    }

    /// The selected profile's session; `locked` when nothing is selected.
    fn session(&self) -> Result<Arc<Session>> {
        self.active_session()
            .map(|(_, s)| s)
            .ok_or(StorageError::Locked)
    }

    /// Make `session` the selected one, wiping whatever was selected before.
    fn activate(&self, id: String, session: Session) {
        let mut active = relock(&self.active);
        if let Some(old) = active.take() {
            old.session.lock();
        }
        *active = Some(Active {
            id,
            session: Arc::new(session),
        });
    }

    // ------------------------------------------------------------------ profiles

    pub fn profile_list(&self) -> Result<ProfileList> {
        let selected = self.active_session().map(|(id, _)| id);
        let mut list = self.reg().list(selected.as_deref());
        let demo = relock(&self.device).demo_profile.clone();
        for p in &mut list.profiles {
            p.demo = demo.as_deref() == Some(p.id.as_str());
        }
        Ok(list)
    }

    /// Create a new account with its own vault and master password, select it and leave it
    /// unlocked. Nothing is left behind if any step fails.
    pub fn profile_create(&self, display_name: &str, password: String) -> Result<ProfileInfo> {
        let _ops = relock(&self.ops);
        let name = normalize_display_name(display_name)?;
        if self.reg().key_taken(&name_key(&name), None) {
            return Err(StorageError::ProfileNameTaken);
        }
        let id = uuid::new_v4()?;
        let dir_rel = format!("{VAULTS_DIR}/{id}");
        let folder = self.dir.join(&dir_rel);
        let session = self.new_session(folder.join(VAULT_FILE));
        let made = self
            .make_profile_folder(&folder)
            // Slow (Argon2id + ML-KEM key generation); no registry or session lock is held.
            .and_then(|()| session.create(password));
        if let Err(e) = made {
            discard_folder(&self.dir, &folder);
            return Err(e);
        }
        let now = now_millis();
        let entry = Entry {
            id: id.clone(),
            display_name: name,
            created_at: now,
            last_used_at: Some(now),
            dir: dir_rel,
        };
        let info = entry.info();
        if let Err(e) = self.reg().add(entry) {
            session.lock(); // closes the database before its folder goes away
            drop(session);
            discard_folder(&self.dir, &folder);
            return Err(e);
        }
        self.activate(id, session);
        Ok(info)
    }

    /// Create `vaults/<id>` (and `vaults/` if needed), both owner-only.
    fn make_profile_folder(&self, folder: &Path) -> Result<()> {
        files::create_private_dir(&self.dir.join(VAULTS_DIR))?;
        files::create_private_dir(folder)?;
        Ok(())
    }

    /// Select an account (still locked). Whatever was unlocked before is wiped.
    pub fn profile_select(&self, id: &str) -> Result<()> {
        let session = {
            let mut reg = self.reg();
            let path = {
                let e = reg.get(id).ok_or(StorageError::NotFound)?;
                reg.vault_path(e)
            };
            // Remembering the last account is a convenience: never block a login on it.
            let _ = reg.touch(id);
            self.new_session(path)
        };
        self.activate(id.to_string(), session);
        Ok(())
    }

    /// Rename the selected, unlocked account.
    pub fn profile_rename(&self, id: &str, display_name: &str) -> Result<ProfileInfo> {
        let name = normalize_display_name(display_name)?;
        if self.reg().get(id).is_none() {
            return Err(StorageError::NotFound);
        }
        match self.active_session() {
            Some((active_id, s)) if active_id == id && s.status()?.unlocked => {}
            _ => return Err(StorageError::Locked),
        }
        self.reg().rename(id, &name).map(|p| self.marked(p))
    }

    /// Delete an account from this device: its master password (verified with Argon2id even if it
    /// is not the selected one) and its name (or `CONFIRM`) are both required. Only that
    /// account's own files are deleted.
    pub fn profile_remove(&self, id: &str, password: String, confirmation: &str) -> Result<()> {
        let _ops = relock(&self.ops);
        let (entry, vault_path) = {
            let reg = self.reg();
            let e = reg.get(id).ok_or(StorageError::NotFound)?.clone();
            let p = reg.vault_path(&e);
            (e, p)
        };
        let typed = confirmation.trim();
        if typed != "CONFIRM" && typed != entry.display_name {
            return Err(StorageError::ConfirmationMismatch);
        }
        // A temporary session reads the bundle from disk and runs Argon2id; a profile whose vault
        // file is already gone has nothing left to protect.
        let probe = self.new_session(vault_path.clone());
        if probe.has_vault_file()? {
            probe.check_password(password)?;
        }
        drop(probe);

        {
            let mut active = relock(&self.active);
            if active.as_ref().is_some_and(|a| a.id == id) {
                if let Some(a) = active.take() {
                    a.session.lock();
                }
            }
        }
        self.delete_profile_files(&entry)?;
        self.reg().remove(id)?;
        self.forget_demo_if(id);
        Ok(())
    }

    /// Delete exactly the files that belong to `entry`, nothing else.
    fn delete_profile_files(&self, entry: &Entry) -> Result<()> {
        let reg = self.reg();
        if let Some(folder) = reg.profile_dir(entry) {
            // The registry validated `dir` as `vaults/<id>`; check once more that it is a real
            // folder directly under `vaults/` (not a link out of the data folder).
            let meta = match std::fs::symlink_metadata(&folder) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => {
                    return Err(StorageError::Storage(
                        "The account's files could not be deleted.".into(),
                    ))
                }
            };
            let vaults_dir = self.dir.join(VAULTS_DIR);
            if !meta.is_dir() || folder.parent() != Some(vaults_dir.as_path()) {
                return Err(StorageError::Storage(
                    "The account's folder is not where it should be, so nothing was deleted."
                        .into(),
                ));
            }
            std::fs::remove_dir_all(&folder)?;
            let _ = std::fs::remove_dir(self.dir.join(VAULTS_DIR));
        } else {
            debug_assert_eq!(entry.dir, LEGACY_DIR);
            for name in [VAULT_FILE, "vault.db-wal", "vault.db-shm"] {
                match std::fs::remove_file(self.dir.join(name)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => {
                        return Err(StorageError::Storage(
                            "The account's files could not be deleted.".into(),
                        ))
                    }
                }
            }
        }
        Ok(())
    }

    /// Import a vault file chosen in the native dialog as a new account (still locked: it opens
    /// with the master password it was created with). `None` if the dialog is cancelled.
    pub fn profile_import_file(&self, display_name: &str) -> Result<Option<ProfileInfo>> {
        let _ops = relock(&self.ops);
        let name = normalize_display_name(display_name)?;
        if self.reg().key_taken(&name_key(&name), None) {
            return Err(StorageError::ProfileNameTaken);
        }
        let Some(src) = self.picker.open_vault_file()? else {
            return Ok(None);
        };
        if files::is_inside(&self.dir, &src) {
            return Err(StorageError::invalid(
                "Choose a file outside the app's own data folder.",
            ));
        }
        files::check_vault_header(&src)?;

        let id = uuid::new_v4()?;
        let dir_rel = format!("{VAULTS_DIR}/{id}");
        let folder = self.dir.join(&dir_rel);
        let result = self.import_into(&src, &folder, &id, &name, &dir_rel);
        match result {
            Ok(info) => {
                self.activate(id, self.new_session(folder.join(VAULT_FILE)));
                Ok(Some(info))
            }
            Err(e) => {
                discard_folder(&self.dir, &folder);
                Err(e)
            }
        }
    }

    fn import_into(
        &self,
        src: &Path,
        folder: &Path,
        id: &str,
        name: &str,
        dir_rel: &str,
    ) -> Result<ProfileInfo> {
        self.make_profile_folder(folder)?;
        let dest = folder.join(VAULT_FILE);
        files::copy_private(src, &dest)?;
        {
            // Deeper structural checks on the private copy, still without any password.
            let mut store = Store::open(&dest).map_err(|e| match e {
                StorageError::Storage(m) => StorageError::UnsupportedFile(m),
                other => other,
            })?;
            let unsupported = |m: &str| StorageError::UnsupportedFile(m.into());
            let bundle = store
                .bundle_json()
                .map_err(|_| unsupported("That vault file could not be read."))?
                .ok_or_else(|| unsupported("That file does not contain a vault."))?;
            VaultKeyBundle::from_json(&bundle)
                .map_err(|_| unsupported("The vault's key data is damaged or unreadable."))?;
            store
                .device_id()?
                .ok_or_else(|| unsupported("That vault file is incomplete."))?;
            // A truncated or damaged copy is caught here, not at the first unlock.
            if !store
                .is_structurally_sound()
                .map_err(|_| unsupported("That vault file is damaged."))?
            {
                return Err(unsupported("That vault file is damaged."));
            }
            store
                .all_item_rows()
                .map_err(|_| unsupported("That vault file's contents could not be read."))?;
            // The copy is a new replica on this device.
            store.set_device_id(&uuid::new_v4()?)?;
        }
        let now = now_millis();
        let entry = Entry {
            id: id.to_string(),
            display_name: name.to_string(),
            created_at: now,
            last_used_at: Some(now),
            dir: dir_rel.to_string(),
        };
        let info = entry.info();
        self.reg().add(entry)?;
        Ok(info)
    }

    /// Write a copy of the selected, unlocked vault (still fully encrypted) to a place chosen in
    /// the native save dialog. Password-gated. `None` if the dialog is cancelled.
    pub fn vault_export_file(&self, password: String) -> Result<Option<ExportedFile>> {
        let (id, session) = self.active_session().ok_or(StorageError::Locked)?;
        let name = self
            .reg()
            .get(&id)
            .map_or_else(|| "vault".to_string(), |e| e.display_name.clone());
        let suggested = files::suggested_export_name(&name, now_millis());
        let picker = Arc::clone(&self.picker);
        let data_dir = self.dir.clone();
        let written = session.export_with(password, move || {
            let chosen = picker.save_vault_file(&suggested)?;
            if let Some(p) = &chosen {
                if files::is_inside(&data_dir, p) {
                    return Err(StorageError::invalid(
                        "Choose a place outside the app's own data folder.",
                    ));
                }
            }
            Ok(chosen)
        })?;
        Ok(written.map(|p| ExportedFile {
            path: p.to_string_lossy().into_owned(),
        }))
    }

    // ------------------------------------------------------------------ restore from a backup

    /// Fresh-device flow: connect to the backup target in `input`, download the key bundle, items,
    /// snapshots and tombstones into a **new** account (still locked: it opens with the master
    /// password the vault was created with). Needs no unlocked vault.
    ///
    /// Every rule that can fail without the network (the name, its uniqueness, the target
    /// settings) is checked **before** the first request. A failure at any later step removes the
    /// half-made folder completely and leaves the previously selected account selected.
    pub fn profile_restore_from_backup(
        &self,
        display_name: &str,
        input: BackupTargetInput,
    ) -> Result<ProfileInfo> {
        let _ops = relock(&self.ops);
        let name = normalize_display_name(display_name)?;
        if self.reg().key_taken(&name_key(&name), None) {
            return Err(StorageError::ProfileNameTaken);
        }
        let cfg = TargetConfig::from_input(&input, true)?;
        drop(input);

        let id = uuid::new_v4()?;
        let dir_rel = format!("{VAULTS_DIR}/{id}");
        let folder = self.dir.join(&dir_rel);
        match self.restore_into_folder(&cfg, &folder, &id, &name, &dir_rel) {
            Ok(info) => {
                self.activate(id, self.new_session(folder.join(VAULT_FILE)));
                Ok(info)
            }
            Err(e) => {
                discard_folder(&self.dir, &folder);
                Err(e)
            }
        }
    }

    fn restore_into_folder(
        &self,
        cfg: &TargetConfig,
        folder: &Path,
        id: &str,
        name: &str,
        dir_rel: &str,
    ) -> Result<ProfileInfo> {
        self.make_profile_folder(folder)?;
        let dest = folder.join(VAULT_FILE);
        // Owner-only from the first byte, so the `-wal` and `-shm` files are private too.
        drop(files::create_private_file(&dest)?);
        {
            let mut store = Store::open(&dest)?;
            restore::restore_into(&mut store, cfg, &self.runtime, &uuid::new_v4()?)?;
        }
        let now = now_millis();
        let entry = Entry {
            id: id.to_string(),
            display_name: name.to_string(),
            created_at: now,
            last_used_at: Some(now),
            dir: dir_rel.to_string(),
        };
        let info = entry.info();
        self.reg().add(entry)?;
        Ok(info)
    }

    // ------------------------------------------------------------------ backup targets

    /// The connection and conditional-write probe for settings that are not saved. Needs no
    /// unlocked vault (and no selected account). A target that cannot be reached is an answer
    /// (`ok: false` with a message), not an error; only invalid settings are errors.
    pub fn backup_test(&self, input: &BackupTargetInput) -> Result<BackupTestResult> {
        let cfg = TargetConfig::from_input(input, true)?;
        let report = probe_config(&self.runtime, &cfg);
        Ok(BackupTestResult {
            ok: report.ok,
            message: report.message,
            conditional_writes: report.conditional,
        })
    }

    pub fn backup_list(&self) -> Result<Vec<BackupTargetView>> {
        self.session()?.backup_list()
    }

    pub fn backup_add(&self, input: BackupTargetInput) -> Result<BackupTargetView> {
        self.session()?.backup_add(input)
    }

    pub fn backup_update(&self, id: &str, input: BackupTargetInput) -> Result<BackupTargetView> {
        self.session()?.backup_update(id, input)
    }

    pub fn backup_remove(&self, id: &str) -> Result<()> {
        self.session()?.backup_remove(id)
    }

    pub fn backup_set_master(&self, id: &str) -> Result<()> {
        self.session()?.backup_set_master(id)
    }

    pub fn backup_run(&self, id: Option<&str>) -> Result<Vec<BackupTargetView>> {
        self.session()?.backup_run(id)
    }

    pub fn backup_reveal_secret(&self, id: &str, password: String) -> Result<String> {
        self.session()?.backup_reveal_secret(id, password)
    }

    // ------------------------------------------------------------------ sync trigger 3

    /// Is an unlocked vault holding queued jobs for an enabled target? (What the window-close
    /// hook asks: if not, the window closes at once.)
    pub fn has_pending_sync(&self) -> bool {
        self.active_session().is_some_and(|(_, s)| s.sync_pending())
    }

    /// Trigger 3 (app exit or backgrounding): send the queued jobs, but never wait longer than
    /// `deadline` in total across all targets. `true` when nothing is left in any queue
    /// (also with the vault locked: there is nothing to send then). The queue is durable, so a
    /// `false` loses nothing: the rest goes at the next trigger.
    pub fn flush_for_exit(&self, deadline: Duration) -> bool {
        self.active_session()
            .is_none_or(|(_, s)| s.flush_sync(deadline))
    }

    /// The `sync_flush` command: the same bounded flush, for the webview to call when a phone app
    /// goes to the background. A locked vault (or none selected) is a no-op `{complete: true}`.
    pub fn sync_flush(&self) -> SyncFlush {
        SyncFlush {
            complete: self.flush_for_exit(EXIT_FLUSH),
        }
    }

    // ------------------------------------------------------------------ links

    /// Show a web address in the system browser (the only door the webview has to do that).
    pub fn open_url(&self, url: &str) -> Result<()> {
        let checked = links::validate_open_url(url)?;
        self.opener.open(&checked)
    }

    // ------------------------------------------------------------------ vault lifecycle

    pub fn status(&self) -> Result<VaultStatus> {
        // While the gate is closed the answer is the neutral "no account, locked": the UI polls
        // this command, and it must reveal no account name before Stage 1 is passed.
        match self.active_session().filter(|_| !self.gate_is_closed()) {
            None => Ok(VaultStatus {
                exists: false,
                unlocked: false,
                profile: None,
                damaged_items: 0,
                sync_revision: 0,
                reconciling: false,
                remote_damaged: 0,
            }),
            Some((id, session)) => {
                let mut st = session.status()?;
                st.profile = self.reg().get(&id).map(Entry::info).map(|p| self.marked(p));
                Ok(st)
            }
        }
    }

    pub fn unlock(&self, password: String) -> Result<()> {
        let (id, session) = self.active_session().ok_or(StorageError::NoVault)?;
        session.unlock(password)?;
        let _ = self.reg().touch(&id);
        Ok(())
    }

    pub fn lock(&self) {
        if let Some((_, session)) = self.active_session() {
            session.lock();
        }
    }

    pub fn change_password(&self, old_password: String, new_password: String) -> Result<()> {
        self.session()?.change_password(old_password, new_password)
    }

    // ------------------------------------------------------------------ items (selected profile)

    pub fn item_list(&self) -> Result<Vec<ItemSummary>> {
        self.session()?.item_list()
    }

    pub fn prefs_get(&self) -> Result<PrefsView> {
        self.session()?.prefs_get()
    }

    pub fn prefs_set_generator(&self, generator: GeneratorPrefs) -> Result<PrefsView> {
        self.session()?.prefs_set_generator(generator)
    }

    pub fn item_get(&self, id: &str) -> Result<ItemView> {
        self.session()?.item_get(id)
    }

    pub fn item_reveal(&self, id: &str, password: Option<String>) -> Result<ItemDraft> {
        self.session()?.item_reveal(id, password)
    }

    pub fn item_begin_edit(&self, id: &str, password: Option<String>) -> Result<BeginEdit> {
        self.session()?.item_begin_edit(id, password)
    }

    pub fn item_create(&self, draft: ItemDraft) -> Result<ItemSummary> {
        self.session()?.item_create(draft)
    }

    pub fn item_update(
        &self,
        id: &str,
        token: &str,
        expected_version: u64,
        draft: ItemDraft,
    ) -> Result<ItemSummary> {
        self.session()?
            .item_update(id, token, expected_version, draft)
    }

    pub fn item_trash(&self, id: &str) -> Result<()> {
        self.session()?.item_trash(id)
    }

    pub fn item_restore(&self, id: &str) -> Result<ItemSummary> {
        self.session()?.item_restore(id)
    }

    /// "Got it" for the copy another device sent (clears `changedElsewhere`).
    pub fn item_review_other_device(&self, id: &str) -> Result<()> {
        self.session()?.item_review_other_device(id)
    }

    pub fn item_archive(&self, id: &str) -> Result<ItemSummary> {
        self.session()?.item_archive(id)
    }

    pub fn item_unarchive(&self, id: &str) -> Result<ItemSummary> {
        self.session()?.item_unarchive(id)
    }

    pub fn item_clone(&self, id: &str, password: Option<String>) -> Result<ItemSummary> {
        self.session()?.item_clone(id, password)
    }

    pub fn trash_list(&self) -> Result<Vec<TrashEntry>> {
        self.session()?.trash_list()
    }

    pub fn item_purge(&self, id: &str, password: String, confirmation: &str) -> Result<()> {
        self.session()?.item_purge(id, password, confirmation)
    }

    pub fn history_list(&self, id: &str) -> Result<Vec<VersionEntry>> {
        self.session()?.history_list(id)
    }

    pub fn history_get(
        &self,
        id: &str,
        version: u64,
        password: Option<String>,
    ) -> Result<ItemDraft> {
        self.session()?.history_get(id, version, password)
    }

    pub fn history_restore(
        &self,
        id: &str,
        version: u64,
        password: Option<String>,
    ) -> Result<ItemSummary> {
        self.session()?.history_restore(id, version, password)
    }

    pub fn totp_now(&self, id: &str, password: Option<String>) -> Result<TotpNow> {
        self.session()?.totp_now(id, password)
    }
}

#[path = "app_drive.rs"]
mod drive_ops;

impl Drop for AppCore {
    /// Quitting locks the open vault: the keys are wiped and the replication worker is stopped.
    fn drop(&mut self) {
        if let Some(active) = relock(&self.active).take() {
            active.session.lock();
        }
    }
}
