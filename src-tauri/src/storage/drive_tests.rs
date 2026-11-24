//! Behavioural tests of the encrypted drive through [`AppCore`] with a real temp vault: the
//! Files section, item attachments, the size limit, the re-prompt rules, the on-disk secrecy
//! guarantees and the failure paths. Everything runs in temp folders.

use super::app::AppCore;
use super::drive::{self, MSG_DAMAGED_FILE, MSG_MISSING_BLOB};
use super::files::TestPicker;
use super::model::{ItemDraft, ItemType};
use super::session::{SessionConfig, EDIT_TOKEN_TTL};
use super::StorageError;
use crate::crypto::KdfParams;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PW: &str = "correct horse battery staple";
const WRONG: &str = "definitely not the password";
const MIB: usize = 1024 * 1024;

fn fast() -> SessionConfig {
    SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: EDIT_TOKEN_TTL,
    }
}

struct Fx {
    /// The app's data folder.
    dir: tempfile::TempDir,
    /// Where the "files on the person's disk" live (outside the data folder).
    src: tempfile::TempDir,
    picker: Arc<TestPicker>,
    app: AppCore,
}

impl Fx {
    /// An app with one unlocked profile.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let picker = Arc::new(TestPicker::default());
        let app = AppCore::open(dir.path(), fast(), picker.clone()).unwrap();
        app.profile_create("Alpha", PW.into()).unwrap();
        Self {
            dir,
            src: tempfile::tempdir().unwrap(),
            picker,
            app,
        }
    }

    /// Quit and start the app again, then unlock.
    fn restart(&mut self) {
        self.app.lock();
        self.app = AppCore::open(self.dir.path(), fast(), self.picker.clone()).unwrap();
        let id = self.app.profile_list().unwrap().profiles[0].id.clone();
        self.app.profile_select(&id).unwrap();
        self.app.unlock(PW.into()).unwrap();
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.src.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// Queue the next pick and add; the entries of the added files.
    fn add(&self, item: Option<&str>, paths: &[&Path]) -> Vec<super::FileEntry> {
        self.picker
            .queue_files(paths.iter().map(|p| p.to_path_buf()).collect());
        self.app.file_add(item).unwrap().unwrap()
    }

    fn add_one(&self, item: Option<&str>, name: &str, bytes: &[u8]) -> super::FileEntry {
        let p = self.write(name, bytes);
        self.add(item, &[&p]).remove(0)
    }

    fn vault_dir(&self) -> PathBuf {
        let id = self.app.profile_list().unwrap().profiles[0].id.clone();
        self.dir.path().join("vaults").join(id)
    }

    fn attachments_dir(&self) -> PathBuf {
        self.vault_dir().join("attachments")
    }

    fn blob_of(&self, id: &str) -> PathBuf {
        self.attachments_dir().join(format!("{id}.bin"))
    }

    fn item(&self, title: &str) -> String {
        self.app.item_create(note(title)).unwrap().id
    }

    fn file_count(&self) -> u64 {
        self.app
            .test_session()
            .test_vault(|v| v.store().file_count().unwrap())
    }

    fn row(&self, id: &str) -> super::drive::FileRow {
        self.app
            .test_session()
            .test_vault(|v| v.store().file_row(id).unwrap().unwrap())
    }
}

fn note(title: &str) -> ItemDraft {
    ItemDraft {
        item_type: ItemType::SecureNote,
        title: title.into(),
        favorite: false,
        tags: vec![],
        folder: None,
        fields: BTreeMap::from([("body".to_string(), "text".to_string())]),
        custom_fields: vec![],
    }
}

/// Deterministic, non-repeating test data.
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

fn code<T: std::fmt::Debug>(r: super::Result<T>) -> &'static str {
    r.expect_err("expected an error").code()
}

fn message<T: std::fmt::Debug>(r: super::Result<T>) -> String {
    r.expect_err("expected an error").to_string()
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

/// Every byte the app keeps for this profile on disk: database, write-ahead log, shared memory,
/// every blob and every other file in the profile folder.
fn all_profile_bytes(fx: &Fx) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                walk(&p, out);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(&fx.vault_dir(), &mut out);
    out
}

// ============================================================ round trips

