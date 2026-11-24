//! Pull and reconcile (`ai-docs/STORAGE_AND_SYNC.md` §6): bring what other devices put in the
//! backup into this vault. Runs on the replicator thread at unlock (never blocking the unlock) and
//! at the start of "Back up now"; the caller drains the queue afterwards.
//!
//! The **source** is the master target, or the first enabled one when there is no master. Every
//! other target is a replica: it is only ever written to, never read for content.
//!
//! One pass:
//!
//! 1. List `vault/tombstones/` and `vault/items/` (paged). Compare each object's ETag with the one
//!    stored in `remote_objects`; an unchanged ETag means the object is already known, so a
//!    second pass over an unchanged bucket makes **zero downloads**.
//! 2. Tombstones first: a proof of deletion wins over everything. It is authenticated, then the
//!    item is erased here exactly like a local permanent delete, except that the source is not
//!    told a deletion it already knows (the other targets are).
//! 3. For each changed latest state: download it and let the vault authenticate it (key id ==
//!    header id, tag verifies, payload parses). A state that does not authenticate is **skipped
//!    and counted**, never applied.
//! 4. Classify it against the local head, by version and by *lineage* (each state records the
//!    state it replaced, inside its sealed payload):
//!    * identical: nothing;
//!    * no such item here: take it over with its history;
//!    * rolled back (its version is lower than one already seen for this key): ignore it;
//!    * its lineage contains the local head (**fast-forward**): take it over, with the missing
//!      snapshots on the way;
//!    * the local head's lineage contains it (**local is ahead**): nothing (the push side will
//!      replace it);
//!    * neither (**diverged**): the local head stays, and the remote state is kept as one more
//!      immutable version flagged "from another device". Restoring it (the existing
//!      restore-from-version) makes the merged state a descendant of both, so the other device
//!      fast-forwards to it.
//! 5. Every change is written by the vault in one transaction that re-checks the local head
//!    (see [`super::pull_apply`]); afterwards the vault's index is refreshed and its
//!    `syncRevision` goes up.
//!
//! Bookkeeping (`remote_objects`): a remote state that **grows into this device's own** (same,
//! taken over, in this device's lineage, new, or a stale rollback) is remembered under its own
//! key, so the next push writes over it with `If-Match`. A state that was kept apart (diverged,
//! or unreadable here) is remembered under `<key>#merged` instead: the next pass does not
//! download it again, but a push is *not* allowed to replace it blindly (it goes through the
//! conflict check, which leaves a merged state alone).
//!
//! Nothing here sleeps, retries or polls. An unreachable source is recorded in that target's
//! health by the caller; the push side carries on regardless.

use super::pull_apply::{Applied, Head, Plan, Snap, Verdict};
use super::remote::{keys, ListEntry, ObjectStore, RemoteError, MAX_META_BYTES, MAX_OBJECT_BYTES};
use super::replicator::RuntimeTarget;
use crate::crypto::envelope;
use crate::storage::db::Store;
use crate::storage::vault::Vault;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

/// A bucket with more objects than this is not a Hexis vault (the same bound as a restore).
pub const MAX_OBJECTS: usize = 100_000;
/// How many steps back a lineage is followed before it counts as "not provable".
const MAX_WALK: usize = 5_000;
/// How many snapshots of one new item are downloaded.
const MAX_ITEM_SNAPSHOTS: usize = 2_000;

/// How a pull reaches the vault: one short visit at a time, under the session lock.
///
/// The replicator thread never holds a key. Whenever it needs one it hands a closure to the
/// link, which runs it on the vault of the unlock this link belongs to. `false` means that
/// vault is gone (locked, or locked and unlocked again): the pass stops quietly.
pub trait VaultLink: Send + Sync {
    fn with_vault(&self, f: &mut dyn FnMut(&mut Vault)) -> bool;
}

fn visit<T>(link: &dyn VaultLink, f: impl FnOnce(&mut Vault) -> T) -> Option<T> {
    let mut f = Some(f);
    let mut out = None;
    let ran = link.with_vault(&mut |v| {
        if let Some(f) = f.take() {
            out = Some(f(v));
        }
    });
    if ran {
        out
    } else {
        None
    }
}

/// What one pass did (the tests read it; the app only needs the vault's own counters).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PullReport {
    pub new_items: usize,
    pub fast_forwarded: usize,
    pub local_ahead: usize,
    pub other_device: usize,
    pub deleted: usize,
    /// Objects (and snapshots that came with an item) that failed authentication.
    pub damaged: usize,
    pub rolled_back: usize,
    /// Items the person edited while the pull was deciding; left for the next pass.
    pub raced: usize,
}

