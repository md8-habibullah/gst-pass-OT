//! Passkeys for browsers on a Linux computer (`ai-docs/PASSKEYS.md`, "Browsers on Linux").
//!
//! Linux has no passkey provider interface that browsers use, so Hexis ships a browser extension. The extension hands a
//! page's passkey request to a small native host (this same program started with `--native-host`, see [`host`]), which
//! passes it over a private socket ([`socket`]) to the running Hexis, which asks the person in its own window.
//!
//! * **This module** is the part that decides: it checks each request ([`wire`]), queues it for the window, and answers
//!   it through the same façade the Android provider uses ([`crate::provider::Provider`]) with the page's origin as the
//!   caller, so the rules are the same ones: the website name must belong to the page (and not be a name many sites
//!   share, see [`crate::passkey::origin::may_name_a_site`]), the private key never leaves the core, a request that
//!   requires verification needs the master password, and a login that asks for the master password asks for it.
//! * **Nothing is answered without a person.** A request waits in a queue until the person chooses in the Hexis window
//!   (a click is the user presence; a typed master password is the verification). Nothing is signed for a program that
//!   merely opened the socket: the most such a program gets is a prompt in the window, naming the website it claims.
//! * It is **off** until Settings > Browsers switches it on: with it off no socket exists and the extension falls back to
//!   the browser's own passkey flow.

pub mod wire;

pub mod extension;
#[cfg(unix)]
pub mod host;
#[cfg(target_os = "linux")]
pub mod install;
#[cfg(unix)]
pub mod socket;

#[cfg(test)]
mod tests;

use crate::provider::Provider;
use crate::storage::app::AppCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use wire::{Checked, Kind};

/// How many requests may wait for the window at once. More are told to use the browser's own flow.
const MAX_QUEUE: usize = 8;
/// How often a waiting request checks whether the browser end hung up.
const POLL: Duration = Duration::from_millis(400);
/// How many request ids are remembered to refuse a repeat.
const SEEN_IDS: usize = 512;

/// What the window shows for the request in front. Nothing here is secret: the site and the kind of request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    /// `get` (sign in) or `create` (save a new passkey).
    pub kind: &'static str,
    /// The page that asked, as the extension says it (`https://example.com`).
    pub origin: String,
    /// Milliseconds until the request gives up.
    pub remaining_ms: u64,
}

/// What `bridge_wait` answers: the number of the latest change and the request in front, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waited {
    pub version: u64,
    pub front: Option<View>,
    /// The vault is locked: the window shows the lock screen (and a note that a website is waiting), not the request.
    pub locked: bool,
}

/// What the person chose in the window.
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    /// Sign in with this passkey (`key` is the field it is in, `passkey`, `passkey#2` …).
    #[serde(rename_all = "camelCase")]
    Use {
        item_id: String,
        key: String,
        password: Option<String>,
    },
    /// Save a new passkey on an existing login (`{"existing": id}`) or on a new one (`{"new": {"title": …}}`).
    #[serde(rename_all = "camelCase")]
    Create {
        target: Value,
        password: Option<String>,
    },
    /// Let the browser use its own passkey flow for this page.
    Decline,
    /// Stop: the page is told the person did not allow it.
    Cancel,
}

/// `Debug` names what was chosen and never prints a typed password.
impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let typed = |p: &Option<String>| if p.is_some() { "<typed>" } else { "<none>" };
        match self {
            Action::Use {
                item_id,
                key,
                password,
            } => write!(f, "Use({item_id}, {key}, password {})", typed(password)),
            Action::Create { target, password } => {
                write!(f, "Create({target}, password {})", typed(password))
            }
            Action::Decline => f.write_str("Decline"),
            Action::Cancel => f.write_str("Cancel"),
        }
    }
}

/// Write a new file that is private from its first byte: created with the final mode (never wider, and never written first with
/// the default one), and refusing to follow or reuse anything already at the path (a link planted there is removed, not followed).
pub(crate) fn write_new(path: &std::path::Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path); // a leftover of an earlier run, or a link somebody planted
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

/// A failure the window can show: a stable code and a sentence for the person (the same shape every command's error has).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
}

