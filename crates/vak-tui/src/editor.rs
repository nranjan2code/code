const HISTORY_MAX: usize = 500;
const UNDO_MAX: usize = 200;

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
}
