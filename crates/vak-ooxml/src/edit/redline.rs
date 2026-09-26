//! A paragraph edit as a redline that marks only what changed
//! (docs/design/72-openxml-documents.md, R7 and O1).
//!
//! The paragraph is read the way `doc_read` shows it, and its text is
//! compared with the text it should read word by word
//! ([`super::textdiff`]). Only the words that differ become tracked
//! changes: a run the change splits keeps its formatting on every piece,
//! new text takes the formatting of the text it replaces (or of the text
//! just before it), and every element the change does not touch is copied
//! byte for byte.
//!
//! What a paragraph edit never changes:
//! - text Word computes or someone else owns: a field's result, another
//!   author's tracked change, text inside an equation or a drawing. A
//!   change that would alter it is refused, naming it;
//! - content the reader does not show: footnote and endnote marks, images,
//!   bookmarks and comment ranges, field codes, hidden and white text, and
//!   another author's deletions. It stays where it is, so an edit never
//!   removes what the model could not see;
//! - the paragraph's leading and trailing whitespace, which the reader
//!   trims.
//!
//! The author's own earlier tracked changes are revised, never stacked:
//! the comparison is made against the paragraph as it read before them, so
//! revising a draft's change leaves one change, and asking for the original
//! text withdraws it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use super::textdiff::{self, fold_text};
use super::word::run_content;
use super::{EditContext, EditError, fail};
use crate::Limits;
use crate::splice::{Splice, Tree, escape_attr, escape_text, start_tag};
use crate::xml::{self, XmlEvent};

/// The WordprocessingML part being edited.
pub(super) struct Part<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub tree: &'a Tree,
    pub limits: &'a Limits,
    /// The WordprocessingML prefix with its colon (`w:`).
    pub w: &'a str,
}

/// The paragraph after an edit, as the postcondition checks it.
pub(super) struct Views {
    /// Its text with the author's tracked changes accepted.
    pub accepted: String,
    /// Its text with the author's tracked changes rejected.
    pub rejected: String,
    /// The content the reader does not show, by element name, in order.
    pub fixed: Vec<String>,
}

/// What [`replace`] produced.
pub(super) struct Replaced {
    pub summary: String,
    /// The part's new bytes; `None` when the paragraph already reads so.
    pub bytes: Option<Vec<u8>>,
    /// The requested text, folded as [`textdiff::fold`] compares it.
    pub accepted: String,
    pub rejected: String,
    pub fixed: Vec<String>,
}

/// Reader markers a paragraph's new text must not carry: the text is what
/// the paragraph should read, and the marked content stays where it is.
const MARKERS: &[&str] = &[
    "[inserted by ",
    "[deleted by ",
    "[hidden: ",
    "[white text: ",
];

/// Zero-width elements that may sit inside a stretch folded into a change.
const NEUTRAL: &[&str] = &[
    "bookmarkStart",
    "bookmarkEnd",
    "commentRangeStart",
    "commentRangeEnd",
    "proofErr",
    "permStart",
    "permEnd",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeStart",
    "moveToRangeEnd",
    "customXmlInsRangeStart",
    "customXmlInsRangeEnd",
    "customXmlDelRangeStart",
    "customXmlDelRangeEnd",
    "customXmlMoveFromRangeStart",
    "customXmlMoveFromRangeEnd",
    "customXmlMoveToRangeStart",
    "customXmlMoveToRangeEnd",
    "lastRenderedPageBreak",
];

/// Why a stretch of visible text cannot change.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Guard {
    /// A field's code or result, which Word computes.
    Field,
    /// Part of a tracked change by another author.
    Tracked(String),
    /// Text inside an element this op does not edit.
    Element(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Visible and untracked.
    Plain,
    /// Deleted by the author's own earlier tracked change: part of the
    /// paragraph as it read before that change, so it may be restored.
    OwnDeleted,
    /// Visible and fixed; the group says why.
    Guarded(usize),
}

/// Where new text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Spot {
    /// Before or after an element, among its siblings.
    Before(usize),
    After(usize),
    /// Inside a run, before character `offset` of its content element
    /// `child` (`child` equal to the run's element count is its end).
    InRun {
        run: usize,
        child: usize,
        offset: usize,
    },
    /// At the end of the paragraph's content.
    End,
}

/// One content element's text.
#[derive(Debug, Clone)]
struct Chunk {
    node: usize,
    run: Option<usize>,
    /// The element's position among its run's content elements.
    child: usize,
    text: Vec<char>,
    /// Index of its first character in the paragraph's text.
    start: usize,
    role: Role,
    /// A `w:t` or `w:delText`, which may be split; anything else is whole.
    splittable: bool,
    link: Option<usize>,
    order: usize,
}

/// Visible text that cannot change, and where text placed around it goes.
#[derive(Debug, Clone)]
struct Group {
    guard: Guard,
    /// `None` when the group starts or ends outside this paragraph.
    before: Option<Spot>,
    after: Option<Spot>,
    end_order: usize,
}

