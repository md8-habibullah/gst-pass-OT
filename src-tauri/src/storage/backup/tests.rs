//! Behavioural tests of Phase 4a: targets sealed in the vault, replication to in-memory buckets
//! (with fault injection), the single-writer rule, health states, restore into a fresh profile,
//! shared targets in both directions, and export stripping. Everything runs in temp folders and
//! in-memory buckets; the real S3 client is covered by `s3.rs` (loopback stub) and
//! `mock_s3_tests.rs` (the verifying mock server).

use super::dbops::{OutboxStats, TargetRow};
use super::remote::mem::{MemFactory, MemStore, Mode, Op};
use super::remote::{keys, NetKind, RemoteError};
use super::replicator::{compute_health, MSG_DIVERGED, MSG_NEWER_REMOTE, MSG_OTHER_VAULT};
use super::types::{BackupTargetInput, BackupTargetView, ConditionalWrites, HealthState};
use super::{BackupRuntime, Timing};
use crate::crypto::KdfParams;
use crate::storage::app::AppCore;
use crate::storage::db::Store;
use crate::storage::files::TestPicker;
use crate::storage::links::RecordingOpener;
use crate::storage::model::{ItemDraft, ItemType};
use crate::storage::session::{SessionConfig, EDIT_TOKEN_TTL};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod prefs_tests;
mod pull_tests;

pub(super) const PW: &str = "correct horse battery staple";
pub(super) const SECRET: &str = "SECRETKEY-9d41c7-do-not-leak";
pub(super) const KEY_ID: &str = "AKIDEXAMPLE0042";

pub(super) fn fast() -> SessionConfig {
    SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: EDIT_TOKEN_TTL,
        purge_grace: std::time::Duration::ZERO,
    }
}

/// No worker thread: queues move only when a test calls `backup_run` or the flush.
pub(super) fn manual() -> Timing {
    Timing {
        lagging_after: Duration::from_secs(300),
        failing_after: 5,
        background: false,
    }
}

/// The real worker thread (it only ever wakes on a trigger; there are no timers).
fn background() -> Timing {
    Timing {
        background: true,
        ..manual()
    }
}

struct Fx {
    dir: tempfile::TempDir,
    app: AppCore,
    mem: Arc<MemFactory>,
    picker: Arc<TestPicker>,
    #[allow(dead_code)]
    opener: Arc<RecordingOpener>,
}

impl Fx {
    fn new(timing: Timing) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mem = MemFactory::new();
        let picker = Arc::new(TestPicker::default());
        let opener = Arc::new(RecordingOpener::new());
        let app = AppCore::open(dir.path(), fast(), picker.clone())
            .unwrap()
            .with_backup_runtime(BackupRuntime::new(mem.clone(), timing))
            .with_opener(opener.clone());
        Self {
            dir,
            app,
            mem,
            picker,
            opener,
        }
    }

    /// Account "Main", unlocked, with one master target on bucket `bk`.
    fn with_target(timing: Timing) -> Self {
        let fx = Self::new(timing);
        fx.app.profile_create("Main", PW.into()).unwrap();
        let mut i = inp("Primary", "bkt");
        i.master = true;
        fx.app.backup_add(i).unwrap();
        fx
    }

    fn bucket(&self, name: &str) -> Arc<MemStore> {
        self.mem.bucket(name)
    }

    fn view(&self, name: &str) -> BackupTargetView {
        self.app
            .backup_list()
            .unwrap()
            .into_iter()
            .find(|v| v.name == name)
            .unwrap_or_else(|| panic!("no target {name}"))
    }

    fn state(&self, name: &str) -> HealthState {
        self.view(name).health.state
    }

    fn run(&self) -> Vec<BackupTargetView> {
        self.app.backup_run(None).unwrap()
    }

    /// Only the push half of "Back up now": no pull, so the single-writer rule is tested on its
    /// own (the pull has its own tests below).
    fn drain(&self) -> Vec<BackupTargetView> {
        let replicator = self
            .app
            .test_session()
            .test_vault(|v| v.backup_replicator().unwrap());
        replicator.drain_now(None);
        self.app.backup_list().unwrap()
    }

    fn id_of(&self, profile: &str) -> String {
        self.app
            .profile_list()
            .unwrap()
            .profiles
            .into_iter()
            .find(|p| p.display_name == profile)
            .unwrap_or_else(|| panic!("no profile {profile}"))
            .id
    }

    /// Select `profile` and unlock it (this locks whatever was open).
    fn switch_to(&self, profile: &str) {
        self.app.profile_select(&self.id_of(profile)).unwrap();
        self.app.unlock(PW.into()).unwrap();
    }

    fn data_listing(&self) -> Vec<String> {
        fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().into_owned());
                if p.is_dir() {
                    walk(&p, base, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self.dir.path(), self.dir.path(), &mut out);
        out.sort();
        out
    }
}

pub(super) fn inp(name: &str, bucket: &str) -> BackupTargetInput {
    BackupTargetInput {
        name: name.into(),
        endpoint: None,
        region: "us-east-1".into(),
        bucket: bucket.into(),
        prefix: String::new(),
        access_key_id: KEY_ID.into(),
        secret_access_key: SECRET.into(),
        path_style: false,
        enabled: true,
        master: false,
        share_in_master: true,
    }
}

pub(super) fn login(title: &str, password: &str) -> ItemDraft {
    ItemDraft {
        item_type: ItemType::Login,
        title: title.into(),
        favorite: false,
        reprompt: false,
        tags: vec![],
        folder: None,
        fields: [("username", "ana@example.com"), ("password", password)]
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect::<BTreeMap<_, _>>(),
        custom_fields: vec![],
    }
}

pub(super) fn edit_password(app: &AppCore, id: &str, password: &str) {
    let b = app.item_begin_edit(id, Some(PW.into())).unwrap();
    let mut d = b.draft.clone();
    d.fields.insert("password".into(), password.into());
    app.item_update(id, &b.token, b.version, d).unwrap();
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for: {what}");
}

fn code<T: std::fmt::Debug>(r: crate::storage::Result<T>) -> &'static str {
    r.expect_err("expected an error").code()
}

fn item_blob(fx: &Fx, id: &str) -> Vec<u8> {
    fx.app
        .test_session()
        .test_vault(|v| v.store().item_row(id).unwrap().unwrap().blob)
}

fn count_rows(fx: &Fx, table: &str) -> i64 {
    fx.app.test_session().test_vault(|v| {
        v.store()
            .conn()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    })
}

fn bucket_has(b: &MemStore, key: &str) -> bool {
    b.keys().iter().any(|k| k == key)
}

fn contains(data: &[u8], needle: &str) -> bool {
    data.windows(needle.len()).any(|w| w == needle.as_bytes())
}

// ============================================================ targets: add, list, secrecy

#[test]
fn a_new_target_is_saved_sealed_and_the_secret_never_comes_back() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    let v = fx.app.backup_add(inp("Primary", "bkt")).unwrap();
    assert_eq!((v.name.as_str(), v.bucket.as_str()), ("Primary", "bkt"));
    assert!(v.has_secret && v.enabled && v.share_in_master && !v.master);
    assert_eq!(v.health.conditional_writes, ConditionalWrites::Strong);
    assert_eq!(
        v.health.state,
        HealthState::Healthy,
        "the probe just succeeded"
    );
    assert!(v.health.last_success_at.is_some());
    let json = serde_json::to_string(&v).unwrap();
    assert!(!json.contains(SECRET), "{json}");
    assert!(json.contains("\"hasSecret\":true"));
    assert_eq!(fx.app.backup_list().unwrap(), vec![v]);
    // the probe object was deleted again
    assert!(fx.bucket("bkt").keys().is_empty());
}

#[test]
fn invalid_settings_are_refused_before_any_network_call() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    let edits: Vec<fn(&mut BackupTargetInput)> = vec![
        |i| i.name = "  ".into(),
        |i| i.secret_access_key = String::new(),
        |i| i.secret_access_key = "has space".into(),
        |i| i.region = String::new(),
        |i| i.bucket = "x".into(),
        |i| i.access_key_id = String::new(),
        |i| i.prefix = "a/../b".into(),
        |i| i.endpoint = Some("http://s3.example.com".into()),
        |i| i.endpoint = Some("https://user:pw@s3.example.com".into()),
        |i| i.endpoint = Some("javascript:alert(1)".into()),
        |i| {
            i.master = true;
            i.enabled = false;
        },
    ];
    for edit in edits {
        let mut i = inp("T", "bkt");
        edit(&mut i);
        assert_eq!(code(fx.app.backup_add(i)), "invalid_input");
    }
    assert!(fx.mem.opened().is_empty(), "no bucket was contacted");
    assert!(fx.app.backup_list().unwrap().is_empty());
}

