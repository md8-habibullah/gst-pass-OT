//! Preferences that belong to the vault and follow it to every device. So far: the password
//! generator's last settings (length and character types), so the Generator page and the item
//! editor open with what the person chose last time, on every device, and the folders the person
//! made on purpose (so an empty folder can exist; the folders items sit in need no entry here).
//!
//! * One small JSON document, sealed with the vault's `meta` sub-key as the object `prefs`, kept
//!   in the `meta` table (key `prefs`, base64). Nothing readable is stored: a plain database
//!   copy shows a blob, like every other vault object.
//! * Replicated like the other vault objects, to every enabled target, as `vault/meta/prefs.enc`
//!   under the single-writer rule. It rides the existing outbox job kind `targets` with the ref
//!   `prefs` (the shared-targets snapshot has an empty ref), so no schema migration is needed.
//! * Pulled from the source target at unlock and on "Back up now" (one `HEAD`, a download only
//!   when the ETag changed), and restored with the vault.
//! * **The newest write wins.** The envelope's version is a millisecond stamp that is always
//!   higher than any version this device has seen, so a later change on any device replaces an
//!   earlier one, and a device with a wrong clock cannot lock the others out (it adopts the
//!   higher version it reads, and its next change goes above it).
//! * Fields this build does not know are kept when it saves, so an older build never erases a
//!   setting a newer one added.

use super::dbops::{self, JobKind};
use super::PREFS_META;
use crate::crypto::{envelope, now_millis, KeyPurpose};
use crate::storage::error::{Result, StorageError};
use crate::storage::vault::Vault;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

/// The envelope id of the sealed document.
pub const PREFS_ID: &str = "prefs";
/// The outbox `ref` that tells a `targets` job to send the preferences, not the target snapshot.
pub const PREFS_REF: &str = "prefs";

/// The generator accepts lengths from 4 to 128 (the sliders offer less; the front end clamps).
pub const MIN_LENGTH: u32 = 4;
pub const MAX_LENGTH: u32 = 128;

/// The password generator's last settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorPrefs {
    pub length: u32,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    pub avoid_ambiguous: bool,
}

impl GeneratorPrefs {
    /// A saved setting must be one the generator can actually use.
    pub fn check(&self) -> Result<()> {
        if !(MIN_LENGTH..=MAX_LENGTH).contains(&self.length) {
            return Err(StorageError::invalid(
                "The password length must be between 4 and 128.",
            ));
        }
        if !(self.lower || self.upper || self.digits || self.symbols) {
            return Err(StorageError::invalid("Select at least one character type."));
        }
        Ok(())
    }
}

/// Everything the vault remembers for the person. `v` is the document's own format version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    #[serde(default = "one")]
    pub v: u32,
    #[serde(default)]
    pub generator: Option<GeneratorPrefs>,
    /// Folders made on purpose, as `Work/Clients` paths. Written only when there is one, so a vault that never made a
    /// folder is sealed exactly as an older build seals it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
    /// Settings a newer build added; carried through untouched.
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_json::Value>,
}

fn one() -> u32 {
    1
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            v: 1,
            generator: None,
            folders: Vec::new(),
            other: BTreeMap::new(),
        }
    }
}

impl Prefs {
    /// What the pages get: a stored generator setting that this build cannot use (written by
    /// something else) is treated as "nothing saved" rather than shown.
    pub fn usable(mut self) -> Self {
        if self.generator.as_ref().is_some_and(|g| g.check().is_err()) {
            self.generator = None;
        }
        self
    }
}

/// What the pages are given (`prefs_get`): only the settings this build understands, and only
/// those it can use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefsView {
    pub generator: Option<GeneratorPrefs>,
    /// The folders made on purpose, sorted. The screen adds the ones items sit in.
    pub folders: Vec<String>,
}

impl From<Prefs> for PrefsView {
    fn from(p: Prefs) -> Self {
        let p = p.usable();
        Self {
            generator: p.generator,
            folders: p.folders,
        }
    }
}

/// What a folder command did: the folder it made (the vault's spelling), how many items it changed, and the folder list now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderChange {
    /// The path that was made, for `folder_create`; `None` for the other commands.
    pub path: Option<String>,
    /// How many items got a new version because their folder changed.
    pub moved: usize,
    pub prefs: PrefsView,
}

/// What a pull made of the preferences object it read from the source target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefsVerdict {
    /// It is newer than this device's copy: taken over.
    Adopted,
    /// The same version (or the same bytes).
    Same,
    /// This device's copy is newer; the push side will replace the bucket's.
    LocalAhead,
    /// It does not authenticate (a flipped bit, a planted file): never applied, counted, and this
    /// device's own copy is queued to replace it.
    Damaged,
    /// It authenticates but this build cannot read it (a newer app wrote it): left exactly as it
    /// is, not counted, never overwritten by this build.
    Unreadable,
}

impl Vault {
    fn prefs_blob(&self) -> Option<Vec<u8>> {
        let b64 = self.store.meta(PREFS_META).ok().flatten()?;
        B64.decode(b64.trim()).ok()
    }

