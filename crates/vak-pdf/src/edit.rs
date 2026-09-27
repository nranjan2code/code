//! L3: the typed op engine for PDFs (docs/design/77-pdf-documents.md).
//!
//! A new PDF is set from its ops on Vakyartha's own A4 blank. An existing
//! PDF is edited in place: an op names a place from a read of the exact
//! file (`page:3`, `page:3/line:12`), and every anchor in one call means
//! that read, whatever the ops before it did. The ops Word shares keep
//! Word's names: `replace_paragraph_text` and `delete_paragraph` act on one
//! line, `add_paragraph` and `add_table` set new content on new pages.
//! A PDF adds comments, highlights, form fields and page operations.
//!
//! The file is always written whole and clean (see `write`), then read
//! again: an op whose change the re-read does not show fails the call, and
//! nothing is written.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io::Write as _;

use serde::{Deserialize, Serialize};

use crate::content::{TextOp, TextOperator};
use crate::file::File;
use crate::layout::{self, Block, Face, Style};
use crate::lexer::Operations;
use crate::object::{Dict, Object};
use crate::read::{self, Document, PageDetail};
use crate::write::{self, Objects, real};
use crate::{Error, Limits};

/// Most ops one call may carry.
pub const MAX_OPS: usize = 200;

/// The paragraph styles a new PDF offers: the ones a new Word document
/// offers, so a model uses the same names for both.
pub const DOCUMENT_STYLES: &[&str] = &[
    "Normal",
    "Title",
    "Subtitle",
    "Heading 1",
    "Heading 2",
    "Heading 3",
    "List Bullet",
    "List Number",
    "Quote",
];

/// A table cell as a model writes one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl Cell {
    fn text(&self) -> String {
        match self {
            Cell::Bool(value) => value.to_string(),
            Cell::Number(value) if value.fract() == 0.0 && value.abs() < 1e15 => {
                format!("{}", *value as i64)
            }
            Cell::Number(value) => value.to_string(),
            Cell::Text(text) => text.clone(),
        }
    }
}

/// The op set. Every op a new file needs works without an anchor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PdfOp {
    /// The line `anchor` names, redrawn with `text` in Helvetica where the
    /// old text was; the old text is removed from the page.
    ReplaceParagraphText {
        anchor: String,
        text: String,
    },
    /// Removes the line `anchor` names from the page.
    DeleteParagraph {
        anchor: String,
    },
    /// New content on new pages after the page `after` names, or at the end.
    AddParagraph {
        text: String,
        #[serde(default)]
        style: Option<String>,
        #[serde(default)]
        after: Option<String>,
    },
    AddTable {
        rows: Vec<Vec<Cell>>,
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        header: Option<bool>,
    },
    AddPageBreak {
        #[serde(default)]
        after: Option<String>,
    },
    SetTitle {
        title: String,
    },
    /// A sticky-note comment beside the line `anchor` names.
    AddComment {
        anchor: String,
        text: String,
    },
    Highlight {
        anchor: String,
        #[serde(default)]
        note: Option<String>,
    },
    /// Sets a form field by its name as a read lists it.
    FillField {
        field: String,
        value: String,
    },
    RotatePage {
        anchor: String,
        degrees: i64,
    },
    DeletePage {
        anchor: String,
    },
    /// Moves a page after the page `after` names, or to the front.
    MovePage {
        anchor: String,
        #[serde(default)]
        after: Option<String>,
    },
}

fn shorten(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= 60 {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(59).collect::<String>())
    }
}

impl PdfOp {
    pub fn name(&self) -> &'static str {
        match self {
            PdfOp::ReplaceParagraphText { .. } => "replace_paragraph_text",
            PdfOp::DeleteParagraph { .. } => "delete_paragraph",
            PdfOp::AddParagraph { .. } => "add_paragraph",
            PdfOp::AddTable { .. } => "add_table",
            PdfOp::AddPageBreak { .. } => "add_page_break",
            PdfOp::SetTitle { .. } => "set_title",
            PdfOp::AddComment { .. } => "add_comment",
            PdfOp::Highlight { .. } => "highlight",
            PdfOp::FillField { .. } => "fill_field",
            PdfOp::RotatePage { .. } => "rotate_page",
            PdfOp::DeletePage { .. } => "delete_page",
            PdfOp::MovePage { .. } => "move_page",
        }
    }

    /// What the op does, in words a person choosing among changes reads.
    pub fn label(&self) -> String {
        match self {
            PdfOp::ReplaceParagraphText { anchor, text } => {
                format!("Rewrite {anchor} as “{}”", shorten(text))
            }
            PdfOp::DeleteParagraph { anchor } => format!("Delete {anchor}"),
            PdfOp::AddParagraph { text, .. } => format!("Add “{}”", shorten(text)),
            PdfOp::AddTable { rows, .. } => format!("Add a table of {} rows", rows.len()),
            PdfOp::AddPageBreak { .. } => "Start a new page".into(),
            PdfOp::SetTitle { title } => format!("Set the title to “{}”", shorten(title)),
            PdfOp::AddComment { anchor, text } => {
                format!("Comment on {anchor}: “{}”", shorten(text))
            }
            PdfOp::Highlight { anchor, .. } => format!("Highlight {anchor}"),
            PdfOp::FillField { field, value } => {
                format!("Fill {field} with “{}”", shorten(value))
            }
            PdfOp::RotatePage { anchor, degrees } => format!("Rotate {anchor} by {degrees}°"),
            PdfOp::DeletePage { anchor } => format!("Delete {anchor}"),
            PdfOp::MovePage { anchor, after } => match after {
                Some(after) => format!("Move {anchor} after {after}"),
                None => format!("Move {anchor} to the front"),
            },
        }
    }

    /// The places an op touches, for telling whether two edits made
    /// against the same revision overlap.
    pub fn keys(&self) -> Vec<String> {
        match self {
            PdfOp::ReplaceParagraphText { anchor, .. }
            | PdfOp::DeleteParagraph { anchor }
            | PdfOp::AddComment { anchor, .. }
            | PdfOp::Highlight { anchor, .. } => vec![anchor.clone()],
            PdfOp::RotatePage { anchor, .. } | PdfOp::DeletePage { anchor } => {
                vec![page_key(anchor)]
            }
            PdfOp::MovePage { anchor, after } => {
                let mut keys = vec![page_key(anchor)];
                keys.extend(after.as_deref().map(page_key));
                keys.push("page order".into());
                keys
            }
            PdfOp::AddParagraph { after, .. }
            | PdfOp::AddTable { after, .. }
            | PdfOp::AddPageBreak { after } => {
                vec![format!(
                    "new pages after {}",
                    after.as_deref().unwrap_or("the end")
                )]
            }
            PdfOp::SetTitle { .. } => vec!["title".into()],
            PdfOp::FillField { field, .. } => vec![format!("field {field}")],
        }
    }

    fn after(&self) -> Option<Option<&str>> {
        match self {
            PdfOp::AddParagraph { after, .. }
            | PdfOp::AddTable { after, .. }
            | PdfOp::AddPageBreak { after } => Some(after.as_deref()),
            _ => None,
        }
    }

    fn block(&self) -> Option<Result<Block, String>> {
        match self {
            PdfOp::AddParagraph { text, style, .. } => Some(
                Style::named(style.as_deref())
                    .map(|style| Block::Paragraph {
                        text: text.clone(),
                        style,
                    })
                    .ok_or_else(|| {
                        format!(
                            "style {:?} is not one a PDF offers; use one of {}",
                            style.as_deref().unwrap_or_default(),
                            DOCUMENT_STYLES.join(", ")
                        )
                    }),
            ),
            PdfOp::AddTable { rows, header, .. } => Some(Ok(Block::Table {
                rows: rows
                    .iter()
                    .map(|row| row.iter().map(Cell::text).collect())
                    .collect(),
                header: header.unwrap_or(true),
            })),
            PdfOp::AddPageBreak { .. } => Some(Ok(Block::PageBreak)),
            _ => None,
        }
    }
}

