//! A chunk's encoded packets on disk until the joiner muxes them (fork 6E.3, DESIGN-6E3 §2.4).
//!
//! Record: `u32 len, u8 key, u32 duration, i32 cts` (little endian), then `len` bytes. Reading
//! checks every length against what is left of the file, so a truncated or damaged file is an
//! error, never a panic or a huge allocation.

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

/// One encoded picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpillPacket {
    pub data: Vec<u8>,
    /// A sync sample (IDR).
    pub key: bool,
    /// In the track's timescale.
    pub duration: u32,
    /// Composition offset.
    pub cts: i32,
}

const HEADER: u64 = 13;

/// Appends packets to a spill file.
pub struct SpillWriter {
    out: BufWriter<std::fs::File>,
}

impl SpillWriter {
    /// Creates (or truncates) `path`.
    pub fn create(path: &Path) -> io::Result<SpillWriter> {
        Ok(SpillWriter { out: BufWriter::new(std::fs::File::create(path)?) })
    }

    /// Writes one packet (packets above 4 GiB are refused).
    pub fn push(&mut self, p: &SpillPacket) -> io::Result<()> {
        let len = u32::try_from(p.data.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "packet too large"))?;
        self.out.write_all(&len.to_le_bytes())?;
        self.out.write_all(&[u8::from(p.key)])?;
        self.out.write_all(&p.duration.to_le_bytes())?;
        self.out.write_all(&p.cts.to_le_bytes())?;
        self.out.write_all(&p.data)
    }

    /// Flushes and closes the file.
    pub fn finish(mut self) -> io::Result<()> {
        self.out.flush()?;
        self.out.get_ref().sync_data()
    }
}

/// Every packet of the spill file at `path`.
pub fn read_all(path: &Path) -> io::Result<Vec<SpillPacket>> {
    let file = std::fs::File::open(path)?;
    let mut left = file.metadata()?.len();
    let mut r = BufReader::new(file);
    let mut out = Vec::new();
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, format!("spill file {}: {m}", path.display()));
    while left > 0 {
        if left < HEADER {
            return Err(bad("truncated record header"));
        }
        let mut h = [0u8; HEADER as usize];
        r.read_exact(&mut h)?;
        left -= HEADER;
        let field = |i: usize| -> [u8; 4] { [h[i], h[i + 1], h[i + 2], h[i + 3]] };
        let len = u32::from_le_bytes(field(0));
        let key = match h[4] {
            0 => false,
            1 => true,
            _ => return Err(bad("bad key flag")),
        };
        let duration = u32::from_le_bytes(field(5));
        let cts = i32::from_le_bytes(field(9));
        if u64::from(len) > left {
            return Err(bad("record longer than the file"));
        }
        let mut data = vec![0u8; len as usize];
        r.read_exact(&mut data)?;
        left -= u64::from(len);
        out.push(SpillPacket { data, key, duration, cts });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fc-spill-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d.join("0.0.pkts")
    }

    #[test]
    fn packets_round_trip() {
        let p = tmp("rt");
        let pk = vec![
            SpillPacket { data: vec![0, 0, 1, 0x26, 1, 2, 3], key: true, duration: 1001, cts: 0 },
            SpillPacket { data: vec![], key: false, duration: 1001, cts: -2 },
            SpillPacket { data: vec![7; 70_000], key: false, duration: u32::MAX, cts: i32::MIN },
        ];
        let mut w = SpillWriter::create(&p).expect("create");
        for k in &pk {
            w.push(k).expect("push");
        }
        w.finish().expect("finish");
        assert_eq!(read_all(&p).expect("read"), pk);
        let empty = tmp("empty");
        SpillWriter::create(&empty).expect("create").finish().expect("finish");
        assert!(read_all(&empty).expect("read").is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap_or(&p));
        let _ = std::fs::remove_dir_all(empty.parent().unwrap_or(&empty));
    }

    #[test]
    fn damaged_files_are_errors() {
        let p = tmp("bad");
        let mut good = Vec::new();
        good.extend(5u32.to_le_bytes());
        good.push(1);
        good.extend(1001u32.to_le_bytes());
        good.extend(0i32.to_le_bytes());
        good.extend([1, 2, 3, 4, 5]);
        // every truncation of a valid record, a huge length, a bad flag
        for cut in 1..good.len() {
            std::fs::write(&p, &good[..cut]).expect("write");
            assert!(read_all(&p).is_err(), "cut {cut}");
        }
        let mut huge = good.clone();
        huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&p, &huge).expect("write");
        assert!(read_all(&p).is_err());
        let mut flag = good.clone();
        flag[4] = 9;
        std::fs::write(&p, &flag).expect("write");
        assert!(read_all(&p).is_err());
        std::fs::write(&p, &good).expect("write");
        assert_eq!(read_all(&p).expect("read").len(), 1);
        assert!(read_all(&p.with_extension("missing")).is_err());
        let _ = std::fs::remove_dir_all(p.parent().unwrap_or(&p));
    }
}
