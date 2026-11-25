//! The universal importer and exporter (slice Q4).
//!
//! Files from other password managers are parsed **in the core**, from a path the core opened
//! itself (the webview never names a path), into [`ImportedEntry`] records that wipe themselves
//! when dropped. [`parse`] is the single entry point for the 25 formats; [`sniff`] guesses the
//! format from the file's content; [`export`] writes the plaintext Bitwarden-compatible files.
//!
//! Every parser works on untrusted bytes: all sizes are bounded ([`limits`]), errors are fixed
//! sentences that never contain file content, and no parser may panic. [`parse`] also catches a
//! panic inside a third-party crate and turns it into the same "damaged file" sentence.

pub mod authenticators;
pub mod bitwarden;
pub mod csv_import;
pub mod entry;
pub mod export;
pub mod jsonutil;
pub mod kdbx;
pub mod keepass_xml;
pub mod limits;
pub mod onepux;
pub mod otp;
pub mod proton;
pub mod sniff;
pub mod text;
pub mod vendors_json;
pub mod zipsafe;

#[cfg(test)]
mod tests;

use crate::storage::{Result, StorageError};
pub use entry::{EntryKind, ImportedAttachment, ImportedEntry};
use limits::{
    unsupported, MAX_ENTRIES, MAX_FILE_BYTES, MAX_WARNINGS, MSG_DAMAGED, MSG_TOO_LARGE,
    MSG_TOO_MANY,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroize;

/// The format ids of `src/backend/types.ts`; the JSON wire form is the same string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImportFormat {
    #[serde(rename = "keepass-kdbx")]
    KeepassKdbx,
    #[serde(rename = "keepass-xml")]
    KeepassXml,
    #[serde(rename = "bitwarden-json")]
    BitwardenJson,
    #[serde(rename = "bitwarden-csv")]
    BitwardenCsv,
    #[serde(rename = "proton-json")]
    ProtonJson,
    #[serde(rename = "proton-zip")]
    ProtonZip,
    #[serde(rename = "proton-csv")]
    ProtonCsv,
    #[serde(rename = "1password-1pux")]
    Onepassword1pux,
    #[serde(rename = "1password-csv")]
    OnepasswordCsv,
    #[serde(rename = "lastpass-csv")]
    LastpassCsv,
    #[serde(rename = "dashlane-csv")]
    DashlaneCsv,
    #[serde(rename = "dashlane-json")]
    DashlaneJson,
    #[serde(rename = "nordpass-csv")]
    NordpassCsv,
    #[serde(rename = "enpass-csv")]
    EnpassCsv,
    #[serde(rename = "enpass-json")]
    EnpassJson,
    #[serde(rename = "chrome-csv")]
    ChromeCsv,
    #[serde(rename = "firefox-csv")]
    FirefoxCsv,
    #[serde(rename = "edge-csv")]
    EdgeCsv,
    #[serde(rename = "brave-csv")]
    BraveCsv,
    #[serde(rename = "safari-csv")]
    SafariCsv,
    #[serde(rename = "aegis-json")]
    AegisJson,
    #[serde(rename = "2fas-json")]
    TwofasJson,
    #[serde(rename = "otpauth-list")]
    OtpauthList,
    #[serde(rename = "google-migration")]
    GoogleMigration,
    #[serde(rename = "custom-csv")]
    CustomCsv,
}

impl ImportFormat {
    pub const ALL: [ImportFormat; 25] = [
        Self::KeepassKdbx,
        Self::KeepassXml,
        Self::BitwardenJson,
        Self::BitwardenCsv,
        Self::ProtonJson,
        Self::ProtonZip,
        Self::ProtonCsv,
        Self::Onepassword1pux,
        Self::OnepasswordCsv,
        Self::LastpassCsv,
        Self::DashlaneCsv,
        Self::DashlaneJson,
        Self::NordpassCsv,
        Self::EnpassCsv,
        Self::EnpassJson,
        Self::ChromeCsv,
        Self::FirefoxCsv,
        Self::EdgeCsv,
        Self::BraveCsv,
        Self::SafariCsv,
        Self::AegisJson,
        Self::TwofasJson,
        Self::OtpauthList,
        Self::GoogleMigration,
        Self::CustomCsv,
    ];