impl Failure {
    pub(crate) fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<crate::storage::error::StorageError> for Failure {
    fn from(e: crate::storage::error::StorageError) -> Self {
        Self {
            code: e.code().into(),
            message: e.to_string(),
        }
    }
}

/// Where this installation lives, for registering with browsers and for the socket. Set by the running app only; the tests and
/// the browser-less e2e shim set a temporary one, so nothing ever touches the real home folder by accident.
#[derive(Debug, Clone)]
pub struct Env {
    /// The home folder the browsers' per-user folders are below.
    pub home: PathBuf,
    /// Hexis's own data folder (the launcher script is kept here).
    pub data_dir: PathBuf,
    /// The socket to listen on.
    pub socket: PathBuf,
}

/// What Settings > Browsers shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// This computer can use the bridge (Linux desktop).
    pub supported: bool,
    /// The switch.
    pub enabled: bool,
    /// The socket is open right now.
    pub listening: bool,
    /// Why the socket could not be opened, when the switch is on and it is not open.
    pub error: Option<String>,
    /// Each browser Hexis knows: found, registered, stale.
    pub browsers: Vec<Value>,
    /// Browsers found that run in a sandbox and so cannot use Hexis.
    pub sandboxed: Vec<String>,
    /// The folders holding the extension, once Hexis has written them (switching on, or connecting a browser, does).
    pub extension: Option<ExtensionFolders>,
    /// The extension's addresses, so the page can name them.
    pub chromium_id: &'static str,
    pub firefox_id: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionFolders {
    pub chromium: String,
    pub firefox: String,
}

struct Extras {
    env: Option<Env>,
    error: Option<String>,
    /// Called when a request arrives, so the window can ask to come to the front.
    attention: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// The extensions' addresses (kept next to the code that writes them into browsers' manifests).
mod ids {
    pub const CHROMIUM: &str = "pejbejenceojhjhcanlfnibgdihclnhm";
    pub const FIREFOX: &str = "hexis-passkeys@habibullah.dev";
}

#[cfg(target_os = "linux")]
fn io_failure(e: std::io::Error) -> Failure {
    Failure::new(
        "storage",
        &format!("Hexis could not change the browsers' folders: {e}"),
    )
}

struct Pending {
    id: String,
    kind: Kind,
    origin: String,
    request: String,
    deadline: Instant,
    reply: SyncSender<String>,
}

#[derive(Default)]
struct Inner {
    queue: VecDeque<Pending>,
    /// The ids of the latest requests, answered or not: an id is used once, so the window's request in front can never turn
    /// into another request with the same id between two looks (the extension makes a new random id for every call).
    seen: VecDeque<String>,
    /// Goes up with every change a waiting window should hear about.
    version: u64,
}

/// The queue, and the rules that answer it.
pub struct Bridge {
    core: Arc<AppCore>,
    provider: Provider,
    inner: Mutex<Inner>,
    changed: Condvar,
    extras: Mutex<Extras>,
    #[cfg(unix)]
    server: Mutex<Option<socket::Server>>,
}

impl Bridge {
    pub fn new(core: Arc<AppCore>) -> Self {
        Self {
            provider: Provider::new(Arc::clone(&core)),
            core,
            inner: Mutex::new(Inner::default()),
            changed: Condvar::new(),
            extras: Mutex::new(Extras {
                env: None,
                error: None,
                attention: None,
            }),
            #[cfg(unix)]
            server: Mutex::new(None),
        }
    }

    fn extras(&self) -> MutexGuard<'_, Extras> {
        self.extras.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Production and the e2e shim: say where this installation lives. Without it the bridge is not supported.
    pub fn set_env(&self, env: Env) {
        self.extras().env = Some(env);
    }

    /// Production: how the window is asked to come to the front when a request arrives.
    pub fn set_attention(&self, f: impl Fn() + Send + Sync + 'static) {
        self.extras().attention = Some(Arc::new(f));
    }

    fn env(&self) -> Option<Env> {
        self.extras().env.clone()
    }