#[test]
fn names_are_unique_per_vault_ignoring_case_and_the_number_of_targets_is_capped() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    fx.app.backup_add(inp("Primary", "bkt")).unwrap();
    assert_eq!(
        code(fx.app.backup_add(inp("  primary ", "bkt2"))),
        "invalid_input"
    );
    for n in 0..11 {
        fx.app
            .backup_add(inp(&format!("T{n}"), &format!("bucket-{n:02}")))
            .unwrap();
    }
    assert_eq!(fx.app.backup_list().unwrap().len(), 12);
    assert_eq!(
        code(fx.app.backup_add(inp("One too many", "bucket-xx"))),
        "invalid_input"
    );
}

#[test]
fn there_is_exactly_one_master_and_it_is_listed_first() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    let a = fx.app.backup_add(inp("A", "bucket-a")).unwrap();
    let mut b = inp("B", "bucket-b");
    b.master = true;
    fx.app.backup_add(b).unwrap();
    fx.app.backup_add(inp("C", "bucket-c")).unwrap();
    let names = |fx: &Fx| -> Vec<(String, bool)> {
        fx.app
            .backup_list()
            .unwrap()
            .into_iter()
            .map(|v| (v.name, v.master))
            .collect()
    };
    assert_eq!(
        names(&fx),
        [("B".into(), true), ("A".into(), false), ("C".into(), false)]
    );
    // a new master demotes the old one
    fx.app.backup_set_master(&a.id).unwrap();
    assert_eq!(
        names(&fx),
        [("A".into(), true), ("B".into(), false), ("C".into(), false)]
    );
    let mut c = inp("C", "bucket-c");
    c.master = true;
    let id_c = fx.view("C").id;
    fx.app.backup_update(&id_c, c).unwrap();
    let list = names(&fx);
    assert_eq!(list[0], ("C".into(), true));
    assert_eq!(list.iter().filter(|(_, m)| *m).count(), 1);
    assert_eq!(
        code(
            fx.app
                .backup_set_master("00000000-0000-4000-8000-000000000000")
        ),
        "not_found"
    );
}

#[test]
fn a_disabled_target_cannot_become_master() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    let mut i = inp("Off", "bkt");
    i.enabled = false;
    let v = fx.app.backup_add(i).unwrap();
    assert_eq!(v.health.state, HealthState::Disabled);
    assert_eq!(code(fx.app.backup_set_master(&v.id)), "invalid_input");
}

#[test]
fn a_failed_probe_still_saves_the_target_and_shows_why() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    fx.bucket("bkt").fail_all(None, RemoteError::BadCredentials);
    let v = fx.app.backup_add(inp("Primary", "bkt")).unwrap();
    assert_eq!(
        v.health.state,
        HealthState::Failing,
        "wrong keys are failing at once"
    );
    assert_eq!(
        v.health.last_error.as_deref(),
        Some(RemoteError::BadCredentials.message().as_str())
    );
    assert_eq!(v.health.conditional_writes, ConditionalWrites::Unknown);
    // nothing to retype: fix the service and retry
    fx.bucket("bkt").clear_faults();
    let after = fx.app.backup_run(Some(&v.id)).unwrap();
    assert_eq!(after[0].health.state, HealthState::Healthy);
    assert_eq!(after[0].health.last_error, None);
    assert!(fx.bucket("bkt").keys().contains(&keys::BUNDLE.to_string()));
}

#[test]
fn conditional_write_support_is_recorded_per_target() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    fx.bucket("weak-bucket").set_mode(Mode::Ignore);
    fx.bucket("legacy-bucket").set_mode(Mode::Reject);
    let mode = |name: &str, bucket: &str| {
        fx.app
            .backup_add(inp(name, bucket))
            .unwrap()
            .health
            .conditional_writes
    };
    assert_eq!(mode("Weak", "weak-bucket"), ConditionalWrites::Weak);
    assert_eq!(mode("Legacy", "legacy-bucket"), ConditionalWrites::Unknown);
    assert_eq!(mode("Strong", "strong-bucket"), ConditionalWrites::Strong);
}

// ============================================================ backup_test (no vault needed)

#[test]
fn backup_test_needs_no_unlocked_vault_and_leaves_nothing_behind() {
    let fx = Fx::new(manual()); // no profile at all
    let r = fx.app.backup_test(&inp("x", "bkt")).unwrap();
    assert!(r.ok, "{r:?}");
    assert_eq!(r.conditional_writes, ConditionalWrites::Strong);
    assert!(fx.bucket("bkt").keys().is_empty());
    assert_eq!(code(fx.app.backup_list()), "locked");

    fx.bucket("bkt").set_mode(Mode::Ignore);
    assert_eq!(
        fx.app
            .backup_test(&inp("x", "bkt"))
            .unwrap()
            .conditional_writes,
        ConditionalWrites::Weak
    );
}

#[test]
fn backup_test_reports_failures_as_answers_without_secrets_and_bad_input_as_errors() {
    let fx = Fx::new(manual());
    fx.bucket("secret-bucket")
        .fail_all(None, RemoteError::AccessDenied);
    let r = fx.app.backup_test(&inp("x", "secret-bucket")).unwrap();
    assert!(!r.ok);
    assert_eq!(r.message, RemoteError::AccessDenied.message());
    for s in [SECRET, KEY_ID, "secret-bucket"] {
        assert!(!r.message.contains(s), "{}", r.message);
    }
    fx.bucket("bkt2")
        .fail_all(None, RemoteError::Unreachable(NetKind::Dns));
    let r = fx.app.backup_test(&inp("x", "bkt2")).unwrap();
    assert!(!r.ok && r.message.contains("address"), "{}", r.message);
    let mut bad = inp("x", "bkt");
    bad.endpoint = Some("http://example.com".into());
    assert_eq!(code(fx.app.backup_test(&bad)), "invalid_input");
}

// ============================================================ update / remove / reveal

#[test]
fn an_empty_secret_keeps_the_stored_one_and_reveal_needs_the_password() {
    let fx = Fx::with_target(manual());
    let id = fx.view("Primary").id;
    assert_eq!(fx.app.backup_reveal_secret(&id, PW.into()).unwrap(), SECRET);
    assert_eq!(
        code(fx.app.backup_reveal_secret(&id, "wrong password!".into())),
        "wrong_password"
    );
    assert_eq!(
        code(
            fx.app
                .backup_reveal_secret("00000000-0000-4000-8000-000000000000", PW.into())
        ),
        "not_found"
    );

    let mut u = inp("Renamed", "bkt");
    u.secret_access_key = String::new();
    u.master = true;
    let v = fx.app.backup_update(&id, u).unwrap();
    assert_eq!(v.name, "Renamed");
    assert!(v.has_secret);
    assert_eq!(fx.app.backup_reveal_secret(&id, PW.into()).unwrap(), SECRET);

    let mut u = inp("Renamed", "bkt");
    u.secret_access_key = "A-NEW-SECRET".into();
    u.master = true;
    fx.app.backup_update(&id, u).unwrap();
    assert_eq!(
        fx.app.backup_reveal_secret(&id, PW.into()).unwrap(),
        "A-NEW-SECRET"
    );
    assert_eq!(
        code(fx.app.backup_update("nope", inp("x", "bkt"))),
        "not_found"
    );
}

#[test]
fn moving_a_target_to_another_bucket_uploads_everything_again() {
    let fx = Fx::with_target(manual());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    fx.run();
    assert!(bucket_has(&fx.bucket("bkt"), &keys::item(&item.id)));
    let id = fx.view("Primary").id;
    let mut u = inp("Primary", "bk-new");
    u.secret_access_key = String::new();
    u.master = true;
    let v = fx.app.backup_update(&id, u).unwrap();
    assert!(
        v.health.queued >= 3,
        "bundle, version and item are queued again: {}",
        v.health.queued
    );
    fx.run();
    let moved = fx.bucket("bk-new");
    assert!(bucket_has(&moved, keys::BUNDLE));
    assert!(bucket_has(&moved, &keys::item(&item.id)));
}

