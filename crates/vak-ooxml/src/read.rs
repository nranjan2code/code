//! L2 read projections with anchors and O6 labels.
//!
//! A projection is data for a reader, never instructions: hidden runs,
//! tracked deletions, comments, hidden slides and sheets, off-slide shapes
//! and speaker notes are all kept, and each is labelled for what it is so a
//! prompt injection hidden in a document is visible as hidden content.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::ops::Range;

use serde::Serialize;

use crate::package::{Inspection, Package, Vocabulary};
use crate::xml::{self, Element, XmlEvent};
use crate::{Error, Limits};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Heading,
    Paragraph,
    TableRow,
    Comment,
    SheetRow,
    DefinedName,
    Slide,
    Shape,
    Notes,
    Page,
}

/// One addressable piece of a document. `anchor` is what an op or a
/// citation names (docs/design/72, "Anchors").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unit {
    pub anchor: String,
    pub kind: UnitKind,
    /// Heading level; 0 for everything that is not a heading.
    pub level: u8,
    pub text: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    pub anchor: String,
    pub title: String,
    pub level: u8,
    pub units: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Table {
    pub anchor: String,
    pub title: String,
    /// First row is the header row.
    pub rows: Vec<Vec<String>>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Document {
    pub inspection: Inspection,
    pub title: Option<String>,
    pub stats: Vec<(String, usize)>,
    pub units: Vec<Unit>,
    pub sections: Vec<Section>,
    pub tables: Vec<Table>,
    /// Parts of the vocabulary this reader does not project yet. Stated so
    /// a reader never mistakes an omission for absence.
    pub not_read: Vec<&'static str>,
}

/// Opens and projects one package.
pub fn read<R: Read + Seek>(reader: R, limits: Limits) -> Result<Document, Error> {
    let mut package = Package::open(reader, limits)?;
    project(&mut package)
}

pub fn project<R: Read + Seek>(package: &mut Package<R>) -> Result<Document, Error> {
    let inspection = package.inspect()?;
    let title = core_title(package)?;
    let mut document = Document {
        inspection,
        title,
        stats: Vec::new(),
        units: Vec::new(),
        sections: Vec::new(),
        tables: Vec::new(),
        not_read: Vec::new(),
    };
    match package.format().vocabulary {
        Vocabulary::Word => word(package, &mut document)?,
        Vocabulary::Excel => excel(package, &mut document)?,
        Vocabulary::PowerPoint => powerpoint(package, &mut document)?,
        Vocabulary::Visio => visio(package, &mut document)?,
    }
    Ok(document)
}

impl Document {
    /// The document as anchored lines: `[anchor] text  ⟨labels⟩`.
    pub fn lines(&self) -> Vec<String> {
        self.units.iter().map(render_unit).collect()
    }

    /// A section by anchor, exact title, or title fragment, in that order.
    pub fn section(&self, wanted: &str) -> Option<&Section> {
        let wanted = wanted.trim().trim_start_matches('#').trim().to_lowercase();
        self.sections
            .iter()
            .find(|section| section.anchor.to_lowercase() == wanted)
            .or_else(|| {
                self.sections
                    .iter()
                    .find(|section| section.title.trim().to_lowercase() == wanted)
            })
            .or_else(|| {
                self.sections
                    .iter()
                    .find(|section| section.title.to_lowercase().contains(&wanted))
            })
    }

    /// A table by anchor, exact title or title fragment; the first table
    /// when nothing is named.
    pub fn table(&self, wanted: Option<&str>) -> Option<&Table> {
        match wanted.map(|value| value.trim().to_lowercase()) {
            None => self.tables.first(),
            Some(wanted) => self
                .tables
                .iter()
                .find(|table| {
                    table.anchor.to_lowercase() == wanted || table.title.to_lowercase() == wanted
                })
                .or_else(|| {
                    self.tables
                        .iter()
                        .find(|table| table.title.to_lowercase().contains(&wanted))
                }),
        }
    }

    pub fn outline(&self) -> Vec<String> {
        self.sections
            .iter()
            .map(|section| {
                let indent = "  ".repeat(usize::from(section.level.saturating_sub(1)));
                format!("{indent}- [{}] {}", section.anchor, section.title)
            })
            .collect()
    }
}

fn render_unit(unit: &Unit) -> String {
    let mut line = format!("[{}] ", unit.anchor);
    if unit.kind == UnitKind::Heading {
        line.push_str(&"#".repeat(usize::from(unit.level.clamp(1, 6))));
        line.push(' ');
    }
    line.push_str(&unit.text);
    if !unit.labels.is_empty() {
        line.push_str("  ⟨");
        line.push_str(&unit.labels.join("; "));
        line.push('⟩');
    }
    line
}

fn core_title<R: Read + Seek>(package: &mut Package<R>) -> Result<Option<String>, Error> {
    let Some(part) = package
        .related_part("", "core-properties")?
        .filter(|part| package.has_part(part))
    else {
        return Ok(None);
    };
    let bytes = package.read_part(&part)?;
    let mut title = String::new();
    let mut inside = false;
    xml::walk(&bytes, &part, package.limits(), |event| {
        match event {
            XmlEvent::Open(element) if element.local() == "title" => inside = true,
            XmlEvent::Close(name) if xml::local_name(&name) == "title" => inside = false,
            XmlEvent::Text(text) if inside => title.push_str(&text),
            _ => {}
        }
        Ok(())
    })?;
    let title = title.trim().to_string();
    Ok((!title.is_empty()).then_some(title))
}

fn is_on(element: &Element) -> bool {
    !matches!(element.attr("val"), Some("0" | "false" | "off"))
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

// ---- Word ------------------------------------------------------------------

#[derive(Default)]
struct Paragraph {
    anchor: String,
    style: Option<String>,
    outline: Option<u8>,
    text: String,
    labels: Vec<String>,
    comments: Vec<String>,
    in_table: bool,
    text_box: bool,
}

#[derive(Default)]
struct Run {
    hidden: bool,
    white: bool,
}

#[derive(Default)]
struct WordTotals {
    hidden_runs: usize,
    tracked: usize,
}

fn word<R: Read + Seek>(package: &mut Package<R>, document: &mut Document) -> Result<(), Error> {
    let main = package.main_part().to_string();
    let styles = match package.related_part(&main, "styles")? {
        Some(part) => {
            let bytes = package.read_part(&part)?;
            heading_styles(&bytes, &part, package.limits())?
        }
        None => HashMap::new(),
    };
    let comments = match package.related_part(&main, "comments")? {
        Some(part) => {
            let bytes = package.read_part(&part)?;
            word_comments(&bytes, &part, package.limits())?
        }
        None => HashMap::new(),
    };
    let bytes = package.read_part(&main)?;
    let limits = *package.limits();

    let mut stack: Vec<Paragraph> = Vec::new();
    let mut run = Run::default();
    let mut in_run = false;
    let mut in_run_properties = false;
    let mut revision: Vec<(bool, String)> = Vec::new();
    let mut in_text = false;
    let mut in_instruction = false;
    let mut instructions = String::new();
    let mut table_depth = 0usize;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut paragraph_ordinal = 0usize;
    let mut table_ordinal = 0usize;
    let mut units: Vec<Unit> = Vec::new();
    let mut tables: Vec<Table> = Vec::new();
    let mut totals = WordTotals::default();
    let mut risky_fields = 0usize;
    let mut text_box_depth = 0usize;

    xml::walk(&bytes, &main, &limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "p" => {
                    let nested = !stack.is_empty();
                    let anchor = match element.attr("paraId") {
                        Some(id) => format!("p:{id}"),
                        None => {
                            paragraph_ordinal += 1;
                            format!("p@{paragraph_ordinal}")
                        }
                    };
                    stack.push(Paragraph {
                        anchor,
                        in_table: table_depth > 0,
                        text_box: nested || text_box_depth > 0,
                        ..Paragraph::default()
                    });
                }
                "txbxContent" => text_box_depth += 1,
                "pStyle" => {
                    if let (Some(paragraph), Some(value)) = (stack.last_mut(), element.attr("val"))
                    {
                        paragraph.style = Some(value.to_string());
                    }
                }
                "outlineLvl" => {
                    if let (Some(paragraph), Some(value)) = (stack.last_mut(), element.attr("val"))
                    {
                        paragraph.outline = value.parse::<u8>().ok().map(|level| level + 1);
                    }
                }
                "r" => {
                    run = Run::default();
                    in_run = true;
                }
                "rPr" => in_run_properties = true,
                "vanish" | "specVanish" if in_run_properties && is_on(&element) => {
                    run.hidden = true
                }
                "color" if in_run_properties => {
                    run.white = element
                        .attr("val")
                        .is_some_and(|value| value.eq_ignore_ascii_case("FFFFFF"));
                }
                "ins" | "moveTo" => {
                    totals.tracked += 1;
                    revision.push((true, element.attr("author").unwrap_or("unknown").into()));
                }
                "del" | "moveFrom" => {
                    totals.tracked += 1;
                    revision.push((false, element.attr("author").unwrap_or("unknown").into()));
                }
                "t" | "delText" => in_text = true,
                "instrText" => in_instruction = true,
                "fldSimple" => {
                    if let Some(instruction) = element.attr("instr") {
                        instructions.push_str(instruction);
                        instructions.push(' ');
                    }
                }
                "tab" if in_run => push_word_text(&mut stack, &run, &revision, "\t", &mut totals),
                "br" | "cr" if in_run => {
                    push_word_text(&mut stack, &run, &revision, " ", &mut totals)
                }
                "noBreakHyphen" if in_run => {
                    push_word_text(&mut stack, &run, &revision, "-", &mut totals)
                }
                "commentReference" => {
                    if let (Some(paragraph), Some(id)) = (stack.last_mut(), element.attr("id")) {
                        paragraph.comments.push(id.to_string());
                    }
                }
                "tbl" => {
                    table_depth += 1;
                    if table_depth == 1 {
                        table_rows.clear();
                    }
                }
                "tr" if table_depth == 1 => row.clear(),
                "tc" if table_depth == 1 => cell.clear(),
                _ => {}
            },
            XmlEvent::Text(text) => {
                if in_text {
                    push_word_text(&mut stack, &run, &revision, &text, &mut totals);
                } else if in_instruction {
                    instructions.push_str(&text);
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "t" | "delText" => in_text = false,
                "instrText" => {
                    in_instruction = false;
                    instructions.push(' ');
                }
                "rPr" => in_run_properties = false,
                "r" => in_run = false,
                "ins" | "del" | "moveTo" | "moveFrom" => {
                    revision.pop();
                }
                "txbxContent" => text_box_depth = text_box_depth.saturating_sub(1),
                "p" => {
                    let Some(mut paragraph) = stack.pop() else {
                        return Ok(());
                    };
                    if let Some(field) = risky_field(&instructions) {
                        paragraph
                            .labels
                            .push(format!("{field} field (never executed)"));
                        risky_fields += 1;
                    }
                    instructions.clear();
                    let text = paragraph.text.trim().to_string();
                    let level = heading_level(&paragraph, &styles);
                    let in_cell = paragraph.in_table && !paragraph.text_box;
                    if in_cell {
                        if !cell.is_empty() && !text.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(&text);
                    } else if !text.is_empty() || !paragraph.labels.is_empty() {
                        let mut labels = paragraph.labels.clone();
                        if paragraph.text_box {
                            labels.push("text box".into());
                        }
                        units.push(Unit {
                            anchor: paragraph.anchor.clone(),
                            kind: if level > 0 {
                                UnitKind::Heading
                            } else {
                                UnitKind::Paragraph
                            },
                            level,
                            text,
                            labels,
                        });
                    }
                    attach_comments(&mut units, &paragraph, &comments);
                }
                "tc" if table_depth == 1 => row.push(std::mem::take(&mut cell)),
                "tr" if table_depth == 1 => table_rows.push(std::mem::take(&mut row)),
                "tbl" => {
                    table_depth = table_depth.saturating_sub(1);
                    if table_depth == 0 {
                        table_ordinal += 1;
                        let anchor = format!("tbl@{table_ordinal}");
                        for (index, cells) in table_rows.iter().enumerate() {
                            units.push(Unit {
                                anchor: format!("{anchor}/r{}", index + 1),
                                kind: UnitKind::TableRow,
                                level: 0,
                                text: cells.join(" | "),
                                labels: Vec::new(),
                            });
                        }
                        tables.push(Table {
                            anchor,
                            title: format!("Table {table_ordinal}"),
                            rows: std::mem::take(&mut table_rows),
                            labels: Vec::new(),
                        });
                    }
                }
                _ => {}
            },
        }
        Ok(())
    })?;

    let mut sections = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        if unit.kind != UnitKind::Heading {
            continue;
        }
        let end = units[index + 1..]
            .iter()
            .position(|next| next.kind == UnitKind::Heading && next.level <= unit.level)
            .map(|offset| index + 1 + offset)
            .unwrap_or(units.len());
        sections.push(Section {
            anchor: unit.anchor.clone(),
            title: unit.text.clone(),
            level: unit.level,
            units: index..end,
        });
    }
    let paragraphs = units
        .iter()
        .filter(|unit| matches!(unit.kind, UnitKind::Paragraph | UnitKind::Heading))
        .count();
    let word_count = units
        .iter()
        .filter(|unit| unit.kind != UnitKind::Comment)
        .map(|unit| words(&unit.text))
        .sum();
    document.stats = vec![
        ("paragraphs".into(), paragraphs),
        ("words".into(), word_count),
        ("headings".into(), sections.len()),
        ("tables".into(), tables.len()),
        ("comments".into(), comments.len()),
        ("tracked changes".into(), totals.tracked),
        ("hidden runs".into(), totals.hidden_runs),
        ("risky fields".into(), risky_fields),
    ];
    document.units = units;
    document.sections = sections;
    document.tables = tables;
    document.not_read = vec![
        "headers and footers",
        "footnotes and endnotes",
        "images and charts",
    ];
    Ok(())
}

