//! GFM-to-Slack `mrkdwn` projection.
//!
//! Slack's `mrkdwn` dialect diverges from GFM in exactly the spots that
//! matter most: bold uses a single `*`, italic uses `_`, links use
//! `<url|text>`, there is no heading syntax and no table syntax at all.
//! This module renders a conservative GFM subset into text that actually
//! looks right when Slack's client parses it.

/// Convert a conservative GFM subset to Slack `mrkdwn`.
pub fn markdown_to_mrkdwn(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len() + 64);
    let mut in_fence = false;
    let mut fence = Vec::new();
    let mut table = Vec::new();
    let mut blank_pending = false;

    for line in markdown.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            flush_table(&mut out, &mut table);
            if in_fence {
                out.push_str("```\n");
                out.push_str(&fence.join("\n"));
                out.push_str("\n```\n");
                fence.clear();
            }
            in_fence = !in_fence;
            blank_pending = false;
            continue;
        }
        if in_fence {
            fence.push(line.to_string());
            continue;
        }

        if trimmed.is_empty() {
            flush_table(&mut out, &mut table);
            blank_pending = !out.is_empty();
            continue;
        }

        if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
            if blank_pending {
                out.push('\n');
            }
            blank_pending = false;
            table.push(trimmed.to_string());
            continue;
        }
        flush_table(&mut out, &mut table);

        if is_horizontal_rule(trimmed) {
            if blank_pending {
                out.push('\n');
            }
            blank_pending = false;
            out.push_str("──────────\n");
            continue;
        }

        if let Some(after) = trimmed.strip_prefix('#')
            && after.starts_with(' ')
        {
            let text = after.trim().trim_start_matches('#').trim();
            if !text.is_empty() {
                if blank_pending && !out.is_empty() {
                    out.push('\n');
                }
                blank_pending = false;
                out.push('*');
                out.push_str(&inline_mrkdwn(text));
                out.push_str("*\n");
                continue;
            }
        }

        if let Some(quote) = trimmed.strip_prefix("> ") {
            blank_pending = false;
            out.push_str("&gt; ");
            out.push_str(&inline_mrkdwn(quote));
            out.push('\n');
            continue;
        }

        let indent = line.len() - trimmed.len();
        let depth = indent / 2;

        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            blank_pending = false;
            out.push_str(&"  ".repeat(depth));
            out.push_str(bullet_for_depth(depth));
            out.push(' ');
            out.push_str(&inline_mrkdwn(item));
            out.push('\n');
            continue;
        }

        if let Some((number, rest)) = split_ordered_item(trimmed) {
            blank_pending = false;
            out.push_str(&"  ".repeat(depth));
            out.push_str(&number);
            out.push_str(". ");
            out.push_str(&inline_mrkdwn(rest));
            out.push('\n');
            continue;
        }

        if blank_pending && !out.is_empty() {
            out.push('\n');
        }
        blank_pending = false;
        out.push_str(&inline_mrkdwn(line));
        out.push('\n');
    }
    if in_fence {
        out.push_str("```\n");
        out.push_str(&fence.join("\n"));
        out.push_str("\n```\n");
    }
    flush_table(&mut out, &mut table);
    out.trim_end_matches('\n').to_string()
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

fn split_ordered_item(trimmed: &str) -> Option<(String, &str)> {
    let digits_end = trimmed.find(|c: char| !c.is_ascii_digit())?;
    if digits_end == 0 {
        return None;
    }
    let (number, rest) = trimmed.split_at(digits_end);
    let rest = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    Some((number.to_string(), rest))
}

fn bullet_for_depth(depth: usize) -> &'static str {
    match depth % 3 {
        0 => "•",
        1 => "◦",
        _ => "▪",
    }
}

