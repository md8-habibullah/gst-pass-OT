//! Behavioural tests of the whole storage stack through [`Session`] (the same API the Tauri
//! commands use), plus direct database tampering for the integrity cases.

use super::model::{CustomField, ItemDraft, ItemEvent, ItemType};
use super::session::{Session, SessionConfig, EDIT_TOKEN_TTL};
use super::{Result, StorageError};
use crate::crypto::KdfParams;
use std::collections::BTreeMap;
use std::time::Duration;

const PW: &str = "correct horse battery staple";
const WRONG: &str = "definitely not the password";
const MISSING: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

fn fast(ttl: Duration) -> SessionConfig {
    SessionConfig {
        kdf: KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        },
        edit_token_ttl: ttl,
        purge_grace: std::time::Duration::ZERO,
    }
}

struct Fx {
    dir: tempfile::TempDir,
    s: Session,
}

impl Fx {
    fn new() -> Self {
        Self::with_ttl(EDIT_TOKEN_TTL)
    }

    fn with_ttl(ttl: Duration) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let s = Session::new(dir.path().join("vault.db"), fast(ttl));
        s.create(PW.into()).unwrap();
        Self { dir, s }
    }

    fn reopen(&self) -> Session {
        Session::new(self.dir.path().join("vault.db"), fast(EDIT_TOKEN_TTL))
    }

    /// Create a login and return its id.
    fn add(&self, title: &str, password: &str) -> String {
        self.s.item_create(login(title, password)).unwrap().id
    }

    /// Edit through the real two-step flow.
    fn edit(&self, id: &str, f: impl FnOnce(&mut ItemDraft)) -> super::ItemSummary {
        let b = self.s.item_begin_edit(id, Some(PW.into())).unwrap();
        let mut d = b.draft.clone();
        f(&mut d);
        self.s.item_update(id, &b.token, b.version, d).unwrap()
    }

    fn versions(&self, id: &str) -> u64 {
        self.s.test_vault(|v| v.store().count_versions(id).unwrap())
    }

    fn item_version(&self, id: &str) -> u64 {
        self.s
            .test_vault(|v| v.store().item_row(id).unwrap().unwrap().version)
    }

    fn snapshot_blob(&self, id: &str, version: u64) -> Vec<u8> {
        self.s
            .test_vault(|v| v.store().version_row(id, version).unwrap().unwrap().blob)
    }
}

fn login(title: &str, password: &str) -> ItemDraft {
    ItemDraft {
        item_type: ItemType::Login,
        title: title.into(),
        favorite: false,
        reprompt: false,
        tags: vec![],
        folder: None,
        fields: BTreeMap::from([
            ("username".into(), "ana@example.com".into()),
            ("password".into(), password.into()),
            ("url".into(), "https://www.example.com/login".into()),
        ]),
        custom_fields: vec![],
    }
}

fn code<T: std::fmt::Debug>(r: Result<T>) -> &'static str {
    r.expect_err("expected an error").code()
}

// ============================================================ vault lifecycle

#[test]
fn status_follows_the_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let s = Session::new(dir.path().join("vault.db"), fast(EDIT_TOKEN_TTL));
    let st = s.status().unwrap();
    assert!(!st.exists && !st.unlocked);
    assert_eq!(code(s.unlock(PW.into())), "no_vault");
    assert!(
        !dir.path().join("vault.db").exists(),
        "status and a failed unlock must not create the file"
    );

    s.create(PW.into()).unwrap();
    let st = s.status().unwrap();
    assert!(st.exists && st.unlocked);

    s.lock();
    s.lock(); // idempotent
    let st = s.status().unwrap();
    assert!(st.exists && !st.unlocked);

    assert_eq!(code(s.unlock(WRONG.into())), "wrong_password");
    assert!(!s.status().unwrap().unlocked);
    s.unlock(PW.into()).unwrap();
    assert!(s.status().unwrap().unlocked);
}

#[test]
fn create_is_refused_twice_and_for_short_passwords() {
    let dir = tempfile::tempdir().unwrap();
    let s = Session::new(dir.path().join("vault.db"), fast(EDIT_TOKEN_TTL));
    assert_eq!(code(s.create("short".into())), "invalid_input");
    assert!(!s.status().unwrap().exists);
    s.create(PW.into()).unwrap();
    assert_eq!(code(s.create(PW.into())), "vault_exists");
    s.lock();
    assert_eq!(
        code(s.create("another long password".into())),
        "vault_exists"
    );
    // a second session object on the same file also sees it
    let other = Session::new(dir.path().join("vault.db"), fast(EDIT_TOKEN_TTL));
    assert_eq!(code(other.create(PW.into())), "vault_exists");
}

#[test]
fn a_locked_vault_refuses_every_command() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    fx.s.lock();
    let d = || login("x", "y");
    assert_eq!(code(fx.s.item_list()), "locked");
    assert_eq!(code(fx.s.item_get(&id)), "locked");
    assert_eq!(code(fx.s.item_reveal(&id, Some(PW.into()))), "locked");
    assert_eq!(code(fx.s.item_begin_edit(&id, Some(PW.into()))), "locked");
    assert_eq!(code(fx.s.item_create(d())), "locked");
    assert_eq!(code(fx.s.item_update(&id, "tok", 1, d())), "locked");
    assert_eq!(code(fx.s.item_trash(&id)), "locked");
    assert_eq!(code(fx.s.item_restore(&id)), "locked");
    assert_eq!(code(fx.s.trash_list()), "locked");
    assert_eq!(code(fx.s.item_purge(&id, PW.into(), "CONFIRM")), "locked");
    assert_eq!(code(fx.s.history_list(&id)), "locked");
    assert_eq!(code(fx.s.history_get(&id, 1, Some(PW.into()))), "locked");
    assert_eq!(
        code(fx.s.history_restore(&id, 1, Some(PW.into()))),
        "locked"
    );
    assert_eq!(code(fx.s.totp_now(&id, None)), "locked");
    assert_eq!(
        code(fx.s.change_password(PW.into(), "another long password".into())),
        "locked"
    );
}

#[test]
fn a_panic_while_the_session_is_held_leaves_the_vault_locked() {
    let fx = Fx::new();
    fx.add("GitHub", "pw-one-111");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fx.s.test_vault(|_| panic!("simulated bug while the vault is open"));
    }));
    assert!(result.is_err());
    assert!(!fx.s.status().unwrap().unlocked, "poisoned state must lock");
    assert_eq!(code(fx.s.item_list()), "locked");
    fx.s.unlock(PW.into()).unwrap();
    assert_eq!(fx.s.item_list().unwrap().len(), 1);
}