fn push_word_text(
    stack: &mut [Paragraph],
    run: &Run,
    revision: &[(bool, String)],
    text: &str,
    totals: &mut WordTotals,
) {
    let Some(paragraph) = stack.last_mut() else {
        return;
    };
    let marked = match revision.last() {
        Some((true, author)) => format!("[inserted by {author}: {text}]"),
        Some((false, author)) => format!("[deleted by {author}: {text}]"),
        None => text.to_string(),
    };
    if run.hidden {
        totals.hidden_runs += 1;
        paragraph.text.push_str(&format!("[hidden: {marked}]"));
        add_label(&mut paragraph.labels, "hidden text");
    } else if run.white && !text.trim().is_empty() {
        paragraph.text.push_str(&format!("[white text: {marked}]"));
        add_label(&mut paragraph.labels, "white text");
    } else {
        paragraph.text.push_str(&marked);
    }
}

fn add_label(labels: &mut Vec<String>, label: &str) {
    if !labels.iter().any(|existing| existing == label) {
        labels.push(label.to_string());
    }
}

fn attach_comments(
    units: &mut Vec<Unit>,
    paragraph: &Paragraph,
    comments: &HashMap<String, (String, String)>,
) {
    for id in &paragraph.comments {
        if let Some((author, text)) = comments.get(id) {
            units.push(Unit {
                anchor: format!("{}/comment:{id}", paragraph.anchor),
                kind: UnitKind::Comment,
                level: 0,
                text: text.clone(),
                labels: vec![format!("comment by {author}")],
            });
        }
    }
}

