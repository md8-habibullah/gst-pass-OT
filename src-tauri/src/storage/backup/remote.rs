//! The object-store seam of the backup engine.
//!
//! [`ObjectStore`] is everything replication, the capability probe and restore need from a
//! bucket. The real implementation is `s3::S3Store`; unit tests use the in-memory `MemStore`
//! (with fault injection), so the queue, backoff, health and conflict logic is tested without a
//! network.
//!
//! Keys handed to a store are *logical* (`vault/items/<id>.enc`); a store adds the target's
//! folder prefix itself and strips it from listings. Error values carry no text from the
//! server and none of the caller's data: [`RemoteError::message`] is fixed wording, so a bucket
//! name, an access key or a presigned URL can never end up in a message, a log or the vault file.

use super::types::TargetConfig;
use crate::storage::error::StorageError;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

/// Largest single object that is downloaded (an envelope is at most 64 MiB plus its header).
pub const MAX_OBJECT_BYTES: u64 = 72 * 1024 * 1024;
/// Largest small metadata object (key bundle, shared-targets snapshot).
pub const MAX_META_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetKind {
    Dns,
    Connect,
    Tls,
    Timeout,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    /// 412: the write's precondition was false.
    PreconditionFailed,
    /// 404 on an object.
    NotFound,
    /// 404 on the bucket itself.
    BucketMissing,
    /// 401 / `InvalidAccessKeyId`: the service does not know this access key.
    BadCredentials,
    /// `SignatureDoesNotMatch`: wrong secret key or region.
    BadSignature,
    /// `RequestTimeTooSkewed`: this device's clock is off.
    ClockSkew,
    /// 403: the key is known but not allowed to do this.
    AccessDenied,
    Unreachable(NetKind),
    /// 5xx, 429 and 409 (a conflicting request was in flight): worth retrying.
    Server(u16),
    /// Any other 4xx.
    Rejected(u16),
    /// 3xx: never followed for signed requests.
    Redirect,
    TooLarge,
    /// The answer was not what an S3 service sends.
    Malformed,
    /// The service does not implement what was asked (for example conditional writes).
    Unsupported,
    /// The target's settings cannot make a request (for example an insecure endpoint).
    BadConfig,
    /// A file on this device could not be read or written while it was being sent or received
    /// (encrypted drive). Not the service's fault: retried at the next trigger.
    LocalIo,
}

impl RemoteError {
    /// Errors that retrying cannot fix: wrong keys, a missing bucket, a refused request. The
    /// target shows `failing` at once instead of after five more attempts.
    pub fn is_hard(&self) -> bool {
        matches!(
            self,
            Self::BucketMissing
                | Self::BadCredentials
                | Self::BadSignature
                | Self::ClockSkew
                | Self::AccessDenied
                | Self::Rejected(_)
                | Self::Redirect
                | Self::BadConfig
        )
    }

    /// Fixed, human-readable wording. Contains nothing from the request or the response.
    pub fn message(&self) -> String {
        match self {
            Self::PreconditionFailed => {
                "The object changed on the storage service while it was being written.".into()
            }
            Self::NotFound => "The object was not found on the storage service.".into(),
            Self::BucketMissing => {
                "The bucket was not found. Check the bucket name, the region and the endpoint."
                    .into()
            }
            Self::BadCredentials => {
                "The storage service does not know this access key id. Check the key and the endpoint."
                    .into()
            }
            Self::BadSignature => {
                "The storage service rejected the signature. The secret access key or the region is probably wrong."
                    .into()
            }
            Self::ClockSkew => {
                "This device's clock is too far from the storage service's clock. Fix the date and time."
                    .into()
            }
            Self::AccessDenied => {
                "The storage service refused access (HTTP 403). Check the keys and the bucket permissions."
                    .into()
            }
            Self::Unreachable(NetKind::Dns) => {
                "The storage service's address could not be found. Check the endpoint and the internet connection."
                    .into()
            }
            Self::Unreachable(NetKind::Connect) => {
                "Could not connect to the storage service. Check the endpoint and the internet connection."
                    .into()
            }
            Self::Unreachable(NetKind::Tls) => {
                "A secure connection to the storage service could not be made (certificate or TLS problem)."
                    .into()
            }
            Self::Unreachable(NetKind::Timeout) => {
                "The storage service did not answer in time.".into()
            }
            Self::Unreachable(NetKind::Io) => {
                "The connection to the storage service was interrupted.".into()
            }
            Self::Server(code) => format!(
                "The storage service had a problem (HTTP {code}). It will be tried again."
            ),
            Self::Rejected(code) => {
                format!("The storage service rejected the request (HTTP {code}).")
            }
            Self::Redirect => {
                "The storage service tried to redirect the request, which is not allowed. Check the endpoint and the region."
                    .into()
            }
            Self::TooLarge => "A stored object is larger than allowed.".into(),
            Self::Malformed => {
                "The storage service sent an answer that could not be understood.".into()
            }
            Self::Unsupported => {
                "The storage service does not support what was asked.".into()
            }
            Self::BadConfig => {
                "The target's settings cannot be used to reach the storage service.".into()
            }
            Self::LocalIo => {
                "A file on this device could not be read or written during the transfer.".into()
            }
        }
    }
}

