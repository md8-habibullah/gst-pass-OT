//! Tests of the presets, the choice, its validation, the saved file, the launcher seam and the
//! wipe hook. Temp folders only.

use super::*;
use serde_json::json;

const NAMES: [&str; 11] = [
    "Tally", "Vellum", "Drizzle", "Trim", "Plinth", "Kerf", "Basalt", "Cairn", "Kovren", "Tivlo",
    "Orvyn",
];

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Width and height from the IHDR chunk, if `bytes` starts like a PNG.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

fn input(preset: Option<&str>, name: Option<&str>, lowercase: bool) -> DisguiseInput {
    DisguiseInput {
        preset_id: preset.map(str::to_string),
        custom_name: name.map(str::to_string),
        lowercase,
    }
}

fn err_message(r: Result<DisguiseInput>) -> (String, String) {
    let e = r.expect_err("expected a refusal");
    (e.code().to_string(), e.to_string())
}

// ------------------------------------------------------------------ the embedded presets

#[test]
fn presets_json_matches_the_contract() {
    let file: PresetFile = serde_json::from_str(PRESETS_JSON).expect("presets.json parses");
    assert_eq!(file.presets.len(), 11);
    let names: Vec<&str> = file.presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, NAMES, "the names, in the user's order");
    let mut ids: Vec<&str> = file.presets.iter().map(|p| p.id.as_str()).collect();
    for id in &ids {
        assert!(
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase()),
            "{id:?} must match ^[a-z]+$"
        );
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 11, "ids are unique");
    for p in &file.presets {
        assert!(!p.description.trim().is_empty());
        let hex = p.background.strip_prefix('#').unwrap_or("");
        assert!(
            hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
            "{} has a #RRGGBB background",
            p.id
        );
    }
    // none was dropped for lack of an embedded icon, and the order is kept
    let loaded: Vec<&str> = presets().iter().map(PresetAssets::id).collect();
    let in_file: Vec<&str> = file.presets.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(loaded, in_file);
    // every embedded icon belongs to a preset
    assert_eq!(ICONS.len(), 11);
    for (id, _, _) in ICONS {
        assert!(
            in_file.contains(id),
            "{id} is embedded but not in presets.json"
        );
    }
}

#[test]
fn every_embedded_png_is_a_png_of_the_expected_size() {
    for p in presets() {
        assert!(p.png_512.starts_with(&PNG_SIGNATURE), "{} 512", p.id());
        assert!(p.png_128.starts_with(&PNG_SIGNATURE), "{} 128", p.id());
        assert_eq!(png_size(p.png_512), Some((512, 512)), "{} 512", p.id());
        assert_eq!(png_size(p.png_128), Some((128, 128)), "{} 128", p.id());
        let (w, h, px) = decode_png_rgba(p.png_512).expect("decodes");
        assert_eq!((w, h), (512, 512));
        assert_eq!(px.len(), 512 * 512 * 4);
        let (w, h, px) = decode_png_rgba(p.png_128).expect("decodes");
        assert_eq!((w, h, px.len()), (128, 128, 128 * 128 * 4));
    }
    assert_eq!(png_size(ORIGINAL_ICON_PNG), Some((512, 512)));
    assert!(decode_png_rgba(ORIGINAL_ICON_PNG).is_ok());
}

#[test]
fn decoding_junk_is_an_error_not_a_panic() {
    assert!(decode_png_rgba(&[]).is_err());
    assert!(decode_png_rgba(&PNG_SIGNATURE).is_err());
    assert!(decode_png_rgba(b"not a png at all").is_err());
    let good = presets()[0].png_512;
    assert!(
        decode_png_rgba(&good[..good.len() / 2]).is_err(),
        "truncated"
    );
}

#[test]
fn preset_lookup_is_exact() {
    assert_eq!(preset("kerf").map(|p| p.preset.name.as_str()), Some("Kerf"));
    for bad in [
        "", "Kerf", "kerf ", " kerf", "ker", "../kerf", "kerf\n", "original",
    ] {
        assert!(preset(bad).is_none(), "{bad:?}");
    }
}

// ------------------------------------------------------------------ validation

