//! The native messaging host: what a browser starts when the Hexis extension sends it a message.
//!
//! A browser talks to a native host over its standard input and output: each message is a JSON text with a four-byte length
//! in front, in the machine's byte order. The Hexis program is that host when it is started with `--native-host` (the
//! program the browser's manifest names is a tiny launcher script that adds that flag, see [`super::install`]). The host
//! reads **one** message, hands it to the running Hexis over the private socket, writes the answer, and exits. It holds no
//! vault, key or password and decides nothing: it is a pipe.

use super::{socket, wire};
use std::io::{self, Read, Write};
use std::path::Path;

/// The largest message the host takes from the browser (a real request is a few hundred bytes).
const MAX_IN: usize = wire::MAX_LINE_BYTES;

/// Read one length-prefixed message. `Ok(None)` when the browser closed the pipe without sending anything.
pub fn read_message(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_IN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too long",
        ));
    }
    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn write_message(output: &mut impl Write, text: &str) -> io::Result<()> {
    let bytes = text.as_bytes();
    let len = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message too long"))?;
    output.write_all(&len.to_ne_bytes())?;
    output.write_all(bytes)?;
    output.flush()
}

/// The id of a request, so that even an answer made here (no Hexis running) can be matched to it.
fn id_of(message: &str) -> String {
    serde_json::from_str::<serde_json::Value>(message)
        .ok()
        .and_then(|v| v["id"].as_str().map(str::to_string))
        .filter(|id| id.len() <= 64)
        .unwrap_or_default()
}

/// What to say for one message from the browser: Hexis's answer, or (no Hexis running, or a message that is not one the
/// extension writes) an answer made here.
fn answer_for(body: &[u8], socket_path: &Path) -> String {
    let message = String::from_utf8_lossy(body).into_owned();
    if message.contains('\n') || message.contains('\r') {
        // one message is one line on the socket; a JSON text with raw line breaks inside is not one the extension writes
        return wire::answer_error(
            Some(&id_of(&message)),
            "invalid_input",
            "That message is not understood.",
        );
    }
    match socket::ask(socket_path, &message) {
        Ok(reply) => reply,
        Err(_) => wire::answer_fallback(&id_of(&message), "not_running"),
    }
}

/// Relay one message from `input` to the Hexis that listens on `socket_path` and write the answer to `output`.
pub fn relay(input: &mut impl Read, output: &mut impl Write, socket_path: &Path) -> io::Result<()> {
    let Some(body) = read_message(input)? else {
        return Ok(());
    };
    write_message(output, &answer_for(&body, socket_path))
}

/// `hexis --native-host …`: the browser's own arguments (the extension's address, or the manifest's path) are ignored.
pub fn run() -> i32 {
    let Ok(path) = socket::default_path() else {
        return 1;
    };
    let body = match read_message(&mut io::stdin().lock()) {
        Ok(Some(body)) => body,
        Ok(None) => return 0,
        Err(_) => return 1,
    };
    // From here the browser needs nothing more from this end of the pipe. When it closes the pipe (the page was closed or the
    // extension gave up) this process ends, which closes the socket, which tells Hexis to drop the prompt it showed.
    let watcher = std::thread::Builder::new()
        .name("hexis-host-stdin".into())
        .spawn(|| {
            let mut byte = [0u8; 1];
            let mut stdin = io::stdin().lock();
            loop {
                match stdin.read(&mut byte) {
                    Ok(0) | Err(_) => std::process::exit(0),
                    Ok(_) => {}
                }
            }
        });
    if watcher.is_err() {
        return 1;
    }
    match write_message(&mut io::stdout().lock(), &answer_for(&body, &path)) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
