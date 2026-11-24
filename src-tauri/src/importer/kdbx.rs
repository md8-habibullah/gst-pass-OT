//! KeePass databases (`.kdbx`, versions 3.x and 4.x) opened in memory with a password and/or a
//! key file, using the pure-Rust `keepass` crate.
//!
//! The crate does not bound the key-derivation cost a file asks for (a crafted file could demand
//! terabytes of memory), so the header is read first and a file asking for more than
//! [`MAX_KDF_MEMORY_BYTES`] of memory, more work than [`MAX_KDF_WORK`], or an absurd lane or
//! round count is refused before any key derivation starts.

use super::keepass_xml::{add_entry, KpEntry, KpField};
use super::limits::{
    invalid, unsupported, MAX_AES_KDF_ROUNDS, MAX_ATTACHMENT_BYTES, MAX_DEPTH, MAX_KDF_ITERATIONS,
    MAX_KDF_LANES, MAX_KDF_MEMORY_BYTES, MAX_KDF_WORK, MSG_DAMAGED, MSG_KDBX_KEY,
    MSG_KDBX_NEEDS_PASSWORD, MSG_KDBX_OLD, MSG_KDF_TOO_HEAVY, MSG_NO_ROWS, MSG_WRONG_FORMAT,
};
use super::{ImportedAttachment, ParseOptions, ParsedImport};
use crate::storage::Result;
use keepass::db::DatabaseOpenError;
use keepass::{Database, DatabaseKey};

/// First 8 bytes of every KDBX 2/3/4 file.
pub const MAGIC: [u8; 8] = [0x03, 0xD9, 0xA2, 0x9A, 0x67, 0xFB, 0x4B, 0xB5];
/// KeePass 1.x (`.kdb`) uses another second signature.
const MAGIC_KDB: [u8; 8] = [0x03, 0xD9, 0xA2, 0x9A, 0x65, 0xFB, 0x4B, 0xB5];

const UUID_AES_KDF: [u8; 16] = [
    0xC9, 0xD9, 0xF3, 0x9A, 0x62, 0x8A, 0x44, 0x60, 0xBF, 0x74, 0x0D, 0x08, 0xC1, 0x8A, 0x4F, 0xEA,
];

pub fn is_kdbx(bytes: &[u8]) -> bool {
    bytes.starts_with(&MAGIC) || bytes.starts_with(&MAGIC_KDB)
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let out = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

/// What the header says key derivation will cost.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct KdfCost {
    pub aes_rounds: u64,
    pub memory_bytes: u64,
    pub iterations: u64,
    pub lanes: u32,
    pub known: bool,
}

fn variant_value(cur: &mut Cursor<'_>) -> Option<Vec<(String, u8, Vec<u8>)>> {
    let _version = cur.u16()?;
    let mut out = Vec::new();
    loop {
        let kind = cur.u8()?;
        if kind == 0 {
            return Some(out);
        }
        let klen = usize::try_from(cur.u32()?).ok()?;
        let key = String::from_utf8_lossy(cur.take(klen)?).into_owned();
        let vlen = usize::try_from(cur.u32()?).ok()?;
        let value = cur.take(vlen)?.to_vec();
        out.push((key, kind, value));
        if out.len() > 256 {
            return None;
        }
    }
}

fn le_u64(v: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(v.get(..8)?.try_into().ok()?))
}

fn le_u32(v: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(v.get(..4)?.try_into().ok()?))
}

