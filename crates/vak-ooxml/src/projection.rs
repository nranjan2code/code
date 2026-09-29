//! What a view shows of a file (docs/design/72, P4, "The views").
//!
//! A projection is the reader's own output, paged for a client: the outline
//! (headings, sheets or slides, each with the index of its first unit), and
//! one page of units under a byte budget. A client asks for the next page,
//! or for the page that starts at a section, and never unzips a package or
//! mounts document markup; everything it draws is text with anchors and
//! labels (O6).

use chrono::{Duration, NaiveDate};
use serde::Serialize;

use crate::package::{Conformance, Package, Vocabulary};
use crate::read::{CellStyle, Document, SheetGeometry, TableStyleRange, Unit, UnitKind};
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
    /// Visual-only package-scoped media IDs for image anchors on this page.
    pub image_object_ids: std::collections::HashMap<String, String>,
    /// Visual-only XLSX formatting for cells on this page. Kept separate from
    /// each unit's readable values so extraction and RAG stay content-first.
    pub cell_styles: std::collections::HashMap<String, CellStyle>,
    /// Native Excel table boundaries and banding hints for the sheet grid.
    pub table_styles: Vec<TableStyleRange>,
    /// Formatted cell text for the Canvas. Raw values remain in `units` for
    /// extraction, citations, and RAG.
    pub display_values: std::collections::HashMap<String, String>,
    /// Bounded visual sizing metadata for rows and columns on this page's sheets.
    pub sheet_geometry: SheetGeometry,
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
    let visible_cells: std::collections::HashSet<String> = units
        .iter()
        .filter(|unit| unit.kind == UnitKind::SheetRow)
        .flat_map(|unit| {
            let sheet = unit.anchor.rsplit_once('!').map(|(sheet, _)| sheet);
            unit.cells
                .iter()
                .filter_map(move |(cell, _)| sheet.map(|sheet| format!("{sheet}!{cell}")))
        })
        .collect();
    let cell_styles = document
        .cell_styles
        .iter()
        .filter_map(|(address, style)| {
            visible_cells
                .contains(address)
                .then(|| (address.clone(), style.clone()))
        })
        .collect();
    let visible_sheets: std::collections::HashSet<String> = units
        .iter()
        .filter(|unit| unit.kind == UnitKind::SheetRow)
        .filter_map(|unit| {
            unit.anchor
                .rsplit_once('!')
                .map(|(sheet, _)| sheet.to_string())
        })
        .collect();
    let table_styles = document
        .table_styles
        .iter()
        .filter(|style| visible_sheets.contains(style.sheet_anchor.trim_end_matches('!')))
        .cloned()
        .collect();
    let display_values = units
        .iter()
        .filter(|unit| unit.kind == UnitKind::SheetRow)
        .flat_map(|unit| {
            let sheet = unit.anchor.rsplit_once('!').map(|(sheet, _)| sheet);
            unit.cells.iter().filter_map(move |(cell, value)| {
                let address = format!("{}!{cell}", sheet?);
                let format = document
                    .cell_styles
                    .get(&address)?
                    .number_format
                    .as_deref()?;
                let shown = format_excel_number(value, format)?;
                (shown != *value).then_some((address, shown))
            })
        })
        .collect();
    let sheet_geometry = SheetGeometry {
        default_column_widths: document
            .sheet_geometry
            .default_column_widths
            .iter()
            .filter(|(sheet, _)| visible_sheets.contains(*sheet))
            .map(|(sheet, width)| (sheet.clone(), *width))
            .collect(),
        default_row_heights: document
            .sheet_geometry
            .default_row_heights
            .iter()
            .filter(|(sheet, _)| visible_sheets.contains(*sheet))
            .map(|(sheet, height)| (sheet.clone(), *height))
            .collect(),
        column_widths: document
            .sheet_geometry
            .column_widths
            .iter()
            .filter(|(key, _)| {
                visible_sheets
                    .iter()
                    .any(|sheet| key.starts_with(&format!("{sheet}!")))
            })
            .map(|(key, width)| (key.clone(), *width))
            .collect(),
        row_heights: document
            .sheet_geometry
            .row_heights
            .iter()
            .filter(|(key, _)| {
                visible_sheets
                    .iter()
                    .any(|sheet| key.starts_with(&format!("{sheet}!")))
            })
            .map(|(key, height)| (key.clone(), *height))
            .collect(),
        merged_ranges: document
            .sheet_geometry
            .merged_ranges
            .iter()
            .filter(|range| visible_sheets.contains(range.sheet_anchor.trim_end_matches('!')))
            .cloned()
            .collect(),
    };
    let image_object_ids = units
        .iter()
        .filter_map(|unit| {
            document
                .image_object_ids
                .get(&unit.anchor)
                .map(|id| (unit.anchor.clone(), id.clone()))
        })
        .collect();
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
        image_object_ids,
        cell_styles,
        table_styles,
        display_values,
        sheet_geometry,
        not_read: document.not_read.clone(),
    }
}

