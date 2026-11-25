//! Behavioural tests of the account layer ([`AppCore`]): several vaults in one app, archive and
//! clone, new item types, vault files in and out, and the on-disk guarantees. Everything runs in
//! temp folders; the user's real data folder is never touched.

use super::app::AppCore;
use super::db::Store;
use super::files::TestPicker;
use super::model::{CustomField, ItemDraft, ItemEvent, ItemType};
use super::session::{Session, SessionConfig, EDIT_TOKEN_TTL};
use super::uuid;
use super::{Result, StorageError};
use crate::crypto::KdfParams;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PW: &str = "correct horse battery staple";
const PW2: &str = "a second long master password";
const WRONG: &str = "definitely not the password";

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
    dir: tempfile::TempDir,
    picker: Arc<TestPicker>,
    app: AppCore,
}

impl Fx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let picker = Arc::new(TestPicker::default());
        let app = AppCore::open(dir.path(), fast(), picker.clone()).unwrap();
        Self { dir, picker, app }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Simulate quitting and starting the app again.
    fn restart(&mut self) {
        self.app.lock();
        self.app = AppCore::open(self.dir.path(), fast(), self.picker.clone()).unwrap();
    }

    /// An empty scratch folder outside the data folder (for export targets and the like).
    fn outside(&self) -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn names(&self) -> Vec<String> {
        self.app
            .profile_list()
            .unwrap()
            .profiles
            .into_iter()
            .map(|p| p.display_name)
            .collect()
    }

    fn id_of(&self, name: &str) -> String {
        self.app
            .profile_list()
            .unwrap()
            .profiles
            .into_iter()
            .find(|p| p.display_name == name)
            .unwrap_or_else(|| panic!("no profile {name}"))
            .id
    }

    fn device_id(&self) -> String {
        self.app
            .test_session()
            .test_vault(|v| v.store().device_id().unwrap().unwrap())
    }

    /// Sorted names of everything in the data folder (one level).
    fn listing(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.root())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
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

fn login(title: &str, password: &str) -> ItemDraft {
    draft(
        ItemType::Login,
        title,
        &[("username", "ana@example.com"), ("password", password)],
    )
}

fn card(title: &str) -> ItemDraft {
    draft(
        ItemType::Card,
        title,
        &[
            ("cardholder", "Ana Example"),
            ("brand", "Visa"),
            ("number", "4111 1111 1111 1111"),
            ("expMonth", "09"),
            ("expYear", "2029"),
            ("code", "737"),
        ],
    )
}

fn edit(app: &AppCore, id: &str, f: impl FnOnce(&mut ItemDraft)) -> super::ItemSummary {
    let b = app.item_begin_edit(id, Some(PW.into())).unwrap();
    let mut d = b.draft.clone();
    f(&mut d);
    app.item_update(id, &b.token, b.version, d).unwrap()
}

fn bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
}

// ============================================================ first run and creation

#[test]
fn a_fresh_app_has_no_accounts_and_nothing_is_unlocked() {
    let fx = Fx::new();
    let list = fx.app.profile_list().unwrap();
    assert!(list.profiles.is_empty());
    assert_eq!(list.selected_id, None);
    assert_eq!(list.last_used_id, None);
    let st = fx.app.status().unwrap();
    assert!(!st.exists && !st.unlocked && st.profile.is_none());
    assert_eq!(code(fx.app.item_list()), "locked");
    assert_eq!(code(fx.app.unlock(PW.into())), "no_vault");
    assert_eq!(code(fx.app.profile_select("nope")), "not_found");
    assert!(
        !fx.root().join("profiles.json").exists() && !fx.root().join("vaults").exists(),
        "looking around must not create anything"
    );
}

#[test]
fn creating_an_account_selects_and_unlocks_it() {
    let fx = Fx::new();
    let info = fx.app.profile_create("  Personal ", PW.into()).unwrap();
    assert_eq!(info.display_name, "Personal");
    assert!(info.last_used_at.is_some());
    let st = fx.app.status().unwrap();
    assert!(st.exists && st.unlocked);
    assert_eq!(st.profile.unwrap().id, info.id);
    let list = fx.app.profile_list().unwrap();
    assert_eq!(list.selected_id.as_deref(), Some(info.id.as_str()));
    assert_eq!(list.last_used_id.as_deref(), Some(info.id.as_str()));
    assert!(fx
        .root()
        .join("vaults")
        .join(&info.id)
        .join("vault.db")
        .is_file());
    assert!(fx.root().join("profiles.json").is_file());
    fx.app.item_create(login("GitHub", "pw-1")).unwrap();
    assert_eq!(fx.app.item_list().unwrap().len(), 1);
}

#[test]
fn account_names_are_normalized_and_validated() {
    let fx = Fx::new();
    let too_long = "n".repeat(41);
    for bad in [
        "",
        "   ",
        "x\u{200B}y",
        "a\u{202E}b",
        too_long.as_str(),
        "tab\there",
    ] {
        assert_eq!(
            code(fx.app.profile_create(bad, PW.into())),
            "invalid_input",
            "{bad:?}"
        );
    }
    assert!(fx.listing().is_empty(), "a bad name creates nothing");
    let info = fx.app.profile_create("Ａna   Ｌee", PW.into()).unwrap();
    assert_eq!(info.display_name, "Ana Lee", "NFKC + whitespace collapse");
}

#[test]
fn names_are_unique_across_case_width_and_unicode_form() {
    let fx = Fx::new();
    fx.app.profile_create("Zoë", PW.into()).unwrap(); // precomposed
    for dup in [
        "zoë",
        "ZOË",
        "Zoe\u{0308}", // decomposed
        "ZOE\u{0308}", // decomposed, upper case
        "  Zoë  ",
        "Ｚｏë", // fullwidth letters
    ] {
        assert_eq!(
            code(fx.app.profile_create(dup, PW2.into())),
            "profile_name_taken",
            "{dup:?}"
        );
    }
    assert_eq!(fx.names(), ["Zoë"]);
    // the duplicate attempts left no folders behind
    let vaults: Vec<_> = std::fs::read_dir(fx.root().join("vaults"))
        .unwrap()
        .collect();
    assert_eq!(vaults.len(), 1);
    // a genuinely different name is fine
    fx.app.profile_create("Zoë 2", PW2.into()).unwrap();
}

#[test]
fn a_failed_create_leaves_nothing_behind() {
    // weak password
    let fx = Fx::new();
    assert_eq!(
        code(fx.app.profile_create("Work", "short".into())),
        "invalid_input"
    );
    assert!(fx.listing().is_empty());

    // the folder cannot be made: a plain file named `vaults` is in the way (and must survive)
    let fx = Fx::new();
    std::fs::write(fx.root().join("vaults"), b"not a folder").unwrap();
    assert_eq!(code(fx.app.profile_create("Work", PW.into())), "storage");
    assert_eq!(fx.listing(), ["vaults"]);
    assert_eq!(bytes(&fx.root().join("vaults")), b"not a folder");
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());

    // the account list cannot be saved: the vault just made is removed again
    let fx = Fx::new();
    std::fs::create_dir(fx.root().join("profiles.json.tmp")).unwrap();
    assert_eq!(code(fx.app.profile_create("Work", PW.into())), "storage");
    assert_eq!(
        fx.listing(),
        ["profiles.json.tmp"],
        "no vault folder, no registry"
    );
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());
    assert_eq!(code(fx.app.item_list()), "locked");
    // and once the obstacle is gone, the same name works (nothing is half-registered)
    std::fs::remove_dir(fx.root().join("profiles.json.tmp")).unwrap();
    fx.app.profile_create("Work", PW.into()).unwrap();
}

#[test]
fn a_failed_create_keeps_the_previously_selected_account() {
    let fx = Fx::new();
    fx.app.profile_create("First", PW.into()).unwrap();
    fx.app.item_create(login("kept", "pw-1")).unwrap();
    assert_eq!(
        code(fx.app.profile_create("Second", "short".into())),
        "invalid_input"
    );
    assert_eq!(
        code(fx.app.profile_create("first", PW2.into())),
        "profile_name_taken"
    );
    // still selected, still unlocked, still intact
    assert_eq!(fx.app.item_list().unwrap().len(), 1);
    assert_eq!(
        fx.app.status().unwrap().profile.unwrap().display_name,
        "First"
    );
}

