//! Approval and confirm card rendering.

use crossterm::style::Color;

use crate::render::Screen;
use crate::theme::{self, Theme};

pub struct ApprovalCard {
    pub id: String,
    pub tool: String,
    pub args_json: String,
    pub reason: String,
}

pub fn draw_approval(
    card: &ApprovalCard,
    selected_action: Option<bool>,
    screen: &mut Screen,
    theme: &Theme,
) {
    let styled = |text: String, color: Color| {
        format!("{}{text}{}", theme::fg(color), theme::fg(Color::Reset))
    };
    let tool_label = if card.tool == "finops-budget" {
        "budget raise request".to_string()
    } else {
        card.tool.clone()
    };
    let mut lines = vec![styled(format!("╭─ approval · {tool_label}"), theme.warning)];
    if card.tool == "webfetch"
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&card.args_json)
        && let Some(url) = v.get("url").and_then(|u| u.as_str())
    {
        lines.push(styled(
            format!("│ url · {}", trunc_cells(url, 120)),
            theme.accent,
        ));
    }
    for line in pretty_args_lines(&card.args_json, 6) {
        lines.push(styled(format!("│ {}", line.trim_start()), theme.dim));
    }
    if !card.reason.trim().is_empty() {
        lines.push(styled(
            format!("│ rule · {}", trunc_cells(&card.reason, 90)),
            theme.dim,
        ));
    }
    lines.push(action_line(
        'y',
        "yes",
        selected_action == Some(true),
        theme,
    ));
    lines.push(action_line(
        'n',
        "no",
        selected_action == Some(false),
        theme,
    ));
    lines.push(styled("╰─ e edit-diff · Esc close".to_string(), theme.dim));
    screen.redraw_block(&lines);
}

/// Inline confirm card — same shape as an approval so the interaction reads
/// identically.
pub fn draw_confirm(prompt: &str, screen: &mut Screen, theme: &Theme) {
    let styled = |text: String, color: Color| {
        format!("{}{text}{}", theme::fg(color), theme::fg(Color::Reset))
    };
    let mut lines = vec![styled("╭─ confirm".to_string(), theme.warning)];
    for line in prompt.lines() {
        lines.push(styled(format!("│ {line}"), theme.dim));
    }
    lines.push(styled("╰─ y confirm · n/Esc cancel".to_string(), theme.dim));
    screen.redraw_block(&lines);
}

fn action_line(key: char, label: &str, selected: bool, theme: &Theme) -> String {
    let marker = if selected { "› [" } else { "  [" };
    let text = format!("│ {marker}{key}] {label}");
    let color = if selected { theme.accent } else { theme.dim };
    format!("{}{text}{}", theme::fg(color), theme::fg(Color::Reset))
}

fn pretty_args_lines(args_json: &str, max_lines: usize) -> Vec<String> {
    let v: serde_json::Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(_) => return vec![format!("    {}", trunc_cells(args_json, 100))],
    };
    let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| args_json.to_string());
    let mut lines: Vec<String> = pretty.lines().map(|l| format!("    {l}")).collect();
    if lines.len() > max_lines {
        let rest = lines.len() - max_lines;
        lines.truncate(max_lines);
        lines.push(format!("    … (+{rest} lines)"));
    }
    lines
}

fn trunc_cells(s: &str, max: usize) -> String {
    let mut w = 0usize;
    for (i, c) in s.char_indices() {
        w += crate::width::char_width(c);
        if w > max {
            return format!("{}…", &s[..i]);
        }
    }
    s.to_string()
}
