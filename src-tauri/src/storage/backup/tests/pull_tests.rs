//! Pull and reconcile (`STORAGE_AND_SYNC.md` §6): two simulated devices on in-memory buckets
//! (profile "A" uploads, profile "B" is restored from the bucket and edits, both share the
//! same keys). Covers fast-forward, local ahead, diverged (with the other-device copy, the review
//! flag and the merge that lets the other device fast-forward), the rollback guard, tombstones,
//! damaged remote objects, the idempotent second pass, new remote items with their history,
//! the unreachable source, and a pull that never blocks the unlock.

use super::*;
use crate::storage::backup::pull_apply::{Applied, Plan};

const NEW_ID: &str = "11111111-1111-4111-8111-111111111111";
const NEW_ID_2: &str = "22222222-2222-4222-8222-222222222222";

fn password_of(fx: &Fx, id: &str) -> String {
    fx.app.item_reveal(id, Some(PW.into())).unwrap().fields["password"].clone()
}

fn version_of(fx: &Fx, id: &str) -> u64 {
    fx.app.item_get(id).unwrap().version
}

fn other_device_entries(fx: &Fx, id: &str) -> usize {
    fx.app
        .history_list(id)
        .unwrap()
        .iter()
        .filter(|e| e.other_device)
        .count()
}

fn sync_revision(fx: &Fx) -> u64 {
    fx.app.status().unwrap().sync_revision
}

/// What the person's own device (profile A) holds for `id`, as bytes.
fn remote_item(fx: &Fx, id: &str) -> Vec<u8> {
    fx.bucket("bkt").raw(&keys::item(id)).unwrap()
}

// ============================================================ fast-forward

#[test]
fn a_fast_forward_takes_over_the_other_devices_edits_with_the_history_on_the_way() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx); // B: v2 "from-B", uploaded
    edit_password(&fx.app, &x, "from-B-again"); // B: v3
    fx.run();

    fx.switch_to("A"); // A still holds v1
    assert_eq!(version_of(&fx, &x), 1);
    assert_eq!(sync_revision(&fx), 0);
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(
        views[0].health.queued, 0,
        "nothing to send back: A is the same as the bucket"
    );

    assert_eq!(version_of(&fx, &x), 3);
    assert_eq!(password_of(&fx, &x), "from-B-again");
    assert_eq!(
        item_blob(&fx, &x),
        remote_item(&fx, &x),
        "A now holds the bucket's bytes"
    );
    let entries = fx.app.history_list(&x).unwrap();
    assert_eq!(
        entries.iter().map(|e| e.version).collect::<Vec<_>>(),
        [3, 2, 1],
        "the states between the old head and the new one came along"
    );
    assert!(entries.iter().all(|e| !e.other_device));
    assert!(!fx.app.item_get(&x).unwrap().changed_elsewhere);
    assert_eq!(sync_revision(&fx), 1, "the page is told something changed");
    let listed = fx.app.item_list().unwrap();
    assert_eq!(listed.iter().find(|i| i.id == x).unwrap().version, 3);
    assert_eq!(fx.app.status().unwrap().remote_damaged, 0);
    assert!(!fx.app.status().unwrap().reconciling);
}

#[test]
fn a_state_records_the_state_it_replaced() {
    let fx = Fx::with_target(manual());
    let x = fx.app.item_create(login("Bank", "pw1")).unwrap().id;
    edit_password(&fx.app, &x, "pw2");
    fx.app.history_restore(&x, 1, Some(PW.into())).unwrap();
    let chain = fx.app.test_session().test_vault(|v| {
        let rows = v.store().version_rows(&x).unwrap();
        rows.iter()
            .map(|r| {
                let (payload, _) = v.open_snapshot(&x, r.version, &r.blob).unwrap();
                (r.version, r.ts, payload.parent)
            })
            .collect::<Vec<_>>()
    });
    // newest first: v3 -> v2 -> v1 -> nothing
    assert_eq!(chain.len(), 3);
    assert_eq!(chain[2].2, None);
    for pair in chain.windows(2) {
        let (newer, older) = (&pair[0], &pair[1]);
        let parent = newer.2.expect("every later state names its parent");
        assert_eq!((parent.version, parent.ts), (older.0, older.1));
    }
}