    /// The wire id (same as the serde name).
    pub fn id(self) -> &'static str {
        match self {
            Self::KeepassKdbx => "keepass-kdbx",
            Self::KeepassXml => "keepass-xml",
            Self::BitwardenJson => "bitwarden-json",
            Self::BitwardenCsv => "bitwarden-csv",
            Self::ProtonJson => "proton-json",
            Self::ProtonZip => "proton-zip",
            Self::ProtonCsv => "proton-csv",
            Self::Onepassword1pux => "1password-1pux",
            Self::OnepasswordCsv => "1password-csv",
            Self::LastpassCsv => "lastpass-csv",
            Self::DashlaneCsv => "dashlane-csv",
            Self::DashlaneJson => "dashlane-json",
            Self::NordpassCsv => "nordpass-csv",
            Self::EnpassCsv => "enpass-csv",
            Self::EnpassJson => "enpass-json",
            Self::ChromeCsv => "chrome-csv",
            Self::FirefoxCsv => "firefox-csv",
            Self::EdgeCsv => "edge-csv",
            Self::BraveCsv => "brave-csv",
            Self::SafariCsv => "safari-csv",
            Self::AegisJson => "aegis-json",
            Self::TwofasJson => "2fas-json",
            Self::OtpauthList => "otpauth-list",
            Self::GoogleMigration => "google-migration",
            Self::CustomCsv => "custom-csv",
        }
    }

    /// A KeePass database is opened with a password and/or a key file.
    pub fn needs_password(self) -> bool {
        self == Self::KeepassKdbx
    }

    pub fn needs_keyfile(self) -> bool {
        self == Self::KeepassKdbx
    }

    pub fn is_csv(self) -> bool {
        matches!(
            self,
            Self::BitwardenCsv
                | Self::ProtonCsv
                | Self::OnepasswordCsv
                | Self::LastpassCsv
                | Self::DashlaneCsv
                | Self::NordpassCsv
                | Self::EnpassCsv
                | Self::ChromeCsv
                | Self::FirefoxCsv
                | Self::EdgeCsv
                | Self::BraveCsv
                | Self::SafariCsv
                | Self::CustomCsv
        )
    }
}

/// Column names (header text) chosen in the manual mapper for "another CSV".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CsvMapping {
    pub title: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub url: Option<String>,
    pub totp: Option<String>,
    pub notes: Option<String>,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    SkipDuplicates,
    AppendVersion,
    ImportAll,
    /// Entries that are the same account as one already here (the same name, user name and website, or a login and a
    /// one-time code of the same account) are merged into it: nothing either has is dropped, and every note is kept.
    Merge,
}

/// `import_preview` / `import_commit` options (`ImportOptions` in `types.ts`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    pub handle: String,
    pub format: ImportFormat,
    pub mode: ImportMode,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub keyfile_handle: Option<String>,
    #[serde(default)]
    pub mapping: Option<CsvMapping>,
}

impl Drop for ImportOptions {
    fn drop(&mut self) {
        if let Some(p) = &mut self.password {
            p.zeroize();
        }
    }
}

impl std::fmt::Debug for ImportOptions {
    /// Shows no handle and no password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportOptions")
            .field("format", &self.format)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportPick {
    pub handle: String,
    pub file_name: String,
    pub size: u64,
    /// A format id, or `"unknown"`.
    pub format: String,
    pub needs_password: bool,
    pub needs_keyfile: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyfilePick {
    pub handle: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CsvInspect {
    pub delimiter: String,
    pub headers: Vec<String>,
    pub sample: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub entries: usize,
    pub totp_seeds: usize,
    pub attachments: usize,
    pub duplicates: usize,
    pub skipped: usize,
    pub warnings: Vec<String>,
    /// Up to 50 titles of the entries, so the person can see what was read. No secrets.
    pub titles: Vec<String>,
}

/// `added`, `appended`, `skipped` and `failed` are the contract of `ImportOutcome`;
/// `warnings` is an additive field (the attachment notice and the reasons for `failed`).
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ImportOutcome {
    pub added: usize,
    pub appended: usize,
    pub skipped: usize,
    pub failed: usize,
    pub warnings: Vec<String>,
}

/// `export_plain`'s result. `warnings` is additive and absent when empty.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExportedPath {
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What an import file may need beyond its bytes.
#[derive(Default)]
pub struct ParseOptions<'a> {
    pub password: Option<&'a str>,
    pub keyfile: Option<&'a [u8]>,
    pub mapping: Option<&'a CsvMapping>,
}

/// The result of parsing one file.
#[derive(Default)]
pub struct ParsedImport {
    pub entries: Vec<ImportedEntry>,
    /// Records that were read but have nothing to import (blank rows, trashed items, ...).
    pub skipped: usize,
    /// Notes about the file as a whole (never any file content; titles are short and plain).
    pub warnings: Vec<String>,
    reasons: BTreeMap<&'static str, usize>,
    dropped_warnings: usize,
}

impl std::fmt::Debug for ParsedImport {
    /// Counts only: never an entry.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedImport")
            .field("entries", &self.entries.len())
            .field("skipped", &self.skipped)
            .field("warnings", &self.warnings.len())
            .finish_non_exhaustive()
    }
}

impl ParsedImport {
    pub fn push(&mut self, entry: ImportedEntry) -> Result<()> {
        if self.entries.len() >= MAX_ENTRIES {
            return Err(unsupported(MSG_TOO_MANY));
        }
        self.entries.push(entry);
        Ok(())
    }

