//! Q2: the disguise ("App icon and name").
//!
//! The person can give the app one of 11 neutral looks (a name and an icon), switch the name to
//! lower case, or type a name of their own. This module is everything about that which does not
//! need Tauri:
//!
//! * the **presets**: `assets/disguise/presets.json` (the single source of truth, also read by the
//!   UI and by `scripts/android_patch_manifest.py`) embedded with `include_str!`, plus the 512 px
//!   and 128 px PNG of every preset embedded with `include_bytes!`;
//! * the **choice** ([`DisguiseInput`]), its **validation** and the **applied name**;
//! * the **saved state**, `disguise.json` next to the account registry (owner-only, replaced
//!   atomically; a missing or damaged file means "original look" and is never an error);
//! * the **launcher seam** ([`Launcher`], like `UrlOpener`): what actually changes outside the
//!   app (window title and icon, the Linux launcher entry, the Android launcher alias). The
//!   production implementation lives in `commands::disguise` because it needs the Tauri
//!   `AppHandle`; tests and the e2e shim use [`NoLauncher`] / [`RecordingLauncher`];
//! * the Linux launcher entry itself ([`desktop`]) and the [`wipe_hook`] the secure wipe calls.
//!
//! `disguise.json` is **not secret and not encrypted**: it has to be read before any unlock, so
//! anyone who can read the data folder can read which look was chosen (the launcher entry and its
//! icon reveal the same by design). It holds no vault data.

use crate::storage::error::{Result, StorageError};
use crate::storage::files::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

pub mod desktop;

#[cfg(test)]
mod tests;

/// File name of the saved choice, inside the app data folder (next to `profiles.json`).
pub const FILE_NAME: &str = "disguise.json";
const FILE_VERSION: u64 = 1;
/// A saved choice is a few dozen bytes; anything larger is not ours.
const MAX_FILE_BYTES: u64 = 4096;
/// Longest custom name, in characters, after trimming.
pub const MAX_NAME_CHARS: usize = 30;
/// The name shown for the original look.
pub const ORIGINAL_NAME: &str = "Hexis";
/// The original (bundled) icon, 512 px, for the launcher entry when only the name was changed.
pub const ORIGINAL_ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");
/// `assets/disguise/presets.json`, embedded.
pub const PRESETS_JSON: &str = include_str!("../../assets/disguise/presets.json");

// ------------------------------------------------------------------ presets

/// One entry of `presets.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Tile colour, `#RRGGBB` (used for the Android adaptive-icon background).
    pub background: String,
}

#[derive(Deserialize)]
struct PresetFile {
    presets: Vec<Preset>,
}

/// A preset together with its embedded icons.
#[derive(Debug, Clone)]
pub struct PresetAssets {
    pub preset: Preset,
    /// `src-tauri/icons/disguise/<id>.png`, 512 x 512.
    pub png_512: &'static [u8],
    /// `src-tauri/icons/disguise/<id>-128.png`, 128 x 128.
    pub png_128: &'static [u8],
}

impl PresetAssets {
    pub fn id(&self) -> &str {
        &self.preset.id
    }
}

macro_rules! embedded_icons {
    ($($id:literal),+ $(,)?) => {
        &[$((
            $id,
            include_bytes!(concat!("../icons/disguise/", $id, ".png")) as &[u8],
            include_bytes!(concat!("../icons/disguise/", $id, "-128.png")) as &[u8],
        )),+]
    };
}

/// Every embedded icon, in the order of `presets.json` (a test keeps the two in step).
static ICONS: &[(&str, &[u8], &[u8])] = embedded_icons!(
    "tally", "vellum", "drizzle", "trim", "plinth", "kerf", "basalt", "cairn", "kovren", "tivlo",
    "orvyn",
);

fn load_embedded() -> Vec<PresetAssets> {
    let Ok(file) = serde_json::from_str::<PresetFile>(PRESETS_JSON) else {
        return Vec::new();
    };
    file.presets
        .into_iter()
        .filter_map(|preset| {
            let (_, png_512, png_128) = ICONS.iter().find(|(id, _, _)| *id == preset.id)?;
            Some(PresetAssets {
                preset,
                png_512,
                png_128,
            })
        })
        .collect()
}

/// The 11 presets, in the order of `presets.json`. Parsed once.
pub fn presets() -> &'static [PresetAssets] {
    static PRESETS: OnceLock<Vec<PresetAssets>> = OnceLock::new();
    PRESETS.get_or_init(load_embedded).as_slice()
}

/// The preset with this id, if there is one.
pub fn preset(id: &str) -> Option<&'static PresetAssets> {
    presets().iter().find(|p| p.preset.id == id)
}

// ------------------------------------------------------------------ the choice

/// What the person picked (`DisguiseInput` of `types.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisguiseInput {
    /// `None` is the original icon.
    #[serde(default)]
    pub preset_id: Option<String>,
    /// A name of the person's own; replaces the preset's name.
    #[serde(default)]
    pub custom_name: Option<String>,
    #[serde(default)]
    pub lowercase: bool,
}

