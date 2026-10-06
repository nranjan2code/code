//! Markup to plain text: tags dropped (with what `script` and `style`
//! hold), character references decoded, whitespace collapsed.

pub(crate) fn strip_markup(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let opens_tag = after[1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'));
        if !opens_tag {
            out.push('<');
            rest = &after[1..];
            continue;
        }
        let Some(end) = after.find('>') else {
            // A lone `<` is text.
            out.push_str(after);
            rest = "";
            break;
        };
        let tag = after[1..end].trim_start_matches('/').to_ascii_lowercase();
        let name: String = tag
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        rest = &after[end + 1..];
        if !after[1..].starts_with('/') && (name == "script" || name == "style") {
            let close = format!("</{name}");
            rest = match rest.to_ascii_lowercase().find(&close) {
                Some(at) => rest[at..].find('>').map_or("", |gt| &rest[at + gt + 1..]),
                None => "",
            };
        }
        if matches!(
            name.as_str(),
            "p" | "br" | "div" | "li" | "tr" | "h1" | "h2" | "h3"
        ) {
            out.push(' ');
        }
    }
    out.push_str(rest);
    collapse(&decode_references(&out))
}

fn decode_references(raw: &str) -> String {
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
