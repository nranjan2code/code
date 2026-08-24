use std::io::Write;

use crossterm::style::Color;

use crate::theme::Theme;
use crate::width::str_width;

pub struct Screen {
    out: std::io::Stdout,
    theme: Theme,
    header: Option<HeaderState>,
    history: Vec<String>,
    transient: Option<TransientState>,
    last_transient: Option<String>,
    /// Accessibility: ASCII glyphs, no imposed colors (doc 21 §P2).
    plain: bool,
    /// Accessibility: linear text only — strips styling and decorative
    /// glyphs so screen readers announce clean sentences.
    reader: bool,
}

#[derive(Clone)]
struct TransientState {
    surface: String,
    rows: usize,
    cursor_row: usize,
    cursor_col: usize,
}

struct HeaderState {
    version: String,
    provider: String,
    model: String,
    session: String,
}

impl Screen {
    pub fn new(theme: Theme) -> Self {
        Screen {
            out: std::io::stdout(),
            theme,
            header: None,
            history: Vec::new(),
            transient: None,
            last_transient: None,
            plain: false,
            reader: false,
        }
    }

    pub fn set_accessibility(&mut self, plain: bool, reader: bool) {
        self.plain = plain;
        self.reader = reader;
        self.last_transient = None;
        self.render_workspace();
    }

    pub fn accessibility(&self) -> (bool, bool) {
        (self.plain, self.reader)
    }

    pub fn accessible(&self) -> bool {
        self.plain || self.reader
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.last_transient = None;
        self.render_workspace();
    }

    pub fn clear_viewport(&mut self) {
        self.put("\x1b[2J\x1b[H");
        self.history.clear();
        self.transient = None;
        self.last_transient = None;
    }

    pub fn resize(&mut self) {
        self.last_transient = None;
        self.render_workspace();
    }

    pub fn update_agent_identity(&mut self, provider: &str, model: &str) {
        if let Some(header) = self.header.as_mut() {
            header.provider = provider.to_string();
            header.model = model.to_string();
        }
        self.render_workspace();
    }

    fn put(&mut self, s: &str) {
        self.out.write_all(s.as_bytes()).ok();
        self.out.flush().ok();
    }

    pub fn inline(&mut self, s: &str) {
        self.put(s);
    }

    pub fn line(&mut self, s: &str) {
        self.push_history(s.to_string());
    }

    pub fn styled(&mut self, s: &str, color: Color) {
        let text = terminal_newlines(s);
        self.push_history(format!(
            "{}{text}{}",
            crate::theme::fg(color),
            crate::theme::fg(Color::Reset)
        ));
    }

    pub fn dim(&mut self, s: &str) {
        self.styled(s, self.theme.dim);
    }

    pub fn accent(&mut self, s: &str) {
        self.styled(s, self.theme.accent);
    }

    pub fn success(&mut self, s: &str) {
        self.styled(s, self.theme.success);
    }

    pub fn warn(&mut self, s: &str) {
        self.styled(s, self.theme.warning);
    }

    pub fn error(&mut self, s: &str) {
        self.styled(s, self.theme.error);
    }

    pub fn user_message(&mut self, text: &str, queued: bool) {
        let label = if queued { "you · queued" } else { "you" };
        self.styled(&format!("╭─ {label}"), self.theme.accent);
        for line in text.lines() {
            self.line(&format!("│ {line}"));
        }
        self.dim("╰─");
    }

    /// Prints a pre-styled string containing raw ANSI escapes as one line.
    pub fn md_line(&mut self, styled_ansi: &str) {
        self.push_history(styled_ansi.to_string());
    }

    pub fn tool_start(&mut self, name: &str, args_hint: &str) {
        let hint = if args_hint.is_empty() {
            String::new()
        } else {
            format!(" {args_hint}")
        };
        self.push_history(format!(
            "{}╭─ tool {}{}",
            crate::theme::fg(self.theme.dim),
            name,
            crate::theme::fg(Color::Reset)
        ));
        if !hint.is_empty() {
            self.dim(&format!(
                "╰─{}",
                trunc_width(&hint, self.cols().saturating_sub(3))
            ));
        }
    }

