//! Behavioural tests of the import / export commands of [`AppCore`], on real temp vaults:
//! the file handles, the preview, the three duplicate modes and their history, the outbox jobs,
//! a failing entry, rollback, handle expiry, the plaintext export and its guards. Every file is
//! written inside a `tempfile` folder; nothing outside is touched.

use super::AppCore;
use crate::crypto::KdfParams;
use crate::importer::kdbx::test_support::{sample_db, save};
use crate::importer::{AttachmentSink, ImportOptions, NoSink, ParseOptions};
use crate::storage::backup::remote::mem::MemFactory;
use crate::storage::backup::{BackupRuntime, Timing};
use crate::storage::files::TestPicker;
use crate::storage::model::{CustomField, ItemDraft, ItemEvent, ItemType};
use crate::storage::session::{SessionConfig, EDIT_TOKEN_TTL};
use crate::storage::{BackupTargetInput, Result, StorageError};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PW: &str = "correct horse battery staple";
const PW2: &str = "a second long master password";

macro_rules! fixture {
    ($n:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/import/",
            $n
        )) as &[u8]
    };
}

fn fast() -> SessionConfig {
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

struct Fx {
    /// The app's data folder.
    _data: tempfile::TempDir,
    /// Where the files to import and the exports live (outside the data folder).
    files: tempfile::TempDir,
    picker: Arc<TestPicker>,
    app: AppCore,
}

impl Fx {
    fn new() -> Self {
        Self::build(None)
    }

    /// With in-memory backup targets (no network): one enabled target is added.
    fn with_target() -> Self {
        let fx = Self::build(Some(BackupRuntime::new(
            MemFactory::new(),
            Timing {
                background: false,
                ..Timing::default()
            },
        )));
        fx.app
            .backup_add(BackupTargetInput {
                name: "Mem".into(),
                endpoint: Some("https://s3.example.net".into()),
                region: "r".into(),
                bucket: "mem-bucket".into(),
                prefix: "p".into(),
                access_key_id: "AKIA".into(),
                secret_access_key: "secret".into(),
                path_style: true,
                enabled: true,
                master: true,
                share_in_master: false,
            })
            .unwrap();
        fx
    }

    fn build(runtime: Option<Arc<BackupRuntime>>) -> Self {
        let data = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let picker = Arc::new(TestPicker::default());
        let mut app = AppCore::open(data.path(), fast(), picker.clone()).unwrap();
        if let Some(rt) = runtime {
            app = app.with_backup_runtime(rt);
        }
        app.profile_create("Main", PW.into()).unwrap();
        Self {
            _data: data,
            files,
            picker,
            app,
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.app.data_dir().to_path_buf()
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.files.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// Queue `bytes` as the next picked file and pick it.
    fn pick(&self, name: &str, bytes: &[u8]) -> crate::importer::ImportPick {
        let p = self.write(name, bytes);
        self.picker.queue_import(p);
        self.app.import_pick().unwrap().expect("a file was picked")
    }

    fn titles(&self) -> Vec<String> {
        let mut t: Vec<String> = self
            .app
            .item_list()
            .unwrap()
            .into_iter()
            .map(|s| s.title)
            .collect();
        t.sort();
        t
    }

    fn id_of(&self, title: &str) -> String {
        self.app
            .item_list()
            .unwrap()
            .into_iter()
            .find(|s| s.title == title)
            .unwrap_or_else(|| panic!("no item {title}"))
            .id
    }

    fn count(&self, sql: &'static str) -> i64 {
        self.app.test_session().test_vault(|v| {
            v.store()
                .conn()
                .query_row(sql, [], |r| r.get::<_, i64>(0))
                .unwrap()
        })
    }

    fn queued(&self) -> u64 {
        let target = self.app.backup_list().unwrap()[0].id.clone();
        self.app
            .test_session()
            .test_vault(|v| v.store().outbox_stats(&target).unwrap().queued)
    }

    /// Pick `bytes` and import them in `mode` (as `format`).
    fn import(
        &self,
        format: &str,
        mode: &str,
        name: &str,
        bytes: &[u8],
    ) -> crate::importer::ImportOutcome {
        let pick = self.pick(name, bytes);
        self.app
            .import_commit(opts(&pick.handle, format, mode))
            .unwrap()
    }
}

fn opts(handle: &str, format: &str, mode: &str) -> ImportOptions {
    serde_json::from_value(serde_json::json!({"handle": handle, "format": format, "mode": mode}))
        .unwrap()
}

fn opts_with(v: serde_json::Value) -> ImportOptions {
    serde_json::from_value(v).unwrap()
}

fn code<T: std::fmt::Debug>(r: Result<T>) -> &'static str {
    r.expect_err("expected an error").code()
}

fn draft(item_type: ItemType, title: &str, fields: &[(&str, &str)]) -> ItemDraft {
    ItemDraft {
        item_type,
        title: title.into(),
        favorite: false,
        reprompt: false,
        tags: vec![],
        folder: None,
        fields: fields
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect::<BTreeMap<_, _>>(),
        custom_fields: vec![],
    }
}

const CHROME: &[u8] = b"name,url,username,password,note\nGitHub,https://github.com/login,me,pw-one,\"two\nlines\"\nNews,https://news.example.com,reader,secret,\n";

// ------------------------------------------------------------------ picking

#[test]
fn picking_sniffs_the_content_and_hides_the_path() {
    let fx = Fx::new();
    let sub = fx.files.path().join("deep").join("folder");
    std::fs::create_dir_all(&sub).unwrap();
    let path = sub.join("export.txt"); // the name says nothing
    std::fs::write(&path, fixture!("bitwarden.json")).unwrap();
    fx.picker.queue_import(path);
    let pick = fx.app.import_pick().unwrap().unwrap();
    assert_eq!(pick.format, "bitwarden-json");
    assert_eq!(pick.file_name, "export.txt");
    assert_eq!(pick.size, fixture!("bitwarden.json").len() as u64);
    assert!(!pick.needs_password && !pick.needs_keyfile);
    assert_eq!(pick.handle.len(), 32);
    assert!(pick.handle.chars().all(|c| c.is_ascii_hexdigit()));
    // Nothing the webview gets mentions a folder.
    let wire = serde_json::to_string(&pick).unwrap();
    assert!(!wire.contains("deep") && !wire.contains("folder") && !wire.contains('/'));
    let v: serde_json::Value = serde_json::from_str(&wire).unwrap();
    for key in [
        "handle",
        "fileName",
        "size",
        "format",
        "needsPassword",
        "needsKeyfile",
    ] {
        assert!(v.get(key).is_some(), "{key}");
    }
    // Two picks, two different handles.
    assert_ne!(
        fx.pick("again.json", fixture!("bitwarden.json")).handle,
        pick.handle
    );
}

#[test]
fn unknown_files_and_cancelled_dialogs() {
    let fx = Fx::new();
    assert!(
        fx.app.import_pick().unwrap().is_none(),
        "a cancelled dialog"
    );
    let pick = fx.pick("mystery.bin", &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(pick.format, "unknown");
    // A folder is not a file.
    fx.picker.queue_import(fx.files.path().to_path_buf());
    assert_eq!(code(fx.app.import_pick()), "unsupported_file");
    // A file that vanished.
    fx.picker.queue_import(fx.files.path().join("nope.csv"));
    assert_eq!(code(fx.app.import_pick()), "unsupported_file");
}

#[test]
fn a_keepass_database_asks_for_a_password_and_a_key_file() {
    let fx = Fx::new();
    let kdbx = save(&sample_db(), Some("db password"), None);
    let pick = fx.pick("vault.kdbx", &kdbx);
    assert_eq!(pick.format, "keepass-kdbx");
    assert!(pick.needs_password && pick.needs_keyfile);
    let key = fx.write("db.key", b"key file content");
    fx.picker.queue_keyfile(key);
    let kf = fx.app.import_pick_keyfile().unwrap().unwrap();
    assert_eq!(kf.file_name, "db.key");
    assert_eq!(kf.handle.len(), 32);
    assert!(fx.app.import_pick_keyfile().unwrap().is_none());
}

#[test]
fn every_command_needs_an_unlocked_vault_and_does_not_open_a_dialog_first() {
    let fx = Fx::new();
    let pick = fx.pick("a.csv", CHROME);
    fx.picker.queue_import(fx.write("b.csv", CHROME));
    fx.picker.queue_keyfile(fx.write("k", b"k"));
    fx.app.lock();
    assert_eq!(code(fx.app.import_pick()), "locked");
    assert_eq!(code(fx.app.import_pick_keyfile()), "locked");
    assert_eq!(code(fx.app.inspect_csv_headers(&pick.handle)), "locked");
    assert_eq!(
        code(
            fx.app
                .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        ),
        "locked"
    );
    assert_eq!(
        code(
            fx.app
                .import_commit(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        ),
        "locked"
    );
    assert_eq!(code(fx.app.export_plain("csv", PW.into())), "locked");
    // The queued files were never consumed: no dialog opened while locked.
    fx.app.unlock(PW.into()).unwrap();
    assert!(fx.app.import_pick().unwrap().is_some());
    assert!(fx.app.import_pick_keyfile().unwrap().is_some());
}

#[test]
fn csv_headers_come_from_the_handle_and_cells_are_capped() {
    let fx = Fx::new();
    let long = "x".repeat(300);
    let text = format!("Site Label;Login ID;Secret;Address\n{long};me;pw;https://x.example\nb;c;d;e\nf;g;h;i\nj;k;l;m\nn;o;p;q\nr;s;t;u\n");
    let pick = fx.pick("odd.csv", text.as_bytes());
    assert_eq!(pick.format, "custom-csv");
    let i = fx.app.inspect_csv_headers(&pick.handle).unwrap();
    assert_eq!(i.delimiter, ";");
    assert_eq!(
        i.headers,
        vec!["Site Label", "Login ID", "Secret", "Address"]
    );
    assert_eq!(i.sample.len(), 5, "the first five rows");
    assert_eq!(i.sample[0][0].chars().count(), 80);
    assert_eq!(
        code(
            fx.app
                .inspect_csv_headers("0123456789abcdef0123456789abcdef")
        ),
        "not_found"
    );
    // The mapper's choices drive the import.
    let o = opts_with(serde_json::json!({
        "handle": pick.handle, "format": "custom-csv", "mode": "skip_duplicates",
        "mapping": {"title": "Site Label", "username": "Login ID", "password": "Secret", "url": "Address"}
    }));
    assert_eq!(fx.app.import_preview(o).unwrap().entries, 6);
    // Without a mapping that makes sense the file is refused with a sentence.
    assert_eq!(
        code(
            fx.app
                .import_preview(opts(&pick.handle, "custom-csv", "skip_duplicates"))
        ),
        "invalid_input"
    );
}

// ------------------------------------------------------------------ preview and commit

#[test]
fn the_preview_counts_only_one_time_codes_that_will_work() {
    let fx = Fx::new();
    let json = br#"{"encrypted":false,"folders":[],"items":[
      {"type":1,"name":"Good","login":{"username":"a","password":"p","totp":"JBSWY3DPEHPK3PXP"}},
      {"type":1,"name":"Bad key","login":{"username":"b","password":"p","totp":"not.a@base32-key1"}}]}"#;
    let pick = fx.pick("bw.json", json);
    let p = fx
        .app
        .import_preview(opts(&pick.handle, "bitwarden-json", "skip_duplicates"))
        .unwrap();
    assert_eq!((p.entries, p.totp_seeds), (2, 1));
    assert!(
        p.warnings.iter().any(|w| w.contains("Bad key")),
        "{:?}",
        p.warnings
    );
}

#[test]
fn a_preview_changes_nothing_and_a_commit_adds_everything_with_history() {
    let fx = Fx::with_target();
    let pick = fx.pick("chrome.csv", CHROME);
    let versions = fx.count("SELECT count(*) FROM versions");
    let items = fx.count("SELECT count(*) FROM items");
    let queued = fx.queued();

    let p = fx
        .app
        .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap();
    assert_eq!(
        (
            p.entries,
            p.totp_seeds,
            p.attachments,
            p.duplicates,
            p.skipped
        ),
        (2, 0, 0, 0, 0)
    );
    assert_eq!(p.titles, vec!["GitHub", "News"]);
    // The preview is repeatable and writes nothing.
    fx.app
        .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap();
    assert!(fx.app.item_list().unwrap().is_empty());
    assert_eq!(fx.count("SELECT count(*) FROM versions"), versions);
    assert_eq!(fx.count("SELECT count(*) FROM items"), items);
    assert_eq!(fx.queued(), queued);

    let out = fx
        .app
        .import_commit(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap();
    assert_eq!(
        (out.added, out.appended, out.skipped, out.failed),
        (2, 0, 0, 0)
    );
    assert_eq!(fx.titles(), vec!["GitHub", "News"]);
    // Each item has its first version in the history, and it is queued for every target.
    assert_eq!(fx.count("SELECT count(*) FROM versions"), versions + 2);
    assert_eq!(fx.count("SELECT count(*) FROM items WHERE dirty = 1"), 2);
    assert_eq!(
        fx.queued(),
        queued + 4,
        "a version job and an item job per new item"
    );
    let id = fx.id_of("GitHub");
    let h = fx.app.history_list(&id).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].event, ItemEvent::Created);
    let d = fx.app.item_reveal(&id, Some(PW.into())).unwrap();
    assert_eq!(d.fields["password"], "pw-one");
    assert_eq!(d.fields["notes"], "two\nlines");
    assert_eq!(d.fields["url"], "https://github.com/login");
    // The worker drains what the import queued.
    fx.app.backup_run(None).unwrap();
    assert_eq!(fx.queued(), 0);
}

#[test]
fn the_outcome_has_the_four_counts_and_the_additive_notes() {
    let fx = Fx::new();
    let out = fx.import("chrome-csv", "skip_duplicates", "c.csv", CHROME);
    let v = serde_json::to_value(&out).unwrap();
    for key in ["added", "appended", "skipped", "failed", "warnings"] {
        assert!(v.get(key).is_some(), "{key}");
    }
    let pick = fx.pick("c2.csv", CHROME);
    let p = serde_json::to_value(
        fx.app
            .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
            .unwrap(),
    )
    .unwrap();
    for key in [
        "entries",
        "totpSeeds",
        "attachments",
        "duplicates",
        "skipped",
        "warnings",
        "titles",
    ] {
        assert!(p.get(key).is_some(), "{key}");
    }
}

#[test]
fn skip_duplicates_a_second_identical_import_adds_nothing() {
    let fx = Fx::with_target();
    let first = fx.import("chrome-csv", "skip_duplicates", "c.csv", CHROME);
    assert_eq!(first.added, 2);
    let versions = fx.count("SELECT count(*) FROM versions");
    let queued = fx.queued();
    let pick = fx.pick("c-again.csv", CHROME);
    let preview = fx
        .app
        .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap();
    assert_eq!(preview.duplicates, 2);
    let second = fx
        .app
        .import_commit(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap();
    assert_eq!(
        (second.added, second.appended, second.skipped, second.failed),
        (0, 0, 2, 0)
    );
    assert_eq!(fx.titles().len(), 2);
    assert_eq!(
        fx.count("SELECT count(*) FROM versions"),
        versions,
        "no new history"
    );
    assert_eq!(fx.queued(), queued, "no new jobs");
}

#[test]
fn import_all_adds_the_duplicates_again() {
    let fx = Fx::new();
    fx.import("chrome-csv", "skip_duplicates", "c.csv", CHROME);
    let out = fx.import("chrome-csv", "import_all", "c2.csv", CHROME);
    assert_eq!((out.added, out.skipped), (2, 0));
    assert_eq!(fx.titles(), vec!["GitHub", "GitHub", "News", "News"]);
}

#[test]
fn append_version_adds_a_version_only_when_the_content_differs() {
    let fx = Fx::with_target();
    fx.import("chrome-csv", "skip_duplicates", "c.csv", CHROME);
    fx.app.backup_run(None).unwrap();
    let github = fx.id_of("GitHub");
    let news = fx.id_of("News");
    let queued = fx.queued();

    // A newer file: GitHub has a new password (and no note column), News is unchanged.
    let newer = b"name,url,username,password\nGitHub,https://github.com/login,me,pw-two\nNews,https://news.example.com,reader,secret\n";
    let pick = fx.pick("newer.csv", newer);
    let pre = fx
        .app
        .import_preview(opts(&pick.handle, "chrome-csv", "append_version"))
        .unwrap();
    assert_eq!(pre.duplicates, 2);
    let out = fx
        .app
        .import_commit(opts(&pick.handle, "chrome-csv", "append_version"))
        .unwrap();
    assert_eq!(
        (out.added, out.appended, out.skipped, out.failed),
        (0, 1, 1, 0)
    );
    assert_eq!(fx.titles(), vec!["GitHub", "News"], "no new items");

    let h = fx.app.history_list(&github).unwrap();
    assert_eq!(h.len(), 2, "the old content stays in the history");
    assert_eq!(h[0].version, 2);
    assert_eq!(h[0].event, ItemEvent::Edited);
    assert_eq!(h[0].changed_fields, vec!["password"]);
    assert_eq!(fx.app.history_list(&news).unwrap().len(), 1);
    let now = fx.app.item_reveal(&github, Some(PW.into())).unwrap();
    assert_eq!(now.fields["password"], "pw-two");
    assert_eq!(
        now.fields["notes"], "two\nlines",
        "what the file lacks is kept"
    );
    let old = fx.app.history_get(&github, 1, Some(PW.into())).unwrap();
    assert_eq!(old.fields["password"], "pw-one");
    assert_eq!(
        fx.queued(),
        queued + 2,
        "the new version and the item head are queued"
    );

    // Importing the newer file again changes nothing.
    let again = fx.import("chrome-csv", "append_version", "newer2.csv", newer);
    assert_eq!((again.appended, again.skipped), (0, 2));
    assert_eq!(fx.app.history_list(&github).unwrap().len(), 2);
}

#[test]
fn the_duplicate_rule_is_type_title_username_and_host() {
    let fx = Fx::new();
    let existing = |title: &str, user: &str, url: &str| {
        draft(
            ItemType::Login,
            title,
            &[("username", user), ("password", "x"), ("url", url)],
        )
    };
    fx.app
        .item_create(existing(
            "Mail",
            "Ana@Example.com",
            "https://www.mail.example/login",
        ))
        .unwrap();
    fx.app
        .item_create(draft(ItemType::SecureNote, "Memo", &[("body", "b")]))
        .unwrap();
    // Same title and user, different case and a path on the same host: a duplicate.
    // Same title, another user, or another host, or another type: not.
    let file = b"name,url,username,password\nMAIL,https://mail.example/other,ana@example.com,p\nMail,https://mail.example,someone@else,p\nMail,https://another.example,ana@example.com,p\n";
    let pick = fx.pick("rule.csv", file);
    let p = fx
        .app
        .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap();
    assert_eq!((p.entries, p.duplicates), (3, 1));
    let note = b"folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n,,note,memo,other body,,0,,,,\n,,login,memo,,,0,,u,p,\n";
    let pick = fx.pick("note.csv", note);
    let p = fx
        .app
        .import_preview(opts(&pick.handle, "bitwarden-csv", "skip_duplicates"))
        .unwrap();
    assert_eq!(
        (p.entries, p.duplicates),
        (2, 1),
        "the note matches the note, the login does not"
    );
}

#[test]
fn a_file_does_not_hide_its_own_records_and_the_trash_does_not_block() {
    let fx = Fx::new();
    let twice =
        b"name,url,username,password\nSame,https://s.example,u,one\nSame,https://s.example,u,two\n";
    let out = fx.import("chrome-csv", "skip_duplicates", "t.csv", twice);
    assert_eq!(
        out.added, 2,
        "duplicates are looked up in the vault as it was before the import"
    );
    let again = fx.import("chrome-csv", "skip_duplicates", "t2.csv", twice);
    assert_eq!((again.added, again.skipped), (0, 2));
    // Trashed items are not duplicates.
    for s in fx.app.item_list().unwrap() {
        fx.app.item_trash(&s.id).unwrap();
    }
    let back = fx.import("chrome-csv", "skip_duplicates", "t3.csv", twice);
    assert_eq!(back.added, 2);
}

#[test]
fn an_entry_that_cannot_be_saved_is_counted_and_the_others_still_go_in() {
    let fx = Fx::new();
    let huge = "x".repeat(70_000); // over the 64 KiB limit of one field
    let json = format!(
        r#"{{"encrypted":false,"items":[
        {{"type":1,"name":"First","login":{{"username":"a","password":"1"}}}},
        {{"type":1,"name":"Too big","login":{{"username":"b","password":"{huge}"}}}},
        {{"type":1,"name":"Third","login":{{"username":"c","password":"3"}}}}]}}"#
    );
    let pick = fx.pick("b.json", json.as_bytes());
    let pre = fx
        .app
        .import_preview(opts(&pick.handle, "bitwarden-json", "skip_duplicates"))
        .unwrap();
    assert_eq!(pre.entries, 3);
    assert!(pre
        .warnings
        .iter()
        .any(|w| w.contains("Too big") && w.contains("cannot be imported")));
    let out = fx
        .app
        .import_commit(opts(&pick.handle, "bitwarden-json", "skip_duplicates"))
        .unwrap();
    assert_eq!((out.added, out.failed), (2, 1));
    assert!(out
        .warnings
        .iter()
        .any(|w| w.contains("Too big") && w.contains("was not imported")));
    assert_eq!(fx.titles(), vec!["First", "Third"]);
    // The notes never contain the value itself.
    assert!(out.warnings.iter().all(|w| !w.contains("xxxxxxxx")));
}

