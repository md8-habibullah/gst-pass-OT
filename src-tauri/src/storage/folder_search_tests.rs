//! Folders, the search inside items, and the rule for erasing a trashed item for good, through the same
//! [`Session`] the Tauri commands use.

use super::model::{CustomField, ItemDraft, ItemType};
use super::session::{Session, SessionConfig, EDIT_TOKEN_TTL};
use super::Result;
use crate::crypto::KdfParams;
use std::collections::BTreeMap;
use std::time::Duration;

const PW: &str = "correct horse battery staple";
const WRONG: &str = "definitely not the password";

fn cfg(grace: Duration) -> SessionConfig {
    SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: EDIT_TOKEN_TTL,
        purge_grace: grace,
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    s: Session,
}

impl Fx {
    fn new() -> Self {
        Self::with_grace(Duration::ZERO)
    }

    fn with_grace(grace: Duration) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let s = Session::new(dir.path().join("vault.db"), cfg(grace));
        s.create(PW.into()).unwrap();
        Self { _dir: dir, s }
    }

    fn add(&self, item_type: ItemType, title: &str, fields: &[(&str, &str)]) -> String {
        self.s
            .item_create(draft(item_type, title, fields))
            .unwrap()
            .id
    }

    fn folder_of(&self, id: &str) -> Option<String> {
        self.s
            .item_list()
            .unwrap()
            .into_iter()
            .find(|i| i.id == id)
            .unwrap()
            .folder
    }

    fn edit(&self, id: &str, f: impl FnOnce(&mut ItemDraft)) {
        let b = self.s.item_begin_edit(id, Some(PW.into())).unwrap();
        let mut d = b.draft.clone();
        f(&mut d);
        self.s.item_update(id, &b.token, b.version, d).unwrap();
    }
}

fn draft(item_type: ItemType, title: &str, fields: &[(&str, &str)]) -> ItemDraft {
    ItemDraft {
        item_type,
        title: title.into(),
        favorite: false,
        reprompt: false,
        tags: Vec::new(),
        folder: None,
        fields: fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        custom_fields: Vec::new(),
    }
}

fn code<T>(r: Result<T>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(e) => e.code(),
    }
}

fn in_folder(fx: &Fx, title: &str, folder: &str) -> String {
    let mut d = draft(ItemType::Login, title, &[("username", "u")]);
    d.folder = Some(folder.into());
    fx.s.item_create(d).unwrap().id
}

// ------------------------------------------------------------------ folders

#[test]
fn a_folder_can_be_made_empty_and_is_kept_with_the_preferences() {
    let fx = Fx::new();
    let (path, prefs) = fx.s.folder_create(" Work // Clients ").unwrap();
    assert_eq!(path, "Work/Clients");
    assert_eq!(prefs.folders, ["Work/Clients"]);
    assert_eq!(fx.s.prefs_get().unwrap().folders, ["Work/Clients"]);

    // the same folder in another capitalisation is the one that exists
    let (again, prefs) = fx.s.folder_create("work/CLIENTS").unwrap();
    assert_eq!(again, "Work/Clients");
    assert_eq!(prefs.folders, ["Work/Clients"]);
}

#[test]
fn a_folder_that_items_sit_in_keeps_its_spelling_for_a_new_folder_too() {
    let fx = Fx::new();
    in_folder(&fx, "a", "Home/Bills");
    let (path, _) = fx.s.folder_create("home/bills").unwrap();
    assert_eq!(path, "Home/Bills");
}

#[test]
fn folders_have_limits_and_say_so() {
    let fx = Fx::new();
    for bad in [
        "",
        "   ",
        " / / ",
        "a/b/c/d/e/f/g",
        &"x".repeat(65),
        "bell\u{7}",
        &["a".repeat(60), "b".repeat(60), "c".repeat(60)].join("/"),
    ] {
        assert_eq!(code(fx.s.folder_create(bad)), "invalid_input", "{bad:?}");
    }
    assert!(fx.s.prefs_get().unwrap().folders.is_empty());
    // the largest allowed ones work
    fx.s.folder_create(&"x".repeat(64)).unwrap();
    fx.s.folder_create("a/b/c/d/e/f").unwrap();
}

