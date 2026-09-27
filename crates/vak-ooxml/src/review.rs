//! L5: keeping some of a draft's changes (docs/design/72, P3).
//!
//! A draft is its source plus the ops that produced it. When a person keeps
//! some changes and not others, the kept ops are replayed against the same
//! source by the same engine, so the narrower draft is produced and checked
//! exactly as the original was. Nothing is ever patched out of a finished
//! file.
//!
//! The unit of choice is an atom: one op, except that `set_cells` splits
//! into one atom per cell. An atom that names an anchor an earlier op minted
//! (a new paragraph or slide), or a sheet an earlier op added, requires that
//! op's atom.

use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;

use serde::Serialize;

use crate::diff::{Change, diff};
use crate::edit::{self, Applied, EditContext, EditError, OfficeOp};
use crate::read::{self, Document};
use crate::{Format, Limits};

/// Above this many ops a draft is accepted or rejected as a whole: every
/// choice costs one replay per op.
pub const MAX_OPS: usize = 100;

/// One change a person can keep or leave out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choice {
    /// `"3"` for op 3, `"3:B4"` for one cell of a `set_cells` op.
    pub id: String,
    pub label: String,
    /// Choices this one cannot be kept without.
    pub requires: Vec<String>,
    /// What this change does, compared with the state before it.
    pub changes: Vec<Change>,
}

/// The choices a draft offers, each with the changes it makes.
///
/// Fails, with a reason a person can read, when the recorded ops do not
/// reproduce `draft` (it was changed after it was written, or `source` is
/// not the file it was made from); such a draft can only be taken whole.
pub fn choices(
    source: &[u8],
    ops: &[OfficeOp],
    context: &EditContext,
    limits: Limits,
    target: Option<Format>,
    draft: &Document,
) -> Result<Vec<Choice>, String> {
    if ops.len() > MAX_OPS {
        return Err(format!(
            "the draft was made by {} edits, and choosing among more than {MAX_OPS} is not offered",
            ops.len()
        ));
    }
    let mut previous = read::read(Cursor::new(source.to_vec()), limits)
        .map_err(|error| format!("the file the draft was made from does not read: {error}"))?;
    let mut state = source.to_vec();
    let mut created = Vec::with_capacity(ops.len());
    let mut changes = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter().enumerate() {
        let applied = edit::apply(&state, std::slice::from_ref(op), context, limits, target)
            .map_err(|error| {
                format!(
                    "replaying edit {} of the draft failed: {}",
                    index + 1,
                    error.message
                )
            })?;
        changes.push(diff(Some(&previous), &applied.document).changes);
        created.push(
            applied
                .results
                .first()
                .map(|result| result.created.clone())
                .unwrap_or_default(),
        );
        state = applied.bytes;
        previous = applied.document;
    }
    if !diff(Some(&previous), draft).is_empty() {
        return Err(
            "the draft is not what its recorded edits produce: it changed after it was written, or the file it was made from changed"
                .into(),
        );
    }
    let mut choices: Vec<Choice> = Vec::new();
    let mut op_of: Vec<usize> = Vec::new();
    for (index, (op, op_changes)) in ops.iter().zip(changes).enumerate() {
        match split_cells(op, &op_changes) {
            Some(cells) => {
                for (cell, cell_changes) in cells {
                    choices.push(Choice {
                        id: format!("{index}:{cell}"),
                        label: label(op, Some(&cell)),
                        requires: Vec::new(),
                        changes: cell_changes,
                    });
                    op_of.push(index);
                }
            }
            None => {
                choices.push(Choice {
                    id: index.to_string(),
                    label: label(op, None),
                    requires: Vec::new(),
                    changes: op_changes,
                });
                op_of.push(index);
            }
        }
    }
    for position in 0..choices.len() {
        let needed = requirements(ops, &created, op_of[position], context);
        choices[position].requires = choices
            .iter()
            .zip(&op_of)
            .filter(|(_, op)| needed.contains(op))
            .map(|(choice, _)| choice.id.clone())
            .collect();
    }
    Ok(choices)
}

