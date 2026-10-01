//! L3: the typed op set and its one apply engine (O3–O5, O8, O10).
//!
//! Ops name anchors a read returned. Each op splices the parts it changes
//! (O1), and after the last op the engine writes the package, re-reads it,
//! and checks every op's postcondition against the re-read projection (O5).
//! A failed postcondition fails the whole apply, so a caller never writes a
//! file whose edits did not land.

mod deck;
mod redline;
mod sheet;
mod textdiff;
mod word;

use std::collections::BTreeMap;
use std::fmt;
use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::package::{
    Package, Relationship, Vocabulary, parse_relationships, part_key, rels_part_name,
};
use crate::read::{self, Document};
use crate::splice::{Splice, Tree, escape_attr};
use crate::{Error, Limits};

/// A cell value: a number, a boolean, or text. Text starting with `=` is a
/// formula; a leading `'` writes the rest as literal text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl CellValue {
    /// The value as the text of a Word table cell, where nothing is a
    /// formula.
    pub fn as_text(&self) -> String {
        match self {
            CellValue::Bool(true) => "TRUE".into(),
            CellValue::Bool(false) => "FALSE".into(),
            CellValue::Number(number) => number.to_string(),
            CellValue::Text(text) => text.clone(),
        }
    }
}

/// One paragraph or several, one per line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextValue {
    Lines(Vec<String>),
    One(String),
}

/// One native PowerPoint chart backed by standard cached series values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideChart {
    pub title: String,
    pub chart_type: String,
    pub categories: Vec<String>,
    pub values: Vec<f64>,
}

/// An embedded PowerPoint picture. The bounded base64 payload crosses the
/// worker protocol with its description; the package stores decoded bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideImage {
    pub mime_type: String,
    pub data: String,
    pub alt_text: String,
}

impl TextValue {
    pub fn lines(&self) -> Vec<String> {
        match self {
            TextValue::Lines(lines) => lines.clone(),
            TextValue::One(text) => text.split('\n').map(str::to_string).collect(),
        }
    }
}

/// The op set (docs/design/72-openxml-documents.md, "P2 op set" and
/// "Creating from scratch"). Every op a new file needs works without an
/// anchor, because a model cannot know the anchors an op mints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfficeOp {
    ReplaceParagraphText {
        anchor: String,
        text: String,
    },
    /// After the paragraph `after` names, or at the end of the document.
    AddParagraph {
        text: String,
        #[serde(default)]
        style: Option<String>,
        #[serde(default)]
        after: Option<String>,
    },
    /// After the paragraph `after` names, or at the end of the document;
    /// the first row is a header row unless `header` is false.
    AddTable {
        rows: Vec<Vec<CellValue>>,
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        header: Option<bool>,
    },
    AddImage {
        image: SlideImage,
        #[serde(default)]
        after: Option<String>,
    },
    DeleteParagraph {
        anchor: String,
    },
    SetCells {
        sheet: String,
        cells: BTreeMap<String, CellValue>,
    },
    AppendRows {
        sheet: String,
        rows: Vec<Vec<CellValue>>,
    },
    AddSheet {
        name: String,
    },
    RenameSheet {
        sheet: String,
        name: String,
    },
    FormatCells {
        sheet: String,
        range: String,
        #[serde(default)]
        bold: Option<bool>,
        #[serde(default)]
        italic: Option<bool>,
        #[serde(default)]
        number_format: Option<String>,
        #[serde(default)]
        fill: Option<String>,
        #[serde(default)]
        wrap: Option<bool>,
    },
    SetColumnWidths {
        sheet: String,
        widths: BTreeMap<String, f64>,
    },
    AddChart {
        sheet: String,
        range: String,
        chart_type: String,
        title: String,
        #[serde(default)]
        cell: Option<String>,
    },
    AddExcelTable {
        sheet: String,
        range: String,
        #[serde(default)]
        name: Option<String>,
    },
    AddExcelImage {
        sheet: String,
        cell: String,
        image: SlideImage,
    },
    AddSlideFromLayout {
        layout: String,
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        placeholders: BTreeMap<String, TextValue>,
        /// Native PowerPoint tables inserted into matching layout placeholders
        /// (usually `body` or `idx:1`) using that placeholder's own bounds.
        #[serde(default)]
        tables: BTreeMap<String, Vec<Vec<CellValue>>>,
        /// Native charts inserted into matching content placeholders, with
        /// cached values projected as RAG-readable chart tables.
        #[serde(default)]
        charts: BTreeMap<String, SlideChart>,
        /// PNG/JPEG images embedded as standard media parts with required alt text.
        #[serde(default)]
        images: BTreeMap<String, SlideImage>,
        #[serde(default)]
        notes: Option<String>,
    },
    SetPlaceholderText {
        anchor: String,
        text: TextValue,
    },
    SetNotes {
        anchor: String,
        text: String,
    },
    DeleteSlide {
        anchor: String,
    },
    MoveSlide {
        anchor: String,
        #[serde(default)]
        after: Option<String>,
    },
    SetTitle {
        title: String,
    },
}

