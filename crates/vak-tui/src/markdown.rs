use crossterm::style::Color;

use crate::theme::{Theme, fg};

const RESET_FG: &str = "\x1b[39m";
const BOLD_ON: &str = "\x1b[1m";
const BOLD_OFF: &str = "\x1b[22m";
const ITALIC_ON: &str = "\x1b[3m";
const ITALIC_OFF: &str = "\x1b[23m";

/// Shared keyword list for the hand-rolled highlighter (rust, python,
/// js/ts, go, c, sh, json, toml and neighbors). Must stay sorted; lookups
/// are binary searches over lowercase words.
const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "assert",
    "async",
    "auto",
    "await",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "chan",
    "char",
    "class",
    "clone",
    "companion",
    "const",
    "constructor",
    "continue",
    "crate",
    "data",
    "def",
    "default",
    "defer",
    "del",
    "delete",
    "do",
    "done",
    "double",
    "dyn",
    "echo",
    "elif",
    "else",
    "end",
    "enum",
    "except",
    "export",
    "extends",
    "extern",
    "false",
    "fi",
    "final",
    "finally",
    "float",
    "fn",
    "for",
    "from",
    "fun",
    "func",
    "function",
    "global",
    "go",
    "goto",
    "if",
    "impl",
    "implements",
    "import",
    "in",
    "include",
    "instanceof",
    "int",
    "interface",
    "internal",
    "is",
    "lambda",
    "let",
    "local",
    "long",
    "loop",
    "map",
    "match",
    "mod",
    "move",
    "mut",
    "namespace",
    "new",
    "nil",
    "none",
    "not",
    "null",
    "object",
    "operator",
    "or",
    "override",
    "package",
    "pass",
    "private",
    "protected",
    "pub",
    "public",
    "raise",
    "range",
    "ref",
    "register",
    "require",
    "return",
    "select",
    "self",
    "set",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "super",
    "switch",
    "template",
    "then",
    "this",
    "throw",
    "trait",
    "transient",
    "true",
    "try",
    "type",
    "typedef",
    "typename",
    "typeof",
    "union",
    "unsafe",
    "unsigned",
    "use",
    "using",
    "val",
    "var",
    "virtual",
    "void",
    "volatile",
    "when",
    "where",
    "while",
    "with",
    "yield",
];

#[derive(Debug, Default)]
pub struct LineStyler {
    in_fence: bool,
    lang: String,
}

impl LineStyler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Style one completed source line (no trailing newline), in stream
    /// order. Tracks ``` fences across calls: inside a fence, lines get
    /// syntax highlighting; fence markers themselves render dim.
    pub fn line(&mut self, raw: &str, theme: &Theme) -> String {
        let stripped = raw.trim_start();
        if let Some(info) = stripped.strip_prefix("```") {
            if self.in_fence {
                self.in_fence = false;
                self.lang.clear();
            } else {
                self.in_fence = true;
                self.lang = fence_lang(info);
            }
            return span(raw, theme.dim);
        }
        if self.in_fence {
            return highlight(raw, &self.lang, theme);
        }
        if is_horizontal_rule(stripped) {
            return span("───", theme.dim);
        }
        if let Some(text) = heading_text(stripped) {
            return format!("{BOLD_ON}{}{text}{RESET_FG}{BOLD_OFF}", fg(theme.heading));
        }
        let (indent, body) = split_ascii_indent(raw);
        if let Some(content) = body.strip_prefix("> ") {
            return format!(
                "{indent}{ITALIC_ON}{}{}{RESET_FG}{ITALIC_OFF}",
                fg(theme.dim),
                content.trim_start()
            );
        }
        if let Some(rest) = bullet_body(body) {
            return format!(
                "{indent}{}•{RESET_FG} {}",
                fg(theme.dim),
                inline(rest, theme)
            );
        }
        if body.starts_with('|') && body.trim_end().ends_with('|') {
            return table_line(body, theme);
        }
        if let Some(marker) = numbered_marker(body) {
            return format!(
                "{indent}{}{marker}{RESET_FG}{}",
                fg(theme.accent),
                inline(&body[marker.len()..], theme)
            );
        }
        inline(raw, theme)
    }
}


