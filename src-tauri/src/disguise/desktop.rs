//! The Linux launcher entry: `<data home>/applications/<identifier>.desktop` plus the icon file
//! `<data home>/<identifier>/disguise/<id>.png`.
//!
//! `<data home>` is `$XDG_DATA_HOME` (default `~/.local/share`) in the app and a temp folder in
//! tests: it is always handed in, nothing here reads the environment, so tests can never touch the
//! real launcher folders.
//!
//! Rules this file keeps:
//!
//! * The entry is only ever written or deleted **if it is ours**: an existing file without the
//!   `X-Passman-Managed=true` line is somebody else's and is left alone (the write is refused, the
//!   delete is skipped).
//! * Only two paths are ever written or removed: the entry, and files of ours named `<preset id>.png`
//!   or `original.png` inside the `disguise` icon folder (that folder itself goes when it is empty).
//! * Every string that reaches the file is escaped for its context (Desktop Entry Specification
//!   1.5: the general `string` escapes, plus the `Exec` quoting rule and `%%`), so a name can never
//!   start a new line or a new key, whatever it contains. Names are validated before this point;
//!   the escaping does not rely on that.
//! * The icon files and the entry are replaced atomically and only when their content changed, so
//!   writing the same choice twice changes nothing on disk.
//! * Refreshing the desktop database is best effort: `update-desktop-database <folder>` is spawned
//!   with an argument array (no shell), its result is ignored, and a missing binary is fine.

use super::{presets, DisguiseView, PresetAssets, ORIGINAL_ICON_PNG};
use crate::storage::error::{Result, StorageError};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// The line that marks an entry as written by Passman.
pub const MARKER_LINE: &str = "X-Passman-Managed=true";
/// Icon file used when only the name was changed (the original look keeps its own icon).
pub const ORIGINAL_ICON_FILE: &str = "original.png";
const ICON_DIR_NAME: &str = "disguise";
/// An entry is a few hundred bytes; only this much is read when checking who wrote it.
const MAX_ENTRY_READ: u64 = 64 * 1024;

/// What an operation did on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// The files already were what they should be.
    Unchanged,
    /// The entry (and its icon) were written.
    Written,
    /// The entry and the icons were removed.
    Removed,
}

fn setup_failed() -> StorageError {
    StorageError::Storage("The launcher entry could not be prepared.".into())
}

fn io_failed() -> StorageError {
    StorageError::Storage("The launcher entry could not be written.".into())
}

fn foreign_in_the_way() -> StorageError {
    StorageError::Storage(
        "A launcher entry that Passman did not create is already in the way, so it was left as it is."
            .into(),
    )
}

/// An identifier becomes a file name: letters, digits, `.`, `-` and `_` only, not starting with
/// a dot or a dash, no `..`.
fn valid_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with(['.', '-'])
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn applications_dir_of(data_home: &Path) -> PathBuf {
    data_home.join("applications")
}

fn entry_path_of(data_home: &Path, identifier: &str) -> PathBuf {
    applications_dir_of(data_home).join(format!("{identifier}.desktop"))
}

fn icon_dir_of(data_home: &Path, identifier: &str) -> PathBuf {
    data_home.join(identifier).join(ICON_DIR_NAME)
}

// ------------------------------------------------------------------ escaping

/// The general escape rule for Desktop Entry values of type `string`: backslash, newline, tab and
/// carriage return are written as `\\`, `\n`, `\t`, `\r`; any other control character becomes a
/// space. The result is always one physical line.
pub fn escape_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// One program path as the `Exec` key wants it: quoted when it holds a reserved character (space,
/// tab, newline, `"`, `'`, `\`, `>`, `<`, `~`, `|`, `&`, `;`, `$`, `*`, `?`, `#`, `(`, `)`, `` ` ``),
/// inside quotes `"`, `` ` ``, `$` and `\` get a backslash, `%` is doubled (`%%`, else `%u` and
/// friends would be field codes), and then the general string escape is applied on top, as the
/// specification orders.
pub fn exec_value(path: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let quote = path.chars().any(|c| RESERVED.contains(&c));
    let mut inner = String::with_capacity(path.len() + 2);
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                inner.push('\\');
                inner.push(c);
            }
            '%' => inner.push_str("%%"),
            c => inner.push(c),
        }
    }
    let arg = if quote { format!("\"{inner}\"") } else { inner };
    escape_value(&arg)
}

// ------------------------------------------------------------------ what to point the entry at

