//! Tests of the Linux launcher entry writer. Every test works in a temp folder handed to the
//! writer as its `<data home>`; the user's real `~/.local/share` is never touched.

use super::*;
use crate::disguise::{preset, DisguiseInput, ORIGINAL_ICON_PNG};

const ID: &str = "app.passman.vault";

fn view(preset_id: Option<&str>, name: Option<&str>, lowercase: bool) -> DisguiseView {
    DisguiseInput {
        preset_id: preset_id.map(str::to_string),
        custom_name: name.map(str::to_string),
        lowercase,
    }
    .view(None)
}

fn files(base: &Path) -> LauncherFiles {
    LauncherFiles::new(base, ID, Path::new("/opt/passman/passman"), "passman")
        .unwrap()
        .with_refresh(false)
}

/// Split an entry into `(key, raw value)` pairs and check its overall shape: the group header
/// first, then only `Key=Value` lines, no blank lines, no second group, no duplicate key.
fn parse(text: &str) -> Vec<(String, String)> {
    assert!(text.ends_with('\n'), "ends with a newline");
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("[Desktop Entry]"));
    let mut out: Vec<(String, String)> = Vec::new();
    for line in lines {
        assert!(!line.starts_with('['), "no second group: {line:?}");
        let (k, v) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("not a Key=Value line: {line:?}"));
        assert!(
            !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "bad key {k:?}"
        );
        assert!(out.iter().all(|(seen, _)| seen != k), "duplicate key {k}");
        out.push((k.to_string(), v.to_string()));
    }
    let keys: Vec<&str> = out.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "Type",
            "Name",
            "Comment",
            "Exec",
            "Icon",
            "Terminal",
            "Categories",
            "StartupWMClass",
            "X-Passman-Managed"
        ]
    );
    out
}

/// Undo the general string escape of the Desktop Entry specification (the reader's side).
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('s') => out.push(' '),
            other => panic!("unknown escape \\{other:?} in {s:?}"),
        }
    }
    out
}

fn value<'a>(entries: &'a [(String, String)], key: &str) -> &'a str {
    &entries.iter().find(|(k, _)| k == key).expect("key").1
}

// ------------------------------------------------------------------ the text

#[test]
fn the_entry_text_is_exact() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    let text = f.entry_text(&view(Some("kerf"), None, true), preset("kerf"));
    let expected = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=kerf\n\
         Comment=Cursor and dot\n\
         Exec=/opt/passman/passman\n\
         Icon={}/{ID}/disguise/kerf.png\n\
         Terminal=false\n\
         Categories=Utility;\n\
         StartupWMClass=passman\n\
         X-Passman-Managed=true\n",
        base.path().display()
    );
    assert_eq!(text, expected);
    parse(&text);
}

#[test]
fn a_custom_name_with_the_original_icon_uses_the_bundled_icon() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    let text = f.entry_text(&view(None, Some("Ledger"), false), None);
    let e = parse(&text);
    assert_eq!(value(&e, "Name"), "Ledger");
    assert_eq!(value(&e, "Comment"), "", "nothing to say without a preset");
    assert_eq!(
        value(&e, "Icon"),
        format!("{}/{ID}/disguise/original.png", base.path().display())
    );
    // a custom name over a preset still describes the preset's icon
    let text = f.entry_text(&view(Some("tally"), Some("Ledger"), false), preset("tally"));
    let e = parse(&text);
    assert_eq!(value(&e, "Name"), "Ledger");
    assert_eq!(value(&e, "Comment"), "Minimal grid");
}

#[test]
fn every_preset_renders_a_well_formed_entry() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    for p in crate::disguise::presets() {
        let v = view(Some(p.id()), None, false);
        let e = parse(&f.entry_text(&v, Some(p)));
        assert_eq!(value(&e, "Name"), p.preset.name);
        assert_eq!(value(&e, "Comment"), p.preset.description);
        assert!(value(&e, "Icon").ends_with(&format!("/disguise/{}.png", p.id())));
    }
}