fn page_key(anchor: &str) -> String {
    anchor.split('/').next().unwrap_or(anchor).to_string()
}

/// Who and when, stamped on comments, highlights and the file's dates.
/// Supplied by the runtime, never by the model.
#[derive(Debug, Clone)]
pub struct EditContext {
    pub author: String,
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    pub date: String,
}

impl EditContext {
    /// The date as a PDF writes one: `D:YYYYMMDDHHmmSSZ`.
    fn pdf_date(&self) -> String {
        let digits: String = self
            .date
            .chars()
            .filter(char::is_ascii_digit)
            .take(14)
            .collect();
        format!("D:{digits:0<14}Z")
    }
}

/// A written file and what each op did.
#[derive(Debug, Clone)]
pub struct Applied {
    pub bytes: Vec<u8>,
    /// One line per op, in order.
    pub results: Vec<String>,
    pub document: Document,
    /// What writing did beyond the ops: signatures it invalidated, a font
    /// it substituted, history it dropped.
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

fn fail<T>(at: Option<(usize, &PdfOp)>, message: impl Into<String>) -> Result<T, EditError> {
    Err(EditError {
        op: at.map(|(index, op)| (index, op.name())),
        message: message.into(),
    })
}

/// Applies `ops` to `source`, or to the blank when there is no source, and
/// returns the written file, confirmed by a re-read.
pub fn apply(
    source: Option<&[u8]>,
    ops: &[PdfOp],
    context: &EditContext,
    limits: Limits,
) -> Result<Applied, EditError> {
    if ops.is_empty() {
        return fail(None, "no ops were given");
    }
    if ops.len() > MAX_OPS {
        return fail(
            None,
            format!(
                "{} ops is more than the {MAX_OPS} one call may carry",
                ops.len()
            ),
        );
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match source {
        None => create(ops, context, limits),
        Some(source) => edit(source, ops, context, limits),
    }))
    .unwrap_or_else(|_| {
        fail(
            None,
            "the PDF writer failed on this file; nothing was written",
        )
    })
}

/// A text string as a PDF stores one: PDFDocEncoding when plain ASCII,
/// otherwise UTF-16BE behind a byte-order mark.
fn text_bytes(text: &str) -> Vec<u8> {
    if text.bytes().all(|byte| (0x20..0x7F).contains(&byte)) {
        text.as_bytes().to_vec()
    } else {
        let mut out = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            out.extend_from_slice(&unit.to_be_bytes());
        }
        out
    }
}

fn name(bytes: &[u8]) -> Object {
    Object::Name(bytes.to_vec())
}

fn numbers(values: &[f64]) -> Object {
    Object::Array(values.iter().map(|value| Object::Real(*value)).collect())
}

/// The three Helvetica faces new content is set in.
fn add_fonts(objects: &mut Objects<'_>) -> Dict {
    let mut fonts = Dict::default();
    for face in [Face::Regular, Face::Bold, Face::Italic] {
        let number = objects.add(Object::Dict(Dict(vec![
            (b"Type".to_vec(), name(b"Font")),
            (b"Subtype".to_vec(), name(b"Type1")),
            (b"BaseFont".to_vec(), name(face.base_font())),
            (b"Encoding".to_vec(), name(b"WinAnsiEncoding")),
        ])));
        fonts
            .0
            .push((face.resource().to_vec(), Object::Ref(number, 0)));
    }
    fonts
}

struct Heading {
    title: String,
    page: u32,
    top: f64,
    children: Vec<Heading>,
}

fn insert_heading(roots: &mut Vec<Heading>, depth: usize, heading: Heading) {
    let mut siblings = roots;
    for _ in 0..depth {
        if siblings.is_empty() {
            break;
        }
        let last = siblings.len() - 1;
        siblings = &mut siblings[last].children;
    }
    siblings.push(heading);
}

/// Writes a level of bookmarks under `parent` and returns its first and
/// last items and how many items it holds in all.
fn emit_headings(
    objects: &mut Objects<'_>,
    siblings: &[Heading],
    parent: u32,
) -> (u32, u32, usize) {
    let reserved: Vec<u32> = siblings.iter().map(|_| objects.add(Object::Null)).collect();
    let mut total = 0;
    for (index, heading) in siblings.iter().enumerate() {
        let number = reserved[index];
        let mut dict = Dict(vec![
            (
                b"Title".to_vec(),
                Object::String(text_bytes(&heading.title)),
            ),
            (b"Parent".to_vec(), Object::Ref(parent, 0)),
            (
                b"Dest".to_vec(),
                Object::Array(vec![
                    Object::Ref(heading.page, 0),
                    name(b"XYZ"),
                    Object::Int(0),
                    Object::Real(heading.top),
                    Object::Int(0),
                ]),
            ),
        ]);
        if index > 0 {
            dict.0
                .push((b"Prev".to_vec(), Object::Ref(reserved[index - 1], 0)));
        }
        if let Some(next) = reserved.get(index + 1) {
            dict.0.push((b"Next".to_vec(), Object::Ref(*next, 0)));
        }
        if !heading.children.is_empty() {
            let (first, last, count) = emit_headings(objects, &heading.children, number);
            dict.0.push((b"First".to_vec(), Object::Ref(first, 0)));
            dict.0.push((b"Last".to_vec(), Object::Ref(last, 0)));
            dict.0.push((b"Count".to_vec(), Object::Int(count as i64)));
            total += count;
        }
        objects.set(number, Object::Dict(dict));
        total += 1;
    }
    (
        reserved.first().copied().unwrap_or(parent),
        reserved.last().copied().unwrap_or(parent),
        total,
    )
}

/// A heading set on a new page: level, title, page object, top.
type PlacedHeading = (u8, String, u32, f64);

/// New pages for set content, sharing one resources dictionary.
fn add_pages(
    objects: &mut Objects<'_>,
    laid: &[layout::Laid],
    parent: u32,
    size: [f64; 2],
    resources: u32,
) -> (Vec<u32>, Vec<PlacedHeading>) {
    let mut pages = Vec::new();
    let mut headings = Vec::new();
    for page in laid {
        let content = objects.add_stream(Dict::default(), &page.content);
        let number = objects.add(Object::Dict(Dict(vec![
            (b"Type".to_vec(), name(b"Page")),
            (b"Parent".to_vec(), Object::Ref(parent, 0)),
            (b"MediaBox".to_vec(), numbers(&[0.0, 0.0, size[0], size[1]])),
            (b"Resources".to_vec(), Object::Ref(resources, 0)),
            (b"Contents".to_vec(), Object::Ref(content, 0)),
        ])));
        for (level, title, top) in &page.headings {
            headings.push((*level, title.clone(), number, *top));
        }
        pages.push(number);
    }
    (pages, headings)
}