// ============================================================ local ahead

#[test]
fn a_local_state_that_is_ahead_of_the_bucket_is_left_alone_and_then_pushed() {
    let fx = Fx::with_target(manual());
    let x = fx.app.item_create(login("Bank", "pw1")).unwrap().id;
    fx.run(); // the bucket holds v1
    edit_password(&fx.app, &x, "pw2"); // local v2, not sent yet
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy);
    assert_eq!(sync_revision(&fx), 0, "nothing was taken over");
    assert_eq!(version_of(&fx, &x), 2);
    assert_eq!(password_of(&fx, &x), "pw2");
    assert_eq!(
        remote_item(&fx, &x),
        item_blob(&fx, &x),
        "the push replaced the old state"
    );
    assert_eq!(other_device_entries(&fx, &x), 0);
}

// ============================================================ diverged

#[test]
fn a_diverged_item_keeps_the_local_head_and_stores_the_other_one_as_another_devices_version() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx); // the bucket holds B's v2
    let theirs = remote_item(&fx, &x);

    fx.switch_to("A");
    edit_password(&fx.app, &x, "from-A"); // A's own v2
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(views[0].health.queued, 0);

    // the local head stays; the bucket's head is not touched
    assert_eq!(version_of(&fx, &x), 2);
    assert_eq!(password_of(&fx, &x), "from-A");
    assert_eq!(remote_item(&fx, &x), theirs);
    // the other device's state is one more, immutable version, flagged
    let view = fx.app.item_get(&x).unwrap();
    assert!(view.changed_elsewhere);
    let entries = fx.app.history_list(&x).unwrap();
    assert_eq!(entries.len(), 3);
    let copy = entries.iter().find(|e| e.other_device).unwrap();
    assert_eq!(copy.version, 3, "above every number in use here");
    assert_eq!(copy.event, crate::storage::model::ItemEvent::Edited);
    assert_eq!(
        entries
            .iter()
            .filter(|e| !e.other_device)
            .map(|e| e.version)
            .collect::<Vec<_>>(),
        [2, 1]
    );
    assert_eq!(
        fx.app.history_get(&x, 3, Some(PW.into())).unwrap().fields["password"],
        "from-B"
    );
    assert_eq!(sync_revision(&fx), 1);

    // a second pass over the unchanged bucket downloads nothing and adds nothing
    fx.bucket("bkt").clear_log();
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy);
    assert_eq!(
        fx.bucket("bkt").count(Op::Get),
        0,
        "{:?}",
        fx.bucket("bkt").log()
    );
    assert_eq!(fx.app.history_list(&x).unwrap().len(), 3);
    assert_eq!(sync_revision(&fx), 1);

    // "Got it" clears the flag, the copy stays in the history
    fx.app.item_review_other_device(&x).unwrap();
    assert!(!fx.app.item_get(&x).unwrap().changed_elsewhere);
    assert_eq!(other_device_entries(&fx, &x), 1);
    assert_eq!(
        code(fx.app.item_review_other_device("not-an-id")),
        "invalid_input"
    );
}

#[test]
fn restoring_the_other_devices_copy_clears_the_flag_and_lets_that_device_fast_forward() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx);
    fx.switch_to("A");
    edit_password(&fx.app, &x, "from-A");
    fx.run();
    assert!(fx.app.item_get(&x).unwrap().changed_elsewhere);

    // the existing restore-from-version, on the copy
    let restored = fx.app.history_restore(&x, 3, Some(PW.into())).unwrap();
    assert_eq!(restored.version, 4);
    assert_eq!(password_of(&fx, &x), "from-B");
    assert!(
        !fx.app.item_get(&x).unwrap().changed_elsewhere,
        "restoring a copy from another device counts as looking at it"
    );
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(
        remote_item(&fx, &x),
        item_blob(&fx, &x),
        "the merged state is in the bucket"
    );

    // B grew into the merged state: a plain fast-forward, nothing flagged
    fx.switch_to("B");
    assert_eq!(version_of(&fx, &x), 2);
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(version_of(&fx, &x), 4);
    assert_eq!(password_of(&fx, &x), "from-B");
    assert_eq!(other_device_entries(&fx, &x), 0);
    assert!(!fx.app.item_get(&x).unwrap().changed_elsewhere);
    assert_eq!(item_blob(&fx, &x), remote_item(&fx, &x));
}