    fn guard(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Run a façade operation and read its answer.
    fn run(&self, op: &str, args: Value) -> Result<Value, Failure> {
        let text = self.provider.call(op, &args.to_string());
        let v: Value = serde_json::from_str(&text)
            .map_err(|_| Failure::new("internal", "The passkey service gave no answer."))?;
        if v["ok"] == true {
            Ok(v["data"].clone())
        } else {
            Err(Failure {
                code: v["code"].as_str().unwrap_or("internal").into(),
                message: v["message"].as_str().unwrap_or("That did not work.").into(),
            })
        }
    }

    fn caller(origin: &str) -> Value {
        json!({"kind": "page", "origin": origin})
    }

    fn locked(&self) -> bool {
        self.core
            .status()
            .map(|s| !s.unlocked || s.resumable)
            .unwrap_or(true)
    }

    // ------------------------------------------------------------------ the browser's side

    /// Answer one request line from the socket. Blocks until the person answers, the request times out, or `alive` says
    /// the browser end is gone. Always returns one line of JSON; never panics.
    pub fn submit_line(&self, line: &str, alive: &dyn Fn() -> bool) -> String {
        let checked = match wire::check(line) {
            Ok(c) => c,
            Err(r) => return wire::answer_error(r.id.as_deref(), "invalid_input", r.message),
        };
        match checked {
            Checked::Status { id } => {
                if !self.core.browser_bridge() {
                    return wire::answer_fallback(&id, "disabled");
                }
                wire::answer_status(&id)
            }
            Checked::Ask {
                id,
                kind,
                origin,
                request,
                timeout_ms,
            } => self.ask(id, kind, origin, request, timeout_ms, alive),
        }
    }

    fn ask(
        &self,
        id: String,
        kind: Kind,
        origin: String,
        request: String,
        timeout_ms: u64,
        alive: &dyn Fn() -> bool,
    ) -> String {
        if !self.core.browser_bridge() {
            return wire::answer_fallback(&id, "disabled");
        }
        if self.core.gate().is_closed() {
            return wire::answer_fallback(&id, "app_locked");
        }
        if self
            .core
            .profile_list()
            .map_or(true, |l| l.profiles.is_empty())
        {
            return wire::answer_fallback(&id, "no_vault");
        }
        // Look at the request before bothering the person, while the vault is open: a request this page may not make (another
        // website's name, a name many sites share, something that is not a passkey request) goes to the browser's own flow, which
        // reports it in its own words; a sign-in with no passkey here for this website goes there too, with no prompt at all.
        if !self.locked() {
            let op = match kind {
                Kind::Get => "getOptions",
                Kind::Create => "createOptions",
            };
            match self.run(
                op,
                json!({"request": request, "caller": Self::caller(&origin)}),
            ) {
                Ok(data) => {
                    if kind == Kind::Get && data["matches"].as_array().is_none_or(Vec::is_empty) {
                        return wire::answer_fallback(&id, "no_match");
                    }
                }
                Err(f) if f.code == "invalid_input" => {
                    return wire::answer_fallback(&id, "invalid_request");
                }
                Err(f) => return wire::answer_error(Some(&id), &f.code, &f.message),
            }
        }
        let (tx, rx) = mpsc::sync_channel::<String>(1);
        {
            let mut g = self.guard();
            if g.queue.len() >= MAX_QUEUE
                || g.queue.iter().any(|p| p.id == id)
                || g.seen.contains(&id)
            {
                return wire::answer_fallback(&id, "busy");
            }
            g.seen.push_back(id.clone());
            if g.seen.len() > SEEN_IDS {
                g.seen.pop_front();
            }
            g.queue.push_back(Pending {
                id: id.clone(),
                kind,
                origin,
                request,
                deadline: Instant::now() + Duration::from_millis(timeout_ms),
                reply: tx,
            });
            g.version += 1;
        }
        self.changed.notify_all();
        let attention = self.extras().attention.clone();
        if let Some(f) = attention {
            f();
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.remove(&id);
                return wire::answer_timeout(&id);
            }
            match rx.recv_timeout(left.min(POLL)) {
                Ok(answer) => return answer,
                Err(RecvTimeoutError::Timeout) => {
                    if !alive() {
                        // the tab was closed or the browser went away: the prompt has no one to answer
                        self.remove(&id);
                        return wire::answer_cancelled(&id);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return wire::answer_cancelled(&id),
            }
        }
    }

    fn remove(&self, id: &str) -> Option<Pending> {
        let mut g = self.guard();
        let at = g.queue.iter().position(|p| p.id == id)?;
        let p = g.queue.remove(at);
        g.version += 1;
        drop(g);
        self.changed.notify_all();
        p
    }

    // ------------------------------------------------------------------ the window's side

    fn view_of(p: &Pending) -> View {
        View {
            id: p.id.clone(),
            kind: p.kind.as_str(),
            origin: p.origin.clone(),
            remaining_ms: u64::try_from(
                p.deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
            )
            .unwrap_or(u64::MAX),
        }
    }

    /// The request in front, as it is now.
    pub fn front(&self) -> Waited {
        let (version, front) = {
            let g = self.guard();
            (g.version, g.queue.front().map(Self::view_of))
        };
        Waited {
            version,
            front,
            locked: self.locked(),
        }
    }

    /// Wait until something changed since `after` (a request arrived, ended or was answered), at most `ms`. The window calls this
    /// in a loop while the bridge is on, so it hears of a request at once and costs nothing in between.
    pub fn wait(&self, after: u64, ms: u64) -> Waited {
        let limit = Instant::now() + Duration::from_millis(ms.min(30_000));
        let mut g = self.guard();
        while g.version == after {
            let left = limit.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let (next, timed_out) = self
                .changed
                .wait_timeout(g, left)
                .unwrap_or_else(|p| p.into_inner());
            g = next;
            if timed_out.timed_out() {
                break;
            }
        }
        let (version, front) = (g.version, g.queue.front().map(Self::view_of));
        drop(g);
        Waited {
            version,
            front,
            locked: self.locked(),
        }
    }

    fn find(&self, id: &str) -> Result<(Kind, String, String), Failure> {
        let g = self.guard();
        g.queue
            .iter()
            .find(|p| p.id == id)
            .map(|p| (p.kind, p.origin.clone(), p.request.clone()))
            .ok_or_else(|| Failure::new("not_found", "That request is no longer waiting."))
    }

    /// What the window offers for a request: the passkeys that fit (sign in) or the logins to save on (new passkey).
    pub fn options(&self, id: &str) -> Result<Value, Failure> {
        let (kind, origin, request) = self.find(id)?;
        if self.locked() {
            return Err(Failure::new("locked", "Unlock Hexis first."));
        }
        let op = match kind {
            Kind::Get => "getOptions",
            Kind::Create => "createOptions",
        };
        let mut data = match self.run(
            op,
            json!({"request": request, "caller": Self::caller(&origin)}),
        ) {
            Ok(d) => d,
            Err(f) => {
                if f.code == "invalid_input" {
                    // seen only now because the vault was locked when it came: it is not one this page may make
                    self.send(id, wire::answer_fallback(id, "invalid_request"));
                }
                return Err(f);
            }
        };
        data["kind"] = json!(kind.as_str());
        data["origin"] = json!(origin);
        Ok(data)
    }

    /// Carry out what the person chose. A wrong password, a login that needs the master password and a locked vault leave the
    /// request waiting so the person can try again; every other outcome ends it.
    pub fn answer(&self, id: &str, action: Action) -> Result<(), Failure> {
        let (kind, origin, request) = self.find(id)?;
        let reply = match action {
            Action::Decline => wire::answer_fallback(id, "declined"),
            Action::Cancel => wire::answer_cancelled(id),
            Action::Use {
                item_id,
                key,
                password,
            } => {
                if kind != Kind::Get {
                    return Err(Failure::new(
                        "invalid_input",
                        "That is not a sign-in request.",
                    ));
                }
                Self::need_password_if_required(kind, &request, password.as_deref())?;
                let verified = password.is_some();
                let args = json!({
                    "request": request, "caller": Self::caller(&origin),
                    "itemId": item_id, "key": key,
                    "userVerified": verified, "password": password,
                });
                self.finish("sign", args, id)?
            }
            Action::Create { target, password } => {
                if kind != Kind::Create {
                    return Err(Failure::new("invalid_input", "That is not a save request."));
                }
                Self::need_password_if_required(kind, &request, password.as_deref())?;
                let verified = password.is_some();
                let args = json!({
                    "request": request, "caller": Self::caller(&origin),
                    "target": target,
                    "userVerified": verified, "password": password,
                });
                self.finish("create", args, id)?
            }
        };
        self.send(id, reply);
        Ok(())
    }

    /// A website that requires user verification gets it only from a typed master password; the window is told to ask again
    /// (the request keeps waiting) instead of the page being sent an error.
    fn need_password_if_required(
        kind: Kind,
        request: &str,
        password: Option<&str>,
    ) -> Result<(), Failure> {
        if password.is_some() {
            return Ok(());
        }
        let v: Value = serde_json::from_str(request).unwrap_or(Value::Null);
        let wanted = match kind {
            Kind::Get => &v["userVerification"],
            Kind::Create => &v["authenticatorSelection"]["userVerification"],
        };
        if wanted == "required" {
            return Err(Failure::new(
                "password_required",
                "This website requires your master password.",
            ));
        }
        Ok(())
    }

    /// Run a signing or saving operation. `Err` means "the window should show this and the request stays"; `Ok` is the line to
    /// send, which can itself be an error answer for the page.
    fn finish(&self, op: &str, args: Value, id: &str) -> Result<String, Failure> {
        match self.run(op, args) {
            Ok(data) => Ok(wire::answer_ok(
                id,
                data["responseJson"].as_str().unwrap_or("{}"),
            )),
            Err(f)
                if matches!(
                    f.code.as_str(),
                    "wrong_password" | "password_required" | "locked" | "master_required"
                ) =>
            {
                Err(f)
            }
            Err(f) => Ok(wire::answer_error(Some(id), &f.code, &f.message)),
        }
    }

    fn send(&self, id: &str, line: String) {
        if let Some(p) = self.remove(id) {
            // the other end may be gone already; that is not an error
            let _ = p.reply.try_send(line);
        }
    }

    /// Stop everything that waits: the bridge was switched off, or the app is closing.
    pub fn end_all(&self, reason: &str) {
        let drained: Vec<Pending> = {
            let mut g = self.guard();
            let all: Vec<Pending> = g.queue.drain(..).collect();
            if !all.is_empty() {
                g.version += 1;
            }
            all
        };
        for p in drained {
            let _ = p.reply.try_send(wire::answer_fallback(&p.id, reason));
        }
        self.changed.notify_all();
    }

    // ------------------------------------------------------------------ settings

    /// What Settings > Browsers shows.
    pub fn status(&self) -> Status {
        let env = self.env();
        let enabled = self.core.browser_bridge();
        #[cfg(unix)]
        let listening = self.listening();
        #[cfg(not(unix))]
        let listening = false;
        let mut browsers = Vec::new();
        let mut sandboxed = Vec::new();
        #[cfg(target_os = "linux")]
        if let Some(env) = &env {
            let found = install::status(&env.home, &env.data_dir);
            browsers = found
                .browsers
                .iter()
                .filter_map(|b| serde_json::to_value(b).ok())
                .collect();
            sandboxed = found.sandboxed;
        }
        let extension = env
            .as_ref()
            .filter(|e| extension::present(&e.data_dir))
            .map(|e| {
                let f = extension::folders(&e.data_dir);
                ExtensionFolders {
                    chromium: f.chromium.to_string_lossy().into_owned(),
                    firefox: f.firefox.to_string_lossy().into_owned(),
                }
            });
        Status {
            supported: cfg!(target_os = "linux") && env.is_some(),
            enabled,
            listening,
            error: self.extras().error.clone(),
            browsers,
            sandboxed,
            extension,
            chromium_id: ids::CHROMIUM,
            firefox_id: ids::FIREFOX,
        }
    }

    /// The switch: on opens the socket (and is remembered only if that worked), off closes it and sends everything that waits
    /// to the browsers' own flow.
    #[cfg(unix)]
    pub fn set_enabled(self: &Arc<Self>, on: bool) -> Result<Status, Failure> {
        if on {
            if !cfg!(target_os = "linux") || self.env().is_none() {
                return Err(Failure::new(
                    "unsupported",
                    "Browsers can use Hexis only on a Linux computer.",
                ));
            }
            self.start().map_err(|e| {
                let message = if e.kind() == std::io::ErrorKind::AddrInUse {
                    "Another Hexis is already open and listening for browsers.".to_string()
                } else {
                    format!("Hexis could not open its private socket for browsers: {e}")
                };
                self.extras().error = Some(message.clone());
                Failure::new("bridge_failed", &message)
            })?;
            if let Err(e) = self.core.device_set_browser_bridge(true) {
                self.stop();
                return Err(e.into());
            }
            self.write_extension();
            self.extras().error = None;
        } else {
            self.core.device_set_browser_bridge(false)?;
            self.stop();
            self.extras().error = None;
        }
        Ok(self.status())
    }

    /// Start up with the saved setting: listen when it is on. A problem is kept for the page to show, not raised.
    #[cfg(unix)]
    pub fn start_if_enabled(self: &Arc<Self>) {
        if self.core.browser_bridge() && self.env().is_some() {
            // An AppImage that was replaced by the next version leaves a launcher that names a file that is gone: point it
            // at this program again. Best effort, and only for a launcher a registration made (see `install::repair_launcher`).
            #[cfg(target_os = "linux")]
            if let (Some(env), Ok(program)) = (self.env(), install::current_program()) {
                let _ = install::repair_launcher(&env.data_dir, &program);
            }
            if let Err(e) = self.start() {
                self.extras().error = Some(format!(
                    "Hexis could not open its private socket for browsers: {e}"
                ));
            }
        }
    }

    /// Register Hexis with the chosen browsers (by id). Needs the setting's folders to be known (the real app, or a test).
    #[cfg(target_os = "linux")]
    pub fn register(&self, ids: &[String]) -> Result<Status, Failure> {
        let env = self.env().ok_or_else(|| {
            Failure::new(
                "unsupported",
                "Browsers can use Hexis only on a Linux computer.",
            )
        })?;
        let program = install::current_program().map_err(io_failure)?;
        install::install(&env.home, &env.data_dir, &program, ids).map_err(io_failure)?;
        self.write_extension();
        Ok(self.status())
    }

    /// Take Hexis away from the chosen browsers (none given: all of them).
    #[cfg(target_os = "linux")]
    pub fn unregister(&self, ids: &[String]) -> Result<Status, Failure> {
        let env = self.env().ok_or_else(|| {
            Failure::new(
                "unsupported",
                "Browsers can use Hexis only on a Linux computer.",
            )
        })?;
        install::uninstall(&env.home, &env.data_dir, ids).map_err(io_failure)?;
        Ok(self.status())
    }

    /// Put the extension's files where the page says they are. A problem is not raised: the page shows no folder then.
    fn write_extension(&self) {
        if let Some(env) = self.env() {
            let _ = extension::write(&env.data_dir);
        }
    }

    /// "Remove Hexis from this device": stop listening and leave nothing in any browser's folder. Never fails.
    pub fn forget_browsers(&self) {
        #[cfg(unix)]
        self.stop();
        #[cfg(target_os = "linux")]
        if let Some(env) = self.env() {
            let _ = install::uninstall(&env.home, &env.data_dir, &[]);
        }
        if let Some(env) = self.env() {
            extension::remove(&env.data_dir);
            // the data folder goes too when nothing else is left in it (the wipe leaves it only for what it did not know)
            let _ = std::fs::remove_dir(&env.data_dir);
        }
    }

    // ------------------------------------------------------------------ the listener

    /// Start listening on `path`. Does nothing if it already listens.
    #[cfg(unix)]
    pub fn start_at(self: &Arc<Self>, path: &std::path::Path) -> std::io::Result<()> {
        let mut slot = self.server.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_some() {
            return Ok(());
        }
        *slot = Some(socket::Server::start(Arc::clone(self), path)?);
        Ok(())
    }

    /// Start listening on this installation's socket (`$XDG_RUNTIME_DIR/hexis/bridge.sock` unless the environment names another).
    #[cfg(unix)]
    pub fn start(self: &Arc<Self>) -> std::io::Result<()> {
        let path = match self.env() {
            Some(env) => env.socket,
            None => socket::default_path()?,
        };
        self.start_at(&path)
    }

    /// Stop listening, remove the socket and send everything that waits to the browser's own flow.
    #[cfg(unix)]
    pub fn stop(&self) {
        let server = self.server.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(s) = server {
            s.stop();
        }
        self.end_all("disabled");
    }

    #[cfg(unix)]
    pub fn listening(&self) -> bool {
        self.server
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }
}