fn create(ops: &[PdfOp], context: &EditContext, limits: Limits) -> Result<Applied, EditError> {
    let mut blocks = Vec::new();
    let mut title = None;
    let mut results = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        let at = Some((index, op));
        match op {
            PdfOp::SetTitle { title: text } => {
                title = Some(text.clone());
                results.push(format!("set the title to “{}”", shorten(text)));
            }
            op => match op.block() {
                Some(block) => {
                    if let Some(Some(after)) = op.after() {
                        return fail(
                            at,
                            format!(
                                "a new PDF has no {after} to put content after; leave out after"
                            ),
                        );
                    }
                    let block = block.or_else(|message| fail(at, message))?;
                    check_text(&block).or_else(|message| fail(at, message))?;
                    results.push(op.label());
                    blocks.push(block);
                }
                None => {
                    return fail(
                        at,
                        format!(
                            "a new PDF has no pages or lines yet for {} to name; build it with add_paragraph, add_table and add_page_break",
                            op.name()
                        ),
                    );
                }
            },
        }
    }
    let laid = layout::lay_out(&blocks, layout::A4).or_else(|message| fail(None, message))?;
    let mut objects = Objects::new(&[]);
    let fonts = add_fonts(&mut objects);
    let resources = objects.add(Object::Dict(Dict(vec![(
        b"Font".to_vec(),
        Object::Dict(fonts),
    )])));
    let tree = objects.add(Object::Null);
    let (pages, headings) = add_pages(&mut objects, &laid, tree, layout::A4, resources);
    objects.set(
        tree,
        Object::Dict(Dict(vec![
            (b"Type".to_vec(), name(b"Pages")),
            (
                b"Kids".to_vec(),
                Object::Array(pages.iter().map(|page| Object::Ref(*page, 0)).collect()),
            ),
            (b"Count".to_vec(), Object::Int(pages.len() as i64)),
        ])),
    );
    let mut catalog = Dict(vec![
        (b"Type".to_vec(), name(b"Catalog")),
        (b"Pages".to_vec(), Object::Ref(tree, 0)),
    ]);
    if !headings.is_empty() {
        let mut roots = Vec::new();
        for (level, text, page, top) in headings {
            let depth = usize::from(level.saturating_sub(1));
            insert_heading(
                &mut roots,
                depth,
                Heading {
                    title: text,
                    page,
                    top,
                    children: Vec::new(),
                },
            );
        }
        let outlines = objects.add(Object::Null);
        let (first, last, count) = emit_headings(&mut objects, &roots, outlines);
        objects.set(
            outlines,
            Object::Dict(Dict(vec![
                (b"Type".to_vec(), name(b"Outlines")),
                (b"First".to_vec(), Object::Ref(first, 0)),
                (b"Last".to_vec(), Object::Ref(last, 0)),
                (b"Count".to_vec(), Object::Int(count as i64)),
            ])),
        );
        catalog
            .0
            .push((b"Outlines".to_vec(), Object::Ref(outlines, 0)));
        catalog.0.push((b"PageMode".to_vec(), name(b"UseOutlines")));
    }
    let catalog = objects.add(Object::Dict(catalog));
    let mut info = Dict(vec![
        (b"Producer".to_vec(), Object::String(b"Vakyartha".to_vec())),
        (b"Creator".to_vec(), Object::String(b"Vakyartha".to_vec())),
        (
            b"CreationDate".to_vec(),
            Object::String(context.pdf_date().into_bytes()),
        ),
        (
            b"ModDate".to_vec(),
            Object::String(context.pdf_date().into_bytes()),
        ),
    ]);
    if let Some(title) = &title {
        info.0
            .push((b"Title".to_vec(), Object::String(text_bytes(title))));
    }
    let info = objects.add(Object::Dict(info));
    let bytes = objects.write("1.7", catalog, Some(info), None);
    let document = read::read(&bytes, limits)?;
    if document.page_count != laid.len() {
        return fail(
            None,
            "the written PDF did not read back with its pages; nothing was written",
        );
    }
    if let Some(title) = &title
        && document.title() != Some(title.trim())
    {
        return fail(
            None,
            "the written PDF did not read back with its title; nothing was written",
        );
    }
    confirm_blocks(&document, &blocks, None)?;
    Ok(Applied {
        bytes,
        results,
        document,
        notices: Vec::new(),
    })
}

/// Checks the text of a block can be drawn before anything is laid out,
/// so the error names the op.
fn check_text(block: &Block) -> Result<(), String> {
    match block {
        Block::Paragraph { text, .. } => layout::encode(text).map(|_| ()),
        Block::Table { rows, .. } => rows
            .iter()
            .flatten()
            .try_for_each(|cell| layout::encode(cell).map(|_| ())),
        Block::PageBreak => Ok(()),
    }
}

/// The first words of every paragraph must read back from `pages` (all
/// pages when `None`), or the call fails.
fn confirm_blocks(
    document: &Document,
    blocks: &[Block],
    pages: Option<&[usize]>,
) -> Result<(), EditError> {
    let text: String = document
        .pages
        .iter()
        .enumerate()
        .filter(|(index, _)| pages.is_none_or(|pages| pages.contains(index)))
        .flat_map(|(_, page)| page.lines.iter().map(|line| line.text.as_str()))
        .collect::<Vec<_>>()
        .join(" ");
    let squashed: String = text.split_whitespace().collect();
    for block in blocks {
        let expected = match block {
            Block::Paragraph { text, .. } => text.clone(),
            Block::Table { rows, .. } => rows
                .first()
                .and_then(|row| row.first())
                .cloned()
                .unwrap_or_default(),
            Block::PageBreak => continue,
        };
        let expected: String = expected
            .split_whitespace()
            .take(4)
            .collect::<String>()
            .replace(['\u{2192}'], "->");
        let expected: String = expected
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect();
        let found: String = squashed
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect();
        if !expected.is_empty() && !found.contains(&expected) {
            return fail(
                None,
                "the written PDF did not read back with the new content; nothing was written",
            );
        }
    }
    Ok(())
}

/// A page in the edited order: one of the source's, by index, or a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Base(usize),
    New(u32),
}

#[derive(Default)]
struct PagePlan {
    /// Replacement bytes for page-level operations, by operation index.
    splices: BTreeMap<usize, Vec<u8>>,
    annotations: Vec<u32>,
    rotate: i64,
    font: Option<Vec<u8>>,
}

struct Source<'f> {
    file: File<'f>,
    document: Document,
    details: Vec<PageDetail>,
}

impl Source<'_> {
    fn page(&self, anchor: &str) -> Result<usize, String> {
        if !crate::is_anchor(anchor) {
            return Err(format!(
                "{anchor:?} is not a PDF anchor; name a page as page:3 or a line as page:3/line:12, as doc_read shows them"
            ));
        }
        let number: usize = anchor
            .trim_start_matches("page:")
            .split('/')
            .next()
            .and_then(|page| page.parse().ok())
            .unwrap_or(0);
        if number == 0 || number > self.document.page_count {
            return Err(format!(
                "{anchor} names no page; the document has {} pages",
                self.document.page_count
            ));
        }
        if number > self.details.len() {
            return Err(format!(
                "page {number} was not read (the reader stops at {} pages), so it cannot be edited",
                self.details.len()
            ));
        }
        Ok(number - 1)
    }

    /// A page and line, `(page index, line index)`.
    fn line(&self, anchor: &str) -> Result<(usize, usize), String> {
        let page = self.page(anchor)?;
        let Some(line) = anchor
            .split_once("/line:")
            .and_then(|(_, line)| line.parse::<usize>().ok())
        else {
            return Err(format!(
                "{anchor} names a page; name one line on it, as page:3/line:12"
            ));
        };
        let lines = self
            .document
            .pages
            .get(page)
            .map_or(0, |page| page.lines.len());
        if line == 0 || line > lines {
            return Err(format!(
                "{anchor} names no line; page {} has {lines} lines",
                page + 1
            ));
        }
        Ok((page, line - 1))
    }

    /// A body line an edit can reach, with its text.
    fn body_line(&self, anchor: &str) -> Result<(usize, usize), String> {
        let (page, line) = self.line(anchor)?;
        let detail = &self.details[page];
        if line >= detail.text.len() {
            return Err(format!(
                "{anchor} is a comment or form field, not text on the page; change it with the op for its kind"
            ));
        }
        Ok((page, line))
    }
}

