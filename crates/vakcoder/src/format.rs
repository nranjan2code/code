//! Plain-text formatting helpers for the headless CLI's non-interactive
//! output (tool-call summaries, unified diffs, ANSI stripping).
//!
//! These are adapted from the now-removed `vak-tui` crate, keeping only the
//! plain (non-ANSI, non-interactive) code paths — no crossterm/ratatui
//! rendering, keymap, palette, editor, or event loop involved.

use similar::{ChangeTag, TextDiff};

/// Summarize a tool call's JSON args into a short one-line hint for the CLI's
/// progress output (e.g. `▸ bash echo hi`).
pub fn summarize_args(name: &str, args_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(args_json) else {
        return trunc_cells(args_json, 60);
    };
    let pick = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let hint = match name {
        "bash" => pick("command"),
        "read" | "write" | "edit" => pick("path"),
        "glob" => pick("pattern"),
        "webfetch" => pick("url"),
        "grep" => {
            let base = v.get("path").and_then(|x| x.as_str()).unwrap_or(".");
            format!("{} in {base}", pick("pattern"))
        }
        _ => v.to_string(),
    };
    trunc_cells(&hint, 90)
}

/// Truncate `s` to at most `max` display columns (accounting for
/// double-width characters), appending an ellipsis when truncated.
pub fn trunc_cells(s: &str, max: usize) -> String {
    let mut w = 0usize;
    for (i, c) in s.char_indices() {
        w += char_width(c);
        if w > max {
            return format!("{}…", &s[..i]);
        }
    }
    s.to_string()
}

fn char_width(c: char) -> usize {
    let cp = u32::from(c);
    if cp < 0x20 || (0x7F..0xA0).contains(&cp) {
        return 0;
    }
    if is_zero_width(cp) {
        return 0;
    }
    if is_wide(cp) { 2 } else { 1 }
}

fn is_zero_width(cp: u32) -> bool {
    matches!(
        cp,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            | 0x0711
            | 0x0730..=0x074A
            | 0x07A6..=0x07B0
            | 0x0816..=0x0819
            | 0x081B..=0x0823
            | 0x0825..=0x0827
            | 0x0829..=0x082D
            | 0x0E31
            | 0x0E34..=0x0E3A
            | 0x0E47..=0x0E4E
            | 0x135D..=0x135F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x206A..=0x206F
            | 0x20D0..=0x20F0
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xFEFF
            | 0xE0100..=0xE01EF
    )
}

fn is_wide(cp: u32) -> bool {
    matches!(
        cp,
        0x1100..=0x115F
            | 0x2329..=0x232A
            | 0x2600..=0x27BF
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1B000..=0x1B2FF
            | 0x1F000..=0x1FFFD
            | 0x20000..=0x3FFFD
    )
}

/// Render a plain (no ANSI) unified diff for the `edit` tool's args JSON.
/// Mirrors `vak_tui::app::edit_diff_text` but skips styling entirely since
/// the CLI's plain output has no use for it.
pub fn edit_diff_text(args_json: &str, max_lines: usize) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let pairs: Vec<(String, String)> = if let Some(edits) = v.get("edits").and_then(|e| e.as_array()) {
        edits
            .iter()
            .filter_map(|e| {
                let old = e.get("old_string").and_then(|x| x.as_str())?;
                let new = e.get("new_string").and_then(|x| x.as_str())?;
                Some((old.to_string(), new.to_string()))
            })
            .collect()
    } else {
        let old = v.get("old_string").and_then(|x| x.as_str())?;
        let new = v.get("new_string").and_then(|x| x.as_str())?;
        vec![(old.to_string(), new.to_string())]
    };
    let mut out = String::new();
    let mut any = false;
    for (old, new) in pairs {
        if old.is_empty() && new.is_empty() {
            continue;
        }
        out.push_str(&unified_diff(&old, &new, max_lines));
        out.push('\n');
        any = true;
    }
    any.then_some(out)
}

/// Plain unified diff renderer, adapted from `vak_tui::diffview::unified`
/// with ANSI styling stripped since the caller only wants plain text.
fn unified_diff(old: &str, new: &str, max_lines: usize) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut lines: Vec<String> = Vec::new();
    for group in diff.grouped_ops(3) {
        if group.is_empty() {
            continue;
        }
        let mut old_start = usize::MAX;
        let mut old_end = 0;
        let mut new_start = usize::MAX;
        let mut new_end = 0;
        for op in &group {
            let o = op.old_range();
            let n = op.new_range();
            old_start = old_start.min(o.start);
            old_end = old_end.max(o.end);
            new_start = new_start.min(n.start);
            new_end = new_end.max(n.end);
        }
        lines.push(format!(
            "@@ -{},{} +{},{} @@",
            old_start.saturating_add(1),
            old_end - old_start,
            new_start.saturating_add(1),
            new_end - new_start
        ));
        for op in &group {
            for change in diff.iter_changes(op) {
                let body = change.value().trim_end_matches(['\n', '\r']);
                lines.push(match change.tag() {
                    ChangeTag::Equal => body.to_string(),
                    ChangeTag::Delete => format!("- {body}"),
                    ChangeTag::Insert => format!("+ {body}"),
                });
            }
        }
    }
    render_capped(&lines, max_lines)
}

fn render_capped(lines: &[String], max_lines: usize) -> String {
    let mut out = String::new();
    let keep = if lines.len() > max_lines {
        max_lines.saturating_sub(1)
    } else {
        lines.len()
    };
    for line in &lines[..keep] {
        out.push_str(line);
        out.push('\n');
    }
    if lines.len() > keep {
        out.push_str(&format!("… (+{} more)", lines.len() - keep));
    } else if !out.is_empty() {
        out.truncate(out.len() - 1);
    }
    out
}

/// Strip ANSI escape sequences from a string (used when a tool's captured
/// output includes color codes that plain CLI output should not show).
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
