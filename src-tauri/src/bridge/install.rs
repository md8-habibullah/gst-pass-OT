//! Telling browsers where the Hexis native host is (Linux).
//!
//! A browser starts a native host only if a small JSON file, the *native messaging manifest*, names it and names the
//! extensions that may start it. Hexis writes those files for the browsers the person picks (Settings > Browsers) and
//! removes them again: when asked, and when the app is removed from the device. Everything is per user, in the
//! browser's own configuration folder under the home folder; nothing is written system-wide.
//!
//! What gets written, and nothing else:
//! * `<data folder>/hexis-native-host`, a two-line launcher script that starts this program with `--native-host` (a
//!   manifest cannot pass arguments, and an AppImage has no fixed path, so the launcher is the file it points at);
//! * one `dev.habibullah.hexis.passkeys.json` in each chosen browser's `NativeMessagingHosts` folder.

use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};

/// The name a browser knows the host by. Lower case, digits, `_` and `.` only (a browser rule).
pub const NATIVE_HOST_NAME: &str = "dev.habibullah.hexis.passkeys";
/// The launcher script, in the app's data folder.
pub const LAUNCHER_NAME: &str = "hexis-native-host";
/// The address of the Chromium-family extension (derived from the public key in `extension/manifest.chromium.json`; a test
/// keeps the two equal).
pub const CHROMIUM_EXTENSION_ID: &str = super::ids::CHROMIUM;
/// The Firefox extension's id (`browser_specific_settings.gecko.id` in `extension/manifest.firefox.json`).
pub const FIREFOX_EXTENSION_ID: &str = super::ids::FIREFOX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Family {
    Chromium,
    Firefox,
}

/// One browser Hexis knows how to register with.
#[derive(Debug, Clone, Copy)]
pub struct Browser {
    pub id: &'static str,
    pub name: &'static str,
    pub family: Family,
    /// The browser's own configuration folder, below the home folder. It exists once the browser has been used.
    pub config: &'static str,
    /// Where the manifest goes: below the home folder.
    pub hosts: &'static str,
}

pub const BROWSERS: [Browser; 6] = [
    Browser {
        id: "firefox",
        name: "Firefox",
        family: Family::Firefox,
        config: ".mozilla/firefox",
        hosts: ".mozilla/native-messaging-hosts",
    },
    Browser {
        id: "chrome",
        name: "Google Chrome",
        family: Family::Chromium,
        config: ".config/google-chrome",
        hosts: ".config/google-chrome/NativeMessagingHosts",
    },
    Browser {
        id: "chromium",
        name: "Chromium",
        family: Family::Chromium,
        config: ".config/chromium",
        hosts: ".config/chromium/NativeMessagingHosts",
    },
    Browser {
        id: "brave",
        name: "Brave",
        family: Family::Chromium,
        config: ".config/BraveSoftware/Brave-Browser",
        hosts: ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts",
    },
    Browser {
        id: "edge",
        name: "Microsoft Edge",
        family: Family::Chromium,
        config: ".config/microsoft-edge",
        hosts: ".config/microsoft-edge/NativeMessagingHosts",
    },
    Browser {
        id: "vivaldi",
        name: "Vivaldi",
        family: Family::Chromium,
        config: ".config/vivaldi",
        hosts: ".config/vivaldi/NativeMessagingHosts",
    },
];