fn edit(
    source: &[u8],
    ops: &[PdfOp],
    context: &EditContext,
    limits: Limits,
) -> Result<Applied, EditError> {
    let file = File::open(source, limits)?;
    let (document, details) = read::read_detailed(&file, limits, true)?;
    let base = Source {
        file,
        document,
        details,
    };
    let mut objects = Objects::new(source);
    objects.copy(base.file.numbered());
    let Some(root) = base
        .file
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map(|(number, _)| number)
    else {
        return fail(
            None,
            "the PDF's catalog is not an indirect object, so it cannot be edited safely",
        );
    };
    let mut plans: Vec<PagePlan> = base.details.iter().map(|_| PagePlan::default()).collect();
    let mut order: Vec<Slot> = (0..base.details.len()).map(Slot::Base).collect();
    let mut structural = false;
    let mut title: Option<String> = None;
    let mut flows: Vec<(Option<usize>, Vec<Block>)> = Vec::new();
    let mut touched: HashSet<(usize, usize)> = HashSet::new();
    let mut results = Vec::new();
    let mut notices = Vec::new();
    let mut expectations: Vec<Expect> = Vec::new();
    let mut font_substituted = false;
    let mut form_changed = false;
    let date = context.pdf_date();
    for (index, op) in ops.iter().enumerate() {
        let at = Some((index, op));
        match op {
            PdfOp::ReplaceParagraphText { anchor, text } | PdfOp::AddComment { anchor, text }
                if text.trim().is_empty() =>
            {
                return fail(
                    at,
                    format!(
                        "the text for {anchor} is empty; to remove the line, use delete_paragraph"
                    ),
                );
            }
            PdfOp::ReplaceParagraphText { anchor, text } => {
                let (page, line) = base
                    .body_line(anchor)
                    .or_else(|message| fail(at, message))?;
                let encoded = layout::encode(text.trim()).or_else(|message| fail(at, message))?;
                let old = base.details[page].text[line].text.clone();
                rewrite_line(&base, &mut plans, &mut touched, page, line, Some(&encoded))
                    .or_else(|message| fail(at, message))?;
                font_substituted = true;
                results.push(format!(
                    "page {}, line {}: “{}” → “{}”",
                    page + 1,
                    line + 1,
                    shorten(&old),
                    shorten(text)
                ));
                expectations.push(Expect::Text {
                    page,
                    text: text.trim().to_string(),
                });
            }
            PdfOp::DeleteParagraph { anchor } => {
                let (page, line) = base
                    .body_line(anchor)
                    .or_else(|message| fail(at, message))?;
                let old = base.details[page].text[line].text.clone();
                rewrite_line(&base, &mut plans, &mut touched, page, line, None)
                    .or_else(|message| fail(at, message))?;
                results.push(format!(
                    "page {}, line {}: deleted “{}”",
                    page + 1,
                    line + 1,
                    shorten(&old)
                ));
                expectations.push(Expect::Fewer { page, text: old });
            }
            PdfOp::AddComment { anchor, text } => {
                let (page, line) = base
                    .body_line(anchor)
                    .or_else(|message| fail(at, message))?;
                let number = comment(&mut objects, &base, page, line, text.trim(), context, &date);
                plans[page].annotations.push(number);
                results.push(format!(
                    "page {}, line {}: comment “{}”",
                    page + 1,
                    line + 1,
                    shorten(text)
                ));
                expectations.push(Expect::Label {
                    page,
                    label: "comment".into(),
                    text: text.trim().to_string(),
                });
            }
            PdfOp::Highlight { anchor, note } => {
                let (page, line) = base
                    .body_line(anchor)
                    .or_else(|message| fail(at, message))?;
                let number = highlight(
                    &mut objects,
                    &base,
                    page,
                    line,
                    note.as_deref(),
                    context,
                    &date,
                );
                plans[page].annotations.push(number);
                results.push(format!(
                    "page {}, line {}: highlighted “{}”",
                    page + 1,
                    line + 1,
                    shorten(&base.details[page].text[line].text)
                ));
                expectations.push(Expect::Label {
                    page,
                    label: "highlight".into(),
                    text: String::new(),
                });
            }
            PdfOp::FillField { field, value } => {
                let described = fill_field(&mut objects, &base, root, field, value)
                    .or_else(|message| fail(at, message))?;
                form_changed = true;
                results.push(described);
            }
            PdfOp::RotatePage { anchor, degrees } => {
                let page = base.page(anchor).or_else(|message| fail(at, message))?;
                if degrees % 90 != 0 {
                    return fail(at, "degrees must be a multiple of 90");
                }
                plans[page].rotate += degrees;
                results.push(format!("page {}: rotated {degrees}°", page + 1));
            }
            PdfOp::DeletePage { anchor } => {
                let page = base.page(anchor).or_else(|message| fail(at, message))?;
                let Some(position) = order.iter().position(|slot| *slot == Slot::Base(page)) else {
                    return fail(
                        at,
                        format!("page {} was already deleted by an earlier op", page + 1),
                    );
                };
                if order.len() == 1 {
                    return fail(at, "a PDF needs at least one page");
                }
                order.remove(position);
                structural = true;
                results.push(format!("deleted page {}", page + 1));
            }
            PdfOp::MovePage { anchor, after } => {
                let page = base.page(anchor).or_else(|message| fail(at, message))?;
                let Some(position) = order.iter().position(|slot| *slot == Slot::Base(page)) else {
                    return fail(
                        at,
                        format!("page {} was deleted by an earlier op", page + 1),
                    );
                };
                order.remove(position);
                match after {
                    None => order.insert(0, Slot::Base(page)),
                    Some(after) => {
                        let target = base.page(after).or_else(|message| fail(at, message))?;
                        if target == page {
                            return fail(at, "a page cannot move after itself");
                        }
                        let Some(position) =
                            order.iter().position(|slot| *slot == Slot::Base(target))
                        else {
                            return fail(
                                at,
                                format!("page {} was deleted by an earlier op", target + 1),
                            );
                        };
                        order.insert(position + 1, Slot::Base(page));
                    }
                }
                structural = true;
                results.push(match after {
                    Some(after) => format!("moved page {} after {after}", page + 1),
                    None => format!("moved page {} to the front", page + 1),
                });
            }
            PdfOp::SetTitle { title: text } => {
                title = Some(text.clone());
                results.push(format!("set the title to “{}”", shorten(text)));
            }
            op => {
                let Some(block) = op.block() else {
                    return fail(at, "this op does not apply to a PDF");
                };
                let block = block.or_else(|message| fail(at, message))?;
                check_text(&block).or_else(|message| fail(at, message))?;
                let after = match op.after().flatten() {
                    Some(after) => Some(base.page(after).or_else(|message| fail(at, message))?),
                    None => None,
                };
                match flows.last_mut() {
                    Some((last, blocks)) if *last == after => blocks.push(block),
                    _ => flows.push((after, vec![block])),
                }
                structural = true;
                results.push(op.label());
            }
        }
    }
    if structural && base.details.len() < base.document.page_count {
        return fail(
            None,
            format!(
                "the document has {} pages and only {} were read, so its pages cannot be added, removed or reordered",
                base.document.page_count,
                base.details.len()
            ),
        );
    }
    let fonts = if flows.is_empty() {
        None
    } else {
        let fonts = add_fonts(&mut objects);
        Some(objects.add(Object::Dict(Dict(vec![(
            b"Font".to_vec(),
            Object::Dict(fonts),
        )]))))
    };
    let tree = match base
        .file
        .catalog()
        .and_then(|catalog| catalog.get(b"Pages"))
        .and_then(Object::as_reference)
    {
        Some((number, _)) => number,
        None => {
            return fail(
                None,
                "the PDF's page tree is not an indirect object, so it cannot be edited safely",
            );
        }
    };
    for (after, blocks) in &flows {
        let size = match after {
            Some(page) => page_size(&base.details[*page]),
            None => base.details.last().map_or(layout::A4, page_size),
        };
        let laid = layout::lay_out(blocks, size).or_else(|message| fail(None, message))?;
        let (pages, _) = add_pages(&mut objects, &laid, tree, size, fonts.unwrap_or(0));
        let slots: Vec<Slot> = pages.into_iter().map(Slot::New).collect();
        let position = match after {
            Some(page) => match order.iter().position(|slot| *slot == Slot::Base(*page)) {
                Some(position) => position + 1,
                None => {
                    return fail(
                        None,
                        format!(
                            "page {} was deleted, so nothing can be added after it",
                            page + 1
                        ),
                    );
                }
            },
            None => order.len(),
        };
        for (offset, slot) in slots.into_iter().enumerate() {
            order.insert(position + offset, slot);
        }
    }
    // Page dictionaries: every page is rewritten when the tree is, else
    // only those an op changed.
    for (index, detail) in base.details.iter().enumerate() {
        let plan = &plans[index];
        let changed = structural
            || !plan.splices.is_empty()
            || !plan.annotations.is_empty()
            || plan.rotate != 0;
        if !changed {
            continue;
        }
        let Some(number) = detail.object else {
            return fail(
                None,
                format!(
                    "page {} is not an indirect object, so it cannot be edited safely",
                    index + 1
                ),
            );
        };
        let mut dict = detail.dict.clone();
        let set = |dict: &mut Dict, key: &[u8], value: Object| {
            dict.0.retain(|(existing, _)| existing != key);
            dict.0.push((key.to_vec(), value));
        };
        if structural {
            set(&mut dict, b"Parent", Object::Ref(tree, 0));
            if let Some(media) = detail.media {
                set(&mut dict, b"MediaBox", numbers(&media));
            }
            if let Some(crop) = detail.crop {
                set(&mut dict, b"CropBox", numbers(&crop));
            }
            if !dict.has(b"Resources")
                && let Some(resources) = &detail.resources
            {
                set(&mut dict, b"Resources", Object::Dict(resources.clone()));
            }
        }
        let rotation = (detail.rotate + plan.rotate).rem_euclid(360);
        if structural || plan.rotate != 0 {
            set(&mut dict, b"Rotate", Object::Int(rotation));
        }
        if !plan.annotations.is_empty() {
            let mut annotations: Vec<Object> = base
                .file
                .lookup(&detail.dict, b"Annots")
                .as_array()
                .map(<[Object]>::to_vec)
                .unwrap_or_default();
            annotations.extend(
                plan.annotations
                    .iter()
                    .map(|annotation| Object::Ref(*annotation, 0)),
            );
            set(&mut dict, b"Annots", Object::Array(annotations));
        }
        if !plan.splices.is_empty() {
            let content = splice(&detail.content, &plan.splices);
            let stream = objects.add_stream(Dict::default(), &content);
            set(&mut dict, b"Contents", Object::Ref(stream, 0));
            if let Some(font_name) = &plan.font {
                let mut resources = detail.resources.clone().unwrap_or_default();
                let mut fonts = base
                    .file
                    .dict(base.file.lookup(&resources, b"Font"))
                    .cloned()
                    .unwrap_or_default();
                if !fonts.has(font_name) {
                    let font = objects.add(Object::Dict(Dict(vec![
                        (b"Type".to_vec(), name(b"Font")),
                        (b"Subtype".to_vec(), name(b"Type1")),
                        (b"BaseFont".to_vec(), name(b"Helvetica")),
                        (b"Encoding".to_vec(), name(b"WinAnsiEncoding")),
                    ])));
                    fonts.0.push((font_name.clone(), Object::Ref(font, 0)));
                }
                resources.0.retain(|(key, _)| key != b"Font");
                resources.0.push((b"Font".to_vec(), Object::Dict(fonts)));
                set(&mut dict, b"Resources", Object::Dict(resources));
            }
        }
        objects.set(number, Object::Dict(dict));
    }
    if structural {
        let mut kids = Vec::new();
        for slot in &order {
            match slot {
                Slot::Base(index) => match base.details[*index].object {
                    Some(number) => kids.push(Object::Ref(number, 0)),
                    None => {
                        return fail(
                            None,
                            "a page is not an indirect object, so the pages cannot be reordered",
                        );
                    }
                },
                Slot::New(number) => kids.push(Object::Ref(*number, 0)),
            }
        }
        let mut dict = match objects.get(tree) {
            Some(Object::Dict(dict)) => dict.clone(),
            _ => Dict::default(),
        };
        dict.0
            .retain(|(key, _)| key != b"Kids" && key != b"Count" && key != b"Parent");
        dict.0.push((b"Kids".to_vec(), Object::Array(kids)));
        dict.0
            .push((b"Count".to_vec(), Object::Int(order.len() as i64)));
        objects.set(tree, Object::Dict(dict));
    }
    // A deleted page leaves the file: links and bookmarks to it become null.
    for (index, detail) in base.details.iter().enumerate() {
        if !order.contains(&Slot::Base(index))
            && let Some(number) = detail.object
        {
            objects.remove(number);
        }
    }
    let info = match base.file.trailer.get(b"Info") {
        Some(Object::Ref(number, _)) => Some(*number),
        _ => None,
    };
    let mut info_dict = info
        .and_then(|number| objects.get(number))
        .and_then(Object::as_dict)
        .cloned()
        .or_else(|| {
            base.file
                .dict(base.file.lookup(&base.file.trailer, b"Info"))
                .cloned()
        })
        .unwrap_or_default();
    info_dict.0.retain(|(key, _)| key != b"ModDate");
    info_dict.0.push((
        b"ModDate".to_vec(),
        Object::String(date.clone().into_bytes()),
    ));
    if let Some(title) = &title {
        info_dict.0.retain(|(key, _)| key != b"Title");
        info_dict
            .0
            .push((b"Title".to_vec(), Object::String(text_bytes(title))));
    }
    let info = match info {
        Some(number) => {
            objects.set(number, Object::Dict(info_dict));
            number
        }
        None => objects.add(Object::Dict(info_dict)),
    };
    let id = base.file.trailer.get(b"ID").cloned();
    let version = base.file.header_version().to_string();
    let version = if version.as_str() < "1.4" {
        "1.4".to_string()
    } else {
        version
    };
    let bytes = objects.write(&version, root, Some(info), id.as_ref());

    let document = read::read(&bytes, limits)?;
    let final_index = |page: usize| order.iter().position(|slot| *slot == Slot::Base(page));
    if document.page_count != order.len() {
        return fail(
            None,
            "the written PDF did not read back with its pages; nothing was written",
        );
    }
    for expectation in &expectations {
        let page = match expectation {
            Expect::Text { page, .. } | Expect::Fewer { page, .. } | Expect::Label { page, .. } => {
                *page
            }
        };
        let Some(index) = final_index(page) else {
            continue;
        };
        let Some(read_page) = document.pages.get(index) else {
            continue;
        };
        let squash = |text: &str| text.split_whitespace().collect::<String>();
        let shown = match expectation {
            Expect::Text { text, .. } => read_page
                .lines
                .iter()
                .any(|line| squash(&line.text).contains(&squash(text))),
            Expect::Fewer { text, .. } => {
                let before = base.document.pages[page]
                    .lines
                    .iter()
                    .filter(|line| line.text == *text)
                    .count();
                let after = read_page
                    .lines
                    .iter()
                    .filter(|line| line.text == *text)
                    .count();
                after < before
            }
            Expect::Label { label, text, .. } => read_page.lines.iter().any(|line| {
                line.labels
                    .iter()
                    .any(|existing| existing.starts_with(label.as_str()))
                    && (text.is_empty() || squash(&line.text) == squash(text))
            }),
        };
        if !shown {
            return fail(
                None,
                format!(
                    "the written PDF did not show the change on page {}; nothing was written",
                    page + 1
                ),
            );
        }
    }
    for (_, blocks) in &flows {
        confirm_blocks(&document, blocks, None)?;
    }
    for (index, plan) in plans.iter().enumerate() {
        if plan.rotate != 0
            && let Some(position) = final_index(index)
        {
            let expected = (base.details[index].rotate + plan.rotate).rem_euclid(360);
            if document.pages.get(position).map(|page| page.rotation) != Some(expected) {
                return fail(
                    None,
                    "the written PDF did not show a page's rotation; nothing was written",
                );
            }
        }
    }
    if let Some(title) = &title
        && document.title() != Some(title.trim())
    {
        return fail(
            None,
            "the written PDF did not read back with its new title; nothing was written",
        );
    }
    if font_substituted {
        notices.push("Replaced lines are drawn in Helvetica where the old text was; the document's own font may differ.".into());
    }
    if form_changed {
        notices.push("Viewers redraw the changed form fields when the file is opened.".into());
    }
    if base.document.inspection.signatures > 0 {
        notices.push(format!(
            "The file's {} no longer validate: any change to a signed PDF invalidates its signatures.",
            if base.document.inspection.signatures == 1 { "signature".to_string() } else { format!("{} signatures", base.document.inspection.signatures) }
        ));
    }
    if base.document.inspection.revisions > 1 {
        notices.push(
            "The file's earlier saved revisions were dropped; it keeps only its current content."
                .into(),
        );
    }
    Ok(Applied {
        bytes,
        results,
        document,
        notices,
    })
}