#[test]
fn escape_value_keeps_one_line() {
    assert_eq!(escape_value("plain"), "plain");
    assert_eq!(escape_value("a\\b"), "a\\\\b");
    assert_eq!(escape_value("a\nb"), "a\\nb");
    assert_eq!(escape_value("a\r\nb\tc"), "a\\r\\nb\\tc");
    assert_eq!(escape_value("a\u{0}b\u{1b}c\u{7f}d"), "a b c d");
    assert_eq!(
        escape_value("100% é ;= [x] \"q\" 'q'"),
        "100% é ;= [x] \"q\" 'q'"
    );
    for s in [
        "a\nb",
        "\n\n",
        "x\r",
        "[Desktop Entry]\nExec=evil",
        "\u{85}\u{2028}x",
    ] {
        assert!(!escape_value(s).contains(['\n', '\r']), "{s:?}");
    }
}

#[test]
fn exec_value_follows_the_quoting_rules() {
    let cases = [
        ("/usr/bin/passman", "/usr/bin/passman"),
        ("/opt/My App/passman", "\"/opt/My App/passman\""),
        ("/opt/100%/passman", "/opt/100%%/passman"),
        ("/opt/a b%u/p", "\"/opt/a b%%u/p\""),
        ("/opt/q\"x/p", "\"/opt/q\\\\\"x/p\""),
        ("/opt/$HOME/p", "\"/opt/\\\\$HOME/p\""),
        ("/opt/a`b/p", "\"/opt/a\\\\`b/p\""),
        ("/opt/a\\b/p", "\"/opt/a\\\\\\\\b/p\""),
        ("/opt/it's/p", "\"/opt/it's/p\""),
        ("/opt/(x)/p", "\"/opt/(x)/p\""),
        ("/opt/a;b/p", "\"/opt/a;b/p\""),
        ("/opt/a\nb/p", "\"/opt/a\\nb/p\""),
        ("/opt/é/p", "/opt/é/p"),
    ];
    for (path, want) in cases {
        assert_eq!(exec_value(path), want, "{path:?}");
    }
}

// ------------------------------------------------------------------ injection

#[test]
fn hostile_names_cannot_add_lines_or_keys() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    // these bypass the validation on purpose: the writer must hold on its own
    let hostile = [
        "a%u\nExec=evil",
        "a%u\r\nExec=evil",
        "x\n[Desktop Action Evil]\nExec=evil",
        "x\nX-Passman-Managed=false",
        "\nName=second",
        "tab\there",
        "a\\nExec=evil",
        "a\\\nExec=evil",
        "%u %F %k",
    ];
    for name in hostile {
        let mut v = view(None, Some("x"), false);
        v.applied_name = name.to_string();
        let text = f.entry_text(&v, None);
        let e = parse(&text);
        assert_eq!(
            text.lines().filter(|l| l.starts_with("Exec=")).count(),
            1,
            "exactly one Exec line for {name:?}"
        );
        assert_eq!(
            text.lines().count(),
            10,
            "same number of lines for {name:?}"
        );
        assert_eq!(unescape(value(&e, "Name")), name, "reads back unchanged");
        assert_eq!(value(&e, "Exec"), "/opt/passman/passman");
        assert_eq!(value(&e, "X-Passman-Managed"), "true");
    }
    // the literal two characters backslash and n (what a person can type) stay two characters
    let mut v = view(None, Some("x"), false);
    v.applied_name = "a%u\\nExec=evil".to_string();
    let text = f.entry_text(&v, None);
    assert!(text.contains("\nName=a%u\\\\nExec=evil\n"), "{text}");
}

