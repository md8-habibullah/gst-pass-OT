//! The connection and conditional-write probe (`backup_test`, and on every add/update).
//!
//! The probe writes a tiny throw-away object under `vault/probe/`, reads it back, asks the
//! service to refuse two writes whose preconditions are false, lists the prefix and deletes the
//! object. It needs no unlocked vault: the caller passes the connection details it was given.
//! The object holds random bytes only; nothing about the vault or the person.
//!
//! What it reports as `conditionalWrites`:
//!
//! * `strong`  - `If-None-Match: *` on an existing key *and* `If-Match` with a wrong ETag both
//!   answered 412. Replication then uses real compare-and-swap writes.
//! * `weak`    - the service accepted a write whose precondition was false: it ignores the
//!   headers. Replication then reads before it writes (best effort, not atomic).
//! * `unknown` - the service refused the headers or the probe could not finish the test.

use super::remote::{Condition, ObjectStore, RemoteError};
use super::replicator::BackupRuntime;
use super::types::{ConditionalWrites, TargetConfig};
use crate::crypto::rng;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub ok: bool,
    pub message: String,
    pub conditional: ConditionalWrites,
    /// A failure that retrying cannot fix (wrong keys, missing bucket, ...).
    pub hard: bool,
}

impl ProbeReport {
    fn fail(e: &RemoteError, conditional: ConditionalWrites) -> Self {
        Self {
            ok: false,
            message: e.message(),
            conditional,
            hard: e.is_hard(),
        }
    }

    fn fail_text(message: &str) -> Self {
        Self {
            ok: false,
            message: message.to_string(),
            conditional: ConditionalWrites::Unknown,
            hard: false,
        }
    }
}

/// Open the target's bucket through the runtime's factory and probe it. A configuration that
/// cannot even make a request (for example an insecure endpoint) is a failed probe, not an error.
pub fn probe_config(runtime: &BackupRuntime, cfg: &TargetConfig) -> ProbeReport {
    match runtime.factory.open(cfg) {
        Ok(store) => run(store.as_ref()),
        Err(e) => ProbeReport::fail(&e, ConditionalWrites::Unknown),
    }
}

const MSG_STRONG: &str = "Connected. The storage service enforces safe (conditional) writes.";
const MSG_WEAK: &str = "Connected, but this storage service ignores conditional writes. This app checks before it overwrites, so use one device at a time.";
const MSG_UNKNOWN: &str = "Connected. Whether this storage service enforces conditional writes could not be determined, so this app checks before it overwrites. Use one device at a time.";
const NOTE_DELETE: &str = " The test object could not be deleted, so erasing an item from this backup may not work. Check the key's delete permission.";

fn probe_key() -> Result<String, RemoteError> {
    let b = rng::array::<12>().map_err(|_| RemoteError::BadConfig)?;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!("vault/probe/{hex}.tmp"))
}

/// Run the probe against `store`. Never panics, never returns an error value: the outcome is the
/// report (`ok == false` carries a fixed, secret-free message).
pub fn run(store: &dyn ObjectStore) -> ProbeReport {
    let key = match probe_key() {
        Ok(k) => k,
        Err(e) => return ProbeReport::fail(&e, ConditionalWrites::Unknown),
    };
    let body = key.as_bytes();

    // 1. A write that must succeed. A service that refuses the header is still usable.
    let mut conditions_testable = true;
    match store.put(&key, body, &Condition::IfNoneMatch) {
        Ok(_) => {}
        Err(RemoteError::Unsupported | RemoteError::Rejected(_)) => {
            conditions_testable = false;
            if let Err(e) = store.put(&key, body, &Condition::None) {
                return ProbeReport::fail(&e, ConditionalWrites::Unknown);
            }
        }
        Err(e) => return ProbeReport::fail(&e, ConditionalWrites::Unknown),
    }
    let report = after_write(store, &key, body, conditions_testable);
    // The test object never stays behind. A failed delete is reported, not fatal.
    let deleted = store.delete(&key).is_ok();
    match report {
        ProbeReport {
            ok: true,
            message,
            conditional,
            hard,
        } if !deleted => ProbeReport {
            ok: true,
            message: format!("{message}{NOTE_DELETE}"),
            conditional,
            hard,
        },
        other => other,
    }
}