    pub fn tool_line(&mut self, mark: &str, name: &str, is_error: bool) {
        let color = if is_error {
            self.theme.error
        } else {
            self.theme.success
        };
        self.push_history(format!(
            "{}╰─ {mark} {name}{}",
            crate::theme::fg(color),
            crate::theme::fg(Color::Reset)
        ));
    }

    fn cols(&self) -> usize {
        crossterm::terminal::size()
            .map(|(c, _)| c)
            .unwrap_or(80)
            .max(20) as usize
    }

    fn rows(&self) -> usize {
        crossterm::terminal::size()
            .map(|(_, rows)| rows)
            .unwrap_or(24)
            .max(10) as usize
    }

    /// Kept for callers that commit transcript output before a redraw. In the
    /// retained full-screen renderer the composer is a separate layer, so
    /// transcript writes never need to erase it first.
    pub fn clear_input(&mut self) {}

    fn push_history(&mut self, text: String) {
        for line in terminal_newlines(&text).split("\r\n") {
            let line = if self.accessible() {
                fold_glyphs(line)
            } else {
                line.to_string()
            };
            self.history.push(line);
        }
        const HISTORY_LIMIT: usize = 4_000;
        if self.history.len() > HISTORY_LIMIT {
            self.history.drain(..self.history.len() - HISTORY_LIMIT);
        }
        self.render_transcript();
    }

    /// Copies text to the system clipboard via OSC52. Only ever called from
    /// explicit user actions — clipboard mutation is never automatic.
    pub fn osc52_copy(&mut self, text: &str) {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        self.put(&format!("\x1b]52;c;{encoded}\x07"));
    }

    pub fn redraw_composer(&mut self, label: &str, buf: &str, cursor: usize, footer: &str) {
        let cols = self.cols();
        let layout = composer_layout(cols, label, buf, cursor, footer);
        self.redraw_transient(&layout.lines, None, layout.cursor_row, layout.cursor_col);
    }

    pub fn redraw_palette(
        &mut self,
        query: &str,
        items: &[crate::palette::PaletteItem],
        selected: usize,
    ) {
        let width = panel_width(self.cols());
        let mut lines = vec![top_border("commands", width)];
        lines.push(panel_row(&format!("/{query}"), width));
        if items.is_empty() {
            lines.push(panel_row("  no matching commands", width));
        } else {
            for (index, item) in items.iter().enumerate() {
                let marker = if index == selected { "›" } else { " " };
                let row = format!("{marker} /{:<12} {}", item.name, item.description);
                lines.push(panel_row(&row, width));
            }
        }
        lines.push(bottom_border(
            "↑↓ navigate · Enter choose · Esc close",
            width,
        ));
        self.redraw_transient(&lines, Some(selected + 2), 1, 3 + str_width(query));
    }

    pub fn redraw_picker(
        &mut self,
        title: &str,
        query: &str,
        items: &[crate::palette::ChoiceItem],
        selected: usize,
        allow_custom: bool,
    ) {
        let width = panel_width(self.cols());
        let mut lines = vec![top_border(title, width)];
        let filter = if query.is_empty() {
            "filter…".to_string()
        } else {
            query.to_string()
        };
        lines.push(panel_row(&format!("⌕ {filter}"), width));
        if items.is_empty() {
            let empty = if allow_custom && !query.trim().is_empty() {
                format!("› use custom ID: {}", query.trim())
            } else {
                "  no matching options".to_string()
            };
            lines.push(panel_row(&empty, width));
        } else {
            for (index, item) in items.iter().enumerate() {
                let marker = if index == selected { "›" } else { " " };
                let active = if item.active { "✓" } else { " " };
                lines.push(panel_row(
                    &format!("{marker} {active} {:<24} {}", item.value, item.description),
                    width,
                ));
            }
        }
        lines.push(bottom_border(
            "↑↓ navigate · Enter session · Ctrl-S save project · Esc close",
            width,
        ));
        let selected_row = if items.is_empty() {
            Some(2)
        } else {
            Some(selected + 2)
        };
        self.redraw_transient(&lines, selected_row, 1, 4 + str_width(query));
    }

