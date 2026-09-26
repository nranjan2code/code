//! Word ops. Every change to an existing document is a native tracked
//! change under the runtime-supplied author (docs/design/72, R7), so it
//! survives into Word as a redline a person can accept or reject there. A
//! paragraph's new text is written as a redline of only the words that
//! differ ([`super::redline`]).

use std::io::{Read, Seek};

use super::{EditContext, EditError, Expect, Outcome, Work, fail, redline};
use crate::Limits;
use crate::splice::{Splice, Tree, escape_attr, escape_text, start_tag};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const W_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// Markup inside a paragraph that `delete_paragraph` will not rewrite:
/// existing revisions, fields and text boxes. The error says what to do
/// instead.
const REFUSED_INSIDE: &[&str] = &[
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    "fldChar",
    "fldSimple",
    "instrText",
    "txbxContent",
];

struct Doc {
    part: String,
    bytes: Vec<u8>,
    tree: Tree,
    w: String,
}

fn load<R: Read + Seek>(work: &mut Work<'_, R>) -> Result<Doc, EditError> {
    let part = work.main_part();
    refuse_protected(work, &part)?;
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let Some(w) = tree.prefix_for(W).or_else(|| tree.prefix_for(W_STRICT)) else {
        return fail("the document does not declare the WordprocessingML namespace on its root");
    };
    Ok(Doc {
        part,
        bytes,
        tree,
        w,
    })
}

/// Document protection (O8). Restricted editing that still allows tracked
/// changes permits these ops, since every one of them is a tracked change;
/// read-only, comments-only and forms-only protection refuse them.
fn refuse_protected<R: Read + Seek>(work: &mut Work<'_, R>, main: &str) -> Result<(), EditError> {
    let Some(settings) = work.related(main, "settings")? else {
        return Ok(());
    };
    let bytes = work.get(&settings)?;
    let tree = Tree::parse(&bytes, &settings, work.limits())?;
    let Some(protection) = tree.descendants(0, "documentProtection").next() else {
        return Ok(());
    };
    let element = &tree.nodes[protection].element;
    let enforced = element
        .attr("enforcement")
        .is_some_and(|value| matches!(value, "1" | "true" | "on"));
    if enforced && element.attr("edit") != Some("trackedChanges") {
        return fail(format!(
            "the document is protected ({} editing only); the owner must remove the protection before it can be edited",
            element.attr("edit").unwrap_or("no")
        ));
    }
    Ok(())
}

/// Paragraph anchors exactly as `read` assigns them: `p:<paraId>` when the
/// paragraph has one, otherwise `p@<n>` counting paragraphs without one.
fn paragraphs(tree: &Tree) -> Vec<(String, usize)> {
    let mut ordinal = 0usize;
    tree.descendants(0, "p")
        .map(|index| {
            let anchor = match tree.nodes[index].element.attr("paraId") {
                Some(id) => format!("p:{id}"),
                None => {
                    ordinal += 1;
                    format!("p@{ordinal}")
                }
            };
            (anchor, index)
        })
        .collect()
}

fn find(doc: &Doc, anchor: &str) -> Result<usize, EditError> {
    let all = paragraphs(&doc.tree);
    if let Some((_, index)) = all.iter().find(|(candidate, _)| candidate == anchor) {
        return Ok(*index);
    }
    Err(EditError {
        op: None,
        message: format!(
            "no paragraph {anchor}; use an anchor from doc_read on this exact file (anchors look like p:1A2B3C4D or p@12){}",
            suggestion(doc, &all, anchor)
        ),
    })
}