#[test]
fn a_hostile_program_path_or_class_cannot_add_lines_either() {
    let base = tempfile::tempdir().unwrap();
    let f = LauncherFiles::new(
        base.path(),
        ID,
        Path::new("/opt/x\nExec=evil\n[Desktop Action A]/p"),
        "cls\nExec=evil",
    )
    .unwrap();
    let text = f.entry_text(&view(Some("kerf"), None, false), preset("kerf"));
    let e = parse(&text);
    assert_eq!(text.lines().filter(|l| l.starts_with("Exec=")).count(), 1);
    assert_eq!(text.lines().count(), 10);
    assert_eq!(unescape(value(&e, "StartupWMClass")), "cls\nExec=evil");
    // the whole path is one quoted argument
    assert!(value(&e, "Exec").starts_with('"') && value(&e, "Exec").ends_with('"'));
}

#[test]
fn identifiers_and_paths_are_checked() {
    let base = tempfile::tempdir().unwrap();
    let exe = Path::new("/opt/p/passman");
    for bad in [
        "",
        "../x",
        "a/b",
        "a\\b",
        ".hidden",
        "-x",
        "a b",
        "a..b",
        "a\nb",
        "x.desktop/..",
        "é",
    ] {
        assert!(
            LauncherFiles::new(base.path(), bad, exe, "passman").is_err(),
            "identifier {bad:?}"
        );
        assert!(
            remove_launcher_entry(base.path(), bad).is_err(),
            "remove {bad:?}"
        );
    }
    assert!(LauncherFiles::new(Path::new("relative/dir"), ID, exe, "passman").is_err());
    assert!(LauncherFiles::new(base.path(), ID, Path::new("relative/passman"), "passman").is_err());
    assert!(LauncherFiles::new(base.path(), ID, exe, "").is_err());
    assert!(LauncherFiles::new(base.path(), ID, exe, "  ").is_err());
    for ok in ["app.passman.vault", "passman", "a-b_c.d", "A1"] {
        assert!(
            LauncherFiles::new(base.path(), ok, exe, "passman").is_ok(),
            "{ok}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let not_text = Path::new(std::ffi::OsStr::from_bytes(b"/opt/\xff/passman"));
        assert!(LauncherFiles::new(base.path(), ID, not_text, "passman").is_err());
    }
    // nothing was created by the failed attempts
    assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 0);
}

// ------------------------------------------------------------------ writing

#[test]
fn write_creates_the_entry_and_the_icon_inside_the_given_folder() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    let v = view(Some("kerf"), None, true);
    assert_eq!(f.write(&v, preset("kerf")).unwrap(), Effect::Written);

    let entry = base
        .path()
        .join("applications")
        .join(format!("{ID}.desktop"));
    let icon = base.path().join(ID).join("disguise").join("kerf.png");
    assert_eq!(f.entry_path(), entry);
    assert_eq!(f.icon_dir(), icon.parent().unwrap());
    assert_eq!(f.applications_dir(), base.path().join("applications"));
    assert_eq!(
        std::fs::read_to_string(&entry).unwrap(),
        f.entry_text(&v, preset("kerf"))
    );
    assert_eq!(
        std::fs::read(&icon).unwrap(),
        preset("kerf").unwrap().png_512
    );

    // the icon path in the entry is absolute and is the file that was written
    let text = std::fs::read_to_string(&entry).unwrap();
    let e = parse(&text);
    let icon_value = Path::new(value(&e, "Icon"));
    assert!(icon_value.is_absolute());
    assert_eq!(icon_value, icon);

    // public files (0644) and no temporary file left
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&entry, &icon] {
            let mode = std::fs::metadata(p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o644, "{}", p.display());
        }
    }
    for dir in [entry.parent().unwrap(), icon.parent().unwrap()] {
        for e in std::fs::read_dir(dir).unwrap() {
            let name = e.unwrap().file_name();
            assert!(!name.to_string_lossy().ends_with(".tmp"), "{name:?}");
        }
    }
    // and everything stayed under the base
    let mut all = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            }
            out.push(p);
        }
    }
    walk(base.path(), &mut all);
    assert_eq!(
        all.len(),
        5,
        "applications, the entry, the app's folder, its disguise folder and the icon: {all:?}"
    );
}

