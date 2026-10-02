//! How much of one tool result a request carries
//! (docs/design/68-context-engine.md §3).
//!
//! A tool returns its whole result and the agent loop records it whole in the
//! ledger; a request carries a result verbatim up to [`RESULT_WINDOW_CHARS`].
//! Past that, the request carries a *window*: whole lines from the start and
//! the end, and one line in place of the rest that says exactly which lines
//! were left out, how many characters they hold, and the `recall` call that
//! returns them. Inside a window, a line longer than [`LINE_WINDOW_CHARS`]
//! shows its start and says how many characters it continues for. Nothing is
//! left out without the request saying so, and nothing is lost from the
//! ledger.

/// Most characters of one result a request carries verbatim.
pub const RESULT_WINDOW_CHARS: usize = 30_000;
/// Most characters of one line a window shows. A result that fits whole is
/// never windowed, however long its lines.
pub const LINE_WINDOW_CHARS: usize = 2_000;

/// The window a request carries for `content`, or `None` when it fits whole.
///
/// `recall_id` is the evidence id the omitted lines can be recalled by (the
/// call's `tool_use_id`); without one the note only states what was left out.
/// `first_line` is the 1-based number of `content`'s first line within that
/// evidence, so a window over a recalled range names the evidence's own line
/// numbers.
pub fn window(content: &str, recall_id: Option<&str>, first_line: usize) -> Option<String> {
    if content.chars().count() <= RESULT_WINDOW_CHARS {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    let first_line = first_line.max(1);
    // Where each line starts in the whole result, so a cut line can name the
    // character range that returns the rest of it. Only meaningful when
    // `content` is the evidence from its first line.
    let mut offset = 0usize;
    let shown: Vec<String> = lines
        .iter()
        .map(|line| {
            let chars = line.chars().count();
            let start = offset;
            offset += chars + 1;
            if chars <= LINE_WINDOW_CHARS {
                (*line).to_string()
            } else {
                let kept: String = line.chars().take(LINE_WINDOW_CHARS).collect();
                let hint = match recall_id {
                    Some(id) if first_line == 1 => format!(
                        "; recall {{\"id\": \"{id}\", \"chars\": {{\"start\": {}, \"end\": {}}}}} returns it",
                        start + LINE_WINDOW_CHARS,
                        start + chars
                    ),
                    _ => String::new(),
                };
                format!(
                    "{kept} [line continues for {} more chars{hint}]",
                    chars - LINE_WINDOW_CHARS
                )
            }
        })
        .collect();
    let cost = |line: &String| line.chars().count() + 1;

    let head_budget = RESULT_WINDOW_CHARS * 2 / 5;
    let tail_budget = RESULT_WINDOW_CHARS - head_budget;
    let mut head_end = 0;
    let mut used = 0;
    while head_end < shown.len() && used + cost(&shown[head_end]) <= head_budget {
        used += cost(&shown[head_end]);
        head_end += 1;
    }
    let mut tail_start = shown.len();
    used = 0;
    while tail_start > head_end && used + cost(&shown[tail_start - 1]) <= tail_budget {
        used += cost(&shown[tail_start - 1]);
        tail_start -= 1;
    }

    let mut out: Vec<String> = shown[..head_end].to_vec();
    if head_end < tail_start {
        let omitted_chars: usize = lines[head_end..tail_start]
            .iter()
            .map(|line| line.chars().count() + 1)
            .sum();
        let start = first_line + head_end;
        let end = first_line + tail_start - 1;
        let span = if start == end {
            format!("line {start}")
        } else {
            format!("lines {start}\u{2013}{end}")
        };
        let note = match recall_id {
            Some(id) => format!(
                "[{span} omitted ({omitted_chars} chars); recall {{\"id\": \"{id}\", \"range\": {{\"start\": {start}, \"end\": {end}}}}} returns them]"
            ),
            None => format!("[{span} omitted ({omitted_chars} chars)]"),
        };
        out.push(note);
    }
    out.extend(shown[tail_start..].iter().cloned());
    Some(out.join("\n"))
}

/// `content` as a caller with no ledger to keep the whole in passes it on
/// (a flow node's output, a scheduled script's report): whole, or its window
/// when over-long.
pub fn bounded(content: String) -> String {
    window(&content, None, 1).unwrap_or(content)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_result_that_fits_is_not_windowed() {
        assert_eq!(window("one\ntwo", Some("call-1"), 1), None);
    }

    #[test]
    fn an_over_long_result_keeps_whole_lines_and_names_what_it_left_out() {
        let content: String = (1..=5000)
            .map(|n| format!("line number {n:05}\n"))
            .collect();
        let windowed = window(&content, Some("call-7"), 1).expect("windowed");
        assert!(windowed.chars().count() <= RESULT_WINDOW_CHARS + 200);
        assert!(windowed.starts_with("line number 00001\n"));
        assert!(windowed.ends_with("line number 05000"));
        let note = windowed
            .lines()
            .find(|line| line.starts_with("[lines "))
            .expect("omission note");
        assert!(note.contains("recall {\"id\": \"call-7\""), "{note}");
        for line in windowed.lines().filter(|line| !line.starts_with('[')) {
            assert!(line.starts_with("line number "), "cut line: {line}");
            assert_eq!(line.len(), "line number 00000".len());
        }
    }

    #[test]
    fn the_omitted_range_is_numbered_within_the_evidence() {
        let content: String = (1..=5000).map(|n| format!("row {n:05}\n")).collect();
        let windowed = window(&content, Some("call-2"), 101).expect("windowed");
        let note = windowed
            .lines()
            .find(|line| line.starts_with("[lines "))
            .expect("omission note");
        let shown_head = windowed
            .lines()
            .take_while(|line| !line.starts_with('['))
            .count();
        assert!(
            note.starts_with(&format!("[lines {}\u{2013}", 101 + shown_head)),
            "{note}"
        );
    }

    #[test]
    fn a_long_line_that_fits_is_carried_whole() {
        let long = "x".repeat(LINE_WINDOW_CHARS * 5);
        assert_eq!(window(&long, Some("call-3"), 1), None);
    }

    #[test]
    fn inside_a_window_a_long_line_says_how_much_it_continues_for() {
        let long = "x".repeat(RESULT_WINDOW_CHARS);
        let windowed = window(&format!("head\n{long}\ntail"), None, 1).expect("windowed");
        let continues = RESULT_WINDOW_CHARS - LINE_WINDOW_CHARS;
        assert!(windowed.contains(&format!("[line continues for {continues} more chars]")));
        assert!(windowed.starts_with("head\n") && windowed.ends_with("\ntail"));
    }

    #[test]
    fn a_cut_line_names_the_character_range_that_returns_the_rest() {
        let long = "x".repeat(RESULT_WINDOW_CHARS);
        let windowed = window(&format!("head\n{long}\ntail"), Some("call-9"), 1).expect("windowed");
        let start = "head\n".len() + LINE_WINDOW_CHARS;
        let end = "head\n".len() + RESULT_WINDOW_CHARS;
        assert!(
            windowed.contains(&format!(
                "recall {{\"id\": \"call-9\", \"chars\": {{\"start\": {start}, \"end\": {end}}}}} returns it"
            )),
            "{}",
            &windowed[..windowed.len().min(2_300)]
        );
    }

    #[test]
    fn multibyte_content_is_counted_in_chars() {
        let content = "é".repeat(RESULT_WINDOW_CHARS);
        let lines: String = content
            .chars()
            .collect::<Vec<_>>()
            .chunks(100)
            .map(|chunk| chunk.iter().collect::<String>() + "\n")
            .collect();
        let windowed = window(&lines, None, 1).expect("windowed");
        assert!(windowed.chars().count() <= RESULT_WINDOW_CHARS + 200);
    }
}