impl PullReport {
    pub fn changed(&self) -> bool {
        self.new_items + self.fast_forwarded + self.other_device + self.deleted > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullError {
    /// The target could not be read.
    Remote(RemoteError),
    /// The vault database could not be used.
    Local,
    /// The vault was locked (or the engine stopped) while the pass ran.
    Stopped,
}

impl From<RemoteError> for PullError {
    fn from(e: RemoteError) -> Self {
        Self::Remote(e)
    }
}

type PullResult<T> = std::result::Result<T, PullError>;

/// An ETag as an opaque string, without the quotes or the weak marker some services add.
fn etag_core(e: &str) -> &str {
    e.strip_prefix("W/").unwrap_or(e).trim_matches('"')
}

type Known = Option<(Option<String>, u64)>;

fn unchanged(known: &Known, listed: Option<&str>) -> bool {
    match (known.as_ref().and_then(|(e, _)| e.as_deref()), listed) {
        (Some(a), Some(b)) => etag_core(a) == etag_core(b),
        _ => false,
    }
}

/// The bookkeeping key of a state that was merged without becoming this device's own.
fn merged_key(key: &str) -> String {
    format!("{key}#merged")
}

fn list_all(remote: &dyn ObjectStore, prefix: &str) -> PullResult<Vec<ListEntry>> {
    let mut out = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let page = remote.list(prefix, token.as_deref(), 1000)?;
        out.extend(page.entries);
        if out.len() > MAX_OBJECTS {
            return Err(PullError::Remote(RemoteError::Malformed));
        }
        match page.next_token {
            Some(next) => {
                if token.as_deref() == Some(next.as_str()) {
                    return Err(PullError::Remote(RemoteError::Malformed));
                }
                token = Some(next);
            }
            None => return Ok(out),
        }
    }
}

enum Walk {
    Found,
    NotFound,
    /// A step is missing or has no recorded parent: lineage cannot be proven.
    Unknown,
}

/// Follow the parent pointers from `start` looking for the state `goal = (version, ts)`.
/// Versions only go down along a chain, so the search stops as soon as it has gone past `goal`.
/// With `fetch`, a step this device does not hold is downloaded (and kept in `fetched`).
fn walk(
    link: &dyn VaultLink,
    remote: &dyn ObjectStore,
    id: &str,
    start: Head,
    goal: (u64, u64),
    fetch: bool,
    fetched: &mut Vec<Snap>,
) -> PullResult<Walk> {
    let (mut version, mut parent) = (start.version, start.parent);
    for _ in 0..MAX_WALK {
        let Some(p) = parent else {
            // The chain ends. Only a first version is a proper beginning; anything else was
            // written before parents were recorded, so nothing can be proven about it.
            return Ok(if version == 1 {
                Walk::NotFound
            } else {
                Walk::Unknown
            });
        };
        if (p.version, p.ts) == goal {
            return Ok(Walk::Found);
        }
        if p.version <= goal.0 {
            return Ok(Walk::NotFound);
        }
        let mut node =
            visit(link, |v| v.pull_node_local(id, p.version, p.ts)).ok_or(PullError::Stopped)?;
        if node.is_none() && fetch {
            if let Some(obj) = remote.get(&keys::version(id, p.version, p.ts), MAX_OBJECT_BYTES)? {
                node = visit(link, |v| {
                    v.pull_node_remote(id, p.version, p.ts, &obj.bytes)
                })
                .ok_or(PullError::Stopped)?;
                if node.is_some() {
                    fetched.push(Snap {
                        version: p.version,
                        ts: p.ts,
                        bytes: obj.bytes,
                    });
                }
            }
        }
        let Some(n) = node else {
            return Ok(Walk::Unknown);
        };
        version = n.version;
        parent = n.parent;
    }
    Ok(Walk::Unknown)
}

/// The snapshots of a new item (its history), as far as the backup has them.
fn download_history(remote: &dyn ObjectStore, id: &str) -> PullResult<Vec<Snap>> {
    let mut out = Vec::new();
    for entry in list_all(remote, &keys::versions_of(id))? {
        let Some((entry_id, version, ts)) = keys::parse_version(&entry.key) else {
            continue;
        };
        if entry_id != id {
            continue;
        }
        if out.len() >= MAX_ITEM_SNAPSHOTS {
            break;
        }
        if let Some(obj) = remote.get(&entry.key, MAX_OBJECT_BYTES)? {
            out.push(Snap {
                version,
                ts,
                bytes: obj.bytes,
            });
        }
    }
    Ok(out)
}

struct Pass<'a> {
    db: &'a Store,
    remote: &'a dyn ObjectStore,
    source: &'a str,
    link: &'a dyn VaultLink,
    stop: &'a AtomicBool,
    report: PullReport,
}