/// When a model names a paragraph by its text instead of its anchor
/// (`p:Steady.`), the anchors of the paragraphs that text points at, so
/// the call can be repaired without another read.
fn suggestion(doc: &Doc, all: &[(String, usize)], anchor: &str) -> String {
    let needle = anchor
        .strip_prefix("p:")
        .or_else(|| anchor.strip_prefix("p@"))
        .unwrap_or(anchor)
        .trim();
    if needle.is_empty() {
        return String::new();
    }
    let lower = needle.to_lowercase();
    let texts: Vec<(&str, String)> = all
        .iter()
        .map(|(candidate, index)| (candidate.as_str(), paragraph_text(doc, *index)))
        .collect();
    let exact: Vec<&str> = texts
        .iter()
        .filter(|(_, text)| text.trim().to_lowercase() == lower)
        .map(|(candidate, _)| *candidate)
        .collect();
    if !exact.is_empty() {
        return format!(
            ". The paragraph whose text is {needle:?} is {}",
            exact.join(", ")
        );
    }
    if needle.chars().count() < 4 {
        return String::new();
    }
    let containing: Vec<&str> = texts
        .iter()
        .filter(|(_, text)| text.to_lowercase().contains(&lower))
        .map(|(candidate, _)| *candidate)
        .take(4)
        .collect();
    match containing.len() {
        0 => String::new(),
        1..=3 => format!(
            ". Paragraphs containing {needle:?}: {}",
            containing.join(", ")
        ),
        _ => format!(
            ". Several paragraphs contain {needle:?}, among them {}; read the file to choose",
            containing[..3].join(", ")
        ),
    }
}

/// A paragraph's text from its `w:t` elements, for suggesting an anchor.
fn paragraph_text(doc: &Doc, paragraph: usize) -> String {
    let limits = Limits::default();
    doc.tree
        .descendants(paragraph, "t")
        .filter_map(|node| {
            let inner = doc.tree.nodes[node].inner.clone();
            redline::decode_text(&doc.bytes[inner], &doc.part, &limits).ok()
        })
        .collect()
}

fn refuse_complex(doc: &Doc, paragraph: &usize, anchor: &str) -> Result<(), EditError> {
    for local in REFUSED_INSIDE {
        if doc.tree.descendants(*paragraph, local).next().is_some() {
            return fail(format!(
                "{anchor} contains {} markup, which this op does not rewrite; insert a new paragraph after it instead",
                match *local {
                    "ins" | "del" | "moveFrom" | "moveTo" => "tracked-change",
                    "txbxContent" => "text-box",
                    _ => "field",
                }
            ));
        }
    }
    Ok(())
}

/// What removing a paragraph outright would break, in a new document where
/// nothing is tracked: someone's tracked change, a section's layout, a
/// comment or note left without its mark, or a field, bookmark or comment
/// range that continues into another paragraph.
fn refuse_unremovable(doc: &Doc, paragraph: usize, anchor: &str) -> Result<(), EditError> {
    let tree = &doc.tree;
    let has = |local: &str| tree.descendants(paragraph, local).next().is_some();
    if ["ins", "del", "moveFrom", "moveTo"]
        .iter()
        .any(|local| has(local))
    {
        return fail(format!(
            "{anchor} holds tracked changes, which removing it would discard; accept or reject them in Word first"
        ));
    }
    if has("sectPr") {
        return fail(format!(
            "{anchor} ends a section of the document (its page layout), so removing it would merge two sections; replace its text with an empty string instead"
        ));
    }
    if ["commentReference", "footnoteReference", "endnoteReference"]
        .iter()
        .any(|local| has(local))
    {
        return fail(format!(
            "{anchor} holds the mark of a comment or a note, which would be left with nowhere to point; replace its text instead"
        ));
    }
    let field_marks = |kind: &str| {
        tree.descendants(paragraph, "fldChar")
            .filter(|node| tree.nodes[*node].element.attr("fldCharType") == Some(kind))
            .count()
    };
    let ids = |local: &str| -> std::collections::BTreeSet<String> {
        tree.descendants(paragraph, local)
            .filter_map(|node| tree.nodes[node].element.attr("id").map(str::to_string))
            .collect()
    };
    let split_range = [
        ("bookmarkStart", "bookmarkEnd"),
        ("commentRangeStart", "commentRangeEnd"),
        ("permStart", "permEnd"),
    ]
    .iter()
    .any(|(start, end)| ids(start) != ids(end));
    if field_marks("begin") != field_marks("end") || split_range {
        return fail(format!(
            "{anchor} holds one end of a field, bookmark or comment range that continues into another paragraph; replace its text instead"
        ));
    }
    Ok(())
}

