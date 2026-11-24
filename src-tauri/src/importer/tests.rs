//! Cross-module tests of the importer: every fixture through sniffing, parsing and conversion,
//! the refusal of encrypted exports, the size / bomb / depth limits, and a deterministic
//! fuzz-style run (truncated and mutated bytes) through every parser.
//!
//! **Where the fixtures come from** (`src-tauri/tests/fixtures/import/`, all synthetic, no real
//! secrets; none of them is a real export from the vendor's app):
//!
//! | Fixture | Provenance |
//! |---|---|
//! | `bitwarden.json`, `bitwarden.csv` | synthetic, shaped like Bitwarden's export (the TypeScript importer's knowledge of the format carried over) |
//! | `chrome.csv`, `firefox.csv` | synthetic, same columns as the browsers' password exports |
//! | `keepass.xml` | synthetic, KeePass 2.x "KeePass XML (2.x)" layout |
//! | `.kdbx` files | **real KDBX 4 files**, written by the `keepass` crate inside the tests |
//! | `proton.json`, `proton.csv`, `1password-export.data` (the `export.data` member of a `.1pux`), `dashlane.*`, `enpass.*`, `aegis.json`, `2fas.json`, `safari.csv`, `nordpass.csv`, `lastpass.csv`, `1password.csv` | written from the vendors' public descriptions, **not verified against a real export** |
//! | `otpauth.txt`, `google-migration.txt` | `otpauth://` lines, and a Google Authenticator export link built with the documented protobuf layout |
//! | `*-encrypted.json` | the shape of the vendors' encrypted exports, which must be refused |

use super::kdbx::test_support::{sample_db, save};
use super::limits::*;
use super::sniff::sniff;
use super::zipsafe::test_support::make_zip;
use super::*;
use crate::storage::model::ItemType;

macro_rules! fixture {
    ($n:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/import/",
            $n
        )) as &[u8]
    };
}

fn run(f: ImportFormat, bytes: &[u8]) -> Result<ParsedImport> {
    parse(f, bytes, &ParseOptions::default())
}

/// `(file, what sniffing says, format to parse as, entries, skipped, one-time codes, attachments)`
type Case = (
    &'static str,
    &'static [u8],
    ImportFormat,
    ImportFormat,
    usize,
    usize,
    usize,
    usize,
);