#[test]
fn accepted_choices_are_normalized() {
    let ok = |i: DisguiseInput| i.validated().expect("accepted");
    assert_eq!(ok(input(None, None, false)), input(None, None, false));
    for p in presets() {
        assert_eq!(
            ok(input(Some(p.id()), None, true)),
            input(Some(p.id()), None, true)
        );
    }
    assert_eq!(
        ok(input(None, Some("  Notes  "), false))
            .custom_name
            .as_deref(),
        Some("Notes")
    );
    // empty after trimming counts as "no custom name"
    for blank in ["", " ", "   ", "\t \n", "\u{3000}"] {
        assert_eq!(
            ok(input(Some("tally"), Some(blank), false)).custom_name,
            None,
            "{blank:?}"
        );
    }
    // exactly 30 characters, counted as characters and not bytes
    assert!(ok(input(None, Some(&"a".repeat(30)), false))
        .custom_name
        .is_some());
    assert!(ok(input(None, Some(&"é".repeat(30)), false))
        .custom_name
        .is_some());
    // separators, `%`, `;`, `=` and brackets are fine: they are escaped where they are written
    for odd in [
        "a/b\\c %u",
        "x=y [z];",
        "100% real",
        "Name: $HOME",
        "\"quoted\" 'single'",
    ] {
        assert_eq!(
            ok(input(None, Some(odd), false)).custom_name.as_deref(),
            Some(odd)
        );
    }
}

#[test]
fn refused_choices_say_why_in_fixed_sentences() {
    let icon = (
        "invalid_input".to_string(),
        "That icon is not one of the choices.".to_string(),
    );
    for bad in [
        "unknown", "", "Kerf", "kerf ", "../kerf", "kerf\n", "KERF", "original",
    ] {
        assert_eq!(
            err_message(input(Some(bad), None, false).validated()),
            icon,
            "{bad:?}"
        );
    }
    let length = (
        "invalid_input".to_string(),
        "Use 1 to 30 characters for the name.".to_string(),
    );
    assert_eq!(
        err_message(input(None, Some(&"a".repeat(31)), false).validated()),
        length
    );
    assert_eq!(
        err_message(input(None, Some(&"é".repeat(31)), false).validated()),
        length
    );
    let unprintable = (
        "invalid_input".to_string(),
        "The name contains characters that cannot be shown.".to_string(),
    );
    // invisible and reordering characters are refused
    for bad in [
        "a\u{200B}b",
        "\u{200B}",
        "\u{202E}x",
        "a\u{FEFF}",
        "a\u{E000}",
        "a\u{200B}%u Exec=evil",
    ] {
        assert_eq!(
            err_message(input(None, Some(bad), false).validated()),
            unprintable,
            "{bad:?}"
        );
    }
    let ok = |i: DisguiseInput| i.validated().expect("accepted");
    // control characters (a line break above all) are turned into spaces, never kept and never
    // refused: the name can never start a new line wherever it is written
    for (raw, want) in [
        ("a\nb", "a b"),
        ("a\rb", "a b"),
        ("a\tb", "a b"),
        ("a\u{0}b", "a b"),
        ("a\u{7f}b", "a b"),
        ("a\u{85}b", "a b"),
        ("a\u{1b}[31mb", "a [31mb"),
        ("a%u\nExec=evil", "a%u Exec=evil"),
        ("\nKerf\r\n", "Kerf"),
        ("a\r\nb", "a  b"),
    ] {
        assert_eq!(
            ok(input(None, Some(raw), false)).custom_name.as_deref(),
            Some(want),
            "{raw:?}"
        );
    }
    // the 30-character limit counts the name as it will be stored
    assert!(
        ok(input(None, Some(&format!("{}\n", "a".repeat(30))), false))
            .custom_name
            .is_some()
    );
    assert_eq!(
        err_message(input(None, Some(&format!("{}\nb", "a".repeat(30))), false).validated()),
        length
    );
    // the first problem is the icon, whatever the name
    assert_eq!(
        err_message(input(Some("nope"), Some("a\u{200B}b"), false).validated()).1,
        "That icon is not one of the choices."
    );
}

// ------------------------------------------------------------------ the applied name

#[test]
fn applied_name_precedence_and_lowercase() {
    assert_eq!(input(None, None, false).applied_name(), "Hexis");
    assert_eq!(input(None, None, true).applied_name(), "hexis");
    assert_eq!(input(Some("kerf"), None, false).applied_name(), "Kerf");
    assert_eq!(input(Some("kerf"), None, true).applied_name(), "kerf");
    assert_eq!(input(Some("kovren"), None, true).applied_name(), "kovren");
    // a custom name beats the preset's name
    assert_eq!(
        input(Some("kerf"), Some("Ledger"), false).applied_name(),
        "Ledger"
    );
    assert_eq!(
        input(Some("kerf"), Some("Ledger"), true).applied_name(),
        "ledger"
    );
    assert_eq!(input(None, Some("MiXeD"), true).applied_name(), "mixed");
    assert_eq!(input(None, Some("MiXeD"), false).applied_name(), "MiXeD");
    for (p, name) in presets().iter().zip(NAMES) {
        assert_eq!(input(Some(p.id()), None, false).applied_name(), name);
        assert_eq!(
            input(Some(p.id()), None, true).applied_name(),
            name.to_lowercase()
        );
    }
}

