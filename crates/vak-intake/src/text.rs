//! Markup to plain text on one line: what `html::readable` reads, with
//! whitespace collapsed; character references; bounding.

pub(crate) fn strip_markup(raw: &str) -> String {
    collapse(&crate::html::readable(raw, None).text)
}

/// Character references decoded; one that is unknown or malformed is kept
/// as written.
pub(crate) fn decode_references(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find(';').filter(|end| *end <= 10);
        let decoded = end.and_then(|end| reference(&after[..end]));
        match (end, decoded) {
            (Some(end), Some(c)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn reference(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "middot" => '·',
        "bull" => '•',
        "deg" => '°',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "times" => '×',
        "minus" => '−',
        "euro" => '€',
        "pound" => '£',
        "thinsp" => '\u{2009}',
        "shy" => '\u{00AD}',
        _ => return None,
    })
}

fn collapse(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `max` characters, cut at a character boundary.
pub(crate) fn bounded(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::strip_markup;

    #[test]
    fn markup_becomes_plain_text() {
        assert_eq!(
            strip_markup("<p>Fish &amp; chips</p><script>alert(1)</script><b>now</b>&nbsp;&#x41;"),
            "Fish & chips now A"
        );
        assert_eq!(strip_markup("a < b and c > d"), "a < b and c > d");
        assert_eq!(strip_markup("AT&T &bogus; x"), "AT&T &bogus; x");
    }
}