#[test]
fn a_standalone_file_round_trips_through_add_list_rename_read_save_delete() {
    let fx = Fx::new();
    let content = data(300_000, 1);
    let e = fx.add_one(None, "Passport scan.PDF", &content);
    assert_eq!(e.name, "Passport scan.PDF");
    assert_eq!(e.size, 300_000);
    assert_eq!(e.mime.as_deref(), Some("application/pdf"));
    assert_eq!(e.item_id, None);
    assert!(e.added_at > 1_700_000_000_000);
    assert!(crate::storage::uuid::is_valid(&e.id));

    assert_eq!(fx.app.file_list(None).unwrap(), vec![e.clone()]);
    assert_eq!(&*fx.app.file_read(&e.id).unwrap(), &content[..]);

    let renamed = fx.app.file_rename(&e.id, "  scan.txt ").unwrap();
    assert_eq!(renamed.name, "scan.txt");
    assert_eq!(
        renamed.mime.as_deref(),
        Some("text/plain"),
        "the type follows the name"
    );
    assert_eq!(renamed.size, 300_000);
    assert_eq!(renamed.added_at, e.added_at);
    assert_eq!(fx.app.file_list(None).unwrap(), vec![renamed.clone()]);
    assert_eq!(
        &*fx.app.file_read(&e.id).unwrap(),
        &content[..],
        "a rename leaves the blob alone"
    );

    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("copy.txt");
    fx.picker.set_save(Some(dest.clone()));
    let saved = fx.app.file_save_copy(&e.id, PW.into()).unwrap().unwrap();
    assert_eq!(saved.path, dest.to_string_lossy());
    assert_eq!(std::fs::read(&dest).unwrap(), content);
    assert_eq!(
        fx.picker.suggested_names(),
        vec!["scan.txt".to_string()],
        "the dialog is pre-filled with the file's name"
    );

    assert!(fx.blob_of(&e.id).is_file());
    fx.app.file_delete(&e.id, PW.into()).unwrap();
    assert!(fx.app.file_list(None).unwrap().is_empty());
    assert!(
        !fx.blob_of(&e.id).exists(),
        "the blob is removed with the row"
    );
    assert_eq!(code(fx.app.file_read(&e.id)), "not_found");
    assert_eq!(code(fx.app.file_rename(&e.id, "x")), "not_found");
    assert_eq!(code(fx.app.file_delete(&e.id, PW.into())), "not_found");
}

#[test]
fn sizes_around_the_chunk_boundaries_round_trip_through_the_app() {
    let fx = Fx::new();
    for (i, size) in [0usize, 1, MIB - 1, MIB, MIB + 1, 2 * MIB + 5]
        .into_iter()
        .enumerate()
    {
        let content = data(size, 10 + i as u64);
        let e = fx.add_one(None, &format!("f{i}.bin"), &content);
        assert_eq!(e.size, size as u64);
        assert_eq!(
            &*fx.app.file_read(&e.id).unwrap(),
            &content[..],
            "size {size}"
        );
        let blob_len = std::fs::metadata(fx.blob_of(&e.id)).unwrap().len();
        assert_eq!(blob_len, crate::crypto::stream::blob_len(size as u64));
    }
}

#[test]
fn a_file_just_under_the_limit_round_trips() {
    let fx = Fx::new();
    let content = data(49_900_000, 77);
    let e = fx.add_one(None, "big.zip", &content);
    assert_eq!(e.size, 49_900_000);
    assert_eq!(e.mime.as_deref(), Some("application/zip"));
    let back = fx.app.file_read(&e.id).unwrap();
    assert!(*back == content[..], "49.9 MB comes back identical");
}

#[test]
fn several_files_are_added_in_one_call_and_listed_newest_first() {
    let fx = Fx::new();
    let a = fx.write("a.txt", b"aaa");
    let b = fx.write("b.png", &data(10, 3));
    let c = fx.write("c", b"");
    let added = fx.add(None, &[&a, &b, &c]);
    assert_eq!(
        added.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["a.txt", "b.png", "c"]
    );
    assert_eq!(added[2].mime, None, "an unknown extension has no type");
    assert_eq!(added[2].size, 0);
    let ids: std::collections::HashSet<_> = added.iter().map(|f| f.id.clone()).collect();
    assert_eq!(ids.len(), 3);
    let listed = fx.app.file_list(None).unwrap();
    assert_eq!(listed.len(), 3);
    assert!(listed.windows(2).all(|w| w[0].added_at >= w[1].added_at));
    // a later call adds to the list; a name may repeat (the id is the identity)
    let again = fx.add_one(None, "a.txt", b"other");
    assert_ne!(again.id, added[0].id);
    assert_eq!(fx.app.file_list(None).unwrap().len(), 4);
}

// ============================================================ item attachments