#[test]
fn a_device_that_keeps_its_own_state_ends_up_with_both_on_the_other_device_too() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx);
    fx.switch_to("A");
    edit_password(&fx.app, &x, "from-A");
    fx.run(); // A: diverged; the bucket keeps B's head
    edit_password(&fx.app, &x, "from-A-later"); // A's next edit supersedes it
    fx.run();
    assert_eq!(remote_item(&fx, &x), item_blob(&fx, &x));

    fx.switch_to("B");
    fx.run();
    // B's own head stays; A's line of edits is the copy
    assert_eq!(password_of(&fx, &x), "from-B");
    assert!(fx.app.item_get(&x).unwrap().changed_elsewhere);
    assert_eq!(other_device_entries(&fx, &x), 1);
}

// ============================================================ rollback guard

#[test]
fn an_older_state_in_the_bucket_than_one_already_seen_is_not_merged_and_is_replaced_later() {
    let fx = Fx::new(manual());
    fx.app.profile_create("A", PW.into()).unwrap();
    let mut t = inp("Primary", "bkt");
    t.master = true;
    fx.app.backup_add(t).unwrap();
    let x = fx.app.item_create(login("Shared item", "v1")).unwrap().id;
    fx.run();
    fx.app
        .profile_restore_from_backup("B", inp("Restore source", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();

    fx.switch_to("A");
    edit_password(&fx.app, &x, "a2");
    fx.run();
    let a2 = remote_item(&fx, &x); // A's v2, as it was in the bucket
    edit_password(&fx.app, &x, "a3");
    fx.run(); // the bucket now holds A's v3

    fx.switch_to("B");
    edit_password(&fx.app, &x, "b2"); // B's own v2, B has not pulled yet
    fx.run();
    // B saw A's v3 and kept both: one copy from another device
    assert_eq!(password_of(&fx, &x), "b2");
    assert_eq!(other_device_entries(&fx, &x), 1);

    // the bucket is rolled back to a state older than what B has seen (and not part of its line)
    fx.bucket("bkt").plant(&keys::item(&x), &a2);
    fx.run();
    assert_eq!(
        other_device_entries(&fx, &x),
        1,
        "the stale state was not merged as one more copy"
    );
    assert_eq!(password_of(&fx, &x), "b2");

    // the next edit of B replaces the stale state in the bucket
    edit_password(&fx.app, &x, "b3");
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(remote_item(&fx, &x), item_blob(&fx, &x));
}

// ============================================================ tombstones

#[test]
fn a_tombstone_from_the_bucket_erases_the_item_here_and_is_passed_on_but_not_back() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx); // B is open and holds X (v2)
                              // B also has a replica target
    let mut replica = inp("Replica", "rep");
    replica.master = false;
    fx.app.backup_add(replica).unwrap();
    fx.run();
    assert!(bucket_has(&fx.bucket("rep"), &keys::item(&x)));

    // A deletes X for good
    fx.switch_to("A");
    fx.app.item_trash(&x).unwrap();
    fx.app.item_purge(&x, PW.into(), "CONFIRM").unwrap();
    fx.run();
    assert!(bucket_has(&fx.bucket("bkt"), &keys::tombstone(&x)));
    assert!(!bucket_has(&fx.bucket("bkt"), &keys::item(&x)));

    // B pulls
    fx.switch_to("B");
    fx.bucket("bkt").clear_log();
    fx.run();
    assert_eq!(code(fx.app.item_get(&x)), "not_found");
    assert_eq!(
        count_rows(&fx, "versions"),
        0,
        "no snapshot of the erased item is left"
    );
    assert!(fx.app.item_list().unwrap().is_empty());
    assert_eq!(sync_revision(&fx), 1);
    // the source already knew: nothing was deleted or written there
    assert_eq!(fx.bucket("bkt").count(Op::Delete), 0);
    let writes: Vec<String> = fx
        .bucket("bkt")
        .log()
        .into_iter()
        .filter(|(op, key, _)| *op == Op::Put && key != keys::TARGETS)
        .map(|(_, key, _)| key)
        .collect();
    assert!(
        writes.is_empty(),
        "nothing about X was written to the source: {writes:?}"
    );
    // the replica was told: tombstone written, copies gone
    assert!(bucket_has(&fx.bucket("rep"), &keys::tombstone(&x)));
    assert!(!bucket_has(&fx.bucket("rep"), &keys::item(&x)));
    assert!(fx
        .bucket("rep")
        .keys()
        .iter()
        .all(|k| !k.starts_with(&keys::versions_of(&x))));
    for v in fx.app.backup_list().unwrap() {
        assert_eq!(v.health.queued, 0, "{v:?}");
    }
    // and it can never come back
    assert_eq!(
        code(
            fx.app
                .test_session()
                .test_vault(|v| v.create_item_with_id(x.clone(), login("Again", "x")))
        ),
        "invalid_input"
    );
}

