//! Single-writer replication of the already-encrypted vault objects to the backup targets
//! (`ai-docs/STORAGE_AND_SYNC.md` §6/§7/§10), driven by **four triggers only** (`FEATURE_QUEUE.md`
//! Q0, decision D18, extended by D42): unlock, a committed local change, app exit or backgrounding, and the device
//! coming back online ([`Replicator::request_reconcile`] again, asked for by the page through `sync_reconnect`).
//!
//! * Every storage change queues work for every enabled target **in the same transaction** as
//!   the change (`storage/db.rs`). This module drains those queues.
//! * One background thread per unlocked vault ([`Replicator`]) **sleeps on a condition variable**
//!   until a trigger wakes it: unlock ([`Replicator::request_reconcile`]), a committed change
//!   ([`Replicator::wake`]) or a change of the target list. It then drains and goes back to sleep.
//!   There is **no timer, no polling and no retry schedule** anywhere: a job that fails stays in
//!   the queue with its error and attempt count and is tried again at the next trigger. The thread
//!   is stopped when the vault locks, the profile changes or the core is dropped.
//! * The two other ways in are the person pulling a trigger by hand: [`Replicator::run_now`]
//!   ("Back up now": reconcile, then drain, from the calling thread) and
//!   [`Replicator::flush`] (app exit or backgrounding: drain within a deadline).
//! * Order: the **master** target first, then the others by creation order; inside one target
//!   the key bundle, tombstones, version snapshots, latest states, purges, shared-targets.
//! * **Single writer:** an object whose remote copy this device has not seen (newer, or a
//!   different copy of the same version, or an older one that is not part of this device's
//!   history) is **never overwritten**. The job is *blocked* (kept in the queue, shown as the
//!   target's `lastError`) until the next reconcile has merged it. A bucket that already holds a
//!   *different vault* (other public keys) stops the whole target at the key bundle.
//! * Failures leave the job queued; wrong keys, a missing bucket and other errors that retrying
//!   cannot fix show `failing` at once. Nothing here ever logs or stores a secret, a URL or a
//!   bucket name: messages come from [`RemoteError::message`].

use super::dbops::{Job, JobKind, OutboxStats, TargetRow};
use super::pull::{self, PullError, VaultLink};
use super::remote::{
    keys, Condition, ObjectStore, RemoteError, StoreFactory, MAX_META_BYTES, MAX_OBJECT_BYTES,
};
use super::types::{ConditionalWrites, HealthState, TargetConfig, TargetHealth};
use crate::crypto::{envelope, now_millis, VaultKeyBundle};
use crate::storage::db::Store;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Wording of the "not overwritten" outcomes. Shown to the person as the target's last error.
pub const MSG_NEWER_REMOTE: &str = "This backup already holds a newer version of an item than this device has, so it was not overwritten. Press Back up now to merge it.";
pub const MSG_DIVERGED: &str =
    "This backup holds a different copy of an item at the same version, so it was not overwritten. Press Back up now to merge it.";
pub const MSG_UNSEEN_REMOTE: &str = "This backup holds a change from another device that this device has not merged yet, so it was not overwritten. Press Back up now to merge it.";
pub const MSG_FOREIGN_OBJECT: &str =
    "An object in this backup is not an object of this app, so it was not overwritten.";
pub const MSG_OTHER_VAULT: &str = "This location already holds a different vault. Choose an empty folder (prefix) or another bucket.";
const MSG_LOCAL: &str = "The vault database could not be read for the backup.";

/// What health needs from the clock, and whether a worker thread exists. There is nothing else to
/// tune: the engine has no timers (see the module docs).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// A queue whose oldest job is older than this is `lagging` (read at query time).
    pub lagging_after: Duration,
    /// This many consecutive errors make a target `failing`.
    pub failing_after: u32,
    /// `false`: no worker thread; queues are drained only by `backup_run` and the flush (tests,
    /// scripted runs).
    pub background: bool,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            lagging_after: Duration::from_secs(5 * 60),
            failing_after: 5,
            background: true,
        }
    }
}

/// What the engine needs from the outside world: how to open a bucket, and the timing.
pub struct BackupRuntime {
    pub factory: Arc<dyn StoreFactory>,
    pub timing: Timing,
}

impl BackupRuntime {
    pub fn new(factory: Arc<dyn StoreFactory>, timing: Timing) -> Arc<Self> {
        Arc::new(Self { factory, timing })
    }