impl OfficeOp {
    pub fn name(&self) -> &'static str {
        match self {
            OfficeOp::ReplaceParagraphText { .. } => "replace_paragraph_text",
            OfficeOp::AddParagraph { .. } => "add_paragraph",
            OfficeOp::AddTable { .. } => "add_table",
            OfficeOp::AddImage { .. } => "add_image",
            OfficeOp::DeleteParagraph { .. } => "delete_paragraph",
            OfficeOp::SetCells { .. } => "set_cells",
            OfficeOp::AppendRows { .. } => "append_rows",
            OfficeOp::AddSheet { .. } => "add_sheet",
            OfficeOp::RenameSheet { .. } => "rename_sheet",
            OfficeOp::FormatCells { .. } => "format_cells",
            OfficeOp::SetColumnWidths { .. } => "set_column_widths",
            OfficeOp::AddChart { .. } => "add_chart",
            OfficeOp::AddExcelTable { .. } => "add_excel_table",
            OfficeOp::AddExcelImage { .. } => "add_excel_image",
            OfficeOp::AddSlideFromLayout { .. } => "add_slide_from_layout",
            OfficeOp::SetPlaceholderText { .. } => "set_placeholder_text",
            OfficeOp::SetNotes { .. } => "set_notes",
            OfficeOp::DeleteSlide { .. } => "delete_slide",
            OfficeOp::MoveSlide { .. } => "move_slide",
            OfficeOp::SetTitle { .. } => "set_title",
        }
    }

    fn vocabulary(&self) -> Option<Vocabulary> {
        match self {
            OfficeOp::ReplaceParagraphText { .. }
            | OfficeOp::AddParagraph { .. }
            | OfficeOp::AddTable { .. }
            | OfficeOp::AddImage { .. }
            | OfficeOp::DeleteParagraph { .. } => Some(Vocabulary::Word),
            OfficeOp::SetCells { .. }
            | OfficeOp::AppendRows { .. }
            | OfficeOp::AddSheet { .. }
            | OfficeOp::RenameSheet { .. }
            | OfficeOp::FormatCells { .. }
            | OfficeOp::SetColumnWidths { .. }
            | OfficeOp::AddChart { .. } => Some(Vocabulary::Excel),
            OfficeOp::AddExcelTable { .. } | OfficeOp::AddExcelImage { .. } => {
                Some(Vocabulary::Excel)
            }
            OfficeOp::AddSlideFromLayout { .. }
            | OfficeOp::SetPlaceholderText { .. }
            | OfficeOp::SetNotes { .. }
            | OfficeOp::DeleteSlide { .. }
            | OfficeOp::MoveSlide { .. } => Some(Vocabulary::PowerPoint),
            OfficeOp::SetTitle { .. } => None,
        }
    }
}