enum Expect {
    Text {
        page: usize,
        text: String,
    },
    Fewer {
        page: usize,
        text: String,
    },
    Label {
        page: usize,
        label: String,
        text: String,
    },
}

fn page_size(detail: &PageDetail) -> [f64; 2] {
    let [left, bottom, right, top] =
        detail
            .crop
            .or(detail.media)
            .unwrap_or([0.0, 0.0, layout::A4[0], layout::A4[1]]);
    [
        (right - left).abs().max(72.0),
        (top - bottom).abs().max(72.0),
    ]
}

/// Plans the removal of one line's text operations and, for a
/// replacement, draws `text` in Helvetica where the first of them was.
fn rewrite_line(
    base: &Source<'_>,
    plans: &mut [PagePlan],
    touched: &mut HashSet<(usize, usize)>,
    page: usize,
    line: usize,
    text: Option<&[u8]>,
) -> Result<(), String> {
    let detail = &base.details[page];
    if let Some(reason) = &detail.failure {
        return Err(format!(
            "page {}'s content could not be read ({reason}), so its text cannot be changed",
            page + 1
        ));
    }
    let target = &detail.text[line];
    // Hidden text may be removed, never redrawn as visible text.
    if target.labels != 0 && text.is_some() {
        return Err(format!(
            "line {} of page {} is {}, not text a reader sees; it cannot be rewritten",
            line + 1,
            page + 1,
            crate::content::label_names(target.labels).join(" and ")
        ));
    }
    if !target.editable || target.ops.is_empty() {
        return Err(format!(
            "line {} of page {} is drawn inside a reusable form on the page, which an edit of the page cannot reach",
            line + 1,
            page + 1
        ));
    }
    for (other_index, other) in detail.text.iter().enumerate() {
        if other_index != line && other.ops.iter().any(|op| target.ops.contains(op)) {
            return Err(format!(
                "line {} of page {} shares a text operation with line {}; they can only change together",
                line + 1,
                page + 1,
                other_index + 1
            ));
        }
    }
    let mut ops = target.ops.clone();
    ops.sort_unstable();
    for op in &ops {
        if !touched.insert((page, *op)) {
            return Err(format!(
                "another op in this call already changes line {} of page {}",
                line + 1,
                page + 1
            ));
        }
    }
    let font_name = match &plans[page].font {
        Some(font) => font.clone(),
        None => {
            let existing = base
                .file
                .dict(base.file.lookup(
                    detail.resources.as_ref().unwrap_or(&Dict::default()),
                    b"Font",
                ))
                .cloned()
                .unwrap_or_default();
            let mut candidate = Face::Regular.resource().to_vec();
            let mut suffix = 1;
            while let Some(font) = existing.get(&candidate) {
                let helvetica = base
                    .file
                    .dict(font)
                    .and_then(|font| font.name(b"BaseFont"))
                    .is_some_and(|base_font| base_font == b"Helvetica");
                if helvetica {
                    break;
                }
                candidate = format!("VakH{suffix}").into_bytes();
                suffix += 1;
            }
            candidate
        }
    };
    let available = (target.rect[2] - target.rect[0]).max(1.0);
    for (position, op) in ops.iter().enumerate() {
        let Some(text_op) = detail.text_ops.get(op) else {
            return Err(format!(
                "line {} of page {} could not be located in the page's content",
                line + 1,
                page + 1
            ));
        };
        let drawn = match (position, text) {
            (0, Some(bytes)) => {
                let natural = layout::width(Face::Regular, bytes, target.size.max(1.0));
                let fit = if natural > available * 1.15 {
                    (available * 1.15 / natural).max(0.7)
                } else {
                    1.0
                };
                Some((bytes, fit * 100.0))
            }
            _ => None,
        };
        let bytes = replacement(text_op, drawn, &font_name);
        plans[page].splices.insert(*op, bytes);
    }
    if text.is_some() {
        plans[page].font = Some(font_name);
    }
    Ok(())
}