#[test]
fn attachments_belong_to_their_item_and_show_in_the_item_view() {
    let fx = Fx::new();
    let item = fx.item("Passport");
    let other = fx.item("Other");
    let scan = fx.add_one(Some(&item), "scan.png", &data(2000, 4));
    let doc = fx.add_one(Some(&item), "notes.bin", &data(50, 5));
    let loose = fx.add_one(None, "loose.txt", b"x");

    let view = fx.app.item_get(&item).unwrap();
    let mut atts: Vec<_> = view
        .attachments
        .iter()
        .map(|a| (a.id.clone(), a.name.clone(), a.mime.clone(), a.size))
        .collect();
    atts.sort();
    let mut want = vec![
        (
            scan.id.clone(),
            "scan.png".to_string(),
            "image/png".to_string(),
            2000,
        ),
        (
            doc.id.clone(),
            "notes.bin".to_string(),
            "application/octet-stream".to_string(),
            50,
        ),
    ];
    want.sort();
    assert_eq!(atts, want);
    assert!(fx.app.item_get(&other).unwrap().attachments.is_empty());

    let listed: Vec<_> = fx
        .app
        .file_list(Some(&item))
        .unwrap()
        .into_iter()
        .map(|f| f.id)
        .collect();
    assert_eq!(listed.len(), 2);
    assert!(listed.contains(&scan.id) && listed.contains(&doc.id));
    assert_eq!(fx.app.file_list(None).unwrap(), vec![loose]);
    assert_eq!(fx.app.file_list(Some(&other)).unwrap(), vec![]);
    assert_eq!(scan.item_id.as_deref(), Some(item.as_str()));
}

#[test]
fn files_of_an_item_follow_the_trash_rules() {
    let fx = Fx::new();
    let item = fx.item("Passport");
    let f = fx.add_one(Some(&item), "scan.png", b"png-ish");
    let p = fx.write("more.txt", b"more");

    fx.app.item_trash(&item).unwrap();
    // the trashed item's files are not in the Files section and cannot be reached
    assert!(fx.app.file_list(None).unwrap().is_empty());
    assert_eq!(code(fx.app.file_list(Some(&item))), "in_trash");
    assert_eq!(code(fx.app.file_read(&f.id)), "in_trash");
    assert_eq!(code(fx.app.file_rename(&f.id, "x.png")), "in_trash");
    assert_eq!(code(fx.app.file_delete(&f.id, PW.into())), "in_trash");
    fx.picker.queue_files(vec![p.clone()]);
    assert_eq!(code(fx.app.file_add(Some(&item))), "in_trash");
    assert!(
        fx.blob_of(&f.id).is_file(),
        "an item in the trash keeps its files"
    );
    // restoring brings everything back
    fx.app.item_restore(&item).unwrap();
    assert_eq!(&*fx.app.file_read(&f.id).unwrap(), b"png-ish");
    assert_eq!(fx.app.item_get(&item).unwrap().attachments.len(), 1);
}

#[test]
fn adding_to_a_missing_item_or_a_bad_id_is_refused_before_any_dialog() {
    let fx = Fx::new();
    let p = fx.write("a.txt", b"a");
    fx.picker.queue_files(vec![p]);
    let ghost = crate::storage::uuid::new_v4().unwrap();
    assert_eq!(code(fx.app.file_add(Some(&ghost))), "not_found");
    assert_eq!(code(fx.app.file_add(Some("not-an-id"))), "invalid_input");
    // the queued pick was never consumed: no dialog was opened for a doomed request
    assert_eq!(fx.app.file_add(None).unwrap().unwrap().len(), 1);
    assert_eq!(code(fx.app.file_list(Some(&ghost))), "not_found");
    assert_eq!(code(fx.app.file_read("nope")), "invalid_input");
}

#[test]
fn permanent_deletion_of_an_item_removes_its_files_and_nothing_else() {
    let fx = Fx::new();
    let item = fx.item("Passport");
    let keep = fx.item("Keep");
    let a = fx.add_one(Some(&item), "a.png", b"aaa");
    let b = fx.add_one(Some(&item), "b.png", b"bbb");
    let kept = fx.add_one(Some(&keep), "k.png", b"kkk");
    let loose = fx.add_one(None, "l.txt", b"lll");
    fx.app.item_trash(&item).unwrap();
    fx.app.item_purge(&item, PW.into(), "CONFIRM").unwrap();

    for f in [&a, &b] {
        assert!(!fx.blob_of(&f.id).exists(), "blob of an erased item's file");
        assert_eq!(code(fx.app.file_read(&f.id)), "not_found");
    }
    assert!(fx.blob_of(&kept.id).is_file());
    assert!(fx.blob_of(&loose.id).is_file());
    assert_eq!(fx.file_count(), 2, "only the other files remain");
    assert_eq!(&*fx.app.file_read(&kept.id).unwrap(), b"kkk");
    assert_eq!(&*fx.app.file_read(&loose.id).unwrap(), b"lll");
}