/// Pipe-table row: dim the delimiters, keep cell text readable. The
/// `|---|:--:|` separator renders as a thin rule so tables read as
/// tables without cross-line layout state.
fn table_line(body: &str, theme: &Theme) -> String {
    let trimmed = body.trim();
    let cells: Vec<&str> = trimmed[1..trimmed.len() - 1].split('|').collect();
    let is_separator = cells.iter().all(|c| {
        let t = c.trim().trim_start_matches(':').trim_end_matches(':');
        !t.is_empty() && t.chars().all(|ch| ch == '-')
    });
    if is_separator {
        let rule: String = std::iter::repeat("─".repeat(4)).take(cells.len()).collect::<Vec<_>>().join("┼");
        return span(&format!("  ┌{rule}┐"), theme.dim);
    }
    let mut out = String::from("  ");
    out.push_str(&fg(theme.dim));
    out.push('│');
    out.push_str(&RESET_FG.to_string());
    for c in cells {
        out.push(' ');
        out.push_str(&inline(c.trim(), theme));
        out.push(' ');
        out.push_str(&fg(theme.dim));
        out.push('│');
        out.push_str(&RESET_FG.to_string());
    }
    out
}

fn span(text: &str, color: Color) -> String {
    format!("{}{text}{RESET_FG}", fg(color))
}

fn fence_lang(info: &str) -> String {
    info.trim_start_matches('`')
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ',')
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '_'))
        .collect::<String>()
        .to_lowercase()
}

fn split_ascii_indent(raw: &str) -> (&str, &str) {
    let n = raw
        .as_bytes()
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    raw.split_at(n)
}

fn is_horizontal_rule(line: &str) -> bool {
    line.len() >= 3 && line.chars().all(|c| matches!(c, '-' | '*' | '_'))
}

fn heading_text(stripped: &str) -> Option<&str> {
    let hashes = stripped.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &stripped[hashes..];
    if rest.starts_with(' ') || rest.starts_with('\t') || rest.is_empty() {
        Some(rest.trim())
    } else {
        None
    }
}

fn bullet_body(body: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|g| body.strip_prefix(g))
        .filter(|rest| !rest.trim().is_empty())
}

fn numbered_marker(body: &str) -> Option<&str> {
    let digits_end = body.find(|c: char| !c.is_ascii_digit())?;
    if digits_end == 0 || digits_end > 9 {
        return None;
    }
    let rest = &body[digits_end..];
    if rest.starts_with(". ") || rest.starts_with(") ") {
        Some(&body[..digits_end + 2])
    } else {
        None
    }
}

fn inline(text: &str, theme: &Theme) -> String {
    if !text.contains(['`', '*']) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => match text[i + 1..].find('`') {
                Some(rel) if rel > 0 => {
                    out.push_str(&span(&text[i + 1..i + 1 + rel], theme.code));
                    i += rel + 2;
                }
                _ => {
                    out.push('`');
                    i += 1;
                }
            },
            b'*' if bytes.get(i + 1) == Some(&b'*') => match text[i + 2..].find("**") {
                Some(rel) if rel > 0 => {
                    out.push_str(BOLD_ON);
                    out.push_str(&text[i + 2..i + 2 + rel]);
                    out.push_str(BOLD_OFF);
                    i += rel + 4;
                }
                _ => {
                    out.push('*');
                    i += 1;
                }
            },
            b'*' => match text[i + 1..].find('*') {
                Some(rel) if rel > 0 => {
                    let inner = &text[i + 1..i + 1 + rel];
                    if inner.starts_with(' ') || inner.ends_with(' ') {
                        out.push('*');
                        i += 1;
                    } else {
                        out.push_str(ITALIC_ON);
                        out.push_str(inner);
                        out.push_str(ITALIC_OFF);
                        i += rel + 2;
                    }
                }
                _ => {
                    out.push('*');
                    i += 1;
                }
            },
            _ => match text[i..].chars().next() {
                Some(c) => {
                    out.push(c);
                    i += c.len_utf8();
                }
                None => break,
            },
        }
    }
    out
}