    /// The version of this device's copy (0 when there is none).
    fn prefs_version(&self) -> u64 {
        self.prefs_blob()
            .and_then(|b| envelope::peek_header(&b).ok())
            .map_or(0, |h| h.version)
    }

    /// The stored preferences. Nothing saved, or a stored copy that does not open, means the
    /// defaults: preferences must never get in the way of using the vault.
    pub fn prefs(&self) -> Prefs {
        let Some(blob) = self.prefs_blob() else {
            return Prefs::default();
        };
        self.root
            .open_object(KeyPurpose::Meta, PREFS_ID, None, &blob)
            .ok()
            .and_then(|o| serde_json::from_slice::<Prefs>(&o.plaintext).ok())
            .unwrap_or_default()
    }

    /// Remember the generator's settings and queue them for every enabled target. Saving what is
    /// already saved writes and sends nothing.
    pub fn set_generator_prefs(&mut self, generator: GeneratorPrefs) -> Result<Prefs> {
        generator.check()?;
        let mut prefs = self.prefs();
        if prefs.generator.as_ref() == Some(&generator) {
            return Ok(prefs);
        }
        prefs.generator = Some(generator);
        self.save_prefs(&prefs)?;
        Ok(prefs)
    }

    /// Replace the list of folders made on purpose (already cleaned and checked by the caller) and queue it for every
    /// enabled target. Saving what is already saved writes and sends nothing.
    pub(crate) fn set_folder_list(&mut self, mut folders: Vec<String>) -> Result<Prefs> {
        folders.sort_by_key(|f| f.to_lowercase());
        folders.dedup();
        let mut prefs = self.prefs();
        if prefs.folders == folders {
            return Ok(prefs);
        }
        prefs.folders = folders;
        self.save_prefs(&prefs)?;
        Ok(prefs)
    }

    /// Seal the preferences as the next version and queue them for the targets.
    fn save_prefs(&mut self, prefs: &Prefs) -> Result<()> {
        let json = Zeroizing::new(serde_json::to_vec(prefs)?);
        let now = now_millis();
        // Strictly increasing, so a newer write is always recognisably newer than an older one.
        let version = now.max(self.prefs_version().saturating_add(1));
        let blob = self
            .root
            .seal_object(KeyPurpose::Meta, PREFS_ID, version, now, &json)?;
        self.store.in_tx(|c| {
            dbops::set_meta(c, PREFS_META, &B64.encode(&blob))?;
            dbops::enqueue_for_enabled(c, JobKind::Targets, PREFS_REF, 0, 0, now)
        })?;
        self.wake_backup();
        Ok(())
    }

    /// The preferences object a pull read from `source`. It is authenticated and parsed before
    /// anything is stored; a newer one replaces this device's copy and is queued for the other
    /// targets (the one it came from already holds it).
    pub fn pull_prefs(&mut self, source: &str, bytes: &[u8]) -> PrefsVerdict {
        let Ok(header) = envelope::peek_header(bytes) else {
            return self.damaged_prefs(source);
        };
        if header.item_id != PREFS_ID || header.version == 0 {
            return self.damaged_prefs(source);
        }
        let Ok(opened) =
            self.root
                .open_object(KeyPurpose::Meta, PREFS_ID, Some(header.version), bytes)
        else {
            return self.damaged_prefs(source);
        };
        // Genuine, but written in a shape this build does not know: leave it alone.
        if serde_json::from_slice::<Prefs>(&opened.plaintext).is_err() {
            return PrefsVerdict::Unreadable;
        }
        drop(opened);
        let local = self.prefs_version();
        if header.version < local {
            return PrefsVerdict::LocalAhead;
        }
        if header.version == local {
            return PrefsVerdict::Same;
        }
        let now = now_millis();
        let stored = self.store.in_tx(|c| {
            dbops::set_meta(c, PREFS_META, &B64.encode(bytes))?;
            dbops::enqueue_for_enabled_except(c, JobKind::Targets, PREFS_REF, 0, 0, now, source)
        });
        if stored.is_err() {
            return PrefsVerdict::Damaged;
        }
        // The page reloads what it shows when this number changes.
        self.sync_revision = self.sync_revision.saturating_add(1);
        self.wake_backup();
        PrefsVerdict::Adopted
    }

    /// The copy in the bucket is not authentic. It is never applied; when this device has its own
    /// copy, that copy is queued for the source so the next push replaces the bad one (the pull
    /// remembers the bad object's ETag as the one to overwrite).
    fn damaged_prefs(&mut self, source: &str) -> PrefsVerdict {
        if self.prefs_blob().is_some() {
            let now = now_millis();
            let _ = self.store.in_tx(|c| {
                dbops::enqueue_for_target(c, source, JobKind::Targets, PREFS_REF, 0, 0, now)
            });
            self.wake_backup();
        }
        PrefsVerdict::Damaged
    }
}
