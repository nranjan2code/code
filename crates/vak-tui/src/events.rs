//! Agent event rendering — feed/flush/draw functions for SSE events.

use crossterm::style::Color;
use similar::{ChangeTag, TextDiff};

use crate::markdown::LineStyler;
use crate::render::Screen;
use crate::state::{RunState, ThinkingMode, UiState};
use crate::theme::{self, Theme};
use vak_client::types::{AgentEvent, StreamEvent};

const DIFF_MAX_LINES: usize = 12;
const APPROVAL_DIFF_LINES: usize = 6;
const APPROVAL_DIFF_EDITS: usize = 3;
const ARGS_SUMMARY_WIDTH: usize = 90;
const ARGS_FALLBACK_WIDTH: usize = 60;
const ARGS_LIST_MAX_LINES: usize = 6;
const PREVIEW_CAP_EXPANDED: usize = 30;
const PREVIEW_CAP_COLLAPSED: usize = 6;

pub fn render_event(event: &AgentEvent, ui: &mut UiState, screen: &mut Screen, theme: &Theme) {
    match event {
        AgentEvent::Stream(StreamEvent::TextDelta { delta, .. }) => {
            flush_thinking(ui, screen, theme);
            ui.thinking_shown = false;
            ui.run_state = RunState::Streaming;
            ui.last_response.push_str(delta);
            feed_text(delta, ui, screen, theme);
        }
        AgentEvent::Stream(StreamEvent::ThinkingDelta { delta, .. }) => match ui.thinking_mode {
            ThinkingMode::Off => {}
            ThinkingMode::Indicator if !ui.thinking_shown => {
                screen.clear_input();
                screen.dim("  · thinking…");
                ui.thinking_shown = true;
            }
            ThinkingMode::Indicator => {}
            ThinkingMode::Full => feed_thinking(delta, ui, screen, theme),
        },
        AgentEvent::Stream(_) => {}
        AgentEvent::ToolCallStart {
            id,
            name,
            args_json,
        } => {
            flush_md(ui, screen, theme);
            ui.run_state = RunState::Tool { name: name.clone() };
            ui.tool_args
                .insert(id.clone(), (name.clone(), args_json.clone()));
            render_tool_preview(name, args_json, ui, screen, theme);
        }
        AgentEvent::ToolCallEnd {
            id,
            name,
            is_error,
            result_preview,
        } => {
            flush_md(ui, screen, theme);
            ui.run_state = RunState::Streaming;
            let stored = ui.tool_args.remove(id);
            screen.clear_input();
            screen.tool_line(if *is_error { "✗" } else { "✓" }, name, *is_error);
            if name == "edit"
                && !is_error
                && let Some((_, args_json)) = &stored
                && let Some(diff) = edit_call_diff(args_json)
            {
                for line in diff.lines() {
                    screen.md_line(&format!("  {line}"));
                }
            } else if *is_error
                && let Some(prev) = result_preview.as_deref()
                && !prev.trim().is_empty()
            {
                render_preview_lines(screen, prev, theme.error, ui.expanded_tools);
            } else if ui.expanded_tools
                && let Some(prev) = result_preview.as_deref()
                && !prev.trim().is_empty()
            {
                render_preview_lines(screen, prev, theme.dim, true);
            }
        }
        AgentEvent::TurnEnd { usage } => {
            flush_thinking(ui, screen, theme);
            ui.total_in += usage.input_tokens;
            ui.total_out += usage.output_tokens;
            if let Some(c) =
                crate::pricing::session_cost(&ui.model, usage.input_tokens, usage.output_tokens)
            {
                ui.cost_usd += c;
            }
            flush_md(ui, screen, theme);
        }
        AgentEvent::TurnStart { .. } => {
            ui.styler = LineStyler::new();
            ui.partial.clear();
            ui.last_response.clear();
            ui.thinking_shown = false;
            ui.thinking_partial.clear();
            ui.run_state = RunState::Thinking;
        }
        AgentEvent::RetryScheduled {
            attempt,
            delay_ms,
            reason,
        } => {
            ui.run_state = RunState::Retrying {
                attempt: *attempt,
                delay_ms: *delay_ms,
                reason: reason.clone(),
            };
            screen.clear_input();
            screen.dim(&format!("⟳ retry {attempt} in {delay_ms}ms — {reason}"));
        }
        AgentEvent::RouteFallback {
            to_provider,
            to_model,
        } => {
            ui.run_state = RunState::Streaming;
            screen.clear_input();
            screen.dim(&format!(
                "⤵ route fallback → {to_provider}/{to_model} (frozen ladder leg)"
            ));
        }
        AgentEvent::ContextCompacting { estimated_tokens } => {
            ui.run_state = RunState::Compacting;
            screen.clear_input();
            screen.dim(&format!(
                "📦 compacting context (~{estimated_tokens} tokens)…"
            ));
        }
        AgentEvent::ContextCompacted {
            before_tokens,
            after_tokens,
            summarized_messages,
            selected_messages,
            dropped_messages,
        } => {
            ui.run_state = RunState::Streaming;
            screen.clear_input();
            screen.dim(&format!(
                "📦 context compacted: ~{before_tokens} → ~{after_tokens} tokens ({summarized_messages} summarized · {selected_messages} kept verbatim · {dropped_messages} dropped)"
            ));
        }
        AgentEvent::StopHookContinuation { reason } => {
            screen.clear_input();
            screen.dim(&format!("[stop-hook] {reason} — continuing"));
        }
        AgentEvent::SubagentStarted { label } => {
            flush_md(ui, screen, theme);
            screen.clear_input();
            screen.accent(&format!("  ◆ subagent: {}", trunc_str(label, 70)));
        }
        AgentEvent::SubagentToolCall {
            label,
            name,
            is_error,
        } => {
            flush_md(ui, screen, theme);
            screen.clear_input();
            let mark = if *is_error { "✗" } else { "·" };
            let color = if *is_error { theme.error } else { theme.dim };
            screen.styled(
                &format!("    {} {} ({})", mark, name, trunc_str(label, 24)),
                color,
            );
        }
        AgentEvent::SubagentUsage {
            input_tokens,
            output_tokens,
            ..
        } => {
            ui.sub_in += input_tokens;
            ui.sub_out += output_tokens;
        }
        AgentEvent::SubagentFinished {
            label,
            is_error,
            elapsed_ms,
        } => {
            flush_md(ui, screen, theme);
            screen.clear_input();
            if *is_error {
                screen.error(&format!(
                    "  ◇ subagent failed: {} · {}",
                    trunc_str(label, 60),
                    crate::status::fmt_elapsed(elapsed_ms / 1000)
                ));
            } else {
                screen.success(&format!(
                    "  ◇ subagent done: {} · {}",
                    trunc_str(label, 60),
                    crate::status::fmt_elapsed(elapsed_ms / 1000)
                ));
            }
        }
        AgentEvent::HandoffReset { before_tokens } => {
            screen.dim(&format!(
                "✋ context reset — structured handoff written (~{before_tokens} → fresh segment)"
            ));
        }
        AgentEvent::StreamOpened
        | AgentEvent::ApprovalRequested { .. }
        | AgentEvent::RunFinished { .. }
        | AgentEvent::Lagged => {}
    }
}

