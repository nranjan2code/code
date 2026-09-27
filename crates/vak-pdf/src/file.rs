//! L0 file structure (ISO 32000-2 §7.5): the header, cross-reference
//! tables and streams with their `/Prev` chain, object streams, and a
//! rebuild by scanning when the table is unusable. Every object is parsed
//! once, up front, inside the object limit, so later layers look objects
//! up without touching the bytes again.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::filter;
use crate::lexer::{self, Lexer, Token};
use crate::object::{Dict, NULL, Object, Stream};
use crate::{Error, Limits};

/// Most cross-reference sections followed through `/Prev`.
const MAX_REVISIONS: usize = 256;

/// Most reference hops `resolve` follows before giving up on a chain.
const MAX_HOPS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Free,
    InUse { offset: usize },
    Compressed { stream: u32, index: usize },
}

pub(crate) struct File<'a> {
    bytes: &'a [u8],
    pub(crate) version: String,
    pub(crate) trailer: Dict,
    objects: HashMap<u32, Object>,
    entries: HashMap<u32, Entry>,
    header: usize,
    /// Objects the table names that could not be parsed.
    pub(crate) damaged: usize,
    /// The table was unusable and objects were found by scanning.
    pub(crate) rebuilt: bool,
    /// Cross-reference sections followed, one per saved revision.
    pub(crate) revisions: usize,
    decoded: Cell<usize>,
    limits: Limits,
}

impl<'a> File<'a> {
    pub(crate) fn open(bytes: &'a [u8], limits: Limits) -> Result<Self, Error> {
        let header = lexer::find(
            bytes.get(..1024.min(bytes.len())).unwrap_or_default(),
            b"%PDF-",
            0,
        )
        .ok_or(Error::NotPdf)?;
        let version = bytes
            .get(header + 5..)
            .unwrap_or_default()
            .iter()
            .take(8)
            .take_while(|byte| byte.is_ascii_digit() || **byte == b'.')
            .map(|byte| char::from(*byte))
            .collect();
        let mut file = File {
            bytes,
            version,
            trailer: Dict::default(),
            objects: HashMap::new(),
            entries: HashMap::new(),
            header,
            damaged: 0,
            rebuilt: false,
            revisions: 0,
            decoded: Cell::new(0),
            limits,
        };
        let table = startxref(bytes).and_then(|offset| file.read_chain(offset).ok());
        let mut usable = false;
        if let Some((entries, trailer, revisions)) = table {
            if trailer.has(b"Encrypt") {
                return Err(Error::Encrypted);
            }
            if entries.len() > limits.max_objects {
                return Err(Error::TooManyObjects(limits.max_objects));
            }
            file.entries = entries;
            file.trailer = trailer;
            file.revisions = revisions;
            file.load()?;
            usable = file.catalog().is_some();
        }
        if !usable {
            file.rebuild()?;
        }
        if file.trailer.has(b"Encrypt") {
            return Err(Error::Encrypted);
        }
        if file.catalog().is_none() {
            return Err(Error::Malformed("it has no document catalog".into()));
        }
        Ok(file)
    }

    pub(crate) fn resolve<'b>(&'b self, object: &'b Object) -> &'b Object {
        let mut current = object;
        for _ in 0..MAX_HOPS {
            match current {
                Object::Ref(number, _) => current = self.objects.get(number).unwrap_or(&NULL),
                _ => return current,
            }
        }
        &NULL
    }

    /// The resolved value of `key` in `dict`, or null.
    pub(crate) fn lookup<'b>(&'b self, dict: &'b Dict, key: &[u8]) -> &'b Object {
        dict.get(key)
            .map(|object| self.resolve(object))
            .unwrap_or(&NULL)
    }

    pub(crate) fn dict<'b>(&'b self, object: &'b Object) -> Option<&'b Dict> {
        self.resolve(object).as_dict()
    }

    pub(crate) fn catalog(&self) -> Option<&Dict> {
        self.trailer.get(b"Root").and_then(|root| self.dict(root))
    }

