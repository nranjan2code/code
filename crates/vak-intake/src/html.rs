//! HTML to readable text: what a person reading the page would read, as
//! lines a model can quote. The page's `<main>` (or its one `<article>`, or
//! its `<body>`) is read; scripts, styles, navigation, forms and hidden
//! elements are left out and counted, never silently. Headings keep their
//! level, list items their bullet, table rows their cells, and links are
//! gathered on the side, so the text stays the text.
//!
//! Hostile input: linear in its length, no recursion, no allocation beyond
//! the output and the token list. Every vak call site runs it in the broker
//! worker (invariant 14).

use crate::text::decode_references;

/// What a page reads as.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Readable {
    /// The document's `<title>`, when it has one.
    pub title: Option<String>,
    /// Which part of the page was read: `main`, `article`, `body` or `document`.
    pub region: String,
    /// The readable text, one block per line, blank lines between blocks.
    pub text: String,
    /// Each link in the region read, in order, once: its text and its URL
    /// resolved against the page's address.
    pub links: Vec<Link>,
    /// How many elements of each left-out kind there were.
    pub left_out: LeftOut,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Link {
    pub text: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LeftOut {
    /// `script`, `style`, `noscript`, `template`, `svg`, `iframe`, `object`, `canvas`.
    pub code: usize,
    /// `nav`, `footer`, and links to the page in another language
    /// (`<a hreflang>`).
    pub navigation: usize,
    /// `form`, `button`, `select`, `textarea`, `dialog`.
    pub forms: usize,
    /// Elements marked `hidden`, `aria-hidden="true"` or `display:none`.
    pub hidden: usize,
}

#[derive(Debug)]
enum Token<'a> {
    Text(&'a str),
    Open {
        name: String,
        attrs: &'a str,
        closed: bool,
    },
    Close(String),
}

const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title", "xmp", "noscript"];
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
const CODE: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "iframe", "object", "canvas", "math",
];
const NAVIGATION: &[&str] = &["nav", "footer"];
const FORMS: &[&str] = &["form", "button", "select", "textarea", "dialog"];
const BLOCK: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "caption",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "header",
    "hgroup",
    "main",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "ul",
];

/// Reads `html` as text. `base` is the page's own address, which relative
/// links resolve against; without one they are kept as written.
pub fn readable(html: &str, base: Option<&url::Url>) -> Readable {
    let tokens = tokenize(html);
    let title = tokens
        .iter()
        .enumerate()
        .find_map(|(at, token)| match token {
            Token::Open { name, .. } if name == "title" => match tokens.get(at + 1) {
                Some(Token::Text(text)) => {
                    Some(collapse(&decode_references(text))).filter(|t| !t.is_empty())
                }
                _ => None,
            },
            _ => None,
        });
    let ends = matching_ends(&tokens);
    let (region, range) = region(&tokens, &ends);
    let mut out = Writer::default();
    let mut left_out = LeftOut::default();
    let mut links: Vec<Link> = Vec::new();
    let mut link: Option<(Option<String>, String)> = None;
    let mut lists: Vec<Option<usize>> = Vec::new();
    let mut first_cell = true;
    let mut pre = 0usize;
    let mut at = range.start;
    while at < range.end {
        match &tokens[at] {
            Token::Text(text) => {
                let text = decode_references(text);
                if pre > 0 {
                    out.raw(&text);
                } else {
                    out.text(&text);
                }
                if let Some((_, label)) = link.as_mut() {
                    label.push_str(&text);
                }
            }
            Token::Open {
                name,
                attrs,
                closed,
            } => {
                let name = name.as_str();
                let kind = if CODE.contains(&name) {
                    Some(&mut left_out.code)
                } else if NAVIGATION.contains(&name)
                    || (name == "a" && attr(attrs, "hreflang").is_some())
                {
                    Some(&mut left_out.navigation)
                } else if FORMS.contains(&name) {
                    Some(&mut left_out.forms)
                } else if is_hidden(attrs) {
                    Some(&mut left_out.hidden)
                } else {
                    None
                };
                if let Some(count) = kind {
                    // Skipped whole when its end is in the region; a stray
                    // opening tag with no end leaves out only itself.
                    let end = if *closed || VOID.contains(&name) {
                        Some(at)
                    } else {
                        ends[at].filter(|end| *end < range.end)
                    };
                    if let Some(end) = end {
                        *count += 1;
                        at = end + 1;
                        continue;
                    }
                }
                match name {
                    "br" => out.line(),
                    "hr" => out.block(),
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        out.block();
                        let level = usize::from(name.as_bytes()[1] - b'0');
                        out.mark(&format!("{} ", "#".repeat(level)));
                    }
                    "ul" => {
                        out.block();
                        lists.push(None);
                    }
                    "ol" => {
                        out.block();
                        lists.push(Some(0));
                    }
                    "li" => {
                        out.line();
                        let depth = lists.len().saturating_sub(1);
                        let bullet = match lists.last_mut() {
                            Some(Some(n)) => {
                                *n += 1;
                                format!("{n}. ")
                            }
                            _ => "- ".into(),
                        };
                        out.mark(&format!("{}{bullet}", "  ".repeat(depth)));
                    }
                    "tr" => {
                        out.line();
                        first_cell = true;
                    }
                    "td" | "th" => {
                        if !first_cell {
                            out.mark(" | ");
                        }
                        first_cell = false;
                    }
                    "pre" => {
                        out.block();
                        pre += 1;
                    }
                    "blockquote" => {
                        out.block();
                        out.mark("> ");
                    }
                    "a" => {
                        link = Some((attr(attrs, "href"), String::new()));
                    }
                    "img" => {
                        if let Some(alt) = attr(attrs, "alt").filter(|a| !a.trim().is_empty()) {
                            out.text(&format!(" [image: {}] ", alt.trim()));
                        }
                    }
                    _ if BLOCK.contains(&name) => out.block(),
                    _ => {}
                }
            }
            Token::Close(name) => match name.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "pre" | "blockquote" | "table" => {
                    if name == "pre" {
                        pre = pre.saturating_sub(1);
                    }
                    out.block();
                }
                "ul" | "ol" => {
                    lists.pop();
                    out.block();
                }
                "li" | "tr" | "dt" | "dd" => out.line(),
                "a" => {
                    if let Some((Some(href), label)) = link.take()
                        && let Some(url) = resolve(&href, base)
                    {
                        let text = collapse(&label);
                        if !links.iter().any(|l| l.url == url) {
                            links.push(Link { text, url });
                        }
                    }
                }
                other if BLOCK.contains(&other) => out.block(),
                _ => {}
            },
        }
        at += 1;
    }
    Readable {
        title,
        region: region.into(),
        text: out.finish(),
        links,
        left_out,
    }
}

