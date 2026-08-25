//! Telegram-safe HTML projection and standalone chunking.

/// Convert a conservative GFM subset to Telegram-safe HTML.
pub fn markdown_to_html(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len() + 64);
    let mut in_fence = false;
    let mut fence = Vec::new();
    let mut table = Vec::new();

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            flush_table(&mut out, &mut table);
            if in_fence {
                out.push_str("<pre>");
                out.push_str(&escape_html(&fence.join("\n")));
                out.push_str("</pre>");
                fence.clear();
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            fence.push(line.to_string());
            continue;
        }
        if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
            table.push(trimmed.to_string());
            continue;
        }
        flush_table(&mut out, &mut table);

        if let Some(after) = trimmed.strip_prefix('#')
            && after.starts_with(' ')
        {
            let text = after.trim().trim_start_matches('#').trim();
            if !text.is_empty() {
                out.push_str("<b>");
                out.push_str(&inline_markdown(&escape_html(text)));
                out.push_str("</b>\n\n");
                continue;
            }
        }
        if let Some(quote) = trimmed.strip_prefix("> ") {
            out.push_str("<blockquote>");
            out.push_str(&inline_markdown(&escape_html(quote)));
            out.push_str("</blockquote>\n");
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            out.push_str("• ");
            out.push_str(&inline_markdown(&escape_html(item)));
            out.push('\n');
            continue;
        }
        out.push_str(&inline_markdown(&escape_html(line)));
        out.push('\n');
    }
    if in_fence {
        out.push_str("<pre>");
        out.push_str(&escape_html(&fence.join("\n")));
        out.push_str("</pre>");
    }
    flush_table(&mut out, &mut table);
    out.trim_end_matches('\n').to_string()
}