/// Who and when, stamped on tracked changes, and whether there are any.
/// Supplied by the runtime, never by the model.
#[derive(Debug, Clone)]
pub struct EditContext {
    pub author: String,
    /// ISO 8601, e.g. `2026-09-24T10:00:00Z`.
    pub date: String,
    /// Word edits are tracked changes in an existing document; a new
    /// document (one the edit creates, as from a template) is written
    /// clean (docs/design/72, R7).
    pub tracked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpResult {
    pub op: String,
    pub summary: String,
    /// The postcondition, checked against a re-read of the written package.
    pub check: String,
    /// The anchors this op minted, in order (`p:<paraId>`, `slide:<id>`;
    /// a table mints one per cell paragraph), which later ops may name.
    /// Replaying a subset remaps them (`review`).
    pub created: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Applied {
    pub bytes: Vec<u8>,
    pub results: Vec<OpResult>,
    pub document: Document,
    /// What the engine did beyond the ops, for the record: a signature it
    /// removed because the edit invalidated it (D4).
    pub notices: Vec<String>,
}

/// Why an apply failed. `op` is the 0-based op index when one op is at
/// fault; the message says how to repair the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError {
    pub op: Option<(usize, &'static str)>,
    pub message: String,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.op {
            Some((index, name)) => write!(f, "op {} ({name}): {}", index + 1, self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for EditError {}

impl From<Error> for EditError {
    fn from(error: Error) -> Self {
        EditError {
            op: None,
            message: error.to_string(),
        }
    }
}

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T, EditError> {
    Err(EditError {
        op: None,
        message: message.into(),
    })
}

/// What an op promises about the re-read document.
#[derive(Debug, Clone)]
pub(crate) enum Expect {
    /// The unit with this anchor exists and its text contains each needle.
    UnitContains {
        anchor: String,
        needles: Vec<String>,
    },
    /// Every piece of the unit's text is marked deleted, or the unit is gone.
    UnitDeleted {
        anchor: String,
    },
    /// Some unit's text contains `needle`, under an anchor starting `prefix`.
    AnyUnitContains {
        prefix: String,
        needle: String,
    },
    /// The stored cell is empty, even though the text projection omits it.
    EmptyCell {
        sheet: String,
        cell: String,
    },
    /// No unit has this anchor.
    Absent {
        anchor: String,
    },
    /// The slide outline is exactly this order of slide anchors.
    SlideOrder(Vec<String>),
    /// A section (sheet, slide, heading) with this anchor exists.
    Section(String),
    /// No section has this anchor.
    NoSection(String),
    Title(String),
    /// A native table is present at the owning shape anchor with these rows.
    Table {
        anchor: String,
        rows: Vec<Vec<String>>,
    },
    /// Each named cell of the sheet reads back with this formatting.
    CellFormat {
        sheet: String,
        cells: Vec<String>,
        format: sheet::Format,
    },
    /// The sheet's columns read back with these widths, by letter.
    ColumnWidths {
        sheet: String,
        widths: BTreeMap<String, f64>,
    },
    /// The op adds (1) or removes (-1) one Word paragraph; the written
    /// document's paragraph count is checked against every op's together.
    ParagraphDelta(i64),
    /// The Word paragraph reads `accepted` (compared folded, as
    /// [`textdiff::fold`] does) with `author`'s tracked changes accepted,
    /// reads `rejected` exactly with them rejected, and still holds the
    /// content the reader does not show (`fixed`, by element name).
    /// A clean edit (`tracked` false) leaves no revisions, so the text
    /// with them rejected is the text asked for too.
    Paragraph {
        anchor: String,
        author: String,
        tracked: bool,
        accepted: String,
        rejected: String,
        fixed: Vec<String>,
    },
}

impl Expect {
    fn anchor(&self) -> Option<&str> {
        match self {
            Expect::UnitContains { anchor, .. }
            | Expect::UnitDeleted { anchor }
            | Expect::Absent { anchor }
            | Expect::Paragraph { anchor, .. }
            | Expect::Table { anchor, .. } => Some(anchor),
            _ => None,
        }
    }
}

pub(crate) struct Outcome {
    pub summary: String,
    pub expect: Vec<Expect>,
    pub created: Vec<String>,
}

impl Outcome {
    /// Paragraphs whose whole text this op sets: an earlier op's promise
    /// about one of them is superseded by this op's.
    fn rewritten(&self) -> impl Iterator<Item = &str> {
        self.expect.iter().filter_map(|expect| match expect {
            Expect::Paragraph { anchor, .. }
            | Expect::UnitDeleted { anchor }
            | Expect::Absent { anchor } => Some(anchor.as_str()),
            _ => None,
        })
    }
}

/// Applies `ops` in order to the package in `source` and returns the new
/// package, having re-read it and checked every op's postcondition.
///
/// `target` is the format the output file will be named as. A template
/// becomes a document by changing only its main part's content type. The
/// vocabulary never changes, and a macro-free file never becomes
/// macro-enabled or the reverse (O10).
pub fn plan_chart_locations(ops: &[OfficeOp]) -> Result<Vec<OfficeOp>, EditError> {
    let mut planned = ops.to_vec();
    let mut chart_cells_by_sheet: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut explicit_chart_cells_by_sheet: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for op in &planned {
        if let OfficeOp::AddChart {
            sheet,
            cell: Some(cell),
            ..
        } = op
        {
            explicit_chart_cells_by_sheet
                .entry(sheet.to_ascii_lowercase())
                .or_default()
                .push(cell.clone());
        }
    }
    for index in 0..planned.len() {
        let (sheet_name, range, explicit) = match &planned[index] {
            OfficeOp::AddChart {
                sheet, range, cell, ..
            } => (sheet.clone(), range.clone(), cell.clone()),
            _ => continue,
        };
        let sheet_key = sheet_name.to_ascii_lowercase();
        let cell = if let Some(cell) = explicit {
            cell
        } else {
            let images = planned
                .iter()
                .filter_map(|candidate| match candidate {
                    OfficeOp::AddExcelImage { sheet, cell, .. }
                        if sheet.eq_ignore_ascii_case(&sheet_name) =>
                    {
                        Some(cell.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mut blockers = explicit_chart_cells_by_sheet
                .get(&sheet_key)
                .cloned()
                .unwrap_or_default();
            blockers.extend(
                chart_cells_by_sheet
                    .get(&sheet_name)
                    .cloned()
                    .unwrap_or_default(),
            );
            sheet::chart_cell_avoiding_objects(&range, &images, &blockers)?
        };
        if let OfficeOp::AddChart { cell: target, .. } = &mut planned[index] {
            *target = Some(cell.clone());
        }
        chart_cells_by_sheet
            .entry(sheet_key)
            .or_default()
            .push(cell);
    }
    Ok(planned)
}

pub fn apply(
    source: &[u8],
    ops: &[OfficeOp],
    context: &EditContext,
    limits: Limits,
    target: Option<crate::Format>,
) -> Result<Applied, EditError> {
    let mut package = Package::open(Cursor::new(source.to_vec()), limits)?;
    let format = package.format();
    let vocabulary = format.vocabulary;
    let retarget = target.filter(|target| *target != format);
    if let Some(target) = retarget {
        if target.vocabulary != vocabulary {
            return fail(format!(
                "the source is {} and cannot be saved as .{}",
                vocabulary.with_article(),
                target.extension()
            ));
        }
        if target.macro_enabled != format.macro_enabled {
            return fail(format!(
                "the source is .{} and the output .{}; Vakyartha never turns a file macro-enabled or strips its macros by renaming, so keep the .{} extension",
                format.extension(),
                target.extension(),
                format.extension()
            ));
        }
    }
    if ops.is_empty() && retarget.is_none() {
        return fail("no ops given");
    }
    let mut work = Work::new(&mut package);
    if let Some(target) = retarget {
        let main = work.main_part();
        work.set_override(&main, target.main_content_type())?;
    }
    let paragraphs_before = if vocabulary == Vocabulary::Word {
        let main = work.main_part();
        let bytes = work.get(&main)?;
        Some(count_paragraphs(&bytes, &main, work.limits())?)
    } else {
        None
    };
    let planned_ops = plan_chart_locations(ops)?;
    let mut outcomes = Vec::with_capacity(planned_ops.len());
    for (index, op) in planned_ops.iter().enumerate() {
        let tag = |error: EditError| EditError {
            op: Some((index, op.name())),
            message: error.message,
        };
        if let Some(needed) = op.vocabulary()
            && needed != vocabulary
        {
            return Err(tag(EditError {
                op: None,
                message: format!(
                    "this op edits {}, and this file is {}",
                    needed.with_article(),
                    vocabulary.with_article()
                ),
            }));
        }
        let outcome = match op {
            OfficeOp::ReplaceParagraphText { anchor, text } => {
                word::replace_paragraph_text(&mut work, context, anchor, text)
            }
            OfficeOp::AddParagraph { text, style, after } => {
                word::add_paragraph(&mut work, context, text, style.as_deref(), after.as_deref())
            }
            OfficeOp::AddTable {
                rows,
                after,
                header,
            } => word::add_table(
                &mut work,
                context,
                rows,
                after.as_deref(),
                header.unwrap_or(true),
            ),
            OfficeOp::AddImage { image, after } => {
                word::add_image(&mut work, context, image, after.as_deref())
            }
            OfficeOp::DeleteParagraph { anchor } => {
                word::delete_paragraph(&mut work, context, anchor)
            }
            OfficeOp::SetCells { sheet, cells } => sheet::set_cells(&mut work, sheet, cells),
            OfficeOp::AppendRows { sheet, rows } => sheet::append_rows(&mut work, sheet, rows),
            OfficeOp::AddSheet { name } => sheet::add_sheet(&mut work, name),
            OfficeOp::RenameSheet { sheet, name } => sheet::rename_sheet(&mut work, sheet, name),
            OfficeOp::FormatCells {
                sheet,
                range,
                bold,
                italic,
                number_format,
                fill,
                wrap,
            } => sheet::format_cells(
                &mut work,
                sheet,
                range,
                &sheet::Format {
                    bold: *bold,
                    italic: *italic,
                    number_format: number_format.clone(),
                    fill: fill.clone(),
                    wrap: *wrap,
                },
            ),
            OfficeOp::SetColumnWidths { sheet, widths } => {
                sheet::set_column_widths(&mut work, sheet, widths)
            }
            OfficeOp::AddChart {
                sheet,
                range,
                chart_type,
                title,
                cell,
            } => {
                let placement = match cell.as_deref() {
                    Some(cell) => Some(cell),
                    None => match ops.get(index) {
                        Some(OfficeOp::AddChart { cell, .. }) => cell.as_deref(),
                        _ => None,
                    },
                };
                sheet::add_chart(&mut work, sheet, range, chart_type, title, placement)
            }
            OfficeOp::AddExcelTable { sheet, range, name } => {
                sheet::add_excel_table(&mut work, sheet, range, name.as_deref())
            }
            OfficeOp::AddExcelImage { sheet, cell, image } => {
                sheet::add_excel_image(&mut work, sheet, cell, image)
            }
            OfficeOp::AddSlideFromLayout {
                layout,
                after,
                placeholders,
                tables,
                charts,
                images,
                notes,
            } => deck::add_slide_from_layout(
                &mut work,
                layout,
                after.as_deref(),
                placeholders,
                tables,
                charts,
                images,
                notes.as_deref(),
            ),
            OfficeOp::SetPlaceholderText { anchor, text } => {
                deck::set_placeholder_text(&mut work, anchor, text)
            }
            OfficeOp::SetNotes { anchor, text } => deck::set_notes(&mut work, anchor, text),
            OfficeOp::DeleteSlide { anchor } => deck::delete_slide(&mut work, anchor),
            OfficeOp::MoveSlide { anchor, after } => {
                deck::move_slide(&mut work, anchor, after.as_deref())
            }
            OfficeOp::SetTitle { title } => set_title(&mut work, title),
        }
        .map_err(tag)?;
        outcomes.push((index, op.name(), outcome));
    }
    let mut notices = Vec::new();
    let signatures = drop_signatures(&mut work)?;
    if signatures > 0 {
        notices.push(format!(
            "the source was digitally signed; its {signatures} signature(s) were removed, because any edit invalidates them and a file must not claim a signature it no longer has"
        ));
    }
    let edits = std::mem::take(&mut work.parts);
    drop(work);
    let bytes = package
        .rewrite(Cursor::new(Vec::new()), &edits)?
        .into_inner();
    let document = read::read(Cursor::new(bytes.clone()), limits).map_err(|error| EditError {
        op: None,
        message: format!("the edited package does not read back: {error}"),
    })?;
    // Every op that changes the slide list expects the order it left, so a
    // later one's expectation covers an earlier one's; the earlier one,
    // checked against the final deck, would fail on the later op's change.
    let superseded: Vec<bool> = (0..outcomes.len())
        .map(|position| {
            outcomes[position + 1..].iter().any(|(_, name, _)| {
                matches!(
                    *name,
                    "add_slide_from_layout" | "delete_slide" | "move_slide"
                )
            })
        })
        .collect();
    // A later op that sets a paragraph's whole text supersedes what an
    // earlier op promised about that paragraph.
    let rewritten_later: Vec<Vec<String>> = (0..outcomes.len())
        .map(|position| {
            outcomes[position + 1..]
                .iter()
                .flat_map(|(_, _, outcome)| outcome.rewritten())
                .map(str::to_string)
                .collect()
        })
        .collect();
    let mut written = Written {
        bytes: &bytes,
        limits,
        package: None,
        main: None,
    };
    if let Some(before) = paragraphs_before {
        let delta: i64 = outcomes
            .iter()
            .flat_map(|(_, _, outcome)| &outcome.expect)
            .map(|expect| match expect {
                Expect::ParagraphDelta(delta) => *delta,
                _ => 0,
            })
            .sum();
        let (name, main) = written.main().map_err(|message| EditError {
            op: None,
            message: format!("postcondition failed after writing: {message}"),
        })?;
        let after = count_paragraphs(main, name, &limits).map_err(|error| EditError {
            op: None,
            message: format!("postcondition failed after writing: {}", error.message),
        })?;
        let expected = i64::try_from(before).unwrap_or(i64::MAX) + delta;
        if i64::try_from(after).unwrap_or(i64::MAX) != expected {
            return Err(EditError {
                op: None,
                message: format!(
                    "postcondition failed after writing: the document has {after} paragraphs, and its edits should leave {expected}"
                ),
            });
        }
    }
    let mut results = Vec::with_capacity(outcomes.len());
    for (position, (index, name, outcome)) in outcomes.into_iter().enumerate() {
        for expect in &outcome.expect {
            if superseded[position] && matches!(expect, Expect::SlideOrder(_)) {
                continue;
            }
            if expect.anchor().is_some_and(|anchor| {
                rewritten_later[position]
                    .iter()
                    .any(|later| later == anchor)
            }) {
                continue;
            }
            check(&document, &mut written, expect).map_err(|message| EditError {
                op: Some((index, name)),
                message: format!("postcondition failed after writing: {message}"),
            })?;
        }
        results.push(OpResult {
            op: name.to_string(),
            summary: outcome.summary,
            check: "passed: re-read of the written package confirms the change".into(),
            created: outcome.created,
        });
    }
    Ok(Applied {
        bytes,
        results,
        document,
        notices,
    })
}

/// Removes the package's digital signatures (D4): the signature origin, its
/// relationship from the package, every signature part and their content
/// types. Returns how many signatures there were. Vakyartha does not verify
/// signatures, but it knows an edit breaks every one of them.
fn drop_signatures<R: std::io::Read + std::io::Seek>(
    work: &mut Work<'_, R>,
) -> Result<usize, EditError> {
    let origins: Vec<Relationship> = work
        .relationships("")?
        .into_iter()
        .filter(|relationship| {
            !relationship.external && relationship.kind == crate::package::REL_SIGNATURE_ORIGIN
        })
        .collect();
    let mut signatures: Vec<String> = Vec::new();
    for origin in &origins {
        for relationship in work.relationships(&origin.target)? {
            if !relationship.external
                && relationship.short_kind() == "signature"
                && !signatures.contains(&relationship.target)
            {
                signatures.push(relationship.target);
            }
        }
    }
    let name = "[Content_Types].xml";
    let bytes = work.get(name)?;
    let tree = Tree::parse(&bytes, name, work.limits())?;
    for index in tree.descendants(0, "Override") {
        let element = &tree.nodes[index].element;
        if element
            .attr("ContentType")
            .is_some_and(|kind| kind.eq_ignore_ascii_case(crate::package::CT_SIGNATURE))
            && let Some(part) = element.attr("PartName")
        {
            let part = part.trim_start_matches('/').to_string();
            if !signatures
                .iter()
                .any(|known| part_key(known) == part_key(&part))
            {
                signatures.push(part);
            }
        }
    }
    for part in &signatures {
        work.remove(part);
        work.remove(&rels_part_name(part));
        work.remove_override(part)?;
    }
    for origin in &origins {
        work.remove(&origin.target);
        work.remove(&rels_part_name(&origin.target));
        work.remove_override(&origin.target)?;
        work.remove_relationship("", &origin.id)?;
    }
    Ok(signatures.len().max(usize::from(!origins.is_empty())))
}

/// The written package, opened and its main part read when a check needs
/// them.
struct Written<'a> {
    bytes: &'a [u8],
    limits: Limits,
    package: Option<Package<Cursor<Vec<u8>>>>,
    main: Option<(String, Vec<u8>)>,
}

impl Written<'_> {
    fn package(&mut self) -> Result<&mut Package<Cursor<Vec<u8>>>, String> {
        if self.package.is_none() {
            let package = Package::open(Cursor::new(self.bytes.to_vec()), self.limits)
                .map_err(|error| error.to_string())?;
            self.package = Some(package);
        }
        self.package
            .as_mut()
            .ok_or_else(|| "the written package could not be opened".to_string())
    }

    fn main(&mut self) -> Result<(&str, &[u8]), String> {
        if self.main.is_none() {
            let package = self.package()?;
            let name = package.main_part().to_string();
            let bytes = package
                .read_part(&name)
                .map_err(|error| error.to_string())?;
            self.main = Some((name, bytes));
        }
        match &self.main {
            Some((name, bytes)) => Ok((name.as_str(), bytes.as_slice())),
            None => Err("the main part could not be read".into()),
        }
    }
}

/// Word paragraphs in a main part, as anchors count them.
fn count_paragraphs(bytes: &[u8], part: &str, limits: &Limits) -> Result<usize, EditError> {
    Ok(Tree::parse(bytes, part, limits)?
        .descendants(0, "p")
        .count())
}

/// `p@N` → N.
fn ordinal(anchor: &str) -> Option<usize> {
    anchor.strip_prefix("p@")?.split('/').next()?.parse().ok()
}

/// The Word paragraphs an op names.
fn paragraph_references(op: &OfficeOp) -> Vec<&str> {
    match op {
        OfficeOp::ReplaceParagraphText { anchor, .. } | OfficeOp::DeleteParagraph { anchor } => {
            vec![anchor.as_str()]
        }
        OfficeOp::AddParagraph { after, .. }
        | OfficeOp::AddTable { after, .. }
        | OfficeOp::AddImage { after, .. } => after.as_deref().into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Whether op `later` names a paragraph that removing the paragraph op
/// `earlier` deletes renumbers. In a new document (written clean) a
/// deleted paragraph is removed, so the `p@` paragraphs after a removed
/// `p@N` move up one; `p:` anchors never move.
pub fn renumbered_by(earlier: &OfficeOp, later: &OfficeOp, context: &EditContext) -> bool {
    let OfficeOp::DeleteParagraph { anchor } = earlier else {
        return false;
    };
    let Some(removed) = ordinal(anchor) else {
        return false;
    };
    !context.tracked
        && paragraph_references(later)
            .iter()
            .filter_map(|reference| ordinal(reference))
            .any(|named| named >= removed)
}

/// Refuses ops written against one read that removing a `p@` paragraph
/// earlier in the same call would renumber (see [`renumbered_by`]): they
/// would land on the wrong paragraph. Checked where one call's ops are
/// applied, never on a replay of several calls, whose later ops were
/// written against the renumbered draft.
pub fn check_renumbering(ops: &[OfficeOp], context: &EditContext) -> Result<(), EditError> {
    for (earlier, op) in ops.iter().enumerate() {
        if let Some(later) =
            (earlier + 1..ops.len()).find(|later| renumbered_by(op, &ops[*later], context))
        {
            return Err(EditError {
                op: Some((later, ops[later].name())),
                message: format!(
                    "it names a paragraph that removing {} (op {}) renumbers in this new document; put paragraph deletions after the ops that name later paragraphs, or read the draft again and use its anchors",
                    paragraph_references(op)
                        .first()
                        .copied()
                        .unwrap_or_default(),
                    earlier + 1
                ),
            });
        }
    }
    Ok(())
}

fn check(document: &Document, written: &mut Written<'_>, expect: &Expect) -> Result<(), String> {
    // A paragraph in a Word table cell is not a unit of its own: it is read
    // as part of its row, under its own anchor.
    let unit = |anchor: &str| -> Option<&str> {
        document
            .units
            .iter()
            .find(|unit| unit.anchor == anchor)
            .map(|unit| unit.text.as_str())
            .or_else(|| {
                document
                    .units
                    .iter()
                    .flat_map(|unit| unit.row_cells.iter().flatten())
                    .find(|(paragraph, _)| paragraph == anchor)
                    .map(|(_, text)| text.as_str())
            })
    };
    match expect {
        Expect::UnitContains { anchor, needles } => {
            let text = unit(anchor).ok_or_else(|| format!("{anchor} is missing"))?;
            match needles
                .iter()
                .find(|needle| !text.contains(needle.as_str()))
            {
                Some(missing) => Err(format!("{anchor} does not contain {missing:?}")),
                None => Ok(()),
            }
        }
        Expect::UnitDeleted { anchor } => match unit(anchor) {
            None => Ok(()),
            Some(text) => {
                let mut rest = text;
                while let Some(start) = rest.find("[deleted by ") {
                    if !rest[..start].trim().is_empty() {
                        return Err(format!("{anchor} still has undeleted text"));
                    }
                    let Some(end) = rest[start..].find(']') else {
                        break;
                    };
                    rest = &rest[start + end + 1..];
                }
                if rest.trim().is_empty() {
                    Ok(())
                } else {
                    Err(format!("{anchor} still has undeleted text"))
                }
            }
        },
        Expect::AnyUnitContains { prefix, needle } => {
            if document.units.iter().any(|unit| {
                unit.anchor.starts_with(prefix.as_str()) && unit.text.contains(needle.as_str())
            }) {
                Ok(())
            } else {
                Err(format!("no {prefix}… unit contains {needle:?}"))
            }
        }
        Expect::EmptyCell { sheet, cell } => {
            sheet::check_empty_cell(written.package()?, sheet, cell)
        }
        Expect::Absent { anchor } => match unit(anchor) {
            None => Ok(()),
            Some(_) => Err(format!("{anchor} is still present")),
        },
        Expect::SlideOrder(order) => {
            let actual: Vec<&str> = document
                .sections
                .iter()
                .map(|section| section.anchor.as_str())
                .collect();
            if actual == order.iter().map(String::as_str).collect::<Vec<_>>() {
                Ok(())
            } else {
                Err(format!("slide order is {actual:?}, expected {order:?}"))
            }
        }
        Expect::Section(anchor) => {
            if document
                .sections
                .iter()
                .any(|section| section.anchor == *anchor)
            {
                Ok(())
            } else {
                Err(format!("no section {anchor}"))
            }
        }
        Expect::NoSection(anchor) => {
            if document
                .sections
                .iter()
                .any(|section| section.anchor == *anchor)
            {
                Err(format!("section {anchor} is still present"))
            } else {
                Ok(())
            }
        }
        Expect::Title(title) => {
            if document.title.as_deref() == Some(title.as_str()) {
                Ok(())
            } else {
                Err(format!("title is {:?}", document.title))
            }
        }
        Expect::Table { anchor, rows } => {
            let table = document
                .tables
                .iter()
                .find(|table| table.anchor == *anchor)
                .ok_or_else(|| format!("table at {anchor} is missing"))?;
            if table.rows == *rows {
                Ok(())
            } else {
                Err(format!(
                    "table at {anchor} does not read back with the requested rows"
                ))
            }
        }
        Expect::CellFormat {
            sheet,
            cells,
            format,
        } => sheet::check_format(written.package()?, sheet, cells, format),
        Expect::ColumnWidths { sheet, widths } => {
            sheet::check_widths(written.package()?, sheet, widths)
        }
        Expect::ParagraphDelta(_) => Ok(()),
        Expect::Paragraph {
            anchor,
            author,
            tracked,
            accepted,
            rejected,
            fixed,
        } => {
            let limits = written.limits;
            let (name, bytes) = written.main()?;
            let views = word::paragraph_views(name, bytes, &limits, anchor, author, *tracked)?;
            if textdiff::fold_text(&views.accepted) != *accepted {
                return Err(format!(
                    "{anchor} reads {:?} with the changes accepted, not the text asked for",
                    views.accepted
                ));
            }
            if !*tracked && textdiff::fold_text(&views.rejected) != *accepted {
                return Err(format!(
                    "{anchor} was to be written clean, and reads {:?} with its changes rejected",
                    views.rejected
                ));
            }
            if *tracked && views.rejected != *rejected {
                return Err(format!(
                    "{anchor} reads {:?} with the changes rejected, not what it read before",
                    views.rejected
                ));
            }
            if views.fixed != *fixed {
                return Err(format!(
                    "{anchor} no longer holds the content the edit had to keep ({} before, {} after)",
                    fixed.join(", "),
                    views.fixed.join(", ")
                ));
            }
            Ok(())
        }
    }
}

fn set_title<R: std::io::Read + std::io::Seek>(
    work: &mut Work<'_, R>,
    title: &str,
) -> Result<Outcome, EditError> {
    let Some(part) = work.related("", "core-properties")? else {
        return fail("this file has no core properties part to hold a title");
    };
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let text = crate::splice::escape_text(title);
    let mut splice = Splice::default();
    match tree.descendants(0, "title").next() {
        Some(node) => {
            let node = &tree.nodes[node];
            if node.is_empty_element() {
                let name = node.element.name.clone();
                splice.replace(node.span.clone(), format!("<{name}>{text}</{name}>"));
            } else {
                splice.replace(node.inner.clone(), text);
            }
        }
        None => {
            let Some(prefix) = tree.prefix_for("http://purl.org/dc/elements/1.1/") else {
                return fail("the core properties part does not declare the Dublin Core namespace");
            };
            splice.insert(
                tree.root().inner.end,
                format!("<{prefix}title>{text}</{prefix}title>"),
            );
        }
    }
    work.put(&part, splice.apply(&bytes, &part)?);
    Ok(Outcome {
        summary: format!("title set to {title:?}"),
        expect: vec![Expect::Title(title.to_string())],
        created: Vec::new(),
    })
}

/// Parts as the ops leave them. `None` means removed.
pub(crate) struct Work<'a, R: std::io::Read + std::io::Seek> {
    package: &'a mut Package<R>,
    parts: BTreeMap<String, Option<Vec<u8>>>,
}

impl<'a, R: std::io::Read + std::io::Seek> Work<'a, R> {
    fn new(package: &'a mut Package<R>) -> Self {
        Self {
            package,
            parts: BTreeMap::new(),
        }
    }

    pub fn limits(&self) -> &Limits {
        self.package.limits()
    }

    pub fn main_part(&self) -> String {
        self.package.main_part().to_string()
    }

    pub fn strict(&self) -> bool {
        self.package.conformance() == crate::Conformance::Strict
    }

    fn entry(&self, name: &str) -> Option<&Option<Vec<u8>>> {
        let key = part_key(name);
        self.parts
            .iter()
            .find(|(existing, _)| part_key(existing) == key)
            .map(|(_, value)| value)
    }

    pub fn exists(&self, name: &str) -> bool {
        match self.entry(name) {
            Some(value) => value.is_some(),
            None => self.package.has_part(name),
        }
    }

    pub fn get(&mut self, name: &str) -> Result<Vec<u8>, EditError> {
        match self.entry(name) {
            Some(Some(bytes)) => Ok(bytes.clone()),
            Some(None) => fail(format!("part {name} was removed by an earlier op")),
            None => Ok(self.package.read_part(name)?),
        }
    }

    pub fn put(&mut self, name: &str, bytes: Vec<u8>) {
        let key = part_key(name);
        self.parts.retain(|existing, _| part_key(existing) != key);
        self.parts
            .insert(name.trim_start_matches('/').to_string(), Some(bytes));
    }

    pub fn snapshot(&mut self) -> Result<Vec<u8>, EditError> {
        self.package
            .rewrite(Cursor::new(Vec::new()), &self.parts)
            .map(Cursor::into_inner)
            .map_err(Into::into)
    }

    pub fn remove(&mut self, name: &str) {
        let key = part_key(name);
        self.parts.retain(|existing, _| part_key(existing) != key);
        self.parts
            .insert(name.trim_start_matches('/').to_string(), None);
    }

    /// Current relationships of `source`, read from its relationships part
    /// as the ops have left it.
    pub fn relationships(&mut self, source: &str) -> Result<Vec<Relationship>, EditError> {
        let rels = rels_part_name(source);
        if !self.exists(&rels) {
            return Ok(Vec::new());
        }
        let bytes = self.get(&rels)?;
        Ok(parse_relationships(&bytes, &rels, source, self.limits())?)
    }

    pub fn related(&mut self, source: &str, short_kind: &str) -> Result<Option<String>, EditError> {
        Ok(self
            .relationships(source)?
            .into_iter()
            .find(|relationship| !relationship.external && relationship.short_kind() == short_kind)
            .map(|relationship| relationship.target))
    }

    pub fn by_id(&mut self, source: &str, id: &str) -> Result<Option<String>, EditError> {
        Ok(self
            .relationships(source)?
            .into_iter()
            .find(|relationship| !relationship.external && relationship.id == id)
            .map(|relationship| relationship.target))
    }

    /// Adds a relationship from `source` to the part `target` and returns
    /// its new id.
    pub fn add_relationship(
        &mut self,
        source: &str,
        kind: &str,
        target: &str,
    ) -> Result<String, EditError> {
        let rels = rels_part_name(source);
        let existing = self.relationships(source)?;
        let mut number = existing.len() + 1;
        let id = loop {
            let candidate = format!("rId{number}");
            if !existing
                .iter()
                .any(|relationship| relationship.id == candidate)
            {
                break candidate;
            }
            number += 1;
        };
        let line = format!(
            r#"<Relationship Id="{id}" Type="{}" Target="{}"/>"#,
            escape_attr(kind),
            escape_attr(&relative_target(source, target))
        );
        if self.exists(&rels) {
            let bytes = self.get(&rels)?;
            let tree = Tree::parse(&bytes, &rels, self.limits())?;
            let mut splice = Splice::default();
            let root = tree.root();
            if root.is_empty_element() {
                let name = root.element.name.clone();
                let open = String::from_utf8_lossy(&bytes[root.span.start..root.span.end - 2])
                    .into_owned();
                splice.replace(root.span.clone(), format!("{open}>{line}</{name}>"));
            } else {
                splice.insert(root.inner.end, line);
            }
            self.put(&rels, splice.apply(&bytes, &rels)?);
        } else {
            self.put(
                &rels,
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{line}</Relationships>"#
                )
                .into_bytes(),
            );
            self.add_default_rels_type()?;
        }
        Ok(id)
    }

    pub fn remove_relationship(&mut self, source: &str, id: &str) -> Result<(), EditError> {
        let rels = rels_part_name(source);
        if !self.exists(&rels) {
            return Ok(());
        }
        let bytes = self.get(&rels)?;
        let tree = Tree::parse(&bytes, &rels, self.limits())?;
        let mut splice = Splice::default();
        for index in tree.descendants(0, "Relationship") {
            if tree.nodes[index].element.attr("Id") == Some(id) {
                splice.replace(tree.nodes[index].span.clone(), "");
            }
        }
        self.put(&rels, splice.apply(&bytes, &rels)?);
        Ok(())
    }

    /// Adds or replaces the content-type override for `part`.
    pub fn set_override(&mut self, part: &str, content_type: &str) -> Result<(), EditError> {
        self.remove_override(part)?;
        let name = "[Content_Types].xml";
        let bytes = self.get(name)?;
        let tree = Tree::parse(&bytes, name, self.limits())?;
        let mut splice = Splice::default();
        splice.insert(
            tree.root().inner.end,
            format!(
                r#"<Override PartName="/{}" ContentType="{}"/>"#,
                escape_attr(part.trim_start_matches('/')),
                escape_attr(content_type)
            ),
        );
        self.put(name, splice.apply(&bytes, name)?);
        Ok(())
    }

    pub fn remove_override(&mut self, part: &str) -> Result<(), EditError> {
        let name = "[Content_Types].xml";
        let bytes = self.get(name)?;
        let tree = Tree::parse(&bytes, name, self.limits())?;
        let key = part_key(part);
        let mut splice = Splice::default();
        for index in tree.descendants(0, "Override") {
            if tree.nodes[index]
                .element
                .attr("PartName")
                .is_some_and(|name| part_key(name) == key)
            {
                splice.replace(tree.nodes[index].span.clone(), "");
            }
        }
        if !splice.is_empty() {
            self.put(name, splice.apply(&bytes, name)?);
        }
        Ok(())
    }

    fn add_default_rels_type(&mut self) -> Result<(), EditError> {
        let name = "[Content_Types].xml";
        let bytes = self.get(name)?;
        let tree = Tree::parse(&bytes, name, self.limits())?;
        let has = tree.descendants(0, "Default").any(|index| {
            tree.nodes[index]
                .element
                .attr("Extension")
                .is_some_and(|extension| extension.eq_ignore_ascii_case("rels"))
        });
        if !has {
            let mut splice = Splice::default();
            splice.insert(
                tree.root().inner.end,
                r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
            );
            self.put(name, splice.apply(&bytes, name)?);
        }
        Ok(())
    }

    /// Part names that currently exist, including ones added by ops.
    pub fn part_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .package
            .part_names()
            .filter(|name| self.entry(name).is_none())
            .map(str::to_string)
            .collect();
        names.extend(
            self.parts
                .iter()
                .filter(|(_, value)| value.is_some())
                .map(|(name, _)| name.clone()),
        );
        names
    }

    pub fn content_type(&mut self, part: &str) -> Result<Option<String>, EditError> {
        let name = "[Content_Types].xml";
        let bytes = self.get(name)?;
        let tree = Tree::parse(&bytes, name, self.limits())?;
        let key = part_key(part);
        for index in tree.descendants(0, "Override") {
            let element = &tree.nodes[index].element;
            if element
                .attr("PartName")
                .is_some_and(|name| part_key(name) == key)
            {
                return Ok(element.attr("ContentType").map(str::to_string));
            }
        }
        let extension = key
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_string());
        for index in tree.descendants(0, "Default") {
            let element = &tree.nodes[index].element;
            if element
                .attr("Extension")
                .zip(extension.as_deref())
                .is_some_and(|(have, want)| have.eq_ignore_ascii_case(want))
            {
                return Ok(element.attr("ContentType").map(str::to_string));
            }
        }
        Ok(None)
    }
}

/// `target` written relative to `source`'s directory, as relationship
/// parts conventionally do (`ppt/slides/slide1.xml` → `../slideLayouts/x`).
pub(crate) fn relative_target(source: &str, target: &str) -> String {
    let base: Vec<&str> = match source.rsplit_once('/') {
        Some((directory, _)) => directory.split('/').collect(),
        None => Vec::new(),
    };
    let target_parts: Vec<&str> = target.trim_start_matches('/').split('/').collect();
    let common = base
        .iter()
        .zip(&target_parts)
        .take_while(|(left, right)| left == right)
        .count();
    let mut path: Vec<String> =
        std::iter::repeat_n("..".to_string(), base.len() - common).collect();
    path.extend(target_parts[common..].iter().map(|part| part.to_string()));
    path.join("/")
}

/// The lowest `prefix{n}{suffix}` part name not in use.
pub(crate) fn free_part_name<R: std::io::Read + std::io::Seek>(
    work: &Work<'_, R>,
    prefix: &str,
    suffix: &str,
) -> String {
    let names: Vec<String> = work
        .part_names()
        .iter()
        .map(|name| part_key(name))
        .collect();
    (1..)
        .map(|number| format!("{prefix}{number}{suffix}"))
        .find(|candidate| !names.contains(&part_key(candidate)))
        .unwrap_or_else(|| format!("{prefix}x{suffix}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn implicit_chart_clears_a_later_excel_image_below_the_data() {
        let ops = vec![
            OfficeOp::AddChart {
                sheet: "Sheet1".into(),
                range: "A1:B3".into(),
                chart_type: "bar".into(),
                title: "Daily visitors".into(),
                cell: None,
            },
            OfficeOp::AddExcelImage {
                sheet: "Sheet1".into(),
                cell: "D2".into(),
                image: SlideImage {
                    mime_type: "image/png".into(),
                    data: String::new(),
                    alt_text: "Daily status marker".into(),
                },
            },
        ];

        let planned = plan_chart_locations(&ops).expect("chart placement");
        assert!(matches!(
            &planned[0],
            OfficeOp::AddChart { cell: Some(cell), .. } if cell == "A10"
        ));
    }

    #[test]
    fn explicit_chart_cell_is_preserved_even_with_an_image_nearby() {
        let ops = vec![
            OfficeOp::AddChart {
                sheet: "Sheet1".into(),
                range: "A1:B3".into(),
                chart_type: "bar".into(),
                title: "Daily visitors".into(),
                cell: Some("J4".into()),
            },
            OfficeOp::AddExcelImage {
                sheet: "Sheet1".into(),
                cell: "D2".into(),
                image: SlideImage {
                    mime_type: "image/png".into(),
                    data: String::new(),
                    alt_text: "Daily status marker".into(),
                },
            },
        ];

        let planned = plan_chart_locations(&ops).expect("chart placement");
        assert!(matches!(
            &planned[0],
            OfficeOp::AddChart { cell: Some(cell), .. } if cell == "J4"
        ));
    }

    #[test]
    fn an_implicit_chart_avoids_an_explicit_chart_even_when_it_comes_later() {
        let ops = vec![
            OfficeOp::AddChart {
                sheet: "Sheet1".into(),
                range: "A1:B3".into(),
                chart_type: "bar".into(),
                title: "Automatic".into(),
                cell: None,
            },
            OfficeOp::AddChart {
                sheet: "Sheet1".into(),
                range: "A1:B3".into(),
                chart_type: "line".into(),
                title: "Human placed".into(),
                cell: Some("A10".into()),
            },
        ];

        let planned = plan_chart_locations(&ops).expect("chart placement");
        assert!(matches!(
            &planned[0],
            OfficeOp::AddChart { cell: Some(cell), .. } if cell == "K4"
        ));
        assert!(matches!(
            &planned[1],
            OfficeOp::AddChart { cell: Some(cell), .. } if cell == "A10"
        ));
    }

    #[test]
    fn relative_targets() {
        assert_eq!(
            relative_target("ppt/slides/slide1.xml", "ppt/slideLayouts/slideLayout2.xml"),
            "../slideLayouts/slideLayout2.xml"
        );
        assert_eq!(
            relative_target("ppt/presentation.xml", "ppt/slides/slide3.xml"),
            "slides/slide3.xml"
        );
        assert_eq!(
            relative_target("", "docProps/core.xml"),
            "docProps/core.xml"
        );
        assert_eq!(
            relative_target("xl/workbook.xml", "xl/worksheets/sheet2.xml"),
            "worksheets/sheet2.xml"
        );
    }
}
