//! L1 editing: locate elements by byte span and splice replacements into
//! the original bytes (O1). Everything outside an edited span stays
//! byte-identical, so markup Vak does not model survives an edit.

use std::ops::Range;

use quick_xml::events::Event;

use crate::xml::{Element, local_name};
use crate::{Error, Limits};

/// One element with the byte spans of its tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub element: Element,
    /// From `<` of the start tag to after `>` of the end tag.
    pub span: Range<usize>,
    /// Between the start tag and the end tag; empty for `<x/>`.
    pub inner: Range<usize>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Inside an `mc:AlternateContent` branch a reader skips (a later
    /// `Choice`, or the `Fallback` after a taken one). Anchors never count
    /// skipped nodes, matching [`crate::xml::walk`].
    pub skipped: bool,
}

impl Node {
    pub fn local(&self) -> &str {
        self.element.local()
    }

    pub fn is_empty_element(&self) -> bool {
        self.inner.is_empty() && self.inner.start == self.span.end
    }
}

/// Every element of a part in document order, with the same `DOCTYPE`,
/// depth and attribute bounds as a read.
#[derive(Debug, Clone)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

impl Tree {
    pub fn parse(bytes: &[u8], part: &str, limits: &Limits) -> Result<Self, Error> {
        if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
            return Err(xml_error(part, "UTF-16 parts are not supported"));
        }
        let mut reader = quick_xml::Reader::from_reader(bytes);
        reader.config_mut().check_end_names = true;
        let mut buffer = Vec::new();
        let mut nodes: Vec<Node> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        let mut alternates: Vec<bool> = Vec::new();
        let mut skipping: Option<usize> = None;
        loop {
            let before = position(&reader);
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| xml_error(part, &error.to_string()))?;
            let after = position(&reader);
            match event {
                Event::Start(start) | Event::Empty(start) => {
                    let empty = matches!(
                        bytes
                            .get(before..after)
                            .and_then(|tag| tag.get(tag.len().saturating_sub(2)..)),
                        Some(b"/>")
                    );
                    if open.len() + 1 > limits.max_xml_depth {
                        return Err(Error::TooDeep(part.to_string()));
                    }
                    let element = crate::xml::element_of(&start, part, limits)?;
                    let parent = open.last().copied();
                    let skipped = skipping.is_some() || {
                        let skip = skip_alternate(&element, &mut alternates);
                        if skip {
                            skipping = Some(open.len() + 1);
                        }
                        skip
                    };
                    if !skipped && element.local() == "AlternateContent" && !empty {
                        alternates.push(false);
                    }
                    let index = nodes.len();
                    nodes.push(Node {
                        element,
                        span: before..after,
                        inner: after..after,
                        parent,
                        children: Vec::new(),
                        skipped,
                    });
                    if let Some(parent) = parent {
                        nodes[parent].children.push(index);
                    }
                    if empty {
                        if skipping == Some(open.len() + 1) {
                            skipping = None;
                        }
                    } else {
                        open.push(index);
                    }
                }
                Event::End(_) => {
                    let Some(index) = open.pop() else {
                        return Err(xml_error(part, "unbalanced end tag"));
                    };
                    let node = &mut nodes[index];
                    node.inner = node.inner.start..before;
                    node.span = node.span.start..after;
                    if skipping == Some(open.len() + 1) {
                        skipping = None;
                    } else if !node.skipped && node.element.local() == "AlternateContent" {
                        alternates.pop();
                    }
                }
                Event::DocType(_) => return Err(Error::DocType(part.to_string())),
                Event::Eof => {
                    if nodes.is_empty() {
                        return Err(xml_error(part, "no root element"));
                    }
                    if !open.is_empty() {
                        return Err(xml_error(part, "unexpected end of document"));
                    }
                    return Ok(Self { nodes });
                }
                _ => {}
            }
            buffer.clear();
        }
    }

    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }

    /// Visible (not skipped) descendants of `index` with local name
    /// `local`, in document order.
    pub fn descendants<'a>(
        &'a self,
        index: usize,
        local: &'a str,
    ) -> impl Iterator<Item = usize> + 'a {
        let span = self.nodes[index].span.clone();
        (index + 1..self.nodes.len())
            .take_while(move |candidate| self.nodes[*candidate].span.start < span.end)
            .filter(move |candidate| {
                let node = &self.nodes[*candidate];
                !node.skipped && node.local() == local
            })
    }

    /// Visible direct children of `index` with local name `local`.
    pub fn children<'a>(
        &'a self,
        index: usize,
        local: &'a str,
    ) -> impl Iterator<Item = usize> + 'a {
        self.nodes[index]
            .children
            .iter()
            .copied()
            .filter(move |child| {
                let node = &self.nodes[*child];
                !node.skipped && node.local() == local
            })
    }

    /// Prefix (with its colon, or empty for a default namespace) the root
    /// binds to `namespace`, if any.
    pub fn prefix_for(&self, namespace: &str) -> Option<String> {
        self.root()
            .element
            .attributes
            .iter()
            .find_map(|(key, value)| {
                if value != namespace {
                    return None;
                }
                if key == "xmlns" {
                    Some(String::new())
                } else {
                    key.strip_prefix("xmlns:")
                        .map(|prefix| format!("{prefix}:"))
                }
            })
    }
}

