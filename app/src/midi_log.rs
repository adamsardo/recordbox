//! The controller log the owner can send after a session (`PREPDECK_MIDI_LOG`).
//!
//! `PREPDECK_MIDI_LOG=1` writes `<data folder>/midi.log`; any other value except `0` is taken
//! as the path of the file. One timestamped line per message received (port, bytes, the
//! function and deck its row names or "unmapped", and what it did), per message sent, per
//! port that comes or goes (with where its mapping came from and how many rows were taken),
//! and per state change the interface made for a mapped message. The file is rotated at 5 MB
//! (the previous one is kept as `midi.log.1`).
//!
//! Costs nothing when it is off (one cached flag). Lines are queued to a thread of their own
//! that does the writing, so neither the port threads nor the interface wait for the disk.
//! The audio thread never logs.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

struct Log {
    tx: crossbeam_channel::Sender<String>,
    path: PathBuf,
}

fn log() -> Option<&'static Log> {
    static L: OnceLock<Option<Log>> = OnceLock::new();
    L.get_or_init(|| {
        let v = std::env::var("PREPDECK_MIDI_LOG").ok()?;
        let path = match v.trim() {
            "" | "0" => return None,
            "1" => crate::db::data_dir().join("midi.log"),
            p => PathBuf::from(p),
        };
        let (tx, rx) = crossbeam_channel::unbounded::<String>();
        let p2 = path.clone();
        std::thread::Builder::new().name("midi-log".into()).spawn(move || writer(p2, rx)).ok()?;
        Some(Log { tx, path })
    })
    .as_ref()
}

/// The log is being written.
pub fn on() -> bool {
    log().is_some()
}

/// Where it is written, when it is.
pub fn path() -> Option<PathBuf> {
    log().map(|l| l.path.clone())
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

fn send(line: String) {
    if let Some(l) = log() {
        let _ = l.tx.send(format!("{} {line}", stamp()));
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

/// A message a port received, with what its row made of it.
pub fn rx(port: &str, bytes: &[u8], what: &str) {
    if on() {
        send(format!("IN  [{port}] {} -> {what}", hex(bytes)));
    }
}

/// A light sent to a port: the message and what it shows. (The outputs that change every
/// frame, the jog ring and the level meters, and each on / off of a blinking light are not
/// logged one by one: `sent_summary` gives their counts.)
pub fn tx(port: &str, bytes: &[u8], what: &str) {
    if on() {
        send(format!("OUT [{port}] {} {what}", hex(bytes)));
    }
}

/// How many messages a port was sent and received in all, every 10 s while it has traffic.
pub fn sent_summary(port: &str, received: u64, sent: u64) {
    if on() {
        send(format!("COUNT [{port}] received {received}, sent {sent} in all"));
    }
}

/// A port came or went (`event`: "connected", "disconnected"), with details.
pub fn port(name: &str, event: &str, detail: &str) {
    if on() {
        send(format!("PORT [{name}] {event}{}{detail}", if detail.is_empty() { "" } else { ": " }));
    }
}

/// What the interface did for a mapped message (the state after it).
pub fn did(port: &str, function: &str, deck: u8, state: &str) {
    if on() {
        send(format!("DID [{port}] {function} deck {deck}: {state}"));
    }
}

/// Anything else worth a line (the session's start, the ports seen).
pub fn note(text: &str) {
    if on() {
        send(format!("NOTE {text}"));
    }
}

fn open(path: &PathBuf) -> Option<std::fs::File> {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

/// The name the previous file is kept under: `midi.log` -> `midi.log.1`.
pub fn rotated(path: &std::path::Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".1");
    PathBuf::from(s)
}

fn writer(path: PathBuf, rx: crossbeam_channel::Receiver<String>) {
    let mut file = open(&path);
    let mut size = file.as_ref().and_then(|f| f.metadata().ok()).map(|m| m.len()).unwrap_or(0);
    let mut buf = std::io::BufWriter::new(Vec::<u8>::new());
    let _ = writeln!(buf, "{} NOTE log opened: recordbox {} (pid {})", stamp(), env!("CARGO_PKG_VERSION"), std::process::id());
    loop {
        // Lines that came together are written together; the file is flushed at least every
        // 200 ms while lines come, so a crash loses little.
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(l) => {
                let _ = writeln!(buf, "{l}");
                for l in rx.try_iter().take(10_000) {
                    let _ = writeln!(buf, "{l}");
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        let bytes = std::mem::take(buf.get_mut());
        if bytes.is_empty() {
            continue;
        }
        if size + bytes.len() as u64 > ROTATE_BYTES {
            drop(file.take());
            let _ = std::fs::rename(&path, rotated(&path));
            file = open(&path);
            size = 0;
        }
        if let Some(f) = file.as_mut() {
            if f.write_all(&bytes).is_ok() {
                size += bytes.len() as u64;
                let _ = f.flush();
            }
        } else {
            file = open(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rotation_name_keeps_one_previous_file() {
        assert_eq!(super::rotated(std::path::Path::new("/x/midi.log")), std::path::PathBuf::from("/x/midi.log.1"));
        assert_eq!(super::rotated(std::path::Path::new("/x/s1 session.txt")), std::path::PathBuf::from("/x/s1 session.txt.1"));
    }
}
