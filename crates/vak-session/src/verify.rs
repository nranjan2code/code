//! Verifying a ledger or a record chain without any key (plan M7a-i,
//! docs/design/74 §6.2 A9): every segment's hash chain over the bytes as
//! they are stored, each chaining from the sealed head of the one before.
//! Checks retained segments against their chain and seal manifest without
//! reading content, so it also works for an erased conversation. It cannot
//! establish complete inventory or rollback protection without an external root.

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
    let set = SegmentSet::at(dir);
    let inventory = std::fs::read_dir(dir)
        .map_err(crate::types::SessionError::from)
        .and_then(|_| crate::log::try_segment_numbers(dir));
    let (numbers, seals) = match (inventory, set.verified_seals()) {
        (Ok(numbers), Ok(seals)) => (numbers, seals),
        _ => {
            checked.broken_segment = Some(0);
            return checked;
        }
    };
    for (number, open) in numbers {
        let prev = if number == 1 {
            Some(vak_storage::records::GENESIS)
        } else {
            seals
                .iter()
                .find(|seal| seal.segment == number - 1)
                .map(|seal| seal.head)
        };
        let path = if open {
            set.log_path(number)
        } else {
            set.sealed_path(number)
        };
        let verified = prev
            .ok_or(vak_storage::StorageError::Integrity)
            .and_then(|prev| {
                frame_bytes(&path)
                    .and_then(|bytes| vak_storage::records::verify_bytes(&bytes, prev))
            })
            .and_then(|report| {
                let seal = seals.iter().find(|seal| seal.segment == number);
                if (!open || seal.is_some())
                    && !seal.is_some_and(|seal| {
                        report.torn_tail.is_none()
                            && seal.entries == report.entries
                            && seal.head == report.head
                    })
                {
                    return Err(vak_storage::StorageError::Integrity);
                }
                Ok(report)
            });
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