    /// Real S3 clients, production timing.
    pub fn production() -> Arc<Self> {
        Self::new(Arc::new(super::remote::S3Factory), Timing::default())
    }
}

/// A target as the worker sees it: decrypted settings plus what the probe learned.
#[derive(Clone)]
pub struct RuntimeTarget {
    pub id: String,
    pub position: i64,
    pub master: bool,
    pub conditional: ConditionalWrites,
    pub config: TargetConfig,
}

// ------------------------------------------------------------------ health

/// The five states of `IPC_CONTRACT.md` §6 from what is stored (pure, so every transition is
/// unit-tested).
pub fn compute_health(
    row: &TargetRow,
    stats: &OutboxStats,
    now: u64,
    timing: &Timing,
) -> TargetHealth {
    let lagging_ms = u64::try_from(timing.lagging_after.as_millis()).unwrap_or(u64::MAX);
    let failing =
        row.hard_error || row.consecutive_errors >= timing.failing_after || stats.blocked > 0;
    let state = if !row.enabled {
        HealthState::Disabled
    } else if failing {
        HealthState::Failing
    } else if row.consecutive_errors > 0
        || stats
            .oldest_enqueued_at
            .is_some_and(|t| now.saturating_sub(t) > lagging_ms)
    {
        HealthState::Lagging
    } else if row.last_success_at.is_none() && row.last_checked_at.is_none() {
        HealthState::Untested
    } else {
        HealthState::Healthy
    };
    let transport_error = row.hard_error || row.consecutive_errors > 0;
    let last_error = match state {
        HealthState::Disabled | HealthState::Healthy | HealthState::Untested => None,
        _ if transport_error => row.last_error.clone(),
        _ => stats.first_blocked_error.clone(),
    };
    TargetHealth {
        state,
        queued: stats.queued,
        last_success_at: row.last_success_at,
        last_checked_at: row.last_checked_at,
        last_error,
        conditional_writes: row.conditional,
    }
}

// ------------------------------------------------------------------ the engine

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to do (or nothing was contacted).
    Idle,
    /// The target answered and its jobs were handled.
    Contacted,
    Failed,
}

pub(super) enum JobResult {
    Done,
    /// The job no longer applies (its item was erased); nothing was sent.
    Obsolete,
    /// A conflict that waits for a merge: the job stays queued, other jobs go on.
    Blocked(String),
    /// A network or service error: stop this round; the job stays queued for the next trigger.
    Transient(RemoteError),
    /// Stop this round and show the message (another vault in the bucket, a broken database).
    Fatal(String),
}

pub(super) fn blocked(msg: &str) -> JobResult {
    JobResult::Blocked(msg.to_string())
}

/// Should a drain stop now? (the engine was stopped, or the flush deadline passed)
fn must_stop(stop: &AtomicBool, deadline: Option<Instant>) -> bool {
    stop.load(Ordering::SeqCst) || deadline.is_some_and(|d| Instant::now() >= d)
}

/// The database is the truth about what the service enforces; a target that has never been
/// reached (restored, imported) is probed first so that writes use the right strategy. `Err` is
/// the probe's message and whether it is a hard failure.
fn ensure_probed(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &mut RuntimeTarget,
    row: &TargetRow,
) -> std::result::Result<(), (String, bool)> {
    t.conditional = row.conditional;
    if row.conditional == ConditionalWrites::Unknown && row.last_success_at.is_none() {
        let report = super::probe::run(remote);
        if !report.ok {
            return Err((report.message, report.hard));
        }
        let _ = db.target_set_conditional(&t.id, report.conditional);
        t.conditional = report.conditional;
    }
    Ok(())
}