fn risky_field(instructions: &str) -> Option<&'static str> {
    let upper = instructions.to_ascii_uppercase();
    match upper.split_whitespace().next()? {
        "DDE" | "DDEAUTO" => Some("DDE"),
        "INCLUDETEXT" => Some("INCLUDETEXT"),
        "INCLUDEPICTURE" => Some("INCLUDEPICTURE"),
        "MACROBUTTON" => Some("MACROBUTTON"),
        _ => None,
    }
}

fn heading_level(paragraph: &Paragraph, styles: &HashMap<String, u8>) -> u8 {
    if let Some(level) = paragraph.outline
        && level <= 9
    {
        return level;
    }
    paragraph
        .style
        .as_ref()
        .and_then(|style| styles.get(style).copied())
        .unwrap_or(0)
}

/// Style id → heading level, from built-in style names (`heading 1`,
/// `Title`), which stay English whatever the document's language, or from
/// an explicit outline level on the style.
fn heading_styles(bytes: &[u8], part: &str, limits: &Limits) -> Result<HashMap<String, u8>, Error> {
    let mut levels = HashMap::new();
    let mut current: Option<String> = None;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "style" => current = element.attr("styleId").map(str::to_string),
                "name" => {
                    if let (Some(id), Some(name)) = (&current, element.attr("val")) {
                        let name = name.to_ascii_lowercase();
                        if name == "title" {
                            levels.insert(id.clone(), 1);
                        } else if let Some(level) = name
                            .strip_prefix("heading ")
                            .and_then(|level| level.parse::<u8>().ok())
                        {
                            levels.insert(id.clone(), level);
                        }
                    }
                }
                "outlineLvl" => {
                    if let (Some(id), Some(level)) = (&current, element.attr("val"))
                        && let Ok(level) = level.parse::<u8>()
                        && level < 9
                    {
                        levels.entry(id.clone()).or_insert(level + 1);
                    }
                }
                _ => {}
            },
            XmlEvent::Close(name) if xml::local_name(&name) == "style" => current = None,
            _ => {}
        }
        Ok(())
    })?;
    Ok(levels)
}