pub fn feed_thinking(text: &str, ui: &mut UiState, screen: &mut Screen, theme: &Theme) {
    ui.thinking_partial.push_str(text);
    while let Some(pos) = ui.thinking_partial.find('\n') {
        let raw: String = ui.thinking_partial.drain(..=pos).collect();
        screen.clear_input();
        screen.styled(&format!("  │ {}", raw.trim_end_matches('\n')), theme.dim);
    }
    ui.thinking_shown = true;
}

pub fn flush_thinking(ui: &mut UiState, screen: &mut Screen, theme: &Theme) {
    if ui.thinking_partial.is_empty() {
        return;
    }
    let rest = std::mem::take(&mut ui.thinking_partial);
    screen.clear_input();
    screen.styled(&format!("  │ {rest}"), theme.dim);
}

pub fn render_tool_preview(
    name: &str,
    input: &str,
    ui: &mut UiState,
    screen: &mut Screen,
    theme: &Theme,
) {
    screen.tool_start(name, &summarize_args(input));
    if !ui.expanded_tools {
        return;
    }
    for line in pretty_args_lines(input) {
        screen.styled(line.trim_start(), theme.dim);
    }
}

fn render_preview_lines(screen: &mut Screen, preview: &str, color: Color, expanded: bool) {
    let cap = if expanded {
        PREVIEW_CAP_EXPANDED
    } else {
        PREVIEW_CAP_COLLAPSED
    };
    let lines: Vec<&str> = preview
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let skipped = lines.len().saturating_sub(cap);
    if skipped > 0 {
        screen.styled(&format!("  │ … {skipped} earlier lines"), color);
    }
    for line in lines.iter().skip(skipped) {
        screen.styled(&format!("  │ {line}"), color);
    }
}