#[test]
fn moving_an_item_writes_a_version_and_keeps_the_history() {
    let fx = Fx::new();
    let id = fx.add(ItemType::Login, "GitHub", &[("username", "me")]);
    assert_eq!(
        fx.s.item_move(std::slice::from_ref(&id), Some("Work"))
            .unwrap(),
        1
    );
    assert_eq!(fx.folder_of(&id).as_deref(), Some("Work"));
    let history = fx.s.history_list(&id).unwrap();
    assert_eq!(history.len(), 2, "the move is a version of its own");
    assert!(history
        .iter()
        .any(|h| h.changed_fields == ["folder".to_string()]));

    // moving to where it is already writes nothing
    assert_eq!(
        fx.s.item_move(std::slice::from_ref(&id), Some(" Work "))
            .unwrap(),
        0
    );
    assert_eq!(fx.s.history_list(&id).unwrap().len(), 2);

    // and out of every folder
    assert_eq!(fx.s.item_move(std::slice::from_ref(&id), None).unwrap(), 1);
    assert_eq!(fx.folder_of(&id), None);
    assert_eq!(
        fx.s.item_move(std::slice::from_ref(&id), Some("   "))
            .unwrap(),
        0
    );
}

#[test]
fn several_items_move_at_once_and_a_bad_id_moves_none() {
    let fx = Fx::new();
    let a = fx.add(ItemType::Login, "A", &[]);
    let b = fx.add(ItemType::SecureNote, "B", &[("body", "x")]);
    assert_eq!(
        fx.s.item_move(&[a.clone(), b.clone()], Some("Both"))
            .unwrap(),
        2
    );
    assert_eq!(fx.folder_of(&a).as_deref(), Some("Both"));
    assert_eq!(fx.folder_of(&b).as_deref(), Some("Both"));

    let missing = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string();
    assert_eq!(
        code(fx.s.item_move(&[a.clone(), missing], Some("Elsewhere"))),
        "not_found"
    );
    assert_eq!(fx.folder_of(&a).as_deref(), Some("Both"), "nothing moved");
    assert_eq!(
        code(fx.s.item_move(&["not an id".into()], None)),
        "invalid_input"
    );
}

#[test]
fn moving_needs_no_password_even_for_an_item_that_asks_for_one() {
    let fx = Fx::new();
    let mut d = draft(ItemType::Login, "Bank", &[("password", "s3cret")]);
    d.reprompt = true;
    let id = fx.s.item_create(d).unwrap().id;
    assert_eq!(
        fx.s.item_move(std::slice::from_ref(&id), Some("Money"))
            .unwrap(),
        1
    );
    assert_eq!(fx.folder_of(&id).as_deref(), Some("Money"));
}

#[test]
fn a_trashed_item_cannot_be_moved() {
    let fx = Fx::new();
    let id = fx.add(ItemType::Login, "Old", &[]);
    fx.s.item_trash(&id).unwrap();
    assert_eq!(code(fx.s.item_move(&[id], Some("X"))), "in_trash");
}

#[test]
fn renaming_a_folder_takes_everything_inside_it_along() {
    let fx = Fx::new();
    let a = in_folder(&fx, "a", "Work");
    let b = in_folder(&fx, "b", "Work/Clients");
    let c = in_folder(&fx, "c", "Home");
    fx.s.folder_create("Work/Empty").unwrap();

    let (moved, prefs) = fx.s.folder_rename("Work", "Job").unwrap();
    assert_eq!(moved, 2);
    assert_eq!(fx.folder_of(&a).as_deref(), Some("Job"));
    assert_eq!(fx.folder_of(&b).as_deref(), Some("Job/Clients"));
    assert_eq!(fx.folder_of(&c).as_deref(), Some("Home"));
    assert_eq!(prefs.folders, ["Job/Empty"], "the empty folder came along");
}

#[test]
fn renaming_into_a_name_that_exists_merges_the_two() {
    let fx = Fx::new();
    let a = in_folder(&fx, "a", "Old");
    let b = in_folder(&fx, "b", "New");
    fx.s.folder_rename("Old", "new").unwrap();
    assert_eq!(
        fx.folder_of(&a).as_deref(),
        Some("New"),
        "the existing spelling wins"
    );
    assert_eq!(fx.folder_of(&b).as_deref(), Some("New"));
}

#[test]
fn only_the_capitalisation_of_a_folder_can_change() {
    let fx = Fx::new();
    let a = in_folder(&fx, "a", "work");
    fx.s.folder_rename("work", "Work").unwrap();
    assert_eq!(fx.folder_of(&a).as_deref(), Some("Work"));
}

