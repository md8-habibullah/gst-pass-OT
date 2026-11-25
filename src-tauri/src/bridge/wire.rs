//! What the browser extension and the app say to each other, one JSON object per message.
//!
//! The extension sends a request, the app answers it, and both are checked here before anything else looks at them: the
//! app never trusts the other end of its socket (any program of the same user can open it), so every field is bounded and
//! every origin must be one a browser could really have.
//!
//! Request:  `{"v":1,"id":"<8-64 chars of A-Z a-z 0-9 _ ->","op":"get"|"create"|"status","origin":"https://example.com",
//!            "request":"<the WebAuthn options as JSON text>","timeoutMs":60000}`
//! Answer:   `{"v":1,"id":…,"ok":true,"responseJson":"<the WebAuthn credential as JSON text>"}`, or
//!           `{"v":1,"id":…,"ok":false,"code":"fallback","reason":"disabled|app_locked|no_match|declined|busy"}` (the extension
//!           hands the call to the browser's own passkey flow), `"code":"cancelled"`, `"code":"timeout"`, or
//!           `"code":"error","errorCode":…,"message":…`. A `status` request is answered
//!           `{"v":1,"id":…,"ok":true,"enabled":true}`.

use serde::Deserialize;
use serde_json::json;

/// The most a request may weigh (the WebAuthn options of a real site are a few hundred bytes).
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// The longest line either end may send.
pub const MAX_LINE_BYTES: usize = 128 * 1024;
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const MIN_TIMEOUT_MS: u64 = 5_000;
pub const MAX_TIMEOUT_MS: u64 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Get,
    Create,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Get => "get",
            Kind::Create => "create",
        }
    }
}

/// A request that passed the checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    Status {
        id: String,
    },
    Ask {
        id: String,
        kind: Kind,
        origin: String,
        request: String,
        timeout_ms: u64,
    },
}