#[test]
fn cloning_an_item_does_not_copy_its_files() {
    let fx = Fx::new();
    let item = fx.item("Passport");
    fx.add_one(Some(&item), "scan.png", b"png");
    let copy = fx.app.item_clone(&item, PW.into()).unwrap();
    assert!(fx.app.item_get(&copy.id).unwrap().attachments.is_empty());
    assert_eq!(fx.app.item_get(&item).unwrap().attachments.len(), 1);
    assert_eq!(fx.app.file_list(Some(&copy.id)).unwrap(), vec![]);
}

// ============================================================ limits and refusals

#[test]
fn the_limit_is_52_428_800_bytes_checked_before_anything_is_written() {
    let fx = Fx::new();
    let ok = fx.write("fits.bin", b"small");
    // sparse files: the size is real, the content is not stored
    let over = fx.src.path().join("Huge Video.mp4");
    std::fs::File::create(&over)
        .unwrap()
        .set_len(52_428_801)
        .unwrap();
    let exact = fx.src.path().join("exact.bin");
    std::fs::File::create(&exact)
        .unwrap()
        .set_len(52_428_800)
        .unwrap();

    // exactly the limit passes the up-front check; one byte more does not
    assert!(drive::check_source(&exact).is_ok());
    assert_eq!(
        drive::check_source(&over).unwrap_err().to_string(),
        "Huge Video.mp4 is larger than 50 MB."
    );

    // one oversized file rejects the whole call, even when it is last
    fx.picker.queue_files(vec![ok.clone(), over.clone()]);
    let e = fx.app.file_add(None).unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    assert_eq!(e.to_string(), "Huge Video.mp4 is larger than 50 MB.");
    assert!(
        fx.app.file_list(None).unwrap().is_empty(),
        "nothing was added"
    );
    let leftovers = std::fs::read_dir(fx.attachments_dir())
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "nothing was written to disk");
    assert_eq!(fx.file_count(), 0);
}

#[test]
fn bad_picks_are_refused_without_side_effects() {
    let fx = Fx::new();
    let ok = fx.write("ok.txt", b"ok");
    // a missing file among good ones
    fx.picker
        .queue_files(vec![ok.clone(), fx.src.path().join("gone.txt")]);
    let e = fx.app.file_add(None).unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    assert_eq!(
        e.to_string(),
        "gone.txt could not be read. Check that it still exists."
    );
    // a folder
    fx.picker.queue_files(vec![fx.src.path().to_path_buf()]);
    assert_eq!(code(fx.app.file_add(None)), "invalid_input");
    // a file from the app's own data folder
    fx.picker.queue_files(vec![fx.vault_dir().join("vault.db")]);
    assert_eq!(code(fx.app.file_add(None)), "invalid_input");
    // too many at once
    fx.picker.queue_files(vec![ok.clone(); 101]);
    assert_eq!(code(fx.app.file_add(None)), "invalid_input");
    assert!(fx.app.file_list(None).unwrap().is_empty());
    assert_eq!(fx.file_count(), 0);
}

#[test]
fn cancelling_the_dialog_adds_nothing() {
    let fx = Fx::new();
    // nothing queued = the person cancelled
    assert_eq!(fx.app.file_add(None).unwrap(), None);
    fx.picker.queue_files(vec![]);
    assert_eq!(
        fx.app.file_add(None).unwrap(),
        None,
        "an empty selection is a cancel"
    );
    assert!(fx.app.file_list(None).unwrap().is_empty());
    let f = fx.add_one(None, "a.txt", b"a");
    fx.picker.set_save(None);
    assert_eq!(fx.app.file_save_copy(&f.id, PW.into()).unwrap(), None);
}

#[cfg(unix)]
#[test]
fn a_failure_half_way_removes_the_files_this_call_already_added() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new();
    let first = fx.write("first.txt", b"first");
    let second = fx.write("second.txt", b"second");
    // readable by stat (so the up-front check passes) but not by open
    std::fs::set_permissions(&second, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&second).is_ok() {
        return; // running with privileges that ignore file modes: this case cannot be made
    }
    fx.picker.queue_files(vec![first.clone(), second.clone()]);
    let e = fx.app.file_add(None).unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    assert!(e.to_string().starts_with("second.txt could not be read"));
    assert!(
        fx.app.file_list(None).unwrap().is_empty(),
        "the first file was rolled back"
    );
    assert_eq!(fx.file_count(), 0);
    let names: Vec<_> = std::fs::read_dir(fx.attachments_dir())
        .map(|d| d.map(|e| e.unwrap().file_name()).collect())
        .unwrap_or_default();
    assert!(
        names.is_empty(),
        "no blob or temporary file is left: {names:?}"
    );
}