#[test]
fn two_creates_with_the_same_name_cannot_both_win() {
    let fx = Fx::new();
    let app = Arc::new(fx.app);
    let results: Vec<Result<_>> = std::thread::scope(|s| {
        let handles: Vec<_> = [PW, PW2]
            .into_iter()
            .map(|pw| {
                let app = Arc::clone(&app);
                s.spawn(move || app.profile_create("Twin", pw.into()))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let wins = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(wins, 1, "{results:?}");
    let loser = results.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert_eq!(loser.code(), "profile_name_taken");
    assert_eq!(app.profile_list().unwrap().profiles.len(), 1);
    let folders = std::fs::read_dir(fx.dir.path().join("vaults"))
        .unwrap()
        .count();
    assert_eq!(folders, 1, "the loser's folder was cleaned up");
}

// ============================================================ isolation between accounts

#[test]
fn accounts_are_isolated_from_each_other() {
    let fx = Fx::new();
    let work = fx.app.profile_create("Work", PW.into()).unwrap();
    fx.app.item_create(login("Work secret", "work-pw")).unwrap();
    let old_session = fx.app.test_session();

    let home = fx.app.profile_create("Home", PW2.into()).unwrap();
    // creating Home wiped Work from memory
    assert!(
        !old_session.status().unwrap().unlocked,
        "the old vault was locked"
    );
    assert!(fx.app.item_list().unwrap().is_empty());
    fx.app.item_create(login("Home secret", "home-pw")).unwrap();

    // Work's password never opens Home, and Home's never opens Work
    fx.app.profile_select(&home.id).unwrap();
    assert_eq!(code(fx.app.item_list()), "locked");
    assert_eq!(code(fx.app.unlock(PW.into())), "wrong_password");
    assert_eq!(code(fx.app.unlock(WRONG.into())), "wrong_password");
    fx.app.unlock(PW2.into()).unwrap();
    let titles: Vec<_> = fx
        .app
        .item_list()
        .unwrap()
        .into_iter()
        .map(|i| i.title)
        .collect();
    assert_eq!(titles, ["Home secret"]);
    let home_session = fx.app.test_session();

    // selecting wipes the unlocked one; the other still needs its own password
    fx.app.profile_select(&work.id).unwrap();
    assert!(!home_session.status().unwrap().unlocked);
    assert_eq!(code(fx.app.item_list()), "locked");
    assert_eq!(code(fx.app.unlock(PW2.into())), "wrong_password");
    fx.app.unlock(PW.into()).unwrap();
    let titles: Vec<_> = fx
        .app
        .item_list()
        .unwrap()
        .into_iter()
        .map(|i| i.title)
        .collect();
    assert_eq!(titles, ["Work secret"]);

    // no ids in common, and the files are separate
    let work_dir = fx.root().join("vaults").join(&work.id);
    let home_dir = fx.root().join("vaults").join(&home.id);
    assert_ne!(
        bytes(&work_dir.join("vault.db")),
        bytes(&home_dir.join("vault.db"))
    );

    // item ids of one account do not resolve in the other
    let work_item = fx.app.item_list().unwrap()[0].id.clone();
    fx.app.profile_select(&home.id).unwrap();
    fx.app.unlock(PW2.into()).unwrap();
    assert_eq!(code(fx.app.item_get(&work_item)), "not_found");
    assert_eq!(
        code(fx.app.item_reveal(&work_item, Some(PW2.into()))),
        "not_found"
    );
}

#[test]
fn lock_and_select_wipe_edit_tokens_and_unlocked_state() {
    let fx = Fx::new();
    let a = fx.app.profile_create("A", PW.into()).unwrap();
    let item = fx.app.item_create(login("x", "pw-1")).unwrap();
    let b = fx.app.item_begin_edit(&item.id, Some(PW.into())).unwrap();
    fx.app.lock();
    assert_eq!(code(fx.app.item_list()), "locked");
    assert!(fx.app.status().unwrap().exists);
    fx.app.unlock(PW.into()).unwrap();
    // the edit token did not survive the lock
    assert_eq!(
        code(
            fx.app
                .item_update(&item.id, &b.token, b.version, b.draft.clone())
        ),
        "token_invalid"
    );
    // nor does it survive a switch
    let b = fx.app.item_begin_edit(&item.id, Some(PW.into())).unwrap();
    fx.app.profile_create("B", PW2.into()).unwrap();
    fx.app.profile_select(&a.id).unwrap();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(
        code(fx.app.item_update(&item.id, &b.token, b.version, b.draft)),
        "token_invalid"
    );
}

#[test]
fn password_change_affects_only_the_selected_account() {
    let fx = Fx::new();
    fx.app.profile_create("A", PW.into()).unwrap();
    let a = fx.id_of("A");
    fx.app.profile_create("B", PW.into()).unwrap(); // the very same password on purpose
    let b = fx.id_of("B");
    fx.app
        .change_password(PW.into(), "brand new master password".into())
        .unwrap();
    fx.app.lock();
    assert_eq!(code(fx.app.unlock(PW.into())), "wrong_password");
    fx.app.unlock("brand new master password".into()).unwrap();
    fx.app.profile_select(&a).unwrap();
    assert_eq!(
        code(fx.app.unlock("brand new master password".into())),
        "wrong_password"
    );
    fx.app.unlock(PW.into()).unwrap();
    // two accounts with the same password still have unrelated key material
    let ka = bytes(&fx.root().join("vaults").join(&a).join("vault.db"));
    let kb = bytes(&fx.root().join("vaults").join(&b).join("vault.db"));
    assert_ne!(ka, kb);
}

// ============================================================ rename and remove

#[test]
fn rename_needs_the_selected_unlocked_account_and_a_free_name() {
    let fx = Fx::new();
    let a = fx.app.profile_create("Alpha", PW.into()).unwrap();
    let b = fx.app.profile_create("Beta", PW2.into()).unwrap();
    // Beta is selected and unlocked
    let r = fx.app.profile_rename(&b.id, "  Gamma  ").unwrap();
    assert_eq!(r.display_name, "Gamma");
    assert_eq!(
        code(fx.app.profile_rename(&b.id, "alpha")),
        "profile_name_taken"
    );
    assert_eq!(code(fx.app.profile_rename(&b.id, "")), "invalid_input");
    // renaming to the same name with other case is allowed (it is the same account)
    assert_eq!(
        fx.app.profile_rename(&b.id, "GAMMA").unwrap().display_name,
        "GAMMA"
    );
    // not the selected account
    assert_eq!(code(fx.app.profile_rename(&a.id, "Delta")), "locked");
    assert_eq!(code(fx.app.profile_rename("nope", "Delta")), "not_found");
    // selected but locked
    fx.app.lock();
    assert_eq!(code(fx.app.profile_rename(&b.id, "Delta")), "locked");
    assert_eq!(fx.names().len(), 2);
    // the new name is persisted
    let mut fx = fx;
    fx.restart();
    let mut names = fx.names();
    names.sort();
    assert_eq!(names, ["Alpha", "GAMMA"]);
}

#[test]
fn removing_needs_the_accounts_own_password_and_a_confirmation() {
    let fx = Fx::new();
    let a = fx.app.profile_create("Alpha", PW.into()).unwrap();
    fx.app.item_create(login("alpha item", "pw-a")).unwrap();
    let b = fx.app.profile_create("Beta", PW2.into()).unwrap();
    fx.app.item_create(login("beta item", "pw-b")).unwrap();
    let beta_dir = fx.root().join("vaults").join(&b.id);
    let alpha_dir = fx.root().join("vaults").join(&a.id);
    let beta_before = bytes(&beta_dir.join("vault.db"));

    // Alpha is not selected; it still needs *its* password, even though Beta's is unlocked
    assert_eq!(
        code(fx.app.profile_remove(&a.id, PW2.into(), "Alpha")),
        "wrong_password"
    );
    assert!(alpha_dir.exists());
    assert_eq!(
        code(fx.app.profile_remove(&a.id, PW.into(), "Alfa")),
        "confirmation_mismatch"
    );
    assert!(alpha_dir.exists());
    assert_eq!(
        code(fx.app.profile_remove("nope", PW.into(), "CONFIRM")),
        "not_found"
    );
    // nothing was disturbed by the refusals; Beta is still unlocked
    assert_eq!(fx.app.item_list().unwrap().len(), 1);

    // the literal CONFIRM works too
    fx.app.profile_remove(&a.id, PW.into(), "CONFIRM").unwrap();
    assert!(!alpha_dir.exists());
    assert_eq!(fx.names(), ["Beta"]);
    // the other account is byte-identical and still unlocked
    assert_eq!(bytes(&beta_dir.join("vault.db")), beta_before);
    assert_eq!(fx.app.item_list().unwrap()[0].title, "beta item");
    // removal is persisted
    let mut fx = fx;
    fx.restart();
    assert_eq!(fx.names(), ["Beta"]);
}

#[test]
fn removing_the_selected_account_locks_and_deselects_it() {
    let fx = Fx::new();
    let a = fx.app.profile_create("Alpha", PW.into()).unwrap();
    fx.app.item_create(login("x", "pw")).unwrap();
    let session = fx.app.test_session();
    fx.app.profile_remove(&a.id, PW.into(), "Alpha").unwrap();
    assert!(!session.status().unwrap().unlocked, "its keys were wiped");
    let st = fx.app.status().unwrap();
    assert!(!st.exists && !st.unlocked && st.profile.is_none());
    assert_eq!(code(fx.app.item_list()), "locked");
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());
    assert!(
        !fx.root().join("vaults").exists(),
        "the empty vaults folder is tidied"
    );
    // the same name can be used again
    fx.app.profile_create("Alpha", PW.into()).unwrap();
}

#[test]
fn removing_an_account_whose_files_are_already_gone_still_works() {
    let fx = Fx::new();
    let a = fx.app.profile_create("Alpha", PW.into()).unwrap();
    fx.app.profile_create("Beta", PW2.into()).unwrap();
    std::fs::remove_dir_all(fx.root().join("vaults").join(&a.id)).unwrap();
    // nothing left to protect: no password is needed, the entry just goes
    fx.app
        .profile_remove(&a.id, "whatever".into(), "Alpha")
        .unwrap();
    assert_eq!(fx.names(), ["Beta"]);
}

#[test]
fn removal_never_deletes_outside_the_accounts_own_folder() {
    let fx = Fx::new();
    let a = fx.app.profile_create("Alpha", PW.into()).unwrap();
    // a sibling that must survive, and a link planted where the account's folder is
    let victim = fx.outside();
    std::fs::write(victim.path().join("precious.txt"), b"keep").unwrap();
    fx.app.lock();
    let folder = fx.root().join("vaults").join(&a.id);
    let real = fx.root().join("vaults").join("real-aside");
    std::fs::rename(&folder, &real).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(victim.path(), &folder).unwrap();
        // password check fails first (no vault file behind the link)... then, with the link
        // pointing at a folder that has no vault.db, the entry has "nothing to protect"
        let r = fx.app.profile_remove(&a.id, PW.into(), "Alpha");
        assert_eq!(
            r.unwrap_err().code(),
            "storage",
            "a linked folder is refused"
        );
    }
    assert!(victim.path().join("precious.txt").exists());
}

#[test]
fn removing_the_adopted_single_vault_deletes_only_its_files() {
    let dir = tempfile::tempdir().unwrap();
    // a vault from the single-vault era, plus unrelated files that must survive
    let legacy = Session::new(dir.path().join("vault.db"), fast());
    legacy.create(PW.into()).unwrap();
    legacy.item_create(login("old", "pw")).unwrap();
    legacy.lock();
    drop(legacy);
    std::fs::write(dir.path().join("notes.txt"), b"mine").unwrap();
    let picker = Arc::new(TestPicker::default());
    let app = AppCore::open(dir.path(), fast(), picker).unwrap();
    let old = app.profile_list().unwrap().profiles.remove(0);
    assert_eq!(old.display_name, "My vault");
    app.profile_create("New", PW2.into()).unwrap();

    app.profile_remove(&old.id, PW.into(), "My vault").unwrap();
    assert!(!dir.path().join("vault.db").exists());
    assert!(
        dir.path().join("notes.txt").exists(),
        "unrelated files are untouched"
    );
    assert!(dir.path().join("vaults").is_dir());
    assert_eq!(app.profile_list().unwrap().profiles.len(), 1);
}

// ============================================================ legacy adoption and registry

#[test]
fn a_single_vault_is_adopted_in_place_without_being_touched() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = Session::new(dir.path().join("vault.db"), fast());
    legacy.create(PW.into()).unwrap();
    legacy.item_create(login("Old item", "old-pw")).unwrap();
    legacy.lock();
    drop(legacy);
    let before = bytes(&dir.path().join("vault.db"));

    let picker = Arc::new(TestPicker::default());
    let app = AppCore::open(dir.path(), fast(), picker.clone()).unwrap();
    let list = app.profile_list().unwrap();
    assert_eq!(list.profiles.len(), 1);
    assert_eq!(list.profiles[0].display_name, "My vault");
    assert_eq!(
        list.selected_id, None,
        "nothing is selected until the person picks"
    );
    // reading the list, selecting and a wrong password change nothing on disk
    app.profile_select(&list.profiles[0].id).unwrap();
    let st = app.status().unwrap();
    assert!(st.exists && !st.unlocked);
    assert_eq!(code(app.unlock(WRONG.into())), "wrong_password");
    assert_eq!(
        bytes(&dir.path().join("vault.db")),
        before,
        "byte-identical until a write"
    );
    assert!(!dir.path().join("vaults").exists(), "nothing was moved");

    // the old password opens it and the old item is there
    app.unlock(PW.into()).unwrap();
    assert_eq!(app.item_list().unwrap()[0].title, "Old item");
    app.item_create(login("New item", "new-pw")).unwrap();
    app.lock();
    drop(app);
    // adoption is remembered (registry), and the vault is still where it was
    let app = AppCore::open(dir.path(), fast(), picker).unwrap();
    assert_eq!(app.profile_list().unwrap().profiles.len(), 1);
    app.profile_select(&app.profile_list().unwrap().profiles[0].id)
        .unwrap();
    app.unlock(PW.into()).unwrap();
    assert_eq!(app.item_list().unwrap().len(), 2);
    assert!(dir.path().join("vault.db").exists());
}

#[test]
fn last_used_tracks_selection_and_unlock_and_survives_restarts() {
    let mut fx = Fx::new();
    let a = fx.app.profile_create("Alpha", PW.into()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    let b = fx.app.profile_create("Beta", PW2.into()).unwrap();
    let list = fx.app.profile_list().unwrap();
    assert_eq!(list.last_used_id.as_deref(), Some(b.id.as_str()));
    assert_eq!(list.profiles[0].id, b.id, "newest-used first");

    std::thread::sleep(std::time::Duration::from_millis(3));
    fx.app.profile_select(&a.id).unwrap();
    let list = fx.app.profile_list().unwrap();
    assert_eq!(list.last_used_id.as_deref(), Some(a.id.as_str()));
    assert_eq!(list.selected_id.as_deref(), Some(a.id.as_str()));

    fx.restart();
    let list = fx.app.profile_list().unwrap();
    assert_eq!(
        list.last_used_id.as_deref(),
        Some(a.id.as_str()),
        "remembered across restarts"
    );
    assert_eq!(
        list.selected_id, None,
        "a restart starts at the login screen"
    );
    assert_eq!(list.profiles.len(), 2);
}

#[test]
fn a_corrupt_registry_is_set_aside_and_rebuilt_from_the_folders() {
    let mut fx = Fx::new();
    fx.app.profile_create("Alpha", PW.into()).unwrap();
    fx.app.item_create(login("alpha item", "pw")).unwrap();
    fx.app.profile_create("Beta", PW2.into()).unwrap();
    fx.app.item_create(login("beta item", "pw")).unwrap();
    fx.app.lock();
    std::fs::write(fx.root().join("profiles.json"), b"{ this is not json").unwrap();

    fx.restart();
    let list = fx.app.profile_list().unwrap();
    assert_eq!(list.profiles.len(), 2);
    let mut names = fx.names();
    names.sort();
    assert_eq!(names, ["Recovered vault 1", "Recovered vault 2"]);
    // the damaged file is kept, never deleted
    let aside: Vec<_> = fx
        .listing()
        .into_iter()
        .filter(|n| n.starts_with("profiles.json.corrupt-"))
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(bytes(&fx.root().join(&aside[0])), b"{ this is not json");
    // every recovered account opens with its own password
    let mut opened = Vec::new();
    for p in &list.profiles {
        fx.app.profile_select(&p.id).unwrap();
        let pw = if fx.app.unlock(PW.into()).is_ok() {
            PW
        } else {
            PW2
        };
        fx.app.unlock(pw.into()).unwrap();
        opened.push(fx.app.item_list().unwrap()[0].title.clone());
    }
    opened.sort();
    assert_eq!(opened, ["alpha item", "beta item"]);
    // and the rebuilt registry is saved: a second restart needs no recovery
    fx.restart();
    assert_eq!(fx.app.profile_list().unwrap().profiles.len(), 2);
    let again = fx
        .listing()
        .into_iter()
        .filter(|n| n.starts_with("profiles.json.corrupt-"))
        .count();
    assert_eq!(again, 1);
}

#[test]
fn a_tampered_registry_cannot_point_deletion_outside_the_data_folder() {
    let fx = Fx::new();
    let a = fx.app.profile_create("Alpha", PW.into()).unwrap();
    fx.app.lock();
    let victim = fx.outside();
    std::fs::write(victim.path().join("precious.txt"), b"keep").unwrap();
    let evil = format!(
        r#"{{"version":1,"profiles":[{{"id":"{}","displayName":"Alpha","createdAt":1,"lastUsedAt":null,"dir":"{}"}}]}}"#,
        a.id,
        victim.path().display()
    );
    std::fs::write(fx.root().join("profiles.json"), evil).unwrap();
    for bad_dir in ["../../etc", "vaults/../..", "/tmp", "vaults/other"] {
        let reg = format!(
            r#"{{"version":1,"profiles":[{{"id":"{}","displayName":"Alpha","createdAt":1,"dir":"{bad_dir}"}}]}}"#,
            a.id
        );
        std::fs::write(fx.root().join("profiles.json"), reg).unwrap();
        let app = AppCore::open(fx.root(), fast(), Arc::new(TestPicker::default())).unwrap();
        // treated as corruption: rebuilt from what is really on disk, never trusted
        let list = app.profile_list().unwrap();
        assert_eq!(list.profiles.len(), 1, "{bad_dir}");
        assert_eq!(list.profiles[0].display_name, "Recovered vault 1");
        let id = list.profiles[0].id.clone();
        assert_eq!(id, a.id, "the folder name is the id");
        app.profile_remove(&id, PW.into(), "CONFIRM").unwrap();
        assert!(victim.path().join("precious.txt").exists());
        // put the account back for the next round
        let _ = std::fs::remove_dir_all(fx.root().join("vaults"));
        let s = Session::new(
            fx.root().join("vaults").join(&a.id).join("vault.db"),
            fast(),
        );
        s.create(PW.into()).unwrap();
        s.lock();
    }
}

#[test]
fn a_registry_from_a_newer_version_is_never_touched() {
    let dir = tempfile::tempdir().unwrap();
    let text = br#"{"version":99,"profiles":[],"somethingNew":true}"#;
    std::fs::write(dir.path().join("profiles.json"), text).unwrap();
    let r = AppCore::open(dir.path(), fast(), Arc::new(TestPicker::default()));
    let e = r.err().expect("must refuse to start");
    assert_eq!(e.code(), "storage");
    assert!(e.to_string().contains("newer version"));
    assert_eq!(bytes(&dir.path().join("profiles.json")), text);
}

#[test]
fn a_registry_with_duplicate_names_is_treated_as_corrupt() {
    let fx = Fx::new();
    fx.app.profile_create("Alpha", PW.into()).unwrap();
    fx.app.profile_create("Beta", PW2.into()).unwrap();
    fx.app.lock();
    let list = fx.app.profile_list().unwrap();
    let reg = serde_json::json!({
        "version": 1,
        "profiles": list.profiles.iter().map(|p| serde_json::json!({
            "id": p.id, "displayName": "Same", "createdAt": 1, "dir": format!("vaults/{}", p.id)
        })).collect::<Vec<_>>()
    });
    std::fs::write(fx.root().join("profiles.json"), reg.to_string()).unwrap();
    let app = AppCore::open(fx.root(), fast(), Arc::new(TestPicker::default())).unwrap();
    let mut names: Vec<_> = app
        .profile_list()
        .unwrap()
        .profiles
        .into_iter()
        .map(|p| p.display_name)
        .collect();
    names.sort();
    assert_eq!(names, ["Recovered vault 1", "Recovered vault 2"]);
}

#[cfg(unix)]
#[test]
fn files_and_folders_are_private_to_the_owner() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new();
    let info = fx.app.profile_create("Alpha", PW.into()).unwrap();
    fx.app.item_create(login("x", "pw")).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(fx.root()), 0o700);
    assert_eq!(mode(&fx.root().join("profiles.json")), 0o600);
    assert_eq!(mode(&fx.root().join("vaults")), 0o700);
    assert_eq!(mode(&fx.root().join("vaults").join(&info.id)), 0o700);
    let v = fx.root().join("vaults").join(&info.id).join("vault.db");
    assert_eq!(mode(&v), 0o600);
    // while the vault is open SQLite keeps a write-ahead log and a shared-memory file beside it;
    // they must be as private as the database itself
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", v.display()));
        assert!(
            side.exists(),
            "{side:?} should exist while the vault is open"
        );
        assert_eq!(mode(&side), 0o600, "{side:?}");
    }
    // the same holds for an imported copy and for a vault created the single-vault way
    let out = fx.outside();
    let file = out.path().join("copy.hexis");
    fx.picker.set_save(Some(file.clone()));
    fx.app.vault_export_file(PW.into()).unwrap().unwrap();
    assert_eq!(mode(&file), 0o600);
    fx.picker.set_open(Some(file));
    let imported = fx.app.profile_import_file("Imported").unwrap().unwrap();
    fx.app.unlock(PW.into()).unwrap();
    fx.app.item_create(login("y", "pw")).unwrap();
    let iv = fx.root().join("vaults").join(&imported.id).join("vault.db");
    assert_eq!(mode(&iv), 0o600);
    assert_eq!(mode(iv.parent().unwrap()), 0o700);
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", iv.display()));
        assert_eq!(mode(&side), 0o600, "{side:?}");
    }
    let legacy = tempfile::tempdir().unwrap();
    let s = Session::new(legacy.path().join("vault.db"), fast());
    s.create(PW.into()).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let side = PathBuf::from(format!(
            "{}{suffix}",
            legacy.path().join("vault.db").display()
        ));
        assert_eq!(mode(&side), 0o600, "{side:?}");
    }
}

