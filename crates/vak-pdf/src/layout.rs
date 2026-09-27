//! Setting new content on pages: paragraphs in the styles a new Word
//! document offers, lists, tables and page breaks, in the standard
//! Helvetica faces with their metrics, so nothing is embedded and every
//! viewer draws the same letters. Text is WinAnsi; a character Helvetica
//! cannot draw is refused by name, never dropped.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::LazyLock;

use crate::text;
use crate::write::{real, write_string};

/// A4, the size a new Word document has, with its one-inch margins.
pub(crate) const A4: [f64; 2] = [595.28, 841.89];
const MARGIN: f64 = 72.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Face {
    Regular,
    Bold,
    Italic,
}

impl Face {
    /// The resource name new content selects the face by.
    pub(crate) fn resource(self) -> &'static [u8] {
        match self {
            Face::Regular => b"VakH",
            Face::Bold => b"VakHB",
            Face::Italic => b"VakHI",
        }
    }

    pub(crate) fn base_font(self) -> &'static [u8] {
        match self {
            Face::Regular => b"Helvetica",
            Face::Bold => b"Helvetica-Bold",
            Face::Italic => b"Helvetica-Oblique",
        }
    }
}

/// Helvetica advances, in thousandths of the size, for WinAnsi 32–255.
const HELVETICA: [u16; 224] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
    350, 556, 350, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 350, 611, 350, 350,
    222, 222, 333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 350, 500, 667, 278, 333, 556,
    556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 556,
    537, 278, 333, 333, 365, 556, 834, 834, 834, 611, 667, 667, 667, 667, 667, 667, 1000, 722, 667,
    667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722,
    722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278,
    278, 556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];

/// Helvetica-Bold advances, in thousandths of the size, for WinAnsi 32–255.
const HELVETICA_BOLD: [u16; 224] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
    350, 556, 350, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 350, 611, 350, 350,
    278, 278, 500, 500, 350, 556, 1000, 333, 1000, 556, 333, 944, 350, 500, 667, 278, 333, 556,
    556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 611,
    556, 278, 333, 333, 365, 556, 834, 834, 834, 611, 722, 722, 722, 722, 722, 722, 1000, 722, 667,
    667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722,
    722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278,
    278, 611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556,
];

/// Width of WinAnsi-encoded text, in points.
pub(crate) fn width(face: Face, bytes: &[u8], size: f64) -> f64 {
    let table = match face {
        Face::Bold => &HELVETICA_BOLD,
        Face::Regular | Face::Italic => &HELVETICA,
    };
    let units: u32 = bytes
        .iter()
        .map(|byte| {
            byte.checked_sub(32)
                .and_then(|index| table.get(usize::from(index)))
                .copied()
                .map_or(556, u32::from)
        })
        .sum();
    f64::from(units) * size / 1000.0
}

static WIN_ANSI: LazyLock<HashMap<char, u8>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    for code in (32u8..=255).rev() {
        if let Some(character) = text::win_ansi(code) {
            map.insert(character, code);
        }
    }
    map.insert(' ', 32);
    map.insert('-', b'-');
    map
});