impl Pass<'_> {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn known(&self, key: &str) -> Known {
        self.db.remote_object(self.source, key).ok().flatten()
    }

    /// The highest version this device has seen for a key (either kind of bookkeeping).
    fn top_seen(&self, key: &str) -> u64 {
        let of = |k: &str| self.known(k).map_or(0, |(_, v)| v);
        of(key).max(of(&merged_key(key)))
    }

    /// Already dealt with: its ETag is the one remembered for the key or for its merged copy.
    fn is_unchanged(&self, key: &str, listed: Option<&str>) -> bool {
        unchanged(&self.known(key), listed) || unchanged(&self.known(&merged_key(key)), listed)
    }

    /// The state in the bucket now belongs to this device's lineage: remember its ETag (the next
    /// push replaces it with `If-Match`). The version never goes down: that is what the rollback
    /// guard compares against.
    fn record_main(&self, key: &str, etag: Option<&str>, version: u64) {
        let top = self.top_seen(key).max(version);
        let _ = self.db.set_remote_object(self.source, key, etag, top);
    }

    /// The state in the bucket was kept apart (see the module docs).
    fn record_merged(&self, key: &str, etag: Option<&str>, version: u64) {
        let top = self.top_seen(key).max(version);
        let _ = self
            .db
            .set_remote_object(self.source, &merged_key(key), etag, top);
    }

    fn apply(&mut self, plan: Plan) -> PullResult<Applied> {
        let source = self.source.to_string();
        let result = visit(self.link, |v| v.pull_apply(&source, plan))
            .ok_or(PullError::Stopped)?
            .map_err(|_| PullError::Local)?;
        self.report.damaged += result.skipped_snapshots;
        Ok(result.applied)
    }

    fn tombstones(&mut self) -> PullResult<HashSet<String>> {
        let mut dead = HashSet::new();
        for entry in list_all(self.remote, keys::TOMBSTONES_PREFIX)? {
            if self.stopped() {
                return Err(PullError::Stopped);
            }
            let Some(id) = keys::parse_tombstone(&entry.key) else {
                continue;
            };
            if self.is_unchanged(&entry.key, entry.etag.as_deref()) {
                dead.insert(id.to_string());
                continue;
            }
            let Some(obj) = self.remote.get(&entry.key, MAX_META_BYTES)? else {
                continue;
            };
            let etag = obj.etag.or(entry.etag);
            match self.apply(Plan::Tombstone {
                id: id.to_string(),
                blob: obj.bytes,
            })? {
                // Only a proof that authenticates counts: a forged one must not hide the item.
                Applied::Damaged => self.report.damaged += 1,
                Applied::Changed => {
                    dead.insert(id.to_string());
                    self.report.deleted += 1;
                    self.record_main(&entry.key, etag.as_deref(), 0);
                }
                _ => {
                    dead.insert(id.to_string());
                    self.record_main(&entry.key, etag.as_deref(), 0);
                }
            }
        }
        Ok(dead)
    }

    fn items(&mut self, dead: &HashSet<String>) -> PullResult<()> {
        for entry in list_all(self.remote, keys::ITEMS_PREFIX)? {
            if self.stopped() {
                return Err(PullError::Stopped);
            }
            let Some(id) = keys::parse_item(&entry.key) else {
                continue;
            };
            if dead.contains(id) {
                continue;
            }
            if self.is_unchanged(&entry.key, entry.etag.as_deref()) {
                continue;
            }
            let Some(obj) = self.remote.get(&entry.key, MAX_OBJECT_BYTES)? else {
                continue;
            };
            self.item(id, &entry.key, obj.bytes, obj.etag.or(entry.etag))?;
        }
        Ok(())
    }

    fn item(
        &mut self,
        id: &str,
        key: &str,
        bytes: Vec<u8>,
        etag: Option<String>,
    ) -> PullResult<()> {
        let etag = etag.as_deref();
        let verdict =
            visit(self.link, |v| v.pull_classify(id, &bytes)).ok_or(PullError::Stopped)?;
        // The header is authenticated by now unless the verdict says damaged.
        let remote_version = envelope::peek_header(&bytes).map_or(0, |h| h.version);
        match verdict {
            Verdict::Damaged => self.report.damaged += 1,
            Verdict::Skip => self.record_merged(key, etag, 0),
            Verdict::Same => self.record_main(key, etag, remote_version),
            Verdict::New(_) => {
                let snapshots = download_history(self.remote, id)?;
                let plan = Plan::New {
                    id: id.into(),
                    head: bytes,
                    snapshots,
                };
                match self.apply(plan)? {
                    Applied::Changed => {
                        self.report.new_items += 1;
                        self.record_main(key, etag, remote_version);
                    }
                    Applied::Unchanged => self.record_main(key, etag, remote_version),
                    Applied::Raced => self.report.raced += 1,
                    Applied::Damaged => self.report.damaged += 1,
                }
            }
            Verdict::Differs { remote, local } => {
                self.differs(id, key, bytes, etag, remote, local)?;
            }
        }
        Ok(())
    }

    fn differs(
        &mut self,
        id: &str,
        key: &str,
        bytes: Vec<u8>,
        etag: Option<&str>,
        remote: Head,
        local: Head,
    ) -> PullResult<()> {
        if remote.version < self.top_seen(key) {
            // A state this device has already seen something newer than: a stale copy of the
            // bucket, or a replay. Not merged, not flagged; remembered as replaceable, so the next
            // push puts this device's state over it and it is not downloaded again.
            self.report.rolled_back += 1;
            self.record_main(key, etag, remote.version);
            return Ok(());
        }
        let local_ref = (local.version, local.ts);
        let remote_ref = (remote.version, remote.ts);
        let mut fetched = Vec::new();
        let fast_forward = if remote.version > local.version {
            matches!(
                walk(
                    self.link,
                    self.remote,
                    id,
                    remote,
                    local_ref,
                    true,
                    &mut fetched
                )?,
                Walk::Found
            )
        } else {
            false
        };
        if fast_forward {
            // The head's own snapshot, if the backup has it (otherwise the vault makes it).
            if let Some(obj) = self.remote.get(
                &keys::version(id, remote.version, remote.ts),
                MAX_OBJECT_BYTES,
            )? {
                fetched.push(Snap {
                    version: remote.version,
                    ts: remote.ts,
                    bytes: obj.bytes,
                });
            }
            let plan = Plan::FastForward {
                id: id.into(),
                expect: local_ref,
                head: bytes,
                snapshots: fetched,
            };
            match self.apply(plan)? {
                Applied::Changed => {
                    self.report.fast_forwarded += 1;
                    self.record_main(key, etag, remote.version);
                }
                Applied::Unchanged => self.record_main(key, etag, remote.version),
                Applied::Raced => self.report.raced += 1,
                Applied::Damaged => self.report.damaged += 1,
            }
            return Ok(());
        }
        let local_ahead = remote.version < local.version
            && matches!(
                walk(
                    self.link,
                    self.remote,
                    id,
                    local,
                    remote_ref,
                    false,
                    &mut Vec::new()
                )?,
                Walk::Found
            );
        if local_ahead {
            self.report.local_ahead += 1;
            self.record_main(key, etag, remote.version);
            return Ok(());
        }
        // Both sides changed it (or the lineage cannot be proven, which is treated the same way,
        // because nothing is ever lost by keeping both).
        let plan = Plan::OtherDevice {
            id: id.into(),
            expect: local_ref,
            head: bytes,
        };
        match self.apply(plan)? {
            Applied::Changed => {
                self.report.other_device += 1;
                self.record_merged(key, etag, remote.version);
            }
            Applied::Unchanged => self.record_merged(key, etag, remote.version),
            Applied::Raced => self.report.raced += 1,
            Applied::Damaged => self.report.damaged += 1,
        }
        Ok(())
    }
}

/// One pass over the source target. `Err(Remote)` means the source could not be read; the caller
/// records that in the target's health.
pub fn reconcile(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &RuntimeTarget,
    link: &dyn VaultLink,
    stop: &AtomicBool,
) -> PullResult<PullReport> {
    let mut pass = Pass {
        db,
        remote,
        source: &t.id,
        link,
        stop,
        report: PullReport::default(),
    };
    let result = pass.tombstones().and_then(|dead| pass.items(&dead));
    let damaged = pass.report.damaged;
    // Whatever happened, the vault learns how many objects were refused (and the page, through
    // the status, that something changed).
    let _ = visit(link, |v| v.set_remote_damaged(damaged));
    result?;
    Ok(pass.report)
}