/// Drain the queue of one target until it is empty, a transient error stops it, `stop` is set or
/// the `deadline` passes. Every outcome is recorded in the target's health columns.
/// `already_counted`: this trigger's pull has just recorded a failed attempt for this target, so
/// a failure here updates the message but is not counted a second time.
pub fn run_drain(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &RuntimeTarget,
    force: bool,
    already_counted: bool,
    stop: &AtomicBool,
    deadline: Option<Instant>,
) -> Outcome {
    let fail = |msg: &str, hard: bool| {
        let _ = if already_counted {
            db.target_note_failure(&t.id, msg, hard, now_millis())
        } else {
            db.target_record_failure(&t.id, msg, hard, now_millis())
        };
        Outcome::Failed
    };
    let row = match db.target_row(&t.id) {
        Ok(Some(r)) if r.enabled => r,
        Ok(_) => return Outcome::Idle,
        Err(_) => return Outcome::Failed,
    };
    let mut t = t.clone();
    if let Err((message, hard)) = ensure_probed(db, remote, &mut t, &row) {
        return fail(&message, hard);
    }
    let t = &t;
    let mut cursor = (-1i64, 0i64);
    let mut contacted = false;
    loop {
        if must_stop(stop, deadline) {
            return Outcome::Idle;
        }
        let batch = match db.outbox_batch(&t.id, cursor, 64) {
            Ok(b) => b,
            Err(_) => return fail(MSG_LOCAL, true),
        };
        if batch.is_empty() {
            break;
        }
        for job in batch {
            if must_stop(stop, deadline) {
                return Outcome::Idle;
            }
            cursor = (job.rank, job.seq);
            match execute(db, remote, t, &job) {
                JobResult::Done => {
                    contacted = true;
                    let _ = db.outbox_complete(job.seq, job.gen);
                }
                JobResult::Obsolete => {
                    let _ = db.outbox_complete(job.seq, job.gen);
                }
                JobResult::Blocked(msg) => {
                    contacted = true;
                    let _ = db.outbox_defer(job.seq, &msg, true);
                }
                JobResult::Transient(e) => {
                    let _ = db.outbox_defer(job.seq, &e.message(), false);
                    return fail(&e.message(), e.is_hard());
                }
                JobResult::Fatal(msg) => {
                    let _ = db.outbox_defer(job.seq, &msg, false);
                    return fail(&msg, true);
                }
            }
        }
    }
    if !contacted {
        // Nothing was queued. A forced run ("Back up now"), or a target that has been failing,
        // still checks that the bucket answers, so a recovered target turns healthy without
        // waiting for new work. (Only ever on a trigger; nothing here repeats by itself.)
        if !(force || row.consecutive_errors > 0 || row.hard_error) {
            return Outcome::Idle;
        }
        if let Err(e) = remote.list("vault/", None, 1) {
            return fail(&e.message(), e.is_hard());
        }
    }
    let _ = db.target_record_success(&t.id, now_millis());
    Outcome::Contacted
}

fn execute(db: &Store, remote: &dyn ObjectStore, t: &RuntimeTarget, job: &Job) -> JobResult {
    let local = |_| JobResult::Fatal(MSG_LOCAL.to_string());
    match job.kind {
        JobKind::Bundle => match db.bundle_json() {
            Ok(Some(json)) => push_bundle(db, remote, t, &json),
            Ok(None) => JobResult::Obsolete,
            Err(e) => local(e),
        },
        JobKind::Item => match db.item_row(&job.target_ref) {
            Ok(Some(row)) => push_envelope(
                db,
                remote,
                t,
                &keys::item(&job.target_ref),
                &row.blob,
                row.version,
            ),
            Ok(None) => JobResult::Obsolete,
            Err(e) => local(e),
        },
        JobKind::Version => match db.version_blob(&job.target_ref, job.version, job.ts) {
            Ok(Some(blob)) => push_immutable(
                db,
                remote,
                t,
                &keys::version(&job.target_ref, job.version, job.ts),
                &blob,
            ),
            Ok(None) => JobResult::Obsolete,
            Err(e) => local(e),
        },
        JobKind::Tombstone => match db.tombstone_blob(&job.target_ref) {
            Ok(Some(blob)) => {
                push_immutable(db, remote, t, &keys::tombstone(&job.target_ref), &blob)
            }
            Ok(None) => JobResult::Obsolete,
            Err(e) => local(e),
        },
        JobKind::Purge => purge_remote(db, remote, t, &job.target_ref),
        // The kind `targets` carries the vault's sealed meta objects: the shared-targets snapshot
        // (empty ref, master only) and the preferences (ref `prefs`, every enabled target).
        JobKind::Targets => {
            let prefs = job.target_ref == super::prefs::PREFS_REF;
            let (meta, key) = if prefs {
                (super::PREFS_META, keys::PREFS)
            } else {
                (super::SHARED_TARGETS_META, keys::TARGETS)
            };
            match db.meta(meta) {
                Ok(Some(b64)) => match B64.decode(b64) {
                    Ok(blob) => {
                        let version = envelope::peek_header(&blob).map_or(1, |h| h.version);
                        let result = push_envelope(db, remote, t, key, &blob, version);
                        // A preference is not worth an alarm: when the bucket holds a newer or
                        // different copy, the pull adopts the newer one; nothing to merge by hand.
                        match result {
                            JobResult::Blocked(_) if prefs => JobResult::Obsolete,
                            other => other,
                        }
                    }
                    Err(_) => JobResult::Obsolete,
                },
                Ok(None) => JobResult::Obsolete,
                Err(e) => local(e),
            }
        }
        JobKind::FileBlob | JobKind::FileMeta | JobKind::FilePurge => {
            super::files_sync::execute(db, remote, t, job)
        }
    }
}