#[test]
fn disabling_clears_the_queue_and_re_enabling_uploads_everything() {
    let fx = Fx::with_target(manual());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    let id = fx.view("Primary").id;
    let mut off = inp("Primary", "bkt");
    off.secret_access_key = String::new();
    off.enabled = false;
    off.master = false;
    let v = fx.app.backup_update(&id, off).unwrap();
    assert_eq!(v.health.state, HealthState::Disabled);
    assert_eq!(v.health.queued, 0);
    let opened_before = fx.mem.opened().len();
    fx.run();
    assert_eq!(
        fx.mem.opened().len(),
        opened_before,
        "a disabled target is never contacted"
    );
    // new work does not queue for it either
    fx.app.item_create(login("More", "pw")).unwrap();
    assert_eq!(fx.view("Primary").health.queued, 0);

    let mut on = inp("Primary", "bkt");
    on.secret_access_key = String::new();
    let v = fx.app.backup_update(&id, on).unwrap();
    assert!(v.enabled);
    fx.run();
    assert!(bucket_has(&fx.bucket("bkt"), &keys::item(&item.id)));
    assert_eq!(fx.view("Primary").health.queued, 0);
}

#[test]
fn removing_a_target_forgets_it_and_leaves_the_bucket_alone() {
    let fx = Fx::with_target(manual());
    fx.app.item_create(login("Bank", "pw1")).unwrap();
    fx.run();
    let before = fx.bucket("bkt").keys();
    assert!(!before.is_empty());
    let id = fx.view("Primary").id;
    fx.app.backup_remove(&id).unwrap();
    assert!(fx.app.backup_list().unwrap().is_empty());
    assert_eq!(
        fx.bucket("bkt").keys(),
        before,
        "nothing is deleted remotely"
    );
    assert_eq!(count_rows(&fx, "targets"), 0);
    assert_eq!(count_rows(&fx, "outbox"), 0);
    assert_eq!(count_rows(&fx, "remote_objects"), 0);
    assert_eq!(code(fx.app.backup_remove(&id)), "not_found");
}

// ============================================================ replication

#[test]
fn every_change_is_replicated_master_first() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    fx.app.backup_add(inp("Second", "bucket-b")).unwrap();
    let mut m = inp("Master", "bucket-a");
    m.master = true;
    fx.app.backup_add(m).unwrap();
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    assert!(fx.view("Master").health.queued >= 3);
    let views = fx.run();
    assert!(views.iter().all(|v| v.health.queued == 0));
    assert!(views.iter().all(|v| v.health.state == HealthState::Healthy));
    assert_eq!(views[0].name, "Master", "the master is listed first");
    let opened = fx.mem.opened();
    assert_eq!(
        &opened[opened.len() - 2..],
        ["bucket-a", "bucket-b"],
        "master drained first"
    );

    for bucket in ["bucket-a", "bucket-b"] {
        let b = fx.bucket(bucket);
        let keys_now = b.keys();
        assert!(keys_now.contains(&keys::BUNDLE.to_string()), "{keys_now:?}");
        assert_eq!(
            b.raw(&keys::item(&item.id)).unwrap(),
            item_blob(&fx, &item.id)
        );
        assert_eq!(
            keys_now
                .iter()
                .filter(|k| k.starts_with("vault/versions/"))
                .count(),
            1
        );
    }
    // the master also holds the shared-targets snapshot, the other target does not
    assert!(bucket_has(&fx.bucket("bucket-a"), keys::TARGETS));
    assert!(!bucket_has(&fx.bucket("bucket-b"), keys::TARGETS));
}

#[test]
fn edits_replicate_as_new_snapshots_and_a_new_latest_state() {
    let fx = Fx::with_target(manual());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    fx.run();
    edit_password(&fx.app, &item.id, "pw2");
    edit_password(&fx.app, &item.id, "pw3");
    // two new snapshots and one pending "latest state" job: the edits collapse into one
    assert_eq!(fx.view("Primary").health.queued, 3);
    fx.run();
    let b = fx.bucket("bkt");
    assert_eq!(
        b.raw(&keys::item(&item.id)).unwrap(),
        item_blob(&fx, &item.id)
    );
    assert_eq!(
        b.keys()
            .iter()
            .filter(|k| k.starts_with("vault/versions/"))
            .count(),
        3
    );
    assert_eq!(fx.view("Primary").health.queued, 0);
}

#[test]
fn a_permanent_delete_uploads_a_tombstone_and_removes_the_remote_copies() {
    let fx = Fx::with_target(manual());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    edit_password(&fx.app, &item.id, "pw2");
    fx.run();
    let b = fx.bucket("bkt");
    assert!(bucket_has(&b, &keys::item(&item.id)));
    fx.app.item_trash(&item.id).unwrap();
    fx.app.item_purge(&item.id, PW.into(), "CONFIRM").unwrap();
    fx.run();
    assert!(!bucket_has(&b, &keys::item(&item.id)));
    assert!(!b
        .keys()
        .iter()
        .any(|k| k.contains(&item.id) && k.starts_with("vault/versions/")));
    assert!(bucket_has(&b, &keys::tombstone(&item.id)));
    assert_eq!(fx.view("Primary").health.queued, 0);
}

#[test]
fn an_item_erased_before_it_was_ever_uploaded_only_leaves_its_tombstone() {
    let fx = Fx::with_target(manual());
    let item = fx.app.item_create(login("Short lived", "pw1")).unwrap();
    fx.app.item_trash(&item.id).unwrap();
    fx.app.item_purge(&item.id, PW.into(), "CONFIRM").unwrap();
    fx.run();
    let keys_now = fx.bucket("bkt").keys();
    assert!(
        !keys_now
            .iter()
            .any(|k| k.contains("vault/items/") || k.contains("vault/versions/")),
        "{keys_now:?}"
    );
    assert!(keys_now.contains(&keys::tombstone(&item.id)));
}

#[test]
fn a_password_change_replaces_the_remote_key_bundle() {
    let fx = Fx::with_target(manual());
    fx.run();
    let before = fx.bucket("bkt").raw(keys::BUNDLE).unwrap();
    fx.app
        .change_password(PW.into(), "another long password 1".into())
        .unwrap();
    fx.run();
    let after = fx.bucket("bkt").raw(keys::BUNDLE).unwrap();
    assert_ne!(before, after);
    let local = fx
        .app
        .test_session()
        .test_vault(|v| v.store().bundle_json().unwrap().unwrap());
    assert_eq!(after, local.into_bytes());
    assert_eq!(fx.state("Primary"), HealthState::Healthy);
}

#[test]
fn a_bucket_that_holds_another_vault_is_never_overwritten() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Other", PW.into()).unwrap();
    fx.app.backup_add(inp("Shared bucket", "bkt")).unwrap();
    fx.app.item_create(login("Other's item", "x")).unwrap();
    fx.run();
    let theirs = fx.bucket("bkt").raw(keys::BUNDLE).unwrap();
    let their_keys = fx.bucket("bkt").keys();

    fx.app.profile_create("Mine", PW.into()).unwrap();
    fx.app.backup_add(inp("Same bucket", "bkt")).unwrap();
    fx.app.item_create(login("My item", "y")).unwrap();
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Failing);
    assert_eq!(views[0].health.last_error.as_deref(), Some(MSG_OTHER_VAULT));
    assert_eq!(fx.bucket("bkt").raw(keys::BUNDLE).unwrap(), theirs);
    assert_eq!(
        fx.bucket("bkt").keys(),
        their_keys,
        "nothing of mine was uploaded"
    );
}

// ============================================================ single writer (two devices)