/// The part of the page to read: its `<main>`, else its one `<article>`,
/// else its `<body>`, else all of it.
fn region(tokens: &[Token<'_>], ends: &[Option<usize>]) -> (&'static str, std::ops::Range<usize>) {
    let opens = |tag: &str| {
        tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| matches!(t, Token::Open { name, .. } if name == tag))
            .map(|(at, _)| at)
            .collect::<Vec<_>>()
    };
    for (tag, only_one) in [("main", false), ("article", true), ("body", false)] {
        let found = opens(tag);
        if found.is_empty() || (only_one && found.len() > 1) {
            continue;
        }
        let start = found[0];
        let end = ends[start].unwrap_or(tokens.len());
        return (tag, start + 1..end);
    }
    ("document", 0..tokens.len())
}

/// For each token, the index of the end tag that closes the element it
/// opens, when one does. One pass with a stack: an end tag closes the
/// nearest open element of its name within [`MAX_DEPTH_SEARCH`] and every
/// element left open inside it; a stray end tag closes nothing.
fn matching_ends(tokens: &[Token<'_>]) -> Vec<Option<usize>> {
    let mut ends = vec![None; tokens.len()];
    let mut open: Vec<(usize, &str)> = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        match token {
            Token::Open { name, closed, .. } if !closed && !VOID.contains(&name.as_str()) => {
                open.push((at, name.as_str()));
            }
            Token::Close(name) => {
                let found = open
                    .iter()
                    .rev()
                    .take(MAX_DEPTH_SEARCH)
                    .position(|(_, n)| n == name);
                if let Some(back) = found {
                    let keep = open.len() - back - 1;
                    ends[open[keep].0] = Some(at);
                    open.truncate(keep);
                }
            }
            _ => {}
        }
    }
    ends
}

/// How far up the open elements an end tag looks for the one it closes.
const MAX_DEPTH_SEARCH: usize = 256;

