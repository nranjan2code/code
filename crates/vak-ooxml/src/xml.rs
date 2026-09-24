//! Bounded XML event walk over one part (O2).
//!
//! `quick-xml` never expands entities, so there is no expansion attack to
//! defend against; a `DOCTYPE` is still refused, because no Open XML
//! producer emits one and a part that carries one is not an honest part.

use quick_xml::events::{BytesStart, Event};

use crate::{Error, Limits};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Qualified name as written, e.g. `w:p`.
    pub name: String,
    pub attributes: Vec<(String, String)>,
}

impl Element {
    /// Local part of the element name (`p` for `w:p`).
    pub fn local(&self) -> &str {
        local_name(&self.name)
    }

    /// First attribute whose local name matches, with or without a prefix.
    pub fn attr(&self, local: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(name, _)| local_name(name) == local)
            .map(|(_, value)| value.as_str())
    }

    /// Attribute with no prefix (`id` but not `r:id`).
    pub fn attr_unprefixed(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Prefixed attribute by local name (`r:id` for `id`).
    pub fn attr_prefixed(&self, local: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key.contains(':') && local_name(key) == local)
            .map(|(_, value)| value.as_str())
    }
}

pub fn local_name(qualified: &str) -> &str {
    qualified
        .rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(qualified)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlEvent {
    Open(Element),
    Close(String),
    Text(String),
}

/// Walks `bytes` as XML, calling `visit` for each element open, close and
/// text run. Empty elements produce an `Open` immediately followed by a
/// `Close`. Entity and character references are resolved into the text.
pub fn walk(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
    mut visit: impl FnMut(XmlEvent) -> Result<(), Error>,
) -> Result<(), Error> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(xml_error(part, "UTF-16 parts are not supported"));
    }
    let mut reader = quick_xml::Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut saw_root = false;
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| xml_error(part, &error.to_string()))?;
        match event {
            Event::Start(start) => {
                depth += 1;
                if depth > limits.max_xml_depth {
                    return Err(Error::TooDeep(part.to_string()));
                }
                saw_root = true;
                let element = element(&start, part, limits)?;
                visit(XmlEvent::Open(element))?;
            }
            Event::Empty(start) => {
                if depth + 1 > limits.max_xml_depth {
                    return Err(Error::TooDeep(part.to_string()));
                }
                saw_root = true;
                let element = element(&start, part, limits)?;
                let name = element.name.clone();
                visit(XmlEvent::Open(element))?;
                visit(XmlEvent::Close(name))?;
            }
            Event::End(end) => {
                depth = depth.saturating_sub(1);
                let name = String::from_utf8_lossy(end.name().as_ref()).into_owned();
                visit(XmlEvent::Close(name))?;
            }
            Event::Text(text) => {
                let text = text
                    .decode()
                    .map_err(|error| xml_error(part, &error.to_string()))?;
                if !text.is_empty() {
                    visit(XmlEvent::Text(text.into_owned()))?;
                }
            }
            Event::CData(data) => {
                let text = data
                    .decode()
                    .map_err(|error| xml_error(part, &error.to_string()))?;
                visit(XmlEvent::Text(text.into_owned()))?;
            }
            Event::GeneralRef(reference) => {
                let resolved = match reference
                    .resolve_char_ref()
                    .map_err(|error| xml_error(part, &error.to_string()))?
                {
                    Some(character) => character.to_string(),
                    None => {
                        let name = reference
                            .decode()
                            .map_err(|error| xml_error(part, &error.to_string()))?;
                        predefined_entity(&name)
                            .ok_or_else(|| xml_error(part, &format!("undeclared entity &{name};")))?
                            .to_string()
                    }
                };
                visit(XmlEvent::Text(resolved))?;
            }
            Event::DocType(_) => return Err(Error::DocType(part.to_string())),
            Event::Eof => {
                if !saw_root {
                    return Err(xml_error(part, "no root element"));
                }
                if depth != 0 {
                    return Err(xml_error(part, "unexpected end of document"));
                }
                return Ok(());
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
        }
        buffer.clear();
    }
}

/// Root element of a part. Reads only up to the root, with the same
/// `DOCTYPE` refusal and attribute bound as a full walk.
pub fn root(bytes: &[u8], part: &str, limits: &Limits) -> Result<Element, Error> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(xml_error(part, "UTF-16 parts are not supported"));
    }
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| xml_error(part, &error.to_string()))?
        {
            Event::Start(start) | Event::Empty(start) => return element(&start, part, limits),
            Event::DocType(_) => return Err(Error::DocType(part.to_string())),
            Event::Eof => return Err(xml_error(part, "no root element")),
            _ => {}
        }
        buffer.clear();
    }
}

fn element(start: &BytesStart<'_>, part: &str, limits: &Limits) -> Result<Element, Error> {
    let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
    let mut attributes = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| xml_error(part, &error.to_string()))?;
        if attributes.len() >= limits.max_attributes {
            return Err(Error::TooManyAttributes(part.to_string()));
        }
        let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::default())
            .map_err(|error| xml_error(part, &error.to_string()))?
            .into_owned();
        attributes.push((key, value));
    }
    Ok(Element { name, attributes })
}

fn predefined_entity(name: &str) -> Option<&'static str> {
    match name {
        "amp" => Some("&"),
        "lt" => Some("<"),
        "gt" => Some(">"),
        "quot" => Some("\""),
        "apos" => Some("'"),
        _ => None,
    }
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

    fn collect(xml: &str) -> Result<Vec<XmlEvent>, Error> {
        let mut events = Vec::new();
        walk(xml.as_bytes(), "t.xml", &Limits::default(), |event| {
            events.push(event);
            Ok(())
        })?;
        Ok(events)
    }

    #[test]
    fn resolves_references_into_text() {
        let events = collect("<a>x &amp; y &#x41;&lt;</a>").unwrap();
        let text: String = events
            .iter()
            .filter_map(|event| match event {
                XmlEvent::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "x & y A<");
    }

    #[test]
    fn refuses_doctype_and_undeclared_entities() {
        let doctype = "<?xml version=\"1.0\"?><!DOCTYPE a [<!ENTITY x \"boom\">]><a>&x;</a>";
        assert_eq!(collect(doctype), Err(Error::DocType("t.xml".into())));
        assert!(matches!(collect("<a>&x;</a>"), Err(Error::Xml { .. })));
    }

    #[test]
    fn enforces_depth_and_attribute_bounds() {
        let deep = format!("{}{}", "<a>".repeat(300), "</a>".repeat(300));
        assert_eq!(collect(&deep), Err(Error::TooDeep("t.xml".into())));
        let attributes: String = (0..300).map(|index| format!(" a{index}=\"1\"")).collect();
        assert_eq!(
            collect(&format!("<a{attributes}/>")),
            Err(Error::TooManyAttributes("t.xml".into()))
        );
    }

    #[test]
    fn rejects_truncated_and_mismatched_markup() {
        assert!(collect("<a><b></a>").is_err());
        assert!(collect("<a>").is_err());
        assert!(collect("").is_err());
    }
}