// ============================================================ locked, and the re-prompt rules

#[test]
fn everything_answers_locked_when_the_vault_is_locked() {
    let fx = Fx::new();
    let f = fx.add_one(None, "a.txt", b"a");
    let item = fx.item("x");
    fx.app.lock();
    assert_eq!(code(fx.app.file_list(None)), "locked");
    assert_eq!(code(fx.app.file_list(Some(&item))), "locked");
    let p = fx.write("n.txt", b"n");
    fx.picker.queue_files(vec![p]);
    assert_eq!(code(fx.app.file_add(None)), "locked");
    assert_eq!(code(fx.app.file_rename(&f.id, "b.txt")), "locked");
    assert_eq!(code(fx.app.file_read(&f.id)), "locked");
    assert_eq!(code(fx.app.file_delete(&f.id, PW.into())), "locked");
    assert_eq!(code(fx.app.file_save_copy(&f.id, PW.into())), "locked");
}

#[test]
fn nothing_selected_means_locked_too() {
    let dir = tempfile::tempdir().unwrap();
    let app = AppCore::open(dir.path(), fast(), Arc::new(TestPicker::default())).unwrap();
    assert_eq!(code(app.file_list(None)), "locked");
    assert_eq!(code(app.file_add(None)), "locked");
}

#[test]
fn delete_and_save_copy_need_the_master_password() {
    let fx = Fx::new();
    let f = fx.add_one(None, "a.txt", b"secret bytes");
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("a.txt");
    fx.picker.set_save(Some(dest.clone()));

    assert_eq!(
        code(fx.app.file_delete(&f.id, WRONG.into())),
        "wrong_password"
    );
    assert_eq!(
        fx.app.file_list(None).unwrap().len(),
        1,
        "a wrong password deletes nothing"
    );

    assert_eq!(
        code(fx.app.file_save_copy(&f.id, WRONG.into())),
        "wrong_password"
    );
    assert!(
        fx.picker.suggested_names().is_empty(),
        "a wrong password never opens the dialog"
    );
    assert!(!dest.exists());

    // preview and rename need none (the contract: only delete and save a copy re-prompt)
    assert_eq!(&*fx.app.file_read(&f.id).unwrap(), b"secret bytes");
    fx.app.file_rename(&f.id, "b.txt").unwrap();

    fx.app.file_save_copy(&f.id, PW.into()).unwrap().unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"secret bytes");
    fx.app.file_delete(&f.id, PW.into()).unwrap();
    assert!(fx.app.file_list(None).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn files_on_disk_and_saved_copies_are_private_to_the_owner() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new();
    let f = fx.add_one(None, "a.txt", &data(3 * MIB, 9));
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&fx.attachments_dir()), 0o700);
    assert_eq!(mode(&fx.blob_of(&f.id)), 0o600);
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("copy.bin");
    fx.picker.set_save(Some(dest.clone()));
    fx.app.file_save_copy(&f.id, PW.into()).unwrap().unwrap();
    assert_eq!(mode(&dest), 0o600, "a saved decrypted copy is owner-only");
    assert_eq!(std::fs::read(&dest).unwrap(), data(3 * MIB, 9));
    // no partial or temporary file is left next to the copy
    let names: Vec<_> = std::fs::read_dir(out.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["copy.bin"]);
}

#[test]
fn a_save_destination_inside_the_data_folder_is_refused() {
    let fx = Fx::new();
    let f = fx.add_one(None, "a.txt", b"a");
    fx.picker
        .set_save(Some(fx.dir.path().join("plain-copy.txt")));
    assert_eq!(
        code(fx.app.file_save_copy(&f.id, PW.into())),
        "invalid_input"
    );
    assert!(!fx.dir.path().join("plain-copy.txt").exists());
}

// ============================================================ secrecy on disk