fn cases() -> Vec<Case> {
    use ImportFormat as F;
    vec![
        (
            "bitwarden.json",
            fixture!("bitwarden.json"),
            F::BitwardenJson,
            F::BitwardenJson,
            5,
            2,
            1,
            0,
        ),
        (
            "bitwarden.csv",
            fixture!("bitwarden.csv"),
            F::BitwardenCsv,
            F::BitwardenCsv,
            2,
            0,
            1,
            0,
        ),
        (
            "keepass.xml",
            fixture!("keepass.xml"),
            F::KeepassXml,
            F::KeepassXml,
            2,
            1,
            1,
            1,
        ),
        (
            "proton.json",
            fixture!("proton.json"),
            F::ProtonJson,
            F::ProtonJson,
            6,
            1,
            2,
            0,
        ),
        (
            "proton.csv",
            fixture!("proton.csv"),
            F::ProtonCsv,
            F::ProtonCsv,
            3,
            0,
            1,
            0,
        ),
        (
            "1password.csv",
            fixture!("1password.csv"),
            F::OnepasswordCsv,
            F::OnepasswordCsv,
            1,
            0,
            1,
            0,
        ),
        (
            "lastpass.csv",
            fixture!("lastpass.csv"),
            F::LastpassCsv,
            F::LastpassCsv,
            3,
            0,
            0,
            0,
        ),
        (
            "dashlane.csv",
            fixture!("dashlane.csv"),
            F::DashlaneCsv,
            F::DashlaneCsv,
            2,
            0,
            1,
            0,
        ),
        (
            "dashlane.json",
            fixture!("dashlane.json"),
            F::DashlaneJson,
            F::DashlaneJson,
            4,
            0,
            1,
            0,
        ),
        (
            "nordpass.csv",
            fixture!("nordpass.csv"),
            F::NordpassCsv,
            F::NordpassCsv,
            4,
            0,
            0,
            0,
        ),
        // Enpass CSV has no header row the sniffer could recognise: it is chosen by hand.
        (
            "enpass.csv",
            fixture!("enpass.csv"),
            F::CustomCsv,
            F::EnpassCsv,
            2,
            0,
            0,
            0,
        ),
        (
            "enpass.json",
            fixture!("enpass.json"),
            F::EnpassJson,
            F::EnpassJson,
            4,
            1,
            1,
            1,
        ),
        (
            "chrome.csv",
            fixture!("chrome.csv"),
            F::ChromeCsv,
            F::ChromeCsv,
            2,
            0,
            0,
            0,
        ),
        (
            "firefox.csv",
            fixture!("firefox.csv"),
            F::FirefoxCsv,
            F::FirefoxCsv,
            1,
            0,
            0,
            0,
        ),
        (
            "safari.csv",
            fixture!("safari.csv"),
            F::SafariCsv,
            F::SafariCsv,
            1,
            0,
            1,
            0,
        ),
        (
            "aegis.json",
            fixture!("aegis.json"),
            F::AegisJson,
            F::AegisJson,
            2,
            3,
            2,
            0,
        ),
        (
            "2fas.json",
            fixture!("2fas.json"),
            F::TwofasJson,
            F::TwofasJson,
            2,
            1,
            2,
            0,
        ),
        (
            "otpauth.txt",
            fixture!("otpauth.txt"),
            F::OtpauthList,
            F::OtpauthList,
            2,
            2,
            2,
            0,
        ),
        (
            "google-migration.txt",
            fixture!("google-migration.txt"),
            F::GoogleMigration,
            F::GoogleMigration,
            2,
            0,
            2,
            0,
        ),
    ]
}