pub fn feed_text(text: &str, ui: &mut UiState, screen: &mut Screen, theme: &Theme) {
    ui.partial.push_str(text);
    while let Some(pos) = ui.partial.find('\n') {
        let raw: String = ui.partial.drain(..=pos).collect();
        let styled = ui.styler.line(raw.trim_end_matches('\n'), theme);
        screen.clear_input();
        screen.md_line(&styled);
    }
}

pub fn flush_md(ui: &mut UiState, screen: &mut Screen, theme: &Theme) {
    if ui.partial.is_empty() {
        return;
    }
    let rest = std::mem::take(&mut ui.partial);
    ui.last_response.push_str(&rest);
    let styled = ui.styler.line(&rest, theme);
    screen.clear_input();
    screen.md_line(&styled);
}

pub fn finish_outcome(
    is_error: bool,
    summary: &str,
    ui: &mut UiState,
    screen: &mut Screen,
    theme: &Theme,
) {
    flush_md(ui, screen, theme);
    let dollars = if ui.cost_usd > 0.0 {
        format!(" · ~{}", crate::pricing::format_cost(ui.cost_usd))
    } else {
        String::new()
    };
    if is_error {
        screen.error(&format!("── failed: {summary}"));
        return;
    }
    screen.success(&format!(
        "── completed · Σ ↑{} ↓{}{dollars}",
        crate::status::fmt_tokens(ui.total_in),
        crate::status::fmt_tokens(ui.total_out),
    ));
    if !summary.trim().is_empty() {
        screen.styled(summary, theme.dim);
    }
}

pub fn status_ansi(ui: &UiState, running: bool, theme: &Theme) -> String {
    let state = &ui.run_state;
    let motion = crate::status::frame_motion(0, running);
    let (spinner, spinner_color) = match state {
        RunState::Retrying { .. } | RunState::Compacting => (
            if running {
                format!("⏳ {}", state.label())
            } else {
                state.label()
            },
            theme.warning,
        ),
        RunState::Tool { .. } => (format!("⚙ {}", state.label()), theme.accent),
        RunState::Thinking => (format!("{motion} thinking"), theme.spinner),
        RunState::Streaming => (motion.to_string(), theme.spinner),
    };
    let mut segments = vec![format!(
        "↑{} ↓{}",
        crate::status::fmt_tokens(ui.total_in),
        crate::status::fmt_tokens(ui.total_out)
    )];
    if ui.cost_usd > 0.0 {
        segments.push(format!("~{}", crate::pricing::format_cost(ui.cost_usd)));
    }
    if ui.sub_in + ui.sub_out > 0 {
        segments.push(format!(
            "◆ ↑{} ↓{}",
            crate::status::fmt_tokens(ui.sub_in),
            crate::status::fmt_tokens(ui.sub_out)
        ));
    }
    format!(
        "{}{}{} {}{}{}",
        theme::fg(spinner_color),
        spinner,
        theme::fg(Color::Reset),
        theme::fg(theme.dim),
        segments.join(" · "),
        theme::fg(Color::Reset),
    )
}