// ============================================================ on-disk secrecy

#[test]
fn nothing_readable_from_an_item_is_in_the_database_files() {
    let fx = Fx::new();
    let info = fx
        .app
        .profile_create("Leakcheck Profile", PW.into())
        .unwrap();
    let mut d = login("LEAKTITLE-4f9a", "LEAKPASS-7731");
    d.fields
        .insert("username".into(), "leak.user@example.org".into());
    d.fields
        .insert("url".into(), "https://leak.example.net/login".into());
    d.fields.insert("notes".into(), "LEAKNOTE-0042".into());
    d.tags = vec!["leaktag".into()];
    d.folder = Some("LeakFolder".into());
    d.custom_fields = vec![CustomField {
        name: "LeakFieldName".into(),
        value: "LEAKCUSTOM-9911".into(),
        hidden: true,
    }];
    let login_item = fx.app.item_create(d).unwrap();
    fx.app.item_create(card("LEAKCARD-titled")).unwrap();
    fx.app
        .item_create(draft(
            ItemType::SshKey,
            "LEAKSSH-titled",
            &[
                ("privateKey", "LEAKPRIVATEKEY-5521"),
                ("publicKey", "ssh-ed25519 AAAALEAKPUB leak@laptop"),
            ],
        ))
        .unwrap();
    edit(&fx.app, &login_item.id, |d| {
        d.fields
            .insert("password".into(), "LEAKPASS-EDITED-8812".into());
    });
    fx.app.item_archive(&login_item.id).unwrap();
    let gone = fx
        .app
        .item_create(login("LEAKGONE-title", "LEAKGONE-pw"))
        .unwrap();
    fx.app.item_trash(&gone.id).unwrap();

    let needles = [
        "LEAKTITLE-4f9a",
        "LEAKPASS-7731",
        "leak.user@example.org",
        "leak.example.net",
        "LEAKNOTE-0042",
        "leaktag",
        "LeakFolder",
        "LeakFieldName",
        "LEAKCUSTOM-9911",
        "LEAKCARD-titled",
        "4111 1111 1111 1111",
        "4111111111111111",
        "Ana Example",
        "LEAKSSH-titled",
        "LEAKPRIVATEKEY-5521",
        "AAAALEAKPUB",
        "leak@laptop",
        "LEAKPASS-EDITED-8812",
        "LEAKGONE-title",
        "LEAKGONE-pw",
        "Leakcheck Profile",
        PW,
    ];
    let vdir = fx.root().join("vaults").join(&info.id);
    let check = |label: &str| {
        for entry in std::fs::read_dir(&vdir).unwrap() {
            let path = entry.unwrap().path();
            let data = bytes(&path);
            for n in needles {
                assert!(!contains(&data, n), "{label}: {n:?} found in {path:?}");
            }
        }
    };
    check("while unlocked (database and write-ahead log)");
    fx.app.lock();
    check("after locking");
    // the registry holds the display name (documented) and nothing else about the vault
    let reg = String::from_utf8(bytes(&fx.root().join("profiles.json"))).unwrap();
    assert!(reg.contains("Leakcheck Profile"));
    for n in needles.iter().filter(|n| **n != "Leakcheck Profile") {
        assert!(!reg.contains(n), "{n:?} in the registry");
    }
    // sanity: the scan does see plain text when it is there
    assert!(contains(b"xx LEAKTITLE-4f9a yy", "LEAKTITLE-4f9a"));
}

