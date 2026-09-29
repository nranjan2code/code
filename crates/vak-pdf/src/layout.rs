//! Setting new content on pages: paragraphs in the styles a new Word
//! document offers, lists, tables and page breaks, in the standard
//! Helvetica faces with their metrics, so nothing is embedded and every
//! viewer draws the same letters. Text is WinAnsi; a character Helvetica
//! cannot draw is refused by name, never dropped.

use std::collections::HashMap;
use std::io::Cursor;
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
    Chart {
        title: String,
        categories: Vec<String>,
        values: Vec<f64>,
    },
    Image {
        data: Vec<u8>,
        mime_type: String,
        alt_text: String,
    },
    PageBreak,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImageAsset {
    pub(crate) name: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) data: Vec<u8>,
    pub(crate) jpeg: bool,
}

impl Block {
    pub(crate) fn validate(self) -> Result<Self, String> {
        if let Self::Image {
            data,
            mime_type,
            alt_text,
        } = &self
        {
            if alt_text.trim().is_empty() || alt_text.len() > 2048 {
                return Err("an image needs alternative text from 1 to 2,048 bytes".into());
            }
            encode(&format!("Image description: {alt_text}"))?;
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
                .map_err(|_| "image data must be valid base64".to_string())?;
            if bytes.len() > 1_048_576 {
                return Err("an image may be at most 1 MiB".into());
            }
            match mime_type.as_str() {
                "image/png" => {
                    decode_png(&bytes)?;
                }
                "image/jpeg" => {
                    jpeg_dimensions(&bytes)?;
                }
                _ => return Err("image type must be image/png or image/jpeg".into()),
            }
        }
        if let Self::Chart {
            title,
            categories,
            values,
        } = &self
        {
            if title.trim().is_empty() {
                return Err("a chart needs a title".into());
            }
            if categories.is_empty() || categories.len() != values.len() || categories.len() > 20 {
                return Err("a chart needs 1–20 category labels and one value per label".into());
            }
            if values
                .iter()
                .any(|value| !value.is_finite() || value.abs() > 1.0e12)
            {
                return Err("chart values must be finite numbers between -1e12 and 1e12".into());
            }
            if categories.iter().any(|category| category.trim().is_empty()) {
                return Err("chart category labels cannot be empty".into());
            }
            encode(title)?;
            for category in categories {
                encode(category)?;
            }
        }
        Ok(self)
    }
}

/// One page of set content: its content stream and the headings on it,
/// as (level, title, top of the heading) for bookmarks.
#[derive(Debug, Default)]
pub(crate) struct Laid {
    pub(crate) content: Vec<u8>,
    pub(crate) headings: Vec<(u8, String, f64)>,
    pub(crate) images: Vec<ImageAsset>,
}

