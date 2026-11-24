//! The real object store: SigV4-presigned requests (`rusty-s3`, sans-IO) sent with `ureq`
//! over rustls (no OpenSSL).
//!
//! Safety rules, each pinned by a test:
//! * `https` only, except plain `http` to a loopback address (the mock server and a local MinIO);
//! * **no redirects are ever followed**: a signed request must go to the host it was signed for,
//!   so a 3xx is an error, not a hop;
//! * connect, response, body and total timeouts; response bodies are read with a hard size cap;
//! * the signature travels in the query string of the request only for as long as the request
//!   (valid for 15 minutes, never stored, never logged); [`RemoteError`] messages are fixed
//!   wording, so no URL, bucket name, key or secret can appear in an error;
//! * environment proxies are honoured for internet endpoints and never used for loopback.

use super::remote::{
    Condition, ListEntry, ListPage, NetKind, Object, ObjectMeta, ObjectStore, PutOk, RemoteError,
};
use super::types::{is_loopback, TargetConfig};
use rusty_s3::actions::{DeleteObject, GetObject, HeadObject, ListObjectsV2, PutObject};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
use url::{Host, Url};

/// How long a presigned URL stays valid. AWS tolerates 15 minutes of clock difference, so a
/// shorter window would reject requests from a device whose clock is a little slow.
const SIGN_FOR: Duration = Duration::from_secs(900);
const MAX_ERROR_BODY: u64 = 8 * 1024;
const MAX_LIST_BODY: u64 = 8 * 1024 * 1024;
const MAX_ETAG_LEN: usize = 256;
/// Whole-transfer time allowed for one encrypted-drive object (up to about 50 MiB). The usual
/// limits above are for small objects; a slow uplink needs minutes, and the transfer still ends.
/// Largest encrypted-drive object that is sent (a 50 MiB file plus its chunk overhead).
const MAX_FILE_OBJECT: u64 = 52_428_800 + 1024 * 1024;
const FILE_TRANSFER: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy)]
pub struct S3Limits {
    pub connect: Duration,
    pub response: Duration,
    pub body: Duration,
    pub global: Duration,
}

impl Default for S3Limits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            response: Duration::from_secs(30),
            body: Duration::from_secs(60),
            global: Duration::from_secs(120),
        }
    }
}

pub struct S3Store {
    bucket: Bucket,
    credentials: Credentials,
    agent: ureq::Agent,
    /// `""` or `"folder/"`.
    prefix: String,
}

impl S3Store {
    pub fn new(config: &TargetConfig) -> Result<Self, RemoteError> {
        Self::with_limits(config, S3Limits::default())
    }

    pub fn with_limits(config: &TargetConfig, limits: S3Limits) -> Result<Self, RemoteError> {
        let (endpoint, is_aws) = match config.endpoint.as_deref() {
            None => (
                Url::parse(&format!("https://s3.{}.amazonaws.com", config.region))
                    .map_err(|_| RemoteError::BadConfig)?,
                true,
            ),
            Some(e) => (Url::parse(e).map_err(|_| RemoteError::BadConfig)?, false),
        };
        let loopback = is_loopback(&endpoint);
        // Defence in depth: validation already refuses these, but this is the code that sends.
        let scheme_ok = endpoint.scheme() == "https" || (endpoint.scheme() == "http" && loopback);
        if !scheme_ok || !endpoint.username().is_empty() || endpoint.password().is_some() {
            return Err(RemoteError::BadConfig);
        }
        // Virtual-host style needs a real DNS name: not for an IP address or localhost, and not
        // for an AWS bucket whose dotted name would break the TLS wildcard certificate.
        let raw_ip = matches!(endpoint.host(), Some(Host::Ipv4(_) | Host::Ipv6(_)));
        let style =
            if config.path_style || loopback || raw_ip || (is_aws && config.bucket.contains('.')) {
                UrlStyle::Path
            } else {
                UrlStyle::VirtualHost
            };
        let bucket = Bucket::new(
            endpoint,
            style,
            config.bucket.clone(),
            config.region.clone(),
        )
        .map_err(|_| RemoteError::BadConfig)?;
        let credentials = Credentials::new(
            config.access_key_id.clone(),
            config.secret_access_key.clone(),
        );

        let mut builder = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .https_only(!loopback)
            .timeout_resolve(Some(limits.connect))
            .timeout_connect(Some(limits.connect))
            .timeout_recv_response(Some(limits.response))
            .timeout_recv_body(Some(limits.body))
            .timeout_send_body(Some(limits.body))
            .timeout_global(Some(limits.global))
            .max_response_header_size(16 * 1024);
        if loopback {
            builder = builder.proxy(None);
        }
        let agent = ureq::Agent::new_with_config(builder.build());
        let prefix = if config.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", config.prefix)
        };
        Ok(Self {
            bucket,
            credentials,
            agent,
            prefix,
        })
    }

    fn full_key(&self, logical: &str) -> String {
        format!("{}{}", self.prefix, logical)
    }

    /// Fold a non-success answer into a [`RemoteError`], reading at most 8 KiB of its body for
    /// the S3 `<Code>`.
    fn fail(mut res: ureq::http::Response<ureq::Body>) -> RemoteError {
        let status = res.status().as_u16();
        let body = res
            .body_mut()
            .with_config()
            .limit(MAX_ERROR_BODY)
            .read_to_vec()
            .ok();
        let code = body.as_deref().and_then(error_code);
        // The URL is signed a moment before it is sent, so "Request has expired" means this
        // device's clock is behind the service's (S3 answers it as a plain AccessDenied).
        if code.as_deref() == Some("AccessDenied") && body.as_deref().is_some_and(says_expired) {
            return RemoteError::ClockSkew;
        }
        map_status(status, code.as_deref())
    }
}

