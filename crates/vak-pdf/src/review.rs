//! L5: keeping some of a PDF draft's changes (docs/design/77).
//!
//! A draft is its source plus the ops that made it. Every anchor in one
//! call names the source, so each op stands alone: the changes a person
//! keeps are replayed against the same source by the same engine, and the
//! narrower draft is produced and checked exactly as the original was.
//! Nothing is ever patched out of a finished file. A draft made in several
//! calls names places in drafts in between, so it is taken whole.

use serde::Serialize;

use crate::Limits;
use crate::diff::{Change, diff};
use crate::edit::{self, Applied, EditContext, EditError, PdfOp};
use crate::read::{self, Document};

/// Above this many ops a draft is accepted or rejected as a whole: every
/// choice costs one replay.
pub const MAX_OPS: usize = 100;

/// One change a person can keep or leave out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choice {
    /// `"3"` for op 3.
    pub id: String,
    pub label: String,
    /// Choices this one cannot be kept without; ops that each name the
    /// source never depend on one another.
    pub requires: Vec<String>,
    pub changes: Vec<Change>,
}

fn one_step(steps: &[Vec<PdfOp>]) -> Result<&[PdfOp], String> {
    match steps {
        [ops] if ops.len() <= MAX_OPS => Ok(ops),
        [ops] => Err(format!(
            "the draft was made by {} edits, and choosing among more than {MAX_OPS} is not offered",
            ops.len()
        )),
        _ => Err(format!(
            "the draft was made in {} rounds of edits, each naming places in the one before, so its changes are taken together",
            steps.len()
        )),
    }
}

/// Whether two reads show the same document: every line, and the title.
fn same(left: &Document, right: &Document) -> bool {
    left.page_count == right.page_count
        && left.info.title == right.info.title
        && left.lines() == right.lines()
}

/// The recorded ops must reproduce the draft, or it can only be taken
/// whole: narrowing it would silently drop whatever changed it later.
fn reproduces(
    source: Option<&[u8]>,
    ops: &[PdfOp],
    context: &EditContext,
    limits: Limits,
    draft: &Document,
) -> Result<(), String> {
    let whole = edit::apply(source, ops, context, limits).map_err(|error| {
        format!("the draft's recorded edits no longer apply to the file it was made from: {error}")
    })?;
    if same(&whole.document, draft) {
        Ok(())
    } else {
        Err("the draft does not match the edits recorded for it; it was changed after it was written".into())
    }
}

/// The choices a draft offers, each with the changes it makes alone.
pub fn choices(
    source: Option<&[u8]>,
    steps: &[Vec<PdfOp>],
    context: &EditContext,
    limits: Limits,
    draft: &Document,
) -> Result<Vec<Choice>, String> {
    let ops = one_step(steps)?;
    reproduces(source, ops, context, limits, draft)?;
    let before =
        match source {
            Some(bytes) => Some(read::read(bytes, limits).map_err(|error| {
                format!("the file the draft was made from does not read: {error}")
            })?),
            None => None,
        };
    let mut choices = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        let alone = edit::apply(source, std::slice::from_ref(op), context, limits)
            .map_err(|error| format!("change {} cannot be kept on its own: {error}", index + 1))?;
        choices.push(Choice {
            id: index.to_string(),
            label: op.label(),
            requires: Vec::new(),
            changes: diff(before.as_ref(), &alone.document).changes,
        });
    }
    Ok(choices)
}

/// Replays the choices in `keep` (ids from [`choices`]) against `source`.
pub fn narrow(
    source: Option<&[u8]>,
    steps: &[Vec<PdfOp>],
    keep: &[String],
    context: &EditContext,
    limits: Limits,
    draft: &Document,
) -> Result<Applied, EditError> {
    let fail = |message: String| Err(EditError { op: None, message });
    let ops = match one_step(steps) {
        Ok(ops) => ops,
        Err(message) => return fail(message),
    };
    let mut kept = vec![false; ops.len()];
    for id in keep {
        match id.parse::<usize>().ok().filter(|index| *index < ops.len()) {
            Some(index) => kept[index] = true,
            None => return fail(format!("{id:?} is not a change in this draft")),
        }
    }
    if !kept.contains(&true) {
        return fail("no change was kept; reject the draft instead".into());
    }
    if let Err(message) = reproduces(source, ops, context, limits, draft) {
        return fail(message);
    }
    let ops: Vec<PdfOp> = ops
        .iter()
        .zip(&kept)
        .filter(|(_, kept)| **kept)
        .map(|(op, _)| op.clone())
        .collect();
    edit::apply(source, &ops, context, limits)
}
