//! L3: the typed op set and its one apply engine (O3–O5, O8, O10).
//!
//! Ops name anchors a read returned. Each op splices the parts it changes
//! (O1), and after the last op the engine writes the package, re-reads it,
//! and checks every op's postcondition against the re-read projection (O5).
//! A failed postcondition fails the whole apply, so a caller never writes a
//! file whose edits did not land.

mod deck;
mod sheet;
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

/// One paragraph or several, one per line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextValue {
    Lines(Vec<String>),
    One(String),
}

impl TextValue {
    pub fn lines(&self) -> Vec<String> {
        match self {
            TextValue::Lines(lines) => lines.clone(),
            TextValue::One(text) => text.split('\n').map(str::to_string).collect(),
        }
    }
}

/// The v1 op set (docs/design/72-openxml-documents.md, "P2 op set").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfficeOp {
    ReplaceParagraphText {
        anchor: String,
        text: String,
    },
    InsertParagraphAfter {
        anchor: String,
        text: String,
        #[serde(default)]
        style: Option<String>,
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
    AddSlideFromLayout {
        layout: String,
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        placeholders: BTreeMap<String, TextValue>,
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
            OfficeOp::InsertParagraphAfter { .. } => "insert_paragraph_after",
            OfficeOp::DeleteParagraph { .. } => "delete_paragraph",
            OfficeOp::SetCells { .. } => "set_cells",
            OfficeOp::AppendRows { .. } => "append_rows",
            OfficeOp::AddSheet { .. } => "add_sheet",
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
            | OfficeOp::InsertParagraphAfter { .. }
            | OfficeOp::DeleteParagraph { .. } => Some(Vocabulary::Word),
            OfficeOp::SetCells { .. } | OfficeOp::AppendRows { .. } | OfficeOp::AddSheet { .. } => {
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

/// Who and when, stamped on tracked changes. Supplied by the runtime, never
/// by the model.
#[derive(Debug, Clone)]
pub struct EditContext {
    pub author: String,
    /// ISO 8601, e.g. `2026-09-24T10:00:00Z`.
    pub date: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpResult {
    pub op: String,
    pub summary: String,
    /// The postcondition, checked against a re-read of the written package.
    pub check: String,
    /// The anchor this op minted (`p:<paraId>`, `slide:<id>`), which later
    /// ops may name. Replaying a subset remaps it (`review`).
    pub created: Option<String>,
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
    /// No unit has this anchor.
    Absent {
        anchor: String,
    },
    /// The slide outline is exactly this order of slide anchors.
    SlideOrder(Vec<String>),
    /// A section (sheet, slide, heading) with this anchor exists.
    Section(String),
    Title(String),
}

pub(crate) struct Outcome {
    pub summary: String,
    pub expect: Vec<Expect>,
    pub created: Option<String>,
}

/// Applies `ops` in order to the package in `source` and returns the new
/// package, having re-read it and checked every op's postcondition.
///
/// `target` is the format the output file will be named as. A template
/// becomes a document by changing only its main part's content type. The
/// vocabulary never changes, and a macro-free file never becomes
/// macro-enabled or the reverse (O10).
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
    let mut outcomes = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter().enumerate() {
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
            OfficeOp::InsertParagraphAfter {
                anchor,
                text,
                style,
            } => word::insert_paragraph_after(&mut work, context, anchor, text, style.as_deref()),
            OfficeOp::DeleteParagraph { anchor } => {
                word::delete_paragraph(&mut work, context, anchor)
            }
            OfficeOp::SetCells { sheet, cells } => sheet::set_cells(&mut work, sheet, cells),
            OfficeOp::AppendRows { sheet, rows } => sheet::append_rows(&mut work, sheet, rows),
            OfficeOp::AddSheet { name } => sheet::add_sheet(&mut work, name),
            OfficeOp::AddSlideFromLayout {
                layout,
                after,
                placeholders,
            } => deck::add_slide_from_layout(&mut work, layout, after.as_deref(), placeholders),
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
    let mut results = Vec::with_capacity(outcomes.len());
    for (position, (index, name, outcome)) in outcomes.into_iter().enumerate() {
        for expect in &outcome.expect {
            if superseded[position] && matches!(expect, Expect::SlideOrder(_)) {
                continue;
            }
            check(&document, expect).map_err(|message| EditError {
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

fn check(document: &Document, expect: &Expect) -> Result<(), String> {
    let unit = |anchor: &str| document.units.iter().find(|unit| unit.anchor == anchor);
    match expect {
        Expect::UnitContains { anchor, needles } => {
            let unit = unit(anchor).ok_or_else(|| format!("{anchor} is missing"))?;
            match needles
                .iter()
                .find(|needle| !unit.text.contains(needle.as_str()))
            {
                Some(missing) => Err(format!("{anchor} does not contain {missing:?}")),
                None => Ok(()),
            }
        }
        Expect::UnitDeleted { anchor } => match unit(anchor) {
            None => Ok(()),
            Some(unit) => {
                let mut rest = unit.text.as_str();
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
        Expect::Title(title) => {
            if document.title.as_deref() == Some(title.as_str()) {
                Ok(())
            } else {
                Err(format!("title is {:?}", document.title))
            }
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
        created: None,
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
mod tests {
    use super::*;

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