#[test]
fn nothing_readable_from_a_backup_target_is_in_the_database_files() {
    use super::backup::remote::mem::MemFactory;
    use super::backup::{BackupRuntime, Timing};
    use super::BackupTargetInput;

    let dir = tempfile::tempdir().unwrap();
    let app = AppCore::open(dir.path(), fast(), Arc::new(TestPicker::default()))
        .unwrap()
        .with_backup_runtime(BackupRuntime::new(
            MemFactory::new(),
            Timing {
                background: false,
                ..Timing::default()
            },
        ));
    let info = app
        .profile_create("Targetcheck Profile", PW.into())
        .unwrap();
    let target =
        |name: &str, bucket: &str, secret: &str, master: bool, share: bool| BackupTargetInput {
            name: name.into(),
            endpoint: Some("https://s3.leak-provider.example.net".into()),
            region: "leak-region-9".into(),
            bucket: bucket.into(),
            prefix: "leak/prefix/ana".into(),
            access_key_id: "AKIALEAKKEYID0042".into(),
            secret_access_key: secret.into(),
            path_style: true,
            enabled: true,
            master,
            share_in_master: share,
        };
    app.backup_add(target(
        "LEAKTARGET-shared",
        "leak-bucket-shared",
        "LEAKSECRET-shared-0001",
        true,
        true,
    ))
    .unwrap();
    app.backup_add(target(
        "LEAKTARGET-private",
        "leak-bucket-private",
        "LEAKSECRET-private-0002",
        false,
        false,
    ))
    .unwrap();
    app.item_create(login("LEAKTITLE-target", "LEAKPASS-target"))
        .unwrap();
    app.backup_run(None).unwrap();
    // an edit of a target after the first save leaves no old copy behind either
    let id = app.backup_list().unwrap()[1].id.clone();
    app.backup_update(
        &id,
        target(
            "LEAKTARGET-private-2",
            "leak-bucket-private-2",
            "LEAKSECRET-private-0003",
            false,
            false,
        ),
    )
    .unwrap();
    app.backup_run(None).unwrap();

    let needles = [
        "LEAKTARGET",
        "leak-bucket",
        "LEAKSECRET",
        "leak-provider",
        "leak-region",
        "leak/prefix",
        "AKIALEAKKEYID0042",
        "LEAKTITLE-target",
        "LEAKPASS-target",
    ];
    let vdir = dir.path().join("vaults").join(&info.id);
    let check = |label: &str| {
        let mut seen = 0;
        for entry in std::fs::read_dir(&vdir).unwrap() {
            let path = entry.unwrap().path();
            let data = bytes(&path);
            seen += 1;
            for n in needles {
                assert!(!contains(&data, n), "{label}: {n:?} found in {path:?}");
            }
        }
        assert!(seen >= 1, "the scan looked at the database file");
    };
    check("while unlocked (database and write-ahead log)");
    app.lock();
    check("after locking");
    let reg = String::from_utf8(bytes(&dir.path().join("profiles.json"))).unwrap();
    for n in needles {
        assert!(!reg.contains(n), "{n:?} in the registry");
    }
    // and the scan is not blind: the settings do open with the master password
    app.unlock(PW.into()).unwrap();
    assert_eq!(
        app.backup_reveal_secret(&app.backup_list().unwrap()[0].id, PW.into())
            .unwrap(),
        "LEAKSECRET-shared-0001"
    );
}