#[test]
fn a_database_error_rolls_the_whole_import_back() {
    let fx = Fx::with_target();
    fx.app
        .item_create(draft(ItemType::SecureNote, "Existing", &[("body", "b")]))
        .unwrap();
    let items = fx.count("SELECT count(*) FROM items");
    let queued = fx.queued();
    // Break the second write of every item.
    fx.app.test_session().test_vault(|v| {
        v.store()
            .conn()
            .execute_batch("DROP TABLE versions")
            .unwrap();
    });
    let pick = fx.pick("c.csv", CHROME);
    let err = fx
        .app
        .import_commit(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap_err();
    assert_eq!(err.code(), "storage");
    assert_eq!(
        fx.count("SELECT count(*) FROM items"),
        items,
        "the items rows were rolled back"
    );
    assert_eq!(fx.queued(), queued, "and so were the jobs");
    assert_eq!(
        fx.titles(),
        vec!["Existing"],
        "the in-memory list was not touched either"
    );
}

#[test]
fn a_commit_without_a_preview_parses_the_file_itself() {
    let fx = Fx::new();
    let out = fx.import("chrome-csv", "skip_duplicates", "c.csv", CHROME);
    assert_eq!(out.added, 2);
}

#[test]
fn the_commit_imports_what_the_preview_showed_even_if_the_file_changed() {
    let fx = Fx::new();
    let pick = fx.pick("c.csv", CHROME);
    let o = || opts(&pick.handle, "chrome-csv", "skip_duplicates");
    assert_eq!(fx.app.import_preview(o()).unwrap().entries, 2);
    // Someone replaces the file on disk after the preview.
    let path = fx.files.path().join("c.csv");
    std::fs::write(
        &path,
        b"name,url,username,password\nOnly,https://o.example,u,p\n",
    )
    .unwrap();
    assert_eq!(fx.app.import_commit(o()).unwrap().added, 2);
    assert_eq!(fx.titles(), vec!["GitHub", "News"]);
    // The preview is consumed: the next read of the (changed) file sees the new content.
    assert_eq!(fx.app.import_commit(o()).unwrap().added, 1);
    assert!(fx.titles().contains(&"Only".to_string()));
}

#[test]
fn another_password_means_checking_the_file_again() {
    let fx = Fx::new();
    let kdbx = save(&sample_db(), Some("right password"), None);
    let pick = fx.pick("db.kdbx", &kdbx);
    let with = |pw: &str| {
        opts_with(serde_json::json!({
            "handle": pick.handle, "format": "keepass-kdbx", "mode": "skip_duplicates", "password": pw
        }))
    };
    assert_eq!(
        fx.app
            .import_preview(with("right password"))
            .unwrap()
            .entries,
        2
    );
    // A stored preview is not an unlock: another password is checked against the file again.
    let e = fx.app.import_preview(with("wrong password")).unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    assert_eq!(
        e.to_string(),
        "That password or key file does not open this KeePass database."
    );
    assert_eq!(
        fx.app
            .import_commit(with("wrong password"))
            .unwrap_err()
            .code(),
        "invalid_input"
    );
    assert_eq!(fx.titles().len(), 0);
}

#[test]
fn a_keepass_database_with_a_key_file_and_attachments() {
    #[derive(Default)]
    struct Collect(Mutex<Vec<(String, String, Vec<u8>)>>);
    impl AttachmentSink for Collect {
        fn add(&self, item_id: &str, name: &str, bytes: &[u8]) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .push((item_id.into(), name.into(), bytes.to_vec()));
            Ok(())
        }
    }

    let fx = Fx::new();
    let keyfile = b"key file content";
    let kdbx = save(&sample_db(), Some("pw pw pw pw"), Some(keyfile));
    let pick = fx.pick("db.kdbx", &kdbx);
    fx.picker.queue_keyfile(fx.write("db.key", keyfile));
    let kf = fx.app.import_pick_keyfile().unwrap().unwrap();
    let o = |mode: &str| {
        opts_with(serde_json::json!({
            "handle": pick.handle, "format": "keepass-kdbx", "mode": mode,
            "password": "pw pw pw pw", "keyfileHandle": kf.handle
        }))
    };
    // Without the key file it does not open.
    let no_key = opts_with(serde_json::json!({
        "handle": pick.handle, "format": "keepass-kdbx", "mode": "skip_duplicates", "password": "pw pw pw pw"
    }));
    assert_eq!(code(fx.app.import_preview(no_key)), "invalid_input");
    let pre = fx.app.import_preview(o("skip_duplicates")).unwrap();
    assert_eq!((pre.entries, pre.totp_seeds, pre.attachments), (2, 1, 1));

    // The attachment goes into the encrypted drive of the saved item (encrypted from memory).
    let out = fx.app.import_commit(o("skip_duplicates")).unwrap();
    assert_eq!(out.added, 2);
    assert!(
        out.warnings.iter().all(|w| !w.contains("attachment")),
        "{:?}",
        out.warnings
    );
    // `file_list(None)` is the drive's own top level (files with no item): the attachment belongs to its item.
    assert!(fx.app.file_list(None).unwrap().is_empty());
    let mut files = Vec::new();
    for item in fx.app.item_list().unwrap() {
        files.extend(fx.app.file_list(Some(&item.id)).unwrap());
    }
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].name, "hello.txt");
    assert_eq!(&fx.app.file_read(&files[0].id).unwrap()[..], b"Hello!");
    assert!(files[0]
        .item_id
        .as_deref()
        .is_some_and(|id| fx.app.item_get(id).is_ok()));

    // With no place for attachments the note says so: nothing is lost silently.
    let out = fx
        .app
        .import_commit_with_sink(o("import_all"), &NoSink)
        .unwrap();
    assert!(
        out.warnings
            .iter()
            .any(|w| w == "1 attachment was not imported (the encrypted drive is not available)."),
        "{:?}",
        out.warnings
    );

    // With an explicit sink the attachment goes to the saved item.
    let sink = Collect::default();
    let again = fx
        .app
        .import_commit_with_sink(o("import_all"), &sink)
        .unwrap();
    assert_eq!(again.added, 2);
    let got = sink.0.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].1.as_str(), got[0].2.as_slice()),
        ("hello.txt", &b"Hello!"[..])
    );
    assert!(fx.app.item_get(&got[0].0).is_ok(), "the id is a saved item");
    assert!(again.warnings.iter().all(|w| !w.contains("attachment")));
}

