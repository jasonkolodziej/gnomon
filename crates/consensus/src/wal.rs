//! Framed, checksummed, fsync'd append-only files: the durability primitive
//! under both the Raft [`DiskStore`](crate::disk::DiskStore) and the
//! controller's [`EventJournal`](crate::journal::EventJournal).
//!
//! Every record is `len: u32 LE | crc32(payload): u32 LE | payload`.
//! Appends are `write_all` + `sync_data`, so a record that was acknowledged
//! survives a crash. Recovery reads frames front to back:
//!
//! - a short header, a length running past end-of-file, or a checksum
//!   mismatch on the **final** record is a torn tail (the crash interrupted
//!   the last append): it is dropped and the file truncated back to the last
//!   good frame;
//! - a checksum mismatch on any **earlier** record is real corruption and an
//!   error — silently skipping it would lose acknowledged, committed data.
//!
//! Small single-record files (vote, metadata, snapshot) are replaced
//! atomically with [`atomic_write`]: temp file + fsync + rename + fsync dir.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const HEADER: usize = 8;

/// IEEE CRC-32 (the zlib/Ethernet polynomial), bitwise: no table, no dep.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Encode one payload as a frame.
pub fn encode_frame(payload: &[u8]) -> io::Result<Vec<u8>> {
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "record larger than 4 GiB"))?;
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&crc32(payload).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// One decoded frame and where it starts in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub offset: u64,
    pub payload: Vec<u8>,
}

/// Result of scanning a framed file.
#[derive(Debug, PartialEq, Eq)]
pub struct Scan {
    pub frames: Vec<Frame>,
    /// Byte length of the valid prefix (where the next append belongs).
    pub good_len: u64,
    /// True when a torn final record was found past `good_len`.
    pub torn_tail: bool,
}

fn corrupt(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Decode every frame in `buf` (see the module docs for the torn-tail rule).
pub fn scan(buf: &[u8]) -> io::Result<Scan> {
    let mut frames = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let rest = &buf[pos..];
        if rest.len() < HEADER {
            return Ok(torn(frames, pos));
        }
        let len = u32::from_le_bytes(rest[0..4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(rest[4..8].try_into().unwrap());
        let Some(end) = HEADER.checked_add(len).filter(|&e| e <= rest.len()) else {
            return Ok(torn(frames, pos));
        };
        let payload = &rest[HEADER..end];
        if crc32(payload) != crc {
            if pos + end == buf.len() {
                return Ok(torn(frames, pos));
            }
            return Err(corrupt(format!(
                "checksum mismatch in record at byte {pos} (not the final record)"
            )));
        }
        frames.push(Frame {
            offset: pos as u64,
            payload: payload.to_vec(),
        });
        pos += end;
    }
    Ok(Scan {
        frames,
        good_len: pos as u64,
        torn_tail: false,
    })
}

fn torn(frames: Vec<Frame>, pos: usize) -> Scan {
    Scan {
        frames,
        good_len: pos as u64,
        torn_tail: true,
    }
}

/// fsync a directory so a rename/create inside it is durable.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Atomically replace `path` with a single framed record: write a temp file,
/// fsync it, rename it over `path`, fsync the directory.
pub fn atomic_write(path: &Path, payload: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = tmp_path(path);
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&encode_frame(payload)?)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_dir(dir)
}

/// Read a file written by [`atomic_write`]. `Ok(None)` if it does not exist;
/// any damage is an error (an atomic replace is never torn).
pub fn read_atomic(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let buf = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let s = scan(&buf)?;
    match (s.torn_tail, s.frames.len()) {
        (false, 1) => Ok(s.frames.into_iter().next().map(|f| f.payload)),
        _ => Err(corrupt(format!("{} is damaged", path.display()))),
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// An append-only framed log file.
pub struct FramedLog {
    path: PathBuf,
    file: File,
    len: u64,
}

impl FramedLog {
    /// Open (creating if absent) and recover: returns the log positioned
    /// after the last good frame plus every good frame. A torn tail is
    /// truncated away (durably) before returning.
    pub fn open(path: &Path) -> io::Result<(FramedLog, Vec<Frame>)> {
        let buf = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let s = scan(&buf)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        if s.torn_tail {
            file.set_len(s.good_len)?;
            file.sync_all()?;
        }
        if let Some(dir) = path.parent() {
            sync_dir(dir)?;
        }
        let log = FramedLog {
            path: path.to_path_buf(),
            file,
            len: s.good_len,
        };
        Ok((log, s.frames))
    }

    /// Current byte length (the offset the next append starts at).
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Append records durably; returns each record's starting offset.
    pub fn append<'a>(
        &mut self,
        payloads: impl IntoIterator<Item = &'a [u8]>,
    ) -> io::Result<Vec<u64>> {
        let mut buf = Vec::new();
        let mut offsets = Vec::new();
        for p in payloads {
            offsets.push(self.len + buf.len() as u64);
            buf.extend_from_slice(&encode_frame(p)?);
        }
        if buf.is_empty() {
            return Ok(offsets);
        }
        use std::io::{Seek, SeekFrom};
        self.file.seek(SeekFrom::Start(self.len))?;
        self.file.write_all(&buf)?;
        self.file.sync_data()?;
        self.len += buf.len() as u64;
        Ok(offsets)
    }

    /// Durably cut the log at `offset` (drop that record and all after it).
    pub fn truncate(&mut self, offset: u64) -> io::Result<()> {
        if offset >= self.len {
            return Ok(());
        }
        self.file.set_len(offset)?;
        self.file.sync_all()?;
        self.len = offset;
        Ok(())
    }

    /// Atomically replace the whole log with `payloads` (compaction after a
    /// purge); returns the new offsets.
    pub fn rewrite<'a>(
        &mut self,
        payloads: impl IntoIterator<Item = &'a [u8]>,
    ) -> io::Result<Vec<u64>> {
        let tmp = tmp_path(&self.path);
        let mut buf = Vec::new();
        let mut offsets = Vec::new();
        for p in payloads {
            offsets.push(buf.len() as u64);
            buf.extend_from_slice(&encode_frame(p)?);
        }
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&buf)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;
        if let Some(dir) = self.path.parent() {
            sync_dir(dir)?;
        }
        self.file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        self.len = buf.len() as u64;
        Ok(offsets)
    }
}