fn word_comments(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
) -> Result<HashMap<String, (String, String)>, Error> {
    let mut comments = HashMap::new();
    let mut current: Option<(String, String)> = None;
    let mut text = String::new();
    let mut in_text = false;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "comment" => {
                    current = element.attr("id").map(|id| {
                        (
                            id.to_string(),
                            element.attr("author").unwrap_or("unknown").to_string(),
                        )
                    });
                    text.clear();
                }
                "t" => in_text = true,
                "p" if !text.is_empty() => text.push(' '),
                _ => {}
            },
            XmlEvent::Text(value) if in_text => text.push_str(&value),
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "t" => in_text = false,
                "comment" => {
                    if let Some((id, author)) = current.take() {
                        comments.insert(id, (author, text.trim().to_string()));
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok(comments)
}

// ---- Excel -----------------------------------------------------------------

fn excel<R: Read + Seek>(package: &mut Package<R>, document: &mut Document) -> Result<(), Error> {
    let main = package.main_part().to_string();
    let limits = *package.limits();
    let bytes = package.read_part(&main)?;
    let mut sheets: Vec<(String, Option<String>, String)> = Vec::new();
    let mut defined_names: Vec<(String, String)> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut name_text = String::new();
    xml::walk(&bytes, &main, &limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "sheet" => {
                    if let (Some(name), Some(id)) =
                        (element.attr("name"), element.attr_prefixed("id"))
                    {
                        sheets.push((
                            name.to_string(),
                            element.attr("state").map(str::to_string),
                            id.to_string(),
                        ));
                    }
                }
                "definedName" => {
                    current_name = element.attr("name").map(str::to_string);
                    name_text.clear();
                }
                _ => {}
            },
            XmlEvent::Text(text) if current_name.is_some() => name_text.push_str(&text),
            XmlEvent::Close(name) if xml::local_name(&name) == "definedName" => {
                if let Some(name) = current_name.take() {
                    defined_names.push((name, name_text.trim().to_string()));
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    let shared = match package.related_part(&main, "sharedStrings")? {
        Some(part) => {
            let bytes = package.read_part(&part)?;
            shared_strings(&bytes, &part, &limits)?
        }
        None => Vec::new(),
    };

    let mut units = Vec::new();
    let mut sections = Vec::new();
    let mut tables = Vec::new();
    let mut total_rows = 0usize;
    let mut formulas = 0usize;
    let sheet_count = sheets.len();
    for (name, state, relationship) in sheets {
        let quoted = quote_sheet(&name);
        let anchor = format!("{quoted}!");
        let mut labels = Vec::new();
        match state.as_deref() {
            Some("hidden") => labels.push("hidden sheet".to_string()),
            Some("veryHidden") => labels.push("very hidden sheet".to_string()),
            _ => {}
        }
        let start = units.len();
        let Some(part) = package.part_by_relationship_id(&main, &relationship)? else {
            continue;
        };
        let is_worksheet = package
            .content_type(&part)
            .is_some_and(|content_type| content_type.contains("worksheet"));
        if !is_worksheet {
            sections.push(Section {
                anchor,
                title: format!("{name} (chart or dialog sheet, not read)"),
                level: 1,
                units: start..start,
            });
            continue;
        }
        let bytes = package.read_part(&part)?;
        let rows = sheet_rows(&bytes, &part, &limits, &shared, &mut formulas)?;
        let max_column = rows
            .iter()
            .flat_map(|row| row.cells.iter().map(|(column, _)| *column))
            .max()
            .unwrap_or(0);
        let mut table_rows = vec![
            std::iter::once(String::new())
                .chain((1..=max_column).map(column_name))
                .collect::<Vec<_>>(),
        ];
        for row in &rows {
            total_rows += 1;
            let mut dense = vec![String::new(); max_column as usize];
            for (column, value) in &row.cells {
                if let Some(slot) = dense.get_mut(*column as usize - 1) {
                    *slot = value.clone();
                }
            }
            let first = row.cells.first().map(|(column, _)| *column).unwrap_or(1);
            let last = row.cells.last().map(|(column, _)| *column).unwrap_or(1);
            let mut row_labels = labels.clone();
            if row.hidden {
                row_labels.push("hidden row".into());
            }
            units.push(Unit {
                anchor: format!(
                    "{quoted}!{}{}:{}{}",
                    column_name(first),
                    row.number,
                    column_name(last),
                    row.number
                ),
                kind: UnitKind::SheetRow,
                level: 0,
                text: row
                    .cells
                    .iter()
                    .map(|(column, value)| {
                        format!("{}{}: {value}", column_name(*column), row.number)
                    })
                    .collect::<Vec<_>>()
                    .join(" | "),
                labels: row_labels,
            });
            table_rows.push(
                std::iter::once(row.number.to_string())
                    .chain(dense)
                    .collect(),
            );
        }
        sections.push(Section {
            anchor: anchor.clone(),
            title: name.clone(),
            level: 1,
            units: start..units.len(),
        });
        tables.push(Table {
            anchor,
            title: name,
            rows: table_rows,
            labels,
        });
    }
    document.stats = vec![
        ("sheets".into(), sheet_count),
        ("rows".into(), total_rows),
        ("formulas".into(), formulas),
        ("defined names".into(), defined_names.len()),
    ];
    for (name, reference) in defined_names {
        units.push(Unit {
            anchor: name.clone(),
            kind: UnitKind::DefinedName,
            level: 0,
            text: format!("defined name {name} = {reference}"),
            labels: Vec::new(),
        });
    }
    document.units = units;
    document.sections = sections;
    document.tables = tables;
    document.not_read = vec![
        "number formats (values are shown as stored)",
        "charts, pivot tables and cell comments",
        "recalculation (a formula shows the value cached in the file, which may be stale)",
    ];
    Ok(())
}

struct SheetRow {
    number: u32,
    hidden: bool,
    cells: Vec<(u32, String)>,
}

#[derive(Default)]
struct CellState {
    column: u32,
    kind: Option<String>,
    value: String,
    formula: Option<String>,
}

fn sheet_rows(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
    shared: &[String],
    formulas: &mut usize,
) -> Result<Vec<SheetRow>, Error> {
    let mut rows: Vec<SheetRow> = Vec::new();
    let mut cell: Option<CellState> = None;
    let mut in_value = false;
    let mut in_formula = false;
    let mut in_inline = false;
    let mut next_row = 1u32;
    let mut next_column = 1u32;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "row" => {
                    let number = element
                        .attr("r")
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(next_row);
                    next_row = number.saturating_add(1);
                    next_column = 1;
                    rows.push(SheetRow {
                        number,
                        hidden: element
                            .attr("hidden")
                            .is_some_and(|value| value == "1" || value == "true"),
                        cells: Vec::new(),
                    });
                }
                "c" => {
                    let column = element.attr("r").and_then(column_of).unwrap_or(next_column);
                    next_column = column.saturating_add(1);
                    cell = Some(CellState {
                        column,
                        kind: element.attr("t").map(str::to_string),
                        ..CellState::default()
                    });
                }
                "v" => in_value = true,
                "f" => {
                    in_formula = true;
                    if let Some(cell) = cell.as_mut() {
                        cell.formula.get_or_insert_with(String::new);
                    }
                }
                "is" => in_inline = true,
                _ => {}
            },
            XmlEvent::Text(text) => {
                if let Some(cell) = cell.as_mut() {
                    if in_formula {
                        if let Some(formula) = cell.formula.as_mut() {
                            formula.push_str(&text);
                        }
                    } else if in_value || in_inline {
                        cell.value.push_str(&text);
                    }
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "v" => in_value = false,
                "f" => in_formula = false,
                "is" => in_inline = false,
                "c" => {
                    if let Some(state) = cell.take() {
                        let shown = match state.kind.as_deref() {
                            Some("s") => state
                                .value
                                .trim()
                                .parse::<usize>()
                                .ok()
                                .and_then(|index| shared.get(index).cloned())
                                .unwrap_or_default(),
                            Some("b") if state.value.trim() == "1" => "TRUE".into(),
                            Some("b") => "FALSE".into(),
                            Some("e") => format!("#ERROR {}", state.value.trim()),
                            _ => state.value.clone(),
                        };
                        let rendered = match state.formula {
                            Some(formula) => {
                                *formulas += 1;
                                if formula.trim().is_empty() {
                                    format!("(shared formula) [cached: {shown}]")
                                } else {
                                    format!("={} [cached: {shown}]", formula.trim())
                                }
                            }
                            None => shown,
                        };
                        if !rendered.is_empty()
                            && let Some(row) = rows.last_mut()
                        {
                            row.cells.push((state.column, rendered));
                        }
                    }
                }
                _ => {}
            },
        }
        Ok(())
    })?;
    rows.retain(|row| !row.cells.is_empty());
    Ok(rows)
}

