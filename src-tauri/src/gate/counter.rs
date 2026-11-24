//! `gate.json`: the Stage 1 state file next to the account registry.
//!
//! It holds the mode and the switches, the Argon2id hashes (the credential, its reverse, the
//! optional custom panic code) and the **attempt counter**. The counter record carries a MAC
//! (HMAC-SHA-256 keyed with an HKDF of a random per-install key kept in the same file), so a
//! casual edit (someone changing `attempts` to 0 in a text editor) is noticed.
//!
//! Honest limit: the key sits in the same file, so this detects *casual* edits only. Whoever can
//! write the file and read it can forge a record, delete the file (which switches the gate off) or
//! restore an older copy of the whole data folder (which resets the counter). The gate is a
//! privacy screen; the cryptographic protection is the master password (`ARCHITECTURE.md` D15).
//!
//! The file is owner-only (0600) and replaced atomically with the file and the folder fsynced.

use super::credential::Phc;
use super::{GateMode, MAX_ATTEMPTS};
use crate::crypto::{now_millis, rng};
use crate::storage::files;
use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use base64::Engine;
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::path::Path;
use zeroize::Zeroizing;

pub const GATE_FILE: &str = "gate.json";
const FILE_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAC_SALT: &[u8] = b"passman/gate/v1";
const MAC_INFO: &[u8] = b"counter-mac";

/// Everything the file stores except the counter record.
#[derive(Clone)]
pub struct Persist {
    pub mode: GateMode,
    pub reverse_panic: bool,
    pub biometric: bool,
    pub panic_button: bool,
    pub credential: Phc,
    pub reversed: Option<Phc>,
    pub panic_code: Option<Phc>,
    pub install_key: Zeroizing<[u8; 32]>,
}

/// A fresh random per-install MAC key.
pub fn new_install_key() -> Result<Zeroizing<[u8; 32]>, crate::crypto::CryptoError> {
    let mut key = Zeroizing::new([0u8; 32]);
    rng::fill(&mut key[..])?;
    Ok(key)
}

/// What the counter record of the file says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterRead {
    /// A record whose MAC checks out.
    Valid { attempts: u32, epoch: u64 },
    /// Missing, unparsable or with a bad MAC. `hint` is the attempts value if one could be read.
    Damaged { hint: Option<u32> },
}