fn etag_of(res: &ureq::http::Response<ureq::Body>) -> Option<String> {
    res.headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= MAX_ETAG_LEN)
        .map(str::to_string)
}

impl ObjectStore for S3Store {
    fn put(&self, key: &str, body: &[u8], cond: &Condition) -> Result<PutOk, RemoteError> {
        let full = self.full_key(key);
        let mut action = PutObject::new(&self.bucket, Some(&self.credentials), &full);
        let mut sent: Option<(&str, String)> = None;
        match cond {
            Condition::None => {}
            Condition::IfNoneMatch => {
                action.headers_mut().insert("if-none-match", "*");
                sent = Some(("if-none-match", "*".to_string()));
            }
            Condition::IfMatch(etag) => {
                action.headers_mut().insert("if-match", etag.as_str());
                sent = Some(("if-match", etag.clone()));
            }
        }
        let url = action.sign(SIGN_FOR);
        let mut req = self.agent.put(url.as_str());
        if let Some((name, value)) = &sent {
            req = req.header(*name, value.as_str());
        }
        let res = req.send(body).map_err(map_error)?;
        let status = res.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(PutOk {
                etag: etag_of(&res),
            });
        }
        Err(Self::fail(res))
    }

    fn put_file(&self, key: &str, path: &Path, cond: &Condition) -> Result<PutOk, RemoteError> {
        let file = std::fs::File::open(path).map_err(|_| RemoteError::LocalIo)?;
        let len = file.metadata().map_err(|_| RemoteError::LocalIo)?.len();
        if len > MAX_FILE_OBJECT {
            return Err(RemoteError::TooLarge);
        }
        let full = self.full_key(key);
        let mut action = PutObject::new(&self.bucket, Some(&self.credentials), &full);
        let mut sent: Option<(&str, String)> = None;
        match cond {
            Condition::None => {}
            Condition::IfNoneMatch => {
                action.headers_mut().insert("if-none-match", "*");
                sent = Some(("if-none-match", "*".to_string()));
            }
            Condition::IfMatch(etag) => {
                action.headers_mut().insert("if-match", etag.as_str());
                sent = Some(("if-match", etag.clone()));
            }
        }
        let url = action.sign(SIGN_FOR);
        let mut req = self.agent.put(url.as_str());
        if let Some((name, value)) = &sent {
            req = req.header(*name, value.as_str());
        }
        // `&File` is sent with its length (no chunked encoding) and read from disk as it goes.
        let res = req
            .config()
            .timeout_global(Some(FILE_TRANSFER))
            .timeout_send_body(Some(FILE_TRANSFER))
            .timeout_recv_response(Some(FILE_TRANSFER))
            .build()
            .send(&file)
            .map_err(map_error)?;
        let status = res.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(PutOk {
                etag: etag_of(&res),
            });
        }
        Err(Self::fail(res))
    }

    fn get_to(
        &self,
        key: &str,
        dest: &mut dyn Write,
        max_bytes: u64,
    ) -> Result<Option<u64>, RemoteError> {
        let full = self.full_key(key);
        let url = GetObject::new(&self.bucket, Some(&self.credentials), &full).sign(SIGN_FOR);
        let mut res = self
            .agent
            .get(url.as_str())
            .config()
            .timeout_global(Some(FILE_TRANSFER))
            .timeout_recv_body(Some(FILE_TRANSFER))
            .build()
            .call()
            .map_err(map_error)?;
        let status = res.status().as_u16();
        if status == 404 {
            return match Self::fail(res) {
                RemoteError::BucketMissing => Err(RemoteError::BucketMissing),
                _ => Ok(None),
            };
        }
        if !(200..300).contains(&status) {
            return Err(Self::fail(res));
        }
        if res.body().content_length().is_some_and(|n| n > max_bytes) {
            return Err(RemoteError::TooLarge);
        }
        let mut reader = res.body_mut().with_config().limit(max_bytes).reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(map_read_error(e)),
            };
            dest.write_all(&buf[..n])
                .map_err(|_| RemoteError::LocalIo)?;
            total += n as u64;
        }
        Ok(Some(total))
    }

    fn get(&self, key: &str, max_bytes: u64) -> Result<Option<Object>, RemoteError> {
        let full = self.full_key(key);
        let url = GetObject::new(&self.bucket, Some(&self.credentials), &full).sign(SIGN_FOR);
        let mut res = self.agent.get(url.as_str()).call().map_err(map_error)?;
        let status = res.status().as_u16();
        if status == 404 {
            // A missing key is `None`; a missing bucket is an error.
            return match Self::fail(res) {
                RemoteError::BucketMissing => Err(RemoteError::BucketMissing),
                _ => Ok(None),
            };
        }
        if !(200..300).contains(&status) {
            return Err(Self::fail(res));
        }
        if res.body().content_length().is_some_and(|n| n > max_bytes) {
            return Err(RemoteError::TooLarge);
        }
        let etag = etag_of(&res);
        let bytes = res
            .body_mut()
            .with_config()
            .limit(max_bytes)
            .read_to_vec()
            .map_err(map_error)?;
        Ok(Some(Object { bytes, etag }))
    }

    fn head(&self, key: &str) -> Result<Option<ObjectMeta>, RemoteError> {
        let full = self.full_key(key);
        let url = HeadObject::new(&self.bucket, Some(&self.credentials), &full).sign(SIGN_FOR);
        let res = self.agent.head(url.as_str()).call().map_err(map_error)?;
        let status = res.status().as_u16();
        if status == 404 {
            return Ok(None);
        }
        if !(200..300).contains(&status) {
            return Err(Self::fail(res));
        }
        let size = res
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        Ok(Some(ObjectMeta {
            size,
            etag: etag_of(&res),
        }))
    }

    fn delete(&self, key: &str) -> Result<(), RemoteError> {
        let full = self.full_key(key);
        let url = DeleteObject::new(&self.bucket, Some(&self.credentials), &full).sign(SIGN_FOR);
        let res = self.agent.delete(url.as_str()).call().map_err(map_error)?;
        let status = res.status().as_u16();
        if (200..300).contains(&status) || status == 404 {
            return Ok(());
        }
        Err(Self::fail(res))
    }

    fn list(
        &self,
        prefix: &str,
        token: Option<&str>,
        max_keys: usize,
    ) -> Result<ListPage, RemoteError> {
        let full_prefix = self.full_key(prefix);
        let mut action = ListObjectsV2::new(&self.bucket, Some(&self.credentials));
        action.with_prefix(full_prefix.as_str());
        action.with_max_keys(max_keys.clamp(1, 1000));
        if let Some(t) = token {
            action.with_continuation_token(t);
        }
        let url = action.sign(SIGN_FOR);
        let mut res = self.agent.get(url.as_str()).call().map_err(map_error)?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Self::fail(res));
        }
        let text = res
            .body_mut()
            .with_config()
            .limit(MAX_LIST_BODY)
            .read_to_string()
            .map_err(map_error)?;
        let parsed = ListObjectsV2::parse_response(&text).map_err(|_| RemoteError::Malformed)?;
        let entries = parsed
            .contents
            .into_iter()
            .filter_map(|c| {
                // rusty-s3 already undid `encoding-type=url` (form style, as AWS and MinIO send
                // it), so a second decoding here would corrupt keys that contain `%` or `+`.
                let logical = c.key.strip_prefix(&self.prefix)?.to_string();
                Some(ListEntry {
                    key: logical,
                    size: c.size.unwrap_or(0),
                    etag: c.etag,
                })
            })
            .collect();
        Ok(ListPage {
            entries,
            next_token: parsed.next_continuation_token,
        })
    }
}

