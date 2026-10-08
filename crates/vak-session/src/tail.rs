//! Reading a segment directory (a session ledger or a record chain) from a
//! position onward (plan M6): what a derived index uses to catch up without
//! rereading what it already took. A sealed segment holds the same frames as
//! the open one it replaced, so a position stays valid across a seal.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A place in a segment directory: the segment, and how many of its frames
/// lie before it. The default is the start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Position {
    pub segment: u64,
    pub frames: u64,
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
            if !visit(at, &frame.entry) {
                return at;
            }
        }
    }
    at
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
