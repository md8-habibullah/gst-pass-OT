//! Known-answer tests. Expected values come from independent implementations
//! (`scripts/gen_crypto_kat.py`: Python `cryptography` + stdlib), never from this code.

use super::kat_vectors::*;
use super::{aead, envelope, kdf, kem, KeyPurpose, MasterPassword, SecretKey32};

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2));
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn key32(bytes: &[u8]) -> SecretKey32 {
    SecretKey32::from_slice(bytes).unwrap()
}

#[test]
fn argon2id_matches_independent_implementation() {
    let salt = unhex(ARGON2_SALT_HEX);
    for (password, m, t, p, expected) in ARGON2_VECTORS {
        let params = kdf::KdfParams {
            m_cost_kib: *m,
            t_cost: *t,
            p_cost: *p,
        };
        let key = kdf::derive_password_key(&MasterPassword::new(password).unwrap(), &salt, &params)
            .unwrap();
        assert_eq!(hex(key.as_bytes()), *expected, "m={m} t={t} p={p}");
    }
}

#[test]
fn nfkc_matches_python_unicodedata() {
    for (input, expected) in NFKC_VECTORS {
        assert_eq!(
            hex(MasterPassword::new(input).unwrap().as_bytes()),
            *expected,
            "{input:?}"
        );
    }
}

#[test]
fn hkdf_subkeys_match_independent_implementation() {
    let root = key32(&(0u8..32).collect::<Vec<_>>());
    for (label, context, expected) in SUBKEY_VECTORS {
        let purpose = match *label {
            "keyring" => KeyPurpose::Keyring,
            "vrk-commit" => KeyPurpose::VrkCommit,
            "item" => KeyPurpose::Item,
            "version" => KeyPurpose::Version,
            "attachment" => KeyPurpose::Attachment,
            "meta" => KeyPurpose::Meta,
            other => panic!("unknown label {other}"),
        };
        assert_eq!(purpose.label(), *label);
        let key = super::derive_subkey(&root, purpose, context).unwrap();
        assert_eq!(hex(key.as_bytes()), *expected, "{label}/{context}");
    }
}

#[test]
fn hybrid_combiner_matches_independent_implementation() {
    let ct: Vec<u8> = (0..1568).map(|i| (i % 251) as u8).collect();
    let wrap = kem::combine(&[0x11; 32], &[0x22; 32], &ct, &[0x33; 32], &[0x44; 32]).unwrap();
    assert_eq!(hex(wrap.as_bytes()), COMBINER_EXPECTED_HEX);
}

#[test]
fn xchacha20_poly1305_matches_independent_implementation_and_ietf_draft() {
    for (key, nonce, aad, plaintext, expected) in XCHACHA_VECTORS {
        let (key, nonce, aad, plaintext, expected) = (
            key32(&unhex(key)),
            unhex(nonce),
            unhex(aad),
            unhex(plaintext),
            unhex(expected),
        );
        let nonce: [u8; 24] = nonce.try_into().unwrap();
        assert_eq!(
            aead::encrypt(&key, &nonce, &aad, &plaintext).unwrap(),
            expected
        );
        assert_eq!(
            &aead::decrypt(&key, &nonce, &aad, &expected).unwrap()[..],
            &plaintext[..]
        );
    }
}

#[test]
fn envelope_bytes_match_independent_implementation() {
    let key = key32(&(0u8..32).collect::<Vec<_>>());
    let nonce: [u8; 24] = (0u8..24).collect::<Vec<_>>().try_into().unwrap();
    let expected = unhex(ENVELOPE_HEX);

    // Rust can read what Python wrote ...
    let opened = envelope::open(
        &key,
        &expected,
        &envelope::Expect {
            item_id: "kat-object-1",
            version: Some(7),
        },
    )
    .unwrap();
    assert_eq!(&opened.plaintext[..], ENVELOPE_PLAINTEXT.as_bytes());
    assert_eq!(opened.header.timestamp, 1_732_360_000_000);

    // ... and writes byte-identical output for the same nonce.
    let header = envelope::Header::new("kat-object-1", 7, 1_732_360_000_000).unwrap();
    let built =
        envelope::seal_with_nonce(&key, &header, ENVELOPE_PLAINTEXT.as_bytes(), &nonce).unwrap();
    assert_eq!(built, expected);
}

// ---- On-disk format stability -------------------------------------------------------------
// A vault key bundle and an item envelope written by this code with the format-v1 parameters.
// If these tests fail after a change, the stored format changed: that needs a version bump and a
// migration, not a regenerated fixture. (`scripts/` cross-checks the same bundle in Python.)

const FIXTURE_PASSWORD: &str = "fixture passphrase v1";
const FIXTURE_ITEM_ID: &str = "fixture-item";
const FIXTURE_PLAINTEXT: &[u8] = b"fixture plaintext";
const FIXTURE_BUNDLE: &str = include_str!("fixtures/bundle_v1.json");
const FIXTURE_ITEM_KEY_HEX: &str = include_str!("fixtures/bundle_v1.item_key.hex");
const FIXTURE_ITEM_ENVELOPE_HEX: &str = include_str!("fixtures/bundle_v1.item.hex");

#[test]
fn bundle_v1_fixture_still_unlocks() {
    use super::{unlock, VaultKeyBundle};
    let bundle = VaultKeyBundle::from_json(FIXTURE_BUNDLE.trim()).unwrap();
    let vrk = unlock(&bundle, &MasterPassword::new(FIXTURE_PASSWORD).unwrap()).unwrap();

    let key = vrk.object_key(KeyPurpose::Item, FIXTURE_ITEM_ID).unwrap();
    assert_eq!(hex(key.as_bytes()), FIXTURE_ITEM_KEY_HEX.trim());

    let stored = unhex(FIXTURE_ITEM_ENVELOPE_HEX.trim());
    let opened = vrk
        .open_object(KeyPurpose::Item, FIXTURE_ITEM_ID, Some(1), &stored)
        .unwrap();
    assert_eq!(&opened.plaintext[..], FIXTURE_PLAINTEXT);
    assert_eq!(opened.header.timestamp, 1_732_360_000_000);
}

#[test]
#[ignore = "rewrites the fixtures; run only when the stored format intentionally changes"]
fn regenerate_bundle_fixture() {
    use super::{create, KdfParams};
    let dir = std::path::Path::new(file!())
        .parent()
        .unwrap()
        .join("fixtures");
    // Fast (but valid) parameters keep the fixture cheap to unlock in every test run.
    let params = KdfParams {
        m_cost_kib: 19_456,
        t_cost: 2,
        p_cost: 1,
    };
    let (bundle, vrk) = create(&MasterPassword::new(FIXTURE_PASSWORD).unwrap(), &params).unwrap();
    let key = vrk.object_key(KeyPurpose::Item, FIXTURE_ITEM_ID).unwrap();
    let sealed = vrk
        .seal_object(
            KeyPurpose::Item,
            FIXTURE_ITEM_ID,
            1,
            1_732_360_000_000,
            FIXTURE_PLAINTEXT,
        )
        .unwrap();
    std::fs::write(dir.join("bundle_v1.json"), bundle.to_json().unwrap() + "\n").unwrap();
    std::fs::write(
        dir.join("bundle_v1.item_key.hex"),
        hex(key.as_bytes()) + "\n",
    )
    .unwrap();
    std::fs::write(dir.join("bundle_v1.item.hex"), hex(&sealed) + "\n").unwrap();
}