#[test]
fn no_name_type_or_content_is_readable_in_any_file_of_the_profile() {
    let fx = Fx::new();
    let item = fx.item("Item for attachments");
    let marker = "PLAINMARKER-5c1e8b-DO-NOT-LEAK";
    let content = [
        marker.as_bytes(),
        &data(2_300_000, 12)[..],
        marker.as_bytes(),
    ]
    .concat();
    let a = fx.add_one(None, "LEAKNAME-tax-return-2026.pdf", &content);
    let b = fx.add_one(Some(&item), "LEAKNAME-passport.png", marker.as_bytes());
    fx.app.file_rename(&a.id, "LEAKRENAMED-final.pdf").unwrap();
    // once with the live write-ahead log, once after a lock folded it back
    for phase in 0..2 {
        for (path, bytes) in all_profile_bytes(&fx) {
            for needle in [
                marker,
                "LEAKNAME",
                "LEAKRENAMED",
                "tax-return",
                "passport",
                "application/pdf",
                "image/png",
            ] {
                assert_eq!(
                    count_in(&bytes, needle.as_bytes()),
                    0,
                    "{needle:?} found in {path:?} (phase {phase})"
                );
            }
        }
        if phase == 0 {
            fx.app.lock();
            fx.app.unlock(PW.into()).unwrap();
        }
    }
    // the files still read fine afterwards
    let back = fx.app.file_read(&a.id).unwrap();
    assert_eq!(count_in(&back, marker.as_bytes()), 2);
    assert_eq!(&*fx.app.file_read(&b.id).unwrap(), marker.as_bytes());
    // what is in the clear: the ids, the ciphertext size and the owner
    let row = fx.row(&b.id);
    assert_eq!(row.item_id.as_deref(), Some(item.as_str()));
    assert_eq!(
        row.blob_len,
        crate::crypto::stream::blob_len(marker.len() as u64)
    );
}

#[test]
fn each_file_has_its_own_key_and_a_blob_is_bound_to_its_id() {
    let fx = Fx::new();
    let same = data(5000, 21);
    let a = fx.add_one(None, "a.bin", &same);
    let b = fx.add_one(None, "b.bin", &same);
    let blob_a = std::fs::read(fx.blob_of(&a.id)).unwrap();
    let blob_b = std::fs::read(fx.blob_of(&b.id)).unwrap();
    assert_eq!(blob_a.len(), blob_b.len());
    assert_ne!(blob_a, blob_b, "identical content gives unrelated blobs");
    // moving one blob under another file's name must not decrypt (the id is in the header and
    // in every chunk's AAD, and the key differs)
    std::fs::write(fx.blob_of(&b.id), &blob_a).unwrap();
    assert_eq!(message(fx.app.file_read(&b.id)), MSG_DAMAGED_FILE);
    assert_eq!(code(fx.app.file_read(&b.id)), "crypto");
    // the original is untouched
    assert_eq!(&*fx.app.file_read(&a.id).unwrap(), &same[..]);
}

// ============================================================ damaged data

#[test]
fn a_damaged_blob_reports_a_fixed_sentence_and_leaves_no_partial_copy() {
    let fx = Fx::new();
    let content = data(3 * MIB + 100, 31);
    let f = fx.add_one(None, "a.bin", &content);
    let path = fx.blob_of(&f.id);
    let original = std::fs::read(&path).unwrap();
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("copy.bin");
    fx.picker.set_save(Some(dest.clone()));

    // one bit flipped in the last chunk, in a middle chunk, in the header
    for pos in [original.len() - 20, original.len() / 2, 10] {
        let mut bad = original.clone();
        bad[pos] ^= 1;
        std::fs::write(&path, &bad).unwrap();
        let e = fx.app.file_read(&f.id).unwrap_err();
        assert_eq!(
            e,
            StorageError::Crypto(MSG_DAMAGED_FILE.into()),
            "bit at {pos}"
        );
        let e = fx.app.file_save_copy(&f.id, PW.into()).unwrap_err();
        assert_eq!(e.code(), "crypto");
        assert!(!dest.exists(), "no copy of damaged data is kept");
        assert_eq!(
            std::fs::read_dir(out.path()).unwrap().count(),
            0,
            "no .part file left"
        );
    }
    // shorter (length no longer matches the row), by a byte and by a whole chunk
    for cut in [1usize, MIB + 40] {
        std::fs::write(&path, &original[..original.len() - cut]).unwrap();
        assert_eq!(message(fx.app.file_read(&f.id)), MSG_DAMAGED_FILE);
    }
    // longer
    let mut longer = original.clone();
    longer.extend_from_slice(&[0u8; 100]);
    std::fs::write(&path, &longer).unwrap();
    assert_eq!(message(fx.app.file_read(&f.id)), MSG_DAMAGED_FILE);
    // restoring the bytes makes it readable again (the failed reads changed nothing)
    std::fs::write(&path, &original).unwrap();
    assert_eq!(&*fx.app.file_read(&f.id).unwrap(), &content[..]);
}