#[test]
fn a_tombstone_beats_an_item_that_is_still_listed() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx);
    let head = remote_item(&fx, &x); // B's v2 as it was in the bucket
    fx.switch_to("A");
    fx.app.item_trash(&x).unwrap();
    fx.app.item_purge(&x, PW.into(), "CONFIRM").unwrap();
    fx.run();
    // a half-finished purge elsewhere leaves the head behind next to the tombstone
    fx.bucket("bkt").plant(&keys::item(&x), &head);
    fx.switch_to("B");
    fx.run();
    assert_eq!(code(fx.app.item_get(&x)), "not_found");
    fx.switch_to("A");
    fx.run();
    assert_eq!(
        code(fx.app.item_get(&x)),
        "not_found",
        "the leftover head is ignored"
    );
    assert_eq!(count_rows(&fx, "items"), 0);
}

// ============================================================ damaged objects

#[test]
fn a_damaged_remote_object_is_skipped_counted_and_never_applied() {
    let fx = Fx::with_target(manual());
    let x = fx.app.item_create(login("Bank", "pw1")).unwrap().id;
    let y = fx.app.item_create(login("Mail", "pw2")).unwrap().id;
    fx.run();
    let before = fx.app.item_list().unwrap();
    let b = fx.bucket("bkt");

    // 1. not an envelope at all, under a new item's key
    b.plant(&keys::item(NEW_ID), b"this is not an envelope");
    // 2. a genuine envelope of another item, under the wrong key (header id != key id)
    b.plant(&keys::item(NEW_ID_2), &remote_item(&fx, &y));
    // 3. a real item whose bytes were flipped in the bucket
    let mut tampered = remote_item(&fx, &x);
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    b.plant(&keys::item(&x), &tampered);
    // 4. a proof of deletion that is not one
    b.plant(&keys::tombstone(&y), b"{}");

    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    let status = fx.app.status().unwrap();
    assert_eq!(status.remote_damaged, 4);
    assert!(status.unlocked, "no crash, nothing stopped");
    assert_eq!(fx.app.item_list().unwrap(), before, "nothing was applied");
    assert_eq!(password_of(&fx, &x), "pw1");
    assert_eq!(password_of(&fx, &y), "pw2");
    assert_eq!(sync_revision(&fx), 0);
    assert_eq!(count_rows(&fx, "items"), 2);
    assert_eq!(count_rows(&fx, "tombstones"), 0);
    // the same objects are counted again (not remembered as good), and still not applied
    assert_eq!(fx.run().len(), 1);
    assert_eq!(fx.app.status().unwrap().remote_damaged, 4);
    assert_eq!(fx.app.item_list().unwrap(), before);
}

// ============================================================ new items, history, idempotence

