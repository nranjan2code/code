//! L0 lexing (ISO 32000-2 §7.2): tokens and objects from bytes, shared by
//! the file loader, object streams, content streams and CMaps. Every
//! access is bounds-checked and nesting is bounded, so no input can index
//! out of range or recurse without limit.

use crate::object::{Dict, Object};

/// Deepest array and dictionary nesting an object may have.
pub(crate) const MAX_NESTING: usize = 64;

/// Most operands kept for one content-stream operator; real operators take
/// at most a handful, so more is junk that must not grow without bound.
const MAX_OPERANDS: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token<'a> {
    Int(i64),
    Real(f64),
    String(Vec<u8>),
    Name(Vec<u8>),
    ArrayOpen,
    ArrayClose,
    DictOpen,
    DictClose,
    Keyword(&'a [u8]),
}

pub(crate) fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

pub(crate) fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// Keywords that end an object: meeting one inside an array or dictionary
/// means the object was never closed.
fn ends_object(keyword: &[u8]) -> bool {
    matches!(
        keyword,
        b"endobj" | b"obj" | b"stream" | b"endstream" | b"xref" | b"trailer" | b"startxref"
    )
}

/// The first position at or after `from` where `needle` occurs.
pub(crate) fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| position + from)
}

/// The last position at or after `from` where `needle` occurs.
pub(crate) fn rfind(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .rposition(|window| window == needle)
        .map(|position| position + from)
}