/// Map any (de)serialization or I/O failure to `io::Error`.
pub fn data_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

#[cfg(test)]
pub(crate) mod testdir {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A fresh, unique directory under the system temp dir, removed on drop.
    pub struct TestDir(pub PathBuf);

    impl TestDir {
        pub fn new(tag: &str) -> TestDir {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "gnomon-consensus-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TestDir(p)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testdir::TestDir;
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn append_reopen_returns_every_record_in_order() {
        let d = TestDir::new("wal-roundtrip");
        let p = d.0.join("log");
        let (mut log, frames) = FramedLog::open(&p).unwrap();
        assert!(frames.is_empty());
        let offs = log.append([&b"one"[..], b"two"]).unwrap();
        log.append([&b"three"[..]]).unwrap();
        drop(log);
        let (log, frames) = FramedLog::open(&p).unwrap();
        let got: Vec<&[u8]> = frames.iter().map(|f| f.payload.as_slice()).collect();
        assert_eq!(got, vec![&b"one"[..], b"two", b"three"]);
        assert_eq!(frames[1].offset, offs[1]);
        assert_eq!(log.len(), fs::metadata(&p).unwrap().len());
    }

    #[test]
    fn torn_tail_is_dropped_and_truncated() {
        let d = TestDir::new("wal-torn");
        let p = d.0.join("log");
        let (mut log, _) = FramedLog::open(&p).unwrap();
        log.append([&b"good-1"[..], b"good-2"]).unwrap();
        let good_len = log.len();
        drop(log);
        // Simulate a crash mid-append: a header promising more bytes than exist.
        let mut torn = fs::read(&p).unwrap();
        torn.extend_from_slice(&encode_frame(b"half-written").unwrap()[..12]);
        fs::write(&p, &torn).unwrap();

        let (mut log, frames) = FramedLog::open(&p).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(fs::metadata(&p).unwrap().len(), good_len);
        log.append([&b"after"[..]]).unwrap();
        drop(log);
        let (_, frames) = FramedLog::open(&p).unwrap();
        assert_eq!(frames.last().unwrap().payload, b"after");
    }

    #[test]
    fn checksum_failure_on_final_record_is_a_torn_tail() {
        let mut buf = encode_frame(b"a").unwrap();
        let mut last = encode_frame(b"bbbb").unwrap();
        *last.last_mut().unwrap() ^= 0xFF;
        buf.extend_from_slice(&last);
        let s = scan(&buf).unwrap();
        assert!(s.torn_tail);
        assert_eq!(s.frames.len(), 1);
        assert_eq!(s.good_len, 9);
    }

    #[test]
    fn corrupted_middle_record_is_an_error() {
        let d = TestDir::new("wal-corrupt");
        let p = d.0.join("log");
        let (mut log, _) = FramedLog::open(&p).unwrap();
        log.append([&b"first"[..], b"second", b"third"]).unwrap();
        drop(log);
        let mut buf = fs::read(&p).unwrap();
        buf[HEADER + 1] ^= 0xFF; // flip a payload byte of the first record
        fs::write(&p, &buf).unwrap();
        let err = FramedLog::open(&p)
            .err()
            .expect("mid-file corruption must fail");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn truncate_and_rewrite_survive_reopen() {
        let d = TestDir::new("wal-trunc");
        let p = d.0.join("log");
        let (mut log, _) = FramedLog::open(&p).unwrap();
        let offs = log.append([&b"a"[..], b"b", b"c"]).unwrap();
        log.truncate(offs[1]).unwrap();
        drop(log);
        let (mut log, frames) = FramedLog::open(&p).unwrap();
        assert_eq!(frames.len(), 1);
        log.rewrite([&b"x"[..], b"y"]).unwrap();
        log.append([&b"z"[..]]).unwrap();
        drop(log);
        let (_, frames) = FramedLog::open(&p).unwrap();
        let got: Vec<&[u8]> = frames.iter().map(|f| f.payload.as_slice()).collect();
        assert_eq!(got, vec![&b"x"[..], b"y", b"z"]);
    }

    #[test]
    fn atomic_write_replaces_and_reads_back() {
        let d = TestDir::new("wal-atomic");
        let p = d.0.join("vote");
        assert_eq!(read_atomic(&p).unwrap(), None);
        atomic_write(&p, b"v1").unwrap();
        atomic_write(&p, b"v2").unwrap();
        assert_eq!(read_atomic(&p).unwrap().as_deref(), Some(&b"v2"[..]));
        assert!(!d.0.join("vote.tmp").exists());
        fs::write(&p, b"garbage!!").unwrap();
        assert!(read_atomic(&p).is_err());
    }
}