#[test]
fn every_fixture_is_sniffed_parsed_and_converted() {
    for (name, bytes, sniffed, format, entries, skipped, seeds, attachments) in cases() {
        assert_eq!(sniff(bytes), Some(sniffed), "sniffing {name}");
        let p = run(format, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(p.entries.len(), entries, "{name}: entries");
        assert_eq!(p.skipped, skipped, "{name}: skipped");
        assert_eq!(p.seed_count(), seeds, "{name}: one-time codes");
        assert_eq!(p.attachment_count(), attachments, "{name}: attachments");
        for e in &p.entries {
            let c = e.to_draft();
            let mut d = c.draft;
            d.normalize()
                .unwrap_or_else(|err| panic!("{name}: a converted draft is invalid: {err}"));
            assert!(!d.title.trim().is_empty(), "{name}: a title is always set");
        }
    }
}

#[test]
fn every_format_id_is_unique_and_matches_the_wire_name() {
    let mut seen = std::collections::HashSet::new();
    for f in ImportFormat::ALL {
        assert!(seen.insert(f.id()), "{} twice", f.id());
        assert_eq!(
            serde_json::to_string(&f).unwrap(),
            format!("\"{}\"", f.id())
        );
        let back: ImportFormat = serde_json::from_str(&format!("\"{}\"", f.id())).unwrap();
        assert_eq!(back, f);
    }
    assert_eq!(seen.len(), 25);
    let wants_secrets: Vec<_> = ImportFormat::ALL
        .into_iter()
        .filter(|f| f.needs_password())
        .collect();
    assert_eq!(wants_secrets, vec![ImportFormat::KeepassKdbx]);
}

#[test]
fn the_ids_are_the_ones_the_frontend_declares() {
    // The type union in `src/backend/types.ts` is the contract; skip quietly if it moved.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/backend/types.ts");
    let Ok(ts) = std::fs::read_to_string(path) else {
        return;
    };
    for f in ImportFormat::ALL {
        assert!(
            ts.contains(&format!("\"{}\"", f.id())),
            "{} is not in types.ts",
            f.id()
        );
    }
}

#[test]
fn spot_checks_of_what_the_parsers_read() {
    let p = run(ImportFormat::FirefoxCsv, fixture!("firefox.csv")).unwrap();
    assert_eq!(p.entries[0].created, Some(1_700_000_000_000));
    assert_eq!(p.entries[0].to_draft().draft.title, "example.org");

    let p = run(ImportFormat::TwofasJson, fixture!("2fas.json")).unwrap();
    let d = p.entries[1].to_draft().draft;
    assert_eq!(d.item_type, ItemType::Totp);
    assert!(d.fields["totp"].contains("digits=8") && d.fields["totp"].contains("period=60"));

    let p = run(
        ImportFormat::GoogleMigration,
        fixture!("google-migration.txt"),
    )
    .unwrap();
    assert!(p.entries[0].totp[0].starts_with("JBSWY3DPEHPK3PXP"));

    let p = run(ImportFormat::BitwardenJson, fixture!("bitwarden.json")).unwrap();
    let kinds: Vec<_> = p
        .entries
        .iter()
        .map(|e| e.to_draft().draft.item_type)
        .collect();
    assert_eq!(
        kinds,
        vec![
            ItemType::Login,
            ItemType::SecureNote,
            ItemType::Card,
            ItemType::Identity,
            ItemType::SshKey
        ]
    );
}

#[test]
fn archives_and_databases_built_in_the_test_are_recognised_and_read() {
    // 1Password: a zip whose `export.data` member is the JSON.
    let data = fixture!("1password-export.data");
    let onepux = make_zip(&[("export.data", data)], false);
    assert_eq!(sniff(&onepux), Some(ImportFormat::Onepassword1pux));
    let p = run(ImportFormat::Onepassword1pux, &onepux).unwrap();
    assert_eq!(p.entries.len(), 6);
    assert_eq!(p.skipped, 1);

    // Proton Pass: a zip with the JSON inside a folder.
    let proton = make_zip(&[("Proton Pass/data.json", fixture!("proton.json"))], false);
    assert_eq!(sniff(&proton), Some(ImportFormat::ProtonZip));
    assert_eq!(
        run(ImportFormat::ProtonZip, &proton).unwrap().entries.len(),
        6
    );

    // KeePass: a real KDBX 4 file, opened with a password, and with a password and a key file.
    let kdbx = save(&sample_db(), Some("correct horse battery"), None);
    assert_eq!(sniff(&kdbx), Some(ImportFormat::KeepassKdbx));
    let opts = ParseOptions {
        password: Some("correct horse battery"),
        ..ParseOptions::default()
    };
    assert_eq!(
        parse(ImportFormat::KeepassKdbx, &kdbx, &opts)
            .unwrap()
            .entries
            .len(),
        2
    );
    let keyfile: &[u8] = b"a key file for the test";
    let both = save(&sample_db(), Some("pw pw pw pw"), Some(keyfile));
    let opts = ParseOptions {
        password: Some("pw pw pw pw"),
        keyfile: Some(keyfile),
        ..ParseOptions::default()
    };
    assert_eq!(
        parse(ImportFormat::KeepassKdbx, &both, &opts)
            .unwrap()
            .entries
            .len(),
        2
    );
}

#[test]
fn encrypted_exports_are_recognised_and_refused_with_one_sentence() {
    use ImportFormat as F;
    for (bytes, format) in [
        (fixture!("bitwarden-encrypted.json"), F::BitwardenJson),
        (fixture!("aegis-encrypted.json"), F::AegisJson),
        (fixture!("2fas-encrypted.json"), F::TwofasJson),
    ] {
        assert_eq!(sniff(bytes), Some(format), "{}", format.id());
        let e = run(format, bytes).err().unwrap();
        assert_eq!(e.to_string(), MSG_ENCRYPTED_EXPORT, "{}", format.id());
        assert_eq!(e.code(), "unsupported_file");
    }
    assert_eq!(
        MSG_ENCRYPTED_EXPORT,
        "This export is encrypted. Export it unencrypted from the app, import it, then delete the file."
    );
}

// ------------------------------------------------------------------ limits

#[test]
fn a_file_over_256_mib_is_refused_by_every_parser_without_being_read() {
    // Zero pages are mapped lazily, so this costs address space, not memory.
    let big = vec![0u8; usize::try_from(MAX_FILE_BYTES).unwrap() + 1];
    for f in ImportFormat::ALL {
        let e = parse(f, &big, &ParseOptions::default()).err().unwrap();
        assert_eq!(e.to_string(), MSG_TOO_LARGE, "{}", f.id());
    }
}

#[test]
fn a_zip_bomb_is_refused_by_both_zip_formats() {
    // 24 MiB of zeros deflate to about 24 KiB: a ratio of about 1000 to 1.
    let bomb = vec![0u8; 24 * 1024 * 1024];
    for (name, format) in [
        ("export.data", ImportFormat::Onepassword1pux),
        ("data.json", ImportFormat::ProtonZip),
    ] {
        let z = make_zip(&[(name, &bomb)], false);
        assert!(z.len() < 256 * 1024, "the bomb is small on disk");
        let e = run(format, &z).err().unwrap();
        assert_eq!(e.code(), "unsupported_file");
        assert_eq!(e.to_string(), MSG_ZIP_UNSAFE, "{}", format.id());
    }
}

#[test]
fn zip_member_names_are_never_used_as_paths() {
    // A traversal name is just a name: nothing is written anywhere, and nothing is found by it.
    let z = make_zip(
        &[
            ("../../../etc/passwd", b"x"),
            ("export.data", fixture!("1password-export.data")),
        ],
        false,
    );
    assert_eq!(
        run(ImportFormat::Onepassword1pux, &z)
            .unwrap()
            .entries
            .len(),
        6
    );
}

#[test]
fn more_than_200_000_entries_are_refused() {
    let mut text = String::from("name,url,username,password\n");
    for _ in 0..=MAX_ENTRIES {
        text.push_str("a,,,p\n");
    }
    let e = run(ImportFormat::ChromeCsv, text.as_bytes()).err().unwrap();
    assert_eq!(e.to_string(), MSG_TOO_MANY);
}

#[test]
fn nesting_is_limited_in_json_and_in_xml() {
    let deep_json = format!(
        "{{\"items\":{}1{}}}",
        "[".repeat(MAX_DEPTH + 10),
        "]".repeat(MAX_DEPTH + 10)
    );
    for f in [
        ImportFormat::BitwardenJson,
        ImportFormat::ProtonJson,
        ImportFormat::DashlaneJson,
        ImportFormat::EnpassJson,
        ImportFormat::AegisJson,
        ImportFormat::TwofasJson,
    ] {
        assert!(run(f, deep_json.as_bytes()).is_err(), "{}", f.id());
    }
    let mut xml = String::from("<KeePassFile><Root>");
    for _ in 0..(MAX_DEPTH + 10) {
        xml.push_str("<Group>");
    }
    assert!(run(ImportFormat::KeepassXml, xml.as_bytes()).is_err());
}

#[test]
fn xml_entities_and_doctypes_are_never_expanded() {
    let laughs = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">]><KeePassFile><Root><Group><Entry><String><Key>Title</Key><Value>&lol2;</Value></String></Entry></Group></Root></KeePassFile>"#;
    let e = run(ImportFormat::KeepassXml, laughs.as_bytes())
        .err()
        .unwrap();
    assert_eq!(e.to_string(), MSG_XML_DTD);
    // An entity that is not one of the five predefined ones is never looked up.
    let unknown = r#"<KeePassFile><Root><Group><Entry><String><Key>Title</Key><Value>a&custom;b</Value></String></Entry></Group></Root></KeePassFile>"#;
    match run(ImportFormat::KeepassXml, unknown.as_bytes()) {
        Ok(p) => assert!(p.entries.iter().all(|e| !e.title.contains("lol"))),
        Err(e) => assert_eq!(e.code(), "unsupported_file"),
    }
}

#[test]
fn csv_edge_cases_through_the_whole_pipeline() {
    // BOM, CRLF, a quoted newline and an escaped quote, a semicolon delimiter.
    let text = "\u{feff}name;url;username;password;note\r\n\"A \"\"quoted\"\" name\";https://a.example;u;\"p;w\";\"two\r\nlines\"\r\nB;;v;q;\r\n";
    let p = run(ImportFormat::ChromeCsv, text.as_bytes()).unwrap();
    assert_eq!(p.entries.len(), 2);
    assert_eq!(p.entries[0].title, "A \"quoted\" name");
    assert_eq!(p.entries[0].password, "p;w");
    assert!(p.entries[0].notes.contains("two") && p.entries[0].notes.contains("lines"));
    // Tab and pipe.
    for d in ['\t', '|'] {
        let t = format!("name{d}url{d}username{d}password\nA{d}https://a.example{d}u{d}p\n");
        assert_eq!(
            run(ImportFormat::ChromeCsv, t.as_bytes())
                .unwrap()
                .entries
                .len(),
            1,
            "{d:?}"
        );
    }
    // Invalid UTF-8 inside one cell is replaced, not fatal.
    let mut raw = b"name,url,username,password\nbad ".to_vec();
    raw.extend([0xFF, 0xFE, b'x']);
    raw.extend(b",https://a.example,u,p\n");
    let p = run(ImportFormat::ChromeCsv, &raw).unwrap();
    assert!(p.entries[0].title.starts_with("bad "));
    // A file that is mostly binary is not a CSV.
    assert!(run(ImportFormat::ChromeCsv, &[0u8; 4096]).is_err());
}

// ------------------------------------------------------------------ fuzz-style

/// A small deterministic generator (xorshift64*), so the fuzz run is the same every time.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap()).unwrap()
    }

    fn byte(&mut self) -> u8 {
        u8::try_from(self.next() & 0xFF).unwrap()
    }
}

