//! Replication of the encrypted drive: what a bucket holds (ciphertext only), the write-once blob
//! and the single-writer metadata, deletion, retries, seeding a new target and restore into a
//! fresh profile. Most tests run against in-memory buckets (with fault injection); the last group
//! drives the real S3 client against `scripts/mock_s3.py` (skipped, with a message, when
//! `python3` is not installed).

use super::dbops::{self, JobKind};
use super::files_sync::{self, keys};
use super::remote::mem::{MemFactory, MemStore, Mode, Op};
use super::remote::{Condition, ObjectStore, RemoteError, S3Factory};
use super::replicator::MSG_NEWER_REMOTE;
use super::restore::{self, RestoreCounts};
use super::s3::S3Store;
use super::tests::{fast, inp, login, manual, PW};
use super::types::{BackupTargetInput, HealthState, TargetConfig};
use super::{BackupRuntime, Timing};
use crate::crypto::{envelope, stream};
use crate::storage::app::AppCore;
use crate::storage::db::Store;
use crate::storage::drive::FileRow;
use crate::storage::files::TestPicker;
use crate::storage::FileEntry;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::Duration;

const MIB: usize = 1024 * 1024;
const MARKER: &str = "PLAINMARKER-7a41-DO-NOT-LEAK";

struct Fx {
    /// Keeps the app folder alive.
    _dir: tempfile::TempDir,
    src: tempfile::TempDir,
    app: AppCore,
    mem: Arc<MemFactory>,
    picker: Arc<TestPicker>,
}

impl Fx {
    /// Account "Main", unlocked, one master target on bucket `bkt`, its first upload done.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mem = MemFactory::new();
        let picker = Arc::new(TestPicker::default());
        let app = AppCore::open(dir.path(), fast(), picker.clone())
            .unwrap()
            .with_backup_runtime(BackupRuntime::new(mem.clone(), manual()));
        app.profile_create("Main", PW.into()).unwrap();
        let mut master = inp("Primary", "bkt");
        master.master = true;
        app.backup_add(master).unwrap();
        let fx = Self {
            _dir: dir,
            src: tempfile::tempdir().unwrap(),
            app,
            mem,
            picker,
        };
        fx.run();
        fx
    }

    fn bucket(&self) -> Arc<MemStore> {
        self.mem.bucket("bkt")
    }

    fn add(&self, item: Option<&str>, name: &str, bytes: &[u8]) -> FileEntry {
        add_file(&self.app, &self.picker, self.src.path(), item, name, bytes)
    }

    /// "Back up now": the first target's state afterwards.
    fn run(&self) -> HealthState {
        self.app.backup_run(None).unwrap()[0].health.state
    }

    fn queued(&self) -> u64 {
        self.app.backup_list().unwrap()[0].health.queued
    }

    fn local_dir(&self) -> std::path::PathBuf {
        self.app
            .test_session()
            .test_vault(|v| v.store().path().parent().unwrap().join("attachments"))
    }

    fn local_blob(&self, id: &str) -> Vec<u8> {
        std::fs::read(self.local_dir().join(format!("{id}.bin"))).unwrap()
    }

    fn row(&self, id: &str) -> FileRow {
        self.app
            .test_session()
            .test_vault(|v| v.store().file_row(id).unwrap().unwrap())
    }

    /// Queue the same job again, as after a crash between the upload and the bookkeeping.
    fn requeue(&self, kind: JobKind, id: &str) {
        self.app.test_session().test_vault(|v| {
            dbops::enqueue_for_enabled(v.store().conn(), kind, id, 0, 0, 5).unwrap();
        });
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

    fn switch_to(&self, profile: &str) {
        self.app.profile_select(&self.id_of(profile)).unwrap();
        self.app.unlock(PW.into()).unwrap();
    }
}

fn add_file(
    app: &AppCore,
    picker: &TestPicker,
    src: &std::path::Path,
    item: Option<&str>,
    name: &str,
    bytes: &[u8],
) -> FileEntry {
    let p = src.join(name);
    std::fs::write(&p, bytes).unwrap();
    picker.queue_files(vec![p]);
    app.file_add(item).unwrap().unwrap().remove(0)
}

