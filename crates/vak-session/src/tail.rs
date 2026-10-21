//! Reading a segment directory (a session ledger or a record chain) from a
//! position onward (plan M6): what a derived index uses to catch up without
//! rereading what it already took. A sealed segment holds the same frames as
//! the open one it replaced, so a position stays valid across a seal.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::types::SessionError;

/// A place in a segment directory: the segment, and how many of its frames
/// lie before it. The default is the start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Position {
    pub segment: u64,
    pub frames: u64,
}

fn corrupt(error: impl std::fmt::Display) -> SessionError {
    SessionError::Corrupt {
        line: 0,
        message: error.to_string(),
    }
}

/// A verified, read-only walk for authoritative decisions. Unlike [`tail`],
/// an unreadable segment, incomplete frame or failed fold is an error, never
/// a successfully consumed prefix. Callers must discard state on error.
pub fn try_tail(
    dir: &Path,
    from: Position,
    mut visit: impl FnMut(Position, &[u8]) -> Result<bool, SessionError>,
) -> Result<Position, SessionError> {
    let numbers = crate::log::try_segment_numbers(dir)?;
    let segments = vak_storage::segments::SegmentSet::at(dir);
    let seals = segments.verified_seals().map_err(corrupt)?;
    if numbers.is_empty() {
        return if from == Position::default() && seals.is_empty() {
            Ok(from)
        } else {
            Err(corrupt("the chain no longer holds the reader's position"))
        };
    }
    if numbers
        .last()
        .is_some_and(|(number, _)| *number < from.segment)
    {
        return Err(corrupt("the chain no longer holds the reader's position"));
    }
    for seal in &seals {
        if seal.segment == from.segment && from.frames > seal.entries {
            return Err(corrupt("the chain no longer holds the reader's position"));
        }
        let unread = seal.segment > from.segment
            || (seal.segment == from.segment && from.frames < seal.entries);
        if unread
            && numbers
                .binary_search_by_key(&seal.segment, |(number, _)| *number)
                .is_err()
        {
            return Err(corrupt("an unread segment is missing"));
        }
    }
    let key = crate::keys::of(dir)?;
    let mut at = from;
    for (number, open) in numbers {
        if number < from.segment {
            continue;
        }
        let path = if open {
            segments.log_path(number)
        } else {
            segments.sealed_path(number)
        };
        let previous = if number <= 1 {
            vak_storage::records::GENESIS
        } else {
            seals
                .iter()
                .find(|seal| seal.segment == number - 1)
                .ok_or_else(|| corrupt("the previous segment has no seal"))?
                .head
        };
        let bytes = vak_storage::segments::frame_bytes(&path).map_err(corrupt)?;
        let report = vak_storage::records::verify_bytes(&bytes, previous).map_err(corrupt)?;
        if report.torn_tail.is_some() {
            return Err(corrupt("the chain has an incomplete frame"));
        }
        if !open {
            let seal = seals
                .iter()
                .find(|seal| seal.segment == number)
                .ok_or_else(|| corrupt("the segment has no seal"))?;
            if seal.head != report.head || seal.entries != report.entries {
                return Err(corrupt("the segment does not match its seal"));
            }
        }
        let skip = if number == from.segment {
            from.frames
        } else {
            0
        };
        if skip > report.entries {
            return Err(corrupt("the chain no longer holds the reader's position"));
        }
        // The complete segment hash chain and seal were verified above.
        // Already folded payloads need not be opened again.
        let frames = vak_storage::records::located_entries_after(&bytes, skip, key.as_ref())
            .map_err(corrupt)?;
        at = Position {
            segment: number,
            frames: skip,
        };
        for (index, frame) in frames.iter().enumerate() {
            at.frames = skip + index as u64 + 1;
            if !visit(at, &frame.entry)? {
                return Ok(at);
            }
        }
    }
    Ok(at)
}

/// The latest verified position. Reads only the newest data segment and its
/// seal metadata; it never recovers or truncates files on a reader's behalf.
pub fn try_head(dir: &Path) -> Result<Position, SessionError> {
    let Some(&(last, _)) = crate::log::try_segment_numbers(dir)?.last() else {
        return try_tail(dir, Position::default(), |_, _| Ok(true));
    };
    try_tail(
        dir,
        Position {
            segment: last,
            frames: 0,
        },
        |_, _| Ok(true),
    )
}

/// Visits each frame after `from`, oldest first, with the position just
/// after it, until `visit` returns `false`. Returns the position after the
/// last frame visited. A segment that cannot be read ends the walk there, so
/// the next walk tries it again.
pub fn tail(
    dir: &Path,
    from: Position,
    mut visit: impl FnMut(Position, &[u8]) -> bool,
) -> Position {
    located_tail(dir, from, |at, frame| visit(at, &frame.entry))
}