#[test]
fn a_sink_that_fails_leaves_a_note_not_an_error() {
    struct Broken;
    impl AttachmentSink for Broken {
        fn add(&self, _: &str, _: &str, _: &[u8]) -> Result<()> {
            Err(StorageError::invalid("no space"))
        }
    }
    let fx = Fx::new();
    let kdbx = save(&sample_db(), Some("pw"), None);
    let pick = fx.pick("db.kdbx", &kdbx);
    let o = opts_with(serde_json::json!({
        "handle": pick.handle, "format": "keepass-kdbx", "mode": "skip_duplicates", "password": "pw"
    }));
    let out = fx.app.import_commit_with_sink(o, &Broken).unwrap();
    assert_eq!(out.added, 2);
    assert!(out
        .warnings
        .iter()
        .any(|w| w == "1 attachment was not saved."));
}

// ------------------------------------------------------------------ handles

#[test]
fn handles_expire_after_their_time() {
    let fx = Fx::new();
    fx.app
        .test_session()
        .test_import_ttl(Duration::from_millis(80));
    let pick = fx.pick("c.csv", CHROME);
    assert!(fx
        .app
        .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .is_ok());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        code(
            fx.app
                .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        ),
        "not_found"
    );
    assert_eq!(
        code(
            fx.app
                .import_commit(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        ),
        "not_found"
    );
    assert_eq!(code(fx.app.inspect_csv_headers(&pick.handle)), "not_found");
    assert_eq!(
        fx.app.test_session().test_import_counts(),
        (0, 0),
        "the parsed entries went too"
    );
    assert!(fx.titles().is_empty());
}