#[test]
fn a_missing_blob_is_a_clear_error_and_the_file_can_still_be_deleted() {
    let fx = Fx::new();
    let f = fx.add_one(None, "a.bin", b"abc");
    std::fs::remove_file(fx.blob_of(&f.id)).unwrap();
    assert_eq!(message(fx.app.file_read(&f.id)), MSG_MISSING_BLOB);
    assert_eq!(code(fx.app.file_read(&f.id)), "storage");
    // the metadata is intact, so the file is still listed and renamable
    assert_eq!(fx.app.file_list(None).unwrap().len(), 1);
    fx.app.file_rename(&f.id, "b.bin").unwrap();
    fx.app.file_delete(&f.id, PW.into()).unwrap();
    assert!(fx.app.file_list(None).unwrap().is_empty());
}

#[test]
fn damaged_metadata_hides_that_file_only() {
    let fx = Fx::new();
    let good = fx.add_one(None, "good.txt", b"good");
    let bad = fx.add_one(None, "bad.txt", b"bad");
    fx.app.test_session().test_vault(|v| {
        v.store_mut()
            .conn()
            .execute(
                "UPDATE files SET meta = x'0102' WHERE id = ?1",
                [bad.id.as_str()],
            )
            .unwrap();
    });
    assert_eq!(fx.app.file_list(None).unwrap(), vec![good.clone()]);
    assert_eq!(code(fx.app.file_read(&bad.id)), "crypto");
    assert_eq!(code(fx.app.file_rename(&bad.id, "x")), "crypto");
    assert_eq!(&*fx.app.file_read(&good.id).unwrap(), b"good");
    // a damaged file can still be deleted (the owner column is plain)
    fx.app.file_delete(&bad.id, PW.into()).unwrap();
    assert!(!fx.blob_of(&bad.id).exists());
}

#[test]
fn metadata_swapped_between_files_is_detected() {
    let fx = Fx::new();
    let a = fx.add_one(None, "a.txt", b"aaaa");
    let b = fx.add_one(None, "b.txt", b"bbbbbbbb");
    fx.app.test_session().test_vault(|v| {
        let c = v.store().conn();
        let meta_a: Vec<u8> = c
            .query_row("SELECT meta FROM files WHERE id = ?1", [&a.id], |r| {
                r.get(0)
            })
            .unwrap();
        c.execute(
            "UPDATE files SET meta = ?2 WHERE id = ?1",
            rusqlite::params![&b.id, meta_a],
        )
        .unwrap();
    });
    // the envelope is bound to its file id: a copied-over name does not authenticate
    assert_eq!(code(fx.app.file_read(&b.id)), "crypto");
    assert_eq!(fx.app.file_list(None).unwrap().len(), 1);
}

// ============================================================ lifecycle

#[test]
fn files_survive_lock_unlock_and_a_restart() {
    let mut fx = Fx::new();
    let item = fx.item("Passport");
    let content = data(1_500_000, 41);
    let f = fx.add_one(Some(&item), "scan.png", &content);
    let g = fx.add_one(None, "loose.txt", b"loose");
    fx.app.lock();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(&*fx.app.file_read(&f.id).unwrap(), &content[..]);
    fx.restart();
    assert_eq!(&*fx.app.file_read(&f.id).unwrap(), &content[..]);
    assert_eq!(fx.app.file_list(None).unwrap(), vec![g.clone()]);
    assert_eq!(fx.app.item_get(&item).unwrap().attachments.len(), 1);
    // and a rename made before the restart is still there
    fx.app.file_rename(&g.id, "renamed.txt").unwrap();
    fx.restart();
    assert_eq!(fx.app.file_list(None).unwrap()[0].name, "renamed.txt");
}

#[test]
fn a_new_master_password_keeps_the_files_readable() {
    let fx = Fx::new();
    let f = fx.add_one(None, "a.txt", b"still here");
    fx.app
        .change_password(PW.into(), "a brand new master password".into())
        .unwrap();
    fx.app.lock();
    fx.app.unlock("a brand new master password".into()).unwrap();
    assert_eq!(&*fx.app.file_read(&f.id).unwrap(), b"still here");
}

#[test]
fn locking_before_the_encryption_starts_aborts_it_and_leaves_nothing() {
    let fx = Fx::new();
    let src = fx.write("big.bin", &data(3 * MIB, 51));
    let session = fx.app.test_session();
    // step 1 happens while unlocked
    let ticket = session.test_vault(|v| v.file_add_prepare(None)).unwrap();
    // the vault locks before the (slow) encryption starts
    fx.app.lock();
    let e = drive::stage(ticket, &src).err().unwrap();
    assert_eq!(e, StorageError::Locked);
    let left = std::fs::read_dir(fx.attachments_dir())
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(left, 0, "no temporary file is left");
    fx.app.unlock(PW.into()).unwrap();
    assert!(fx.app.file_list(None).unwrap().is_empty());
}