/// How the copy in the bucket relates to the one about to be written. `seen` says whether this
/// device already knows the bucket's state (it is in the device's own history, or was merged as a
/// copy "from another device"): a state that is known is not news, so it is either safe to
/// replace (older) or is left alone without raising an alarm (newer, or a different copy of the
/// same version, which the person already has in the history to look at).
enum Compare {
    Same,
    RemoteOlder { seen: bool },
    RemoteNewer { seen: bool },
    Diverged { seen: bool },
    Foreign,
}

fn compare(
    db: &Store,
    key: &str,
    remote_bytes: &[u8],
    local_bytes: &[u8],
    local_version: u64,
) -> Compare {
    if remote_bytes == local_bytes {
        return Compare::Same;
    }
    let Ok(h) = envelope::peek_header(remote_bytes) else {
        return Compare::Foreign;
    };
    let item = keys::parse_item(key);
    let seen = item.is_some_and(|id| {
        db.snapshot_known(id, h.version, h.timestamp)
            .unwrap_or(false)
    });
    match h.version.cmp(&local_version) {
        std::cmp::Ordering::Greater => Compare::RemoteNewer { seen },
        std::cmp::Ordering::Equal => Compare::Diverged { seen },
        // Objects that are not an item's latest state (the shared-targets snapshot) have no
        // lineage: an older copy is simply replaced.
        std::cmp::Ordering::Less => Compare::RemoteOlder {
            seen: seen || item.is_none(),
        },
    }
}

/// What a write does about the copy that is already in the bucket (`Err` = stop, with the job's
/// result).
fn decide(c: Compare) -> std::result::Result<(), JobResult> {
    match c {
        Compare::Same => Err(JobResult::Done), // the caller records the ETag
        Compare::RemoteOlder { seen: true } => Ok(()),
        Compare::RemoteOlder { seen: false } => Err(blocked(MSG_UNSEEN_REMOTE)),
        // Already merged into the history: leave the bucket as it is, the next edit supersedes it.
        Compare::RemoteNewer { seen: true } | Compare::Diverged { seen: true } => {
            Err(JobResult::Done)
        }
        Compare::RemoteNewer { seen: false } => Err(blocked(MSG_NEWER_REMOTE)),
        Compare::Diverged { seen: false } => Err(blocked(MSG_DIVERGED)),
        Compare::Foreign => Err(blocked(MSG_FOREIGN_OBJECT)),
    }
}

fn remember(db: &Store, t: &RuntimeTarget, key: &str, etag: Option<&str>, version: u64) {
    let _ = db.set_remote_object(&t.id, key, etag, version);
}

/// A mutable object (latest item state, shared-targets snapshot) under the single-writer rule.
pub(super) fn push_envelope(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &RuntimeTarget,
    key: &str,
    bytes: &[u8],
    version: u64,
) -> JobResult {
    let strong = t.conditional == ConditionalWrites::Strong;
    let known = db.remote_object(&t.id, key).ok().flatten();
    let first = if strong {
        match known {
            Some((Some(etag), _)) => Condition::IfMatch(etag),
            _ => Condition::IfNoneMatch,
        }
    } else {
        // No reliable preconditions: look before writing (best effort, not atomic).
        match remote.get(key, MAX_OBJECT_BYTES) {
            Err(e) => return JobResult::Transient(e),
            Ok(None) => {}
            Ok(Some(obj)) => match compare(db, key, &obj.bytes, bytes, version) {
                Compare::Same => {
                    remember(db, t, key, obj.etag.as_deref(), version);
                    return JobResult::Done;
                }
                other => {
                    if let Err(result) = decide(other) {
                        return result;
                    }
                }
            },
        }
        Condition::None
    };
    match remote.put(key, bytes, &first) {
        Ok(ok) => {
            remember(db, t, key, ok.etag.as_deref(), version);
            JobResult::Done
        }
        Err(RemoteError::PreconditionFailed | RemoteError::NotFound) if strong => {
            resolve_conflict(db, remote, t, key, bytes, version)
        }
        Err(e) => JobResult::Transient(e),
    }
}