/// The choice plus what it resolves to (`DisguiseView` of `types.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisguiseView {
    pub preset_id: Option<String>,
    pub custom_name: Option<String>,
    pub lowercase: bool,
    pub applied_name: String,
    /// What the launcher could and could not do on this platform.
    pub note: Option<String>,
}

impl DisguiseView {
    /// The original look: what a reset or a wipe puts back.
    pub fn original() -> Self {
        DisguiseInput::default().view(None)
    }

    /// Nothing is disguised at all (no icon, no name of their own, no lower-casing).
    pub fn is_original(&self) -> bool {
        self.preset_id.is_none() && self.custom_name.is_none() && !self.lowercase
    }
}

/// Characters that render as nothing or reorder text. A name made of them would look empty, or
/// could hide itself in a menu, so they are refused.
fn is_invisible_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}' | '\u{E0001}' | '\u{E0020}'..='\u{E007F}'
        | '\u{E000}'..='\u{F8FF}'
    )
}

/// A custom name: control characters (newline, tab, escape, NUL ...) become one space each, so a
/// name pasted with a line break can never start a new line anywhere it is written; then trimmed;
/// empty after trimming means "no custom name"; 1..=30 characters; no invisible formatting
/// characters (refused). Path separators and `%` are fine (they are escaped wherever the name is
/// written).
fn normalize_custom_name(raw: &str) -> Result<Option<String>> {
    let spaced: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let name = spaced.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(StorageError::invalid(
            "Use 1 to 30 characters for the name.",
        ));
    }
    if name.chars().any(is_invisible_format) {
        return Err(StorageError::invalid(
            "The name contains characters that cannot be shown.",
        ));
    }
    Ok(Some(name.to_string()))
}

impl DisguiseInput {
    /// Check the choice and return it normalized (custom name trimmed, `""` became `None`).
    pub fn validated(self) -> Result<Self> {
        if let Some(id) = &self.preset_id {
            if preset(id).is_none() {
                return Err(StorageError::invalid(
                    "That icon is not one of the choices.",
                ));
            }
        }
        let custom_name = match &self.custom_name {
            None => None,
            Some(raw) => normalize_custom_name(raw)?,
        };
        Ok(Self {
            preset_id: self.preset_id,
            custom_name,
            lowercase: self.lowercase,
        })
    }

    /// The custom name when set, else the preset's name, else `Hexis`; lower-cased on request.
    pub fn applied_name(&self) -> String {
        let base = match (&self.custom_name, &self.preset_id) {
            (Some(name), _) => name.clone(),
            (None, Some(id)) => {
                preset(id).map_or_else(|| ORIGINAL_NAME.to_string(), |p| p.preset.name.clone())
            }
            (None, None) => ORIGINAL_NAME.to_string(),
        };
        if self.lowercase {
            base.to_lowercase()
        } else {
            base
        }
    }

    pub fn view(&self, note: Option<String>) -> DisguiseView {
        DisguiseView {
            preset_id: self.preset_id.clone(),
            custom_name: self.custom_name.clone(),
            lowercase: self.lowercase,
            applied_name: self.applied_name(),
            note,
        }
    }
}

// ------------------------------------------------------------------ saved state

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    version: u64,
    #[serde(default)]
    preset_id: Option<String>,
    #[serde(default)]
    custom_name: Option<String>,
    #[serde(default)]
    lowercase: bool,
}

fn read_saved(dir: &Path) -> Option<DisguiseInput> {
    let path = dir.join(FILE_NAME);
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let stored: Stored = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    if stored.version != FILE_VERSION {
        return None;
    }
    // A hand-edited file must not smuggle an unknown icon or a hostile name past the checks.
    DisguiseInput {
        preset_id: stored.preset_id,
        custom_name: stored.custom_name,
        lowercase: stored.lowercase,
    }
    .validated()
    .ok()
}

/// The saved choice. A missing, damaged, oversized, newer-format or invalid file means the
/// defaults (original look): this never fails, so it can never stop the app from starting.
pub fn load(dir: &Path) -> DisguiseInput {
    read_saved(dir).unwrap_or_default()
}

fn save_failed() -> StorageError {
    StorageError::Storage(
        "Your choice could not be saved. Check that the app's data folder can be written.".into(),
    )
}

/// Save the choice: owner-only file (0600 on Unix), written to a temporary file, fsynced and
/// renamed over `disguise.json`, so a crash leaves the old choice or the new one, never half.
pub fn save(dir: &Path, input: &DisguiseInput) -> Result<()> {
    let input = input.clone().validated()?;
    let stored = Stored {
        version: FILE_VERSION,
        preset_id: input.preset_id,
        custom_name: input.custom_name,
        lowercase: input.lowercase,
    };
    let mut bytes = serde_json::to_vec_pretty(&stored).map_err(|_| save_failed())?;
    bytes.push(b'\n');
    write_atomic(&dir.join(FILE_NAME), &bytes).map_err(|_| save_failed())
}

