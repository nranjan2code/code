//! PowerPoint ops. New slides come only from the deck's own layouts and
//! their placeholders, never free-positioned shapes, so output follows the
//! template and stays reliable with small models.

use std::collections::BTreeMap;
use std::io::{Read, Seek};

use super::{EditError, Expect, Outcome, TextValue, Work, fail, free_part_name};
use crate::package::rels_part_name;
use crate::splice::{Splice, Tree, escape_attr, escape_text};

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const P_STRICT: &str = "http://purl.oclc.org/ooxml/presentationml/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const A_STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const R_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const SLIDE_TYPE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
const LAYOUT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";
const NOTES_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";

struct Deck {
    part: String,
    bytes: Vec<u8>,
    tree: Tree,
    p: String,
    r: String,
    list: Option<usize>,
    /// (slide id, relationship id, sldId node) in presentation order.
    slides: Vec<(String, String, usize)>,
}

fn deck<R2: Read + Seek>(work: &mut Work<'_, R2>) -> Result<Deck, EditError> {
    let part = work.main_part();
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let Some(p) = tree.prefix_for(P).or_else(|| tree.prefix_for(P_STRICT)) else {
        return fail("the presentation does not declare the PresentationML namespace");
    };
    if tree.children(0, "modifyVerifier").next().is_some() {
        return fail(
            "the presentation has a password to modify (O8); the owner must remove it before it can be edited",
        );
    }
    let r = tree
        .prefix_for(R)
        .or_else(|| tree.prefix_for(R_STRICT))
        .unwrap_or_else(|| "r:".into());
    let list = tree.children(0, "sldIdLst").next();
    let slides = list
        .map(|list| {
            tree.children(list, "sldId")
                .filter_map(|node| {
                    let element = &tree.nodes[node].element;
                    Some((
                        element.attr_unprefixed("id")?.to_string(),
                        element.attr_prefixed("id")?.to_string(),
                        node,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Deck {
        part,
        bytes,
        tree,
        p,
        r,
        list,
        slides,
    })
}

/// `slide:256` or `slide:256/...` → `256`.
fn slide_id(anchor: &str) -> Result<&str, EditError> {
    anchor
        .strip_prefix("slide:")
        .map(|rest| rest.split('/').next().unwrap_or(rest))
        .filter(|id| !id.is_empty())
        .ok_or_else(|| EditError {
            op: None,
            message: format!(
                "{anchor:?} is not a slide anchor; use one like slide:256 from doc_read"
            ),
        })
}

fn slide_part<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    deck: &Deck,
    id: &str,
) -> Result<String, EditError> {
    let Some((_, relationship, _)) = deck.slides.iter().find(|(slide, _, _)| slide == id) else {
        let known: Vec<String> = deck
            .slides
            .iter()
            .map(|(slide, _, _)| format!("slide:{slide}"))
            .collect();
        return fail(format!("no slide:{id}; slides: {}", known.join(", ")));
    };
    match work.by_id(&deck.part, relationship)? {
        Some(part) => Ok(part),
        None => fail(format!("slide:{id} points at no part")),
    }
}

fn prefixes(tree: &Tree) -> Result<(String, String), EditError> {
    let p = tree.prefix_for(P).or_else(|| tree.prefix_for(P_STRICT));
    let a = tree.prefix_for(A).or_else(|| tree.prefix_for(A_STRICT));
    match (p, a) {
        (Some(p), Some(a)) => Ok((p, a)),
        _ => fail(
            "the slide does not declare the PresentationML and DrawingML namespaces on its root",
        ),
    }
}

/// A shape's `(id, name, placeholder type, placeholder idx)`.
fn shape_facts(tree: &Tree, shape: usize) -> (String, String, Option<String>, Option<String>) {
    let (id, name) = tree
        .descendants(shape, "cNvPr")
        .next()
        .map(|node| {
            let element = &tree.nodes[node].element;
            (
                element.attr("id").unwrap_or_default().to_string(),
                element.attr("name").unwrap_or_default().to_string(),
            )
        })
        .unwrap_or_default();
    let placeholder = tree
        .descendants(shape, "ph")
        .next()
        .map(|node| &tree.nodes[node].element);
    let kind = placeholder.map(|element| element.attr("type").unwrap_or("body").to_string());
    let index = placeholder.and_then(|element| element.attr("idx").map(str::to_string));
    (id, name, kind, index)
}

/// Whether a placeholder of `kind`/`index` answers to `key` (`title`,
/// `subtitle`, `body`, a raw type, or `idx:N`).
fn placeholder_matches(key: &str, kind: Option<&str>, index: Option<&str>) -> bool {
    let key = key.trim().to_ascii_lowercase();
    if let Some(wanted) = key.strip_prefix("idx:") {
        return index == Some(wanted);
    }
    match (key.as_str(), kind) {
        ("title", Some("title" | "ctrTitle")) => true,
        ("subtitle", Some("subTitle")) => true,
        (wanted, Some(kind)) => kind.eq_ignore_ascii_case(wanted),
        _ => false,
    }
}

fn paragraphs(
    a: &str,
    lines: &[String],
    paragraph_properties: &str,
    run_properties: &str,
) -> String {
    if lines.iter().all(|line| line.is_empty()) {
        return format!("<{a}p/>");
    }
    lines
        .iter()
        .map(|line| {
            if line.is_empty() {
                format!("<{a}p>{paragraph_properties}</{a}p>")
            } else {
                format!(
                    "<{a}p>{paragraph_properties}<{a}r>{run_properties}<{a}t>{}</{a}t></{a}r></{a}p>",
                    escape_text(line)
                )
            }
        })
        .collect()
}

/// Replaces the text of shape `shape` in `tree`, keeping its body and list
/// properties and the first paragraph's and run's formatting.
fn replace_shape_text(
    splice: &mut Splice,
    bytes: &[u8],
    tree: &Tree,
    shape: usize,
    lines: &[String],
) -> Result<(), EditError> {
    let (p, a) = prefixes(tree)?;
    match tree.children(shape, "txBody").next() {
        Some(body) => {
            let existing: Vec<usize> = tree.children(body, "p").collect();
            let slice = |index: usize| {
                String::from_utf8_lossy(&bytes[tree.nodes[index].span.clone()]).into_owned()
            };
            let paragraph_properties = existing
                .first()
                .and_then(|first| tree.children(*first, "pPr").next())
                .map(slice)
                .unwrap_or_default();
            let run_properties = existing
                .first()
                .and_then(|first| tree.descendants(*first, "rPr").next())
                .map(slice)
                .unwrap_or_default();
            let new = paragraphs(&a, lines, &paragraph_properties, &run_properties);
            match (existing.first(), existing.last()) {
                (Some(first), Some(last)) => splice.replace(
                    tree.nodes[*first].span.start..tree.nodes[*last].span.end,
                    new,
                ),
                _ => {
                    let node = &tree.nodes[body];
                    if node.is_empty_element() {
                        return fail("the shape's text body is empty markup this op cannot extend");
                    }
                    splice.insert(node.inner.end, new);
                }
            }
        }
        None => {
            let after = tree
                .children(shape, "style")
                .chain(tree.children(shape, "spPr"))
                .map(|index| tree.nodes[index].span.end)
                .max();
            let Some(after) = after else {
                return fail("the shape has no shape properties to place text after");
            };
            splice.insert(
                after,
                format!(
                    "<{p}txBody><{a}bodyPr/><{a}lstStyle/>{}</{p}txBody>",
                    paragraphs(&a, lines, "", "")
                ),
            );
        }
    }
    Ok(())
}

fn expect_text(anchor: String, lines: &[String]) -> Expect {
    let needles: Vec<String> = lines
        .iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    if needles.is_empty() {
        Expect::Absent { anchor }
    } else {
        Expect::UnitContains { anchor, needles }
    }
}

pub(super) fn set_placeholder_text<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    anchor: &str,
    text: &TextValue,
) -> Result<Outcome, EditError> {
    let deck = deck(work)?;
    let id = slide_id(anchor)?.to_string();
    let target = anchor
        .split_once('/')
        .map(|(_, rest)| rest.to_string())
        .ok_or_else(|| EditError {
            op: None,
            message: format!(
                "{anchor:?} names no shape; use slide:ID/shape:N or slide:ID/placeholder:title"
            ),
        })?;
    let part = slide_part(work, &deck, &id)?;
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let mut shapes = Vec::new();
    let mut found = None;
    for shape in tree.descendants(0, "sp") {
        let (shape_id, name, kind, index) = shape_facts(&tree, shape);
        let hit = match target.split_once(':') {
            Some(("shape", wanted)) => shape_id == wanted,
            Some(("placeholder", wanted)) => {
                placeholder_matches(wanted, kind.as_deref(), index.as_deref())
            }
            _ => false,
        };
        if hit && found.is_none() {
            found = Some((shape, shape_id.clone()));
        }
        shapes.push(match kind {
            Some(kind) => format!("shape:{shape_id} ({name}, placeholder {kind})"),
            None => format!("shape:{shape_id} ({name})"),
        });
    }
    let Some((shape, shape_id)) = found else {
        return fail(format!(
            "slide:{id} has no text shape {target}; shapes: {}",
            shapes.join(", ")
        ));
    };
    let lines = text.lines();
    let mut splice = Splice::default();
    replace_shape_text(&mut splice, &bytes, &tree, shape, &lines)?;
    work.put(&part, splice.apply(&bytes, &part)?);
    Ok(Outcome {
        summary: format!(
            "slide:{id}/shape:{shape_id} text set ({} line(s))",
            lines.len()
        ),
        expect: vec![expect_text(format!("slide:{id}/shape:{shape_id}"), &lines)],
        created: Vec::new(),
    })
}

/// Creates `slide`'s notes page, holding `lines`, from the deck's notes
/// master, and returns its part.
fn create_notes<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    deck_part: &str,
    slide: &str,
    lines: &[String],
) -> Result<String, EditError> {
    let Some(master) = work.related(deck_part, "notesMaster")? else {
        return fail(
            "this deck has no notes master, so speaker notes cannot be added to a slide that has none; open it in PowerPoint and add notes there once",
        );
    };
    let part = free_part_name(work, "ppt/notesSlides/notesSlide", ".xml");
    work.put(
        &part,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/><p:cNvSpPr><a:spLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvSpPr><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{}</p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"#,
            paragraphs("a:", lines, "", "")
        )
        .into_bytes(),
    );
    work.set_override(&part, NOTES_TYPE)?;
    work.add_relationship(&part, &format!("{R}/notesMaster"), &master)?;
    work.add_relationship(&part, &format!("{R}/slide"), slide)?;
    work.add_relationship(slide, &format!("{R}/notesSlide"), &part)?;
    Ok(part)
}

pub(super) fn set_notes<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    anchor: &str,
    text: &str,
) -> Result<Outcome, EditError> {
    let deck = deck(work)?;
    let id = slide_id(anchor)?.to_string();
    let part = slide_part(work, &deck, &id)?;
    let Some(notes) = work.related(&part, "notesSlide")? else {
        if work.strict() {
            return fail("adding a notes page to a Strict presentation is not supported yet");
        }
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        create_notes(work, &deck.part, &part, &lines)?;
        return Ok(Outcome {
            summary: format!("slide:{id} speaker notes added"),
            expect: vec![expect_text(format!("slide:{id}/notes"), &lines)],
            created: Vec::new(),
        });
    };
    let bytes = work.get(&notes)?;
    let tree = Tree::parse(&bytes, &notes, work.limits())?;
    let body = tree.descendants(0, "sp").find(|shape| {
        let (_, _, kind, _) = shape_facts(&tree, *shape);
        kind.as_deref() == Some("body")
    });
    let Some(body) = body else {
        return fail(format!("slide:{id}'s notes page has no notes placeholder"));
    };
    let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let mut splice = Splice::default();
    replace_shape_text(&mut splice, &bytes, &tree, body, &lines)?;
    work.put(&notes, splice.apply(&bytes, &notes)?);
    Ok(Outcome {
        summary: format!("slide:{id} speaker notes set"),
        expect: vec![expect_text(format!("slide:{id}/notes"), &lines)],
        created: Vec::new(),
    })
}