#[test]
fn a_new_remote_item_arrives_with_its_history_and_a_second_pass_downloads_nothing() {
    let fx = Fx::new(manual());
    fx.app.profile_create("A", PW.into()).unwrap();
    let mut t = inp("Primary", "bkt");
    t.master = true;
    fx.app.backup_add(t).unwrap();
    let first = fx.app.item_create(login("First", "f1")).unwrap().id;
    fx.run();
    fx.app
        .profile_restore_from_backup("B", inp("Restore source", "bkt"))
        .unwrap();
    // A carries on after B was made
    fx.switch_to("A");
    let second = fx.app.item_create(login("Second", "s1")).unwrap().id;
    edit_password(&fx.app, &second, "s2");
    edit_password(&fx.app, &second, "s3");
    fx.run();

    fx.switch_to("B");
    assert_eq!(fx.app.item_list().unwrap().len(), 1);
    // B has a replica too: what it takes over from the source has to reach it
    fx.app.backup_add(inp("Replica", "rep")).unwrap();
    fx.run();
    assert_eq!(fx.app.item_list().unwrap().len(), 2);
    assert_eq!(password_of(&fx, &second), "s3");
    assert_eq!(
        fx.app
            .history_list(&second)
            .unwrap()
            .iter()
            .map(|e| e.version)
            .collect::<Vec<_>>(),
        [3, 2, 1]
    );
    assert_eq!(
        fx.app
            .history_get(&second, 1, Some(PW.into()))
            .unwrap()
            .fields["password"],
        "s1"
    );
    assert!(sync_revision(&fx) >= 1);
    assert_eq!(item_blob(&fx, &second), remote_item(&fx, &second));
    assert!(bucket_has(&fx.bucket("rep"), &keys::item(&second)));
    assert!(bucket_has(&fx.bucket("rep"), &keys::item(&first)));
    assert_eq!(
        fx.bucket("rep")
            .keys()
            .iter()
            .filter(|k| k.starts_with(&keys::versions_of(&second)))
            .count(),
        3
    );

    // second pass: the ETags are the ones already known, so nothing is downloaded
    fx.bucket("bkt").clear_log();
    let revision = sync_revision(&fx);
    let views = fx.run();
    for v in &views {
        assert_eq!(v.health.state, HealthState::Healthy, "{v:?}");
    }
    assert_eq!(
        fx.bucket("bkt").count(Op::Get),
        0,
        "{:?}",
        fx.bucket("bkt").log()
    );
    assert!(
        fx.bucket("bkt").count(Op::List) >= 2,
        "it did look: tombstones and items"
    );
    assert_eq!(sync_revision(&fx), revision);
}

// ============================================================ the source and its health