impl From<RemoteError> for StorageError {
    fn from(e: RemoteError) -> Self {
        match e {
            RemoteError::Unreachable(_) => Self::Network(e.message()),
            _ => Self::Remote(e.message()),
        }
    }
}

/// Precondition on a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    None,
    /// `If-None-Match: *`: only if the key does not exist.
    IfNoneMatch,
    /// `If-Match: <etag>`: only if the stored object still has this (opaque) ETag.
    IfMatch(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutOk {
    pub etag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub bytes: Vec<u8>,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    pub size: u64,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    /// Logical key (the target's folder prefix is already removed).
    pub key: String,
    pub size: u64,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPage {
    pub entries: Vec<ListEntry>,
    /// Pass back as `token` for the next page; `None` when the listing is complete.
    pub next_token: Option<String>,
}

pub trait ObjectStore: Send + Sync {
    fn put(&self, key: &str, body: &[u8], cond: &Condition) -> Result<PutOk, RemoteError>;
    /// `Ok(None)` when the object does not exist. Larger than `max_bytes` is `TooLarge`.
    fn get(&self, key: &str, max_bytes: u64) -> Result<Option<Object>, RemoteError>;
    fn head(&self, key: &str) -> Result<Option<ObjectMeta>, RemoteError>;
    /// Deleting something that is not there succeeds.
    fn delete(&self, key: &str) -> Result<(), RemoteError>;
    fn list(
        &self,
        prefix: &str,
        token: Option<&str>,
        max_keys: usize,
    ) -> Result<ListPage, RemoteError>;

    /// Upload the file at `path` as one object (encrypted drive blobs). A real client streams it
    /// from disk; the default reads it into memory (used by the in-memory test bucket).
    fn put_file(&self, key: &str, path: &Path, cond: &Condition) -> Result<PutOk, RemoteError> {
        let bytes = std::fs::read(path).map_err(|_| RemoteError::LocalIo)?;
        self.put(key, &bytes, cond)
    }

    /// Download an object into `dest` (encrypted drive blobs and metadata). `Ok(None)` when the
    /// object does not exist; otherwise the number of bytes written. Larger than `max_bytes` is
    /// `TooLarge`, and nothing past the limit is written. A real client streams to `dest`.
    fn get_to(
        &self,
        key: &str,
        dest: &mut dyn Write,
        max_bytes: u64,
    ) -> Result<Option<u64>, RemoteError> {
        match self.get(key, max_bytes)? {
            None => Ok(None),
            Some(obj) => {
                dest.write_all(&obj.bytes)
                    .map_err(|_| RemoteError::LocalIo)?;
                Ok(Some(obj.bytes.len() as u64))
            }
        }
    }
}

/// Builds the store for a target's settings (the S3 client in the app, in-memory buckets in tests).
pub trait StoreFactory: Send + Sync {
    fn open(&self, config: &TargetConfig) -> Result<Arc<dyn ObjectStore>, RemoteError>;
}

/// The production factory: a real S3 client with rustls.
pub struct S3Factory;

impl StoreFactory for S3Factory {
    fn open(&self, config: &TargetConfig) -> Result<Arc<dyn ObjectStore>, RemoteError> {
        Ok(Arc::new(super::s3::S3Store::new(config)?))
    }
}

/// Every key of a vault, in one place.
pub mod keys {
    pub const BUNDLE: &str = "vault/meta/keybundle.json";
    pub const TARGETS: &str = "vault/meta/targets.enc";
    pub const ITEMS_PREFIX: &str = "vault/items/";
    pub const VERSIONS_PREFIX: &str = "vault/versions/";
    pub const TOMBSTONES_PREFIX: &str = "vault/tombstones/";

    pub fn item(id: &str) -> String {
        format!("{ITEMS_PREFIX}{id}.enc")
    }

    pub fn version(id: &str, version: u64, ts: u64) -> String {
        format!("{VERSIONS_PREFIX}{id}/{version}_{ts}.enc")
    }

    pub fn versions_of(id: &str) -> String {
        format!("{VERSIONS_PREFIX}{id}/")
    }

    pub fn tombstone(id: &str) -> String {
        format!("{TOMBSTONES_PREFIX}{id}.enc")
    }

    /// `vault/items/<id>.enc` -> `<id>` (and nothing for any other shape).
    pub fn parse_item(key: &str) -> Option<&str> {
        let id = key.strip_prefix(ITEMS_PREFIX)?.strip_suffix(".enc")?;
        crate::storage::uuid::is_valid(id).then_some(id)
    }

    pub fn parse_tombstone(key: &str) -> Option<&str> {
        let id = key.strip_prefix(TOMBSTONES_PREFIX)?.strip_suffix(".enc")?;
        crate::storage::uuid::is_valid(id).then_some(id)
    }

    /// `vault/versions/<id>/<n>_<ts>.enc` -> `(<id>, n, ts)`.
    pub fn parse_version(key: &str) -> Option<(&str, u64, u64)> {
        let rest = key.strip_prefix(VERSIONS_PREFIX)?;
        let (id, file) = rest.split_once('/')?;
        let stem = file.strip_suffix(".enc")?;
        let (n, ts) = stem.split_once('_')?;
        if !crate::storage::uuid::is_valid(id) || n.is_empty() || ts.is_empty() {
            return None;
        }
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if !digits(n) || !digits(ts) {
            return None;
        }
        Some((id, n.parse().ok()?, ts.parse().ok()?))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const ID: &str = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";

        #[test]
        fn keys_round_trip() {
            assert_eq!(parse_item(&item(ID)), Some(ID));
            assert_eq!(parse_tombstone(&tombstone(ID)), Some(ID));
            assert_eq!(
                parse_version(&version(ID, 3, 1700000000123)),
                Some((ID, 3, 1700000000123))
            );
            assert_eq!(item(ID), format!("vault/items/{ID}.enc"));
            assert_eq!(version(ID, 2, 9), format!("vault/versions/{ID}/2_9.enc"));
        }

        #[test]
        fn foreign_shapes_are_not_parsed() {
            assert_eq!(parse_item("vault/items/notes.txt"), None);
            assert_eq!(parse_item("vault/items/../x.enc"), None);
            assert_eq!(parse_item(&format!("vault/other/{ID}.enc")), None);
            assert_eq!(parse_version(&format!("vault/versions/{ID}/x_1.enc")), None);
            assert_eq!(parse_version(&format!("vault/versions/{ID}/1.enc")), None);
            assert_eq!(parse_version("vault/versions/nope/1_2.enc"), None);
            assert_eq!(parse_tombstone(&format!("vault/tombstones/{ID}")), None);
        }
    }
}

// ------------------------------------------------------------------ in-memory store (tests)

#[cfg(test)]
pub mod mem {
    use super::*;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Mutex;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Op {
        Put,
        Get,
        Head,
        Delete,
        List,
    }

    /// How the fake bucket treats `If-None-Match` / `If-Match`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Mode {
        /// Like S3: a false precondition answers 412.
        Enforce,
        /// Like a legacy service: the headers are ignored.
        Ignore,
        /// The service refuses requests that carry them.
        Reject,
    }

    struct Fault {
        op: Option<Op>,
        err: RemoteError,
        remaining: Option<usize>,
    }

    struct Inner {
        objects: BTreeMap<String, (Vec<u8>, String)>,
        counter: u64,
        mode: Mode,
        page: usize,
        faults: Vec<Fault>,
        log: Vec<(Op, String, Condition)>,
        /// Every call waits this long first (a slow service), outside the bucket's lock.
        delay: std::time::Duration,
    }

    /// An in-memory bucket with fault injection and a call log.
    pub struct MemStore {
        inner: Mutex<Inner>,
    }

    impl MemStore {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                inner: Mutex::new(Inner {
                    objects: BTreeMap::new(),
                    counter: 0,
                    mode: Mode::Enforce,
                    page: 1000,
                    faults: Vec::new(),
                    log: Vec::new(),
                    delay: std::time::Duration::ZERO,
                }),
            })
        }

        /// Make every call take at least `d` (a slow service).
        pub fn set_delay(&self, d: std::time::Duration) {
            self.lock().delay = d;
        }

        fn pause(&self) {
            let d = self.lock().delay;
            if !d.is_zero() {
                std::thread::sleep(d);
            }
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
            self.inner.lock().unwrap_or_else(|p| p.into_inner())
        }

        pub fn set_mode(&self, mode: Mode) {
            self.lock().mode = mode;
        }

        pub fn set_page_size(&self, n: usize) {
            self.lock().page = n.max(1);
        }

        /// Every call of `op` (or of any op) fails with `err` until [`clear_faults`](Self::clear_faults).
        pub fn fail_all(&self, op: Option<Op>, err: RemoteError) {
            self.lock().faults.push(Fault {
                op,
                err,
                remaining: None,
            });
        }

        /// The next `n` calls of `op` fail with `err`.
        pub fn fail_next(&self, op: Option<Op>, err: RemoteError, n: usize) {
            self.lock().faults.push(Fault {
                op,
                err,
                remaining: Some(n),
            });
        }

        pub fn clear_faults(&self) {
            self.lock().faults.clear();
        }

        pub fn keys(&self) -> Vec<String> {
            self.lock().objects.keys().cloned().collect()
        }

        pub fn raw(&self, key: &str) -> Option<Vec<u8>> {
            self.lock().objects.get(key).map(|(b, _)| b.clone())
        }

        pub fn etag_of(&self, key: &str) -> Option<String> {
            self.lock().objects.get(key).map(|(_, e)| e.clone())
        }

        /// Write behind the client's back (another device, a manual edit).
        pub fn plant(&self, key: &str, bytes: &[u8]) -> String {
            let mut g = self.lock();
            g.counter += 1;
            let etag = format!("\"mem-{}\"", g.counter);
            g.objects
                .insert(key.to_string(), (bytes.to_vec(), etag.clone()));
            etag
        }

        pub fn remove(&self, key: &str) {
            self.lock().objects.remove(key);
        }

        pub fn log(&self) -> Vec<(Op, String, Condition)> {
            self.lock().log.clone()
        }

        pub fn clear_log(&self) {
            self.lock().log.clear();
        }

        pub fn count(&self, op: Op) -> usize {
            self.lock().log.iter().filter(|(o, _, _)| *o == op).count()
        }

        pub fn total_calls(&self) -> usize {
            self.lock().log.len()
        }

        fn enter(g: &mut Inner, op: Op, key: &str, cond: &Condition) -> Result<(), RemoteError> {
            g.log.push((op, key.to_string(), cond.clone()));
            for f in &mut g.faults {
                if f.op.is_some_and(|o| o != op) {
                    continue;
                }
                match &mut f.remaining {
                    None => return Err(f.err.clone()),
                    Some(0) => {}
                    Some(n) => {
                        *n -= 1;
                        return Err(f.err.clone());
                    }
                }
            }
            g.faults.retain(|f| f.remaining != Some(0));
            Ok(())
        }
    }

    impl ObjectStore for MemStore {
        fn put(&self, key: &str, body: &[u8], cond: &Condition) -> Result<PutOk, RemoteError> {
            self.pause();
            let mut g = self.lock();
            Self::enter(&mut g, Op::Put, key, cond)?;
            if *cond != Condition::None {
                match g.mode {
                    Mode::Reject => return Err(RemoteError::Unsupported),
                    Mode::Ignore => {}
                    Mode::Enforce => match (cond, g.objects.get(key)) {
                        (Condition::IfNoneMatch, Some(_)) => {
                            return Err(RemoteError::PreconditionFailed)
                        }
                        (Condition::IfMatch(_), None) => return Err(RemoteError::NotFound),
                        (Condition::IfMatch(want), Some((_, have))) if want != have => {
                            return Err(RemoteError::PreconditionFailed)
                        }
                        _ => {}
                    },
                }
            }
            g.counter += 1;
            let etag = format!("\"mem-{}\"", g.counter);
            g.objects
                .insert(key.to_string(), (body.to_vec(), etag.clone()));
            Ok(PutOk { etag: Some(etag) })
        }

        fn get(&self, key: &str, max_bytes: u64) -> Result<Option<Object>, RemoteError> {
            self.pause();
            let mut g = self.lock();
            Self::enter(&mut g, Op::Get, key, &Condition::None)?;
            match g.objects.get(key) {
                None => Ok(None),
                Some((b, _)) if b.len() as u64 > max_bytes => Err(RemoteError::TooLarge),
                Some((b, e)) => Ok(Some(Object {
                    bytes: b.clone(),
                    etag: Some(e.clone()),
                })),
            }
        }

        fn head(&self, key: &str) -> Result<Option<ObjectMeta>, RemoteError> {
            self.pause();
            let mut g = self.lock();
            Self::enter(&mut g, Op::Head, key, &Condition::None)?;
            Ok(g.objects.get(key).map(|(b, e)| ObjectMeta {
                size: b.len() as u64,
                etag: Some(e.clone()),
            }))
        }

        fn delete(&self, key: &str) -> Result<(), RemoteError> {
            self.pause();
            let mut g = self.lock();
            Self::enter(&mut g, Op::Delete, key, &Condition::None)?;
            g.objects.remove(key);
            Ok(())
        }

        fn list(
            &self,
            prefix: &str,
            token: Option<&str>,
            max_keys: usize,
        ) -> Result<ListPage, RemoteError> {
            self.pause();
            let mut g = self.lock();
            Self::enter(&mut g, Op::List, prefix, &Condition::None)?;
            let limit = max_keys.clamp(1, g.page);
            let mut entries = Vec::new();
            let mut more = false;
            for (k, (b, e)) in g.objects.range(prefix.to_string()..) {
                if !k.starts_with(prefix) {
                    break;
                }
                if token.is_some_and(|t| k.as_str() <= t) {
                    continue;
                }
                if entries.len() == limit {
                    more = true;
                    break;
                }
                entries.push(ListEntry {
                    key: k.clone(),
                    size: b.len() as u64,
                    etag: Some(e.clone()),
                });
            }
            let next_token = more
                .then(|| entries.last().map(|e| e.key.clone()))
                .flatten();
            Ok(ListPage {
                entries,
                next_token,
            })
        }
    }

    /// Hands out one in-memory bucket per bucket name, so a restore into a fresh profile finds
    /// what a backup wrote.
    #[derive(Default)]
    pub struct MemFactory {
        buckets: Mutex<HashMap<String, Arc<MemStore>>>,
        /// Settings of every `open` call, for tests that check which target was contacted.
        opened: Mutex<Vec<String>>,
    }

    impl MemFactory {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub fn bucket(&self, name: &str) -> Arc<MemStore> {
            let mut g = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(g.entry(name.to_string()).or_insert_with(MemStore::new))
        }

        pub fn opened(&self) -> Vec<String> {
            self.opened
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }
    }

    impl StoreFactory for MemFactory {
        fn open(&self, config: &TargetConfig) -> Result<Arc<dyn ObjectStore>, RemoteError> {
            self.opened
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(config.bucket.clone());
            Ok(self.bucket(&config.bucket))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn conditional_puts_behave_like_s3() {
            let s = MemStore::new();
            let a = s
                .put("k", b"1", &Condition::IfNoneMatch)
                .unwrap()
                .etag
                .unwrap();
            assert_eq!(
                s.put("k", b"2", &Condition::IfNoneMatch).unwrap_err(),
                RemoteError::PreconditionFailed
            );
            assert_eq!(
                s.put("k", b"2", &Condition::IfMatch("\"x\"".into()))
                    .unwrap_err(),
                RemoteError::PreconditionFailed
            );
            let b = s
                .put("k", b"3", &Condition::IfMatch(a))
                .unwrap()
                .etag
                .unwrap();
            assert_eq!(s.raw("k").unwrap(), b"3");
            assert_eq!(s.etag_of("k"), Some(b));
            assert_eq!(
                s.put("gone", b"x", &Condition::IfMatch("\"x\"".into()))
                    .unwrap_err(),
                RemoteError::NotFound
            );
        }

        #[test]
        fn weak_and_rejecting_modes() {
            let s = MemStore::new();
            s.set_mode(Mode::Ignore);
            s.put("k", b"1", &Condition::IfNoneMatch).unwrap();
            s.put("k", b"2", &Condition::IfNoneMatch).unwrap();
            assert_eq!(s.raw("k").unwrap(), b"2");
            s.set_mode(Mode::Reject);
            assert_eq!(
                s.put("k", b"3", &Condition::IfNoneMatch).unwrap_err(),
                RemoteError::Unsupported
            );
            s.put("k", b"3", &Condition::None).unwrap();
        }

        #[test]
        fn listing_pages_and_faults() {
            let s = MemStore::new();
            s.set_page_size(2);
            for i in 0..5 {
                s.put(&format!("p/{i}"), b"x", &Condition::None).unwrap();
            }
            s.put("q/0", b"x", &Condition::None).unwrap();
            let mut seen = Vec::new();
            let mut token: Option<String> = None;
            loop {
                let page = s.list("p/", token.as_deref(), 1000).unwrap();
                assert!(page.entries.len() <= 2);
                seen.extend(page.entries.into_iter().map(|e| e.key));
                match page.next_token {
                    Some(t) => token = Some(t),
                    None => break,
                }
            }
            assert_eq!(seen, ["p/0", "p/1", "p/2", "p/3", "p/4"]);

            s.fail_next(Some(Op::Get), RemoteError::Server(503), 2);
            assert!(s.get("p/0", 10).is_err());
            assert!(s.get("p/0", 10).is_err());
            assert!(s.get("p/0", 10).unwrap().is_some());
            s.fail_all(None, RemoteError::AccessDenied);
            assert_eq!(s.head("p/0").unwrap_err(), RemoteError::AccessDenied);
            s.clear_faults();
            assert!(s.head("p/0").unwrap().is_some());
            assert_eq!(s.get("p/0", 0).unwrap_err(), RemoteError::TooLarge);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_never_carry_request_data_and_hard_errors_are_classified() {
        let all = [
            RemoteError::PreconditionFailed,
            RemoteError::NotFound,
            RemoteError::BucketMissing,
            RemoteError::BadCredentials,
            RemoteError::BadSignature,
            RemoteError::ClockSkew,
            RemoteError::AccessDenied,
            RemoteError::Unreachable(NetKind::Dns),
            RemoteError::Unreachable(NetKind::Connect),
            RemoteError::Unreachable(NetKind::Tls),
            RemoteError::Unreachable(NetKind::Timeout),
            RemoteError::Unreachable(NetKind::Io),
            RemoteError::Server(503),
            RemoteError::Rejected(400),
            RemoteError::Redirect,
            RemoteError::TooLarge,
            RemoteError::Malformed,
            RemoteError::Unsupported,
            RemoteError::BadConfig,
        ];
        for e in &all {
            let m = e.message();
            assert!(!m.is_empty());
            assert!(!m.contains("http://") && !m.contains("https://"), "{m}");
            assert!(!m.contains("X-Amz"), "{m}");
        }
        assert!(RemoteError::AccessDenied.is_hard());
        assert!(RemoteError::BadSignature.is_hard());
        assert!(!RemoteError::Server(500).is_hard());
        assert!(!RemoteError::Unreachable(NetKind::Timeout).is_hard());
        assert_eq!(
            StorageError::from(RemoteError::Unreachable(NetKind::Dns)).code(),
            "network"
        );
        assert_eq!(
            StorageError::from(RemoteError::AccessDenied).code(),
            "remote"
        );
        // the only status codes that appear are the service's own, never a URL
        assert!(RemoteError::Server(503).message().contains("503"));
    }
}