/// A hyperlink: a `w:hyperlink`, or a HYPERLINK field's result.
#[derive(Debug, Clone)]
struct Link {
    before: Option<Spot>,
    after: Option<Spot>,
    end_order: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weight {
    /// A footnote or endnote mark: new text after the text it follows goes
    /// after it, so the mark stays with its sentence.
    Attach,
    /// Bookmarks, comment ranges and the like.
    Neutral,
    Significant,
}

/// Zero-width content the reader does not show.
#[derive(Debug, Clone)]
struct Fixed {
    gap: usize,
    order: usize,
    weight: Weight,
    after: Spot,
    name: String,
}

/// The author's own earlier tracked insertion.
#[derive(Debug, Clone)]
struct OwnInsertion {
    node: usize,
    gap: usize,
    text: String,
}

#[derive(Debug, Default)]
struct Paragraph {
    node: usize,
    chars: Vec<char>,
    /// The chunk each character belongs to.
    owner: Vec<usize>,
    chunks: Vec<Chunk>,
    chunk_of: HashMap<usize, usize>,
    groups: Vec<Group>,
    links: Vec<Link>,
    fixed: Vec<Fixed>,
    own_insertions: Vec<OwnInsertion>,
    /// The author's own simple deletions, each with its runs.
    own_deletions: HashMap<usize, Vec<usize>>,
    in_own_deletion: HashMap<usize, usize>,
    /// Each run's content elements (everything but `w:rPr`), in order.
    run_children: HashMap<usize, Vec<usize>>,
    /// `w:pPr/w:rPr`: the paragraph mark's formatting.
    mark_properties: Option<usize>,
}

/// Changes the paragraph at node `paragraph` to read `text`, as tracked
/// changes by `context.author` numbered from `first_id`.
pub(super) fn replace(
    part: &Part<'_>,
    paragraph: usize,
    anchor: &str,
    text: &str,
    context: &EditContext,
    first_id: u64,
) -> Result<Replaced, EditError> {
    let model = read(part, paragraph, &context.author)?;
    let wanted = text.replace("\r\n", "\n").replace('\r', "\n");
    let original: String = model.chars.iter().collect();
    for marker in MARKERS {
        if wanted.matches(marker).count() > original.matches(marker).count() {
            return fail(format!(
                "{anchor}: the text contains the reader's marker {:?}; give the paragraph's text as it should read, with no markers (tracked changes by others, hidden text and fields stay where they are)",
                marker.trim_end()
            ));
        }
    }
    let old = &model.chars;
    let lead = old.iter().take_while(|c| c.is_whitespace()).count();
    let trail = old[lead..]
        .iter()
        .rev()
        .take_while(|c| c.is_whitespace())
        .count();
    let mut new: Vec<char> = old[..lead].to_vec();
    new.extend(wanted.trim().chars());
    new.extend_from_slice(&old[old.len() - trail..]);

    let significant: HashSet<usize> = model
        .fixed
        .iter()
        .filter(|fixed| fixed.weight != Weight::Neutral)
        .map(|fixed| fixed.gap)
        .collect();
    let changes = textdiff::changes(old, &new, |range: Range<usize>| {
        !range.clone().any(|index| model.guarded(index).is_some())
            && !(range.start..=range.end).any(|gap| significant.contains(&gap))
    });

    for change in &changes {
        if let Some(index) = change
            .delete
            .clone()
            .find(|index| model.guarded(*index).is_some())
        {
            return Err(model.refusal(anchor, index, false));
        }
    }
    let mut keep = vec![true; old.len()];
    for change in &changes {
        for index in change.delete.clone() {
            keep[index] = false;
        }
    }

    let mut by_gap: BTreeMap<usize, Vec<&OwnInsertion>> = BTreeMap::new();
    for own in &model.own_insertions {
        by_gap.entry(own.gap).or_default().push(own);
    }
    let mut kept_own: HashSet<usize> = HashSet::new();
    let mut placed: Vec<(Spot, Option<usize>, bool, String)> = Vec::new();
    for change in changes.iter().filter(|change| !change.insert.is_empty()) {
        let gap = change.delete.end;
        let insert: String = new[change.insert.clone()].iter().collect();
        let existing: String = by_gap
            .get(&gap)
            .map(|owns| owns.iter().map(|own| own.text.as_str()).collect())
            .unwrap_or_default();
        if !existing.is_empty() && fold_text(&existing) == fold_text(&insert) {
            kept_own.extend(by_gap.get(&gap).into_iter().flatten().map(|own| own.node));
            continue;
        }
        let (spot, run, strip_style) = model.placement(anchor, gap, change.delete.clone())?;
        placed.push((spot, run, strip_style, insert));
    }
    let dropped: HashSet<usize> = model
        .own_insertions
        .iter()
        .map(|own| own.node)
        .filter(|node| !kept_own.contains(node))
        .collect();
    // Whether any character changes state: newly deleted, or restored from
    // the author's own earlier deletion.
    let restates = model.chunks.iter().any(|chunk| {
        (chunk.start..chunk.start + chunk.text.len()).any(|index| match chunk.role {
            Role::Plain => !keep[index],
            Role::OwnDeleted => keep[index],
            Role::Guarded(_) => false,
        })
    });

    let accepted = fold_text(&new.iter().collect::<String>());
    let fixed: Vec<String> = model.fixed.iter().map(|fixed| fixed.name.clone()).collect();
    if placed.is_empty() && dropped.is_empty() && !restates {
        return Ok(Replaced {
            summary: format!("{anchor}: no change; it already reads that"),
            bytes: None,
            accepted,
            rejected: original,
            fixed,
        });
    }

    let mut at: BTreeMap<Spot, Vec<(String, String)>> = BTreeMap::new();
    for (spot, run, strip_style, insert) in placed {
        let properties = model.properties(part, run, strip_style)?;
        at.entry(model.normalize(spot))
            .or_default()
            .push((properties, insert));
    }
    let mut emitter = Emitter::new(part, &model, &keep, at, dropped, context, first_id);
    let inner = emitter.children(model.node)?;
    if !emitter.at.is_empty() {
        return fail(format!(
            "{anchor}: the change could not be placed in the paragraph; nothing was written"
        ));
    }
    let node = &part.tree.nodes[model.node];
    let mut splice = Splice::default();
    if node.is_empty_element() {
        let open = String::from_utf8_lossy(&part.bytes[node.span.start..node.span.end - 2])
            .trim_end()
            .to_string();
        let mut whole = format!("{open}>").into_bytes();
        whole.extend(inner);
        whole.extend(format!("</{}>", node.element.name).into_bytes());
        splice.replace(node.span.clone(), whole);
    } else {
        splice.replace(node.inner.clone(), inner);
    }
    let bytes = splice.apply(part.bytes, part.name)?;

    let summary = if changes.is_empty() {
        format!(
            "{anchor}: its earlier tracked changes are withdrawn; it reads as it did before them"
        )
    } else {
        let described: Vec<String> = changes
            .iter()
            .map(|change| {
                describe(
                    &old[change.delete.clone()].iter().collect::<String>(),
                    &new[change.insert.clone()].iter().collect::<String>(),
                )
            })
            .collect();
        let shown = described.len().min(3);
        let mut list = described[..shown].join(", ");
        if described.len() > shown {
            list.push_str(&format!(" and {} more", described.len() - shown));
        }
        format!(
            "{anchor}: {} tracked change{}, only where the text differs: {list}",
            described.len(),
            if described.len() == 1 { "" } else { "s" }
        )
    };
    Ok(Replaced {
        summary,
        bytes: Some(bytes),
        accepted,
        rejected: original,
        fixed,
    })
}

/// The paragraph's text with `author`'s changes accepted and rejected, and
/// the content the reader does not show.
pub(super) fn views(part: &Part<'_>, paragraph: usize, author: &str) -> Result<Views, EditError> {
    let model = read(part, paragraph, author)?;
    let mut accepted = String::new();
    let mut own = model.own_insertions.iter().peekable();
    for (index, character) in model.chars.iter().enumerate() {
        while let Some(insertion) = own.next_if(|insertion| insertion.gap == index) {
            accepted.push_str(&insertion.text);
        }
        if model.chunks[model.owner[index]].role != Role::OwnDeleted {
            accepted.push(*character);
        }
    }
    for insertion in own {
        accepted.push_str(&insertion.text);
    }
    Ok(Views {
        accepted,
        rejected: model.chars.iter().collect(),
        fixed: model.fixed.iter().map(|fixed| fixed.name.clone()).collect(),
    })
}

/// `"old" → "new"`, `"old" deleted` or `"new" inserted`.
fn describe(old: &str, new: &str) -> String {
    fn excerpt(text: &str) -> String {
        let characters: Vec<char> = text.chars().collect();
        if characters.len() <= 40 {
            format!("{text:?}")
        } else {
            format!(
                "{:?}",
                format!("{}…", characters[..39].iter().collect::<String>())
            )
        }
    }
    match (old.is_empty(), new.is_empty()) {
        (false, false) => format!("{} → {}", excerpt(old), excerpt(new)),
        (false, true) => format!("{} deleted", excerpt(old)),
        _ => format!("{} inserted", excerpt(new)),
    }
}

/// The text of a `w:t` (or any element holding only text), decoded.
pub(super) fn decode_text(inner: &[u8], part: &str, limits: &Limits) -> Result<String, EditError> {
    if !inner.contains(&b'&') && !inner.contains(&b'<') {
        return std::str::from_utf8(inner)
            .map(str::to_string)
            .map_err(|_| EditError {
                op: None,
                message: format!("part {part} is not valid UTF-8"),
            });
    }
    let mut wrapped = Vec::with_capacity(inner.len() + 7);
    wrapped.extend_from_slice(b"<t>");
    wrapped.extend_from_slice(inner);
    wrapped.extend_from_slice(b"</t>");
    let mut text = String::new();
    xml::walk(&wrapped, part, limits, |event| {
        if let XmlEvent::Text(piece) = event {
            text.push_str(&piece);
        }
        Ok(())
    })?;
    Ok(text)
}

// ---- Reading the paragraph ------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct Context {
    link: Option<usize>,
    guard: Option<usize>,
    own_deletion: Option<usize>,
}