// ============================================================ create / list / view

#[test]
fn created_items_start_at_version_one_with_snapshot_and_latest() {
    let fx = Fx::new();
    let sum = fx.s.item_create(login("GitHub", "pw-one-111")).unwrap();
    assert_eq!(sum.version, 1);
    assert_eq!(sum.title, "GitHub");
    assert_eq!(sum.subtitle.as_deref(), Some("ana@example.com"));
    assert!(!sum.has_totp);
    assert_eq!(fx.versions(&sum.id), 1);
    assert_eq!(fx.item_version(&sum.id), 1);
    let h = fx.s.history_list(&sum.id).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].event, ItemEvent::Created);
    assert!(h[0].changed_fields.is_empty());
}

#[test]
fn list_is_newest_first_and_hides_secrets() {
    let fx = Fx::new();
    let a = fx.add("Alpha", "pw-a-aaaa");
    std::thread::sleep(Duration::from_millis(3));
    let b = fx.add("Beta", "pw-b-bbbb");
    std::thread::sleep(Duration::from_millis(3));
    fx.edit(&a, |d| d.favorite = true);
    let list = fx.s.item_list().unwrap();
    assert_eq!(
        list.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        [a.as_str(), b.as_str()]
    );
    assert!(list[0].favorite);
    let json = serde_json::to_string(&list).unwrap();
    assert!(!json.contains("pw-a-aaaa") && !json.contains("pw-b-bbbb"));
}

#[test]
fn subtitles_prefer_username_then_website_host() {
    let fx = Fx::new();
    let mut d = login("No user", "pw-xxxxxx");
    d.fields.remove("username");
    let sum = fx.s.item_create(d).unwrap();
    assert_eq!(sum.subtitle.as_deref(), Some("example.com"));
    let with_user = fx.s.item_create(login("With user", "pw-xxxxxx")).unwrap();
    assert_eq!(with_user.subtitle.as_deref(), Some("ana@example.com"));
}

#[test]
fn get_masks_secrets_and_reveal_returns_them() {
    let fx = Fx::new();
    let mut d = login("GitHub", "s3cret-pass-word");
    d.fields.insert("notes".into(), "recovery: 1234".into());
    d.custom_fields = vec![
        CustomField {
            name: "Plan".into(),
            value: "Team".into(),
            hidden: false,
        },
        CustomField {
            name: "PIN".into(),
            value: "4321".into(),
            hidden: true,
        },
    ];
    let id = fx.s.item_create(d).unwrap().id;

    let view = fx.s.item_get(&id).unwrap();
    let json = serde_json::to_string(&view).unwrap();
    for secret in ["s3cret-pass-word", "recovery: 1234", "4321"] {
        assert!(!json.contains(secret), "{secret} leaked through item_get");
    }
    assert!(json.contains("ana@example.com") && json.contains("Team"));

    let revealed = fx.s.item_reveal(&id, Some(PW.into())).unwrap();
    assert_eq!(revealed.fields["password"], "s3cret-pass-word");
    assert_eq!(revealed.fields["notes"], "recovery: 1234");
    assert_eq!(revealed.custom_fields[1].value, "4321");
}

#[test]
fn malformed_and_unknown_ids() {
    let fx = Fx::new();
    assert_eq!(code(fx.s.item_get("not-a-uuid")), "invalid_input");
    assert_eq!(code(fx.s.item_get("")), "invalid_input");
    assert_eq!(code(fx.s.item_get(MISSING)), "not_found");
    assert_eq!(code(fx.s.item_trash(MISSING)), "not_found");
    assert_eq!(code(fx.s.item_restore(MISSING)), "not_found");
    assert_eq!(code(fx.s.history_list(MISSING)), "not_found");
    assert_eq!(code(fx.s.totp_now(MISSING, None)), "not_found");
    assert_eq!(
        code(fx.s.item_reveal(MISSING, Some(PW.into()))),
        "not_found"
    );
}

// ============================================================ versions and history

#[test]
fn every_state_change_adds_one_snapshot_and_bumps_the_version() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111"); // v1 created
    let mut expected = 1;
    assert_eq!((fx.versions(&id), fx.item_version(&id)), (1, 1));

    fx.edit(&id, |d| {
        d.fields.insert("password".into(), "pw-two-222".into());
    }); // v2 edited
    expected += 1;
    assert_eq!(
        (fx.versions(&id), fx.item_version(&id)),
        (expected, expected)
    );

    fx.s.item_trash(&id).unwrap(); // v3 trashed
    expected += 1;
    assert_eq!(
        (fx.versions(&id), fx.item_version(&id)),
        (expected, expected)
    );

    fx.s.item_restore(&id).unwrap(); // v4 restored
    expected += 1;
    assert_eq!(
        (fx.versions(&id), fx.item_version(&id)),
        (expected, expected)
    );

    fx.s.history_restore(&id, 1, Some(PW.into())).unwrap(); // v5 restoredFromVersion
    expected += 1;
    assert_eq!(
        (fx.versions(&id), fx.item_version(&id)),
        (expected, expected)
    );
    assert_eq!(expected, 5);
}