fn after_write(
    store: &dyn ObjectStore,
    key: &str,
    body: &[u8],
    conditions_testable: bool,
) -> ProbeReport {
    // 2. Read it back.
    match store.get(key, 4096) {
        Ok(Some(obj)) if obj.bytes == body => {}
        Ok(_) => {
            return ProbeReport::fail_text(
                "The storage service did not return what was just written. It cannot be used as a backup.",
            )
        }
        Err(e) => return ProbeReport::fail(&e, ConditionalWrites::Unknown),
    }

    // 3. Conditional writes: both preconditions are false for the object that now exists.
    let conditional = if conditions_testable {
        let none_match = classify(store.put(key, body, &Condition::IfNoneMatch));
        let wrong_etag = classify(store.put(
            key,
            body,
            &Condition::IfMatch("\"hexis-probe-no-such-etag\"".into()),
        ));
        match (none_match, wrong_etag) {
            (Verdict::Enforced, Verdict::Enforced) => ConditionalWrites::Strong,
            (Verdict::Ignored, _) | (_, Verdict::Ignored) => ConditionalWrites::Weak,
            _ => ConditionalWrites::Unknown,
        }
    } else {
        ConditionalWrites::Unknown
    };

    // 4. Listing is what a restore needs.
    if let Err(e) = store.list("vault/", None, 10) {
        return ProbeReport::fail(&e, conditional);
    }

    ProbeReport {
        ok: true,
        message: match conditional {
            ConditionalWrites::Strong => MSG_STRONG,
            ConditionalWrites::Weak => MSG_WEAK,
            ConditionalWrites::Unknown => MSG_UNKNOWN,
        }
        .to_string(),
        conditional,
        hard: false,
    }
}

enum Verdict {
    Enforced,
    Ignored,
    Unclear,
}

fn classify(r: Result<super::remote::PutOk, RemoteError>) -> Verdict {
    match r {
        Err(RemoteError::PreconditionFailed) => Verdict::Enforced,
        // The service accepted a write whose precondition was false.
        Ok(_) => Verdict::Ignored,
        Err(_) => Verdict::Unclear,
    }
}

#[cfg(test)]
mod tests {
    use super::super::remote::mem::{MemStore, Mode, Op};
    use super::*;

    #[test]
    fn strong_service_is_reported_strong_and_leaves_nothing_behind() {
        let s = MemStore::new();
        let r = run(s.as_ref());
        assert!(r.ok, "{r:?}");
        assert_eq!(r.conditional, ConditionalWrites::Strong);
        assert_eq!(r.message, MSG_STRONG);
        assert!(s.keys().is_empty(), "the probe object was deleted");
        // write, read, two conditional writes, list, delete - all under vault/
        assert!(s.log().iter().all(|(_, k, _)| k.starts_with("vault/")));
        assert_eq!(s.count(Op::Delete), 1);
        assert_eq!(s.count(Op::List), 1);
    }

    #[test]
    fn service_that_ignores_headers_is_weak() {
        let s = MemStore::new();
        s.set_mode(Mode::Ignore);
        let r = run(s.as_ref());
        assert!(r.ok);
        assert_eq!(r.conditional, ConditionalWrites::Weak);
        assert!(s.keys().is_empty());
    }

    #[test]
    fn service_that_refuses_headers_is_unknown_but_usable() {
        let s = MemStore::new();
        s.set_mode(Mode::Reject);
        let r = run(s.as_ref());
        assert!(r.ok, "{r:?}");
        assert_eq!(r.conditional, ConditionalWrites::Unknown);
        assert!(s.keys().is_empty());
    }

    #[test]
    fn failures_carry_a_fixed_message_and_the_hardness() {
        let s = MemStore::new();
        s.fail_all(None, RemoteError::AccessDenied);
        let r = run(s.as_ref());
        assert!(!r.ok);
        assert!(r.hard);
        assert_eq!(r.message, RemoteError::AccessDenied.message());

        let s = MemStore::new();
        s.fail_all(None, RemoteError::Server(503));
        let r = run(s.as_ref());
        assert!(!r.ok);
        assert!(!r.hard);
    }

    #[test]
    fn a_listing_failure_fails_the_probe_and_still_cleans_up() {
        let s = MemStore::new();
        s.fail_all(Some(Op::List), RemoteError::AccessDenied);
        let r = run(s.as_ref());
        assert!(!r.ok);
        assert!(s.keys().is_empty(), "the object was deleted anyway");
    }

    #[test]
    fn a_failed_delete_is_a_note_not_a_failure() {
        let s = MemStore::new();
        s.fail_all(Some(Op::Delete), RemoteError::AccessDenied);
        let r = run(s.as_ref());
        assert!(r.ok, "{r:?}");
        assert!(r.message.contains("could not be deleted"));
    }

    #[test]
    fn probe_keys_are_random() {
        let a = probe_key().unwrap();
        let b = probe_key().unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with("vault/probe/") && a.ends_with(".tmp"));
    }
}