/// The bytes that stand in for one text-showing operation: a move of the
/// same length, and for the first operation of a replaced line, the new
/// text in Helvetica with the page's own text state put back after it.
fn replacement(op: &TextOp, text: Option<(&[u8], f64)>, font: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    match &op.operator {
        TextOperator::NextLineShow => out.extend_from_slice(b"T* "),
        TextOperator::NextLineSpacedShow { word, character } => {
            let _ = write!(out, "{} Tw {} Tc T* ", real(*word), real(*character));
        }
        TextOperator::Show | TextOperator::ShowArray => {}
    }
    match text {
        Some((bytes, scale)) => {
            write::write_name(&mut out, font);
            let _ = write!(out, " {} Tf 0 Tc 0 Tw {} Tz ", real(op.size), real(scale));
            write::write_string(&mut out, bytes);
            out.extend_from_slice(b" Tj ");
            if !op.font.is_empty() {
                write::write_name(&mut out, &op.font);
                let _ = write!(out, " {} Tf ", real(op.size));
            }
            let _ = write!(
                out,
                "{} Tc {} Tw {} Tz",
                real(op.char_spacing),
                real(op.word_spacing),
                real(op.scale * 100.0)
            );
        }
        None => {
            let adjust = if op.size.abs() > 1e-9 && op.scale.abs() > 1e-9 {
                -op.advance / (op.size * op.scale) * 1000.0
            } else {
                0.0
            };
            let _ = write!(out, "[{}] TJ", real(adjust));
        }
    }
    out
}

