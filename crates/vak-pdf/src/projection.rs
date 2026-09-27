//! The page-by-page projection a client draws of a PDF (docs/design/77):
//! the same shape as the Office projection, so one view shows both. Each
//! page is a `page` unit followed by one `paragraph` unit per line.

use serde::Serialize;

use crate::read::Document;

/// A projection page's default size, well inside the worker protocol's
/// limit even when every character needs escaping.
pub const PAGE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unit {
    pub anchor: String,
    pub kind: &'static str,
    pub level: u8,
    pub text: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutlineEntry {
    pub anchor: String,
    pub title: String,
    pub level: u8,
    pub first_unit: usize,
    pub units: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Projection {
    pub vocabulary: &'static str,
    pub kind: &'static str,
    pub extension: &'static str,
    pub macro_enabled: bool,
    pub strict: bool,
    pub title: Option<String>,
    pub stats: Vec<(String, usize)>,
    pub flags: Vec<String>,
    pub sensitivity_labels: Vec<String>,
    pub outline: Vec<OutlineEntry>,
    pub total_units: usize,
    pub from: usize,
    pub next: Option<usize>,
    pub units: Vec<Unit>,
    pub not_read: Vec<String>,
}

/// Every unit of the document, in order.
pub fn units(document: &Document) -> Vec<Unit> {
    let mut units = Vec::new();
    for page in &document.pages {
        let mut labels = Vec::new();
        if page.rotation != 0 {
            labels.push(format!("turned {}°", page.rotation));
        }
        if page.lines.is_empty() {
            labels.push(match (&page.not_read, page.images) {
                (Some(reason), _) => format!("not read: {reason}"),
                (None, 0) => "no text".into(),
                (None, _) => "no text: likely a scan".into(),
            });
        }
        units.push(Unit {
            anchor: format!("page:{}", page.number),
            kind: "page",
            level: 1,
            text: format!("Page {}", page.number),
            labels,
        });
        for line in &page.lines {
            units.push(Unit {
                anchor: line.anchor.clone(),
                kind: "paragraph",
                level: 0,
                text: line.text.clone(),
                labels: line.labels.clone(),
            });
        }
    }
    units
}

fn page_unit(units: &[Unit], page: usize) -> Option<usize> {
    let anchor = format!("page:{page}");
    units.iter().position(|unit| unit.anchor == anchor)
}

/// The page of `document` starting at unit `from`, holding as many units
/// as fit in `budget` bytes, and at least one.
pub fn project(document: &Document, from: usize, budget: usize) -> Projection {
    let all = units(document);
    let from = from.min(all.len());
    let mut used = 0usize;
    let mut end = from;
    for unit in &all[from..] {
        let size = unit.text.len()
            + unit.anchor.len()
            + unit.labels.iter().map(String::len).sum::<usize>()
            + 64;
        if end > from && used + size > budget {
            break;
        }
        used += size;
        end += 1;
    }
    let outline: Vec<(String, String, u8, usize)> = if document.outline.is_empty() {
        document
            .pages
            .iter()
            .filter_map(|page| {
                page_unit(&all, page.number).map(|first| {
                    (
                        format!("page:{}", page.number),
                        format!("Page {}", page.number),
                        1,
                        first,
                    )
                })
            })
            .collect()
    } else {
        document
            .outline
            .iter()
            .filter_map(|bookmark| {
                let page = bookmark.page?;
                let first = page_unit(&all, page)?;
                Some((
                    format!("page:{page}"),
                    bookmark.title.clone(),
                    u8::try_from(bookmark.level.clamp(1, 6)).unwrap_or(1),
                    first,
                ))
            })
            .collect()
    };
    let outline = outline
        .iter()
        .enumerate()
        .map(|(index, (anchor, title, level, first))| {
            let next = outline
                .get(index + 1)
                .map(|(_, _, _, first)| *first)
                .unwrap_or(all.len());
            OutlineEntry {
                anchor: anchor.clone(),
                title: title.clone(),
                level: *level,
                first_unit: *first,
                units: next.saturating_sub(*first),
            }
        })
        .collect();
    Projection {
        vocabulary: "pdf",
        kind: "PDF document",
        extension: "pdf",
        macro_enabled: false,
        strict: false,
        title: document.info.title.clone(),
        stats: document
            .stats()
            .into_iter()
            .map(|(name, count)| (name.to_string(), count))
            .collect(),
        flags: document.inspection.flags(),
        sensitivity_labels: Vec::new(),
        outline,
        total_units: all.len(),
        from,
        next: (end < all.len()).then_some(end),
        units: all[from..end].to_vec(),
        not_read: document.not_read.clone(),
    }
}

/// The unit an anchor names: a line, or a page's own unit.
pub fn locate(document: &Document, anchor: &str) -> Option<usize> {
    if !crate::is_anchor(anchor) {
        return None;
    }
    units(document)
        .iter()
        .position(|unit| unit.anchor == anchor)
}

/// Where the page holding unit `focus` starts: at its PDF page's unit.
pub fn page_start(document: &Document, focus: usize, _budget: usize) -> usize {
    let all = units(document);
    all[..=focus.min(all.len().saturating_sub(1))]
        .iter()
        .rposition(|unit| unit.kind == "page")
        .unwrap_or(0)
}