/// The program the entry should start. In an AppImage the running binary lives in a temporary
/// mount that disappears when the app quits, so the `APPIMAGE` variable (the AppImage file
/// itself) is preferred when it is an absolute path to an existing file; otherwise the running
/// executable, without the ` (deleted)` suffix Linux appends after the file was replaced.
pub fn exec_target(current_exe: &Path, appimage: Option<&OsStr>) -> PathBuf {
    if let Some(p) = appimage.filter(|v| !v.is_empty()).map(Path::new) {
        if p.is_absolute() && p.is_file() {
            return p.to_path_buf();
        }
    }
    strip_deleted_suffix(current_exe)
}

fn strip_deleted_suffix(path: &Path) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(s) => PathBuf::from(s),
        None => path.to_path_buf(),
    }
}

/// The window class to put in `StartupWMClass`: the executable's file name. Tauri passes no
/// explicit class to GTK (it only hands the identifier over as the application id when
/// `app.enableGtkAppId` is set, which is off), so the class comes from GTK's default, the program
/// name, which is the executable's file name. **Not verified on a running desktop.**
pub fn wm_class_of(current_exe: &Path) -> String {
    strip_deleted_suffix(current_exe)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ------------------------------------------------------------------ file helpers

/// Replace `path` with `bytes` (world-readable, 0644: icons and launcher entries are public
/// assets), unless it already holds exactly that. Returns whether it was written.
fn write_public_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    use std::io::Write;
    if let Ok(existing) = std::fs::read(path) {
        if existing == bytes {
            return Ok(false);
        }
    }
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent folder"))?;
    let mut tmp_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?
        .to_os_string();
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o644);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        crate::storage::files::fsync_dir(dir)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(|()| true)
}

/// Who wrote the file at `path`?
#[derive(Debug, PartialEq, Eq)]
enum Owner {
    Absent,
    Ours,
    Foreign,
}

fn owner_of(path: &Path) -> Owner {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Owner::Absent,
        Err(_) => return Owner::Foreign,
    };
    let mut text = String::new();
    if file.take(MAX_ENTRY_READ).read_to_string(&mut text).is_err() {
        return Owner::Foreign;
    }
    if text.lines().any(|l| l.trim_end() == MARKER_LINE) {
        Owner::Ours
    } else {
        Owner::Foreign
    }
}

/// Is `name` a file this module writes into the icon folder?
fn is_our_icon_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name == ORIGINAL_ICON_FILE
        || name
            .strip_suffix(".png")
            .is_some_and(|id| presets().iter().any(|p| p.preset.id == id))
}

/// Delete our icon files except `keep`, then the folder if nothing else is in it.
fn prune_icons(dir: &Path, keep: Option<&str>) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut removed = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !is_our_icon_name(&name) || keep.is_some_and(|k| name == OsStr::new(k)) {
            continue;
        }
        if entry
            .file_type()
            .is_ok_and(|t| t.is_file() || t.is_symlink())
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed = true;
        }
    }
    if keep.is_none() {
        // Only succeeds when the folder is empty: files that are not ours keep it alive.
        let _ = std::fs::remove_dir(dir);
    }
    removed
}

/// Spawn `<program> <dir>` without a shell and without waiting for it on the calling thread.
/// Returns whether the program could be started; failures and a missing binary are ignored.
pub fn spawn_refresh(program: &str, dir: &Path) -> bool {
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new(program)
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // Reap the child so it never lingers as a zombie.
    let _ = std::thread::Builder::new()
        .name("desktop-db-refresh".into())
        .spawn(move || {
            let _ = child.wait();
        });
    true
}

// ------------------------------------------------------------------ removal

/// Remove the launcher entry and the disguise icons, ignoring absence. An entry that is not ours
/// (no `X-Passman-Managed=true` line) is left alone, and so is every file in the icon folder
/// that this module did not write. Returns [`Effect::Removed`] when something was deleted.
pub fn remove_launcher_entry(data_home: &Path, identifier: &str) -> Result<Effect> {
    if !valid_identifier(identifier) {
        return Err(setup_failed());
    }
    let entry = entry_path_of(data_home, identifier);
    let mut removed = false;
    if owner_of(&entry) == Owner::Ours {
        match std::fs::remove_file(&entry) {
            Ok(()) => removed = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(io_failed()),
        }
    }
    removed |= prune_icons(&icon_dir_of(data_home, identifier), None);
    Ok(if removed {
        Effect::Removed
    } else {
        Effect::Unchanged
    })
}

// ------------------------------------------------------------------ the writer

