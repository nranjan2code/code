//! L4: a semantic diff between two reads of a PDF (docs/design/77).
//!
//! Like the Office diff it compares what a reader sees, so it describes a
//! change the way a person does ("Page 3: line rewritten", "Page 5 moved",
//! "comment added") and works the same for an Agent's edit and for a file
//! changed elsewhere. Pages are matched by their text, never by object
//! numbers, which a clean rewrite renumbers. The change and impact shapes
//! are the Office ones, so one review surface draws both.

use serde::Serialize;

use crate::read::{Document, Line, Page};

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
    /// Where a person would look: a page, or the document's properties.
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

/// Labels that mark text on the page itself, as opposed to a comment,
/// highlight or form field laid over it.
const BODY_LABELS: &[&str] = &["invisible", "white", "tiny", "off-page"];

fn is_body(line: &Line) -> bool {
    line.labels
        .iter()
        .all(|label| BODY_LABELS.contains(&label.as_str()))
}

fn shown(line: &Line) -> String {
    if line.labels.is_empty() {
        line.text.clone()
    } else {
        format!("{}  ⟨{}⟩", line.text, line.labels.join("; "))
    }
}

/// The text that identifies a page across versions.
fn signature(page: &Page) -> String {
    page.lines
        .iter()
        .filter(|line| is_body(line))
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pairs of indices of a longest common subsequence of `a` and `b`.
fn common<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > 25_000_000 {
        return Vec::new();
    }
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if a[i] == b[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut pairs = Vec::new();
    while i < n && j < m {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

fn page_text(page: &Page) -> String {
    let text = page
        .lines
        .iter()
        .filter(|line| is_body(line))
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" / ");
    if text.chars().count() > 300 {
        format!("{}…", text.chars().take(299).collect::<String>())
    } else if text.is_empty() {
        "(no text)".into()
    } else {
        text
    }
}

/// Line changes between two versions of one page.
fn lines(before: &Page, after: &Page, changes: &mut Vec<Change>) {
    let section = format!("Page {}", after.number);
    let a: Vec<String> = before.lines.iter().map(shown).collect();
    let b: Vec<String> = after.lines.iter().map(shown).collect();
    let pairs = common(&a, &b);
    let mut previous = (0usize, 0usize);
    for &(i, j) in pairs.iter().chain(std::iter::once(&(a.len(), b.len()))) {
        let removed: Vec<usize> = (previous.0..i).collect();
        let added: Vec<usize> = (previous.1..j).collect();
        let paired = removed.len().min(added.len());
        for offset in 0..paired {
            let (old, new) = (removed[offset], added[offset]);
            let both_body = is_body(&before.lines[old]) && is_body(&after.lines[new]);
            if both_body {
                changes.push(Change {
                    section: section.clone(),
                    anchor: after.lines[new].anchor.clone(),
                    kind: ChangeKind::Changed,
                    before: Some(a[old].clone()),
                    after: Some(b[new].clone()),
                });
            } else {
                changes.push(Change {
                    section: section.clone(),
                    anchor: before.lines[old].anchor.clone(),
                    kind: ChangeKind::Removed,
                    before: Some(a[old].clone()),
                    after: None,
                });
                changes.push(Change {
                    section: section.clone(),
                    anchor: after.lines[new].anchor.clone(),
                    kind: ChangeKind::Added,
                    before: None,
                    after: Some(b[new].clone()),
                });
            }
        }
        for &old in &removed[paired..] {
            changes.push(Change {
                section: section.clone(),
                anchor: before.lines[old].anchor.clone(),
                kind: ChangeKind::Removed,
                before: Some(a[old].clone()),
                after: None,
            });
        }
        for &new in &added[paired..] {
            changes.push(Change {
                section: section.clone(),
                anchor: after.lines[new].anchor.clone(),
                kind: ChangeKind::Added,
                before: None,
                after: Some(b[new].clone()),
            });
        }
        previous = (i + 1, j + 1);
    }
    if before.rotation != after.rotation {
        changes.push(Change {
            section,
            anchor: format!("page:{}", after.number),
            kind: ChangeKind::Changed,
            before: Some(format!("turned {}°", before.rotation)),
            after: Some(format!("turned {}°", after.rotation)),
        });
    }
}

/// Compares `before` (absent for a new file) with `after`.
pub fn diff(before: Option<&Document>, after: &Document) -> Diff {
    let mut changes = Vec::new();
    let Some(before) = before else {
        for page in &after.pages {
            changes.push(Change {
                section: format!("Page {}", page.number),
                anchor: format!("page:{}", page.number),
                kind: ChangeKind::Added,
                before: None,
                after: Some(page_text(page)),
            });
        }
        return finish(changes);
    };
    if before.info.title != after.info.title {
        changes.push(Change {
            section: "Properties".into(),
            anchor: "title".into(),
            kind: ChangeKind::Changed,
            before: before.info.title.clone(),
            after: after.info.title.clone(),
        });
    }
    let a: Vec<String> = before.pages.iter().map(signature).collect();
    let b: Vec<String> = after.pages.iter().map(signature).collect();
    let pairs = common(&a, &b);
    let mut matched_before = vec![None; before.pages.len()];
    let mut matched_after = vec![None; after.pages.len()];
    for &(i, j) in &pairs {
        matched_before[i] = Some(j);
        matched_after[j] = Some(i);
    }
    // Pages outside the common sequence with the same text moved.
    for j in 0..after.pages.len() {
        if matched_after[j].is_some() {
            continue;
        }
        if let Some(i) = (0..before.pages.len())
            .find(|i| matched_before[*i].is_none() && a[*i] == b[j] && !b[j].is_empty())
        {
            matched_before[i] = Some(j);
            matched_after[j] = Some(i);
            changes.push(Change {
                section: format!("Page {}", after.pages[j].number),
                anchor: format!("page:{}", after.pages[j].number),
                kind: ChangeKind::Moved,
                before: Some(format!("page {}", before.pages[i].number)),
                after: Some(format!("page {}", after.pages[j].number)),
            });
        }
    }
    // Between matched pages, an unmatched page on each side at the same
    // step is the same page with its text changed.
    let mut previous = (0usize, 0usize);
    let mut anchors: Vec<(usize, usize)> = pairs.clone();
    anchors.push((before.pages.len(), after.pages.len()));
    for (i, j) in anchors {
        let removed: Vec<usize> = (previous.0..i.min(before.pages.len()))
            .filter(|index| matched_before[*index].is_none())
            .collect();
        let added: Vec<usize> = (previous.1..j.min(after.pages.len()))
            .filter(|index| matched_after[*index].is_none())
            .collect();
        let paired = removed.len().min(added.len());
        for offset in 0..paired {
            matched_before[removed[offset]] = Some(added[offset]);
            matched_after[added[offset]] = Some(removed[offset]);
        }
        for &index in &removed[paired..] {
            let page = &before.pages[index];
            changes.push(Change {
                section: format!("Page {}", page.number),
                anchor: format!("page:{}", page.number),
                kind: ChangeKind::Removed,
                before: Some(page_text(page)),
                after: None,
            });
        }
        for &index in &added[paired..] {
            let page = &after.pages[index];
            changes.push(Change {
                section: format!("Page {}", page.number),
                anchor: format!("page:{}", page.number),
                kind: ChangeKind::Added,
                before: None,
                after: Some(page_text(page)),
            });
        }
        previous = (i + 1, j + 1);
    }
    for (j, page) in after.pages.iter().enumerate() {
        if let Some(i) = matched_after[j] {
            lines(&before.pages[i], page, &mut changes);
        }
    }
    finish(changes)
}

fn finish(changes: Vec<Change>) -> Diff {
    let mut summary: Vec<String> = Vec::new();
    let mut sections: Vec<&str> = Vec::new();
    for change in &changes {
        if !sections.contains(&change.section.as_str()) {
            sections.push(&change.section);
        }
    }
    for section in sections {
        let count = |kind: ChangeKind| {
            changes
                .iter()
                .filter(|change| change.section == section && change.kind == kind)
                .count()
        };
        let parts: Vec<String> = [
            (ChangeKind::Changed, "changed"),
            (ChangeKind::Added, "added"),
            (ChangeKind::Removed, "removed"),
            (ChangeKind::Moved, "moved"),
        ]
        .iter()
        .filter_map(|(kind, word)| {
            let count = count(*kind);
            (count > 0).then(|| format!("{count} {word}"))
        })
        .collect();
        summary.push(format!("{section}: {}", parts.join(", ")));
    }
    Diff { changes, summary }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactKind {
    Signature,
    /// Saved revisions an accepted rewrite drops.
    Revisions,
}

/// Something accepting the draft does beyond its visible changes, stated
/// before acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Impact {
    pub kind: ImpactKind,
    pub message: String,
    pub warning: bool,
}

/// What accepting `after` in place of `before` (absent for a new file) does
/// to signatures and to the file's saved history.
pub fn impact(before: Option<&Document>, after: &Document) -> Vec<Impact> {
    let mut impacts = Vec::new();
    let Some(before) = before else {
        return impacts;
    };
    if before.inspection.signatures > 0 && !diff(Some(before), after).is_empty() {
        impacts.push(Impact {
            kind: ImpactKind::Signature,
            message: format!(
                "The file has {} signature field{}. Accepting rewrites the file, which invalidates any signature in it; sign it again if it must stay signed.",
                before.inspection.signatures,
                if before.inspection.signatures == 1 { "" } else { "s" }
            ),
            warning: true,
        });
    }
    if before.inspection.revisions > 1 && after.inspection.revisions <= 1 {
        impacts.push(Impact {
            kind: ImpactKind::Revisions,
            message: format!(
                "The file had {} saved revisions. The accepted file keeps only its current content, so earlier versions, and anything removed from them, are gone.",
                before.inspection.revisions
            ),
            warning: false,
        });
    }
    impacts
}