fn data(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn drive_keys(b: &MemStore) -> Vec<String> {
    b.keys()
        .into_iter()
        .filter(|k| k.starts_with(keys::PREFIX))
        .collect()
}

fn count_in(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

fn puts_to(b: &MemStore, key: &str) -> usize {
    b.log()
        .iter()
        .filter(|(op, k, _)| *op == Op::Put && k == key)
        .count()
}

// ============================================================ what the bucket holds

#[test]
fn a_new_file_reaches_the_bucket_as_two_sealed_objects() {
    let fx = Fx::new();
    let item = fx.app.item_create(login("Passport", "pw")).unwrap().id;
    let content = [
        MARKER.as_bytes(),
        &data(2_300_000, 3)[..],
        MARKER.as_bytes(),
    ]
    .concat();
    let a = fx.add(None, "LEAKNAME-tax-return.pdf", &content);
    let b = fx.add(Some(&item), "LEAKNAME-passport.png", MARKER.as_bytes());
    assert!(fx.queued() >= 4);
    assert_eq!(fx.run(), HealthState::Healthy);
    assert_eq!(fx.queued(), 0);

    let bucket = fx.bucket();
    let mut have = drive_keys(&bucket);
    have.sort();
    let mut want = vec![
        keys::blob(&a.id),
        keys::meta(&a.id),
        keys::blob(&b.id),
        keys::meta(&b.id),
    ];
    want.sort();
    assert_eq!(have, want);

    // the blob is exactly the local ciphertext; the metadata exactly the sealed row
    assert_eq!(
        bucket.raw(&keys::blob(&a.id)).unwrap(),
        fx.local_blob(&a.id)
    );
    assert_eq!(bucket.raw(&keys::meta(&b.id)).unwrap(), fx.row(&b.id).meta);

    // nothing readable anywhere in the bucket: not the content, not a name, not a type
    for key in bucket.keys() {
        let bytes = bucket.raw(&key).unwrap();
        for needle in [
            MARKER,
            "LEAKNAME",
            "tax-return",
            "passport.png",
            "application/pdf",
            "image/png",
        ] {
            assert_eq!(
                count_in(&bytes, needle.as_bytes()),
                0,
                "{needle:?} in {key}"
            );
        }
        assert_eq!(count_in(key.as_bytes(), b"LEAKNAME"), 0, "a name in a key");
    }
    // the owner of an attachment is inside the sealed metadata, nowhere else in the drive objects
    for key in drive_keys(&bucket) {
        let bytes = bucket.raw(&key).unwrap();
        assert_eq!(count_in(&bytes, item.as_bytes()), 0, "item id in {key}");
        assert_eq!(count_in(key.as_bytes(), item.as_bytes()), 0);
    }
    // the metadata envelope is bound to its file
    let meta = bucket.raw(&keys::meta(&a.id)).unwrap();
    assert_eq!(
        envelope::peek_header(&meta).unwrap().item_id,
        crate::storage::vault_drive::meta_object_id(&a.id)
    );
}

#[test]
fn the_blob_is_sent_before_its_metadata_and_both_are_write_once_on_first_upload() {
    let fx = Fx::new();
    let a = fx.add(None, "a.bin", &data(5000, 1));
    fx.bucket().clear_log();
    fx.run();
    let log = fx.bucket().log();
    let pos = |key: &str| log.iter().position(|(op, k, _)| *op == Op::Put && k == key);
    let blob = pos(&keys::blob(&a.id)).expect("blob put");
    let meta = pos(&keys::meta(&a.id)).expect("meta put");
    assert!(blob < meta, "a name never appears before its data");
    assert_eq!(log[blob].2, Condition::IfNoneMatch);
    assert_eq!(log[meta].2, Condition::IfNoneMatch);
}

#[test]
fn nothing_is_sent_twice_and_a_repeated_job_is_harmless() {
    let fx = Fx::new();
    let a = fx.add(None, "a.bin", &data(5000, 2));
    fx.run();
    let blob_etag = fx.bucket().etag_of(&keys::blob(&a.id)).unwrap();
    fx.bucket().clear_log();
    assert_eq!(fx.run(), HealthState::Healthy);
    assert_eq!(
        fx.bucket().count(Op::Put),
        0,
        "an empty queue sends nothing"
    );

    // the same jobs again
    fx.requeue(JobKind::FileBlob, &a.id);
    fx.requeue(JobKind::FileMeta, &a.id);
    assert_eq!(fx.run(), HealthState::Healthy);
    assert_eq!(fx.queued(), 0);
    assert_eq!(
        fx.bucket().etag_of(&keys::blob(&a.id)).unwrap(),
        blob_etag,
        "the immutable blob was not rewritten (the 412 counted as success)"
    );
    assert_eq!(
        fx.bucket().raw(&keys::blob(&a.id)).unwrap(),
        fx.local_blob(&a.id)
    );
}

#[test]
fn a_service_that_ignores_preconditions_is_asked_before_a_blob_is_written() {
    let fx = Fx::new();
    // a second target on a service that ignores conditional headers
    fx.mem.bucket("weak").set_mode(Mode::Ignore);
    fx.app.backup_add(inp("Weak", "weak")).unwrap();
    let a = fx.add(None, "a.bin", &data(3000, 4));
    fx.app.backup_run(None).unwrap();
    let weak = fx.mem.bucket("weak");
    assert_eq!(weak.raw(&keys::blob(&a.id)).unwrap(), fx.local_blob(&a.id));
    assert_eq!(weak.raw(&keys::meta(&a.id)).unwrap(), fx.row(&a.id).meta);

    // again: a HEAD finds the blob, no PUT is made for it
    weak.clear_log();
    fx.requeue(JobKind::FileBlob, &a.id);
    fx.app.backup_run(None).unwrap();
    assert_eq!(puts_to(&weak, &keys::blob(&a.id)), 0);
    assert!(weak
        .log()
        .iter()
        .any(|(op, k, _)| *op == Op::Head && *k == keys::blob(&a.id)));
}

// ============================================================ changes

#[test]
fn a_rename_replaces_only_the_metadata_object() {
    let fx = Fx::new();
    let a = fx.add(None, "a.txt", &data(4000, 5));
    fx.run();
    let blob_etag = fx.bucket().etag_of(&keys::blob(&a.id)).unwrap();
    let meta_before = fx.bucket().raw(&keys::meta(&a.id)).unwrap();
    let meta_etag = fx.bucket().etag_of(&keys::meta(&a.id)).unwrap();
    fx.bucket().clear_log();

    fx.app.file_rename(&a.id, "renamed.txt").unwrap();
    assert_eq!(fx.run(), HealthState::Healthy);
    assert_eq!(fx.bucket().etag_of(&keys::blob(&a.id)).unwrap(), blob_etag);
    assert_ne!(fx.bucket().etag_of(&keys::meta(&a.id)).unwrap(), meta_etag);
    let meta_after = fx.bucket().raw(&keys::meta(&a.id)).unwrap();
    assert_ne!(meta_after, meta_before);
    assert_eq!(meta_after, fx.row(&a.id).meta);
    assert_eq!(puts_to(&fx.bucket(), &keys::blob(&a.id)), 0);
    // guarded by the ETag this device last saw (the single-writer rule)
    let (_, _, cond) = fx
        .bucket()
        .log()
        .into_iter()
        .find(|(op, k, _)| *op == Op::Put && *k == keys::meta(&a.id))
        .unwrap();
    assert_eq!(cond, Condition::IfMatch(meta_etag));
    assert_eq!(envelope::peek_header(&meta_after).unwrap().version, 2);
}

#[test]
fn a_newer_metadata_object_in_the_bucket_is_never_overwritten() {
    let fx = Fx::new();
    let a = fx.add(None, "a.txt", b"abc");
    fx.run();
    fx.app.file_rename(&a.id, "b.txt").unwrap();
    fx.run();
    let v2 = fx.row(&a.id).meta;
    fx.app.file_rename(&a.id, "c.txt").unwrap();
    fx.run();
    let remote_v3 = fx.bucket().raw(&keys::meta(&a.id)).unwrap();
    assert_ne!(remote_v3, v2);

    // another device writes the newer copy (a new ETag) while this device is still at version 2 ...
    fx.bucket().plant(&keys::meta(&a.id), &remote_v3);
    fx.app.test_session().test_vault(|v| {
        v.store()
            .conn()
            .execute(
                "UPDATE files SET meta = ?1, meta_version = 2 WHERE id = ?2",
                rusqlite::params![v2, a.id],
            )
            .unwrap();
    });
    fx.requeue(JobKind::FileMeta, &a.id);
    let views = fx.app.backup_run(None).unwrap();
    // ... and the bucket's newer copy is left alone, with the reason shown
    assert_eq!(views[0].health.state, HealthState::Failing);
    assert_eq!(
        views[0].health.last_error.as_deref(),
        Some(MSG_NEWER_REMOTE)
    );
    assert_eq!(fx.bucket().raw(&keys::meta(&a.id)).unwrap(), remote_v3);
    assert!(views[0].health.queued >= 1, "the blocked job stays queued");
}

#[test]
fn deleting_a_file_removes_both_objects_and_drops_uploads_that_never_happened() {
    let fx = Fx::new();
    let sent = fx.add(None, "sent.bin", &data(3000, 6));
    fx.run();
    assert_eq!(drive_keys(&fx.bucket()).len(), 2);
    let unsent = fx.add(None, "unsent.bin", &data(3000, 7));

    fx.app.file_delete(&sent.id, PW.into()).unwrap();
    fx.app.file_delete(&unsent.id, PW.into()).unwrap();
    fx.bucket().clear_log();
    assert_eq!(fx.run(), HealthState::Healthy);
    assert!(drive_keys(&fx.bucket()).is_empty());
    assert_eq!(
        puts_to(&fx.bucket(), &keys::blob(&unsent.id)),
        0,
        "a file deleted before its first upload is never uploaded"
    );
    assert_eq!(puts_to(&fx.bucket(), &keys::meta(&unsent.id)), 0);
    // the metadata goes first, so no name outlives its data
    let deletes: Vec<_> = fx
        .bucket()
        .log()
        .into_iter()
        .filter(|(op, k, _)| *op == Op::Delete && k.contains(&sent.id))
        .map(|(_, k, _)| k)
        .collect();
    assert_eq!(deletes, vec![keys::meta(&sent.id), keys::blob(&sent.id)]);
    assert_eq!(fx.queued(), 0);
}

#[test]
fn erasing_an_item_removes_its_files_from_the_bucket_and_keeps_the_others() {
    let fx = Fx::new();
    let item = fx.app.item_create(login("Passport", "pw")).unwrap().id;
    let mine = fx.add(Some(&item), "mine.bin", &data(3000, 8));
    let other = fx.add(None, "other.bin", &data(3000, 9));
    fx.run();
    assert_eq!(drive_keys(&fx.bucket()).len(), 4);

    fx.app.item_trash(&item).unwrap();
    fx.app.item_purge(&item, PW.into(), "CONFIRM").unwrap();
    assert_eq!(fx.run(), HealthState::Healthy);
    let mut have = drive_keys(&fx.bucket());
    have.sort();
    let mut want = vec![keys::blob(&other.id), keys::meta(&other.id)];
    want.sort();
    assert_eq!(have, want);
    assert!(!drive_keys(&fx.bucket())
        .iter()
        .any(|k| k.contains(&mine.id)));
}

// ============================================================ failures

#[test]
fn a_failed_upload_stays_queued_with_its_error_and_the_next_trigger_retries_it() {
    let fx = Fx::new();
    let a = fx.add(None, "a.bin", &data(2 * MIB, 10));
    fx.bucket()
        .fail_next(Some(Op::Put), RemoteError::Server(503), 1);
    let views = fx.app.backup_run(None).unwrap();
    assert_ne!(views[0].health.state, HealthState::Healthy);
    assert!(views[0].health.queued >= 2, "{views:?}");
    let err = views[0].health.last_error.clone().unwrap();
    assert!(err.contains("503"), "{err}");
    assert!(!drive_keys(&fx.bucket()).contains(&keys::blob(&a.id)));
    assert!(
        !drive_keys(&fx.bucket()).contains(&keys::meta(&a.id)),
        "no name without its data"
    );

    // the next trigger finishes the job
    assert_eq!(fx.run(), HealthState::Healthy);
    assert_eq!(fx.queued(), 0);
    assert_eq!(
        fx.bucket().raw(&keys::blob(&a.id)).unwrap(),
        fx.local_blob(&a.id)
    );
    assert!(drive_keys(&fx.bucket()).contains(&keys::meta(&a.id)));
}

#[test]
fn a_missing_local_blob_blocks_only_its_own_file() {
    let fx = Fx::new();
    let broken = fx.add(None, "broken.bin", &data(3000, 11));
    let fine = fx.add(None, "fine.bin", &data(3000, 12));
    std::fs::remove_file(fx.local_dir().join(format!("{}.bin", broken.id))).unwrap();
    let views = fx.app.backup_run(None).unwrap();
    assert_eq!(views[0].health.state, HealthState::Failing);
    assert!(views[0]
        .health
        .last_error
        .clone()
        .unwrap()
        .contains("missing on this device"));
    // the other file still went through; the broken one's name was held back with its data
    let have = drive_keys(&fx.bucket());
    assert!(have.contains(&keys::blob(&fine.id)));
    assert!(have.contains(&keys::meta(&fine.id)));
    assert!(!have.contains(&keys::blob(&broken.id)));
    assert!(
        !have.contains(&keys::meta(&broken.id)),
        "a name is never sent without its data"
    );
    // deleting the broken file clears the message
    fx.app.file_delete(&broken.id, PW.into()).unwrap();
    assert_eq!(fx.run(), HealthState::Healthy);
    assert_eq!(fx.queued(), 0);
}

#[test]
fn a_different_object_under_a_blobs_name_is_never_overwritten() {
    let fx = Fx::new();
    let a = fx.add(None, "a.bin", &data(3000, 13));
    fx.bucket()
        .plant(&keys::blob(&a.id), b"somebody else's object");
    assert_eq!(fx.run(), HealthState::Failing);
    assert_eq!(
        fx.bucket().raw(&keys::blob(&a.id)).unwrap(),
        b"somebody else's object"
    );
    assert!(
        !drive_keys(&fx.bucket()).contains(&keys::meta(&a.id)),
        "no name for data that is not ours"
    );
}

// ============================================================ seeding a new target

#[test]
fn a_target_added_later_receives_every_existing_file() {
    let fx = Fx::new();
    let a = fx.add(None, "a.bin", &data(3000, 14));
    let item = fx.app.item_create(login("Passport", "pw")).unwrap().id;
    let b = fx.add(Some(&item), "b.bin", &data(2 * MIB + 7, 15));
    fx.run();
    fx.app.backup_add(inp("Offsite", "offsite")).unwrap();
    fx.app.backup_run(None).unwrap();
    let off = fx.mem.bucket("offsite");
    for f in [&a, &b] {
        assert_eq!(off.raw(&keys::blob(&f.id)).unwrap(), fx.local_blob(&f.id));
        assert_eq!(off.raw(&keys::meta(&f.id)).unwrap(), fx.row(&f.id).meta);
    }
    // and both targets are kept in step afterwards
    fx.app.file_rename(&a.id, "a2.bin").unwrap();
    fx.app.backup_run(None).unwrap();
    assert_eq!(off.raw(&keys::meta(&a.id)).unwrap(), fx.row(&a.id).meta);
    assert_eq!(
        fx.bucket().raw(&keys::meta(&a.id)).unwrap(),
        fx.row(&a.id).meta
    );
}

// ============================================================ restore

/// A fresh vault file restored from the bucket named `bucket`, below its own temp folder.
fn restore_fresh(fx: &Fx, bucket: &str) -> (tempfile::TempDir, Store, RestoreCounts) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("vault.db")).unwrap();
    let cfg = TargetConfig::from_input(&inp("src", bucket), true).unwrap();
    let rt = BackupRuntime::new(fx.mem.clone(), manual());
    let counts = restore::restore_into(&mut store, &cfg, &rt, "device-r").unwrap();
    (dir, store, counts)
}