pub fn edit_diff_text(old: &str, new: &str, file_hint: Option<&str>) -> String {
    diff_text(old, new, file_hint, DIFF_MAX_LINES)
}

pub fn learned_spec(input: &str) -> (String, Option<String>) {
    let Some((tool, args_json)) = input.split_once(char::is_whitespace) else {
        return (input.to_string(), None);
    };
    (
        input.to_string(),
        derive_spec(tool.trim(), args_json.trim()),
    )
}

pub fn approval_edit_diff(args_json: &str) -> Option<String> {
    let v = serde_json::from_str::<serde_json::Value>(args_json).ok()?;
    let edits = v.get("edits")?.as_array()?;
    let mut out = String::new();
    let mut shown = 0usize;
    for e in edits.iter().take(APPROVAL_DIFF_EDITS) {
        let old = e.get("old_string").and_then(|x| x.as_str()).unwrap_or("");
        let new = e.get("new_string").and_then(|x| x.as_str()).unwrap_or("");
        if old.is_empty() && new.is_empty() {
            continue;
        }
        out.push_str(&diff_text(old, new, None, APPROVAL_DIFF_LINES));
        out.push('\n');
        shown += 1;
    }
    (shown > 0).then_some(out)
}

pub fn pretty_args_lines(args_json: &str) -> Vec<String> {
    let v: serde_json::Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(_) => return vec![format!("    {}", trunc_str(args_json, 100))],
    };
    let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| args_json.to_string());
    let mut lines: Vec<String> = pretty.lines().map(|l| format!("    {l}")).collect();
    if lines.len() > ARGS_LIST_MAX_LINES {
        let rest = lines.len() - ARGS_LIST_MAX_LINES;
        lines.truncate(ARGS_LIST_MAX_LINES);
        lines.push(format!("    … (+{rest} lines)"));
    }
    lines
}

pub fn summarize_args(args_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(args_json) else {
        return trunc_str(args_json, ARGS_FALLBACK_WIDTH);
    };
    let pick = |k: &str| v.get(k).and_then(|x| x.as_str());
    let hint = pick("command")
        .or_else(|| pick("pattern"))
        .or_else(|| pick("path"))
        .or_else(|| pick("url"))
        .map(str::to_string)
        .unwrap_or_else(|| v.to_string());
    trunc_str(&hint, ARGS_SUMMARY_WIDTH)
}

pub fn trunc_one(s: &str, max_width: usize) -> String {
    trunc_str(s, max_width)
}

pub fn trunc_cells(cells: &[String], max_width: usize) -> Vec<String> {
    cells.iter().map(|c| trunc_str(c, max_width)).collect()
}

pub fn budget_marker(usd: f64, cap: Option<f64>) -> String {
    let Some(cap) = cap else {
        return String::new();
    };
    match vak_core::finops::alert_level(usd, cap) {
        Some(vak_core::finops::AlertLevel::Full) => format!("✗ ${usd:.2}/${cap:.2}"),
        Some(vak_core::finops::AlertLevel::Eighty) => format!("⚠ ${usd:.2}/${cap:.2}"),
        None => String::new(),
    }
}