#[test]
fn a_folder_cannot_be_moved_inside_itself_and_must_exist() {
    let fx = Fx::new();
    in_folder(&fx, "a", "Work");
    assert_eq!(
        code(fx.s.folder_rename("Work", "Work/Sub")),
        "invalid_input"
    );
    assert_eq!(code(fx.s.folder_rename("Nowhere", "Else")), "not_found");
    assert_eq!(code(fx.s.folder_rename("Work", "  ")), "invalid_input");
    assert_eq!(code(fx.s.folder_remove("Nowhere")), "not_found");
    assert_eq!(fx.s.folder_rename("Work", "Work").unwrap().0, 0);
}

#[test]
fn moving_a_folder_into_another_keeps_its_structure() {
    let fx = Fx::new();
    let a = in_folder(&fx, "a", "Clients/Acme");
    fx.s.folder_create("Work").unwrap();
    fx.s.folder_rename("Clients", "Work/Clients").unwrap();
    assert_eq!(fx.folder_of(&a).as_deref(), Some("Work/Clients/Acme"));
}

#[test]
fn removing_a_folder_moves_what_was_inside_up_one_level() {
    let fx = Fx::new();
    let a = in_folder(&fx, "a", "Work/Clients");
    let b = in_folder(&fx, "b", "Work/Clients/Acme");
    let c = in_folder(&fx, "c", "Work");
    fx.s.folder_create("Work/Clients/Empty").unwrap();

    let (moved, prefs) = fx.s.folder_remove("Work/Clients").unwrap();
    assert_eq!(moved, 2);
    assert_eq!(fx.folder_of(&a).as_deref(), Some("Work"));
    assert_eq!(fx.folder_of(&b).as_deref(), Some("Work/Acme"));
    assert_eq!(fx.folder_of(&c).as_deref(), Some("Work"));
    assert!(prefs.folders.iter().all(|f| !f.starts_with("Work/Clients")));
    assert!(prefs.folders.contains(&"Work/Empty".to_string()));

    // a top-level folder: its items come out of every folder
    fx.s.folder_remove("Work").unwrap();
    assert_eq!(fx.folder_of(&a), None);
    assert_eq!(fx.folder_of(&b).as_deref(), Some("Acme"));
}

#[test]
fn an_edit_that_names_a_folder_is_cleaned_like_one_made_here() {
    let fx = Fx::new();
    let id = fx.add(ItemType::Login, "A", &[]);
    fx.edit(&id, |d| d.folder = Some(" Work // Clients ".into()));
    assert_eq!(fx.folder_of(&id).as_deref(), Some("Work/Clients"));
}

// ------------------------------------------------------------------ search

fn hits(fx: &Fx, q: &str, old: bool) -> Vec<(String, Vec<String>, Option<u64>)> {
    fx.s.item_search(q, old)
        .unwrap()
        .into_iter()
        .map(|h| (h.id, h.matched, h.old_version))
        .collect()
}

#[test]
fn search_finds_by_any_field_and_says_which() {
    let fx = Fx::new();
    let gh = fx.add(
        ItemType::Login,
        "GitHub",
        &[
            ("username", "me@example.com"),
            ("url", "https://github.com\nhttps://gist.github.com"),
            ("password", "hunter2-correct-horse"),
            ("notes", "recovery email is the old one"),
        ],
    );
    fx.add(
        ItemType::Login,
        "Other",
        &[("username", "someone@else.org")],
    );

    let r = hits(&fx, "gist.github.com", false);
    assert_eq!(r.len(), 1);
    assert_eq!((&r[0].0, &r[0].1), (&gh, &vec!["Website".to_string()]));
    assert_eq!(hits(&fx, "recovery email", false)[0].1, ["Notes"]);
    assert_eq!(hits(&fx, "EXAMPLE.com github", false).len(), 1);
    assert_eq!(hits(&fx, "nothing like this", false).len(), 0);
    // a password is never compared
    assert!(hits(&fx, "hunter2", false).is_empty());
    assert!(hits(&fx, "correct-horse", false).is_empty());
}

