//! The writer (ISO 32000-2 §7.3, §7.5): PDF syntax from objects, and a
//! whole file from an object set. A file is always written whole and
//! clean: only what the trailer reaches is kept, renumbered from 1, so
//! content an edit removed is gone from the file rather than left in an
//! earlier revision. A stream the edit did not touch keeps its encoded
//! bytes exactly.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write as _;

use crate::object::{Dict, Object};

/// An object to write. A stream made in memory carries its bytes; a stream
/// copied from a source file carries none and is read from that file.
pub(crate) struct Entry {
    pub(crate) object: Object,
    pub(crate) data: Option<Vec<u8>>,
}

pub(crate) struct Objects<'s> {
    source: &'s [u8],
    entries: BTreeMap<u32, Entry>,
    next: u32,
}

impl<'s> Objects<'s> {
    pub(crate) fn new(source: &'s [u8]) -> Self {
        Self {
            source,
            entries: BTreeMap::new(),
            next: 1,
        }
    }

    /// Copies a source file's objects under their own numbers.
    pub(crate) fn copy(&mut self, objects: &HashMap<u32, Object>) {
        for (number, object) in objects {
            self.entries.insert(
                *number,
                Entry {
                    object: object.clone(),
                    data: None,
                },
            );
            self.next = self.next.max(number.saturating_add(1));
        }
    }

    pub(crate) fn add(&mut self, object: Object) -> u32 {
        let number = self.next;
        self.next += 1;
        self.entries.insert(number, Entry { object, data: None });
        number
    }

    /// Adds a stream made in memory, compressed.
    pub(crate) fn add_stream(&mut self, mut dict: Dict, data: &[u8]) -> u32 {
        dict.0
            .retain(|(key, _)| key != b"Filter" && key != b"DecodeParms" && key != b"Length");
        dict.0
            .push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
        let number = self.next;
        self.next += 1;
        self.entries.insert(
            number,
            Entry {
                object: Object::Stream(crate::object::Stream { dict, data: 0..0 }),
                data: Some(deflate(data)),
            },
        );
        number
    }

    /// Adds an already encoded stream without changing its filter data.
    pub(crate) fn add_encoded_stream(&mut self, mut dict: Dict, data: &[u8]) -> u32 {
        dict.0.retain(|(key, _)| key != b"Length");
        let number = self.next;
        self.next += 1;
        self.entries.insert(
            number,
            Entry {
                object: Object::Stream(crate::object::Stream { dict, data: 0..0 }),
                data: Some(data.to_vec()),
            },
        );
        number
    }

    pub(crate) fn set(&mut self, number: u32, object: Object) {
        let data = self.entries.remove(&number).and_then(|entry| entry.data);
        self.entries.insert(number, Entry { object, data });
    }

    pub(crate) fn remove(&mut self, number: u32) {
        self.entries.remove(&number);
    }

    pub(crate) fn get(&self, number: u32) -> Option<&Object> {
        self.entries.get(&number).map(|entry| &entry.object)
    }

    /// The file: header, every object `root` and `info` reach, renumbered
    /// from 1, a cross-reference table and the trailer.
    pub(crate) fn write(
        &self,
        version: &str,
        root: u32,
        info: Option<u32>,
        id: Option<&Object>,
    ) -> Vec<u8> {
        let mut reached = HashSet::new();
        let mut stack: Vec<u32> = std::iter::once(root).chain(info).collect();
        while let Some(number) = stack.pop() {
            if !reached.insert(number) {
                continue;
            }
            if let Some(entry) = self.entries.get(&number) {
                references(&entry.object, &mut stack, 0);
            } else {
                reached.remove(&number);
            }
        }
        let mut order: Vec<u32> = reached.into_iter().collect();
        order.sort_unstable();
        let numbers: HashMap<u32, u32> = order
            .iter()
            .enumerate()
            .map(|(index, number)| (*number, index as u32 + 1))
            .collect();
        let mut out = format!("%PDF-{version}\n%").into_bytes();
        out.extend_from_slice(&[0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
        let mut offsets = Vec::with_capacity(order.len());
        for number in &order {
            let Some(entry) = self.entries.get(number) else {
                continue;
            };
            offsets.push(out.len());
            let renumbered = numbers.get(number).copied().unwrap_or(0);
            let _ = writeln!(out, "{renumbered} 0 obj");
            match &entry.object {
                Object::Stream(stream) => {
                    let data: &[u8] = match &entry.data {
                        Some(data) => data,
                        None => self.source.get(stream.data.clone()).unwrap_or_default(),
                    };
                    let mut dict = stream.dict.clone();
                    dict.0.retain(|(key, _)| key != b"Length");
                    dict.0
                        .push((b"Length".to_vec(), Object::Int(data.len() as i64)));
                    write_dict(&mut out, &dict, &numbers, 0);
                    out.extend_from_slice(b"\nstream\n");
                    out.extend_from_slice(data);
                    out.extend_from_slice(b"\nendstream");
                }
                object => write_object(&mut out, object, &numbers, 0),
            }
            out.extend_from_slice(b"\nendobj\n");
        }
        let table = out.len();
        let _ = write!(out, "xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
        for offset in &offsets {
            let _ = writeln!(out, "{offset:010} 00000 n ");
        }
        let mut trailer = Dict(vec![
            (b"Size".to_vec(), Object::Int(offsets.len() as i64 + 1)),
            (b"Root".to_vec(), Object::Ref(root, 0)),
        ]);
        if let Some(info) = info {
            trailer.0.push((b"Info".to_vec(), Object::Ref(info, 0)));
        }
        if let Some(id @ Object::Array(_)) = id {
            trailer.0.push((b"ID".to_vec(), id.clone()));
        }
        out.extend_from_slice(b"trailer\n");
        write_dict(&mut out, &trailer, &numbers, 0);
        let _ = write!(out, "\nstartxref\n{table}\n%%EOF\n");
        out
    }
}

pub(crate) fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

fn references(object: &Object, stack: &mut Vec<u32>, depth: usize) {
    if depth > 64 {
        return;
    }
    match object {
        Object::Ref(number, _) => stack.push(*number),
        Object::Array(items) => {
            for item in items {
                references(item, stack, depth + 1);
            }
        }
        Object::Dict(dict) => {
            for (_, value) in dict.iter() {
                references(value, stack, depth + 1);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                references(value, stack, depth + 1);
            }
        }
        _ => {}
    }
}

/// A real number as PDF writes one: fixed point, at most four places, no
/// exponent.
pub(crate) fn real(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "" | "-0" | "-" => "0".into(),
        text => text.to_string(),
    }
}

pub(crate) fn write_name(out: &mut Vec<u8>, name: &[u8]) {
    out.push(b'/');
    for &byte in name {
        let plain = (0x21..=0x7E).contains(&byte)
            && !matches!(
                byte,
                b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' | b'#'
            );
        if plain {
            out.push(byte);
        } else {
            let _ = write!(out, "#{byte:02X}");
        }
    }
}

pub(crate) fn write_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.push(b'(');
    for &byte in bytes {
        match byte {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(byte);
            }
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            0..=0x1F | 0x7F => {
                let _ = write!(out, "\\{byte:03o}");
            }
            _ => out.push(byte),
        }
    }
    out.push(b')');
}

