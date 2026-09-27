//! Characters (ISO 32000-2 §7.9.2, §9.6.5, §9.10): the single-byte base
//! encodings, glyph names, text strings, and ToUnicode CMaps.

use std::collections::HashMap;

use crate::lexer::{Lexer, Token};

/// WinAnsiEncoding codes 128–159 (Windows-1252); 0 is unassigned.
const WIN_ANSI_HIGH: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC,
    0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

/// MacRomanEncoding codes 128–255.
const MAC_ROMAN_HIGH: [u16; 128] = [
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4, 0x00E3,
    0x00E5, 0x00E7, 0x00E9, 0x00E8, 0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3,
    0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, 0x2020, 0x00B0, 0x00A2, 0x00A3,
    0x00A7, 0x2022, 0x00B6, 0x00DF, 0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8,
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211, 0x220F, 0x03C0, 0x222B, 0x00AA,
    0x00BA, 0x03A9, 0x00E6, 0x00F8, 0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB,
    0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153, 0x2013, 0x2014, 0x201C, 0x201D,
    0x2018, 0x2019, 0x00F7, 0x25CA, 0x00FF, 0x0178, 0x2044, 0x00A4, 0x2039, 0x203A, 0xFB01, 0xFB02,
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, 0x00CB, 0x00C8, 0x00CD, 0x00CE,
    0x00CF, 0x00CC, 0x00D3, 0x00D4, 0, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC,
    0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];

/// StandardEncoding codes 160–255; 0 is unassigned.
const STANDARD_HIGH: [u16; 96] = [
    0, 0x00A1, 0x00A2, 0x00A3, 0x2044, 0x00A5, 0x0192, 0x00A7, 0x00A4, 0x0027, 0x201C, 0x00AB,
    0x2039, 0x203A, 0xFB01, 0xFB02, 0, 0x2013, 0x2020, 0x2021, 0x00B7, 0, 0x00B6, 0x2022, 0x201A,
    0x201E, 0x201D, 0x00BB, 0x2026, 0x2030, 0, 0x00BF, 0, 0x0060, 0x00B4, 0x02C6, 0x02DC, 0x00AF,
    0x02D8, 0x02D9, 0x00A8, 0, 0x02DA, 0x00B8, 0, 0x02DD, 0x02DB, 0x02C7, 0x2014, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00C6, 0, 0x00AA, 0, 0, 0, 0, 0x0141, 0x00D8, 0x0152, 0x00BA, 0,
    0, 0, 0, 0, 0x00E6, 0, 0, 0, 0x0131, 0, 0, 0x0142, 0x00F8, 0x0153, 0x00DF, 0, 0, 0, 0,
];

/// PDFDocEncoding codes 24–31.
const PDF_DOC_LOW: [u16; 8] = [
    0x02D8, 0x02C7, 0x02C6, 0x02D9, 0x02DD, 0x02DB, 0x02DA, 0x02DC,
];

/// PDFDocEncoding codes 128–160; 0 is unassigned.
const PDF_DOC_HIGH: [u16; 33] = [
    0x2022, 0x2020, 0x2021, 0x2026, 0x2014, 0x2013, 0x0192, 0x2044, 0x2039, 0x203A, 0x2212, 0x2030,
    0x201E, 0x201C, 0x201D, 0x2018, 0x2019, 0x201A, 0x2122, 0xFB01, 0xFB02, 0x0141, 0x0152, 0x0160,
    0x0178, 0x017D, 0x0131, 0x0142, 0x0153, 0x0161, 0x017E, 0, 0x20AC,
];

fn from_table(table: &[u16], index: usize) -> Option<char> {
    table
        .get(index)
        .copied()
        .filter(|unit| *unit != 0)
        .and_then(|unit| char::from_u32(u32::from(unit)))
}

/// Latin-1 above 160, with the no-break space read as a space and the soft
/// hyphen as a hyphen, as both are drawn.
fn latin1(code: u8) -> Option<char> {
    match code {
        0xA0 => Some(' '),
        0xAD => Some('-'),
        0xA1..=0xFF => Some(char::from(code)),
        _ => None,
    }
}

pub(crate) fn win_ansi(code: u8) -> Option<char> {
    match code {
        0x20..=0x7E => Some(char::from(code)),
        0x80..=0x9F => from_table(&WIN_ANSI_HIGH, usize::from(code - 0x80)),
        _ => latin1(code),
    }
}

