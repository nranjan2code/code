//! L4: a semantic diff between two read projections (docs/design/72, P3).
//!
//! The diff compares what a reader sees, anchor by anchor, so it describes
//! changes the way a person thinks about them ("Budget: B4 100 → 150",
//! "Slide 2 added", "§ Outlook: paragraph rewritten") and works the same
//! for an Agent's edit and for a file changed outside Vak. It is computed
//! from two re-reads, never from anyone's description of the change.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::package::Vocabulary;
use crate::read::{Document, Unit, UnitKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
    Moved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// Where a person would look: a heading, a sheet, a slide.
    pub section: String,
    pub anchor: String,
    pub kind: ChangeKind,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diff {
    pub changes: Vec<Change>,
    /// One line per section, in document order.
    pub summary: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Compares `before` (absent for a new file) with `after`.
pub fn diff(before: Option<&Document>, after: &Document) -> Diff {
    let mut changes = Vec::new();
    let before_title = before.and_then(|document| document.title.clone());
    if before.is_some() && before_title != after.title {
        changes.push(Change {
            section: "Properties".into(),
            anchor: "title".into(),
            kind: ChangeKind::Changed,
            before: before_title,
            after: after.title.clone(),
        });
    }
    let Some(before) = before else {
        let section = match after.inspection.format.vocabulary {
            Vocabulary::Excel => "Workbook",
            Vocabulary::PowerPoint => "Deck",
            _ => "Document",
        };
        changes.push(Change {
            section: section.into(),
            anchor: String::new(),
            kind: ChangeKind::Added,
            before: None,
            after: Some(format!(
                "new file: {}",
                after
                    .stats
                    .iter()
                    .filter(|(_, count)| *count > 0)
                    .map(|(name, count)| format!("{count} {name}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        });
        return finish(changes);
    };
    match after.inspection.format.vocabulary {
        Vocabulary::Excel => cells(before, after, &mut changes),
        Vocabulary::PowerPoint => {
            slide_order(before, after, &mut changes);
            units(before, after, &mut changes, |unit| {
                unit.kind != UnitKind::Slide
            });
            slide_titles(before, after, &mut changes);
        }
        _ => units(before, after, &mut changes, |_| true),
    }
    finish(changes)
}

fn finish(changes: Vec<Change>) -> Diff {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<String, BTreeMap<&'static str, usize>> = HashMap::new();
    for change in &changes {
        if !order.contains(&change.section) {
            order.push(change.section.clone());
        }
        let word = match change.kind {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Changed => "changed",
            ChangeKind::Moved => "moved",
        };
        *counts
            .entry(change.section.clone())
            .or_default()
            .entry(word)
            .or_default() += 1;
    }
    let summary = order
        .iter()
        .map(|section| {
            let parts: Vec<String> = counts[section]
                .iter()
                .map(|(word, count)| format!("{count} {word}"))
                .collect();
            format!("{section}: {}", parts.join(", "))
        })
        .collect();
    Diff { changes, summary }
}

/// The section title each unit belongs to.
fn section_of(document: &Document) -> Vec<String> {
    let fallback = match document.inspection.format.vocabulary {
        Vocabulary::Word => "Before the first heading",
        _ => "Document",
    };
    let mut titles = vec![fallback.to_string(); document.units.len()];
    for section in &document.sections {
        for index in section.units.clone() {
            if let Some(slot) = titles.get_mut(index) {
                *slot = section.title.clone();
            }
        }
    }
    titles
}

/// Anchor-keyed comparison of units that `keep` selects.
fn units(
    before: &Document,
    after: &Document,
    changes: &mut Vec<Change>,
    keep: impl Fn(&Unit) -> bool,
) {
    let before_sections = section_of(before);
    let after_sections = section_of(after);
    let old: HashMap<&str, (usize, &Unit)> = before
        .units
        .iter()
        .enumerate()
        .filter(|(_, unit)| keep(unit))
        .map(|(index, unit)| (unit.anchor.as_str(), (index, unit)))
        .collect();
    let mut seen = std::collections::HashSet::new();
    for (index, unit) in after
        .units
        .iter()
        .enumerate()
        .filter(|(_, unit)| keep(unit))
    {
        seen.insert(unit.anchor.as_str());
        match old.get(unit.anchor.as_str()) {
            Some((_, previous)) if previous.text == unit.text && previous.labels == unit.labels => {
            }
            Some((_, previous)) => changes.push(Change {
                section: after_sections[index].clone(),
                anchor: unit.anchor.clone(),
                kind: ChangeKind::Changed,
                before: Some(previous.text.clone()),
                after: Some(unit.text.clone()),
            }),
            None => changes.push(Change {
                section: after_sections[index].clone(),
                anchor: unit.anchor.clone(),
                kind: ChangeKind::Added,
                before: None,
                after: Some(unit.text.clone()),
            }),
        }
    }
    let mut removed: Vec<&(usize, &Unit)> = old
        .iter()
        .filter(|(anchor, _)| !seen.contains(*anchor))
        .map(|(_, entry)| entry)
        .collect();
    removed.sort_by_key(|(index, _)| *index);
    for (index, unit) in removed {
        changes.push(Change {
            section: before_sections[*index].clone(),
            anchor: unit.anchor.clone(),
            kind: ChangeKind::Removed,
            before: Some(unit.text.clone()),
            after: None,
        });
    }
}

/// Cell-by-cell comparison per sheet, plus sheets added or removed.
fn cells(before: &Document, after: &Document, changes: &mut Vec<Change>) {
    fn grid(document: &Document) -> BTreeMap<String, Vec<(String, String)>> {
        let sections = section_of(document);
        let mut sheets: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for section in &document.sections {
            sheets.entry(section.title.clone()).or_default();
        }
        for (index, unit) in document.units.iter().enumerate() {
            if unit.kind == UnitKind::SheetRow {
                sheets
                    .entry(sections[index].clone())
                    .or_default()
                    .extend(unit.cells.iter().cloned());
            }
        }
        sheets
    }
    // Cell anchors use the sheet's anchor as the reader writes it
    // (`'Q4 plan'!B4`), so a change can be cited and commented on exactly
    // like a cell `doc_read` returned.
    let prefixes: HashMap<&str, &str> = before
        .sections
        .iter()
        .chain(&after.sections)
        .map(|section| (section.title.as_str(), section.anchor.as_str()))
        .collect();
    let prefix = |sheet: &str| -> String {
        prefixes
            .get(sheet)
            .map(|anchor| anchor.to_string())
            .unwrap_or_else(|| format!("{sheet}!"))
    };
    let old = grid(before);
    let new = grid(after);
    for (sheet, cells) in &new {
        let Some(previous) = old.get(sheet) else {
            changes.push(Change {
                section: sheet.clone(),
                anchor: prefix(sheet),
                kind: ChangeKind::Added,
                before: None,
                after: Some(format!("sheet added with {} cell(s)", cells.len())),
            });
            continue;
        };
        let previous: HashMap<&str, &str> = previous
            .iter()
            .map(|(address, value)| (address.as_str(), value.as_str()))
            .collect();
        let current: HashMap<&str, &str> = cells
            .iter()
            .map(|(address, value)| (address.as_str(), value.as_str()))
            .collect();
        for (address, value) in cells {
            match previous.get(address.as_str()) {
                Some(old) if strip_stale(old) == strip_stale(value) => {}
                Some(old) => changes.push(Change {
                    section: sheet.clone(),
                    anchor: format!("{}{address}", prefix(sheet)),
                    kind: ChangeKind::Changed,
                    before: Some((*old).to_string()),
                    after: Some(value.clone()),
                }),
                None => changes.push(Change {
                    section: sheet.clone(),
                    anchor: format!("{}{address}", prefix(sheet)),
                    kind: ChangeKind::Added,
                    before: None,
                    after: Some(value.clone()),
                }),
            }
        }
        for (address, value) in old.get(sheet).into_iter().flatten() {
            if !current.contains_key(address.as_str()) {
                changes.push(Change {
                    section: sheet.clone(),
                    anchor: format!("{}{address}", prefix(sheet)),
                    kind: ChangeKind::Removed,
                    before: Some(value.clone()),
                    after: None,
                });
            }
        }
    }
    for sheet in old.keys() {
        if !new.contains_key(sheet) {
            changes.push(Change {
                section: sheet.clone(),
                anchor: prefix(sheet),
                kind: ChangeKind::Removed,
                before: Some("sheet".into()),
                after: None,
            });
        }
    }
}

/// A cached value turning stale is a consequence of another change, not a
/// change a person made, so it does not count on its own.
fn strip_stale(value: &str) -> &str {
    value
        .split_once(" [cached")
        .map(|(formula, _)| formula)
        .unwrap_or(value)
}

fn slide_order(before: &Document, after: &Document, changes: &mut Vec<Change>) {
    let old: Vec<&str> = before.sections.iter().map(|s| s.anchor.as_str()).collect();
    let new: Vec<&str> = after.sections.iter().map(|s| s.anchor.as_str()).collect();
    for section in &after.sections {
        if !old.contains(&section.anchor.as_str()) {
            changes.push(Change {
                section: section.title.clone(),
                anchor: section.anchor.clone(),
                kind: ChangeKind::Added,
                before: None,
                after: Some(section.title.clone()),
            });
        }
    }
    for section in &before.sections {
        if !new.contains(&section.anchor.as_str()) {
            changes.push(Change {
                section: section.title.clone(),
                anchor: section.anchor.clone(),
                kind: ChangeKind::Removed,
                before: Some(section.title.clone()),
                after: None,
            });
        }
    }
    let kept_old: Vec<&str> = old
        .iter()
        .copied()
        .filter(|anchor| new.contains(anchor))
        .collect();
    let kept_new: Vec<&str> = new
        .iter()
        .copied()
        .filter(|anchor| old.contains(anchor))
        .collect();
    if kept_old != kept_new {
        for (position, anchor) in kept_new.iter().enumerate() {
            if kept_old.get(position) != Some(anchor)
                && let Some(section) = after.sections.iter().find(|s| s.anchor == *anchor)
            {
                changes.push(Change {
                    section: section.title.clone(),
                    anchor: section.anchor.clone(),
                    kind: ChangeKind::Moved,
                    before: old
                        .iter()
                        .position(|a| a == anchor)
                        .map(|i| format!("position {}", i + 1)),
                    after: new
                        .iter()
                        .position(|a| a == anchor)
                        .map(|i| format!("position {}", i + 1)),
                });
            }
        }
    }
}

/// A slide's title changing is reported once, against the slide itself.
fn slide_titles(before: &Document, after: &Document, changes: &mut Vec<Change>) {
    let title = |text: &str| {
        text.split_once(": ")
            .map(|(_, title)| title.to_string())
            .unwrap_or_default()
    };
    for unit in after
        .units
        .iter()
        .filter(|unit| unit.kind == UnitKind::Slide)
    {
        if let Some(previous) = before
            .units
            .iter()
            .find(|old| old.kind == UnitKind::Slide && old.anchor == unit.anchor)
            && title(&previous.text) != title(&unit.text)
        {
            changes.push(Change {
                section: unit.text.clone(),
                anchor: unit.anchor.clone(),
                kind: ChangeKind::Changed,
                before: Some(title(&previous.text)),
                after: Some(title(&unit.text)),
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactKind {
    Signature,
    Label,
}

/// Something accepting the draft does beyond its visible changes, stated
/// before acceptance (D4, O9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Impact {
    pub kind: ImpactKind,
    pub message: String,
    /// It removes or weakens something: a signature, or a label's
    /// protection.
    pub warning: bool,
}

/// What accepting `after` in place of `before` (absent for a new file) does
/// to digital signatures and sensitivity labels.
pub fn impact(before: Option<&Document>, after: &Document) -> Vec<Impact> {
    let mut impacts = Vec::new();
    let was_signed = before.is_some_and(|before| before.inspection.signed);
    let signed = after.inspection.signed;
    if was_signed && !signed {
        impacts.push(Impact {
            kind: ImpactKind::Signature,
            message: "The file is digitally signed. Accepting removes its signatures, because any edit invalidates them; sign it again in Office if it must stay signed.".into(),
            warning: true,
        });
    } else if signed && before.is_some_and(|before| !diff(Some(before), after).is_empty()) {
        impacts.push(Impact {
            kind: ImpactKind::Signature,
            message: "The draft still carries a signature, but its content differs from the file that was signed, so the signature no longer holds. Vak does not verify signatures.".into(),
            warning: true,
        });
    } else if signed && before.is_none() {
        impacts.push(Impact {
            kind: ImpactKind::Signature,
            message: "The new file carries a digital signature Vak has not verified.".into(),
            warning: false,
        });
    }
    let old: &[String] = before.map_or(&[], |before| &before.inspection.sensitivity_labels);
    let new = &after.inspection.sensitivity_labels;
    let join = |labels: &[String]| labels.join(", ");
    let label = |message: String, warning: bool| Impact {
        kind: ImpactKind::Label,
        message,
        warning,
    };
    match (old.is_empty(), new.is_empty()) {
        (true, true) => {}
        (false, true) => impacts.push(label(
            format!(
                "Accepting removes the sensitivity label {}, and with it the limits it puts on where the file may go.",
                join(old)
            ),
            true,
        )),
        (true, false) => impacts.push(label(
            format!("Accepting adds the sensitivity label {}.", join(new)),
            false,
        )),
        (false, false) if old == new.as_slice() => impacts.push(label(
            format!("Labelled {}; the label is kept.", join(new)),
            false,
        )),
        (false, false) => impacts.push(label(
            format!(
                "The sensitivity label changes from {} to {}.",
                join(old),
                join(new)
            ),
            true,
        )),
    }
    impacts
}