/// Read the key-derivation settings from the outer header. `None` means the header is not a
/// readable KDBX 3/4 header.
pub fn kdf_cost(bytes: &[u8]) -> Option<KdfCost> {
    let mut cur = Cursor { buf: bytes, pos: 8 };
    let _minor = cur.u16()?;
    let major = cur.u16()?;
    let mut cost = KdfCost::default();
    match major {
        3 => loop {
            let id = cur.u8()?;
            let len = usize::from(cur.u16()?);
            let data = cur.take(len)?;
            match id {
                0 => break,
                6 => {
                    cost.aes_rounds = le_u64(data)?;
                    cost.known = true;
                }
                _ => {}
            }
        },
        4 => loop {
            let id = cur.u8()?;
            let len = usize::try_from(cur.u32()?).ok()?;
            let data = cur.take(len)?;
            match id {
                0 => break,
                11 => {
                    let mut inner = Cursor { buf: data, pos: 0 };
                    let dict = variant_value(&mut inner)?;
                    cost.known = true;
                    let uuid = dict
                        .iter()
                        .find(|(k, _, _)| k == "$UUID")
                        .map(|(_, _, v)| v.as_slice());
                    if uuid == Some(&UUID_AES_KDF[..]) {
                        cost.aes_rounds = dict
                            .iter()
                            .find(|(k, _, _)| k == "R")
                            .and_then(|(_, _, v)| le_u64(v))?;
                    } else {
                        cost.memory_bytes = dict
                            .iter()
                            .find(|(k, _, _)| k == "M")
                            .and_then(|(_, _, v)| le_u64(v))?;
                        cost.iterations = dict
                            .iter()
                            .find(|(k, _, _)| k == "I")
                            .and_then(|(_, _, v)| le_u64(v))?;
                        cost.lanes = dict
                            .iter()
                            .find(|(k, _, _)| k == "P")
                            .and_then(|(_, _, v)| le_u32(v))?;
                    }
                }
                _ => {}
            }
        },
        _ => return None,
    }
    cost.known.then_some(cost)
}

/// Refuse a file whose key derivation would cost more than this app allows.
pub fn check_kdf(bytes: &[u8]) -> Result<()> {
    let cost = kdf_cost(bytes).ok_or_else(|| unsupported(MSG_DAMAGED))?;
    let heavy = cost.aes_rounds > MAX_AES_KDF_ROUNDS
        || cost.memory_bytes > MAX_KDF_MEMORY_BYTES
        || cost.iterations > MAX_KDF_ITERATIONS
        || cost.lanes > MAX_KDF_LANES
        || (cost.memory_bytes / 1024).saturating_mul(cost.iterations) > MAX_KDF_WORK;
    if heavy {
        return Err(unsupported(MSG_KDF_TOO_HEAVY));
    }
    Ok(())
}

pub fn parse(bytes: &[u8], opts: &ParseOptions<'_>) -> Result<ParsedImport> {
    if !is_kdbx(bytes) {
        return Err(unsupported(MSG_WRONG_FORMAT));
    }
    if bytes.starts_with(&MAGIC_KDB) {
        return Err(unsupported(MSG_KDBX_OLD));
    }
    let password = opts.password.filter(|p| !p.is_empty());
    if password.is_none() && opts.keyfile.is_none() {
        return Err(invalid(MSG_KDBX_NEEDS_PASSWORD));
    }
    check_kdf(bytes)?;
    let mut key = DatabaseKey::new();
    if let Some(p) = password {
        key = key.with_password(p);
    }
    if let Some(mut kf) = opts.keyfile {
        key = key
            .with_keyfile(&mut kf)
            .map_err(|_| unsupported(MSG_DAMAGED))?;
    }
    let db = match Database::parse(bytes, key) {
        Ok(db) => db,
        Err(
            DatabaseOpenError::Io(_)
            | DatabaseOpenError::UnexpectedEof
            | DatabaseOpenError::VersionParse(_)
            | DatabaseOpenError::UnsupportedVersion,
        ) => return Err(unsupported(MSG_DAMAGED)),
        Err(_) => return Err(invalid(MSG_KDBX_KEY)),
    };
    convert(&db)
}