fn all_parsers_and_sniffer(bytes: &[u8]) {
    let _ = sniff(bytes);
    for f in ImportFormat::ALL {
        // Parsers must return, not panic; their errors are fixed sentences.
        if let Err(e) = parse(f, bytes, &ParseOptions::default()) {
            assert!(!e.to_string().is_empty());
        }
    }
}

#[test]
fn every_fixture_into_every_parser() {
    for (_, bytes, ..) in cases() {
        all_parsers_and_sniffer(bytes);
    }
    for bytes in [
        fixture!("bitwarden-encrypted.json"),
        fixture!("aegis-encrypted.json"),
        fixture!("2fas-encrypted.json"),
        fixture!("custom.csv"),
    ] {
        all_parsers_and_sniffer(bytes);
    }
}

#[test]
fn random_bytes_and_hostile_prefixes_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for len in [0usize, 1, 2, 3, 7, 16, 64, 255, 1024, 4096] {
        let mut buf = vec![0u8; len];
        for _ in 0..6 {
            for b in &mut buf {
                *b = rng.byte();
            }
            all_parsers_and_sniffer(&buf);
        }
    }
    let hostile: [&[u8]; 14] = [
        b"{",
        b"[",
        b"\"",
        b"{\"items\":",
        b"{\"items\":[{",
        b"<",
        b"<KeePassFile",
        b"<KeePassFile><Root><Group><Entry><String><Key>",
        b"PK\x03\x04",
        b"PK\x05\x06\x00\x00",
        b"\x03\xD9\xA2\x9A\x67\xFB\x4B\xB5",
        b"otpauth://",
        b"otpauth-migration://offline?data=",
        b"name,url,username,password\n\"",
    ];
    for h in hostile {
        all_parsers_and_sniffer(h);
    }
}

