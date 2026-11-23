//! The unlocked vault: item lifecycle, immutable history, restricted trash, purge, TOTP.
//!
//! A [`Vault`] exists only while the vault is unlocked; dropping it closes the database and
//! wipes the root key. Password checks (the re-prompt guard) are **not** done here: the
//! [`Session`](super::session::Session) verifies the master password before it calls the gated
//! operations, and the trash policy ("title and time only") is enforced here, in Rust.

use super::db::{ItemRow, Store};
use super::error::{Result, StorageError};
use super::model::{
    changed_fields, totp_account, ItemDraft, ItemEvent, ItemPayload, ItemSummary, ItemView,
    TotpNow, TrashEntry, VersionEntry,
};
use super::uuid;
use crate::crypto::{now_millis, KeyPurpose, VaultKeyBundle, VaultRootKey};
use std::collections::HashMap;
use zeroize::Zeroizing;

/// In-memory index entry. Trashed items keep only what the trash view may show.
enum IndexEntry {
    Active(ItemSummary),
    Trashed {
        title: String,
        deleted_at: u64,
        #[allow(dead_code)]
        version: u64,
    },
}

pub struct Vault {
    store: Store,
    root: VaultRootKey,
    bundle: VaultKeyBundle,
    index: HashMap<String, IndexEntry>,
    damaged: Vec<String>,
}

fn check_id(id: &str) -> Result<()> {
    if uuid::is_valid(id) {
        Ok(())
    } else {
        Err(StorageError::invalid("That is not a valid item id."))
    }
}

fn next_version(v: u64) -> Result<u64> {
    v.checked_add(1)
        .ok_or_else(|| StorageError::Storage("This item has too many versions.".into()))
}

impl Vault {
    /// Take ownership of an opened store and the unlocked root key; build the in-memory index.
    ///
    /// An item whose stored data fails authentication is skipped (and counted) instead of
    /// blocking the whole vault; every other item stays usable.
    pub fn open(store: Store, bundle: VaultKeyBundle, root: VaultRootKey) -> Result<Self> {
        let mut vault = Self {
            store,
            root,
            bundle,
            index: HashMap::new(),
            damaged: Vec::new(),
        };
        for row in vault.store.all_item_rows()? {
            match vault.open_row(&row) {
                Ok(payload) => vault.index_insert(&payload, row.version),
                Err(_) => vault.damaged.push(row.id),
            }
        }
        Ok(vault)
    }

    // ------------------------------------------------------------------ internals

    fn open_row(&self, row: &ItemRow) -> Result<ItemPayload> {
        let opened =
            self.root
                .open_object(KeyPurpose::Item, &row.id, Some(row.version), &row.blob)?;
        let payload: ItemPayload = serde_json::from_slice(&opened.plaintext)?;
        payload.check(&row.id)?;
        Ok(payload)
    }

    fn open_snapshot(&self, id: &str, version: u64, blob: &[u8]) -> Result<(ItemPayload, u64)> {
        let opened = self
            .root
            .open_object(KeyPurpose::Version, id, Some(version), blob)?;
        let payload: ItemPayload = serde_json::from_slice(&opened.plaintext)?;
        payload.check(id)?;
        Ok((payload, opened.header.timestamp))
    }

    fn load_latest(&self, id: &str) -> Result<(ItemPayload, u64)> {
        let row = self.store.item_row(id)?.ok_or(StorageError::NotFound)?;
        let payload = self.open_row(&row)?;
        Ok((payload, row.version))
    }

    fn index_insert(&mut self, payload: &ItemPayload, version: u64) {
        let entry = if payload.deleted {
            IndexEntry::Trashed {
                title: payload.title.clone(),
                deleted_at: payload.deleted_at.unwrap_or(payload.updated_at),
                version,
            }
        } else {
            IndexEntry::Active(payload.summary(version))
        };
        self.index.insert(payload.id.clone(), entry);
    }

    /// Seal the payload twice (latest state and immutable snapshot, independent keys and
    /// nonces) and write both in one transaction.
    fn commit(
        &mut self,
        payload: ItemPayload,
        prev_version: Option<u64>,
        version: u64,
    ) -> Result<ItemSummary> {
        let ts = payload.updated_at;
        let json = Zeroizing::new(serde_json::to_vec(&payload)?);
        let item_blob = self
            .root
            .seal_object(KeyPurpose::Item, &payload.id, version, ts, &json)?;
        let snapshot_blob =
            self.root
                .seal_object(KeyPurpose::Version, &payload.id, version, ts, &json)?;
        drop(json);
        self.store.commit_state(
            &payload.id,
            prev_version,
            version,
            ts,
            &item_blob,
            &snapshot_blob,
        )?;
        self.index_insert(&payload, version);
        Ok(payload.summary(version))
    }

    fn entry(&self, id: &str) -> Result<&IndexEntry> {
        check_id(id)?;
        self.index.get(id).ok_or(StorageError::NotFound)
    }