/// Device A uploads item X (v1); device B restores, edits X (v2) and uploads it. Afterwards `A`
/// is locked and B is the open profile. Returns X's id.
fn two_devices(fx: &Fx) -> String {
    fx.app.profile_create("A", PW.into()).unwrap();
    let mut t = inp("Primary", "bkt");
    t.master = true;
    fx.app.backup_add(t).unwrap();
    let x = fx.app.item_create(login("Shared item", "v1")).unwrap();
    fx.run();

    fx.app
        .profile_restore_from_backup("B", inp("Restore source", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    edit_password(&fx.app, &x.id, "from-B");
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    x.id
}

#[test]
fn a_remote_item_that_is_newer_is_not_overwritten_and_the_target_shows_why() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx);
    edit_password(&fx.app, &x, "from-B-again"); // B: v3, uploaded
    fx.run();
    let remote_before = fx.bucket("bkt").raw(&keys::item(&x)).unwrap();

    fx.switch_to("A");
    edit_password(&fx.app, &x, "from-A"); // A: v2
    let views = fx.drain();
    assert_eq!(views[0].health.state, HealthState::Failing);
    assert_eq!(
        views[0].health.last_error.as_deref(),
        Some(MSG_NEWER_REMOTE)
    );
    assert_eq!(
        fx.bucket("bkt").raw(&keys::item(&x)).unwrap(),
        remote_before
    );
    assert!(views[0].health.queued >= 1, "the blocked job stays queued");
}

#[test]
fn two_different_copies_of_the_same_version_are_not_overwritten_either() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx); // remote: B's v2
    let remote_before = fx.bucket("bkt").raw(&keys::item(&x)).unwrap();
    fx.switch_to("A");
    edit_password(&fx.app, &x, "from-A"); // A: another v2
    let views = fx.drain();
    assert_eq!(views[0].health.state, HealthState::Failing);
    assert_eq!(views[0].health.last_error.as_deref(), Some(MSG_DIVERGED));
    assert_eq!(
        fx.bucket("bkt").raw(&keys::item(&x)).unwrap(),
        remote_before
    );
    // the immutable snapshots of both devices coexist (different timestamps)
    assert!(
        fx.bucket("bkt")
            .keys()
            .iter()
            .filter(|k| k.starts_with("vault/versions/"))
            .count()
            >= 3
    );
}

#[test]
fn on_a_service_that_ignores_preconditions_the_device_looks_before_it_writes() {
    let fx = Fx::new(manual());
    fx.bucket("bkt").set_mode(Mode::Ignore);
    let x = two_devices(&fx);
    assert_eq!(
        fx.view("Primary").health.conditional_writes,
        ConditionalWrites::Weak,
        "the restored target was probed before its first drain"
    );
    edit_password(&fx.app, &x, "from-B-again");
    fx.run();
    let remote_before = fx.bucket("bkt").raw(&keys::item(&x)).unwrap();

    fx.switch_to("A");
    edit_password(&fx.app, &x, "from-A");
    let views = fx.drain();
    assert_eq!(views[0].health.state, HealthState::Failing);
    assert_eq!(
        views[0].health.last_error.as_deref(),
        Some(MSG_NEWER_REMOTE)
    );
    assert_eq!(
        fx.bucket("bkt").raw(&keys::item(&x)).unwrap(),
        remote_before
    );
}

#[test]
fn an_older_remote_copy_is_replaced_by_the_newer_local_one() {
    let fx = Fx::with_target(manual());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    fx.run();
    // someone (a previous device) left an older state of the same item behind
    let older = fx.bucket("bkt").raw(&keys::item(&item.id)).unwrap();
    edit_password(&fx.app, &item.id, "pw2");
    fx.bucket("bkt").plant(&keys::item(&item.id), &older);
    fx.run();
    assert_eq!(
        fx.bucket("bkt").raw(&keys::item(&item.id)).unwrap(),
        item_blob(&fx, &item.id)
    );
    assert_eq!(fx.state("Primary"), HealthState::Healthy);
}

// ============================================================ health and failures

fn row(enabled: bool) -> TargetRow {
    TargetRow {
        id: "t".into(),
        position: 1,
        master: false,
        enabled,
        share_in_master: true,
        config: vec![],
        seeded: true,
        conditional: ConditionalWrites::Strong,
        last_success_at: None,
        last_checked_at: None,
        last_error: None,
        consecutive_errors: 0,
        hard_error: false,
        created_at: 0,
    }
}

#[test]
fn health_states_follow_the_rules_of_the_contract() {
    let t = Timing::default();
    let now = 10_000_000u64;
    let h = |row: &TargetRow, stats: &OutboxStats| compute_health(row, stats, now, &t);
    let idle = OutboxStats::default();

    // untested: never reached, nothing known
    assert_eq!(h(&row(true), &idle).state, HealthState::Untested);
    // healthy: reached, queue empty or young
    let mut ok = row(true);
    ok.last_success_at = Some(now - 1000);
    ok.last_checked_at = Some(now - 1000);
    assert_eq!(h(&ok, &idle).state, HealthState::Healthy);
    let young = OutboxStats {
        queued: 3,
        oldest_enqueued_at: Some(now - 60_000),
        ..Default::default()
    };
    assert_eq!(h(&ok, &young).state, HealthState::Healthy);
    // lagging: oldest job older than 5 minutes
    let old = OutboxStats {
        queued: 3,
        oldest_enqueued_at: Some(now - 301_000),
        ..Default::default()
    };
    assert_eq!(h(&ok, &old).state, HealthState::Lagging);
    // ... or a failed attempt that has not reached five yet
    let mut flaky = ok.clone();
    flaky.consecutive_errors = 4;
    flaky.last_error =
        Some("The storage service had a problem (HTTP 503). It will be tried again.".into());
    let health = h(&flaky, &young);
    assert_eq!(health.state, HealthState::Lagging);
    assert!(health.last_error.as_deref().unwrap().contains("503"));
    // failing: five consecutive errors
    flaky.consecutive_errors = 5;
    assert_eq!(h(&flaky, &young).state, HealthState::Failing);
    // failing at once: an error retrying cannot fix
    let mut hard = ok.clone();
    hard.consecutive_errors = 1;
    hard.hard_error = true;
    hard.last_error = Some("denied".into());
    assert_eq!(h(&hard, &idle).state, HealthState::Failing);
    // failing: a job waits for a person (single-writer conflict), with that reason
    let blocked = OutboxStats {
        queued: 2,
        blocked: 1,
        first_blocked_error: Some(MSG_NEWER_REMOTE.into()),
        ..Default::default()
    };
    let health = h(&ok, &blocked);
    assert_eq!(health.state, HealthState::Failing);
    assert_eq!(health.last_error.as_deref(), Some(MSG_NEWER_REMOTE));
    assert_eq!(health.queued, 2);
    // disabled wins over everything, and carries no error
    let mut off = hard.clone();
    off.enabled = false;
    let health = h(&off, &blocked);
    assert_eq!(
        (health.state, health.last_error),
        (HealthState::Disabled, None)
    );
    // a healthy target never shows an old error
    let mut healed = ok.clone();
    healed.last_error = Some("stale".into());
    assert_eq!(h(&healed, &idle).last_error, None);
}

#[test]
fn five_failed_attempts_make_a_target_failing_and_one_success_heals_it() {
    let fx = Fx::with_target(manual());
    fx.app.item_create(login("Bank", "pw1")).unwrap();
    fx.bucket("bkt").fail_all(None, RemoteError::Server(503));
    for attempt in 1..=4 {
        let v = fx.run().remove(0);
        assert_eq!(
            v.health.state,
            HealthState::Lagging,
            "after {attempt} failures"
        );
        assert!(v.health.last_error.unwrap().contains("503"));
        assert!(v.health.queued > 0);
    }
    let v = fx.run().remove(0);
    assert_eq!(v.health.state, HealthState::Failing, "the fifth failure");
    fx.bucket("bkt").clear_faults();
    let v = fx.run().remove(0);
    assert_eq!(v.health.state, HealthState::Healthy);
    assert_eq!(v.health.last_error, None);
    assert_eq!(v.health.queued, 0);
}

#[test]
fn a_service_that_refuses_the_keys_is_failing_at_the_first_attempt() {
    let fx = Fx::with_target(manual());
    fx.app.item_create(login("Bank", "pw1")).unwrap();
    fx.bucket("bkt").fail_all(None, RemoteError::AccessDenied);
    let v = fx.run().remove(0);
    assert_eq!(v.health.state, HealthState::Failing);
    assert_eq!(
        v.health.last_error.as_deref(),
        Some(RemoteError::AccessDenied.message().as_str())
    );
    assert!(v.health.queued > 0, "nothing was lost");
    fx.bucket("bkt").clear_faults();
    assert_eq!(fx.run()[0].health.state, HealthState::Healthy);
    assert_eq!(
        fx.bucket("bkt")
            .keys()
            .iter()
            .filter(|k| k.starts_with("vault/items/"))
            .count(),
        1
    );
}