pub(crate) fn mac_roman(code: u8) -> Option<char> {
    match code {
        0x20..=0x7E => Some(char::from(code)),
        0xCA => Some(' '),
        0x80..=0xFF => from_table(&MAC_ROMAN_HIGH, usize::from(code - 0x80)),
        _ => None,
    }
}

pub(crate) fn standard(code: u8) -> Option<char> {
    match code {
        0x27 => Some('\u{2019}'),
        0x60 => Some('\u{2018}'),
        0x20..=0x7E => Some(char::from(code)),
        0xA0..=0xFF => from_table(&STANDARD_HIGH, usize::from(code - 0xA0)),
        _ => None,
    }
}

pub(crate) fn pdf_doc(code: u8) -> Option<char> {
    match code {
        b'\t' | b'\n' | b'\r' => Some(' '),
        0x18..=0x1F => from_table(&PDF_DOC_LOW, usize::from(code - 0x18)),
        0x20..=0x7E => Some(char::from(code)),
        0x80..=0xA0 => from_table(&PDF_DOC_HIGH, usize::from(code - 0x80)),
        0xAD => None,
        _ => latin1(code),
    }
}

/// A text string (§7.9.2.2): UTF-16BE or UTF-8 behind a byte-order mark,
/// otherwise PDFDocEncoding. Control characters read as spaces.
pub fn text_string(bytes: &[u8]) -> String {
    let text: String = if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        bytes.iter().filter_map(|byte| pdf_doc(*byte)).collect()
    };
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

pub(crate) fn utf16(bytes: &[u8]) -> String {
    char::decode_utf16(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair)),
    )
    .map(|unit| unit.unwrap_or('\u{FFFD}'))
    .collect()
}

/// The text a glyph name stands for (the Adobe Glyph List conventions):
/// a listed name, `uniXXXX`, `uXXXX`, a ligature of names joined by `_`,
/// or any of those with a `.suffix` variant.
pub(crate) fn glyph(name: &[u8]) -> Option<String> {
    let name = std::str::from_utf8(name).ok()?;
    let base = match name.split_once('.') {
        Some(("", _)) => return None,
        Some((base, _)) => base,
        None => name,
    };
    if base.contains('_') {
        let parts: Option<String> = base.split('_').map(|part| glyph(part.as_bytes())).collect();
        return parts.filter(|text| !text.is_empty());
    }
    if let Some(hex) = base.strip_prefix("uni")
        && !hex.is_empty()
        && hex.len() % 4 == 0
        && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        let units: Vec<u16> = hex
            .as_bytes()
            .chunks(4)
            .filter_map(|chunk| {
                std::str::from_utf8(chunk)
                    .ok()
                    .and_then(|digits| u16::from_str_radix(digits, 16).ok())
            })
            .collect();
        let text: String = char::decode_utf16(units).filter_map(Result::ok).collect();
        return (!text.is_empty()).then_some(text);
    }
    if let Some(hex) = base.strip_prefix('u')
        && (4..=6).contains(&hex.len())
        && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .map(String::from);
    }
    if base.len() == 1 && base.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Some(base.to_string());
    }
    if let Some(ligature) = match base {
        "ff" => Some("ff"),
        "fi" => Some("fi"),
        "fl" => Some("fl"),
        "ffi" => Some("ffi"),
        "ffl" => Some("ffl"),
        _ => None,
    } {
        return Some(ligature.to_string());
    }
    named(base).map(String::from)
}