/// The page content with each planned operation's bytes replaced.
fn splice(content: &[u8], splices: &BTreeMap<usize, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 256);
    let mut copied = 0usize;
    let mut operations = Operations::new(content);
    let mut index = 0usize;
    while let Some(operation) = operations.next_operation() {
        if let Some(bytes) = splices.get(&index)
            && let Some(before) = content.get(copied..operation.span.start)
        {
            out.extend_from_slice(before);
            out.extend_from_slice(bytes);
            copied = operation.span.end;
        }
        index += 1;
    }
    out.extend_from_slice(content.get(copied..).unwrap_or_default());
    out
}

fn annotation_base(
    page_object: Option<u32>,
    context: &EditContext,
    date: &str,
    subtype: &[u8],
    rect: [f64; 4],
) -> Dict {
    let mut dict = Dict(vec![
        (b"Type".to_vec(), name(b"Annot")),
        (b"Subtype".to_vec(), name(subtype)),
        (b"Rect".to_vec(), numbers(&rect)),
        (b"T".to_vec(), Object::String(text_bytes(&context.author))),
        (b"M".to_vec(), Object::String(date.as_bytes().to_vec())),
        (
            b"CreationDate".to_vec(),
            Object::String(date.as_bytes().to_vec()),
        ),
        (b"F".to_vec(), Object::Int(4)),
    ]);
    if let Some(page) = page_object {
        dict.0.push((b"P".to_vec(), Object::Ref(page, 0)));
    }
    dict
}

fn appearance(
    objects: &mut Objects<'_>,
    width: f64,
    height: f64,
    content: &str,
    multiply: bool,
) -> u32 {
    let mut dict = Dict(vec![
        (b"Type".to_vec(), name(b"XObject")),
        (b"Subtype".to_vec(), name(b"Form")),
        (b"BBox".to_vec(), numbers(&[0.0, 0.0, width, height])),
    ]);
    if multiply {
        dict.0.push((
            b"Resources".to_vec(),
            Object::Dict(Dict(vec![(
                b"ExtGState".to_vec(),
                Object::Dict(Dict(vec![(
                    b"M".to_vec(),
                    Object::Dict(Dict(vec![(b"BM".to_vec(), name(b"Multiply"))])),
                )])),
            )])),
        ));
    }
    objects.add_stream(dict, content.as_bytes())
}

fn comment(
    objects: &mut Objects<'_>,
    base: &Source<'_>,
    page: usize,
    line: usize,
    text: &str,
    context: &EditContext,
    date: &str,
) -> u32 {
    let detail = &base.details[page];
    let rect = detail.text[line].rect;
    let [box_left, _, _, _] = detail.crop.or(detail.media).unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let left = (rect[0] - 22.0).max(box_left + 2.0);
    let top = rect[3];
    let icon = [left, top - 18.0, left + 18.0, top];
    let normal = appearance(
        objects,
        18.0,
        18.0,
        "1 0.82 0.2 rg 0.45 0.35 0.1 RG 0.8 w 0.5 0.5 17 17 re B 0.45 0.35 0.1 rg 4 12 10 1.2 re f 4 8.5 10 1.2 re f 4 5 7 1.2 re f",
        false,
    );
    let mut dict = annotation_base(detail.object, context, date, b"Text", icon);
    dict.0
        .push((b"Contents".to_vec(), Object::String(text_bytes(text))));
    dict.0.push((b"Name".to_vec(), name(b"Comment")));
    dict.0.push((b"C".to_vec(), numbers(&[1.0, 0.82, 0.2])));
    dict.0.push((
        b"AP".to_vec(),
        Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(normal, 0))])),
    ));
    objects.add(Object::Dict(dict))
}

fn highlight(
    objects: &mut Objects<'_>,
    base: &Source<'_>,
    page: usize,
    line: usize,
    note: Option<&str>,
    context: &EditContext,
    date: &str,
) -> u32 {
    let detail = &base.details[page];
    let [left, bottom, right, top] = detail.text[line].rect;
    let rect = [left - 1.0, bottom - 1.0, right + 1.0, top + 1.0];
    let (width, height) = (rect[2] - rect[0], rect[3] - rect[1]);
    let normal = appearance(
        objects,
        width,
        height,
        &format!(
            "/M gs 1 0.9 0.2 rg 0 0 {} {} re f",
            real(width),
            real(height)
        ),
        true,
    );
    let mut dict = annotation_base(detail.object, context, date, b"Highlight", rect);
    dict.0.push((
        b"QuadPoints".to_vec(),
        numbers(&[
            rect[0], rect[3], rect[2], rect[3], rect[0], rect[1], rect[2], rect[1],
        ]),
    ));
    dict.0.push((b"C".to_vec(), numbers(&[1.0, 0.9, 0.2])));
    if let Some(note) = note.map(str::trim).filter(|note| !note.is_empty()) {
        dict.0
            .push((b"Contents".to_vec(), Object::String(text_bytes(note))));
    }
    dict.0.push((
        b"AP".to_vec(),
        Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(normal, 0))])),
    ));
    objects.add(Object::Dict(dict))
}

/// One form field: its object, full name, and the objects of its widgets.
struct Field {
    object: u32,
    name: String,
    kind: Vec<u8>,
    flags: i64,
    widgets: Vec<u32>,
}

fn collect_fields(
    base: &Source<'_>,
    node: &Object,
    parent: &str,
    inherited: (&[u8], i64),
    depth: usize,
    out: &mut Vec<Field>,
) {
    if depth > 16 || out.len() > 5_000 {
        return;
    }
    let Some((object, _)) = node.as_reference() else {
        return;
    };
    let Some(dict) = base.file.dict(node) else {
        return;
    };
    let own = base
        .file
        .lookup(dict, b"T")
        .as_string()
        .map(crate::text::text_string)
        .unwrap_or_default();
    let name = match (parent.is_empty(), own.is_empty()) {
        (_, true) => parent.to_string(),
        (true, false) => own.clone(),
        (false, false) => format!("{parent}.{own}"),
    };
    let kind = dict.name(b"FT").unwrap_or(inherited.0);
    let flags = base
        .file
        .lookup(dict, b"Ff")
        .as_i64()
        .unwrap_or(inherited.1);
    let kids: Vec<&Object> = base
        .file
        .lookup(dict, b"Kids")
        .as_array()
        .map(|kids| kids.iter().collect())
        .unwrap_or_default();
    let named_kids: Vec<&Object> = kids
        .iter()
        .copied()
        .filter(|kid| base.file.dict(kid).is_some_and(|kid| kid.has(b"T")))
        .collect();
    if named_kids.is_empty() {
        let mut widgets: Vec<u32> = kids
            .iter()
            .filter_map(|kid| kid.as_reference().map(|(number, _)| number))
            .collect();
        if dict.name(b"Subtype") == Some(b"Widget") || widgets.is_empty() {
            widgets.push(object);
        }
        out.push(Field {
            object,
            name,
            kind: kind.to_vec(),
            flags,
            widgets,
        });
    } else {
        for kid in named_kids {
            collect_fields(base, kid, &name, (kind, flags), depth + 1, out);
        }
    }
}