fn shared_strings(bytes: &[u8], part: &str, limits: &Limits) -> Result<Vec<String>, Error> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    let mut phonetic = 0usize;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "si" => current.clear(),
                "rPh" => phonetic += 1,
                "t" if phonetic == 0 => in_text = true,
                _ => {}
            },
            XmlEvent::Text(text) if in_text => current.push_str(&text),
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "t" => in_text = false,
                "rPh" => phonetic = phonetic.saturating_sub(1),
                "si" => strings.push(std::mem::take(&mut current)),
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok(strings)
}

fn column_of(reference: &str) -> Option<u32> {
    let letters: String = reference
        .chars()
        .take_while(|character| character.is_ascii_alphabetic())
        .collect();
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    letters.chars().try_fold(0u32, |total, character| {
        Some(total * 26 + (character.to_ascii_uppercase() as u32 - 'A' as u32 + 1))
    })
}

/// `1` → `A`, `27` → `AA`.
pub fn column_name(mut column: u32) -> String {
    let mut name = Vec::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        name.push(char::from(b'A' + remainder as u8));
        column = (column - 1) / 26;
    }
    name.iter().rev().collect()
}

fn quote_sheet(name: &str) -> String {
    if name
        .chars()
        .all(|character| character.is_alphanumeric() || character == '_' || character == '.')
    {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

// ---- PowerPoint ------------------------------------------------------------

#[derive(Default, Clone)]
struct Shape {
    id: String,
    name: String,
    hidden: bool,
    placeholder: Option<String>,
    offset: Option<(i64, i64)>,
    extent: Option<(i64, i64)>,
    paragraphs: Vec<String>,
    table: Vec<Vec<String>>,
}

fn powerpoint<R: Read + Seek>(
    package: &mut Package<R>,
    document: &mut Document,
) -> Result<(), Error> {
    let main = package.main_part().to_string();
    let limits = *package.limits();
    let bytes = package.read_part(&main)?;
    let mut slide_ids: Vec<(String, String)> = Vec::new();
    let mut size: Option<(i64, i64)> = None;
    xml::walk(&bytes, &main, &limits, |event| {
        if let XmlEvent::Open(element) = event {
            match element.local() {
                "sldId" => {
                    if let (Some(id), Some(relationship)) =
                        (element.attr_unprefixed("id"), element.attr_prefixed("id"))
                    {
                        slide_ids.push((id.to_string(), relationship.to_string()));
                    }
                }
                "sldSz" => {
                    size = element
                        .attr("cx")
                        .and_then(|cx| cx.parse().ok())
                        .zip(element.attr("cy").and_then(|cy| cy.parse().ok()));
                }
                _ => {}
            }
        }
        Ok(())
    })?;

    let mut units = Vec::new();
    let mut sections = Vec::new();
    let mut tables = Vec::new();
    let mut hidden_slides = 0usize;
    let mut with_notes = 0usize;
    for (index, (slide_id, relationship)) in slide_ids.iter().enumerate() {
        let number = index + 1;
        let Some(part) = package.part_by_relationship_id(&main, relationship)? else {
            continue;
        };
        let bytes = package.read_part(&part)?;
        let (hidden, shapes) = slide_shapes(&bytes, &part, &limits)?;
        let anchor = format!("slide:{slide_id}");
        let title = shapes
            .iter()
            .find(|shape| matches!(shape.placeholder.as_deref(), Some("title" | "ctrTitle")))
            .map(|shape| shape.paragraphs.join(" "))
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Slide {number}"));
        let start = units.len();
        let mut slide_labels = Vec::new();
        if hidden {
            hidden_slides += 1;
            slide_labels.push("hidden slide".to_string());
        }
        units.push(Unit {
            anchor: anchor.clone(),
            kind: UnitKind::Slide,
            level: 1,
            text: format!("Slide {number}: {title}"),
            labels: slide_labels.clone(),
        });
        for shape in &shapes {
            let mut labels = slide_labels.clone();
            if shape.hidden {
                labels.push("hidden shape".into());
            }
            if let (Some((x, y)), Some((width, height)), Some((slide_width, slide_height))) =
                (shape.offset, shape.extent, size)
                && (x >= slide_width || y >= slide_height || x + width <= 0 || y + height <= 0)
            {
                labels.push("off-slide, not visible when presented".into());
            }
            let text = shape.paragraphs.join(" / ");
            if !text.trim().is_empty() {
                units.push(Unit {
                    anchor: format!("{anchor}/shape:{}", shape.id),
                    kind: UnitKind::Shape,
                    level: 0,
                    text,
                    labels: labels.clone(),
                });
            }
            if !shape.table.is_empty() {
                tables.push(Table {
                    anchor: format!("{anchor}/shape:{}", shape.id),
                    title: format!("Slide {number} · {}", shape.name),
                    rows: shape.table.clone(),
                    labels,
                });
            }
        }
        if let Some(notes_part) = package.related_part(&part, "notesSlide")? {
            let bytes = package.read_part(&notes_part)?;
            let (_, note_shapes) = slide_shapes(&bytes, &notes_part, &limits)?;
            let notes: Vec<String> = note_shapes
                .iter()
                .filter(|shape| shape.placeholder.as_deref() == Some("body"))
                .map(|shape| shape.paragraphs.join(" / "))
                .filter(|text| !text.trim().is_empty())
                .collect();
            if !notes.is_empty() {
                with_notes += 1;
                units.push(Unit {
                    anchor: format!("{anchor}/notes"),
                    kind: UnitKind::Notes,
                    level: 0,
                    text: notes.join(" / "),
                    labels: vec!["speaker notes".into()],
                });
            }
        }
        sections.push(Section {
            anchor,
            title: format!("Slide {number}: {title}"),
            level: 1,
            units: start..units.len(),
        });
    }
    document.stats = vec![
        ("slides".into(), slide_ids.len()),
        ("hidden slides".into(), hidden_slides),
        ("slides with notes".into(), with_notes),
        ("tables".into(), tables.len()),
    ];
    document.units = units;
    document.sections = sections;
    document.tables = tables;
    document.not_read = vec![
        "comments",
        "chart data and SmartArt text",
        "text inherited from layouts and masters",
    ];
    Ok(())
}

fn slide_shapes(bytes: &[u8], part: &str, limits: &Limits) -> Result<(bool, Vec<Shape>), Error> {
    let mut hidden_slide = false;
    let mut shapes: Vec<Shape> = Vec::new();
    let mut stack: Vec<Shape> = Vec::new();
    let mut group_depth = 0usize;
    let mut paragraph: Option<String> = None;
    let mut in_text = false;
    let mut row: Option<Vec<String>> = None;
    let mut cell: Option<String> = None;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "sld" => {
                    hidden_slide = element
                        .attr("show")
                        .is_some_and(|value| value == "0" || value == "false")
                }
                "grpSp" => group_depth += 1,
                "sp" | "pic" | "graphicFrame" | "cxnSp" => stack.push(Shape::default()),
                "cNvPr" => {
                    if let Some(shape) = stack.last_mut()
                        && shape.id.is_empty()
                    {
                        shape.id = element.attr("id").unwrap_or_default().to_string();
                        shape.name = element.attr("name").unwrap_or_default().to_string();
                        shape.hidden = element
                            .attr("hidden")
                            .is_some_and(|value| value == "1" || value == "true");
                    }
                }
                "ph" => {
                    if let Some(shape) = stack.last_mut() {
                        shape.placeholder =
                            Some(element.attr("type").unwrap_or("body").to_string());
                    }
                }
                "off" if group_depth == 0 => {
                    if let Some(shape) = stack.last_mut()
                        && shape.offset.is_none()
                    {
                        shape.offset = element
                            .attr("x")
                            .and_then(|x| x.parse().ok())
                            .zip(element.attr("y").and_then(|y| y.parse().ok()));
                    }
                }
                "ext" if group_depth == 0 => {
                    if let Some(shape) = stack.last_mut()
                        && shape.extent.is_none()
                        && element.attr("cx").is_some()
                    {
                        shape.extent = element
                            .attr("cx")
                            .and_then(|x| x.parse().ok())
                            .zip(element.attr("cy").and_then(|y| y.parse().ok()));
                    }
                }
                "p" => paragraph = Some(String::new()),
                "t" => in_text = true,
                "br" => {
                    if let Some(paragraph) = paragraph.as_mut() {
                        paragraph.push(' ');
                    }
                }
                "tr" => row = Some(Vec::new()),
                "tc" => cell = Some(String::new()),
                _ => {}
            },
            XmlEvent::Text(text) if in_text => {
                if let Some(paragraph) = paragraph.as_mut() {
                    paragraph.push_str(&text);
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "t" => in_text = false,
                "p" => {
                    if let Some(text) = paragraph.take() {
                        let text = text.trim().to_string();
                        if let Some(cell) = cell.as_mut() {
                            if !cell.is_empty() && !text.is_empty() {
                                cell.push(' ');
                            }
                            cell.push_str(&text);
                        } else if !text.is_empty()
                            && let Some(shape) = stack.last_mut()
                        {
                            shape.paragraphs.push(text);
                        }
                    }
                }
                "tc" => {
                    if let (Some(text), Some(row)) = (cell.take(), row.as_mut()) {
                        row.push(text);
                    }
                }
                "tr" => {
                    if let (Some(cells), Some(shape)) = (row.take(), stack.last_mut()) {
                        shape.table.push(cells);
                    }
                }
                "sp" | "pic" | "graphicFrame" | "cxnSp" => {
                    if let Some(shape) = stack.pop() {
                        shapes.push(shape);
                    }
                }
                "grpSp" => group_depth = group_depth.saturating_sub(1),
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok((hidden_slide, shapes))
}

// ---- Visio -----------------------------------------------------------------

struct VisioPage {
    id: String,
    name: String,
    background: bool,
    relationship: Option<String>,
}

fn visio<R: Read + Seek>(package: &mut Package<R>, document: &mut Document) -> Result<(), Error> {
    let main = package.main_part().to_string();
    let limits = *package.limits();
    let Some(pages_part) = package.related_part(&main, "pages")? else {
        document.not_read = vec!["pages (the drawing has no pages part)"];
        return Ok(());
    };
    let bytes = package.read_part(&pages_part)?;
    let mut pages: Vec<VisioPage> = Vec::new();
    xml::walk(&bytes, &pages_part, &limits, |event| {
        if let XmlEvent::Open(element) = event {
            match element.local() {
                "Page" => pages.push(VisioPage {
                    id: element.attr("ID").unwrap_or_default().to_string(),
                    name: element
                        .attr("Name")
                        .or_else(|| element.attr("NameU"))
                        .unwrap_or_default()
                        .to_string(),
                    background: element.attr("Background").is_some_and(|value| value == "1"),
                    relationship: None,
                }),
                "Rel" => {
                    if let (Some(page), Some(id)) = (pages.last_mut(), element.attr_prefixed("id"))
                    {
                        page.relationship = Some(id.to_string());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })?;
    let mut units = Vec::new();
    let mut sections = Vec::new();
    let mut shape_count = 0usize;
    for page in pages {
        let anchor = format!("page:{}", page.id);
        let start = units.len();
        let labels = if page.background {
            vec!["background page".to_string()]
        } else {
            Vec::new()
        };
        units.push(Unit {
            anchor: anchor.clone(),
            kind: UnitKind::Page,
            level: 1,
            text: format!("Page: {}", page.name),
            labels: labels.clone(),
        });
        if let Some(relationship) = page.relationship
            && let Some(part) = package.part_by_relationship_id(&pages_part, &relationship)?
        {
            let bytes = package.read_part(&part)?;
            for (shape_id, shape_name, text) in visio_shapes(&bytes, &part, &limits)? {
                shape_count += 1;
                if text.is_empty() {
                    continue;
                }
                units.push(Unit {
                    anchor: format!("{anchor}/shape:{shape_id}"),
                    kind: UnitKind::Shape,
                    level: 0,
                    text: if shape_name.is_empty() {
                        text
                    } else {
                        format!("{shape_name}: {text}")
                    },
                    labels: labels.clone(),
                });
            }
        }
        sections.push(Section {
            anchor,
            title: page.name,
            level: 1,
            units: start..units.len(),
        });
    }
    document.stats = vec![
        ("pages".into(), sections.len()),
        ("shapes".into(), shape_count),
    ];
    document.units = units;
    document.sections = sections;
    document.not_read = vec![
        "text and geometry inherited from masters",
        "shape data (properties) and connectors",
    ];
    Ok(())
}

fn visio_shapes(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
) -> Result<Vec<(String, String, String)>, Error> {
    let mut shapes = Vec::new();
    let mut stack: Vec<(String, String, String)> = Vec::new();
    let mut in_text = 0usize;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "Shape" => stack.push((
                    element.attr("ID").unwrap_or_default().to_string(),
                    element
                        .attr("Name")
                        .or_else(|| element.attr("NameU"))
                        .unwrap_or_default()
                        .to_string(),
                    String::new(),
                )),
                "Text" => in_text += 1,
                _ => {}
            },
            XmlEvent::Text(text) if in_text > 0 => {
                if let Some(shape) = stack.last_mut() {
                    shape.2.push_str(&text);
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "Text" => in_text = in_text.saturating_sub(1),
                "Shape" => {
                    if let Some((id, name, text)) = stack.pop() {
                        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        shapes.push((id, name, text));
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_names_round_trip() {
        for (column, name) in [(1, "A"), (26, "Z"), (27, "AA"), (52, "AZ"), (703, "AAA")] {
            assert_eq!(column_name(column), name);
            assert_eq!(column_of(&format!("{name}12")), Some(column));
        }
    }

    #[test]
    fn sheet_names_are_quoted_when_needed() {
        assert_eq!(quote_sheet("Budget"), "Budget");
        assert_eq!(quote_sheet("Q1 plan"), "'Q1 plan'");
        assert_eq!(quote_sheet("Bob's"), "'Bob''s'");
    }
}