impl Checked {
    pub fn id(&self) -> &str {
        match self {
            Checked::Status { id } | Checked::Ask { id, .. } => id,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    v: Option<u64>,
    id: Option<String>,
    op: Option<String>,
    origin: Option<String>,
    request: Option<String>,
    timeout_ms: Option<u64>,
}

/// Why a request was refused before anything was done with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// The id, when it could be read, so the answer can carry it.
    pub id: Option<String>,
    pub message: &'static str,
}

fn reject(id: Option<String>, message: &'static str) -> Rejected {
    Rejected { id, message }
}

fn id_ok(id: &str) -> bool {
    (8..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `https://host[:port]` (or `http://localhost[:port]`) and nothing more, lower-cased. A browser's origin never has a
/// path, a user name, a query or a space, so anything that has one did not come from a browser.
pub fn clean_origin(origin: &str) -> Option<String> {
    let (scheme, rest) = origin.split_once("://")?;
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    let host = host.to_ascii_lowercase();
    if !crate::passkey::origin::plain_host(&host) || host.starts_with('.') || host.ends_with('.') {
        return None;
    }
    let secure = scheme == "https" || (scheme == "http" && host == "localhost");
    if !secure {
        return None;
    }
    if let Some(p) = port {
        let digits_only = !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit());
        if !digits_only || p.parse::<u32>().map_or(true, |n| n == 0 || n > 65_535) {
            return None;
        }
    }
    Some(match port {
        Some(p) => format!("{scheme}://{host}:{p}"),
        None => format!("{scheme}://{host}"),
    })
}

/// Read and check one request line.
pub fn check(line: &str) -> Result<Checked, Rejected> {
    if line.len() > MAX_LINE_BYTES {
        return Err(reject(None, "That message is too long."));
    }
    let raw: Raw =
        serde_json::from_str(line).map_err(|_| reject(None, "That message is not understood."))?;
    let id = raw.id.filter(|i| id_ok(i));
    let Some(id) = id else {
        return Err(reject(None, "That message has no valid id."));
    };
    if raw.v != Some(1) {
        return Err(reject(Some(id), "That message version is not supported."));
    }
    match raw.op.as_deref() {
        Some("status") => Ok(Checked::Status { id }),
        Some(op @ ("get" | "create")) => {
            let Some(origin) = raw.origin.as_deref().and_then(clean_origin) else {
                return Err(reject(Some(id), "That page address is not valid."));
            };
            let request = raw.request.unwrap_or_default();
            if request.is_empty() || request.len() > MAX_REQUEST_BYTES {
                return Err(reject(
                    Some(id),
                    "That passkey request is empty or too long.",
                ));
            }
            let timeout_ms = raw
                .timeout_ms
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
            Ok(Checked::Ask {
                id,
                kind: if op == "get" { Kind::Get } else { Kind::Create },
                origin,
                request,
                timeout_ms,
            })
        }
        _ => Err(reject(Some(id), "That operation is not known.")),
    }
}

// ------------------------------------------------------------------ answers

fn base(id: &str) -> serde_json::Value {
    json!({ "v": 1, "id": id })
}

fn finish(mut v: serde_json::Value, extra: serde_json::Value) -> String {
    if let (Some(a), Some(b)) = (v.as_object_mut(), extra.as_object()) {
        a.extend(b.clone());
    }
    v.to_string()
}

pub fn answer_ok(id: &str, response_json: &str) -> String {
    finish(base(id), json!({"ok": true, "responseJson": response_json}))
}

/// Any web page can ask this, so it says no more than that the switch is on (not whether the vault is locked).
pub fn answer_status(id: &str) -> String {
    finish(base(id), json!({"ok": true, "enabled": true}))
}

/// The extension should hand this call to the browser's own passkey flow.
pub fn answer_fallback(id: &str, reason: &str) -> String {
    finish(
        base(id),
        json!({"ok": false, "code": "fallback", "reason": reason}),
    )
}

pub fn answer_cancelled(id: &str) -> String {
    finish(base(id), json!({"ok": false, "code": "cancelled"}))
}

pub fn answer_timeout(id: &str) -> String {
    finish(base(id), json!({"ok": false, "code": "timeout"}))
}

pub fn answer_error(id: Option<&str>, error_code: &str, message: &str) -> String {
    finish(
        base(id.unwrap_or("")),
        json!({"ok": false, "code": "error", "errorCode": error_code, "message": message}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(extra: &str) -> String {
        format!(
            r#"{{"v":1,"id":"abcdefgh12","op":"get","origin":"https://example.com","request":"{{}}"{extra}}}"#
        )
    }

    #[test]
    fn a_good_request_is_read_and_its_timeout_is_kept_in_bounds() {
        let Checked::Ask {
            kind,
            origin,
            timeout_ms,
            ..
        } = check(&line("")).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (kind, origin.as_str(), timeout_ms),
            (Kind::Get, "https://example.com", DEFAULT_TIMEOUT_MS)
        );
        for (asked, got) in [
            (1, MIN_TIMEOUT_MS),
            (30_000, 30_000),
            (9_999_999, MAX_TIMEOUT_MS),
        ] {
            let Checked::Ask { timeout_ms, .. } =
                check(&line(&format!(r#","timeoutMs":{asked}"#))).unwrap()
            else {
                panic!()
            };
            assert_eq!(timeout_ms, got);
        }
        assert_eq!(
            check(r#"{"v":1,"id":"abcdefgh12","op":"status"}"#).unwrap(),
            Checked::Status {
                id: "abcdefgh12".into()
            }
        );
    }

    #[test]
    fn origins_that_no_browser_would_send_are_refused() {
        for bad in [
            "http://example.com",
            "https://example.com/path",
            "https://user@example.com",
            "https://exa mple.com",
            "https://evil.com\\.example.com",
            "https://example.com:0",
            "https://example.com:70000",
            "https://example.com:",
            "https://.example.com",
            "ftp://example.com",
            "example.com",
            "https://",
            "https://[::1]",
            "javascript:alert(1)",
        ] {
            assert_eq!(clean_origin(bad), None, "{bad}");
        }
        assert_eq!(
            clean_origin("HTTPS://Example.COM:8443").as_deref(),
            None,
            "the scheme is lower-case in an origin"
        );
        assert_eq!(
            clean_origin("https://Example.COM:8443").as_deref(),
            Some("https://example.com:8443")
        );
        assert_eq!(
            clean_origin("http://localhost:3000").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(
            clean_origin("http://localhost").as_deref(),
            Some("http://localhost")
        );
    }

    #[test]
    fn bad_requests_name_what_is_wrong_and_keep_a_usable_id() {
        assert!(check("not json").is_err());
        assert!(check("{}").unwrap_err().id.is_none());
        assert!(check(r#"{"v":1,"id":"short","op":"get"}"#)
            .unwrap_err()
            .id
            .is_none());
        assert_eq!(
            check(r#"{"v":2,"id":"abcdefgh12","op":"get"}"#)
                .unwrap_err()
                .id
                .as_deref(),
            Some("abcdefgh12")
        );
        assert_eq!(
            check(r#"{"v":1,"id":"abcdefgh12","op":"delete"}"#)
                .unwrap_err()
                .id
                .as_deref(),
            Some("abcdefgh12")
        );
        assert!(
            check(r#"{"v":1,"id":"abcdefgh12","op":"get","origin":"https://example.com"}"#)
                .is_err()
        );
        let huge = format!(
            r#"{{"v":1,"id":"abcdefgh12","op":"create","origin":"https://example.com","request":"{}"}}"#,
            "x".repeat(MAX_REQUEST_BYTES + 1)
        );
        assert!(check(&huge).is_err());
        assert!(check(&"x".repeat(MAX_LINE_BYTES + 1)).is_err());
        assert!(check(r#"{"v":1,"id":"abcdefgh12!","op":"status"}"#).is_err());
    }

    #[test]
    fn every_answer_is_one_line_with_the_id() {
        for a in [
            answer_ok("abcdefgh12", "{\"a\":\"line\\nbreak\"}"),
            answer_status("abcdefgh12"),
            answer_fallback("abcdefgh12", "no_match"),
            answer_cancelled("abcdefgh12"),
            answer_timeout("abcdefgh12"),
            answer_error(Some("abcdefgh12"), "invalid_input", "no\nway"),
        ] {
            assert!(!a.contains('\n'), "{a}");
            let v: serde_json::Value = serde_json::from_str(&a).unwrap();
            assert_eq!(v["v"], 1);
            assert_eq!(v["id"], "abcdefgh12");
        }
    }
}
