//! Wire types, internal target configuration and input validation for backup targets
//! (`ai-docs/IPC_CONTRACT.md` §6).
//!
//! Rules enforced here, in Rust, before anything is stored or any network call is made:
//! the endpoint is `https` (plain `http` only for loopback), has no credentials, query or
//! fragment; names, regions, buckets and prefixes use a small safe alphabet; secrets are trimmed
//! and may not contain whitespace. Nothing that holds a secret implements `Debug` with its value.

use crate::storage::error::{Result, StorageError};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use url::{Host, Url};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_PREFIX_CHARS: usize = 200;
pub const MAX_ENDPOINT_CHARS: usize = 512;
const MAX_KEY_ID_CHARS: usize = 128;
const MAX_SECRET_CHARS: usize = 512;

fn yes() -> bool {
    true
}

/// What the UI sends to add, update or test a target. The secret key is wiped on drop.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct BackupTargetInput {
    pub name: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    pub access_key_id: String,
    /// Empty on `backup_update` keeps the stored secret.
    #[serde(default)]
    pub secret_access_key: String,
    #[serde(default)]
    pub path_style: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub master: bool,
    #[serde(default = "yes")]
    pub share_in_master: bool,
}

impl std::fmt::Debug for BackupTargetInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BackupTargetInput(<redacted>)")
    }
}

/// How well a bucket honours `If-None-Match` / `If-Match` on writes (`STORAGE_AND_SYNC.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConditionalWrites {
    /// Both preconditions were enforced (answered 412) by the probe.
    Strong,
    /// The service accepted at least one write whose precondition was false: it ignores them.
    Weak,
    /// The probe could not tell (the service rejected the headers, or the probe did not run).
    Unknown,
}

impl ConditionalWrites {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strong => "strong",
            Self::Weak => "weak",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "strong" => Self::Strong,
            "weak" => Self::Weak,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Untested,
    Healthy,
    Lagging,
    Failing,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetHealth {
    pub state: HealthState,
    pub queued: u64,
    pub last_success_at: Option<u64>,
    pub last_checked_at: Option<u64>,
    pub last_error: Option<String>,
    pub conditional_writes: ConditionalWrites,
}

/// What the UI gets back. Never contains the secret key (`has_secret` says whether one is stored).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupTargetView {
    pub id: String,
    pub name: String,
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key_id: String,
    pub has_secret: bool,
    pub path_style: bool,
    pub enabled: bool,
    pub master: bool,
    pub share_in_master: bool,
    pub health: TargetHealth,
}

/// Result of `backup_test`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupTestResult {
    pub ok: bool,
    pub message: String,
    pub conditional_writes: ConditionalWrites,
}

/// Connection details of one target. This is the plaintext that is sealed into the vault file;
/// it exists in memory only while the vault is unlocked (or during a test/restore) and is wiped
/// on drop. `Debug` never prints a value.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct TargetConfig {
    pub name: String,
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub path_style: bool,
}

impl std::fmt::Debug for TargetConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TargetConfig(<redacted>)")
    }
}

impl TargetConfig {
    /// Validate and normalize an input. `secret_required` is true when no stored secret exists
    /// to fall back on (add, test, restore).
    pub fn from_input(input: &BackupTargetInput, secret_required: bool) -> Result<Self> {
        let secret = normalize_secret(&input.secret_access_key)?;
        if secret.is_empty() && secret_required {
            return Err(StorageError::invalid("Enter the secret access key."));
        }
        let region = normalize_region(&input.region)?;
        let endpoint = normalize_endpoint(input.endpoint.as_deref())?;
        let bucket = normalize_bucket(&input.bucket)?;
        let cfg = Self {
            name: normalize_name(&input.name)?,
            endpoint,
            region,
            bucket,
            prefix: normalize_prefix(&input.prefix)?,
            access_key_id: normalize_key_id(&input.access_key_id)?,
            secret_access_key: secret,
            path_style: input.path_style,
        };
        Ok(cfg)
    }

    /// The same target as the UI shows it (no secret).
    pub fn has_secret(&self) -> bool {
        !self.secret_access_key.is_empty()
    }
}

// ------------------------------------------------------------------ field rules

fn bad(message: &str) -> StorageError {
    StorageError::invalid(message)
}

/// Unicode NFKC, trimmed, whitespace runs collapsed, 1-60 characters, no control characters.
pub fn normalize_name(raw: &str) -> Result<String> {
    let nfkc: String = raw.nfkc().collect();
    let trimmed = nfkc.trim();
    if trimmed.is_empty() {
        return Err(bad("Enter a name for this backup target."));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(bad(
            "The target name contains characters that cannot be shown.",
        ));
    }
    let collapsed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_NAME_CHARS {
        return Err(bad("Use at most 60 characters for the target name."));
    }
    Ok(collapsed)
}