fn says_expired(body: &[u8]) -> bool {
    std::str::from_utf8(body).is_ok_and(|t| t.contains("Request has expired"))
}

/// `<Code>…</Code>` of an S3 error document, if it is a plain short word.
fn error_code(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    let start = text.find("<Code>")? + "<Code>".len();
    let end = text[start..].find("</Code>")? + start;
    let code = &text[start..end];
    (!code.is_empty() && code.len() <= 64 && code.bytes().all(|c| c.is_ascii_alphanumeric()))
        .then(|| code.to_string())
}

fn map_status(status: u16, code: Option<&str>) -> RemoteError {
    match code {
        Some("SignatureDoesNotMatch" | "AuthorizationQueryParametersError") => {
            return RemoteError::BadSignature
        }
        Some("InvalidAccessKeyId") => return RemoteError::BadCredentials,
        Some("RequestTimeTooSkewed") => return RemoteError::ClockSkew,
        Some("NoSuchBucket") => return RemoteError::BucketMissing,
        Some("NotImplemented") => return RemoteError::Unsupported,
        _ => {}
    }
    match status {
        300..=399 => RemoteError::Redirect,
        401 => RemoteError::BadCredentials,
        403 => RemoteError::AccessDenied,
        404 => RemoteError::NotFound,
        412 => RemoteError::PreconditionFailed,
        501 => RemoteError::Unsupported,
        // 409 = a conflicting request was in flight (AWS `ConditionalRequestConflict`), 408 and
        // 429 = slow down: all worth trying again.
        408 | 409 | 429 | 500..=599 => RemoteError::Server(status),
        other => RemoteError::Rejected(other),
    }
}