/// Where the browsers run in a sandbox that cannot start a program outside it, so the host cannot work (a Snap or Flatpak
/// browser). Looked for so the page can say why a browser does not work instead of offering a switch that does nothing.
const SANDBOXED: [(&str, &str); 4] = [
    ("Firefox (Snap)", "snap/firefox"),
    ("Chromium (Snap)", "snap/chromium"),
    ("Firefox (Flatpak)", ".var/app/org.mozilla.firefox"),
    ("Chrome (Flatpak)", ".var/app/com.google.Chrome"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub family: Family,
    /// The browser has a configuration folder here, so it is installed and has been started.
    pub found: bool,
    /// Hexis is registered with it and the registration points at this program.
    pub registered: bool,
    /// A registration exists but points somewhere else (the program moved, or an older install): registering again fixes it.
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsersStatus {
    pub browsers: Vec<BrowserStatus>,
    /// Browsers found that run in a sandbox and so cannot use Hexis.
    pub sandboxed: Vec<String>,
}

/// The program the launcher starts: the AppImage file when running from one (its inside moves every start), else this program.
pub fn current_program() -> io::Result<PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
        if appimage.is_absolute() && appimage.is_file() {
            return Ok(appimage);
        }
    }
    std::env::current_exe()
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

pub fn launcher_text(program: &Path) -> String {
    format!(
        "#!/bin/sh\nexec {} --native-host \"$@\"\n",
        shell_quote(program)
    )
}

pub fn manifest_text(browser: &Browser, launcher: &Path) -> String {
    let mut m = serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "Lets the Hexis extension ask the Hexis app for passkeys",
        "path": launcher.to_string_lossy(),
        "type": "stdio",
    });
    match browser.family {
        Family::Chromium => {
            m["allowed_origins"] =
                serde_json::json!([format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")]);
        }
        Family::Firefox => {
            m["allowed_extensions"] = serde_json::json!([FIREFOX_EXTENSION_ID]);
        }
    }
    serde_json::to_string_pretty(&m).expect("a manifest serializes") + "\n"
}

fn manifest_path(home: &Path, browser: &Browser) -> PathBuf {
    home.join(browser.hosts)
        .join(format!("{NATIVE_HOST_NAME}.json"))
}

fn launcher_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LAUNCHER_NAME)
}

/// What is registered, for the Settings page.
pub fn status(home: &Path, data_dir: &Path) -> BrowsersStatus {
    let launcher = launcher_path(data_dir);
    let browsers = BROWSERS
        .iter()
        .map(|b| {
            let (registered, stale) = match std::fs::read_to_string(manifest_path(home, b)) {
                Ok(text) => {
                    let ours = text == manifest_text(b, &launcher) && launcher.is_file();
                    (ours, !ours)
                }
                Err(_) => (false, false),
            };
            BrowserStatus {
                id: b.id,
                name: b.name,
                family: b.family,
                found: home.join(b.config).is_dir(),
                registered,
                stale,
            }
        })
        .collect();
    let sandboxed = SANDBOXED
        .iter()
        .filter(|(_, dir)| home.join(dir).is_dir())
        .map(|(name, _)| (*name).to_string())
        .collect();
    BrowsersStatus {
        browsers,
        sandboxed,
    }
}

fn write_private(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // written beside the target and renamed over it, so a browser never reads half a file
    let tmp = path.with_extension("tmp");
    super::write_new(&tmp, text.as_bytes(), mode)?;
    std::fs::rename(&tmp, path)
}

/// Register Hexis with the chosen browsers (by [`Browser::id`]); unknown ids and browsers that were never started are skipped
/// and reported. Returns the ids that were registered.
pub fn install(
    home: &Path,
    data_dir: &Path,
    program: &Path,
    ids: &[String],
) -> io::Result<Vec<&'static str>> {
    let launcher = launcher_path(data_dir);
    let mut done = Vec::new();
    for b in BROWSERS.iter().filter(|b| ids.iter().any(|i| i == b.id)) {
        if !home.join(b.config).is_dir() {
            continue;
        }
        if done.is_empty() {
            write_private(&launcher, &launcher_text(program), 0o700)?;
        }
        write_private(&manifest_path(home, b), &manifest_text(b, &launcher), 0o644)?;
        done.push(b.id);
    }
    Ok(done)
}

/// Take Hexis away from the browsers. `ids` empty means every browser. The launcher goes when no manifest of ours is left.
pub fn uninstall(home: &Path, data_dir: &Path, ids: &[String]) -> io::Result<()> {
    for b in BROWSERS
        .iter()
        .filter(|b| ids.is_empty() || ids.iter().any(|i| i == b.id))
    {
        let path = manifest_path(home, b);
        // only a file that names this host is ours to remove
        if std::fs::read_to_string(&path).is_ok_and(|t| t.contains(NATIVE_HOST_NAME)) {
            std::fs::remove_file(&path)?;
        }
    }
    let any_left = BROWSERS.iter().any(|b| {
        std::fs::read_to_string(manifest_path(home, b)).is_ok_and(|t| t.contains(NATIVE_HOST_NAME))
    });
    if !any_left {
        let launcher = launcher_path(data_dir);
        if launcher.is_file() {
            std::fs::remove_file(launcher)?;
        }
    }
    Ok(())
}

/// The home folder, for the real thing.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}