    /// Ok for active items; `InTrash` / `NotFound` otherwise. The single place the trash
    /// policy for reading is decided.
    pub fn require_active(&self, id: &str) -> Result<()> {
        match self.entry(id)? {
            IndexEntry::Active(_) => Ok(()),
            IndexEntry::Trashed { .. } => Err(StorageError::InTrash),
        }
    }

    pub fn require_trashed(&self, id: &str) -> Result<()> {
        match self.entry(id)? {
            IndexEntry::Trashed { .. } => Ok(()),
            IndexEntry::Active(_) => Err(StorageError::NotInTrash),
        }
    }

    // ------------------------------------------------------------------ vault-level

    pub fn bundle(&self) -> &VaultKeyBundle {
        &self.bundle
    }

    /// Persist a bundle produced by `change_password`, only if `old` is still the stored one.
    pub fn replace_bundle(&mut self, old: &VaultKeyBundle, new: VaultKeyBundle) -> Result<()> {
        self.store
            .replace_bundle(&old.to_json()?, &new.to_json()?)?;
        self.bundle = new;
        Ok(())
    }

    /// Number of stored items whose data failed authentication at unlock.
    pub fn damaged_items(&self) -> usize {
        self.damaged.len()
    }

    // ------------------------------------------------------------------ reading