#[test]
fn writing_the_same_look_twice_changes_nothing() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    let v = view(Some("orvyn"), Some("Ledger"), false);
    assert_eq!(f.write(&v, preset("orvyn")).unwrap(), Effect::Written);
    let before: Vec<_> = [f.entry_path(), f.icon_dir().join("orvyn.png")]
        .iter()
        .map(|p| {
            (
                std::fs::read(p).unwrap(),
                std::fs::metadata(p).unwrap().modified().unwrap(),
            )
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(f.write(&v, preset("orvyn")).unwrap(), Effect::Unchanged);
    assert_eq!(f.sync(&v, preset("orvyn")).unwrap(), Effect::Unchanged);
    let after: Vec<_> = [f.entry_path(), f.icon_dir().join("orvyn.png")]
        .iter()
        .map(|p| {
            (
                std::fs::read(p).unwrap(),
                std::fs::metadata(p).unwrap().modified().unwrap(),
            )
        })
        .collect();
    assert_eq!(before, after, "same bytes and same modification time");
}

#[test]
fn a_damaged_entry_or_icon_is_repaired() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    let v = view(Some("tally"), None, false);
    f.write(&v, preset("tally")).unwrap();
    std::fs::write(f.icon_dir().join("tally.png"), b"damaged").unwrap();
    let mut text = std::fs::read_to_string(f.entry_path()).unwrap();
    text.push_str("Exec=appended-by-someone\n");
    std::fs::write(f.entry_path(), text).unwrap();
    assert_eq!(f.write(&v, preset("tally")).unwrap(), Effect::Written);
    assert_eq!(
        std::fs::read(f.icon_dir().join("tally.png")).unwrap(),
        preset("tally").unwrap().png_512
    );
    assert_eq!(
        std::fs::read_to_string(f.entry_path()).unwrap(),
        f.entry_text(&v, preset("tally"))
    );
}

#[test]
fn changing_the_look_replaces_the_icon_and_keeps_foreign_files() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    f.write(&view(Some("tally"), None, false), preset("tally"))
        .unwrap();
    // somebody else's files in our icon folder
    std::fs::write(f.icon_dir().join("notes.txt"), b"mine").unwrap();
    std::fs::write(f.icon_dir().join("other.png"), b"mine too").unwrap();
    assert_eq!(
        f.write(&view(Some("kerf"), None, false), preset("kerf"))
            .unwrap(),
        Effect::Written
    );
    assert!(
        !f.icon_dir().join("tally.png").exists(),
        "the old look's icon is gone"
    );
    assert!(f.icon_dir().join("kerf.png").exists());
    assert_eq!(
        std::fs::read(f.icon_dir().join("notes.txt")).unwrap(),
        b"mine"
    );
    assert_eq!(
        std::fs::read(f.icon_dir().join("other.png")).unwrap(),
        b"mine too"
    );
    // to the original icon with a name of one's own
    f.write(&view(None, Some("Ledger"), false), None).unwrap();
    assert!(!f.icon_dir().join("kerf.png").exists());
    assert_eq!(
        std::fs::read(f.icon_dir().join(ORIGINAL_ICON_FILE)).unwrap(),
        ORIGINAL_ICON_PNG
    );
}

#[test]
fn sync_removes_the_entry_for_the_original_look_and_writes_it_otherwise() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    let original = view(None, None, false);
    assert_eq!(
        f.sync(&original, None).unwrap(),
        Effect::Unchanged,
        "nothing to remove"
    );
    assert!(!f.entry_path().exists());

    assert_eq!(
        f.sync(&view(Some("kerf"), None, true), preset("kerf"))
            .unwrap(),
        Effect::Written
    );
    assert!(f.entry_path().exists());
    assert_eq!(f.sync(&original, None).unwrap(), Effect::Removed);
    assert!(!f.entry_path().exists());
    assert!(!f.icon_dir().exists(), "the empty icon folder went too");
    assert_eq!(
        f.sync(&original, None).unwrap(),
        Effect::Unchanged,
        "idempotent"
    );

    // lower-case alone is a disguise: it writes an entry with the original icon
    assert_eq!(
        f.sync(&view(None, None, true), None).unwrap(),
        Effect::Written
    );
    let e = parse(&std::fs::read_to_string(f.entry_path()).unwrap());
    assert_eq!(value(&e, "Name"), "passman");
    assert!(f.icon_dir().join(ORIGINAL_ICON_FILE).exists());
}