// ============================================================ new item types

#[test]
fn card_identity_and_ssh_items_round_trip_through_the_whole_lifecycle() {
    let fx = Fx::new();
    fx.app.profile_create("Types", PW.into()).unwrap();

    let identity = draft(
        ItemType::Identity,
        "Me",
        &[
            ("firstName", "Ana"),
            ("lastName", "Example"),
            ("email", "ana@example.com"),
            ("nationalId", "123-45-6789"),
            ("address1", "1 Main St"),
        ],
    );
    let ssh = draft(
        ItemType::SshKey,
        "Laptop key",
        &[
            ("publicKey", "ssh-ed25519 AAAAC3Nza ana@laptop"),
            (
                "privateKey",
                "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----",
            ),
            ("passphrase", "hunter2hunter2"),
        ],
    );
    for (d, subtitle) in [
        (card("Visa"), "Visa •••• 1111"),
        (identity, "Ana Example"),
        (ssh, "ana@laptop"),
    ] {
        let created = fx.app.item_create(d.clone()).unwrap();
        assert_eq!(created.item_type, d.item_type);
        assert_eq!(created.subtitle.as_deref(), Some(subtitle));
        assert_eq!(created.version, 1);
        assert!(!created.archived);
        let id = created.id.clone();

        // reveal returns exactly what was saved
        let revealed = fx.app.item_reveal(&id, Some(PW.into())).unwrap();
        assert_eq!(revealed, d);
        // edit through the two-step flow; history records which fields changed, never values
        let updated = edit(&fx.app, &id, |x| {
            x.title = format!("{} v2", x.title);
            x.fields.insert("notes".into(), "NOTE-after-edit".into());
        });
        assert_eq!(updated.version, 2);
        let hist = fx.app.history_list(&id).unwrap();
        assert_eq!(hist[0].event, ItemEvent::Edited);
        assert!(hist[0].changed_fields.contains(&"title".to_string()));
        assert!(hist[0].changed_fields.contains(&"notes".to_string()));
        assert!(hist
            .iter()
            .all(|h| h.changed_fields.iter().all(|f| !f.contains("NOTE"))));
        // the first snapshot is intact
        assert_eq!(fx.app.history_get(&id, 1, Some(PW.into())).unwrap(), d);

        // archive, trash, restore (stays archived), restore an old version, clone
        assert!(fx.app.item_archive(&id).unwrap().archived);
        fx.app.item_trash(&id).unwrap();
        assert_eq!(code(fx.app.item_get(&id)), "in_trash");
        assert!(
            fx.app.item_restore(&id).unwrap().archived,
            "restore returns it to the archive"
        );
        let restored = fx.app.history_restore(&id, 1, Some(PW.into())).unwrap();
        assert_eq!(restored.title, d.title);
        let copy = fx.app.item_clone(&id, Some(PW.into())).unwrap();
        assert_eq!(copy.item_type, d.item_type);
        assert_eq!(copy.title, format!("{} (copy)", d.title));
        assert!(!copy.archived);
        assert_eq!(
            fx.app
                .item_reveal(&copy.id, Some(PW.into()))
                .unwrap()
                .fields,
            d.fields
        );
    }
    // all three types and their copies are listed
    assert_eq!(fx.app.item_list().unwrap().len(), 6);
}