pub(super) fn add_slide_from_layout<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    layout: &str,
    after: Option<&str>,
    placeholders: &BTreeMap<String, TextValue>,
    notes: Option<&str>,
) -> Result<Outcome, EditError> {
    if work.strict() {
        return fail("adding a slide to a Strict presentation is not supported yet");
    }
    let mut layouts = Vec::new();
    let mut chosen = None;
    for part in work.part_names() {
        if work.content_type(&part)?.as_deref() != Some(LAYOUT_TYPE) {
            continue;
        }
        let bytes = work.get(&part)?;
        let tree = Tree::parse(&bytes, &part, work.limits())?;
        let name = tree
            .descendants(0, "cSld")
            .next()
            .and_then(|node| tree.nodes[node].element.attr("name"))
            .unwrap_or_default()
            .to_string();
        if chosen.is_none() && name.trim().eq_ignore_ascii_case(layout.trim()) {
            chosen = Some((part.clone(), tree));
        }
        layouts.push(name);
    }
    let Some((layout_part, layout_tree)) = chosen else {
        layouts.sort();
        layouts.dedup();
        return fail(format!(
            "no layout {layout:?}; this deck's layouts: {}",
            layouts.join(", ")
        ));
    };

    let mut shapes = Vec::new();
    let mut keys = Vec::new();
    for shape in layout_tree.descendants(0, "sp") {
        let (_, name, kind, index) = shape_facts(&layout_tree, shape);
        let Some(kind) = kind else {
            continue;
        };
        if matches!(kind.as_str(), "dt" | "ftr" | "sldNum" | "hdr") {
            continue;
        }
        keys.push(match kind.as_str() {
            "title" | "ctrTitle" => "title".to_string(),
            "subTitle" => "subtitle".to_string(),
            other => match &index {
                Some(index) if other == "body" => format!("body (or idx:{index})"),
                _ => other.to_string(),
            },
        });
        // The slide's placeholder names the layout's exactly: a content
        // placeholder has no type (it means `obj`), a text one says `body`.
        let written_type = layout_tree
            .descendants(shape, "ph")
            .next()
            .and_then(|ph| layout_tree.nodes[ph].element.attr("type"))
            .map(str::to_string);
        shapes.push((name, kind, index, written_type));
    }
    for key in placeholders.keys() {
        if !shapes
            .iter()
            .any(|(_, kind, index, _)| placeholder_matches(key, Some(kind), index.as_deref()))
        {
            return fail(format!(
                "layout {layout:?} has no placeholder {key:?}; it has: {}",
                keys.join(", ")
            ));
        }
    }

    let deck = deck(work)?;
    let a = "a:";
    let mut used = std::collections::HashSet::new();
    let mut body = String::new();
    for (position, (name, kind, index, written_type)) in shapes.iter().enumerate() {
        let text = placeholders
            .iter()
            .find(|(key, _)| {
                placeholder_matches(key, Some(kind), index.as_deref())
                    && used.insert((*key).clone())
            })
            .map(|(_, text)| text.lines())
            .unwrap_or_default();
        let type_attr = written_type
            .as_ref()
            .map(|kind| format!(r#" type="{}""#, escape_attr(kind)))
            .unwrap_or_default();
        let index_attr = index
            .as_ref()
            .map(|index| format!(r#" idx="{}""#, escape_attr(index)))
            .unwrap_or_default();
        body.push_str(&format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="{}" name="{}"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph{type_attr}{index_attr}/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{}</p:txBody></p:sp>"#,
            position + 2,
            escape_attr(name),
            paragraphs(a, &text, "", "")
        ));
    }
    let slide = free_part_name(work, "ppt/slides/slide", ".xml");
    work.put(
        &slide,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>{body}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#
        )
        .into_bytes(),
    );
    work.set_override(&slide, SLIDE_TYPE)?;
    work.add_relationship(&slide, &format!("{R}/slideLayout"), &layout_part)?;
    let relationship = work.add_relationship(&deck.part, &format!("{R}/slide"), &slide)?;

    let used_ids = deck
        .tree
        .nodes
        .iter()
        .filter(|node| node.local() == "sldId")
        .filter_map(|node| {
            node.element
                .attr_unprefixed("id")
                .and_then(|id| id.parse::<u32>().ok())
        });
    let new_id = used_ids.max().map(|max| max + 1).unwrap_or(256).max(256);
    let (p, r) = (&deck.p, &deck.r);
    let entry = format!(r#"<{p}sldId id="{new_id}" {r}id="{relationship}"/>"#);
    let mut splice = Splice::default();
    let mut order: Vec<String> = deck
        .slides
        .iter()
        .map(|(id, _, _)| format!("slide:{id}"))
        .collect();
    match (deck.list, after) {
        (Some(_), Some(after)) => {
            let after_id = slide_id(after)?;
            let Some(position) = deck.slides.iter().position(|(id, _, _)| id == after_id) else {
                return fail(format!("no slide {after} to add after"));
            };
            splice.insert(deck.tree.nodes[deck.slides[position].2].span.end, entry);
            order.insert(position + 1, format!("slide:{new_id}"));
        }
        (Some(list), None) => {
            let node = &deck.tree.nodes[list];
            if node.is_empty_element() {
                splice.replace(
                    node.span.clone(),
                    format!("<{p}sldIdLst>{entry}</{p}sldIdLst>"),
                );
            } else {
                splice.insert(node.inner.end, entry);
            }
            order.push(format!("slide:{new_id}"));
        }
        (None, _) => {
            let before = deck
                .tree
                .children(0, "sldSz")
                .chain(deck.tree.children(0, "notesSz"))
                .map(|index| deck.tree.nodes[index].span.start)
                .min();
            let Some(before) = before else {
                return fail(
                    "the presentation has no slide list and no slide size to place one before",
                );
            };
            splice.insert(before, format!("<{p}sldIdLst>{entry}</{p}sldIdLst>"));
            order.push(format!("slide:{new_id}"));
        }
    }
    work.put(&deck.part, splice.apply(&deck.bytes, &deck.part)?);

    let mut expect = vec![Expect::SlideOrder(order)];
    if let Some(notes) = notes.filter(|notes| !notes.trim().is_empty()) {
        let lines: Vec<String> = notes.split('\n').map(str::to_string).collect();
        create_notes(work, &deck.part, &slide, &lines)?;
        expect.push(expect_text(format!("slide:{new_id}/notes"), &lines));
    }
    for (key, text) in placeholders {
        if let Some(first) = text
            .lines()
            .into_iter()
            .map(|line| line.trim().to_string())
            .find(|line| !line.is_empty())
        {
            if key.eq_ignore_ascii_case("title") {
                expect.push(Expect::UnitContains {
                    anchor: format!("slide:{new_id}"),
                    needles: vec![first],
                });
            } else {
                expect.push(Expect::AnyUnitContains {
                    prefix: format!("slide:{new_id}/shape:"),
                    needle: first,
                });
            }
        }
    }
    Ok(Outcome {
        summary: format!("slide:{new_id} added from layout {layout:?}"),
        expect,
        created: vec![format!("slide:{new_id}")],
    })
}

pub(super) fn delete_slide<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    anchor: &str,
) -> Result<Outcome, EditError> {
    let deck = deck(work)?;
    let id = slide_id(anchor)?.to_string();
    let part = slide_part(work, &deck, &id)?;
    let Some((_, relationship, _)) = deck
        .slides
        .iter()
        .find(|(slide, _, _)| *slide == id)
        .cloned()
    else {
        return fail(format!("no slide:{id}"));
    };
    if deck.slides.len() == 1 {
        return fail("this is the deck's only slide; add another before deleting it");
    }
    let notes = work.related(&part, "notesSlide")?;
    let own_rels = [
        rels_part_name(&deck.part),
        rels_part_name(&part),
        notes.as_deref().map(rels_part_name).unwrap_or_default(),
    ];
    for rels in work.part_names() {
        if !rels.to_ascii_lowercase().ends_with(".rels") || own_rels.contains(&rels) {
            continue;
        }
        let source = rels
            .rsplit_once("_rels/")
            .map(|(directory, file)| format!("{directory}{}", file.trim_end_matches(".rels")))
            .unwrap_or_default();
        if work
            .relationships(&source)?
            .iter()
            .any(|relationship| !relationship.external && relationship.target == part)
        {
            return fail(format!(
                "slide:{id} is referenced by {source} (a link or custom show); remove that reference first"
            ));
        }
    }
    let mut splice = Splice::default();
    for node in &deck.tree.nodes {
        if node.local() == "sldId" && node.element.attr_unprefixed("id") == Some(id.as_str()) {
            splice.replace(node.span.clone(), "");
        }
    }
    work.put(&deck.part, splice.apply(&deck.bytes, &deck.part)?);
    work.remove_relationship(&deck.part, &relationship)?;
    if let Some(notes) = notes {
        work.remove(&notes);
        work.remove(&rels_part_name(&notes));
        work.remove_override(&notes)?;
    }
    work.remove(&part);
    work.remove(&rels_part_name(&part));
    work.remove_override(&part)?;
    Ok(Outcome {
        summary: format!("slide:{id} deleted"),
        expect: vec![
            Expect::Absent {
                anchor: format!("slide:{id}"),
            },
            Expect::SlideOrder(
                deck.slides
                    .iter()
                    .filter(|(slide, _, _)| *slide != id)
                    .map(|(slide, _, _)| format!("slide:{slide}"))
                    .collect(),
            ),
        ],
        created: Vec::new(),
    })
}

pub(super) fn move_slide<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    anchor: &str,
    after: Option<&str>,
) -> Result<Outcome, EditError> {
    let deck = deck(work)?;
    let id = slide_id(anchor)?.to_string();
    let Some(from) = deck.slides.iter().position(|(slide, _, _)| *slide == id) else {
        return fail(format!("no slide:{id}"));
    };
    let Some(list) = deck.list else {
        return fail("the presentation has no slide list");
    };
    let moving = &deck.tree.nodes[deck.slides[from].2];
    let entry = String::from_utf8_lossy(&deck.bytes[moving.span.clone()]).into_owned();
    let mut order: Vec<String> = deck
        .slides
        .iter()
        .map(|(id, _, _)| format!("slide:{id}"))
        .collect();
    let moved = order.remove(from);
    let mut splice = Splice::default();
    splice.replace(moving.span.clone(), "");
    match after {
        Some(after) => {
            let after_id = slide_id(after)?;
            if after_id == id {
                return fail("a slide cannot move after itself");
            }
            let Some(target) = deck
                .slides
                .iter()
                .position(|(slide, _, _)| slide == after_id)
            else {
                return fail(format!("no slide {after} to move after"));
            };
            splice.insert(deck.tree.nodes[deck.slides[target].2].span.end, entry);
            let position = order
                .iter()
                .position(|slide| *slide == format!("slide:{after_id}"))
                .unwrap_or(0);
            order.insert(position + 1, moved);
        }
        None => {
            splice.insert(deck.tree.nodes[list].inner.start, entry);
            order.insert(0, moved);
        }
    }
    work.put(&deck.part, splice.apply(&deck.bytes, &deck.part)?);
    Ok(Outcome {
        summary: format!(
            "slide:{id} moved {}",
            after
                .map(|after| format!("after {after}"))
                .unwrap_or_else(|| "to the start".into())
        ),
        expect: vec![Expect::SlideOrder(order)],
        created: Vec::new(),
    })
}