#[derive(Debug, Clone)]
struct Field {
    instruction: String,
    in_result: bool,
    /// The group guarding the field's code and, unless the field is a
    /// link, its result. Fields inside a computed field share its group.
    group: usize,
    owns_group: bool,
    link: Option<usize>,
}

struct Walker<'p, 'a> {
    part: &'p Part<'a>,
    author: &'p str,
    order: usize,
    fields: Vec<Field>,
    model: Paragraph,
}

fn read(part: &Part<'_>, paragraph: usize, author: &str) -> Result<Paragraph, EditError> {
    let mut walker = Walker {
        part,
        author,
        order: 0,
        fields: Vec::new(),
        model: Paragraph {
            node: paragraph,
            ..Paragraph::default()
        },
    };
    walker.carry_fields()?;
    walker.model.mark_properties = part
        .tree
        .children(paragraph, "pPr")
        .next()
        .and_then(|properties| part.tree.children(properties, "rPr").next());
    walker.container(paragraph, Context::default())?;
    Ok(walker.model)
}

fn is_on(element: &crate::xml::Element) -> bool {
    !matches!(element.attr("val"), Some("0" | "false" | "off"))
}

/// Whether a run is hidden or white, from its own `w:rPr`, as the reader
/// decides it.
fn run_flags(tree: &Tree, run: usize) -> (bool, bool) {
    let Some(properties) = tree.children(run, "rPr").next() else {
        return (false, false);
    };
    let mut hidden = false;
    let mut white = false;
    for &child in &tree.nodes[properties].children {
        let node = &tree.nodes[child];
        match node.local() {
            "vanish" | "specVanish" if is_on(&node.element) => hidden = true,
            "color" => {
                white = node
                    .element
                    .attr("val")
                    .is_some_and(|value| value.eq_ignore_ascii_case("FFFFFF"));
            }
            _ => {}
        }
    }
    (hidden, white)
}

fn is_link_instruction(instruction: &str) -> bool {
    instruction
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("HYPERLINK"))
}

impl Walker<'_, '_> {
    fn next_order(&mut self) -> usize {
        self.order += 1;
        self.order
    }

    fn decode(&self, node: usize) -> Result<String, EditError> {
        let inner = self.part.tree.nodes[node].inner.clone();
        decode_text(&self.part.bytes[inner], self.part.name, self.part.limits)
    }

    /// Fields a paragraph starts inside (a table of contents, an index):
    /// their begin is in an earlier paragraph.
    fn carry_fields(&mut self) -> Result<(), EditError> {
        let tree = self.part.tree;
        let mut open: Vec<(String, bool)> = Vec::new();
        for index in 0..self.model.node {
            let node = &tree.nodes[index];
            if node.skipped {
                continue;
            }
            match node.local() {
                "fldChar" => match node.element.attr("fldCharType") {
                    Some("begin") => open.push((String::new(), false)),
                    Some("separate") => {
                        if let Some(top) = open.last_mut() {
                            top.1 = true;
                        }
                    }
                    Some("end") => {
                        open.pop();
                    }
                    _ => {}
                },
                "instrText" => {
                    let text = self.decode(index)?;
                    if let Some((instruction, false)) = open.last_mut() {
                        instruction.push_str(&text);
                        instruction.push(' ');
                    }
                }
                _ => {}
            }
        }
        for (instruction, in_result) in open {
            self.open_field(instruction, None);
            if in_result {
                self.enter_result();
            }
        }
        Ok(())
    }

    fn open_field(&mut self, instruction: String, before: Option<Spot>) {
        let shared = self
            .fields
            .iter()
            .find(|field| field.link.is_none())
            .map(|field| field.group);
        let (group, owns_group) = match shared {
            Some(group) => (group, false),
            None => (self.group(Guard::Field, before, None), true),
        };
        self.fields.push(Field {
            instruction,
            in_result: false,
            group,
            owns_group,
            link: None,
        });
    }

    fn enter_result(&mut self) {
        let (group, is_link) = match self.fields.last() {
            Some(field) if !field.in_result => (
                field.group,
                field.owns_group && is_link_instruction(&field.instruction),
            ),
            _ => return,
        };
        let link = if is_link {
            let before = self.model.groups[group].before;
            self.model.links.push(Link {
                before,
                after: None,
                end_order: usize::MAX,
            });
            Some(self.model.links.len() - 1)
        } else {
            None
        };
        if let Some(field) = self.fields.last_mut() {
            field.in_result = true;
            field.link = link;
        }
    }

    fn close_field(&mut self, after: Spot, order: usize) {
        let Some(field) = self.fields.pop() else {
            return;
        };
        if field.owns_group {
            let group = &mut self.model.groups[field.group];
            group.after = Some(after);
            group.end_order = order;
        }
        if let Some(link) = field.link {
            let link = &mut self.model.links[link];
            link.after = Some(after);
            link.end_order = order;
        }
    }

    fn group(&mut self, guard: Guard, before: Option<Spot>, after: Option<Spot>) -> usize {
        self.model.groups.push(Group {
            guard,
            before,
            after,
            end_order: usize::MAX,
        });
        self.model.groups.len() - 1
    }