#[test]
fn truncated_and_mutated_fixtures_never_panic() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let specials = b"\"{}[],<>\n\\;";
    for (name, bytes, _, format, ..) in cases() {
        // Every prefix at 40 evenly spread lengths.
        for i in 0..40 {
            let cut = bytes.len() * i / 40;
            let part = &bytes[..cut];
            let _ = sniff(part);
            let _ = parse(format, part, &ParseOptions::default());
        }
        // 60 copies with 1 to 4 bytes changed, inserted or removed.
        for _ in 0..60 {
            let mut m = bytes.to_vec();
            for _ in 0..=rng.below(4) {
                if m.is_empty() {
                    break;
                }
                let at = rng.below(m.len());
                match rng.below(3) {
                    0 => m[at] = rng.byte(),
                    1 => m.insert(at, specials[rng.below(specials.len())]),
                    _ => {
                        m.remove(at);
                    }
                }
            }
            let _ = sniff(&m);
            match parse(format, &m, &ParseOptions::default()) {
                Ok(p) => {
                    for e in &p.entries {
                        let mut d = e.to_draft().draft;
                        let _ = d.normalize();
                    }
                }
                Err(e) => assert!(!e.to_string().is_empty(), "{name}"),
            }
        }
    }
}

#[test]
fn damaged_archives_and_databases_never_panic() {
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    let onepux = make_zip(&[("export.data", fixture!("1password-export.data"))], false);
    let proton = make_zip(&[("data.json", fixture!("proton.json"))], false);
    for (zip, format) in [
        (&onepux, ImportFormat::Onepassword1pux),
        (&proton, ImportFormat::ProtonZip),
    ] {
        for cut in (0..zip.len()).step_by(7) {
            let _ = sniff(&zip[..cut]);
            let _ = run(format, &zip[..cut]);
        }
        for _ in 0..150 {
            let mut m = zip.clone();
            let at = rng.below(m.len());
            m[at] ^= u8::try_from(1 + rng.below(255)).unwrap();
            let _ = sniff(&m);
            let _ = run(format, &m);
        }
    }
    // KDBX: cuts anywhere, and flips in the encrypted part (a flip inside the header could change
    // the key-derivation cost, which the parser refuses to run when it is too high but which
    // would still make a test slow).
    let kdbx = save(&sample_db(), Some("pw"), None);
    let opts = ParseOptions {
        password: Some("pw"),
        ..ParseOptions::default()
    };
    for cut in (0..kdbx.len()).step_by(97) {
        assert!(parse(ImportFormat::KeepassKdbx, &kdbx[..cut], &opts).is_err());
    }
    for _ in 0..20 {
        let mut m = kdbx.clone();
        let at = m.len() - 1 - rng.below(200);
        m[at] ^= u8::try_from(1 + rng.below(255)).unwrap();
        assert!(parse(ImportFormat::KeepassKdbx, &m, &opts).is_err());
    }
}

