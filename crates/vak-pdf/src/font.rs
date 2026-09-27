//! Fonts as far as text needs them (ISO 32000-2 §9.6–9.10): how a shown
//! string splits into character codes, what text each code stands for,
//! and how far each advances. Glyph outlines are never read.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::file::File;
use crate::object::{Dict, Object};
use crate::text::{self, CMap};

/// Most CID widths kept for one font; CIDs are 16-bit.
const MAX_WIDTHS: usize = 65_536;

/// The advance assumed for a glyph whose width the font does not give,
/// in thousandths of the font size.
const FALLBACK_WIDTH: f64 = 500.0;

enum Widths {
    Simple {
        first: u32,
        widths: Vec<f64>,
        missing: f64,
    },
    Cid {
        default: f64,
        widths: HashMap<u32, f64>,
    },
}

pub(crate) struct Font {
    pub(crate) name: String,
    /// Codes are two bytes unless a code space says otherwise (Type0).
    two_byte: bool,
    to_unicode: Option<CMap>,
    /// An embedded encoding CMap, kept only for its code space.
    encoding_cmap: Option<CMap>,
    /// Text for each single-byte code, from the base encoding and
    /// `/Differences`.
    simple: Option<Vec<Option<String>>>,
    /// Codes are UCS-2 or UTF-16 (a predefined `Uni…-UCS2`/`-UTF16` CMap).
    unicode_codes: bool,
    widths: Widths,
    /// Glyph space to text space: 1/1000, or a Type3 font's matrix.
    scale: f64,
    /// False when no code of this font can be turned into text.
    pub(crate) decodable: bool,
}

/// One shown string: its text, total glyph advance in text space units
/// (before the font size), and the counts spacing operators apply to.
#[derive(Default)]
pub(crate) struct Shown {
    pub(crate) text: String,
    pub(crate) width: f64,
    pub(crate) codes: usize,
    pub(crate) spaces: usize,
    pub(crate) unmapped: usize,
}

impl Font {
    pub(crate) fn load(file: &File, dict: &Dict) -> Font {
        let subtype = file.lookup(dict, b"Subtype").as_name().unwrap_or(b"Type1");
        let name = file
            .lookup(dict, b"BaseFont")
            .as_name()
            .map(|name| {
                let name = String::from_utf8_lossy(name);
                // A subset prefix (`ABCDEF+`) names nothing a person knows.
                match name.split_once('+') {
                    Some((prefix, rest)) if prefix.len() == 6 => rest.to_string(),
                    _ => name.into_owned(),
                }
            })
            .unwrap_or_else(|| "unnamed font".to_string());
        let to_unicode = match file.lookup(dict, b"ToUnicode") {
            Object::Stream(stream) => file
                .decode(stream)
                .ok()
                .map(|data| CMap::parse(&data))
                .filter(CMap::has_mappings),
            _ => None,
        };
        if subtype == b"Type0" {
            let (unicode_codes, encoding_cmap) = match file.lookup(dict, b"Encoding") {
                Object::Name(encoding) => (is_unicode_cmap(encoding), None),
                Object::Stream(stream) => (
                    false,
                    file.decode(stream)
                        .ok()
                        .map(|data| CMap::parse(&data))
                        .filter(CMap::has_ranges),
                ),
                _ => (false, None),
            };
            let descendant = file
                .lookup(dict, b"DescendantFonts")
                .as_array()
                .and_then(|fonts| fonts.first())
                .and_then(|font| file.dict(font));
            let decodable = to_unicode.is_some() || unicode_codes;
            return Font {
                name,
                two_byte: true,
                to_unicode,
                encoding_cmap,
                simple: None,
                unicode_codes,
                widths: cid_widths(file, descendant),
                scale: 0.001,
                decodable,
            };
        }
        let simple = simple_encoding(file, dict, subtype, &name);
        let scale = if subtype == b"Type3" {
            file.lookup(dict, b"FontMatrix")
                .as_array()
                .and_then(|matrix| matrix.first())
                .and_then(Object::as_f64)
                .map(f64::abs)
                .filter(|scale| *scale > 0.0 && *scale < 1.0)
                .unwrap_or(0.001)
        } else {
            0.001
        };
        let decodable = to_unicode.is_some() || simple.is_some();
        Font {
            widths: simple_widths(file, dict, &name, subtype == b"Type3"),
            name,
            two_byte: false,
            to_unicode,
            encoding_cmap: None,
            simple,
            unicode_codes: false,
            scale,
            decodable,
        }
    }