    /// The guard that already covers content here: an enclosing guarded
    /// element, or a computed field's code or result.
    fn enclosing_guard(&self, context: Context) -> Option<usize> {
        context.guard.or_else(|| {
            self.fields
                .last()
                .filter(|field| !(field.in_result && field.link.is_some()))
                .map(|field| field.group)
        })
    }

    fn link_here(&self, context: Context) -> Option<usize> {
        self.fields
            .last()
            .filter(|field| field.in_result)
            .and_then(|field| field.link)
            .or(context.link)
    }

    fn fixed(&mut self, node: usize, weight: Weight, after: Spot) -> usize {
        let order = self.next_order();
        let name = self.part.tree.nodes[node].local().to_string();
        self.model.fixed.push(Fixed {
            gap: self.model.chars.len(),
            order,
            weight,
            after,
            name,
        });
        order
    }

    #[allow(clippy::too_many_arguments)]
    fn chunk(
        &mut self,
        node: usize,
        run: Option<usize>,
        child: usize,
        text: &str,
        role: Role,
        splittable: bool,
        link: Option<usize>,
    ) -> usize {
        let start = self.model.chars.len();
        let index = self.model.chunks.len();
        let characters: Vec<char> = text.chars().collect();
        self.model
            .owner
            .extend(std::iter::repeat_n(index, characters.len()));
        self.model.chars.extend(characters.iter().copied());
        self.model.chunk_of.insert(node, index);
        let order = self.next_order();
        self.model.chunks.push(Chunk {
            node,
            run,
            child,
            text: characters,
            start,
            role,
            splittable,
            link,
            order,
        });
        order
    }

    fn container(&mut self, node: usize, context: Context) -> Result<(), EditError> {
        let tree = self.part.tree;
        for &child in &tree.nodes[node].children {
            let child_node = &tree.nodes[child];
            if child_node.skipped || child_node.local().ends_with("Pr") {
                continue;
            }
            match child_node.local() {
                "r" => self.run(child, context)?,
                "hyperlink" => {
                    self.model.links.push(Link {
                        before: Some(Spot::Before(child)),
                        after: Some(Spot::After(child)),
                        end_order: usize::MAX,
                    });
                    let link = self.model.links.len() - 1;
                    self.container(
                        child,
                        Context {
                            link: Some(link),
                            ..context
                        },
                    )?;
                    self.model.links[link].end_order = self.next_order();
                }
                "smartTag" | "customXml" | "sdt" | "sdtContent" | "dir" | "bdo" => {
                    self.container(child, context)?;
                }
                "fldSimple" => self.simple_field(child, context)?,
                "ins" | "del" | "moveTo" | "moveFrom" => self.revision(child, context)?,
                _ => self.element(child, context, None)?,
            }
        }
        Ok(())
    }

    fn simple_field(&mut self, node: usize, context: Context) -> Result<(), EditError> {
        let instruction = self.part.tree.nodes[node]
            .element
            .attr("instr")
            .unwrap_or_default();
        if let Some(group) = self.enclosing_guard(context) {
            return self.container(
                node,
                Context {
                    guard: Some(group),
                    ..context
                },
            );
        }
        if is_link_instruction(instruction) {
            self.model.links.push(Link {
                before: Some(Spot::Before(node)),
                after: Some(Spot::After(node)),
                end_order: usize::MAX,
            });
            let link = self.model.links.len() - 1;
            self.container(
                node,
                Context {
                    link: Some(link),
                    ..context
                },
            )?;
            self.model.links[link].end_order = self.next_order();
            return Ok(());
        }
        let group = self.group(
            Guard::Field,
            Some(Spot::Before(node)),
            Some(Spot::After(node)),
        );
        self.container(
            node,
            Context {
                guard: Some(group),
                ..context
            },
        )?;
        self.model.groups[group].end_order = self.next_order();
        Ok(())
    }

    /// A tracked change the author owns can be revised only when it is
    /// plain: runs of text and nothing else.
    fn simple(&self, wrapper: usize) -> bool {
        let tree = self.part.tree;
        tree.nodes[wrapper].children.iter().all(|&child| {
            let node = &tree.nodes[child];
            if node.skipped || node.local() != "r" {
                return false;
            }
            let (hidden, white) = run_flags(tree, child);
            !hidden
                && !white
                && node.children.iter().all(|&content| {
                    let content = &tree.nodes[content];
                    !content.skipped
                        && matches!(
                            content.local(),
                            "rPr" | "t" | "delText" | "tab" | "br" | "cr" | "noBreakHyphen"
                        )
                })
        })
    }

    fn simple_text(&self, wrapper: usize) -> Result<String, EditError> {
        let tree = self.part.tree;
        let mut text = String::new();
        for &run in &tree.nodes[wrapper].children {
            for &content in &tree.nodes[run].children {
                match tree.nodes[content].local() {
                    "t" | "delText" => text.push_str(&self.decode(content)?),
                    "tab" => text.push('\t'),
                    "br" | "cr" => text.push(' '),
                    "noBreakHyphen" => text.push('-'),
                    _ => {}
                }
            }
        }
        Ok(text)
    }

    fn revision(&mut self, node: usize, context: Context) -> Result<(), EditError> {
        let tree = self.part.tree;
        let element = &tree.nodes[node].element;
        let local = element.local();
        let author = element.attr("author").unwrap_or("unknown").to_string();
        let guard = self.enclosing_guard(context);
        let own = matches!(local, "ins" | "del")
            && author == self.author
            && guard.is_none()
            && context.own_deletion.is_none()
            && self.simple(node);
        match (local, own) {
            ("ins", true) => {
                let text = self.simple_text(node)?;
                self.next_order();
                self.model.own_insertions.push(OwnInsertion {
                    node,
                    gap: self.model.chars.len(),
                    text,
                });
            }
            ("del", true) => {
                let runs: Vec<usize> = tree.nodes[node].children.clone();
                for &run in &runs {
                    self.model.in_own_deletion.insert(run, node);
                }
                self.model.own_deletions.insert(node, runs.clone());
                for run in runs {
                    self.run(
                        run,
                        Context {
                            own_deletion: Some(node),
                            ..context
                        },
                    )?;
                }
            }
            ("ins" | "moveTo", false) => {
                let group = match guard {
                    Some(group) => group,
                    None => self.group(
                        Guard::Tracked(author),
                        Some(Spot::Before(node)),
                        Some(Spot::After(node)),
                    ),
                };
                self.container(
                    node,
                    Context {
                        guard: Some(group),
                        ..context
                    },
                )?;
                if guard.is_none() {
                    self.model.groups[group].end_order = self.next_order();
                }
            }
            _ => {
                self.fixed(node, Weight::Significant, Spot::After(node));
            }
        }
        Ok(())
    }

