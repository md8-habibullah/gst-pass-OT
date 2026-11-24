//! Batch import and plaintext export on the unlocked vault (slice Q4).
//!
//! This file is a child module of `vault.rs` (`#[path]` declaration there), so it can use the
//! vault's private index and sealing helpers without widening anything.
//!
//! **One transaction.** [`Vault::import_batch`] seals every accepted item (its latest state and its
//! immutable version snapshot, two keys and two nonces, exactly as `create_item` / `update_item`
//! do), then writes all of them, their `versions` rows and the backup outbox jobs for every
//! enabled target inside a single SQLite transaction. A database error rolls everything back and
//! the in-memory index is not touched. An entry that fails validation (a field too long, an
//! invalid code) is counted as failed and does not stop the others. The backup worker is woken
//! once, after the commit.
//!
//! **Duplicates** ([`import_dup_index`](Vault::import_dup_index)): same item type, same title
//! (case-insensitive) and, for logins, the same username (case-insensitive) and the same primary
//! host. Only items that are in the vault *before* the import count (not the file's own earlier
//! records, and not the trash), so a file never hides its own records and importing the same
//! file twice with `skip_duplicates` adds nothing the second time.

use super::{IndexEntry, Vault};
use crate::crypto::{now_millis, KeyPurpose};
use crate::importer::entry::{dup_key, dup_key_parts, DupKey};
use crate::importer::export::ExportRecord;
use crate::importer::limits::{warn_title, MAX_WARNINGS};
use crate::importer::ImportMode;
use crate::storage::backup::dbops::{self, JobKind};
use crate::storage::error::{Result, StorageError};
use crate::storage::model::{
    changed_fields, CustomField, ItemDraft, ItemEvent, ItemPayload, ItemSummary, ItemType,
};
use crate::storage::uuid;
use rusqlite::params;
use std::collections::HashMap;
use zeroize::Zeroizing;

/// One item to import: its content, the time the source says it was created, and notes the
/// conversion made about it.
pub struct BatchItem {
    pub draft: ItemDraft,
    pub created: Option<u64>,
    pub warnings: Vec<String>,
}

/// What a batch did. `ids` has one slot per input item, in order: the id of the item that was
/// created or got a new version, `None` for one that was skipped or failed.
#[derive(Debug, Default)]
pub struct BatchOutcome {
    pub added: usize,
    pub appended: usize,
    pub skipped: usize,
    pub failed: usize,
    pub ids: Vec<Option<String>>,
    /// Conversion notes and the reason of every failure (capped).
    pub warnings: Vec<String>,
    dropped: usize,
}

impl BatchOutcome {
    fn note(&mut self, message: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(message);
        } else {
            self.dropped += 1;
        }
    }

    fn fail(&mut self, title: &str, reason: &str) {
        self.failed += 1;
        self.ids.push(None);
        self.note(format!(
            "\u{201c}{title}\u{201d} was not imported: {reason}"
        ));
    }

    fn finish(&mut self) {
        if self.dropped > 0 {
            let n = std::mem::take(&mut self.dropped);
            self.warnings.push(format!("{n} more notes are not shown."));
        }
    }
}

/// An item sealed and ready to be written.
struct Prepared {
    id: String,
    /// `None`: a new item; `Some(v)`: the stored version this one replaces.
    prev: Option<u64>,
    version: u64,
    ts: u64,
    item_blob: Vec<u8>,
    snapshot_blob: Vec<u8>,
    summary: ItemSummary,
}

/// What a batch's plaintext export of the vault holds.
pub struct ExportSet {
    pub records: Vec<ExportRecord>,
    /// Items that could not be decrypted (damaged), so they are not in `records`.
    pub unreadable: usize,
}

fn version_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StorageError::Storage("A stored number is out of range.".into()))
}

/// The text of an error that is safe to show next to an entry's title.
fn reason_of(e: &StorageError) -> String {
    match e {
        StorageError::InvalidInput(m) => m.clone(),
        _ => "it could not be saved.".to_string(),
    }
}