/// Replays the choices in `keep` (ids from [`choices`]) against `source`.
/// A kept choice whose requirement was left out fails, naming both, and so
/// does a `draft` the ops do not reproduce: narrowing it would silently drop
/// whatever changed it afterwards.
pub fn narrow(
    source: &[u8],
    ops: &[OfficeOp],
    keep: &[String],
    context: &EditContext,
    limits: Limits,
    target: Option<Format>,
    draft: &Document,
) -> Result<Applied, EditError> {
    if ops.len() > MAX_OPS {
        return edit_fail(format!(
            "the draft was made by {} edits, and choosing among more than {MAX_OPS} is not offered",
            ops.len()
        ));
    }
    let mut whole = vec![false; ops.len()];
    let mut cells: Vec<Vec<String>> = vec![Vec::new(); ops.len()];
    for id in keep {
        let (index, cell) = match id.split_once(':') {
            Some((index, cell)) => (index, Some(cell)),
            None => (id.as_str(), None),
        };
        let Some(index) = index
            .parse::<usize>()
            .ok()
            .filter(|index| *index < ops.len())
        else {
            return edit_fail(format!("{id:?} is not a change in this draft"));
        };
        match cell {
            None => whole[index] = true,
            Some(cell) => {
                let known = matches!(&ops[index], OfficeOp::SetCells { cells, .. }
                    if cells.keys().any(|key| key.eq_ignore_ascii_case(cell)));
                if !known {
                    return edit_fail(format!("{id:?} is not a change in this draft"));
                }
                cells[index].push(cell.to_ascii_uppercase());
            }
        }
    }
    let kept: Vec<(usize, OfficeOp)> = ops
        .iter()
        .enumerate()
        .filter_map(|(index, op)| {
            if whole[index] {
                return Some((index, op.clone()));
            }
            match op {
                OfficeOp::SetCells { sheet, cells: all } if !cells[index].is_empty() => Some((
                    index,
                    OfficeOp::SetCells {
                        sheet: sheet.clone(),
                        cells: all
                            .iter()
                            .filter(|(key, _)| cells[index].contains(&key.to_ascii_uppercase()))
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect(),
                    },
                )),
                _ => None,
            }
        })
        .collect();
    if kept.is_empty() {
        return edit_fail("no change was kept; reject the draft instead");
    }
    let original = edit::apply(source, ops, context, limits, target)?;
    if !diff(Some(&original.document), draft).is_empty() {
        return edit_fail(
            "the draft is not what its recorded edits produce, so it can only be accepted or rejected whole",
        );
    }
    let created: Vec<Vec<String>> = original
        .results
        .iter()
        .map(|result| result.created.clone())
        .collect();
    let kept_ops: Vec<usize> = kept.iter().map(|(index, _)| *index).collect();
    for (index, _) in &kept {
        if let Some(missing) = requirements(ops, &created, *index, context)
            .into_iter()
            .find(|needed| !kept_ops.contains(needed))
        {
            return Err(EditError {
                op: Some((*index, ops[*index].name())),
                message: format!(
                    "it builds on edit {} ({}), which was left out; keep both or neither",
                    missing + 1,
                    label(&ops[missing], None)
                ),
            });
        }
    }
    let mut minted: HashMap<String, String> = HashMap::new();
    let mut state = source.to_vec();
    let mut results = Vec::with_capacity(kept.len());
    let mut notices: Vec<String> = Vec::new();
    let mut last: Option<Applied> = None;
    for (index, op) in kept {
        let op = remap(op, &minted);
        let applied = edit::apply(&state, std::slice::from_ref(&op), context, limits, target)
            .map_err(|error| EditError {
                op: Some((index, op.name())),
                message: error.message,
            })?;
        if let Some(result) = applied.results.first() {
            for (before, now) in created[index].iter().zip(&result.created) {
                minted.insert(before.clone(), now.clone());
            }
        }
        results.extend(applied.results.iter().cloned());
        for notice in &applied.notices {
            if !notices.contains(notice) {
                notices.push(notice.clone());
            }
        }
        state.clone_from(&applied.bytes);
        last = Some(applied);
    }
    let Some(last) = last else {
        return edit_fail("no change was kept; reject the draft instead");
    };
    Ok(Applied {
        bytes: last.bytes,
        results,
        document: last.document,
        notices,
    })
}

