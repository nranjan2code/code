#[derive(Debug, Default, Clone)]
pub struct Editor {
    buf: String,
    cursor: usize,
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: Option<String>,
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

    fn set_text(&mut self, text: &str) {
        self.buf = text.to_string();
        self.cursor = text.chars().count();
    }

    pub fn view(&self) -> (&str, usize) {
        (&self.buf, self.cursor)
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
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
