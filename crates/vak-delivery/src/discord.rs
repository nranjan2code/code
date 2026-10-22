//! GFM-to-Discord markdown projection.
//!
//! Discord supports the common GFM constructs natively, including headings
//! and masked links. Tables are lowered to aligned code blocks.
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

        if is_spoiler(trimmed) {
            flush_table(&mut out, &mut table);
            if blank_pending && !out.is_empty() {
                out.push('\n');
            }
            blank_pending = false;
            out.push_str(trimmed);
            out.push('\n');
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

        if let Some(rest) = strip_task_item(trimmed) {
            if blank_pending && !out.is_empty() {
                out.push('\n');
            }
            blank_pending = false;
            out.push_str(&format!("{rest}\n"));
            continue;
        }

        if blank_pending && !out.is_empty() {
            out.push('\n');
        }
        blank_pending = false;
        out.push_str(line);
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

fn is_spoiler(trimmed: &str) -> bool {
    trimmed.starts_with("||") && trimmed.ends_with("||") && trimmed.len() > 4
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

fn strip_task_item(trimmed: &str) -> Option<String> {
    trimmed
        .strip_prefix("- [ ] ")
        .map(|rest| format!("☐ {}", rest))
        .or_else(|| {
            trimmed
                .strip_prefix("- [x] ")
                .map(|rest| format!("☑ {}", rest))
        })
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
    fn keeps_masked_links_for_discord_to_render() {
        let out = markdown_to_discord("[site](https://example.com)");
        assert_eq!(out, "[site](https://example.com)");
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