#[test]
fn locking_and_changing_account_clear_every_handle() {
    let fx = Fx::new();
    let pick = fx.pick("c.csv", CHROME);
    fx.app
        .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap();
    assert_eq!(fx.app.test_session().test_import_counts(), (1, 1));
    fx.app.lock();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(
        code(
            fx.app
                .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        ),
        "not_found"
    );
    assert_eq!(fx.app.test_session().test_import_counts(), (0, 0));

    // Another account: its own, empty table; and going back finds nothing either.
    let pick = fx.pick("c2.csv", CHROME);
    let first = fx.app.profile_list().unwrap().selected_id.clone().unwrap();
    fx.app.profile_create("Second", PW2.into()).unwrap();
    assert_eq!(
        code(
            fx.app
                .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        ),
        "not_found"
    );
    fx.app.profile_select(&first).unwrap();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(
        code(
            fx.app
                .import_commit(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        ),
        "not_found"
    );
}

#[test]
fn cancel_forgets_the_file_and_the_preview() {
    let fx = Fx::new();
    let pick = fx.pick("c.csv", CHROME);
    fx.app
        .import_preview(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        .unwrap();
    fx.app.import_cancel(&pick.handle).unwrap();
    assert_eq!(fx.app.test_session().test_import_counts(), (0, 0));
    assert_eq!(
        code(
            fx.app
                .import_commit(opts(&pick.handle, "chrome-csv", "skip_duplicates"))
        ),
        "not_found"
    );
    // Cancelling something unknown, twice, or with the vault locked is harmless.
    fx.app.import_cancel(&pick.handle).unwrap();
    fx.app.import_cancel("zzzz").unwrap();
    fx.app.lock();
    fx.app.import_cancel(&pick.handle).unwrap();
}

#[test]
fn only_the_latest_files_and_previews_are_kept() {
    let fx = Fx::new();
    let picks: Vec<_> = (0..20)
        .map(|i| fx.pick(&format!("f{i}.csv"), CHROME))
        .collect();
    let (files, _) = fx.app.test_session().test_import_counts();
    assert_eq!(files, 16);
    assert_eq!(
        code(fx.app.inspect_csv_headers(&picks[0].handle)),
        "not_found"
    );
    assert!(fx.app.inspect_csv_headers(&picks[19].handle).is_ok());
    for p in &picks[16..] {
        fx.app
            .import_preview(opts(&p.handle, "chrome-csv", "skip_duplicates"))
            .unwrap();
    }
    assert_eq!(
        fx.app.test_session().test_import_counts().1,
        2,
        "two plaintext previews at most"
    );
}

#[test]
fn a_key_file_handle_is_not_a_file_to_import_and_the_other_way_round() {
    let fx = Fx::new();
    let pick = fx.pick("c.csv", CHROME);
    fx.picker.queue_keyfile(fx.write("k", b"key"));
    let kf = fx.app.import_pick_keyfile().unwrap().unwrap();
    assert_eq!(
        code(
            fx.app
                .import_preview(opts(&kf.handle, "chrome-csv", "skip_duplicates"))
        ),
        "not_found"
    );
    let o = opts_with(serde_json::json!({
        "handle": pick.handle, "format": "chrome-csv", "mode": "skip_duplicates", "keyfileHandle": pick.handle
    }));
    assert_eq!(code(fx.app.import_preview(o)), "not_found");
}

#[test]
fn options_have_the_wire_shape_of_the_frontend() {
    let o: ImportOptions = serde_json::from_str(
        r#"{"handle":"h","format":"1password-1pux","mode":"append_version","password":"p","keyfileHandle":"k","mapping":{"title":"A","folder":"B"}}"#,
    )
    .unwrap();
    assert_eq!(o.format.id(), "1password-1pux");
    assert_eq!(o.mapping.as_ref().unwrap().folder.as_deref(), Some("B"));
    assert!(serde_json::from_str::<ImportOptions>(
        r#"{"handle":"h","format":"nope","mode":"import_all"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<ImportOptions>(
        r#"{"handle":"h","format":"safari-csv","mode":"replace"}"#
    )
    .is_err());
    // The debug text shows neither the password nor the handle.
    let shown = format!("{o:?}");
    assert!(!shown.contains("\"p\"") && !shown.contains("keyfile") && !shown.contains("\"h\""));
    let _ = ParseOptions::default();
}

// ------------------------------------------------------------------ plaintext export

fn all_types() -> Vec<ItemDraft> {
    let mut login = draft(
        ItemType::Login,
        "GitHub",
        &[
            ("username", "ana@example.com"),
            ("password", "p@ss w0rd"),
            ("url", "https://github.com/login"),
            ("totp", "JBSWY3DPEHPK3PXP"),
            ("notes", "line one\nline two"),
        ],
    );
    login.folder = Some("Work".into());
    login.tags = vec!["code".into(), "2fa".into()];
    login.favorite = true;
    login.custom_fields = vec![
        CustomField {
            name: "PIN".into(),
            value: "1234".into(),
            hidden: true,
        },
        CustomField {
            name: "Label".into(),
            value: "main".into(),
            hidden: false,
        },
    ];
    let card = draft(
        ItemType::Card,
        "Visa",
        &[
            ("cardholder", "Ana Example"),
            ("brand", "Visa"),
            ("number", "4111111111111111"),
            ("expMonth", "09"),
            ("expYear", "2029"),
            ("code", "123"),
            ("pin", "9999"),
            ("notes", "travel card"),
        ],
    );
    let identity = draft(
        ItemType::Identity,
        "Me",
        &[
            ("firstName", "Ana"),
            ("lastName", "Example"),
            ("email", "ana@example.com"),
            ("phone", "555-0100"),
            ("address1", "1 Road"),
            ("city", "Town"),
            ("country", "ES"),
            ("passport", "X123"),
        ],
    );
    let ssh = draft(
        ItemType::SshKey,
        "Server key",
        &[
            ("publicKey", "ssh-ed25519 AAAAC3 ana@laptop"),
            (
                "privateKey",
                "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----",
            ),
            ("passphrase", "open sesame"),
            ("fingerprint", "SHA256:abc"),
        ],
    );
    let note = draft(ItemType::SecureNote, "Memo", &[("body", "remember\nthis")]);
    let totp = draft(
        ItemType::Totp,
        "Authenticator",
        &[
            ("issuer", "Bank"),
            ("account", "ana"),
            ("totp", "JBSWY3DPEHPK3PXP"),
        ],
    );
    vec![login, card, identity, ssh, note, totp]
}

fn canonical(mut d: ItemDraft) -> ItemDraft {
    d.normalize().unwrap();
    d.tags.sort();
    d.custom_fields.sort_by(|a, b| a.name.cmp(&b.name));
    d
}

fn fill(fx: &Fx) {
    for d in all_types() {
        fx.app.item_create(d).unwrap();
    }
}

#[test]
fn the_json_export_reads_back_with_every_field() {
    let fx = Fx::with_target();
    fill(&fx);
    let dest = fx.files.path().join("export.json");
    fx.picker.set_export(Some(dest.clone()));
    let out = fx
        .app
        .export_plain("bitwarden-json", PW.into())
        .unwrap()
        .unwrap();
    assert_eq!(out.path, dest.to_string_lossy());
    assert!(out.warnings.is_empty());
    let names = fx.picker.export_names();
    assert_eq!(names.len(), 1);
    assert!(
        names[0].starts_with("Main-") && names[0].ends_with("-plaintext.json"),
        "{}",
        names[0]
    );

    let bytes = std::fs::read(&dest).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["encrypted"], false);
    assert_eq!(v["items"].as_array().unwrap().len(), 6);
    // In the file, as the format says: plain values.
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("p@ss w0rd") && text.contains("4111111111111111"));

    // Read it with the importer again: every field of every item comes back.
    let parsed = crate::importer::parse(
        crate::importer::ImportFormat::BitwardenJson,
        &bytes,
        &ParseOptions::default(),
    )
    .unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let mut back: Vec<ItemDraft> = parsed
        .entries
        .iter()
        .map(|e| canonical(e.to_draft().draft))
        .collect();
    let mut want: Vec<ItemDraft> = all_types().into_iter().map(canonical).collect();
    back.sort_by(|a, b| a.title.cmp(&b.title));
    want.sort_by(|a, b| a.title.cmp(&b.title));
    assert_eq!(back, want);
}