    pub(crate) fn show(&self, bytes: &[u8]) -> Shown {
        let mut shown = Shown::default();
        let fallback = if self.two_byte { 2 } else { 1 };
        let mut index = 0;
        while let Some(rest) = bytes.get(index..).filter(|rest| !rest.is_empty()) {
            let length = match (&self.encoding_cmap, &self.to_unicode) {
                (Some(cmap), _) => cmap.code_length(rest, fallback),
                (None, Some(cmap)) if cmap.has_ranges() => cmap.code_length(rest, fallback),
                _ => fallback,
            }
            .clamp(1, rest.len());
            let code = text::code_value(rest.get(..length).unwrap_or_default());
            index += length;
            shown.codes += 1;
            if length == 1 && code == 32 {
                shown.spaces += 1;
            }
            shown.width += self.width(code);
            let mapped: Option<Cow<'_, str>> = self
                .to_unicode
                .as_ref()
                .and_then(|cmap| cmap.lookup(code, length))
                .map(Cow::Borrowed)
                .or_else(|| {
                    (self.unicode_codes && length == 2)
                        .then(|| char::from_u32(code))
                        .flatten()
                        .map(|character| Cow::Owned(character.to_string()))
                })
                .or_else(|| {
                    (length == 1)
                        .then_some(self.simple.as_ref())
                        .flatten()
                        .and_then(|table| table.get(code as usize))
                        .and_then(|text| text.as_deref())
                        .map(Cow::Borrowed)
                });
            match mapped {
                Some(text) => {
                    for character in text.chars() {
                        push_readable(&mut shown.text, character);
                    }
                }
                None => {
                    shown.unmapped += 1;
                    shown.text.push('\u{FFFD}');
                }
            }
        }
        shown.width *= self.scale;
        shown
    }

    fn width(&self, code: u32) -> f64 {
        match &self.widths {
            Widths::Simple {
                first,
                widths,
                missing,
            } => code
                .checked_sub(*first)
                .and_then(|index| widths.get(index as usize))
                .copied()
                .filter(|width| *width > 0.0)
                .unwrap_or(*missing),
            Widths::Cid { default, widths } => widths.get(&code).copied().unwrap_or(*default),
        }
    }
}

/// Pushes a character as a reader would type it: a Latin ligature as its
/// letters, so the text can be searched, and a control character as a space.
fn push_readable(text: &mut String, character: char) {
    match character {
        '\u{FB00}' => text.push_str("ff"),
        '\u{FB01}' => text.push_str("fi"),
        '\u{FB02}' => text.push_str("fl"),
        '\u{FB03}' => text.push_str("ffi"),
        '\u{FB04}' => text.push_str("ffl"),
        '\u{FB05}' | '\u{FB06}' => text.push_str("st"),
        character if character.is_control() => text.push(' '),
        character => text.push(character),
    }
}

/// A predefined CMap whose codes are Unicode code units.
fn is_unicode_cmap(name: &[u8]) -> bool {
    let name = String::from_utf8_lossy(name);
    name.starts_with("Uni") && (name.contains("UCS2") || name.contains("UTF16"))
}

type BaseEncoding = fn(u8) -> Option<char>;

fn named_encoding(name: &[u8]) -> Option<BaseEncoding> {
    match name {
        b"WinAnsiEncoding" => Some(text::win_ansi),
        b"MacRomanEncoding" => Some(text::mac_roman),
        b"StandardEncoding" => Some(text::standard),
        b"PDFDocEncoding" => Some(text::pdf_doc),
        _ => None,
    }
}