/// The write's precondition failed (or the object vanished): look at what is there now.
fn resolve_conflict(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &RuntimeTarget,
    key: &str,
    bytes: &[u8],
    version: u64,
) -> JobResult {
    let current = match remote.get(key, MAX_OBJECT_BYTES) {
        Ok(c) => c,
        Err(e) => return JobResult::Transient(e),
    };
    let (cond, expect_etag) = match &current {
        // It vanished between the failed write and this read: create it.
        None => (Condition::IfNoneMatch, None),
        Some(obj) => match compare(db, key, &obj.bytes, bytes, version) {
            Compare::Same => {
                remember(db, t, key, obj.etag.as_deref(), version);
                return JobResult::Done;
            }
            other => match decide(other) {
                Err(result) => return result,
                // Older and known: ours is the newer state; overwrite it, guarded by what we
                // just read.
                Ok(()) => (
                    obj.etag.clone().map_or(Condition::None, Condition::IfMatch),
                    obj.etag.clone(),
                ),
            },
        },
    };
    let _ = expect_etag;
    match remote.put(key, bytes, &cond) {
        Ok(ok) => {
            remember(db, t, key, ok.etag.as_deref(), version);
            JobResult::Done
        }
        // Somebody wrote in between: do not guess, try again later from a fresh read.
        Err(e) => JobResult::Transient(e),
    }
}

/// A write-once object (version snapshot, tombstone): 412 means it is already there, which is
/// success. Targets without reliable preconditions look first.
fn push_immutable(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &RuntimeTarget,
    key: &str,
    bytes: &[u8],
) -> JobResult {
    let _ = db;
    if t.conditional == ConditionalWrites::Strong {
        return match remote.put(key, bytes, &Condition::IfNoneMatch) {
            Ok(_) | Err(RemoteError::PreconditionFailed) => JobResult::Done,
            Err(e) => JobResult::Transient(e),
        };
    }
    match remote.head(key) {
        Ok(Some(_)) => JobResult::Done,
        Ok(None) => match remote.put(key, bytes, &Condition::None) {
            Ok(_) => JobResult::Done,
            Err(e) => JobResult::Transient(e),
        },
        Err(e) => JobResult::Transient(e),
    }
}

/// The key bundle. A bucket that holds a bundle with *other public keys* is another vault: stop.
fn push_bundle(
    db: &Store,
    remote: &dyn ObjectStore,
    t: &RuntimeTarget,
    local_json: &str,
) -> JobResult {
    let strong = t.conditional == ConditionalWrites::Strong;
    let current = match remote.get(keys::BUNDLE, MAX_META_BYTES) {
        Ok(c) => c,
        Err(e) => return JobResult::Transient(e),
    };
    let cond = match &current {
        None => {
            if strong {
                Condition::IfNoneMatch
            } else {
                Condition::None
            }
        }
        Some(obj) => {
            if obj.bytes == local_json.as_bytes() {
                remember(db, t, keys::BUNDLE, obj.etag.as_deref(), 0);
                return JobResult::Done;
            }
            let same_vault = match (
                std::str::from_utf8(&obj.bytes)
                    .ok()
                    .and_then(|s| VaultKeyBundle::from_json(s).ok()),
                VaultKeyBundle::from_json(local_json).ok(),
            ) {
                (Some(remote_bundle), Some(local_bundle)) => {
                    remote_bundle.public_keys == local_bundle.public_keys
                }
                _ => false,
            };
            if !same_vault {
                return JobResult::Fatal(MSG_OTHER_VAULT.to_string());
            }
            // Same vault, new password: replace, guarded by the ETag just read.
            match (&obj.etag, strong) {
                (Some(e), true) => Condition::IfMatch(e.clone()),
                _ => Condition::None,
            }
        }
    };
    match remote.put(keys::BUNDLE, local_json.as_bytes(), &cond) {
        Ok(ok) => {
            remember(db, t, keys::BUNDLE, ok.etag.as_deref(), 0);
            JobResult::Done
        }
        Err(e) => JobResult::Transient(e),
    }
}