#[test]
fn a_restore_brings_files_back_with_names_owners_and_content() {
    let fx = Fx::new();
    let item = fx.app.item_create(login("Passport", "pw")).unwrap().id;
    let big = data(2 * MIB + 5, 21);
    let a = fx.add(None, "Tax return.pdf", &big);
    let b = fx.add(Some(&item), "scan.png", &data(777, 22));
    let empty = fx.add(None, "empty.txt", b"");
    fx.app.file_rename(&b.id, "scan-2.png").unwrap();
    fx.run();

    let info = fx
        .app
        .profile_restore_from_backup("Restored", inp("Where from", "bkt"))
        .unwrap();
    assert_eq!(info.display_name, "Restored");
    // before the first unlock nothing about a file is readable
    assert_eq!(fx.app.file_list(None).unwrap_err().code(), "locked");
    fx.app.unlock(PW.into()).unwrap();

    let mut standalone = fx.app.file_list(None).unwrap();
    standalone.sort_by(|x, y| x.name.cmp(&y.name));
    assert_eq!(
        standalone
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["Tax return.pdf", "empty.txt"]
    );
    assert_eq!(standalone[0].size, big.len() as u64);
    assert_eq!(standalone[0].mime.as_deref(), Some("application/pdf"));
    assert_eq!(standalone[0].added_at, a.added_at);
    assert_eq!(
        &**fx.app.file_read(&a.id).unwrap(),
        &big[..],
        "a multi-chunk file decrypts to the original bytes"
    );
    assert!(fx.app.file_read(&empty.id).unwrap().is_empty());
    // the item's attachment learned its owner from the sealed metadata, and has its new name
    let view = fx.app.item_get(&item).unwrap();
    assert_eq!(view.attachments.len(), 1);
    assert_eq!(view.attachments[0].name, "scan-2.png");
    assert_eq!(view.attachments[0].id, b.id);
    assert_eq!(fx.app.file_read(&b.id).unwrap().len(), 777);
    // restored files are ordinary files: they can be renamed and deleted again
    fx.app.file_rename(&a.id, "Tax.pdf").unwrap();
    fx.app.file_delete(&empty.id, PW.into()).unwrap();
    assert_eq!(fx.app.file_list(None).unwrap().len(), 1);
}