    /// Active items, most recently changed first. Served from the index, no decryption.
    pub fn list_active(&self) -> Vec<ItemSummary> {
        let mut out: Vec<ItemSummary> = self
            .index
            .values()
            .filter_map(|e| match e {
                IndexEntry::Active(s) => Some(s.clone()),
                IndexEntry::Trashed { .. } => None,
            })
            .collect();
        out.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    /// Trash listing: id, title, deletion time. Nothing else is ever exposed.
    pub fn list_trash(&self) -> Vec<TrashEntry> {
        let mut out: Vec<TrashEntry> = self
            .index
            .iter()
            .filter_map(|(id, e)| match e {
                IndexEntry::Trashed {
                    title, deleted_at, ..
                } => Some(TrashEntry {
                    id: id.clone(),
                    title: title.clone(),
                    deleted_at: *deleted_at,
                }),
                IndexEntry::Active(_) => None,
            })
            .collect();
        out.sort_by(|a, b| {
            b.deleted_at
                .cmp(&a.deleted_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    /// Masked view of an active item (no secret values).
    pub fn get_view(&self, id: &str) -> Result<ItemView> {
        self.require_active(id)?;
        let (payload, version) = self.load_latest(id)?;
        Ok(payload.view(version))
    }

    /// Full decrypted content and its version. Password-gated by the caller.
    pub fn reveal(&self, id: &str) -> Result<(ItemDraft, u64)> {
        self.require_active(id)?;
        let (payload, version) = self.load_latest(id)?;
        Ok((payload.to_draft(), version))
    }

    pub fn current_version(&self, id: &str) -> Result<u64> {
        self.require_active(id)?;
        match self.entry(id)? {
            IndexEntry::Active(s) => Ok(s.version),
            IndexEntry::Trashed { .. } => Err(StorageError::InTrash),
        }
    }

    /// Current one-time code. The secret never leaves this function.
    pub fn totp_now(&self, id: &str) -> Result<TotpNow> {
        self.require_active(id)?;
        let (payload, _) = self.load_latest(id)?;
        let secret = payload
            .fields
            .get("totp")
            .ok_or_else(|| StorageError::invalid("This item has no one-time code."))?;
        let account = totp_account(secret)?;
        let secs = now_millis() / 1000;
        let code = account.code_at(secs);
        Ok(TotpNow {
            code: code.code,
            period: code.period,
            digits: code.digits,
            seconds_remaining: code.seconds_remaining,
            valid_until_ms: (secs + code.seconds_remaining) * 1000,
        })
    }

    // ------------------------------------------------------------------ writing

    pub fn create_item(&mut self, draft: ItemDraft) -> Result<ItemSummary> {
        let id = uuid::new_v4()?;
        self.create_item_with_id(id, draft)
    }

    /// Create with a chosen id (used by `create_item`, and by tests of the tombstone rule).
    pub(crate) fn create_item_with_id(
        &mut self,
        id: String,
        mut draft: ItemDraft,
    ) -> Result<ItemSummary> {
        check_id(&id)?;
        draft.normalize()?;
        let payload = ItemPayload::new(id, &draft, now_millis());
        self.commit(payload, None, 1)
    }

    /// Edit an active item. `expected_version` is the version the editor was opened on.
    /// Saving without any change writes nothing and returns the current summary.
    pub fn update_item(
        &mut self,
        id: &str,
        expected_version: u64,
        mut draft: ItemDraft,
    ) -> Result<ItemSummary> {
        self.require_active(id)?;
        let (old, version) = self.load_latest(id)?;
        if version != expected_version {
            return Err(StorageError::VersionConflict);
        }
        draft.normalize()?;
        if draft.item_type != old.item_type {
            return Err(StorageError::invalid("An item's type cannot be changed."));
        }
        let changed = changed_fields(&old.to_draft(), &draft);
        if changed.is_empty() {
            return Ok(old.summary(version));
        }
        let mut next = old.clone();
        next.apply_draft(&draft);
        next.updated_at = now_millis().max(old.updated_at);
        next.event = ItemEvent::Edited;
        next.restored_from = None;
        next.changed_fields = changed;
        self.commit(next, Some(version), next_version(version)?)
    }

    /// Soft delete: a new version with `deleted = true`. Nothing is removed.
    pub fn trash_item(&mut self, id: &str) -> Result<()> {
        self.require_active(id)?;
        let (old, version) = self.load_latest(id)?;
        let now = now_millis().max(old.updated_at);
        let mut next = old.clone();
        next.deleted = true;
        next.deleted_at = Some(now);
        next.updated_at = now;
        next.event = ItemEvent::Trashed;
        next.restored_from = None;
        next.changed_fields = Vec::new();
        self.commit(next, Some(version), next_version(version)?)?;
        Ok(())
    }

    /// Bring an item back from the trash with a new version.
    pub fn restore_item(&mut self, id: &str) -> Result<ItemSummary> {
        self.require_trashed(id)?;
        let (old, version) = self.load_latest(id)?;
        let mut next = old.clone();
        next.deleted = false;
        next.deleted_at = None;
        next.updated_at = now_millis().max(old.updated_at);
        next.event = ItemEvent::Restored;
        next.restored_from = None;
        next.changed_fields = Vec::new();
        self.commit(next, Some(version), next_version(version)?)
    }

    /// Everything that must hold before a purge is attempted: the item is in the trash and
    /// `confirmation` is its title or the word `CONFIRM`. Cheap, so the session runs it before
    /// spending an Argon2id run on the password.
    pub fn purge_checks(&self, id: &str, confirmation: &str) -> Result<()> {
        let title = match self.entry(id)? {
            IndexEntry::Trashed { title, .. } => title,
            IndexEntry::Active(_) => return Err(StorageError::NotInTrash),
        };
        let typed = confirmation.trim();
        if typed != "CONFIRM" && typed != title.trim() {
            return Err(StorageError::ConfirmationMismatch);
        }
        Ok(())
    }

    /// Permanent deletion (tombstone, then every row of the item). The master password is
    /// checked by the caller; the in-trash state and the confirmation text are re-checked here.
    pub fn purge_item(&mut self, id: &str, confirmation: &str) -> Result<()> {
        self.purge_checks(id, confirmation)?;
        self.store.purge(id, now_millis())?;
        self.index.remove(id);
        Ok(())
    }

    // ------------------------------------------------------------------ history

    /// Timeline of an active item, newest first. Event, time and field *names* only.
    pub fn history_list(&self, id: &str) -> Result<Vec<VersionEntry>> {
        self.require_active(id)?;
        let mut out = Vec::new();
        for row in self.store.version_rows(id)? {
            let (payload, timestamp) = self.open_snapshot(id, row.version, &row.blob)?;
            out.push(VersionEntry {
                version: row.version,
                timestamp,
                event: payload.event,
                changed_fields: payload.changed_fields.clone(),
                restored_from: payload.restored_from,
            });
        }
        Ok(out)
    }

    /// Content of one historical snapshot. Password-gated by the caller.
    pub fn history_get(&self, id: &str, version: u64) -> Result<ItemDraft> {
        self.require_active(id)?;
        let row = self
            .store
            .version_row(id, version)?
            .ok_or(StorageError::NotFound)?;
        let (payload, _) = self.open_snapshot(id, row.version, &row.blob)?;
        Ok(payload.to_draft())
    }

    /// Make an old snapshot the newest version (history stays linear; nothing is rewritten).
    pub fn history_restore(&mut self, id: &str, version: u64) -> Result<ItemSummary> {
        self.require_active(id)?;
        let (latest, current) = self.load_latest(id)?;
        let row = self
            .store
            .version_row(id, version)?
            .ok_or(StorageError::NotFound)?;
        let (snapshot, _) = self.open_snapshot(id, row.version, &row.blob)?;
        let target = snapshot.to_draft();
        let mut next = latest.clone();
        next.apply_draft(&target);
        next.changed_fields = changed_fields(&latest.to_draft(), &target);
        next.updated_at = now_millis().max(latest.updated_at);
        next.event = ItemEvent::RestoredFromVersion;
        next.restored_from = Some(version);
        self.commit(next, Some(current), next_version(current)?)
    }

    // ------------------------------------------------------------------ test support

    #[cfg(test)]
    pub fn store(&self) -> &Store {
        &self.store
    }

    #[cfg(test)]
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }
}