#[derive(Clone, Copy)]
enum CommentKind {
    Hash,
    DashDash,
    SlashSlash,
    None,
}

fn comment_kind(lang: &str) -> CommentKind {
    match lang {
        "" | "text" | "txt" | "plain" | "md" | "markdown" | "diff" => CommentKind::None,
        "py" | "python" | "sh" | "bash" | "zsh" | "shell" | "console" | "toml" | "yaml" | "yml"
        | "ruby" | "rb" | "r" | "perl" | "conf" | "ini" => CommentKind::Hash,
        "sql" | "lua" | "haskell" | "hs" => CommentKind::DashDash,
        _ => CommentKind::SlashSlash,
    }
}

fn comment_at(bytes: &[u8], i: usize, kind: CommentKind) -> bool {
    match kind {
        CommentKind::Hash => bytes[i] == b'#',
        CommentKind::DashDash => bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'-'),
        CommentKind::SlashSlash => bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/'),
        CommentKind::None => false,
    }
}

/// Syntax-highlight one completed source line. Hand-rolled tokenizer adequate
/// for common langs (rust, python, js/ts, go, c, sh, json, toml): line
/// comments (`//`, `#`, `--`), strings with basic backslash awareness,
/// numbers, and one shared keyword list matched case-insensitively on word
/// boundaries. Keywords accent, strings green, comments dim, numbers yellow;
/// all other bytes pass through untouched.
pub fn highlight(code_line: &str, lang: &str, theme: &Theme) -> String {
    let comment = comment_kind(lang);
    let mut out = String::with_capacity(code_line.len() + 32);
    let bytes = code_line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if comment_at(bytes, i, comment) {
            out.push_str(&span(&code_line[i..], theme.dim));
            break;
        }
        match bytes[i] {
            q @ (b'"' | b'\'') => {
                let mut j = i + 1;
                while j < bytes.len() {
                    if bytes[j] == b'\\' && j + 1 < bytes.len() {
                        j += 2;
                        continue;
                    }
                    if bytes[j] == q {
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                let end = j.min(bytes.len());
                out.push_str(&span(&code_line[i..end], theme.success));
                i = end;
            }
            b'0'..=b'9' => {
                let mut j = i + 1;
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'.' | b'_'))
                {
                    j += 1;
                }
                out.push_str(&span(&code_line[i..j], theme.warning));
                i = j;
            }
            b if b.is_ascii_alphanumeric() || b == b'_' => {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                let word = &code_line[i..j];
                if KEYWORDS
                    .binary_search(&word.to_lowercase().as_str())
                    .is_ok()
                {
                    out.push_str(&span(word, theme.accent));
                } else {
                    out.push_str(word);
                }
                i = j;
            }
            _ => match code_line[i..].chars().next() {
                Some(c) => {
                    out.push(c);
                    i += c.len_utf8();
                }
                None => break,
            },
        }
    }
    out
}

/// Removes SGR (and other CSI) escape sequences — for plain-text consumers
/// like `exec` logs where ANSI would be noise.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod markdown_tests {
    use super::*;
    use crate::theme;

    #[test]
    fn table_rows_dim_pipes_and_render_cells() {
        let mut ls = LineStyler::new();
        let th = theme::resolve("dark", &Default::default());
        let out = ls.line("| name | qty |", &th);
        assert!(out.contains("name") && out.contains("qty"));
        assert!(out.contains('\u{2502}'), "pipe delimiter present");
        assert_eq!(out.matches('\u{2502}').count(), 3);
    }

    #[test]
    fn table_separator_renders_as_rule() {
        let mut ls = LineStyler::new();
        let th = theme::resolve("dark", &Default::default());
        let out = ls.line("| --- | :---: |", &th);
        assert!(out.contains('\u{2500}'));
        assert!(!out.contains("---"));
    }

    #[test]
    fn non_table_pipe_line_untouched_by_table_path() {
        let mut ls = LineStyler::new();
        let th = theme::resolve("dark", &Default::default());
        // No trailing pipe → not a table row.
        let out = ls.line("a | b", &th);
        assert!(out.contains("a | b"));
    }
}
