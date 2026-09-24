//! Word ops. Every change to an existing document is a native tracked
//! change under the runtime-supplied author (docs/design/72, R7), so it
//! survives into Word as a redline a person can accept or reject there.

use std::io::{Read, Seek};

use super::{EditContext, EditError, Expect, Outcome, Work, fail};
use crate::splice::{Splice, Tree, escape_attr, escape_text, start_tag};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const W_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// Markup inside a paragraph that v1 ops will not rewrite: existing
/// revisions, fields and text boxes. The error says what to do instead.
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
    paragraphs(&doc.tree)
        .into_iter()
        .find(|(candidate, _)| candidate == anchor)
        .map(|(_, index)| index)
        .ok_or_else(|| EditError {
            op: None,
            message: format!(
                "no paragraph {anchor}; use an anchor from doc_read on this exact file (anchors look like p:1A2B3C4D or p@12)"
            ),
        })
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
fn run_content(w: &str, text: &str) -> String {
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

/// The first run's formatting, unless copying it would hide the new text
/// or duplicate a revision id.
fn copied_run_properties(doc: &Doc, run: Option<usize>) -> String {
    let Some(run) = run else {
        return String::new();
    };
    let Some(properties) = doc.tree.children(run, "rPr").next() else {
        return String::new();
    };
    for unsafe_local in ["vanish", "specVanish", "rPrChange"] {
        if doc
            .tree
            .descendants(properties, unsafe_local)
            .next()
            .is_some()
        {
            return String::new();
        }
    }
    let white = doc.tree.descendants(properties, "color").any(|color| {
        doc.tree.nodes[color]
            .element
            .attr("val")
            .is_some_and(|value| value.eq_ignore_ascii_case("FFFFFF"))
    });
    if white {
        return String::new();
    }
    String::from_utf8_lossy(&doc.bytes[doc.tree.nodes[properties].span.clone()]).into_owned()
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
    refuse_complex(&doc, &paragraph, anchor)?;
    let runs = runs(&doc, paragraph);
    let mut id = next_revision_id(&doc);
    let mut splice = Splice::default();
    let w = &doc.w;
    for run in &runs {
        let deleted = deleted_run(&doc, *run)?;
        splice.replace(
            doc.tree.nodes[*run].span.clone(),
            format!("<{w}del{}>{deleted}</{w}del>", revision(&doc, id, context)),
        );
        id += 1;
    }
    if !text.is_empty() {
        let properties = copied_run_properties(&doc, runs.first().copied());
        let inserted = format!(
            "<{w}ins{}><{w}r>{properties}{}</{w}r></{w}ins>",
            revision(&doc, id, context),
            run_content(w, text)
        );
        append_to_paragraph(&doc, &mut splice, paragraph, &inserted);
    }
    work.put(&doc.part, splice.apply(&doc.bytes, &doc.part)?);
    let expect = if text.trim().is_empty() {
        Expect::UnitDeleted {
            anchor: anchor.to_string(),
        }
    } else {
        Expect::UnitContains {
            anchor: anchor.to_string(),
            needles: lines(text),
        }
    };
    Ok(Outcome {
        summary: format!(
            "{anchor}: text replaced as a tracked change ({} run(s) marked deleted)",
            runs.len()
        ),
        expect: vec![expect],
    })
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
    let first = next_revision_id(&doc);
    let style_xml = style_id
        .map(|id| format!(r#"<{w}pStyle {w}val="{}"/>"#, escape_attr(&id)))
        .unwrap_or_default();
    let paragraph_xml = format!(
        r#"<{w}p {w14}paraId="{para_id}"><{w}pPr>{style_xml}<{w}rPr><{w}ins{}/></{w}rPr></{w}pPr><{w}ins{}><{w}r>{}</{w}r></{w}ins></{w}p>"#,
        revision(&doc, first, context),
        revision(&doc, first + 1, context),
        run_content(&w, text)
    );
    splice.insert(doc.tree.nodes[paragraph].span.end, paragraph_xml);
    work.put(&doc.part, splice.apply(&doc.bytes, &doc.part)?);
    let new_anchor = format!("p:{para_id}");
    Ok(Outcome {
        summary: format!("{new_anchor} inserted after {anchor} as a tracked insertion"),
        expect: vec![Expect::UnitContains {
            anchor: new_anchor,
            needles: lines(text),
        }],
    })
}

pub(super) fn delete_paragraph<R: Read + Seek>(
    work: &mut Work<'_, R>,
    context: &EditContext,
    anchor: &str,
) -> Result<Outcome, EditError> {
    let doc = load(work)?;
    let paragraph = find(&doc, anchor)?;
    refuse_complex(&doc, &paragraph, anchor)?;
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