#[test]
fn the_csv_export_reads_back_logins_and_notes() {
    let fx = Fx::new();
    fill(&fx);
    let dest = fx.files.path().join("export.csv");
    fx.picker.set_export(Some(dest.clone()));
    fx.app.export_plain("csv", PW.into()).unwrap().unwrap();
    assert!(fx.picker.export_names()[0].ends_with("-plaintext.csv"));
    let bytes = std::fs::read(&dest).unwrap();
    let parsed = crate::importer::parse(
        crate::importer::ImportFormat::BitwardenCsv,
        &bytes,
        &ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(parsed.entries.len(), 6);
    let login = parsed.entries.iter().find(|e| e.title == "GitHub").unwrap();
    assert_eq!(login.username, "ana@example.com");
    assert_eq!(login.password, "p@ss w0rd");
    assert_eq!(login.totp, vec!["JBSWY3DPEHPK3PXP"]);
    assert_eq!(login.notes, "line one\nline two");
    assert_eq!(login.folder.as_deref(), Some("Work"));
    assert!(login.favorite);
    let note = parsed.entries.iter().find(|e| e.title == "Memo").unwrap();
    assert!(note.notes.contains("remember\nthis"));
    // Card numbers are in the file as text (this is the plaintext export).
    assert!(String::from_utf8_lossy(&bytes).contains("4111111111111111"));
}

#[test]
fn the_export_needs_the_master_password_first_and_opens_no_dialog_without_it() {
    let fx = Fx::new();
    fill(&fx);
    let dest = fx.files.path().join("never.json");
    fx.picker.set_export(Some(dest.clone()));
    assert_eq!(
        code(
            fx.app
                .export_plain("bitwarden-json", "wrong password here".into())
        ),
        "wrong_password"
    );
    assert!(
        fx.picker.export_names().is_empty(),
        "the dialog was never opened"
    );
    assert!(!dest.exists());
    // The right password with a cancelled dialog: nothing is written, nothing is an error.
    fx.picker.set_export(None);
    assert!(fx
        .app
        .export_plain("bitwarden-json", PW.into())
        .unwrap()
        .is_none());
    assert_eq!(fx.picker.export_names().len(), 1);
    assert!(!dest.exists());
    // Only the two documented formats.
    assert_eq!(code(fx.app.export_plain("xml", PW.into())), "invalid_input");
    assert_eq!(code(fx.app.export_plain("", PW.into())), "invalid_input");
}

#[test]
fn the_export_refuses_the_apps_own_data_folder() {
    let fx = Fx::new();
    fill(&fx);
    let inside = fx.data_dir().join("export.json");
    fx.picker.set_export(Some(inside.clone()));
    assert_eq!(
        code(fx.app.export_plain("bitwarden-json", PW.into())),
        "invalid_input"
    );
    assert!(!inside.exists());
}

#[cfg(unix)]
#[test]
fn the_export_file_is_private_whole_and_leaves_no_temp_file() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new();
    fill(&fx);
    let dest = fx.files.path().join("private.json");
    fx.picker.set_export(Some(dest.clone()));
    fx.app
        .export_plain("bitwarden-json", PW.into())
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // Exporting over an earlier export (the save dialog asked first) keeps it private.
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
    fx.app
        .export_plain("bitwarden-json", PW.into())
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&dest).unwrap()).is_ok());
    let leftovers: Vec<_> = std::fs::read_dir(fx.files.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp") || n.contains(".part"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn an_unwritable_destination_is_an_error_with_a_fixed_sentence() {
    let fx = Fx::new();
    fill(&fx);
    fx.picker
        .set_export(Some(fx.files.path().join("missing-folder").join("x.json")));
    let e = fx
        .app
        .export_plain("bitwarden-json", PW.into())
        .unwrap_err();
    assert_eq!(e.code(), "storage");
    assert!(!e.to_string().contains("missing-folder"));
}