    /// Count a record that is not imported; the reasons are summarised in the warnings.
    pub fn skip(&mut self, reason: &'static str) {
        self.skipped += 1;
        *self.reasons.entry(reason).or_default() += 1;
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(message.into());
        } else {
            self.dropped_warnings += 1;
        }
    }

    /// Turn the skip reasons into warnings. Called once by [`parse`].
    pub fn close(&mut self) {
        for (reason, n) in std::mem::take(&mut self.reasons) {
            let lead = if n == 1 {
                "1 record was".to_string()
            } else {
                format!("{n} records were")
            };
            self.warn(format!("{lead} not imported: {reason}."));
        }
        if self.dropped_warnings > 0 {
            let n = std::mem::take(&mut self.dropped_warnings);
            self.warnings.push(format!("{n} more notes are not shown."));
        }
    }

    /// Total number of one-time code secrets and attachments.
    pub fn seed_count(&self) -> usize {
        self.entries.iter().map(ImportedEntry::seed_count).sum()
    }

    /// Number of one-time codes that will work after the import (what the preview promises).
    pub fn usable_seed_count(&self) -> usize {
        self.entries
            .iter()
            .map(ImportedEntry::usable_seed_count)
            .sum()
    }

    pub fn attachment_count(&self) -> usize {
        self.entries.iter().map(|e| e.attachments.len()).sum()
    }
}

/// Where parsed attachments go. `AppCore::import_commit` uses the encrypted drive; [`NoSink`] is for
/// callers with no drive, and the commit then reports the files as not imported.
///
/// `add` is called after the items are saved, with the id of the saved item, and **without** any
/// vault lock held, so an implementation may call back into the app.
pub trait AttachmentSink: Send + Sync {
    /// Is there anywhere to put attachments right now?
    fn available(&self) -> bool {
        true
    }

    fn add(&self, item_id: &str, name: &str, bytes: &[u8]) -> Result<()>;
}

/// A sink with no drive: attachments are counted and reported, not stored.
pub struct NoSink;

impl AttachmentSink for NoSink {
    fn available(&self) -> bool {
        false
    }

    fn add(&self, _item_id: &str, _name: &str, _bytes: &[u8]) -> Result<()> {
        Err(StorageError::invalid(
            "The encrypted drive is not available yet.",
        ))
    }
}

/// Parse a whole file in memory. Never panics: a panic inside a parsing crate becomes the
/// "damaged file" sentence.
pub fn parse(format: ImportFormat, bytes: &[u8], opts: &ParseOptions<'_>) -> Result<ParsedImport> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(unsupported(MSG_TOO_LARGE));
    }
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parse_inner(format, bytes, opts)
    }));
    match run {
        Ok(result) => {
            let mut parsed = result?;
            parsed.close();
            Ok(parsed)
        }
        Err(_) => Err(unsupported(MSG_DAMAGED)),
    }
}

fn parse_inner(
    format: ImportFormat,
    bytes: &[u8],
    opts: &ParseOptions<'_>,
) -> Result<ParsedImport> {
    use ImportFormat as F;
    match format {
        F::KeepassKdbx => kdbx::parse(bytes, opts),
        F::KeepassXml => keepass_xml::parse(bytes),
        F::BitwardenJson => bitwarden::parse_json(bytes),
        F::ProtonJson => proton::parse_json(bytes),
        F::ProtonZip => proton::parse_zip(bytes),
        F::Onepassword1pux => onepux::parse(bytes),
        F::DashlaneJson => vendors_json::parse_dashlane(bytes),
        F::EnpassJson => vendors_json::parse_enpass(bytes),
        F::AegisJson => authenticators::parse_aegis(bytes),
        F::TwofasJson => authenticators::parse_2fas(bytes),
        F::OtpauthList => authenticators::parse_list(bytes),
        F::GoogleMigration => authenticators::parse_migration(bytes),
        F::BitwardenCsv
        | F::ProtonCsv
        | F::OnepasswordCsv
        | F::LastpassCsv
        | F::DashlaneCsv
        | F::NordpassCsv
        | F::EnpassCsv
        | F::ChromeCsv
        | F::FirefoxCsv
        | F::EdgeCsv
        | F::BraveCsv
        | F::SafariCsv
        | F::CustomCsv => csv_import::parse(format, bytes, opts.mapping),
    }
}