/// Overwrite a file with zeros, fsync, then unlink it (best effort on SSDs and copy-on-write file
/// systems, see `FEATURE_QUEUE.md` on the wipe). A missing file is fine.
fn shred_file(path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.is_file() {
        // `len` is at most a few KiB for our own file; a huge file is simply removed.
        if meta.len() <= MAX_FILE_BYTES {
            let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
            f.write_all(&vec![0u8; usize::try_from(meta.len()).unwrap_or(0)])?;
            f.sync_all()?;
        }
        std::fs::remove_file(path)
    } else if meta.file_type().is_symlink() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

/// For the secure wipe (slice Q1 calls this, or [`crate::storage::AppCore::disguise_wipe`]):
/// forget the disguise everywhere this module wrote it. Erases `disguise.json` in `data_dir` and
/// asks the launcher to go back to the original look (it removes the launcher entry and its
/// icons, and puts the window title and icon back). Both steps always run; the first failure is
/// reported afterwards.
pub fn wipe_hook(data_dir: &Path, launcher: &dyn Launcher) -> Result<()> {
    let file = shred_file(&data_dir.join(FILE_NAME));
    let reset = launcher.apply(&DisguiseView::original(), None);
    file.map_err(|_| {
        StorageError::Storage("The saved icon and name choice could not be erased.".into())
    })?;
    reset.map(|_| ())
}

// ------------------------------------------------------------------ the launcher seam

/// Whatever changes outside the app when the disguise changes.
///
/// `view` is what to show (its `note` is `None`); `preset` is the chosen preset's assets, `None`
/// for the original icon. Returns the platform note: what the launcher did and did not do. An
/// `Err` means the launcher could not do its job; the choice is saved either way and the caller
/// shows the error's message as the note.
pub trait Launcher: Send + Sync {
    fn apply(&self, view: &DisguiseView, preset: Option<&PresetAssets>) -> Result<Option<String>>;

    /// A note that is always true on this platform (shown before anything was applied).
    fn standing_note(&self) -> Option<String> {
        None
    }
}

/// The default when nothing was installed: changes nothing, says nothing.
pub struct NoLauncher;

impl Launcher for NoLauncher {
    fn apply(
        &self,
        _view: &DisguiseView,
        _preset: Option<&PresetAssets>,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

/// One call to a [`RecordingLauncher`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub view: DisguiseView,
    /// The id of the preset assets the launcher was handed (`None`: the original icon).
    pub preset_id: Option<String>,
}

/// Remembers what it was asked to apply and changes nothing (tests and the browser e2e shim).
#[derive(Default)]
pub struct RecordingLauncher {
    calls: Mutex<Vec<Applied>>,
    outcome: Mutex<Option<std::result::Result<Option<String>, String>>>,
}

impl RecordingLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer every call with this note.
    #[must_use]
    pub fn with_note(self, note: &str) -> Self {
        *lock(&self.outcome) = Some(Ok(Some(note.to_string())));
        self
    }

    /// Fail every call with this message (to test that a launcher failure never loses the choice).
    #[must_use]
    pub fn failing(self, message: &str) -> Self {
        *lock(&self.outcome) = Some(Err(message.to_string()));
        self
    }

    pub fn calls(&self) -> Vec<Applied> {
        lock(&self.calls).clone()
    }

    pub fn last(&self) -> Option<Applied> {
        lock(&self.calls).last().cloned()
    }

    pub fn count(&self) -> usize {
        lock(&self.calls).len()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Launcher for RecordingLauncher {
    fn apply(&self, view: &DisguiseView, preset: Option<&PresetAssets>) -> Result<Option<String>> {
        lock(&self.calls).push(Applied {
            view: view.clone(),
            preset_id: preset.map(|p| p.preset.id.clone()),
        });
        match lock(&self.outcome).clone() {
            None => Ok(None),
            Some(Ok(note)) => Ok(note),
            Some(Err(message)) => Err(StorageError::Storage(message)),
        }
    }
}

// ------------------------------------------------------------------ PNG pixels

/// Decode a PNG to 8-bit RGBA: `(width, height, pixels)`. Used for the window icon. Refuses
/// anything that is not a plain, bounded PNG (the inputs are our own embedded files).
pub fn decode_png_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let bad = || StorageError::Storage("An icon could not be read.".into());
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(bytes),
        png::Limits {
            bytes: 8 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|_| bad())?;
    let size = reader.output_buffer_size().ok_or_else(bad)?;
    let mut pixels = vec![0u8; size];
    let info = reader.next_frame(&mut pixels).map_err(|_| bad())?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(bad());
    }
    pixels.truncate(info.buffer_size());
    Ok((info.width, info.height, pixels))
}