/// Text as WinAnsi bytes. Typographic characters Helvetica lacks become
/// their plain equivalents; any other character is an error that names it.
pub(crate) fn encode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len());
    let mut missing: Vec<char> = Vec::new();
    for character in text.chars() {
        let plain: Option<&str> = match character {
            '\t' | '\u{00A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' => Some(" "),
            '\u{200B}' | '\u{FEFF}' => Some(""),
            '\u{2010}' | '\u{2011}' | '\u{2212}' => Some("-"),
            '\u{FB00}' => Some("ff"),
            '\u{FB01}' => Some("fi"),
            '\u{FB02}' => Some("fl"),
            '\u{FB03}' => Some("ffi"),
            '\u{FB04}' => Some("ffl"),
            '\u{2192}' => Some("->"),
            '\u{2190}' => Some("<-"),
            '\u{2264}' => Some("<="),
            '\u{2265}' => Some(">="),
            '\u{2260}' => Some("!="),
            '\u{2248}' => Some("~"),
            _ => None,
        };
        if let Some(plain) = plain {
            out.extend_from_slice(plain.as_bytes());
        } else if let Some(code) = WIN_ANSI.get(&character) {
            out.push(*code);
        } else if !missing.contains(&character) {
            missing.push(character);
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(format!(
            "the text has characters the PDF's Helvetica cannot draw: {} (Latin text, digits and common punctuation can be written; write these another way)",
            missing
                .iter()
                .take(12)
                .map(|character| format!("{character:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    Normal,
    Title,
    Subtitle,
    Heading(u8),
    Bullet,
    Number,
    Quote,
}

impl Style {
    /// A style by its Word name, or `None` for one a new document lacks.
    pub(crate) fn named(name: Option<&str>) -> Option<Style> {
        let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
            return Some(Style::Normal);
        };
        Some(match name.to_ascii_lowercase().as_str() {
            "normal" => Style::Normal,
            "title" => Style::Title,
            "subtitle" => Style::Subtitle,
            "heading 1" | "heading1" => Style::Heading(1),
            "heading 2" | "heading2" => Style::Heading(2),
            "heading 3" | "heading3" => Style::Heading(3),
            "list bullet" => Style::Bullet,
            "list number" => Style::Number,
            "quote" => Style::Quote,
            _ => return None,
        })
    }

    /// Face, size, line height, space before and after, and left indent.
    fn metrics(self) -> (Face, f64, f64, f64, f64, f64) {
        match self {
            Style::Normal => (Face::Regular, 11.0, 15.5, 0.0, 7.0, 0.0),
            Style::Title => (Face::Bold, 24.0, 30.0, 0.0, 12.0, 0.0),
            Style::Subtitle => (Face::Regular, 15.0, 20.0, 0.0, 12.0, 0.0),
            Style::Heading(1) => (Face::Bold, 17.0, 22.0, 12.0, 6.0, 0.0),
            Style::Heading(2) => (Face::Bold, 14.0, 19.0, 10.0, 5.0, 0.0),
            Style::Heading(_) => (Face::Bold, 12.0, 16.0, 8.0, 4.0, 0.0),
            Style::Bullet | Style::Number => (Face::Regular, 11.0, 15.5, 0.0, 3.0, 20.0),
            Style::Quote => (Face::Italic, 11.0, 15.5, 2.0, 7.0, 24.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Block {
    Paragraph {
        text: String,
        style: Style,
    },
    Table {
        rows: Vec<Vec<String>>,
        header: bool,
    },
    PageBreak,
}

/// One page of set content: its content stream and the headings on it,
/// as (level, title, top of the heading) for bookmarks.
#[derive(Debug, Default)]
pub(crate) struct Laid {
    pub(crate) content: Vec<u8>,
    pub(crate) headings: Vec<(u8, String, f64)>,
}

struct Pager {
    size: [f64; 2],
    pages: Vec<Laid>,
    y: f64,
}

impl Pager {
    fn new(size: [f64; 2]) -> Self {
        Self {
            size,
            pages: vec![Laid::default()],
            y: size[1] - MARGIN,
        }
    }

    fn page(&mut self) -> &mut Laid {
        if self.pages.is_empty() {
            self.pages.push(Laid::default());
        }
        let last = self.pages.len() - 1;
        &mut self.pages[last]
    }

    fn bottom(&self) -> f64 {
        MARGIN
    }

    fn fresh(&self) -> bool {
        (self.y - (self.size[1] - MARGIN)).abs() < 0.01
    }

    fn break_page(&mut self) {
        self.pages.push(Laid::default());
        self.y = self.size[1] - MARGIN;
    }

    /// Room for `height` more points on this page, starting a new one if
    /// there is not, unless the page is still empty.
    fn need(&mut self, height: f64) {
        if self.y - height < self.bottom() && !self.fresh() {
            self.break_page();
        }
    }

    fn text(
        &mut self,
        face: Face,
        size: f64,
        x: f64,
        baseline: f64,
        bytes: &[u8],
        gray: Option<f64>,
    ) {
        let page = self.page();
        if let Some(gray) = gray {
            let _ = write!(page.content, "{} g ", real(gray));
        }
        let _ = write!(
            page.content,
            "BT /{} {} Tf {} {} Td ",
            String::from_utf8_lossy(face.resource()),
            real(size),
            real(x),
            real(baseline)
        );
        write_string(&mut page.content, bytes);
        page.content.extend_from_slice(b" Tj ET\n");
        if gray.is_some() {
            page.content.extend_from_slice(b"0 g\n");
        }
    }
}

/// Breaks WinAnsi text into lines no wider than `limit` points: at spaces,
/// and inside a word only when the word alone is wider.
fn wrap(face: Face, size: f64, bytes: &[u8], limit: f64) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    for paragraph in bytes.split(|byte| *byte == b'\n') {
        let mut line: Vec<u8> = Vec::new();
        for word in paragraph
            .split(|byte| *byte == b' ')
            .filter(|word| !word.is_empty())
        {
            let candidate_width = if line.is_empty() {
                width(face, word, size)
            } else {
                width(face, &line, size) + width(face, b" ", size) + width(face, word, size)
            };
            if candidate_width <= limit {
                if !line.is_empty() {
                    line.push(b' ');
                }
                line.extend_from_slice(word);
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            for &byte in word {
                if !line.is_empty() && width(face, &line, size) + width(face, &[byte], size) > limit
                {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(byte);
            }
        }
        lines.push(line);
    }
    lines
}

/// Sets `blocks` on pages of `size` points. Every paragraph's text must
/// already be WinAnsi-encodable (see [`encode`]); an error names what is
/// not.
pub(crate) fn lay_out(blocks: &[Block], size: [f64; 2]) -> Result<Vec<Laid>, String> {
    let mut pager = Pager::new(size);
    let content_width = size[0] - 2.0 * MARGIN;
    let mut number = 0usize;
    let mut first = true;
    for block in blocks {
        match block {
            Block::PageBreak => {
                pager.break_page();
                number = 0;
            }
            Block::Paragraph { text, style } => {
                let (face, font_size, leading, before, after, indent) = style.metrics();
                if *style == Style::Number {
                    number += 1;
                } else {
                    number = 0;
                }
                let bytes = encode(text)?;
                let marker: Option<Vec<u8>> = match style {
                    Style::Bullet => Some(vec![0x95]),
                    Style::Number => Some(format!("{number}.").into_bytes()),
                    _ => None,
                };
                let right_indent = if *style == Style::Quote { indent } else { 0.0 };
                let lines = wrap(
                    face,
                    font_size,
                    &bytes,
                    content_width - indent - right_indent,
                );
                if !first {
                    pager.y -= before;
                }
                first = false;
                let keep = if matches!(style, Style::Heading(_) | Style::Title) {
                    leading * lines.len() as f64 + 2.0 * 15.5
                } else {
                    leading
                };
                pager.need(keep);
                if let Style::Heading(level) = style {
                    let top = pager.y;
                    pager
                        .page()
                        .headings
                        .push((*level, text.trim().to_string(), top));
                } else if *style == Style::Title {
                    let top = pager.y;
                    pager
                        .page()
                        .headings
                        .push((0, text.trim().to_string(), top));
                }
                let gray = (*style == Style::Subtitle).then_some(0.35);
                for (index, line) in lines.iter().enumerate() {
                    pager.need(leading);
                    let baseline = pager.y - font_size;
                    if index == 0
                        && let Some(marker) = &marker
                    {
                        pager.text(
                            Face::Regular,
                            font_size,
                            MARGIN + indent - 14.0,
                            baseline,
                            marker,
                            None,
                        );
                    }
                    if !line.is_empty() {
                        pager.text(face, font_size, MARGIN + indent, baseline, line, gray);
                    }
                    pager.y -= leading;
                }
                pager.y -= after;
            }
            Block::Table { rows, header } => {
                number = 0;
                if !first {
                    pager.y -= 4.0;
                }
                first = false;
                table(&mut pager, rows, *header, content_width)?;
                pager.y -= 8.0;
            }
        }
    }
    Ok(pager.pages)
}

fn table(
    pager: &mut Pager,
    rows: &[Vec<String>],
    header: bool,
    content_width: f64,
) -> Result<(), String> {
    const SIZE: f64 = 10.0;
    const LEADING: f64 = 13.0;
    const PAD: f64 = 4.0;
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return Ok(());
    }
    let encoded: Vec<Vec<Vec<u8>>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| encode(cell))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<_, _>>()?;
    let natural: Vec<f64> = (0..columns)
        .map(|column| {
            encoded
                .iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    let face = if header && index == 0 {
                        Face::Bold
                    } else {
                        Face::Regular
                    };
                    row.get(column)
                        .map(|cell| width(face, cell, SIZE) + 2.0 * PAD)
                })
                .fold(24.0, f64::max)
                .min(content_width * 0.6)
        })
        .collect();
    let total: f64 = natural.iter().sum();
    let widths: Vec<f64> = natural
        .iter()
        .map(|width| width / total * content_width)
        .collect();
    for (index, row) in encoded.iter().enumerate() {
        let face = if header && index == 0 {
            Face::Bold
        } else {
            Face::Regular
        };
        let cells: Vec<Vec<Vec<u8>>> = (0..columns)
            .map(|column| {
                let cell = row.get(column).map(Vec::as_slice).unwrap_or_default();
                wrap(face, SIZE, cell, widths[column] - 2.0 * PAD)
            })
            .collect();
        let lines = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let height = lines as f64 * LEADING + 2.0 * PAD;
        pager.need(height);
        let top = pager.y;
        let mut x = MARGIN;
        let page = pager.page();
        if header && index == 0 {
            let _ = writeln!(
                page.content,
                "0.92 g {} {} {} {} re f 0 g",
                real(MARGIN),
                real(top - height),
                real(content_width),
                real(height)
            );
        }
        let _ = writeln!(page.content, "0.6 G 0.5 w");
        for width in &widths {
            let _ = writeln!(
                page.content,
                "{} {} {} {} re S",
                real(x),
                real(top - height),
                real(*width),
                real(height)
            );
            x += width;
        }
        let _ = writeln!(page.content, "0 G");
        let mut x = MARGIN;
        for (column, cell) in cells.iter().enumerate() {
            for (line_index, line) in cell.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let baseline = top - PAD - SIZE - line_index as f64 * LEADING + 1.5;
                pager.text(face, SIZE, x + PAD, baseline, line, None);
            }
            x += widths[column];
        }
        pager.y -= height;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn widths_encoding_and_wrapping() {
        assert!((width(Face::Regular, b"Hello", 10.0) - 22.78).abs() < 0.01);
        assert_eq!(encode("Café – 5 → 6").unwrap(), b"Caf\xe9 \x96 5 -> 6");
        assert!(encode("日本").unwrap_err().contains("'日'"));
        let lines = wrap(Face::Regular, 10.0, b"one two three four", 50.0);
        assert!(lines.len() > 1);
        assert!(
            lines
                .iter()
                .all(|line| width(Face::Regular, line, 10.0) <= 50.0)
        );
    }

    #[test]
    fn long_documents_flow_onto_new_pages() {
        let blocks: Vec<Block> = (0..120)
            .map(|index| Block::Paragraph {
                text: format!("Paragraph {index} with enough words to fill a line or two of text."),
                style: if index % 40 == 0 {
                    Style::Heading(1)
                } else {
                    Style::Normal
                },
            })
            .collect();
        let pages = lay_out(&blocks, A4).unwrap();
        assert!(pages.len() >= 3, "{}", pages.len());
        assert_eq!(
            pages.iter().map(|page| page.headings.len()).sum::<usize>(),
            3
        );
    }
}