#[test]
fn export_from_one_account_imports_into_another_unchanged() {
    let fx = Fx::new();
    fill(&fx);
    let dest = fx.files.path().join("move.json");
    fx.picker.set_export(Some(dest.clone()));
    fx.app
        .export_plain("bitwarden-json", PW.into())
        .unwrap()
        .unwrap();

    fx.app.profile_create("Second", PW2.into()).unwrap();
    assert!(fx.app.item_list().unwrap().is_empty());
    let bytes = std::fs::read(&dest).unwrap();
    let pick = fx.pick("move.json", &bytes);
    assert_eq!(pick.format, "bitwarden-json");
    let out = fx
        .app
        .import_commit(opts(&pick.handle, "bitwarden-json", "skip_duplicates"))
        .unwrap();
    assert_eq!((out.added, out.failed), (6, 0));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let mut got = Vec::new();
    for s in fx.app.item_list().unwrap() {
        got.push(canonical(
            fx.app.item_reveal(&s.id, Some(PW2.into())).unwrap(),
        ));
    }
    let mut want: Vec<ItemDraft> = all_types().into_iter().map(canonical).collect();
    got.sort_by(|a, b| a.title.cmp(&b.title));
    want.sort_by(|a, b| a.title.cmp(&b.title));
    assert_eq!(got, want);
    // And the same file again adds nothing.
    let again = fx
        .app
        .import_commit(opts(&pick.handle, "bitwarden-json", "skip_duplicates"))
        .unwrap();
    assert_eq!((again.added, again.skipped), (0, 6));
}