    fn run(&mut self, run: usize, context: Context) -> Result<(), EditError> {
        let tree = self.part.tree;
        let (hidden, white) = run_flags(tree, run);
        let content: Vec<usize> = tree.nodes[run]
            .children
            .iter()
            .copied()
            .filter(|child| {
                let node = &tree.nodes[*child];
                !node.skipped && node.local() != "rPr"
            })
            .collect();
        self.model.run_children.insert(run, content.clone());
        for (index, &child) in content.iter().enumerate() {
            let before = Spot::InRun {
                run,
                child: index,
                offset: 0,
            };
            let after = Spot::InRun {
                run,
                child: index + 1,
                offset: 0,
            };
            let local = tree.nodes[child].local();
            match local {
                "t" | "delText" | "tab" | "br" | "cr" | "noBreakHyphen" => {
                    let text = match local {
                        "t" | "delText" => self.decode(child)?,
                        "tab" => "\t".to_string(),
                        "noBreakHyphen" => "-".to_string(),
                        _ => " ".to_string(),
                    };
                    if text.is_empty() {
                        self.fixed(child, Weight::Neutral, after);
                        continue;
                    }
                    if hidden || (white && !text.trim().is_empty()) {
                        self.fixed(child, Weight::Significant, after);
                        continue;
                    }
                    let link = self.link_here(context);
                    let (role, stray) = match self.enclosing_guard(context) {
                        Some(group) => (Role::Guarded(group), false),
                        None if context.own_deletion.is_some() => (Role::OwnDeleted, false),
                        // Deleted text outside a deletion: shown as text,
                        // and not this op's to change.
                        None if local == "delText" => (
                            Role::Guarded(self.group(
                                Guard::Element("delText".into()),
                                Some(before),
                                Some(after),
                            )),
                            true,
                        ),
                        None => (Role::Plain, false),
                    };
                    let order = self.chunk(
                        child,
                        Some(run),
                        index,
                        &text,
                        role,
                        matches!(local, "t" | "delText"),
                        link,
                    );
                    if let (true, Role::Guarded(group)) = (stray, role) {
                        self.model.groups[group].end_order = order;
                    }
                }
                "fldChar" => {
                    let order = self.fixed(child, Weight::Significant, after);
                    match tree.nodes[child].element.attr("fldCharType") {
                        Some("begin") => self.open_field(String::new(), Some(before)),
                        Some("separate") => self.enter_result(),
                        Some("end") => self.close_field(after, order),
                        _ => {}
                    }
                }
                "instrText" => {
                    let text = self.decode(child)?;
                    if let Some(field) = self.fields.last_mut()
                        && !field.in_result
                    {
                        field.instruction.push_str(&text);
                        field.instruction.push(' ');
                    }
                    self.fixed(child, Weight::Neutral, after);
                }
                "footnoteReference" | "endnoteReference" => {
                    self.fixed(child, Weight::Attach, after);
                }
                _ => self.element(child, context, Some((run, index)))?,
            }
        }
        Ok(())
    }

    /// Any other element: zero-width when the reader finds no text in it,
    /// and otherwise text that cannot change.
    fn element(
        &mut self,
        node: usize,
        context: Context,
        in_run: Option<(usize, usize)>,
    ) -> Result<(), EditError> {
        let local = self.part.tree.nodes[node].local().to_string();
        let (before, after) = match in_run {
            Some((run, index)) => (
                Spot::InRun {
                    run,
                    child: index,
                    offset: 0,
                },
                Spot::InRun {
                    run,
                    child: index + 1,
                    offset: 0,
                },
            ),
            None => (Spot::Before(node), Spot::After(node)),
        };
        let text = self.element_text(node)?;
        if text.is_empty() {
            let weight = if NEUTRAL.contains(&local.as_str()) {
                Weight::Neutral
            } else {
                Weight::Significant
            };
            self.fixed(node, weight, after);
            return Ok(());
        }
        let enclosing = self.enclosing_guard(context);
        let group = match enclosing {
            Some(group) => group,
            None => self.group(Guard::Element(local), Some(before), Some(after)),
        };
        let link = self.link_here(context);
        let order = self.chunk(
            node,
            in_run.map(|(run, _)| run),
            in_run.map_or(0, |(_, index)| index),
            &text,
            Role::Guarded(group),
            false,
            link,
        );
        if enclosing.is_none() {
            self.model.groups[group].end_order = order;
        }
        Ok(())
    }

    /// The text the reader attributes to this paragraph from inside an
    /// element: nested paragraphs (a text box) are their own.
    fn element_text(&self, node: usize) -> Result<String, EditError> {
        let tree = self.part.tree;
        let end = tree.nodes[node].span.end;
        let mut text = String::new();
        let mut skip_until = 0usize;
        for index in node + 1..tree.nodes.len() {
            let descendant = &tree.nodes[index];
            if descendant.span.start >= end {
                break;
            }
            if descendant.skipped || descendant.span.start < skip_until {
                continue;
            }
            match descendant.local() {
                "p" => skip_until = descendant.span.end,
                "t" | "delText" => text.push_str(&self.decode(index)?),
                "tab" => text.push('\t'),
                "br" | "cr" => text.push(' '),
                "noBreakHyphen" => text.push('-'),
                _ => {}
            }
        }
        Ok(text)
    }
}

// ---- Placing the change ----------------------------------------------------------

impl Paragraph {
    fn guarded(&self, index: usize) -> Option<usize> {
        match self.chunks[self.owner[index]].role {
            Role::Guarded(group) => Some(group),
            _ => None,
        }
    }

    fn chunk_at(&self, index: usize) -> &Chunk {
        &self.chunks[self.owner[index]]
    }

    fn refusal(&self, anchor: &str, index: usize, inside: bool) -> EditError {
        let Some(group) = self.guarded(index) else {
            return EditError {
                op: None,
                message: format!("{anchor}: the change cannot be placed there"),
            };
        };
        let text: String = (0..self.chars.len())
            .filter(|at| self.guarded(*at) == Some(group))
            .map(|at| self.chars[at])
            .collect();
        let text = text.trim();
        let text = if text.chars().count() > 60 {
            format!("{}…", text.chars().take(59).collect::<String>())
        } else {
            text.to_string()
        };
        let what = match &self.groups[group].guard {
            Guard::Field => {
                "a field's result that Word computes (a cross-reference, page number or date)"
                    .to_string()
            }
            Guard::Tracked(author) => {
                format!("part of a tracked change by {author} that this op cannot accept or reject")
            }
            Guard::Element(name) => match name.as_str() {
                "oMath" | "oMathPara" => "an equation".to_string(),
                "drawing" | "pict" | "object" => "text inside a drawing".to_string(),
                _ => "content this op does not edit".to_string(),
            },
        };
        let message = if inside {
            format!(
                "{anchor}: the new text would go inside {text:?}, {what}; put it before or after that text instead"
            )
        } else {
            format!(
                "{anchor}: the change would alter {text:?}, {what}; keep that text exactly as it is (the rest of the paragraph can change)"
            )
        };
        EditError { op: None, message }
    }

