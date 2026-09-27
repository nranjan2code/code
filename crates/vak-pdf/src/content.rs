//! L2 text extraction: a content-stream interpreter that tracks only what
//! places and styles text (ISO 32000-2 §8.4, §9.3–9.4): the graphics state
//! stack, the text matrices, fonts, fill colour, rendering mode, and form
//! XObjects. It draws nothing. Each shown string becomes a run at a page
//! position; runs on one baseline with the same labels join into a line.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use crate::Limits;
use crate::file::File;
use crate::font::Font;
use crate::lexer::Operations;
use crate::object::{Dict, Object};

pub(crate) const INVISIBLE: u8 = 1;
pub(crate) const WHITE: u8 = 2;
pub(crate) const TINY: u8 = 4;
pub(crate) const OFF_PAGE: u8 = 8;

/// The label words a line's flags stand for, in a fixed order.
pub(crate) fn label_names(flags: u8) -> Vec<String> {
    [
        (INVISIBLE, "invisible"),
        (WHITE, "white"),
        (TINY, "tiny"),
        (OFF_PAGE, "off-page"),
    ]
    .iter()
    .filter(|(flag, _)| flags & flag != 0)
    .map(|(_, name)| name.to_string())
    .collect()
}

/// Deepest `q` nesting tracked; deeper saves are counted, not stored.
const MAX_SAVES: usize = 64;

/// Most operators interpreted for one page, forms included.
const MAX_OPERATIONS: usize = 2_000_000;

/// Most distinct fonts loaded for one document.
const MAX_FONTS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Matrix([f64; 6]);

impl Matrix {
    const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn translate(x: f64, y: f64) -> Matrix {
        Matrix([1.0, 0.0, 0.0, 1.0, x, y])
    }

    fn from_operands(operands: &[Object]) -> Option<Matrix> {
        let values: Vec<f64> = operands.iter().take(6).filter_map(Object::as_f64).collect();
        let values: [f64; 6] = values.try_into().ok()?;
        Some(Matrix(values))
    }

    /// `self × other`, as PDF composes transformations.
    fn then(self, other: Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = other.0;
        Matrix([
            a * g + b * i,
            a * h + b * j,
            c * g + d * i,
            c * h + d * j,
            e * g + f * i + k,
            e * h + f * j + l,
        ])
    }
}

#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Option<Rc<Font>>,
    /// The resource name the font was selected by, for an editor that must
    /// select it again.
    font_name: Vec<u8>,
    size: f64,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
    rise: f64,
    render: i64,
    white: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            ctm: Matrix::IDENTITY,
            font: None,
            font_name: Vec::new(),
            size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render: 0,
            white: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FontKey {
    Object(u32),
    Inline(usize),
}

pub(crate) type FontCache = HashMap<FontKey, Rc<Font>>;

/// One line of a page, with where it is and which top-level operations
/// drew it, so an editor can address exactly those operations.
#[derive(Debug, Clone)]
pub(crate) struct TextLine {
    pub(crate) text: String,
    pub(crate) labels: u8,
    /// `[left, bottom, right, top]` in default user space.
    pub(crate) rect: [f64; 4],
    /// The line's text size in user space.
    pub(crate) size: f64,
    /// Indices of the page-level operations that showed its text.
    pub(crate) ops: Vec<usize>,
    /// False when part of it came from a form XObject, which an edit of the
    /// page's content cannot reach.
    pub(crate) editable: bool,
}