/// Merge `new` (from a file) into `old` (in the vault) for `append_version`.
///
/// Nothing the vault holds is blanked by a file that lacks it: a non-empty imported field value
/// replaces the stored one, an empty one keeps it; custom fields are merged by name (the file's
/// value wins); tags are added; the stored title and folder stay (the title only differs in case
/// by the duplicate rule); a favourite stays a favourite.
pub fn merge_drafts(old: &ItemDraft, new: &ItemDraft) -> ItemDraft {
    let mut out = old.clone();
    for (k, v) in &new.fields {
        if !v.trim().is_empty() {
            out.fields.insert(k.clone(), v.clone());
        }
    }
    for c in &new.custom_fields {
        match out
            .custom_fields
            .iter_mut()
            .find(|o| o.name.eq_ignore_ascii_case(&c.name))
        {
            Some(o) => {
                o.value = c.value.clone();
                o.hidden = c.hidden;
            }
            None => out.custom_fields.push(CustomField {
                name: c.name.clone(),
                value: c.value.clone(),
                hidden: c.hidden,
            }),
        }
    }
    for t in &new.tags {
        if !out.tags.contains(t) {
            out.tags.push(t.clone());
        }
    }
    if out.folder.is_none() {
        out.folder = new.folder.clone();
    }
    out.favorite = old.favorite || new.favorite;
    out
}

impl Vault {
    /// Existing duplicate candidates: the key of every non-trashed item (archived ones too) and
    /// the id of the most recently changed item with that key. Logins are decrypted to read their
    /// username and website; the other types are keyed by type and title alone.
    pub fn import_dup_index(&self) -> Result<HashMap<DupKey, String>> {
        let mut best: HashMap<DupKey, (u64, String)> = HashMap::new();
        for (id, entry) in &self.index {
            let IndexEntry::Active(s) = entry else {
                continue;
            };
            let key = if s.item_type == ItemType::Login {
                match self.load_latest(id) {
                    Ok((p, _)) => {
                        let get = |k: &str| p.fields.get(k).map_or("", String::as_str).to_owned();
                        let (username, url) =
                            (Zeroizing::new(get("username")), Zeroizing::new(get("url")));
                        dup_key_parts(s.item_type, &s.title, &username, &url)
                    }
                    // A login that cannot be read cannot be compared; it is not a duplicate.
                    Err(_) => continue,
                }
            } else {
                dup_key_parts(s.item_type, &s.title, "", "")
            };
            let newer = best
                .get(&key)
                .is_none_or(|(at, other)| (s.updated_at, id) > (*at, other));
            if newer {
                best.insert(key, (s.updated_at, id.clone()));
            }
        }
        Ok(best.into_iter().map(|(k, (_, id))| (k, id)).collect())
    }