pub(crate) struct Lexer<'a> {
    data: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    pub(crate) fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek() {
            if is_whitespace(byte) {
                self.pos += 1;
            } else if byte == b'%' {
                while let Some(byte) = self.peek() {
                    if byte == b'\n' || byte == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub(crate) fn next_token(&mut self) -> Option<Token<'a>> {
        self.skip_whitespace();
        let byte = self.peek()?;
        let start = self.pos;
        match byte {
            b'(' => {
                self.pos += 1;
                Some(Token::String(self.literal_string()))
            }
            b'<' if self.data.get(start + 1) == Some(&b'<') => {
                self.pos += 2;
                Some(Token::DictOpen)
            }
            b'<' => {
                self.pos += 1;
                Some(Token::String(self.hex_string()))
            }
            b'>' if self.data.get(start + 1) == Some(&b'>') => {
                self.pos += 2;
                Some(Token::DictClose)
            }
            b'[' => {
                self.pos += 1;
                Some(Token::ArrayOpen)
            }
            b']' => {
                self.pos += 1;
                Some(Token::ArrayClose)
            }
            b'/' => {
                self.pos += 1;
                Some(Token::Name(self.name()))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => Some(self.number()),
            b'>' | b')' | b'{' | b'}' => {
                self.pos += 1;
                Some(Token::Keyword(
                    self.data.get(start..self.pos).unwrap_or_default(),
                ))
            }
            _ => {
                while let Some(byte) = self.peek() {
                    if is_whitespace(byte) || is_delimiter(byte) {
                        break;
                    }
                    self.pos += 1;
                }
                Some(Token::Keyword(
                    self.data.get(start..self.pos).unwrap_or_default(),
                ))
            }
        }
    }

    /// A number. Writers also emit doubled signs, a lone sign or a lone
    /// point; those read as viewers read them, the sign folded and an empty
    /// number as zero.
    fn number(&mut self) -> Token<'a> {
        let mut negative = false;
        while let Some(sign @ (b'+' | b'-')) = self.peek() {
            if sign == b'-' {
                negative = !negative;
            }
            self.pos += 1;
        }
        let mut integer: Option<i64> = Some(0);
        let mut value = 0f64;
        let mut fraction = false;
        let mut scale = 1f64;
        let mut digits = 0usize;
        while let Some(byte) = self.peek() {
            match byte {
                b'0'..=b'9' => {
                    let digit = byte - b'0';
                    digits += 1;
                    if fraction {
                        scale /= 10.0;
                        value += f64::from(digit) * scale;
                    } else {
                        value = value * 10.0 + f64::from(digit);
                        integer = integer
                            .and_then(|current| current.checked_mul(10))
                            .and_then(|current| current.checked_add(i64::from(digit)));
                    }
                }
                b'.' if !fraction => fraction = true,
                _ => break,
            }
            self.pos += 1;
        }
        if digits == 0 {
            return Token::Real(0.0);
        }
        match integer {
            Some(integer) if !fraction => Token::Int(if negative { -integer } else { integer }),
            _ => Token::Real(if negative { -value } else { value }),
        }
    }

    fn literal_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(byte) = self.peek() {
            self.pos += 1;
            match byte {
                b'(' => {
                    depth += 1;
                    out.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return out;
                    }
                    out.push(byte);
                }
                b'\\' => {
                    let Some(next) = self.peek() else {
                        break;
                    };
                    self.pos += 1;
                    match next {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'0'..=b'7' => {
                            let mut value = u32::from(next - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(digit @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(digit - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((value & 0xff) as u8);
                        }
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                b'\r' => {
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(byte),
            }
        }
        out
    }

    fn hex_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        while let Some(byte) = self.peek() {
            self.pos += 1;
            let nibble = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                b'>' => break,
                _ => continue,
            };
            match high.take() {
                Some(high) => out.push(high << 4 | nibble),
                None => high = Some(nibble),
            }
        }
        if let Some(high) = high {
            out.push(high << 4);
        }
        out
    }

    fn name(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let hex = |byte: Option<&u8>| byte.and_then(|byte| char::from(*byte).to_digit(16));
        while let Some(byte) = self.peek() {
            if is_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            self.pos += 1;
            if byte == b'#'
                && let (Some(high), Some(low)) = (
                    hex(self.data.get(self.pos)),
                    hex(self.data.get(self.pos + 1)),
                )
            {
                out.push((high * 16 + low) as u8);
                self.pos += 2;
                continue;
            }
            out.push(byte);
        }
        out
    }

    /// The next object. `references` reads `n g R` as a reference, which
    /// content streams and CMaps never contain.
    pub(crate) fn object(&mut self, references: bool) -> Option<Object> {
        let token = self.next_token()?;
        self.object_from(token, references)
    }

    /// The object that starts with `token`, or `None` when the token does
    /// not start one or the object is unterminated or nested too deep.
    pub(crate) fn object_from(&mut self, token: Token<'a>, references: bool) -> Option<Object> {
        self.nested(token, references, 0)
    }

    fn nested(&mut self, token: Token<'a>, references: bool, depth: usize) -> Option<Object> {
        if depth > MAX_NESTING {
            return None;
        }
        Some(match token {
            Token::Int(value) => {
                if references
                    && value >= 0
                    && let Some(reference) = self.reference_after(value)
                {
                    return Some(reference);
                }
                Object::Int(value)
            }
            Token::Real(value) => Object::Real(value),
            Token::String(bytes) => Object::String(bytes),
            Token::Name(name) => Object::Name(name),
            Token::ArrayOpen => {
                let mut items = Vec::new();
                loop {
                    match self.next_token()? {
                        Token::ArrayClose => break,
                        Token::Keyword(keyword) if ends_object(keyword) => return None,
                        Token::Keyword(b"true") => items.push(Object::Bool(true)),
                        Token::Keyword(b"false") => items.push(Object::Bool(false)),
                        Token::Keyword(b"null") => items.push(Object::Null),
                        Token::Keyword(_) | Token::DictClose => {}
                        token => items.push(self.nested(token, references, depth + 1)?),
                    }
                }
                Object::Array(items)
            }
            Token::DictOpen => {
                let mut entries = Vec::new();
                loop {
                    match self.next_token()? {
                        Token::DictClose => break,
                        Token::Name(key) => match self.next_token()? {
                            Token::DictClose => {
                                entries.push((key, Object::Null));
                                break;
                            }
                            Token::Keyword(keyword) if ends_object(keyword) => return None,
                            Token::Keyword(b"true") => entries.push((key, Object::Bool(true))),
                            Token::Keyword(b"false") => entries.push((key, Object::Bool(false))),
                            Token::Keyword(b"null") => entries.push((key, Object::Null)),
                            Token::Keyword(_) | Token::ArrayClose => {}
                            token => {
                                let value = self.nested(token, references, depth + 1)?;
                                entries.push((key, value));
                            }
                        },
                        Token::Keyword(keyword) if ends_object(keyword) => return None,
                        _ => {}
                    }
                }
                Object::Dict(Dict(entries))
            }
            Token::Keyword(b"true") => Object::Bool(true),
            Token::Keyword(b"false") => Object::Bool(false),
            Token::Keyword(b"null") => Object::Null,
            Token::Keyword(_) | Token::ArrayClose | Token::DictClose => return None,
        })
    }

    fn reference_after(&mut self, number: i64) -> Option<Object> {
        let save = self.pos;
        if let Some(Token::Int(generation)) = self.next_token()
            && let Ok(generation) = u16::try_from(generation)
            && matches!(self.next_token(), Some(Token::Keyword(b"R")))
            && let Ok(number) = u32::try_from(number)
        {
            return Some(Object::Ref(number, generation));
        }
        self.pos = save;
        None
    }
}

/// The operations of a content stream, in order: each operator with the
/// operands before it. Inline image data (`BI … ID … EI`) is skipped and
/// reported as a bare `BI`.
pub(crate) struct Operations<'a> {
    lexer: Lexer<'a>,
    operands: Vec<Object>,
}

impl<'a> Operations<'a> {
    pub(crate) fn new(content: &'a [u8]) -> Self {
        Self {
            lexer: Lexer::new(content, 0),
            operands: Vec::new(),
        }
    }

    fn skip_inline_image(&mut self) {
        loop {
            match self.lexer.next_token() {
                None => return,
                Some(Token::Keyword(b"ID")) => break,
                Some(_) => {}
            }
        }
        let data = self.lexer.data;
        let mut index = self.lexer.pos + 1;
        while index + 1 < data.len() {
            if data.get(index..index + 2) == Some(b"EI")
                && data.get(index - 1).is_some_and(|byte| is_whitespace(*byte))
                && data
                    .get(index + 2)
                    .is_none_or(|byte| is_whitespace(*byte) || is_delimiter(*byte))
            {
                self.lexer.pos = index + 2;
                return;
            }
            index += 1;
        }
        self.lexer.pos = data.len();
    }
}

impl<'a> Iterator for Operations<'a> {
    type Item = (&'a [u8], Vec<Object>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let token = self.lexer.next_token()?;
            let operand = match token {
                Token::Keyword(b"true") => Some(Object::Bool(true)),
                Token::Keyword(b"false") => Some(Object::Bool(false)),
                Token::Keyword(b"null") => Some(Object::Null),
                Token::Keyword(b"BI") => {
                    self.skip_inline_image();
                    self.operands.clear();
                    return Some((b"BI", Vec::new()));
                }
                Token::Keyword(operator) => {
                    return Some((operator, std::mem::take(&mut self.operands)));
                }
                token => self.lexer.object_from(token, false),
            };
            if let Some(operand) = operand
                && self.operands.len() < MAX_OPERANDS
            {
                self.operands.push(operand);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Option<Object> {
        Lexer::new(text.as_bytes(), 0).object(true)
    }

    #[test]
    fn strings_names_numbers_and_references() {
        assert_eq!(
            parse(
                r"(a\(b\) \101\n\
c)"
            ),
            Some(Object::String(b"a(b) A\nc".to_vec()))
        );
        assert_eq!(
            parse("<48 65 6c6C 6>"),
            Some(Object::String(b"Hell`".to_vec()))
        );
        assert_eq!(parse("/A#20B"), Some(Object::Name(b"A B".to_vec())));
        assert_eq!(parse("-.5"), Some(Object::Real(-0.5)));
        assert_eq!(parse("--3"), Some(Object::Int(3)));
        assert_eq!(parse("12 0 R"), Some(Object::Ref(12, 0)));
        assert_eq!(parse("12 0 obj"), Some(Object::Int(12)));
        let Some(Object::Dict(dict)) = parse("<< /Type /Page /Kids [1 0 R 2 0 R] /N null >>")
        else {
            panic!("not a dictionary");
        };
        assert_eq!(dict.name(b"Type"), Some(&b"Page"[..]));
        assert_eq!(
            dict.get(b"Kids"),
            Some(&Object::Array(vec![Object::Ref(1, 0), Object::Ref(2, 0)]))
        );
        assert_eq!(dict.get(b"N"), Some(&Object::Null));
    }

    #[test]
    fn nesting_and_unterminated_objects_are_refused() {
        let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
        assert_eq!(parse(&deep), None);
        assert_eq!(parse("<< /A [1 2 endobj"), None);
        assert!(parse(&format!("{}1{}", "[".repeat(10), "]".repeat(10))).is_some());
    }

    #[test]
    fn operations_skip_inline_images() {
        let content = b"BT /F1 12 Tf (Hi) Tj ET BI /W 2 /H 1 /BPC 8 /CS /G ID \x00EI\xff EI Q";
        let operators: Vec<Vec<u8>> = Operations::new(content)
            .map(|(operator, _)| operator.to_vec())
            .collect();
        assert_eq!(
            operators,
            [&b"BT"[..], b"Tf", b"Tj", b"ET", b"BI", b"Q"]
                .iter()
                .map(|operator| operator.to_vec())
                .collect::<Vec<_>>()
        );
    }
}
