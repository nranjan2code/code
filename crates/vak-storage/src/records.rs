//! Append-only record segments of per-entry frames.
//!
//! Frame on disk: `len u32 LE | flags u8 | payload | hash[32]` where `len`
//! counts flags + payload, `payload` is zstd of the entry (sealed under the
//! scope key when flags bit 0 is set, with the entry's sequence number as
//! AAD), and `hash = SHA-256(prev_hash || len || flags || payload)`. The
//! chain is over the bytes as stored, so it verifies with no key and keeps
//! verifying after the key is destroyed.

use crate::seal::{self, KEY_LEN};
use crate::{Result, StorageError};
use ring::digest::{Context, SHA256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

pub const GENESIS: [u8; 32] = [0; 32];
const SEALED: u8 = 1;
const MAX_FRAME: usize = 64 * 1024 * 1024;
const HASH: usize = 32;

#[derive(Clone)]
pub struct ScopeKey(pub [u8; KEY_LEN]);

#[derive(Debug, PartialEq, Eq)]
pub struct ChainReport {
    pub entries: u64,
    pub head: [u8; 32],
    /// Bytes covered by complete, verified frames.
    pub valid_len: u64,
    /// Where an incomplete trailing frame begins, if the file has one.
    pub torn_tail: Option<u64>,
    /// Frames sealed under a scope key. Zero means the segment is plaintext
    /// frames and may be compressed whole when sealed.
    pub encrypted_frames: u64,
}

fn link(prev: &[u8; 32], body: &[u8]) -> [u8; 32] {
    let mut c = Context::new(&SHA256);
    c.update(prev);
    c.update(&(body.len() as u32).to_le_bytes());
    c.update(body);
    let mut out = [0u8; 32];
    out.copy_from_slice(c.finish().as_ref());
    out
}

struct Frame<'a> {
    flags: u8,
    payload: &'a [u8],
}

/// Walks frames, verifying the chain. Calls `visit` for each frame.
fn walk(
    data: &[u8],
    start: [u8; 32],
    mut visit: impl FnMut(u64, Frame<'_>) -> Result<()>,
) -> Result<ChainReport> {
    let mut pos = 0usize;
    let mut prev = start;
    let mut n = 0u64;
    let mut encrypted = 0u64;
    loop {
        let rest = &data[pos..];
        if rest.is_empty() {
            return Ok(ChainReport {
                entries: n,
                head: prev,
                valid_len: pos as u64,
                torn_tail: None,
                encrypted_frames: encrypted,
            });
        }
        let torn = ChainReport {
            entries: n,
            head: prev,
            valid_len: pos as u64,
            torn_tail: Some(pos as u64),
            encrypted_frames: encrypted,
        };
        if rest.len() < 4 {
            return Ok(torn);
        }
        let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if len == 0 || len > MAX_FRAME {
            return Err(StorageError::ChainBroken(n));
        }
        if rest.len() < 4 + len + HASH {
            return Ok(torn);
        }
        let body = &rest[4..4 + len];
        let stored = &rest[4 + len..4 + len + HASH];
        let h = link(&prev, body);
        if h.as_slice() != stored {
            return Err(StorageError::ChainBroken(n));
        }
        visit(
            n,
            Frame {
                flags: body[0],
                payload: &body[1..],
            },
        )?;
        if body[0] & SEALED != 0 {
            encrypted += 1;
        }
        prev = h;
        n += 1;
        pos += 4 + len + HASH;
    }
}

/// Verifies the whole chain without any key.
pub fn verify_chain(path: &Path) -> Result<ChainReport> {
    verify_chain_from(path, GENESIS)
}

/// Verifies a segment whose first frame chains from `prev` (the head of the
/// segment before it).
pub fn verify_chain_from(path: &Path, prev: [u8; 32]) -> Result<ChainReport> {
    verify_bytes(&fs::read(path)?, prev)
}

/// Verifies frames held in memory. Never panics on any input.
pub fn verify_bytes(data: &[u8], prev: [u8; 32]) -> Result<ChainReport> {
    walk(data, prev, |_, _| Ok(()))
}

fn aad(seq: u64) -> [u8; 8] {
    seq.to_le_bytes()
}

/// Reads every entry. A sealed frame needs `key`; without it, or after the
/// key is destroyed, it is `Undecryptable` while the chain still verifies.
pub fn read_entries(path: &Path, key: Option<&ScopeKey>) -> Result<Vec<Vec<u8>>> {
    entries_from_bytes(&fs::read(path)?, GENESIS, key)
}

/// As `read_entries`, over frames in memory that chain from `prev`.
pub fn entries_from_bytes(
    data: &[u8],
    prev: [u8; 32],
    key: Option<&ScopeKey>,
) -> Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    walk(data, prev, |seq, f| {
        let compressed = if f.flags & SEALED != 0 {
            let k = key.ok_or(StorageError::Undecryptable)?;
            seal::open(&k.0, &aad(seq), f.payload).map_err(|_| StorageError::Undecryptable)?
        } else {
            f.payload.to_vec()
        };
        out.push(seal::decompress(&compressed)?);
        Ok(())
    })?;
    Ok(out)
}

