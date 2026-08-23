const HISTORY_MAX: usize = 500;
const UNDO_MAX: usize = 200;

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
        let out = std::mem::take(&mut self.buf);
        self.cursor = 0;
        self.history_idx = None;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
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
        self.record_before(EditKind::Other);
        for c in s.chars() {
            let byte = self.byte_of_char(self.cursor);
            self.buf.insert(byte, c);
            self.cursor += 1;
        }
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
