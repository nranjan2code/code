//! What a view shows of a file (docs/design/72, P4, "The views").
//!
//! A projection is the reader's own output, paged for a client: the outline
//! (headings, sheets or slides, each with the index of its first unit), and
//! one page of units under a byte budget. A client asks for the next page,
//! or for the page that starts at a section, and never unzips a package or
//! mounts document markup; everything it draws is text with anchors and
//! labels (O6).

use serde::Serialize;

use crate::package::{Conformance, Package, Vocabulary};
use crate::read::{Document, Unit};
use crate::{Error, Limits};

/// A page's default size, well inside the worker protocol's limit even
/// when every character needs escaping.
pub const PAGE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Projection {
    pub vocabulary: Vocabulary,
    /// "Word document", "Excel workbook", …
    pub kind: &'static str,
    pub extension: &'static str,
    pub macro_enabled: bool,
    pub strict: bool,
    pub title: Option<String>,
    pub stats: Vec<(String, usize)>,
    /// One line each: macros, external links, hidden content and the like.
    pub flags: Vec<String>,
    pub sensitivity_labels: Vec<String>,
    pub outline: Vec<OutlineEntry>,
    pub total_units: usize,
    /// Index of the first unit on this page.
    pub from: usize,
    /// Where the next page starts; absent on the last page.
    pub next: Option<usize>,
    pub units: Vec<Unit>,
    /// What this reader does not show yet, so an omission is never read as
    /// absence.
    pub not_read: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutlineEntry {
    pub anchor: String,
    pub title: String,
    pub level: u8,
    pub first_unit: usize,
    pub units: usize,
}

/// The page of `document` starting at unit `from`, holding as many units as
/// fit in `budget` bytes, and at least one. A unit larger than the whole
/// budget is cut, and says so in its labels.
pub fn project(document: &Document, from: usize, budget: usize) -> Projection {
    let inspection = &document.inspection;
    let from = from.min(document.units.len());
    let mut used = 0usize;
    let mut units = Vec::new();
    let mut next = None;
    for (index, unit) in document.units.iter().enumerate().skip(from) {
        let size = weight(unit);
        if !units.is_empty() && used + size > budget {
            next = Some(index);
            break;
        }
        used += size;
        units.push(if size > budget {
            cut(unit, budget)
        } else {
            unit.clone()
        });
    }
    Projection {
        vocabulary: inspection.format.vocabulary,
        kind: inspection.format.vocabulary.label(),
        extension: inspection.format.extension(),
        macro_enabled: inspection.format.macro_enabled,
        strict: inspection.conformance == Conformance::Strict,
        title: document.title.clone(),
        stats: document.stats.clone(),
        flags: inspection.flags(),
        sensitivity_labels: inspection.sensitivity_labels.clone(),
        outline: document
            .sections
            .iter()
            .map(|section| OutlineEntry {
                anchor: section.anchor.clone(),
                title: section.title.clone(),
                level: section.level,
                first_unit: section.units.start,
                units: section.units.len(),
            })
            .collect(),
        total_units: document.units.len(),
        from,
        next,
        units,
        not_read: document.not_read.clone(),
    }
}

/// The index of the unit a citation names: the unit with exactly that
/// anchor; for a cell or range (`Budget!B4`, `Budget!B4:D9`), the row that
/// holds its first cell; otherwise the first unit inside the section or
/// slide it names. `None` when the file has no such place.
pub fn locate(document: &Document, anchor: &str) -> Option<usize> {
    let anchor = anchor.trim();
    if let Some(index) = document.units.iter().position(|unit| unit.anchor == anchor) {
        return Some(index);
    }
    if let Some((sheet, cells)) = anchor.rsplit_once('!') {
        let first = cells
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let prefix = format!("{sheet}!");
        if let Some(index) = document.units.iter().position(|unit| {
            unit.anchor.starts_with(&prefix)
                && unit
                    .cells
                    .iter()
                    .any(|(address, _)| address.eq_ignore_ascii_case(&first))
        }) {
            return Some(index);
        }
        let row: String = first
            .chars()
            .skip_while(char::is_ascii_alphabetic)
            .collect();
        if !row.is_empty() {
            let row_suffix = |unit: &Unit| {
                unit.anchor
                    .rsplit_once(':')
                    .map(|(_, last)| {
                        last.trim_start_matches(|c: char| c.is_ascii_alphabetic()) == row
                    })
                    .unwrap_or(false)
            };
            if let Some(index) = document
                .units
                .iter()
                .position(|unit| unit.anchor.starts_with(&prefix) && row_suffix(unit))
            {
                return Some(index);
            }
        }
    }
    document.units.iter().position(|unit| {
        unit.anchor.starts_with(anchor)
            && matches!(
                unit.anchor.as_bytes().get(anchor.len()),
                Some(b'/' | b'!' | b':')
            )
    })
}

/// Where a page opened at the cited unit `focus` starts: the start of the
/// innermost section that holds it (a sheet's first row, the heading above
/// a paragraph, a slide's title), so a citation opens with its context,
/// when everything from there through the cited unit fits in `budget`;
/// otherwise the cited unit itself.
pub fn page_start(document: &Document, focus: usize, budget: usize) -> usize {
    if focus >= document.units.len() {
        return focus;
    }
    let start = document
        .sections
        .iter()
        .filter(|section| section.units.contains(&focus))
        .map(|section| section.units.start)
        .max()
        .unwrap_or(focus);
    let span: usize = document.units[start..=focus].iter().map(weight).sum();
    if span <= budget { start } else { focus }
}

/// Roughly what a unit costs once serialised.
fn weight(unit: &Unit) -> usize {
    64 + unit.anchor.len()
        + unit.text.len()
        + unit
            .labels
            .iter()
            .map(|label| label.len() + 4)
            .sum::<usize>()
        + unit
            .cells
            .iter()
            .map(|(address, value)| address.len() + value.len() + 8)
            .sum::<usize>()
}

fn cut(unit: &Unit, budget: usize) -> Unit {
    let keep = budget.saturating_sub(1024).max(1024);
    let mut end = keep.min(unit.text.len());
    while !unit.text.is_char_boundary(end) {
        end -= 1;
    }
    let mut cut = unit.clone();
    cut.text.truncate(end);
    cut.labels.push(format!(
        "shown up to {} of {} characters; doc_read pages through the rest",
        cut.text.chars().count(),
        unit.text.chars().count()
    ));
    cut.cells.clear();
    cut
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Structure {
    pub main_part: String,
    pub parts: Vec<PartInfo>,
    pub relationships: Vec<RelationshipInfo>,
    pub untyped_parts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartInfo {
    pub name: String,
    pub content_type: Option<String>,
    /// Uncompressed size, as the archive's directory states it.
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelationshipInfo {
    /// The part the relationship belongs to; empty for the package itself.
    pub source: String,
    pub id: String,
    /// Last segment of the relationship type (`image`, `hyperlink`).
    pub kind: String,
    pub target: String,
    /// External targets are recorded and never followed.
    pub external: bool,
}

/// The package's parts, their content types and every relationship, for
/// the Structure view (U4). Reads only the relationship parts.
pub fn structure<R: std::io::Read + std::io::Seek>(
    reader: R,
    limits: Limits,
) -> Result<Structure, Error> {
    let mut package = Package::open(reader, limits)?;
    let parts: Vec<PartInfo> = package
        .part_sizes()
        .into_iter()
        .map(|(name, size)| PartInfo {
            content_type: package.content_type(&name),
            name,
            size,
        })
        .collect();
    let mut relationships = Vec::new();
    let sources: Vec<String> = parts
        .iter()
        .filter_map(|part| crate::package::rels_source(&part.name))
        .collect();
    for source in sources {
        for relationship in package.relationships(&source)? {
            relationships.push(RelationshipInfo {
                source: source.clone(),
                id: relationship.id.clone(),
                kind: relationship.short_kind().to_string(),
                target: relationship.target.clone(),
                external: relationship.external,
            });
        }
    }
    let inspection = package.inspect()?;
    Ok(Structure {
        main_part: package.main_part().to_string(),
        parts,
        relationships,
        untyped_parts: inspection.untyped_parts,
    })
}
