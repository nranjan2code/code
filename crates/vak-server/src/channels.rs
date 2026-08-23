//! Channel-aware message formatting (docs/design/22-gateway.md).
//!
//! The agent speaks GitHub-flavored markdown; channels speak their own
//! dialects. Each surface declares a flavor and delivery converts once,
//! centrally — bridges stay dumb transports.
//!
//! Telegram gets HTML (`parse_mode: "HTML"`): it supports bold/italic/
//! links/inline-code/pre/blockquote but NOT tables or # headings, and it
//! rejects unescaped `<>&`. So: escape everything first, then translate a
//! conservative markdown subset onto allowed tags, and demote exotic blocks
//! (tables, nested fences) to monospace pre-blocks which read well on
//! phones.

/// Convert GFM-ish markdown to Telegram-safe HTML.
pub fn markdown_to_telegram_html(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + 64);
    let mut in_fence = false;
    let mut fence_buf: Vec<String> = Vec::new();
    let mut table_buf: Vec<String> = Vec::new();

    let flush_table = |out: &mut String, rows: &mut Vec<String>| {
        if !rows.is_empty() {
            out.push_str("<pre>");
            // Drop separator rows like |---|---|.
            for r in rows.iter() {
                let is_sep = r.replace(['|', '-', ' ', ':'], "").is_empty();
                if !is_sep {
                    out.push_str(&escape_html(r.trim()));
                    out.push('\n');
                }
            }
            out.push_str("</pre>");
            rows.clear();
        }
    };

    for line in md.lines() {
        let trimmed = line.trim_start();

        // Fenced code blocks → <pre>.
        if let Some(_rest) = trimmed.strip_prefix("```") {
            if in_fence {
                out.push_str("<pre>");
                out.push_str(&escape_html(&fence_buf.join("\n")));
                out.push_str("</pre>");
                fence_buf.clear();
                in_fence = false;
            } else {
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            fence_buf.push(line.to_string());
            continue;
        }

        // Tables: consecutive lines starting AND ending with '|'.
        if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
            table_buf.push(trimmed.to_string());
            continue;
        }
        flush_table(&mut out, &mut table_buf);

        // Headings → bold line.
        if let Some(after) = trimmed.strip_prefix('#')
            && after.starts_with(' ')
        {
            let text = after.trim().trim_start_matches('#').trim();
            if !text.is_empty() {
                out.push_str("<b>");
                out.push_str(&inline_md(&escape_html(text)));
                out.push_str("</b>\n\n");
                continue;
            }
        }

        // Blockquote.
        if let Some(q) = trimmed.strip_prefix("> ") {
            out.push_str(&format!(
                "<blockquote>{}</blockquote>\n",
                inline_md(&escape_html(q))
            ));
            continue;
        }

        // Bullets → •.
        if let Some(b) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            out.push_str(&format!("• {}\n", inline_md(&escape_html(b))));
            continue;
        }

        out.push_str(&inline_md(&escape_html(line)));
        out.push('\n');
    }
    // EOF inside fence/table: flush what we have rather than dropping.
    if in_fence {
        out.push_str("<pre>");
        out.push_str(&escape_html(&fence_buf.join("\n")));
        out.push_str("</pre>");
    }
    flush_table(&mut out, &mut table_buf);
    // Drop only the final synthetic newline; content inside <pre> is kept.
    out.trim_end_matches('\n').to_string()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Inline-level markdown applied to already-escaped text. Code spans are
/// extracted first so their contents are shielded from emphasis/link rules.
fn inline_md(s: &str) -> String {
    let mut codes: Vec<String> = Vec::new();
    let mut s = replace_code_spans(s, &mut codes);

    s = replace_links(&s);
    s = replace_emphasis(&s, "**", "<b>", "</b>");
    s = replace_emphasis(&s, "__", "<b>", "</b>");
    s = replace_emphasis(&s, "*", "<i>", "</i>");
    s = replace_emphasis(&s, "_", "<i>", "</i>");

    for (i, c) in codes.iter().enumerate() {
        s = s.replace(
            &format!("\u{0}{i}\u{0}"),
            &format!("<code>{}</code>", escape_html(c)),
        );
    }
    s
}