/// After a permanent delete: remove the item's latest state and every snapshot on the target.
fn purge_remote(db: &Store, remote: &dyn ObjectStore, t: &RuntimeTarget, id: &str) -> JobResult {
    if let Err(e) = remote.delete(&keys::item(id)) {
        return JobResult::Transient(e);
    }
    let prefix = keys::versions_of(id);
    let mut token: Option<String> = None;
    loop {
        let page = match remote.list(&prefix, token.as_deref(), 1000) {
            Ok(p) => p,
            Err(e) => return JobResult::Transient(e),
        };
        for entry in &page.entries {
            if let Err(e) = remote.delete(&entry.key) {
                return JobResult::Transient(e);
            }
        }
        match page.next_token {
            Some(next) => token = Some(next),
            None => break,
        }
    }
    let _ = db.forget_remote_objects(&t.id, &keys::item(id), &prefix);
    JobResult::Done
}

// ------------------------------------------------------------------ the worker thread

#[derive(Default)]
struct State {
    targets: Vec<RuntimeTarget>,
    stores: HashMap<String, Arc<dyn ObjectStore>>,
    /// A drain was asked for since the worker last looked: a trigger fired (unlock, a committed
    /// change, a change of the target list).
    wake: bool,
    /// The unlock trigger: pull and reconcile before draining.
    reconcile: bool,
    /// How the worker reaches the unlocked vault for a pull.
    link: Option<Arc<dyn VaultLink>>,
}