fn named(name: &str) -> Option<char> {
    Some(match name {
        "space" | "nbspace" | "nonbreakingspace" => ' ',
        "exclam" => '!',
        "quotedbl" => '"',
        "numbersign" => '#',
        "dollar" => '$',
        "percent" => '%',
        "ampersand" => '&',
        "quotesingle" => '\'',
        "parenleft" => '(',
        "parenright" => ')',
        "asterisk" => '*',
        "plus" => '+',
        "comma" => ',',
        "hyphen" | "sfthyphen" | "softhyphen" => '-',
        "period" => '.',
        "slash" => '/',
        "zero" => '0',
        "one" => '1',
        "two" => '2',
        "three" => '3',
        "four" => '4',
        "five" => '5',
        "six" => '6',
        "seven" => '7',
        "eight" => '8',
        "nine" => '9',
        "colon" => ':',
        "semicolon" => ';',
        "less" => '<',
        "equal" => '=',
        "greater" => '>',
        "question" => '?',
        "at" => '@',
        "bracketleft" => '[',
        "backslash" => '\\',
        "bracketright" => ']',
        "asciicircum" => '^',
        "underscore" => '_',
        "grave" => '`',
        "braceleft" => '{',
        "bar" => '|',
        "braceright" => '}',
        "asciitilde" => '~',
        "quoteleft" => '\u{2018}',
        "quoteright" => '\u{2019}',
        "quotedblleft" => '\u{201C}',
        "quotedblright" => '\u{201D}',
        "quotesinglbase" => '\u{201A}',
        "quotedblbase" => '\u{201E}',
        "guillemotleft" => '\u{00AB}',
        "guillemotright" => '\u{00BB}',
        "guilsinglleft" => '\u{2039}',
        "guilsinglright" => '\u{203A}',
        "endash" => '\u{2013}',
        "emdash" => '\u{2014}',
        "bullet" => '\u{2022}',
        "ellipsis" => '\u{2026}',
        "dagger" => '\u{2020}',
        "daggerdbl" => '\u{2021}',
        "perthousand" => '\u{2030}',
        "trademark" => '\u{2122}',
        "copyright" => '\u{00A9}',
        "registered" => '\u{00AE}',
        "degree" => '\u{00B0}',
        "section" => '\u{00A7}',
        "paragraph" => '\u{00B6}',
        "periodcentered" | "middot" => '\u{00B7}',
        "minus" => '\u{2212}',
        "multiply" => '\u{00D7}',
        "divide" => '\u{00F7}',
        "plusminus" => '\u{00B1}',
        "fraction" => '\u{2044}',
        "Euro" => '\u{20AC}',
        "cent" => '\u{00A2}',
        "sterling" => '\u{00A3}',
        "yen" => '\u{00A5}',
        "currency" => '\u{00A4}',
        "florin" => '\u{0192}',
        "brokenbar" => '\u{00A6}',
        "exclamdown" => '\u{00A1}',
        "questiondown" => '\u{00BF}',
        "logicalnot" => '\u{00AC}',
        "mu" => '\u{00B5}',
        "ordfeminine" => '\u{00AA}',
        "ordmasculine" => '\u{00BA}',
        "onesuperior" => '\u{00B9}',
        "twosuperior" => '\u{00B2}',
        "threesuperior" => '\u{00B3}',
        "onequarter" => '\u{00BC}',
        "onehalf" => '\u{00BD}',
        "threequarters" => '\u{00BE}',
        "macron" => '\u{00AF}',
        "acute" => '\u{00B4}',
        "cedilla" => '\u{00B8}',
        "dieresis" => '\u{00A8}',
        "circumflex" => '\u{02C6}',
        "tilde" => '\u{02DC}',
        "breve" => '\u{02D8}',
        "dotaccent" => '\u{02D9}',
        "ring" => '\u{02DA}',
        "ogonek" => '\u{02DB}',
        "caron" => '\u{02C7}',
        "hungarumlaut" => '\u{02DD}',
        "Agrave" => 'À',
        "Aacute" => 'Á',
        "Acircumflex" => 'Â',
        "Atilde" => 'Ã',
        "Adieresis" => 'Ä',
        "Aring" => 'Å',
        "AE" => 'Æ',
        "Ccedilla" => 'Ç',
        "Egrave" => 'È',
        "Eacute" => 'É',
        "Ecircumflex" => 'Ê',
        "Edieresis" => 'Ë',
        "Igrave" => 'Ì',
        "Iacute" => 'Í',
        "Icircumflex" => 'Î',
        "Idieresis" => 'Ï',
        "Eth" => 'Ð',
        "Ntilde" => 'Ñ',
        "Ograve" => 'Ò',
        "Oacute" => 'Ó',
        "Ocircumflex" => 'Ô',
        "Otilde" => 'Õ',
        "Odieresis" => 'Ö',
        "Oslash" => 'Ø',
        "Ugrave" => 'Ù',
        "Uacute" => 'Ú',
        "Ucircumflex" => 'Û',
        "Udieresis" => 'Ü',
        "Yacute" => 'Ý',
        "Thorn" => 'Þ',
        "germandbls" => 'ß',
        "agrave" => 'à',
        "aacute" => 'á',
        "acircumflex" => 'â',
        "atilde" => 'ã',
        "adieresis" => 'ä',
        "aring" => 'å',
        "ae" => 'æ',
        "ccedilla" => 'ç',
        "egrave" => 'è',
        "eacute" => 'é',
        "ecircumflex" => 'ê',
        "edieresis" => 'ë',
        "igrave" => 'ì',
        "iacute" => 'í',
        "icircumflex" => 'î',
        "idieresis" => 'ï',
        "eth" => 'ð',
        "ntilde" => 'ñ',
        "ograve" => 'ò',
        "oacute" => 'ó',
        "ocircumflex" => 'ô',
        "otilde" => 'õ',
        "odieresis" => 'ö',
        "oslash" => 'ø',
        "ugrave" => 'ù',
        "uacute" => 'ú',
        "ucircumflex" => 'û',
        "udieresis" => 'ü',
        "yacute" => 'ý',
        "thorn" => 'þ',
        "ydieresis" => 'ÿ',
        "OE" => 'Œ',
        "oe" => 'œ',
        "Scaron" => 'Š',
        "scaron" => 'š',
        "Zcaron" => 'Ž',
        "zcaron" => 'ž',
        "Ydieresis" => 'Ÿ',
        "Lslash" => 'Ł',
        "lslash" => 'ł',
        "dotlessi" => 'ı',
        "notequal" => '≠',
        "lessequal" => '≤',
        "greaterequal" => '≥',
        "infinity" => '∞',
        "partialdiff" => '∂',
        "summation" => '∑',
        "product" => '∏',
        "integral" => '∫',
        "radical" => '√',
        "approxequal" => '≈',
        "Delta" => '∆',
        "Omega" => 'Ω',
        "pi" => 'π',
        "lozenge" => '◊',
        "alpha" => 'α',
        "beta" => 'β',
        "gamma" => 'γ',
        "delta" => 'δ',
        "epsilon" => 'ε',
        "theta" => 'θ',
        "lambda" => 'λ',
        "sigma" => 'σ',
        "phi" => 'φ',
        "omega" => 'ω',
        "arrowleft" => '←',
        "arrowright" => '→',
        "arrowup" => '↑',
        "arrowdown" => '↓',
        _ => return None,
    })
}