    /// Save a whole import in one transaction (see the module notes). `items` are consumed in
    /// order; the outcome has one `ids` slot per item.
    pub fn import_batch(
        &mut self,
        items: impl Iterator<Item = BatchItem>,
        mode: ImportMode,
    ) -> Result<BatchOutcome> {
        let dups = if mode == ImportMode::ImportAll {
            HashMap::new()
        } else {
            self.import_dup_index()?
        };
        let now = now_millis();
        let mut out = BatchOutcome::default();
        let mut prepared: Vec<Prepared> = Vec::new();
        // The latest in-batch state of items that already got a new version in this batch, so a
        // second record for the same item builds on the first one's result.
        let mut working: HashMap<String, (ItemPayload, u64)> = HashMap::new();

        for item in items {
            let BatchItem {
                mut draft,
                created,
                warnings,
            } = item;
            for w in warnings {
                out.note(w);
            }
            let label = warn_title(&draft.title);
            if let Err(e) = draft.normalize() {
                out.fail(&label, &reason_of(&e));
                continue;
            }
            let existing = (mode != ImportMode::ImportAll)
                .then(|| dups.get(&dup_key(&draft)))
                .flatten();
            let step = match (mode, existing) {
                (ImportMode::SkipDuplicates, Some(_)) => Ok(None),
                (ImportMode::AppendVersion, Some(id)) => {
                    self.prepare_append(id, &draft, now, &mut working)
                }
                _ => self.prepare_new(&draft, created, now).map(Some),
            };
            match step {
                Ok(Some(p)) => {
                    if p.prev.is_some() {
                        out.appended += 1;
                    } else {
                        out.added += 1;
                    }
                    out.ids.push(Some(p.id.clone()));
                    prepared.push(p);
                }
                Ok(None) => {
                    out.skipped += 1;
                    out.ids.push(None);
                }
                Err(e) => out.fail(&label, &reason_of(&e)),
            }
        }
        out.finish();
        if prepared.is_empty() {
            return Ok(out);
        }

        self.store.in_tx(|conn| {
            let at = now_millis();
            for p in &prepared {
                let (version, ts) = (version_i64(p.version)?, version_i64(p.ts)?);
                match p.prev {
                    None => {
                        conn.execute(
                            "INSERT INTO items (id, version, ts, blob, dirty) VALUES (?1, ?2, ?3, ?4, 1)",
                            params![p.id, version, ts, p.item_blob],
                        )?;
                    }
                    Some(prev) => {
                        let changed = conn.execute(
                            "UPDATE items SET version = ?2, ts = ?3, blob = ?4, dirty = 1
                             WHERE id = ?1 AND version = ?5",
                            params![p.id, version, ts, p.item_blob, version_i64(prev)?],
                        )?;
                        if changed != 1 {
                            return Err(StorageError::VersionConflict);
                        }
                    }
                }
                conn.execute(
                    "INSERT INTO versions (item_id, version, ts, blob) VALUES (?1, ?2, ?3, ?4)",
                    params![p.id, version, ts, p.snapshot_blob],
                )?;
                // The replication work commits (or rolls back) with the items.
                dbops::enqueue_for_enabled(conn, JobKind::Version, &p.id, p.version, p.ts, at)?;
                dbops::enqueue_for_enabled(conn, JobKind::Item, &p.id, 0, 0, at)?;
            }
            Ok(())
        })?;

        for p in prepared {
            self.index.insert(p.id, IndexEntry::Active(p.summary));
        }
        self.wake_backup();
        Ok(out)
    }

    fn seal_pair(&self, payload: &ItemPayload, version: u64) -> Result<(Vec<u8>, Vec<u8>)> {
        let ts = payload.updated_at;
        let json = Zeroizing::new(serde_json::to_vec(payload)?);
        let item_blob = self
            .root
            .seal_object(KeyPurpose::Item, &payload.id, version, ts, &json)?;
        let snapshot_blob =
            self.root
                .seal_object(KeyPurpose::Version, &payload.id, version, ts, &json)?;
        Ok((item_blob, snapshot_blob))
    }

    /// Version 1 of a new item. `draft` is already normalized.
    fn prepare_new(&self, draft: &ItemDraft, created: Option<u64>, now: u64) -> Result<Prepared> {
        let id = uuid::new_v4()?;
        let mut payload = ItemPayload::new(id.clone(), draft, now);
        if let Some(c) = created.filter(|c| *c > 0 && *c <= now) {
            payload.created_at = c;
        }
        let (item_blob, snapshot_blob) = self.seal_pair(&payload, 1)?;
        Ok(Prepared {
            id,
            prev: None,
            version: 1,
            ts: payload.updated_at,
            item_blob,
            snapshot_blob,
            summary: payload.summary(1),
        })
    }