struct Shared {
    path: PathBuf,
    rt: Arc<BackupRuntime>,
    state: Mutex<State>,
    cv: Condvar,
    stopped: AtomicBool,
    alive: AtomicBool,
    /// A pull is queued or running (`VaultStatus.reconciling`).
    reconciling: AtomicBool,
    /// The source target whose pull failed during the current trigger (its push is then not
    /// counted as a second failed attempt).
    source_failed: Mutex<Option<String>>,
    /// The drain connection; holding this lock *is* the per-engine drain lock.
    db: Mutex<Option<Store>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Shared {
    fn sorted_targets(&self) -> Vec<RuntimeTarget> {
        let mut v = lock(&self.state).targets.clone();
        v.sort_by_key(|t| (!t.master, t.position));
        v
    }

    fn remote_for(
        &self,
        t: &RuntimeTarget,
    ) -> std::result::Result<Arc<dyn ObjectStore>, RemoteError> {
        if let Some(s) = lock(&self.state).stores.get(&t.id) {
            return Ok(Arc::clone(s));
        }
        let store = self.rt.factory.open(&t.config)?;
        lock(&self.state)
            .stores
            .insert(t.id.clone(), Arc::clone(&store));
        Ok(store)
    }

    /// Run `f` with the engine's database connection (opened on first use). `None` when the
    /// database cannot be opened.
    fn with_db<T>(&self, f: impl FnOnce(&Store) -> T) -> Option<T> {
        let mut guard = lock(&self.db);
        if guard.is_none() {
            *guard = Store::open(&self.path).ok();
        }
        guard.as_ref().map(f)
    }

    fn drain_one(&self, t: &RuntimeTarget, force: bool, deadline: Option<Instant>) -> Outcome {
        let mut guard = lock(&self.db);
        if guard.is_none() {
            match Store::open(&self.path) {
                Ok(s) => *guard = Some(s),
                Err(_) => return Outcome::Failed,
            }
        }
        let Some(db) = guard.as_ref() else {
            return Outcome::Failed;
        };
        let remote = match self.remote_for(t) {
            Ok(r) => r,
            Err(e) => {
                let _ = db.target_record_failure(&t.id, &e.message(), e.is_hard(), now_millis());
                return Outcome::Failed;
            }
        };
        let counted = lock(&self.source_failed).as_deref() == Some(t.id.as_str());
        run_drain(
            db,
            remote.as_ref(),
            t,
            force,
            counted,
            &self.stopped,
            deadline,
        )
    }

    /// The pull of the unlock trigger and of "Back up now": read the source target (the master,
    /// or the first enabled target) and merge what other devices put there. A source that cannot
    /// be reached is recorded in its health and the push side goes on regardless.
    fn reconcile_source(&self) {
        self.reconcile_inner();
        self.reconciling.store(false, Ordering::SeqCst);
    }

    fn reconcile_inner(&self) {
        *lock(&self.source_failed) = None;
        let Some(link) = lock(&self.state).link.clone() else {
            return;
        };
        let Some(mut source) = self.sorted_targets().into_iter().next() else {
            return;
        };
        let mut guard = lock(&self.db);
        if guard.is_none() {
            match Store::open(&self.path) {
                Ok(s) => *guard = Some(s),
                Err(_) => return,
            }
        }
        let Some(db) = guard.as_ref() else { return };
        let source_id = source.id.clone();
        let failed = |message: &str, hard: bool| {
            let _ = db.target_record_failure(&source_id, message, hard, now_millis());
            *lock(&self.source_failed) = Some(source_id.clone());
        };
        let row = match db.target_row(&source.id) {
            Ok(Some(r)) if r.enabled => r,
            _ => return,
        };
        let remote = match self.remote_for(&source) {
            Ok(r) => r,
            Err(e) => return failed(&e.message(), e.is_hard()),
        };
        // A restored or imported target has never been reached: find out what the service can do
        // before the first request that matters (the push side relies on it).
        if let Err((message, hard)) = ensure_probed(db, remote.as_ref(), &mut source, &row) {
            return failed(&message, hard);
        }
        match pull::reconcile(db, remote.as_ref(), &source, link.as_ref(), &self.stopped) {
            Ok(_) => {
                let _ = db.target_record_success(&source.id, now_millis());
            }
            Err(PullError::Remote(e)) => failed(&e.message(), e.is_hard()),
            Err(PullError::Local) => failed(MSG_LOCAL, true),
            Err(PullError::Stopped) => {}
        }
    }

    /// One pass over every target, master first.
    fn drain_all(&self, force: bool, deadline: Option<Instant>) {
        for t in self.sorted_targets() {
            if must_stop(&self.stopped, deadline) {
                return;
            }
            self.drain_one(&t, force, deadline);
        }
    }
}

/// Sleep until a trigger wakes the thread, then (after an unlock) pull and reconcile, then drain,
/// then sleep again. Nothing in here ever wakes itself: no timeout on the wait, and a failed job
/// does not re-arm the flag.
fn worker_loop(shared: &Arc<Shared>) {
    loop {
        let reconcile = {
            let mut st = lock(&shared.state);
            loop {
                if shared.stopped.load(Ordering::SeqCst) {
                    return;
                }
                if st.wake {
                    break;
                }
                st = shared.cv.wait(st).unwrap_or_else(|p| p.into_inner());
            }
            st.wake = false;
            std::mem::take(&mut st.reconcile)
        };
        if reconcile {
            shared.reconcile_source();
        }
        shared.drain_all(false, None);
        *lock(&shared.source_failed) = None;
    }
}

/// The replication engine of one unlocked vault.
pub struct Replicator {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Replicator {
    pub fn new(path: PathBuf, rt: Arc<BackupRuntime>) -> Self {
        Self {
            shared: Arc::new(Shared {
                path,
                rt,
                state: Mutex::new(State::default()),
                cv: Condvar::new(),
                stopped: AtomicBool::new(false),
                alive: AtomicBool::new(false),
                reconciling: AtomicBool::new(false),
                source_failed: Mutex::new(None),
                db: Mutex::new(None),
            }),
            thread: Mutex::new(None),
        }
    }

    fn ensure_thread(&self) {
        if self.shared.stopped.load(Ordering::SeqCst) || !self.shared.rt.timing.background {
            return;
        }
        let mut slot = lock(&self.thread);
        if slot.is_some() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        shared.alive.store(true, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("hexis-backup".into())
            .spawn(move || {
                worker_loop(&shared);
                // Settings (and with them the keys) are wiped as the worker ends.
                let mut st = lock(&shared.state);
                st.targets.clear();
                st.stores.clear();
                drop(st);
                *lock(&shared.db) = None;
                shared.reconciling.store(false, Ordering::SeqCst);
                shared.alive.store(false, Ordering::SeqCst);
            });
        match spawned {
            Ok(handle) => *slot = Some(handle),
            Err(_) => self.shared.alive.store(false, Ordering::SeqCst),
        }
    }

    /// How a pull reaches the unlocked vault. Set once, before the first trigger.
    pub fn set_link(&self, link: Arc<dyn VaultLink>) {
        lock(&self.shared.state).link = Some(link);
    }

    /// Trigger 1 (unlock): pull and reconcile on the worker thread, then drain. Returns at once;
    /// the unlock never waits for the network.
    pub fn request_reconcile(&self) {
        if !self.shared.rt.timing.background || self.shared.stopped.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut st = lock(&self.shared.state);
            if st.targets.is_empty() || st.link.is_none() {
                return;
            }
            st.reconcile = true;
            st.wake = true;
            self.shared.reconciling.store(true, Ordering::SeqCst);
        }
        self.shared.cv.notify_all();
        self.ensure_thread();
    }

