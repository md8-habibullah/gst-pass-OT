//! The vault's preferences (the password generator's last settings): sealed in the vault
//! database, replicated to every target, restored with the vault, pulled by other devices, and
//! the newest write wins (`prefs.rs`).

use super::*;
use crate::crypto::{envelope, now_millis, KeyPurpose};
use crate::storage::backup::dbops;
use crate::storage::backup::prefs::{GeneratorPrefs, Prefs, PREFS_ID};
use crate::storage::backup::PREFS_META;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

fn gen(length: u32) -> GeneratorPrefs {
    GeneratorPrefs {
        length,
        lower: true,
        upper: true,
        digits: true,
        symbols: false,
        avoid_ambiguous: true,
    }
}

fn saved(fx: &Fx) -> Option<GeneratorPrefs> {
    fx.app.prefs_get().unwrap().generator
}

/// The sealed blob this device holds (base64), as stored in the vault database.
fn local_b64(fx: &Fx) -> Option<String> {
    fx.app
        .test_session()
        .test_vault(|v| v.store().meta(PREFS_META).unwrap())
}

fn local_bytes(fx: &Fx) -> Vec<u8> {
    B64.decode(local_b64(fx).expect("a stored copy")).unwrap()
}

fn version_of_blob(blob: &[u8]) -> u64 {
    envelope::peek_header(blob).unwrap().version
}

fn revision(fx: &Fx) -> u64 {
    fx.app.status().unwrap().sync_revision
}

/// Two milliseconds apart at least, so "later" is later on any clock resolution.
fn tick() {
    std::thread::sleep(Duration::from_millis(3));
}

/// Device A (account "A", master target "Primary" on bucket `bkt`) saved `first` and uploaded it;
/// device B was restored from the bucket and is the open profile.
fn two_devices_with_prefs(fx: &Fx, first: u32) {
    fx.app.profile_create("A", PW.into()).unwrap();
    let mut t = inp("Primary", "bkt");
    t.master = true;
    fx.app.backup_add(t).unwrap();
    fx.app.prefs_set_generator(gen(first)).unwrap();
    fx.run();
    fx.app
        .profile_restore_from_backup("B", inp("Restore source", "bkt"))
        .unwrap();
    fx.app.unlock(PW.into()).unwrap();
}

#[test]
fn nothing_is_saved_and_nothing_is_sent_until_the_person_changes_something() {
    let fx = Fx::with_target(manual());
    assert_eq!(saved(&fx), None);
    assert_eq!(local_b64(&fx), None);
    fx.run();
    assert!(!bucket_has(&fx.bucket("bkt"), keys::PREFS));
}

#[test]
fn the_settings_are_kept_sealed_and_survive_a_lock() {
    let fx = Fx::with_target(manual());
    let view = fx.app.prefs_set_generator(gen(30)).unwrap();
    assert_eq!(view.generator, Some(gen(30)));
    assert_eq!(saved(&fx), Some(gen(30)));

    let blob = local_bytes(&fx);
    assert_eq!(envelope::peek_header(&blob).unwrap().item_id, PREFS_ID);
    for word in ["length", "generator", "symbols", "avoidAmbiguous"] {
        assert!(!contains(&blob, word), "the stored copy shows {word}");
    }

    fx.app.lock();
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(
        saved(&fx),
        Some(gen(30)),
        "still there after a lock and an unlock"
    );
}

#[test]
fn a_setting_the_generator_cannot_use_is_refused_and_changes_nothing() {
    let fx = Fx::with_target(manual());
    fx.app.prefs_set_generator(gen(20)).unwrap();
    let before = local_b64(&fx);
    for bad in [
        gen(3),
        gen(129),
        GeneratorPrefs {
            lower: false,
            upper: false,
            digits: false,
            symbols: false,
            ..gen(20)
        },
    ] {
        assert_eq!(code(fx.app.prefs_set_generator(bad)), "invalid_input");
    }
    assert_eq!(local_b64(&fx), before);
    assert_eq!(saved(&fx), Some(gen(20)));
    // the limits themselves are fine
    fx.app.prefs_set_generator(gen(4)).unwrap();
    fx.app.prefs_set_generator(gen(128)).unwrap();
}

#[test]
fn saving_the_same_settings_again_writes_and_sends_nothing() {
    let fx = Fx::with_target(manual());
    fx.app.prefs_set_generator(gen(30)).unwrap();
    fx.run();
    let first = local_b64(&fx);
    fx.bucket("bkt").clear_log();
    fx.app.prefs_set_generator(gen(30)).unwrap();
    assert_eq!(local_b64(&fx), first, "no new version");
    fx.run();
    assert_eq!(
        fx.bucket("bkt").count(Op::Put),
        0,
        "{:?}",
        fx.bucket("bkt").log()
    );
}