fn next_revision_id(doc: &Doc) -> u64 {
    let key = format!("{}id", doc.w);
    doc.tree
        .nodes
        .iter()
        .flat_map(|node| node.element.attributes.iter())
        .filter(|(name, _)| *name == key)
        .filter_map(|(_, value)| value.parse::<u64>().ok())
        .max()
        .map(|max| max + 1)
        .unwrap_or(1)
}

fn revision(doc: &Doc, id: u64, context: &EditContext) -> String {
    let w = &doc.w;
    format!(
        r#" {w}id="{id}" {w}author="{}" {w}date="{}""#,
        escape_attr(&context.author),
        escape_attr(&context.date)
    )
}

/// Run content for `text`: tabs and line breaks become `w:tab`/`w:br`.
pub(super) fn run_content(w: &str, text: &str) -> String {
    let mut out = String::new();
    for (line_index, line) in text.split('\n').enumerate() {
        if line_index > 0 {
            out.push_str(&format!("<{w}br/>"));
        }
        for (piece_index, piece) in line.split('\t').enumerate() {
            if piece_index > 0 {
                out.push_str(&format!("<{w}tab/>"));
            }
            if !piece.is_empty() {
                out.push_str(&format!(
                    r#"<{w}t xml:space="preserve">{}</{w}t>"#,
                    escape_text(piece)
                ));
            }
        }
    }
    out
}