#[test]
fn history_is_append_only_and_restore_keeps_it_linear() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-A-first");
    fx.edit(&id, |d| {
        d.fields.insert("password".into(), "pw-B-second".into());
    });
    fx.edit(&id, |d| {
        d.fields.insert("password".into(), "pw-C-third".into());
        d.title = "GitHub work".into();
    });

    let before: Vec<Vec<u8>> = (1..=3).map(|v| fx.snapshot_blob(&id, v)).collect();
    let h = fx.s.history_list(&id).unwrap();
    assert_eq!(h.iter().map(|e| e.version).collect::<Vec<_>>(), [3, 2, 1]);
    assert_eq!(
        h.iter().map(|e| e.event).collect::<Vec<_>>(),
        [ItemEvent::Edited, ItemEvent::Edited, ItemEvent::Created]
    );
    assert_eq!(h[0].changed_fields, ["title", "password"]);
    assert_eq!(h[1].changed_fields, ["password"]);
    assert!(h.windows(2).all(|w| w[0].timestamp >= w[1].timestamp));

    // reading an old snapshot needs the password and returns the old content
    assert_eq!(
        code(fx.s.history_get(&id, 1, Some(WRONG.into()))),
        "wrong_password"
    );
    let old = fx.s.history_get(&id, 1, Some(PW.into())).unwrap();
    assert_eq!(old.fields["password"], "pw-A-first");
    assert_eq!(old.title, "GitHub");

    // restore version 1 => a NEW version 4 with the old content
    assert_eq!(
        code(fx.s.history_restore(&id, 1, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(fx.versions(&id), 3, "a rejected restore changes nothing");
    let sum = fx.s.history_restore(&id, 1, Some(PW.into())).unwrap();
    assert_eq!(sum.version, 4);
    assert_eq!(sum.title, "GitHub");
    let now = fx.s.item_reveal(&id, Some(PW.into())).unwrap();
    assert_eq!(now.fields["password"], "pw-A-first");

    let h = fx.s.history_list(&id).unwrap();
    assert_eq!(
        h.iter().map(|e| e.version).collect::<Vec<_>>(),
        [4, 3, 2, 1]
    );
    assert_eq!(h[0].event, ItemEvent::RestoredFromVersion);
    assert_eq!(h[0].restored_from, Some(1));
    assert_eq!(h[0].changed_fields, ["title", "password"]);

    // old snapshots are byte-for-byte untouched
    let after: Vec<Vec<u8>> = (1..=3).map(|v| fx.snapshot_blob(&id, v)).collect();
    assert_eq!(before, after);
    // and snapshot 3 still has the pre-restore content
    assert_eq!(
        fx.s.history_get(&id, 3, Some(PW.into())).unwrap().fields["password"],
        "pw-C-third"
    );
}

#[test]
fn history_get_for_a_missing_version_is_not_found() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    assert_eq!(code(fx.s.history_get(&id, 9, Some(PW.into()))), "not_found");
    assert_eq!(
        code(fx.s.history_restore(&id, 9, Some(PW.into()))),
        "not_found"
    );
}

#[test]
fn saving_without_changes_writes_no_version() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    let b = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    let sum =
        fx.s.item_update(&id, &b.token, b.version, b.draft.clone())
            .unwrap();
    assert_eq!(sum.version, 1);
    assert_eq!(fx.versions(&id), 1);
    assert_eq!(
        fx.s.test_token_count(),
        0,
        "a successful save consumes the token"
    );
}

#[test]
fn an_items_type_cannot_change() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    let b = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    let mut d = b.draft.clone();
    d.item_type = ItemType::SecureNote;
    d.fields = BTreeMap::from([("body".into(), "text".into())]);
    assert_eq!(
        code(fx.s.item_update(&id, &b.token, b.version, d)),
        "invalid_input"
    );
}

// ============================================================ re-prompt guard

#[test]
fn every_gated_command_rejects_a_wrong_password_and_changes_nothing() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    fx.edit(&id, |d| d.favorite = true);
    let trashed = fx.add("Old", "pw-old-000");
    fx.s.item_trash(&trashed).unwrap();
    let (v_before, n_before) = (fx.item_version(&id), fx.versions(&id));

    assert_eq!(
        code(fx.s.item_reveal(&id, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.item_begin_edit(&id, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.history_get(&id, 1, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.history_restore(&id, 1, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.item_purge(&trashed, WRONG.into(), "CONFIRM")),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.change_password(WRONG.into(), "another long password".into())),
        "wrong_password"
    );

    assert_eq!(
        (fx.item_version(&id), fx.versions(&id)),
        (v_before, n_before)
    );
    assert_eq!(
        fx.s.test_token_count(),
        0,
        "no token is issued for a wrong password"
    );
    assert_eq!(
        fx.s.trash_list().unwrap().len(),
        1,
        "the trashed item is still there"
    );
    // the same calls with the right password succeed
    fx.s.item_reveal(&id, Some(PW.into())).unwrap();
    fx.s.history_get(&id, 1, Some(PW.into())).unwrap();
}

#[test]
fn the_password_is_nfkc_normalized_like_at_creation() {
    let dir = tempfile::tempdir().unwrap();
    let s = Session::new(dir.path().join("vault.db"), fast(EDIT_TOKEN_TTL));
    s.create("caf\u{e9}-password".into()).unwrap(); // precomposed é
    let id = s.item_create(login("x", "pw-xxxxxxx")).unwrap().id;
    // decomposed e + combining acute
    s.item_reveal(&id, Some("cafe\u{301}-password".into()))
        .unwrap();
}

#[test]
fn edit_tokens_are_single_use() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    let b = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    assert_eq!(fx.s.test_token_count(), 1);
    let mut d = b.draft.clone();
    d.title = "GitHub 2".into();
    fx.s.item_update(&id, &b.token, b.version, d.clone())
        .unwrap();
    d.title = "GitHub 3".into();
    assert_eq!(
        code(fx.s.item_update(&id, &b.token, b.version, d)),
        "token_invalid"
    );
    assert_eq!(fx.s.item_get(&id).unwrap().title, "GitHub 2");
}

#[test]
fn edit_tokens_are_bound_to_item_and_version() {
    let fx = Fx::new();
    let a = fx.add("Alpha", "pw-a-aaaa");
    let b = fx.add("Beta", "pw-b-bbbb");
    let ta = fx.s.item_begin_edit(&a, Some(PW.into())).unwrap();

    // wrong item: rejected and burned
    let mut d = ta.draft.clone();
    d.title = "Hijacked".into();
    assert_eq!(
        code(fx.s.item_update(&b, &ta.token, 1, d.clone())),
        "token_invalid"
    );
    assert_eq!(fx.s.test_token_count(), 0);
    assert_eq!(fx.s.item_get(&b).unwrap().title, "Beta");

    // wrong expected version: rejected and burned
    let ta = fx.s.item_begin_edit(&a, Some(PW.into())).unwrap();
    assert_eq!(
        code(fx.s.item_update(&a, &ta.token, 7, d.clone())),
        "token_invalid"
    );
    assert_eq!(fx.s.test_token_count(), 0);

    // malformed tokens
    for bad in [
        "",
        "not base64 !!",
        "AAAA",
        &"A".repeat(43),
        &"A".repeat(200),
    ] {
        assert_eq!(
            code(fx.s.item_update(&a, bad, 1, d.clone())),
            "token_invalid",
            "{bad:?}"
        );
    }
    assert_eq!(fx.s.item_get(&a).unwrap().title, "Alpha");
}

#[test]
fn edit_tokens_die_with_the_session_lock() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    let b = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    fx.s.lock();
    let mut d = b.draft.clone();
    d.title = "after lock".into();
    assert_eq!(
        code(fx.s.item_update(&id, &b.token, b.version, d.clone())),
        "locked"
    );
    fx.s.unlock(PW.into()).unwrap();
    assert_eq!(
        code(fx.s.item_update(&id, &b.token, b.version, d)),
        "token_invalid"
    );
    assert_eq!(fx.s.item_get(&id).unwrap().title, "GitHub");
}