#[test]
fn an_error_never_contains_file_content() {
    // A distinctive secret in a broken file must not come back in the error text.
    let secret = "SECRET-MARKER-7f3a9";
    let broken = format!("name,url,username,password\n\"{secret},https://a.example,u,p");
    let e = run(ImportFormat::ChromeCsv, broken.as_bytes())
        .err()
        .unwrap();
    assert!(!e.to_string().contains(secret));
    let json = format!("{{\"items\":[{{\"name\":\"{secret}\",");
    let e = run(ImportFormat::BitwardenJson, json.as_bytes())
        .err()
        .unwrap();
    assert!(!e.to_string().contains(secret));
    let xml = format!("<KeePassFile><Root><Group><Entry><String><Key>{secret}");
    if let Err(e) = run(ImportFormat::KeepassXml, xml.as_bytes()) {
        assert!(!e.to_string().contains(secret));
    }
}

#[test]
fn debug_output_of_parsed_data_shows_no_values() {
    let p = run(ImportFormat::BitwardenJson, fixture!("bitwarden.json")).unwrap();
    let shown = format!("{p:?} {:?}", p.entries);
    for secret in [
        "4111111111111111",
        "JBSWY3DPEHPK3PXP",
        "https://a.example",
        "Box",
    ] {
        assert!(
            !shown.contains(secret),
            "{secret} leaked into the debug text"
        );
    }
}
