//! Verifying a ledger or a record chain without any key (plan M7a-i,
//! docs/design/74 §6.2 A9): every segment's hash chain over the bytes as
//! they are stored, each chaining from the sealed head of the one before.
//! It proves nothing was changed or reordered; it reads no content, so it
//! works for an erased conversation too.

use std::path::Path;
use vak_storage::segments::{SegmentSet, frame_bytes};

/// What verifying one segment directory found.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Checked {
    pub segments: u64,
    pub frames: u64,
    /// The open segment ends in a frame that was not finished: what a
    /// crash mid-write leaves, and what the next writer trims.
    pub torn_tail: bool,
    /// The first segment whose chain does not verify, if any.
    pub broken_segment: Option<u64>,
}

/// Whether `dir` holds record segments.
pub fn is_segment_dir(dir: &Path) -> bool {
    !crate::log::segment_numbers(dir).is_empty()
}

/// Verifies every segment in `dir`, oldest first, and stops at the first
/// that does not verify.
pub fn verify(dir: &Path) -> Checked {
    let mut checked = Checked::default();
    let Ok(set) = SegmentSet::open(dir) else {
        return checked;
    };
    for (number, open) in crate::log::segment_numbers(dir) {
        let path = if open {
            set.log_path(number)
        } else {
            set.sealed_path(number)
        };
        // The oldest segment left may follow ones retention dropped, whose
        // seal entries stay: its chain starts at the head recorded there.
        let verified = set
            .prev_head(number)
            .and_then(|prev| frame_bytes(&path).map(|bytes| (prev, bytes)))
            .and_then(|(prev, bytes)| vak_storage::records::verify_bytes(&bytes, prev));
        match verified {
            Ok(report) => {
                checked.segments += 1;
                checked.frames += report.entries;
                checked.torn_tail |= open && report.torn_tail.is_some();
            }
            Err(_) => {
                checked.broken_segment = Some(number);
                break;
            }
        }
    }
    checked
}