#[test]
fn search_reads_custom_fields_tags_and_folders() {
    let fx = Fx::new();
    let mut d = draft(ItemType::Login, "Router", &[]);
    d.tags = vec!["network".into()];
    d.folder = Some("Home/Devices".into());
    d.custom_fields = vec![
        CustomField {
            name: "Admin page".into(),
            value: "192.168.1.1".into(),
            hidden: false,
        },
        CustomField {
            name: "Recovery".into(),
            value: "violet-meadow-9".into(),
            hidden: true,
        },
    ];
    let id = fx.s.item_create(d).unwrap().id;
    for q in [
        "network",
        "devices",
        "admin page",
        "192.168.1.1",
        "recovery",
    ] {
        assert_eq!(hits(&fx, q, false).len(), 1, "{q}");
    }
    assert!(
        hits(&fx, "violet-meadow", false).is_empty(),
        "a hidden value stays out"
    );
    assert_eq!(hits(&fx, "router devices network", false)[0].0, id);
}

#[test]
fn old_versions_are_searched_only_when_asked() {
    let fx = Fx::new();
    let id = fx.add(ItemType::Login, "Alpha Bank", &[("username", "u")]);
    fx.edit(&id, |d| d.title = "Beta Bank".into());
    fx.edit(&id, |d| d.title = "Gamma Bank".into());

    assert!(hits(&fx, "alpha", false).is_empty());
    let r = hits(&fx, "alpha", true);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0, id);
    assert_eq!(r[0].2, Some(1), "version 1 is where it matched");
    assert_eq!(hits(&fx, "beta", true)[0].2, Some(2));
    // an item whose current text matches is reported once, as a current match
    let now = hits(&fx, "gamma", true);
    assert_eq!(now.len(), 1);
    assert_eq!(now[0].2, None);
    assert_eq!(hits(&fx, "bank", true).len(), 1);
}

#[test]
fn trashed_items_are_not_searched_at_all() {
    let fx = Fx::new();
    let id = fx.add(ItemType::Login, "Hidden gem", &[("username", "u")]);
    assert_eq!(hits(&fx, "gem", true).len(), 1);
    fx.s.item_trash(&id).unwrap();
    assert!(hits(&fx, "gem", true).is_empty());
    fx.s.item_restore(&id).unwrap();
    assert_eq!(hits(&fx, "gem", false).len(), 1);
}

#[test]
fn an_item_that_asks_again_keeps_its_notes_out_of_search_old_versions_included() {
    let fx = Fx::new();
    let mut d = draft(
        ItemType::SecureNote,
        "Safe",
        &[("body", "combination thirty-one")],
    );
    d.reprompt = true;
    let id = fx.s.item_create(d).unwrap().id;
    assert!(hits(&fx, "combination", false).is_empty());
    assert!(hits(&fx, "combination", true).is_empty());
    assert_eq!(hits(&fx, "safe", false).len(), 1, "its title is findable");

    // the same note without the flag is found
    let open = fx.add(
        ItemType::SecureNote,
        "Shed",
        &[("body", "combination thirty-one")],
    );
    assert_eq!(hits(&fx, "combination", false)[0].0, open);
    let _ = id;
}

#[test]
fn an_empty_query_finds_nothing_and_a_huge_one_is_refused() {
    let fx = Fx::new();
    fx.add(ItemType::Login, "A", &[]);
    assert!(hits(&fx, "   ", false).is_empty());
    assert_eq!(
        code(fx.s.item_search(&"a".repeat(201), false)),
        "invalid_input"
    );
    assert_eq!(code(fx.s.item_search(&"a".repeat(200), false)), "ok");
}

#[test]
fn search_needs_an_open_vault() {
    let fx = Fx::new();
    fx.s.lock();
    assert_eq!(code(fx.s.item_search("a", false)), "locked");
    assert_eq!(code(fx.s.folder_create("A")), "locked");
    assert_eq!(code(fx.s.item_move(&[], None)), "locked");
}

// ------------------------------------------------------------------ erasing for good

#[test]
fn an_old_item_needs_the_password_and_nothing_else() {
    // a grace of one millisecond: the item below is "old" by the time it is looked at
    let fx = Fx::with_grace(Duration::from_millis(1));
    let id = fx.add(ItemType::Login, "Old", &[]);
    fx.s.item_trash(&id).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let plan = fx.s.item_purge_plan(&id).unwrap();
    assert!(!plan.recent);
    assert!(plan.sum.is_none());
    assert_eq!(
        code(fx.s.item_purge(&id, WRONG.into(), "")),
        "wrong_password"
    );
    assert_eq!(fx.s.trash_list().unwrap().len(), 1);
    fx.s.item_purge(&id, PW.into(), "").unwrap();
    assert!(fx.s.trash_list().unwrap().is_empty());
}