fn flush_table(out: &mut String, rows: &mut Vec<String>) {
    if rows.is_empty() {
        return;
    }
    out.push_str("<pre>");
    for row in rows.iter() {
        if !row.replace(['|', '-', ' ', ':'], "").is_empty() {
            out.push_str(&escape_html(row.trim()));
            out.push('\n');
        }
    }
    out.push_str("</pre>");
    rows.clear();
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn inline_markdown(value: &str) -> String {
    let mut codes = Vec::new();
    let mut value = replace_code_spans(value, &mut codes);
    value = replace_links(&value);
    value = replace_emphasis(&value, "**", "<b>", "</b>");
    value = replace_emphasis(&value, "__", "<b>", "</b>");
    value = replace_emphasis(&value, "*", "<i>", "</i>");
    value = replace_emphasis(&value, "_", "<i>", "</i>");
    for (index, code) in codes.iter().enumerate() {
        value = value.replace(
            &format!("\u{0}{index}\u{0}"),
            &format!("<code>{}</code>", escape_html(code)),
        );
    }
    value
}

fn replace_code_spans(value: &str, sink: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some((before, after)) = rest.split_once('`') {
        match after.split_once('`') {
            Some((content, tail)) => {
                sink.push(content.to_string());
                out.push_str(before);
                out.push_str(&format!("\u{0}{}\u{0}", sink.len() - 1));
                rest = tail;
            }
            None => {
                out.push_str(before);
                out.push('`');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn replace_links(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(open) = rest.find('[') {
        let Some(relative_text_end) = rest[open..].find("](") else {
            break;
        };
        let text_end = open + relative_text_end;
        let Some(relative_url_end) = rest[text_end..].find(')') else {
            break;
        };
        let url_end = text_end + relative_url_end;
        let url = &rest[text_end + 2..url_end];
        if url.contains(['"', '<', '>', ' ']) {
            out.push_str(&rest[..open + 1]);
            rest = &rest[open + 1..];
            continue;
        }
        out.push_str(&rest[..open]);
        out.push_str("<a href=\"");
        out.push_str(url);
        out.push_str("\">");
        out.push_str(&rest[open + 1..text_end]);
        out.push_str("</a>");
        rest = &rest[url_end + 1..];
    }
    out.push_str(rest);
    out
}

fn replace_emphasis(value: &str, marker: &str, open: &str, close: &str) -> String {
    if marker.len() == 1 {
        replace_spaced_pairs(value, marker, open, close)
    } else {
        replace_pairs(value, marker, open, close)
    }
}

fn replace_pairs(value: &str, marker: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    loop {
        match rest.split_once(marker) {
            Some((before, after)) => match after.split_once(marker) {
                Some((inner, tail)) if !inner.is_empty() => {
                    out.push_str(before);
                    out.push_str(open);
                    out.push_str(inner);
                    out.push_str(close);
                    rest = tail;
                }
                _ => {
                    out.push_str(before);
                    out.push_str(marker);
                    rest = after;
                }
            },
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

fn replace_spaced_pairs(value: &str, marker: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    loop {
        match rest.split_once(marker) {
            Some((before, after)) => {
                let opens = after.chars().next().is_some_and(|c| !c.is_whitespace());
                let boundary = before.is_empty()
                    || before.chars().last().is_some_and(|c| !c.is_alphanumeric());
                match after.split_once(marker) {
                    Some((inner, tail)) if opens && boundary && !inner.trim().is_empty() => {
                        out.push_str(before);
                        out.push_str(open);
                        out.push_str(inner.trim_end_matches(marker));
                        out.push_str(close);
                        rest = tail;
                    }
                    _ => {
                        out.push_str(before);
                        out.push_str(marker);
                        rest = after;
                    }
                }
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

#[derive(Clone)]
struct OpenTag {
    name: String,
    opening: String,
}

/// Split HTML into independently parseable chunks. Open tags are closed at
/// every boundary and reopened in the following chunk.
pub fn split_html_chunks(html: &str, max_chars: Option<usize>) -> Vec<String> {
    let Some(cap) = max_chars else {
        return vec![html.to_string()];
    };
    if html.chars().count() <= cap {
        return vec![html.to_string()];
    }

    if cap < 32 {
        return plain_chunks(html, cap);
    }
    let tokens = html_tokens(html);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut open_tags: Vec<OpenTag> = Vec::new();
    for token in tokens {
        if token.starts_with('<') && token.ends_with('>') {
            let mut prospective = open_tags.clone();
            update_tag_stack(&token, &mut prospective);
            let projected =
                current.chars().count() + token.chars().count() + closing_tags_len(&prospective);
            if !current.is_empty() && projected > cap {
                flush_chunk(&mut current, &open_tags, &mut chunks);
                reopen_tags(&mut current, &open_tags);
            }
            update_tag_stack(&token, &mut open_tags);
            current.push_str(&token);
            continue;
        }
        for character in token.chars() {
            if current.chars().count() + 1 + closing_tags_len(&open_tags) > cap {
                flush_chunk(&mut current, &open_tags, &mut chunks);
                reopen_tags(&mut current, &open_tags);
            }
            current.push(character);
        }
    }
    if !current.trim().is_empty() {
        flush_chunk(&mut current, &open_tags, &mut chunks);
    }
    chunks
}

fn html_tokens(html: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        if start > 0 {
            tokens.push(rest[..start].to_string());
        }
        let Some(end) = rest[start..].find('>') else {
            tokens.push(rest[start..].to_string());
            return tokens;
        };
        let end = start + end + 1;
        tokens.push(rest[start..end].to_string());
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        tokens.push(rest.to_string());
    }
    tokens
}

fn reopen_tags(current: &mut String, tags: &[OpenTag]) {
    for tag in tags {
        current.push_str(&tag.opening);
    }
}

fn plain_chunks(html: &str, cap: usize) -> Vec<String> {
    let plain = strip_html(html);
    let chars: Vec<char> = plain.chars().collect();
    chars
        .chunks(cap)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn update_tag_stack(token: &str, stack: &mut Vec<OpenTag>) {
    if !token.starts_with('<') || !token.ends_with('>') {
        return;
    }
    let body = token[1..token.len() - 1].trim();
    if let Some(name) = body.strip_prefix('/') {
        if let Some(index) = stack.iter().rposition(|tag| tag.name == name.trim()) {
            stack.remove(index);
        }
        return;
    }
    let name = body.split_whitespace().next().unwrap_or_default();
    if !name.is_empty() && !body.ends_with('/') {
        stack.push(OpenTag {
            name: name.to_string(),
            opening: token.to_string(),
        });
    }
}

fn closing_tags_len(tags: &[OpenTag]) -> usize {
    tags.iter().map(|tag| tag.name.chars().count() + 3).sum()
}

fn flush_chunk(current: &mut String, open_tags: &[OpenTag], chunks: &mut Vec<String>) {
    for tag in open_tags.iter().rev() {
        current.push_str("</");
        current.push_str(&tag.name);
        current.push('>');
    }
    let chunk = std::mem::take(current).trim().to_string();
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
}

/// Remove HTML tags for a last-resort plain Telegram retry.
pub fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_and_escapes_supported_markdown() {
        let html = markdown_to_html(
            "# Title\n\n**bold** *it* `code` [site](https://x.com/a)\n\n> quoted\n- item",
        );
        assert!(html.contains("<b>Title</b>"));
        assert!(html.contains("<b>bold</b>"));
        assert!(html.contains("<i>it</i>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<blockquote>quoted</blockquote>"));
        assert!(html.contains("• item"));
        assert!(!markdown_to_html("<script>x</script>").contains("<script>"));
    }

    #[test]
    fn chunker_closes_and_reopens_tags() {
        let html = format!("<b>{}</b>", "word ".repeat(200));
        let chunks = split_html_chunks(&html, Some(180));
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.starts_with("<b>") && chunk.ends_with("</b>"))
        );
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 180));
    }
}