/// A failure while reading a response body: ureq wraps its own errors in `io::Error`.
fn map_read_error(e: std::io::Error) -> RemoteError {
    match e.downcast::<ureq::Error>() {
        Ok(inner) => map_error(inner),
        Err(io) => map_error(ureq::Error::Io(io)),
    }
}

fn map_error(e: ureq::Error) -> RemoteError {
    use ureq::Error as E;
    match e {
        E::HostNotFound => RemoteError::Unreachable(NetKind::Dns),
        E::ConnectionFailed => RemoteError::Unreachable(NetKind::Connect),
        E::Timeout(_) => RemoteError::Unreachable(NetKind::Timeout),
        E::Tls(_) | E::Rustls(_) => RemoteError::Unreachable(NetKind::Tls),
        E::Io(io) => match io.kind() {
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotConnected => {
                RemoteError::Unreachable(NetKind::Connect)
            }
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                RemoteError::Unreachable(NetKind::Timeout)
            }
            _ => RemoteError::Unreachable(NetKind::Io),
        },
        E::BodyExceedsLimit(_) => RemoteError::TooLarge,
        E::TooManyRedirects | E::RedirectFailed => RemoteError::Redirect,
        E::RequireHttpsOnly(_) | E::TlsRequired | E::BadUri(_) | E::InvalidProxyUrl => {
            RemoteError::BadConfig
        }
        E::StatusCode(c) => map_status(c, None),
        _ => RemoteError::Malformed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    fn cfg(endpoint: &str) -> TargetConfig {
        TargetConfig {
            name: "t".into(),
            endpoint: Some(endpoint.into()),
            region: "us-east-1".into(),
            bucket: "secret-bucket-name".into(),
            prefix: "folder".into(),
            access_key_id: "AKIDSECRETKEYID".into(),
            secret_access_key: "SUPERSECRETVALUE".into(),
            path_style: true,
        }
    }

    #[derive(Debug, Clone)]
    struct Seen {
        method: String,
        target: String,
        headers: Vec<(String, String)>,
        body_len: usize,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    type Reply = (u16, Vec<(String, String)>, Vec<u8>);

    /// A one-thread HTTP stub on loopback: answers each request with what `respond` returns and
    /// records what it saw. `None` from `respond` = accept and never answer.
    struct Stub {
        port: u16,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    fn read_request(stream: &mut TcpStream) -> Option<Seen> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 2048];
        let end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            let n = stream.read(&mut chunk).ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split(' ');
        let method = first.next()?.to_string();
        let target = first.next()?.to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        let len: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let mut have = buf.len() - end - 4;
        while have < len {
            let n = stream.read(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            have += n;
        }
        Some(Seen {
            method,
            target,
            headers,
            body_len: len,
        })
    }

    fn stub(respond: impl Fn(&Seen) -> Option<Reply> + Send + 'static) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let Some(req) = read_request(&mut s) else {
                    continue;
                };
                seen2.lock().unwrap().push(req.clone());
                match respond(&req) {
                    None => {
                        std::thread::sleep(Duration::from_secs(3));
                    }
                    Some((status, headers, body)) => {
                        // A HEAD answer carries the length of the object it describes, but no body.
                        let own_length = headers
                            .iter()
                            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
                        let mut head = format!("HTTP/1.1 {status} X\r\nConnection: close\r\n");
                        if !own_length {
                            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
                        }
                        for (k, v) in headers {
                            head.push_str(&format!("{k}: {v}\r\n"));
                        }
                        head.push_str("\r\n");
                        let _ = s.write_all(head.as_bytes());
                        if req.method != "HEAD" {
                            let _ = s.write_all(&body);
                        }
                    }
                }
            }
        });
        Stub { port, seen }
    }

    fn fast_limits() -> S3Limits {
        S3Limits {
            connect: Duration::from_millis(800),
            response: Duration::from_millis(600),
            body: Duration::from_millis(900),
            global: Duration::from_secs(3),
        }
    }

    fn store_for(stub: &Stub) -> S3Store {
        S3Store::with_limits(
            &cfg(&format!("http://127.0.0.1:{}", stub.port)),
            fast_limits(),
        )
        .unwrap()
    }

    fn xml_error(code: &str) -> Vec<u8> {
        format!("<?xml version=\"1.0\"?><Error><Code>{code}</Code><Message>m</Message><BucketName>secret-bucket-name</BucketName></Error>")
            .into_bytes()
    }

    #[test]
    fn insecure_or_credentialed_endpoints_cannot_even_be_built() {
        for e in [
            "http://s3.example.com",
            "ftp://s3.example.com",
            "https://user:pw@s3.example.com",
        ] {
            let err = S3Store::new(&cfg(e)).err().expect(e);
            assert_eq!(err, RemoteError::BadConfig, "{e}");
        }
        assert!(S3Store::new(&cfg("https://s3.example.com")).is_ok());
        assert!(S3Store::new(&cfg("http://localhost:9000")).is_ok());
        let mut aws = cfg("https://x");
        aws.endpoint = None;
        assert!(S3Store::new(&aws).is_ok());
    }

    #[test]
    fn a_put_is_signed_in_the_query_and_carries_the_precondition_header() {
        let s = stub(|_| Some((200, vec![("ETag".into(), "\"abc\"".into())], vec![])));
        let store = store_for(&s);
        let ok = store
            .put("vault/items/x.enc", b"hello", &Condition::IfNoneMatch)
            .unwrap();
        assert_eq!(ok.etag.as_deref(), Some("\"abc\""));
        store
            .put(
                "vault/items/x.enc",
                b"hello",
                &Condition::IfMatch("\"abc\"".into()),
            )
            .unwrap();
        store
            .put("vault/items/x.enc", b"hello", &Condition::None)
            .unwrap();
        let seen = s.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 3);
        for r in &seen {
            assert_eq!(r.method, "PUT");
            assert_eq!(r.body_len, 5);
            assert!(
                r.header("authorization").is_none(),
                "signature belongs in the query"
            );
            assert!(
                r.target
                    .starts_with("/secret-bucket-name/folder/vault/items/x.enc?"),
                "{}",
                r.target
            );
            assert!(r.target.contains("X-Amz-Signature="));
            assert!(r.target.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        }
        assert_eq!(seen[0].header("if-none-match"), Some("*"));
        assert!(seen[0]
            .target
            .contains("SignedHeaders=host%3Bif-none-match"));
        assert_eq!(seen[1].header("if-match"), Some("\"abc\""));
        assert!(seen[1].target.contains("SignedHeaders=host%3Bif-match"));
        assert!(seen[2].header("if-match").is_none() && seen[2].header("if-none-match").is_none());
        assert!(!seen[2].target.contains("if-"));
    }

    #[test]
    fn status_codes_and_s3_codes_map_to_stable_errors_without_leaking_anything() {
        let cases: Vec<(u16, Option<&str>, RemoteError)> = vec![
            (
                403,
                Some("SignatureDoesNotMatch"),
                RemoteError::BadSignature,
            ),
            (403, Some("InvalidAccessKeyId"), RemoteError::BadCredentials),
            (403, Some("RequestTimeTooSkewed"), RemoteError::ClockSkew),
            (403, Some("AccessDenied"), RemoteError::AccessDenied),
            (403, None, RemoteError::AccessDenied),
            (404, Some("NoSuchBucket"), RemoteError::BucketMissing),
            (
                412,
                Some("PreconditionFailed"),
                RemoteError::PreconditionFailed,
            ),
            (
                409,
                Some("ConditionalRequestConflict"),
                RemoteError::Server(409),
            ),
            (503, Some("SlowDown"), RemoteError::Server(503)),
            (501, Some("NotImplemented"), RemoteError::Unsupported),
            (400, Some("InvalidArgument"), RemoteError::Rejected(400)),
        ];
        for (status, code, want) in cases {
            let body = code.map(xml_error).unwrap_or_default();
            let s = stub(move |_| Some((status, vec![], body.clone())));
            let store = store_for(&s);
            let got = store.put("k", b"x", &Condition::None).unwrap_err();
            assert_eq!(got, want, "{status} {code:?}");
            let msg = got.message();
            for secret in [
                "secret-bucket-name",
                "AKIDSECRETKEYID",
                "SUPERSECRETVALUE",
                "127.0.0.1",
                "X-Amz",
            ] {
                assert!(!msg.contains(secret), "{msg}");
            }
        }
    }

    #[test]
    fn an_expired_signature_means_this_devices_clock_is_behind() {
        let body =
            b"<Error><Code>AccessDenied</Code><Message>Request has expired</Message></Error>"
                .to_vec();
        let s = stub(move |_| Some((403, vec![], body.clone())));
        let store = store_for(&s);
        assert_eq!(store.get("k", 100).unwrap_err(), RemoteError::ClockSkew);
        // a plain AccessDenied stays an access problem
        let s = stub(|_| Some((403, vec![], xml_error("AccessDenied"))));
        assert_eq!(
            store_for(&s).get("k", 100).unwrap_err(),
            RemoteError::AccessDenied
        );
    }

    #[test]
    fn a_redirect_is_an_error_and_is_never_followed() {
        let s = stub(|_| {
            Some((
                307,
                vec![("Location".into(), "http://127.0.0.1:1/elsewhere".into())],
                vec![],
            ))
        });
        let store = store_for(&s);
        assert_eq!(store.get("k", 100).unwrap_err(), RemoteError::Redirect);
        assert_eq!(
            store.put("k", b"x", &Condition::None).unwrap_err(),
            RemoteError::Redirect
        );
        assert_eq!(
            s.seen.lock().unwrap().len(),
            2,
            "exactly one request per call"
        );
    }

    #[test]
    fn get_head_delete_semantics() {
        let s = stub(|r| match r.method.as_str() {
            "GET" if r.target.contains("missing") => Some((404, vec![], xml_error("NoSuchKey"))),
            "GET" if r.target.contains("nobucket") => {
                Some((404, vec![], xml_error("NoSuchBucket")))
            }
            "GET" => Some((
                200,
                vec![("ETag".into(), "\"e1\"".into())],
                b"payload".to_vec(),
            )),
            "HEAD" if r.target.contains("missing") => Some((404, vec![], vec![])),
            "HEAD" => Some((
                200,
                vec![
                    ("ETag".into(), "\"e1\"".into()),
                    ("Content-Length".into(), "7".into()),
                ],
                vec![],
            )),
            "DELETE" if r.target.contains("missing") => Some((404, vec![], xml_error("NoSuchKey"))),
            "DELETE" => Some((204, vec![], vec![])),
            _ => Some((400, vec![], vec![])),
        });
        let store = store_for(&s);
        let obj = store.get("have", 100).unwrap().unwrap();
        assert_eq!(
            (obj.bytes.as_slice(), obj.etag.as_deref()),
            (&b"payload"[..], Some("\"e1\""))
        );
        assert_eq!(store.get("missing", 100).unwrap(), None);
        assert_eq!(
            store.get("nobucket", 100).unwrap_err(),
            RemoteError::BucketMissing
        );
        assert_eq!(store.head("missing").unwrap(), None);
        assert!(store.head("have").unwrap().is_some());
        store.delete("have").unwrap();
        store.delete("missing").unwrap();
    }

    #[test]
    fn responses_are_size_capped() {
        let s = stub(|_| Some((200, vec![], vec![b'x'; 5000])));
        let store = store_for(&s);
        assert_eq!(store.get("k", 4000).unwrap_err(), RemoteError::TooLarge);
        assert_eq!(store.get("k", 5000).unwrap().unwrap().bytes.len(), 5000);
    }

    #[test]
    fn network_failures_are_classified() {
        // nothing listens on this port
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let store =
            S3Store::with_limits(&cfg(&format!("http://127.0.0.1:{closed}")), fast_limits())
                .unwrap();
        assert_eq!(
            store.head("k").unwrap_err(),
            RemoteError::Unreachable(NetKind::Connect)
        );
        // accepts and never answers
        let s = stub(|_| None);
        let store = store_for(&s);
        assert_eq!(
            store.head("k").unwrap_err(),
            RemoteError::Unreachable(NetKind::Timeout)
        );
    }

    #[test]
    fn listing_follows_tokens_decodes_keys_and_strips_the_folder() {
        let page = |keys: &[&str], next: Option<&str>| {
            let mut x = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>b</Name><Prefix>folder/vault/items/</Prefix><KeyCount>2</KeyCount><MaxKeys>1000</MaxKeys><EncodingType>url</EncodingType>",
            );
            x.push_str(&format!("<IsTruncated>{}</IsTruncated>", next.is_some()));
            if let Some(n) = next {
                x.push_str(&format!(
                    "<NextContinuationToken>{n}</NextContinuationToken>"
                ));
            }
            for k in keys {
                x.push_str(&format!(
                    "<Contents><Key>{k}</Key><LastModified>2026-11-23T10:00:00.000Z</LastModified><ETag>&quot;e&quot;</ETag><Size>42</Size></Contents>"
                ));
            }
            x.push_str("</ListBucketResult>");
            x.into_bytes()
        };
        let p1 = page(
            &["folder/vault/items/a%20b.enc", "folder/vault/items/c.enc"],
            Some("TOKEN-1"),
        );
        let p2 = page(&["folder/vault/items/d.enc"], None);
        let s = stub(move |r| {
            if r.target.contains("continuation-token=TOKEN-1") {
                Some((200, vec![], p2.clone()))
            } else {
                Some((200, vec![], p1.clone()))
            }
        });
        let store = store_for(&s);
        let first = store.list("vault/items/", None, 1000).unwrap();
        assert_eq!(
            first
                .entries
                .iter()
                .map(|e| e.key.as_str())
                .collect::<Vec<_>>(),
            ["vault/items/a b.enc", "vault/items/c.enc"]
        );
        assert_eq!(first.entries[0].size, 42);
        assert_eq!(first.next_token.as_deref(), Some("TOKEN-1"));
        let second = store
            .list("vault/items/", first.next_token.as_deref(), 1000)
            .unwrap();
        assert_eq!(second.entries.len(), 1);
        assert_eq!(second.next_token, None);
        let seen = s.seen.lock().unwrap().clone();
        assert!(
            seen[0].target.contains("prefix=folder%2Fvault%2Fitems%2F"),
            "{}",
            seen[0].target
        );
        assert!(seen[0].target.contains("list-type=2"));
    }

    #[test]
    fn listed_keys_are_decoded_exactly_once() {
        // The service percent-encodes keys and writes a space as `+`: this key is really
        // `a%20b +.enc` (a literal percent sign, a space and a plus).
        let body = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>b</Name><Prefix>folder/</Prefix><KeyCount>1</KeyCount><MaxKeys>1000</MaxKeys><EncodingType>url</EncodingType><IsTruncated>false</IsTruncated><Contents><Key>folder/a%2520b+%2B.enc</Key><LastModified>2026-11-23T10:00:00.000Z</LastModified><ETag>&quot;e&quot;</ETag><Size>1</Size></Contents></ListBucketResult>".to_vec();
        let s = stub(move |_| Some((200, vec![], body.clone())));
        let page = store_for(&s).list("", None, 10).unwrap();
        assert_eq!(page.entries[0].key, "a%20b +.enc");
    }
}
