//! Phase 2: local storage, immutable version history and the trash lifecycle.
//!
//! * [`db`] holds SQLite (opaque envelopes only, one transaction per state change)
//! * [`model`] defines the item payload schema 1, drafts, limits, and the wire types of `IPC_CONTRACT.md`
//! * [`vault`] is the unlocked vault (lifecycle, history, trash, purge, TOTP, in-memory index)
//! * [`session`] owns the locked/unlocked state, the master-password re-prompt guard, edit tokens
//! * [`profiles`] is the account registry (several vaults per app); [`files`] holds the atomic
//!   file helpers and the native file-picker seam
//! * [`app`] ([`AppCore`]) ties the registry, the one selected profile and its session together
//! * [`backup`] (Phase 4a) is the S3 client, the replication engine and restore; [`links`] validates
//!   the one address the webview may ask the system browser to open
//!
//! Nothing here depends on Tauri; `crate::commands` is the thin IPC layer on top.

pub mod app;
mod app_disguise;
pub mod backup;
pub mod db;
pub mod demo;
pub mod device;
pub mod drive;
pub mod error;
pub mod files;
pub mod legacy;
pub mod links;
pub mod model;
pub mod profiles;
pub mod session;
pub mod uuid;
pub mod vault;
mod vault_drive;

#[cfg(test)]
mod app_demo_tests;
#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod drive_tests;
#[cfg(test)]
mod tests;

pub use app::{AppCore, ExportedFile, SyncFlush, EXIT_FLUSH};
pub use backup::prefs::{GeneratorPrefs, PrefsView};
pub use backup::{BackupRuntime, BackupTargetInput, BackupTargetView, BackupTestResult, Timing};
pub use drive::FileEntry;
pub use error::{Result, StorageError};
pub use files::{FilePicker, NoPicker};
pub use links::{NoOpener, RecordingOpener, UrlOpener};
pub use model::{
    BeginEdit, ItemDraft, ItemSummary, ItemView, TotpNow, TrashEntry, VaultStatus, VersionEntry,
};
pub use profiles::{ProfileInfo, ProfileList};
pub use session::{Session, SessionConfig};