/// Cuts an incomplete trailing frame so the log can be appended to again.
pub fn truncate_torn_tail(path: &Path) -> Result<ChainReport> {
    truncate_torn_tail_from(path, GENESIS)
}

/// As `truncate_torn_tail`, for a segment whose chain starts at `prev`.
pub fn truncate_torn_tail_from(path: &Path, prev: [u8; 32]) -> Result<ChainReport> {
    let r = verify_chain_from(path, prev)?;
    if r.torn_tail.is_some() {
        let f = OpenOptions::new().write(true).open(path)?;
        f.set_len(r.valid_len)?;
        f.sync_all()?;
    }
    Ok(ChainReport {
        torn_tail: None,
        ..r
    })
}

/// One plaintext entry located in a segment's frame bytes: where its frame
/// starts, its length on disk, and the entry itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedEntry {
    pub offset: u64,
    pub len: u64,
    pub entry: Vec<u8>,
}

/// The plaintext entries of `data` from the frame at `from` onward, without
/// verifying the chain: an index that already holds a segment's prefix reads
/// only its suffix, and checks each entry against its own digest. A torn
/// tail ends the list; a sealed frame is `Undecryptable`.
pub fn located_entries(data: &[u8], from: u64) -> Result<Vec<LocatedEntry>> {
    let mut out = Vec::new();
    let mut pos = usize::try_from(from).map_err(|_| StorageError::Malformed("offset"))?;
    while pos < data.len() {
        let rest = &data[pos..];
        if rest.len() < 4 {
            break;
        }
        let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if len == 0 || len > MAX_FRAME {
            return Err(StorageError::Malformed("frame length"));
        }
        let total = 4 + len + HASH;
        if rest.len() < total {
            break;
        }
        out.push(LocatedEntry {
            offset: pos as u64,
            len: total as u64,
            entry: plaintext(&rest[4..4 + len])?,
        });
        pos += total;
    }
    Ok(out)
}

/// The plaintext entry of the one frame at `offset` (`len` bytes on disk).
pub fn entry_at(data: &[u8], offset: u64, len: u64) -> Result<Vec<u8>> {
    let start = usize::try_from(offset).map_err(|_| StorageError::Malformed("offset"))?;
    let total = usize::try_from(len).map_err(|_| StorageError::Malformed("length"))?;
    let frame = data
        .get(start..start.saturating_add(total))
        .filter(|frame| frame.len() == total && total > 4 + HASH)
        .ok_or(StorageError::Malformed("frame out of range"))?;
    let body_len = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if 4 + body_len + HASH != total {
        return Err(StorageError::Malformed("frame length"));
    }
    plaintext(&frame[4..4 + body_len])
}

fn plaintext(body: &[u8]) -> Result<Vec<u8>> {
    if body[0] & SEALED != 0 {
        return Err(StorageError::Undecryptable);
    }
    seal::decompress(&body[1..])
}

pub struct RecordWriter {
    file: File,
    head: [u8; 32],
    seq: u64,
    /// Frames written since the last sync.
    dirty: bool,
}

static SYNCS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Record syncs this process has issued: what a durability budget counts.
pub fn syncs() -> u64 {
    SYNCS.load(std::sync::atomic::Ordering::Relaxed)
}

impl Drop for RecordWriter {
    fn drop(&mut self) {
        let _ = self.sync();
    }
}