// ------------------------------------------------------------------ removal and ownership

#[test]
fn removing_what_is_not_there_is_fine() {
    let base = tempfile::tempdir().unwrap();
    assert_eq!(
        remove_launcher_entry(base.path(), ID).unwrap(),
        Effect::Unchanged
    );
    assert_eq!(
        remove_launcher_entry(&base.path().join("missing"), ID).unwrap(),
        Effect::Unchanged
    );
    assert_eq!(
        std::fs::read_dir(base.path()).unwrap().count(),
        0,
        "nothing was created"
    );
}

#[test]
fn remove_deletes_only_what_this_module_wrote() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    f.write(&view(Some("kerf"), None, false), preset("kerf"))
        .unwrap();
    // neighbours that must survive
    let apps = base.path().join("applications");
    std::fs::write(
        apps.join("firefox.desktop"),
        b"[Desktop Entry]\nName=Firefox\n",
    )
    .unwrap();
    std::fs::write(apps.join(format!("{ID}.desktop.bak")), b"backup").unwrap();
    std::fs::write(base.path().join(ID).join("profiles.json"), b"{}").unwrap();
    std::fs::create_dir(base.path().join(ID).join("vaults")).unwrap();
    std::fs::create_dir(base.path().join("icons")).unwrap();
    std::fs::write(base.path().join("icons").join("kerf.png"), b"not ours").unwrap();

    assert_eq!(f.remove().unwrap(), Effect::Removed);
    assert!(!f.entry_path().exists());
    assert!(!f.icon_dir().exists());
    assert!(apps.join("firefox.desktop").exists());
    assert!(apps.join(format!("{ID}.desktop.bak")).exists());
    assert!(base.path().join(ID).join("profiles.json").exists());
    assert!(base.path().join(ID).join("vaults").is_dir());
    assert!(base.path().join("icons").join("kerf.png").exists());
}

#[test]
fn remove_keeps_the_icon_folder_when_it_holds_something_else() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    f.write(&view(Some("kerf"), None, false), preset("kerf"))
        .unwrap();
    std::fs::write(f.icon_dir().join("keep-me.txt"), b"x").unwrap();
    assert_eq!(f.remove().unwrap(), Effect::Removed);
    assert!(f.icon_dir().join("keep-me.txt").exists());
    assert!(!f.icon_dir().join("kerf.png").exists());
}

#[test]
fn an_entry_that_is_not_ours_is_neither_overwritten_nor_deleted() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    std::fs::create_dir_all(f.applications_dir()).unwrap();
    let foreign = "[Desktop Entry]\nType=Application\nName=Mine\nExec=mine\n";
    std::fs::write(f.entry_path(), foreign).unwrap();

    let e = f
        .write(&view(Some("kerf"), None, false), preset("kerf"))
        .expect_err("refused");
    assert_eq!(e.code(), "storage");
    assert!(
        e.to_string().contains("did not create"),
        "a fixed sentence without a path: {e}"
    );
    assert!(!e.to_string().contains(&base.path().display().to_string()));
    assert_eq!(std::fs::read_to_string(f.entry_path()).unwrap(), foreign);
    assert!(!f.icon_dir().exists(), "nothing else was written either");

    assert_eq!(f.remove().unwrap(), Effect::Unchanged);
    assert_eq!(std::fs::read_to_string(f.entry_path()).unwrap(), foreign);
    assert_eq!(
        f.sync(&view(None, None, false), None).unwrap(),
        Effect::Unchanged,
        "the original look does not delete a foreign entry"
    );
    assert!(f.entry_path().exists());
}