#[test]
fn trashed_items_are_not_exported_and_archived_ones_are() {
    let fx = Fx::new();
    fill(&fx);
    fx.app.item_trash(&fx.id_of("Memo")).unwrap();
    fx.app.item_archive(&fx.id_of("Visa")).unwrap();
    let dest = fx.files.path().join("e.json");
    fx.picker.set_export(Some(dest.clone()));
    fx.app
        .export_plain("bitwarden-json", PW.into())
        .unwrap()
        .unwrap();
    let text = std::fs::read_to_string(&dest).unwrap();
    assert!(text.contains("Visa") && !text.contains("remember"));
}

// ============================================================ the same account in two apps (Merge)

/// An Ente Auth plain-text line: the standard link plus the app's own `codeDisplay` (its note, tags, pin).
fn ente_line(issuer: &str, account: &str, secret: &str, note: &str, tag: &str) -> String {
    let display =
        serde_json::json!({"pinned": false, "trashed": false, "tags": [tag], "note": note})
            .to_string();
    let encoded: String = display
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!(
        "otpauth://totp/{issuer}:{account}?secret={secret}&issuer={issuer}&codeDisplay={encoded}\n"
    )
}

fn bitwarden_login(
    name: &str,
    username: &str,
    url: &str,
    notes: &str,
    totp: Option<&str>,
) -> String {
    serde_json::json!({
        "encrypted": false, "folders": [],
        "items": [{"id": "i1", "type": 1, "name": name, "notes": notes, "favorite": false,
            "login": {"uris": [{"uri": url}], "username": username, "password": "bw-password", "totp": totp}}]
    })
    .to_string()
}

const SECRET: &str = "JBSWY3DPEHPK3PXP";

fn reveal(fx: &Fx, title: &str) -> ItemDraft {
    fx.app
        .item_reveal(&fx.id_of(title), Some(PW.into()))
        .unwrap()
}

#[test]
fn a_code_from_an_authenticator_joins_the_login_of_the_same_account_and_every_note_is_kept() {
    let fx = Fx::new();
    let login = fx
        .app
        .item_create(draft(
            ItemType::Login,
            "GitHub",
            &[
                ("username", "ana@example.com"),
                ("password", "bw-password"),
                ("url", "https://github.com/login"),
                ("notes", "Recovery: abc-123"),
            ],
        ))
        .unwrap();
    fx.app
        .item_create(draft(ItemType::Login, "Other", &[("username", "someone")]))
        .unwrap();

    let file = ente_line(
        "GitHub",
        "ana@example.com",
        SECRET,
        "Ente backup code: 1234-5678",
        "work",
    );
    let pick = fx.pick("ente.txt", file.as_bytes());
    assert_eq!(pick.format, "otpauth-list");
    let preview = fx
        .app
        .import_preview(opts(&pick.handle, "otpauth-list", "merge"))
        .unwrap();
    assert_eq!((preview.entries, preview.duplicates), (1, 1), "{preview:?}");

    let out = fx
        .app
        .import_commit(opts(&pick.handle, "otpauth-list", "merge"))
        .unwrap();
    assert_eq!((out.added, out.appended, out.failed), (0, 1, 0), "{out:?}");

    // one item, not two: the login now has the code, and both notes
    assert_eq!(fx.titles(), ["GitHub", "Other"]);
    let merged = reveal(&fx, "GitHub");
    assert_eq!(merged.item_type, ItemType::Login);
    assert_eq!(merged.fields["password"], "bw-password");
    assert!(
        merged.fields["totp"].contains(SECRET),
        "{:?}",
        merged.fields["totp"]
    );
    let notes = &merged.fields["notes"];
    assert!(notes.contains("Recovery: abc-123"), "{notes}");
    assert!(notes.contains("Ente backup code: 1234-5678"), "{notes}");
    assert_eq!(merged.tags, ["work"]);
    // the history shows the merge as a new version of the same item
    assert_eq!(fx.app.history_list(&login.id).unwrap().len(), 2);
}