fn dimensions_ok(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 10_000
        || height > 10_000
        || u64::from(width) * u64::from(height) > 40_000_000
    {
        return Err("image dimensions must be at most 10,000 by 10,000 and 40 megapixels".into());
    }
    Ok(())
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err("JPEG data has an invalid signature".into());
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        i += 2;
        if marker == 0xd8 || marker == 0xd9 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if i + 2 > bytes.len() {
            break;
        }
        let len = usize::from(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
        if len < 2 || i + len > bytes.len() {
            break;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if len < 7 {
                break;
            }
            let h = u32::from(u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]));
            let w = u32::from(u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]));
            dimensions_ok(w, h)?;
            return Ok((w, h));
        }
        i += len;
    }
    Err("JPEG data has no supported frame dimensions".into())
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("PNG data has an invalid signature".into());
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_limits(png::Limits {
        bytes: 16 * 1024 * 1024,
    });
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("invalid PNG: {e}"))?;
    let info = reader.info();
    let width = info.width;
    let height = info.height;
    dimensions_ok(width, height)?;
    if u64::from(width) * u64::from(height) > 4_000_000 {
        return Err("PNG must be at most 4 megapixels after decoding".into());
    }
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("invalid PNG image data: {e}"))?;
    buf.truncate(frame.buffer_size());
    let channels = match frame.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Indexed => return Err("indexed PNG could not be expanded".into()),
    };
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for px in buf.chunks_exact(channels) {
        let (r, g, b, a) = match channels {
            4 => (px[0], px[1], px[2], px[3]),
            3 => (px[0], px[1], px[2], 255),
            2 => (px[0], px[0], px[0], px[1]),
            _ => (px[0], px[0], px[0], 255),
        };
        for c in [r, g, b] {
            rgb.push(
                ((u16::from(c) * u16::from(a) + 255u16 * (255u16 - u16::from(a))) / 255) as u8,
            );
        }
    }
    Ok((width, height, rgb))
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
            Block::Chart {
                title,
                categories,
                values,
            } => {
                let title = encode(title)?;
                let labels: Vec<Vec<u8>> = categories
                    .iter()
                    .map(|item| encode(item))
                    .collect::<Result<_, _>>()?;
                chart(&mut pager, &title, &labels, values, content_width)?;
                let mut rows = vec![vec!["Category".to_string(), "Value".to_string()]];
                rows.extend(
                    categories
                        .iter()
                        .zip(values)
                        .map(|(category, value)| vec![category.clone(), value.to_string()]),
                );
                pager.y -= 5.0;
                table(&mut pager, &rows, true, content_width)?;
                pager.y -= 12.0;
            }
            Block::Image {
                data,
                mime_type,
                alt_text,
            } => {
                let encoded =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
                        .map_err(|_| "image data must be valid base64".to_string())?;
                let (width, height, raw, jpeg) = if mime_type == "image/jpeg" {
                    let (w, h) = jpeg_dimensions(&encoded)?;
                    (w, h, encoded, true)
                } else {
                    let (w, h, pixels) = decode_png(&encoded)?;
                    (w, h, pixels, false)
                };
                let max_w = content_width.min(420.0);
                let max_h = 300.0;
                let scale = (max_w / f64::from(width))
                    .min(max_h / f64::from(height))
                    .min(1.0);
                let draw_w = f64::from(width) * scale;
                let draw_h = f64::from(height) * scale;
                let caption = encode(&format!("Image description: {alt_text}"))?;
                let caption_lines = wrap(Face::Regular, 9.0, &caption, content_width);
                pager.need(draw_h + 12.0 + caption_lines.len() as f64 * 12.0);
                let x = (pager.size[0] - draw_w) / 2.0;
                let y = pager.y - draw_h;
                let page = pager.page();
                let name = format!("Im{}", page.images.len() + 1).into_bytes();
                page.content.extend_from_slice(
                    format!(
                        "q {} 0 0 {} {} {} cm /{} Do Q\n",
                        real(draw_w),
                        real(draw_h),
                        real(x),
                        real(y),
                        String::from_utf8_lossy(&name)
                    )
                    .as_bytes(),
                );
                page.images.push(ImageAsset {
                    name,
                    width,
                    height,
                    data: raw,
                    jpeg,
                });
                pager.y = y - 12.0;
                for line in caption_lines {
                    pager.text(Face::Regular, 9.0, MARGIN, pager.y - 9.0, &line, None);
                    pager.y -= 12.0;
                }
                pager.y -= 8.0;
            }
        }
    }
    Ok(pager.pages)
}

