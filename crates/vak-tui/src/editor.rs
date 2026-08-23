use crossterm::event::{KeyCode, KeyModifiers};

const HISTORY_MAX: usize = 500;
const UNDO_MAX: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComposerMode {
    #[default]
    Emacs,
    Vim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VimState {
    #[default]
    Normal,
    Insert,
}

/// A yanked (copied) region awaiting `p`/`P`. Linewise text carries its
/// trailing newline implicitly.
#[derive(Debug, Clone)]
struct Yank {
    text: String,
    linewise: bool,
}

/// Pastes above either threshold collapse into a one-line placeholder chip so
/// a megabyte-scale paste never triggers proportional synchronous layout.
/// The payload is held verbatim and substituted back at submit time, so the
/// ledger still receives the exact submitted input.
pub const STASH_CHAR_LIMIT: usize = 4_000;
pub const STASH_LINE_LIMIT: usize = 60;

#[derive(Debug, Clone)]
struct Stash {
    label: String,
    text: String,
}

fn human_bytes(n: usize) -> String {
    if n >= 1_048_576 {
        format!("{:.1} MB", n as f64 / 1_048_576.0)
    } else if n >= 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Backspace,
    Other,
}

#[derive(Debug, Clone)]
struct Snapshot {
    buf: String,
    cursor: usize,
}

#[derive(Debug, Default, Clone)]
struct Search {
    query: String,
    idx: usize,
}

#[derive(Debug, Default, Clone)]
pub struct Editor {
    buf: String,
    cursor: usize,
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: Option<String>,
    search: Option<Search>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<EditKind>,
    stashes: Vec<Stash>,
    next_stash: usize,
    mode: ComposerMode,
    vim_state: VimState,
    /// Pending Vim operator (`d`/`c`/`y`/`g`) awaiting a motion.
    op: Option<char>,
    yank: Option<Yank>,
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, c: char) {
        self.record_before(EditKind::Insert);
        let byte = self.byte_of_char(self.cursor);
        self.buf.insert(byte, c);
        self.cursor += 1;
        self.history_idx = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.record_before(EditKind::Backspace);
        let byte = self.byte_of_char(self.cursor);
        let prev = self.byte_of_char(self.cursor - 1);
        self.buf.replace_range(prev..byte, "");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        let byte = self.byte_of_char(self.cursor);
        if byte >= self.buf.len() {
            return;
        }
        self.record_before(EditKind::Other);
        let next = self.byte_of_char(self.cursor + 1);
        self.buf.replace_range(byte..next, "");
    }

    pub fn left(&mut self) {
        self.end_edit_group();
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.end_edit_group();
        if self.cursor < self.chars().count() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.end_edit_group();
        let before = &self.buf[..self.byte_of_char(self.cursor)];
        self.cursor = before
            .rfind('\n')
            .map(|byte| self.buf[..=byte].chars().count())
            .unwrap_or(0);
    }

    pub fn end(&mut self) {
        self.end_edit_group();
        let byte = self.byte_of_char(self.cursor);
        self.cursor += self.buf[byte..]
            .find('\n')
            .map(|end| self.buf[byte..byte + end].chars().count())
            .unwrap_or_else(|| self.buf[byte..].chars().count());
    }

    pub fn clear(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        self.record_before(EditKind::Other);
        self.buf.clear();
        self.cursor = 0;
        self.history_idx = None;
    }

    /// Deletes back to the start of the previous word.
    pub fn delete_word_back(&mut self) {
        let target = self.word_start();
        if target == self.cursor {
            return;
        }
        self.record_before(EditKind::Other);
        let byte = self.byte_of_char(self.cursor);
        let prev = self.byte_of_char(target);
        self.buf.replace_range(prev..byte, "");
        self.cursor = target;
    }

    pub fn word_left(&mut self) {
        self.end_edit_group();
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        self.end_edit_group();
        let chars: Vec<char> = self.buf.chars().collect();
        let mut i = self.cursor;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }

    pub fn delete_word_forward(&mut self) {
        let chars: Vec<char> = self.buf.chars().collect();
        let mut target = self.cursor;
        while target < chars.len() && chars[target].is_whitespace() {
            target += 1;
        }
        while target < chars.len() && !chars[target].is_whitespace() {
            target += 1;
        }
        if target == self.cursor {
            return;
        }
        self.record_before(EditKind::Other);
        let start = self.byte_of_char(self.cursor);
        let end = self.byte_of_char(target);
        self.buf.replace_range(start..end, "");
    }

    pub fn delete_to_line_end(&mut self) {
        let start = self.byte_of_char(self.cursor);
        let end = self.buf[start..]
            .find('\n')
            .map(|offset| start + offset)
            .unwrap_or(self.buf.len());
        if start == end {
            return;
        }
        self.record_before(EditKind::Other);
        self.buf.replace_range(start..end, "");
    }

    pub fn undo(&mut self) {
        let Some(previous) = self.undo.pop() else {
            return;
        };
        self.redo.push(self.snapshot());
        self.restore(previous);
        self.last_edit = None;
    }

    pub fn redo(&mut self) {
        let Some(next) = self.redo.pop() else {
            return;
        };
        self.push_undo(self.snapshot());
        self.restore(next);
        self.last_edit = None;
    }

    fn word_start(&self) -> usize {
        let chars: Vec<char> = self.buf.chars().take(self.cursor).collect();
        let mut i = chars.len();
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    pub fn take(&mut self) -> String {
        self.resolve_stashes();
        let out = std::mem::take(&mut self.buf);
        self.cursor = 0;
        self.history_idx = None;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
        self.stashes.clear();
        if !out.trim().is_empty() {
            self.history.push(out.clone());
        }
        out
    }

    pub fn history_prev(&mut self) {
        self.end_edit_group();
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_idx {
            None => {
                self.draft = Some(self.buf.clone());
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_idx = Some(idx);
        self.set_text(&self.history[idx].clone());
    }

    pub fn history_next(&mut self) {
        self.end_edit_group();
        let Some(idx) = self.history_idx else {
            return;
        };
        if idx + 1 >= self.history.len() {
            self.history_idx = None;
            let text = self.draft.take().unwrap_or_default();
            self.set_text(&text);
            return;
        }
        self.history_idx = Some(idx + 1);
        let text = self.history[idx + 1].clone();
        self.set_text(&text);
    }

    pub fn paste_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let lines = s.lines().count().max(1);
        if s.chars().count() > STASH_CHAR_LIMIT || lines > STASH_LINE_LIMIT {
            self.stash_large(s);
            return;
        }
        self.record_before(EditKind::Other);
        for c in s.chars() {
            let byte = self.byte_of_char(self.cursor);
            self.buf.insert(byte, c);
            self.cursor += 1;
        }
    }

    fn stash_large(&mut self, s: &str) {
        self.record_before(EditKind::Other);
        let n = self.next_stash;
        self.next_stash += 1;
        let size = human_bytes(s.len());
        let label = format!(
            "[stashed paste #{} · {} · {} lines · Ctrl-O expands]",
            n,
            size,
            s.lines().count().max(1)
        );
        self.stashes.push(Stash {
            label,
            text: s.to_string(),
        });
        self.insert_last_label();
    }

    fn insert_last_label(&mut self) {
        let Some(stash) = self.stashes.last() else {
            return;
        };
        let label = stash.label.clone();
        let needs_nl = !self.buf.is_empty()
            && !self.text_before().ends_with('\n')
            && !self.text_after().starts_with('\n');
        if needs_nl {
            self.insert('\n');
        }
        for c in label.chars() {
            self.insert(c);
        }
    }

    /// Expands the stashed paste whose placeholder contains the cursor.
    /// Returns a short status message describing what happened.
    pub fn expand_stash_at_cursor(&mut self) -> Option<String> {
        let cur_byte = self.byte_of_char(self.cursor);
        let mut target: Option<(usize, usize, usize)> = None;
        for (idx, stash) in self.stashes.iter().enumerate() {
            let Some(start) = self.buf.find(stash.label.as_str()) else {
                continue;
            };
            let end = start + stash.label.len();
            if start <= cur_byte && cur_byte <= end {
                target = Some((idx, start, end));
                break;
            }
        }
        let (idx, start, end) = target?;
        let payload = self.stashes.remove(idx).text;
        let char_pos = self.buf[..start].chars().count();
        self.record_before(EditKind::Other);
        self.buf.replace_range(start..end, "");
        self.cursor = char_pos;
        for c in payload.chars() {
            self.insert_raw(c);
            self.cursor += 1;
        }
        Some(format!("expanded paste · {} B", payload.len()))
    }

    pub fn has_stashes(&self) -> bool {
        !self.stashes.is_empty()
    }

    pub fn stash_count(&self) -> usize {
        self.stashes.len()
    }

    /// Replaces every remaining placeholder with its verbatim payload.
    /// Called before submission so the ledger receives the exact input.
    pub fn resolve_stashes(&mut self) {
        while !self.stashes.is_empty() {
            let label = self.stashes[0].label.clone();
            let Some(byte) = self.buf.find(label.as_str()) else {
                self.stashes.remove(0);
                continue;
            };
            let payload = self.stashes.remove(0).text;
            let char_pos = self.buf[..byte].chars().count();
            self.buf.replace_range(byte..byte + label.len(), "");
            self.cursor = char_pos;
            for c in payload.chars() {
                self.insert_raw(c);
                self.cursor += 1;
            }
        }
    }

    fn insert_raw(&mut self, c: char) {
        let byte = self.byte_of_char(self.cursor);
        self.buf.insert(byte, c);
    }

    pub fn line_col(&self) -> (usize, usize) {
        let before = &self.buf[..self.byte_of_char(self.cursor)];
        let line = before.matches('\n').count();
        let col = match before.rsplit_once('\n') {
            Some((_, tail)) => tail.chars().count(),
            None => self.cursor,
        };
        (line, col)
    }

    pub fn text_before(&self) -> &str {
        &self.buf[..self.byte_of_char(self.cursor)]
    }

    pub fn text_after(&self) -> &str {
        &self.buf[self.byte_of_char(self.cursor)..]
    }

    pub fn is_multiline(&self) -> bool {
        self.buf.contains('\n')
    }

    /// Persists the current buffer as a crash-safe draft. Written before an
    /// external editor launches; consumed by [`Editor::recover_draft`] on a
    /// later start if the process dies mid-edit.
    pub fn save_draft(&self, path: &std::path::Path) -> bool {
        std::fs::write(path, self.buf.as_bytes()).is_ok()
    }

    /// Restores a previously staged draft, consuming the file. Returns
    /// whether a non-empty draft was restored.
    pub fn recover_draft(&mut self, path: &std::path::Path) -> bool {
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let _ = std::fs::remove_file(path);
        if text.trim().is_empty() {
            return false;
        }
        self.set_text(&text);
        true
    }

    pub fn load_history(&mut self, path: &std::path::Path) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        for line in text.lines() {
            if line.is_empty() || self.history.last().is_some_and(|last| last == line) {
                continue;
            }
            if self.history.len() == HISTORY_MAX {
                self.history.remove(0);
            }
            self.history.push(line.to_string());
        }
    }

    pub fn save_history(&self, path: &std::path::Path) {
        let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
            return;
        };
        let Some(name) = path.file_name() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let start = self.history.len().saturating_sub(HISTORY_MAX);
        let mut body = String::new();
        for entry in &self.history[start..] {
            body.push_str(entry);
            body.push('\n');
        }
        let tmp = parent.join(format!("{}.tmp", name.to_string_lossy()));
        if std::fs::write(&tmp, body).is_err() {
            return;
        }
        if std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    pub fn view(&self) -> (&str, usize) {
        (&self.buf, self.cursor)
    }

    pub fn set_text(&mut self, text: &str) {
        self.buf = text.to_string();
        self.cursor = self.buf.chars().count();
        self.last_edit = None;
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Reverse history search (readline-style). The buffer being edited when
    /// search starts is snapshotted into `draft` and restored on cancel.
    pub fn begin_search(&mut self) {
        if self.search.is_some() {
            return;
        }
        let snapshot = self.draft.take().unwrap_or_else(|| self.buf.clone());
        self.draft = Some(snapshot);
        self.search = Some(Search {
            query: String::new(),
            idx: self.history.len(),
        });
    }

    pub fn search_active(&self) -> bool {
        self.search.is_some()
    }

    pub fn search_query(&self) -> &str {
        self.search.as_ref().map(|s| s.query.as_str()).unwrap_or("")
    }

    pub fn search_push(&mut self, c: char) {
        if let Some(s) = self.search.as_mut() {
            s.query.push(c);
        }
        self.search_from_cursor();
    }

    pub fn search_backspace(&mut self) {
        if let Some(s) = self.search.as_mut() {
            s.query.pop();
            s.idx = self.history.len();
        }
        self.search_from_cursor();
    }

    pub fn search_next(&mut self) {
        if let Some(s) = self.search.as_ref() {
            let target = s.idx;
            let q = s.query.clone();
            if let Some(i) = self.find_match(q.as_str(), target) {
                if let Some(s) = self.search.as_mut() {
                    s.idx = i;
                }
                self.set_text(&self.history[i].clone());
            }
        }
    }

    pub fn accept_search(&mut self) {
        self.search = None;
        self.draft = None;
        self.history_idx = None;
    }

    pub fn cancel_search(&mut self) {
        self.search = None;
        let restored = self.draft.take().unwrap_or_default();
        self.set_text(&restored);
    }

    fn find_match(&self, query: &str, from: usize) -> Option<usize> {
        let mut i = from.min(self.history.len());
        while i > 0 {
            i -= 1;
            if !query.is_empty() && self.history[i].contains(query) {
                return Some(i);
            }
        }
        None
    }

    fn search_from_cursor(&mut self) {
        let (q, start) = match self.search.as_ref() {
            Some(s) => (s.query.clone(), s.idx),
            None => return,
        };
        if q.is_empty() {
            return;
        }
        if let Some(i) = self.find_match(&q, start) {
            if let Some(s) = self.search.as_mut() {
                s.idx = i;
            }
            self.set_text(&self.history[i].clone());
        }
    }

    fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.buf.chars()
    }

    fn byte_of_char(&self, char_idx: usize) -> usize {
        self.buf
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.buf.len())
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            buf: self.buf.clone(),
            cursor: self.cursor,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.buf = snapshot.buf;
        self.cursor = snapshot.cursor;
        self.history_idx = None;
    }

    fn record_before(&mut self, kind: EditKind) {
        if self.last_edit != Some(kind) || kind == EditKind::Other {
            self.push_undo(self.snapshot());
        }
        self.redo.clear();
        self.last_edit = Some(kind);
        self.history_idx = None;
    }

    fn push_undo(&mut self, snapshot: Snapshot) {
        if self.undo.len() == UNDO_MAX {
            self.undo.remove(0);
        }
        self.undo.push(snapshot);
    }

    fn end_edit_group(&mut self) {
        self.last_edit = None;
    }

    pub fn mode(&self) -> ComposerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ComposerMode) {
        if self.mode != mode {
            self.mode = mode;
            self.vim_state = VimState::Normal;
            self.op = None;
        }
    }

    pub fn vim_state(&self) -> VimState {
        self.vim_state
    }

    pub fn enter_normal(&mut self) {
        self.vim_state = VimState::Normal;
        self.op = None;
        // Vim semantics: leaving insert steps back one char when possible.
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn enter_insert(&mut self) {
        self.vim_state = VimState::Insert;
        self.op = None;
        self.end_edit_group();
    }

    /// Vim normal-mode key handling. Consumes modal keys and returns true;
    /// unmapped keys (Enter, Ctrl-*, arrows…) return false so they fall
    /// through to the regular keymap.
    pub fn vim_normal_key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        if self.mode != ComposerMode::Vim {
            return false;
        }
        if code == KeyCode::Char('r') && mods == KeyModifiers::CONTROL {
            if self.vim_state == VimState::Normal {
                self.redo();
            }
            return true;
        }
        if code == KeyCode::Esc && mods.is_empty() {
            // Insert exit or pending-op cancel; self-contained so the app
            // layer's interception stays optional.
            if self.vim_state == VimState::Insert {
                self.enter_normal();
            } else {
                self.op = None;
            }
            return true;
        }
        if self.vim_state != VimState::Normal {
            return false;
        }
        if !mods.is_empty() && mods != KeyModifiers::SHIFT {
            return false;
        }
        let Some(c) = (match code {
            KeyCode::Char(c) => Some(c),
            _ => None,
        }) else {
            return false;
        };
        // Operator + motion composition (d/c/y/g with a pending first key).
        if let Some(op) = self.op {
            self.op = None;
            return self.apply_op(op, c);
        }
        match c {
            'h' => {
                self.end_edit_group();
                let ls = self.line_start_char();
                self.cursor = self.cursor.saturating_sub(1).max(ls);
                true
            }
            'l' => {
                self.end_edit_group();
                let (_, le) = self.current_line_bounds();
                self.cursor = (self.cursor + 1).min(le);
                true
            }
            'j' => {
                self.next_line();
                true
            }
            'k' => {
                self.prev_line();
                true
            }
            '0' => {
                self.home();
                true
            }
            '^' | '_' => {
                self.home_line_start();
                true
            }
            '$' => {
                self.end();
                true
            }
            'G' => {
                self.cursor = self.chars().count();
                true
            }
            'g' => {
                self.op = Some('g');
                true
            }
            'w' => {
                self.end_edit_group();
                self.cursor = self.scan_word_forward();
                true
            }
            'b' => {
                self.word_left();
                true
            }
            'e' => {
                self.word_end();
                true
            }
            'x' => {
                if self.byte_of_char(self.cursor) < self.buf.len() {
                    self.record_before(EditKind::Other);
                    self.delete();
                }
                true
            }
            'D' => {
                self.record_before(EditKind::Other);
                let text = self.line_tail();
                self.yank = Some(Yank {
                    text,
                    linewise: false,
                });
                self.delete_to_line_end();
                true
            }
            'C' => {
                self.record_before(EditKind::Other);
                let text = self.line_tail();
                self.yank = Some(Yank {
                    text,
                    linewise: false,
                });
                self.delete_to_line_end();
                self.enter_insert();
                true
            }
            'd' => {
                self.op = Some('d');
                true
            }
            'c' => {
                self.op = Some('c');
                true
            }
            'y' => {
                self.op = Some('y');
                true
            }
            'p' => {
                self.paste_yank(false);
                true
            }
            'P' => {
                self.paste_yank(true);
                true
            }
            'u' => {
                self.undo();
                true
            }
            'i' => {
                self.enter_insert();
                true
            }
            'a' => {
                if self.cursor < self.chars().count() {
                    self.cursor += 1;
                }
                self.enter_insert();
                true
            }
            'I' => {
                self.home_line_start();
                self.enter_insert();
                true
            }
            'A' => {
                self.end();
                self.enter_insert();
                true
            }
            'o' => {
                self.end();
                self.record_before(EditKind::Other);
                self.insert('\n');
                self.enter_insert();
                true
            }
            'O' => {
                self.home();
                self.record_before(EditKind::Other);
                self.insert('\n');
                self.left();
                self.enter_insert();
                true
            }
            _ => false,
        }
    }

    fn apply_op(&mut self, op: char, motion: char) -> bool {
        // Doubled operator = whole-line scope (dd/cc/yy).
        if motion == op && matches!(op, 'd' | 'c' | 'y') {
            return self.op_linewise(op);
        }
        if op == 'g' {
            if motion == 'g' {
                self.cursor = 0;
                return true;
            }
            return false;
        }
        let target = match motion {
            'w' => self.scan_word_forward(),
            'e' => self.scan_word_end().saturating_sub(0).max(self.cursor),
            'b' => self.word_start(),
            '$' => self.line_end_char(),
            'h' => self.cursor.saturating_sub(1),
            'l' => (self.cursor + 1).min(self.chars().count()),
            '0' => self.line_start_char(),
            '^' => self.line_first_non_blank(),
            'G' => self.chars().count(),
            'g' => 0,
            _ => return false,
        };
        let (start, end) = if target >= self.cursor {
            (self.cursor, target)
        } else {
            (target, self.cursor)
        };
        match op {
            'd' => {
                let text = self.slice_chars(start, end);
                self.record_before(EditKind::Other);
                self.replace_range_chars(start, end, "");
                self.yank = Some(Yank {
                    text,
                    linewise: false,
                });
                self.cursor = start.min(self.chars().count());
                true
            }
            'c' => {
                let text = self.slice_chars(start, end);
                self.record_before(EditKind::Other);
                self.replace_range_chars(start, end, "");
                self.yank = Some(Yank {
                    text,
                    linewise: false,
                });
                self.cursor = start;
                self.enter_insert();
                true
            }
            'y' => {
                self.yank = Some(Yank {
                    text: self.slice_chars(start, end),
                    linewise: false,
                });
                self.cursor = start;
                true
            }
            _ => false,
        }
    }

    fn op_linewise(&mut self, op: char) -> bool {
        let (ls_char, le_char) = self.current_line_bounds();
        let line_text = self.slice_chars(ls_char, le_char);
        let total = self.chars().count();
        let (start, end) = if le_char < total {
            // Not the last line: swallow the trailing newline.
            (ls_char, le_char + 1)
        } else if ls_char > 0 {
            // Last line with no newline of its own: swallow the
            // preceding one instead so the join stays clean.
            (ls_char - 1, le_char)
        } else {
            (ls_char, le_char)
        };
        match op {
            'd' | 'c' => {
                self.record_before(EditKind::Other);
                if op == 'd' {
                    self.replace_range_chars(start, end, "");
                    self.cursor = start.min(self.chars().count());
                } else {
                    // Change clears the content but keeps the line.
                    self.replace_range_chars(ls_char, le_char, "");
                    self.cursor = ls_char;
                }
                self.yank = Some(Yank {
                    text: format!("{line_text}\n"),
                    linewise: true,
                });
                if op == 'c' {
                    self.enter_insert();
                }
                true
            }
            'y' => {
                self.yank = Some(Yank {
                    text: format!("{line_text}\n"),
                    linewise: true,
                });
                self.cursor = ls_char;
                true
            }
            _ => false,
        }
    }

    fn paste_yank(&mut self, before: bool) {
        let Some(y) = self.yank.clone() else {
            return;
        };
        self.record_before(EditKind::Other);
        if y.linewise {
            if before {
                let ls = self.line_start_char();
                self.insert_at_char(ls, &y.text);
                self.cursor = ls;
            } else {
                let le = self.line_end_char();
                if self.byte_of_char(le) < self.buf.len() {
                    let at = le + 1;
                    self.insert_at_char(at, &y.text);
                    self.cursor = at;
                } else {
                    let at = self.chars().count();
                    let payload = format!("\n{}", y.text.trim_end_matches('\n'));
                    self.insert_at_char(at, &payload);
                    self.cursor = at;
                }
            }
        } else {
            let at = if before || self.cursor == self.chars().count() {
                self.cursor
            } else {
                self.cursor + 1
            };
            self.insert_at_char(at, &y.text);
            self.cursor = at;
        }
    }

    fn insert_at_char(&mut self, char_idx: usize, text: &str) {
        let byte = self.byte_of_char(char_idx.min(self.chars().count()));
        self.buf.insert_str(byte, text);
    }

    fn slice_chars(&self, start: usize, end: usize) -> String {
        self.buf[self.byte_of_char(start)..self.byte_of_char(end)].to_string()
    }

    fn replace_range_chars(&mut self, start: usize, end: usize, replacement: &str) {
        let s = self.byte_of_char(start);
        let e = self.byte_of_char(end);
        self.buf.replace_range(s..e, replacement);
    }

    fn current_line_bounds(&self) -> (usize, usize) {
        let cur = self.byte_of_char(self.cursor);
        let ls = self.buf[..cur].rfind('\n').map(|b| b + 1).unwrap_or(0);
        let le_off = self.buf[cur..]
            .find('\n')
            .map(|o| cur + o)
            .unwrap_or(self.buf.len());
        (
            self.buf[..ls].chars().count(),
            self.buf[..le_off].chars().count(),
        )
    }

    fn line_start_char(&self) -> usize {
        self.current_line_bounds().0
    }

    fn line_end_char(&self) -> usize {
        self.current_line_bounds().1
    }

    fn line_first_non_blank(&self) -> usize {
        let (ls, le) = self.current_line_bounds();
        let line = &self.buf[self.byte_of_char(ls)..self.byte_of_char(le)];
        ls + line.chars().take_while(|c| c.is_whitespace()).count()
    }

    fn home_line_start(&mut self) {
        self.cursor = self.line_first_non_blank();
    }

    fn line_tail(&self) -> String {
        let (_, le) = self.current_line_bounds();
        self.slice_chars(self.cursor, le)
    }

    fn scan_word_forward(&self) -> usize {
        let chars: Vec<char> = self.buf.chars().collect();
        let mut i = self.cursor;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        i
    }

    fn scan_word_end(&self) -> usize {
        let chars: Vec<char> = self.buf.chars().collect();
        let mut i = self.cursor;
        while i + 1 < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        while i + 1 < chars.len() && !chars[i + 1].is_whitespace() {
            i += 1;
        }
        i
    }

    fn word_end(&mut self) {
        self.end_edit_group();
        self.cursor = self.scan_word_end();
    }

    fn next_line(&mut self) {
        self.end_edit_group();
        let col = self.cursor.saturating_sub(self.line_start_char());
        let (_, le) = self.current_line_bounds();
        if le >= self.chars().count() {
            return;
        }
        let next_start = le + 1;
        let next_len = self.buf[self.byte_of_char(next_start)..]
            .find('\n')
            .map(|o| {
                self.buf[self.byte_of_char(next_start)..next_start + o]
                    .chars()
                    .count()
            })
            .unwrap_or_else(|| self.buf[self.byte_of_char(next_start)..].chars().count());
        self.cursor = next_start + col.min(next_len);
    }

    fn prev_line(&mut self) {
        self.end_edit_group();
        let ls = self.line_start_char();
        if ls == 0 {
            return;
        }
        let col = self.cursor - ls;
        let prev_ls = self.buf[..self.byte_of_char(ls) - 1]
            .rfind('\n')
            .map(|b| b + 1)
            .unwrap_or(0);
        let prev_len = self.buf[prev_ls..self.byte_of_char(ls) - 1].chars().count();
        self.cursor = prev_ls + col.min(prev_len);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod vim_tests {
    use super::*;

    fn vim_editor(text: &str) -> Editor {
        let mut e = Editor::new();
        e.set_mode(ComposerMode::Vim);
        e.set_text(text);
        // A fresh modal buffer starts with the cursor at home.
        e.vim_normal_key(KeyCode::Char('g'), KeyModifiers::NONE);
        e.vim_normal_key(KeyCode::Char('g'), KeyModifiers::NONE);
        e
    }

    fn type_insert(e: &mut Editor, s: &str) {
        for c in s.chars() {
            e.insert(c);
        }
    }

    #[test]
    fn emacs_is_default_and_vim_starts_in_normal() {
        let mut e = Editor::new();
        assert_eq!(e.mode(), ComposerMode::Emacs);
        e.set_mode(ComposerMode::Vim);
        assert_eq!(e.vim_state(), VimState::Normal);
        // 'h' is motion in normal mode, not insert.
        assert!(e.vim_normal_key(KeyCode::Char('h'), KeyModifiers::NONE));
        assert_eq!(e.view().0, "");
        // Switching back restores plain typing.
        e.set_mode(ComposerMode::Emacs);
        e.insert('h');
        assert_eq!(e.view().0, "h");
    }

    #[test]
    fn normal_motions_move_the_cursor() {
        let mut e = vim_editor("hello world\nsecond");
        for _ in 0..20 {
            e.vim_normal_key(KeyCode::Char('l'), KeyModifiers::NONE);
        }
        let (buf, cur) = e.view();
        assert_eq!(&buf[..cur], "hello world");
        e.vim_normal_key(KeyCode::Char('^'), KeyModifiers::NONE);
        assert_eq!(e.view().1, 0);
        e.vim_normal_key(KeyCode::Char('w'), KeyModifiers::NONE);
        assert_eq!(e.view().1, 6);
        e.vim_normal_key(KeyCode::Char('$'), KeyModifiers::NONE);
        assert_eq!(e.view().1, 11);
        e.vim_normal_key(KeyCode::Char('G'), KeyModifiers::NONE);
        let total = e.view().0.chars().count();
        assert_eq!(e.view().1, total);
        e.vim_normal_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(e.vim_normal_key(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(e.view().1, 0);
        // j/k navigate lines preserving column ('second' starts at 12).
        e.vim_normal_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(e.view().1, 12, "j lands at column 0 of 'second'");
        for _ in 0..4 {
            e.vim_normal_key(KeyCode::Char('l'), KeyModifiers::NONE);
        }
        e.vim_normal_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(e.view().1, 4, "k restores column 4 on 'hello world'");
    }

    #[test]
    fn x_dd_dw_delete_with_yank_paste() {
        let mut e = vim_editor("alpha beta gamma");
        assert!(e.vim_normal_key(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(e.view().0, "lpha beta gamma");

        e.set_text("one two\nthree");
        e.vim_normal_key(KeyCode::Char('g'), KeyModifiers::NONE);
        e.vim_normal_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(e.vim_normal_key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(e.vim_normal_key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(e.view().0, "three");

        // p pastes the linewise yank below.
        assert!(e.vim_normal_key(KeyCode::Char('p'), KeyModifiers::NONE));
        assert_eq!(e.view().0, "three\none two");

        e.set_text("kill this keep");
        e.vim_normal_key(KeyCode::Char('g'), KeyModifiers::NONE);
        e.vim_normal_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(e.vim_normal_key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(e.vim_normal_key(KeyCode::Char('w'), KeyModifiers::NONE));
        assert_eq!(e.view().0, "this keep");
        assert!(e.vim_normal_key(KeyCode::Char('P'), KeyModifiers::NONE));
        assert_eq!(e.view().0, "kill this keep");
    }

    #[test]
    fn cc_yanks_line_and_enters_insert() {
        let mut e = vim_editor("replace me\nkeep");
        assert!(e.vim_normal_key(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(e.vim_normal_key(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(e.vim_state(), VimState::Insert);
        type_insert(&mut e, "new line");
        assert_eq!(e.view().0, "new line\nkeep");
        // Esc returns to normal and steps back one char.
        e.vim_normal_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(e.vim_state(), VimState::Normal);
        assert_eq!(e.view().1, "new line".chars().count() - 1);
    }

    #[test]
    fn insert_entry_points_and_open_lines() {
        let mut e = vim_editor("abc");
        assert!(e.vim_normal_key(KeyCode::Char('A'), KeyModifiers::NONE));
        assert_eq!(e.vim_state(), VimState::Insert);
        type_insert(&mut e, "-tail");
        assert_eq!(e.view().0, "abc-tail");

        let mut e2 = vim_editor("first\nlast");
        assert!(e2.vim_normal_key(KeyCode::Char('$'), KeyModifiers::NONE));
        assert!(e2.vim_normal_key(KeyCode::Char('o'), KeyModifiers::NONE));
        type_insert(&mut e2, "middle");
        assert_eq!(e2.view().0, "first\nmiddle\nlast");

        let mut e3 = vim_editor("first");
        assert!(e3.vim_normal_key(KeyCode::Char('O'), KeyModifiers::NONE));
        type_insert(&mut e3, "zeroth");
        assert_eq!(e3.view().0, "zeroth\nfirst");
    }

    #[test]
    fn unmapped_keys_fall_through_to_keymap() {
        let mut e = vim_editor("");
        assert!(!e.vim_normal_key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!e.vim_normal_key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!e.vim_normal_key(KeyCode::Up, KeyModifiers::NONE));
        // Emacs mode never consumes.
        let mut em = Editor::new();
        em.set_text("x");
        assert!(!em.vim_normal_key(KeyCode::Char('x'), KeyModifiers::NONE));
    }

    #[test]
    fn u_undoes_and_dollar_d_deletes_to_line_end() {
        // D at end-of-line is a no-op.
        let mut e = vim_editor("keep cut");
        e.vim_normal_key(KeyCode::Char('$'), KeyModifiers::NONE);
        e.vim_normal_key(KeyCode::Char('D'), KeyModifiers::NONE);
        assert_eq!(e.view().0, "keep cut");
        e.vim_normal_key(KeyCode::Char('u'), KeyModifiers::NONE);
        assert_eq!(e.view().0, "keep cut");
        // Mid-line D trims the tail. Vim's w skips the trailing space, so
        // the space survives — same as real vim.
        let mut e2 = vim_editor("head tail");
        e2.vim_normal_key(KeyCode::Char('w'), KeyModifiers::NONE);
        e2.vim_normal_key(KeyCode::Char('D'), KeyModifiers::NONE);
        assert_eq!(e2.view().0, "head ");
    }
}