/// How a text-showing operator moved and what state it drew with, for an
/// editor that replaces it by an equal move.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TextOperator {
    Show,
    ShowArray,
    NextLineShow,
    NextLineSpacedShow { word: f64, character: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextOp {
    pub(crate) operator: TextOperator,
    /// Total horizontal advance in text space units.
    pub(crate) advance: f64,
    pub(crate) font: Vec<u8>,
    pub(crate) size: f64,
    pub(crate) scale: f64,
    pub(crate) char_spacing: f64,
    pub(crate) word_spacing: f64,
}

/// What one page's content yielded.
#[derive(Default)]
pub(crate) struct PageText {
    pub(crate) lines: Vec<TextLine>,
    /// Every top-level text-showing operation, by index.
    pub(crate) text_ops: HashMap<usize, TextOp>,
    pub(crate) images: usize,
    pub(crate) undecodable: BTreeSet<String>,
    pub(crate) unmapped: usize,
    /// The page had more lines or operators than the limits allow.
    pub(crate) truncated: bool,
    /// A form XObject's content could not be decoded.
    pub(crate) errors: Vec<String>,
}

struct Run {
    x: f64,
    y: f64,
    end: f64,
    size: f64,
    text: String,
    labels: u8,
    /// Continues the previous string of the same `TJ` array; `true` when
    /// the adjustment between them was wide enough to be a word space.
    joined: Option<bool>,
    /// The page-level operation that showed it; `None` inside a form.
    op: Option<usize>,
}

struct Current {
    text: String,
    /// Characters in `text`, kept as it grows.
    chars: usize,
    labels: u8,
    y: f64,
    start: f64,
    end: f64,
    size: f64,
    rect: [f64; 4],
    ops: Vec<usize>,
    editable: bool,
}

fn run_rect(run: &Run) -> [f64; 4] {
    [
        run.x.min(run.end),
        run.y - run.size * 0.25,
        run.x.max(run.end),
        run.y + run.size * 0.85,
    ]
}

struct Lines {
    lines: Vec<TextLine>,
    current: Option<Current>,
    max_lines: usize,
    max_chars: usize,
    truncated: bool,
}

impl Lines {
    fn push(&mut self, run: Run) {
        if run.text.is_empty() {
            return;
        }
        if let Some(current) = &mut self.current {
            let em = current.size.max(run.size).max(0.1);
            let same_line = (run.y - current.y).abs() <= em * 0.5 && run.labels == current.labels;
            if same_line {
                let joins = match run.joined {
                    Some(space) => {
                        if space && !current.text.ends_with(' ') && !run.text.starts_with(' ') {
                            current.text.push(' ');
                            current.chars += 1;
                        }
                        true
                    }
                    None => {
                        let gap = run.x - current.end;
                        if gap < -em * 2.0 || run.x < current.start - em {
                            false
                        } else {
                            if gap > em * 2.0 {
                                current.text.push_str("   ");
                                current.chars += 3;
                            } else if gap > em * 0.2
                                && !current.text.ends_with(' ')
                                && !run.text.starts_with(' ')
                            {
                                current.text.push(' ');
                                current.chars += 1;
                            }
                            true
                        }
                    }
                };
                let run_chars = run.text.chars().count();
                if joins && current.chars + run_chars <= self.max_chars {
                    current.text.push_str(&run.text);
                    current.chars += run_chars;
                    current.end = run.end;
                    current.size = current.size.max(run.size);
                    let [left, bottom, right, top] = run_rect(&run);
                    current.rect = [
                        current.rect[0].min(left),
                        current.rect[1].min(bottom),
                        current.rect[2].max(right),
                        current.rect[3].max(top),
                    ];
                    match run.op {
                        Some(op) if !current.ops.contains(&op) => current.ops.push(op),
                        Some(_) => {}
                        None => current.editable = false,
                    }
                    return;
                }
            }
        }
        self.flush();
        self.current = Some(Current {
            chars: run.text.chars().count(),
            rect: run_rect(&run),
            ops: run.op.into_iter().collect(),
            editable: run.op.is_some(),
            text: run.text,
            labels: run.labels,
            y: run.y,
            start: run.x,
            end: run.end,
            size: run.size,
        });
    }

    fn flush(&mut self) {
        let Some(current) = self.current.take() else {
            return;
        };
        let text = current.text.trim();
        if text.is_empty() {
            return;
        }
        if self.lines.len() >= self.max_lines {
            self.truncated = true;
            return;
        }
        self.lines.push(TextLine {
            text: text.to_string(),
            labels: current.labels,
            rect: current.rect,
            size: current.size,
            ops: current.ops,
            editable: current.editable,
        });
    }
}

pub(crate) struct Interpreter<'f, 'a, 'c> {
    file: &'f File<'a>,
    limits: &'f Limits,
    page_box: [f64; 4],
    fonts: &'c mut FontCache,
    lines: Lines,
    out: PageText,
    forms: Vec<u32>,
    operations: usize,
}

