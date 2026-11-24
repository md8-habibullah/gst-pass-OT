//! Phase 4a: backup targets and single-writer replication (`ai-docs/STORAGE_AND_SYNC.md` §6/§7,
//! `ai-docs/IPC_CONTRACT.md` §6).
//!
//! * [`types`]      wire types, input validation (https-only endpoints, safe field alphabets)
//! * [`remote`]     the [`remote::ObjectStore`] seam, key layout, secret-free error wording,
//!   and (tests only) an in-memory bucket with fault injection
//! * [`s3`]         the real client: `rusty-s3` presigning + blocking `ureq` with rustls
//! * [`dbops`]      SQL of `targets`, `outbox`, `remote_objects` and the restore inserts
//! * [`probe`]      connection + conditional-write probe (works without an unlocked vault)
//! * [`replicator`] the queue drain, the single-writer rule, health, the (timer-free) worker thread
//! * [`pull`]       pull and reconcile on unlock and on "Back up now" (network side, lineage test)
//! * [`pull_apply`] the vault side of a pull: authenticate, decide-and-write under the session lock
//! * [`vault_ext`]  target settings sealed in the vault, the shared-targets snapshot, the engine
//!   handle owned by an unlocked [`Vault`](super::vault::Vault)
//! * [`restore`]    download a backup into a brand-new vault file (no unlocked vault needed)

pub mod dbops;
pub mod files_sync;
pub mod probe;
pub mod pull;
pub mod pull_apply;
pub mod remote;
pub mod replicator;
pub mod restore;
pub mod s3;
pub mod types;
pub mod vault_ext;

#[cfg(test)]
mod files_sync_tests;
#[cfg(test)]
mod mock_tests;
#[cfg(test)]
mod tests;

/// `meta` key: the sealed shared-targets snapshot this device holds (base64).
pub const SHARED_TARGETS_META: &str = "shared_targets";
/// `meta` key: a snapshot downloaded by a restore, imported at the next unlock (base64).
pub const PENDING_TARGETS_META: &str = "pending_targets";

pub use probe::{probe_config, ProbeReport};
pub use pull::VaultLink;
pub use replicator::{BackupRuntime, Timing};
pub use types::{
    BackupTargetInput, BackupTargetView, BackupTestResult, ConditionalWrites, HealthState,
    TargetConfig, TargetHealth,
};