fn flush_table(out: &mut String, rows: &mut Vec<String>) {
    if rows.is_empty() {
        return;
    }
    let parsed: Vec<Vec<String>> = rows
        .iter()
        .filter(|row| !is_separator_row(row))
        .map(|row| {
            row.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect();

    if parsed.is_empty() {
        rows.clear();
        return;
    }

    let columns = parsed.iter().map(|row| row.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; columns];
    for row in &parsed {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    out.push_str("```\n");
    for (row_index, row) in parsed.iter().enumerate() {
        for (index, width) in widths.iter().enumerate() {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            out.push_str(cell);
            out.push_str(&" ".repeat(width.saturating_sub(cell.chars().count())));
            if index + 1 < columns {
                out.push_str("  ");
            }
        }
        out.push('\n');
        if row_index == 0 {
            let underline: usize = widths.iter().sum::<usize>() + (columns.saturating_sub(1) * 2);
            out.push_str(&"─".repeat(underline));
            out.push('\n');
        }
    }
    out.push_str("```\n");
    rows.clear();
}

fn is_separator_row(row: &str) -> bool {
    row.replace(['|', '-', ' ', ':'], "").is_empty()
}

/// Slack's mrkdwn has only three literal characters that must be escaped
/// outside of code spans: `&`, `<`, `>`.
fn escape_mrkdwn(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn inline_mrkdwn(value: &str) -> String {
    let mut codes = Vec::new();
    let value = replace_code_spans(value, &mut codes);
    let mut value = escape_mrkdwn(&value);
    value = replace_links(&value);
    value = replace_pairs(&value, "~~", "~", "~");
    // GFM bold (** or __) -> Slack bold (*); GFM italic (* or _) -> Slack
    // italic (_). Bold must be swapped before italic so a leftover single
    // `*`/`_` is read as the italic marker, not a stray bold delimiter.
    value = replace_pairs(&value, "**", "\u{1}", "\u{1}");
    value = replace_pairs(&value, "__", "\u{1}", "\u{1}");
    value = replace_spaced_pairs(&value, "*", "_", "_");
    value = replace_spaced_pairs(&value, "_", "_", "_");
    value = value.replace('\u{1}', "*");
    for (index, code) in codes.iter().enumerate() {
        value = value.replace(
            &format!("\u{0}{index}\u{0}"),
            &format!("`{}`", code.replace('`', "'")),
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
        out.push('<');
        out.push_str(url);
        out.push('|');
        out.push_str(&rest[open + 1..text_end]);
        out.push('>');
        rest = &rest[url_end + 1..];
    }
    out.push_str(rest);
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bold_and_italic_conventions() {
        let out = markdown_to_mrkdwn("**bold** and *italic* and _also italic_");
        assert!(out.contains("*bold*"));
        assert!(out.contains("_italic_"));
        assert!(out.contains("_also italic_"));
    }

    #[test]
    fn converts_links_to_angle_pipe_form() {
        let out = markdown_to_mrkdwn("[site](https://example.com)");
        assert_eq!(out, "<https://example.com|site>");
    }

    #[test]
    fn headings_become_bold_lines() {
        let out = markdown_to_mrkdwn("# Title\n\nbody");
        assert!(out.starts_with("*Title*"));
    }

    #[test]
    fn renders_bullets_and_ordered_items() {
        let out = markdown_to_mrkdwn("- one\n- two\n1. first\n2. second");
        assert!(out.contains("• one"));
        assert!(out.contains("• two"));
        assert!(out.contains("1. first"));
        assert!(out.contains("2. second"));
    }

    #[test]
    fn renders_table_as_aligned_code_block() {
        let out = markdown_to_mrkdwn("| A | B |\n| - | - |\n| 1 | 22 |");
        assert!(out.contains("```"));
        assert!(out.contains("A"));
        assert!(out.contains("22"));
    }

    #[test]
    fn strikethrough_and_code_spans_convert() {
        let out = markdown_to_mrkdwn("~~gone~~ and `code`");
        assert!(out.contains("~gone~"));
        assert!(out.contains("`code`"));
    }

    #[test]
    fn escapes_ampersand_and_angle_brackets_outside_code() {
        let out = markdown_to_mrkdwn("A & B < C > D");
        assert!(out.contains("&amp;"));
        assert!(out.contains("&lt;"));
        assert!(out.contains("&gt;"));
    }
}