#[test]
fn run_with_an_empty_queue_still_checks_the_connection() {
    let fx = Fx::with_target(manual());
    fx.run();
    fx.bucket("bkt").clear_log();
    assert_eq!(fx.drain()[0].health.state, HealthState::Healthy);
    assert_eq!(
        fx.bucket("bkt").count(Op::List),
        1,
        "a cheap listing proves the target is reachable"
    );
    fx.bucket("bkt")
        .fail_all(Some(Op::List), RemoteError::Unreachable(NetKind::Timeout));
    let v = fx.drain().remove(0);
    assert!(v.health.last_error.unwrap().contains("did not answer"));
}

#[test]
fn run_for_one_target_leaves_the_others_untouched() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    let a = fx.app.backup_add(inp("A", "bucket-a")).unwrap();
    fx.app.backup_add(inp("B", "bucket-b")).unwrap();
    fx.app.item_create(login("Bank", "pw1")).unwrap();
    let views = fx.app.backup_run(Some(&a.id)).unwrap();
    assert_eq!(
        views.iter().find(|v| v.name == "A").unwrap().health.queued,
        0
    );
    assert!(views.iter().find(|v| v.name == "B").unwrap().health.queued > 0);
    assert!(fx.bucket("bucket-b").keys().is_empty());
    assert_eq!(
        code(
            fx.app
                .backup_run(Some("00000000-0000-4000-8000-000000000000"))
        ),
        "not_found"
    );
}

// ============================================================ the worker thread

#[test]
fn the_worker_replicates_in_the_background_and_stops_when_the_vault_locks() {
    let fx = Fx::with_target(background());
    let replicator = fx
        .app
        .test_session()
        .test_vault(|v| v.backup_replicator().unwrap());
    assert!(replicator.is_running());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    let b = fx.bucket("bkt");
    wait_for("the item to appear in the bucket", || {
        bucket_has(&b, &keys::item(&item.id))
    });
    wait_for("an empty queue", || fx.view("Primary").health.queued == 0);

    fx.app.lock();
    assert!(replicator.is_stopped());
    wait_for("the worker to end", || !replicator.is_running());
    assert!(
        replicator.held_target_ids().is_empty(),
        "settings are wiped when it stops"
    );
    // nothing more is sent after the lock
    let calls = b.total_calls();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(b.total_calls(), calls);
}

#[test]
fn switching_to_another_profile_stops_the_worker_of_the_old_one() {
    let fx = Fx::with_target(background());
    let replicator = fx
        .app
        .test_session()
        .test_vault(|v| v.backup_replicator().unwrap());
    assert!(replicator.is_running());
    fx.app.profile_create("Second", PW.into()).unwrap();
    assert!(replicator.is_stopped());
    wait_for("the old worker to end", || !replicator.is_running());
    // a profile without targets never starts a worker
    let second = fx
        .app
        .test_session()
        .test_vault(|v| v.backup_replicator().unwrap());
    assert!(!second.is_running());
}

#[test]
fn dropping_the_app_stops_the_worker() {
    let fx = Fx::with_target(background());
    let replicator = fx
        .app
        .test_session()
        .test_vault(|v| v.backup_replicator().unwrap());
    drop(fx.app);
    assert!(replicator.is_stopped());
    wait_for("the worker to end", || !replicator.is_running());
}

// ============================================================ no timers: three triggers only

/// Wait for the "Primary" queue to empty, then let the worker finish its pass (it records the
/// success in the database after the last job, and may make one more, empty, pass).
fn settle(fx: &Fx) {
    wait_for("an empty queue", || fx.view("Primary").health.queued == 0);
    std::thread::sleep(Duration::from_millis(80));
}

/// Wait until a failed attempt is recorded for "Primary", then let the worker go back to sleep.
fn wait_for_failure(fx: &Fx) {
    wait_for("a recorded failure", || {
        fx.view("Primary").health.last_error.is_some()
    });
    std::thread::sleep(Duration::from_millis(80));
}

fn unreachable() -> RemoteError {
    RemoteError::Unreachable(NetKind::Timeout)
}

fn edit(fx: &Fx, id: &str, change: impl FnOnce(&mut ItemDraft)) {
    let b = fx.app.item_begin_edit(id, Some(PW.into())).unwrap();
    let mut d = b.draft.clone();
    change(&mut d);
    fx.app.item_update(id, &b.token, b.version, d).unwrap();
}

#[test]
fn a_drained_worker_sleeps_and_makes_no_store_call_at_all() {
    let fx = Fx::with_target(background());
    let b = fx.bucket("bkt");
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    wait_for("the item to appear in the bucket", || {
        bucket_has(&b, &keys::item(&item.id))
    });
    settle(&fx);
    let calls = b.total_calls();
    assert!(calls > 0, "the upload itself used the store");
    // a quiet period: no timer, no poll, no health check, no retry
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        b.total_calls(),
        calls,
        "an idle worker never touches the store"
    );
    // reading the app's state does not wake it either
    fx.app.backup_list().unwrap();
    fx.app.status().unwrap();
    fx.app.item_list().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(b.total_calls(), calls);
}

#[test]
fn a_failed_job_waits_for_the_next_trigger_and_is_not_retried_by_the_worker() {
    let fx = Fx::with_target(background());
    settle(&fx);
    let b = fx.bucket("bkt");
    b.fail_all(None, unreachable());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    wait_for_failure(&fx);
    let calls = b.total_calls();
    assert!(calls > 0, "the first attempt reached the store");
    // an old worker would have backed off and tried again; this one waits for a trigger
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(b.total_calls(), calls, "no retry without a trigger");
    let v = fx.view("Primary");
    assert!(v.health.queued > 0, "the job stays queued");
    assert_eq!(v.health.state, HealthState::Lagging);
    assert!(!bucket_has(&b, &keys::item(&item.id)));

    // trigger 2, a committed change: everything queued is tried again, the earlier job too
    b.clear_faults();
    let second = fx.app.item_create(login("Mail", "pw2")).unwrap();
    wait_for("both items to arrive", || {
        bucket_has(&b, &keys::item(&item.id)) && bucket_has(&b, &keys::item(&second.id))
    });
    settle(&fx);
    assert_eq!(fx.state("Primary"), HealthState::Healthy);
}

#[test]
fn unlocking_is_a_trigger_that_sends_what_is_queued() {
    let fx = Fx::with_target(background());
    settle(&fx);
    let b = fx.bucket("bkt");
    b.fail_all(None, unreachable());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    wait_for_failure(&fx);
    fx.app.lock();
    b.clear_faults();
    let calls = b.total_calls();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(b.total_calls(), calls, "a locked vault sends nothing");
    fx.app.unlock(PW.into()).unwrap();
    wait_for("the queued item to be sent after the unlock", || {
        bucket_has(&b, &keys::item(&item.id))
    });
    settle(&fx);
    assert_eq!(fx.state("Primary"), HealthState::Healthy);
}

#[test]
fn coming_back_online_is_a_trigger_that_sends_what_was_made_offline() {
    let fx = Fx::with_target(background());
    settle(&fx);
    let b = fx.bucket("bkt");
    // offline: the app still works, and the changes wait in the durable queue
    b.fail_all(None, unreachable());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    let second = fx.app.item_create(login("Mail", "pw2")).unwrap();
    wait_for_failure(&fx);
    assert_eq!(
        fx.app.item_list().unwrap().len(),
        2,
        "no connection, no loss of function"
    );
    let calls = b.total_calls();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(b.total_calls(), calls, "nothing is retried on a schedule");
    // the connection returns, and the page says so: no new change, no unlock, no exit
    b.clear_faults();
    fx.app.sync_reconnect();
    wait_for("what was made offline to be sent", || {
        bucket_has(&b, &keys::item(&item.id)) && bucket_has(&b, &keys::item(&second.id))
    });
    settle(&fx);
    assert_eq!(fx.state("Primary"), HealthState::Healthy);
}

#[test]
fn coming_back_online_with_the_vault_locked_or_nothing_selected_does_nothing() {
    let fx = Fx::with_target(background());
    settle(&fx);
    let b = fx.bucket("bkt");
    fx.app.lock();
    let calls = b.total_calls();
    fx.app.sync_reconnect();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(b.total_calls(), calls, "a locked vault contacts nobody");
}