/// The run's bytes with each `w:t` renamed `w:delText`, as a deletion
/// requires.
fn deleted_run(doc: &Doc, run: usize) -> Result<String, EditError> {
    let node = &doc.tree.nodes[run];
    let mut splice = Splice::default();
    for text in doc.tree.descendants(run, "t") {
        let text = &doc.tree.nodes[text];
        let renamed = text.element.name.replace(":t", ":delText");
        let renamed = if renamed == "t" {
            "delText".to_string()
        } else {
            renamed
        };
        if text.is_empty_element() {
            splice.replace(
                text.span.start - node.span.start..text.span.end - node.span.start,
                start_tag(&renamed, &text.element.attributes, true),
            );
        } else {
            splice.replace(
                text.span.start - node.span.start..text.inner.start - node.span.start,
                start_tag(&renamed, &text.element.attributes, false),
            );
            splice.replace(
                text.inner.end - node.span.start..text.span.end - node.span.start,
                format!("</{renamed}>"),
            );
        }
    }
    let bytes = splice.apply(&doc.bytes[node.span.clone()], &doc.part)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn runs(doc: &Doc, paragraph: usize) -> Vec<usize> {
    doc.tree.descendants(paragraph, "r").collect()
}

/// Inserts `content` as the last content of `paragraph`, expanding an
/// empty `<w:p/>` when needed.
fn append_to_paragraph(doc: &Doc, splice: &mut Splice, paragraph: usize, content: &str) {
    let node = &doc.tree.nodes[paragraph];
    if node.is_empty_element() {
        let open = String::from_utf8_lossy(&doc.bytes[node.span.start..node.span.end - 2])
            .trim_end()
            .to_string();
        splice.replace(
            node.span.clone(),
            format!("{open}>{content}</{}>", node.element.name),
        );
    } else {
        splice.insert(node.inner.end, content.to_string());
    }
}

fn lines(text: &str) -> Vec<String> {
    text.split(['\n', '\t'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn replace_paragraph_text<R: Read + Seek>(
    work: &mut Work<'_, R>,
    context: &EditContext,
    anchor: &str,
    text: &str,
) -> Result<Outcome, EditError> {
    let doc = load(work)?;
    let paragraph = find(&doc, anchor)?;
    let limits = *work.limits();
    let part = redline::Part {
        name: &doc.part,
        bytes: &doc.bytes,
        tree: &doc.tree,
        limits: &limits,
        w: &doc.w,
    };
    let replaced = redline::replace(
        &part,
        paragraph,
        anchor,
        text,
        context,
        next_revision_id(&doc),
    )?;
    if let Some(bytes) = replaced.bytes {
        work.put(&doc.part, bytes);
    }
    Ok(Outcome {
        summary: replaced.summary,
        expect: vec![Expect::Paragraph {
            anchor: anchor.to_string(),
            author: context.author.clone(),
            tracked: context.tracked,
            accepted: replaced.accepted,
            rejected: replaced.rejected,
            fixed: replaced.fixed,
        }],
        created: None,
    })
}

/// The paragraph `anchor` of a written main part, as a paragraph edit's
/// postcondition reads it.
pub(super) fn paragraph_views(
    name: &str,
    bytes: &[u8],
    limits: &Limits,
    anchor: &str,
    author: &str,
    tracked: bool,
) -> Result<redline::Views, String> {
    let tree = Tree::parse(bytes, name, limits).map_err(|error| error.to_string())?;
    let Some(w) = tree.prefix_for(W).or_else(|| tree.prefix_for(W_STRICT)) else {
        return Err("the document does not declare the WordprocessingML namespace".into());
    };
    let Some((_, paragraph)) = paragraphs(&tree)
        .into_iter()
        .find(|(candidate, _)| candidate == anchor)
    else {
        return Err(format!("{anchor} is missing"));
    };
    let part = redline::Part {
        name,
        bytes,
        tree: &tree,
        limits,
        w: &w,
    };
    redline::views(&part, paragraph, author, tracked).map_err(|error| error.message)
}

pub(super) fn insert_paragraph_after<R: Read + Seek>(
    work: &mut Work<'_, R>,
    context: &EditContext,
    anchor: &str,
    text: &str,
    style: Option<&str>,
) -> Result<Outcome, EditError> {
    if text.trim().is_empty() {
        return fail("text must not be empty");
    }
    let style_id = match style {
        Some(style) => Some(resolve_style(work, style)?),
        None => None,
    };
    let doc = load(work)?;
    let paragraph = find(&doc, anchor)?;
    let w = doc.w.clone();
    let mut splice = Splice::default();

    let (w14, root_tag) = ensure_w14(&doc)?;
    if let Some((range, tag)) = root_tag {
        splice.replace(range, tag);
    }
    let para_id = new_para_id(&doc);
    let style_xml = style_id
        .map(|id| format!(r#"<{w}pStyle {w}val="{}"/>"#, escape_attr(&id)))
        .unwrap_or_default();
    let paragraph_xml = if context.tracked {
        let first = next_revision_id(&doc);
        format!(
            r#"<{w}p {w14}paraId="{para_id}"><{w}pPr>{style_xml}<{w}rPr><{w}ins{}/></{w}rPr></{w}pPr><{w}ins{}><{w}r>{}</{w}r></{w}ins></{w}p>"#,
            revision(&doc, first, context),
            revision(&doc, first + 1, context),
            run_content(&w, text)
        )
    } else {
        let properties = if style_xml.is_empty() {
            String::new()
        } else {
            format!("<{w}pPr>{style_xml}</{w}pPr>")
        };
        format!(
            r#"<{w}p {w14}paraId="{para_id}">{properties}<{w}r>{}</{w}r></{w}p>"#,
            run_content(&w, text)
        )
    };
    splice.insert(doc.tree.nodes[paragraph].span.end, paragraph_xml);
    work.put(&doc.part, splice.apply(&doc.bytes, &doc.part)?);
    let new_anchor = format!("p:{para_id}");
    Ok(Outcome {
        summary: if context.tracked {
            format!("{new_anchor} inserted after {anchor} as a tracked insertion")
        } else {
            format!("{new_anchor} inserted after {anchor}")
        },
        expect: vec![
            Expect::UnitContains {
                anchor: new_anchor.clone(),
                needles: lines(text),
            },
            Expect::ParagraphDelta(1),
        ],
        created: Some(new_anchor),
    })
}

pub(super) fn delete_paragraph<R: Read + Seek>(
    work: &mut Work<'_, R>,
    context: &EditContext,
    anchor: &str,
) -> Result<Outcome, EditError> {
    let doc = load(work)?;
    let paragraph = find(&doc, anchor)?;
    if context.tracked {
        refuse_complex(&doc, &paragraph, anchor)?;
    } else {
        refuse_unremovable(&doc, paragraph, anchor)?;
    }
    let node = &doc.tree.nodes[paragraph];
    if let Some(parent) = node.parent {
        let parent_node = &doc.tree.nodes[parent];
        let siblings: Vec<usize> = parent_node
            .children
            .iter()
            .copied()
            .filter(|child| {
                let child = &doc.tree.nodes[*child];
                !child.skipped && matches!(child.local(), "p" | "tbl" | "sdt")
            })
            .collect();
        let last = siblings.last() == Some(&paragraph);
        if parent_node.local() == "body" && last {
            return fail(format!(
                "{anchor} is the document's last paragraph, which Word requires; replace its text with an empty string instead"
            ));
        }
        if parent_node.local() == "tc" && siblings.len() == 1 {
            return fail(format!(
                "{anchor} is the only paragraph in its table cell, which Word requires; replace its text with an empty string instead"
            ));
        }
    }
    if !context.tracked {
        // A new document is written clean: the paragraph simply goes.
        let mut splice = Splice::default();
        splice.replace(node.span.clone(), "");
        work.put(&doc.part, splice.apply(&doc.bytes, &doc.part)?);
        let mut expect = vec![Expect::ParagraphDelta(-1)];
        if anchor.starts_with("p:") {
            expect.push(Expect::Absent {
                anchor: anchor.to_string(),
            });
        }
        return Ok(Outcome {
            summary: format!("{anchor} removed"),
            expect,
            created: None,
        });
    }
    let w = doc.w.clone();
    let mut id = next_revision_id(&doc);
    let mut splice = Splice::default();
    for run in runs(&doc, paragraph) {
        let deleted = deleted_run(&doc, run)?;
        splice.replace(
            doc.tree.nodes[run].span.clone(),
            format!("<{w}del{}>{deleted}</{w}del>", revision(&doc, id, context)),
        );
        id += 1;
    }
    let mark = format!("<{w}del{}/>", revision(&doc, id, context));
    match doc.tree.children(paragraph, "pPr").next() {
        Some(properties) => {
            let properties_node = &doc.tree.nodes[properties];
            match doc.tree.children(properties, "rPr").next() {
                Some(run_properties) => {
                    let run_properties = &doc.tree.nodes[run_properties];
                    if run_properties.is_empty_element() {
                        splice.replace(
                            run_properties.span.clone(),
                            format!("<{w}rPr>{mark}</{w}rPr>"),
                        );
                    } else {
                        splice.insert(run_properties.inner.start, mark);
                    }
                }
                None => {
                    let before = doc
                        .tree
                        .children(properties, "sectPr")
                        .chain(doc.tree.children(properties, "pPrChange"))
                        .map(|child| doc.tree.nodes[child].span.start)
                        .min();
                    let wrapped = format!("<{w}rPr>{mark}</{w}rPr>");
                    if properties_node.is_empty_element() {
                        splice.replace(
                            properties_node.span.clone(),
                            format!("<{w}pPr>{wrapped}</{w}pPr>"),
                        );
                    } else {
                        splice.insert(before.unwrap_or(properties_node.inner.end), wrapped);
                    }
                }
            }
        }
        None => {
            let content = format!("<{w}pPr><{w}rPr>{mark}</{w}rPr></{w}pPr>");
            if node.is_empty_element() {
                append_to_paragraph(&doc, &mut splice, paragraph, &content);
            } else {
                splice.insert(node.inner.start, content);
            }
        }
    }
    work.put(&doc.part, splice.apply(&doc.bytes, &doc.part)?);
    Ok(Outcome {
        summary: format!("{anchor} deleted as a tracked change"),
        expect: vec![Expect::UnitDeleted {
            anchor: anchor.to_string(),
        }],
        created: None,
    })
}

/// The prefix bound to the Word 2010 namespace, and the rewritten root
/// start tag when it had to be declared (plus `mc:Ignorable`).
#[allow(clippy::type_complexity)]
fn ensure_w14(doc: &Doc) -> Result<(String, Option<(std::ops::Range<usize>, String)>), EditError> {
    if let Some(prefix) = doc.tree.prefix_for(W14) {
        return Ok((prefix, None));
    }
    let root = doc.tree.root();
    let mut attributes = root.element.attributes.clone();
    if attributes.iter().any(|(key, _)| key == "xmlns:w14") {
        return fail("the document binds the w14 prefix to another namespace");
    }
    attributes.push(("xmlns:w14".into(), W14.into()));
    let mc = match doc.tree.prefix_for(MC) {
        Some(prefix) => prefix,
        None => {
            if attributes.iter().any(|(key, _)| key == "xmlns:mc") {
                return fail("the document binds the mc prefix to another namespace");
            }
            attributes.push(("xmlns:mc".into(), MC.into()));
            "mc:".into()
        }
    };
    let ignorable = format!("{mc}Ignorable");
    match attributes.iter_mut().find(|(key, _)| *key == ignorable) {
        Some((_, value)) => {
            if !value.split_whitespace().any(|prefix| prefix == "w14") {
                value.push_str(" w14");
            }
        }
        None => attributes.push((ignorable, "w14".into())),
    }
    let tag = start_tag(&root.element.name, &attributes, root.is_empty_element());
    let end = if root.is_empty_element() {
        root.span.end
    } else {
        root.inner.start
    };
    Ok(("w14:".into(), Some((root.span.start..end, tag))))
}

fn new_para_id(doc: &Doc) -> String {
    let existing: std::collections::HashSet<String> = doc
        .tree
        .nodes
        .iter()
        .filter_map(|node| node.element.attr("paraId"))
        .map(str::to_ascii_uppercase)
        .collect();
    (0x1A00_0001u32..0x7FFF_FFFF)
        .map(|value| format!("{value:08X}"))
        .find(|candidate| !existing.contains(candidate))
        .unwrap_or_else(|| "7FFFFFFE".into())
}

/// A paragraph style by id or by display name (`Heading 1`).
fn resolve_style<R: Read + Seek>(
    work: &mut Work<'_, R>,
    wanted: &str,
) -> Result<String, EditError> {
    let main = work.main_part();
    let Some(part) = work.related(&main, "styles")? else {
        return fail("the document has no styles part, so no style can be applied");
    };
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let mut available = Vec::new();
    for style in tree.descendants(0, "style") {
        let element = &tree.nodes[style].element;
        if element.attr("type") != Some("paragraph") {
            continue;
        }
        let Some(id) = element.attr("styleId") else {
            continue;
        };
        let name = tree
            .children(style, "name")
            .next()
            .and_then(|name| tree.nodes[name].element.attr("val"))
            .unwrap_or(id);
        if id == wanted || name.eq_ignore_ascii_case(wanted) || id.eq_ignore_ascii_case(wanted) {
            return Ok(id.to_string());
        }
        available.push(name.to_string());
    }
    available.truncate(30);
    fail(format!(
        "no paragraph style {wanted:?}; available: {}",
        available.join(", ")
    ))
}