/// The text of each single-byte code, or `None` when the font gives no way
/// to know it: a symbol font with its own built-in encoding and no
/// `/Differences`.
fn simple_encoding(
    file: &File,
    dict: &Dict,
    subtype: &[u8],
    name: &str,
) -> Option<Vec<Option<String>>> {
    let symbolic = ["Symbol", "Dingbats", "Wingding", "Webding"]
        .iter()
        .any(|symbol| name.contains(symbol));
    let default: Option<BaseEncoding> = if symbolic || subtype == b"Type3" {
        None
    } else if subtype == b"TrueType" {
        Some(text::win_ansi)
    } else {
        Some(text::standard)
    };
    let mut differences: Vec<(u8, Option<String>)> = Vec::new();
    let base = match file.lookup(dict, b"Encoding") {
        Object::Name(encoding) => named_encoding(encoding).or(default),
        Object::Dict(encoding) => {
            if let Some(items) = file.lookup(encoding, b"Differences").as_array() {
                let mut code: Option<u8> = None;
                for item in items {
                    match file.resolve(item) {
                        Object::Int(value) => code = u8::try_from(*value).ok(),
                        Object::Name(glyph) => {
                            if let Some(current) = code {
                                differences.push((current, text::glyph(glyph)));
                                code = current.checked_add(1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            encoding
                .get(b"BaseEncoding")
                .and_then(Object::as_name)
                .and_then(named_encoding)
                .or(default)
        }
        _ => default,
    };
    if base.is_none() && differences.is_empty() {
        return None;
    }
    let mut table: Vec<Option<String>> = (0..=255u8)
        .map(|code| base.and_then(|base| base(code)).map(String::from))
        .collect();
    for (code, text) in differences {
        if let Some(slot) = table.get_mut(usize::from(code)) {
            *slot = text;
        }
    }
    Some(table)
}

fn simple_widths(file: &File, dict: &Dict, name: &str, type3: bool) -> Widths {
    let first = file
        .lookup(dict, b"FirstChar")
        .as_i64()
        .and_then(|first| u32::try_from(first).ok())
        .unwrap_or(0);
    let widths: Vec<f64> = file
        .lookup(dict, b"Widths")
        .as_array()
        .map(|widths| {
            widths
                .iter()
                .take(256)
                .map(|width| file.resolve(width).as_f64().unwrap_or(0.0))
                .collect()
        })
        .unwrap_or_default();
    let descriptor_missing = file
        .dict(file.lookup(dict, b"FontDescriptor"))
        .and_then(|descriptor| file.lookup(descriptor, b"MissingWidth").as_f64())
        .filter(|width| *width > 0.0);
    // The standard 14 fonts come without widths; Courier is monospaced.
    let missing = descriptor_missing.unwrap_or(if name.contains("Courier") {
        600.0
    } else if type3 {
        0.0
    } else {
        FALLBACK_WIDTH
    });
    Widths::Simple {
        first,
        widths,
        missing,
    }
}

fn cid_widths(file: &File, descendant: Option<&Dict>) -> Widths {
    let Some(descendant) = descendant else {
        return Widths::Cid {
            default: 1000.0,
            widths: HashMap::new(),
        };
    };
    let default = file
        .lookup(descendant, b"DW")
        .as_f64()
        .filter(|width| *width > 0.0)
        .unwrap_or(1000.0);
    let mut widths = HashMap::new();
    if let Some(items) = file.lookup(descendant, b"W").as_array() {
        let mut index = 0;
        while let Some(first) = items
            .get(index)
            .and_then(|item| file.resolve(item).as_i64())
        {
            let Ok(first) = u32::try_from(first) else {
                break;
            };
            match items.get(index + 1).map(|item| file.resolve(item)) {
                Some(Object::Array(list)) => {
                    for (offset, width) in list.iter().enumerate() {
                        if widths.len() >= MAX_WIDTHS {
                            break;
                        }
                        if let (Some(width), Some(code)) = (
                            file.resolve(width).as_f64(),
                            first.checked_add(offset as u32),
                        ) {
                            widths.insert(code, width);
                        }
                    }
                    index += 2;
                }
                Some(last) => {
                    let (Some(last), Some(width)) = (
                        last.as_i64().and_then(|last| u32::try_from(last).ok()),
                        items
                            .get(index + 2)
                            .and_then(|item| file.resolve(item).as_f64()),
                    ) else {
                        break;
                    };
                    for code in first..=last {
                        if widths.len() >= MAX_WIDTHS {
                            break;
                        }
                        widths.insert(code, width);
                    }
                    index += 3;
                }
                None => break,
            }
        }
    }
    Widths::Cid { default, widths }
}
