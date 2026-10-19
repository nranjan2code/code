use similar::{ChangeTag, TextDiff};

/// How a text file changed: its unified hunks (three lines of context) and
/// the lines added and removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextChange {
    pub hunks: String,
    pub hunk_count: usize,
    pub additions: u64,
    pub deletions: u64,
}

impl TextChange {
    pub fn is_empty(&self) -> bool {
        self.hunk_count == 0
    }
}

/// The change from `before` to `after`, line by line.
pub fn text_change(before: &str, after: &str) -> TextChange {
    let diff = TextDiff::from_lines(before, after);
    let mut change = TextChange::default();
    for hunk in diff.unified_diff().iter_hunks() {
        change.hunk_count += 1;
        change.hunks.push_str(&format!("{hunk}\n"));
    }
    for line in diff.iter_all_changes() {
        match line.tag() {
            ChangeTag::Insert => change.additions += 1,
            ChangeTag::Delete => change.deletions += 1,
            ChangeTag::Equal => {}
        }
    }
    change
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_is_all_additions() {
        let change = text_change("", "a\nb\n");
        assert_eq!((change.additions, change.deletions), (2, 0));
        assert!(change.hunks.contains("+a"));
        assert_eq!(change.hunk_count, 1);
    }

    #[test]
    fn an_edit_counts_what_moved_and_nothing_else() {
        let change = text_change("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!((change.additions, change.deletions), (1, 1));
        assert!(change.hunks.contains("-b") && change.hunks.contains("+B"));
    }

    #[test]
    fn the_same_text_is_no_change() {
        assert!(text_change("a\n", "a\n").is_empty());
    }
}