#[test]
fn the_flush_is_a_trigger_and_reports_whether_everything_was_sent() {
    let fx = Fx::with_target(background());
    settle(&fx);
    let b = fx.bucket("bkt");
    b.fail_all(None, unreachable());
    let item = fx.app.item_create(login("Bank", "pw1")).unwrap();
    wait_for_failure(&fx);
    assert!(fx.app.has_pending_sync());
    // still unreachable: not complete, and the queue is kept
    assert!(!fx.app.flush_for_exit(Duration::from_secs(5)));
    assert!(fx.view("Primary").health.queued > 0);
    assert!(fx.app.has_pending_sync());
    // reachable again: the flush alone (no other trigger) sends it
    b.clear_faults();
    assert!(fx.app.flush_for_exit(Duration::from_secs(5)));
    assert!(bucket_has(&b, &keys::item(&item.id)));
    assert_eq!(fx.view("Primary").health.queued, 0);
    assert!(!fx.app.has_pending_sync());
    assert_eq!(fx.state("Primary"), HealthState::Healthy);
}

#[test]
fn the_flush_returns_by_its_deadline_even_when_the_store_is_slow() {
    let fx = Fx::with_target(manual());
    fx.run(); // probed, healthy, empty queue
    let b = fx.bucket("bkt");
    for n in 0..6 {
        fx.app
            .item_create(login(&format!("Item {n}"), "pw"))
            .unwrap();
    }
    let queued = fx.view("Primary").health.queued;
    assert!(queued >= 12, "an item and a snapshot per item: {queued}");
    b.set_delay(Duration::from_millis(150));
    let start = Instant::now();
    let complete = fx.app.flush_for_exit(Duration::from_millis(300));
    let took = start.elapsed();
    assert!(!complete, "a slow store cannot finish 12 uploads in 300 ms");
    assert!(
        took < Duration::from_millis(1500),
        "the flush must not run past its deadline: {took:?}"
    );
    // what was not sent is still queued for the next trigger
    let left = fx.view("Primary").health.queued;
    assert!(left > 0 && left < queued + 1, "left {left} of {queued}");
    // a later flush with enough time (the first one's thread ends at its next job boundary)
    b.set_delay(Duration::ZERO);
    assert!(fx.app.flush_for_exit(Duration::from_secs(10)));
    assert_eq!(fx.view("Primary").health.queued, 0);
}

#[test]
fn flushing_with_nothing_to_send_or_nothing_open_is_complete_at_once() {
    let fx = Fx::new(background());
    // no account at all
    assert!(fx.app.flush_for_exit(Duration::from_secs(8)));
    assert!(!fx.app.has_pending_sync());
    fx.app.profile_create("Main", PW.into()).unwrap();
    // an account without targets
    fx.app.item_create(login("Bank", "pw1")).unwrap();
    assert!(!fx.app.has_pending_sync());
    assert!(fx.app.flush_for_exit(Duration::from_secs(8)));
    // with a target and an empty queue
    let mut t = inp("Primary", "bkt");
    t.master = true;
    fx.app.backup_add(t).unwrap();
    settle(&fx);
    let b = fx.bucket("bkt");
    let calls = b.total_calls();
    let start = Instant::now();
    assert!(fx.app.flush_for_exit(Duration::from_secs(8)));
    assert!(start.elapsed() < Duration::from_millis(500));
    assert_eq!(
        b.total_calls(),
        calls,
        "an empty queue is not even a health check"
    );
    // locked: the keys are gone, so there is nothing a flush could do
    fx.app.lock();
    assert!(!fx.app.has_pending_sync());
    assert!(fx.app.flush_for_exit(Duration::from_millis(50)));
}

#[test]
fn every_way_of_changing_the_vault_queues_its_upload_and_wakes_the_worker() {
    let fx = Fx::with_target(background());
    let b = fx.bucket("bkt");
    let key_of = |id: &str| keys::item(id);
    // (the worker alone does all of this; nothing here calls backup_run or the flush)
    let synced = |fx: &Fx, id: &str| {
        wait_for("an empty queue", || fx.view("Primary").health.queued == 0);
        assert_eq!(
            b.raw(&key_of(id)).unwrap(),
            item_blob(fx, id),
            "the bucket holds the newest state"
        );
    };

    // create
    let x = fx.app.item_create(login("Bank", "pw1")).unwrap().id;
    synced(&fx, &x);
    let versions_of = |id: &str| {
        b.keys()
            .iter()
            .filter(|k| k.starts_with(&keys::versions_of(id)))
            .count()
    };
    assert_eq!(versions_of(&x), 1);
    // update
    edit_password(&fx.app, &x, "pw2");
    synced(&fx, &x);
    assert_eq!(versions_of(&x), 2);
    // favorite, tag and folder edits are updates of the same kind
    edit(&fx, &x, |d| d.favorite = true);
    synced(&fx, &x);
    edit(&fx, &x, |d| d.tags = vec!["work".into()]);
    synced(&fx, &x);
    edit(&fx, &x, |d| d.folder = Some("Money".into()));
    synced(&fx, &x);
    assert_eq!(versions_of(&x), 5);
    // archive and unarchive
    fx.app.item_archive(&x).unwrap();
    synced(&fx, &x);
    fx.app.item_unarchive(&x).unwrap();
    synced(&fx, &x);
    assert_eq!(versions_of(&x), 7);
    // restore from an old version
    fx.app.history_restore(&x, 1, Some(PW.into())).unwrap();
    synced(&fx, &x);
    assert_eq!(versions_of(&x), 8);
    // trash and restore from the trash
    fx.app.item_trash(&x).unwrap();
    synced(&fx, &x);
    fx.app.item_restore(&x).unwrap();
    synced(&fx, &x);
    assert_eq!(versions_of(&x), 10);
    // clone is a new item
    let y = fx.app.item_clone(&x, Some(PW.into())).unwrap().id;
    synced(&fx, &y);
    // purge: the remote copies go and the tombstone is left
    fx.app.item_trash(&y).unwrap();
    synced(&fx, &y);
    fx.app.item_purge(&y, PW.into(), "CONFIRM").unwrap();
    wait_for("the purge to reach the bucket", || {
        fx.view("Primary").health.queued == 0 && !bucket_has(&b, &key_of(&y))
    });
    assert!(bucket_has(&b, &keys::tombstone(&y)));
    assert_eq!(versions_of(&y), 0);
    // change the master password: the key bundle is replaced
    let bundle_before = b.raw(keys::BUNDLE).unwrap();
    fx.app
        .change_password(PW.into(), "another long passphrase 123".into())
        .unwrap();
    wait_for("the new bundle to reach the bucket", || {
        b.raw(keys::BUNDLE).is_some_and(|now| now != bundle_before)
    });
    wait_for("an empty queue", || fx.view("Primary").health.queued == 0);
    assert_eq!(fx.state("Primary"), HealthState::Healthy);
}

// ============================================================ restore

/// One account with items (one with history, one in the trash, one erased) replicated to the
/// master bucket `bk`, plus a shared and a device-only target.
struct Source {
    kept: String,
    trashed: String,
    erased: String,
}

fn populate(fx: &Fx) -> Source {
    fx.app.profile_create("Original", PW.into()).unwrap();
    let mut master = inp("Master", "bkt");
    master.master = true;
    fx.app.backup_add(master).unwrap();
    let mut shared = inp("Offsite", "offsite");
    shared.secret_access_key = "OFFSITE-SECRET".into();
    fx.app.backup_add(shared).unwrap();
    let mut private = inp("This device only", "private-bucket");
    private.share_in_master = false;
    private.secret_access_key = "PRIVATE-SECRET".into();
    fx.app.backup_add(private).unwrap();

    let kept = fx.app.item_create(login("Kept", "k1")).unwrap().id;
    edit_password(&fx.app, &kept, "k2");
    let trashed = fx.app.item_create(login("Trashed", "t1")).unwrap().id;
    fx.app.item_trash(&trashed).unwrap();
    let erased = fx.app.item_create(login("Erased", "e1")).unwrap().id;
    fx.app.item_trash(&erased).unwrap();
    fx.run();
    fx.app.item_purge(&erased, PW.into(), "CONFIRM").unwrap();
    fx.run();
    Source {
        kept,
        trashed,
        erased,
    }
}