    /// Replaces the input block with a single pre-styled status row.
    pub fn redraw_status(&mut self, styled_ansi: &str) {
        self.redraw_raw_transient(styled_ansi.to_string(), 1, 0, 0);
    }

    pub fn redraw_block(&mut self, lines: &[String]) {
        let rows = lines.len().max(1);
        self.redraw_raw_transient(lines.join("\r\n"), rows, rows - 1, 0);
    }

    pub fn panel(&mut self, title: &str, rows: &[String], footer: &str) {
        let width = panel_width(self.cols());
        let mut lines = Vec::with_capacity(rows.len() + 2);
        lines.push(top_border(title, width));
        lines.extend(rows.iter().map(|row| panel_row(row, width)));
        lines.push(bottom_border(footer, width));
        let surface = self.surface_lines(&lines, None);
        for line in surface.split("\r\n") {
            self.history.push(line.to_string());
        }
        self.render_transcript();
    }

    pub fn redraw_modal(
        &mut self,
        title: &str,
        rows: &[String],
        scroll: usize,
        footer: &str,
        selected: Option<usize>,
    ) {
        let height = self.rows().saturating_sub(self.header_rows()).max(3);
        let body_height = height.saturating_sub(2);
        let width = panel_width(self.cols());
        let max_scroll = rows.len().saturating_sub(body_height);
        let scroll = scroll.min(max_scroll);
        let mut lines = Vec::with_capacity(height);
        lines.push(top_border(title, width));
        let mut selected_row = None;
        for (offset, row) in rows.iter().skip(scroll).take(body_height).enumerate() {
            if selected == Some(scroll + offset) {
                selected_row = Some(lines.len());
            }
            lines.push(panel_row(row, width));
        }
        while lines.len() < height - 1 {
            lines.push(panel_row("", width));
        }
        let position = if rows.len() > body_height {
            format!(
                "{footer} · {}–{} of {}",
                scroll + 1,
                (scroll + body_height).min(rows.len()),
                rows.len()
            )
        } else {
            footer.to_string()
        };
        lines.push(bottom_border(&position, width));
        self.redraw_transient(&lines, selected_row, height - 1, 1);
    }

    pub fn redraw_running(
        &mut self,
        status: &str,
        pending: &str,
        queued_count: usize,
        approval_count: usize,
    ) {
        let queued = if queued_count == 0 {
            String::new()
        } else {
            format!(" · {queued_count} queued")
        };
        let approvals = if approval_count == 0 {
            String::new()
        } else {
            format!(" · Alt-A review {approval_count} approval")
        };
        let footer = format!("Enter steer · Tab follow-up{queued}{approvals} · Ctrl-C interrupt");
        let layout = composer_layout(
            self.cols(),
            &format!("agent · {}", crate::markdown::strip_ansi(status)),
            pending,
            pending.chars().count(),
            &footer,
        );
        self.redraw_transient(&layout.lines, None, layout.cursor_row, layout.cursor_col);
    }

    fn redraw_transient(
        &mut self,
        lines: &[String],
        selected_row: Option<usize>,
        cursor_row: usize,
        cursor_col: usize,
    ) {
        self.redraw_raw_transient(
            self.surface_lines(lines, selected_row),
            lines.len(),
            cursor_row,
            cursor_col,
        );
    }

