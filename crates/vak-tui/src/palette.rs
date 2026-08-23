#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Default)]
pub struct CommandPalette {
    query: String,
    selected: usize,
    /// Custom/plugin-contributed actions merged with built-in commands.
    extras: Vec<PaletteItem>,
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
    pub fn new(extras: Vec<PaletteItem>) -> Self {
        Self {
            extras,
            ..Self::default()
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
                fuzzy_score(name, &self.query).map(|score| {
                    (
                        score,
                        PaletteItem {
                            name: (*name).to_string(),
                            description: (*description).to_string(),
                        },
                    )
                })
            })
            .chain(self.extras.iter().filter_map(|item| {
                fuzzy_score(&item.name, &self.query).map(|score| (score + 1, item.clone()))
            }))
            .collect();
        ranked.sort_by_key(|(score, item)| (*score, item.name.clone()));
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn extras() -> Vec<PaletteItem> {
        vec![PaletteItem {
            name: "review".to_string(),
            description: "project review command".to_string(),
        }]
    }

    #[test]
    fn extras_merge_with_builtins_and_fuzzy_filter() {
        let mut p = CommandPalette::new(extras());
        for c in "hel".chars() {
            p.push(c);
        }
        let names: Vec<String> = p.items().iter().map(|i| i.name.clone()).collect();
        assert_eq!(names.first().map(String::as_str), Some("help"));

        p.backspace();
        p.backspace();
        p.backspace();
        for c in "rev".chars() {
            p.push(c);
        }
        let items = p.items();
        assert!(
            items.iter().any(|i| i.name == "review"),
            "custom command surfaces in the palette: {items:?}"
        );
        for _ in 0..3 {
            p.backspace();
        }
        p.push('z');
        p.push('z');
        assert!(p.items().is_empty());
    }

    #[test]
    fn builtin_commands_rank_ahead_of_equal_extras() {
        let mut p = CommandPalette::new(extras());
        // "review" fuzzy-matches "review" exactly; a builtin prefix match on
        // the same query would outrank it.
        for c in "resume".chars() {
            p.push(c);
        }
        let names: Vec<String> = p.items().iter().map(|i| i.name.clone()).collect();
        assert_eq!(names.first().map(String::as_str), Some("resume"));
    }
}