/// What name uniqueness compares.
pub fn name_key(normalized: &str) -> String {
    normalized.to_lowercase().nfkc().collect()
}

fn normalize_region(raw: &str) -> Result<String> {
    let r = raw.trim();
    if r.is_empty() {
        return Err(bad(
            "Enter the region (for example us-east-1; some services accept any name or \"auto\").",
        ));
    }
    let ok = r.len() <= 64
        && r.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if !ok {
        return Err(bad(
            "The region may only contain letters, digits, dots, dashes and underscores.",
        ));
    }
    Ok(r.to_string())
}

fn normalize_bucket(raw: &str) -> Result<String> {
    let b = raw.trim();
    if b.is_empty() {
        return Err(bad("Enter the bucket name."));
    }
    let chars_ok = b
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'));
    let edges_ok = !b.starts_with(['.', '-']) && !b.ends_with(['.', '-']) && !b.contains("..");
    if !(3..=63).contains(&b.len()) || !chars_ok || !edges_ok {
        return Err(bad(
            "The bucket name must be 3 to 63 characters of letters, digits, dots and dashes.",
        ));
    }
    Ok(b.to_string())
}

/// Folder inside the bucket: no leading or trailing slash, no `.` or `..` parts, no empty parts.
fn normalize_prefix(raw: &str) -> Result<String> {
    let p = raw.trim();
    if p.chars().any(|c| c.is_control() || c == '\\') {
        return Err(bad(
            "The folder (prefix) contains characters that cannot be used.",
        ));
    }
    let mut parts = Vec::new();
    for seg in p.split('/') {
        match seg.trim() {
            "" => {}
            "." | ".." => return Err(bad("The folder (prefix) may not contain . or .. parts.")),
            s => parts.push(s),
        }
    }
    let joined = parts.join("/");
    if joined.chars().count() > MAX_PREFIX_CHARS {
        return Err(bad("The folder (prefix) is too long."));
    }
    Ok(joined)
}

fn normalize_key_id(raw: &str) -> Result<String> {
    let k = raw.trim();
    if k.is_empty() {
        return Err(bad("Enter the access key id."));
    }
    if k.chars().count() > MAX_KEY_ID_CHARS
        || k.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(bad("The access key id contains spaces or is too long."));
    }
    Ok(k.to_string())
}

/// Trimmed (a pasted key often carries a newline); interior whitespace is refused. Empty is
/// allowed here: the caller decides whether a secret is required.
fn normalize_secret(raw: &str) -> Result<String> {
    let s = raw.trim();
    if s.chars().count() > MAX_SECRET_CHARS
        || s.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(bad(
            "The secret access key contains spaces or line breaks, or is too long. Paste it again.",
        ));
    }
    Ok(s.to_string())
}