    fn redraw_raw_transient(
        &mut self,
        surface: String,
        rows: usize,
        cursor_row: usize,
        cursor_col: usize,
    ) {
        let signature = format!("{rows}:{cursor_row}:{cursor_col}:{surface}");
        if self.last_transient.as_deref() == Some(signature.as_str()) {
            return;
        }
        let same_height = self.transient.as_ref().is_some_and(|old| old.rows == rows);
        self.transient = Some(TransientState {
            surface,
            rows,
            cursor_row,
            cursor_col,
        });
        self.last_transient = Some(signature);
        if same_height {
            self.render_transient();
        } else {
            self.render_workspace();
        }
    }

    pub fn banner(&mut self, version: &str, provider: &str, model: &str, session: &str) {
        let session = session
            .rsplit('/')
            .next()
            .unwrap_or(session)
            .trim_end_matches(".jsonl");
        let short_session: String = session.chars().take(8).collect();
        self.header = Some(HeaderState {
            version: version.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            session: short_session,
        });
        self.render_workspace();
    }

    pub fn bell(&mut self) {
        self.put("\x07");
    }

    /// OSC9 desktop notification (iTerm2/WezTerm/Kitty-style); harmless in
    /// terminals that ignore unknown OSC sequences.
    pub fn notify(&mut self, msg: &str) {
        let sanitized: String = msg
            .chars()
            .map(|c| if c == '\x07' || c == '\x1b' { ' ' } else { c })
            .collect();
        self.put(&format!("\x1b]9;{sanitized}\x07"));
    }

    pub fn set_title(&mut self, title: &str) {
        self.put(&format!("\x1b]2;{title}\x07"));
    }

    fn surface_lines(&self, lines: &[String], selected_row: Option<usize>) -> String {
        let accessible = self.accessible();
        let mut rendered = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            let (fg, bg) = if selected_row == Some(index) && !accessible {
                (self.theme.heading, self.theme.selected_bg)
            } else if index == 0 && !accessible {
                (self.theme.accent, self.theme.panel_bg)
            } else if index + 1 == lines.len() && !accessible {
                (self.theme.dim, self.theme.panel_bg)
            } else if !accessible {
                (self.theme.heading, self.theme.panel_bg)
            } else {
                (Color::Reset, Color::Reset)
            };
            let line = if accessible {
                fold_glyphs(line)
            } else {
                line.clone()
            };
            rendered.push(format!(
                "{}{}{}{}{}",
                crate::theme::fg(fg),
                crate::theme::bg(bg),
                line,
                crate::theme::fg(Color::Reset),
                crate::theme::bg(Color::Reset),
            ));
        }
        rendered.join("\r\n")
    }

    fn render_workspace(&mut self) {
        let mut frame = String::from("\x1b[?25l\x1b[2J\x1b[H");
        for (index, line) in self.header_lines().iter().enumerate() {
            frame.push_str(&format!("\x1b[{};1H{line}", index + 1));
        }
        self.append_transcript_frame(&mut frame);
        self.append_transient_frame(&mut frame, true);
        self.put(&frame);
    }

    fn render_transcript(&mut self) {
        let mut frame = String::from("\x1b[?25l");
        self.append_transcript_frame(&mut frame);
        self.append_transient_frame(&mut frame, false);
        self.put(&frame);
    }

    fn render_transient(&mut self) {
        let mut frame = String::from("\x1b[?25l");
        self.append_transient_frame(&mut frame, true);
        self.put(&frame);
    }

    fn append_transcript_frame(&self, frame: &mut String) {
        let top = self.header_rows() + 1;
        let transient_rows = self.transient.as_ref().map_or(0, |state| state.rows);
        let bottom = self.rows().saturating_sub(transient_rows);
        if bottom < top {
            return;
        }
        for row in top..=bottom {
            frame.push_str(&format!("\x1b[{row};1H\x1b[2K"));
        }
        let available = bottom - top + 1;
        let width = self.cols().saturating_sub(2).max(1);
        let mut visible = Vec::new();
        for logical in self.history.iter().rev() {
            let wrapped = wrap_ansi(logical, width);
            for row in wrapped.into_iter().rev() {
                visible.push(row);
                if visible.len() == available {
                    break;
                }
            }
            if visible.len() == available {
                break;
            }
        }
        visible.reverse();
        let start = bottom + 1 - visible.len();
        for (offset, line) in visible.iter().enumerate() {
            frame.push_str(&format!("\x1b[{};2H{line}", start + offset));
        }
    }

    fn append_transient_frame(&self, frame: &mut String, clear: bool) {
        let Some(state) = self.transient.as_ref() else {
            frame.push_str("\x1b[?25h");
            return;
        };
        let start = self.rows().saturating_sub(state.rows) + 1;
        if clear {
            for row in start..=self.rows() {
                frame.push_str(&format!("\x1b[{row};1H\x1b[2K"));
            }
        }
        frame.push_str(&format!("\x1b[{start};1H{}", state.surface));
        let cursor_row = start + state.cursor_row.min(state.rows.saturating_sub(1));
        frame.push_str(&format!(
            "\x1b[{cursor_row};{}H\x1b[?25h",
            state.cursor_col + 1
        ));
    }

    fn header_rows(&self) -> usize {
        usize::from(self.header.is_some()) * 5
    }

    fn header_lines(&self) -> Vec<String> {
        let Some(header) = self.header.as_ref() else {
            return Vec::new();
        };
        let width = panel_width(self.cols());
        let lines = vec![
            top_border(&format!("◆ VakCoder {}", header.version), width),
            panel_row(&format!("provider {}", header.provider), width),
            panel_row(&format!("model    {}", header.model), width),
            panel_row(&format!("session  {}", header.session), width),
            bottom_border("Ctrl-P commands · @ attach · ! shell", width),
        ];
        self.surface_lines(&lines, None)
            .split("\r\n")
            .map(str::to_string)
            .collect()
    }
}