    /// A pull is queued or running.
    pub fn is_reconciling(&self) -> bool {
        self.shared.reconciling.load(Ordering::SeqCst)
    }

    /// Pull and reconcile from the calling thread (tests, and the first half of `run_now`).
    pub fn reconcile_now(&self) {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return;
        }
        self.shared.reconciling.store(true, Ordering::SeqCst);
        self.shared.reconcile_source();
    }

    /// Replace the set of targets the worker serves (enabled ones only) and wake it.
    pub fn set_targets(&self, targets: Vec<RuntimeTarget>) {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return;
        }
        let has_any = !targets.is_empty();
        {
            let mut st = lock(&self.shared.state);
            st.stores.clear();
            st.targets = targets;
            st.wake = true;
        }
        self.shared.cv.notify_all();
        if has_any {
            self.ensure_thread();
        }
    }

    /// Trigger: new work was committed. Wakes the worker, which drains and sleeps again.
    pub fn wake(&self) {
        {
            let mut st = lock(&self.shared.state);
            if st.targets.is_empty() {
                return;
            }
            st.wake = true;
        }
        self.shared.cv.notify_all();
        self.ensure_thread();
    }

    /// "Back up now": pull and reconcile, then drain (for one target or all), from the calling
    /// thread. Failures are recorded in the targets' health; the caller reads it back.
    pub fn run_now(&self, only: Option<&str>) {
        self.reconcile_now();
        self.drain_now(only);
        *lock(&self.shared.source_failed) = None;
    }

    /// Drain only, from the calling thread (the part of `run_now` that sends).
    pub fn drain_now(&self, only: Option<&str>) {
        for t in self.shared.sorted_targets() {
            if self.shared.stopped.load(Ordering::SeqCst) {
                return;
            }
            if only.is_none_or(|id| id == t.id) {
                self.shared.drain_one(&t, true, None);
            }
        }
    }

    /// Trigger 3 (app exit or backgrounding): send what is still queued, but never wait longer
    /// than `deadline` in total across all targets. Returns `true` when nothing is left in any
    /// enabled target's queue afterwards. `false` means the deadline passed, or a target could
    /// not be reached, or a job is blocked: nothing is lost either way, because the queue is
    /// durable and is retried at the next trigger.
    ///
    /// The work runs on its own thread so this call returns by the deadline even if a request
    /// is stuck in a slow connection; that thread stops at the next job boundary after the
    /// deadline (or at once when the engine is stopped).
    pub fn flush(&self, deadline: Duration) -> bool {
        if self.is_stopped() || lock(&self.shared.state).targets.is_empty() {
            return true;
        }
        let end = Instant::now() + deadline;
        let shared = Arc::clone(&self.shared);
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("hexis-flush".into())
            .spawn(move || {
                shared.drain_all(false, Some(end));
                let left = shared
                    .with_db(|db| db.outbox_pending_enabled())
                    .and_then(Result::ok);
                let _ = tx.send(left == Some(0));
            });
        if spawned.is_err() {
            return false;
        }
        rx.recv_timeout(deadline).unwrap_or(false)
    }

    /// Stop the worker and wipe the settings it holds. Does not wait for a request in flight
    /// (it ends within the request timeouts and then touches nothing).
    pub fn stop(&self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        {
            let mut st = lock(&self.shared.state);
            st.targets.clear();
            st.stores.clear();
        }
        self.shared.cv.notify_all();
    }

    /// `stop`, then wait for the thread to end (tests).
    pub fn stop_and_join(&self) {
        self.stop();
        if let Some(h) = lock(&self.thread).take() {
            let _ = h.join();
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::SeqCst)
    }

    /// Is the worker thread running right now?
    pub fn is_running(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    /// Settings currently held (ids), for tests.
    #[cfg(test)]
    pub fn held_target_ids(&self) -> Vec<String> {
        lock(&self.shared.state)
            .targets
            .iter()
            .map(|t| t.id.clone())
            .collect()
    }
}

impl Drop for Replicator {
    fn drop(&mut self) {
        self.stop();
    }
}