#[test]
fn a_restore_skips_what_does_not_belong_together_and_counts_it() {
    let fx = Fx::new();
    let good = fx.add(None, "good.bin", &data(2 * MIB + 1, 31));
    let tiny = fx.add(None, "tiny.bin", &data(10, 32));
    let truncated = fx.add(None, "trunc.bin", &data(2 * MIB + 1, 33));
    let swapped = fx.add(None, "swapped.bin", &data(3000, 34));
    let no_meta = fx.add(None, "nometa.bin", &data(3000, 35));
    let no_blob = fx.add(None, "noblob.bin", &data(3000, 36));
    let bad_meta = fx.add(None, "badmeta.bin", &data(3000, 37));
    let meta_of_other = fx.add(None, "otherheader.bin", &data(3000, 38));
    fx.run();
    let b = fx.bucket();
    // a truncated blob: header intact, but too short for its chunk count
    let t = b.raw(&keys::blob(&truncated.id)).unwrap();
    b.plant(&keys::blob(&truncated.id), &t[..t.len() - 100]);
    // a complete blob of another file under this file's name
    b.plant(
        &keys::blob(&swapped.id),
        &b.raw(&keys::blob(&tiny.id)).unwrap(),
    );
    b.remove(&keys::meta(&no_meta.id));
    b.remove(&keys::blob(&no_blob.id));
    b.plant(&keys::meta(&bad_meta.id), b"not an envelope");
    // a metadata envelope sealed for another file
    b.plant(
        &keys::meta(&meta_of_other.id),
        &b.raw(&keys::meta(&good.id)).unwrap(),
    );
    // objects that are not drive objects are ignored, not counted
    b.plant("vault/attachments/readme.txt", b"hello");
    b.plant("vault/attachments/not-a-uuid.bin", b"hello");

    let (dir, store, counts) = restore_fresh(&fx, "bkt");
    assert_eq!(counts.files, 2, "{counts:?}");
    assert_eq!(counts.files_skipped, 6, "{counts:?}");
    let mut ids: Vec<_> = store
        .file_rows_unindexed()
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    ids.sort();
    let mut want = vec![good.id.clone(), tiny.id.clone()];
    want.sort();
    assert_eq!(ids, want);
    // only the two good blobs are on disk, nothing half-written
    let mut names: Vec<_> = std::fs::read_dir(dir.path().join("attachments"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut want_names = vec![format!("{}.bin", good.id), format!("{}.bin", tiny.id)];
    want_names.sort();
    assert_eq!(names, want_names);
    // rows have no owner and stay out of the lists until the first unlock opens their metadata
    assert!(store.file_rows(None).unwrap().is_empty());
    // the restored blob is the uploaded ciphertext, byte for byte
    assert_eq!(
        std::fs::read(
            dir.path()
                .join("attachments")
                .join(format!("{}.bin", good.id))
        )
        .unwrap(),
        b.raw(&keys::blob(&good.id)).unwrap()
    );
}

#[test]
fn a_blob_is_checked_against_its_own_id_and_length_without_any_key() {
    let fx = Fx::new();
    let a = fx.add(None, "a.bin", &data(2 * MIB + 3, 41));
    fx.run();
    let blob = fx.bucket().raw(&keys::blob(&a.id)).unwrap();
    // exactly what an upload wrote passes; a flipped byte inside the file id of the header does
    // not
    let mut moved = blob.clone();
    moved[8] ^= 1;
    fx.bucket().plant(&keys::blob(&a.id), &moved);
    let (_, _, counts) = restore_fresh(&fx, "bkt");
    assert_eq!((counts.files, counts.files_skipped), (0, 1));
    // a blob that is longer than its three chunks can be is refused as well ...
    let mut longer = blob.clone();
    longer.extend_from_slice(&vec![0u8; MIB]);
    fx.bucket().plant(&keys::blob(&a.id), &longer);
    let (_, _, counts) = restore_fresh(&fx, "bkt");
    assert_eq!((counts.files, counts.files_skipped), (0, 1));
    // ... and one that is too short for them
    fx.bucket()
        .plant(&keys::blob(&a.id), &blob[..blob.len() - 5]);
    let (_, _, counts) = restore_fresh(&fx, "bkt");
    assert_eq!((counts.files, counts.files_skipped), (0, 1));
    // garbage that is not even a header
    fx.bucket().plant(&keys::blob(&a.id), b"short");
    let (_, _, counts) = restore_fresh(&fx, "bkt");
    assert_eq!((counts.files, counts.files_skipped), (0, 1));
    // the untouched blob restores
    fx.bucket().plant(&keys::blob(&a.id), &blob);
    let (_, _, counts) = restore_fresh(&fx, "bkt");
    assert_eq!((counts.files, counts.files_skipped), (1, 0));
}

#[test]
fn a_restored_blob_whose_length_does_not_match_its_sealed_size_stays_hidden() {
    let fx = Fx::new();
    let good = fx.add(None, "good.bin", &data(2000, 43));
    let cut = fx.add(None, "cut.bin", &data(3000, 44));
    fx.run();
    // a one-chunk blob with 50 bytes missing is structurally possible (any length up to a chunk
    // is), so the download accepts it; the sealed size says what the length must be
    let t = fx.bucket().raw(&keys::blob(&cut.id)).unwrap();
    fx.bucket().plant(&keys::blob(&cut.id), &t[..t.len() - 50]);
    let (_, _, counts) = restore_fresh(&fx, "bkt");
    assert_eq!((counts.files, counts.files_skipped), (2, 0));

    fx.app
        .profile_restore_from_backup("Restored", inp("Where from", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    let names: Vec<_> = fx
        .app
        .file_list(None)
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(names, ["good.bin"]);
    assert_eq!(fx.app.file_read(&good.id).unwrap().len(), 2000);
    assert!(fx.app.file_read(&cut.id).is_err());
}

/// A bucket that dies in the middle of a blob download.
struct DiesMidBlob(Arc<MemStore>);

impl ObjectStore for DiesMidBlob {
    fn put(
        &self,
        key: &str,
        body: &[u8],
        cond: &Condition,
    ) -> Result<super::remote::PutOk, RemoteError> {
        self.0.put(key, body, cond)
    }
    fn get(&self, key: &str, max: u64) -> Result<Option<super::remote::Object>, RemoteError> {
        self.0.get(key, max)
    }
    fn head(&self, key: &str) -> Result<Option<super::remote::ObjectMeta>, RemoteError> {
        self.0.head(key)
    }
    fn delete(&self, key: &str) -> Result<(), RemoteError> {
        self.0.delete(key)
    }
    fn list(
        &self,
        prefix: &str,
        token: Option<&str>,
        max_keys: usize,
    ) -> Result<super::remote::ListPage, RemoteError> {
        self.0.list(prefix, token, max_keys)
    }
    fn get_to(
        &self,
        key: &str,
        dest: &mut dyn std::io::Write,
        max_bytes: u64,
    ) -> Result<Option<u64>, RemoteError> {
        let Some(obj) = self.0.get(key, max_bytes)? else {
            return Ok(None);
        };
        let half = obj.bytes.len() / 2;
        dest.write_all(&obj.bytes[..half])
            .map_err(|_| RemoteError::LocalIo)?;
        Err(RemoteError::Server(503))
    }
}

#[test]
fn a_network_error_while_downloading_a_file_stops_the_restore_and_leaves_nothing() {
    let fx = Fx::new();
    let a = fx.add(None, "a.bin", &data(3000, 51));
    fx.run();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("vault.db")).unwrap();
    let mut counts = RestoreCounts::default();

    // the connection dies halfway through the blob
    let dying = DiesMidBlob(fx.bucket());
    let e = files_sync::restore_files(&store, &dying, &mut counts).unwrap_err();
    assert_eq!(e.code(), "remote");
    assert_eq!(counts.files, 0);
    assert!(store.file_rows_unindexed().unwrap().is_empty());
    let left = std::fs::read_dir(dir.path().join("attachments"))
        .unwrap()
        .count();
    assert_eq!(left, 0, "no blob and no partial download is left behind");

    // every request failing (also the listing) is the same
    let remote = fx.bucket();
    remote.fail_all(Some(Op::Get), RemoteError::Server(503));
    let e = files_sync::restore_files(&store, &*remote, &mut counts).unwrap_err();
    assert_eq!(e.code(), "remote");
    assert!(store.file_rows_unindexed().unwrap().is_empty());

    // and once the service answers again the same call completes
    remote.clear_faults();
    files_sync::restore_files(&store, &*remote, &mut counts).unwrap();
    assert_eq!(counts.files, 1);
    assert_eq!(store.file_rows_unindexed().unwrap()[0].id, a.id);
}

#[test]
fn files_added_on_another_device_appear_after_a_restore_but_not_by_a_later_unlock() {
    // device A holds one file; "B" is a restore of it
    let fx = Fx::new();
    fx.add(None, "first.bin", &data(100, 61));
    fx.run();
    fx.app
        .profile_restore_from_backup("B", inp("Where from", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(fx.app.file_list(None).unwrap().len(), 1);

    // A adds a second file and uploads it
    fx.switch_to("Main");
    let second = fx.add(None, "second.bin", &data(100, 62));
    fx.run();
    assert!(drive_keys(&fx.bucket()).contains(&keys::blob(&second.id)));

    // B unlocks and backs up again: the new file is not pulled (the documented limit)
    fx.switch_to("B");
    fx.app.backup_run(None).unwrap();
    let names: Vec<_> = fx
        .app
        .file_list(None)
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(names, ["first.bin"]);

    // a fresh restore does see both
    fx.app
        .profile_restore_from_backup("C", inp("Where from", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
    let mut names: Vec<_> = fx
        .app
        .file_list(None)
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    names.sort();
    assert_eq!(names, ["first.bin", "second.bin"]);
}

// ============================================================ the real S3 client (mock service)

const ACCESS: &str = "AKIAMOCKTESTKEY01";
const MOCK_SECRET: &str = "mock-secret-for-signature-checks-0001";
const REGION: &str = "eu-west-1";
const BUCKET: &str = "passman-mock-bucket";

struct Mock {
    child: Child,
    port: u16,
}

impl Mock {
    /// `None` (after printing why) when the mock cannot be started here.
    fn start(extra: &[&str]) -> Option<Self> {
        if Command::new("python3").arg("--version").output().is_err() {
            println!("SKIPPED: python3 is not installed, so scripts/mock_s3.py cannot run");
            return None;
        }
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/mock_s3.py");
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(&script)
            .args(["--port", "0", "--access-key", ACCESS])
            .args(["--secret-key", MOCK_SECRET])
            .args(["--region", REGION, "--bucket", BUCKET])
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn python3");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout).lines();
            if let Some(Ok(first)) = lines.next() {
                let _ = tx.send(first);
            }
            for _ in lines {} // keep draining so the server never blocks on a full pipe
        });
        let first = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("mock did not start");
        let port = first
            .strip_prefix("MOCK_S3_PORT=")
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("unexpected first line {first:?}"));
        Some(Self { child, port })
    }

    fn input(&self, name: &str) -> BackupTargetInput {
        let mut i = inp(name, BUCKET);
        i.endpoint = Some(format!("http://127.0.0.1:{}", self.port));
        i.region = REGION.into();
        i.prefix = "my/vault".into();
        i.access_key_id = ACCESS.into();
        i.secret_access_key = MOCK_SECRET.into();
        i.path_style = true;
        i
    }

    /// The service answers every request with `status` (403, 500, 503) or, with `None`, works
    /// again.
    fn fail(&self, status: Option<u16>) {
        use std::io::{Read, Write};
        let body = serde_json::json!({ "status": status }).to_string();
        let mut s = std::net::TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(
            s,
            "POST /__mock/fail HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.port,
            body.len()
        )
        .unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).unwrap();
        assert!(
            raw.starts_with("HTTP/1.0 200") || raw.starts_with("HTTP/1.1 200"),
            "{raw}"
        );
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn real_app(dir: &std::path::Path, picker: Arc<TestPicker>) -> AppCore {
    AppCore::open(dir, fast(), picker)
        .unwrap()
        .with_backup_runtime(BackupRuntime::new(
            Arc::new(S3Factory),
            Timing {
                background: false,
                ..Timing::default()
            },
        ))
}

fn client(mock: &Mock) -> S3Store {
    S3Store::new(&TargetConfig::from_input(&mock.input("direct"), true).unwrap()).unwrap()
}

#[test]
fn real_s3_carries_a_multi_chunk_file_as_ciphertext_and_restores_it() {
    let Some(mock) = Mock::start(&[]) else { return };
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let picker = Arc::new(TestPicker::default());
    let app = real_app(dir.path(), picker.clone());
    app.profile_create("Original", PW.into()).unwrap();
    let mut master = mock.input("Mock master");
    master.master = true;
    app.backup_add(master).unwrap();

    let item = app.item_create(login("Passport", "pw")).unwrap().id;
    let big = [MARKER.as_bytes(), &data(2 * MIB + 99, 71)[..]].concat();
    let a = add_file(&app, &picker, src.path(), None, "LEAKNAME-big.pdf", &big);
    let b = add_file(&app, &picker, src.path(), Some(&item), "scan.png", b"tiny");
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(
        (v.health.state, v.health.queued),
        (HealthState::Healthy, 0),
        "{v:?}"
    );

    let direct = client(&mock);
    let local_blob = |id: &str| -> Vec<u8> {
        let profile = app.profile_list().unwrap().profiles.remove(0).id;
        std::fs::read(
            dir.path()
                .join("vaults")
                .join(profile)
                .join("attachments")
                .join(format!("{id}.bin")),
        )
        .unwrap()
    };
    for f in [&a, &b] {
        let blob = direct
            .get(&keys::blob(&f.id), stream::MAX_BLOB_BYTES)
            .unwrap()
            .expect("blob on the service");
        assert_eq!(
            blob.bytes,
            local_blob(&f.id),
            "streamed upload, whole object"
        );
        assert_eq!(count_in(&blob.bytes, MARKER.as_bytes()), 0);
        let meta = direct
            .get(&keys::meta(&f.id), 1 << 20)
            .unwrap()
            .expect("metadata on the service");
        for needle in ["LEAKNAME", "scan.png", "application/pdf", "image/png"] {
            assert_eq!(count_in(&meta.bytes, needle.as_bytes()), 0, "{needle}");
        }
    }

    // rename: only the metadata object changes
    let blob_etag = direct.head(&keys::blob(&a.id)).unwrap().unwrap().etag;
    let meta_etag = direct.head(&keys::meta(&a.id)).unwrap().unwrap().etag;
    app.file_rename(&a.id, "renamed.pdf").unwrap();
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(v.health.state, HealthState::Healthy, "{v:?}");
    assert_eq!(
        direct.head(&keys::blob(&a.id)).unwrap().unwrap().etag,
        blob_etag
    );
    assert_ne!(
        direct.head(&keys::meta(&a.id)).unwrap().unwrap().etag,
        meta_etag
    );

    // restore into a fresh profile of another app folder through the same signed requests
    let dir2 = tempfile::tempdir().unwrap();
    let app2 = real_app(dir2.path(), Arc::new(TestPicker::default()));
    let info = app2
        .profile_restore_from_backup("Restored", mock.input("Where from"))
        .unwrap();
    assert_eq!(info.display_name, "Restored");
    app2.unlock(PW.into()).unwrap();
    let files = app2.file_list(None).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "renamed.pdf");
    assert_eq!(&**app2.file_read(&a.id).unwrap(), &big[..]);
    let view = app2.item_get(&item).unwrap();
    assert_eq!(view.attachments.len(), 1);
    assert_eq!(&**app2.file_read(&b.id).unwrap(), b"tiny");

    // delete: both objects leave the service
    app.file_delete(&a.id, PW.into()).unwrap();
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(v.health.state, HealthState::Healthy, "{v:?}");
    assert!(direct.head(&keys::blob(&a.id)).unwrap().is_none());
    assert!(direct.head(&keys::meta(&a.id)).unwrap().is_none());
    assert!(direct.head(&keys::blob(&b.id)).unwrap().is_some());
}

#[test]
fn real_s3_failures_keep_the_upload_queued_until_the_service_works_again() {
    let Some(mock) = Mock::start(&[]) else { return };
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let picker = Arc::new(TestPicker::default());
    let app = real_app(dir.path(), picker.clone());
    app.profile_create("Original", PW.into()).unwrap();
    let mut master = mock.input("Mock master");
    master.master = true;
    app.backup_add(master).unwrap();
    app.backup_run(None).unwrap();

    let a = add_file(&app, &picker, src.path(), None, "a.bin", &data(MIB + 5, 81));
    mock.fail(Some(503));
    let v = app.backup_run(None).unwrap().remove(0);
    assert_ne!(v.health.state, HealthState::Healthy);
    assert!(v.health.queued >= 2, "{v:?}");
    mock.fail(None);
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(
        (v.health.state, v.health.queued),
        (HealthState::Healthy, 0),
        "{v:?}"
    );
    assert!(client(&mock)
        .head(&keys::blob(&a.id))
        .unwrap()
        .is_some_and(|m| m.size == stream::blob_len(MIB as u64 + 5)));
}

#[test]
fn a_service_that_refuses_large_objects_leaves_the_file_local_and_queued() {
    let Some(mock) = Mock::start(&["--max-object-mib", "1"]) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let picker = Arc::new(TestPicker::default());
    let app = real_app(dir.path(), picker.clone());
    app.profile_create("Original", PW.into()).unwrap();
    let mut master = mock.input("Mock master");
    master.master = true;
    app.backup_add(master).unwrap();
    app.backup_run(None).unwrap();

    let big = data(2 * MIB, 91);
    let a = add_file(&app, &picker, src.path(), None, "big.bin", &big);
    let v = app.backup_run(None).unwrap().remove(0);
    assert_ne!(v.health.state, HealthState::Healthy, "{v:?}");
    assert!(v.health.queued >= 2, "{v:?}");
    assert!(v.health.last_error.is_some());
    assert!(client(&mock).head(&keys::blob(&a.id)).unwrap().is_none());
    // the file itself is untouched and readable
    assert_eq!(&**app.file_read(&a.id).unwrap(), &big[..]);
}