#[test]
fn field_schemas_and_light_validation_are_enforced_in_rust() {
    let fx = Fx::new();
    fx.app.profile_create("Rules", PW.into()).unwrap();
    let bad = |d: ItemDraft| code(fx.app.item_create(d));
    // a field that belongs to another type
    let mut d = card("c");
    d.fields.insert("username".into(), "x".into());
    assert_eq!(bad(d), "invalid_input");
    for number in [
        "12ab",
        "1234567",
        "4111 1111 1111 1111 1111 1111",
        "٤١١١١١١١١",
    ] {
        let mut d = card("c");
        d.fields.insert("number".into(), number.into());
        assert_eq!(bad(d), "invalid_input", "{number}");
    }
    for (k, v) in [
        ("expMonth", "0"),
        ("expMonth", "13"),
        ("expMonth", "ab"),
        ("expYear", "29"),
        ("expYear", "20290"),
    ] {
        let mut d = card("c");
        d.fields.insert(k.into(), v.into());
        assert_eq!(bad(d), "invalid_input", "{k}={v}");
    }
    let mut d = draft(ItemType::SshKey, "k", &[("privateKey", "x")]);
    d.fields
        .insert("privateKey".into(), "k".repeat(64 * 1024 + 1));
    assert_eq!(bad(d), "invalid_input");
    // valid edge values pass, whitespace is trimmed, empty values are dropped
    let mut d = card("edge");
    d.fields
        .insert("number".into(), "  4111-1111-1111-1111  ".into());
    d.fields.insert("expMonth".into(), "12".into());
    d.fields.insert("pin".into(), String::new());
    let made = fx.app.item_create(d).unwrap();
    let r = fx.app.item_reveal(&made.id, Some(PW.into())).unwrap();
    assert_eq!(r.fields["number"], "4111-1111-1111-1111");
    assert!(!r.fields.contains_key("pin"));
}

#[test]
fn payloads_written_before_the_archive_flag_still_open() {
    let fx = Fx::new();
    let info = fx.app.profile_create("Old", PW.into()).unwrap();
    let id = uuid::new_v4().unwrap();
    // exactly what an older build sealed: schema 1, no `archived` key
    let legacy = serde_json::json!({
        "schema": 1, "id": id, "type": "login", "title": "Legacy item", "favorite": true,
        "tags": ["old"], "folder": null,
        "fields": {"username": "old@example.com", "password": "old-pw"},
        "customFields": [], "attachments": [],
        "createdAt": 1_700_000_000_000u64, "updatedAt": 1_700_000_000_000u64,
        "deleted": false, "deletedAt": null,
        "event": "created", "restoredFrom": null, "changedFields": []
    });
    assert!(legacy.get("archived").is_none());
    fx.app.test_session().test_vault(|v| {
        v.test_commit_raw(&id, 1, legacy.to_string().as_bytes())
            .unwrap()
    });

    // a cold start reads it back from disk through the real unlock path
    let mut fx = fx;
    fx.restart();
    fx.app.profile_select(&info.id).unwrap();
    fx.app.unlock(PW.into()).unwrap();
    let list = fx.app.item_list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "Legacy item");
    assert!(!list[0].archived, "a missing flag means not archived");
    assert!(!list[0].reprompt, "a missing flag means no re-prompt");
    assert_eq!(fx.app.status().unwrap().damaged_items, 0);
    let r = fx.app.item_reveal(&id, Some(PW.into())).unwrap();
    assert_eq!(r.fields["password"], "old-pw");
    assert_eq!(
        fx.app.history_get(&id, 1, Some(PW.into())).unwrap().title,
        "Legacy item"
    );

    // and new writes on top of it work: archive, edit, clone
    let a = fx.app.item_archive(&id).unwrap();
    assert!(a.archived && a.version == 2);
    let e = edit(&fx.app, &id, |d| d.title = "Legacy renamed".into());
    assert!(e.archived, "an edit keeps the flag");
    assert_eq!(e.version, 3);
    let copy = fx.app.item_clone(&id, Some(PW.into())).unwrap();
    assert_eq!(copy.title, "Legacy renamed (copy)");
    let hist: Vec<_> = fx
        .app
        .history_list(&id)
        .unwrap()
        .into_iter()
        .map(|h| h.event)
        .collect();
    assert_eq!(
        hist,
        [ItemEvent::Edited, ItemEvent::Archived, ItemEvent::Created]
    );
}

// ============================================================ archive and clone

#[test]
fn archive_is_a_flag_with_versioned_events_and_no_password() {
    let fx = Fx::new();
    fx.app.profile_create("Arc", PW.into()).unwrap();
    let a = fx.app.item_create(login("Keep", "pw-1")).unwrap();
    let b = fx.app.item_create(login("Other", "pw-2")).unwrap();
    let t = fx.app.item_create(login("Binned", "pw-3")).unwrap();
    fx.app.item_trash(&t.id).unwrap();

    let arch = fx.app.item_archive(&a.id).unwrap();
    assert!(arch.archived);
    assert_eq!(arch.version, 2);
    // archived items stay in the list (flagged) and fully usable; trashed ones are not listed
    let list = fx.app.item_list().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().find(|i| i.id == a.id).unwrap().archived);
    assert!(!list.iter().find(|i| i.id == b.id).unwrap().archived);
    assert!(list.iter().all(|i| i.id != t.id));
    assert_eq!(fx.app.item_get(&a.id).unwrap().title, "Keep");
    assert_eq!(
        fx.app.item_reveal(&a.id, Some(PW.into())).unwrap().fields["password"],
        "pw-1"
    );
    let e = edit(&fx.app, &a.id, |d| d.title = "Kept".into());
    assert!(e.archived);

    // idempotent: no new version, no new history row
    let again = fx.app.item_archive(&a.id).unwrap();
    assert_eq!(again.version, e.version);
    let un = fx.app.item_unarchive(&a.id).unwrap();
    assert!(!un.archived);
    assert_eq!(un.version, e.version + 1);
    assert_eq!(fx.app.item_unarchive(&a.id).unwrap().version, un.version);

    let events: Vec<_> = fx
        .app
        .history_list(&a.id)
        .unwrap()
        .into_iter()
        .map(|h| (h.event, h.changed_fields))
        .collect();
    assert_eq!(
        events,
        [
            (ItemEvent::Unarchived, vec![]),
            (ItemEvent::Edited, vec!["title".to_string()]),
            (ItemEvent::Archived, vec![]),
            (ItemEvent::Created, vec![]),
        ]
    );

    // trashed or unknown items cannot be (un)archived
    assert_eq!(code(fx.app.item_archive(&t.id)), "in_trash");
    assert_eq!(code(fx.app.item_unarchive(&t.id)), "in_trash");
    assert_eq!(
        code(fx.app.item_archive("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")),
        "not_found"
    );
    assert_eq!(code(fx.app.item_archive("not an id")), "invalid_input");
    // locked: refused
    fx.app.lock();
    assert_eq!(code(fx.app.item_archive(&a.id)), "locked");
    assert_eq!(code(fx.app.item_unarchive(&a.id)), "locked");
    assert_eq!(code(fx.app.item_clone(&a.id, Some(PW.into()))), "locked");
}