fn terminal_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

/// Accessible rendering: box drawing and decorative marks become ASCII so
/// screen readers announce words instead of geometry, and plain terminals
/// never show tofu boxes.
pub fn fold_glyphs(s: &str) -> String {
    let pairs = [
        ('╭', "+"),
        ('╰', "+"),
        ('╮', "+"),
        ('╯', "+"),
        ('├', "+"),
        ('┤', "+"),
        ('└', "+"),
        ('┘', "+"),
        ('┌', "+"),
        ('┐', "+"),
        ('─', "-"),
        ('│', "|"),
    ];
    let mut replaced = s.to_string();
    for (from, to) in pairs {
        replaced = replaced.replace(from, to);
    }
    replaced = replaced.replace("📦 ", "").replace("⏳ ", "");
    replaced = replaced.replace(['⏳', '📦'], "");
    let mut out = String::with_capacity(replaced.len());
    for c in replaced.chars() {
        match c {
            '✓' => out.push('+'),
            '✗' => out.push('x'),
            '◆' | '●' => out.push('*'),
            '▸' | '›' => out.push('>'),
            '◇' => out.push('*'),
            '⚙' => out.push('#'),
            '⟳' => out.push('~'),
            '…' => out.push_str("..."),
            '·' => out.push('.'),
            '↑' => out.push('^'),
            '↓' => out.push('v'),
            other => out.push(other),
        }
    }
    out
}