#[test]
fn edit_tokens_expire() {
    let fx = Fx::with_ttl(Duration::ZERO);
    let id = fx.add("GitHub", "pw-one-111");
    let b = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    let mut d = b.draft.clone();
    d.title = "too late".into();
    assert_eq!(
        code(fx.s.item_update(&id, &b.token, b.version, d)),
        "token_invalid"
    );
    assert_eq!(fx.s.item_get(&id).unwrap().title, "GitHub");
}

#[test]
fn a_rejected_draft_keeps_the_token_so_the_input_can_be_fixed() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    let b = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    let mut bad = b.draft.clone();
    bad.title = "   ".into();
    assert_eq!(
        code(fx.s.item_update(&id, &b.token, b.version, bad)),
        "invalid_input"
    );
    assert_eq!(fx.s.test_token_count(), 1);
    let mut good = b.draft.clone();
    good.title = "Fixed".into();
    assert_eq!(
        fx.s.item_update(&id, &b.token, b.version, good)
            .unwrap()
            .title,
        "Fixed"
    );
}

#[test]
fn a_stale_edit_is_a_version_conflict_and_burns_the_token() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    let first = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap(); // opened at v1
    fx.edit(&id, |d| d.title = "Changed elsewhere".into()); // now v2
    let mut d = first.draft.clone();
    d.title = "Stale".into();
    assert_eq!(
        code(fx.s.item_update(&id, &first.token, first.version, d)),
        "version_conflict"
    );
    assert_eq!(fx.s.item_get(&id).unwrap().title, "Changed elsewhere");
    assert_eq!(fx.s.test_token_count(), 0);

    // the same rule at the vault level
    let d = login("direct", "pw-direct-1");
    let r = fx.s.test_vault(|v| v.update_item(&id, 1, d));
    assert_eq!(code(r), "version_conflict");
}

#[test]
fn outstanding_tokens_are_capped() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    let first = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    for _ in 0..16 {
        fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    }
    assert_eq!(fx.s.test_token_count(), 16);
    // the oldest one was evicted
    assert_eq!(
        code(fx.s.item_update(&id, &first.token, first.version, first.draft.clone())),
        "token_invalid"
    );
}

#[test]
fn creating_trashing_and_restoring_need_no_password() {
    // (compile-time documentation: these Session methods take no password argument)
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    fx.s.item_trash(&id).unwrap();
    fx.s.item_restore(&id).unwrap();
    assert_eq!(fx.s.item_get(&id).unwrap().title, "GitHub");
}

// ============================================================ per-item re-prompt

#[test]
fn an_ordinary_item_works_without_the_password_once_the_vault_is_open() {
    let fx = Fx::new();
    let mut d = login("Cloud", "pw-cloud-11");
    d.fields.insert("totp".into(), "JBSWY3DPEHPK3PXP".into());
    let id = fx.s.item_create(d).unwrap().id;
    fx.edit(&id, |d| d.title = "Cloud 2".into());

    assert_eq!(
        fx.s.item_reveal(&id, None).unwrap().fields["password"],
        "pw-cloud-11"
    );
    assert_eq!(fx.s.history_get(&id, 1, None).unwrap().title, "Cloud");
    assert_eq!(fx.s.totp_now(&id, None).unwrap().code.len(), 6);
    let b = fx.s.item_begin_edit(&id, None).unwrap();
    assert_eq!(b.version, 2);
    let mut edited = b.draft.clone();
    edited.title = "Cloud 3".into();
    assert_eq!(
        fx.s.item_update(&id, &b.token, b.version, edited)
            .unwrap()
            .version,
        3
    );
    let copy = fx.s.item_clone(&id, None).unwrap();
    assert_eq!(copy.title, "Cloud 3 (copy)");
    assert!(!copy.reprompt);
    assert_eq!(fx.s.history_restore(&id, 1, None).unwrap().version, 4);
}