fn position(reader: &quick_xml::Reader<&[u8]>) -> usize {
    usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
}

fn skip_alternate(element: &Element, alternates: &mut [bool]) -> bool {
    let Some(taken) = alternates.last_mut() else {
        return false;
    };
    match element.local() {
        "Choice" if *taken => true,
        "Choice" => {
            *taken = true;
            false
        }
        "Fallback" => *taken,
        _ => false,
    }
}

/// Byte-range replacements over one part. Ranges must not overlap; an
/// insertion is an empty range.
#[derive(Debug, Default, Clone)]
pub struct Splice {
    edits: Vec<(Range<usize>, Vec<u8>)>,
}

impl Splice {
    pub fn replace(&mut self, range: Range<usize>, bytes: impl Into<Vec<u8>>) {
        self.edits.push((range, bytes.into()));
    }

    pub fn insert(&mut self, at: usize, bytes: impl Into<Vec<u8>>) {
        self.edits.push((at..at, bytes.into()));
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    pub fn apply(mut self, source: &[u8], part: &str) -> Result<Vec<u8>, Error> {
        self.edits
            .sort_by_key(|(range, _)| (range.start, range.end));
        let mut out = Vec::with_capacity(source.len());
        let mut cursor = 0;
        for (range, bytes) in &self.edits {
            if range.start < cursor || range.end > source.len() || range.start > range.end {
                return Err(xml_error(part, "overlapping or out-of-range edits"));
            }
            out.extend_from_slice(&source[cursor..range.start]);
            out.extend_from_slice(bytes);
            cursor = range.end;
        }
        out.extend_from_slice(&source[cursor..]);
        Ok(out)
    }
}

/// Escapes text content.
pub fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escapes an attribute value written in double quotes.
pub fn escape_attr(text: &str) -> String {
    escape_text(text).replace('"', "&quot;")
}

/// Rebuilds a start tag from an element, with `attributes` replacing the
/// element's own. Used only for tags Vak changes; untouched tags are never
/// rewritten.
pub fn start_tag(name: &str, attributes: &[(String, String)], empty: bool) -> String {
    let mut tag = format!("<{name}");
    for (key, value) in attributes {
        tag.push_str(&format!(" {key}=\"{}\"", escape_attr(value)));
    }
    tag.push_str(if empty { "/>" } else { ">" });
    tag
}

pub fn has_local(node: &Node, local: &str) -> bool {
    local_name(&node.element.name) == local
}

fn xml_error(part: &str, message: &str) -> Error {
    Error::Xml {
        part: part.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn spans_cover_tags_exactly() {
        let xml = br#"<?xml version="1.0"?><a x="1"><b>hi</b><c/></a>"#;
        let tree = Tree::parse(xml, "t", &Limits::default()).unwrap();
        let text = std::str::from_utf8(xml).unwrap();
        assert_eq!(
            &text[tree.nodes[0].span.clone()],
            r#"<a x="1"><b>hi</b><c/></a>"#
        );
        assert_eq!(&text[tree.nodes[1].span.clone()], "<b>hi</b>");
        assert_eq!(&text[tree.nodes[1].inner.clone()], "hi");
        assert_eq!(&text[tree.nodes[2].span.clone()], "<c/>");
        assert!(tree.nodes[2].is_empty_element());
        assert_eq!(tree.nodes[0].children, vec![1, 2]);
    }

    #[test]
    fn splice_keeps_everything_else_byte_identical() {
        let xml = b"<a>  <b>old</b>\n<!-- keep --><c/></a>";
        let tree = Tree::parse(xml, "t", &Limits::default()).unwrap();
        let mut splice = Splice::default();
        splice.replace(tree.nodes[1].inner.clone(), "new");
        splice.insert(tree.nodes[2].span.end, "<d/>");
        let out = splice.apply(xml, "t").unwrap();
        assert_eq!(out, b"<a>  <b>new</b>\n<!-- keep --><c/><d/></a>");
        let mut overlapping = Splice::default();
        overlapping.replace(0..5, "x");
        overlapping.replace(3..6, "y");
        assert!(overlapping.apply(xml, "t").is_err());
    }

    #[test]
    fn skipped_alternate_branches_are_marked() {
        let xml = br#"<r xmlns:mc="m"><mc:AlternateContent><mc:Choice><p/></mc:Choice><mc:Fallback><p/></mc:Fallback></mc:AlternateContent><p/></r>"#;
        let tree = Tree::parse(xml, "t", &Limits::default()).unwrap();
        let visible: Vec<usize> = tree.descendants(0, "p").collect();
        assert_eq!(visible.len(), 2, "the fallback copy is not counted");
    }

    #[test]
    fn prefixes_are_read_from_the_root() {
        let xml = br#"<x:doc xmlns:x="urn:w" xmlns="urn:d"/>"#;
        let tree = Tree::parse(xml, "t", &Limits::default()).unwrap();
        assert_eq!(tree.prefix_for("urn:w").as_deref(), Some("x:"));
        assert_eq!(tree.prefix_for("urn:d").as_deref(), Some(""));
        assert_eq!(tree.prefix_for("urn:none"), None);
    }
}
