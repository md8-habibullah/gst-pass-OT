//! The private socket between the native host (started by the browser) and the running Hexis.
//!
//! The socket lives in a folder only the user can enter (`$XDG_RUNTIME_DIR/hexis`, mode 0700, checked every time it is
//! used, so a folder somebody else made first is refused). One request per connection: the host writes one JSON line,
//! the app writes one JSON line back and closes.

use super::{wire, Bridge};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How many requests are handled at once. A flood gets the browser's own flow.
const MAX_CONNECTIONS: usize = 16;
/// A client has this long to send its one line.
const READ_WAIT: Duration = Duration::from_secs(10);

fn my_uid() -> io::Result<u32> {
    Ok(std::fs::metadata("/proc/self")?.uid())
}

/// `$XDG_RUNTIME_DIR/hexis/bridge.sock`, or `/tmp/hexis-<uid>/bridge.sock` where there is no runtime folder.
pub fn default_path() -> io::Result<PathBuf> {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir).join("hexis").join("bridge.sock")),
        None => Ok(PathBuf::from(format!("/tmp/hexis-{}", my_uid()?)).join("bridge.sock")),
    }
}

/// The folder must be ours and closed to everyone else, and not a link to somewhere else.
fn check_folder(dir: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != my_uid()? || meta.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the folder for Hexis's browser socket is not private to this user",
        ));
    }
    Ok(())
}

fn prepare_folder(dir: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    check_folder(dir)
}

pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Listen on `path`. A socket left by a crash is replaced; one that answers (another Hexis) is an error.
    pub fn start(bridge: Arc<Bridge>, path: &Path) -> io::Result<Self> {
        let dir = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no folder"))?;
        prepare_folder(dir)?;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if !meta.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "something other than a socket is in the way",
                ));
            }
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another Hexis is already listening",
                ));
            }
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("hexis-bridge".into())
                .spawn(move || accept_loop(&listener, &bridge, &stop, &active))?
        };
        Ok(Self {
            path: path.to_path_buf(),
            stop,
            thread: Some(thread),
        })
    }

    pub fn stop(mut self) {
        self.shut();
    }

    fn shut(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shut();
    }
}

fn accept_loop(
    listener: &UnixListener,
    bridge: &Arc<Bridge>,
    stop: &AtomicBool,
    active: &Arc<AtomicUsize>,
) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    let _ = reply_busy(stream);
                    continue;
                }
                let bridge = Arc::clone(bridge);
                let counter = Arc::clone(active);
                let spawned = std::thread::Builder::new()
                    .name("hexis-bridge-conn".into())
                    .spawn(move || {
                        let _ = handle(&bridge, stream);
                        counter.fetch_sub(1, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    // the thread never ran, so it never gave its place back
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(60));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn reply_busy(mut stream: UnixStream) -> io::Result<()> {
    let mut line = wire::answer_fallback("", "busy");
    line.push('\n');
    stream.write_all(line.as_bytes())
}

/// Read one line of at most [`wire::MAX_LINE_BYTES`] bytes.
fn read_line(stream: &UnixStream) -> io::Result<String> {
    stream.set_read_timeout(Some(READ_WAIT))?;
    let mut reader = BufReader::new(stream.take(wire::MAX_LINE_BYTES as u64 + 1));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.len() > wire::MAX_LINE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(line.trim_end().to_string())
}

/// Has the other end closed its side? (Checked without waiting.)
fn peer_gone(stream: &UnixStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return true;
    }
    let mut byte = [0u8; 1];
    let gone = match (&*stream).read(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != io::ErrorKind::WouldBlock,
    };
    let _ = stream.set_nonblocking(false);
    gone
}

fn handle(bridge: &Bridge, mut stream: UnixStream) -> io::Result<()> {
    let line = read_line(&stream)?;
    let alive_stream = stream.try_clone()?;
    let mut answer = bridge.submit_line(&line, &|| !peer_gone(&alive_stream));
    answer.push('\n');
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(answer.as_bytes())
}

/// The folder and the socket must be ours, as the app checked when it made them. The browser's side checks again before it says
/// anything: a folder somebody else made first (the `/tmp` fallback when there is no runtime folder), a link to another place, or a
/// socket of another user is never talked to, however the extension was registered.
fn check_socket(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::FileTypeExt;
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the socket has no folder"))?;
    check_folder(dir)?;
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket() || meta.uid() != my_uid()? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "that is not Hexis's own socket",
        ));
    }
    Ok(())
}

/// Ask a running Hexis one question (what the native host does). `Err` means no Hexis is listening (or not one of ours).
pub fn ask(path: &Path, line: &str) -> io::Result<String> {
    check_socket(path)?;
    let mut stream = UnixStream::connect(path)?;
    let mut out = line.trim_end().to_string();
    out.push('\n');
    stream.write_all(out.as_bytes())?;
    // the person may take their time: the app ends the request itself when it times out
    stream.set_read_timeout(Some(Duration::from_secs(wire::MAX_TIMEOUT_MS / 1000 + 15)))?;
    let mut reader = BufReader::new((&stream).take(wire::MAX_LINE_BYTES as u64 + 1));
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    if reply.is_empty() || reply.len() > wire::MAX_LINE_BYTES {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no answer"));
    }
    Ok(reply.trim_end().to_string())
}