#[test]
fn restoring_into_a_fresh_profile_brings_back_items_history_trash_and_shared_targets() {
    let fx = Fx::new(manual());
    let src = populate(&fx);
    let original_id = fx.id_of("Original");

    let info = fx
        .app
        .profile_restore_from_backup("Restored", inp("Where from", "bkt"))
        .unwrap();
    assert_eq!(info.display_name, "Restored");
    // selected but locked, next to the original
    let st = fx.app.status().unwrap();
    assert!(st.exists && !st.unlocked);
    assert_eq!(st.profile.unwrap().id, info.id);
    assert_eq!(fx.app.profile_list().unwrap().profiles.len(), 2);
    assert_ne!(info.id, original_id);
    assert_eq!(code(fx.app.item_list()), "locked");
    // wrong password: the data is authenticated only now
    assert_eq!(
        code(fx.app.unlock("not the password!!".into())),
        "wrong_password"
    );
    fx.app.unlock(PW.into()).unwrap();

    let items = fx.app.item_list().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, src.kept);
    assert_eq!(items[0].version, 2);
    assert_eq!(
        fx.app
            .item_reveal(&src.kept, Some(PW.into()))
            .unwrap()
            .fields["password"],
        "k2"
    );
    assert_eq!(
        fx.app.history_list(&src.kept).unwrap().len(),
        2,
        "version history came along"
    );
    assert_eq!(fx.app.trash_list().unwrap()[0].id, src.trashed);
    assert_eq!(
        code(fx.app.item_get(&src.erased)),
        "not_found",
        "an erased item stays erased"
    );

    // shared targets reappear with their credentials; the device-only one does not
    let names: Vec<String> = fx
        .app
        .backup_list()
        .unwrap()
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert_eq!(
        names,
        ["Master", "Offsite"],
        "master first, device-only target absent"
    );
    let master = fx.view("Master");
    assert!(master.master && master.share_in_master && master.has_secret);
    assert_eq!(
        fx.app.backup_reveal_secret(&master.id, PW.into()).unwrap(),
        SECRET
    );
    let offsite = fx.view("Offsite");
    assert_eq!(
        fx.app.backup_reveal_secret(&offsite.id, PW.into()).unwrap(),
        "OFFSITE-SECRET"
    );
    assert_eq!(
        (
            offsite.bucket.as_str(),
            offsite.access_key_id.as_str(),
            offsite.region.as_str()
        ),
        ("offsite", KEY_ID, "us-east-1")
    );

    // and the restored vault replicates: a new item reaches both targets (each is probed first,
    // because this device has never talked to a restored target)
    let fresh = fx.app.item_create(login("After restore", "n1")).unwrap();
    let views = fx.run();
    assert!(
        views.iter().all(|v| v.health.state == HealthState::Healthy),
        "{views:?}"
    );
    assert!(views
        .iter()
        .all(|v| v.health.conditional_writes == ConditionalWrites::Strong));
    assert!(bucket_has(&fx.bucket("bkt"), &keys::item(&fresh.id)));
    assert!(bucket_has(&fx.bucket("offsite"), &keys::item(&fresh.id)));
    assert!(
        bucket_has(&fx.bucket("offsite"), &keys::item(&src.kept)),
        "the new target got a first upload"
    );
}

#[test]
fn restoring_from_a_target_without_a_snapshot_yields_no_targets() {
    let fx = Fx::new(manual());
    populate(&fx);
    // "offsite" holds the vault but is not the master, so it has no shared-targets snapshot
    fx.app
        .profile_restore_from_backup("From offsite", inp("Offsite again", "offsite"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    assert!(fx.app.backup_list().unwrap().is_empty());
    assert_eq!(fx.app.item_list().unwrap().len(), 1);
}

#[test]
fn restore_checks_the_name_and_the_settings_before_the_first_request() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Taken", PW.into()).unwrap();
    assert_eq!(
        code(
            fx.app
                .profile_restore_from_backup("taken ", inp("x", "bkt"))
        ),
        "profile_name_taken"
    );
    assert_eq!(
        code(fx.app.profile_restore_from_backup("   ", inp("x", "bkt"))),
        "invalid_input"
    );
    assert_eq!(
        code(
            fx.app
                .profile_restore_from_backup(&"n".repeat(41), inp("x", "bkt"))
        ),
        "invalid_input"
    );
    let mut bad = inp("x", "bkt");
    bad.endpoint = Some("http://example.com".into());
    assert_eq!(
        code(fx.app.profile_restore_from_backup("Fine", bad)),
        "invalid_input"
    );
    let mut nosecret = inp("x", "bkt");
    nosecret.secret_access_key = String::new();
    assert_eq!(
        code(fx.app.profile_restore_from_backup("Fine", nosecret)),
        "invalid_input"
    );
    assert!(fx.mem.opened().is_empty(), "no bucket was contacted");
}

#[test]
fn a_failed_restore_leaves_nothing_behind_and_keeps_the_open_account_open() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Open one", PW.into()).unwrap();
    let it = fx.app.item_create(login("Mine", "x")).unwrap();
    let listing = fx.data_listing();
    let registry = std::fs::read(fx.dir.path().join("profiles.json")).unwrap();
    let check = |why: &str| {
        assert_eq!(fx.data_listing(), listing, "{why}: files");
        assert_eq!(
            std::fs::read(fx.dir.path().join("profiles.json")).unwrap(),
            registry,
            "{why}: registry"
        );
        assert_eq!(fx.app.profile_list().unwrap().profiles.len(), 1, "{why}");
        assert_eq!(
            fx.app.item_get(&it.id).unwrap().title,
            "Mine",
            "{why}: still unlocked"
        );
    };

    // nothing at that location
    let e = fx
        .app
        .profile_restore_from_backup("New", inp("x", "empty-bucket"))
        .unwrap_err();
    assert_eq!(e.code(), "remote");
    assert!(e.to_string().contains("No vault of this app"), "{e}");
    check("empty bucket");

    // the service refuses the keys / is down
    fx.bucket("denied")
        .fail_all(None, RemoteError::BadCredentials);
    assert_eq!(
        code(
            fx.app
                .profile_restore_from_backup("New", inp("x", "denied"))
        ),
        "remote"
    );
    check("wrong keys");
    fx.bucket("down")
        .fail_all(None, RemoteError::Unreachable(NetKind::Connect));
    assert_eq!(
        code(fx.app.profile_restore_from_backup("New", inp("x", "down"))),
        "network"
    );
    check("unreachable");

    // a bundle that is not a key bundle of this app
    fx.bucket("junk").plant(keys::BUNDLE, b"{\"hello\": 1}");
    assert_eq!(
        code(fx.app.profile_restore_from_backup("New", inp("x", "junk"))),
        "remote"
    );
    check("junk bundle");
    fx.bucket("junk2").plant(keys::BUNDLE, &[0xff, 0xfe, 0x00]);
    assert_eq!(
        code(fx.app.profile_restore_from_backup("New", inp("x", "junk2"))),
        "remote"
    );
    check("binary bundle");
}

#[test]
fn a_damaged_or_misplaced_object_stops_the_restore_and_cleans_up() {
    for case in [
        "garbage item",
        "item under another id",
        "snapshot with another version",
        "tombstone for another item",
    ] {
        let fx = Fx::new(manual());
        let src = populate(&fx);
        let b = fx.bucket("bkt");
        let other = item_blob(&fx, &src.kept);
        match case {
            "garbage item" => {
                b.plant(&keys::item(&src.kept), b"not an envelope");
            }
            "item under another id" => {
                b.plant(&keys::item("00000000-0000-4000-8000-0000000000aa"), &other);
            }
            "snapshot with another version" => {
                let snap = b
                    .keys()
                    .into_iter()
                    .find(|k| k.starts_with("vault/versions/"))
                    .unwrap();
                let (id, n, ts) = keys::parse_version(&snap).unwrap();
                let bytes = b.raw(&snap).unwrap();
                b.plant(&keys::version(id, n + 7, ts), &bytes);
            }
            _ => {
                b.plant(
                    &keys::tombstone("00000000-0000-4000-8000-0000000000bb"),
                    &other,
                );
            }
        }
        let listing = fx.data_listing();
        let e = fx
            .app
            .profile_restore_from_backup("Broken", inp("x", "bkt"))
            .unwrap_err();
        assert_eq!(e.code(), "remote", "{case}: {e}");
        assert_eq!(fx.data_listing(), listing, "{case}");
        assert_eq!(fx.app.profile_list().unwrap().profiles.len(), 1, "{case}");
    }
}