/// Writes and removes this app's launcher entry under one `<data home>`.
#[derive(Debug, Clone)]
pub struct LauncherFiles {
    data_home: PathBuf,
    identifier: String,
    exec: String,
    wm_class: String,
    refresh: bool,
}

impl LauncherFiles {
    /// `data_home` and `exec_path` must be absolute (`exec_path` also valid text), `identifier`
    /// is the app identifier (it becomes a file name), `wm_class` the window class for
    /// `StartupWMClass`.
    pub fn new(
        data_home: &Path,
        identifier: &str,
        exec_path: &Path,
        wm_class: &str,
    ) -> Result<Self> {
        let exec = exec_path.to_str().ok_or_else(setup_failed)?;
        if !valid_identifier(identifier)
            || !data_home.is_absolute()
            || !exec_path.is_absolute()
            || wm_class.trim().is_empty()
        {
            return Err(setup_failed());
        }
        Ok(Self {
            data_home: data_home.to_path_buf(),
            identifier: identifier.to_string(),
            exec: exec.to_string(),
            wm_class: wm_class.to_string(),
            refresh: true,
        })
    }

    /// Switch the `update-desktop-database` call on or off (tests switch it off).
    #[must_use]
    pub fn with_refresh(mut self, refresh: bool) -> Self {
        self.refresh = refresh;
        self
    }

    pub fn applications_dir(&self) -> PathBuf {
        applications_dir_of(&self.data_home)
    }

    pub fn entry_path(&self) -> PathBuf {
        entry_path_of(&self.data_home, &self.identifier)
    }

    pub fn icon_dir(&self) -> PathBuf {
        icon_dir_of(&self.data_home, &self.identifier)
    }

    fn icon_file_name(preset: Option<&PresetAssets>) -> String {
        match preset {
            Some(p) => format!("{}.png", p.preset.id),
            None => ORIGINAL_ICON_FILE.to_string(),
        }
    }

    /// The exact text of the entry for this look.
    pub fn entry_text(&self, view: &DisguiseView, preset: Option<&PresetAssets>) -> String {
        let icon = self.icon_dir().join(Self::icon_file_name(preset));
        let comment = preset.map_or("", |p| p.preset.description.as_str());
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name={}\n\
             Comment={}\n\
             Exec={}\n\
             Icon={}\n\
             Terminal=false\n\
             Categories=Utility;\n\
             StartupWMClass={}\n\
             {MARKER_LINE}\n",
            escape_value(&view.applied_name),
            escape_value(comment),
            exec_value(&self.exec),
            escape_value(&icon.to_string_lossy()),
            escape_value(&self.wm_class),
        )
    }

    /// Write the icon and the entry for this look. Refuses (without touching anything) when an
    /// entry that is not ours is in the way.
    pub fn write(&self, view: &DisguiseView, preset: Option<&PresetAssets>) -> Result<Effect> {
        let entry = self.entry_path();
        if owner_of(&entry) == Owner::Foreign {
            return Err(foreign_in_the_way());
        }
        let icon_dir = self.icon_dir();
        let icon_name = Self::icon_file_name(preset);
        let icon_bytes = preset.map_or(ORIGINAL_ICON_PNG, |p| p.png_512);
        std::fs::create_dir_all(&icon_dir).map_err(|_| io_failed())?;
        let mut changed = write_public_if_changed(&icon_dir.join(&icon_name), icon_bytes)
            .map_err(|_| io_failed())?;
        changed |= prune_icons(&icon_dir, Some(&icon_name));
        std::fs::create_dir_all(self.applications_dir()).map_err(|_| io_failed())?;
        changed |= write_public_if_changed(&entry, self.entry_text(view, preset).as_bytes())
            .map_err(|_| io_failed())?;
        Ok(if changed {
            Effect::Written
        } else {
            Effect::Unchanged
        })
    }

    /// Remove the entry and the icons (see [`remove_launcher_entry`]).
    pub fn remove(&self) -> Result<Effect> {
        remove_launcher_entry(&self.data_home, &self.identifier)
    }

    /// Make the launcher match `view`: the original look (no icon, no name of their own, no
    /// lower-casing) removes the entry, so no second "Passman" appears next to the one a package
    /// installed; every other look writes it. The desktop database is refreshed afterwards when
    /// something changed.
    pub fn sync(&self, view: &DisguiseView, preset: Option<&PresetAssets>) -> Result<Effect> {
        let effect = if view.is_original() {
            self.remove()?
        } else {
            self.write(view, preset)?
        };
        if self.refresh && effect != Effect::Unchanged {
            spawn_refresh("update-desktop-database", &self.applications_dir());
        }
        Ok(effect)
    }
}