fn edit_fail<T>(message: impl Into<String>) -> Result<T, EditError> {
    Err(EditError {
        op: None,
        message: message.into(),
    })
}

/// A `set_cells` op of several cells, split per cell, when every change it
/// made belongs to exactly one of its cells.
fn split_cells(op: &OfficeOp, changes: &[Change]) -> Option<Vec<(String, Vec<Change>)>> {
    let OfficeOp::SetCells { cells, .. } = op else {
        return None;
    };
    let keys: Vec<String> = cells.keys().map(|key| key.to_ascii_uppercase()).collect();
    let mut unique = keys.clone();
    unique.sort();
    unique.dedup();
    if keys.len() < 2 || unique.len() != keys.len() {
        return None;
    }
    let mut split: BTreeMap<&str, Vec<Change>> =
        keys.iter().map(|key| (key.as_str(), Vec::new())).collect();
    for change in changes {
        let address = change.anchor.rsplit_once('!').map(|(_, address)| address)?;
        split.get_mut(address)?.push(change.clone());
    }
    Some(
        keys.iter()
            .map(|key| (key.clone(), split.remove(key.as_str()).unwrap_or_default()))
            .collect(),
    )
}

/// The earlier ops `index` builds on.
fn requirements(
    ops: &[OfficeOp],
    created: &[Vec<String>],
    index: usize,
    context: &EditContext,
) -> Vec<usize> {
    let references = references(&ops[index]);
    let sheet = match &ops[index] {
        OfficeOp::SetCells { sheet, .. }
        | OfficeOp::AppendRows { sheet, .. }
        | OfficeOp::FormatCells { sheet, .. }
        | OfficeOp::SetColumnWidths { sheet, .. }
        | OfficeOp::RenameSheet { sheet, .. } => Some(sheet),
        _ => None,
    };
    (0..index)
        .filter(|earlier| {
            let minted = created.get(*earlier).is_some_and(|minted| {
                minted
                    .iter()
                    .any(|minted| references.iter().any(|reference| names(reference, minted)))
            });
            // A sheet exists under this name because an earlier op added
            // it or gave it the name.
            let added = matches!((&ops[*earlier], sheet),
                (OfficeOp::AddSheet { name } | OfficeOp::RenameSheet { name, .. }, Some(sheet))
                    if name.eq_ignore_ascii_case(sheet));
            // A later op named a paragraph by the numbering an earlier
            // removal left; without that removal it would land elsewhere.
            let renumbered = edit::renumbered_by(&ops[*earlier], &ops[index], context);
            minted || added || renumbered
        })
        .collect()
}

