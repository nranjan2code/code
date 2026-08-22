const HISTORY_MAX: usize = 500;

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
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, c: char) {
        let byte = self.byte_of_char(self.cursor);
        self.buf.insert(byte, c);
        self.cursor += 1;
        self.history_idx = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
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
        let next = self.byte_of_char(self.cursor + 1);
        self.buf.replace_range(byte..next, "");
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.chars().count() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.chars().count();
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.history_idx = None;
    }

    /// Deletes back to the start of the previous word.
    pub fn delete_word_back(&mut self) {
        let target = self.word_start();
        let byte = self.byte_of_char(self.cursor);
        let prev = self.byte_of_char(target);
        self.buf.replace_range(prev..byte, "");
        self.cursor = target;
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
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
        if !out.trim().is_empty() {
            self.history.push(out.clone());
        }
        out
    }

    pub fn history_prev(&mut self) {
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
        for c in s.chars() {
            self.insert(c);
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
}