fn tokenize(html: &str) -> Vec<Token<'_>> {
    let bytes = html.as_bytes();
    let mut tokens = Vec::new();
    let mut text_start = 0usize;
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes[at] != b'<' {
            at += 1;
            continue;
        }
        let next = bytes.get(at + 1).copied().unwrap_or(0);
        let markup_end = if html[at..].starts_with("<!--") {
            Some(find(bytes, at + 4, b"-->").map_or(bytes.len(), |end| end + 3))
        } else if next == b'!' || next == b'?' {
            Some(find(bytes, at, b">").map_or(bytes.len(), |end| end + 1))
        } else {
            None
        };
        if let Some(end) = markup_end {
            push_text(&mut tokens, &html[text_start..at]);
            at = end;
            text_start = end;
            continue;
        }
        let closing = next == b'/';
        let name_start = at + 1 + usize::from(closing);
        if !bytes.get(name_start).is_some_and(u8::is_ascii_alphabetic) {
            at += 1;
            continue;
        }
        let name_end = bytes[name_start..]
            .iter()
            .position(|b| !(b.is_ascii_alphanumeric() || *b == b'-' || *b == b':'))
            .map_or(bytes.len(), |n| name_start + n);
        let Some(tag_end) = tag_end(bytes, name_end) else {
            break;
        };
        push_text(&mut tokens, &html[text_start..at]);
        let name = html[name_start..name_end].to_ascii_lowercase();
        let inner = &html[name_end..tag_end];
        at = tag_end + 1;
        text_start = at;
        if closing {
            tokens.push(Token::Close(name));
            continue;
        }
        let closed = inner.trim_end().ends_with('/');
        let raw = RAW_TEXT.contains(&name.as_str()) && !closed;
        tokens.push(Token::Open {
            name: name.clone(),
            attrs: inner,
            closed,
        });
        if raw {
            let close = format!("</{name}");
            let end = find_ci(bytes, at, close.as_bytes()).unwrap_or(bytes.len());
            push_text(&mut tokens, &html[at..end]);
            let after = find(bytes, end, b">").map_or(bytes.len(), |gt| gt + 1);
            tokens.push(Token::Close(name));
            at = after;
            text_start = after;
        }
    }
    push_text(&mut tokens, &html[text_start..]);
    tokens
}

fn push_text<'a>(tokens: &mut Vec<Token<'a>>, text: &'a str) {
    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
}

/// The `>` that ends a tag, outside quoted attribute values.
fn tag_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (at, b) in bytes.iter().enumerate().skip(from) {
        match (quote, *b) {
            (Some(q), b) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(*b),
            (None, b'>') => return Some(at),
            _ => {}
        }
    }
    None
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|n| from + n)
}

fn find_ci(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
        .map(|n| from + n)
}

/// One attribute's value, decoded.
fn attr(attrs: &str, wanted: &str) -> Option<String> {
    let bytes = attrs.as_bytes();
    let mut at = 0usize;
    while at < bytes.len() {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
            at += 1;
        }
        let name_start = at;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && bytes[at] != b'='
            && bytes[at] != b'/'
        {
            at += 1;
        }
        let name = &attrs[name_start..at];
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let mut value = None;
        if bytes.get(at) == Some(&b'=') {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            match bytes.get(at) {
                Some(q @ (b'"' | b'\'')) => {
                    let start = at + 1;
                    let end = bytes[start..]
                        .iter()
                        .position(|b| b == q)
                        .map_or(bytes.len(), |n| start + n);
                    value = Some(&attrs[start..end]);
                    at = end + 1;
                }
                _ => {
                    let start = at;
                    while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
                        at += 1;
                    }
                    value = Some(&attrs[start..at]);
                }
            }
        }
        if name.eq_ignore_ascii_case(wanted) {
            return Some(decode_references(value.unwrap_or("")));
        }
        if name.is_empty() {
            at += 1;
        }
    }
    None
}

fn is_hidden(attrs: &str) -> bool {
    if attr(attrs, "hidden").is_some() {
        return true;
    }
    if attr(attrs, "aria-hidden").is_some_and(|v| v.trim().eq_ignore_ascii_case("true")) {
        return true;
    }
    attr(attrs, "style").is_some_and(|style| {
        let style: String = style
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        style.contains("display:none") || style.contains("visibility:hidden")
    })
}