fn chart(
    pager: &mut Pager,
    title: &[u8],
    categories: &[Vec<u8>],
    values: &[f64],
    width: f64,
) -> Result<(), String> {
    const TITLE_SIZE: f64 = 12.0;
    const LABEL_SIZE: f64 = 9.0;
    const LEADING: f64 = 13.0;
    if self::width(Face::Bold, title, TITLE_SIZE) > width {
        return Err("chart title is too long to fit on one line; shorten it".into());
    }
    let row_lines: Vec<Vec<Vec<u8>>> = categories
        .iter()
        .map(|label| wrap(Face::Regular, LABEL_SIZE, label, 112.0))
        .collect();
    let height = 34.0
        + row_lines
            .iter()
            .map(|lines| lines.len().max(1) as f64 * LEADING + 3.0)
            .sum::<f64>()
        + 4.0;
    if height > pager.size[1] - 2.0 * MARGIN {
        return Err("chart labels are too long to fit on one page; shorten them".into());
    }
    pager.need(height);
    let top = pager.y;
    pager.text(
        Face::Bold,
        TITLE_SIZE,
        MARGIN,
        top - TITLE_SIZE,
        title,
        None,
    );
    let label_width = 120.0;
    let value_width = 84.0;
    let bar_width = (width - label_width - value_width - 8.0).max(60.0);
    let max_value = values.iter().copied().fold(0.0f64, f64::max).max(0.0);
    let min_value = values.iter().copied().fold(0.0f64, f64::min).min(0.0);
    let span = (max_value - min_value).max(1.0);
    let zero_x = MARGIN + label_width + bar_width * (0.0 - min_value) / span;
    let mut y = top - 34.0;
    for (row_labels, value) in row_lines.iter().zip(values) {
        let row_height = row_labels.len().max(1) as f64 * LEADING;
        for (index, line) in row_labels.iter().enumerate() {
            pager.text(
                Face::Regular,
                LABEL_SIZE,
                MARGIN,
                y - LABEL_SIZE - index as f64 * LEADING,
                line,
                None,
            );
        }
        let x_value = MARGIN + label_width + bar_width * (value - min_value) / span;
        let left = zero_x.min(x_value);
        let bar = (x_value - zero_x).abs().max(1.0);
        let baseline = y - row_height + 2.0;
        let content = &mut pager.page().content;
        let _ = writeln!(
            content,
            "0.18 0.34 0.68 rg {} {} {} 9 re f 0 g",
            real(left),
            real(baseline),
            real(bar)
        );
        let rendered = format!("{value}");
        let encoded = encode(&rendered)?;
        pager.text(
            Face::Regular,
            LABEL_SIZE,
            MARGIN + label_width + bar_width + 8.0,
            y - LABEL_SIZE,
            &encoded,
            None,
        );
        y -= row_height + 3.0;
    }
    pager.y = y - 4.0;
    Ok(())
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
    let prepared: Vec<(Vec<Vec<Vec<u8>>>, f64)> = encoded
        .iter()
        .enumerate()
        .map(|(index, row)| {
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
            (cells, height)
        })
        .collect();
    let header_height = header
        .then(|| prepared.first().map(|(_, height)| *height))
        .flatten()
        .unwrap_or(0.0);
    let page_content_height = pager.size[1] - 2.0 * MARGIN;
    for (index, (cells, height)) in prepared.iter().enumerate() {
        let repeat_height = if header && index > 0 {
            header_height
        } else {
            0.0
        };
        if height + repeat_height > page_content_height {
            return Err("a table row and repeated header are too tall to fit on one page".into());
        }
        let previous_page_count = pager.pages.len();
        pager.need(height + repeat_height);
        if header && index > 0 && pager.pages.len() > previous_page_count {
            let (header_cells, header_height) = &prepared[0];
            draw_table_row(
                pager,
                header_cells,
                &widths,
                *header_height,
                content_width,
                true,
            );
        }
        draw_table_row(
            pager,
            cells,
            &widths,
            *height,
            content_width,
            header && index == 0,
        );
    }
    Ok(())
}

fn draw_table_row(
    pager: &mut Pager,
    cells: &[Vec<Vec<u8>>],
    widths: &[f64],
    height: f64,
    content_width: f64,
    header: bool,
) {
    const SIZE: f64 = 10.0;
    const LEADING: f64 = 13.0;
    const PAD: f64 = 4.0;
    let top = pager.y;
    let mut x = MARGIN;
    let page = pager.page();
    if header {
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
    for width in widths {
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
            pager.text(
                if header { Face::Bold } else { Face::Regular },
                SIZE,
                x + PAD,
                baseline,
                line,
                None,
            );
        }
        x += widths[column];
    }
    pager.y -= height;
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
