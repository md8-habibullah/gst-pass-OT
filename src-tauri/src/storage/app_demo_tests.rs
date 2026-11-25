//! The demo vault and the device settings through [`AppCore`]: it is made only for an empty app, opens
//! without a typed password only for itself, is forgotten when it is deleted or the app is erased, and
//! the zone and setup flag outlive an erase. Everything runs in temp folders.

use super::app::AppCore;
use super::demo::{DEMO_NAME, DEMO_PASSWORD};
use super::device;
use super::files::TestPicker;
use super::session::{SessionConfig, EDIT_TOKEN_TTL};
use super::StorageError;
use crate::crypto::KdfParams;
use crate::gate::wipe::NoObserver;
use std::sync::Arc;

const PW: &str = "correct horse battery staple";

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

    /// Quit and start the app again on the same folder.
    fn restart(&mut self) {
        self.app.lock();
        self.app = AppCore::open(self.dir.path(), fast(), self.picker.clone()).unwrap();
    }
}

#[test]
fn the_demo_is_a_named_vault_full_of_believable_items() {
    let fx = Fx::new();
    let info = fx.app.demo_spawn().unwrap();
    assert_eq!(info.display_name, DEMO_NAME);
    assert!(info.demo);

    let items = fx.app.item_list().unwrap();
    assert!((15..=25).contains(&items.len()), "{} items", items.len());
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    for name in ["Google", "Microsoft account", "GitHub", "Amazon"] {
        assert!(titles.contains(&name), "{name} missing from {titles:?}");
    }

    let list = fx.app.profile_list().unwrap();
    assert_eq!(list.profiles.len(), 1);
    assert!(list.profiles[0].demo, "the list marks the demo vault");
    let st = fx.app.status().unwrap();
    assert!(st.unlocked);
    assert!(
        st.profile.unwrap().demo,
        "so does the status of the open vault"
    );
}

#[test]
fn the_demo_keeps_its_mark_when_it_is_renamed() {
    let fx = Fx::new();
    let id = fx.app.demo_spawn().unwrap().id;
    let renamed = fx.app.profile_rename(&id, "Practice").unwrap();
    assert!(renamed.demo);
    assert!(fx.app.status().unwrap().profile.unwrap().demo);
}

#[test]
fn a_demo_is_made_only_for_an_app_with_no_vault() {
    let fx = Fx::new();
    fx.app.profile_create("Mine", PW.into()).unwrap();
    let err = fx.app.demo_spawn().unwrap_err();
    assert!(matches!(err, StorageError::InvalidInput(_)), "{err:?}");
    let list = fx.app.profile_list().unwrap();
    assert_eq!(list.profiles.len(), 1, "nothing was added");
    assert!(!list.profiles[0].demo);
}

#[test]
fn only_the_demo_opens_without_a_password() {
    let fx = Fx::new();
    let demo = fx.app.demo_spawn().unwrap();
    let mine = fx.app.profile_create("Mine", PW.into()).unwrap();
    fx.app.lock();

    fx.app.demo_open().unwrap();
    let st = fx.app.status().unwrap();
    assert!(st.unlocked);
    assert_eq!(st.profile.unwrap().id, demo.id);

    // another vault stays locked until its own password is typed
    fx.app.lock();
    fx.app.profile_select(&mine.id).unwrap();
    assert!(!fx.app.status().unwrap().unlocked);
    assert!(fx.app.item_list().is_err());
}

#[test]
fn the_demo_can_be_opened_again_after_a_restart() {
    let mut fx = Fx::new();
    let id = fx.app.demo_spawn().unwrap().id;
    fx.restart();
    assert!(!fx.app.status().unwrap().unlocked);
    fx.app.demo_open().unwrap();
    let st = fx.app.status().unwrap();
    assert!(st.unlocked && st.profile.unwrap().id == id);
}

#[test]
fn deleting_the_demo_forgets_it_and_an_empty_app_can_make_a_new_one() {
    let fx = Fx::new();
    let first = fx.app.demo_spawn().unwrap();
    fx.app
        .profile_remove(&first.id, DEMO_PASSWORD.to_string(), "CONFIRM")
        .unwrap();
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());
    assert!(matches!(fx.app.demo_open(), Err(StorageError::NotFound)));

    let second = fx.app.demo_spawn().unwrap();
    assert_ne!(first.id, second.id);
    assert!(second.demo);
}