/// Plain-text unified diff of two buffers; empty when they are identical.
/// `file_hint` adds `---/+++` headers naming the file when present.
fn diff_text(old: &str, new: &str, file_hint: Option<&str>, max_lines: usize) -> String {
    if old == new {
        return String::new();
    }
    let diff = TextDiff::from_lines(old, new);
    let mut lines: Vec<String> = Vec::new();
    if let Some(hint) = file_hint {
        lines.push(format!("--- a/{hint}"));
        lines.push(format!("+++ b/{hint}"));
    }
    let mut changed = false;
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
                match change.tag() {
                    ChangeTag::Equal => lines.push(body.to_string()),
                    ChangeTag::Delete => {
                        changed = true;
                        lines.push(format!("- {body}"));
                    }
                    ChangeTag::Insert => {
                        changed = true;
                        lines.push(format!("+ {body}"));
                    }
                }
            }
        }
    }
    if !changed {
        return String::new();
    }
    let keep = if lines.len() > max_lines {
        max_lines.saturating_sub(1)
    } else {
        lines.len()
    };
    let mut out: String = lines[..keep].join("\n");
    if lines.len() > keep {
        out.push_str(&format!("\n… (+{} more)", lines.len() - keep));
    }
    out
}

/// Unified-diff text for a stored `edit` call, accepting both the canonical
/// `edits[]` array and models' occasional flat `{old_string,new_string}` shape.
/// None when nothing renderable.
fn edit_call_diff(args_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let path = v.get("path").and_then(|p| p.as_str());
    let pairs: Vec<(String, String)> =
        if let Some(edits) = v.get("edits").and_then(|e| e.as_array()) {
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
        let d = diff_text(&old, &new, path, DIFF_MAX_LINES);
        if d.is_empty() {
            continue;
        }
        out.push_str(&d);
        out.push('\n');
        any = true;
    }
    any.then_some(out)
}

/// Derives a scoped, round-trip-validated rule spec from `tool args_json`.
/// Returns None when no safe scope can be derived (opaque bash, missing args).
fn derive_spec(tool: &str, args_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let spec = match tool {
        "bash" => {
            let cmd = v.get("command").and_then(|c| c.as_str())?;
            let word = cmd
                .split_whitespace()
                .next()?
                .trim_start_matches(|c: char| {
                    !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.'
                })
                .to_string();
            if word.is_empty()
                || !word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            {
                return None;
            }
            format!("bash({word} *)")
        }
        "write" | "edit" => format!("{}({})", tool, v.get("path").and_then(|p| p.as_str())?),
        "mcp" => {
            let server = v.get("server").and_then(|s| s.as_str())?;
            if v.get("action").and_then(|a| a.as_str()) != Some("call") {
                return None;
            }
            format!("mcp({server}/*)")
        }
        "task" => format!("task({})", v.get("label").and_then(|l| l.as_str())?),
        _ => return None,
    };
    let rule = vak_permission::Rule::parse(&spec).ok()?;
    rule.matches(tool, &v).then_some(spec)
}

fn trunc_str(s: &str, max: usize) -> String {
    let mut w = 0usize;
    for (i, c) in s.char_indices() {
        w += crate::width::char_width(c);
        if w > max {
            return format!("{}…", &s[..i]);
        }
    }
    s.to_string()
}

