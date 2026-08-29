//! GFM-to-Discord markdown projection.
//!
//! Discord's message markdown is close to GFM already (bold, italic,
//! strikethrough, inline/fenced code, blockquotes, and `#`/`##`/`###`
//! headings all match). The two real gaps are links — `[text](url)` is not
//! turned into a hyperlink in a plain message, only inside embeds — and
//! tables, which Discord has no syntax for at all.
pub fn markdown_to_discord(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len() + 64);
    let mut in_fence = false;
    let mut table = Vec::new();
    let mut blank_pending = false;

    for line in markdown.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            flush_table(&mut out, &mut table);
            out.push_str(line);
            out.push('\n');
            in_fence = !in_fence;
            blank_pending = false;
            continue;
        }
        if in_fence {
            out.push_str(line);
            out.push('\n');
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

        if blank_pending && !out.is_empty() {
            out.push('\n');
        }
        blank_pending = false;
        out.push_str(&replace_links(line));
        out.push('\n');
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

/// Discord suppresses the hyperlink for a plain-message `[text](url)`; it
/// only renders the text and the raw URL runs together. Render both the
/// label and a wrapped URL (`<...>` suppresses Discord's own link-preview
/// embed) so the destination is still visible and doesn't spam an embed.
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
        if url.contains(['<', '>', ' ']) {
            out.push_str(&rest[..open + 1]);
            rest = &rest[open + 1..];
            continue;
        }
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..text_end]);
        out.push_str(" (<");
        out.push_str(url);
        out.push_str(">)");
        rest = &rest[url_end + 1..];
    }
    out.push_str(rest);
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_native_markdown_unchanged() {
        let out = markdown_to_discord("**bold** *italic* ~~strike~~ `code` ||spoiler||");
        assert!(out.contains("**bold**"));
        assert!(out.contains("*italic*"));
        assert!(out.contains("~~strike~~"));
        assert!(out.contains("`code`"));
        assert!(out.contains("||spoiler||"));
    }

    #[test]
    fn preserves_headings_and_blockquotes_and_lists() {
        let out = markdown_to_discord("# Title\n\n> quoted\n- item\n1. first");
        assert!(out.contains("# Title"));
        assert!(out.contains("> quoted"));
        assert!(out.contains("- item"));
        assert!(out.contains("1. first"));
    }

    #[test]
    fn rewrites_links_with_a_visible_suppressed_url() {
        let out = markdown_to_discord("[site](https://example.com)");
        assert_eq!(out, "site (<https://example.com>)");
    }

    #[test]
    fn renders_table_as_aligned_code_block() {
        let out = markdown_to_discord("| A | B |\n| - | - |\n| 1 | 22 |");
        assert!(out.contains("```"));
        assert!(out.contains("A"));
        assert!(out.contains("22"));
    }

    #[test]
    fn preserves_fenced_code_blocks_untouched() {
        let out = markdown_to_discord("```rust\nfn main() {}\n```");
        assert!(out.contains("```rust"));
        assert!(out.contains("fn main() {}"));
    }

    #[test]
    fn renders_horizontal_rule() {
        let out = markdown_to_discord("above\n\n---\n\nbelow");
        assert!(out.contains("──────────"));
    }
}