#[test]
fn every_change_is_a_higher_version_so_the_newest_always_wins() {
    let fx = Fx::with_target(manual());
    let mut last = 0;
    for n in [20, 21, 22, 23] {
        fx.app.prefs_set_generator(gen(n)).unwrap();
        let v = version_of_blob(&local_bytes(&fx));
        assert!(v > last, "{v} should be above {last}");
        last = v;
    }
}

#[test]
fn the_settings_are_sent_to_the_backup_sealed_and_a_settled_vault_sends_nothing_more() {
    let fx = Fx::with_target(manual());
    fx.app.prefs_set_generator(gen(30)).unwrap();
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    let remote = fx.bucket("bkt").raw(keys::PREFS).expect("it was sent");
    assert_eq!(remote, local_bytes(&fx));
    assert!(!contains(&remote, "length") && !contains(&remote, "generator"));

    fx.bucket("bkt").clear_log();
    fx.run();
    assert_eq!(
        fx.bucket("bkt").count(Op::Put),
        0,
        "{:?}",
        fx.bucket("bkt").log()
    );
    assert_eq!(
        fx.bucket("bkt").count(Op::Get),
        0,
        "the ETag is the one already known: only a HEAD is made"
    );
}

#[test]
fn a_restored_device_starts_with_the_saved_settings() {
    let fx = Fx::new(manual());
    two_devices_with_prefs(&fx, 30);
    assert_eq!(saved(&fx), Some(gen(30)));
}

#[test]
fn a_change_on_one_device_reaches_the_other_on_its_next_pull() {
    let fx = Fx::new(manual());
    two_devices_with_prefs(&fx, 30); // B is open, shows 30
    fx.app.prefs_set_generator(gen(12)).unwrap();
    fx.run(); // B sends 12

    fx.switch_to("A"); // A still holds 30
    assert_eq!(saved(&fx), Some(gen(30)));
    assert_eq!(revision(&fx), 0);
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(saved(&fx), Some(gen(12)));
    assert_eq!(revision(&fx), 1, "the page is told something changed");
    assert_eq!(local_bytes(&fx), fx.bucket("bkt").raw(keys::PREFS).unwrap());

    // a second pass finds nothing new
    let before = revision(&fx);
    fx.run();
    assert_eq!(revision(&fx), before);
}

#[test]
fn when_both_devices_changed_it_the_later_change_wins_on_both() {
    let fx = Fx::new(manual());
    two_devices_with_prefs(&fx, 30);
    // A changes it first and does not send it yet ...
    fx.switch_to("A");
    fx.app.prefs_set_generator(gen(40)).unwrap();
    // ... then B changes it later and sends it
    fx.switch_to("B");
    tick();
    fx.app.prefs_set_generator(gen(12)).unwrap();
    fx.run();

    fx.switch_to("A");
    fx.run();
    assert_eq!(saved(&fx), Some(gen(12)), "A takes over the later change");
    assert_eq!(local_bytes(&fx), fx.bucket("bkt").raw(keys::PREFS).unwrap());
    fx.switch_to("B");
    fx.run();
    assert_eq!(saved(&fx), Some(gen(12)));
}

#[test]
fn a_later_local_change_replaces_an_earlier_one_in_the_bucket() {
    let fx = Fx::new(manual());
    two_devices_with_prefs(&fx, 30);
    fx.app.prefs_set_generator(gen(12)).unwrap(); // B, earlier
    fx.run();
    fx.switch_to("A");
    tick();
    fx.app.prefs_set_generator(gen(40)).unwrap(); // A, later, without pulling first
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(saved(&fx), Some(gen(40)), "A keeps its later change");
    assert_eq!(local_bytes(&fx), fx.bucket("bkt").raw(keys::PREFS).unwrap());
    fx.switch_to("B");
    fx.run();
    assert_eq!(saved(&fx), Some(gen(40)));
}