/// Most mappings one CMap may hold, and the widest range one line may
/// expand to.
const MAX_CMAP_ENTRIES: usize = 200_000;
const MAX_RANGE: u32 = 65_535;

/// A ToUnicode CMap (§9.10.3), or the code space of an embedded encoding
/// CMap: how a string splits into character codes, and what text each
/// code stands for.
#[derive(Debug, Default)]
pub(crate) struct CMap {
    ranges: Vec<(Vec<u8>, Vec<u8>)>,
    map: HashMap<u64, String>,
}

fn key(code: u32, length: usize) -> u64 {
    (length as u64) << 32 | u64::from(code)
}

pub(crate) fn code_value(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |value, byte| value << 8 | u32::from(*byte))
}

/// The text a destination string in a CMap stands for: UTF-16BE, or one
/// byte per character when a writer left it odd.
fn destination(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes.len().is_multiple_of(2) {
        utf16(bytes)
    } else {
        bytes.iter().map(|byte| char::from(*byte)).collect()
    }
}

impl CMap {
    pub(crate) fn parse(data: &[u8]) -> CMap {
        let mut cmap = CMap::default();
        let mut lexer = Lexer::new(data, 0);
        while let Some(token) = lexer.next_token() {
            match token {
                Token::Keyword(b"begincodespacerange") => {
                    while let Some(Token::String(low)) = lexer.next_token() {
                        let Some(Token::String(high)) = lexer.next_token() else {
                            break;
                        };
                        if low.len() == high.len()
                            && (1..=4).contains(&low.len())
                            && cmap.ranges.len() < 256
                        {
                            cmap.ranges.push((low, high));
                        }
                    }
                }
                Token::Keyword(b"beginbfchar") => {
                    while let Some(Token::String(source)) = lexer.next_token() {
                        let text = match lexer.next_token() {
                            Some(Token::String(target)) => destination(&target),
                            Some(Token::Name(name)) => glyph(&name).unwrap_or_default(),
                            _ => break,
                        };
                        cmap.insert(&source, 0, text);
                    }
                }
                Token::Keyword(b"beginbfrange") => {
                    while let Some(Token::String(low)) = lexer.next_token() {
                        let Some(Token::String(high)) = lexer.next_token() else {
                            break;
                        };
                        if low.len() != high.len() || low.len() > 4 {
                            break;
                        }
                        let (first, last) = (code_value(&low), code_value(&high));
                        let span = last.saturating_sub(first).min(MAX_RANGE);
                        match lexer.next_token() {
                            Some(Token::String(target)) => {
                                for offset in 0..=span {
                                    let mut target = target.clone();
                                    bump(&mut target, offset);
                                    cmap.insert(&low, offset, destination(&target));
                                }
                            }
                            Some(Token::ArrayOpen) => {
                                let mut offset = 0u32;
                                loop {
                                    match lexer.next_token() {
                                        Some(Token::String(target)) => {
                                            if offset <= span {
                                                cmap.insert(&low, offset, destination(&target));
                                            }
                                            offset = offset.saturating_add(1);
                                        }
                                        Some(Token::ArrayClose) | None => break,
                                        Some(_) => {}
                                    }
                                }
                            }
                            _ => break,
                        }
                    }
                }
                _ => {}
            }
        }
        cmap
    }

