#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Default)]
pub struct CommandPalette {
    query: String,
    selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceItem {
    pub value: String,
    pub description: String,
    pub active: bool,
}

#[derive(Debug)]
pub struct ChoicePicker {
    query: String,
    selected: usize,
    choices: Vec<ChoiceItem>,
    allow_custom: bool,
}

impl ChoicePicker {
    pub fn new(choices: Vec<ChoiceItem>, allow_custom: bool) -> Self {
        let selected = choices
            .iter()
            .position(|choice| choice.active)
            .unwrap_or_default();
        Self {
            query: String::new(),
            selected,
            choices,
            allow_custom,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn filtered(&self) -> Vec<&ChoiceItem> {
        let query = self.query.to_ascii_lowercase();
        self.choices
            .iter()
            .filter(|choice| {
                query.is_empty()
                    || choice.value.to_ascii_lowercase().contains(&query)
                    || choice.description.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }

    pub fn up(&mut self) {
        let count = self.filtered().len();
        if count > 0 {
            self.selected = self.selected.checked_sub(1).unwrap_or(count - 1);
        }
    }

    pub fn down(&mut self) {
        let count = self.filtered().len();
        if count > 0 {
            self.selected = (self.selected + 1) % count;
        }
    }

    pub fn selected(&self) -> usize {
        self.selected.min(self.filtered().len().saturating_sub(1))
    }

    pub fn value(&self) -> Option<String> {
        let filtered = self.filtered();
        if let Some(choice) = filtered.get(self.selected()) {
            return Some(choice.value.clone());
        }
        (self.allow_custom && !self.query.trim().is_empty()).then(|| self.query.trim().to_string())
    }
}

impl CommandPalette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn up(&mut self, count: usize) {
        if count > 0 {
            self.selected = self.selected.checked_sub(1).unwrap_or(count - 1);
        }
    }

    pub fn down(&mut self, count: usize) {
        if count > 0 {
            self.selected = (self.selected + 1) % count;
        }
    }

    pub fn selected(&self, count: usize) -> usize {
        self.selected.min(count.saturating_sub(1))
    }

    pub fn items(&self) -> Vec<PaletteItem> {
        let mut ranked: Vec<(usize, PaletteItem)> = crate::commands::COMMANDS
            .iter()
            .filter_map(|(name, description)| {
                fuzzy_score(name, &self.query)
                    .map(|score| (score, PaletteItem { name, description }))
            })
            .collect();
        ranked.sort_by_key(|(score, item)| (*score, item.name));
        ranked.into_iter().map(|(_, item)| item).take(8).collect()
    }
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let candidate = candidate.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    if candidate.starts_with(&query) {
        return Some(candidate.len().saturating_sub(query.len()));
    }
    let mut offset = 0usize;
    let mut score = candidate.len();
    for needle in query.chars() {
        let found = candidate[offset..].find(needle)?;
        score += found;
        offset += found + needle.len_utf8();
    }
    Some(score + 100)
}