fn convert(db: &Database) -> Result<ParsedImport> {
    let mut out = ParsedImport::default();
    let recycle = db.meta.recyclebin_uuid;
    let mut too_big = 0usize;
    // Iterative walk, in file order: (group, names of the groups above it, below the root).
    let mut stack = vec![(db.root().id(), Vec::<String>::new())];
    let mut guard = 0usize;
    while let Some((group_id, path)) = stack.pop() {
        guard += 1;
        if guard > super::limits::MAX_ENTRIES + 1 {
            return Err(unsupported(MSG_DAMAGED));
        }
        let Some(group) = db.group(group_id) else {
            continue;
        };
        if recycle.is_some_and(|r| r == group.id().uuid()) {
            let n = group.entries().count();
            for _ in 0..n {
                out.skip("it was in the KeePass Recycle Bin");
            }
            continue;
        }
        for entry in group.entries() {
            let mut kp = KpEntry::default();
            kp.id = entry.id().uuid().to_string();
            kp.path = path.clone();
            kp.tags = entry.tags.clone();
            let mut keys: Vec<&String> = entry.fields.keys().collect();
            keys.sort();
            for k in keys {
                if let Some(v) = entry.fields.get(k) {
                    kp.fields.push(KpField {
                        key: k.clone(),
                        value: super::limits::own_capped(v.get()),
                        protected: v.is_protected(),
                    });
                }
            }
            kp.created = entry
                .times
                .creation
                .and_then(|t| u64::try_from(t.and_utc().timestamp_millis()).ok());
            kp.modified = entry
                .times
                .last_modification
                .and_then(|t| u64::try_from(t.and_utc().timestamp_millis()).ok());
            let mut names: Vec<(&str, _)> = entry.attachments_named().collect();
            names.sort_by(|a, b| a.0.cmp(b.0));
            for (name, att) in names {
                let data = att.data.get();
                if data.len() > MAX_ATTACHMENT_BYTES {
                    too_big += 1;
                } else {
                    kp.attachments.push(ImportedAttachment {
                        name: name.to_string(),
                        bytes: data.clone(),
                    });
                }
            }
            add_entry(&mut out, &kp)?;
        }
        let mut children: Vec<_> = group.group_ids().collect();
        children.reverse(); // the stack pops the first child first
        for child_id in children {
            let Some(child) = db.group(child_id) else {
                continue;
            };
            let mut p = path.clone();
            if p.len() >= MAX_DEPTH {
                return Err(unsupported(MSG_DAMAGED));
            }
            p.push(child.name.clone());
            stack.push((child_id, p));
        }
    }
    if too_big > 0 {
        out.warn(format!(
            "{too_big} attached files were too large and were not read."
        ));
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

/// Real `.kdbx` files for tests, written with the same crate (see `tests/fixtures/import`).
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use keepass::db::{fields, Value};

    pub fn sample_db() -> Database {
        let mut db = Database::new();
        let mut root = db.root_mut();
        root.add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Root entry");
            e.set_unprotected(fields::USERNAME, "rootuser");
            e.set_protected(fields::PASSWORD, "rootpw");
            e.set_unprotected(fields::URL, "https://root.example");
            e.set_unprotected(fields::NOTES, "a note");
            e.set_protected(
                fields::OTP,
                "otpauth://totp/Root:me?secret=JBSWY3DPEHPK3PXP&issuer=Root",
            );
            e.set_protected("Recovery code", "abc-123");
            e.tags.push("one".into());
            e.add_attachment("hello.txt", Value::unprotected(b"Hello!".to_vec()));
        });
        root.add_group()
            .edit(|g| g.name = "Work".into())
            .add_group()
            .edit(|g| g.name = "Servers".into())
            .add_entry()
            .edit(|e| {
                e.set_unprotected(fields::TITLE, "Deep");
                e.set_protected(fields::PASSWORD, "deeppw");
            });
        db
    }

    pub fn save(db: &Database, password: Option<&str>, keyfile: Option<&[u8]>) -> Vec<u8> {
        let mut key = DatabaseKey::new();
        if let Some(p) = password {
            key = key.with_password(p);
        }
        if let Some(mut k) = keyfile {
            key = key.with_keyfile(&mut k).unwrap();
        }
        let mut out = Vec::new();
        db.save(&mut out, key).unwrap();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{sample_db, save};
    use super::*;

    #[test]
    fn reads_a_password_protected_database() {
        let file = save(&sample_db(), Some("correct horse"), None);
        assert!(is_kdbx(&file));
        let p = parse(
            &file,
            &ParseOptions {
                password: Some("correct horse"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.entries.len(), 2);
        let titles: Vec<&str> = p.entries.iter().map(|e| e.title.as_str()).collect();
        assert!(titles.contains(&"Root entry") && titles.contains(&"Deep"));
        let root = p.entries.iter().find(|e| e.title == "Root entry").unwrap();
        assert_eq!(
            (root.username.as_str(), root.password.as_str()),
            ("rootuser", "rootpw")
        );
        assert_eq!(root.urls, vec!["https://root.example"]);
        assert_eq!(root.notes, "a note");
        assert_eq!(root.totp.len(), 1);
        assert_eq!(root.tags, vec!["one"]);
        assert_eq!(root.custom_fields.len(), 1);
        assert!(root.custom_fields[0].hidden);
        assert_eq!(root.attachments.len(), 1);
        assert_eq!(root.attachments[0].bytes, b"Hello!");
        let deep = p.entries.iter().find(|e| e.title == "Deep").unwrap();
        assert_eq!(deep.folder.as_deref(), Some("Work"));
        assert_eq!(deep.tags, vec!["Servers"]);
    }

    #[test]
    fn a_key_file_alone_or_with_a_password_opens_the_database() {
        let keyfile = b"this is a key file with some content";
        let file = save(&sample_db(), Some("pw pw pw pw"), Some(keyfile));
        let both = ParseOptions {
            password: Some("pw pw pw pw"),
            keyfile: Some(keyfile),
            ..Default::default()
        };
        assert_eq!(parse(&file, &both).unwrap().entries.len(), 2);
        let only_pw = ParseOptions {
            password: Some("pw pw pw pw"),
            ..Default::default()
        };
        assert_eq!(
            parse(&file, &only_pw).err().unwrap().code(),
            "invalid_input"
        );
        let only_key = save(&sample_db(), None, Some(keyfile));
        let k = ParseOptions {
            keyfile: Some(keyfile),
            ..Default::default()
        };
        assert_eq!(parse(&only_key, &k).unwrap().entries.len(), 2);
    }

    #[test]
    fn wrong_password_and_missing_credentials_have_fixed_messages() {
        let file = save(&sample_db(), Some("right password"), None);
        let wrong = parse(
            &file,
            &ParseOptions {
                password: Some("wrong password"),
                ..Default::default()
            },
        )
        .err()
        .unwrap();
        assert_eq!(wrong.code(), "invalid_input");
        assert_eq!(wrong.to_string(), MSG_KDBX_KEY);
        let none = parse(&file, &ParseOptions::default()).err().unwrap();
        assert_eq!(none.to_string(), MSG_KDBX_NEEDS_PASSWORD);
        let empty = parse(
            &file,
            &ParseOptions {
                password: Some(""),
                ..Default::default()
            },
        )
        .err()
        .unwrap();
        assert_eq!(empty.to_string(), MSG_KDBX_NEEDS_PASSWORD);
        assert!(!wrong.to_string().contains("right password"));
    }

    #[test]
    fn heavy_key_derivation_is_refused_before_it_runs() {
        let file = save(&sample_db(), Some("password"), None);
        let cost = kdf_cost(&file).unwrap();
        assert!(cost.memory_bytes > 0 && cost.iterations > 0 && cost.lanes > 0);
        assert!(check_kdf(&file).is_ok());
        // Rewrite the memory parameter ("M") inside the header to 2 TiB.
        let mut evil = file.clone();
        let at = (0..evil.len().saturating_sub(16)).find(|&i| {
            evil.get(i..i + 6) == Some(&[0x05, 1, 0, 0, 0, b'M'][..])
                && evil.get(i + 6..i + 10) == Some(&[8, 0, 0, 0][..])
        });
        let at = at.expect("memory parameter present in the header");
        evil[at + 10..at + 18].copy_from_slice(&(2u64 << 40).to_le_bytes());
        let e = parse(
            &evil,
            &ParseOptions {
                password: Some("password"),
                ..Default::default()
            },
        )
        .err()
        .unwrap();
        assert_eq!(e.to_string(), MSG_KDF_TOO_HEAVY);
        assert_eq!(check_kdf(&evil).err().unwrap().code(), "unsupported_file");
    }

    #[test]
    fn old_garbage_and_truncated_files_are_clean_errors() {
        let opts = ParseOptions {
            password: Some("x"),
            ..Default::default()
        };
        assert!(parse(b"", &opts).is_err());
        assert!(parse(b"not a database at all", &opts).is_err());
        let mut kdb = MAGIC_KDB.to_vec();
        kdb.extend([0u8; 64]);
        assert_eq!(parse(&kdb, &opts).err().unwrap().to_string(), MSG_KDBX_OLD);
        let file = save(&sample_db(), Some("x"), None);
        for cut in [8, 12, 40, 100, file.len() / 2, file.len() - 1] {
            assert!(
                parse(&file[..cut.min(file.len())], &opts).is_err(),
                "cut {cut}"
            );
        }
        let mut flipped = file.clone();
        let mid = flipped.len() / 2;
        flipped[mid] ^= 0xFF;
        assert!(parse(&flipped, &opts).is_err());
    }
}
