//! Bounded, lossless Open XML package engine
//! (docs/design/72-openxml-documents.md).
//!
//! Every package is hostile input (O2): this crate enforces its own byte,
//! entry, ratio, depth and attribute bounds, because the process it runs in
//! may have no OS sandbox at all (F11). It depends on no other vak crate so
//! it can be tested without a server, a model or a sandbox, and every vak
//! call site runs it inside the broker worker (invariant 14).
//!
//! Layers present so far:
//! - [`package`]: L0, the OPC reader, content types, relationship graph,
//!   format detection, security inspection, and the raw-copy writer (O1).
//! - [`xml`]: L1 reading, a bounded event walk that refuses `DOCTYPE`.
//! - [`read`]: L2 read projections with anchors and O6 labels for Word,
//!   Excel, PowerPoint and Visio.

pub mod diff;
pub mod edit;
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod package;
pub mod projection;
pub mod read;
pub mod review;
pub mod splice;
pub mod xml;

pub use package::{
    Conformance, ExternalRelationship, Format, FormatKind, Inspection, Package, Relationship,
    Vocabulary,
};

use std::fmt;

/// Bounds applied to one package. Byte bounds are checked against the
/// bytes actually decompressed, never only against the sizes a ZIP
/// directory declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_entries: usize,
    pub max_part_bytes: u64,
    pub max_total_bytes: u64,
    /// Largest declared expansion ratio accepted for a part bigger than
    /// [`Limits::ratio_floor_bytes`]; small parts legitimately compress well.
    pub max_ratio: u64,
    pub ratio_floor_bytes: u64,
    pub max_xml_depth: usize,
    pub max_attributes: usize,
    pub max_relationship_parts: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_part_bytes: 64 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_ratio: 200,
            ratio_floor_bytes: 1024 * 1024,
            max_xml_depth: 256,
            max_attributes: 256,
            max_relationship_parts: 5_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Io(String),
    /// An OLE compound file: an encrypted Open XML package or a legacy
    /// binary Office file. Neither is readable yet.
    CompoundFile,
    NotZip(String),
    TooManyEntries(usize),
    InvalidPartName(String),
    DuplicatePart(String),
    EncryptedEntry(String),
    UnsupportedCompression(String),
    PartTooLarge(String),
    TotalTooLarge,
    CompressionRatio(String),
    MissingPart(String),
    Xml {
        part: String,
        message: String,
    },
    DocType(String),
    TooDeep(String),
    TooManyAttributes(String),
    NoMainPart,
    RelationshipEscape(String),
    UnsupportedFormat(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(message) => write!(f, "package could not be read: {message}"),
            Error::CompoundFile => f.write_str(
                "file is an OLE compound file: an encrypted Open XML package or a legacy \
                 binary Office format (.doc/.xls/.ppt/.vsd); neither can be read yet",
            ),
            Error::NotZip(message) => {
                write!(
                    f,
                    "file is not an Open XML package (not a ZIP archive): {message}"
                )
            }
            Error::TooManyEntries(count) => {
                write!(f, "package has {count} entries, over the entry limit")
            }
            Error::InvalidPartName(name) => write!(f, "package has an invalid part name {name:?}"),
            Error::DuplicatePart(name) => {
                write!(
                    f,
                    "package has duplicate part {name:?} (names are case-insensitive)"
                )
            }
            Error::EncryptedEntry(name) => write!(f, "package entry {name:?} is ZIP-encrypted"),
            Error::UnsupportedCompression(name) => {
                write!(
                    f,
                    "package entry {name:?} uses an unsupported compression method"
                )
            }
            Error::PartTooLarge(name) => write!(f, "part {name:?} exceeds the per-part size limit"),
            Error::TotalTooLarge => {
                f.write_str("package exceeds the total decompressed size limit")
            }
            Error::CompressionRatio(name) => {
                write!(f, "part {name:?} exceeds the compression-ratio limit")
            }
            Error::MissingPart(name) => write!(f, "package is missing required part {name:?}"),
            Error::Xml { part, message } => write!(f, "part {part:?} is not valid XML: {message}"),
            Error::DocType(part) => {
                write!(
                    f,
                    "part {part:?} declares a DOCTYPE, which Open XML never uses"
                )
            }
            Error::TooDeep(part) => write!(f, "part {part:?} exceeds the XML depth limit"),
            Error::TooManyAttributes(part) => {
                write!(f, "part {part:?} has an element over the attribute limit")
            }
            Error::NoMainPart => {
                f.write_str("package has no officeDocument relationship to a main part")
            }
            Error::RelationshipEscape(target) => {
                write!(f, "relationship target {target:?} escapes the package")
            }
            Error::UnsupportedFormat(content_type) => {
                write!(
                    f,
                    "main part content type {content_type:?} is not a supported Open XML format"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// Lowercased file extensions of the Open XML family this crate opens.
pub const EXTENSIONS: &[&str] = &[
    "docx", "docm", "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "xlam", "pptx", "pptm", "potx",
    "potm", "ppsx", "ppsm", "ppam", "vsdx", "vsdm", "vstx", "vstm", "vssx", "vssm",
];

/// True when `path` names a file of the Open XML family by its extension.
/// Detection is by content type once opened; this only routes a path.
pub fn is_openxml_path(path: &str) -> bool {
    path.rsplit_once('.')
        .map(|(_, extension)| EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// True when `anchor` has the shape of an anchor the reader returns
/// (`p:1A2B3C4D`, `p@12`, `Budget!B4`, `'Q4 plan'!A5:B5`, `Budget!`,
/// `slide:256`, `slide:256/shape:3`, `slide:256/placeholder:title`,
/// `slide:256/notes`, `page:0/shape:5`). It checks shape only: whether the
/// anchor exists is a question for a read of the file.
pub fn is_anchor(anchor: &str) -> bool {
    if anchor.is_empty() || anchor.len() > 300 || anchor.chars().any(char::is_control) {
        return false;
    }
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if let Some(id) = anchor.strip_prefix("p:") {
        return (1..=8).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    if let Some(index) = anchor.strip_prefix("p@") {
        return digits(index);
    }
    if let Some(rest) = anchor.strip_prefix("slide:") {
        let (id, part) = rest.split_once('/').unwrap_or((rest, ""));
        return digits(id)
            && (part.is_empty()
                || part == "notes"
                || part.strip_prefix("shape:").is_some_and(digits)
                || part.strip_prefix("placeholder:").is_some_and(|name| {
                    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
                }));
    }
    if let Some(rest) = anchor.strip_prefix("page:") {
        let (id, part) = rest.split_once('/').unwrap_or((rest, ""));
        return digits(id) && (part.is_empty() || part.strip_prefix("shape:").is_some_and(digits));
    }
    let Some((sheet, cells)) = anchor.rsplit_once('!') else {
        return false;
    };
    let sheet_ok = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(quoted) => !quoted.is_empty() && !quoted.replace("''", "").contains('\''),
        None => {
            !sheet.is_empty()
                && sheet
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        }
    };
    let cell = |text: &str| {
        let letters = text.bytes().take_while(u8::is_ascii_alphabetic).count();
        (1..=3).contains(&letters) && digits(&text[letters..])
    };
    sheet_ok
        && (cells.is_empty()
            || match cells.split_once(':') {
                Some((from, to)) => cell(from) && cell(to),
                None => cell(cells),
            })
}