/// The result of reading the file at start-up.
pub enum Loaded {
    /// No file: the gate is not configured.
    Absent,
    /// A file that cannot be used at all (not JSON, a bad hash, an unknown version).
    Corrupt,
    Ready(Box<Persist>, CounterRead),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnDisk {
    version: u32,
    mode: GateMode,
    reverse_panic: bool,
    biometric: bool,
    panic_button: bool,
    credential: String,
    reversed: Option<String>,
    panic_code: Option<String>,
    install_key: String,
    /// Read as a loose JSON value on purpose: a hand-edited counter must be a *damaged counter*,
    /// not a damaged file.
    #[serde(default)]
    counter: Option<Value>,
}

fn mac_key(install_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    let hk = Hkdf::<Sha256>::new(Some(MAC_SALT), install_key);
    // Expanding 32 bytes from a SHA-256 HKDF cannot fail; an unfilled key would only make every
    // record read as damaged.
    let _ = hk.expand(MAC_INFO, &mut out[..]);
    out
}

/// The tag over one counter record.
pub fn tag(install_key: &[u8; 32], attempts: u32, epoch: u64) -> [u8; 32] {
    let key = mac_key(install_key);
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(&key[..]) else {
        return [0u8; 32];
    };
    mac.update(b"v1");
    mac.update(&attempts.to_le_bytes());
    mac.update(&epoch.to_le_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

fn verify_tag(install_key: &[u8; 32], attempts: u32, epoch: u64, presented: &[u8]) -> bool {
    let key = mac_key(install_key);
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(&key[..]) else {
        return false;
    };
    mac.update(b"v1");
    mac.update(&attempts.to_le_bytes());
    mac.update(&epoch.to_le_bytes());
    mac.verify_slice(presented).is_ok()
}

/// Interpret the `counter` value of a file. Never fails: anything wrong is `Damaged`.
fn read_record(install_key: &[u8; 32], counter: Option<&Value>) -> CounterRead {
    let Some(Value::Object(map)) = counter else {
        return CounterRead::Damaged { hint: None };
    };
    let attempts = map.get("attempts").and_then(Value::as_u64);
    let hint = attempts.map(|a| a.min(u64::from(MAX_ATTEMPTS)) as u32);
    let epoch = map.get("epoch").and_then(Value::as_u64);
    let tag_bytes = map
        .get("tag")
        .and_then(Value::as_str)
        .and_then(|t| B64.decode(t).ok());
    match (attempts, epoch, tag_bytes) {
        (Some(a), Some(e), Some(t)) if a <= u64::from(u32::MAX) => {
            if verify_tag(install_key, a as u32, e, &t) {
                CounterRead::Valid {
                    attempts: a as u32,
                    epoch: e,
                }
            } else {
                CounterRead::Damaged { hint }
            }
        }
        _ => CounterRead::Damaged { hint },
    }
}

fn read_file(dir: &Path) -> Result<Option<Vec<u8>>, ()> {
    let path = dir.join(GATE_FILE);
    match std::fs::metadata(&path) {
        Ok(m) if m.len() > MAX_FILE_BYTES => return Err(()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    }
    match std::fs::read(&path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

fn phc_of(text: &str) -> Option<Phc> {
    Phc::parse(text)
}

/// Read and validate the whole file (start-up).
pub fn load(dir: &Path) -> Loaded {
    let bytes = match read_file(dir) {
        Ok(Some(b)) => b,
        Ok(None) => return Loaded::Absent,
        Err(()) => return Loaded::Corrupt,
    };
    let Ok(doc) = serde_json::from_slice::<OnDisk>(&bytes) else {
        return Loaded::Corrupt;
    };
    if doc.version != FILE_VERSION {
        return Loaded::Corrupt;
    }
    let Some(credential) = phc_of(&doc.credential) else {
        return Loaded::Corrupt;
    };
    let reversed = match doc.reversed.as_deref().map(phc_of) {
        None => None,
        Some(Some(p)) => Some(p),
        Some(None) => return Loaded::Corrupt,
    };
    let panic_code = match doc.panic_code.as_deref().map(phc_of) {
        None => None,
        Some(Some(p)) => Some(p),
        Some(None) => return Loaded::Corrupt,
    };
    let Some(key) = B64
        .decode(&doc.install_key)
        .ok()
        .and_then(|k| <[u8; 32]>::try_from(k.as_slice()).ok())
    else {
        return Loaded::Corrupt;
    };
    let install_key = Zeroizing::new(key);
    let counter = read_record(&install_key, doc.counter.as_ref());
    Loaded::Ready(
        Box::new(Persist {
            mode: doc.mode,
            reverse_panic: doc.reverse_panic,
            biometric: doc.biometric,
            panic_button: doc.panic_button,
            credential,
            reversed,
            panic_code,
            install_key,
        }),
        counter,
    )
}

/// Re-read **only the counter** for one attempt, verifying it with the key held in memory (never
/// the key in the file). A missing, unparsable or tampered file is a damaged counter.
pub fn read_counter(dir: &Path, install_key: &[u8; 32]) -> CounterRead {
    let Ok(Some(bytes)) = read_file(dir) else {
        return CounterRead::Damaged { hint: None };
    };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        return CounterRead::Damaged { hint: None };
    };
    read_record(install_key, doc.get("counter"))
}

/// Write the whole file atomically (0600, file and folder fsynced) with this counter record.
pub fn store(dir: &Path, p: &Persist, attempts: u32, epoch: u64) -> std::io::Result<()> {
    let t = tag(&p.install_key, attempts, epoch);
    let doc = OnDisk {
        version: FILE_VERSION,
        mode: p.mode,
        reverse_panic: p.reverse_panic,
        biometric: p.biometric,
        panic_button: p.panic_button,
        credential: p.credential.to_phc_string(),
        reversed: p.reversed.as_ref().map(Phc::to_phc_string),
        panic_code: p.panic_code.as_ref().map(Phc::to_phc_string),
        install_key: B64.encode(&p.install_key[..]),
        counter: Some(json!({
            "attempts": attempts,
            "epoch": epoch,
            "tag": B64.encode(t),
        })),
    };
    let bytes = serde_json::to_vec_pretty(&doc).map_err(std::io::Error::other)?;
    files::write_atomic(&dir.join(GATE_FILE), &bytes)
}

/// Move an unusable file aside as `gate.json.corrupt-<ms>` (never deleted here; the wipe removes
/// it with everything else).
pub fn quarantine(dir: &Path) {
    let from = dir.join(GATE_FILE);
    let to = dir.join(format!("{GATE_FILE}.corrupt-{}", now_millis()));
    let _ = std::fs::rename(from, to);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KdfParams;
    use crate::gate::credential::{hash_new, Normalised};

    fn fast() -> KdfParams {
        KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        }
    }

    fn persist() -> Persist {
        let h = |s: &str| hash_new(&Normalised::new(s), &fast()).unwrap();
        Persist {
            mode: GateMode::Pin,
            reverse_panic: true,
            biometric: false,
            panic_button: true,
            credential: h("2468"),
            reversed: Some(h("8642")),
            panic_code: Some(h("1111")),
            install_key: new_install_key().unwrap(),
        }
    }

    fn ready(dir: &Path) -> (Persist, CounterRead) {
        match load(dir) {
            Loaded::Ready(p, c) => (*p, c),
            Loaded::Absent => panic!("absent"),
            Loaded::Corrupt => panic!("corrupt"),
        }
    }

    fn edit(dir: &Path, f: impl FnOnce(&mut Value)) {
        let path = dir.join(GATE_FILE);
        let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        f(&mut v);
        std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }

    #[test]
    fn the_tag_depends_on_every_input() {
        let k = [7u8; 32];
        let t = tag(&k, 3, 9);
        assert_eq!(t, tag(&k, 3, 9));
        assert_ne!(t, tag(&k, 4, 9));
        assert_ne!(t, tag(&k, 3, 10));
        assert_ne!(t, tag(&[8u8; 32], 3, 9));
        assert!(verify_tag(&k, 3, 9, &t));
        assert!(!verify_tag(&k, 3, 9, &t[..31]));
        assert!(!verify_tag(&k, 2, 9, &t));
    }

    #[test]
    fn a_file_round_trips_and_holds_only_hashes() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load(dir.path()), Loaded::Absent));
        let p = persist();
        store(dir.path(), &p, 4, 11).unwrap();
        let (back, counter) = ready(dir.path());
        assert_eq!(
            counter,
            CounterRead::Valid {
                attempts: 4,
                epoch: 11
            }
        );
        assert_eq!(back.mode, GateMode::Pin);
        assert!(back.reverse_panic && back.panic_button && !back.biometric);
        assert_eq!(
            back.credential.to_phc_string(),
            p.credential.to_phc_string()
        );
        assert!(back.reversed.is_some() && back.panic_code.is_some());
        assert_eq!(&back.install_key[..], &p.install_key[..]);
        let text = std::fs::read_to_string(dir.path().join(GATE_FILE)).unwrap();
        for plain in ["2468", "8642", "1111"] {
            // hashes and base64 may contain digit runs by chance; the credentials as fields do not
            assert!(!text.contains(&format!("\"{plain}\"")), "{plain}");
        }
        assert!(text.matches("$argon2id$").count() == 3);
        assert!(!text.contains("length"), "no length of anything is stored");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(GATE_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(
            !dir.path().join("gate.json.tmp").exists(),
            "no temp file is left behind"
        );
    }

    #[test]
    fn editing_the_counter_by_hand_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let p = persist();
        store(dir.path(), &p, 9, 5).unwrap();
        let key = *p.install_key;
        // reset the attempts to 0 and keep the old tag
        edit(dir.path(), |v| v["counter"]["attempts"] = json!(0));
        assert_eq!(
            read_counter(dir.path(), &key),
            CounterRead::Damaged { hint: Some(0) }
        );
        // epoch edited
        store(dir.path(), &p, 9, 5).unwrap();
        edit(dir.path(), |v| v["counter"]["epoch"] = json!(1));
        assert!(matches!(
            read_counter(dir.path(), &key),
            CounterRead::Damaged { hint: Some(9) }
        ));
        // tag edited, removed, or of the wrong type
        for patch in [json!("AAAA"), json!(7), Value::Null] {
            store(dir.path(), &p, 9, 5).unwrap();
            edit(dir.path(), |v| v["counter"]["tag"] = patch.clone());
            assert!(matches!(
                read_counter(dir.path(), &key),
                CounterRead::Damaged { hint: Some(9) }
            ));
        }
        // the whole record removed, replaced by junk, or of the wrong type
        for patch in [
            Value::Null,
            json!("junk"),
            json!([1, 2]),
            json!({"attempts": "x"}),
        ] {
            store(dir.path(), &p, 9, 5).unwrap();
            edit(dir.path(), |v| v["counter"] = patch.clone());
            assert!(matches!(
                read_counter(dir.path(), &key),
                CounterRead::Damaged { .. }
            ));
        }
        store(dir.path(), &p, 9, 5).unwrap();
        edit(dir.path(), |v| {
            v.as_object_mut().unwrap().remove("counter");
        });
        assert_eq!(
            read_counter(dir.path(), &key),
            CounterRead::Damaged { hint: None }
        );
        // a record signed with another key (a swapped file)
        store(dir.path(), &p, 2, 8).unwrap();
        assert!(matches!(
            read_counter(dir.path(), &[1u8; 32]),
            CounterRead::Damaged { hint: Some(2) }
        ));
    }

    #[test]
    fn a_damaged_counter_does_not_make_the_file_unusable() {
        let dir = tempfile::tempdir().unwrap();
        store(dir.path(), &persist(), 9, 5).unwrap();
        edit(dir.path(), |v| v["counter"] = json!("junk"));
        let (_, counter) = ready(dir.path());
        assert!(matches!(counter, CounterRead::Damaged { .. }));
    }

    #[test]
    fn a_missing_or_garbage_file_reads_as_a_damaged_counter_while_running() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_counter(dir.path(), &[1u8; 32]),
            CounterRead::Damaged { hint: None }
        );
        std::fs::write(dir.path().join(GATE_FILE), b"not json at all").unwrap();
        assert_eq!(
            read_counter(dir.path(), &[1u8; 32]),
            CounterRead::Damaged { hint: None }
        );
    }

    #[test]
    fn files_that_cannot_be_used_are_corrupt_not_exhausted() {
        let dir = tempfile::tempdir().unwrap();
        let p = persist();
        let good = |dir: &Path| store(dir, &p, 0, 1).unwrap();
        for patch in [
            Box::new(|v: &mut Value| v["version"] = json!(2)) as Box<dyn Fn(&mut Value)>,
            Box::new(|v| v["credential"] = json!("not a hash")),
            Box::new(|v| v["reversed"] = json!("not a hash")),
            Box::new(|v| v["panicCode"] = json!(5)),
            Box::new(|v| v["installKey"] = json!("AAAA")),
            Box::new(|v| v["mode"] = json!("fingerprint")),
            Box::new(|v| {
                v.as_object_mut().unwrap().remove("credential");
            }),
        ] {
            good(dir.path());
            edit(dir.path(), |v| patch(v));
            assert!(matches!(load(dir.path()), Loaded::Corrupt));
        }
        std::fs::write(dir.path().join(GATE_FILE), b"{").unwrap();
        assert!(matches!(load(dir.path()), Loaded::Corrupt));
        std::fs::write(dir.path().join(GATE_FILE), vec![b' '; 70_000]).unwrap();
        assert!(matches!(load(dir.path()), Loaded::Corrupt));
    }

    #[test]
    fn quarantine_moves_the_file_aside_without_deleting_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(GATE_FILE), b"junk").unwrap();
        quarantine(dir.path());
        assert!(!dir.path().join(GATE_FILE).exists());
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("gate.json.corrupt-"));
    }
}