    /// Where an insertion at `gap` goes, the run whose formatting it takes,
    /// and whether to drop that run's character style (the new text is
    /// outside the link the style belongs to). `deleted` is the text it
    /// replaces, if any.
    fn placement(
        &self,
        anchor: &str,
        gap: usize,
        deleted: Range<usize>,
    ) -> Result<(Spot, Option<usize>, bool), EditError> {
        if !deleted.is_empty() {
            // A replacement goes where the text it replaces ends and looks
            // like most of that text.
            let last = deleted.end - 1;
            let mut counts: Vec<(usize, usize)> = Vec::new();
            for index in deleted.clone() {
                if let Some(run) = self.chunk_at(index).run {
                    match counts.iter_mut().find(|(known, _)| *known == run) {
                        Some((_, count)) => *count += 1,
                        None => counts.push((run, 1)),
                    }
                }
            }
            let dominant = counts
                .iter()
                .fold(
                    None,
                    |best: Option<(usize, usize)>, &(run, count)| match best {
                        Some((_, most)) if most >= count => best,
                        _ => Some((run, count)),
                    },
                )
                .map(|(run, _)| run);
            let dominant_link = deleted
                .clone()
                .find(|index| self.chunk_at(*index).run == dominant)
                .and_then(|index| self.chunk_at(index).link);
            let strip = dominant_link != self.chunk_at(last).link;
            return Ok((self.after_char(last), dominant, strip));
        }
        if gap > 0 {
            let left = gap - 1;
            let chunk = self.chunk_at(left);
            if let Role::Guarded(group) = chunk.role {
                if gap < self.chars.len() && self.guarded(gap) == Some(group) {
                    return Err(self.refusal(anchor, left, true));
                }
                let group = &self.groups[group];
                let Some(after) = group.after else {
                    return Err(self.refusal(anchor, left, true));
                };
                return Ok((
                    self.skip_attached(gap, after, group.end_order),
                    chunk.run.or_else(|| self.nearest_run(gap)),
                    false,
                ));
            }
            let at_link_end = chunk
                .link
                .filter(|link| gap == self.chars.len() || self.chunk_at(gap).link != Some(*link));
            if let Some(link) = at_link_end
                && let Some(after) = self.links[link].after
            {
                return Ok((
                    self.skip_attached(gap, after, self.links[link].end_order),
                    chunk.run,
                    true,
                ));
            }
            return Ok((
                self.skip_attached(gap, self.after_char(left), chunk.order),
                chunk.run,
                false,
            ));
        }
        if self.chars.is_empty() {
            return Ok((Spot::End, None, false));
        }
        let first = self.chunk_at(0);
        if let Role::Guarded(group) = first.role {
            let Some(before) = self.groups[group].before else {
                return Err(self.refusal(anchor, 0, true));
            };
            return Ok((before, first.run.or_else(|| self.nearest_run(0)), false));
        }
        if let Some(link) = first.link
            && let Some(before) = self.links[link].before
        {
            return Ok((before, first.run, true));
        }
        Ok((self.before_char(0), first.run, false))
    }

    /// After a footnote or endnote mark that follows the text at `gap`, so
    /// new text never separates a mark from the sentence it annotates.
    fn skip_attached(&self, gap: usize, base: Spot, base_order: usize) -> Spot {
        let mut following: Vec<&Fixed> = self
            .fixed
            .iter()
            .filter(|fixed| fixed.gap == gap && fixed.order > base_order)
            .collect();
        following.sort_by_key(|fixed| fixed.order);
        let mut spot = base;
        for fixed in following {
            match fixed.weight {
                Weight::Attach => spot = fixed.after,
                Weight::Neutral => {}
                Weight::Significant => break,
            }
        }
        spot
    }

    /// The nearest run with editable text, for formatting new text next to
    /// something that has none of its own.
    fn nearest_run(&self, gap: usize) -> Option<usize> {
        let editable = |index: &usize| {
            let chunk = self.chunk_at(*index);
            (!matches!(chunk.role, Role::Guarded(_)))
                .then_some(chunk.run)
                .flatten()
        };
        (0..gap)
            .rev()
            .find_map(|index| editable(&index))
            .or_else(|| (gap..self.chars.len()).find_map(|index| editable(&index)))
    }

    fn after_char(&self, index: usize) -> Spot {
        let chunk = self.chunk_at(index);
        let Some(run) = chunk.run else {
            return Spot::After(chunk.node);
        };
        let offset = index - chunk.start + 1;
        if chunk.splittable && offset < chunk.text.len() {
            Spot::InRun {
                run,
                child: chunk.child,
                offset,
            }
        } else {
            Spot::InRun {
                run,
                child: chunk.child + 1,
                offset: 0,
            }
        }
    }

    fn before_char(&self, index: usize) -> Spot {
        let chunk = self.chunk_at(index);
        let Some(run) = chunk.run else {
            return Spot::Before(chunk.node);
        };
        Spot::InRun {
            run,
            child: chunk.child,
            offset: if chunk.splittable {
                index - chunk.start
            } else {
                0
            },
        }
    }

    /// The same place, named so that nothing is split that need not be: a
    /// run's edge becomes a place beside the run (or beside the author's
    /// deletion the run is the edge of).
    fn normalize(&self, spot: Spot) -> Spot {
        let Spot::InRun { run, child, offset } = spot else {
            return spot;
        };
        let Some(children) = self.run_children.get(&run) else {
            return spot;
        };
        let length = |child: usize| {
            children
                .get(child)
                .and_then(|node| self.chunk_of.get(node))
                .map_or(0, |chunk| self.chunks[*chunk].text.len())
        };
        let (child, offset) = if offset > 0 && offset >= length(child) {
            (child + 1, 0)
        } else {
            (child, offset)
        };
        let at_start = child == 0 && offset == 0;
        let at_end = child >= children.len();
        match self.in_own_deletion.get(&run) {
            None if at_start => Spot::Before(run),
            None if at_end => Spot::After(run),
            Some(wrapper) => {
                let runs = self.own_deletions.get(wrapper);
                if at_start && runs.and_then(|runs| runs.first()) == Some(&run) {
                    Spot::Before(*wrapper)
                } else if at_end && runs.and_then(|runs| runs.last()) == Some(&run) {
                    Spot::After(*wrapper)
                } else {
                    Spot::InRun { run, child, offset }
                }
            }
            None => Spot::InRun { run, child, offset },
        }
    }

