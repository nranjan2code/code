//! Bounded, read-only PDF parser (docs/design/77-pdf-documents.md).
//!
//! Every PDF is hostile input: this crate bounds the file, the object
//! count, every decoded stream and the whole document's decoding, the page
//! and line counts, and every nesting depth, because the process it runs in
//! may have no OS sandbox at all. It is written from the specification
//! (ISO 32000-2) with no PDF library beneath it, depends on no other vak
//! crate, and every vak call site runs it inside the broker worker
//! (invariant 14). Nothing in a PDF is executed or followed: JavaScript,
//! actions, embedded files, forms and links are counted and named.
//!
//! Layers:
//! - `lexer`, `object`, `filter`: L0 tokens, objects and stream filters.
//! - `file`: L0 structure, the cross-reference chain, object streams, and
//!   a rebuild by scanning when the table is damaged.
//! - `text`, `font`, `content`: L1 encodings, ToUnicode CMaps, fonts, and
//!   a content interpreter that places and labels text.
//! - [`read`]: L2, the anchored read projection and security inspection.

mod content;
pub mod diff;
pub mod edit;
mod file;
mod filter;
#[cfg(feature = "fixtures")]
pub mod fixtures;
mod font;
mod layout;
mod lexer;
mod object;
pub mod projection;
pub mod read;
pub mod review;
mod text;
mod write;

pub use read::{
    Bookmark, Document, ExternalLink, Info, Inspection, Line, MAX_LINE_CHARS, Page, read,
};

use std::fmt;

/// Bounds applied to one file. Decoded sizes are counted as bytes actually
/// produced, never as a stream declares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_objects: usize,
    /// Most bytes any one stream may decode to.
    pub max_stream_bytes: usize,
    /// Most bytes all streams of one document may decode to together.
    pub max_decoded_bytes: usize,
    /// Most pages read; the rest are counted and named as not read.
    pub max_pages: usize,
    pub max_lines_per_page: usize,
    pub max_line_chars: usize,
    pub max_form_depth: usize,
    pub max_bookmarks: usize,
    pub max_annotations: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 128 * 1024 * 1024,
            max_objects: 500_000,
            max_stream_bytes: 64 * 1024 * 1024,
            max_decoded_bytes: 512 * 1024 * 1024,
            max_pages: 2_000,
            max_lines_per_page: 5_000,
            max_line_chars: MAX_LINE_CHARS,
            max_form_depth: 8,
            max_bookmarks: 2_000,
            max_annotations: 1_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TooLarge {
        bytes: u64,
        limit: u64,
    },
    NotPdf,
    /// Encrypted files are refused, even those that open without a
    /// password: decryption is not implemented.
    Encrypted,
    Malformed(String),
    TooManyObjects(usize),
    NoPages,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLarge { bytes, limit } => write!(
                f,
                "file is {} MiB, over the {} MiB limit for a PDF",
                bytes / (1024 * 1024),
                limit / (1024 * 1024)
            ),
            Error::NotPdf => f.write_str("file is not a PDF: it has no %PDF- header"),
            Error::Encrypted => f.write_str(
                "the PDF is encrypted; encrypted PDFs cannot be read yet, even ones that open \
                 without a password",
            ),
            Error::Malformed(message) => write!(f, "the PDF is damaged: {message}"),
            Error::TooManyObjects(limit) => {
                write!(
                    f,
                    "the PDF has more than {limit} objects, over the object limit"
                )
            }
            Error::NoPages => f.write_str("the PDF has no pages"),
        }
    }
}

impl std::error::Error for Error {}

/// Lowercased file extensions this crate opens.
pub const EXTENSIONS: &[&str] = &["pdf"];

/// True when `path` names a PDF by its extension. Only a route: the file
/// must still start like one.
pub fn is_pdf_path(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

/// True when `bytes` start as a PDF does. Used to route a file whatever
/// its name; the reader itself also accepts a header up to 1 KiB in.
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// True when `anchor` has the shape the reader gives a place:
/// `page:3` or `page:3/line:12`, counted from 1. Shape only: whether the
/// place exists is a question for a read of the file.
pub fn is_anchor(anchor: &str) -> bool {
    let number = |text: &str| {
        !text.is_empty()
            && text.len() <= 7
            && !text.starts_with('0')
            && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    let Some(rest) = anchor.strip_prefix("page:") else {
        return false;
    };
    match rest.split_once("/line:") {
        Some((page, line)) => number(page) && number(line),
        None => number(rest),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn routes_and_anchors() {
        assert!(is_pdf_path("inbox/Report.PDF"));
        assert!(!is_pdf_path("report.pdf.txt"));
        assert!(sniff(b"%PDF-1.7\n"));
        assert!(!sniff(b"# notes on %PDF-1.7"));
        assert!(is_anchor("page:3"));
        assert!(is_anchor("page:12/line:4"));
        for bad in [
            "page:0",
            "page:03",
            "page:",
            "page:3/line:",
            "slide:3",
            "page:3/shape:1",
        ] {
            assert!(!is_anchor(bad), "{bad}");
        }
    }
}