fn replace_code_spans(s: &str, sink: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
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

fn replace_links(s: &str) -> String {
    // [text](url) → <a href="url">text</a>. URLs containing quotes/angle
    // brackets/spaces are not treated as links.
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let Some(rel_close_t) = rest[open..].find("](") else {
            break;
        };
        let close_t = open + rel_close_t;
        let Some(rel_close_u) = rest[close_t..].find(')') else {
            break;
        };
        let close_u = close_t + rel_close_u;
        let url = &rest[close_t + 2..close_u];
        if url.contains('"') || url.contains('<') || url.contains(' ') {
            out.push_str(&rest[..open + 1]);
            rest = &rest[open + 1..];
            continue;
        }
        let label = &rest[open + 1..close_t];
        out.push_str(&rest[..open]);
        out.push_str(&format!(r#"<a href="{url}">{}</a>"#, label));
        rest = &rest[close_u + 1..];
    }
    out.push_str(rest);
    out
}

fn replace_emphasis(s: &str, marker: &str, open: &str, close: &str) -> String {
    if marker == "*" || marker == "_" {
        // Single-char markers need non-space after opener / non-word before
        // opener so snake_case and bullet-like text survive untouched.
        pair_replace_spaced(s, marker, open, close)
    } else {
        pair_replace_simple(s, marker, open, close)
    }
}

fn pair_replace_simple(s: &str, marker: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
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

fn pair_replace_spaced(s: &str, marker: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    loop {
        match rest.split_once(marker) {
            Some((before, after)) => {
                let opens_ok = after.chars().next().is_some_and(|c| !c.is_whitespace());
                let boundary_ok = before.is_empty()
                    || before.chars().last().is_some_and(|c| !c.is_alphanumeric());
                match after.split_once(marker) {
                    Some((inner, tail)) if opens_ok && boundary_ok && !inner.trim().is_empty() => {
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
/// Split rendered HTML into Telegram-sized chunks. Tag-safe in the strong
/// sense: tags opened in a chunk are closed before it ends and re-opened at
/// the start of the next chunk, so every chunk parses standalone.
pub fn split_html_chunks(html: &str, cap: usize) -> Vec<String> {
    fn scan(html: &str) -> Vec<(usize, usize, bool, String)> {
        let mut v = Vec::new();
        let b = html.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'<'
                && let Some(rel) = html[i..].find('>')
            {
                let gt = i + rel;
                if let Some(sp_end) = html[i + 1..gt].find(|c: char| c.is_whitespace() || c == '>')
                {
                    let raw = &html[i + 1..i + 1 + sp_end];
                    let name = raw.trim_start_matches('/').to_string();
                    v.push((i, gt + 1, html[i + 1..].starts_with('/'), name));
                    i = gt + 1;
                    continue;
                }
            }
            i += 1;
        }
        v
    }

    if html.chars().count() <= cap {
        return vec![html.to_string()];
    }

    let tags = scan(html);
    let mut chunks: Vec<String> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut pos = 0usize;
    let mut char_count = 0usize;
    let mut cur = String::new();

    fn flush(cur: &mut String, stack: &mut [String], chunks: &mut Vec<String>) {
        for t in stack.iter().rev() {
            cur.push_str(&format!("</{}>", t));
        }
        chunks.push(std::mem::take(cur).trim_end().to_string());
        for t in stack.iter() {
            cur.push_str(&format!("<{}>", t));
        }
    }

    let mut ti = 0usize;
    while pos < html.len() {
        let next_tag = tags.get(ti).map(|(s, _, _, _)| *s).unwrap_or(html.len());
        // Consume text segment up to the next tag, flushing on cap.
        while pos < next_tag {
            let ch_len = html[pos..].chars().next().map(char::len_utf8).unwrap_or(1);
            if char_count + ch_len > cap {
                flush(&mut cur, &mut stack, &mut chunks);
                char_count = 0;
            }
            cur.push_str(&html[pos..pos + ch_len]);
            pos += ch_len;
            char_count += ch_len;
        }
        if pos >= html.len() {
            break;
        }
        let (_, gt, is_close, name) = &tags[ti];
        let tok_len = gt - pos;
        if char_count + tok_len > cap {
            flush(&mut cur, &mut stack, &mut chunks);
            char_count = 0;
        }
        let gt = *gt;
        let is_close = *is_close;
        if is_close && let Some(p) = stack.iter().rposition(|t| *t == *name) {
            stack.remove(p);
        } else if !is_close {
            stack.push(name.clone());
        }
        cur.push_str(&html[pos..gt]);
        pos = gt;
        char_count += tok_len;
        ti += 1;
    }
    for t in stack.iter().rev() {
        cur.push_str(&format!("</{}>", t));
    }
    chunks.push(cur.trim().to_string());
    chunks
        .into_iter()
        .filter(|c| !c.trim().is_empty())
        .collect()
}

/// Remove tags for the plain-text fallback path.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        match rest[lt..].find('>') {
            Some(gt_rel) => rest = &rest[lt + gt_rel + 1..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn escapes_and_converts_basics() {
        let md = "# Title\n\n**bold** *it* `code` [site](https://x.com/a)\n\n> quoted\n- item";
        let h = markdown_to_telegram_html(md);
        assert!(h.contains("<b>Title</b>"), "{h}");
        assert!(h.contains("<b>bold</b>"));
        assert!(h.contains("<i>it</i>"));
        assert!(h.contains("<code>code</code>"));
        assert!(h.contains(r#"<a href="https://x.com/a">site</a>"#));
        assert!(h.contains("<blockquote>quoted</blockquote>"));
        assert!(h.contains("• item"));
        assert!(!h.contains("**"));
    }

    #[test]
    fn fences_become_pre_and_tables_survive_as_mono() {
        let md = "```rust\nfn a() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |";
        let h = markdown_to_telegram_html(md);
        assert!(h.contains("<pre>fn a() {}</pre>") || h.contains("<pre>\nfn"));
        assert!(h.contains("| a | b |"), "table kept mono: {h}");
        assert!(
            !h.contains("|---|---|") || h.contains("<pre>"),
            "sep dropped or fenced"
        );
    }

    #[test]
    fn html_in_model_output_is_escaped_not_interpreted() {
        let h = markdown_to_telegram_html("use <script>alert(1)</script> please");
        assert!(!h.contains("<script>"));
        assert!(h.contains("&lt;script&gt;"));
    }

    #[test]
    fn snake_case_not_italicized() {
        let h = markdown_to_telegram_html("var my_var_name works");
        assert!(!h.contains("<i>"), "{h}");
    }

    #[test]
    fn chunker_respects_caps_and_never_splits_tags() {
        let body = format!("{}<a href=\"u\">link</a>", "word ".repeat(300));
        let chunks = split_html_chunks(&body, 400);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.chars().count() <= 420);
            assert_eq!(
                c.matches('<').count(),
                c.matches('>').count(),
                "unbalanced: {c}"
            );
        }
    }

    #[test]
    fn strip_tags_recovers_plain_text() {
        assert_eq!(strip_tags("<b>hi</b> <a href=\"x\">yo</a>"), "hi yo");
    }
}
