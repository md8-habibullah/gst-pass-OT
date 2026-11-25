//! The browser extension, carried inside the program and written to the data folder when Hexis registers with a browser.
//!
//! The extension is a few small files (`extension/` in the repository). Hexis keeps them inside itself and writes them,
//! one copy per browser family, to `<data folder>/extension/chromium` and `<data folder>/extension/firefox`, so the page in
//! Settings can name a folder the person loads in the browser, and the extension always matches the program that wrote it.
//! Nothing is fetched from anywhere.

use std::io;
use std::path::{Path, PathBuf};

/// One file of the extension: its name in the folder and its content.
struct Part {
    name: &'static str,
    bytes: &'static [u8],
}

const SHARED: [Part; 6] = [
    Part {
        name: "page.js",
        bytes: include_bytes!("../../../extension/src/page.js"),
    },
    Part {
        name: "content.js",
        bytes: include_bytes!("../../../extension/src/content.js"),
    },
    Part {
        name: "background.js",
        bytes: include_bytes!("../../../extension/src/background.js"),
    },
    Part {
        name: "icon-32.png",
        bytes: include_bytes!("../../../extension/icons/icon-32.png"),
    },
    Part {
        name: "icon-64.png",
        bytes: include_bytes!("../../../extension/icons/icon-64.png"),
    },
    Part {
        name: "icon-128.png",
        bytes: include_bytes!("../../../extension/icons/icon-128.png"),
    },
];

const CHROMIUM_MANIFEST: &[u8] = include_bytes!("../../../extension/manifest.chromium.json");
const FIREFOX_MANIFEST: &[u8] = include_bytes!("../../../extension/manifest.firefox.json");

/// Where the two copies live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    pub chromium: PathBuf,
    pub firefox: PathBuf,
}

pub fn folders(data_dir: &Path) -> Folders {
    let base = data_dir.join("extension");
    Folders {
        chromium: base.join("chromium"),
        firefox: base.join("firefox"),
    }
}

fn write_one(dir: &Path, manifest: &[u8]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let files = SHARED
        .iter()
        .map(|p| (p.name, p.bytes))
        .chain(std::iter::once(("manifest.json", manifest)));
    for (name, bytes) in files {
        let path = dir.join(name);
        // a file that is already right is left alone (its time does not move), a different one is replaced whole
        if std::fs::read(&path).is_ok_and(|have| have == bytes) {
            continue;
        }
        let tmp = dir.join(format!("{name}.tmp"));
        super::write_new(&tmp, bytes, 0o644)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(())
}

/// Write both copies. Safe to call again: it makes the folders match this program.
pub fn write(data_dir: &Path) -> io::Result<Folders> {
    let f = folders(data_dir);
    write_one(&f.chromium, CHROMIUM_MANIFEST)?;
    write_one(&f.firefox, FIREFOX_MANIFEST)?;
    Ok(f)
}

/// Do both copies exist and match this program?
pub fn present(data_dir: &Path) -> bool {
    let f = folders(data_dir);
    let ok = |dir: &Path, manifest: &[u8]| {
        SHARED
            .iter()
            .map(|p| (p.name, p.bytes))
            .chain(std::iter::once(("manifest.json", manifest)))
            .all(|(name, bytes)| std::fs::read(dir.join(name)).is_ok_and(|have| have == bytes))
    };
    ok(&f.chromium, CHROMIUM_MANIFEST) && ok(&f.firefox, FIREFOX_MANIFEST)
}

/// Remove both copies, and the folder around them when nothing else is in it.
pub fn remove(data_dir: &Path) {
    let base = data_dir.join("extension");
    let f = folders(data_dir);
    for dir in [&f.chromium, &f.firefox] {
        for part in SHARED.iter().map(|p| p.name).chain(["manifest.json"]) {
            let _ = std::fs::remove_file(dir.join(part));
            let _ = std::fs::remove_file(dir.join(format!("{part}.tmp")));
        }
        let _ = std::fs::remove_dir(dir);
    }
    let _ = std::fs::remove_dir(base);
}
