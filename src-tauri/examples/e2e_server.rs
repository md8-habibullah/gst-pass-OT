//! DEV-ONLY end-to-end shim: the real Rust backend behind a tiny local HTTP endpoint, so the UI
//! can be driven in a plain browser (and by scripted tests) against *real* vault code without a
//! desktop window.
//!
//! ```text
//! cargo run --example e2e_server -- --data-dir /tmp/hexis-e2e --port 1432 --fast-kdf
//! ```
//!
//! * `POST /ipc` with `{"cmd": "item_list", "args": {...}}` runs the command through Tauri's
//!   mock runtime (the same IPC dispatch, argument mapping and serialization as the app) and
//!   answers `{"ok": <value>}` or `{"err": {"code": "...", "message": "..."}}`.
//! * `GET /health` answers `{"ok": true}`.
//! * `POST /e2e/picker` with `{"open": "/path" | null, "save": "/path" | null}` sets what the
//!   stand-in "file dialogs" return (see below); it only ever sets those two variables.
//! * Importer / exporter (slice Q4): all of its commands (`import_pick`, `import_pick_keyfile`,
//!   `inspect_csv_headers`, `import_preview`, `import_commit`, `import_cancel`, `export_plain`)
//!   are served through `POST /ipc` like every other command. Their three dialogs share the one
//!   queue of the drive's dialogs (below): `POST /e2e/pick-next` with `{"path": "/absolute/file"}`
//!   (the one-file spelling; `{"path": null}` is "the person cancels") answers the next
//!   `import_pick`, `import_pick_keyfile` or `export_plain` save dialog; an empty queue cancels.
//!   After a successful `export_plain`, `GET /e2e/last-saved` describes the file it wrote.
//! * The Android passkey provider's façade (`hexis_lib::provider`, the one entry the Kotlin service calls through JNI) is
//!   reachable at `POST /e2e/provider` with `{"op": "create", "args": {...}}` and answers its own text as JSON, so a test can
//!   make a passkey in the open vault the way a website would, then look at it in the UI. It always answers a browser or
//!   Own-window caller; an app caller would look the website up on the network, which this tool never does.
//! * Browsers on Linux (`hexis_lib::bridge`): with `--bridge-home <folder>` the shim tells the bridge that this stand-in is "the
//!   home folder": the browsers' per-user folders are below it (`<folder>/.config/chromium/NativeMessagingHosts`, ...) and
//!   the private socket is `<folder>/run/hexis/bridge.sock` (so a browser started with `HOME=<folder>` and
//!   `XDG_RUNTIME_DIR=<folder>/run` reaches this Hexis, never the real one). The switch and the registration are the real
//!   commands (`bridge_set`, `bridge_register`, ...), served through `POST /ipc`; `GET /e2e/bridge` answers
//!   `{"ok": {"listening", "socket", "home", "waiting"}}`. The window's long poll `bridge_wait` is cut to 250 ms here, because
//!   this tool handles one request at a time. The shim also starts as the native messaging host when its first argument is
//!   `--native-host` (the launcher script the bridge writes starts it that way). The folder must be absolute, empty or
//!   created by this tool, and is never the real home folder or one that contains it.
//! * `open_url` opens nothing here: the address is validated exactly as in the app and then only
//!   recorded. `GET /e2e/opened` answers `{"ok": ["https://..."]}`; `POST /e2e/opened` with
//!   `{}` clears the list.
//! * The disguise (`disguise_get` / `disguise_set`, slice Q2) runs on a launcher that **records** every
//!   call: `GET /e2e/disguise-applied` answers the last applied look, `{"ok": {"presetId": "kerf",
//!   "appliedName": "Kerf", "customName": null, "lowercase": false, "note": null, "count": 1}}`
//!   (`{"ok": null}` before the first call); the same fields are repeated at the top level next to
//!   `ok`. With `--xdg-data-home <dir>` the launcher also **really writes** the Linux launcher entry
//!   `<dir>/applications/<identifier>.desktop` and its icon `<dir>/<identifier>/disguise/<id>.png`
//!   (the original look removes them again). That folder must be absolute, empty or created by this
//!   tool, and is never the real `$XDG_DATA_HOME` / `~/.local/share`; `update-desktop-database` is
//!   not run. Without the flag nothing outside the data folder is ever written.
//! * `--fast-backup` makes a queue read as `lagging` after one second instead of five minutes, so
//!   that state can be watched in a test. (There are no retry timers any more: a failed job is
//!   retried at the next trigger, that is unlock, a committed change, "Back up now" or the flush.)
//!   Backup targets are reached with the real S3 client, e.g. `scripts/mock_s3.py` on loopback.
//! * `--idle-unit-ms <n>` makes one "minute" of the inactivity lock last `n` milliseconds, so the
//!   1/5/15/30 minute timer can be tested in seconds (production: 60 000). The "after leaving the
//!   app" timer (`app_leave` / `app_return`) is scaled the same way: one "second" is a sixtieth of
//!   that unit.
//! * `--lock-unit-ms <n>` does the same for the waits after wrong passcodes (3 wrong: 1.5 minutes,
//!   6: 5, 9: 45 minutes, 12: 8 hours, 15: 3 days). One "minute" of waiting lasts `n` milliseconds
//!   (0 = no waits at all, the default here; production: 60 000), so the whole ladder can be walked
//!   through in seconds. Leave it out of a test that does not look at the waits. The Stage 1 gate
//!   (`gate_*`, `app_reset`, `app_settings_*`, `activity_ping`) is served like every other
//!   command; with the gate off nothing changes. A wipe here only ever touches `--data-dir`.
//! * Encrypted drive (`file_*`): `file_read` answers the file's raw bytes with
//!   `Content-Type: application/octet-stream` and the header `x-hexis-binary: 1` (errors keep the
//!   normal JSON `{err}` shape). The native pickers of `file_add` and `file_save_copy` are driven
//!   by `POST /e2e/pick-next` with `{"paths": ["/abs/a", "/abs/b"]}`: the next dialog (an "add
//!   files" dialog gets all the paths, a "save a copy" dialog the first one) answers with that
//!   entry; `{"paths": []}` cancels it; with nothing queued every dialog is cancelled (the save
//!   dialogs then fall back to `HEXIS_E2E_SAVE_PATH`). `GET /e2e/last-saved` answers
//!   `{"ok": null}` or `{"ok": {"path", "suggestedName", "exists", "size", "contentBase64"}}` for
//!   the file the most recent save dialog chose, a copy of a drive file or a plaintext export
//!   (content only up to 8 MiB). `pick-next` takes `{"paths": [...]}` or the one-file `{"path": ...}`.
//!
//! Safety rails (this is a development tool, never part of a release build):
//! * binds `127.0.0.1` only; handles one request at a time;
//! * the `Host` header must be `127.0.0.1:<port>` or `localhost:<port>` (DNS-rebinding guard);
//! * a request that carries an `Origin` must come from `http://localhost:1431` or
//!   `http://127.0.0.1:1431` (the e2e UI dev server) and is the only origin CORS ever allows;
//!   `POST` needs `Content-Type: application/json`, so other web pages cannot even send one;
//! * refuses to start on the real app data folder (`~/.local/share/dev.habibullah.hexis`), on any
//!   folder containing or inside it, and on any non-empty folder it did not create itself;
//! * the native file dialogs are replaced by two environment variables, `HEXIS_E2E_OPEN_PATH`
//!   (file to import) and `HEXIS_E2E_SAVE_PATH` (where to export), read again on every call;
//!   unset or empty means "the dialog was cancelled". The importer's three dialogs do not read
//!   them; they use the `POST /e2e/pick-next` queue described above. Neither route can reach the
//!   app's own data folder: `export_plain` refuses a destination inside it.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hexis_lib::commands::{invoke_handler, AppState};
use hexis_lib::crypto::KdfParams;
use hexis_lib::disguise::desktop::{exec_target, wm_class_of, LauncherFiles};
use hexis_lib::disguise::{DisguiseView, Launcher, PresetAssets, RecordingLauncher};
use hexis_lib::storage::{
    AppCore, BackupRuntime, FilePicker, RecordingOpener, SessionConfig, StorageError, Timing,
};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
use tauri::test::{
    get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

const DEFAULT_PORT: u16 = 1432;
const ALLOWED_ORIGINS: [&str; 2] = ["http://localhost:1431", "http://127.0.0.1:1431"];
const OPEN_VAR: &str = "HEXIS_E2E_OPEN_PATH";
const SAVE_VAR: &str = "HEXIS_E2E_SAVE_PATH";
const MARKER: &str = ".hexis-e2e";
/// The longest the window's `bridge_wait` may wait here (production: 30 s).
const BRIDGE_WAIT_MAX_MS: u64 = 250;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Most paths one `pick-next` entry may carry (the drive accepts 100 files per add).
const MAX_PICK_PATHS: usize = 100;
/// `last-saved` includes the file's content up to this size.
const LAST_SAVED_INLINE_BYTES: u64 = 8 * 1024 * 1024;

/// Answers queued by `POST /e2e/pick-next`; each dialog consumes one entry.
static PICK_QUEUE: Mutex<VecDeque<Vec<PathBuf>>> = Mutex::new(VecDeque::new());
/// What the most recent save dialog answered: (chosen path, suggested name).
static LAST_SAVED: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

fn next_pick() -> Option<Vec<PathBuf>> {
    PICK_QUEUE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .pop_front()
}

fn remember_saved(path: &Path, suggested: &str) {
    *LAST_SAVED.lock().unwrap_or_else(|p| p.into_inner()) =
        Some((path.to_path_buf(), suggested.to_string()));
}

/// Stand-in for the native dialogs: queued answers first (`/e2e/pick-next`), then an environment
/// variable for the vault-file dialogs.
struct EnvPicker;

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

impl FilePicker for EnvPicker {
    fn open_vault_file(&self) -> Result<Option<PathBuf>, StorageError> {
        Ok(env_path(OPEN_VAR))
    }

    fn save_vault_file(&self, suggested_name: &str) -> Result<Option<PathBuf>, StorageError> {
        let chosen = next_pick()
            .and_then(|mut entry| (!entry.is_empty()).then(|| entry.swap_remove(0)))
            .or_else(|| env_path(SAVE_VAR));
        if let Some(path) = &chosen {
            remember_saved(path, suggested_name);
        }
        Ok(chosen)
    }

    fn open_files(&self) -> Result<Option<Vec<PathBuf>>, StorageError> {
        Ok(next_pick().filter(|paths| !paths.is_empty()))
    }

    fn save_file(&self, suggested_name: &str) -> Result<Option<PathBuf>, StorageError> {
        self.save_vault_file(suggested_name)
    }

    fn open_import_file(&self) -> Result<Option<PathBuf>, StorageError> {
        Ok(next_pick().and_then(|mut entry| (!entry.is_empty()).then(|| entry.swap_remove(0))))
    }

    fn open_keyfile(&self) -> Result<Option<PathBuf>, StorageError> {
        Ok(next_pick().and_then(|mut entry| (!entry.is_empty()).then(|| entry.swap_remove(0))))
    }

    fn save_export_file(&self, suggested_name: &str) -> Result<Option<PathBuf>, StorageError> {
        let chosen =
            next_pick().and_then(|mut entry| (!entry.is_empty()).then(|| entry.swap_remove(0)));
        if let Some(path) = &chosen {
            remember_saved(path, suggested_name);
        }
        Ok(chosen)
    }
}

struct Options {
    data_dir: PathBuf,
    port: u16,
    fast_kdf: bool,
    fast_backup: bool,
    xdg_data_home: Option<PathBuf>,
    /// Stand-in for the home folder in the bridge's eyes (`--bridge-home`): browsers' folders and the socket live below it.
    bridge_home: Option<PathBuf>,
    /// How long one "minute" of the inactivity timer lasts (production: 60 000 ms).
    idle_unit_ms: Option<u64>,
    /// How long one "minute" of the wait after wrong passcodes lasts (production: 60 000 ms; 0: no waits).
    lock_unit_ms: u64,
    /// Switch the online time check off (saved in `device.json` like the Settings switch), so a run makes no network request.
    no_time_check: bool,
}

fn usage() -> String {
    "usage: e2e_server --data-dir <folder> [--port <n>] [--fast-kdf] [--fast-backup] [--xdg-data-home <folder>] [--bridge-home <folder>] [--idle-unit-ms <n>] [--lock-unit-ms <n>] [--no-time-check]"
        .into()
}

fn parse_args() -> Result<Options, String> {
    let mut data_dir = None;
    let mut port = DEFAULT_PORT;
    let mut fast_kdf = false;
    let mut fast_backup = false;
    let mut xdg_data_home = None;
    let mut bridge_home = None;
    let mut idle_unit_ms = None;
    let mut lock_unit_ms = 0;
    let mut no_time_check = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data-dir" => data_dir = Some(PathBuf::from(args.next().ok_or_else(usage)?)),
            "--port" => {
                port = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .filter(|p| *p != 0)
                    .ok_or("--port needs a number from 1 to 65535")?;
            }
            "--fast-kdf" => fast_kdf = true,
            "--fast-backup" => fast_backup = true,
            "--xdg-data-home" => {
                xdg_data_home = Some(PathBuf::from(args.next().ok_or_else(usage)?));
            }
            "--bridge-home" => {
                bridge_home = Some(PathBuf::from(args.next().ok_or_else(usage)?));
            }
            "--idle-unit-ms" => {
                idle_unit_ms = Some(
                    args.next()
                        .and_then(|n| n.parse().ok())
                        .filter(|n| (1..=60_000).contains(n))
                        .ok_or("--idle-unit-ms needs a number of milliseconds from 1 to 60000")?,
                );
            }
            "--lock-unit-ms" => {
                lock_unit_ms = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| *n <= 60_000)
                    .ok_or("--lock-unit-ms needs a number of milliseconds from 0 to 60000")?;
            }
            "--no-time-check" => no_time_check = true,
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("unknown argument {other:?}\n{}", usage())),
        }
    }
    Ok(Options {
        data_dir: data_dir.ok_or_else(usage)?,
        port,
        fast_kdf,
        fast_backup,
        xdg_data_home,
        bridge_home,
        idle_unit_ms,
        lock_unit_ms,
        no_time_check,
    })
}