    fn insert(&mut self, source: &[u8], offset: u32, text: String) {
        if self.map.len() >= MAX_CMAP_ENTRIES || source.is_empty() || source.len() > 4 {
            return;
        }
        let code = code_value(source).saturating_add(offset);
        self.map.insert(key(code, source.len()), text);
    }

    pub(crate) fn has_mappings(&self) -> bool {
        !self.map.is_empty()
    }

    pub(crate) fn has_ranges(&self) -> bool {
        !self.ranges.is_empty()
    }

    pub(crate) fn lookup(&self, code: u32, length: usize) -> Option<&str> {
        self.map.get(&key(code, length)).map(String::as_str)
    }

    /// How many bytes the code at the start of `bytes` takes: the first
    /// length whose bytes all fall inside one code-space range, byte by
    /// byte (§9.7.6.2), else `fallback`.
    pub(crate) fn code_length(&self, bytes: &[u8], fallback: usize) -> usize {
        for length in 1..=4 {
            let Some(code) = bytes.get(..length) else {
                break;
            };
            let inside = self.ranges.iter().any(|(low, high)| {
                low.len() == length
                    && code
                        .iter()
                        .zip(low.iter().zip(high.iter()))
                        .all(|(byte, (low, high))| low <= byte && byte <= high)
            });
            if inside {
                return length;
            }
        }
        fallback
    }
}

/// Adds `offset` to the last UTF-16 unit (or byte) of a range's target.
fn bump(target: &mut [u8], offset: u32) {
    match target.len() {
        0 => {}
        1 => {
            if let Some(last) = target.last_mut() {
                *last = last.wrapping_add(offset as u8);
            }
        }
        length => {
            if let Some(unit) = target.get_mut(length - 2..) {
                let value = u16::from_be_bytes([unit[0], unit[1]]).wrapping_add(offset as u16);
                unit.copy_from_slice(&value.to_be_bytes());
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn base_encodings_differ_where_the_specification_says() {
        assert_eq!(win_ansi(0x93), Some('\u{201C}'));
        assert_eq!(win_ansi(0xE9), Some('é'));
        assert_eq!(standard(0x27), Some('\u{2019}'));
        assert_eq!(standard(0xAE), Some('\u{FB01}'));
        assert_eq!(mac_roman(0x8E), Some('é'));
        assert_eq!(pdf_doc(0x84), Some('\u{2014}'));
    }

    #[test]
    fn text_strings_and_glyph_names() {
        assert_eq!(text_string(b"\xfe\xff\x00H\x00i"), "Hi");
        assert_eq!(text_string(b"Caf\xe9"), "Café");
        assert_eq!(glyph(b"eacute").as_deref(), Some("é"));
        assert_eq!(glyph(b"uni00410042").as_deref(), Some("AB"));
        assert_eq!(glyph(b"u1F600").as_deref(), Some("😀"));
        assert_eq!(glyph(b"f_f_i").as_deref(), Some("ffi"));
        assert_eq!(glyph(b"a.sc").as_deref(), Some("a"));
        assert_eq!(glyph(b".notdef"), None);
        assert_eq!(glyph(b"g123"), None);
    }

    #[test]
    fn tounicode_cmap_chars_ranges_and_code_space() {
        let cmap = CMap::parse(
            b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
              1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
              2 beginbfchar <0001> <0048> <0002> <00660069> endbfchar\n\
              2 beginbfrange <0010> <0012> <0061> <0020> <0021> [<0058> <0059>] endbfrange\n\
              endcmap",
        );
        assert_eq!(cmap.lookup(1, 2), Some("H"));
        assert_eq!(cmap.lookup(2, 2), Some("fi"));
        assert_eq!(cmap.lookup(0x12, 2), Some("c"));
        assert_eq!(cmap.lookup(0x21, 2), Some("Y"));
        assert_eq!(cmap.code_length(&[0, 1, 0, 2], 1), 2);
    }
}