#[test]
fn a_password_that_is_sent_is_always_checked_even_for_an_ordinary_item() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    let before = (fx.item_version(&id), fx.versions(&id));
    assert_eq!(
        code(fx.s.item_reveal(&id, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.item_begin_edit(&id, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.item_clone(&id, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.history_get(&id, 1, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.history_restore(&id, 1, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!((fx.item_version(&id), fx.versions(&id)), before);
    assert_eq!(fx.s.test_token_count(), 0);
}

#[test]
fn an_item_with_the_reprompt_flag_asks_for_the_master_password_every_time() {
    let fx = Fx::new();
    let mut d = login("Bank", "pw-bank-111");
    d.fields.insert("totp".into(), "JBSWY3DPEHPK3PXP".into());
    d.reprompt = true;
    let made = fx.s.item_create(d).unwrap();
    let id = made.id.clone();
    assert!(made.reprompt);
    assert!(fx.s.item_list().unwrap()[0].reprompt);
    assert!(
        fx.s.item_get(&id).unwrap().reprompt,
        "the masked view says so too"
    );
    fx.edit(&id, |d| d.title = "Bank 2".into()); // version 2

    let (v, n) = (fx.item_version(&id), fx.versions(&id));
    assert_eq!(code(fx.s.item_reveal(&id, None)), "password_required");
    assert_eq!(code(fx.s.item_begin_edit(&id, None)), "password_required");
    assert_eq!(code(fx.s.item_clone(&id, None)), "password_required");
    assert_eq!(code(fx.s.history_get(&id, 1, None)), "password_required");
    assert_eq!(
        code(fx.s.history_restore(&id, 1, None)),
        "password_required"
    );
    // the neighbouring codes live in the same answer, so a flagged item withholds them too
    assert_eq!(code(fx.s.totp_now(&id, None)), "password_required");
    assert_eq!(
        code(fx.s.item_reveal(&id, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.totp_now(&id, Some(WRONG.into()))),
        "wrong_password"
    );
    assert_eq!(
        (fx.item_version(&id), fx.versions(&id)),
        (v, n),
        "nothing changed"
    );
    assert_eq!(fx.s.test_token_count(), 0, "no token without the password");
    assert_eq!(fx.s.item_list().unwrap().len(), 1, "no copy was made");

    // with the right password everything works, and the clone keeps the flag
    assert_eq!(
        fx.s.item_reveal(&id, Some(PW.into())).unwrap().fields["password"],
        "pw-bank-111"
    );
    let shown = fx.s.totp_now(&id, Some(PW.into())).unwrap();
    assert_eq!(
        (shown.code.len(), shown.next.len(), shown.previous.len()),
        (6, 6, 6)
    );
    assert_eq!(
        fx.s.history_get(&id, 1, Some(PW.into())).unwrap().title,
        "Bank"
    );
    assert!(fx.s.item_clone(&id, Some(PW.into())).unwrap().reprompt);
    fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
}

#[test]
fn the_flag_is_a_normal_edit_and_shows_up_in_the_history_by_name_only() {
    let fx = Fx::new();
    let id = fx.add("GitHub", "pw-one-111");
    assert!(!fx.s.item_list().unwrap()[0].reprompt);
    // turning it on needs no password (it only adds protection) ...
    let on = fx.edit(&id, |d| d.reprompt = true);
    assert!(on.reprompt);
    let history = fx.s.history_list(&id).unwrap();
    assert_eq!(history[0].changed_fields, vec!["reprompt".to_string()]);
    // ... turning it off starts from begin_edit, which now asks
    assert_eq!(code(fx.s.item_begin_edit(&id, None)), "password_required");
    let b = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    assert!(b.draft.reprompt);
    let mut d = b.draft.clone();
    d.reprompt = false;
    assert!(
        !fx.s
            .item_update(&id, &b.token, b.version, d)
            .unwrap()
            .reprompt
    );
    assert!(fx.s.item_reveal(&id, None).is_ok());
}

#[test]
fn the_flag_survives_a_restart_and_the_trash() {
    let fx = Fx::new();
    let id = fx.add("Bank", "pw-bank-111");
    fx.edit(&id, |d| d.reprompt = true);
    fx.s.item_trash(&id).unwrap();
    fx.s.item_restore(&id).unwrap();
    assert_eq!(code(fx.s.item_reveal(&id, None)), "password_required");
    let again = fx.reopen();
    again.unlock(PW.into()).unwrap();
    assert!(again.item_list().unwrap()[0].reprompt);
    assert_eq!(code(again.item_reveal(&id, None)), "password_required");
    assert!(again.item_reveal(&id, Some(PW.into())).is_ok());
    // trashed and unknown items keep their own errors, flag or not
    fx.s.item_trash(&id).unwrap();
    assert_eq!(code(fx.s.item_reveal(&id, None)), "in_trash");
    assert_eq!(code(fx.s.item_reveal(MISSING, None)), "not_found");
    assert_eq!(code(fx.s.totp_now(MISSING, None)), "not_found");
}

#[test]
fn the_commands_that_never_had_a_per_item_exception_still_ask_every_time() {
    // `item_purge` and `change_password` take a plain `String`: there is no way to call them without a password.
    let fx = Fx::new();
    let id = fx.add("Old", "pw-old-000");
    fx.s.item_trash(&id).unwrap();
    assert_eq!(
        code(fx.s.item_purge(&id, WRONG.into(), "CONFIRM")),
        "wrong_password"
    );
    assert_eq!(
        code(fx.s.change_password(WRONG.into(), "another long password".into())),
        "wrong_password"
    );
    fx.s.item_purge(&id, PW.into(), "CONFIRM").unwrap();
}

// ============================================================ trash

#[test]
fn the_trash_hides_everything_but_title_and_time() {
    let fx = Fx::new();
    let mut d = login("Bank", "bank-secret-pw");
    d.fields.insert("totp".into(), "JBSWY3DPEHPK3PXP".into());
    d.fields.insert("notes".into(), "very private note".into());
    let id = fx.s.item_create(d).unwrap().id;
    let token = fx.s.item_begin_edit(&id, Some(PW.into())).unwrap();
    fx.s.item_trash(&id).unwrap();

    assert_eq!(code(fx.s.item_get(&id)), "in_trash");
    assert_eq!(code(fx.s.item_reveal(&id, Some(PW.into()))), "in_trash");
    assert_eq!(code(fx.s.item_begin_edit(&id, Some(PW.into()))), "in_trash");
    assert_eq!(code(fx.s.history_list(&id)), "in_trash");
    assert_eq!(code(fx.s.history_get(&id, 1, Some(PW.into()))), "in_trash");
    assert_eq!(
        code(fx.s.history_restore(&id, 1, Some(PW.into()))),
        "in_trash"
    );
    assert_eq!(code(fx.s.totp_now(&id, None)), "in_trash");
    assert_eq!(code(fx.s.item_trash(&id)), "in_trash");
    // an edit token handed out before the trashing cannot save into the trash
    assert_eq!(
        code(fx.s.item_update(&id, &token.token, token.version, token.draft.clone())),
        "in_trash"
    );
    assert_eq!(fx.s.test_token_count(), 0);

    assert!(fx.s.item_list().unwrap().is_empty());
    let trash = fx.s.trash_list().unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(
        (trash[0].id.as_str(), trash[0].title.as_str()),
        (id.as_str(), "Bank")
    );
    assert!(trash[0].deleted_at > 1_700_000_000_000);
    let json = serde_json::to_value(&trash).unwrap();
    assert_eq!(
        json[0].as_object().unwrap().len(),
        3,
        "exactly id, title, deletedAt"
    );
    let text = json.to_string();
    for secret in [
        "bank-secret-pw",
        "JBSWY3DPEHPK3PXP",
        "very private note",
        "ana@example.com",
    ] {
        assert!(
            !text.contains(secret),
            "{secret} visible in the trash listing"
        );
    }
}

#[test]
fn restore_returns_full_access_with_a_new_version() {
    let fx = Fx::new();
    let id = fx.add("Bank", "bank-secret-pw");
    fx.s.item_trash(&id).unwrap();
    assert_eq!(code(fx.s.item_restore(MISSING)), "not_found");
    let sum = fx.s.item_restore(&id).unwrap();
    assert_eq!(sum.version, 3);
    assert_eq!(code(fx.s.item_restore(&id)), "not_in_trash");
    assert_eq!(
        fx.s.item_reveal(&id, Some(PW.into())).unwrap().fields["password"],
        "bank-secret-pw"
    );
    assert!(fx.s.trash_list().unwrap().is_empty());
    let h = fx.s.history_list(&id).unwrap();
    assert_eq!(
        h.iter().map(|e| e.event).collect::<Vec<_>>(),
        [ItemEvent::Restored, ItemEvent::Trashed, ItemEvent::Created]
    );
}

#[test]
fn nothing_is_ever_purged_automatically() {
    let fx = Fx::new();
    let id = fx.add("Bank", "bank-secret-pw");
    fx.s.item_trash(&id).unwrap();
    for _ in 0..3 {
        fx.s.lock();
        let again = fx.reopen();
        again.unlock(PW.into()).unwrap();
        assert_eq!(again.trash_list().unwrap().len(), 1);
        assert_eq!(
            again.test_vault(|v| v.store().count_versions(&id).unwrap()),
            2
        );
    }
}

#[test]
fn purge_needs_the_trash_and_the_password() {
    // (the test configuration treats every trashed item as an old one, so the sum is not in play here; it is in
    // `folder_search_tests.rs`)
    let fx = Fx::new();
    let id = fx.add("Bank account", "bank-secret-pw");

    // active items cannot be purged, whatever else is right or wrong
    assert_eq!(code(fx.s.item_purge(&id, PW.into(), "")), "not_in_trash");
    assert_eq!(
        code(fx.s.item_purge(&id, WRONG.into(), "x")),
        "not_in_trash"
    );

    fx.s.item_trash(&id).unwrap();
    // wrong password
    assert_eq!(
        code(fx.s.item_purge(&id, WRONG.into(), "")),
        "wrong_password"
    );
    assert_eq!(fx.s.trash_list().unwrap().len(), 1, "still in the trash");
    assert_eq!(fx.versions(&id), 2);

    // the right password is enough for an old item, whatever else is typed
    fx.s.item_purge(&id, PW.into(), "").unwrap();
    assert!(fx.s.trash_list().unwrap().is_empty());
    assert_eq!(code(fx.s.item_get(&id)), "not_found");
}

#[test]
fn purge_erases_every_row_of_the_item_and_only_that_item() {
    let fx = Fx::new();
    let gone = fx.add("Doomed", "pw-doomed-1");
    let kept = fx.add("Kept", "pw-kept-111");
    fx.edit(&gone, |d| d.favorite = true);
    fx.edit(&kept, |d| d.favorite = true);
    for id in [&gone, &kept] {
        let id = id.as_str();
        fx.s.test_vault(|v| {
            let c = v.store().conn();
            c.execute(
                "INSERT INTO attachments (id, item_id, size) VALUES ('att-' || ?1, ?1, 5)",
                [id],
            )
            .unwrap();
            c.execute(
                "INSERT INTO conflicts (item_id, ts, blob) VALUES (?1, 1, x'00')",
                [id],
            )
            .unwrap();
        });
    }
    fx.s.item_trash(&gone).unwrap();
    fx.s.item_purge(&gone, PW.into(), "CONFIRM").unwrap();

    fx.s.test_vault(|v| {
        for (t, col) in [
            ("items", "id"),
            ("versions", "item_id"),
            ("attachments", "item_id"),
            ("conflicts", "item_id"),
        ] {
            assert_eq!(
                v.store().count(t, col, &gone),
                0,
                "{t} still holds rows of the purged item"
            );
            assert!(
                v.store().count(t, col, &kept) > 0,
                "{t} lost rows of the other item"
            );
        }
        assert!(v.store().is_tombstoned(&gone).unwrap());
        assert!(!v.store().is_tombstoned(&kept).unwrap());
    });
    assert_eq!(fx.s.item_get(&kept).unwrap().title, "Kept");
    assert_eq!(fx.versions(&kept), 2);

    // a tombstoned id can never be created again
    let r =
        fx.s.test_vault(|v| v.create_item_with_id(gone.clone(), login("Zombie", "pw-zombie-1")));
    assert_eq!(code(r), "invalid_input");
    assert_eq!(code(fx.s.item_get(&gone)), "not_found");

    // and it survives a restart
    fx.s.lock();
    let again = fx.reopen();
    again.unlock(PW.into()).unwrap();
    assert!(again.trash_list().unwrap().is_empty());
    let r = again.test_vault(|v| v.create_item_with_id(gone, login("Zombie", "pw-zombie-1")));
    assert_eq!(code(r), "invalid_input");
}

// ============================================================ TOTP

#[test]
fn a_login_with_two_codes_gives_each_its_own_code_and_refuses_a_wrong_key() {
    let fx = Fx::new();
    let mut d = login("Amazon", "pw-amazon-1");
    d.fields.insert("totp".into(), "JBSWY3DPEHPK3PXP".into());
    d.fields.insert("totp#2".into(), "ONSWG4TFOQYTEMZU".into());
    d.fields
        .insert("username#2".into(), "work@example.com".into());
    d.fields
        .insert("password#2".into(), "work-pw-123456".into());
    let sum = fx.s.item_create(d).unwrap();
    assert!(sum.has_totp);

    let first = fx.s.totp_now(&sum.id, None).unwrap();
    let by_key = |k: &str| fx.s.totp_now_of(&sum.id, Some(k), None);
    assert_eq!(by_key("totp").unwrap().code, first.code);
    let second = by_key("totp#2").unwrap();
    assert_eq!(second.code.len(), 6);
    assert_ne!(second.code, first.code, "different seeds, different codes");
    // only a code field can be asked for: not a password, not a number that is not stored
    for bad in ["password#2", "totp#3", "totp#1", "url", "nope"] {
        assert_eq!(code(by_key(bad)), "invalid_input", "{bad}");
    }

    // the masked view carries both sign-ins and both codes, and no secret
    let view = fx.s.item_get(&sum.id).unwrap();
    let keys: Vec<&str> = view.fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "username",
            "username#2",
            "password",
            "password#2",
            "url",
            "totp",
            "totp#2"
        ]
    );
    let json = serde_json::to_string(&view).unwrap();
    assert!(!json.contains("work-pw-123456") && !json.contains("ONSWG4TFOQYTEMZU"));
}

#[test]
fn totp_codes_are_computed_in_rust_and_the_secret_stays_inside() {
    let fx = Fx::new();
    let mut d = login("Cloud", "pw-cloud-11");
    d.fields.insert("totp".into(), "JBSWY3DPEHPK3PXP".into());
    let sum = fx.s.item_create(d).unwrap();
    assert!(sum.has_totp);

    let now = fx.s.totp_now(&sum.id, None).unwrap();
    assert_eq!(now.digits, 6);
    assert_eq!(now.period, 30);
    assert!((1..=30).contains(&now.seconds_remaining));
    assert_eq!(now.code.len(), 6);
    assert!(now.code.chars().all(|c| c.is_ascii_digit()));
    assert_eq!(
        now.valid_until_ms % 30_000,
        0,
        "windows are aligned to the period"
    );

    // independent recomputation for the window that ends at valid_until_ms
    let account =
        crate::crypto::TotpAccount::new(&crate::crypto::TotpParams::defaults("JBSWY3DPEHPK3PXP"))
            .unwrap();
    assert_eq!(
        now.code,
        account.code_at(now.valid_until_ms / 1000 - 30).code
    );
    // the window after this one, and the window before it, recomputed independently
    assert_eq!(now.next, account.code_at(now.valid_until_ms / 1000).code);
    assert_eq!(
        now.previous,
        account.code_at(now.valid_until_ms / 1000 - 60).code
    );
    assert_eq!((now.next.len(), now.previous.len()), (6, 6));

    let text = serde_json::to_string(&now).unwrap();
    assert!(!text.contains("JBSWY3DPEHPK3PXP"));
    assert!(!serde_json::to_string(&fx.s.item_get(&sum.id).unwrap())
        .unwrap()
        .contains("JBSWY3DPEHPK3PXP"));
}

#[test]
fn totp_from_an_otpauth_uri_and_without_a_secret() {
    let fx = Fx::new();
    let uri = "otpauth://totp/Acme:ana@example.com?secret=JBSWY3DPEHPK3PXP&issuer=Acme&digits=8&period=60";
    let d = ItemDraft {
        item_type: ItemType::Totp,
        title: "Acme".into(),
        favorite: false,
        reprompt: false,
        tags: vec![],
        folder: None,
        fields: BTreeMap::from([
            ("totp".into(), uri.into()),
            ("issuer".into(), "Acme".into()),
        ]),
        custom_fields: vec![],
    };
    let sum = fx.s.item_create(d).unwrap();
    assert_eq!(sum.subtitle.as_deref(), Some("Acme"));
    let now = fx.s.totp_now(&sum.id, None).unwrap();
    assert_eq!((now.digits, now.period), (8, 60));

    let plain = fx.add("No code", "pw-plain-11");
    assert_eq!(code(fx.s.totp_now(&plain, None)), "invalid_input");
}

#[test]
fn invalid_content_never_reaches_the_database() {
    let fx = Fx::new();
    let mut bad = login("", "pw");
    assert_eq!(code(fx.s.item_create(bad.clone())), "invalid_input");
    bad.title = "ok".into();
    bad.fields.insert("totp".into(), "!!!".into());
    assert_eq!(code(fx.s.item_create(bad)), "invalid_input");
    assert!(fx.s.item_list().unwrap().is_empty());
    fx.s.test_vault(|v| {
        let n: i64 = v
            .store()
            .conn()
            .query_row("SELECT count(*) FROM items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    });
}

// ============================================================ persistence and integrity

#[test]
fn everything_survives_closing_and_reopening() {
    let fx = Fx::new();
    let a = fx.add("Alpha", "pw-a-aaaa");
    let b = fx.add("Beta", "pw-b-bbbb");
    fx.edit(&a, |d| {
        d.fields.insert("password".into(), "pw-a-changed".into());
        d.tags = vec!["work".into()];
    });
    fx.s.item_trash(&b).unwrap();
    fx.s.lock();

    let again = fx.reopen();
    assert!(again.status().unwrap().exists);
    assert_eq!(code(again.item_list()), "locked");
    again.unlock(PW.into()).unwrap();
    let list = again.item_list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (
            list[0].title.as_str(),
            list[0].version,
            list[0].tags.clone()
        ),
        ("Alpha", 2, vec!["work".to_string()])
    );
    assert_eq!(again.trash_list().unwrap()[0].title, "Beta");
    assert_eq!(
        again.item_reveal(&a, Some(PW.into())).unwrap().fields["password"],
        "pw-a-changed"
    );
    assert_eq!(again.history_list(&a).unwrap().len(), 2);
    assert_eq!(code(again.unlock(WRONG.into())), "wrong_password");
}

#[test]
fn tampered_item_blobs_are_errors_and_never_panics() {
    let fx = Fx::new();
    let a = fx.add("Alpha", "pw-a-aaaa");
    let b = fx.add("Beta", "pw-b-bbbb");

    // flip one ciphertext byte of A's latest state (kept as a real BLOB)
    fx.s.test_vault(|v| {
        let mut blob = v.store().item_row(&a).unwrap().unwrap().blob;
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        v.store()
            .conn()
            .execute(
                "UPDATE items SET blob = ?2 WHERE id = ?1",
                rusqlite::params![&a, blob],
            )
            .unwrap();
    });
    for r in [
        code(fx.s.item_get(&a)),
        code(fx.s.item_reveal(&a, Some(PW.into()))),
        code(fx.s.item_begin_edit(&a, Some(PW.into()))),
        code(fx.s.totp_now(&a, None)),
    ] {
        assert_eq!(r, "crypto");
    }
    assert_eq!(
        fx.s.item_get(&b).unwrap().title,
        "Beta",
        "other items are unaffected"
    );

    // at the next unlock the damaged item is skipped and counted; the vault still opens
    fx.s.lock();
    fx.s.unlock(PW.into()).unwrap();
    assert_eq!(fx.s.status().unwrap().damaged_items, 1);
    let list = fx.s.item_list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, b);
}

#[test]
fn swapped_truncated_and_empty_blobs_are_rejected() {
    let fx = Fx::new();
    let a = fx.add("Alpha", "pw-a-aaaa");
    let b = fx.add("Beta", "pw-b-bbbb");
    let blob_of = |id: &str| -> Vec<u8> {
        fx.s.test_vault(|v| v.store().item_row(id).unwrap().unwrap().blob)
    };
    let set = |id: &str, blob: &[u8]| {
        fx.s.test_vault(|v| {
            v.store()
                .conn()
                .execute(
                    "UPDATE items SET blob = ?2 WHERE id = ?1",
                    rusqlite::params![id, blob],
                )
                .unwrap();
        });
    };
    let (blob_a, blob_b) = (blob_of(&a), blob_of(&b));

    set(&a, &blob_b); // B's valid envelope in A's row
    assert_eq!(code(fx.s.item_get(&a)), "crypto");
    set(&a, &blob_a[..blob_a.len() / 2]); // truncated
    assert_eq!(code(fx.s.item_get(&a)), "crypto");
    set(&a, &[]); // empty
    assert_eq!(code(fx.s.item_get(&a)), "crypto");
    set(&a, &[0u8; 64]); // garbage
    assert_eq!(code(fx.s.item_get(&a)), "crypto");
    // a version SNAPSHOT of A (different key purpose) is not accepted as the latest state
    let snap = fx.snapshot_blob(&a, 1);
    set(&a, &snap);
    assert_eq!(code(fx.s.item_get(&a)), "crypto");
    // a column of the wrong SQLite type (TEXT instead of BLOB) is a clean storage error
    fx.s.test_vault(|v| {
        v.store()
            .conn()
            .execute("UPDATE items SET blob = 'not a blob' WHERE id = ?1", [&a])
            .unwrap();
    });
    assert_eq!(code(fx.s.item_get(&a)), "storage");
    set(&a, &blob_a); // restored: works again
    assert_eq!(fx.s.item_get(&a).unwrap().title, "Alpha");
}

#[test]
fn tampered_snapshots_fail_history_without_panicking() {
    let fx = Fx::new();
    let id = fx.add("Alpha", "pw-a-aaaa");
    fx.edit(&id, |d| d.favorite = true);
    fx.s.test_vault(|v| {
        let mut blob = v.store().version_row(&id, 1).unwrap().unwrap().blob;
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        v.store()
            .conn()
            .execute(
                "UPDATE versions SET blob = ?2 WHERE item_id = ?1 AND version = 1",
                rusqlite::params![&id, blob],
            )
            .unwrap();
    });
    assert_eq!(code(fx.s.history_list(&id)), "crypto");
    assert_eq!(code(fx.s.history_get(&id, 1, Some(PW.into()))), "crypto");
    assert_eq!(
        code(fx.s.history_restore(&id, 1, Some(PW.into()))),
        "crypto"
    );
    // the intact snapshot is still readable and nothing changed
    assert!(fx.s.history_get(&id, 2, Some(PW.into())).unwrap().favorite);
    assert_eq!(fx.item_version(&id), 2);
}

#[test]
fn damaged_key_material_is_reported_not_trusted() {
    let fx = Fx::new();
    fx.s.lock();
    // corrupt the stored bundle JSON
    let path = fx.dir.path().join("vault.db");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute(
        "UPDATE meta SET value = '{\"not\":\"a bundle\"}' WHERE key = 'bundle'",
        [],
    )
    .unwrap();
    drop(c);
    let again = fx.reopen();
    let e = again.unlock(PW.into()).unwrap_err();
    assert!(matches!(e.code(), "crypto" | "storage"), "{e:?}");
    assert!(!again.status().unwrap().unlocked);
}

// ============================================================ master password change

#[test]
fn changing_the_password_re_seals_keys_only() {
    let fx = Fx::new();
    let id = fx.add("Alpha", "pw-a-aaaa");
    let blob_before = fx.snapshot_blob(&id, 1);
    let new_pw = "a brand new master password";

    assert_eq!(
        code(fx.s.change_password(PW.into(), "short".into())),
        "invalid_input"
    );
    assert_eq!(
        code(fx.s.change_password(WRONG.into(), new_pw.into())),
        "wrong_password"
    );
    fx.s.change_password(PW.into(), new_pw.into()).unwrap();
    assert_eq!(
        fx.snapshot_blob(&id, 1),
        blob_before,
        "no data is re-encrypted"
    );
    // the live session keeps working, with the new password for the guard
    assert_eq!(
        code(fx.s.item_reveal(&id, Some(PW.into()))),
        "wrong_password"
    );
    assert_eq!(
        fx.s.item_reveal(&id, Some(new_pw.into())).unwrap().fields["password"],
        "pw-a-aaaa"
    );

    fx.s.lock();
    let again = fx.reopen();
    assert_eq!(code(again.unlock(PW.into())), "wrong_password");
    again.unlock(new_pw.into()).unwrap();
    assert_eq!(again.item_list().unwrap().len(), 1);
}

// ============================================================ scale

#[test]
fn five_hundred_items_list_and_unlock_quickly() {
    let fx = Fx::new();
    fx.s.test_vault(|v| {
        v.store()
            .conn()
            .pragma_update(None, "synchronous", "OFF")
            .unwrap();
    });
    for i in 0..500 {
        let mut d = login(&format!("Site {i:03}"), &format!("pw-number-{i:03}"));
        d.tags = vec![format!("group{}", i % 7)];
        fx.s.item_create(d).unwrap();
    }
    let t = std::time::Instant::now();
    let list = fx.s.item_list().unwrap();
    let list_time = t.elapsed();
    assert_eq!(list.len(), 500);
    assert!(
        list_time < Duration::from_secs(1),
        "list took {list_time:?}"
    );

    fx.s.lock();
    let t = std::time::Instant::now();
    fx.s.unlock(PW.into()).unwrap();
    let unlock_time = t.elapsed();
    assert_eq!(fx.s.item_list().unwrap().len(), 500);
    assert_eq!(fx.s.status().unwrap().damaged_items, 0);
    assert!(
        unlock_time < Duration::from_secs(10),
        "unlock with 500 items took {unlock_time:?}"
    );
    eprintln!("500 items: list {list_time:?}, unlock incl. Argon2id + index build {unlock_time:?}");
}

#[test]
fn custom_fields_round_trip_in_order() {
    let fx = Fx::new();
    let mut d = login("Router", "pw-router-1");
    d.custom_fields = (0..5)
        .map(|i| CustomField {
            name: format!("Field {i}"),
            value: format!("value {i}"),
            hidden: i % 2 == 0,
        })
        .collect();
    let id = fx.s.item_create(d.clone()).unwrap().id;
    let back = fx.s.item_reveal(&id, Some(PW.into())).unwrap();
    assert_eq!(back.custom_fields, d.custom_fields);
    let view = fx.s.item_get(&id).unwrap();
    assert_eq!(view.custom_fields.len(), 5);
    assert_eq!(view.custom_fields[0].value, None);
    assert_eq!(view.custom_fields[1].value.as_deref(), Some("value 1"));
}

#[test]
fn errors_serialize_as_code_and_message() {
    let fx = Fx::new();
    let e: StorageError = fx.s.item_get(MISSING).unwrap_err();
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["code"], "not_found");
    assert!(v["message"].as_str().unwrap().len() > 5);
}