fn fill_field(
    objects: &mut Objects<'_>,
    base: &Source<'_>,
    root: u32,
    wanted: &str,
    value: &str,
) -> Result<String, String> {
    let catalog = base.file.catalog().ok_or("the PDF has no catalog")?;
    let form_entry = catalog.get(b"AcroForm").cloned();
    let Some(form) = form_entry.as_ref().and_then(|form| base.file.dict(form)) else {
        return Err("the PDF has no form fields to fill".into());
    };
    let mut fields = Vec::new();
    if let Some(top) = base.file.lookup(form, b"Fields").as_array() {
        for field in top {
            collect_fields(base, field, "", (b"", 0), 0, &mut fields);
        }
    }
    let wanted = wanted.trim();
    let matches: Vec<&Field> = {
        let exact: Vec<&Field> = fields.iter().filter(|field| field.name == wanted).collect();
        if !exact.is_empty() {
            exact
        } else {
            fields
                .iter()
                .filter(|field| {
                    field.name.eq_ignore_ascii_case(wanted)
                        || field
                            .name
                            .rsplit('.')
                            .next()
                            .is_some_and(|last| last.eq_ignore_ascii_case(wanted))
                })
                .collect()
        }
    };
    let field = match matches.as_slice() {
        [field] => *field,
        [] => {
            let names: Vec<&str> = fields
                .iter()
                .take(30)
                .map(|field| field.name.as_str())
                .collect();
            return Err(if names.is_empty() {
                "the PDF has no form fields to fill".into()
            } else {
                format!(
                    "no field is named {wanted:?}; the fields are: {}",
                    names.join(", ")
                )
            });
        }
        several => {
            let names: Vec<&str> = several.iter().map(|field| field.name.as_str()).collect();
            return Err(format!(
                "{wanted:?} names several fields ({}); give the full name",
                names.join(", ")
            ));
        }
    };
    let get = |objects: &Objects<'_>, number: u32| -> Dict {
        match objects.get(number) {
            Some(Object::Dict(dict)) => dict.clone(),
            _ => Dict::default(),
        }
    };
    let set = |dict: &mut Dict, key: &[u8], value: Object| {
        dict.0.retain(|(existing, _)| existing != key);
        dict.0.push((key.to_vec(), value));
    };
    let described = match field.kind.as_slice() {
        b"Sig" => {
            return Err(format!(
                "{} is a signature field, which is never signed or changed here",
                field.name
            ));
        }
        b"Btn" => {
            if field.flags & (1 << 16) != 0 {
                return Err(format!(
                    "{} is a push button, which holds no value",
                    field.name
                ));
            }
            let mut states: Vec<Vec<u8>> = Vec::new();
            for widget in &field.widgets {
                let dict = get(objects, *widget);
                if let Some(normal) = base
                    .file
                    .dict(base.file.lookup(&dict, b"AP"))
                    .and_then(|appearance| base.file.dict(base.file.lookup(appearance, b"N")))
                {
                    for (key, _) in normal.iter() {
                        if key != b"Off" && !states.iter().any(|state| state == key) {
                            states.push(key.to_vec());
                        }
                    }
                }
            }
            let lowered = value.trim().to_ascii_lowercase();
            let chosen: Vec<u8> =
                if matches!(lowered.as_str(), "off" | "no" | "false" | "unchecked" | "") {
                    b"Off".to_vec()
                } else if let Some(state) = states
                    .iter()
                    .find(|state| String::from_utf8_lossy(state).eq_ignore_ascii_case(value.trim()))
                {
                    state.clone()
                } else if matches!(lowered.as_str(), "on" | "yes" | "true" | "checked" | "x") {
                    states.first().cloned().unwrap_or_else(|| b"Yes".to_vec())
                } else {
                    return Err(format!(
                        "{} is a check box or radio button; set it to on or off{}",
                        field.name,
                        if states.is_empty() {
                            String::new()
                        } else {
                            format!(
                                ", or to one of {}",
                                states
                                    .iter()
                                    .map(|state| String::from_utf8_lossy(state).into_owned())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                    ));
                };
            let mut dict = get(objects, field.object);
            set(&mut dict, b"V", Object::Name(chosen.clone()));
            objects.set(field.object, Object::Dict(dict));
            for widget in &field.widgets {
                let mut dict = get(objects, *widget);
                let on_states: Vec<Vec<u8>> = base
                    .file
                    .dict(base.file.lookup(&dict, b"AP"))
                    .and_then(|appearance| base.file.dict(base.file.lookup(appearance, b"N")))
                    .map(|normal| normal.iter().map(|(key, _)| key.to_vec()).collect())
                    .unwrap_or_default();
                let state = if on_states.contains(&chosen) {
                    chosen.clone()
                } else {
                    b"Off".to_vec()
                };
                if *widget == field.object {
                    set(&mut dict, b"V", Object::Name(chosen.clone()));
                }
                set(&mut dict, b"AS", Object::Name(state));
                objects.set(*widget, Object::Dict(dict));
            }
            format!(
                "field {}: set to {}",
                field.name,
                String::from_utf8_lossy(&chosen)
            )
        }
        kind => {
            let options: Option<Vec<Object>> = objects
                .get(field.object)
                .and_then(Object::as_dict)
                .and_then(|dict| {
                    base.file
                        .lookup(dict, b"Opt")
                        .as_array()
                        .map(<[Object]>::to_vec)
                });
            if kind == b"Ch"
                && let Some(options) = options
            {
                let names: Vec<String> = options
                    .iter()
                    .filter_map(|option| match base.file.resolve(option) {
                        Object::String(bytes) => Some(crate::text::text_string(bytes)),
                        Object::Array(pair) => pair
                            .get(1)
                            .or(pair.first())
                            .and_then(|item| item.as_string())
                            .map(crate::text::text_string),
                        _ => None,
                    })
                    .collect();
                if !names.is_empty() && !names.iter().any(|option| option == value.trim()) {
                    return Err(format!("{} takes one of: {}", field.name, names.join(", ")));
                }
            }
            let mut dict = get(objects, field.object);
            set(&mut dict, b"V", Object::String(text_bytes(value)));
            dict.0.retain(|(key, _)| key != b"AP");
            objects.set(field.object, Object::Dict(dict));
            for widget in field
                .widgets
                .iter()
                .filter(|widget| **widget != field.object)
            {
                let mut dict = get(objects, *widget);
                dict.0.retain(|(key, _)| key != b"AP");
                objects.set(*widget, Object::Dict(dict));
            }
            format!("field {}: set to “{}”", field.name, shorten(value))
        }
    };
    // Viewers redraw fields whose values changed.
    match &form_entry {
        Some(Object::Ref(number, _)) => {
            let mut dict = get(objects, *number);
            set(&mut dict, b"NeedAppearances", Object::Bool(true));
            objects.set(*number, Object::Dict(dict));
        }
        Some(Object::Dict(form)) => {
            let mut form = form.clone();
            set(&mut form, b"NeedAppearances", Object::Bool(true));
            let mut catalog = get(objects, root);
            set(&mut catalog, b"AcroForm", Object::Dict(form));
            objects.set(root, Object::Dict(catalog));
        }
        _ => {}
    }
    Ok(described)
}