#[test]
fn json_shapes_are_exactly_the_frontends() {
    // what the frontend sends
    let i: DisguiseInput =
        serde_json::from_value(json!({"presetId": "kerf", "customName": null, "lowercase": true}))
            .unwrap();
    assert_eq!(i, input(Some("kerf"), None, true));
    let i: DisguiseInput = serde_json::from_value(
        json!({"presetId": null, "customName": "Notes", "lowercase": false}),
    )
    .unwrap();
    assert_eq!(i, input(None, Some("Notes"), false));
    // what it gets back
    let v = serde_json::to_value(input(Some("kerf"), None, true).view(None)).unwrap();
    assert_eq!(
        v,
        json!({"presetId": "kerf", "customName": null, "lowercase": true, "appliedName": "kerf", "note": null})
    );
    let v = serde_json::to_value(DisguiseView::original()).unwrap();
    assert_eq!(
        v,
        json!({"presetId": null, "customName": null, "lowercase": false, "appliedName": "Hexis", "note": null})
    );
    let v = serde_json::to_value(input(None, Some("A"), false).view(Some("hello".into()))).unwrap();
    assert_eq!(v["note"], "hello");
    assert!(DisguiseView::original().is_original());
    assert!(!input(None, None, true).view(None).is_original());
    assert!(!input(Some("kerf"), None, false).view(None).is_original());
    assert!(!input(None, Some("A"), false).view(None).is_original());
}

// ------------------------------------------------------------------ the saved file

#[test]
fn missing_file_means_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load(dir.path()), DisguiseInput::default());
    assert_eq!(
        load(&dir.path().join("does-not-exist")),
        DisguiseInput::default()
    );
}

#[test]
fn save_and_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    for choice in [
        input(None, None, false),
        input(Some("kerf"), None, true),
        input(Some("orvyn"), Some("Ledger"), false),
        input(None, Some("Quiet notes"), true),
    ] {
        save(dir.path(), &choice).unwrap();
        assert_eq!(load(dir.path()), choice);
    }
    // the file says what it is and carries no extra fields
    let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v,
        json!({"version": 1, "presetId": null, "customName": "Quiet notes", "lowercase": true})
    );
    // no temporary file is left behind
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, [FILE_NAME]);
}

#[cfg(unix)]
#[test]
fn the_saved_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), &input(Some("tally"), None, false)).unwrap();
    let mode = std::fs::metadata(dir.path().join(FILE_NAME))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn save_normalizes_and_refuses_what_validation_refuses() {
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), &input(None, Some("  Padded  "), false)).unwrap();
    assert_eq!(load(dir.path()).custom_name.as_deref(), Some("Padded"));
    let before = std::fs::read(dir.path().join(FILE_NAME)).unwrap();
    assert!(save(dir.path(), &input(Some("nope"), None, false)).is_err());
    assert!(save(dir.path(), &input(None, Some("a\u{200B}b"), false)).is_err());
    assert_eq!(std::fs::read(dir.path().join(FILE_NAME)).unwrap(), before);
}

#[test]
fn a_damaged_or_hostile_file_means_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let good = r#"{"version":1,"presetId":"kerf","customName":null,"lowercase":true}"#;
    std::fs::write(&path, good).unwrap();
    assert_eq!(
        load(dir.path()),
        input(Some("kerf"), None, true),
        "control: the good file loads"
    );
    let spaces = " ".repeat(5000);
    let hostile: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("garbage", b"\x00\xff\xfe not json".to_vec()),
        ("truncated", good.as_bytes()[..good.len() - 8].to_vec()),
        ("json null", b"null".to_vec()),
        ("an array", b"[1,2,3]".to_vec()),
        (
            "no version",
            br#"{"presetId":"kerf","customName":null,"lowercase":true}"#.to_vec(),
        ),
        (
            "newer version",
            br#"{"version":2,"presetId":"kerf","customName":null,"lowercase":true}"#.to_vec(),
        ),
        (
            "version as text",
            br#"{"version":"1","presetId":null,"customName":null,"lowercase":false}"#.to_vec(),
        ),
        (
            "unknown preset",
            br#"{"version":1,"presetId":"evil","customName":null,"lowercase":false}"#.to_vec(),
        ),
        (
            "path in preset",
            br#"{"version":1,"presetId":"../kerf","customName":null,"lowercase":false}"#.to_vec(),
        ),
        (
            "zero-width character in name",
            br#"{"version":1,"presetId":null,"customName":"a\u200bExec=evil","lowercase":false}"#
                .to_vec(),
        ),
        (
            "name too long",
            format!(
                r#"{{"version":1,"presetId":null,"customName":"{}","lowercase":false}}"#,
                "x".repeat(31)
            )
            .into_bytes(),
        ),
        (
            "lowercase as text",
            br#"{"version":1,"presetId":null,"customName":null,"lowercase":"yes"}"#.to_vec(),
        ),
        ("oversized", format!("{good}{spaces}").into_bytes()),
    ];
    for (what, bytes) in hostile {
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(load(dir.path()), DisguiseInput::default(), "{what}");
    }
    // a folder where the file should be
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_eq!(load(dir.path()), DisguiseInput::default(), "a folder");
}

