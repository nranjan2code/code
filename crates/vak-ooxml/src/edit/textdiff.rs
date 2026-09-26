//! Word-level comparison for a redline that marks only what changed.
//!
//! Text is compared as tokens (a word, one whitespace character, or one
//! other character), so a change never splits a word. The changes are then
//! cleaned the way a person reads a redline: an unchanged stretch no longer
//! than the changes on both sides of it is folded into one change, so a
//! rewritten sentence reads as one change rather than as confetti.

use std::ops::Range;

/// Old characters `delete` give way to new characters `insert` (char
/// indices into each text). Either may be empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub delete: Range<usize>,
    pub insert: Range<usize>,
}

/// Above this many token edits the two texts are compared as one change:
/// past it a word-by-word redline is noise, and the comparison's memory
/// grows with the square of the edits.
const MAX_EDITS: usize = 1_000;

/// The changes that turn `old` into `new`, in order and never touching.
/// `absorbable(range)` says whether an unchanged stretch of `old` may be
/// folded into the changes around it; a stretch that must stay where it is
/// (a field's result, a footnote mark between its words) is never folded.
pub(crate) fn changes(
    old: &[char],
    new: &[char],
    absorbable: impl Fn(Range<usize>) -> bool,
) -> Vec<Change> {
    let old_tokens = tokens(old);
    let new_tokens = tokens(new);
    let old_keys: Vec<String> = old_tokens.iter().map(|range| key(old, range)).collect();
    let new_keys: Vec<String> = new_tokens.iter().map(|range| key(new, range)).collect();

    let mut prefix = 0;
    while prefix < old_keys.len() && prefix < new_keys.len() && old_keys[prefix] == new_keys[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_keys.len() - prefix
        && suffix < new_keys.len() - prefix
        && old_keys[old_keys.len() - 1 - suffix] == new_keys[new_keys.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_middle = &old_keys[prefix..old_keys.len() - suffix];
    let new_middle = &new_keys[prefix..new_keys.len() - suffix];

    // Token-index regions: (old tokens, new tokens) that differ.
    let mut regions: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    match myers(old_middle, new_middle) {
        Some(steps) => {
            let (mut a, mut b) = (prefix, prefix);
            let mut open: Option<(usize, usize)> = None;
            for step in steps {
                match step {
                    Step::Equal => {
                        if let Some((a_start, b_start)) = open.take() {
                            regions.push((a_start..a, b_start..b));
                        }
                        a += 1;
                        b += 1;
                    }
                    Step::Delete => {
                        open.get_or_insert((a, b));
                        a += 1;
                    }
                    Step::Insert => {
                        open.get_or_insert((a, b));
                        b += 1;
                    }
                }
            }
            if let Some((a_start, b_start)) = open {
                regions.push((a_start..a, b_start..b));
            }
        }
        None => regions.push((
            prefix..old_keys.len() - suffix,
            prefix..new_keys.len() - suffix,
        )),
    }

    let at = |tokens: &[Range<usize>], len: usize, index: usize| {
        tokens.get(index).map(|token| token.start).unwrap_or(len)
    };
    let mut changes: Vec<Change> = regions
        .into_iter()
        .filter(|(a, b)| !a.is_empty() || !b.is_empty())
        .map(|(a, b)| Change {
            delete: at(&old_tokens, old.len(), a.start)..at(&old_tokens, old.len(), a.end),
            insert: at(&new_tokens, new.len(), b.start)..at(&new_tokens, new.len(), b.end),
        })
        .collect();
    fold_short_equalities(&mut changes, absorbable);
    changes
}

/// Folds an unchanged stretch between two changes into them when it is no
/// longer than the larger side of each (the semantic clean-up of Myers'
/// diff as diff-match-patch does it), until nothing more folds.
fn fold_short_equalities(changes: &mut Vec<Change>, absorbable: impl Fn(Range<usize>) -> bool) {
    loop {
        let mut folded = false;
        for index in 1..changes.len() {
            let (before, after) = (&changes[index - 1], &changes[index]);
            let equal = before.delete.end..after.delete.start;
            let length = equal.len();
            if length <= before.delete.len().max(before.insert.len())
                && length <= after.delete.len().max(after.insert.len())
                && absorbable(equal)
            {
                let merged = Change {
                    delete: before.delete.start..after.delete.end,
                    insert: before.insert.start..after.insert.end,
                };
                changes[index - 1] = merged;
                changes.remove(index);
                folded = true;
                break;
            }
        }
        if !folded {
            return;
        }
    }
}

/// Words (letters and digits, joined across `.`, `,`, apostrophes and
/// hyphens between them), single whitespace characters, and single other
/// characters. Scripts written without spaces go a character at a time.
fn tokens(text: &[char]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let start = index;
        if is_word(text[index]) {
            index += 1;
            loop {
                match text.get(index) {
                    Some(next) if is_word(*next) => index += 1,
                    Some(joiner)
                        if is_joiner(*joiner)
                            && text.get(index + 1).is_some_and(|after| is_word(*after)) =>
                    {
                        index += 2;
                    }
                    _ => break,
                }
            }
        } else {
            index += 1;
        }
        out.push(start..index);
    }
    out
}

fn is_word(character: char) -> bool {
    (character.is_alphanumeric() || character == '_') && !is_unspaced(character)
}

/// Scripts written without spaces between words.
fn is_unspaced(character: char) -> bool {
    matches!(
        u32::from(character),
        0x0E00..=0x0EFF // Thai, Lao
            | 0x1000..=0x109F // Myanmar
            | 0x1780..=0x17FF // Khmer
            | 0x3040..=0x30FF // Hiragana, Katakana
            | 0x3400..=0x4DBF // CJK extension A
            | 0x4E00..=0x9FFF // CJK unified ideographs
            | 0xF900..=0xFAFF // CJK compatibility ideographs
            | 0x20000..=0x2FA1F // CJK extensions B onwards
    )
}

fn is_joiner(character: char) -> bool {
    matches!(character, '.' | ',' | '\'' | '’' | '-' | '‐' | '‑')
}

fn key(text: &[char], range: &Range<usize>) -> String {
    text[range.clone()]
        .iter()
        .map(|character| fold(*character))
        .collect()
}

/// The character compared in place of `character`. A line break the model
/// writes compares equal to the space the reader shows for one, and
/// typographic quotes, no-break spaces and hyphens compare equal to their
/// plain forms, so retyping a curly apostrophe is not a change. The
/// document keeps its own character wherever the two compare equal.
pub(crate) fn fold(character: char) -> char {
    match character {
        '\n' | '\r' | '\u{a0}' | '\u{2007}' | '\u{202f}' => ' ',
        '‘' | '’' | '‚' | '‛' | '′' => '\'',
        '“' | '”' | '„' | '‟' | '″' => '"',
        '‐' | '‑' => '-',
        other => other,
    }
}

/// `text` with every character [`fold`]ed.
pub(crate) fn fold_text(text: &str) -> String {
    text.chars().map(fold).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Equal,
    Delete,
    Insert,
}

/// Myers' O((N+M)D) shortest edit script. `None` when more than
/// [`MAX_EDITS`] edits are needed.
fn myers(a: &[String], b: &[String]) -> Option<Vec<Step>> {
    let (n, m) = (a.len(), b.len());
    let max = n + m;
    let offset = max + 1;
    let mut v = vec![0usize; 2 * max + 3];
    // `trace[d]` is v on diagonals -d-1..=d+1 as step d found it.
    let mut trace: Vec<Vec<usize>> = Vec::new();
    for d in 0..=max.min(MAX_EDITS) {
        trace.push(v[offset - d - 1..=offset + d + 1].to_vec());
        for step in 0..=d {
            let slot = offset - d + 2 * step;
            let k = slot as isize - offset as isize;
            let mut x = if step == 0 || (step != d && v[slot - 1] < v[slot + 1]) {
                v[slot + 1]
            } else {
                v[slot - 1] + 1
            };
            let mut y = usize::try_from(x as isize - k).ok()?;
            while x < n && y < m && a[x] == b[y] {
                x += 1;
                y += 1;
            }
            v[slot] = x;
            if x >= n && y >= m {
                return backtrack(&trace, n, m, d);
            }
        }
    }
    None
}

fn backtrack(trace: &[Vec<usize>], n: usize, m: usize, last: usize) -> Option<Vec<Step>> {
    let mut steps = Vec::new();
    let (mut x, mut y) = (n as isize, m as isize);
    for d in (0..=last).rev() {
        let window = trace.get(d)?;
        let d = d as isize;
        let at = |k: isize| -> Option<isize> {
            window
                .get(usize::try_from(k + d + 1).ok()?)
                .map(|value| *value as isize)
        };
        let k = x - y;
        let previous_k = if k == -d || (k != d && at(k - 1)? < at(k + 1)?) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = at(previous_k)?;
        let previous_y = previous_x - previous_k;
        while x > previous_x && y > previous_y {
            steps.push(Step::Equal);
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            steps.push(if x == previous_x {
                Step::Insert
            } else {
                Step::Delete
            });
        }
        x = previous_x;
        y = previous_y;
    }
    steps.reverse();
    Some(steps)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    /// The changes as (deleted, inserted) strings.
    fn diff(old: &str, new: &str) -> Vec<(String, String)> {
        let (old, new) = (chars(old), chars(new));
        changes(&old, &new, |_| true)
            .into_iter()
            .map(|change| {
                (
                    old[change.delete].iter().collect(),
                    new[change.insert].iter().collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_changed_word_is_the_only_change() {
        assert_eq!(
            diff(
                "continues for twelve months, see the schedule.",
                "continues for twenty-four months, see the schedule."
            ),
            vec![("twelve".to_string(), "twenty-four".to_string())]
        );
    }

    #[test]
    fn words_are_never_split() {
        assert_eq!(
            diff("for twelve months", "for twenty months"),
            vec![("twelve".to_string(), "twenty".to_string())]
        );
        assert_eq!(
            diff("Fees of 1,250.00 apply", "Fees of 1,300.00 apply"),
            vec![("1,250.00".to_string(), "1,300.00".to_string())]
        );
        // Neighbouring changed words read as one change.
        assert_eq!(
            diff("the schedule applies", "the schedules apply"),
            vec![(
                "schedule applies".to_string(),
                "schedules apply".to_string()
            )]
        );
    }

    #[test]
    fn a_short_unchanged_stretch_between_changes_is_folded_in() {
        assert_eq!(
            diff("the Effective Date applies", "the Commencement Day applies"),
            vec![("Effective Date".to_string(), "Commencement Day".to_string())]
        );
        // A long unchanged stretch keeps the changes apart.
        assert_eq!(
            diff(
                "Alpha says the quarterly figures are final. Beta",
                "Gamma says the quarterly figures are final. Delta"
            ),
            vec![
                ("Alpha".to_string(), "Gamma".to_string()),
                ("Beta".to_string(), "Delta".to_string())
            ]
        );
    }

    #[test]
    fn a_stretch_that_must_stay_is_never_folded() {
        let (old, new) = (
            chars("the Effective Date applies"),
            chars("the Commencement Day applies"),
        );
        let changes = changes(&old, &new, |_| false);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn insertions_and_deletions_alone() {
        assert_eq!(
            diff("payable within 30 days", "payable within 30 business days"),
            vec![(String::new(), "business ".to_string())]
        );
        assert_eq!(
            diff("payable within 30 business days", "payable within 30 days"),
            vec![("business ".to_string(), String::new())]
        );
        assert_eq!(
            diff("", "New text"),
            vec![(String::new(), "New text".to_string())]
        );
        assert_eq!(
            diff("Old text", ""),
            vec![("Old text".to_string(), String::new())]
        );
        assert!(diff("Same", "Same").is_empty());
        assert!(diff("", "").is_empty());
    }

    #[test]
    fn typographic_forms_compare_equal() {
        assert!(diff("the Supplier’s “rights”", "the Supplier's \"rights\"").is_empty());
        assert!(diff("Section\u{a0}4.2", "Section 4.2").is_empty());
        assert!(diff("one two", "one\ntwo").is_empty());
    }

    #[test]
    fn unspaced_scripts_compare_a_character_at_a_time() {
        assert_eq!(
            diff("本契約は東京で締結", "本契約は大阪で締結"),
            vec![("東京".to_string(), "大阪".to_string())]
        );
    }

    #[test]
    fn a_rewrite_reads_as_one_change() {
        let changes = diff(
            "The supplier shall deliver the goods within ten days of the order.",
            "Delivery is due no later than two weeks after each purchase order is placed.",
        );
        assert_eq!(changes.len(), 1, "{changes:?}");
    }

    #[test]
    fn beyond_the_edit_bound_the_texts_are_one_change() {
        let old: String = (0..1_500).map(|index| format!("a{index} ")).collect();
        let new: String = (0..1_500).map(|index| format!("b{index} ")).collect();
        let changes = diff(&old, &new);
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn edits_far_apart_in_a_long_text_stay_small() {
        let body: String = (0..2_000).map(|index| format!("w{index} ")).collect();
        let old = format!("First {body}last");
        let new = format!("Initial {body}final");
        assert_eq!(
            diff(&old, &new),
            vec![
                ("First".to_string(), "Initial".to_string()),
                ("last".to_string(), "final".to_string())
            ]
        );
    }
}