impl<'f, 'a, 'c> Interpreter<'f, 'a, 'c> {
    pub(crate) fn new(
        file: &'f File<'a>,
        limits: &'f Limits,
        page_box: [f64; 4],
        fonts: &'c mut FontCache,
    ) -> Self {
        Self {
            file,
            limits,
            page_box,
            fonts,
            lines: Lines {
                lines: Vec::new(),
                current: None,
                max_lines: limits.max_lines_per_page,
                max_chars: limits.max_line_chars,
                truncated: false,
            },
            out: PageText::default(),
            forms: Vec::new(),
            operations: 0,
        }
    }

    pub(crate) fn run_page(&mut self, content: &[u8], resources: Option<&'f Dict>) {
        self.run(content, resources, State::default(), 0);
    }

    pub(crate) fn finish(mut self) -> PageText {
        self.lines.flush();
        self.out.lines = std::mem::take(&mut self.lines.lines);
        self.out.truncated |= self.lines.truncated;
        self.out
    }

    fn run(&mut self, content: &[u8], resources: Option<&'f Dict>, mut state: State, depth: usize) {
        let mut saved: Vec<State> = Vec::new();
        let mut uncounted_saves = 0usize;
        let mut text_matrix = Matrix::IDENTITY;
        let mut line_matrix = Matrix::IDENTITY;
        let top = depth == 0;
        for (index, (operator, operands)) in Operations::new(content).enumerate() {
            let op = top.then_some(index);
            self.operations += 1;
            if self.operations > MAX_OPERATIONS {
                self.out.truncated = true;
                return;
            }
            let number = |index: usize| operands.get(index).and_then(Object::as_f64);
            match operator {
                b"q" => {
                    if saved.len() < MAX_SAVES {
                        saved.push(state.clone());
                    } else {
                        uncounted_saves += 1;
                    }
                }
                b"Q" => {
                    if uncounted_saves > 0 {
                        uncounted_saves -= 1;
                    } else if let Some(previous) = saved.pop() {
                        state = previous;
                    }
                }
                b"cm" => {
                    if let Some(matrix) = Matrix::from_operands(&operands) {
                        state.ctm = matrix.then(state.ctm);
                    }
                }
                b"BT" => {
                    text_matrix = Matrix::IDENTITY;
                    line_matrix = Matrix::IDENTITY;
                }
                b"Tf" => {
                    state.font_name = operands
                        .first()
                        .and_then(Object::as_name)
                        .map(<[u8]>::to_vec)
                        .unwrap_or_default();
                    state.font = operands
                        .first()
                        .and_then(Object::as_name)
                        .and_then(|name| self.font(resources, name));
                    if let Some(size) = number(1) {
                        state.size = size;
                    }
                }
                b"Tc" => state.char_spacing = number(0).unwrap_or(state.char_spacing),
                b"Tw" => state.word_spacing = number(0).unwrap_or(state.word_spacing),
                b"Tz" => state.scale = number(0).map(|scale| scale / 100.0).unwrap_or(state.scale),
                b"TL" => state.leading = number(0).unwrap_or(state.leading),
                b"Ts" => state.rise = number(0).unwrap_or(state.rise),
                b"Tr" => {
                    state.render = operands
                        .first()
                        .and_then(Object::as_i64)
                        .unwrap_or(state.render)
                }
                b"Td" | b"TD" => {
                    if let (Some(x), Some(y)) = (number(0), number(1)) {
                        if operator == b"TD" {
                            state.leading = -y;
                        }
                        line_matrix = Matrix::translate(x, y).then(line_matrix);
                        text_matrix = line_matrix;
                    }
                }
                b"Tm" => {
                    if let Some(matrix) = Matrix::from_operands(&operands) {
                        text_matrix = matrix;
                        line_matrix = matrix;
                    }
                }
                b"T*" => {
                    line_matrix = Matrix::translate(0.0, -state.leading).then(line_matrix);
                    text_matrix = line_matrix;
                }
                b"Tj" => {
                    if let Some(bytes) = operands.first().and_then(Object::as_string) {
                        let advance = self.show(&state, &mut text_matrix, bytes, None, op);
                        self.record(op, TextOperator::Show, advance, &state);
                    }
                }
                b"'" | b"\"" => {
                    let operator = if operator == b"\"" {
                        state.word_spacing = number(0).unwrap_or(state.word_spacing);
                        state.char_spacing = number(1).unwrap_or(state.char_spacing);
                        TextOperator::NextLineSpacedShow {
                            word: state.word_spacing,
                            character: state.char_spacing,
                        }
                    } else {
                        TextOperator::NextLineShow
                    };
                    line_matrix = Matrix::translate(0.0, -state.leading).then(line_matrix);
                    text_matrix = line_matrix;
                    let string = if operator == TextOperator::NextLineShow {
                        0
                    } else {
                        2
                    };
                    if let Some(bytes) = operands.get(string).and_then(Object::as_string) {
                        let advance = self.show(&state, &mut text_matrix, bytes, None, op);
                        self.record(op, operator, advance, &state);
                    }
                }
                b"TJ" => {
                    if let Some(items) = operands.first().and_then(Object::as_array) {
                        let mut joined = None;
                        let mut advance = 0.0;
                        for item in items {
                            if let Some(bytes) = item.as_string() {
                                advance += self.show(&state, &mut text_matrix, bytes, joined, op);
                                joined = Some(false);
                            } else if let Some(adjustment) = item.as_f64() {
                                let x = -adjustment / 1000.0 * state.size * state.scale;
                                advance += x;
                                text_matrix = Matrix::translate(x, 0.0).then(text_matrix);
                                if joined.is_some() && adjustment <= -200.0 {
                                    joined = Some(true);
                                }
                            }
                        }
                        self.record(op, TextOperator::ShowArray, advance, &state);
                    }
                }
                b"g" => state.white = number(0).is_some_and(|gray| gray >= 0.99),
                b"rg" => {
                    state.white =
                        (0..3).all(|index| number(index).is_some_and(|value| value >= 0.99))
                }
                b"k" => {
                    state.white =
                        (0..4).all(|index| number(index).is_some_and(|value| value <= 0.01))
                }
                b"sc" | b"scn" => {
                    let values: Vec<f64> = operands.iter().filter_map(Object::as_f64).collect();
                    state.white = match values.len() {
                        1 | 3 => values.iter().all(|value| *value >= 0.99),
                        4 => values.iter().all(|value| *value <= 0.01),
                        _ => false,
                    };
                }
                b"cs" => state.white = false,
                b"Do" => {
                    if let Some(name) = operands.first().and_then(Object::as_name) {
                        self.xobject(resources, name, &state, depth);
                    }
                }
                b"BI" => self.out.images += 1,
                _ => {}
            }
        }
    }