// ------------------------------------------------------------------ the launcher seam

#[test]
fn no_launcher_does_nothing_and_says_nothing() {
    let v = DisguiseView::original();
    assert_eq!(NoLauncher.apply(&v, None).unwrap(), None);
    assert_eq!(NoLauncher.standing_note(), None);
}

#[test]
fn recording_launcher_records_and_can_be_told_to_fail() {
    let r = RecordingLauncher::new();
    assert_eq!(r.count(), 0);
    assert_eq!(r.last(), None);
    let choice = input(Some("kerf"), None, true);
    assert_eq!(r.apply(&choice.view(None), preset("kerf")).unwrap(), None);
    assert_eq!(r.apply(&DisguiseView::original(), None).unwrap(), None);
    assert_eq!(r.count(), 2);
    let calls = r.calls();
    assert_eq!(calls[0].preset_id.as_deref(), Some("kerf"));
    assert_eq!(calls[0].view.applied_name, "kerf");
    assert_eq!(calls[1].preset_id, None);
    assert_eq!(r.last(), Some(calls[1].clone()));
    let v = serde_json::to_value(&calls[0]).unwrap();
    assert_eq!(v["presetId"], "kerf");
    assert_eq!(v["view"]["appliedName"], "kerf");

    let noted = RecordingLauncher::new().with_note("hello");
    assert_eq!(
        noted
            .apply(&DisguiseView::original(), None)
            .unwrap()
            .as_deref(),
        Some("hello")
    );
    let failing = RecordingLauncher::new().failing("nope");
    let e = failing.apply(&DisguiseView::original(), None).unwrap_err();
    assert_eq!((e.code(), e.to_string().as_str()), ("storage", "nope"));
    assert_eq!(failing.count(), 1, "a failed call is still recorded");
}

// ------------------------------------------------------------------ the wipe hook

#[test]
fn wipe_hook_erases_the_file_and_resets_the_launcher() {
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), &input(Some("kerf"), Some("Ledger"), true)).unwrap();
    assert!(dir.path().join(FILE_NAME).exists());
    let launcher = RecordingLauncher::new();
    wipe_hook(dir.path(), &launcher).unwrap();
    assert!(!dir.path().join(FILE_NAME).exists());
    assert_eq!(load(dir.path()), DisguiseInput::default());
    let last = launcher.last().expect("the launcher was asked to reset");
    assert_eq!(last.view, DisguiseView::original());
    assert_eq!(last.preset_id, None);
    // nothing to erase is fine too
    wipe_hook(dir.path(), &launcher).unwrap();
    assert_eq!(launcher.count(), 2);
}

#[test]
fn wipe_hook_reports_a_launcher_failure_after_erasing_the_file() {
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), &input(Some("kerf"), None, false)).unwrap();
    let launcher = RecordingLauncher::new().failing("launcher is stuck");
    let e = wipe_hook(dir.path(), &launcher).unwrap_err();
    assert_eq!(e.to_string(), "launcher is stuck");
    assert!(
        !dir.path().join(FILE_NAME).exists(),
        "the file is gone anyway"
    );
}

#[test]
fn shredding_overwrites_before_unlinking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret.json");
    std::fs::write(&path, b"{\"customName\":\"visible\"}").unwrap();
    // keep a second name for the same inode: after the shred it must read as zeros
    let alias = dir.path().join("alias.json");
    std::fs::hard_link(&path, &alias).unwrap();
    shred_file(&path).unwrap();
    assert!(!path.exists());
    let left = std::fs::read(&alias).unwrap();
    assert_eq!(left.len(), 24);
    assert!(
        left.iter().all(|b| *b == 0),
        "the bytes were overwritten, not just unlinked"
    );
    shred_file(&path).unwrap(); // absence is fine
}
