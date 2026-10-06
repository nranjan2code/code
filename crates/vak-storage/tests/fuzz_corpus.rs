//! The stable fuzz gate for the frame and segment readers (plan M2): the
//! same entry points the libFuzzer targets in `fuzz/` drive, run over the
//! seed corpus in `fuzz/corpus/<target>/` and a fixed number of
//! deterministic mutations of it. A reader must answer every input with a
//! value, never a panic, and the torn-write property holds byte by byte.
//!
//! `VAK_UPDATE_CORPUS=1 cargo test -p vak-storage --test fuzz_corpus`
//! rewrites the seed corpus from the readers' real output.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use vak_storage::records::{
    self, GENESIS, RecordWriter, ScopeKey, entries_from_bytes, verify_bytes,
};
use vak_storage::segments::{SegmentSet, decode_sealed};

const MUTATIONS: usize = 20_000;

fn corpus_dir(target: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus")
        .join(target)
}

/// A chain of `items`, plain or sealed under `key`, as its file holds it.
fn chain_bytes(items: &[&[u8]], key: Option<&ScopeKey>) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chain.log");
    let mut writer = RecordWriter::open(&path).unwrap();
    for item in items {
        writer.append(item, key).unwrap();
    }
    writer.sync().unwrap();
    drop(writer);
    std::fs::read(&path).unwrap()
}

/// A sealed segment of `items`, as its file holds it.
fn sealed_bytes(items: &[&[u8]]) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let set = SegmentSet::open(dir.path()).unwrap();
    let lock = set.lock().unwrap();
    let mut writer = set.writer(1, &lock).unwrap();
    for item in items {
        writer.append(item, None).unwrap();
    }
    writer.sync().unwrap();
    drop(writer);
    set.seal(1, &lock).unwrap();
    std::fs::read(set.sealed_path(1)).unwrap()
}

fn seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let key = ScopeKey([7; 32]);
    let big = vec![b'x'; 4096];
    vec![
        ("frames", "empty", Vec::new()),
        (
            "frames",
            "plain",
            chain_bytes(&[b"one", b"two", b"three"], None),
        ),
        (
            "frames",
            "sealed-key",
            chain_bytes(&[b"secret", &big], Some(&key)),
        ),
        (
            "segments",
            "sealed-plain",
            sealed_bytes(&[b"one", &big, b"three"]),
        ),
        ("segments", "sealed-small", sealed_bytes(&[b"x"])),
    ]
}

/// The corpus for `target`: the committed files, or the seeds when none is
/// committed yet.
fn corpus(target: &str) -> Vec<Vec<u8>> {
    let dir = corpus_dir(target);
    let mut found: Vec<Vec<u8>> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|entry| std::fs::read(entry.ok()?.path()).ok())
                .collect()
        })
        .unwrap_or_default();
    if found.is_empty() {
        found = seeds()
            .into_iter()
            .filter(|(t, _, _)| *t == target)
            .map(|(_, _, bytes)| bytes)
            .collect();
    }
    found
}

/// The `frames` target: what `fuzz/fuzz_targets/frames.rs` runs.
fn frames(data: &[u8]) {
    let _ = verify_bytes(data, GENESIS);
    let _ = entries_from_bytes(data, GENESIS, None);
    let _ = entries_from_bytes(data, [7; 32], Some(&ScopeKey([1; 32])));
}

/// The `segments` target: what `fuzz/fuzz_targets/segments.rs` runs.
fn segments(data: &[u8]) {
    if let Ok(frames) = decode_sealed(data) {
        let _ = verify_bytes(&frames, GENESIS);
    }
}

/// xorshift64*: deterministic, so a failure reproduces from its index.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn mutate(rng: &mut Rng, input: &[u8], other: &[u8]) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..=rng.below(4) {
        match rng.below(5) {
            0 if !out.is_empty() => {
                let at = rng.below(out.len());
                out[at] ^= 1 << rng.below(8);
            }
            1 => out.truncate(rng.below(out.len() + 1)),
            2 => {
                let at = rng.below(out.len() + 1);
                out.insert(at, rng.next() as u8);
            }
            3 if !other.is_empty() => {
                let from = rng.below(other.len());
                let to = from + rng.below(other.len() - from + 1);
                let at = rng.below(out.len() + 1);
                out.splice(at..at, other[from..to].iter().copied());
            }
            _ if !out.is_empty() => {
                let at = rng.below(out.len());
                out[at] = [0, 0xff, 0x7f, 0x80][rng.below(4)];
            }
            _ => {}
        }
    }
    out
}

fn run(target: &str, reader: fn(&[u8])) {
    let corpus = corpus(target);
    assert!(!corpus.is_empty(), "{target} has a corpus");
    for input in &corpus {
        reader(input);
    }
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for index in 0..MUTATIONS {
        let input = &corpus[rng.below(corpus.len())];
        let other = &corpus[rng.below(corpus.len())];
        let mutated = mutate(&mut rng, input, other);
        let outcome = std::panic::catch_unwind(|| reader(&mutated));
        assert!(outcome.is_ok(), "{target} mutation {index} panicked");
    }
}

#[test]
fn frame_reader_never_panics_on_corpus_or_mutations() {
    run("frames", frames);
}

#[test]
fn segment_reader_never_panics_on_corpus_or_mutations() {
    run("segments", segments);
}

/// A crash can stop a write at any byte. Every prefix of a chain reads as
/// the whole entries it holds, with the rest reported torn, never as an
/// error or a different entry.
#[test]
fn a_write_torn_at_any_byte_leaves_the_entries_before_it() {
    let items: [&[u8]; 4] = [b"first", b"second entry", &[9; 300], b"last"];
    let full = chain_bytes(&items, None);
    let mut boundaries = Vec::new();
    for count in 0..=items.len() {
        boundaries.push(chain_bytes(&items[..count], None).len());
    }
    let mut last = 0usize;
    for cut in 0..=full.len() {
        let report = verify_bytes(&full[..cut], GENESIS).expect("a torn prefix still verifies");
        let whole = boundaries.iter().filter(|end| **end <= cut).count() - 1;
        assert_eq!(report.entries as usize, whole, "cut at byte {cut}");
        assert_eq!(
            report.torn_tail.is_some(),
            !boundaries.contains(&cut),
            "cut at byte {cut}"
        );
        let entries = entries_from_bytes(&full[..cut], GENESIS, None).expect("readable prefix");
        assert_eq!(entries.len(), whole);
        assert!(
            whole >= last,
            "entries never go backwards as more bytes land"
        );
        last = whole;
    }
    // And the file form repairs to exactly that prefix.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chain.log");
    let cut = boundaries[2] + 3;
    std::fs::write(&path, &full[..cut]).unwrap();
    let repaired = records::truncate_torn_tail(&path).unwrap();
    assert_eq!(repaired.entries, 2);
    assert_eq!(std::fs::read(&path).unwrap(), full[..boundaries[2]]);
}

/// Rewrites the committed seed corpus when asked.
#[test]
fn seed_corpus_is_committed() {
    if std::env::var_os("VAK_UPDATE_CORPUS").is_some() {
        for (target, name, bytes) in seeds() {
            let dir = corpus_dir(target);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(name), bytes).unwrap();
        }
    }
    for target in ["frames", "segments"] {
        assert!(
            std::fs::read_dir(corpus_dir(target)).is_ok_and(|mut entries| entries.next().is_some()),
            "fuzz/corpus/{target} is committed; regenerate with VAK_UPDATE_CORPUS=1"
        );
    }
}