#[test]
fn objects_that_are_not_hexis_objects_are_ignored_by_a_restore() {
    let fx = Fx::new(manual());
    let src = populate(&fx);
    let b = fx.bucket("bkt");
    b.plant("vault/items/readme.txt", b"hello");
    b.plant("vault/other/thing.enc", b"hello");
    b.plant("vault/versions/not-a-uuid/1_2.enc", b"hello");
    b.plant("somewhere/else", b"hello");
    fx.app
        .profile_restore_from_backup("Restored", inp("x", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(fx.app.item_list().unwrap()[0].id, src.kept);
}

#[test]
fn a_big_vault_restores_across_listing_pages() {
    let fx = Fx::with_target(manual());
    let mut ids = Vec::new();
    for n in 0..12 {
        ids.push(
            fx.app
                .item_create(login(&format!("Item {n}"), "pw"))
                .unwrap()
                .id,
        );
    }
    fx.run();
    fx.bucket("bkt").set_page_size(5);
    fx.app
        .profile_restore_from_backup("Restored", inp("x", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    let mut got: Vec<String> = fx
        .app
        .item_list()
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    got.sort();
    ids.sort();
    assert_eq!(got, ids);
}

// ============================================================ shared targets and export

#[test]
fn only_targets_marked_shared_travel_in_an_exported_file() {
    let fx = Fx::new(manual());
    let src = populate(&fx);
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("copy.hexis");
    fx.picker.set_save(Some(dest.clone()));
    fx.app.vault_export_file(PW.into()).unwrap().unwrap();

    {
        let copy = Store::open(&dest).unwrap();
        let rows = copy.targets_all().unwrap();
        assert_eq!(rows.len(), 2, "the device-only target is not in the file");
        assert!(rows.iter().all(|r| r.share_in_master && !r.seeded));
        assert!(copy.meta("shared_targets").unwrap().is_none());
        let n: i64 = copy
            .conn()
            .query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "no queue travels");
        // the sealed settings are unreadable without the vault key
        let bytes = std::fs::read(&dest).unwrap();
        for needle in [
            SECRET,
            "OFFSITE-SECRET",
            "PRIVATE-SECRET",
            "private-bucket",
            "offsite",
        ] {
            assert!(!contains(&bytes, needle), "{needle}");
        }
    }
    // the live vault still has all three
    assert_eq!(fx.app.backup_list().unwrap().len(), 3);

    // importing it elsewhere brings the shared ones back, usable, with a first upload queued
    fx.picker.set_open(Some(dest));
    fx.app.profile_import_file("Imported").unwrap().unwrap();
    fx.app.unlock(PW.into()).unwrap();
    let names: Vec<String> = fx
        .app
        .backup_list()
        .unwrap()
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert_eq!(names, ["Master", "Offsite"]);
    let offsite = fx.view("Offsite");
    assert_eq!(
        fx.app.backup_reveal_secret(&offsite.id, PW.into()).unwrap(),
        "OFFSITE-SECRET"
    );
    assert!(
        offsite.health.queued > 0,
        "the importing device uploads everything once"
    );
    assert_eq!(fx.app.item_list().unwrap()[0].id, src.kept);
}

#[test]
fn the_master_snapshot_follows_every_change_of_the_shared_targets() {
    let fx = Fx::new(manual());
    populate(&fx);
    // stop sharing "Offsite" and add a new shared one; then a restore must see exactly that
    let offsite = fx.view("Offsite");
    let mut u = inp("Offsite", "offsite");
    u.secret_access_key = String::new();
    u.share_in_master = false;
    fx.app.backup_update(&offsite.id, u).unwrap();
    let mut added = inp("Added later", "later");
    added.secret_access_key = "LATER-SECRET".into();
    fx.app.backup_add(added).unwrap();
    fx.run();

    fx.app
        .profile_restore_from_backup("Restored", inp("x", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    let names: Vec<String> = fx
        .app
        .backup_list()
        .unwrap()
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert_eq!(names, ["Master", "Added later"]);
}

#[test]
fn the_snapshot_in_the_bucket_is_sealed_and_only_the_master_holds_it() {
    let fx = Fx::new(manual());
    populate(&fx);
    let snapshot = fx.bucket("bkt").raw(keys::TARGETS).unwrap();
    for needle in [
        SECRET,
        "OFFSITE-SECRET",
        "PRIVATE-SECRET",
        KEY_ID,
        "offsite",
        "private-bucket",
    ] {
        assert!(
            !contains(&snapshot, needle),
            "{needle} readable in the shared snapshot"
        );
    }
    assert!(!bucket_has(&fx.bucket("offsite"), keys::TARGETS));
}

// ============================================================ housekeeping at unlock

#[test]
fn tombstones_from_an_older_build_get_their_sealed_proof_at_the_next_unlock() {
    let fx = Fx::with_target(manual());
    let item = fx.app.item_create(login("Old", "pw")).unwrap();
    fx.app.item_trash(&item.id).unwrap();
    fx.app.item_purge(&item.id, PW.into(), "CONFIRM").unwrap();
    // pretend a Phase 2 build wrote the tombstone: no sealed blob
    fx.app.test_session().test_vault(|v| {
        v.store()
            .conn()
            .execute("UPDATE tombstones SET blob = NULL", [])
            .unwrap();
    });
    fx.app.lock();
    fx.app.unlock(PW.into()).unwrap();
    let blob = fx
        .app
        .test_session()
        .test_vault(|v| v.store().tombstone_blob(&item.id).unwrap());
    assert!(blob.is_some(), "backfilled");
    // a target added now receives it
    fx.app.backup_add(inp("Late", "late-bucket")).unwrap();
    fx.run();
    assert!(bucket_has(
        &fx.bucket("late-bucket"),
        &keys::tombstone(&item.id)
    ));
}

#[test]
fn a_target_whose_settings_cannot_be_decrypted_is_shown_failing_and_can_be_removed() {
    let fx = Fx::with_target(manual());
    fx.app.test_session().test_vault(|v| {
        v.store()
            .conn()
            .execute("UPDATE targets SET config = x'0001'", [])
            .unwrap();
    });
    let v = fx.app.backup_list().unwrap().remove(0);
    assert_eq!(v.name, "Unreadable target");
    assert_eq!(v.health.state, HealthState::Failing);
    assert!(!v.has_secret);
    assert_eq!(
        code(fx.app.backup_reveal_secret(&v.id, PW.into())),
        "crypto"
    );
    fx.app.backup_remove(&v.id).unwrap();
    assert!(fx.app.backup_list().unwrap().is_empty());
}

#[test]
fn nothing_about_a_target_is_visible_in_the_vault_file() {
    let fx = Fx::new(manual());
    let info = fx.app.profile_create("Leak check", PW.into()).unwrap();
    let mut i = inp("Distinctive Target Name", "distinctive-bucket-name");
    i.prefix = "distinctive/prefix".into();
    i.endpoint = Some("https://s3.distinctive-endpoint.example".into());
    i.access_key_id = "DISTINCTIVEKEYID123".into();
    i.master = true;
    fx.bucket("distinctive-bucket-name")
        .fail_all(None, RemoteError::Unreachable(NetKind::Connect));
    fx.app.backup_add(i).unwrap();
    fx.app.item_create(login("Bank", "pw")).unwrap();
    fx.run();
    let needles = [
        SECRET,
        "Distinctive Target Name",
        "distinctive-bucket-name",
        "distinctive/prefix",
        "distinctive-endpoint",
        "DISTINCTIVEKEYID123",
    ];
    let vdir = fx.dir.path().join("vaults").join(&info.id);
    for scan in ["unlocked", "locked"] {
        for entry in std::fs::read_dir(&vdir).unwrap() {
            let path = entry.unwrap().path();
            let data = std::fs::read(&path).unwrap();
            for n in needles {
                assert!(!contains(&data, n), "{scan}: {n:?} in {path:?}");
            }
        }
        fx.app.lock();
    }
    // errors recorded for a target carry no secrets either
    fx.app.unlock(PW.into()).unwrap();
    let v = fx.app.backup_list().unwrap().remove(0);
    let shown = serde_json::to_string(&v).unwrap();
    assert!(!shown.contains(SECRET));
    assert!(v.health.last_error.is_some());
}