    /// A new version of the existing item `id` built from `draft`, or `None` when the merged
    /// content is the same as what is stored.
    fn prepare_append(
        &self,
        id: &str,
        draft: &ItemDraft,
        now: u64,
        working: &mut HashMap<String, (ItemPayload, u64)>,
    ) -> Result<Option<Prepared>> {
        let (old, version) = match working.get(id) {
            Some((p, v)) => (p.clone(), *v),
            None => self.load_latest(id)?,
        };
        let old_draft = old.to_draft();
        let mut merged = merge_drafts(&old_draft, draft);
        merged.normalize()?;
        let changed = changed_fields(&old_draft, &merged);
        if changed.is_empty() {
            return Ok(None);
        }
        let next_version = version
            .checked_add(1)
            .ok_or_else(|| StorageError::Storage("This item has too many versions.".into()))?;
        let mut next = old.clone();
        next.apply_draft(&merged);
        next.updated_at = now.max(old.updated_at);
        next.event = ItemEvent::Edited;
        next.restored_from = None;
        next.changed_fields = changed;
        let (item_blob, snapshot_blob) = self.seal_pair(&next, next_version)?;
        let prepared = Prepared {
            id: id.to_string(),
            prev: Some(version),
            version: next_version,
            ts: next.updated_at,
            item_blob,
            snapshot_blob,
            summary: next.summary(next_version),
        };
        working.insert(id.to_string(), (next, next_version));
        Ok(Some(prepared))
    }

    /// Every non-trashed item (archived ones too), decrypted, in a stable order. For the plaintext
    /// export; password-gated by the caller. An item that cannot be decrypted is counted, not
    /// exported.
    pub fn export_records(&self) -> Result<ExportSet> {
        let mut ids: Vec<&String> = self
            .index
            .iter()
            .filter(|(_, e)| matches!(e, IndexEntry::Active(_)))
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        let mut records = Vec::with_capacity(ids.len());
        let mut unreadable = self.damaged.len();
        for id in ids {
            match self.load_latest(id) {
                Ok((p, _)) => records.push(ExportRecord {
                    id: p.id.clone(),
                    draft: p.to_draft(),
                    created_at: p.created_at,
                    updated_at: p.updated_at,
                }),
                Err(_) => unreadable += 1,
            }
        }
        records.sort_by(|a, b| {
            a.draft
                .title
                .to_lowercase()
                .cmp(&b.draft.title.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(ExportSet {
            records,
            unreadable,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn login(title: &str, user: &str, url: &str, pw: &str) -> ItemDraft {
        let mut fields = BTreeMap::new();
        fields.insert("username".to_string(), user.to_string());
        fields.insert("url".to_string(), url.to_string());
        fields.insert("password".to_string(), pw.to_string());
        ItemDraft {
            item_type: ItemType::Login,
            title: title.into(),
            favorite: false,
            tags: vec![],
            folder: None,
            fields,
            custom_fields: vec![],
        }
    }

    #[test]
    fn merge_keeps_what_the_file_lacks_and_takes_what_it_has() {
        let mut old = login("GitHub", "ana", "https://github.com", "old");
        old.fields.insert("notes".into(), "kept".into());
        old.folder = Some("Work".into());
        old.tags = vec!["a".into()];
        old.favorite = true;
        old.custom_fields = vec![CustomField {
            name: "PIN".into(),
            value: "1".into(),
            hidden: true,
        }];
        let mut new = login("github", "ANA", "https://github.com", "new");
        new.fields.insert("notes".into(), String::new());
        new.folder = Some("Elsewhere".into());
        new.tags = vec!["a".into(), "b".into()];
        new.custom_fields = vec![
            CustomField {
                name: "pin".into(),
                value: "2".into(),
                hidden: true,
            },
            CustomField {
                name: "Extra".into(),
                value: "x".into(),
                hidden: false,
            },
        ];
        let m = merge_drafts(&old, &new);
        assert_eq!(m.title, "GitHub");
        assert_eq!(m.fields["password"], "new");
        assert_eq!(m.fields["notes"], "kept");
        assert_eq!(m.folder.as_deref(), Some("Work"));
        assert_eq!(m.tags, vec!["a", "b"]);
        assert!(m.favorite);
        assert_eq!(m.custom_fields.len(), 2);
        assert_eq!(m.custom_fields[0].value, "2");
        assert_eq!(m.custom_fields[1].name, "Extra");
    }

    #[test]
    fn merging_the_same_content_changes_nothing() {
        let d = login("Site", "me", "https://site.example", "pw");
        let m = merge_drafts(&d, &d);
        assert!(changed_fields(&d, &m).is_empty());
    }
}
