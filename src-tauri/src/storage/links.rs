//! Opening a web link in the system browser (`open_url`, `IPC_CONTRACT.md` §5.4).
//!
//! The webview is not allowed to open anything itself: the only door is the `open_url` command,
//! which validates the address here and then calls an [`UrlOpener`]. In the app the opener is
//! `tauri-plugin-opener`, called from Rust only; the webview holds no `opener:*` permission.

use super::error::{Result, StorageError};
use std::sync::Mutex;
use url::Url;

/// Longest address that is passed on.
pub const MAX_URL_CHARS: usize = 2048;

/// Check an address from the webview and return it in canonical form.
///
/// Only `http` and `https`; a host is required; no credentials (`user:pass@`), no whitespace or
/// control characters (a pasted newline or a tab must not smuggle in a second argument), no
/// backslashes (browsers treat them as slashes). `javascript:`, `file:`, `data:`, `mailto:` and
/// every other scheme are refused.
pub fn validate_open_url(raw: &str) -> Result<String> {
    let bad = || StorageError::invalid("That link cannot be opened.");
    if raw.is_empty()
        || raw.chars().count() > MAX_URL_CHARS
        || raw
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Err(bad());
    }
    let url = Url::parse(raw).map_err(|_| bad())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(StorageError::invalid(
            "Only web links (http or https) can be opened.",
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(bad());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(StorageError::invalid(
            "Links that contain a user name or password cannot be opened.",
        ));
    }
    Ok(url.into())
}

/// Something that can show a web address to the person.
pub trait UrlOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<()>;
}

/// The default when nothing was installed: refuses.
pub struct NoOpener;

impl UrlOpener for NoOpener {
    fn open(&self, _url: &str) -> Result<()> {
        Err(StorageError::Storage(
            "Opening links is not available here.".into(),
        ))
    }
}

/// Remembers what it was asked to open and opens nothing (tests and the browser e2e shim).
#[derive(Default)]
pub struct RecordingOpener {
    urls: Mutex<Vec<String>>,
}

impl RecordingOpener {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn urls(&self) -> Vec<String> {
        self.urls.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn clear(&self) {
        self.urls.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
}

impl UrlOpener for RecordingOpener {
    fn open(&self, url: &str) -> Result<()> {
        self.urls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(url.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_links_pass_and_are_canonical() {
        assert_eq!(
            validate_open_url("https://example.com").unwrap(),
            "https://example.com/"
        );
        assert_eq!(
            validate_open_url("http://localhost:9000/a?b=c#d").unwrap(),
            "http://localhost:9000/a?b=c#d"
        );
        assert_eq!(
            validate_open_url("HTTPS://Example.COM/Path").unwrap(),
            "https://example.com/Path"
        );
        // Extra slashes are what a browser would read as `https://path/`; the opener is handed
        // that canonical form, so what was checked is exactly what is opened.
        assert_eq!(validate_open_url("https:///path").unwrap(), "https://path/");
    }

    #[test]
    fn everything_else_is_refused() {
        for bad in [
            "",
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "file:///etc/passwd",
            "data:text/html;base64,PHNjcmlwdD4=",
            "mailto:a@b.c",
            "ftp://example.com/",
            "vbscript:x",
            "tauri://localhost",
            "//example.com",
            "example.com",
            "https://",
            "https://user:pw@example.com/",
            "https://user@example.com/",
            "https://exa mple.com/",
            "https://example.com/\nhttps://evil.example",
            "https://example.com/\t",
            "https://example.com/\u{0}",
            "https://example.com\\@evil.example",
            " https://example.com",
            "https://example.com ",
        ] {
            assert!(validate_open_url(bad).is_err(), "{bad:?} must be refused");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_CHARS));
        assert!(validate_open_url(&long).is_err());
    }

    #[test]
    fn recording_opener_records_and_no_opener_refuses() {
        let r = RecordingOpener::new();
        r.open("https://example.com/").unwrap();
        assert_eq!(r.urls(), ["https://example.com/"]);
        assert!(NoOpener.open("https://example.com/").is_err());
    }
}
