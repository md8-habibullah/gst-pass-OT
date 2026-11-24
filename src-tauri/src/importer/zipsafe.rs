//! Reading zip archives from untrusted files, in memory only.
//!
//! * A member is never extracted to disk and its name is never used as a path: members are found
//!   by name and read into memory.
//! * The number of members is checked from the end-of-central-directory record **before** the
//!   `zip` crate builds its member table (at most [`MAX_ENTRIES`]).
//! * One member may not exceed 128 MiB, all members together may not exceed 512 MiB, and a member
//!   larger than 1 MiB may not be more compressed than 200:1 (zip bomb guard); the limit is
//!   enforced on the bytes actually produced, not on the sizes the archive claims.

use super::limits::{
    unsupported, MAX_ENTRIES, MAX_ZIP_ENTRY_BYTES, MAX_ZIP_RATIO, MAX_ZIP_TOTAL_BYTES, MSG_DAMAGED,
    MSG_TOO_MANY, MSG_ZIP_UNSAFE, ZIP_RATIO_FLOOR,
};
use crate::storage::StorageError;
use std::io::{Cursor, Read};
use zeroize::Zeroizing;

pub fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Total member count according to the end-of-central-directory record (zip64 aware).
fn declared_members(bytes: &[u8]) -> Option<u64> {
    let tail_start = bytes.len().saturating_sub(22 + 65_535);
    let tail = bytes.get(tail_start..)?;
    let pos = tail.windows(4).rposition(|w| w == b"PK\x05\x06")?;
    let abs = tail_start + pos;
    let total = le16(bytes, abs + 10)?;
    if total != 0xFFFF {
        return Some(u64::from(total));
    }
    // 0xFFFF means "see the zip64 record" (or exactly that many members)
    let Some(locator_at) = abs.checked_sub(20) else {
        return Some(0xFFFF);
    };
    if bytes.get(locator_at..locator_at + 4) != Some(&b"PK\x06\x07"[..]) {
        return Some(0xFFFF);
    }
    let rec_at = usize::try_from(le64(bytes, locator_at + 8)?).ok()?;
    if bytes.get(rec_at..rec_at + 4) != Some(&b"PK\x06\x06"[..]) {
        return None;
    }
    le64(bytes, rec_at + 32)
}

/// An opened archive with the read budget of one import.
pub struct SafeZip<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
    names: Vec<String>,
    total: u64,
}