/// As [`tail`], including canonical frame byte locations for a rebuildable
/// index. Frame offsets remain unchanged when an encrypted segment seals.
pub fn located_tail(
    dir: &Path,
    from: Position,
    mut visit: impl FnMut(Position, &vak_storage::records::LocatedEntry) -> bool,
) -> Position {
    let mut at = from;
    let Ok(segments) = vak_storage::segments::SegmentSet::open(dir) else {
        return at;
    };
    for (number, open) in crate::log::segment_numbers(dir) {
        if number < from.segment {
            continue;
        }
        let path = if open {
            segments.log_path(number)
        } else {
            segments.sealed_path(number)
        };
        let Ok(bytes) = vak_storage::segments::frame_bytes(&path) else {
            return at;
        };
        let Ok(key) = crate::keys::of(dir) else {
            return at;
        };
        let Ok(frames) = vak_storage::records::located_entries(&bytes, 0, key.as_ref()) else {
            return at;
        };
        let skip = if number == from.segment {
            from.frames
        } else {
            0
        };
        if number > at.segment {
            at = Position {
                segment: number,
                frames: 0,
            };
        }
        for (index, frame) in frames.iter().enumerate() {
            let index = index as u64;
            if index < skip {
                continue;
            }
            at = Position {
                segment: number,
                frames: index + 1,
            };
            if !visit(at, frame) {
                return at;
            }
        }
    }
    at
}

/// Incremental catalog ingestion from an indexed byte boundary. Only the
/// unread suffix is opened; a torn append leaves the position unchanged.
pub fn indexed_tail(
    dir: &Path,
    from: Position,
    offset: u64,
    mut visit: impl FnMut(Position, &vak_storage::records::LocatedEntry) -> bool,
) -> Result<Position, SessionError> {
    let segments = vak_storage::segments::SegmentSet::at(dir);
    let key = crate::keys::of(dir)?;
    let numbers = crate::log::try_segment_numbers(dir)?;
    if from != Position::default() && !numbers.iter().any(|(number, _)| *number == from.segment) {
        return Err(corrupt("the chain no longer holds the reader's position"));
    }
    let mut at = from;
    for (number, open) in numbers {
        if number < from.segment {
            continue;
        }
        let path = if open {
            segments.log_path(number)
        } else {
            segments.sealed_path(number)
        };
        let start = if number == from.segment { offset } else { 0 };
        let bytes = vak_storage::segments::frame_suffix(&path, start).map_err(corrupt)?;
        let frames =
            vak_storage::records::located_slice(&bytes, start, key.as_ref()).map_err(corrupt)?;
        let skip = if number == from.segment {
            from.frames
        } else {
            0
        };
        for (index, frame) in frames.iter().enumerate() {
            at = Position {
                segment: number,
                frames: skip + index as u64 + 1,
            };
            if !visit(at, frame) {
                return Ok(at);
            }
        }
    }
    Ok(at)
}

/// The frame just after `at`: what an index that recorded `at` for an entry
/// reads to load it again. `None` when no frame is there.
pub fn read_at(dir: &Path, at: Position) -> Option<Vec<u8>> {
    let mut found = None;
    tail(dir, at, |_, bytes| {
        found = Some(bytes.to_vec());
        false
    });
    found
}

/// The position after the last frame: where a reader that has taken
/// everything stands. Reads only the newest segment.
pub fn head(dir: &Path) -> Position {
    let Some(&(last, _)) = crate::log::segment_numbers(dir).last() else {
        return Position::default();
    };
    tail(
        dir,
        Position {
            segment: last,
            frames: 0,
        },
        |_, _| true,
    )
    .max(Position {
        segment: last,
        frames: 0,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn strict_empty_reads_create_nothing_and_missing_sealed_history_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows");
        assert!(crate::SessionLog::segment_files(&path).is_empty());
        assert!(!path.exists(), "segment metadata reads create nothing");
        assert_eq!(try_head(&path).unwrap(), Position::default());
        assert!(!path.exists());
        let chain = crate::chain::RecordChain::at(&path);
        chain.append(&serde_json::json!({"n": 1})).unwrap();
        let segments = vak_storage::segments::SegmentSet::open(&path).unwrap();
        segments.seal(1, &segments.lock().unwrap()).unwrap();
        std::fs::remove_file(segments.sealed_path(1)).unwrap();
        assert!(try_head(&path).is_err());
        assert!(try_tail(&path, Position::default(), |_, _| Ok(true)).is_err());
    }

    #[test]
    fn a_reader_resumes_where_it_stopped_across_seals() {
        let dir = tempfile::tempdir().unwrap();
        let chain = crate::chain::RecordChain::at(dir.path().join("rows"));
        for n in 0..200u64 {
            // Incompressible, so the rows fill and seal segments.
            let pad: String = (0..64).map(|_| uuid::Uuid::new_v4().to_string()).collect();
            chain
                .append(&serde_json::json!({ "n": n, "pad": pad }))
                .unwrap();
        }
        let mut seen = Vec::new();
        let mut at = Position::default();
        // Read in pieces of 7, as a reader interrupted between catch-ups.
        loop {
            let mut taken = 0;
            let next = tail(chain.path(), at, |_, bytes| {
                let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
                seen.push(value["n"].as_u64().unwrap());
                taken += 1;
                taken < 7
            });
            if next == at {
                break;
            }
            at = next;
        }
        assert_eq!(seen, (0..200).collect::<Vec<_>>());
        assert_eq!(at, head(chain.path()));
        assert!(at.segment > 1, "the rows crossed a seal");
        chain.append(&serde_json::json!({ "n": 200 })).unwrap();
        let mut more = Vec::new();
        tail(chain.path(), at, |_, bytes| {
            more.push(serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["n"].clone());
            true
        });
        assert_eq!(more, [serde_json::json!(200)]);
    }
}