#[cfg(test)]
mod events_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::markdown::strip_ansi;

    fn theme() -> Theme {
        Theme::from_name("dark")
    }

    #[test]
    fn run_states_label_the_row() {
        assert_eq!(RunState::Thinking.label(), "thinking");
        assert_eq!(RunState::Streaming.label(), "streaming");
        assert_eq!(
            RunState::Tool {
                name: "bash".into()
            }
            .label(),
            "tool · bash"
        );
        assert_eq!(
            RunState::Retrying {
                attempt: 2,
                delay_ms: 1500,
                reason: "rate-limited".into()
            }
            .label(),
            "retry 2 in 1500ms — rate-limited"
        );
        assert_eq!(RunState::Compacting.label(), "compacting context");
    }

    #[test]
    fn status_row_shows_typed_state_not_bare_spinner() {
        let theme = theme();
        let retrying = RunState::Retrying {
            attempt: 1,
            delay_ms: 500,
            reason: "overloaded".into(),
        };
        for (state, needle) in [
            (&retrying, "retry 1"),
            (&RunState::Compacting, "compacting context"),
            (
                &RunState::Tool {
                    name: "edit".into(),
                },
                "tool · edit",
            ),
            (&RunState::Thinking, "thinking"),
        ] {
            let ui = UiState {
                run_state: state.clone(),
                ..UiState::default()
            };
            let row = status_ansi(&ui, true, &theme);
            let still_plain = strip_ansi(&status_ansi(&ui, false, &theme));
            assert!(
                !still_plain.contains('\u{280b}') && !still_plain.contains('\u{23f3}'),
                "reduced motion drops animated glyphs: {still_plain}"
            );
            assert!(
                strip_ansi(&row).contains(needle),
                "'{needle}' missing from: {}",
                strip_ansi(&row)
            );
        }
    }

    #[test]
    fn status_row_carries_totals_cost_and_subagent_spend() {
        let theme = theme();
        let mut ui = UiState {
            run_state: RunState::Streaming,
            ..UiState::default()
        };
        let idle = strip_ansi(&status_ansi(&ui, false, &theme));
        assert_eq!(idle, "· ↑0 ↓0", "no dollars while nothing spent: {idle}");

        ui.total_in = 1_300;
        ui.total_out = 20;
        ui.cost_usd = 0.05;
        ui.sub_in = 2_000_000;
        ui.sub_out = 1_000;
        let row = strip_ansi(&status_ansi(&ui, true, &theme));
        assert!(row.contains("↑1.3k ↓20"), "{row}");
        assert!(row.contains("$0.0500"), "{row}");
        assert!(row.contains("◆ ↑2M ↓1k"), "{row}");
    }

    #[test]
    fn budget_marker_matches_finops_thresholds() {
        // Below 80% of cap: silent.
        assert_eq!(budget_marker(7.9, Some(10.0)), "");
        assert_eq!(budget_marker(0.0, Some(10.0)), "");
        // ≥80% warns; ≥100% errors — mirroring AlertLevel.
        assert_eq!(budget_marker(8.0, Some(10.0)), "⚠ $8.00/$10.00");
        assert_eq!(budget_marker(9.99, Some(10.0)), "⚠ $9.99/$10.00");
        assert_eq!(budget_marker(10.0, Some(10.0)), "✗ $10.00/$10.00");
        assert_eq!(budget_marker(150.0, Some(10.0)), "✗ $150.00/$10.00");
        // No/negative cap never alerts (alert_level contract).
        assert_eq!(budget_marker(9.0, None), "");
        assert_eq!(budget_marker(9.0, Some(0.0)), "");
        assert_eq!(budget_marker(9.0, Some(-1.0)), "");
    }

    #[test]
    fn trunc_cells_clips_each_row_to_display_width() {
        let cells = vec!["short".to_string(), "abcdef".to_string()];
        assert_eq!(trunc_cells(&cells, 4), vec!["shor…", "abcd…"]);
        assert_eq!(trunc_cells(&cells, 10), cells);
        assert_eq!(trunc_cells(&[], 8), Vec::<String>::new());
        // Wide chars count as two cells and are cut on char boundaries.
        assert_eq!(trunc_cells(&["中文中文".to_string()], 5), vec!["中文…"]);
    }

    #[test]
    fn summarize_args_picks_meaningful_hints() {
        assert_eq!(
            summarize_args(r#"{"command":"cargo test --lib","timeout":30}"#),
            "cargo test --lib"
        );
        assert_eq!(summarize_args(r#"{"path":"src/lib.rs"}"#), "src/lib.rs");
        assert_eq!(
            summarize_args(r#"{"pattern":"*.rs","path":"crates"}"#),
            "*.rs"
        );
        assert_eq!(
            summarize_args(r#"{"url":"https://example.com/x"}"#),
            "https://example.com/x"
        );
        // Unknown shapes fall back to compact JSON, over-long to ellipsis.
        assert_eq!(summarize_args(r#"{"z":1}"#), r#"{"z":1}"#);
        let long = summarize_args(r#"{"command":"x"}"#);
        assert!(long.chars().count() <= ARGS_SUMMARY_WIDTH + 1);
        assert_eq!(summarize_args("not json"), "not json");
    }

    #[test]
    fn pretty_args_lines_indents_and_caps() {
        let lines = pretty_args_lines(r#"{"path":"a.rs"}"#);
        assert_eq!(lines, vec!["    {", "      \"path\": \"a.rs\"", "    }"]);

        let big = serde_json::json!({"a":1,"b":2,"c":3,"d":4,"e":5});
        let lines = pretty_args_lines(&big.to_string());
        assert_eq!(lines.len(), ARGS_LIST_MAX_LINES + 1);
        assert!(lines.last().unwrap().starts_with("    … (+"));

        assert_eq!(
            pretty_args_lines("broken"),
            vec![format!("    {}", trunc_str("broken", 100))]
        );
    }

    #[test]
    fn learned_spec_scopes_calls_and_rejects_unscopeable() {
        // bash: scoped to first word
        let (_, s) = learned_spec(r#"bash {"command":"cargo test --lib"}"#);
        assert_eq!(s.as_deref(), Some("bash(cargo *)"));

        // opaque bash (command substitution) cannot be scoped safely
        let (_, s) = learned_spec(r#"bash {"command":"echo $(rm -rf /)"}"#);
        assert_eq!(s, None);

        // file tools: exact-path scope that must round-trip match the call
        let (_, s) =
            learned_spec(r#"edit {"path":"src/lib.rs","old_string":"a","new_string":"b"}"#);
        assert_eq!(s.as_deref(), Some("edit(src/lib.rs)"));

        // mcp: server-scoped, only for calls
        let (_, s) = learned_spec(r#"mcp {"action":"call","server":"gh","tool":"pr"}"#);
        assert_eq!(s.as_deref(), Some("mcp(gh/*)"));
        let (_, s) = learned_spec(r#"mcp {"action":"list"}"#);
        assert_eq!(s, None);

        // unknown tools and tool-less input get no persisted rule
        let (_, s) = learned_spec(r#"mystery {}"#);
        assert_eq!(s, None);
        let (echo, s) = learned_spec("bareword");
        assert_eq!(echo, "bareword");
        assert_eq!(s, None);

        // round-trip: the derived bash rule actually matches a sibling command
        let rule = vak_permission::Rule::parse("bash(cargo *)").unwrap();
        assert!(rule.matches(
            "bash",
            &serde_json::json!({"command": "cargo build --release"})
        ));
    }

    #[test]
    fn edit_diff_text_renders_unified_diff_with_optional_header() {
        let d = edit_diff_text("fn a() {}", "fn a() { b(); }", Some("a.rs"));
        assert!(d.contains("--- a/a.rs"), "{d}");
        assert!(d.contains("+++ b/a.rs"), "{d}");
        assert!(d.contains("+ fn a() { b(); }"), "{d}");
        assert!(d.contains("- fn a() {}"), "{d}");
        assert!(d.contains("@@"), "{d}");

        let plain = edit_diff_text("one\n", "one\ntwo\n", None);
        assert!(!plain.contains("---"), "{plain}");
        assert!(plain.contains("+ two"), "{plain}");

        assert_eq!(edit_diff_text("same", "same", Some("x.rs")), "");

        let long_old: String = (0..40).map(|_| "line\n").collect();
        let capped = edit_diff_text(&long_old, "", None);
        assert!(capped.contains("(+"), "cap note expected: {capped}");
        assert!(capped.lines().count() <= DIFF_MAX_LINES);
    }

    #[test]
    fn approval_edit_diff_renders_capped_edits_array_only() {
        let args = r#"{"path":"a.rs","edits":[{"old_string":"x","new_string":"y"},{"old_string":"","new_string":""}]}"#;
        let d = approval_edit_diff(args).unwrap();
        assert!(d.contains("- x"), "{d}");
        assert!(d.contains("+ y"), "{d}");

        assert_eq!(approval_edit_diff(r#"{"path":"a.rs"}"#), None);
        assert_eq!(approval_edit_diff("not json"), None);
    }
}
