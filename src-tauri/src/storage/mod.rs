//! Phase 2: local storage, immutable version history and the trash lifecycle.
//!
//! * [`db`] holds SQLite (opaque envelopes only, one transaction per state change)
//! * [`model`] defines the item payload schema 1, drafts, limits, and the wire types of `IPC_CONTRACT.md`
//! * [`vault`] is the unlocked vault (lifecycle, history, trash, purge, TOTP, in-memory index)
//! * [`session`] owns the locked/unlocked state, the master-password re-prompt guard, edit tokens
//!
//! Nothing here depends on Tauri; `crate::commands` is the thin IPC layer on top.

pub mod db;
pub mod error;
pub mod model;
pub mod session;
pub mod uuid;
pub mod vault;

#[cfg(test)]
mod tests;

pub use error::{Result, StorageError};
pub use model::{
    BeginEdit, ItemDraft, ItemSummary, ItemView, TotpNow, TrashEntry, VaultStatus, VersionEntry,
};
pub use session::{Session, SessionConfig};
