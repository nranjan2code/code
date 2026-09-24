//! Excel ops. Vak does not calculate (deferred): a changed input or formula
//! sets `fullCalcOnLoad`, so Excel recalculates on open, and the reader
//! reports every cached formula value as stale until then.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};

use super::{CellValue, EditError, Expect, Outcome, Work, fail, free_part_name};
use crate::read::column_name;
use crate::splice::{Splice, Tree, escape_attr, escape_text, start_tag};

const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const S_STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const R_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const WORKSHEET_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const MAX_ROW: u32 = 1_048_576;
const MAX_COLUMN: u32 = 16_384;

fn prefix(tree: &Tree) -> Result<String, EditError> {
    match tree.prefix_for(S).or_else(|| tree.prefix_for(S_STRICT)) {
        Some(prefix) => Ok(prefix),
        None => fail("the part does not declare the SpreadsheetML namespace on its root"),
    }
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
        created: None,
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

pub(super) fn add_sheet<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    name: &str,
) -> Result<Outcome, EditError> {
    let name = name.trim();
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
    if work.strict() {
        return fail("adding a sheet to a Strict workbook is not supported yet");
    }
    let book = book(work)?;
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
        created: None,
    })
}
