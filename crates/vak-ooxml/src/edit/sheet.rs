//! Excel ops. Vak does not calculate (deferred): a changed input or formula
//! sets `fullCalcOnLoad`, so Excel recalculates on open, and the reader
//! reports every cached formula value as stale until then.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Seek};

use super::{CellValue, EditError, Expect, Outcome, Work, fail, free_part_name};
use crate::read::column_name;
use crate::splice::{Splice, Tree, escape_attr, escape_text, start_tag};

const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const S_STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const R_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const WORKSHEET_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const DRAWING_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
const CHART_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
const R_DRAWING: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
const R_CHART: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
const R_TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
const R_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const TABLE_TYPE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";
const DRAWING_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MAX_ROW: u32 = 1_048_576;
const MAX_COLUMN: u32 = 16_384;

fn prefix(tree: &Tree) -> Result<String, EditError> {
    match tree.prefix_for(S).or_else(|| tree.prefix_for(S_STRICT)) {
        Some(prefix) => Ok(prefix),
        None => fail("the part does not declare the SpreadsheetML namespace on its root"),
    }
}

/// Add a drawing relationship and insert its reference in worksheet order.
fn add_sheet_drawing<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet_part: &str,
    drawing_part: &str,
) -> Result<(), EditError> {
    let drawing_rel = work.add_relationship(sheet_part, R_DRAWING, drawing_part)?;
    let sheet_bytes = work.get(sheet_part)?;
    let tree = Tree::parse(&sheet_bytes, sheet_part, work.limits())?;
    let s = prefix(&tree)?;
    // Keep the relationship namespace local to the drawing reference. Some
    // spreadsheet consumers fail to resolve an r:id inherited from the
    // worksheet root even though that XML is namespace-equivalent.
    let drawing_tag = format!(r#"<{s}drawing xmlns:r="{R_NS}" r:id="{drawing_rel}"/>"#);
    // SpreadsheetML fixes the order of these worksheet children. Drawings
    // belong after page setup, and before legacy drawings and table parts.
    let late = [
        "legacyDrawing",
        "legacyDrawingHF",
        "picture",
        "oleObjects",
        "controls",
        "webPublishItems",
        "tableParts",
        "extLst",
    ];
    let before = late
        .iter()
        .flat_map(|name| tree.children(0, name))
        .min_by_key(|node| tree.nodes[*node].span.start);
    let position = before
        .map(|node| tree.nodes[node].span.start)
        .unwrap_or(tree.root().inner.end);
    let mut splice = Splice::default();
    splice.insert(position, drawing_tag);
    work.put(sheet_part, splice.apply(&sheet_bytes, sheet_part)?);
    Ok(())
}

/// `B4` or `$B$4` → (column, row), both 1-based.
fn address(text: &str) -> Result<(u32, u32), EditError> {
    let cleaned = text.trim().replace('$', "").to_ascii_uppercase();
    let split = cleaned
        .find(|character: char| character.is_ascii_digit())
        .unwrap_or(cleaned.len());
    let (letters, digits) = cleaned.split_at(split);
    let column = if letters.is_empty()
        || letters.len() > 3
        || !letters.chars().all(|c| c.is_ascii_uppercase())
    {
        None
    } else {
        letters.chars().try_fold(0u32, |total, character| {
            Some(total * 26 + (character as u32 - 'A' as u32 + 1))
        })
    };
    let row = digits.parse::<u32>().ok();
    match (column, row) {
        (Some(column), Some(row))
            if (1..=MAX_COLUMN).contains(&column) && (1..=MAX_ROW).contains(&row) =>
        {
            Ok((column, row))
        }
        _ => fail(format!("{text:?} is not a cell address like B4")),
    }
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

struct Book {
    part: String,
    bytes: Vec<u8>,
    tree: Tree,
    s: String,
}

fn book<R2: Read + Seek>(work: &mut Work<'_, R2>) -> Result<Book, EditError> {
    let part = work.main_part();
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let s = prefix(&tree)?;
    Ok(Book {
        part,
        bytes,
        tree,
        s,
    })
}

/// The sheet's part and its name as the workbook spells it.
fn sheet_part<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    wanted: &str,
) -> Result<(String, String), EditError> {
    let book = book(work)?;
    let mut names = Vec::new();
    for sheet in book.tree.descendants(0, "sheet") {
        let element = &book.tree.nodes[sheet].element;
        let Some(name) = element.attr("name") else {
            continue;
        };
        if name.eq_ignore_ascii_case(wanted.trim().trim_matches('\'')) {
            let Some(id) = element.attr_prefixed("id") else {
                return fail(format!("sheet {name:?} has no relationship id"));
            };
            let main = book.part.clone();
            return match work.by_id(&main, id)? {
                Some(part) => Ok((part, name.to_string())),
                None => fail(format!("sheet {name:?} points at no part")),
            };
        }
        names.push(name.to_string());
    }
    fail(format!("no sheet {wanted:?}; sheets: {}", names.join(", ")))
}

fn cell_xml(
    s: &str,
    reference: &str,
    value: &CellValue,
    style: Option<&str>,
) -> Result<(String, bool), EditError> {
    let style = style
        .map(|style| format!(r#" s="{}""#, escape_attr(style)))
        .unwrap_or_default();
    Ok(match value {
        CellValue::Number(number) => {
            if !number.is_finite() {
                return fail(format!("{reference}: {number} is not a finite number"));
            }
            (
                format!(r#"<{s}c r="{reference}"{style}><{s}v>{number}</{s}v></{s}c>"#),
                false,
            )
        }
        CellValue::Bool(value) => (
            format!(
                r#"<{s}c r="{reference}"{style} t="b"><{s}v>{}</{s}v></{s}c>"#,
                u8::from(*value)
            ),
            false,
        ),
        CellValue::Text(text) => match text.strip_prefix('=') {
            Some(formula) if !formula.trim().is_empty() => (
                format!(
                    r#"<{s}c r="{reference}"{style}><{s}f>{}</{s}f></{s}c>"#,
                    escape_text(formula.trim())
                ),
                true,
            ),
            _ => {
                let literal = text.strip_prefix('\'').unwrap_or(text);
                (
                    format!(
                        r#"<{s}c r="{reference}"{style} t="inlineStr"><{s}is><{s}t xml:space="preserve">{}</{s}t></{s}is></{s}c>"#,
                        escape_text(literal)
                    ),
                    false,
                )
            }
        },
    })
}

/// What the reader will show for a written value.
fn shown(value: &CellValue) -> String {
    match value {
        CellValue::Number(number) => number.to_string(),
        CellValue::Bool(true) => "TRUE".into(),
        CellValue::Bool(false) => "FALSE".into(),
        CellValue::Text(text) => match text.strip_prefix('=') {
            Some(formula) if !formula.trim().is_empty() => format!("={} [", formula.trim()),
            _ => text.strip_prefix('\'').unwrap_or(text).to_string(),
        },
    }
}

pub(super) fn set_cells<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet: &str,
    cells: &BTreeMap<String, CellValue>,
) -> Result<Outcome, EditError> {
    if cells.is_empty() {
        return fail("cells is empty; give at least one address, e.g. {\"B4\": 120}");
    }
    let (part, name) = sheet_part(work, sheet)?;
    let mut targets: BTreeMap<u32, BTreeMap<u32, (&String, &CellValue)>> = BTreeMap::new();
    for (reference, value) in cells {
        let (column, row) = address(reference)?;
        if targets
            .entry(row)
            .or_default()
            .insert(column, (reference, value))
            .is_some()
        {
            return fail(format!("{reference} is given twice"));
        }
    }
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let s = prefix(&tree)?;
    if let Some(protection) = tree.descendants(0, "sheetProtection").next()
        && tree.nodes[protection]
            .element
            .attr("sheet")
            .is_some_and(|value| value == "1" || value == "true")
    {
        return fail(format!(
            "sheet {name:?} is protected; the owner must remove the protection before it can be edited"
        ));
    }
    let Some(data) = tree.descendants(0, "sheetData").next() else {
        return fail(format!("sheet {name:?} has no sheetData"));
    };
    let mut rows: BTreeMap<u32, usize> = BTreeMap::new();
    for row in tree.children(data, "row") {
        let Some(number) = tree.nodes[row]
            .element
            .attr("r")
            .and_then(|r| r.parse::<u32>().ok())
        else {
            return fail(format!(
                "sheet {name:?} has a row without a number, which this op does not edit"
            ));
        };
        rows.insert(number, row);
    }

    let mut splice = Splice::default();
    let mut formulas_touched = false;
    let mut new_rows: Vec<(u32, String)> = Vec::new();
    for (row_number, columns) in &targets {
        let mut built = Vec::new();
        for (column, (_, value)) in columns {
            let reference = format!("{}{row_number}", column_name(*column));
            built.push((*column, reference, *value));
        }
        match rows.get(row_number) {
            None => {
                let mut row_xml = format!(r#"<{s}row r="{row_number}">"#);
                for (_, reference, value) in &built {
                    let (xml, formula) = cell_xml(&s, reference, value, None)?;
                    formulas_touched |= formula;
                    row_xml.push_str(&xml);
                }
                row_xml.push_str(&format!("</{s}row>"));
                new_rows.push((*row_number, row_xml));
            }
            Some(row) => {
                let row_node = &tree.nodes[*row];
                let mut existing: BTreeMap<u32, usize> = BTreeMap::new();
                for cell in tree.children(*row, "c") {
                    let Some((column, _)) = tree.nodes[cell]
                        .element
                        .attr("r")
                        .and_then(|reference| address(reference).ok())
                    else {
                        return fail(format!("row {row_number} has a cell without an address"));
                    };
                    existing.insert(column, cell);
                }
                let mut attributes = row_node.element.attributes.clone();
                attributes.retain(|(key, _)| key != "spans");
                if row_node.is_empty_element() || existing.is_empty() {
                    let mut row_xml = start_tag(&row_node.element.name, &attributes, false);
                    for (_, reference, value) in &built {
                        let (xml, formula) = cell_xml(&s, reference, value, None)?;
                        formulas_touched |= formula;
                        row_xml.push_str(&xml);
                    }
                    row_xml.push_str(&format!("</{}>", row_node.element.name));
                    splice.replace(row_node.span.clone(), row_xml);
                    continue;
                }
                if attributes.len() != row_node.element.attributes.len() {
                    splice.replace(
                        row_node.span.start..row_node.inner.start,
                        start_tag(&row_node.element.name, &attributes, false),
                    );
                }
                for (column, reference, value) in &built {
                    match existing.get(column) {
                        Some(cell) => {
                            let cell_node = &tree.nodes[*cell];
                            if let Some(formula) = tree.children(*cell, "f").next() {
                                let formula = &tree.nodes[formula].element;
                                if formula.attr("t") == Some("array") {
                                    return fail(format!(
                                        "{reference} holds an array formula, which this op does not replace"
                                    ));
                                }
                                if formula.attr("t") == Some("shared")
                                    && formula.attr("ref").is_some()
                                {
                                    return fail(format!(
                                        "{reference} anchors a shared formula other cells use; set those cells too or pick another cell"
                                    ));
                                }
                                formulas_touched = true;
                            }
                            let (xml, formula) =
                                cell_xml(&s, reference, value, cell_node.element.attr("s"))?;
                            formulas_touched |= formula;
                            splice.replace(cell_node.span.clone(), xml);
                        }
                        None => {
                            let at = existing
                                .range(column + 1..)
                                .next()
                                .map(|(_, cell)| tree.nodes[*cell].span.start)
                                .unwrap_or(row_node.inner.end);
                            let (xml, formula) = cell_xml(&s, reference, value, None)?;
                            formulas_touched |= formula;
                            splice.insert(at, xml);
                        }
                    }
                }
            }
        }
    }
    let data_node = &tree.nodes[data];
    if data_node.is_empty_element() {
        let inner: String = new_rows.iter().map(|(_, xml)| xml.as_str()).collect();
        splice.replace(
            data_node.span.clone(),
            format!("<{0}>{inner}</{0}>", data_node.element.name),
        );
    } else {
        for (number, xml) in new_rows {
            let at = rows
                .range(number + 1..)
                .next()
                .map(|(_, row)| tree.nodes[*row].span.start)
                .unwrap_or(data_node.inner.end);
            splice.insert(at, xml);
        }
    }
    if let Some(dimension) = tree.descendants(0, "dimension").next() {
        let node = &tree.nodes[dimension];
        let mut bounds: Vec<(u32, u32)> = node
            .element
            .attr("ref")
            .map(|reference| {
                reference
                    .split(':')
                    .filter_map(|end| address(end).ok())
                    .collect()
            })
            .unwrap_or_default();
        for (row, columns) in &targets {
            for column in columns.keys() {
                bounds.push((*column, *row));
            }
        }
        let (min_column, max_column) = (
            bounds.iter().map(|b| b.0).min().unwrap_or(1),
            bounds.iter().map(|b| b.0).max().unwrap_or(1),
        );
        let (min_row, max_row) = (
            bounds.iter().map(|b| b.1).min().unwrap_or(1),
            bounds.iter().map(|b| b.1).max().unwrap_or(1),
        );
        let mut attributes = node.element.attributes.clone();
        attributes.retain(|(key, _)| key != "ref");
        attributes.push((
            "ref".into(),
            format!(
                "{}{min_row}:{}{max_row}",
                column_name(min_column),
                column_name(max_column)
            ),
        ));
        let end = if node.is_empty_element() {
            node.span.end
        } else {
            node.inner.start
        };
        splice.replace(
            node.span.start..end,
            start_tag(&node.element.name, &attributes, node.is_empty_element()),
        );
    }
    work.put(&part, splice.apply(&bytes, &part)?);
    mark_stale(work, formulas_touched)?;

    let quoted = quote_sheet(&name);
    let expect = cells
        .iter()
        .map(|(reference, value)| {
            let (column, row) = address(reference).unwrap_or((1, 1));
            Expect::AnyUnitContains {
                prefix: format!("{quoted}!"),
                needle: format!("{}{row}: {}", column_name(column), shown(value)),
            }
        })
        .collect();
    Ok(Outcome {
        summary: format!(
            "{} cell(s) set on {name}; formulas recalculate when the file is opened",
            cells.len()
        ),
        expect,
        created: Vec::new(),
    })
}

/// Sets `fullCalcOnLoad` so Excel recalculates on open, and when formulas
/// changed removes the calculation chain, which would otherwise name cells
/// that no longer hold the formulas it lists.
fn mark_stale<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    formulas_touched: bool,
) -> Result<(), EditError> {
    let book = book(work)?;
    let mut splice = Splice::default();
    match book.tree.children(0, "calcPr").next() {
        Some(calc) => {
            let node = &book.tree.nodes[calc];
            let mut attributes = node.element.attributes.clone();
            attributes.retain(|(key, _)| key != "fullCalcOnLoad");
            attributes.push(("fullCalcOnLoad".into(), "1".into()));
            let end = if node.is_empty_element() {
                node.span.end
            } else {
                node.inner.start
            };
            splice.replace(
                node.span.start..end,
                start_tag(&node.element.name, &attributes, node.is_empty_element()),
            );
        }
        None => {
            let after = [
                "sheets",
                "functionGroups",
                "externalReferences",
                "definedNames",
            ]
            .iter()
            .filter_map(|local| book.tree.children(0, local).next())
            .map(|index| book.tree.nodes[index].span.end)
            .max();
            let Some(after) = after else {
                return fail("the workbook has no sheets element");
            };
            splice.insert(after, format!(r#"<{}calcPr fullCalcOnLoad="1"/>"#, book.s));
        }
    }
    work.put(&book.part, splice.apply(&book.bytes, &book.part)?);
    if formulas_touched {
        let main = book.part.clone();
        let chain = work
            .relationships(&main)?
            .into_iter()
            .find(|relationship| relationship.short_kind() == "calcChain");
        if let Some(chain) = chain {
            work.remove_relationship(&main, &chain.id)?;
            work.remove(&chain.target);
            work.remove_override(&chain.target)?;
        }
    }
    Ok(())
}

pub(super) fn append_rows<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet: &str,
    rows: &[Vec<CellValue>],
) -> Result<Outcome, EditError> {
    if rows.is_empty() {
        return fail("rows is empty");
    }
    let (part, name) = sheet_part(work, sheet)?;
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let last = tree
        .descendants(0, "row")
        .filter_map(|row| {
            tree.nodes[row]
                .element
                .attr("r")
                .and_then(|r| r.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    let mut cells = BTreeMap::new();
    for (offset, row) in rows.iter().enumerate() {
        for (column, value) in row.iter().enumerate() {
            let reference = format!(
                "{}{}",
                column_name(column as u32 + 1),
                last + 1 + offset as u32
            );
            cells.insert(reference, value.clone());
        }
    }
    let mut outcome = set_cells(work, &name, &cells)?;
    outcome.summary = format!(
        "{} row(s) appended to {name} from row {}",
        rows.len(),
        last + 1
    );
    Ok(outcome)
}

fn check_sheet_name(name: &str) -> Result<(), EditError> {
    if name.is_empty()
        || name.chars().count() > 31
        || name.contains(['[', ']', ':', '*', '?', '/', '\\'])
        || name.starts_with('\'')
        || name.ends_with('\'')
    {
        return fail(format!(
            "{name:?} is not a valid sheet name (1–31 characters, none of [ ] : * ? / \\, not starting or ending with ')"
        ));
    }
    Ok(())
}

fn refuse_locked_structure(book: &Book) -> Result<(), EditError> {
    if let Some(protection) = book.tree.children(0, "workbookProtection").next()
        && book.tree.nodes[protection]
            .element
            .attr("lockStructure")
            .is_some_and(|value| value == "1" || value == "true")
    {
        return fail(
            "the workbook structure is protected; the owner must remove the protection first",
        );
    }
    Ok(())
}

pub(super) fn add_sheet<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    name: &str,
) -> Result<Outcome, EditError> {
    let name = name.trim();
    check_sheet_name(name)?;
    if work.strict() {
        return fail("adding a sheet to a Strict workbook is not supported yet");
    }
    let book = book(work)?;
    refuse_locked_structure(&book)?;
    let mut ids = BTreeSet::new();
    for sheet in book.tree.descendants(0, "sheet") {
        let element = &book.tree.nodes[sheet].element;
        if element
            .attr("name")
            .is_some_and(|existing| existing.eq_ignore_ascii_case(name))
        {
            return fail(format!("a sheet named {name:?} already exists"));
        }
        if let Some(id) = element
            .attr("sheetId")
            .and_then(|id| id.parse::<u32>().ok())
        {
            ids.insert(id);
        }
    }
    let Some(sheets) = book.tree.children(0, "sheets").next() else {
        return fail("the workbook has no sheets element");
    };
    let Some(r) = book
        .tree
        .prefix_for(R)
        .or_else(|| book.tree.prefix_for(R_STRICT))
    else {
        return fail("the workbook does not declare the relationships namespace");
    };
    let part = free_part_name(work, "xl/worksheets/sheet", ".xml");
    work.put(
        &part,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="{S}" xmlns:r="{R}"><sheetData/></worksheet>"#
        )
        .into_bytes(),
    );
    work.set_override(&part, WORKSHEET_TYPE)?;
    let main = book.part.clone();
    let relationship = work.add_relationship(&main, &format!("{R}/worksheet"), &part)?;
    let sheet_id = ids.last().copied().unwrap_or(0) + 1;
    let sheets_node = &book.tree.nodes[sheets];
    let mut splice = Splice::default();
    splice.insert(
        sheets_node.inner.end,
        format!(
            r#"<{}sheet name="{}" sheetId="{sheet_id}" {r}id="{relationship}"/>"#,
            book.s,
            escape_attr(name)
        ),
    );
    work.put(&book.part, splice.apply(&book.bytes, &book.part)?);
    Ok(Outcome {
        summary: format!("sheet {name:?} added"),
        expect: vec![Expect::Section(format!("{}!", quote_sheet(name)))],
        created: Vec::new(),
    })
}

// ---- names, formats and widths (docs/design/72, "Creating from scratch") ---

fn refuse_protected_sheet(tree: &Tree, name: &str) -> Result<(), EditError> {
    if let Some(protection) = tree.descendants(0, "sheetProtection").next()
        && tree.nodes[protection]
            .element
            .attr("sheet")
            .is_some_and(|value| value == "1" || value == "true")
    {
        return fail(format!(
            "sheet {name:?} is protected; the owner must remove the protection before it can be edited"
        ));
    }
    Ok(())
}

/// Whether XML text names `sheet` the way a formula, a defined name, a chart
/// series or a pivot source does (`Sheet1!A1`, `'My sheet'!A1`,
/// `Sheet1:Sheet3!A1`, `sheet="Sheet1"`), ignoring case as Excel does.
fn names_sheet(text: &str, sheet: &str) -> bool {
    let text = text
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .to_lowercase();
    let sheet = sheet.to_lowercase();
    let quoted = sheet.replace('\'', "''");
    if text.contains(&format!("'{quoted}'!"))
        || text.contains(&format!("'{quoted}:"))
        || text.contains(&format!(":{quoted}'!"))
        || text.contains(&format!("sheet=\"{sheet}\""))
    {
        return true;
    }
    let name_char = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    let starts_a_name = |at: usize| text[..at].chars().next_back().is_none_or(|c| !name_char(c));
    let bare = format!("{sheet}!");
    if text
        .match_indices(bare.as_str())
        .any(|(at, _)| starts_a_name(at))
    {
        return true;
    }
    // The first sheet of a 3-D reference, `Sheet1:Sheet3!A1`; a namespace
    // prefix such as `r:id` is not followed by a name and `!`.
    let range = format!("{sheet}:");
    text.match_indices(range.as_str()).any(|(at, _)| {
        let rest = &text[at + range.len()..];
        let name = rest.chars().take_while(|c| name_char(*c)).count();
        starts_a_name(at) && name > 0 && rest.chars().nth(name) == Some('!')
    })
}

pub(super) fn rename_sheet<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet: &str,
    name: &str,
) -> Result<Outcome, EditError> {
    let name = name.trim();
    check_sheet_name(name)?;
    let book = book(work)?;
    refuse_locked_structure(&book)?;
    let wanted = sheet.trim().trim_matches('\'');
    let mut found = None;
    let mut names = Vec::new();
    for node in book.tree.descendants(0, "sheet") {
        let Some(existing) = book.tree.nodes[node].element.attr("name") else {
            continue;
        };
        if existing.eq_ignore_ascii_case(wanted) {
            found = Some((node, existing.to_string()));
        } else if existing.eq_ignore_ascii_case(name) {
            return fail(format!("a sheet named {existing:?} already exists"));
        }
        names.push(existing.to_string());
    }
    let Some((node, old)) = found else {
        return fail(format!("no sheet {sheet:?}; sheets: {}", names.join(", ")));
    };
    if old == name {
        return fail(format!("sheet {old:?} already has that name"));
    }
    let mut referring = Vec::new();
    for part in work.part_names() {
        let lower = part.to_ascii_lowercase();
        let text_only = lower.contains("sharedstrings")
            || lower.contains("comments")
            || lower.starts_with("docprops/");
        if text_only || !(lower.ends_with(".xml") || lower.ends_with(".vml")) {
            continue;
        }
        let bytes = work.get(&part)?;
        if names_sheet(&String::from_utf8_lossy(&bytes), &old) {
            referring.push(part);
        }
    }
    if !referring.is_empty() {
        referring.sort();
        return fail(format!(
            "sheet {old:?} is named in {} (a formula, defined name, chart or other reference), which renaming would break; rename the sheet before writing anything that names it, or keep its name",
            referring.join(", ")
        ));
    }
    let element = &book.tree.nodes[node];
    let mut attributes = element.element.attributes.clone();
    set_attribute(&mut attributes, "name", name.to_string());
    let end = if element.is_empty_element() {
        element.span.end
    } else {
        element.inner.start
    };
    let mut splice = Splice::default();
    splice.replace(
        element.span.start..end,
        start_tag(
            &element.element.name,
            &attributes,
            element.is_empty_element(),
        ),
    );
    work.put(&book.part, splice.apply(&book.bytes, &book.part)?);
    Ok(Outcome {
        summary: format!("sheet {old:?} renamed {name:?}"),
        expect: vec![
            Expect::Section(format!("{}!", quote_sheet(name))),
            Expect::NoSection(format!("{}!", quote_sheet(&old))),
        ],
        created: Vec::new(),
    })
}

fn set_attribute(attributes: &mut Vec<(String, String)>, key: &str, value: String) {
    match attributes.iter_mut().find(|(existing, _)| existing == key) {
        Some((_, existing)) => *existing = value,
        None => attributes.push((key.to_string(), value)),
    }
}

/// Formatting `format_cells` applies to each cell; `None` leaves that part
/// of a cell's format as it is. `fill` is six hex digits, `number_format`
/// an Excel format code.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Format {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub number_format: Option<String>,
    pub fill: Option<String>,
    pub wrap: Option<bool>,
}

impl Format {
    fn is_empty(&self) -> bool {
        self.bold.is_none()
            && self.italic.is_none()
            && self.number_format.is_none()
            && self.fill.is_none()
            && self.wrap.is_none()
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        let flag = |on: bool, what: &str| {
            if on {
                what.to_string()
            } else {
                format!("not {what}")
            }
        };
        parts.extend(self.bold.map(|on| flag(on, "bold")));
        parts.extend(self.italic.map(|on| flag(on, "italic")));
        parts.extend(
            self.number_format
                .as_ref()
                .map(|code| format!("number format {code}")),
        );
        parts.extend(self.fill.as_ref().map(|fill| format!("fill #{fill}")));
        parts.extend(self.wrap.map(|on| flag(on, "wrapped")));
        parts.join(", ")
    }
}

/// Number formats every Excel version knows by id without declaring them.
const BUILTIN_FORMATS: &[(u32, &str)] = &[
    (0, "General"),
    (1, "0"),
    (2, "0.00"),
    (3, "#,##0"),
    (4, "#,##0.00"),
    (9, "0%"),
    (10, "0.00%"),
    (11, "0.00E+00"),
    (12, "# ?/?"),
    (13, "# ??/??"),
    (49, "@"),
];

/// Most cells one `format_cells` call formats.
const MAX_FORMAT_CELLS: u64 = 20_000;

/// `A1:D4` or `B4`, optionally after `Sheet!` → (first column, first row,
/// last column, last row).
fn cell_range(text: &str) -> Result<(u32, u32, u32, u32), EditError> {
    let text = text.trim();
    let text = text.rsplit_once('!').map_or(text, |(_, cells)| cells);
    let (from, to) = text.split_once(':').unwrap_or((text, text));
    let (first_column, first_row) = address(from)?;
    let (last_column, last_row) = address(to)?;
    Ok((
        first_column.min(last_column),
        first_row.min(last_row),
        first_column.max(last_column),
        first_row.max(last_row),
    ))
}

/// `#1f4e79`, `1F4E79` or `FF1F4E79` → `1F4E79`.
fn fill_colour(text: &str) -> Result<String, EditError> {
    let hex = text.trim().trim_start_matches('#').to_ascii_uppercase();
    let hex = if hex.len() == 8 && hex.starts_with("FF") {
        hex[2..].to_string()
    } else {
        hex
    };
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(hex)
    } else {
        fail(format!(
            "fill {text:?} is not a colour; give six hex digits such as \"D9E2F3\""
        ))
    }
}

/// The workbook's styles part and the entries `format_cells` adds to it.
struct Styles {
    part: String,
    bytes: Vec<u8>,
    tree: Tree,
    s: String,
    fonts: Vec<usize>,
    fills: Vec<usize>,
    xfs: Vec<usize>,
    custom: BTreeMap<u32, String>,
    new_fonts: Vec<String>,
    new_fills: Vec<String>,
    new_formats: Vec<(u32, String)>,
    new_xfs: Vec<String>,
}

impl Styles {
    fn load<R2: Read + Seek>(work: &mut Work<'_, R2>) -> Result<Self, EditError> {
        let main = work.main_part();
        let Some(part) = work.related(&main, "styles")? else {
            return fail("the workbook has no styles part, so no format can be applied");
        };
        let bytes = work.get(&part)?;
        let tree = Tree::parse(&bytes, &part, work.limits())?;
        let s = prefix(&tree)?;
        let list = |local: &str, item: &str| -> Vec<usize> {
            tree.children(0, local)
                .next()
                .map(|list| tree.children(list, item).collect())
                .unwrap_or_default()
        };
        let fonts = list("fonts", "font");
        let fills = list("fills", "fill");
        let xfs = list("cellXfs", "xf");
        if xfs.is_empty() {
            return fail("the styles part has no cell formats to build on");
        }
        let custom = list("numFmts", "numFmt")
            .into_iter()
            .filter_map(|format| {
                let element = &tree.nodes[format].element;
                Some((
                    element.attr("numFmtId")?.parse::<u32>().ok()?,
                    element.attr("formatCode")?.to_string(),
                ))
            })
            .collect();
        Ok(Self {
            part,
            bytes,
            tree,
            s,
            fonts,
            fills,
            xfs,
            custom,
            new_fonts: Vec::new(),
            new_fills: Vec::new(),
            new_formats: Vec::new(),
            new_xfs: Vec::new(),
        })
    }

    fn slice(&self, node: usize) -> String {
        String::from_utf8_lossy(&self.bytes[self.tree.nodes[node].span.clone()]).into_owned()
    }

    fn is_on(&self, node: usize, local: &str) -> bool {
        self.tree.children(node, local).next().is_some_and(|child| {
            !matches!(
                self.tree.nodes[child].element.attr("val"),
                Some("0" | "false")
            )
        })
    }

    fn number(&self, node: usize, attribute: &str) -> usize {
        self.tree.nodes[node]
            .element
            .attr(attribute)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    /// The index of a cell format like `old` with `format` applied, adding
    /// the font, fill, number format and cell format that needs.
    fn derive(&mut self, old: usize, format: &Format) -> Result<usize, EditError> {
        let Some(&xf) = self.xfs.get(old) else {
            return fail(format!(
                "a cell uses cell format {old}, which the styles part does not have"
            ));
        };
        let s = self.s.clone();
        let mut attributes = self.tree.nodes[xf].element.attributes.clone();
        if format.bold.is_some() || format.italic.is_some() {
            let font_index = self.number(xf, "fontId");
            let Some(&font) = self.fonts.get(font_index) else {
                return fail(format!(
                    "cell format {old} names a font the styles part does not have"
                ));
            };
            let bold = format.bold.unwrap_or_else(|| self.is_on(font, "b"));
            let italic = format.italic.unwrap_or_else(|| self.is_on(font, "i"));
            let mut inner = String::new();
            if bold {
                inner.push_str(&format!("<{s}b/>"));
            }
            if italic {
                inner.push_str(&format!("<{s}i/>"));
            }
            for child in self.tree.nodes[font].children.clone() {
                if !matches!(self.tree.nodes[child].local(), "b" | "i") {
                    inner.push_str(&self.slice(child));
                }
            }
            let xml = format!("<{s}font>{inner}</{s}font>");
            let index = find_or_add(
                self.fonts.iter().map(|font| self.slice(*font)).collect(),
                &mut self.new_fonts,
                xml,
            );
            set_attribute(&mut attributes, "fontId", index.to_string());
            set_attribute(&mut attributes, "applyFont", "1".into());
        }
        if let Some(fill) = &format.fill {
            let xml = format!(
                r#"<{s}fill><{s}patternFill patternType="solid"><{s}fgColor rgb="FF{fill}"/><{s}bgColor indexed="64"/></{s}patternFill></{s}fill>"#
            );
            let index = find_or_add(
                self.fills.iter().map(|fill| self.slice(*fill)).collect(),
                &mut self.new_fills,
                xml,
            );
            set_attribute(&mut attributes, "fillId", index.to_string());
            set_attribute(&mut attributes, "applyFill", "1".into());
        }
        if let Some(code) = &format.number_format {
            let id = match BUILTIN_FORMATS.iter().find(|(_, known)| known == code) {
                Some((id, _)) => *id,
                None => match self
                    .custom
                    .iter()
                    .map(|(id, known)| (*id, known.clone()))
                    .chain(self.new_formats.iter().cloned())
                    .find(|(_, known)| known == code)
                {
                    Some((id, _)) => id,
                    None => {
                        let id = self
                            .custom
                            .keys()
                            .copied()
                            .chain(self.new_formats.iter().map(|(id, _)| *id))
                            .max()
                            .unwrap_or(163)
                            .max(163)
                            + 1;
                        self.new_formats.push((id, code.clone()));
                        id
                    }
                },
            };
            set_attribute(&mut attributes, "numFmtId", id.to_string());
            set_attribute(&mut attributes, "applyNumberFormat", "1".into());
        }
        let mut children = String::new();
        let alignment = self.tree.children(xf, "alignment").next();
        match (format.wrap, alignment) {
            (Some(wrap), Some(alignment)) => {
                let element = &self.tree.nodes[alignment].element;
                let mut alignment_attributes = element.attributes.clone();
                alignment_attributes.retain(|(key, _)| key != "wrapText");
                if wrap {
                    alignment_attributes.push(("wrapText".into(), "1".into()));
                }
                children.push_str(&start_tag(&element.name, &alignment_attributes, true));
                set_attribute(&mut attributes, "applyAlignment", "1".into());
            }
            (Some(true), None) => {
                children.push_str(&format!(r#"<{s}alignment wrapText="1"/>"#));
                set_attribute(&mut attributes, "applyAlignment", "1".into());
            }
            (Some(false), None) => {}
            (None, Some(alignment)) => children.push_str(&self.slice(alignment)),
            (None, None) => {}
        }
        for child in self.tree.nodes[xf].children.clone() {
            if self.tree.nodes[child].local() != "alignment" {
                children.push_str(&self.slice(child));
            }
        }
        let name = self.tree.nodes[xf].element.name.clone();
        let xml = if children.is_empty() {
            start_tag(&name, &attributes, true)
        } else {
            format!(
                "{}{children}</{name}>",
                start_tag(&name, &attributes, false)
            )
        };
        Ok(find_or_add(
            self.xfs.iter().map(|xf| self.slice(*xf)).collect(),
            &mut self.new_xfs,
            xml,
        ))
    }

    fn write<R2: Read + Seek>(self, work: &mut Work<'_, R2>) -> Result<(), EditError> {
        if self.new_fonts.is_empty()
            && self.new_fills.is_empty()
            && self.new_formats.is_empty()
            && self.new_xfs.is_empty()
        {
            return Ok(());
        }
        let s = &self.s;
        let mut splice = Splice::default();
        let list = |local: &str| self.tree.children(0, local).next();
        extend_list(
            &mut splice,
            &self.tree,
            list("fonts"),
            &self.new_fonts,
            self.fonts.len(),
        )?;
        extend_list(
            &mut splice,
            &self.tree,
            list("fills"),
            &self.new_fills,
            self.fills.len(),
        )?;
        extend_list(
            &mut splice,
            &self.tree,
            list("cellXfs"),
            &self.new_xfs,
            self.xfs.len(),
        )?;
        if !self.new_formats.is_empty() {
            let items: Vec<String> = self
                .new_formats
                .iter()
                .map(|(id, code)| {
                    format!(
                        r#"<{s}numFmt numFmtId="{id}" formatCode="{}"/>"#,
                        escape_attr(code)
                    )
                })
                .collect();
            match list("numFmts") {
                Some(formats) => extend_list(
                    &mut splice,
                    &self.tree,
                    Some(formats),
                    &items,
                    self.custom.len(),
                )?,
                None => {
                    let Some(fonts) = list("fonts") else {
                        return fail("the styles part has no fonts list");
                    };
                    splice.insert(
                        self.tree.nodes[fonts].span.start,
                        format!(
                            r#"<{s}numFmts count="{}">{}</{s}numFmts>"#,
                            items.len(),
                            items.concat()
                        ),
                    );
                }
            }
        }
        work.put(&self.part, splice.apply(&self.bytes, &self.part)?);
        Ok(())
    }
}

/// The index of `xml` among `existing` then `added`, adding it when new.
fn find_or_add(existing: Vec<String>, added: &mut Vec<String>, xml: String) -> usize {
    if let Some(index) = existing.iter().position(|known| *known == xml) {
        return index;
    }
    match added.iter().position(|known| *known == xml) {
        Some(index) => existing.len() + index,
        None => {
            added.push(xml);
            existing.len() + added.len() - 1
        }
    }
}

/// Appends `items` to a styles list and sets its `count`.
fn extend_list(
    splice: &mut Splice,
    tree: &Tree,
    list: Option<usize>,
    items: &[String],
    existing: usize,
) -> Result<(), EditError> {
    if items.is_empty() {
        return Ok(());
    }
    let Some(list) = list else {
        return fail("the styles part lacks a list this op extends");
    };
    let node = &tree.nodes[list];
    let mut attributes = node.element.attributes.clone();
    set_attribute(
        &mut attributes,
        "count",
        (existing + items.len()).to_string(),
    );
    if node.is_empty_element() {
        splice.replace(
            node.span.clone(),
            format!(
                "{}{}</{}>",
                start_tag(&node.element.name, &attributes, false),
                items.concat(),
                node.element.name
            ),
        );
    } else {
        splice.replace(
            node.span.start..node.inner.start,
            start_tag(&node.element.name, &attributes, false),
        );
        splice.insert(node.inner.end, items.concat());
    }
    Ok(())
}

pub(super) fn format_cells<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet: &str,
    range: &str,
    format: &Format,
) -> Result<Outcome, EditError> {
    if format.is_empty() {
        return fail("give at least one of bold, italic, number_format, fill or wrap");
    }
    let format = Format {
        fill: format.fill.as_deref().map(fill_colour).transpose()?,
        number_format: match format.number_format.as_deref().map(str::trim) {
            Some(code)
                if code.is_empty()
                    || code.chars().count() > 255
                    || code.chars().any(char::is_control) =>
            {
                return fail(format!(
                    "number_format {code:?} is not an Excel format code such as \"#,##0.00\" or \"0%\""
                ));
            }
            code => code.map(str::to_string),
        },
        ..format.clone()
    };
    let (first_column, first_row, last_column, last_row) = cell_range(range)?;
    let count = u64::from(last_column - first_column + 1) * u64::from(last_row - first_row + 1);
    if count > MAX_FORMAT_CELLS {
        return fail(format!(
            "{range} is {count} cells; format at most {MAX_FORMAT_CELLS} in one op"
        ));
    }
    let (part, name) = sheet_part(work, sheet)?;
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let s = prefix(&tree)?;
    refuse_protected_sheet(&tree, &name)?;
    let Some(data) = tree.descendants(0, "sheetData").next() else {
        return fail(format!("sheet {name:?} has no sheetData"));
    };
    let mut rows: BTreeMap<u32, usize> = BTreeMap::new();
    for row in tree.children(data, "row") {
        let Some(number) = tree.nodes[row]
            .element
            .attr("r")
            .and_then(|r| r.parse::<u32>().ok())
        else {
            return fail(format!(
                "sheet {name:?} has a row without a number, which this op does not edit"
            ));
        };
        rows.insert(number, row);
    }
    let mut row_cells: BTreeMap<u32, BTreeMap<u32, usize>> = BTreeMap::new();
    for (number, row) in rows.range(first_row..=last_row) {
        let mut cells = BTreeMap::new();
        for cell in tree.children(*row, "c") {
            let Some((column, _)) = tree.nodes[cell]
                .element
                .attr("r")
                .and_then(|reference| address(reference).ok())
            else {
                return fail(format!("row {number} has a cell without an address"));
            };
            cells.insert(column, cell);
        }
        row_cells.insert(*number, cells);
    }
    let style_of = |row: u32, column: u32| -> usize {
        row_cells
            .get(&row)
            .and_then(|cells| cells.get(&column))
            .and_then(|cell| tree.nodes[*cell].element.attr("s"))
            .and_then(|style| style.parse().ok())
            .unwrap_or(0)
    };
    let mut styles = Styles::load(work)?;
    let mut derived: BTreeMap<usize, usize> = BTreeMap::new();
    for row in first_row..=last_row {
        for column in first_column..=last_column {
            let old = style_of(row, column);
            if let std::collections::btree_map::Entry::Vacant(entry) = derived.entry(old) {
                entry.insert(styles.derive(old, &format)?);
            }
        }
    }
    styles.write(work)?;
    let new_style =
        |row: u32, column: u32| derived.get(&style_of(row, column)).copied().unwrap_or(0);
    let empty_cell = |row: u32, column: u32| {
        format!(
            r#"<{s}c r="{}{row}" s="{}"/>"#,
            column_name(column),
            new_style(row, column)
        )
    };
    let mut splice = Splice::default();
    let mut new_rows: Vec<(u32, String)> = Vec::new();
    for row in first_row..=last_row {
        let Some(row_index) = rows.get(&row) else {
            let cells: String = (first_column..=last_column)
                .map(|column| empty_cell(row, column))
                .collect();
            new_rows.push((row, format!(r#"<{s}row r="{row}">{cells}</{s}row>"#)));
            continue;
        };
        let row_node = &tree.nodes[*row_index];
        let mut attributes = row_node.element.attributes.clone();
        attributes.retain(|(key, _)| key != "spans");
        if row_node.is_empty_element() {
            let cells: String = (first_column..=last_column)
                .map(|column| empty_cell(row, column))
                .collect();
            splice.replace(
                row_node.span.clone(),
                format!(
                    "{}{cells}</{}>",
                    start_tag(&row_node.element.name, &attributes, false),
                    row_node.element.name
                ),
            );
            continue;
        }
        let cells = row_cells.get(&row).cloned().unwrap_or_default();
        let mut inserted = false;
        for column in first_column..=last_column {
            match cells.get(&column) {
                Some(cell) => {
                    let node = &tree.nodes[*cell];
                    let mut cell_attributes = node.element.attributes.clone();
                    set_attribute(
                        &mut cell_attributes,
                        "s",
                        new_style(row, column).to_string(),
                    );
                    let end = if node.is_empty_element() {
                        node.span.end
                    } else {
                        node.inner.start
                    };
                    splice.replace(
                        node.span.start..end,
                        start_tag(
                            &node.element.name,
                            &cell_attributes,
                            node.is_empty_element(),
                        ),
                    );
                }
                None => {
                    let at = cells
                        .range(column + 1..)
                        .next()
                        .map(|(_, cell)| tree.nodes[*cell].span.start)
                        .unwrap_or(row_node.inner.end);
                    splice.insert(at, empty_cell(row, column));
                    inserted = true;
                }
            }
        }
        if inserted && attributes.len() != row_node.element.attributes.len() {
            splice.replace(
                row_node.span.start..row_node.inner.start,
                start_tag(&row_node.element.name, &attributes, false),
            );
        }
    }
    let data_node = &tree.nodes[data];
    if data_node.is_empty_element() {
        let inner: String = new_rows.iter().map(|(_, xml)| xml.as_str()).collect();
        splice.replace(
            data_node.span.clone(),
            format!("<{0}>{inner}</{0}>", data_node.element.name),
        );
    } else {
        for (number, xml) in new_rows {
            let at = rows
                .range(number + 1..)
                .next()
                .map(|(_, row)| tree.nodes[*row].span.start)
                .unwrap_or(data_node.inner.end);
            splice.insert(at, xml);
        }
    }
    if let Some((range, tag)) =
        widened_dimension(&tree, &[(first_column, first_row), (last_column, last_row)])
    {
        splice.replace(range, tag);
    }
    work.put(&part, splice.apply(&bytes, &part)?);
    let cells: Vec<String> = (first_row..=last_row)
        .flat_map(|row| {
            (first_column..=last_column).map(move |column| format!("{}{row}", column_name(column)))
        })
        .collect();
    Ok(Outcome {
        summary: format!(
            "{} cell(s) on {name} formatted: {}",
            cells.len(),
            format.describe()
        ),
        expect: vec![Expect::CellFormat {
            sheet: name,
            cells,
            format,
        }],
        created: Vec::new(),
    })
}

/// The sheet's `dimension` start tag widened to cover `cells`, when the
/// sheet has one.
fn widened_dimension(
    tree: &Tree,
    cells: &[(u32, u32)],
) -> Option<(std::ops::Range<usize>, String)> {
    let dimension = tree.descendants(0, "dimension").next()?;
    let node = &tree.nodes[dimension];
    let mut bounds: Vec<(u32, u32)> = node
        .element
        .attr("ref")
        .map(|reference| {
            reference
                .split(':')
                .filter_map(|end| address(end).ok())
                .collect()
        })
        .unwrap_or_default();
    bounds.extend_from_slice(cells);
    let (min_column, max_column) = (
        bounds.iter().map(|b| b.0).min().unwrap_or(1),
        bounds.iter().map(|b| b.0).max().unwrap_or(1),
    );
    let (min_row, max_row) = (
        bounds.iter().map(|b| b.1).min().unwrap_or(1),
        bounds.iter().map(|b| b.1).max().unwrap_or(1),
    );
    let mut attributes = node.element.attributes.clone();
    attributes.retain(|(key, _)| key != "ref");
    attributes.push((
        "ref".into(),
        format!(
            "{}{min_row}:{}{max_row}",
            column_name(min_column),
            column_name(max_column)
        ),
    ));
    let end = if node.is_empty_element() {
        node.span.end
    } else {
        node.inner.start
    };
    Some((
        node.span.start..end,
        start_tag(&node.element.name, &attributes, node.is_empty_element()),
    ))
}

/// A `col` entry: first and last column, and its attributes.
type ColumnRange = (u32, u32, Vec<(String, String)>);

pub(super) fn set_column_widths<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet: &str,
    widths: &BTreeMap<String, f64>,
) -> Result<Outcome, EditError> {
    if widths.is_empty() {
        return fail(
            "widths is empty; give column letters and widths in characters, e.g. {\"A\": 30, \"B\": 12}",
        );
    }
    let mut wanted: BTreeMap<u32, f64> = BTreeMap::new();
    for (letters, width) in widths {
        let (column, _) = address(&format!(
            "{}1",
            letters.trim().trim_end_matches(char::is_numeric)
        ))
        .map_err(|_| EditError {
            op: None,
            message: format!("{letters:?} is not a column letter like A or BC"),
        })?;
        if !width.is_finite() || *width <= 0.0 || *width > 255.0 {
            return fail(format!(
                "column {letters}: {width} is not a width between 0 and 255 characters"
            ));
        }
        wanted.insert(column, *width);
    }
    let (part, name) = sheet_part(work, sheet)?;
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let s = prefix(&tree)?;
    refuse_protected_sheet(&tree, &name)?;
    let Some(data) = tree.descendants(0, "sheetData").next() else {
        return fail(format!("sheet {name:?} has no sheetData"));
    };
    let columns = tree.children(0, "cols").next();
    let mut ranges: Vec<ColumnRange> = Vec::new();
    if let Some(columns) = columns {
        for column in tree.children(columns, "col") {
            let element = &tree.nodes[column].element;
            let (Some(min), Some(max)) = (
                element
                    .attr("min")
                    .and_then(|value| value.parse::<u32>().ok()),
                element
                    .attr("max")
                    .and_then(|value| value.parse::<u32>().ok()),
            ) else {
                return fail(format!(
                    "sheet {name:?} has a column entry without a range, which this op does not edit"
                ));
            };
            ranges.push((min, max, element.attributes.clone()));
        }
    }
    for (column, width) in &wanted {
        let mut next = Vec::with_capacity(ranges.len() + 2);
        let mut covered = false;
        for (min, max, attributes) in ranges {
            if min <= *column && *column <= max {
                covered = true;
                if min < *column {
                    next.push((min, column - 1, attributes.clone()));
                }
                let mut single = attributes.clone();
                set_attribute(&mut single, "width", width.to_string());
                set_attribute(&mut single, "customWidth", "1".into());
                next.push((*column, *column, single));
                if *column < max {
                    next.push((column + 1, max, attributes));
                }
            } else {
                next.push((min, max, attributes));
            }
        }
        if !covered {
            next.push((
                *column,
                *column,
                vec![
                    ("width".into(), width.to_string()),
                    ("customWidth".into(), "1".into()),
                ],
            ));
        }
        ranges = next;
    }
    ranges.sort_by_key(|(min, _, _)| *min);
    let content: String = ranges
        .into_iter()
        .map(|(min, max, mut attributes)| {
            attributes.retain(|(key, _)| key != "min" && key != "max");
            attributes.insert(0, ("max".into(), max.to_string()));
            attributes.insert(0, ("min".into(), min.to_string()));
            start_tag(&format!("{s}col"), &attributes, true)
        })
        .collect();
    let element = format!("<{s}cols>{content}</{s}cols>");
    let mut splice = Splice::default();
    match columns {
        Some(columns) => splice.replace(tree.nodes[columns].span.clone(), element),
        None => splice.insert(tree.nodes[data].span.start, element),
    }
    work.put(&part, splice.apply(&bytes, &part)?);
    let letters: BTreeMap<String, f64> = wanted
        .iter()
        .map(|(column, width)| (column_name(*column), *width))
        .collect();
    Ok(Outcome {
        summary: format!(
            "column widths set on {name}: {}",
            letters
                .iter()
                .map(|(letter, width)| format!("{letter} {width}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        expect: vec![Expect::ColumnWidths {
            sheet: name,
            widths: letters,
        }],
        created: Vec::new(),
    })
}

pub(super) fn add_chart<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet: &str,
    range: &str,
    chart_type: &str,
    title: &str,
    cell: Option<&str>,
) -> Result<Outcome, EditError> {
    if work.strict() {
        return fail("adding charts to a Strict workbook is not supported yet");
    }
    let chart_type = chart_type.trim().to_ascii_lowercase();
    if !matches!(chart_type.as_str(), "bar" | "line" | "pie") {
        return fail("chart_type must be bar, line or pie");
    }
    if title.trim().is_empty() || title.chars().count() > 200 {
        return fail("chart title must have 1–200 characters");
    }
    let (first, last) = chart_range(range)?;
    if last.0 != first.0 + 1 || last.1 < first.1 + 2 || last.1 - first.1 > 1000 {
        return fail("add_chart needs exactly two columns, a header row and 2–1,000 data rows");
    }
    let (sheet_part, sheet_name) = sheet_part(work, sheet)?;
    let (chart_column, chart_row) = match cell {
        Some(cell) => address(cell)?,
        None => (first.0, last.1.saturating_add(1)),
    };
    if (cell.is_some() || chart_column >= first.0)
        && chart_column <= last.0
        && chart_row >= first.1
        && chart_row <= last.1
    {
        return fail(format!(
            "chart placement {}{} is inside source range {range}; choose a cell outside the data so it stays visible",
            column_name(chart_column),
            chart_row
        ));
    }
    let snapshot = work.snapshot()?;
    let document = crate::read::read(Cursor::new(snapshot), *work.limits())?;
    let section = document
        .section(&format!("{sheet_name}!"))
        .ok_or_else(|| EditError {
            op: None,
            message: format!("sheet {sheet_name:?} is missing from the workbook projection"),
        })?;
    let mut values = BTreeMap::new();
    for unit in &document.units[section.units.clone()] {
        for (address, value) in &unit.cells {
            values.insert(address.to_ascii_uppercase(), value.clone());
        }
    }
    let category_col = column_name(first.0);
    let value_col = column_name(last.0);
    let series_name = values
        .get(&format!("{value_col}{}", first.1))
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| EditError {
            op: None,
            message: format!("{value_col}{} needs a series heading", first.1),
        })?;
    let mut categories = Vec::new();
    let mut numbers = Vec::new();
    for row in first.1 + 1..=last.1 {
        let category = values
            .get(&format!("{category_col}{row}"))
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or_else(|| EditError {
                op: None,
                message: format!("{category_col}{row} needs a category label"),
            })?;
        let raw = values
            .get(&format!("{value_col}{row}"))
            .ok_or_else(|| EditError {
                op: None,
                message: format!("{value_col}{row} needs a numeric value"),
            })?;
        let (raw, stale) = cached_cell(raw);
        let number = raw
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or_else(|| EditError {
                op: None,
                message: format!("{value_col}{row} is not a readable number: {raw:?}"),
            })?;
        categories.push(category);
        numbers.push((number, stale));
    }
    let stale_cache = numbers.iter().any(|(_, stale)| *stale);
    if stale_cache {
        return fail(
            "the source range has stale formula results; open and save the workbook in a spreadsheet app before charting it so the chart and RAG cache agree",
        );
    }
    let numbers: Vec<f64> = numbers.into_iter().map(|(number, _)| number).collect();
    let chart_part = free_part_name(work, "xl/charts/chart", ".xml");
    let chart_name = chart_part.rsplit('/').next().unwrap_or("chart1.xml");
    let chart_id = chart_name
        .trim_start_matches("chart")
        .trim_end_matches(".xml")
        .parse::<u32>()
        .unwrap_or(1);
    let chart_rel;
    let drawing_part = match work.related(&sheet_part, "drawing")? {
        Some(part) => part,
        None => free_part_name(work, "xl/drawings/drawing", ".xml"),
    };
    let chart_xml = native_chart_xml(
        &chart_type,
        title.trim(),
        &sheet_name,
        first,
        last,
        &series_name,
        &categories,
        &numbers,
        chart_id,
    );
    work.put(&chart_part, chart_xml.into_bytes());
    work.set_override(&chart_part, CHART_TYPE)?;
    chart_rel = work.add_relationship(&drawing_part, R_CHART, &chart_part)?;

    let anchor = chart_anchor(
        &chart_rel,
        usize::try_from(chart_column.saturating_sub(1)).unwrap_or(0),
        usize::try_from(chart_row.saturating_sub(1)).unwrap_or(0),
    );
    if work.exists(&drawing_part) {
        let bytes = work.get(&drawing_part)?;
        let tree = Tree::parse(&bytes, &drawing_part, work.limits())?;
        let mut splice = Splice::default();
        splice.insert(tree.root().inner.end, anchor);
        work.put(&drawing_part, splice.apply(&bytes, &drawing_part)?);
    } else {
        work.put(
            &drawing_part,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="{DRAWING_NS}" xmlns:a="{A_NS}" xmlns:c="{CHART_NS}" xmlns:r="{R_NS}">{anchor}</xdr:wsDr>"#
            )
            .into_bytes(),
        );
        work.set_override(&drawing_part, DRAWING_TYPE)?;
        add_sheet_drawing(work, &sheet_part, &drawing_part)?;
    }
    let chart_anchor = format!(
        "chart:{chart_name}@{}{}",
        column_name(chart_column),
        chart_row
    );
    let table_anchor = format!("chart:{chart_name}");
    let rows = std::iter::once(vec!["Category".to_string(), series_name])
        .chain(
            categories
                .iter()
                .zip(numbers.iter())
                .map(|(category, number)| vec![category.clone(), number.to_string()]),
        )
        .collect();
    Ok(Outcome {
        summary: format!(
            "{chart_type} chart added to {sheet_name} from {range} at {}{}",
            column_name(chart_column),
            chart_row
        ),
        expect: vec![Expect::Table {
            anchor: table_anchor,
            rows,
        }],
        created: vec![chart_anchor],
    })
}

/// Choose a chart anchor below its data, moving down before moving sideways
/// when images or earlier charts in the same edit would cover its 6.67 by
/// 4-inch drawing area. Images use a conservative 2 by 1.5-inch footprint.
pub(super) fn chart_cell_avoiding_objects(
    range: &str,
    image_cells: &[String],
    chart_cells: &[String],
) -> Result<String, EditError> {
    let (first, last) = chart_range(range)?;
    let base = (first.0, last.1.saturating_add(1));
    // The default worksheet grid is 64 px wide by 20 px high. DrawingML
    // extents are stored in EMUs (96 dpi), so map the actual drawing bounds
    // onto that grid instead of assuming a fixed cell count.
    let drawing_columns = emu_cells(6_096_000, 64 * 9_525);
    let drawing_rows = emu_cells(3_657_600, 20 * 9_525);
    let image_columns = emu_cells(1_828_800, 64 * 9_525);
    let image_rows = emu_cells(1_371_600, 20 * 9_525);
    let images = image_cells
        .iter()
        .filter_map(|cell| address(cell).ok())
        .collect::<Vec<_>>();
    let charts = chart_cells
        .iter()
        .filter_map(|cell| address(cell).ok())
        .collect::<Vec<_>>();
    let overlaps = |column: u32, row: u32| {
        images.iter().any(|(image_column, image_row)| {
            let image_left = image_column.saturating_sub(1);
            let chart_left = column.saturating_sub(1);
            let image_top = image_row.saturating_sub(1);
            let chart_top = row.saturating_sub(1);
            chart_left < image_left.saturating_add(image_columns)
                && image_left < chart_left.saturating_add(drawing_columns)
                && chart_top < image_top.saturating_add(image_rows)
                && image_top < chart_top.saturating_add(drawing_rows)
        }) || charts.iter().any(|(chart_column, chart_row)| {
            let first_left = chart_column.saturating_sub(1);
            let second_left = column.saturating_sub(1);
            let first_top = chart_row.saturating_sub(1);
            let second_top = row.saturating_sub(1);
            second_left < first_left.saturating_add(drawing_columns)
                && first_left < second_left.saturating_add(drawing_columns)
                && second_top < first_top.saturating_add(drawing_rows)
                && first_top < second_top.saturating_add(drawing_rows)
        })
    };
    for distance in 0..256u32 {
        // Keep the chart near its source data and in the visible worksheet
        // area. Moving down is the least surprising way to clear a nearby
        // image; searching sideways can put a wide chart beyond the viewport.
        let down = base.1.saturating_add(distance);
        if down <= 1_048_576 && !overlaps(base.0, down) {
            return Ok(format!("{}{}", column_name(base.0), down));
        }
        if distance > 0 {
            let right = base.0.saturating_add(distance);
            if right <= 16_384 && !overlaps(right, base.1) {
                return Ok(format!("{}{}", column_name(right), base.1));
            }
            let left = base.0.saturating_sub(distance);
            if left > 0 && !overlaps(left, base.1) {
                return Ok(format!("{}{}", column_name(left), base.1));
            }
        }
    }
    fail("could not find a clear default chart position; specify a cell outside the other drawings")
}

fn emu_cells(extent: u64, cell_extent: u64) -> u32 {
    extent
        .saturating_add(cell_extent.saturating_sub(1))
        .checked_div(cell_extent.max(1))
        .unwrap_or(1)
        .max(1)
        .min(u64::from(u32::MAX)) as u32
}

/// Adds a native, filterable SpreadsheetML table over an existing cell range.
pub(super) fn add_excel_table<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet: &str,
    range: &str,
    requested_name: Option<&str>,
) -> Result<Outcome, EditError> {
    if work.strict() {
        return fail("adding native tables to a Strict workbook is not supported yet");
    }
    let (first, last) = chart_range(range)?;
    let columns = last.0 - first.0 + 1;
    let data_rows = last.1 - first.1;
    if !(1..=64).contains(&columns) || !(1..=10_000).contains(&data_rows) {
        return fail("add_excel_table needs 1–64 columns, a header row and 1–10,000 data rows");
    }
    let (sheet_part, sheet_name) = sheet_part(work, sheet)?;
    let snapshot = work.snapshot()?;
    let document = crate::read::read(Cursor::new(snapshot), *work.limits())?;
    let section = document
        .section(&format!("{}!", quote_sheet(&sheet_name)))
        .ok_or_else(|| EditError {
            op: None,
            message: format!("sheet {sheet_name:?} is missing"),
        })?;
    let cells: BTreeMap<String, String> = document.units[section.units.clone()]
        .iter()
        .flat_map(|unit| unit.cells.iter().cloned())
        .collect();
    let mut headers = Vec::with_capacity(columns as usize);
    for column in first.0..=last.0 {
        let address = format!("{}{ }", column_name(column), first.1).replace(' ', "");
        let header = cells.get(&address).map(|value| value.trim()).unwrap_or("");
        if header.is_empty() || header.chars().count() > 255 {
            return fail(format!(
                "{address} needs a non-empty table header of at most 255 characters"
            ));
        }
        if headers
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(header))
        {
            return fail(format!(
                "table headers must be unique; {header:?} appears more than once"
            ));
        }
        headers.push(header.to_string());
    }

    let existing_relations = work.relationships(&sheet_part)?;
    let mut used_names = BTreeSet::new();
    let mut next_id = 1u32;
    let mut occupied = Vec::new();
    for relation in existing_relations
        .iter()
        .filter(|relation| !relation.external && relation.short_kind() == "table")
    {
        if let Some(name) = table_attribute(work, &relation.target, "displayName")? {
            used_names.insert(name.to_ascii_lowercase());
        }
        if let Some(id) =
            table_attribute(work, &relation.target, "id")?.and_then(|id| id.parse::<u32>().ok())
        {
            next_id = next_id.max(id.saturating_add(1));
        }
        if let Some(reference) = table_attribute(work, &relation.target, "ref")? {
            occupied.push(reference);
        }
    }
    for existing in occupied {
        if ranges_overlap(range, &existing)? {
            return fail(format!(
                "range {range} overlaps an existing Excel table ({existing})"
            ));
        }
    }
    let name = if let Some(name) = requested_name {
        let trimmed = name.trim();
        if !valid_table_name(trimmed) {
            return fail(
                "table name must start with a letter or underscore and contain only letters, digits, underscores or periods",
            );
        }
        trimmed.to_string()
    } else {
        let mut number = next_id;
        loop {
            let candidate = format!("VakTable{number}");
            if !used_names.contains(&candidate.to_ascii_lowercase()) {
                break candidate;
            }
            number += 1;
        }
    };
    if used_names.contains(&name.to_ascii_lowercase()) {
        return fail(format!("Excel table name {name:?} is already in use"));
    }

    let table_part = free_part_name(work, "xl/tables/table", ".xml");
    let column_xml = headers
        .iter()
        .enumerate()
        .map(|(i, header)| {
            format!(
                r#"<tableColumn id="{}" name="{}"/>"#,
                i + 1,
                escape_attr(header)
            )
        })
        .collect::<String>();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="{next_id}" name="{}" displayName="{}" ref="{}" totalsRowShown="0"><autoFilter ref="{}"/><tableColumns count="{}">{column_xml}</tableColumns><tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>"#,
        escape_attr(&name),
        escape_attr(&name),
        range,
        range,
        headers.len()
    );
    work.put(&table_part, xml.into_bytes());
    work.set_override(&table_part, TABLE_TYPE)?;
    let relationship_id = work.add_relationship(&sheet_part, R_TABLE, &table_part)?;
    let sheet_bytes = work.get(&sheet_part)?;
    let tree = Tree::parse(&sheet_bytes, &sheet_part, work.limits())?;
    let s = prefix(&tree)?;
    let mut splice = Splice::default();
    if let Some(parts) = tree.children(0, "tableParts").next() {
        let node = &tree.nodes[parts];
        let count = node
            .element
            .attr("count")
            .and_then(|count| count.parse::<usize>().ok())
            .unwrap_or(0)
            + 1;
        let relationship_prefix = tree.prefix_for(R).unwrap_or_else(|| "r:".into());
        let inner = if node.is_empty_element() {
            String::new()
        } else {
            String::from_utf8_lossy(&sheet_bytes[node.inner.clone()]).into_owned()
        };
        splice.replace(node.span.clone(), format!(r#"<{s}tableParts count="{count}">{inner}<{s}tablePart {relationship_prefix}id="{relationship_id}"/></{s}tableParts>"#));
    } else {
        let relationship_prefix = tree.prefix_for(R).unwrap_or_else(|| "r:".into());
        let late = ["extLst"];
        let before = late
            .iter()
            .flat_map(|name| tree.children(0, name))
            .min_by_key(|node| tree.nodes[*node].span.start);
        let position = before
            .map(|node| tree.nodes[node].span.start)
            .unwrap_or(tree.root().inner.end);
        splice.insert(position, format!(r#"<{s}tableParts count="1"><{s}tablePart {relationship_prefix}id="{relationship_id}"/></{s}tableParts>"#));
    }
    work.put(&sheet_part, splice.apply(&sheet_bytes, &sheet_part)?);
    let table_rows = (first.1..=last.1)
        .map(|row| {
            (first.0..=last.0)
                .map(|column| {
                    cells
                        .get(&format!("{}{}", column_name(column), row))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok(Outcome {
        summary: format!("native Excel table {name} added to {sheet_name}!{range}"),
        expect: vec![Expect::Table {
            anchor: name.clone(),
            rows: table_rows,
        }],
        created: vec![name],
    })
}

fn table_attribute<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    part: &str,
    attribute: &str,
) -> Result<Option<String>, EditError> {
    let bytes = work.get(part)?;
    let tree = Tree::parse(&bytes, part, work.limits())?;
    Ok(tree
        .nodes
        .first()
        .and_then(|root| root.element.attr(attribute))
        .map(str::to_string))
}

fn valid_table_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && (name.as_bytes()[0].is_ascii_alphabetic() || name.starts_with('_'))
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')
        && !name.parse::<u32>().is_ok()
        && !matches!(name.to_ascii_uppercase().as_str(), "R" | "C")
        && address(name).is_err()
}

fn ranges_overlap(left: &str, right: &str) -> Result<bool, EditError> {
    let (a, b) = chart_range(left)?;
    let (c, d) = chart_range(right)?;
    Ok(a.0 <= d.0 && c.0 <= b.0 && a.1 <= d.1 && c.1 <= b.1)
}

/// Embeds a PNG/JPEG as a cell-anchored SpreadsheetDrawing picture.
pub(super) fn add_excel_image<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    sheet: &str,
    cell: &str,
    image: &super::SlideImage,
) -> Result<Outcome, EditError> {
    if work.strict() {
        return fail("adding images to a Strict workbook is not supported yet");
    }
    let (column, row) = address(cell)?;
    let (sheet_part, sheet_name) = sheet_part(work, sheet)?;
    let (image_bytes, width, height) = super::deck::validate_image(image)?;
    let extension = if image.mime_type == "image/png" {
        ".png"
    } else {
        ".jpg"
    };
    let media_part = free_part_name(work, "xl/media/image", extension);
    work.put(&media_part, image_bytes);
    work.set_override(&media_part, &image.mime_type)?;

    let drawing_part = match work.related(&sheet_part, "drawing")? {
        Some(part) => part,
        None => free_part_name(work, "xl/drawings/drawing", ".xml"),
    };
    let drawing_relationship = work.add_relationship(&drawing_part, R_IMAGE, &media_part)?;
    let (picture_id, anchor) = {
        let bytes = if work.exists(&drawing_part) {
            work.get(&drawing_part)?
        } else {
            Vec::new()
        };
        if bytes.is_empty() {
            (
                2usize,
                excel_picture_anchor(
                    2,
                    column,
                    row,
                    width,
                    height,
                    image,
                    &drawing_relationship,
                    "r:",
                ),
            )
        } else {
            let tree = Tree::parse(&bytes, &drawing_part, work.limits())?;
            let prefix = tree.prefix_for(R).unwrap_or_else(|| "r:".into());
            let id = tree
                .descendants(0, "cNvPr")
                .filter_map(|node| {
                    tree.nodes[node]
                        .element
                        .attr("id")
                        .and_then(|id| id.parse::<usize>().ok())
                })
                .max()
                .unwrap_or(1)
                + 1;
            (
                id,
                excel_picture_anchor(
                    id,
                    column,
                    row,
                    width,
                    height,
                    image,
                    &drawing_relationship,
                    &prefix,
                ),
            )
        }
    };
    if work.exists(&drawing_part) {
        let bytes = work.get(&drawing_part)?;
        let tree = Tree::parse(&bytes, &drawing_part, work.limits())?;
        let mut splice = Splice::default();
        splice.insert(tree.root().inner.end, anchor);
        work.put(&drawing_part, splice.apply(&bytes, &drawing_part)?);
    } else {
        work.put(
            &drawing_part,
            format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="{DRAWING_NS}" xmlns:a="{A_NS}" xmlns:r="{R_NS}">{anchor}</xdr:wsDr>"#).into_bytes(),
        );
        work.set_override(&drawing_part, DRAWING_TYPE)?;
        add_sheet_drawing(work, &sheet_part, &drawing_part)?;
    }
    let anchor = format!("{}!image@{picture_id}", quote_sheet(&sheet_name));
    Ok(Outcome {
        summary: format!("image added to {sheet_name}!{cell}"),
        expect: vec![Expect::UnitContains {
            anchor: anchor.clone(),
            needles: vec![image.alt_text.clone()],
        }],
        created: vec![anchor],
    })
}

fn excel_picture_anchor(
    id: usize,
    column: u32,
    row: u32,
    width: u32,
    height: u32,
    image: &super::SlideImage,
    relationship: &str,
    relationship_prefix: &str,
) -> String {
    const MAX_WIDTH: f64 = 1_828_800.0;
    const MAX_HEIGHT: f64 = 1_371_600.0;
    // Normalize the source image into the bounded 2 by 1.5 inch box while
    // preserving its aspect ratio. Small icons are intentionally enlarged to
    // this readable size; large images are reduced to fit.
    let scale = (MAX_WIDTH / f64::from(width)).min(MAX_HEIGHT / f64::from(height));
    let cx = (f64::from(width) * scale).round() as u64;
    let cy = (f64::from(height) * scale).round() as u64;
    format!(
        r#"<xdr:oneCellAnchor><xdr:from><xdr:col>{}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="{cx}" cy="{cy}"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="{id}" name="Image {id}" descr="{}"/><xdr:cNvPicPr><a:picLocks noChangeAspect="1"/></xdr:cNvPicPr></xdr:nvPicPr><xdr:blipFill><a:blip {relationship_prefix}embed="{relationship}"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:oneCellAnchor>"#,
        column - 1,
        row - 1,
        escape_attr(&image.alt_text)
    )
}

fn chart_range(range: &str) -> Result<((u32, u32), (u32, u32)), EditError> {
    let mut parts = range.split(':');
    let first = parts.next().unwrap_or_default();
    let last = parts.next().unwrap_or(first);
    if parts.next().is_some() {
        return fail(format!("{range:?} is not a cell range like A1:B5"));
    }
    let first = address(first)?;
    let last = address(last)?;
    Ok((
        (first.0.min(last.0), first.1.min(last.1)),
        (first.0.max(last.0), first.1.max(last.1)),
    ))
}

fn cached_cell(value: &str) -> (&str, bool) {
    let Some(start) = value.find("[cached: ") else {
        return (value, false);
    };
    let cached_start = start + "[cached: ".len();
    let rest = &value[cached_start..];
    let end = rest.find([',', ']']).unwrap_or(rest.len());
    (
        rest[..end].trim(),
        rest.contains("stale until recalculated"),
    )
}

fn native_chart_xml(
    kind: &str,
    title: &str,
    sheet: &str,
    first: (u32, u32),
    last: (u32, u32),
    series: &str,
    categories: &[String],
    values: &[f64],
    chart_id: u32,
) -> String {
    let category_col = column_name(first.0);
    let value_col = column_name(last.0);
    let category_formula = format!(
        "{}!${category_col}${}:${category_col}${}",
        quote_sheet(sheet),
        first.1 + 1,
        last.1
    );
    let value_formula = format!(
        "{}!${value_col}${}:${value_col}${}",
        quote_sheet(sheet),
        first.1 + 1,
        last.1
    );
    let category_cache: String = categories
        .iter()
        .enumerate()
        .map(|(index, value)| {
            format!(
                r#"<c:pt idx="{index}"><c:v>{}</c:v></c:pt>"#,
                escape_text(value)
            )
        })
        .collect();
    let value_cache: String = values
        .iter()
        .enumerate()
        .map(|(index, value)| format!(r#"<c:pt idx="{index}"><c:v>{value}</c:v></c:pt>"#))
        .collect();
    let series_name = escape_text(series);
    let title = escape_text(title);
    let series = format!(
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>{}!${}${}</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{series_name}</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:f>{category_formula}</c:f><c:strCache><c:ptCount val="{}"/>{category_cache}</c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>{value_formula}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="{}"/>{value_cache}</c:numCache></c:numRef></c:val></c:ser>"#,
        quote_sheet(sheet),
        value_col,
        first.1,
        categories.len(),
        values.len()
    );
    let axis = if kind == "pie" {
        format!(
            r#"<c:pieChart><c:varyColors val="1"/>{series}<c:dLbls><c:showLegendKey val="0"/><c:showVal val="1"/><c:showCatName val="1"/><c:showSerName val="0"/></c:dLbls><c:firstSliceAng val="0"/></c:pieChart>"#
        )
    } else {
        let plot = if kind == "line" {
            "lineChart"
        } else {
            "barChart"
        };
        let style = if kind == "line" {
            format!(
                r#"<c:{plot}><c:grouping val="standard"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>{}!${}${}</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{series_name}</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:f>{category_formula}</c:f><c:strCache><c:ptCount val="{}"/>{category_cache}</c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>{value_formula}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="{}"/>{value_cache}</c:numCache></c:numRef></c:val><c:marker><c:symbol val="circle"/><c:size val="5"/></c:marker></c:ser><c:marker val="1"/><c:smooth val="0"/><c:axId val="{}"/><c:axId val="{}"/></c:{plot}>"#,
                quote_sheet(sheet),
                value_col,
                first.1,
                categories.len(),
                values.len(),
                100000 + chart_id * 2,
                100001 + chart_id * 2
            )
        } else {
            format!(
                r#"<c:{plot}><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>{series}<c:gapWidth val="150"/><c:overlap val="0"/><c:axId val="{}"/><c:axId val="{}"/></c:{plot}>"#,
                100000 + chart_id * 2,
                100001 + chart_id * 2
            )
        };
        format!(
            r#"{style}<c:catAx><c:axId val="{}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:tickLblPos val="nextTo"/><c:crossAx val="{}"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/></c:catAx><c:valAx><c:axId val="{}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/><c:numFmt formatCode="General" sourceLinked="1"/><c:tickLblPos val="nextTo"/><c:crossAx val="{}"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx>"#,
            100000 + chart_id * 2,
            100001 + chart_id * 2,
            100001 + chart_id * 2,
            100000 + chart_id * 2
        )
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="{CHART_NS}" xmlns:a="{A_NS}" xmlns:r="{R_NS}"><c:date1904 val="0"/><c:lang val="en-US"/><c:roundedCorners val="0"/><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/><a:t>{title}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title><c:autoTitleDeleted val="0"/><c:plotArea><c:layout/>{axis}</c:plotArea><c:legend><c:legendPos val="b"/><c:layout/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart><c:printSettings><c:headerFooter/><c:pageMargins b="0.75" l="0.7" r="0.7" t="0.75" header="0.3" footer="0.3"/><c:pageSetup/></c:printSettings></c:chartSpace>"#
    )
}

fn chart_anchor(relationship: &str, column: usize, row: usize) -> String {
    format!(
        r#"<xdr:oneCellAnchor xmlns:xdr="{DRAWING_NS}" xmlns:a="{A_NS}" xmlns:c="{CHART_NS}" xmlns:r="{R_NS}"><xdr:from><xdr:col>{column}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="6096000" cy="3657600"/><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Vakyartha chart"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="6096000" cy="3657600"/></xdr:xfrm><a:graphic><a:graphicData uri="{CHART_NS}"><c:chart r:id="{relationship}"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:oneCellAnchor>"#
    )
}

// ---- postconditions read from the written package -------------------------

/// The part of the sheet named `sheet` in a written workbook.
fn written_sheet(
    package: &mut crate::Package<std::io::Cursor<Vec<u8>>>,
    sheet: &str,
) -> Result<String, String> {
    let main = package.main_part().to_string();
    let bytes = package
        .read_part(&main)
        .map_err(|error| error.to_string())?;
    let tree = Tree::parse(&bytes, &main, package.limits()).map_err(|error| error.to_string())?;
    let id = tree
        .descendants(0, "sheet")
        .find(|node| {
            tree.nodes[*node]
                .element
                .attr("name")
                .is_some_and(|name| name.eq_ignore_ascii_case(sheet))
        })
        .and_then(|node| {
            tree.nodes[node]
                .element
                .attr_prefixed("id")
                .map(str::to_string)
        })
        .ok_or_else(|| format!("no sheet {sheet:?}"))?;
    package
        .part_by_relationship_id(&main, &id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("sheet {sheet:?} points at no part"))
}

pub(super) fn check_format(
    package: &mut crate::Package<std::io::Cursor<Vec<u8>>>,
    sheet: &str,
    cells: &[String],
    format: &Format,
) -> Result<(), String> {
    let part = written_sheet(package, sheet)?;
    let limits = *package.limits();
    let bytes = package
        .read_part(&part)
        .map_err(|error| error.to_string())?;
    let tree = Tree::parse(&bytes, &part, &limits).map_err(|error| error.to_string())?;
    let main = package.main_part().to_string();
    let styles_part = package
        .related_part(&main, "styles")
        .map_err(|error| error.to_string())?
        .ok_or("the workbook has no styles part")?;
    let styles_bytes = package
        .read_part(&styles_part)
        .map_err(|error| error.to_string())?;
    let styles =
        Tree::parse(&styles_bytes, &styles_part, &limits).map_err(|error| error.to_string())?;
    let list = |local: &str, item: &str| -> Vec<usize> {
        styles
            .children(0, local)
            .next()
            .map(|list| styles.children(list, item).collect())
            .unwrap_or_default()
    };
    let (fonts, fills, xfs) = (
        list("fonts", "font"),
        list("fills", "fill"),
        list("cellXfs", "xf"),
    );
    let custom: BTreeMap<u32, String> = list("numFmts", "numFmt")
        .into_iter()
        .filter_map(|node| {
            let element = &styles.nodes[node].element;
            Some((
                element.attr("numFmtId")?.parse().ok()?,
                element.attr("formatCode")?.to_string(),
            ))
        })
        .collect();
    let style_of: std::collections::HashMap<String, usize> = tree
        .descendants(0, "c")
        .filter_map(|cell| {
            let element = &tree.nodes[cell].element;
            Some((
                element.attr("r")?.to_ascii_uppercase(),
                element.attr("s").and_then(|s| s.parse().ok()).unwrap_or(0),
            ))
        })
        .collect();
    let on = |node: usize, local: &str| {
        styles.children(node, local).next().is_some_and(|child| {
            !matches!(styles.nodes[child].element.attr("val"), Some("0" | "false"))
        })
    };
    let number = |node: usize, attribute: &str| -> usize {
        styles.nodes[node]
            .element
            .attr(attribute)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    };
    for cell in cells {
        let style = *style_of
            .get(&cell.to_ascii_uppercase())
            .ok_or_else(|| format!("{cell} is missing"))?;
        let xf = *xfs
            .get(style)
            .ok_or_else(|| format!("{cell} names cell format {style}, which does not exist"))?;
        if format.bold.is_some() || format.italic.is_some() {
            let font = *fonts
                .get(number(xf, "fontId"))
                .ok_or_else(|| format!("{cell}'s font does not exist"))?;
            if let Some(bold) = format.bold
                && on(font, "b") != bold
            {
                return Err(format!(
                    "{cell} is not {}bold",
                    if bold { "" } else { "un" }
                ));
            }
            if let Some(italic) = format.italic
                && on(font, "i") != italic
            {
                return Err(format!(
                    "{cell} is not {}italic",
                    if italic { "" } else { "non-" }
                ));
            }
        }
        if let Some(code) = &format.number_format {
            let id = u32::try_from(number(xf, "numFmtId")).unwrap_or(0);
            let actual = BUILTIN_FORMATS
                .iter()
                .find(|(known, _)| *known == id)
                .map(|(_, code)| code.to_string())
                .or_else(|| custom.get(&id).cloned());
            if actual.as_deref() != Some(code.as_str()) {
                return Err(format!("{cell} has number format {actual:?}, not {code:?}"));
            }
        }
        if let Some(fill) = &format.fill {
            let colour = fills
                .get(number(xf, "fillId"))
                .and_then(|node| styles.descendants(*node, "fgColor").next())
                .and_then(|node| styles.nodes[node].element.attr("rgb"))
                .map(str::to_ascii_uppercase);
            if colour.as_deref() != Some(format!("FF{fill}").as_str()) {
                return Err(format!("{cell} is filled {colour:?}, not #{fill}"));
            }
        }
        if let Some(wrap) = format.wrap {
            let wrapped = styles
                .children(xf, "alignment")
                .next()
                .and_then(|node| styles.nodes[node].element.attr("wrapText"))
                .is_some_and(|value| value == "1" || value == "true");
            if wrapped != wrap {
                return Err(format!(
                    "{cell} is {}wrapped",
                    if wrapped { "" } else { "not " }
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn check_widths(
    package: &mut crate::Package<std::io::Cursor<Vec<u8>>>,
    sheet: &str,
    widths: &BTreeMap<String, f64>,
) -> Result<(), String> {
    let part = written_sheet(package, sheet)?;
    let limits = *package.limits();
    let bytes = package
        .read_part(&part)
        .map_err(|error| error.to_string())?;
    let tree = Tree::parse(&bytes, &part, &limits).map_err(|error| error.to_string())?;
    let columns: Vec<(u32, u32, Option<f64>)> = tree
        .descendants(0, "col")
        .filter_map(|node| {
            let element = &tree.nodes[node].element;
            Some((
                element.attr("min")?.parse().ok()?,
                element.attr("max")?.parse().ok()?,
                element.attr("width").and_then(|width| width.parse().ok()),
            ))
        })
        .collect();
    for (letter, want) in widths {
        let (column, _) = address(&format!("{letter}1")).map_err(|error| error.message)?;
        let width = columns
            .iter()
            .find(|(min, max, _)| *min <= column && column <= *max)
            .and_then(|(_, _, width)| *width)
            .ok_or_else(|| format!("column {letter} has no width"))?;
        if (width - want).abs() > 0.001 {
            return Err(format!("column {letter} is {width} wide, not {want}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::names_sheet;

    #[test]
    fn a_sheet_is_named_by_references_never_by_namespace_prefixes() {
        assert!(names_sheet("<f>Sheet1!A1*2</f>", "Sheet1"));
        assert!(
            names_sheet("<f>SUM(sheet1!A1:A3)</f>", "Sheet1"),
            "case-insensitive"
        );
        assert!(names_sheet("<f>'My sheet'!B2</f>", "My sheet"));
        assert!(names_sheet("<f>&apos;My sheet&apos;!B2</f>", "My sheet"));
        assert!(names_sheet("<f>SUM(Sheet1:Sheet3!A1)</f>", "Sheet1"));
        assert!(names_sheet("<f>SUM(Sheet1:Sheet3!A1)</f>", "Sheet3"));
        assert!(names_sheet(
            r#"<worksheetSource sheet="Data" ref="A1:B4"/>"#,
            "Data"
        ));
        assert!(!names_sheet("<f>MySheet1!A1</f>", "Sheet1"));
        assert!(
            !names_sheet(r#"<sheet name="r" r:id="rId1"/>"#, "r"),
            "a namespace prefix is not a reference"
        );
        assert!(!names_sheet("<t>Sheet1</t>", "Sheet1"));
    }
}