    pub(crate) fn objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.values()
    }

    /// Every object by number, for a writer that copies them.
    pub(crate) fn numbered(&self) -> &HashMap<u32, Object> {
        &self.objects
    }

    /// The file's header version as it declares it (`1.7`).
    pub(crate) fn header_version(&self) -> &str {
        &self.version
    }

    /// A stream's decoded bytes, within the per-stream limit and what is
    /// left of the whole document's decoding budget.
    pub(crate) fn decode(&self, stream: &Stream) -> Result<Vec<u8>, String> {
        let used = self.decoded.get();
        let remaining = self.limits.max_decoded_bytes.saturating_sub(used);
        if remaining == 0 {
            return Err("the document's decoded content is over the size limit".into());
        }
        if stream.dict.has(b"F") {
            return Err("its data is in an external file, which is never opened".into());
        }
        let raw = self
            .bytes
            .get(stream.data.clone())
            .ok_or("its data lies outside the file")?;
        let names: Vec<&[u8]> = match self.lookup(&stream.dict, b"Filter") {
            Object::Name(name) => vec![name.as_slice()],
            Object::Array(items) => items
                .iter()
                .filter_map(|item| self.resolve(item).as_name())
                .collect(),
            _ => Vec::new(),
        };
        let params: Vec<Option<&Dict>> = match self.lookup(&stream.dict, b"DecodeParms") {
            Object::Dict(params) => vec![Some(params)],
            Object::Array(items) => items.iter().map(|item| self.dict(item)).collect(),
            _ => Vec::new(),
        };
        let filters: Vec<(&[u8], Option<&Dict>)> = names
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name, params.get(index).copied().flatten()))
            .collect();
        let data = filter::decode(raw, &filters, remaining.min(self.limits.max_stream_bytes))
            .map_err(|error| error.to_string())?;
        self.decoded.set(used.saturating_add(data.len()));
        Ok(data)
    }

    /// Every section from `start` back through `/Prev`, the newest entry
    /// for each object winning, with the newest trailer.
    fn read_chain(&self, start: usize) -> Result<(HashMap<u32, Entry>, Dict, usize), String> {
        let mut entries = HashMap::new();
        let mut newest: Option<Dict> = None;
        let mut seen = HashSet::new();
        let mut next = Some(start);
        let mut revisions = 0;
        while let Some(offset) = next.take() {
            if !seen.insert(offset) || revisions >= MAX_REVISIONS {
                break;
            }
            let trailer = self.read_section(offset, &mut entries)?;
            revisions += 1;
            next = trailer
                .get(b"Prev")
                .and_then(Object::as_i64)
                .and_then(|offset| usize::try_from(offset).ok());
            if newest.is_none() {
                newest = Some(trailer);
            }
        }
        let trailer = newest.ok_or("no cross-reference section")?;
        if !trailer.has(b"Root") {
            return Err("the trailer names no catalog".into());
        }
        Ok((entries, trailer, revisions))
    }

    /// The section at `offset`, or at `offset` past the header for a file
    /// with bytes in front of it: a classic table and its trailer, or a
    /// cross-reference stream whose dictionary is the trailer.
    fn read_section(
        &self,
        offset: usize,
        entries: &mut HashMap<u32, Entry>,
    ) -> Result<Dict, String> {
        let mut bases = vec![offset];
        if self.header > 0 {
            bases.push(offset.saturating_add(self.header));
        }
        for base in bases {
            let mut lexer = Lexer::new(self.bytes, base);
            match lexer.next_token() {
                Some(Token::Keyword(b"xref")) => return self.read_table(&mut lexer, entries),
                Some(Token::Int(_)) => {
                    if let Ok(trailer) = self.read_xref_stream(base, entries) {
                        return Ok(trailer);
                    }
                }
                _ => {}
            }
        }
        Err(format!("no cross-reference section at offset {offset}"))
    }

    fn read_table(
        &self,
        lexer: &mut Lexer<'a>,
        entries: &mut HashMap<u32, Entry>,
    ) -> Result<Dict, String> {
        let mut free = Vec::new();
        loop {
            match lexer.next_token() {
                Some(Token::Int(first)) => {
                    let Some(Token::Int(count)) = lexer.next_token() else {
                        return Err("bad cross-reference subsection".into());
                    };
                    let first =
                        u32::try_from(first).map_err(|_| "bad cross-reference subsection")?;
                    let count = usize::try_from(count)
                        .ok()
                        .filter(|count| *count <= self.limits.max_objects)
                        .ok_or("bad cross-reference subsection")?;
                    let mut shift = 0u32;
                    for index in 0..count {
                        let (
                            Some(Token::Int(offset)),
                            Some(Token::Int(generation)),
                            Some(Token::Keyword(kind)),
                        ) = (lexer.next_token(), lexer.next_token(), lexer.next_token())
                        else {
                            return Err("bad cross-reference entry".into());
                        };
                        // A common writer bug starts the first subsection at
                        // 1 while still listing object 0's free entry.
                        if index == 0 && first == 1 && kind == b"f" && generation == 65535 {
                            shift = 1;
                        }
                        let Some(number) = (index as u32)
                            .checked_add(first)
                            .and_then(|number| number.checked_sub(shift))
                        else {
                            continue;
                        };
                        if number == 0 {
                            continue;
                        }
                        match kind {
                            b"n" => {
                                let offset = usize::try_from(offset).map_err(|_| "bad offset")?;
                                entries.entry(number).or_insert(Entry::InUse { offset });
                            }
                            b"f" => free.push(number),
                            _ => return Err("bad cross-reference entry".into()),
                        }
                        if entries.len() > self.limits.max_objects {
                            return Err("too many objects".into());
                        }
                    }
                }
                Some(Token::Keyword(b"trailer")) => {
                    let Some(Object::Dict(trailer)) = lexer.object(true) else {
                        return Err("bad trailer".into());
                    };
                    // A hybrid file lists its compressed objects in a stream
                    // the trailer names; its entries come before this
                    // section's free ones.
                    if let Some(offset) = trailer
                        .get(b"XRefStm")
                        .and_then(Object::as_i64)
                        .and_then(|offset| usize::try_from(offset).ok())
                    {
                        let _ = self.read_xref_stream(offset, entries);
                    }
                    for number in free {
                        entries.entry(number).or_insert(Entry::Free);
                    }
                    return Ok(trailer);
                }
                _ => return Err("bad cross-reference table".into()),
            }
        }
    }

    fn read_xref_stream(
        &self,
        offset: usize,
        entries: &mut HashMap<u32, Entry>,
    ) -> Result<Dict, String> {
        let (_, object) = self.parse_indirect(offset, None, true)?;
        let Object::Stream(stream) = object else {
            return Err("not a cross-reference stream".into());
        };
        if stream.dict.name(b"Type") != Some(b"XRef") {
            return Err("not a cross-reference stream".into());
        }
        let data = self.decode(&stream)?;
        let widths: Vec<usize> = stream
            .dict
            .get(b"W")
            .and_then(Object::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(|width| width.as_i64().and_then(|width| usize::try_from(width).ok()))
            .collect();
        let [type_width, field_width, index_width] = widths[..] else {
            return Err("bad /W in a cross-reference stream".into());
        };
        if type_width > 8 || field_width > 8 || index_width > 8 {
            return Err("bad /W in a cross-reference stream".into());
        }
        let row = type_width + field_width + index_width;
        if row == 0 {
            return Err("bad /W in a cross-reference stream".into());
        }
        let size = stream
            .dict
            .get(b"Size")
            .and_then(Object::as_i64)
            .unwrap_or(0)
            .max(0);
        let index: Vec<i64> = stream
            .dict
            .get(b"Index")
            .and_then(Object::as_array)
            .map(|items| items.iter().filter_map(Object::as_i64).collect())
            .unwrap_or_else(|| vec![0, size]);
        let field = |bytes: &[u8]| {
            bytes
                .iter()
                .fold(0u64, |value, byte| value << 8 | u64::from(*byte))
        };
        let mut rows = data.chunks_exact(row);
        for &[first, count] in index.as_chunks::<2>().0 {
            let (Ok(first), Ok(count)) = (u32::try_from(first), u32::try_from(count)) else {
                break;
            };
            for number_offset in 0..count {
                let Some(record) = rows.next() else {
                    return Ok(stream.dict);
                };
                let Some(number) = first.checked_add(number_offset) else {
                    break;
                };
                let kind = if type_width == 0 {
                    1
                } else {
                    field(&record[..type_width])
                };
                let second = field(&record[type_width..type_width + field_width]);
                let third = field(&record[type_width + field_width..]);
                if number == 0 {
                    continue;
                }
                let entry = match kind {
                    0 => Entry::Free,
                    1 => match usize::try_from(second) {
                        Ok(offset) => Entry::InUse { offset },
                        Err(_) => continue,
                    },
                    2 => match (u32::try_from(second), usize::try_from(third)) {
                        (Ok(stream), Ok(index)) => Entry::Compressed { stream, index },
                        _ => continue,
                    },
                    _ => continue,
                };
                entries.entry(number).or_insert(entry);
                if entries.len() > self.limits.max_objects {
                    return Err("too many objects".into());
                }
            }
        }
        Ok(stream.dict)
    }

    /// The indirect object at `offset`: `n g obj`, the object, and for a
    /// stream (when `streams`) its data, found by `/Length` or, when that
    /// is wrong, by the next `endstream`.
    fn parse_indirect(
        &self,
        offset: usize,
        expected: Option<u32>,
        streams: bool,
    ) -> Result<(u32, Object), String> {
        let mut lexer = Lexer::new(self.bytes, offset);
        let (Some(Token::Int(number)), Some(Token::Int(_)), Some(Token::Keyword(b"obj"))) =
            (lexer.next_token(), lexer.next_token(), lexer.next_token())
        else {
            return Err(format!("no object at offset {offset}"));
        };
        let number = u32::try_from(number).map_err(|_| "bad object number")?;
        if expected.is_some_and(|expected| expected != number) {
            return Err(format!("object {number} found where another was expected"));
        }
        let object = lexer
            .object(true)
            .ok_or_else(|| format!("object {number} is malformed"))?;
        let Object::Dict(dict) = object else {
            return Ok((number, object));
        };
        if !streams || !matches!(lexer.next_token(), Some(Token::Keyword(b"stream"))) {
            return Ok((number, Object::Dict(dict)));
        }
        let mut start = lexer.pos;
        match self.bytes.get(start) {
            Some(b'\r') => {
                start += 1;
                if self.bytes.get(start) == Some(&b'\n') {
                    start += 1;
                }
            }
            Some(b'\n') => start += 1,
            _ => {}
        }
        let data = self
            .stream_extent(&dict, start)
            .ok_or_else(|| format!("stream {number} has no end"))?;
        Ok((number, Object::Stream(Stream { dict, data })))
    }

    fn stream_extent(&self, dict: &Dict, start: usize) -> Option<Range<usize>> {
        let declared = match dict.get(b"Length") {
            Some(Object::Int(length)) => usize::try_from(*length).ok(),
            Some(Object::Ref(number, _)) => self.length_object(*number),
            _ => None,
        };
        if let Some(end) = declared.and_then(|length| start.checked_add(length))
            && end <= self.bytes.len()
            && ends_stream(self.bytes, end)
        {
            return Some(start..end);
        }
        let found = lexer::find(self.bytes, b"endstream", start)?;
        let mut end = found;
        if end > start && self.bytes.get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        if end > start && self.bytes.get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        Some(start..end)
    }

    /// An indirect `/Length`: an integer object, read without looking for
    /// a stream so a length can never lead back to another stream.
    fn length_object(&self, number: u32) -> Option<usize> {
        let object = match self.entries.get(&number) {
            Some(Entry::InUse { offset }) => {
                self.parse_indirect(*offset, Some(number), false).ok()?.1
            }
            _ => self.objects.get(&number)?.clone(),
        };
        object
            .as_i64()
            .and_then(|length| usize::try_from(length).ok())
    }

    fn load(&mut self) -> Result<(), Error> {
        let mut compressed: HashMap<u32, Vec<(u32, usize)>> = HashMap::new();
        let entries: Vec<(u32, Entry)> = self
            .entries
            .iter()
            .map(|(number, entry)| (*number, *entry))
            .collect();
        for (number, entry) in entries {
            match entry {
                Entry::InUse { offset } => {
                    let parsed = self
                        .parse_indirect(offset, Some(number), true)
                        .or_else(|error| {
                            if self.header > 0 {
                                self.parse_indirect(
                                    offset.saturating_add(self.header),
                                    Some(number),
                                    true,
                                )
                            } else {
                                Err(error)
                            }
                        });
                    match parsed {
                        Ok((_, object)) => {
                            self.objects.insert(number, object);
                        }
                        Err(_) => self.damaged += 1,
                    }
                }
                Entry::Compressed { stream, index } => {
                    compressed.entry(stream).or_default().push((number, index));
                }
                Entry::Free => {}
            }
        }
        for (stream, members) in compressed {
            let Ok(contents) = self.object_stream(stream) else {
                self.damaged += members.len();
                continue;
            };
            for (number, index) in members {
                let found = contents
                    .get(index)
                    .filter(|(member, _)| *member == number)
                    .or_else(|| contents.iter().find(|(member, _)| *member == number));
                match found {
                    Some((_, object)) => {
                        self.objects.insert(number, object.clone());
                    }
                    None => self.damaged += 1,
                }
            }
        }
        Ok(())
    }

    /// The members of an object stream, in order, as `(number, object)`.
    fn object_stream(&self, number: u32) -> Result<Vec<(u32, Object)>, String> {
        let Some(Object::Stream(stream)) = self.objects.get(&number) else {
            return Err(format!("object stream {number} is missing"));
        };
        if stream.dict.name(b"Type") != Some(b"ObjStm") {
            return Err(format!("object {number} is not an object stream"));
        }
        let count = stream.dict.get(b"N").and_then(Object::as_i64).unwrap_or(0);
        let first = stream
            .dict
            .get(b"First")
            .and_then(Object::as_i64)
            .and_then(|first| usize::try_from(first).ok())
            .ok_or("object stream has no /First")?;
        let count = usize::try_from(count)
            .unwrap_or(0)
            .min(self.limits.max_objects);
        let data = self.decode(stream)?;
        let mut lexer = Lexer::new(&data, 0);
        let mut header = Vec::new();
        for _ in 0..count {
            match (lexer.next_token(), lexer.next_token()) {
                (Some(Token::Int(member)), Some(Token::Int(offset))) => {
                    if let (Ok(member), Ok(offset)) =
                        (u32::try_from(member), usize::try_from(offset))
                    {
                        header.push((member, offset));
                    }
                }
                _ => break,
            }
        }
        Ok(header
            .into_iter()
            .map(|(member, offset)| {
                let object = first
                    .checked_add(offset)
                    .and_then(|start| Lexer::new(&data, start).object(true))
                    .unwrap_or(Object::Null);
                (member, object)
            })
            .collect())
    }

    /// Finds every `n g obj` in the file, the last copy of each object
    /// winning as an incremental update would, then the trailer.
    fn rebuild(&mut self) -> Result<(), Error> {
        self.objects.clear();
        self.damaged = 0;
        self.revisions = 0;
        self.rebuilt = true;
        let bytes = self.bytes;
        let mut found: HashMap<u32, usize> = HashMap::new();
        let mut from = 0;
        while let Some(position) = lexer::find(bytes, b"obj", from) {
            from = position + 3;
            if position >= 3 && bytes.get(position - 3..position) == Some(b"end") {
                continue;
            }
            if bytes
                .get(position + 3)
                .is_some_and(|byte| !lexer::is_whitespace(*byte) && !lexer::is_delimiter(*byte))
            {
                continue;
            }
            if let Some((number, start)) = header_before(bytes, position) {
                found.insert(number, start);
                if found.len() > self.limits.max_objects {
                    return Err(Error::TooManyObjects(self.limits.max_objects));
                }
            }
        }
        self.entries = found
            .iter()
            .map(|(number, offset)| (*number, Entry::InUse { offset: *offset }))
            .collect();
        for (number, offset) in found {
            match self.parse_indirect(offset, Some(number), true) {
                Ok((_, object)) => {
                    self.objects.insert(number, object);
                }
                Err(_) => self.damaged += 1,
            }
        }
        let streams: Vec<u32> = self
            .objects
            .iter()
            .filter(|(_, object)| {
                object
                    .as_stream()
                    .is_some_and(|stream| stream.dict.name(b"Type") == Some(b"ObjStm"))
            })
            .map(|(number, _)| *number)
            .collect();
        for stream in streams {
            if let Ok(members) = self.object_stream(stream) {
                for (number, object) in members {
                    if self.objects.len() >= self.limits.max_objects {
                        return Err(Error::TooManyObjects(self.limits.max_objects));
                    }
                    self.objects.entry(number).or_insert(object);
                }
            }
        }
        self.trailer = self.scanned_trailer().unwrap_or_default();
        Ok(())
    }

    /// The last `trailer` dictionary naming a catalog, a cross-reference
    /// stream's dictionary that does, or failing both, one built around
    /// the catalog object itself.
    fn scanned_trailer(&self) -> Option<Dict> {
        let names_catalog =
            |dict: &Dict| dict.get(b"Root").and_then(|root| self.dict(root)).is_some();
        let mut from = 0;
        let mut last = None;
        while let Some(position) = lexer::find(self.bytes, b"trailer", from) {
            from = position + 7;
            if let Some(Object::Dict(trailer)) = Lexer::new(self.bytes, from).object(true)
                && names_catalog(&trailer)
            {
                last = Some(trailer);
            }
        }
        if last.is_some() {
            return last;
        }
        if let Some(trailer) = self
            .objects
            .values()
            .filter_map(Object::as_stream)
            .map(|stream| &stream.dict)
            .find(|dict| dict.name(b"Type") == Some(b"XRef") && names_catalog(dict))
        {
            return Some(trailer.clone());
        }
        let (number, _) = self.objects.iter().find(|(_, object)| {
            object
                .as_dict()
                .is_some_and(|dict| dict.name(b"Type") == Some(b"Catalog"))
        })?;
        Some(Dict(vec![(b"Root".to_vec(), Object::Ref(*number, 0))]))
    }
}