/// An http(s) address for a link, or `None` for an in-page anchor, a script
/// URL, or anything that does not resolve.
fn resolve(href: &str, base: Option<&url::Url>) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    let url = match base {
        Some(base) => base.join(href).ok()?,
        None => url::Url::parse(href).ok()?,
    };
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn collapse(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Builds the text: runs of whitespace become one space, a line break ends
/// a line, a block break leaves one blank line, and neither repeats.
#[derive(Default)]
struct Writer {
    out: String,
    /// What the current line will start with once it has text.
    pending_mark: String,
    /// 0: none, 1: line break, 2: blank line.
    pending_break: u8,
    space: bool,
}

impl Writer {
    fn line(&mut self) {
        self.pending_break = self.pending_break.max(1);
        self.pending_mark.clear();
        self.space = false;
    }

    fn block(&mut self) {
        self.pending_break = 2;
        self.pending_mark.clear();
        self.space = false;
    }

    fn mark(&mut self, mark: &str) {
        self.pending_mark.push_str(mark);
    }

    fn start(&mut self) {
        if !self.out.is_empty() {
            match self.pending_break {
                0 if self.space => self.out.push(' '),
                0 => {}
                1 => self.out.push('\n'),
                _ => self.out.push_str("\n\n"),
            }
        }
        self.pending_break = 0;
        self.space = false;
        let mark = std::mem::take(&mut self.pending_mark);
        self.out.push_str(&mark);
    }

    fn text(&mut self, text: &str) {
        if text.starts_with(char::is_whitespace) {
            self.space = true;
        }
        let mut words = text.split_whitespace().peekable();
        while let Some(word) = words.next() {
            self.start();
            self.out.push_str(word);
            self.space = words.peek().is_some();
        }
        if text.ends_with(char::is_whitespace) && !self.out.is_empty() {
            self.space = true;
        }
    }

    fn raw(&mut self, text: &str) {
        let text = text.trim_matches('\n');
        if text.is_empty() {
            return;
        }
        self.start();
        self.out.push_str(text);
    }

    fn finish(self) -> String {
        self.out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const PAGE: &str = r##"<!DOCTYPE html><html><head><title>Lighthouse &ndash; Wiki</title>
<style>.a{color:red}</style><script>var x = "<main>";</script></head>
<body><nav><a href="/home">Home</a></nav>
<main id="content"><ul><li><a href="https://fr.example.org/wiki/Phare" hreflang="fr">Français</a></li></ul><h1>Lighthouse</h1>
<p>A <b>lighthouse</b> is a tower.<span class="mw-editsection" aria-hidden="true">[edit]</span>
See <a href="/wiki/Beacon">beacons</a> and <a href="#notes">notes</a>.</p>
<h2>Famous ones</h2>
<ul><li>Tower of Hercules</li><li>Bell Rock<ul><li>Scotland</li></ul></li></ul>
<ol><li>first</li><li>second</li></ol>
<table><tr><th>Name</th><th>Country</th></tr><tr><td>Fastnet</td><td>Ireland</td></tr></table>
<pre>line one
  line two</pre>
<form><input name="q"><button>Search</button></form>
<div style="display: none">secret</div>
<img src="x.png" alt="A lighthouse at dusk">
</main><footer>Footer text</footer></body></html>"##;

    #[test]
    fn a_page_reads_as_its_main_text() {
        let base = url::Url::parse("https://en.example.org/wiki/Lighthouse").unwrap();
        let page = readable(PAGE, Some(&base));
        assert_eq!(page.title.as_deref(), Some("Lighthouse – Wiki"));
        assert_eq!(page.region, "main");
        assert_eq!(
            page.text,
            "# Lighthouse\n\n\
             A lighthouse is a tower. See beacons and notes.\n\n\
             ## Famous ones\n\n\
             - Tower of Hercules\n\
             - Bell Rock\n\n  \
             - Scotland\n\n\
             1. first\n\
             2. second\n\n\
             Name | Country\n\
             Fastnet | Ireland\n\n\
             line one\n  line two\n\n\
             [image: A lighthouse at dusk]"
        );
        assert_eq!(
            page.links,
            vec![Link {
                text: "beacons".into(),
                url: "https://en.example.org/wiki/Beacon".into()
            }]
        );
        assert_eq!(
            page.left_out,
            LeftOut {
                code: 0,
                navigation: 1,
                forms: 1,
                hidden: 2,
            }
        );
    }

    #[test]
    fn without_main_the_body_is_read_and_its_chrome_counted() {
        let page = readable(
            "<body><nav>menu</nav><p>Fish &amp; chips</p><script>alert(1)</script><footer>f</footer></body>",
            None,
        );
        assert_eq!(page.region, "body");
        assert_eq!(page.text, "Fish & chips");
        assert_eq!(page.left_out.navigation, 2);
        assert_eq!(page.left_out.code, 1);
    }

    #[test]
    fn stray_and_unclosed_markup_is_text_or_ignored() {
        assert_eq!(readable("a < b and c > d", None).text, "a < b and c > d");
        assert_eq!(readable("AT&T &bogus; x", None).text, "AT&T &bogus; x");
        assert_eq!(
            readable("<p>open <nav>never closed", None).text,
            "open never closed"
        );
        assert_eq!(readable("<p title='a>b'>quoted</p>", None).text, "quoted");
        assert_eq!(
            readable("<div>x<!-- <p>gone</p> -->y</div>", None).text,
            "xy"
        );
        assert_eq!(readable("unterminated <b", None).text, "unterminated <b");
    }
}