/// Member names as written, with `\` turned into `/` and a leading `./` or `/` removed.
fn tidy(name: &str) -> String {
    let n = name.replace('\\', "/");
    n.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

impl<'a> SafeZip<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, StorageError> {
        let declared = declared_members(bytes).ok_or_else(|| unsupported(MSG_DAMAGED))?;
        if declared > MAX_ENTRIES as u64 {
            return Err(unsupported(MSG_TOO_MANY));
        }
        let archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| unsupported(MSG_DAMAGED))?;
        if archive.len() > MAX_ENTRIES {
            return Err(unsupported(MSG_TOO_MANY));
        }
        let names = archive
            .file_names()
            .map(|n| n.map(|n| tidy(&n)).unwrap_or_default())
            .collect();
        Ok(Self {
            archive,
            names,
            total: 0,
        })
    }

    /// Names of all members, tidied (see [`tidy`]); index `i` is member `i`.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn find(&self, pred: impl Fn(&str) -> bool) -> Option<usize> {
        self.names.iter().position(|n| pred(n))
    }

    /// Read member `index` fully into memory, within the limits above.
    pub fn read(&mut self, index: usize) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        let mut file = self
            .archive
            .by_index(index)
            .map_err(|_| unsupported(MSG_DAMAGED))?;
        if file.is_dir() {
            return Ok(Zeroizing::new(Vec::new()));
        }
        let declared = file.size();
        let packed = file.compressed_size().max(1);
        if declared > MAX_ZIP_ENTRY_BYTES
            || (declared > ZIP_RATIO_FLOOR && declared / packed > MAX_ZIP_RATIO)
        {
            return Err(unsupported(MSG_ZIP_UNSAFE));
        }
        // What the archive may actually produce, whatever it claims.
        let bound = packed
            .saturating_mul(MAX_ZIP_RATIO)
            .clamp(ZIP_RATIO_FLOOR, MAX_ZIP_ENTRY_BYTES);
        let mut out = Zeroizing::new(Vec::with_capacity(declared.min(bound) as usize));
        let got = (&mut file)
            .take(bound + 1)
            .read_to_end(&mut out)
            .map_err(|_| unsupported(MSG_DAMAGED))?;
        if got as u64 > bound {
            return Err(unsupported(MSG_ZIP_UNSAFE));
        }
        self.total = self.total.saturating_add(got as u64);
        if self.total > MAX_ZIP_TOTAL_BYTES {
            return Err(unsupported(MSG_ZIP_UNSAFE));
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io::Write;

    /// Build a zip in memory (deflate unless `stored`).
    pub fn make_zip(members: &[(&str, &[u8])], stored: bool) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default().compression_method(if stored {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            });
            for (name, data) in members {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::make_zip;
    use super::*;

    #[test]
    fn reads_members_by_name_without_touching_the_disk() {
        let z = make_zip(
            &[
                ("export.data", b"{}"),
                ("../../etc/evil", b"x"),
                ("/abs/path", b"y"),
            ],
            false,
        );
        let mut s = SafeZip::open(&z).unwrap();
        assert_eq!(s.names(), ["export.data", "../../etc/evil", "abs/path"]);
        let i = s.find(|n| n == "export.data").unwrap();
        assert_eq!(&**s.read(i).unwrap(), b"{}");
        // traversal names are just names
        let j = s.find(|n| n.contains("evil")).unwrap();
        assert_eq!(&**s.read(j).unwrap(), b"x");
    }

    #[test]
    fn garbage_and_truncation_are_errors() {
        assert!(SafeZip::open(b"PK\x03\x04 not really").is_err());
        assert!(SafeZip::open(b"").is_err());
        let z = make_zip(&[("a", b"hello hello hello")], false);
        for cut in [1, z.len() / 2, z.len() - 1] {
            assert!(SafeZip::open(&z[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn a_compression_bomb_is_refused_before_it_is_expanded() {
        // 8 MiB of zeros deflate to a few KiB: far above 200:1.
        let big = vec![0u8; 8 * 1024 * 1024];
        let z = make_zip(&[("bomb", &big)], false);
        assert!(z.len() < 64 * 1024);
        let mut s = SafeZip::open(&z).unwrap();
        assert_eq!(s.read(0).unwrap_err().code(), "unsupported_file");
    }

    #[test]
    fn ordinary_text_compression_is_fine() {
        let text = "name,url,username,password\n".repeat(200_000);
        let z = make_zip(&[("t.csv", text.as_bytes())], false);
        let mut s = SafeZip::open(&z).unwrap();
        // ~5 MB of repetitive text may compress beyond 200:1; either outcome must be a clean one
        match s.read(0) {
            Ok(b) => assert_eq!(b.len(), text.len()),
            Err(e) => assert_eq!(e.code(), "unsupported_file"),
        }
        let moderate: String = (0..50_000)
            .map(|i| format!("row {i} {}\n", i * 7919 % 10007))
            .collect();
        let z = make_zip(&[("t.csv", moderate.as_bytes())], false);
        let mut s = SafeZip::open(&z).unwrap();
        assert_eq!(s.read(0).unwrap().len(), moderate.len());
    }

    #[test]
    fn too_many_members_are_refused_before_the_table_is_built() {
        // a minimal end-of-central-directory record that claims 0xFFFF members without a zip64 record
        let mut eocd = Vec::from(&b"PK\x05\x06"[..]);
        eocd.extend([0, 0, 0, 0]); // disk numbers
        eocd.extend(0xFFFFu16.to_le_bytes());
        eocd.extend(0xFFFFu16.to_le_bytes());
        eocd.extend([0; 4]); // central directory size
        eocd.extend([0; 4]); // offset
        eocd.extend([0, 0]); // comment length
        assert_eq!(declared_members(&eocd), Some(0xFFFF));
        // 0xFFFF is under 200,000, so the crate decides; it has no central directory to read
        assert!(SafeZip::open(&eocd).is_err());
        // a zip64 record that claims a huge count
        let mut z64 = Vec::new();
        z64.extend(b"PK\x06\x06");
        z64.extend([0u8; 28]);
        z64.extend(5_000_000u64.to_le_bytes()); // total entries at offset 32
        z64.extend([0u8; 16]);
        let rec_at = 0u64;
        z64.extend(b"PK\x06\x07");
        z64.extend([0u8; 4]);
        z64.extend(rec_at.to_le_bytes());
        z64.extend([0u8; 4]);
        z64.extend(b"PK\x05\x06");
        z64.extend([0, 0, 0, 0]);
        z64.extend(0xFFFFu16.to_le_bytes());
        z64.extend(0xFFFFu16.to_le_bytes());
        z64.extend([0u8; 10]);
        assert_eq!(declared_members(&z64), Some(5_000_000));
        let e = SafeZip::open(&z64).err().unwrap();
        assert_eq!(e.code(), "unsupported_file");
        assert!(e.to_string().contains("too many"));
    }

    #[test]
    fn encrypted_or_damaged_members_do_not_panic() {
        let z = make_zip(&[("a", b"some bytes to damage some bytes to damage")], true);
        for i in 0..z.len() {
            let mut m = z.clone();
            if let Some(b) = m.get_mut(i) {
                *b ^= 0xFF;
            }
            if let Ok(mut s) = SafeZip::open(&m) {
                let _ = s.read(0);
            }
        }
    }
}