#[test]
fn the_archive_flag_survives_a_restart_and_history_restore() {
    let mut fx = Fx::new();
    let info = fx.app.profile_create("Arc", PW.into()).unwrap();
    let a = fx.app.item_create(login("Keep", "pw-1")).unwrap();
    fx.app.item_archive(&a.id).unwrap();
    fx.restart();
    fx.app.profile_select(&info.id).unwrap();
    fx.app.unlock(PW.into()).unwrap();
    assert!(fx.app.item_list().unwrap()[0].archived);
    // restoring an old snapshot restores content only: the archive flag stays as it is
    let r = fx.app.history_restore(&a.id, 1, Some(PW.into())).unwrap();
    assert!(r.archived);
    assert_eq!(r.version, 3);
    assert!(fx.app.item_unarchive(&a.id).unwrap().version == 4);
}

#[test]
fn clone_is_password_gated_and_never_returns_secrets() {
    let fx = Fx::new();
    fx.app.profile_create("Cl", PW.into()).unwrap();
    let mut d = card("Travel card");
    d.tags = vec!["travel".into(), "work".into()];
    d.folder = Some("Money".into());
    d.favorite = true;
    d.custom_fields = vec![CustomField {
        name: "Bank".into(),
        value: "SECRET-BANK-REF".into(),
        hidden: true,
    }];
    let src = fx.app.item_create(d.clone()).unwrap();

    assert_eq!(
        code(fx.app.item_clone(&src.id, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.app.item_clone(&src.id, Some(String::new()))),
        "invalid_input",
        "an empty password is refused before any key derivation"
    );
    assert_eq!(
        fx.app.item_list().unwrap().len(),
        1,
        "a refused clone creates nothing"
    );

    let copy = fx.app.item_clone(&src.id, Some(PW.into())).unwrap();
    // what crosses back to the UI is the summary only: no secret value anywhere in it
    let wire = serde_json::to_value(&copy).unwrap();
    let text = wire.to_string();
    // The long secrets cannot turn up by chance. A short one can: "737" or "4111" sit inside a random id or a
    // millisecond timestamp now and then (this test failed once on CI for that reason), so for those the summary's
    // shape is what is checked: exactly the known keys, and no value that is the code itself.
    for secret in ["4111 1111 1111 1111", "4111111111111111", "SECRET-BANK-REF"] {
        assert!(
            !text.contains(secret),
            "{secret} leaked into the clone result"
        );
    }
    let object = wire.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "archived",
            "createdAt",
            "favorite",
            "folder",
            "hasTotp",
            "id",
            "reprompt",
            "subtitle",
            "tags",
            "title",
            "type",
            "updatedAt",
            "version"
        ],
        "the clone result is the summary only: no field values, no custom fields"
    );
    for (key, value) in object {
        assert_ne!(value.as_str(), Some("737"), "the code leaked as {key}");
    }
    assert!(
        !object["subtitle"].as_str().unwrap_or("").contains("4111"),
        "the subtitle shows the card number"
    );
    assert_ne!(copy.id, src.id);
    assert_eq!(copy.title, "Travel card (copy)");
    assert_eq!(copy.version, 1);
    assert!(copy.favorite);
    assert_eq!(copy.tags, ["travel", "work"]);
    assert_eq!(copy.folder.as_deref(), Some("Money"));
    let r = fx.app.item_reveal(&copy.id, Some(PW.into())).unwrap();
    assert_eq!(r.fields, d.fields);
    assert_eq!(r.custom_fields, d.custom_fields);
    // history of the copy starts fresh; the original is untouched
    let hist = fx.app.history_list(&copy.id).unwrap();
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].event, ItemEvent::Created);
    assert_eq!(fx.app.item_get(&src.id).unwrap().version, 1);
    // cloning a clone keeps stacking the suffix, titles never exceed the limit
    let long = fx.app.item_create(login(&"t".repeat(256), "pw")).unwrap();
    let c = fx.app.item_clone(&long.id, Some(PW.into())).unwrap();
    assert!(
        c.title.chars().count() <= 256 && c.title.ends_with(" (copy)"),
        "{}",
        c.title
    );
    // an archived original clones to an active copy
    fx.app.item_archive(&src.id).unwrap();
    assert!(
        !fx.app
            .item_clone(&src.id, Some(PW.into()))
            .unwrap()
            .archived
    );
}

// ============================================================ export and import

/// A vault with a bit of everything, for the round-trip tests.
fn seeded(fx: &Fx) -> (String, Vec<(String, String, u64, bool)>) {
    let info = fx.app.profile_create("Source", PW.into()).unwrap();
    let a = fx.app.item_create(login("Alpha", "pw-1")).unwrap();
    edit(&fx.app, &a.id, |d| d.title = "Alpha 2".into());
    let c = fx.app.item_create(card("Visa")).unwrap();
    fx.app.item_archive(&c.id).unwrap();
    let t = fx.app.item_create(login("Trashed", "pw-t")).unwrap();
    fx.app.item_trash(&t.id).unwrap();
    let mut items: Vec<_> = fx
        .app
        .item_list()
        .unwrap()
        .into_iter()
        .map(|i| (i.id, i.title, i.version, i.archived))
        .collect();
    items.sort();
    (info.id, items)
}