fn names(reference: &str, anchor: &str) -> bool {
    reference == anchor
        || reference
            .strip_prefix(anchor)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn references(op: &OfficeOp) -> Vec<&str> {
    match op {
        OfficeOp::ReplaceParagraphText { anchor, .. }
        | OfficeOp::DeleteParagraph { anchor }
        | OfficeOp::SetPlaceholderText { anchor, .. }
        | OfficeOp::SetNotes { anchor, .. }
        | OfficeOp::DeleteSlide { anchor } => vec![anchor],
        OfficeOp::MoveSlide { anchor, after } => {
            let mut all = vec![anchor.as_str()];
            all.extend(after.as_deref());
            all
        }
        OfficeOp::AddParagraph { after, .. }
        | OfficeOp::AddTable { after, .. }
        | OfficeOp::AddSlideFromLayout { after, .. } => after.as_deref().into_iter().collect(),
        OfficeOp::SetCells { .. }
        | OfficeOp::AppendRows { .. }
        | OfficeOp::AddSheet { .. }
        | OfficeOp::RenameSheet { .. }
        | OfficeOp::FormatCells { .. }
        | OfficeOp::SetColumnWidths { .. }
        | OfficeOp::SetTitle { .. } => Vec::new(),
    }
}

/// Rewrites anchors minted by the original run to the ones this replay
/// minted, so a kept op still names the paragraph or slide it meant.
fn remap(mut op: OfficeOp, minted: &HashMap<String, String>) -> OfficeOp {
    let swap = |value: &mut String| {
        if let Some((before, now)) = minted.iter().find(|(before, _)| names(value, before)) {
            *value = format!("{now}{}", &value[before.len()..]);
        }
    };
    match &mut op {
        OfficeOp::ReplaceParagraphText { anchor, .. }
        | OfficeOp::DeleteParagraph { anchor }
        | OfficeOp::SetPlaceholderText { anchor, .. }
        | OfficeOp::SetNotes { anchor, .. }
        | OfficeOp::DeleteSlide { anchor } => swap(anchor),
        OfficeOp::MoveSlide { anchor, after } => {
            swap(anchor);
            if let Some(after) = after {
                swap(after);
            }
        }
        OfficeOp::AddSlideFromLayout {
            after: Some(after), ..
        }
        | OfficeOp::AddParagraph {
            after: Some(after), ..
        }
        | OfficeOp::AddTable {
            after: Some(after), ..
        } => swap(after),
        _ => {}
    }
    op
}

fn label(op: &OfficeOp, cell: Option<&str>) -> String {
    match op {
        OfficeOp::ReplaceParagraphText { anchor, .. } => format!("Edit paragraph {anchor}"),
        OfficeOp::AddParagraph { after, text, .. } => match after {
            Some(after) => format!("New paragraph after {after}: {}", preview(text)),
            None => format!("New paragraph: {}", preview(text)),
        },
        OfficeOp::AddTable { rows, .. } => {
            let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
            format!(
                "New table, {} {} by {columns} {}",
                rows.len(),
                if rows.len() == 1 { "row" } else { "rows" },
                if columns == 1 { "column" } else { "columns" }
            )
        }
        OfficeOp::DeleteParagraph { anchor } => format!("Delete paragraph {anchor}"),
        OfficeOp::SetCells { sheet, cells } => match cell {
            Some(cell) => format!("Set {sheet}!{cell}"),
            None if cells.len() == 1 => cells
                .keys()
                .next()
                .map(|cell| format!("Set {sheet}!{}", cell.to_ascii_uppercase()))
                .unwrap_or_default(),
            None => format!("Set {} cells on {sheet}", cells.len()),
        },
        OfficeOp::AppendRows { sheet, rows } => {
            format!("Add {} row(s) to {sheet}", rows.len())
        }
        OfficeOp::AddSheet { name } => format!("Add sheet {name:?}"),
        OfficeOp::RenameSheet { sheet, name } => format!("Rename sheet {sheet:?} to {name:?}"),
        OfficeOp::FormatCells { sheet, range, .. } => format!("Format {sheet}!{range}"),
        OfficeOp::SetColumnWidths { sheet, widths } => format!(
            "Set the width of column(s) {} on {sheet}",
            widths.keys().cloned().collect::<Vec<_>>().join(", ")
        ),
        OfficeOp::AddSlideFromLayout { layout, .. } => {
            format!("New slide from layout {layout:?}")
        }
        OfficeOp::SetPlaceholderText { anchor, .. } => format!("Set text of {anchor}"),
        OfficeOp::SetNotes { anchor, .. } => format!("Set speaker notes of {anchor}"),
        OfficeOp::DeleteSlide { anchor } => format!("Delete {anchor}"),
        OfficeOp::MoveSlide { anchor, .. } => format!("Move {anchor}"),
        OfficeOp::SetTitle { title } => format!("Set the title to {title:?}"),
    }
}

/// The start of a new paragraph's text, for a change's label.
fn preview(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 60 {
        text
    } else {
        format!("{}…", text.chars().take(59).collect::<String>())
    }
}