    /// Formatting for new text: `run`'s own, less anything that would hide
    /// it or claim a revision, or the paragraph mark's when there is no run.
    fn properties(
        &self,
        part: &Part<'_>,
        run: Option<usize>,
        strip_style: bool,
    ) -> Result<String, EditError> {
        let properties = match run {
            Some(run) => part.tree.children(run, "rPr").next(),
            None => self.mark_properties,
        };
        let Some(properties) = properties else {
            return Ok(String::new());
        };
        let node = &part.tree.nodes[properties];
        if node.is_empty_element() {
            return Ok(String::new());
        }
        let mut splice = Splice::default();
        let mut kept = 0usize;
        for &child in &node.children {
            let child = &part.tree.nodes[child];
            let drop = matches!(
                child.local(),
                "rPrChange"
                    | "ins"
                    | "del"
                    | "moveFrom"
                    | "moveTo"
                    | "vanish"
                    | "specVanish"
                    | "webHidden"
            ) || (strip_style && child.local() == "rStyle")
                || (child.local() == "color"
                    && child
                        .element
                        .attr("val")
                        .is_some_and(|value| value.eq_ignore_ascii_case("FFFFFF")));
            if drop {
                splice.replace(
                    child.span.start - node.span.start..child.span.end - node.span.start,
                    "",
                );
            } else {
                kept += 1;
            }
        }
        if kept == 0 {
            return Ok(String::new());
        }
        let bytes = splice.apply(&part.bytes[node.span.clone()], part.name)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

// ---- Writing the change ----------------------------------------------------------

enum Segment {
    Run {
        deleted: bool,
        xml: Vec<u8>,
    },
    /// New text, written when the run is, so revision ids follow the
    /// document's order.
    Inserted(Vec<(String, String)>),
}

struct Emitter<'p, 'a> {
    part: &'p Part<'a>,
    model: &'p Paragraph,
    keep: &'p [bool],
    /// Units rewritten in full: plain runs, and the author's deletions.
    rewrite: HashSet<usize>,
    /// Containers re-emitted around a change, copying what did not change.
    open: HashSet<usize>,
    dropped: HashSet<usize>,
    at: BTreeMap<Spot, Vec<(String, String)>>,
    next_id: u64,
    author: &'p str,
    date: &'p str,
}

impl<'p, 'a> Emitter<'p, 'a> {
    fn new(
        part: &'p Part<'a>,
        model: &'p Paragraph,
        keep: &'p [bool],
        at: BTreeMap<Spot, Vec<(String, String)>>,
        dropped: HashSet<usize>,
        context: &'p EditContext,
        first_id: u64,
    ) -> Self {
        let unit = |run: usize| model.in_own_deletion.get(&run).copied().unwrap_or(run);
        let mut rewrite = HashSet::new();
        for chunk in &model.chunks {
            let changed =
                (chunk.start..chunk.start + chunk.text.len()).any(|index| match chunk.role {
                    Role::Plain => !keep[index],
                    Role::OwnDeleted => keep[index],
                    Role::Guarded(_) => false,
                });
            if changed && let Some(run) = chunk.run {
                rewrite.insert(unit(run));
            }
        }
        for spot in at.keys() {
            if let Spot::InRun { run, .. } = spot {
                rewrite.insert(unit(*run));
            }
        }
        /// Marks the containers above `node`, up to the paragraph.
        fn open_above(tree: &Tree, paragraph: usize, open: &mut HashSet<usize>, node: usize) {
            let mut current = tree.nodes[node].parent;
            while let Some(ancestor) = current {
                if !open.insert(ancestor) || ancestor == paragraph {
                    break;
                }
                current = tree.nodes[ancestor].parent;
            }
        }
        let tree = part.tree;
        let mut open = HashSet::new();
        for &unit in &rewrite {
            open_above(tree, model.node, &mut open, unit);
        }
        for &node in &dropped {
            open_above(tree, model.node, &mut open, node);
        }
        for spot in at.keys() {
            match spot {
                Spot::Before(node) | Spot::After(node) => {
                    open_above(tree, model.node, &mut open, *node);
                }
                Spot::End => {
                    open.insert(model.node);
                }
                Spot::InRun { .. } => {}
            }
        }
        Self {
            part,
            model,
            keep,
            rewrite,
            open,
            dropped,
            at,
            next_id: first_id,
            author: &context.author,
            date: &context.date,
        }
    }

    fn revision(&mut self) -> String {
        let id = self.next_id;
        self.next_id += 1;
        let w = self.part.w;
        format!(
            r#" {w}id="{id}" {w}author="{}" {w}date="{}""#,
            escape_attr(self.author),
            escape_attr(self.date)
        )
    }

    fn insertions(&mut self, spot: Spot, out: &mut Vec<u8>) {
        if let Some(list) = self.at.remove(&spot) {
            self.render(list, out);
        }
    }

    fn render(&mut self, list: Vec<(String, String)>, out: &mut Vec<u8>) {
        for (properties, text) in list {
            let w = self.part.w;
            let revision = self.revision();
            out.extend(
                format!(
                    "<{w}ins{revision}><{w}r>{properties}{}</{w}r></{w}ins>",
                    run_content(w, &text)
                )
                .into_bytes(),
            );
        }
    }

    fn children(&mut self, node: usize) -> Result<Vec<u8>, EditError> {
        let tree = self.part.tree;
        let bytes = self.part.bytes;
        let parent = &tree.nodes[node];
        let mut out = Vec::new();
        let mut cursor = parent.inner.start;
        for &child in &parent.children {
            let span = tree.nodes[child].span.clone();
            out.extend_from_slice(&bytes[cursor..span.start]);
            self.insertions(Spot::Before(child), &mut out);
            out.extend(self.node(child)?);
            self.insertions(Spot::After(child), &mut out);
            cursor = span.end;
        }
        out.extend_from_slice(&bytes[cursor.max(parent.inner.start)..parent.inner.end]);
        if node == self.model.node {
            self.insertions(Spot::End, &mut out);
        }
        Ok(out)
    }

    fn node(&mut self, node: usize) -> Result<Vec<u8>, EditError> {
        if self.dropped.contains(&node) {
            return Ok(Vec::new());
        }
        if self.rewrite.contains(&node) {
            return if self.model.own_deletions.contains_key(&node) {
                self.own_deletion(node)
            } else {
                self.plain_run(node)
            };
        }
        let element = &self.part.tree.nodes[node];
        if self.open.contains(&node) && !element.is_empty_element() {
            let mut out = self.part.bytes[element.span.start..element.inner.start].to_vec();
            out.extend(self.children(node)?);
            out.extend_from_slice(&self.part.bytes[element.inner.end..element.span.end]);
            return Ok(out);
        }
        Ok(self.part.bytes[element.span.clone()].to_vec())
    }

    fn close(&self, local: &str) -> Vec<u8> {
        format!("</{}{local}>", self.part.w).into_bytes()
    }

    fn plain_run(&mut self, run: usize) -> Result<Vec<u8>, EditError> {
        let mut out = Vec::new();
        for segment in self.segments(run)? {
            match segment {
                Segment::Run { deleted: true, xml } => {
                    let revision = self.revision();
                    out.extend(format!("<{}del{revision}>", self.part.w).into_bytes());
                    out.extend(xml);
                    out.extend(self.close("del"));
                }
                Segment::Run { xml, .. } => out.extend(xml),
                Segment::Inserted(list) => self.render(list, &mut out),
            }
        }
        Ok(out)
    }

    fn own_deletion(&mut self, wrapper: usize) -> Result<Vec<u8>, EditError> {
        let runs = self
            .model
            .own_deletions
            .get(&wrapper)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        let mut open = false;
        for run in runs {
            for segment in self.segments(run)? {
                match segment {
                    Segment::Run { deleted: true, xml } => {
                        if !open {
                            let revision = self.revision();
                            out.extend(format!("<{}del{revision}>", self.part.w).into_bytes());
                            open = true;
                        }
                        out.extend(xml);
                    }
                    Segment::Run { xml, .. } => {
                        if open {
                            out.extend(self.close("del"));
                            open = false;
                        }
                        out.extend(xml);
                    }
                    Segment::Inserted(list) => {
                        if open {
                            out.extend(self.close("del"));
                            open = false;
                        }
                        self.render(list, &mut out);
                    }
                }
            }
        }
        if open {
            out.extend(self.close("del"));
        }
        Ok(out)
    }

    /// A run cut where its characters change state or new text goes in:
    /// every piece keeps the run's own start tag and formatting.
    fn segments(&mut self, run: usize) -> Result<Vec<Segment>, EditError> {
        let tree = self.part.tree;
        let bytes = self.part.bytes;
        let model: &'p Paragraph = self.model;
        let keep: &'p [bool] = self.keep;
        let node = &tree.nodes[run];
        let open_tag = &bytes[node.span.start..node.inner.start];
        let close_tag = &bytes[node.inner.end..node.span.end];
        let properties: &[u8] = tree
            .children(run, "rPr")
            .next()
            .map(|properties| &bytes[tree.nodes[properties].span.clone()])
            .unwrap_or_default();
        let wrap = |deleted: bool, pieces: Vec<u8>| {
            let mut xml = open_tag.to_vec();
            xml.extend_from_slice(properties);
            xml.extend(pieces);
            xml.extend_from_slice(close_tag);
            Segment::Run { deleted, xml }
        };
        let children = model.run_children.get(&run).cloned().unwrap_or_default();
        let base_deleted = model.in_own_deletion.contains_key(&run);
        let mut segments = Vec::new();
        let mut current: Option<(bool, Vec<u8>)> = None;
        let push = |current: &mut Option<(bool, Vec<u8>)>,
                    segments: &mut Vec<Segment>,
                    deleted: bool,
                    xml: Vec<u8>| {
            match current {
                Some((state, pieces)) if *state == deleted => pieces.extend(xml),
                _ => {
                    if let Some((state, pieces)) = current.take() {
                        segments.push(wrap(state, pieces));
                    }
                    *current = Some((deleted, xml));
                }
            }
        };
        for (index, &child) in children.iter().enumerate() {
            self.split(run, index, 0, &mut current, &mut segments, &wrap);
            let chunk = model
                .chunk_of
                .get(&child)
                .map(|chunk| &model.chunks[*chunk])
                .filter(|chunk| !matches!(chunk.role, Role::Guarded(_)));
            let Some(chunk) = chunk else {
                push(
                    &mut current,
                    &mut segments,
                    base_deleted,
                    bytes[tree.nodes[child].span.clone()].to_vec(),
                );
                continue;
            };
            let deleted: Vec<bool> = (0..chunk.text.len())
                .map(|offset| !keep[chunk.start + offset])
                .collect();
            if !chunk.splittable {
                push(
                    &mut current,
                    &mut segments,
                    deleted.first().copied().unwrap_or(base_deleted),
                    bytes[tree.nodes[child].span.clone()].to_vec(),
                );
                continue;
            }
            let mut from = 0;
            for to in 1..=chunk.text.len() {
                let split_here = to < chunk.text.len()
                    && self.at.contains_key(&Spot::InRun {
                        run,
                        child: index,
                        offset: to,
                    });
                if to == chunk.text.len() || deleted[to] != deleted[from] || split_here {
                    let piece = self.text_piece(child, chunk, from..to, deleted[from]);
                    push(&mut current, &mut segments, deleted[from], piece);
                    if split_here {
                        self.split(run, index, to, &mut current, &mut segments, &wrap);
                    }
                    from = to;
                }
            }
        }
        self.split(run, children.len(), 0, &mut current, &mut segments, &wrap);
        if let Some((state, pieces)) = current.take() {
            segments.push(wrap(state, pieces));
        }
        Ok(segments)
    }

    /// New text inside a run: closes the piece before it.
    fn split(
        &mut self,
        run: usize,
        child: usize,
        offset: usize,
        current: &mut Option<(bool, Vec<u8>)>,
        segments: &mut Vec<Segment>,
        wrap: &dyn Fn(bool, Vec<u8>) -> Segment,
    ) {
        let Some(list) = self.at.remove(&Spot::InRun { run, child, offset }) else {
            return;
        };
        if let Some((state, pieces)) = current.take() {
            segments.push(wrap(state, pieces));
        }
        segments.push(Segment::Inserted(list));
    }

    /// Characters `range` of a `w:t` or `w:delText`, as the element their
    /// state calls for: the original bytes when nothing about it changes.
    fn text_piece(
        &self,
        node: usize,
        chunk: &Chunk,
        range: Range<usize>,
        deleted: bool,
    ) -> Vec<u8> {
        let element = &self.part.tree.nodes[node].element;
        let wanted = if deleted { "delText" } else { "t" };
        if range == (0..chunk.text.len()) && element.local() == wanted {
            return self.part.bytes[self.part.tree.nodes[node].span.clone()].to_vec();
        }
        let prefix = element
            .name
            .strip_suffix(element.local())
            .unwrap_or_default();
        let name = format!("{prefix}{wanted}");
        let mut attributes: Vec<(String, String)> = element
            .attributes
            .iter()
            .filter(|(key, _)| key != "xml:space")
            .cloned()
            .collect();
        attributes.push(("xml:space".into(), "preserve".into()));
        let text: String = chunk.text[range].iter().collect();
        format!(
            "{}{}</{name}>",
            start_tag(&name, &attributes, false),
            escape_text(&text)
        )
        .into_bytes()
    }
}