fn write_dict(out: &mut Vec<u8>, dict: &Dict, numbers: &HashMap<u32, u32>, depth: usize) {
    out.extend_from_slice(b"<<");
    for (key, value) in dict.iter() {
        write_name(out, key);
        out.push(b' ');
        write_object(out, value, numbers, depth + 1);
    }
    out.extend_from_slice(b">>");
}

/// Writes one object. A reference is renumbered through `numbers`; one to
/// an object that is not written becomes `null`.
fn write_object(out: &mut Vec<u8>, object: &Object, numbers: &HashMap<u32, u32>, depth: usize) {
    if depth > 64 {
        out.extend_from_slice(b"null");
        return;
    }
    match object {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Bool(value) => out.extend_from_slice(if *value { b"true" } else { b"false" }),
        Object::Int(value) => {
            let _ = write!(out, "{value}");
        }
        Object::Real(value) => out.extend_from_slice(real(*value).as_bytes()),
        Object::String(bytes) => write_string(out, bytes),
        Object::Name(name) => write_name(out, name),
        Object::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b' ');
                }
                write_object(out, item, numbers, depth + 1);
            }
            out.push(b']');
        }
        Object::Dict(dict) => write_dict(out, dict, numbers, depth),
        Object::Stream(stream) => write_dict(out, &stream.dict, numbers, depth),
        Object::Ref(number, _) => match numbers.get(number) {
            Some(renumbered) => {
                let _ = write!(out, "{renumbered} 0 R");
            }
            None => out.extend_from_slice(b"null"),
        },
    }
}

/// Writes an object with no references to renumber.
#[cfg(test)]
pub(crate) fn content_object(out: &mut Vec<u8>, object: &Object) {
    write_object(out, object, &HashMap::new(), 0);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn syntax_round_trips_through_the_lexer() {
        let object = Object::Dict(Dict(vec![
            (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
            (
                b"Contents".to_vec(),
                Object::String(b"a (b) \\ c\n\x01".to_vec()),
            ),
            (
                b"Rect".to_vec(),
                Object::Array(vec![
                    Object::Real(1.5),
                    Object::Int(-2),
                    Object::Real(0.00001),
                ]),
            ),
            (b"A B".to_vec(), Object::Bool(true)),
        ]));
        let mut out = Vec::new();
        content_object(&mut out, &object);
        let parsed = crate::lexer::Lexer::new(&out, 0).object(true).unwrap();
        let Object::Dict(dict) = parsed else {
            panic!("not a dictionary")
        };
        assert_eq!(
            dict.get(b"Contents"),
            Some(&Object::String(b"a (b) \\ c\n\x01".to_vec()))
        );
        assert_eq!(dict.get(b"A B"), Some(&Object::Bool(true)));
        assert_eq!(
            dict.get(b"Rect"),
            Some(&Object::Array(vec![
                Object::Real(1.5),
                Object::Int(-2),
                Object::Int(0)
            ]))
        );
        assert_eq!(real(-0.00001), "0");
        assert_eq!(real(12.0), "12");
    }
}
