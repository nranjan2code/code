//! Numbered segments of one record chain, with verified sealing.
//!
//! `seg-<n>.log` is the active form. Sealing writes `seg-<n>.sealed`: a magic,
//! a compression flag, then the frames (zstd of the whole file only when every
//! frame is plaintext; encrypted frames are already incompressible). Steps:
//! copy, verify count/hashes/chain continuity, atomic swap into place, then a
//! seal entry in `seals.log`, then the original is removed. A crash between
//! any two steps leaves a readable prefix: the `.log` wins until the seal
//! entry exists, and `recover` finishes or discards the interrupted seal.
//! The next segment chains from the sealed segment's head.

use crate::records::{self, GENESIS, RecordWriter, ScopeKey};
use crate::seal;
use crate::{Result, StorageError};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"VAKSEG01";

/// Held while this process is the one writer of a segment set. Released by
/// an explicit unlock when dropped: closing alone would leave the lock held
/// by any child spawned meanwhile, which holds a duplicate of the
/// descriptor until it execs.
#[derive(Debug)]
pub struct WriterLock {
    file: File,
    dir: PathBuf,
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealStep {
    Copied,
    Verified,
    Swapped,
    Recorded,
}

impl SealStep {
    fn name(self) -> &'static str {
        match self {
            Self::Copied => "copied",
            Self::Verified => "verified",
            Self::Swapped => "swapped",
            Self::Recorded => "recorded",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealEntry {
    pub segment: u64,
    pub entries: u64,
    pub head: [u8; 32],
    pub compressed: bool,
}

impl SealEntry {
    fn encode(&self) -> Vec<u8> {
        format!(
            "seal {} {} {} {}",
            self.segment,
            self.entries,
            seal::hex(&self.head),
            u8::from(self.compressed)
        )
        .into_bytes()
    }

    fn decode(b: &[u8]) -> Result<Self> {
        let bad = || StorageError::Malformed("seal entry");
        let s = std::str::from_utf8(b).map_err(|_| bad())?;
        let mut it = s.split(' ');
        if it.next() != Some("seal") {
            return Err(bad());
        }
        let segment = it.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
        let entries = it.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
        let hex = it.next().ok_or_else(bad)?;
        let compressed = it.next().ok_or_else(bad)? == "1";
        if hex.len() != 64 || !hex.is_ascii() {
            return Err(bad());
        }
        let mut head = [0u8; 32];
        for (i, slot) in head.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
        }
        Ok(Self {
            segment,
            entries,
            head,
            compressed,
        })
    }
}

/// Decodes a sealed-segment file to its raw frames. Never panics on any input.
pub fn decode_sealed(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() < MAGIC.len() + 1 || &bytes[..MAGIC.len()] != MAGIC {
        return Err(StorageError::Malformed("sealed segment header"));
    }
    let body = &bytes[MAGIC.len() + 1..];
    match bytes[MAGIC.len()] {
        0 => Ok(body.to_vec()),
        1 => seal::decompress(body),
        _ => Err(StorageError::Malformed("sealed segment flag")),
    }
}

/// The frame bytes of a segment file, open (`.log`) or sealed (`.sealed`,
/// decoded), so a reader addresses frames the same way in both.
pub fn frame_bytes(path: &Path) -> Result<Vec<u8>> {
    let data = fs::read(path)?;
    if path.extension().is_some_and(|ext| ext == "sealed") {
        decode_sealed(&data)
    } else {
        Ok(data)
    }
}

pub struct SegmentSet {
    dir: PathBuf,
}

impl SegmentSet {
    pub fn open(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// Takes the single-writer lock of this segment set: an exclusive
    /// flock on `<dir>/LOCK`, waiting for another holder to release it.
    pub fn lock(&self) -> Result<WriterLock> {
        let file = self.lock_file()?;
        file.lock()?;
        Ok(WriterLock {
            file,
            dir: self.dir.clone(),
        })
    }

    /// Takes the single-writer lock if no one else holds it; `None` if
    /// another writer (in this or another process) does.
    pub fn try_lock(&self) -> Result<Option<WriterLock>> {
        let file = self.lock_file()?;
        match file.try_lock() {
            Ok(()) => Ok(Some(WriterLock {
                file,
                dir: self.dir.clone(),
            })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn lock_file(&self) -> Result<File> {
        Ok(fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join("LOCK"))?)
    }

    pub fn log_path(&self, n: u64) -> PathBuf {
        self.dir.join(format!("seg-{n:08}.log"))
    }

    pub fn sealed_path(&self, n: u64) -> PathBuf {
        self.dir.join(format!("seg-{n:08}.sealed"))
    }

    fn tmp_path(&self, n: u64) -> PathBuf {
        self.dir.join(format!("seg-{n:08}.sealing"))
    }

    fn seals_path(&self) -> PathBuf {
        self.dir.join("seals.log")
    }

    pub fn seals(&self) -> Result<Vec<SealEntry>> {
        let p = self.seals_path();
        if !p.exists() {
            return Ok(Vec::new());
        }
        let r = records::verify_chain(&p)?;
        if r.torn_tail.is_some() {
            records::truncate_torn_tail(&p)?;
        }
        records::read_entries(&p, None)?
            .iter()
            .map(|e| SealEntry::decode(e))
            .collect()
    }

    /// Where segment `n`'s chain starts: the sealed head of `n - 1`.
    pub fn prev_head(&self, n: u64) -> Result<[u8; 32]> {
        if n <= 1 {
            return Ok(GENESIS);
        }
        self.seals()?
            .into_iter()
            .find(|s| s.segment == n - 1)
            .map(|s| s.head)
            .ok_or(StorageError::Malformed("previous segment is not sealed"))
    }

    /// A writer for the active segment `n` (which must chain from sealed `n-1`).
    /// The writer of segment `n`, for the holder of this set's `lock`: a
    /// segment has one writer at a time, across processes.
    pub fn writer(&self, n: u64, lock: &WriterLock) -> Result<RecordWriter> {
        self.held(lock)?;
        if self.sealed_path(n).exists() {
            return Err(StorageError::Malformed("segment is sealed"));
        }
        RecordWriter::open_from(&self.log_path(n), self.prev_head(n)?)
    }

    /// Reads segment `n` in whichever form exists. The `.log` wins while it
    /// exists, because it is complete until the seal entry has been written.
    pub fn read(&self, n: u64, key: Option<&ScopeKey>) -> Result<Vec<Vec<u8>>> {
        let prev = self.prev_head(n)?;
        let log = self.log_path(n);
        let data = if log.exists() {
            fs::read(log)?
        } else {
            let data = decode_sealed(&fs::read(self.sealed_path(n))?)?;
            let sealed = self
                .seals()?
                .into_iter()
                .find(|s| s.segment == n)
                .ok_or(StorageError::Malformed("sealed segment has no seal entry"))?;
            let r = records::verify_bytes(&data, prev)?;
            if r.torn_tail.is_some() || r.entries != sealed.entries || r.head != sealed.head {
                return Err(StorageError::Integrity);
            }
            data
        };
        records::entries_from_bytes(&data, prev, key)
    }

    /// Seals segment `n` for the holder of this set's `lock`.
    pub fn seal(&self, n: u64, lock: &WriterLock) -> Result<SealEntry> {
        self.seal_until(n, None, lock)
    }

    /// Removes sealed segment `n`'s file, for the holder of `lock`. Its
    /// seal entry stays, and with it the head the next segment chains
    /// from, so every later segment still verifies: this is how rows past
    /// their retention leave an append-only chain, a whole segment at a
    /// time and never one row. Refused for an open segment and for the
    /// newest sealed one, which numbers the next. `false` when the file
    /// is already gone.
    pub fn drop_sealed(&self, n: u64, lock: &WriterLock) -> Result<bool> {
        self.held(lock)?;
        let seals = self.seals()?;
        if !seals.iter().any(|s| s.segment == n) || self.log_path(n).exists() {
            return Err(StorageError::Malformed("segment is not sealed"));
        }
        if seals.iter().map(|s| s.segment).max() == Some(n) {
            return Err(StorageError::Malformed("the newest sealed segment stays"));
        }
        match fs::remove_file(self.sealed_path(n)) {
            Ok(()) => {
                if let Ok(dir) = fs::File::open(&self.dir) {
                    let _ = dir.sync_all();
                }
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn held(&self, lock: &WriterLock) -> Result<()> {
        if lock.dir == self.dir {
            Ok(())
        } else {
            Err(StorageError::Malformed(
                "writer lock of another segment set",
            ))
        }
    }

    /// Seals segment `n`, optionally stopping after `stop` to simulate a crash.
    pub fn seal_until(
        &self,
        n: u64,
        stop: Option<SealStep>,
        lock: &WriterLock,
    ) -> Result<SealEntry> {
        self.held(lock)?;
        let halt = |s: SealStep| -> Result<()> {
            if stop == Some(s) {
                Err(StorageError::Interrupted(s.name()))
            } else {
                Ok(())
            }
        };
        let prev = self.prev_head(n)?;
        let raw = fs::read(self.log_path(n))?;
        let report = records::verify_bytes(&raw, prev)?;
        if let Some(at) = report.torn_tail {
            return Err(StorageError::TornTail(at));
        }
        let compressed = report.encrypted_frames == 0;
        let mut file = Vec::with_capacity(raw.len() + 9);
        file.extend_from_slice(MAGIC);
        file.push(u8::from(compressed));
        if compressed {
            file.extend_from_slice(&seal::compress(&raw)?);
        } else {
            file.extend_from_slice(&raw);
        }
        let tmp = self.tmp_path(n);
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&file)?;
            f.sync_all()?;
        }
        halt(SealStep::Copied)?;

        let copy = records::verify_bytes(&decode_sealed(&fs::read(&tmp)?)?, prev)?;
        if copy.entries != report.entries || copy.head != report.head || copy.torn_tail.is_some() {
            let _ = fs::remove_file(&tmp);
            return Err(StorageError::Integrity);
        }
        halt(SealStep::Verified)?;

        fs::rename(&tmp, self.sealed_path(n))?;
        sync_dir(&self.dir);
        halt(SealStep::Swapped)?;

        let entry = SealEntry {
            segment: n,
            entries: report.entries,
            head: report.head,
            compressed,
        };
        self.record_seal(&entry)?;
        halt(SealStep::Recorded)?;

        fs::remove_file(self.log_path(n))?;
        sync_dir(&self.dir);
        Ok(entry)
    }

    fn record_seal(&self, entry: &SealEntry) -> Result<()> {
        if self.seals()?.iter().any(|s| s.segment == entry.segment) {
            return Ok(());
        }
        let mut w = RecordWriter::open(&self.seals_path())?;
        w.append(&entry.encode(), None)?;
        Ok(())
    }

    /// Finishes or discards any seal a crash interrupted. Idempotent.
    /// Finishes or discards an interrupted seal, for the holder of `lock`.
    pub fn recover(&self, lock: &WriterLock) -> Result<()> {
        self.held(lock)?;
        let mut nums = Vec::new();
        for e in fs::read_dir(&self.dir)? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if let Some(rest) = name.strip_prefix("seg-")
                && let Some((num, ext)) = rest.split_once('.')
                && let Ok(n) = num.parse::<u64>()
            {
                nums.push((n, ext.to_string()));
            }
        }
        nums.sort();
        for (n, ext) in nums {
            match ext.as_str() {
                "sealing" => {
                    fs::remove_file(self.tmp_path(n))?;
                }
                "sealed" if self.log_path(n).exists() => {
                    let prev = self.prev_head(n)?;
                    let log = fs::read(self.log_path(n))?;
                    let want = records::verify_bytes(&log, prev)?;
                    let got = decode_sealed(&fs::read(self.sealed_path(n))?)
                        .and_then(|d| records::verify_bytes(&d, prev));
                    match got {
                        Ok(g) if g.head == want.head && g.entries == want.entries => {
                            self.record_seal(&SealEntry {
                                segment: n,
                                entries: want.entries,
                                head: want.head,
                                compressed: want.encrypted_frames == 0,
                            })?;
                            fs::remove_file(self.log_path(n))?;
                        }
                        _ => fs::remove_file(self.sealed_path(n))?,
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::seal::KEY_LEN;

    fn fill(set: &SegmentSet, n: u64, items: &[&[u8]], k: Option<&ScopeKey>) {
        let lock = set.lock().unwrap();
        let mut w = set.writer(n, &lock).unwrap();
        for i in items {
            w.append(i, k).unwrap();
        }
    }

    #[test]
    fn a_segment_set_has_one_writer_at_a_time() {
        let d = tempfile::tempdir().unwrap();
        let set = SegmentSet::open(d.path()).unwrap();
        let held = set
            .try_lock()
            .unwrap()
            .expect("the first writer takes the lock");
        assert!(
            set.try_lock().unwrap().is_none(),
            "a second writer is refused"
        );
        let other = tempfile::tempdir().unwrap();
        let other_lock = SegmentSet::open(other.path()).unwrap().lock().unwrap();
        assert!(
            set.writer(1, &other_lock).is_err(),
            "another set's lock opens nothing here"
        );
        // A child being spawned holds a duplicate of every open descriptor
        // until it execs; the duplicate here stands in for one.
        let duplicate = held.file.try_clone().unwrap();
        drop(held);
        assert!(
            set.try_lock().unwrap().is_some(),
            "released when the holder drops it, whatever duplicates exist"
        );
        drop(duplicate);
    }

    #[test]
    fn seal_round_trip_plain_is_compressed_and_chains_on() {
        let d = tempfile::tempdir().unwrap();
        let set = SegmentSet::open(d.path()).unwrap();
        let big = vec![b'z'; 50_000];
        fill(&set, 1, &[&big, b"two"], None);
        let before = set.read(1, None).unwrap();
        let e = set.seal(1, &set.lock().unwrap()).unwrap();
        assert!(e.compressed);
        assert!(!set.log_path(1).exists());
        assert!(fs::metadata(set.sealed_path(1)).unwrap().len() < 1_000);
        assert_eq!(set.read(1, None).unwrap(), before);
        fill(&set, 2, &[b"three"], None);
        assert_eq!(set.read(2, None).unwrap(), vec![b"three".to_vec()]);
        assert_eq!(set.prev_head(2).unwrap(), e.head);
        assert!(set.writer(1, &set.lock().unwrap()).is_err());
    }

    #[test]
    fn encrypted_frames_are_copied_not_compressed() {
        let d = tempfile::tempdir().unwrap();
        let set = SegmentSet::open(d.path()).unwrap();
        let k = ScopeKey([4; KEY_LEN]);
        fill(&set, 1, &[b"secret one", b"secret two"], Some(&k));
        let e = set.seal(1, &set.lock().unwrap()).unwrap();
        assert!(!e.compressed);
        assert_eq!(set.read(1, Some(&k)).unwrap().len(), 2);
        assert!(matches!(
            set.read(1, None),
            Err(StorageError::Undecryptable)
        ));
    }

    #[test]
    fn tamper_with_sealed_segment_is_detected() {
        let d = tempfile::tempdir().unwrap();
        let set = SegmentSet::open(d.path()).unwrap();
        let k = ScopeKey([4; KEY_LEN]);
        fill(&set, 1, &[b"a", b"b"], Some(&k));
        set.seal(1, &set.lock().unwrap()).unwrap();
        let p = set.sealed_path(1);
        let mut b = fs::read(&p).unwrap();
        let n = b.len() - 40;
        b[n] ^= 1;
        fs::write(&p, &b).unwrap();
        assert!(set.read(1, Some(&k)).is_err());
        b.truncate(20);
        fs::write(&p, b).unwrap();
        assert!(set.read(1, Some(&k)).is_err());
    }

    #[test]
    fn crash_at_every_step_leaves_a_readable_prefix() {
        for plain in [true, false] {
            for stop in [
                SealStep::Copied,
                SealStep::Verified,
                SealStep::Swapped,
                SealStep::Recorded,
            ] {
                let d = tempfile::tempdir().unwrap();
                let set = SegmentSet::open(d.path()).unwrap();
                let k = ScopeKey([8; KEY_LEN]);
                let key = if plain { None } else { Some(&k) };
                fill(&set, 1, &[b"a", b"b", b"c"], key);
                let want = set.read(1, key).unwrap();
                assert!(matches!(
                    set.seal_until(1, Some(stop), &set.lock().unwrap()),
                    Err(StorageError::Interrupted(_))
                ));
                assert_eq!(set.read(1, key).unwrap(), want, "readable at {stop:?}");
                set.recover(&set.lock().unwrap()).unwrap();
                assert_eq!(set.read(1, key).unwrap(), want, "recovered at {stop:?}");
                if matches!(stop, SealStep::Copied | SealStep::Verified) {
                    assert!(set.log_path(1).exists());
                    set.seal(1, &set.lock().unwrap()).unwrap();
                }
                assert_eq!(set.seals().unwrap().len(), 1);
                assert_eq!(set.read(1, key).unwrap(), want);
                assert!(!set.log_path(1).exists());
            }
        }
    }

    #[test]
    fn crash_with_a_partial_copy_on_disk_is_discarded() {
        let d = tempfile::tempdir().unwrap();
        let set = SegmentSet::open(d.path()).unwrap();
        fill(&set, 1, &[b"a", b"b"], None);
        let want = set.read(1, None).unwrap();
        fs::write(set.tmp_path(1), b"VAKSEG01\x01garbage").unwrap();
        assert_eq!(set.read(1, None).unwrap(), want);
        set.recover(&set.lock().unwrap()).unwrap();
        assert!(!set.tmp_path(1).exists());
        set.seal(1, &set.lock().unwrap()).unwrap();
        assert_eq!(set.read(1, None).unwrap(), want);
    }

    #[test]
    fn a_corrupt_swapped_copy_is_discarded_by_recovery() {
        let d = tempfile::tempdir().unwrap();
        let set = SegmentSet::open(d.path()).unwrap();
        fill(&set, 1, &[b"a", b"b"], None);
        let want = set.read(1, None).unwrap();
        let _ = set.seal_until(1, Some(SealStep::Swapped), &set.lock().unwrap());
        fs::write(set.sealed_path(1), b"VAKSEG01\x00junk").unwrap();
        set.recover(&set.lock().unwrap()).unwrap();
        assert!(!set.sealed_path(1).exists());
        assert_eq!(set.read(1, None).unwrap(), want);
    }

    #[test]
    fn a_torn_segment_is_not_sealed() {
        let d = tempfile::tempdir().unwrap();
        let set = SegmentSet::open(d.path()).unwrap();
        fill(&set, 1, &[b"a", b"b"], None);
        let p = set.log_path(1);
        let mut b = fs::read(&p).unwrap();
        b.truncate(b.len() - 3);
        fs::write(&p, b).unwrap();
        assert!(matches!(
            set.seal(1, &set.lock().unwrap()),
            Err(StorageError::TornTail(_))
        ));
        assert!(set.log_path(1).exists());
    }

    #[test]
    fn a_segment_from_another_chain_does_not_continue_this_one() {
        let d = tempfile::tempdir().unwrap();
        let set = SegmentSet::open(d.path()).unwrap();
        fill(&set, 1, &[b"a"], None);
        set.seal(1, &set.lock().unwrap()).unwrap();
        fill(&set, 2, &[b"b"], None);
        let other = tempfile::tempdir().unwrap();
        let o = SegmentSet::open(other.path()).unwrap();
        fill(&o, 1, &[b"x"], None);
        o.seal(1, &o.lock().unwrap()).unwrap();
        fill(&o, 2, &[b"b"], None);
        fs::copy(o.log_path(2), set.log_path(2)).unwrap();
        assert!(matches!(
            set.read(2, None),
            Err(StorageError::ChainBroken(0))
        ));
    }
}