    fn record(&mut self, op: Option<usize>, operator: TextOperator, advance: f64, state: &State) {
        if let Some(op) = op {
            self.out.text_ops.insert(
                op,
                TextOp {
                    operator,
                    advance,
                    font: state.font_name.clone(),
                    size: state.size,
                    scale: state.scale,
                    char_spacing: state.char_spacing,
                    word_spacing: state.word_spacing,
                },
            );
        }
    }

    /// Shows one string and returns how far it advanced, in text space.
    fn show(
        &mut self,
        state: &State,
        text_matrix: &mut Matrix,
        bytes: &[u8],
        joined: Option<bool>,
        op: Option<usize>,
    ) -> f64 {
        let Some(font) = state.font.clone() else {
            return 0.0;
        };
        let shown = font.show(bytes);
        let advance = (shown.width * state.size
            + shown.codes as f64 * state.char_spacing
            + shown.spaces as f64 * state.word_spacing)
            * state.scale;
        let placed = text_matrix.then(state.ctm);
        let origin = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, state.rise]).then(placed);
        *text_matrix = Matrix::translate(advance, 0.0).then(*text_matrix);
        if !font.decodable {
            self.out.undecodable.insert(font.name.clone());
            return advance;
        }
        self.out.unmapped += shown.unmapped;
        let [_, _, c, d, _, _] = placed.0;
        let size = state.size.abs() * c.hypot(d);
        let [x, y] = [origin.0[4], origin.0[5]];
        let mut labels = 0;
        if matches!(state.render, 3 | 7) {
            labels |= INVISIBLE;
        } else if state.white {
            labels |= WHITE;
        }
        if size < 1.0 && !shown.text.trim().is_empty() {
            labels |= TINY;
        }
        let margin = size.max(1.0);
        let [left, bottom, right, top] = self.page_box;
        if x < left - margin || x > right + margin || y < bottom - margin || y > top + margin {
            labels |= OFF_PAGE;
        }
        let end = text_matrix.then(state.ctm).0[4];
        self.lines.push(Run {
            x,
            y,
            end,
            size,
            text: shown.text,
            labels,
            joined,
            op,
        });
        advance
    }

    fn font(&mut self, resources: Option<&'f Dict>, name: &[u8]) -> Option<Rc<Font>> {
        let file = self.file;
        let fonts = file.dict(file.lookup(resources?, b"Font"))?;
        let entry = fonts.get(name)?;
        let key = match entry {
            Object::Ref(number, _) => FontKey::Object(*number),
            _ => FontKey::Inline(entry as *const Object as usize),
        };
        if let Some(font) = self.fonts.get(&key) {
            return Some(font.clone());
        }
        let font = Rc::new(Font::load(file, file.dict(entry)?));
        if self.fonts.len() < MAX_FONTS {
            self.fonts.insert(key, font.clone());
        }
        Some(font)
    }

    fn xobject(&mut self, resources: Option<&'f Dict>, name: &[u8], state: &State, depth: usize) {
        let file = self.file;
        let Some(resources) = resources else {
            return;
        };
        let Some(entry) = file
            .dict(file.lookup(resources, b"XObject"))
            .and_then(|xobjects| xobjects.get(name))
        else {
            return;
        };
        let number = entry.as_reference().map(|(number, _)| number);
        let Object::Stream(stream) = file.resolve(entry) else {
            return;
        };
        match stream.dict.name(b"Subtype") {
            Some(b"Image") => self.out.images += 1,
            Some(b"Form") => {
                if depth >= self.limits.max_form_depth
                    || number.is_some_and(|number| self.forms.contains(&number))
                {
                    return;
                }
                let content = match file.decode(stream) {
                    Ok(content) => content,
                    Err(error) => {
                        if self.out.errors.len() < 8 {
                            self.out.errors.push(error);
                        }
                        return;
                    }
                };
                let mut inner = state.clone();
                if let Some(matrix) = file
                    .lookup(&stream.dict, b"Matrix")
                    .as_array()
                    .and_then(Matrix::from_operands)
                {
                    inner.ctm = matrix.then(inner.ctm);
                }
                let inner_resources = file
                    .dict(file.lookup(&stream.dict, b"Resources"))
                    .or(Some(resources));
                if let Some(number) = number {
                    self.forms.push(number);
                }
                self.run(&content, inner_resources, inner, depth + 1);
                if number.is_some() {
                    self.forms.pop();
                }
            }
            _ => {}
        }
    }
}