#[test]
fn a_recently_trashed_item_needs_the_password_and_the_sum() {
    let fx = Fx::with_grace(Duration::from_secs(100 * 24 * 3600));
    let id = fx.add(ItemType::Login, "Fresh", &[("username", "u")]);
    // not in the trash yet
    assert_eq!(code(fx.s.item_purge_plan(&id)), "not_in_trash");
    fx.s.item_trash(&id).unwrap();

    let plan = fx.s.item_purge_plan(&id).unwrap();
    assert!(plan.recent);
    let sum = plan.sum.clone().expect("a sum is made for a recent item");
    assert!(sum.len() > 12, "a long sum: {sum}");
    assert_eq!(plan.older_at, plan.deleted_at + 100 * 24 * 3600 * 1000);
    let answer = fx.s.test_purge_answer().unwrap();

    // the wrong password does not use the sum up
    assert_eq!(
        code(fx.s.item_purge(&id, WRONG.into(), &answer.to_string())),
        "wrong_password"
    );
    assert_eq!(fx.s.test_purge_answer(), Some(answer));
    // the right password but no answer, or a wrong one, is refused and uses the sum up
    assert_eq!(code(fx.s.item_purge(&id, PW.into(), "")), "wrong_answer");
    assert_eq!(fx.s.test_purge_answer(), None);
    assert_eq!(fx.s.trash_list().unwrap().len(), 1, "still in the trash");

    // a sum worked out earlier is worthless once a new one was made
    let plan2 = fx.s.item_purge_plan(&id).unwrap();
    assert!(plan2.sum.is_some());
    let answer2 = fx.s.test_purge_answer().unwrap();
    assert_eq!(
        code(fx.s.item_purge(&id, PW.into(), &(answer2.wrapping_add(1)).to_string())),
        "wrong_answer"
    );
    // letters, signs and huge numbers are wrong answers, not crashes
    for junk in ["abc", "-5", "99999999999999999999", "1e3", "  "] {
        fx.s.item_purge_plan(&id).unwrap();
        assert_eq!(
            code(fx.s.item_purge(&id, PW.into(), junk)),
            "wrong_answer",
            "{junk:?}"
        );
    }

    fx.s.item_purge_plan(&id).unwrap();
    let right = fx.s.test_purge_answer().unwrap();
    fx.s.item_purge(&id, PW.into(), &format!(" {right} "))
        .unwrap();
    assert!(fx.s.trash_list().unwrap().is_empty());
    assert_eq!(code(fx.s.item_get(&id)), "not_found");
}

#[test]
fn the_sum_belongs_to_one_item() {
    let fx = Fx::with_grace(Duration::from_secs(100 * 24 * 3600));
    let a = fx.add(ItemType::Login, "A", &[]);
    let b = fx.add(ItemType::Login, "B", &[]);
    fx.s.item_trash(&a).unwrap();
    fx.s.item_trash(&b).unwrap();
    fx.s.item_purge_plan(&a).unwrap();
    let answer_for_a = fx.s.test_purge_answer().unwrap();
    // the right answer, for the wrong item
    assert_eq!(
        code(fx.s.item_purge(&b, PW.into(), &answer_for_a.to_string())),
        "wrong_answer"
    );
    assert_eq!(fx.s.trash_list().unwrap().len(), 2);
}

#[test]
fn a_sum_is_not_made_for_an_item_that_is_not_in_the_trash_or_does_not_exist() {
    let fx = Fx::with_grace(Duration::from_secs(100 * 24 * 3600));
    let id = fx.add(ItemType::Login, "A", &[]);
    assert_eq!(code(fx.s.item_purge_plan(&id)), "not_in_trash");
    assert_eq!(
        code(fx.s.item_purge_plan("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")),
        "not_found"
    );
    assert_eq!(code(fx.s.item_purge_plan("nope")), "invalid_input");
    assert_eq!(fx.s.test_purge_answer(), None);
}

#[test]
fn the_sum_is_gone_when_the_vault_locks() {
    let fx = Fx::with_grace(Duration::from_secs(100 * 24 * 3600));
    let id = fx.add(ItemType::Login, "A", &[]);
    fx.s.item_trash(&id).unwrap();
    fx.s.item_purge_plan(&id).unwrap();
    assert!(fx.s.test_purge_answer().is_some());
    fx.s.lock();
    fx.s.unlock(PW.into()).unwrap();
    assert_eq!(fx.s.test_purge_answer(), None);
}