#[test]
fn an_exported_vault_imports_as_a_working_account_with_the_same_content() {
    let fx = Fx::new();
    let (source_id, items) = seeded(&fx);
    let source_device = fx.device_id();
    let out = fx.outside();
    let file = out.path().join("backup.hexis");
    fx.picker.set_save(Some(file.clone()));

    let done = fx.app.vault_export_file(PW.into()).unwrap().unwrap();
    assert_eq!(done.path, file.to_string_lossy());
    assert!(file.is_file());
    // a single self-contained file: no temp, journal or wal files next to it
    let left: Vec<_> = std::fs::read_dir(out.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, ["backup.hexis"]);
    let suggested = fx.picker.suggested_names();
    assert!(
        suggested[0].starts_with("Source-") && suggested[0].ends_with(".hexis"),
        "{suggested:?}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    super::files::check_vault_header(&file).unwrap();
    // exporting did not disturb the live vault
    assert_eq!(fx.app.item_list().unwrap().len(), 2);

    // import as a new account, still locked; it opens with the original master password
    fx.picker.set_open(Some(file.clone()));
    let imported = fx.app.profile_import_file("Restored").unwrap().unwrap();
    assert_ne!(imported.id, source_id);
    let st = fx.app.status().unwrap();
    assert!(st.exists && !st.unlocked);
    assert_eq!(st.profile.unwrap().display_name, "Restored");
    assert_eq!(code(fx.app.unlock(WRONG.into())), "wrong_password");
    fx.app.unlock(PW.into()).unwrap();

    let mut got: Vec<_> = fx
        .app
        .item_list()
        .unwrap()
        .into_iter()
        .map(|i| (i.id, i.title, i.version, i.archived))
        .collect();
    got.sort();
    assert_eq!(got, items, "same ids, titles, versions and archive flags");
    let alpha = &items.iter().find(|i| i.1 == "Alpha 2").unwrap().0;
    let hist = fx.app.history_list(alpha).unwrap();
    assert_eq!(hist.len(), 2);
    assert_eq!(
        fx.app.history_get(alpha, 1, Some(PW.into())).unwrap().title,
        "Alpha"
    );
    assert_eq!(fx.app.trash_list().unwrap().len(), 1);
    assert_eq!(fx.app.trash_list().unwrap()[0].title, "Trashed");
    // the copy is a new replica: it got its own device id; the original keeps its own
    assert_ne!(fx.device_id(), source_device);
    // the two accounts are independent from now on
    fx.app.item_create(login("Only in the copy", "pw")).unwrap();
    fx.app.profile_select(&source_id).unwrap();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(fx.app.item_list().unwrap().len(), 2);
    assert_eq!(fx.device_id(), source_device);
    // importing the same file again is fine (a separate account each time), names must differ
    assert_eq!(
        code(fx.app.profile_import_file("restored")),
        "profile_name_taken"
    );
    fx.app
        .profile_import_file("Restored again")
        .unwrap()
        .unwrap();
    assert_eq!(fx.names().len(), 3);
}

#[test]
fn export_is_password_gated_before_any_dialog_and_can_be_cancelled() {
    let fx = Fx::new();
    seeded(&fx);
    let out = fx.outside();
    let file = out.path().join("x.hexis");
    fx.picker.set_save(Some(file.clone()));
    assert_eq!(
        code(fx.app.vault_export_file(WRONG.into())),
        "wrong_password"
    );
    assert!(
        fx.picker.suggested_names().is_empty(),
        "no dialog for a wrong password"
    );
    assert!(!file.exists());
    fx.picker.set_save(None);
    assert_eq!(
        fx.app.vault_export_file(PW.into()).unwrap(),
        None,
        "cancel is not an error"
    );
    assert!(std::fs::read_dir(out.path()).unwrap().next().is_none());
    fx.app.lock();
    assert_eq!(code(fx.app.vault_export_file(PW.into())), "locked");
}

#[test]
fn export_refuses_the_apps_own_data_folder_and_never_overwrites_the_live_vault() {
    let fx = Fx::new();
    let (id, _) = seeded(&fx);
    let live = fx.root().join("vaults").join(&id).join("vault.db");
    let before = fx.app.item_list().unwrap().len();
    for target in [
        live.clone(),
        fx.root().join("vaults").join(&id).join("copy.hexis"),
        fx.root().join("elsewhere.hexis"),
    ] {
        fx.picker.set_save(Some(target.clone()));
        assert_eq!(
            code(fx.app.vault_export_file(PW.into())),
            "invalid_input",
            "{target:?}"
        );
    }
    assert!(!fx.root().join("elsewhere.hexis").exists());
    // the live vault still opens and is unchanged in content
    assert_eq!(fx.app.item_list().unwrap().len(), before);
    fx.app.lock();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(fx.app.item_list().unwrap().len(), before);
}

#[test]
fn export_replaces_an_existing_file_atomically_and_cleans_up_after_a_failure() {
    let fx = Fx::new();
    seeded(&fx);
    let out = fx.outside();
    let file = out.path().join("backup.hexis");
    std::fs::write(&file, b"old contents").unwrap();
    // a stale temp file from an earlier crash does not get in the way
    std::fs::write(out.path().join("backup.hexis.part"), b"stale").unwrap();
    fx.picker.set_save(Some(file.clone()));
    fx.app.vault_export_file(PW.into()).unwrap().unwrap();
    super::files::check_vault_header(&file).unwrap();
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);

    // an unwritable destination reports an error and leaves nothing behind
    fx.picker
        .set_save(Some(out.path().join("no-such-folder").join("x.hexis")));
    assert!(fx.app.vault_export_file(PW.into()).is_err());
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);
}

#[test]
fn bad_import_files_are_refused_with_a_specific_message_and_leave_nothing() {
    let fx = Fx::new();
    seeded(&fx);
    let good = fx.outside();
    let vault_file = good.path().join("good.hexis");
    fx.picker.set_save(Some(vault_file.clone()));
    fx.app.vault_export_file(PW.into()).unwrap().unwrap();

    let before_list = fx.app.profile_list().unwrap();
    let before_files = fx.listing();
    let tmp = fx.outside();
    let try_import = |path: PathBuf| -> StorageError {
        fx.picker.set_open(Some(path));
        fx.app.profile_import_file("Imported").unwrap_err()
    };
    let say = |e: &StorageError| format!("{}: {}", e.code(), e);

    // not a database at all
    let garbage = tmp.path().join("garbage.hexis");
    std::fs::write(&garbage, b"hello, this is just text").unwrap();
    let e = try_import(garbage);
    assert_eq!(e.code(), "unsupported_file", "{}", say(&e));
    assert!(e.to_string().contains("not a database"));

    // empty file, and a file shorter than a database header
    let empty = tmp.path().join("empty.hexis");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(try_import(empty).code(), "unsupported_file");

    // a folder
    let e = try_import(tmp.path().to_path_buf());
    assert_eq!(e.code(), "unsupported_file");
    assert!(e.to_string().contains("not a folder"));

    // missing file
    assert_eq!(
        try_import(tmp.path().join("missing.hexis")).code(),
        "unsupported_file"
    );

    // somebody else's SQLite database
    let foreign = tmp.path().join("foreign.db");
    {
        let c = rusqlite::Connection::open(&foreign).unwrap();
        c.execute_batch("CREATE TABLE t (a); INSERT INTO t VALUES (1);")
            .unwrap();
    }
    let e = try_import(foreign);
    assert_eq!(e.code(), "unsupported_file");
    assert!(
        e.to_string().contains("not a vault this app can open"),
        "{e}"
    );

    // written by a newer version of Hexis
    let newer = tmp.path().join("newer.hexis");
    let mut data = bytes(&vault_file);
    data[60..64].copy_from_slice(&((super::db::SCHEMA_VERSION + 1) as i32).to_be_bytes());
    std::fs::write(&newer, data).unwrap();
    let e = try_import(newer);
    assert_eq!(e.code(), "unsupported_file");
    assert!(
        e.to_string().contains("newer version") && e.to_string().contains("Update"),
        "{e}"
    );

    // truncated in the middle of the database
    let cut = tmp.path().join("cut.hexis");
    let data = bytes(&vault_file);
    std::fs::write(&cut, &data[..data.len() / 3]).unwrap();
    assert_eq!(try_import(cut).code(), "unsupported_file");

    // a Hexis database that never got a vault (no key bundle)
    let hollow = tmp.path().join("hollow.hexis");
    drop(Store::open(&hollow).unwrap());
    let e = try_import(hollow);
    assert_eq!(e.code(), "unsupported_file");
    assert!(e.to_string().contains("does not contain a vault"), "{e}");

    // a Hexis database whose key bundle is not a key bundle
    let broken = tmp.path().join("broken.hexis");
    {
        let mut s = Store::open(&broken).unwrap();
        s.create_vault("{\"not\":\"a bundle\"}", &uuid::new_v4().unwrap(), 1)
            .unwrap();
    }
    let e = try_import(broken);
    assert_eq!(e.code(), "unsupported_file");
    assert!(e.to_string().contains("key data"), "{e}");

    // the app's own data folder is not a place to import from
    let inside = fx
        .root()
        .join("vaults")
        .join(&fx.app.profile_list().unwrap().profiles[0].id)
        .join("vault.db");
    assert_eq!(try_import(inside).code(), "invalid_input");

    // after all of that: no new account, no stray folders, selection untouched
    assert_eq!(fx.app.profile_list().unwrap(), before_list);
    assert_eq!(fx.listing(), before_files);
    let vault_dirs = std::fs::read_dir(fx.root().join("vaults")).unwrap().count();
    assert_eq!(vault_dirs, 1);
    assert_eq!(
        fx.app.item_list().unwrap().len(),
        2,
        "the open vault was not disturbed"
    );

    // a cancelled dialog and a bad name are not errors of the file
    fx.picker.set_open(None);
    assert_eq!(fx.app.profile_import_file("Fine").unwrap(), None);
    fx.picker.set_open(Some(vault_file));
    assert_eq!(code(fx.app.profile_import_file("   ")), "invalid_input");
    assert_eq!(
        code(fx.app.profile_import_file("source")),
        "profile_name_taken"
    );
}

#[test]
fn import_never_modifies_the_chosen_file() {
    let fx = Fx::new();
    seeded(&fx);
    let out = fx.outside();
    let file = out.path().join("backup.hexis");
    fx.picker.set_save(Some(file.clone()));
    fx.app.vault_export_file(PW.into()).unwrap().unwrap();
    let before = bytes(&file);
    fx.picker.set_open(Some(file.clone()));
    fx.app.profile_import_file("Copy").unwrap().unwrap();
    fx.app.unlock(PW.into()).unwrap();
    fx.app.item_create(login("new", "pw")).unwrap();
    assert_eq!(bytes(&file), before, "the original file is only ever read");
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);
}