#[test]
fn a_blob_staged_for_an_earlier_unlock_is_not_committed() {
    let fx = Fx::new();
    let src = fx.write("a.bin", b"data");
    let session = fx.app.test_session();
    let ticket = session.test_vault(|v| v.file_add_prepare(None)).unwrap();
    let staged = drive::stage(ticket, &src).unwrap();
    let tmp_name = format!("{}.bin.tmp", staged.id);
    assert!(fx.attachments_dir().join(&tmp_name).is_file());
    // lock and unlock again: a new vault session, a new cancellation flag
    fx.app.lock();
    fx.app.unlock(PW.into()).unwrap();
    let e = session
        .test_vault(|v| v.file_add_commit(staged))
        .unwrap_err();
    assert_eq!(e, StorageError::Locked);
    assert!(
        !fx.attachments_dir().join(&tmp_name).exists(),
        "the refused blob is deleted"
    );
    assert!(fx.app.file_list(None).unwrap().is_empty());
}

#[test]
fn the_owner_leaving_during_an_add_is_noticed_at_the_commit() {
    let fx = Fx::new();
    let item = fx.item("Passport");
    let src = fx.write("a.bin", b"data");
    let session = fx.app.test_session();
    let ticket = session
        .test_vault(|v| v.file_add_prepare(Some(&item)))
        .unwrap();
    let staged = drive::stage(ticket, &src).unwrap();
    fx.app.item_trash(&item).unwrap();
    let e = session
        .test_vault(|v| v.file_add_commit(staged))
        .unwrap_err();
    assert_eq!(e.code(), "in_trash");
    assert!(std::fs::read_dir(fx.attachments_dir())
        .unwrap()
        .next()
        .is_none());
    assert_eq!(fx.file_count(), 0);
}

#[test]
fn leftovers_are_swept_at_unlock_and_foreign_files_are_left_alone() {
    let mut fx = Fx::new();
    let keep = fx.add_one(None, "keep.txt", b"keep");
    let dir = fx.attachments_dir();
    let orphan_id = crate::storage::uuid::new_v4().unwrap();
    std::fs::write(dir.join(format!("{orphan_id}.bin")), b"orphan").unwrap();
    std::fs::write(dir.join(format!("{orphan_id}.bin.tmp")), b"half").unwrap();
    std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
    std::fs::write(dir.join("not-a-uuid.bin"), b"mine too").unwrap();
    fx.restart();
    assert!(!dir.join(format!("{orphan_id}.bin")).exists());
    assert!(!dir.join(format!("{orphan_id}.bin.tmp")).exists());
    assert!(
        dir.join("notes.txt").exists(),
        "unknown names are never touched"
    );
    assert!(dir.join("not-a-uuid.bin").exists());
    assert!(fx.blob_of(&keep.id).is_file());
    assert_eq!(&*fx.app.file_read(&keep.id).unwrap(), b"keep");
}

#[test]
fn rename_rules_through_the_app() {
    let fx = Fx::new();
    let f = fx.add_one(None, "a.png", b"x");
    for bad in [
        "",
        "   ",
        "a/b",
        "a\\b",
        "x\ny",
        &"n".repeat(201),
        "..",
        "evil\u{202E}gnp.exe",
    ] {
        assert_eq!(
            code(fx.app.file_rename(&f.id, bad)),
            "invalid_input",
            "{bad:?}"
        );
    }
    assert_eq!(
        fx.app.file_list(None).unwrap()[0].name,
        "a.png",
        "a refused rename changes nothing"
    );
    let long = "é".repeat(200);
    assert_eq!(
        fx.app
            .file_rename(&f.id, &long)
            .unwrap()
            .name
            .chars()
            .count(),
        200
    );
    // renaming to the same name writes nothing new
    let before = fx.row(&f.id).meta_version;
    fx.app.file_rename(&f.id, &long).unwrap();
    assert_eq!(fx.row(&f.id).meta_version, before);
    // each real rename is a new sealed version
    fx.app.file_rename(&f.id, "b.png").unwrap();
    assert_eq!(fx.row(&f.id).meta_version, before + 1);
}

#[test]
fn an_exported_vault_file_carries_no_file_rows() {
    let fx = Fx::new();
    fx.add_one(None, "a.txt", b"abc");
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("copy.passman");
    fx.picker.set_save(Some(dest.clone()));
    fx.app.vault_export_file(PW.into()).unwrap().unwrap();
    let copy = super::db::Store::open(&dest).unwrap();
    assert_eq!(
        copy.file_count().unwrap(),
        0,
        "blobs do not travel in an export, so neither do rows"
    );
    assert_eq!(
        fx.app.file_list(None).unwrap().len(),
        1,
        "the live vault is unchanged"
    );
}