#[test]
fn a_login_from_a_password_manager_turns_the_code_item_of_the_same_account_into_one_login() {
    let fx = Fx::new();
    // the authenticator's file came first: a code item, with a note
    fx.import(
        "otpauth-list",
        "merge",
        "ente.txt",
        ente_line(
            "GitHub",
            "ana@example.com",
            SECRET,
            "Ente backup code: 1234-5678",
            "work",
        )
        .as_bytes(),
    );
    let code_item = fx.id_of("GitHub (ana@example.com)");
    assert_eq!(
        fx.app.item_get(&code_item).unwrap().item_type,
        ItemType::Totp
    );

    let json = bitwarden_login(
        "GitHub",
        "ana@example.com",
        "https://github.com/login",
        "Recovery: abc-123",
        None,
    );
    let pick = fx.pick("bitwarden.json", json.as_bytes());
    let preview = fx
        .app
        .import_preview(opts(&pick.handle, "bitwarden-json", "merge"))
        .unwrap();
    assert_eq!(preview.duplicates, 1, "{preview:?}");
    let out = fx
        .app
        .import_commit(opts(&pick.handle, "bitwarden-json", "merge"))
        .unwrap();
    assert_eq!((out.added, out.appended), (0, 1), "{out:?}");

    // the same item (same id, same history), now a login that has everything
    assert_eq!(fx.titles(), ["GitHub"]);
    assert_eq!(fx.id_of("GitHub"), code_item);
    let merged = reveal(&fx, "GitHub");
    assert_eq!(merged.item_type, ItemType::Login);
    assert_eq!(merged.fields["username"], "ana@example.com");
    assert_eq!(merged.fields["password"], "bw-password");
    assert_eq!(merged.fields["url"], "https://github.com/login");
    assert!(merged.fields["totp"].contains(SECRET));
    let notes = &merged.fields["notes"];
    assert!(
        notes.contains("Recovery: abc-123") && notes.contains("Ente backup code: 1234-5678"),
        "{notes}"
    );
    // the first version is still the code item, and can be restored as one
    let history = fx.app.history_list(&code_item).unwrap();
    assert_eq!(history.len(), 2);
    let first = fx.app.history_get(&code_item, 1, Some(PW.into())).unwrap();
    assert_eq!(first.item_type, ItemType::Totp);
    fx.app
        .history_restore(&code_item, 1, Some(PW.into()))
        .unwrap();
    assert_eq!(
        fx.app.item_get(&code_item).unwrap().item_type,
        ItemType::Totp,
        "restoring brings the old kind back with it"
    );
}

#[test]
fn the_same_code_secret_is_the_same_account_even_under_another_name() {
    let fx = Fx::new();
    fx.app
        .item_create(draft(
            ItemType::Login,
            "Work VPN",
            &[("username", "ana"), ("totp", SECRET)],
        ))
        .unwrap();
    let out = fx.import(
        "otpauth-list",
        "merge",
        "ente.txt",
        ente_line("Corp", "ana.lopez@corp.example", SECRET, "", "work").as_bytes(),
    );
    assert_eq!((out.added, out.appended), (0, 1), "{out:?}");
    assert_eq!(fx.titles(), ["Work VPN"]);
}

#[test]
fn two_accounts_of_one_service_are_not_mistaken_for_each_other() {
    let fx = Fx::new();
    fx.app
        .item_create(draft(
            ItemType::Login,
            "GitHub",
            &[
                ("username", "ana@example.com"),
                ("url", "https://github.com"),
            ],
        ))
        .unwrap();
    let out = fx.import(
        "otpauth-list",
        "merge",
        "ente.txt",
        ente_line("GitHub", "bob@example.com", "KRSXG5CTMVRXEZLU", "", "x").as_bytes(),
    );
    assert_eq!((out.added, out.appended), (1, 0), "{out:?}");
    assert_eq!(fx.titles().len(), 2);
}

#[test]
fn a_different_code_for_the_same_account_is_kept_in_a_hidden_field() {
    let fx = Fx::new();
    fx.app
        .item_create(draft(
            ItemType::Login,
            "GitHub",
            &[("username", "ana@example.com"), ("totp", SECRET)],
        ))
        .unwrap();
    let out = fx.import(
        "otpauth-list",
        "merge",
        "ente.txt",
        ente_line("GitHub", "ana@example.com", "KRSXG5CTMVRXEZLU", "", "x").as_bytes(),
    );
    assert_eq!((out.added, out.appended), (0, 1), "{out:?}");
    let merged = reveal(&fx, "GitHub");
    assert_eq!(
        merged.fields["totp"], SECRET,
        "the code that was here stays the code"
    );
    let extra = merged
        .custom_fields
        .iter()
        .find(|c| c.name == "One-time code (imported)")
        .expect("the other code is kept");
    assert!(extra.hidden && extra.value.contains("KRSXG5CTMVRXEZLU"));
}

#[test]
fn skipping_duplicates_does_not_reach_across_kinds_and_import_all_never_merges() {
    let fx = Fx::new();
    fx.app
        .item_create(draft(
            ItemType::Login,
            "GitHub",
            &[("username", "ana@example.com")],
        ))
        .unwrap();
    let line = ente_line("GitHub", "ana@example.com", SECRET, "n", "x");
    let out = fx.import("otpauth-list", "skip_duplicates", "a.txt", line.as_bytes());
    assert_eq!((out.added, out.skipped), (1, 0), "{out:?}");
    let out = fx.import("otpauth-list", "import_all", "b.txt", line.as_bytes());
    assert_eq!((out.added, out.appended), (1, 0), "{out:?}");
    assert_eq!(fx.titles().len(), 3);
}

#[test]
fn adding_as_a_new_version_no_longer_replaces_a_different_note() {
    let fx = Fx::new();
    let a = fx
        .app
        .item_create(draft(
            ItemType::Login,
            "Site",
            &[
                ("username", "ana"),
                ("url", "https://site.example"),
                ("notes", "old note"),
            ],
        ))
        .unwrap();
    let json = bitwarden_login(
        "Site",
        "ana",
        "https://site.example",
        "new note from the file",
        None,
    );
    let out = fx.import(
        "bitwarden-json",
        "append_version",
        "bw.json",
        json.as_bytes(),
    );
    assert_eq!((out.added, out.appended), (0, 1), "{out:?}");
    let merged = fx.app.item_reveal(&a.id, Some(PW.into())).unwrap();
    assert_eq!(merged.fields["notes"], "old note\n\nnew note from the file");
}

#[test]
fn merging_the_same_file_twice_changes_nothing_the_second_time() {
    let fx = Fx::new();
    fx.app
        .item_create(draft(
            ItemType::Login,
            "GitHub",
            &[("username", "ana@example.com"), ("notes", "mine")],
        ))
        .unwrap();
    let line = ente_line("GitHub", "ana@example.com", SECRET, "theirs", "work");
    let first = fx.import("otpauth-list", "merge", "a.txt", line.as_bytes());
    assert_eq!(first.appended, 1);
    let second = fx.import("otpauth-list", "merge", "b.txt", line.as_bytes());
    assert_eq!(
        (second.added, second.appended, second.failed),
        (0, 0, 0),
        "{second:?}"
    );
    assert_eq!(reveal(&fx, "GitHub").fields["notes"], "mine\n\ntheirs");
}