fn format_excel_number(raw: &str, format: &str) -> Option<String> {
    let value = raw.parse::<f64>().ok()?;
    if format.contains('%') {
        let decimals = format
            .split_once('.')
            .map(|(_, tail)| {
                tail.chars()
                    .take_while(|ch| *ch == '0' || *ch == '#')
                    .count()
            })
            .unwrap_or(0);
        return Some(format!("{:.*}%", decimals.min(8), value * 100.0));
    }
    if format.contains('y') || format.contains('d') || format.contains('m') {
        let days = value.floor() as i64;
        let date =
            NaiveDate::from_ymd_opt(1899, 12, 30)?.checked_add_signed(Duration::days(days))?;
        let fmt = if format.contains("yyyy") {
            "%Y-%m-%d"
        } else if format.contains("mmm") {
            "%d-%b-%y"
        } else if format.contains("m/d") || format.contains("mm/dd") {
            "%-m/%-d/%y"
        } else {
            "%-m/%-d/%y"
        };
        return Some(date.format(fmt).to_string());
    }
    if format == "General" || format == "@" || format.contains('E') || format.contains('/') {
        return None;
    }
    let decimals = format
        .split_once('.')
        .map(|(_, tail)| {
            tail.chars()
                .take_while(|ch| *ch == '0' || *ch == '#')
                .count()
        })
        .unwrap_or(0)
        .min(8);
    let grouping = format.contains(',');
    let symbol = format
        .chars()
        .find(|ch| !ch.is_ascii_alphanumeric() && !"#0?,.;()_- ".contains(*ch));
    let negative = value < 0.0;
    let absolute = format!("{:.*}", decimals, value.abs());
    let (whole, fraction) = absolute.split_once('.').unwrap_or((&absolute, ""));
    let grouped = if grouping {
        let mut out = String::new();
        for (index, ch) in whole.chars().rev().enumerate() {
            if index > 0 && index % 3 == 0 {
                out.push(',');
            }
            out.push(ch);
        }
        out.chars().rev().collect::<String>()
    } else {
        whole.to_string()
    };
    let number = if decimals == 0 {
        grouped
    } else {
        format!("{grouped}.{fraction}")
    };
    let currency = symbol.map(|ch| format!("{ch}{number}")).unwrap_or(number);
    Some(if negative {
        format!("-{currency}")
    } else {
        currency
    })
}

#[cfg(test)]
mod tests {
    use super::format_excel_number;

    #[test]
    fn formats_common_excel_values_for_canvas() {
        assert_eq!(format_excel_number("0.25", "0%"), Some("25%".into()));
        assert_eq!(
            format_excel_number("1234.5", "#,##0.00"),
            Some("1,234.50".into())
        );
        assert_eq!(
            format_excel_number("45292", "m/d/yy"),
            Some("1/1/24".into())
        );
        assert_eq!(format_excel_number("1234.5", "General"), None);
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
    // A paragraph in a Word table cell is cited by its own anchor and
    // shown in its row.
    if let Some(index) = document.units.iter().position(|unit| {
        unit.row_cells
            .iter()
            .flatten()
            .any(|(paragraph, _)| paragraph == anchor)
    }) {
        return Some(index);
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
        + unit
            .row_cells
            .iter()
            .flatten()
            .map(|(anchor, text)| anchor.len() + text.len() + 8)
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
    cut.row_cells.clear();
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