#[test]
fn a_blocked_write_for_a_preference_never_shows_as_an_error() {
    // The bucket holds a newer copy than this device has seen and the pull has not run: a push
    // alone must neither fail nor raise the "newer on the service" warning about items.
    let fx = Fx::new(manual());
    two_devices_with_prefs(&fx, 30);
    fx.app.prefs_set_generator(gen(12)).unwrap();
    fx.run();
    fx.switch_to("A");
    tick();
    // plant a newer copy than A's, as another device would have written it
    let newer = fx.app.test_session().test_vault(|v| {
        let json = serde_json::to_vec(&Prefs {
            generator: Some(gen(55)),
            ..Prefs::default()
        })
        .unwrap();
        let version = now_millis() + 60_000;
        v.root
            .seal_object(KeyPurpose::Meta, PREFS_ID, version, now_millis(), &json)
            .unwrap()
    });
    fx.bucket("bkt").plant(keys::PREFS, &newer);
    fx.app.prefs_set_generator(gen(40)).unwrap();
    let views = fx.drain(); // push only
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(
        views[0].health.queued, 0,
        "the job was dropped, not left blocked"
    );
    assert_eq!(
        fx.bucket("bkt").raw(keys::PREFS).unwrap(),
        newer,
        "the newer copy was not overwritten"
    );
    // the next pull takes the newer one
    fx.run();
    assert_eq!(saved(&fx), Some(gen(55)));
}

#[test]
fn a_copy_in_the_bucket_that_is_not_authentic_is_never_applied_counted_and_replaced() {
    let fx = Fx::with_target(manual());
    fx.app.prefs_set_generator(gen(30)).unwrap();
    fx.run();
    let good = fx.bucket("bkt").raw(keys::PREFS).unwrap();
    let mut flipped = good.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0x01;
    fx.bucket("bkt").plant(keys::PREFS, &flipped);

    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(saved(&fx), Some(gen(30)), "the bad copy was not applied");
    assert_eq!(
        fx.bucket("bkt").raw(keys::PREFS).unwrap(),
        good,
        "this device's own copy replaced it"
    );
    // once replaced, nothing is reported any more
    fx.run();
    assert_eq!(fx.app.status().unwrap().remote_damaged, 0);
}

#[test]
fn something_that_is_not_an_envelope_is_ignored_too() {
    let fx = Fx::with_target(manual());
    fx.bucket("bkt")
        .plant(keys::PREFS, b"not an envelope at all");
    let views = fx.run();
    assert_eq!(views[0].health.state, HealthState::Healthy, "{views:?}");
    assert_eq!(fx.app.status().unwrap().remote_damaged, 1);
    assert_eq!(saved(&fx), None);
    assert_eq!(revision(&fx), 0);
}

#[test]
fn settings_written_by_a_newer_app_are_left_alone_and_their_extra_fields_survive() {
    let fx = Fx::with_target(manual());
    // a genuine copy with a field this build does not know, in a higher version
    let (future, version) = fx.app.test_session().test_vault(|v| {
        let json = br#"{"v":2,"generator":{"length":33,"lower":true,"upper":true,"digits":true,"symbols":true,"avoidAmbiguous":false},"future":{"theme":"dusk"}}"#;
        let version = now_millis() + 1_000;
        let blob = v
            .root
            .seal_object(KeyPurpose::Meta, PREFS_ID, version, now_millis(), json)
            .unwrap();
        (blob, version)
    });
    fx.bucket("bkt").plant(keys::PREFS, &future);
    fx.run();
    assert_eq!(
        saved(&fx),
        Some(GeneratorPrefs {
            length: 33,
            lower: true,
            upper: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: false
        })
    );
    assert_eq!(version_of_blob(&local_bytes(&fx)), version);

    // changing the setting keeps the field it does not understand
    fx.app.prefs_set_generator(gen(25)).unwrap();
    let kept = fx.app.test_session().test_vault(|v| v.prefs());
    assert_eq!(
        kept.other.get("future"),
        Some(&serde_json::json!({"theme": "dusk"}))
    );
    assert_eq!(kept.generator, Some(gen(25)));
}

#[test]
fn a_genuine_copy_in_a_shape_this_build_cannot_read_is_not_overwritten() {
    let fx = Fx::with_target(manual());
    let odd = fx.app.test_session().test_vault(|v| {
        // a generator written as text: authentic, but not what this build understands
        v.root
            .seal_object(
                KeyPurpose::Meta,
                PREFS_ID,
                now_millis() + 60_000,
                now_millis(),
                br#"{"v":9,"generator":"a new shape"}"#,
            )
            .unwrap()
    });
    fx.bucket("bkt").plant(keys::PREFS, &odd);
    fx.run();
    assert_eq!(
        fx.app.status().unwrap().remote_damaged,
        0,
        "not damaged, just newer than this build"
    );
    assert_eq!(saved(&fx), None);
    // saving something does not erase what a newer app wrote
    fx.app.prefs_set_generator(gen(30)).unwrap();
    fx.run();
    assert_eq!(fx.bucket("bkt").raw(keys::PREFS).unwrap(), odd);
}