impl RecordWriter {
    /// Opens (creating if absent), verifying the existing chain. A torn tail
    /// is an error until `truncate_torn_tail` has been called.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_from(path, GENESIS)
    }

    /// As `open`, for a segment that continues the chain at `prev`.
    pub fn open_from(path: &Path, prev: [u8; 32]) -> Result<Self> {
        let r = if path.exists() {
            verify_chain_from(path, prev)?
        } else {
            ChainReport {
                entries: 0,
                head: prev,
                valid_len: 0,
                torn_tail: None,
                encrypted_frames: 0,
            }
        };
        if let Some(at) = r.torn_tail {
            return Err(StorageError::TornTail(at));
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file,
            head: r.head,
            seq: r.entries,
            dirty: false,
        })
    }

    /// Makes every frame appended so far durable; a no-op when nothing
    /// is pending.
    pub fn sync(&mut self) -> Result<()> {
        if self.dirty {
            self.file.sync_data()?;
            SYNCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.dirty = false;
        }
        Ok(())
    }

    pub fn head(&self) -> [u8; 32] {
        self.head
    }

    pub fn len(&self) -> u64 {
        self.seq
    }

    pub fn is_empty(&self) -> bool {
        self.seq == 0
    }

    /// Compresses, then seals under `key` when given (plaintext frame when
    /// `None`), then appends and syncs.
    pub fn append(&mut self, entry: &[u8], key: Option<&ScopeKey>) -> Result<[u8; 32]> {
        let head = self.append_unsynced(entry, key)?;
        self.sync()?;
        Ok(head)
    }

    /// As `append`, without syncing: the frame is durable at the next
    /// `sync` (group commit). A crash before it loses whole frames from the
    /// tail, which `truncate_torn_tail` repairs; the chain stays valid.
    pub fn append_unsynced(&mut self, entry: &[u8], key: Option<&ScopeKey>) -> Result<[u8; 32]> {
        let compressed = seal::compress(entry)?;
        let (flags, payload) = match key {
            Some(k) => (SEALED, seal::seal(&k.0, &aad(self.seq), &compressed)?),
            None => (0, compressed),
        };
        let mut body = Vec::with_capacity(1 + payload.len());
        body.push(flags);
        body.extend_from_slice(&payload);
        if body.len() > MAX_FRAME {
            return Err(StorageError::Malformed("entry too large"));
        }
        let h = link(&self.head, &body);
        let mut frame = Vec::with_capacity(4 + body.len() + HASH);
        frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
        frame.extend_from_slice(&body);
        frame.extend_from_slice(&h);
        self.file.write_all(&frame)?;
        self.dirty = true;
        self.head = h;
        self.seq += 1;
        Ok(h)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn key() -> ScopeKey {
        ScopeKey([9; KEY_LEN])
    }

    fn log(dir: &Path, entries: &[&[u8]], k: Option<&ScopeKey>) -> std::path::PathBuf {
        let p = dir.join("seg.log");
        let mut w = RecordWriter::open(&p).unwrap();
        for e in entries {
            w.append(e, k).unwrap();
        }
        p
    }

    #[test]
    fn chain_verifies_without_keys() {
        let d = tempfile::tempdir().unwrap();
        let p = log(d.path(), &[b"one", b"two", b"three"], Some(&key()));
        let r = verify_chain(&p).unwrap();
        assert_eq!((r.entries, r.torn_tail), (3, None));
        assert_eq!(
            read_entries(&p, Some(&key())).unwrap(),
            vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
        );
        let mut w = RecordWriter::open(&p).unwrap();
        assert_eq!(w.head(), r.head);
        w.append(b"four", None).unwrap();
        assert_eq!(verify_chain(&p).unwrap().entries, 4);
    }

    #[test]
    fn chain_verifies_after_shred() {
        let d = tempfile::tempdir().unwrap();
        let p = log(d.path(), &[b"alpha", b"beta"], Some(&key()));
        let before = fs::read(&p).unwrap();
        assert!(matches!(
            read_entries(&p, None),
            Err(StorageError::Undecryptable)
        ));
        assert!(matches!(
            read_entries(&p, Some(&ScopeKey([1; KEY_LEN]))),
            Err(StorageError::Undecryptable)
        ));
        assert_eq!(verify_chain(&p).unwrap().entries, 2);
        assert_eq!(fs::read(&p).unwrap(), before);
        assert!(!before.windows(5).any(|w| w == b"alpha"));
    }

    #[test]
    fn compressed_before_encrypted() {
        let d = tempfile::tempdir().unwrap();
        let big = vec![b'a'; 100_000];
        let p = log(d.path(), &[&big], Some(&key()));
        let size = fs::metadata(&p).unwrap().len();
        assert!(size < 1_000, "stored {size} bytes for 100000 repetitive");
        assert_eq!(read_entries(&p, Some(&key())).unwrap()[0], big);
    }

    #[test]
    fn tamper_is_detected_and_torn_tail_recovers() {
        let d = tempfile::tempdir().unwrap();
        let p = log(d.path(), &[b"x", b"y"], None);
        let good = fs::read(&p).unwrap();

        let mut bad = good.clone();
        bad[6] ^= 1;
        fs::write(&p, &bad).unwrap();
        assert!(matches!(
            verify_chain(&p),
            Err(StorageError::ChainBroken(0))
        ));

        for cut in 1..good.len() {
            fs::write(&p, &good[..cut]).unwrap();
            let r = verify_chain(&p).unwrap();
            assert!(r.entries < 2);
            if r.valid_len != cut as u64 {
                assert_eq!(r.torn_tail, Some(r.valid_len));
                assert!(matches!(
                    RecordWriter::open(&p),
                    Err(StorageError::TornTail(_))
                ));
                let t = truncate_torn_tail(&p).unwrap();
                assert_eq!(t.torn_tail, None);
                let mut w = RecordWriter::open(&p).unwrap();
                w.append(b"z", None).unwrap();
                assert_eq!(read_entries(&p, None).unwrap().last().unwrap(), b"z");
            }
        }
    }

    #[test]
    fn len_tracks_entries() {
        let d = tempfile::tempdir().unwrap();
        let p = log(d.path(), &[b"first", b"second"], Some(&key()));
        let w = RecordWriter::open(&p).unwrap();
        assert_eq!(w.len(), 2);
        assert!(!w.is_empty());
    }
}