fn wrap_ansi(s: &str, max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    let mut rows = vec![String::new()];
    let mut width = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(row) = rows.last_mut() {
                row.push(c);
            }
            for next in chars.by_ref() {
                if let Some(row) = rows.last_mut() {
                    row.push(next);
                }
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if c == '\r' || c == '\n' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            rows.push(String::new());
            width = 0;
            continue;
        }
        let cells = crate::width::char_width(c);
        if width > 0 && width + cells > max_width {
            rows.push(String::new());
            width = 0;
        }
        if let Some(row) = rows.last_mut() {
            row.push(c);
        }
        width += cells;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{fold_glyphs, terminal_newlines, wrap_ansi};

    #[test]
    fn committed_multiline_text_returns_to_column_zero() {
        assert_eq!(
            terminal_newlines("one\ntwo\r\nthree\rfour"),
            "one\r\ntwo\r\nthree\r\nfour"
        );
    }

    #[test]
    fn ansi_wrapping_ignores_escape_sequence_width() {
        assert_eq!(
            wrap_ansi("\x1b[31mabcdef\x1b[39m", 3),
            vec!["\x1b[31mabc", "def\x1b[39m"]
        );
    }

    #[test]
    fn glyph_folding_produces_screen_reader_safe_ascii() {
        assert_eq!(
            fold_glyphs("╭─ approval · edit ─╮"),
            "+- approval . edit -+"
        );
        assert_eq!(fold_glyphs("│ ✓ done ✗ failed"), "| + done x failed");
        assert_eq!(
            fold_glyphs("◆ subagent ▸ user ⚙ tool ⟳ retry"),
            "* subagent > user # tool ~ retry"
        );
        assert_eq!(fold_glyphs("📦 compacting context"), "compacting context");
        assert_eq!(fold_glyphs("a…b ↑↓"), "a...b ^v");
        // Regular text is untouched.
        assert_eq!(fold_glyphs("plain words 123"), "plain words 123");
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ComposerLayout {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

pub fn composer_layout(
    cols: usize,
    label: &str,
    buf: &str,
    cursor: usize,
    footer: &str,
) -> ComposerLayout {
    let cols = panel_width(cols);
    let content_width = cols.saturating_sub(4).max(1);
    let mut content = vec![String::new()];
    let mut row = 0usize;
    let mut cells = 0usize;
    let mut cursor_row = 0usize;
    let mut cursor_cells = 0usize;
    let mut char_index = 0usize;
    for c in buf.chars() {
        if char_index == cursor {
            cursor_row = row;
            cursor_cells = cells;
        }
        if c == '\n' {
            content.push(String::new());
            row += 1;
            cells = 0;
        } else {
            let width = crate::width::char_width(c);
            if cells > 0 && cells + width > content_width {
                content.push(String::new());
                row += 1;
                cells = 0;
            }
            content[row].push(c);
            cells += width;
        }
        char_index += 1;
    }
    if cursor >= char_index {
        cursor_row = row;
        cursor_cells = cells;
    }
    let mut lines = vec![top_border(label, cols)];
    lines.extend(content.into_iter().map(|line| panel_row(&line, cols)));
    lines.push(bottom_border(footer, cols));
    ComposerLayout {
        lines,
        cursor_row: cursor_row + 1,
        cursor_col: cursor_cells + 2,
    }
}

fn panel_width(cols: usize) -> usize {
    cols.max(10)
}

fn top_border(label: &str, width: usize) -> String {
    border_line('╭', '╮', label, width)
}

fn bottom_border(label: &str, width: usize) -> String {
    border_line('╰', '╯', label, width)
}

fn border_line(left: char, right: char, label: &str, width: usize) -> String {
    let available = width.saturating_sub(5);
    let label = trunc_width(label, available);
    let fill = width.saturating_sub(5 + str_width(&label));
    format!("{left}─ {label} {}{right}", "─".repeat(fill))
}

fn panel_row(content: &str, width: usize) -> String {
    let inner = width.saturating_sub(4);
    let content = trunc_width(content, inner);
    let padding = inner.saturating_sub(str_width(&content));
    format!("│ {content}{} │", " ".repeat(padding))
}

fn trunc_width(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for c in s.chars() {
        let next = crate::width::char_width(c);
        if width + next + 1 > max {
            break;
        }
        out.push(c);
        width += next;
    }
    out.push('…');
    out
}