// ------------------------------------------------------------------ data folder rules

/// The folder as an absolute path with links resolved as far as it exists.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    // resolve the deepest existing ancestor, keep the rest as written
    let mut tail = Vec::new();
    let mut cur = abs.as_path();
    loop {
        if let Ok(base) = std::fs::canonicalize(cur) {
            return tail.iter().rev().fold(base, |acc, c| acc.join(c));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent;
            }
            _ => return abs,
        }
    }
}

/// Where the real desktop app keeps its data on Linux (and, by name, anywhere).
fn real_data_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(x).join("dev.habibullah.hexis"));
    }
    if let Some(h) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(h).join(".local/share/dev.habibullah.hexis"));
    }
    out
}

fn check_data_dir(dir: &Path) -> Result<(), String> {
    if dir
        .components()
        .any(|c| c.as_os_str() == "dev.habibullah.hexis")
    {
        return Err("refusing: that path names the real Hexis data folder".into());
    }
    let mine = resolved(dir);
    for real in real_data_dirs() {
        let real = resolved(&real);
        if mine.starts_with(&real) || real.starts_with(&mine) {
            return Err(format!(
                "refusing: {} is, contains or lies inside the real Hexis data folder",
                mine.display()
            ));
        }
    }
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            let empty = entries.next().is_none();
            if !empty && !dir.join(MARKER).is_file() {
                return Err(format!(
                    "refusing: {} is not empty and was not created by this tool",
                    dir.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    std::fs::write(
        dir.join(MARKER),
        "created by hexis examples/e2e_server.rs; safe to delete\n",
    )
    .map_err(|e| format!("cannot write into {}: {e}", dir.display()))
}

/// The folder the shim may use as a stand-in for `$XDG_DATA_HOME` (`--xdg-data-home`): absolute,
/// not the real data home of this user, empty or created by this tool. The launcher entry and its
/// icon are the only things written there.
fn check_xdg_data_home(dir: &Path) -> Result<(), String> {
    if !dir.is_absolute() {
        return Err("refusing: --xdg-data-home must be an absolute path".into());
    }
    let mine = resolved(dir);
    let mut real_homes = Vec::new();
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        real_homes.push(PathBuf::from(x));
    }
    if let Some(h) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        real_homes.push(PathBuf::from(h).join(".local/share"));
    }
    if real_homes.iter().any(|real| resolved(real) == mine) {
        return Err(format!(
            "refusing: {} is the real XDG data home, which holds the user's launcher entries",
            mine.display()
        ));
    }
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() && !dir.join(MARKER).is_file() {
                return Err(format!(
                    "refusing: {} is not empty and was not created by this tool",
                    dir.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    std::fs::write(
        dir.join(MARKER),
        "created by hexis examples/e2e_server.rs; safe to delete\n",
    )
    .map_err(|e| format!("cannot write into {}: {e}", dir.display()))
}

// ------------------------------------------------------------------ the disguise launcher

/// Records every disguise the core applies and, with `--xdg-data-home`, also writes the real Linux
/// launcher entry and icon under that folder (never the user's own).
struct ShimLauncher {
    recorder: Arc<RecordingLauncher>,
    files: Option<LauncherFiles>,
}

impl Launcher for ShimLauncher {
    fn apply(
        &self,
        view: &DisguiseView,
        preset: Option<&PresetAssets>,
    ) -> Result<Option<String>, StorageError> {
        let recorded = self.recorder.apply(view, preset)?;
        match &self.files {
            None => Ok(recorded),
            Some(files) => files
                .sync(view, preset)
                .map(|_| Some("Launcher entry updated (e2e data home).".to_string())),
        }
    }
}

/// The app identifier from the real `tauri.conf.json`, so the entry gets the real file name.
fn app_identifier() -> String {
    serde_json::from_str::<Value>(include_str!("../tauri.conf.json"))
        .ok()
        .and_then(|v| {
            v.get("identifier")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "dev.habibullah.hexis".to_string())
}

fn shim_launcher(
    recorder: Arc<RecordingLauncher>,
    xdg_data_home: Option<&Path>,
) -> Result<ShimLauncher, String> {
    let files = match xdg_data_home {
        None => None,
        Some(home) => {
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let appimage = std::env::var_os("APPIMAGE");
            Some(
                LauncherFiles::new(
                    home,
                    &app_identifier(),
                    &exec_target(&exe, appimage.as_deref()),
                    &wm_class_of(&exe),
                )
                .map_err(|e| e.to_string())?
                .with_refresh(false),
            )
        }
    };
    Ok(ShimLauncher { recorder, files })
}

/// The folder the shim may use as a stand-in for the home folder (`--bridge-home`): absolute, empty or created by this tool,
/// and neither the real home folder nor a folder that contains it. Browsers' registrations and the socket are written below it.
fn check_bridge_home(dir: &Path) -> Result<(), String> {
    if !dir.is_absolute() {
        return Err("refusing: --bridge-home must be an absolute path".into());
    }
    let mine = resolved(dir);
    if let Some(h) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        let real = resolved(Path::new(&h));
        if real.starts_with(&mine) {
            return Err(format!(
                "refusing: {} is or contains the real home folder",
                mine.display()
            ));
        }
    }
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() && !dir.join(MARKER).is_file() {
                return Err(format!(
                    "refusing: {} is not empty and was not created by this tool",
                    dir.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    std::fs::write(
        dir.join(MARKER),
        "created by hexis examples/e2e_server.rs; safe to delete\n",
    )
    .map_err(|e| format!("cannot write into {}: {e}", dir.display()))?;
    // the runtime folder (`$XDG_RUNTIME_DIR` of the browser the test starts): private, like the real one
    let run = dir.join("run");
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(&run) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("cannot create {}: {e}", run.display())),
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ the backend

struct Backend {
    _app: tauri::App<MockRuntime>,
    window: WebviewWindow<MockRuntime>,
    opener: Arc<RecordingOpener>,
    launcher: Arc<RecordingLauncher>,
    bridge_home: Option<PathBuf>,
}

/// Replication timing for tests: a waiting queue shows as `lagging` after a second.
fn fast_backup_timing() -> Timing {
    Timing {
        lagging_after: Duration::from_secs(1),
        ..Timing::default()
    }
}

fn build_backend(opts: &Options) -> Result<Backend, String> {
    let Options {
        data_dir,
        fast_kdf,
        fast_backup,
        idle_unit_ms,
        lock_unit_ms,
        no_time_check,
        ..
    } = opts;
    let (data_dir, fast_kdf, fast_backup, lock_unit_ms, no_time_check) = (
        data_dir.as_path(),
        *fast_kdf,
        *fast_backup,
        *lock_unit_ms,
        *no_time_check,
    );
    let idle_unit_ms = *idle_unit_ms;
    let (xdg_data_home, bridge_home) = (opts.xdg_data_home.as_deref(), opts.bridge_home.as_deref());
    let mut cfg = SessionConfig::default();
    if fast_kdf {
        // the smallest parameters the format accepts: for tests only
        cfg.kdf = KdfParams {
            m_cost_kib: 19_456,
            t_cost: 2,
            p_cost: 1,
        };
    }
    let opener = Arc::new(RecordingOpener::new());
    let launcher = Arc::new(RecordingLauncher::new());
    let mut core = AppCore::open(data_dir, cfg, Arc::new(EnvPicker))
        .map_err(|e| e.to_string())?
        .with_opener(opener.clone())
        .with_launcher(Arc::new(shim_launcher(launcher.clone(), xdg_data_home)?));
    if fast_backup {
        core = core.with_backup_runtime(BackupRuntime::new(
            Arc::new(hexis_lib::storage::backup::remote::S3Factory),
            fast_backup_timing(),
        ));
    }
    if let Some(ms) = idle_unit_ms {
        // test hook: one "minute" of the inactivity lock lasts `ms` milliseconds
        core.gate().set_idle_unit(Duration::from_millis(ms));
    }
    // test hook: one "minute" of the wait after wrong passcodes lasts `lock_unit_ms` milliseconds (0: none)
    core.gate()
        .set_lock_unit(Duration::from_millis(lock_unit_ms));
    if no_time_check {
        // test hook: no request leaves the machine; waits rest on the boot clock and the device date
        core.device_set_time_check(false)
            .map_err(|e| e.to_string())?;
    }
    let state = AppState::new(core);
    if let Some(home) = bridge_home {
        // Browsers on this computer, with a stand-in home: nothing of the real one is read or written.
        #[cfg(target_os = "linux")]
        {
            let bridge = state.bridge();
            bridge.set_env(hexis_lib::bridge::Env {
                home: home.to_path_buf(),
                data_dir: data_dir.to_path_buf(),
                socket: home.join("run").join("hexis").join("bridge.sock"),
            });
            bridge.start_if_enabled();
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (home, &state);
            return Err("--bridge-home works on Linux only".into());
        }
    }
    let app = mock_builder()
        .manage(state)
        .invoke_handler(invoke_handler())
        .build(mock_context(noop_assets()))
        .map_err(|e| e.to_string())?;
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .map_err(|e| e.to_string())?;
    Ok(Backend {
        _app: app,
        window,
        opener,
        launcher,
        bridge_home: bridge_home.map(Path::to_path_buf),
    })
}

/// What a command answered.
enum Reply {
    /// `{ok}` or `{err: {code, message}}`.
    Json(Value),
    /// A raw binary answer (`file_read`): the bytes themselves.
    Raw(Vec<u8>),
}

impl Backend {
    /// Run one command.
    fn call(&self, cmd: &str, mut args: Value) -> Reply {
        // The window's long poll must not hold this single-threaded tool for seconds: it asks again at once.
        if cmd == "bridge_wait" {
            if let Some(ms) = args.get("ms").and_then(Value::as_u64) {
                args["ms"] = json!(ms.min(BRIDGE_WAIT_MAX_MS));
            }
        }
        let result = get_ipc_response(
            &self.window,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "tauri://localhost".parse().expect("static url"),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        match result {
            Ok(InvokeResponseBody::Raw(bytes)) => Reply::Raw(bytes),
            Ok(body) => Reply::Json(match body.deserialize::<Value>() {
                Ok(v) => json!({ "ok": v }),
                Err(_) => {
                    json!({"err": {"code": "ipc", "message": "The reply could not be read."}})
                }
            }),
            Err(e) if e.get("code").is_some() && e.get("message").is_some() => {
                Reply::Json(json!({ "err": e }))
            }
            // unknown command, bad arguments, ...: Tauri's own error text
            Err(e) => {
                let message = e.as_str().map_or_else(|| e.to_string(), str::to_string);
                Reply::Json(json!({"err": {"code": "ipc", "message": message}}))
            }
        }
    }
}

// ------------------------------------------------------------------ tiny HTTP layer

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Request, &'static str> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err("headers too large");
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err("connection closed"),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = std::str::from_utf8(&buf[..header_end]).map_err(|_| "bad headers")?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().ok_or("empty request")?.split(' ');
    let method = first.next().ok_or("bad request line")?.to_string();
    let path = first.next().ok_or("bad request line")?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut req = Request {
        method,
        path,
        headers,
        body: Vec::new(),
    };
    let len: usize = match req.header("content-length") {
        Some(v) => v.parse().map_err(|_| "bad content-length")?,
        None => 0,
    };
    if len > MAX_BODY_BYTES {
        return Err("body too large");
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < len {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err("connection closed"),
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(len);
    req.body = body;
    Ok(req)
}

fn respond(stream: &mut TcpStream, status: &str, cors_origin: Option<&str>, body: &Value) {
    let text = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n",
        text.len()
    );
    if let Some(origin) = cors_origin {
        head.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Max-Age: 600\r\n"
        ));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(text.as_bytes());
    let _ = stream.flush();
}

/// A raw binary answer: the bytes, `application/octet-stream` and `x-hexis-binary: 1` (which a
/// page may read across origins).
fn respond_raw(stream: &mut TcpStream, cors_origin: Option<&str>, bytes: &[u8]) {
    // a large file takes longer than the 5 s of the small JSON answers
    let _ = stream.set_write_timeout(Some(Duration::from_secs(120)));
    let mut head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nx-hexis-binary: 1\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n",
        bytes.len()
    );
    if let Some(origin) = cors_origin {
        head.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\nAccess-Control-Expose-Headers: x-hexis-binary\r\n"
        ));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(bytes);
    let _ = stream.flush();
}

fn handle(stream: &mut TcpStream, backend: &Backend, port: u16) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let req = match read_request(stream) {
        Ok(r) => r,
        Err(why) => return respond(stream, "400 Bad Request", None, &json!({"err": why})),
    };
    // DNS-rebinding guard: only our own loopback names
    let host_ok = req
        .header("host")
        .is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    if !host_ok {
        return respond(stream, "403 Forbidden", None, &json!({"err": "bad host"}));
    }
    let origin = req.header("origin");
    if origin.is_some_and(|o| !ALLOWED_ORIGINS.contains(&o)) {
        return respond(
            stream,
            "403 Forbidden",
            None,
            &json!({"err": "origin not allowed"}),
        );
    }
    let cors = origin; // already known to be allowed (or absent)

    match (req.method.as_str(), req.path.as_str()) {
        (
            "OPTIONS",
            "/ipc" | "/e2e/picker" | "/e2e/opened" | "/e2e/pick-next" | "/e2e/provider"
            | "/e2e/bridge",
        ) => respond(stream, "204 No Content", cors, &Value::Null),
        ("GET", "/health") => respond(stream, "200 OK", cors, &json!({"ok": true})),
        ("GET", "/e2e/disguise-applied") => {
            respond(stream, "200 OK", cors, &disguise_applied(&backend.launcher));
        }
        ("GET", "/e2e/opened") => {
            respond(
                stream,
                "200 OK",
                cors,
                &json!({"ok": backend.opener.urls()}),
            );
        }
        ("GET", "/e2e/last-saved") => last_saved(stream, cors),
        ("GET", "/e2e/bridge") => bridge_state(stream, cors, backend),
        ("POST", "/ipc" | "/e2e/picker" | "/e2e/opened" | "/e2e/pick-next" | "/e2e/provider") => {
            let json_type = req
                .header("content-type")
                .is_some_and(|t| t.to_ascii_lowercase().starts_with("application/json"));
            if !json_type {
                return respond(
                    stream,
                    "415 Unsupported Media Type",
                    cors,
                    &json!({"err": "send application/json"}),
                );
            }
            let Ok(parsed) = serde_json::from_slice::<Value>(&req.body) else {
                return respond(stream, "400 Bad Request", cors, &json!({"err": "bad json"}));
            };
            if req.path == "/e2e/picker" {
                return set_picker(stream, cors, &parsed);
            }
            if req.path == "/e2e/pick-next" {
                return pick_next(stream, cors, &parsed);
            }
            if req.path == "/e2e/provider" {
                return provider_call(stream, cors, backend, &parsed);
            }
            if req.path == "/e2e/opened" {
                backend.opener.clear();
                return respond(stream, "200 OK", cors, &json!({"ok": true}));
            }
            let Some(cmd) = parsed.get("cmd").and_then(Value::as_str) else {
                return respond(
                    stream,
                    "400 Bad Request",
                    cors,
                    &json!({"err": "missing cmd"}),
                );
            };
            let args = parsed.get("args").cloned().unwrap_or_else(|| json!({}));
            let args = if args.is_null() { json!({}) } else { args };
            match backend.call(cmd, args) {
                Reply::Json(reply) => respond(stream, "200 OK", cors, &reply),
                Reply::Raw(bytes) => respond_raw(stream, cors, &bytes),
            }
        }
        _ => respond(stream, "404 Not Found", cors, &json!({"err": "not found"})),
    }
}

/// `POST /e2e/provider`: one operation of the passkey provider on the open vault. An app caller is refused here (it would need
/// the network); the answer is the façade's own JSON text.
fn provider_call(stream: &mut TcpStream, cors: Option<&str>, backend: &Backend, body: &Value) {
    use tauri::Manager;
    let op = body.get("op").and_then(Value::as_str).unwrap_or("");
    let args = body.get("args").cloned().unwrap_or_else(|| json!({}));
    if args.pointer("/caller/kind").and_then(Value::as_str) == Some("app") {
        respond(
            stream,
            "400 Bad Request",
            cors,
            &json!({"err": "an app caller needs the network"}),
        );
        return;
    }
    let core = backend._app.state::<AppState>().inner().core();
    let provider = hexis_lib::provider::Provider::new(core).with_links(Box::new(|_| {
        Err(hexis_lib::passkey::PasskeyError::invalid("no network here"))
    }));
    let answer = provider.call(op, &args.to_string());
    let value: Value = serde_json::from_str(&answer).unwrap_or(Value::Null);
    respond(stream, "200 OK", cors, &value)
}

/// `GET /e2e/disguise-applied`: the last look the launcher was asked to apply (`ok` is `null` before
/// the first call). The fields are also repeated at the top level, next to `ok`.
fn disguise_applied(launcher: &RecordingLauncher) -> Value {
    let Some(last) = launcher.last() else {
        return json!({"ok": null, "presetId": null, "appliedName": null, "count": 0});
    };
    let v = &last.view;
    let fields = json!({
        "presetId": last.preset_id,
        "appliedName": v.applied_name,
        "customName": v.custom_name,
        "lowercase": v.lowercase,
        "note": v.note,
        "count": launcher.count(),
    });
    let mut out = fields.clone();
    out["ok"] = fields;
    out
}

/// `{"open": "/path" | null, "save": "/path" | null}` -> the two picker variables.
fn set_picker(stream: &mut TcpStream, cors: Option<&str>, body: &Value) {
    for (key, var) in [("open", OPEN_VAR), ("save", SAVE_VAR)] {
        match body.get(key) {
            None => {}
            Some(Value::Null) => std::env::remove_var(var),
            Some(Value::String(p)) if !p.is_empty() => std::env::set_var(var, p),
            Some(_) => {
                return respond(
                    stream,
                    "400 Bad Request",
                    cors,
                    &json!({"err": format!("{key} must be a path or null")}),
                )
            }
        }
    }
    respond(stream, "200 OK", cors, &json!({"ok": true}));
}

/// `{"paths": ["/abs/a", ...]}` -> the answer of the next file dialog (see the module docs).
fn pick_next(stream: &mut TcpStream, cors: Option<&str>, body: &Value) {
    let bad = |stream: &mut TcpStream, why: &str| {
        respond(stream, "400 Bad Request", cors, &json!({"err": why}))
    };
    // The one-file spelling of the importer / exporter dialogs: `{"path": "/abs/file" | null}` (null cancels).
    let single;
    let list = match body.get("path") {
        Some(Value::Null) => Some(&[][..]),
        Some(v @ Value::String(_)) => {
            single = [v.clone()];
            Some(&single[..])
        }
        _ => None,
    }
    .or_else(|| {
        body.get("paths")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
    });
    let Some(list) = list else {
        return bad(stream, "paths must be a list of absolute paths");
    };
    if list.len() > MAX_PICK_PATHS {
        return bad(stream, "too many paths");
    }
    let mut paths = Vec::with_capacity(list.len());
    for v in list {
        match v.as_str().map(PathBuf::from) {
            Some(p) if p.is_absolute() => paths.push(p),
            _ => return bad(stream, "paths must be a list of absolute paths"),
        }
    }
    let mut queue = PICK_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
    queue.push_back(paths);
    let queued = queue.len();
    drop(queue);
    respond(stream, "200 OK", cors, &json!({"ok": {"queued": queued}}));
}

/// The file the most recent save dialog chose, as it is on disk now.
fn last_saved(stream: &mut TcpStream, cors: Option<&str>) {
    let last = LAST_SAVED.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let Some((path, suggested)) = last else {
        return respond(stream, "200 OK", cors, &json!({"ok": null}));
    };
    let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file());
    let size = meta.as_ref().map(|m| m.len());
    let content = match size {
        Some(n) if n <= LAST_SAVED_INLINE_BYTES => std::fs::read(&path).ok().map(|b| B64.encode(b)),
        _ => None,
    };
    respond(
        stream,
        "200 OK",
        cors,
        &json!({"ok": {
            "path": path.display().to_string(),
            "suggestedName": suggested,
            "exists": meta.is_some(),
            "size": size,
            "contentBase64": content,
        }}),
    );
}

/// `GET /e2e/bridge`: whether the bridge listens, where, and whether a request waits for the window.
fn bridge_state(stream: &mut TcpStream, cors: Option<&str>, backend: &Backend) {
    use tauri::Manager;
    let Some(home) = &backend.bridge_home else {
        return respond(
            stream,
            "200 OK",
            cors,
            &json!({"err": "started without --bridge-home"}),
        );
    };
    let bridge = backend._app.state::<AppState>().inner().bridge().clone();
    #[cfg(unix)]
    let listening = bridge.listening();
    #[cfg(not(unix))]
    let listening = false;
    let waiting = serde_json::to_value(bridge.front())
        .ok()
        .and_then(|v| v.get("front").cloned())
        .is_some_and(|f| !f.is_null());
    respond(
        stream,
        "200 OK",
        cors,
        &json!({"ok": {
            "listening": listening,
            "socket": home.join("run").join("hexis").join("bridge.sock").display().to_string(),
            "home": home.display().to_string(),
            "waiting": waiting,
        }}),
    );
}

fn main() {
    // The launcher script the bridge writes starts this program as the browsers' native messaging host.
    if std::env::args().nth(1).as_deref() == Some("--native-host") {
        std::process::exit(hexis_lib::native_host());
    }
    let opts = match parse_args() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    if let Err(msg) = check_data_dir(&opts.data_dir) {
        eprintln!("{msg}");
        std::process::exit(2);
    }
    if let Some(home) = &opts.xdg_data_home {
        if let Err(msg) = check_xdg_data_home(home) {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
    if let Some(home) = &opts.bridge_home {
        if let Err(msg) = check_bridge_home(home) {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
    let backend = match build_backend(&opts) {
        Ok(b) => b,
        Err(msg) => {
            eprintln!("could not start the backend: {msg}");
            std::process::exit(1);
        }
    };
    let listener = match TcpListener::bind(("127.0.0.1", opts.port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on 127.0.0.1:{}: {e}", opts.port);
            std::process::exit(1);
        }
    };
    println!(
        "hexis e2e shim on http://127.0.0.1:{} (data: {}, kdf: {})\nDEV ONLY. Ctrl+C to stop.",
        opts.port,
        opts.data_dir.display(),
        if opts.fast_kdf {
            "fast, for tests"
        } else {
            "production"
        }
    );
    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => handle(&mut s, &backend, opts.port),
            Err(e) => eprintln!("accept failed: {e}"),
        }
    }
}