/// The offset `startxref` names, from the last one in the file's tail.
fn startxref(bytes: &[u8]) -> Option<usize> {
    let position = lexer::rfind(bytes, b"startxref", bytes.len().saturating_sub(64 * 1024))?;
    match Lexer::new(bytes, position + 9).next_token() {
        Some(Token::Int(offset)) => usize::try_from(offset).ok(),
        _ => None,
    }
}

fn ends_stream(bytes: &[u8], end: usize) -> bool {
    let mut lexer = Lexer::new(bytes, end);
    lexer.skip_whitespace();
    bytes
        .get(lexer.pos..)
        .is_some_and(|rest| rest.starts_with(b"endstream"))
}

/// `n g` before the `obj` at `position`, as `(n, where n starts)`.
fn header_before(bytes: &[u8], position: usize) -> Option<(u32, usize)> {
    let whitespace_back = |index: &mut usize| {
        let start = *index;
        while *index > 0
            && bytes
                .get(*index - 1)
                .is_some_and(|byte| lexer::is_whitespace(*byte))
        {
            *index -= 1;
        }
        *index < start
    };
    let digits_back = |index: &mut usize| {
        let end = *index;
        while *index > 0 && bytes.get(*index - 1).is_some_and(u8::is_ascii_digit) {
            *index -= 1;
        }
        (end > *index && end - *index <= 10).then_some(end)
    };
    let mut index = position;
    if !whitespace_back(&mut index) {
        return None;
    }
    digits_back(&mut index)?;
    if !whitespace_back(&mut index) {
        return None;
    }
    let end = digits_back(&mut index)?;
    let start = index;
    if start > 0
        && !bytes
            .get(start - 1)
            .is_some_and(|byte| lexer::is_whitespace(*byte) || lexer::is_delimiter(*byte))
    {
        return None;
    }
    let number = std::str::from_utf8(bytes.get(start..end)?)
        .ok()?
        .parse()
        .ok()?;
    Some((number, start))
}