#[test]
fn a_folder_or_unreadable_thing_in_the_entry_place_counts_as_foreign() {
    let base = tempfile::tempdir().unwrap();
    let f = files(base.path());
    std::fs::create_dir_all(f.entry_path()).unwrap();
    assert!(f
        .write(&view(Some("kerf"), None, false), preset("kerf"))
        .is_err());
    assert_eq!(f.remove().unwrap(), Effect::Unchanged);
    assert!(f.entry_path().is_dir());
}

// ------------------------------------------------------------------ where the entry points

#[test]
fn exec_target_prefers_an_existing_appimage() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("Passman.AppImage");
    std::fs::write(&image, b"x").unwrap();
    let exe = Path::new("/tmp/.mount_PassmaXYZ/usr/bin/passman");
    assert_eq!(exec_target(exe, Some(image.as_os_str())), image);
    // missing, relative, empty or unset: the running executable
    for bad in [
        dir.path().join("gone.AppImage").into_os_string(),
        std::ffi::OsString::from("Passman.AppImage"),
        std::ffi::OsString::new(),
        dir.path().as_os_str().to_os_string(), // a folder, not a file
    ] {
        assert_eq!(exec_target(exe, Some(bad.as_os_str())), exe, "{bad:?}");
    }
    assert_eq!(exec_target(exe, None), exe);
    // a binary replaced while running
    assert_eq!(
        exec_target(Path::new("/usr/bin/passman (deleted)"), None),
        Path::new("/usr/bin/passman")
    );
}

#[test]
fn the_window_class_is_the_executable_name() {
    assert_eq!(wm_class_of(Path::new("/usr/bin/passman")), "passman");
    assert_eq!(wm_class_of(Path::new("/opt/p/Passman-Dev")), "Passman-Dev");
    assert_eq!(
        wm_class_of(Path::new("/usr/bin/passman (deleted)")),
        "passman"
    );
    assert_eq!(wm_class_of(Path::new("/")), "");
}

// ------------------------------------------------------------------ refreshing the database

#[test]
fn refreshing_the_desktop_database_never_fails_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!spawn_refresh(
        "definitely-not-installed-passman-xyz",
        dir.path()
    ));
    // a program that exits with an error is not our problem either
    if std::process::Command::new("false").status().is_ok() {
        assert!(spawn_refresh("false", dir.path()));
    }
    // the writer with refresh switched on does not mind a missing or failing tool
    let f = LauncherFiles::new(dir.path(), ID, Path::new("/opt/p/passman"), "passman").unwrap();
    f.sync(&view(Some("kerf"), None, false), preset("kerf"))
        .unwrap();
    f.sync(&view(None, None, false), None).unwrap();
}

// ------------------------------------------------------------------ an independent validator

/// `desktop-file-validate` (desktop-file-utils) is an independent check of the entry's syntax.
/// Skipped, with a printed note, when it is not installed.
#[test]
fn desktop_file_validate_accepts_what_is_written() {
    let probe = std::process::Command::new("desktop-file-validate")
        .arg("--help")
        .output();
    if probe.is_err() {
        eprintln!("SKIPPED: desktop-file-validate is not installed");
        return;
    }
    let base = tempfile::tempdir().unwrap();
    // an executable that exists, as the validator checks the Exec target
    let exe = std::env::current_exe().unwrap();
    let f = LauncherFiles::new(base.path(), ID, &exe, &wm_class_of(&exe))
        .unwrap()
        .with_refresh(false);
    for (v, p) in [
        (view(Some("kerf"), None, true), preset("kerf")),
        (
            view(Some("tally"), Some("Ledger 100% /x"), false),
            preset("tally"),
        ),
        (view(None, Some("a%u\\nExec=evil"), false), None),
        (view(None, None, true), None),
    ] {
        f.write(&v, p).unwrap();
        let out = std::process::Command::new("desktop-file-validate")
            .arg(f.entry_path())
            .output()
            .unwrap();
        assert!(
            out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
            "desktop-file-validate rejected {:?}: {}{}",
            v.applied_name,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
