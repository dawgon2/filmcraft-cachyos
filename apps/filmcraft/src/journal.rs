//! Log records to the systemd journal with their levels (LINUX-PLAN L2.8).
//!
//! An app started from the desktop (Plasma, GNOME) runs in a systemd user scope whose standard
//! error already goes to the journal, but as plain lines: no priority, so `journalctl --user -p
//! warning` can't find FilmCraft's warnings. When standard error *is* the journal (systemd sets
//! `JOURNAL_STREAM=<dev>:<inode>` and it matches standard error), records go through the
//! journal's native socket instead, with `PRIORITY`, `SYSLOG_IDENTIFIER=filmcraft` and the
//! module as `CODE_MODULE`; standard error then stays quiet so nothing is logged twice. Started
//! from a terminal, nothing changes (Flatpak passes the journal's socket and stream through, so
//! the same path applies there). The log file is written either way.
//!
//! Pure Rust: one `UnixDatagram` and the journal's documented wire format; no libsystemd.

use std::os::unix::net::UnixDatagram;
use std::path::Path;

/// The journal's native socket.
pub const SOCKET: &str = "/run/systemd/journal/socket";
/// Records larger than this go to standard error instead (the socket's datagram limit is
/// larger, but a log line this long is a bug, not a message).
const MAX_RECORD: usize = 48 * 1024;

/// A connected journal socket.
pub struct Journal {
    sock: UnixDatagram,
}

impl Journal {
    /// The journal, if standard error is already connected to it (see the module docs).
    pub fn detect() -> Option<Journal> {
        let stream = std::env::var("JOURNAL_STREAM").ok()?;
        let (dev, ino) = stderr_id()?;
        if !stream_matches(&stream, dev, ino) {
            return None;
        }
        Journal::connect(Path::new(SOCKET))
    }

    /// Connect to a journal socket at `path`.
    pub fn connect(path: &Path) -> Option<Journal> {
        let sock = UnixDatagram::unbound().ok()?;
        sock.connect(path).ok()?;
        // the journal socket itself never blocks the thread that logs (it may be the UI thread): a
        // full queue fails the send and the record goes to standard error (which, under systemd,
        // is journald's stream socket and can still block if journald stalls)
        sock.set_nonblocking(true).ok()?;
        Some(Journal { sock })
    }

    /// Send one record; `false` when it couldn't be sent (the caller writes standard error).
    pub fn send(&self, level: log::Level, target: &str, message: &str) -> bool {
        if message.len().saturating_add(target.len()) > MAX_RECORD {
            return false; // too big for one datagram: not copied again just to be refused
        }
        let rec = encode(level, target, message);
        rec.len() <= MAX_RECORD && self.sock.send(&rec).is_ok()
    }
}

/// Standard error's (device, inode).
fn stderr_id() -> Option<(u64, u64)> {
    use std::os::fd::AsFd;
    use std::os::unix::fs::MetadataExt;
    let fd = std::io::stderr().as_fd().try_clone_to_owned().ok()?;
    let m = std::fs::File::from(fd).metadata().ok()?;
    Some((m.dev(), m.ino()))
}

/// Does `JOURNAL_STREAM` (`<dev>:<inode>`, decimal) name this device and inode?
pub fn stream_matches(stream: &str, dev: u64, ino: u64) -> bool {
    let Some((d, i)) = stream.trim().split_once(':') else { return false };
    d.parse::<u64>().ok() == Some(dev) && i.parse::<u64>().ok() == Some(ino)
}

/// syslog priority of a log level.
pub fn priority(level: log::Level) -> u8 {
    match level {
        log::Level::Error => 3,
        log::Level::Warn => 4,
        log::Level::Info => 6,
        log::Level::Debug | log::Level::Trace => 7,
    }
}

/// One record in the journal's native format. Every field uses the binary form
/// (`KEY\n<u64 LE length><bytes>\n`), so newlines and any bytes in a message are safe.
pub fn encode(level: log::Level, target: &str, message: &str) -> Vec<u8> {
    let message = message.strip_suffix('\n').unwrap_or(message);
    let mut out = Vec::with_capacity(message.len().saturating_add(target.len()).saturating_add(96));
    let mut field = |key: &str, value: &[u8]| {
        out.extend_from_slice(key.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
        out.extend_from_slice(value);
        out.push(b'\n');
    };
    field("MESSAGE", message.as_bytes());
    field("PRIORITY", priority(level).to_string().as_bytes());
    field("SYSLOG_IDENTIFIER", b"filmcraft");
    field("CODE_MODULE", target.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse the binary form back (test helper).
    fn decode(mut b: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        while let Some(nl) = b.iter().position(|&c| c == b'\n') {
            let key = String::from_utf8(b[..nl].to_vec()).unwrap();
            let len = u64::from_le_bytes(b[nl + 1..nl + 9].try_into().unwrap()) as usize;
            out.push((key, b[nl + 9..nl + 9 + len].to_vec()));
            b = &b[nl + 9 + len + 1..];
        }
        out
    }

    #[test]
    fn records_encode_with_priority_and_identifier() {
        let rec = encode(log::Level::Warn, "filmcraft::audio", "two\nlines\n");
        let f = decode(&rec);
        assert_eq!(f[0], ("MESSAGE".into(), b"two\nlines".to_vec()));
        assert_eq!(f[1], ("PRIORITY".into(), b"4".to_vec()));
        assert_eq!(f[2], ("SYSLOG_IDENTIFIER".into(), b"filmcraft".to_vec()));
        assert_eq!(f[3], ("CODE_MODULE".into(), b"filmcraft::audio".to_vec()));
        assert_eq!([log::Level::Error, log::Level::Info, log::Level::Trace].map(priority), [3, 6, 7]);
        // empty and odd messages encode fine
        assert_eq!(decode(&encode(log::Level::Info, "", ""))[0].1, b"");
        assert_eq!(decode(&encode(log::Level::Info, "t", "\u{0}é"))[0].1, "\u{0}é".as_bytes());
    }

    #[test]
    fn journal_stream_matching() {
        assert!(stream_matches("8:123456", 8, 123456));
        assert!(stream_matches(" 8:123456\n", 8, 123456));
        assert!(!stream_matches("8:123457", 8, 123456));
        for bad in ["", ":", "8", "x:y", "8:", ":5", "-1:5", "99999999999999999999:1"] {
            assert!(!stream_matches(bad, 8, 5), "{bad}");
        }
    }

    #[test]
    fn records_reach_a_journal_socket() {
        let dir = std::env::temp_dir().join(format!("filmcraft-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("socket");
        let server = UnixDatagram::bind(&path).unwrap();
        let j = Journal::connect(&path).expect("connects");
        assert!(j.send(log::Level::Error, "filmcraft", "disk full"));
        let mut buf = vec![0u8; 4096];
        let n = server.recv(&mut buf).unwrap();
        assert_eq!(decode(&buf[..n])[0].1, b"disk full");
        // too big: refused (the caller falls back to standard error)
        assert!(!j.send(log::Level::Info, "t", &"x".repeat(MAX_RECORD + 1)));
        // no socket: no journal
        assert!(Journal::connect(&dir.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
