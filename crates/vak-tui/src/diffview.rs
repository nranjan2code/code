use similar::{ChangeTag, TextDiff};

use crate::theme::{Theme, fg};

const RESET_FG: &str = "\x1b[39m";

/// Render a unified line diff with ANSI styling. Hunk headers dim, context
/// default, deletions "- " in error color, additions "+ " in success color.
/// Total rendered lines are capped at `max_lines`; when truncated a final dim
/// `… (+N more)` line reports the hidden remainder (counted inside the cap).
/// The result has no trailing newline.
pub fn unified(old: &str, new: &str, theme: &Theme, max_lines: usize) -> String {
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
                    ChangeTag::Delete => format!("{}- {body}{RESET_FG}", fg(theme.error)),
                    ChangeTag::Insert => format!("{}+ {body}{RESET_FG}", fg(theme.success)),
                });
            }
        }
    }
    render_capped(&lines, theme, max_lines)
}

fn render_capped(lines: &[String], theme: &Theme, max_lines: usize) -> String {
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
        out.push_str(&format!(
            "{}… (+{} more){RESET_FG}",
            fg(theme.dim),
            lines.len() - keep
        ));
    } else if !out.is_empty() {
        out.truncate(out.len() - 1);
    }
    out
}
