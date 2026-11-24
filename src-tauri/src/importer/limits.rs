//! Hard limits for everything that is read from an untrusted import file, and the fixed
//! sentences used for errors. No message here ever contains file content.

use crate::storage::StorageError;

/// Largest file the importer opens.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Largest key file accepted for a KeePass database.
pub const MAX_KEYFILE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest single archive member after decompression.
pub const MAX_ZIP_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
/// Largest sum of all archive members that are read, after decompression.
pub const MAX_ZIP_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// A member bigger than this may not be more compressed than [`MAX_ZIP_RATIO`] to 1.
pub const ZIP_RATIO_FLOOR: u64 = 1024 * 1024;
pub const MAX_ZIP_RATIO: u64 = 200;
/// Most records (rows, items, accounts, archive members) any import may hold.
pub const MAX_ENTRIES: usize = 200_000;
/// One text value is cut to this many bytes while parsing (Passman itself accepts 64 KiB, so a
/// longer value is later reported as "not importable" instead of silently losing its tail).
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;
/// Most custom fields kept per parsed record (Passman stores 50).
pub const MAX_PARSED_FIELDS: usize = 200;
/// Attachments larger than the encrypted drive accepts (52,428,800 bytes) are not kept.
pub const MAX_ATTACHMENT_BYTES: usize = 52_428_800;
/// Nesting limit for XML, KeePass groups and zip-in-zip style structures.
pub const MAX_DEPTH: usize = 128;
/// The CSV header inspection shows at most this many characters of one cell.
pub const SAMPLE_CELL_CHARS: usize = 80;
pub const SAMPLE_ROWS: usize = 5;
pub const MAX_CSV_COLUMNS: usize = 1000;
/// Most warnings returned to the webview.
pub const MAX_WARNINGS: usize = 100;
/// A title shown in a warning is cut to this many characters.
pub const WARN_TITLE_CHARS: usize = 60;
/// A KeePass key derivation may use at most this much memory (bytes) and work.
pub const MAX_KDF_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_KDF_ITERATIONS: u64 = 1 << 20;
pub const MAX_KDF_LANES: u32 = 256;
pub const MAX_KDF_WORK: u64 = 64 * 1024 * 1024; // KiB * iterations, i.e. 64 GiB of passes
pub const MAX_AES_KDF_ROUNDS: u64 = 1 << 30;

pub const MSG_TOO_LARGE: &str = "That file is too large to import (256 MB at most).";
pub const MSG_KEYFILE_TOO_LARGE: &str = "That key file is too large (64 MB at most).";
pub const MSG_NOT_A_FILE: &str = "Choose a file, not a folder.";
pub const MSG_UNREADABLE: &str = "That file could not be read. Check that it still exists.";
pub const MSG_DAMAGED: &str = "That file is damaged or incomplete, so it cannot be imported.";
pub const MSG_WRONG_FORMAT: &str =
    "That file is not in the format you chose. Check the format and try again.";
pub const MSG_UNKNOWN: &str = "That file is not in a format Passman can import.";
pub const MSG_TOO_MANY: &str = "That file has too many entries to import (200,000 at most).";
pub const MSG_ENCRYPTED_EXPORT: &str = "This export is encrypted. Export it unencrypted from the app, import it, then delete the file.";
pub const MSG_TOO_DEEP: &str =
    "That file is nested too deeply to be a valid export, so it was not read.";
pub const MSG_NOT_TEXT: &str = "That file is not a text file, so it cannot be read as a table.";
pub const MSG_UNCLOSED_QUOTE: &str = "That CSV has an unclosed quote, so it may be cut off.";
pub const MSG_NO_ROWS: &str = "That file has no rows to import.";
pub const MSG_NO_COLUMNS: &str =
    "That table does not have the columns this format needs (for example a password column).";
pub const MSG_KDBX_KEY: &str = "That password or key file does not open this KeePass database.";
pub const MSG_KDBX_NEEDS_PASSWORD: &str = "Enter the password of the KeePass database.";
pub const MSG_KDF_TOO_HEAVY: &str =
    "This KeePass database asks for more memory or time to open than this device allows.";
pub const MSG_ZIP_UNSAFE: &str =
    "That archive is not safe to open (it is too large or too compressed), so it was not read.";
pub const MSG_XML_DTD: &str = "That XML file contains a document type declaration, which is not allowed for safety, so it was not read.";
pub const MSG_XML_PROTECTED: &str = "That XML file holds values that are still encrypted. Export it again with KeePass XML (2.x) from KeePass.";
pub const MSG_KDBX_OLD: &str = "That KeePass file is too old (KeePass 1.x). Open it in KeePass or KeePassXC and save it as a KDBX file first.";
pub const MSG_MAPPING: &str = "Choose a column for the name and for the password or the one-time code, and use only columns that exist in the file.";

pub fn unsupported(msg: &'static str) -> StorageError {
    StorageError::UnsupportedFile(msg.into())
}

pub fn invalid(msg: &'static str) -> StorageError {
    StorageError::InvalidInput(msg.into())
}

/// Cut `s` to at most `max` bytes at a character boundary.
pub fn cap_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// Cut `s` to at most `max` characters.
pub fn cap_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// An owned copy of `s` cut to the per-value parse limit.
pub fn own_capped(s: &str) -> String {
    cap_bytes(s, MAX_VALUE_BYTES).to_string()
}

/// A title for use inside a warning: single line, short, no control characters.
pub fn warn_title(title: &str) -> String {
    let t: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(WARN_TITLE_CHARS)
        .collect();
    if t.trim().is_empty() {
        "(no name)".to_string()
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_respect_character_boundaries() {
        assert_eq!(cap_bytes("héllo", 2), "h");
        assert_eq!(cap_bytes("héllo", 3), "hé");
        assert_eq!(cap_bytes("abc", 10), "abc");
        assert_eq!(cap_chars("héllo", 2), "hé");
        assert_eq!(warn_title("a\nb"), "ab");
        assert_eq!(warn_title("  "), "(no name)");
    }
}