/// `None` (AWS) or a canonical endpoint URL: `https`, or `http` for loopback only; no
/// credentials, query or fragment.
pub fn normalize_endpoint(raw: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if raw.chars().count() > MAX_ENDPOINT_CHARS
        || raw.chars().any(|c| c.is_whitespace() || c.is_control())
        || raw.contains('\\')
    {
        return Err(bad("The endpoint is not a valid web address."));
    }
    let url = Url::parse(raw).map_err(|_| {
        bad("The endpoint is not a valid web address (for example https://s3.example.com).")
    })?;
    if url.host_str().is_none() || url.cannot_be_a_base() {
        return Err(bad("The endpoint needs a host name."));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad(
            "Do not put the keys in the endpoint address. Enter them in the key fields.",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(bad(
            "The endpoint must not contain a query string or fragment.",
        ));
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(&url) => {}
        "http" => {
            return Err(bad(
                "The endpoint must use https. Plain http is only allowed for localhost.",
            ))
        }
        _ => return Err(bad("The endpoint must start with https://.")),
    }
    // Canonical form without a trailing slash (the host part always keeps its own characters).
    Ok(Some(url.as_str().trim_end_matches('/').to_string()))
}

/// `localhost`, `*.localhost`, `127.0.0.0/8` and `::1`.
pub fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> BackupTargetInput {
        BackupTargetInput {
            name: "  Home   MinIO ".into(),
            endpoint: Some("http://127.0.0.1:9000/".into()),
            region: "us-east-1".into(),
            bucket: "passman-test".into(),
            prefix: "/backups//mine/".into(),
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "  s3cr3t-value\n".into(),
            path_style: true,
            enabled: true,
            master: false,
            share_in_master: true,
        }
    }

    #[test]
    fn a_good_input_is_normalized() {
        let c = TargetConfig::from_input(&input(), true).unwrap();
        assert_eq!(c.name, "Home MinIO");
        assert_eq!(c.endpoint.as_deref(), Some("http://127.0.0.1:9000"));
        assert_eq!(c.prefix, "backups/mine");
        assert_eq!(c.secret_access_key, "s3cr3t-value");
        assert!(c.has_secret());
    }

    #[test]
    fn debug_never_prints_a_value() {
        let c = TargetConfig::from_input(&input(), true).unwrap();
        assert!(!format!("{c:?}").contains("s3cr3t"));
        assert!(!format!("{:?}", input()).contains("s3cr3t"));
    }

    #[test]
    fn endpoint_rules() {
        let ok = |s: &str| normalize_endpoint(Some(s));
        assert_eq!(ok("").unwrap(), None);
        assert_eq!(ok("   ").unwrap(), None);
        assert_eq!(normalize_endpoint(None).unwrap(), None);
        assert_eq!(
            ok("https://s3.eu-central-003.backblazeb2.com/")
                .unwrap()
                .as_deref(),
            Some("https://s3.eu-central-003.backblazeb2.com")
        );
        assert_eq!(
            ok("https://objects.example.com/storage/")
                .unwrap()
                .as_deref(),
            Some("https://objects.example.com/storage")
        );
        for loopback in [
            "http://localhost:9000",
            "http://127.0.0.1:9000",
            "http://127.9.9.9",
            "http://[::1]:9000",
            "http://minio.localhost:9000",
        ] {
            assert!(ok(loopback).is_ok(), "{loopback}");
        }
        for refused in [
            "http://s3.example.com",
            "http://192.168.1.10:9000",
            "http://localhost.evil.com",
            "ftp://s3.example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://user:pw@s3.example.com",
            "https://user@s3.example.com",
            "https://s3.example.com/?x=1",
            "https://s3.example.com/#frag",
            "https://s3.example.com/a b",
            "https://s3.example.com\\evil",
            "s3.example.com",
            "https://",
            "https://s3.example.com/\u{0}",
        ] {
            let e = ok(refused).expect_err(refused);
            assert_eq!(e.code(), "invalid_input", "{refused}");
        }
    }

    #[test]
    fn field_rules() {
        let mut i = input();
        i.bucket = "ab".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        i.bucket = "good.bucket-1".into();
        assert!(TargetConfig::from_input(&i, true).is_ok());
        i.bucket = "bad/bucket".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        i.bucket = "double..dot".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        let mut i = input();
        i.prefix = "a/../b".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        i.prefix = "a\\b".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        let mut i = input();
        i.region = "us east".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        i.region = String::new();
        assert!(TargetConfig::from_input(&i, true).is_err());
        let mut i = input();
        i.name = "  ".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        i.name = "x".repeat(61);
        assert!(TargetConfig::from_input(&i, true).is_err());
        i.name = "bad\u{7}name".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        let mut i = input();
        i.access_key_id = "has space".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
        let mut i = input();
        i.secret_access_key = "two words".into();
        assert!(TargetConfig::from_input(&i, true).is_err());
    }

    #[test]
    fn an_empty_secret_is_only_allowed_when_one_is_stored() {
        let mut i = input();
        i.secret_access_key = String::new();
        let e = TargetConfig::from_input(&i, true).expect_err("required");
        assert_eq!(e.code(), "invalid_input");
        let c = TargetConfig::from_input(&i, false).unwrap();
        assert!(!c.has_secret());
    }

    #[test]
    fn wire_shape_is_camel_case_with_defaults() {
        let i: BackupTargetInput = serde_json::from_value(serde_json::json!({
            "name": "n", "endpoint": null, "region": "r1", "bucket": "bucket1",
            "prefix": "", "accessKeyId": "k", "secretAccessKey": "s", "pathStyle": false,
            "enabled": true, "master": true, "shareInMaster": false
        }))
        .unwrap();
        assert!(i.master && !i.share_in_master && i.enabled);
        let lenient: BackupTargetInput = serde_json::from_value(serde_json::json!({
            "name": "n", "region": "r1", "bucket": "bucket1", "accessKeyId": "k"
        }))
        .unwrap();
        assert!(lenient.enabled && lenient.share_in_master && !lenient.master);
        let v = serde_json::to_value(TargetHealth {
            state: HealthState::Lagging,
            queued: 2,
            last_success_at: Some(5),
            last_checked_at: None,
            last_error: None,
            conditional_writes: ConditionalWrites::Weak,
        })
        .unwrap();
        assert_eq!(v["state"], "lagging");
        assert_eq!(v["lastSuccessAt"], 5);
        assert_eq!(v["conditionalWrites"], "weak");
    }
}