#[test]
fn an_erase_forgets_the_demo_but_keeps_the_zone_and_the_setup_flag() {
    let mut fx = Fx::new();
    fx.app.device_set_time_zone("Asia/Dhaka").unwrap();
    fx.app.device_finish_setup().unwrap();
    fx.app.demo_spawn().unwrap();

    fx.app.wipe_with(&mut NoObserver).unwrap();
    assert!(fx.app.profile_list().unwrap().profiles.is_empty());
    assert!(matches!(fx.app.demo_open(), Err(StorageError::NotFound)));
    let d = fx.app.device_get();
    assert_eq!(d.time_zone.as_deref(), Some("Asia/Dhaka"));
    assert!(d.setup_done);

    // the same holds after a restart, and the app can start over with a fresh demo
    fx.restart();
    let d = fx.app.device_get();
    assert_eq!(d.time_zone.as_deref(), Some("Asia/Dhaka"));
    assert!(d.setup_done);
    assert!(fx.app.demo_spawn().is_ok());
}

#[test]
fn the_zone_is_checked_and_survives_a_restart() {
    let mut fx = Fx::new();
    let too_long = "x".repeat(65);
    for bad in [
        "",
        "../etc",
        "Not A Zone!",
        "Asia//Dhaka",
        too_long.as_str(),
    ] {
        assert!(
            fx.app.device_set_time_zone(bad).is_err(),
            "{bad:?} was accepted"
        );
    }
    assert_eq!(
        fx.app.device_get().time_zone,
        None,
        "nothing is saved by a refusal"
    );

    let d = fx.app.device_set_time_zone("Europe/London").unwrap();
    assert_eq!(d.time_zone.as_deref(), Some("Europe/London"));
    assert!(!d.setup_done);
    fx.app.device_finish_setup().unwrap();

    fx.restart();
    let d = fx.app.device_get();
    assert_eq!(d.time_zone.as_deref(), Some("Europe/London"));
    assert!(d.setup_done);
}

#[test]
fn opening_in_search_and_the_list_order_survive_a_restart_and_an_erase() {
    let mut fx = Fx::new();
    let d = fx.app.device_get();
    assert!(!d.open_in_search);
    assert_eq!(d.list_sort, "updated-desc");

    fx.app.device_set_open_in_search(true).unwrap();
    fx.app.device_set_list_sort("versions-desc").unwrap();
    assert!(fx.app.device_set_list_sort("sideways").is_err());
    assert_eq!(fx.app.device_get().list_sort, "versions-desc");

    fx.restart();
    let d = fx.app.device_get();
    assert!(d.open_in_search);
    assert_eq!(d.list_sort, "versions-desc");

    fx.app.demo_spawn().unwrap();
    fx.app.wipe_with(&mut NoObserver).unwrap();
    let d = fx.app.device_get();
    assert!(d.open_in_search);
    assert_eq!(d.list_sort, "versions-desc");
}

#[test]
fn a_hand_edited_list_order_is_the_default_and_keeps_the_other_settings() {
    let fx = Fx::new();
    fx.app.device_set_time_zone("Asia/Dhaka").unwrap();
    let path = fx.dir.path().join(device::DEVICE_FILE);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["listSort"] = serde_json::json!("not-an-order");
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let loaded = device::load(fx.dir.path());
    assert_eq!(loaded.list_sort, device::DEFAULT_SORT);
    assert_eq!(loaded.time_zone.as_deref(), Some("Asia/Dhaka"));
}

#[test]
fn the_theme_and_the_clipboard_time_survive_a_restart_and_an_erase() {
    let mut fx = Fx::new();
    let d = fx.app.device_get();
    assert_eq!(d.theme, device::Theme::System);
    assert_eq!(d.clipboard_clear, 30);

    fx.app.device_set_theme(device::Theme::Dark).unwrap();
    fx.app.device_set_clipboard_clear(60).unwrap();
    for bad in [0, 10, 31, 3600] {
        assert!(
            fx.app.device_set_clipboard_clear(bad).is_err(),
            "{bad} was accepted"
        );
    }
    assert_eq!(fx.app.device_get().clipboard_clear, 60);

    fx.restart();
    let d = fx.app.device_get();
    assert_eq!((d.theme, d.clipboard_clear), (device::Theme::Dark, 60));

    // "erase this device" keeps how the app looks and behaves, like the zone
    fx.app.demo_spawn().unwrap();
    fx.app.wipe_with(&mut NoObserver).unwrap();
    let d = fx.app.device_get();
    assert_eq!((d.theme, d.clipboard_clear), (device::Theme::Dark, 60));
}
