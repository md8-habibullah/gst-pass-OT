//! The demo vault and the device settings, as `AppCore` methods (`ai-docs/FEATURE_QUEUE.md` Q5).
//!
//! A child module of `app.rs` (declared with `#[path]`), so it can reach the core's private state like
//! the gate glue does.
//!
//! * **Demo vault.** `demo_spawn` makes the vault "Hexis123" (password "Hexis123", filled by
//!   `storage/demo.rs`) when the app holds no vault at all, and `demo_open` unlocks it without anyone
//!   typing the password (it is public on purpose: the vault holds nothing real). Only the vault whose id
//!   `device.json` names can be opened that way.
//! * **Device settings.** Time zone, the "first-run welcome was seen" flag and the online time check.

use super::{relock, AppCore};
use crate::clock::{ClockStatus, TrustedClock};
use crate::crypto::{now_millis, rng};
use crate::storage::demo::{self, DEMO_NAME, DEMO_PASSWORD};
use crate::storage::device::{self, DeviceInfo};
use crate::storage::error::{Result, StorageError};
use crate::storage::profiles::ProfileInfo;

impl AppCore {
    // ------------------------------------------------------------------ device settings

    pub fn device_get(&self) -> DeviceInfo {
        DeviceInfo::from(&*relock(&self.device))
    }

    fn device_update(
        &self,
        change: impl FnOnce(&mut device::DeviceSettings),
    ) -> Result<DeviceInfo> {
        let mut d = relock(&self.device);
        let mut next = d.clone();
        change(&mut next);
        device::store(&self.dir, &next)?;
        *d = next;
        Ok(DeviceInfo::from(&*d))
    }

    /// Remember the zone the person chose. Only decides how times are shown; everything stored is UTC.
    pub fn device_set_time_zone(&self, zone: &str) -> Result<DeviceInfo> {
        let zone = device::validate_zone(zone)?;
        self.device_update(|d| d.time_zone = Some(zone))
    }

    /// The first-run welcome was seen (or its choices were made).
    pub fn device_finish_setup(&self) -> Result<DeviceInfo> {
        self.device_update(|d| d.setup_done = true)
    }

    /// Choose the colours: light, dark, or follow the device.
    pub fn device_set_theme(&self, theme: device::Theme) -> Result<DeviceInfo> {
        self.device_update(|d| d.theme = theme)
    }

    /// How long a copied secret stays on the clipboard (15, 30 or 60 seconds).
    pub fn device_set_clipboard_clear(&self, seconds: u32) -> Result<DeviceInfo> {
        let seconds = device::validate_clipboard_clear(seconds)?;
        self.device_update(|d| d.clipboard_clear = seconds)
    }

    /// Open the vault with the cursor in the search box (and the keyboard up on a phone).
    pub fn device_set_open_in_search(&self, on: bool) -> Result<DeviceInfo> {
        self.device_update(|d| d.open_in_search = on)
    }

    /// The order the item list starts in.
    pub fn device_set_list_sort(&self, order: &str) -> Result<DeviceInfo> {
        let order = device::validate_sort(order)?;
        self.device_update(|d| d.list_sort = order)
    }

    /// Switch the online time check on or off. Off, no request is ever made and waits rest on the boot clock alone.
    pub fn device_set_time_check(&self, on: bool) -> Result<DeviceInfo> {
        let info = self.device_update(|d| d.time_check = on)?;
        TrustedClock::global().set_enabled(on);
        if on {
            self.check_time_in_background();
        }
        Ok(info)
    }

    /// What the screen may know about the clock. Also starts a check in the background when one is due.
    pub fn clock_status(&self) -> ClockStatus {
        self.check_time_in_background();
        TrustedClock::global().status()
    }

    /// Check the time now and wait for the answer (about ten seconds at most). A failure is not an error:
    /// the status says what went wrong.
    pub fn clock_sync(&self) -> Result<ClockStatus> {
        let clock = TrustedClock::global();
        if !clock.is_enabled() {
            return Err(StorageError::invalid(
                "The online time check is switched off. Switch it on first.",
            ));
        }
        let _ = clock.sync();
        Ok(clock.status())
    }

    /// The first background check, at start-up.
    pub fn start_clock(&self) {
        self.check_time_in_background();
    }

    /// Ask the time servers on a background thread when a check is due. Test builds never do: a unit test
    /// must not depend on the network (the clock's own tests use a fake fetcher).
    fn check_time_in_background(&self) {
        #[cfg(not(test))]
        TrustedClock::global().refresh_in_background();
    }

    // ------------------------------------------------------------------ the demo vault

    /// Make the demo vault: only when the app holds no vault. It is left selected and unlocked.
    pub fn demo_spawn(&self) -> Result<ProfileInfo> {
        if !self.reg().entries().is_empty() {
            return Err(StorageError::invalid(
                "A vault already exists on this device.",
            ));
        }
        let mut info = self.profile_create(DEMO_NAME, DEMO_PASSWORD.to_string())?;
        let filled = self.session().and_then(|session| {
            let mut fill = |buf: &mut [u8]| {
                let _ = rng::fill(buf);
            };
            session.seed_items(demo::items(now_millis(), &mut fill))
        });
        if let Err(e) = filled {
            // nothing half-made stays behind
            let _ = self.profile_remove(&info.id, DEMO_PASSWORD.to_string(), "CONFIRM");
            return Err(e);
        }
        let id = info.id.clone();
        self.device_update(|d| d.demo_profile = Some(id))?;
        info.demo = true;
        Ok(info)
    }

    /// Select and unlock the demo vault without a typed password. Refused for every other vault.
    pub fn demo_open(&self) -> Result<()> {
        let id = relock(&self.device)
            .demo_profile
            .clone()
            .ok_or(StorageError::NotFound)?;
        if self.reg().get(&id).is_none() {
            return Err(StorageError::NotFound);
        }
        if self.active_session().map(|(active, _)| active).as_deref() != Some(id.as_str()) {
            self.profile_select(&id)?;
        }
        self.unlock(DEMO_PASSWORD.to_string())
    }

    /// `info` with its demo flag set when it describes the demo vault (the registry itself does not know).
    pub(super) fn marked(&self, mut info: ProfileInfo) -> ProfileInfo {
        info.demo = relock(&self.device).demo_profile.as_deref() == Some(info.id.as_str());
        info
    }

    /// The very last step of "remove Hexis from this device": `device.json` goes, the settings in memory fall back
    /// to the defaults (nothing is written), and the data folder is removed when nothing is left in it.
    pub(super) fn forget_device_files(&self) {
        for name in [device::DEVICE_FILE, "device.json.tmp"] {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
        *relock(&self.device) = device::DeviceSettings::default();
        crate::storage::webview_data::remove(&self.dir);
        // only succeeds when empty: anything unknown in there is left alone
        let _ = std::fs::remove_dir(&self.dir);
    }

    /// The demo vault was deleted: forget its id.
    pub(super) fn forget_demo_if(&self, id: &str) {
        if relock(&self.device).demo_profile.as_deref() == Some(id) {
            let _ = self.device_update(|d| d.demo_profile = None);
        }
    }

    /// Everything was erased: forget the demo vault's id (the settings next to it stay).
    pub(super) fn forget_demo_if_any(&self) {
        if relock(&self.device).demo_profile.is_some() {
            let _ = self.device_update(|d| d.demo_profile = None);
        }
    }
}