#[test]
fn a_target_added_later_gets_the_settings_with_everything_else() {
    let fx = Fx::with_target(manual());
    fx.app.prefs_set_generator(gen(30)).unwrap();
    fx.run();
    fx.app.backup_add(inp("Second", "rep")).unwrap();
    fx.run();
    assert_eq!(fx.bucket("rep").raw(keys::PREFS).unwrap(), local_bytes(&fx));
}

#[test]
fn the_settings_go_to_every_enabled_target() {
    let fx = Fx::with_target(manual());
    fx.app.backup_add(inp("Second", "rep")).unwrap();
    fx.app.prefs_set_generator(gen(30)).unwrap();
    fx.run();
    assert_eq!(fx.bucket("bkt").raw(keys::PREFS).unwrap(), local_bytes(&fx));
    assert_eq!(fx.bucket("rep").raw(keys::PREFS).unwrap(), local_bytes(&fx));
}

#[test]
fn a_stored_copy_that_does_not_open_means_defaults_never_an_error() {
    let fx = Fx::with_target(manual());
    fx.app.prefs_set_generator(gen(30)).unwrap();
    fx.app.test_session().test_vault(|v| {
        dbops::set_meta(v.store().conn(), PREFS_META, &B64.encode(b"garbage")).unwrap();
    });
    assert_eq!(saved(&fx), None);
    // and the next change simply starts over
    fx.app.prefs_set_generator(gen(18)).unwrap();
    assert_eq!(saved(&fx), Some(gen(18)));
}

#[test]
fn on_services_that_ignore_or_refuse_preconditions_a_newer_copy_is_still_never_overwritten() {
    for mode in [Mode::Ignore, Mode::Reject] {
        let fx = Fx::new(manual());
        fx.bucket("bkt").set_mode(mode);
        two_devices_with_prefs(&fx, 30);
        fx.run(); // the restored target is probed at its first drain
        let expected = if mode == Mode::Ignore {
            ConditionalWrites::Weak // it accepted a write whose precondition was false
        } else {
            ConditionalWrites::Unknown // it refused the headers: the probe cannot tell
        };
        assert_eq!(
            fx.view("Primary").health.conditional_writes,
            expected,
            "{mode:?}"
        );
        assert_eq!(
            saved(&fx),
            Some(gen(30)),
            "{mode:?}: restored with the settings"
        );
        assert_eq!(
            fx.bucket("bkt").raw(keys::PREFS).unwrap(),
            local_bytes(&fx),
            "{mode:?}"
        );

        // a copy written by another device, newer than anything here, appears behind this one's back
        tick();
        let newer = fx.app.test_session().test_vault(|v| {
            let json = serde_json::to_vec(&Prefs {
                generator: Some(gen(55)),
                ..Prefs::default()
            })
            .unwrap();
            v.root
                .seal_object(
                    KeyPurpose::Meta,
                    PREFS_ID,
                    now_millis() + 60_000,
                    now_millis(),
                    &json,
                )
                .unwrap()
        });
        fx.bucket("bkt").plant(keys::PREFS, &newer);
        fx.app.prefs_set_generator(gen(40)).unwrap();
        let views = fx.drain();
        assert_eq!(
            views[0].health.state,
            HealthState::Healthy,
            "{mode:?}: {views:?}"
        );
        assert_eq!(
            fx.bucket("bkt").raw(keys::PREFS).unwrap(),
            newer,
            "{mode:?}"
        );
        fx.run();
        assert_eq!(
            saved(&fx),
            Some(gen(55)),
            "{mode:?}: the next pull takes the newer one"
        );

        // and a later change of this device does replace an older copy
        tick();
        fx.app.prefs_set_generator(gen(21)).unwrap();
        fx.run();
        assert_eq!(
            fx.bucket("bkt").raw(keys::PREFS).unwrap(),
            local_bytes(&fx),
            "{mode:?}"
        );
    }
}

#[test]
fn the_settings_need_an_open_vault() {
    let fx = Fx::with_target(manual());
    fx.app.prefs_set_generator(gen(30)).unwrap();
    fx.app.lock();
    assert_eq!(code(fx.app.prefs_get()), "locked");
    assert_eq!(code(fx.app.prefs_set_generator(gen(31))), "locked");
    fx.app.unlock(PW.into()).unwrap();
    assert_eq!(saved(&fx), Some(gen(30)), "a refused save changed nothing");
}