#[test]
fn the_master_is_the_source_and_an_unreachable_one_does_not_stop_the_pushes() {
    let fx = Fx::with_target(manual());
    fx.app.backup_add(inp("Replica", "rep")).unwrap();
    fx.app.item_create(login("Bank", "pw1")).unwrap();
    fx.bucket("bkt")
        .fail_all(None, RemoteError::Unreachable(NetKind::Timeout));
    fx.bucket("rep").clear_log();
    let views = fx.run();
    let primary = views.iter().find(|v| v.name == "Primary").unwrap();
    let replica = views.iter().find(|v| v.name == "Replica").unwrap();
    assert_eq!(primary.health.state, HealthState::Lagging);
    assert!(primary
        .health
        .last_error
        .as_deref()
        .unwrap()
        .contains("did not answer"));
    assert!(primary.health.queued > 0);
    assert_eq!(replica.health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(
        replica.health.queued, 0,
        "the replica was pushed to regardless"
    );
    // the replica is only ever written to
    assert!(
        fx.bucket("rep").count(Op::Get) <= 1,
        "at most the push side's own look at the key bundle; never a read for content"
    );
    // one "Back up now" is one failed attempt, not two (its pull and its push)
    let again = fx.run();
    let primary = again.iter().find(|v| v.name == "Primary").unwrap();
    assert_eq!(
        fx.app.test_session().test_vault(|v| {
            v.store()
                .target_row(&primary.id)
                .unwrap()
                .unwrap()
                .consecutive_errors
        }),
        2
    );
    // when the source answers again, everything is sent and the target is healthy
    fx.bucket("bkt").clear_faults();
    for v in fx.run() {
        assert_eq!(v.health.state, HealthState::Healthy, "{v:?}");
        assert_eq!(v.health.queued, 0);
    }
}

#[test]
fn without_a_master_the_first_enabled_target_is_the_source() {
    let fx = Fx::new(manual());
    fx.app.profile_create("Main", PW.into()).unwrap();
    fx.app.backup_add(inp("One", "one")).unwrap();
    fx.app.backup_add(inp("Two", "two")).unwrap();
    fx.run();
    fx.bucket("one").clear_log();
    fx.bucket("two").clear_log();
    fx.run();
    assert!(fx.bucket("one").count(Op::List) >= 2, "listed for the pull");
    assert_eq!(
        fx.bucket("two").count(Op::List),
        1,
        "only the push side's connection check"
    );
}

// ============================================================ the unlock trigger

#[test]
fn the_pull_after_an_unlock_runs_on_the_worker_and_never_blocks_the_unlock() {
    let fx = Fx::new(background());
    let x = two_devices(&fx); // B: v2 in the bucket
    fx.app.lock();
    fx.app.profile_select(&fx.id_of("A")).unwrap();
    wait_for("the old worker to be gone", || {
        fx.app.status().is_ok_and(|s| !s.unlocked)
    });
    let b = fx.bucket("bkt");
    b.set_delay(Duration::from_millis(150));
    fx.app.unlock(PW.into()).unwrap();
    // The unlock has returned, and the pull (at least four requests of 150 ms) has not even
    // finished: it runs on the worker thread.
    assert!(
        fx.app.status().unwrap().reconciling,
        "the pull has only just been asked for"
    );
    assert_eq!(
        version_of(&fx, &x),
        1,
        "still the old state: the pull is under way"
    );
    wait_for("the pull to take over B's edit", || {
        version_of(&fx, &x) == 2 && !fx.app.status().unwrap().reconciling
    });
    assert_eq!(password_of(&fx, &x), "from-B");
    assert_eq!(sync_revision(&fx), 1);
}

// ============================================================ the race with the person's own edit

#[test]
fn a_change_decided_against_an_old_head_steps_back_when_the_item_was_edited_meanwhile() {
    let fx = Fx::new(manual());
    let x = two_devices(&fx);
    let theirs = remote_item(&fx, &x);
    fx.switch_to("A");
    let old_head = fx.app.test_session().test_vault(|v| {
        let row = v.store().item_row(&x).unwrap().unwrap();
        (row.version, row.ts)
    });
    // the person edits after the pull looked at the item and before it wrote
    edit_password(&fx.app, &x, "from-A");
    let source = fx.view("Primary").id;
    let apply = |plan: Plan| {
        fx.app
            .test_session()
            .test_vault(|v| v.pull_apply(&source, plan).unwrap().applied)
    };
    assert_eq!(
        apply(Plan::FastForward {
            id: x.clone(),
            expect: old_head,
            head: theirs.clone(),
            snapshots: vec![],
        }),
        Applied::Raced
    );
    assert_eq!(
        apply(Plan::OtherDevice {
            id: x.clone(),
            expect: old_head,
            head: theirs.clone(),
        }),
        Applied::Raced
    );
    assert_eq!(
        apply(Plan::New {
            id: x.clone(),
            head: theirs.clone(),
            snapshots: vec![],
        }),
        Applied::Raced,
        "the item exists here, so it is not new"
    );
    assert_eq!(
        password_of(&fx, &x),
        "from-A",
        "the edit was not overwritten"
    );
    assert_eq!(other_device_entries(&fx, &x), 0);
    assert_eq!(sync_revision(&fx), 0);
    // a state that does not belong to this vault or this item is refused, whatever the plan says
    let mut forged = theirs;
    let last = forged.len() - 1;
    forged[last] ^= 1;
    assert_eq!(
        apply(Plan::FastForward {
            id: x.clone(),
            expect: old_head,
            head: forged,
            snapshots: vec![],
        }),
        Applied::Damaged
    );
}
