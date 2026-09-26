//! The vocabulary of runtime control traffic: model-visible text the runtime
//! itself authors, as opposed to text a user or the model wrote.
//!
//! Every layer that needs to tell "the user said this" from "the runtime said
//! this" — session projection, the desktop and admin clients, channel
//! delivery, compaction — reads this one module. Before it existed each layer
//! kept its own hand-copied list of text prefixes, and the lists drifted: a
//! nudge added to the agent loop was hidden by the server but shown by the
//! client as a message from the user, and the client's turn count then
//! disagreed with the server's, displacing every later turn.
//!
//! Three classes, each with a different lifetime:
//!
//! * [`ControlKind`] — a synthetic *user-role message the agent loop appends to
//!   the ledger* (repair nudges, stop guards). Persisted, and tagged
//!   structurally at creation (`MessageMeta::control`). That tag is the only
//!   way anything recognises one: there is no text sniffing to drift.
//! * [`CONTEXT_BLOCK_TAGS`] — a `<tag>…</tag>` block the runtime *derives into a
//!   message* when it assembles model input (compaction summary, intent note,
//!   work contract, conversation thread). Never persisted as its own message.
//! * [`InlineHint`] — a marker *line inside other text* (a tool result, a stop
//!   guard's reason). Not a message, so it is stripped line by line.

use serde::{Deserialize, Serialize};

/// A synthetic user-role message the agent loop appends to the ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// A stop hook blocked the turn from ending.
    StopHook,
    /// The stop gate (verification, goal, managed work) blocked completion.
    StopGuard,
    /// A search/fetch result was ignored by the answer that followed it.
    GroundingCheck,
    /// A `vak` fence in the answer did not parse.
    FenceCheck,
    /// The answer restated a card that a tool call had already shown.
    DuplicateCardCheck,
    /// The answer reads as a card but was written as prose.
    PresentationCheck,
    /// A current value was asked for and nothing was retrieved this turn
    /// (docs/design/68-context-engine.md §7).
    FreshnessCheck,
    /// The response carried neither text nor a tool call (a thinking-only
    /// completion): act on the plan, or answer.
    EmptyStep,
    /// Model drift (docs/design/68-context-engine.md §7): the step served a
    /// different directive than the current one — a mismatched-domain tool
    /// call, or a verbatim repeat of a past answer.
    SteeringDrift,
    /// An `emit_*_card` call after a successful retrieval THIS run whose
    /// own payload shares no topic word with either the directive or what
    /// was just retrieved — never checked when nothing was retrieved this
    /// run, since a card built from the model's own reasoning or from data
    /// already in the directive routinely has no vocabulary overlap with
    /// either and would otherwise be gated for being right
    /// (docs/design/68-context-engine.md §7).
    TopicMismatchCheck,
    /// Correctable tool failures went unrepaired across steps: the admitted
    /// schema of each failing tool, re-surfaced with the retries left.
    RepairDirective,
}

impl ControlKind {
    pub const ALL: [ControlKind; 11] = [
        ControlKind::StopHook,
        ControlKind::StopGuard,
        ControlKind::GroundingCheck,
        ControlKind::FenceCheck,
        ControlKind::DuplicateCardCheck,
        ControlKind::PresentationCheck,
        ControlKind::FreshnessCheck,
        ControlKind::EmptyStep,
        ControlKind::SteeringDrift,
        ControlKind::TopicMismatchCheck,
        ControlKind::RepairDirective,
    ];

    /// The literal the message body begins with, for the model's benefit.
    /// Nothing else reads it: consumers use the structural tag.
    pub const fn marker(self) -> &'static str {
        match self {
            ControlKind::StopHook => "[stop-hook]",
            ControlKind::StopGuard => "[stop-guard]",
            ControlKind::GroundingCheck => "[grounding-check]",
            ControlKind::FenceCheck => "[fence-check]",
            ControlKind::DuplicateCardCheck => "[duplicate-card-check]",
            ControlKind::PresentationCheck => "[presentation-check]",
            ControlKind::FreshnessCheck => "[freshness-check]",
            ControlKind::EmptyStep => "[empty-step]",
            ControlKind::SteeringDrift => "[steering-drift]",
            ControlKind::TopicMismatchCheck => "[topic-mismatch]",
            ControlKind::RepairDirective => "[repair-directive]",
        }
    }

    /// Whether this asks the model to redo the answer it just gave, so a card
    /// the model emits afterwards replaces the earlier attempt rather than
    /// adding a second one to the same answer.
    pub const fn retries_answer(self) -> bool {
        matches!(
            self,
            ControlKind::GroundingCheck
                | ControlKind::FenceCheck
                | ControlKind::DuplicateCardCheck
                | ControlKind::PresentationCheck
                | ControlKind::FreshnessCheck
                | ControlKind::EmptyStep
                | ControlKind::TopicMismatchCheck
        )
    }
}

/// A `<tag>…</tag>` block the runtime derives into a model-visible message.
pub const CONTEXT_BLOCK_TAGS: [&str; 10] = [
    "conversation_thread",
    "context_summary",
    "intent",
    "work_contract",
    "managed_work",
    "context_packet",
    "system_reminder",
    "runtime_guidance",
    "scratchpad",
    "workspace_delta",
];

/// A marker line inside other text (a tool result or a stop guard's reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InlineHint {
    Recovery,
    PostToolUseHook,
}

impl InlineHint {
    pub const ALL: [InlineHint; 2] = [InlineHint::Recovery, InlineHint::PostToolUseHook];

    pub const fn marker(self) -> &'static str {
        match self {
            InlineHint::Recovery => "[recovery]",
            InlineHint::PostToolUseHook => "[post-tool-use hook]",
        }
    }

    /// Everything from the marker to the end of the text is the hint (its
    /// body can run over several lines), as opposed to a hint that ends at
    /// "Please continue.".
    pub const fn runs_to_end(self) -> bool {
        matches!(self, InlineHint::Recovery)
    }
}

/// Whether one line is an inline runtime hint rather than conversation.
pub fn is_control_line(line: &str) -> bool {
    let trimmed = line.trim();
    InlineHint::ALL
        .into_iter()
        .any(|hint| trimmed.starts_with(hint.marker()))
}

/// Every inline marker a text-based recogniser (one working on message text
/// rather than the structural tag, such as the desktop client) must know. The
/// TypeScript side is checked against this list in `vak-server`'s tests.
pub fn inline_markers() -> Vec<&'static str> {
    InlineHint::ALL
        .into_iter()
        .map(InlineHint::marker)
        .collect()
}

/// Lines of runtime narration that are not conversation either: the surface
/// stamp, the outcome banner, and the like. Not control *messages* (they have
/// no kind), so they are matched by text alone.
const NARRATION_PREFIXES: [&str; 5] = [
    "Surface:",
    "Outcome:",
    "primary deliverable:",
    "contract_id:",
    "I will write and execute this within the sandbox",
];

/// Whether one line is scaffolding a reader should never see: an inline
/// runtime hint or runtime narration.
pub fn is_scaffolding_line(line: &str) -> bool {
    let trimmed = line.trim();
    NARRATION_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
        || trimmed.eq_ignore_ascii_case("completed")
        || trimmed.eq_ignore_ascii_case("vak")
        || is_control_line(trimmed)
}

/// Removes `<tag>…</tag>` context blocks (an unterminated one runs to the end
/// of the text) and the `[marker]: … Please continue.` spans that stop hooks,
/// stop guards and tool-result hints embed in other text.
pub fn strip_control_blocks(text: &str) -> String {
    let mut out = text.to_string();
    for tag in CONTEXT_BLOCK_TAGS {
        let close_pattern = format!("</{tag}>");
        while let Some(start) = find_open_tag(&out, tag) {
            if let Some(end_offset) = out[start..].find(&close_pattern) {
                let end = start + end_offset + close_pattern.len();
                out.replace_range(start..end, "");
            } else {
                out.truncate(start);
                break;
            }
        }
    }

    // (marker, runs to the end of the text). Persisted control messages are
    // never embedded in other text, so only the inline hints appear here.
    let embedded: [(String, bool); 2] = [
        (
            InlineHint::Recovery.marker().to_string(),
            InlineHint::Recovery.runs_to_end(),
        ),
        (format!("{}:", InlineHint::PostToolUseHook.marker()), false),
    ];
    for (prefix, runs_to_end) in &embedded {
        while let Some(start) = out.find(prefix.as_str()) {
            let remainder = &out[start..];
            if *runs_to_end {
                out.truncate(start);
                break;
            }
            if let Some(end_offset) = remainder.find("Please continue.") {
                let end = start + end_offset + "Please continue.".len();
                out.replace_range(start..end, "");
            } else if let Some(end_offset) = remainder.find("Please continue") {
                let end = start + end_offset + "Please continue".len();
                out.replace_range(start..end, "");
            } else if let Some(newline_offset) = remainder.find('\n') {
                let end = start + newline_offset + 1;
                out.replace_range(start..end, "");
            } else {
                out.truncate(start);
                break;
            }
        }
    }
    out
}

/// Position of the first `<tag>` / `<tag …>` opener, as a whole tag name.
///
/// A bare prefix match (`<intent`) also matched `<intentional>` and, finding
/// no `</intent>`, truncated the rest of the text.
fn find_open_tag(text: &str, tag: &str) -> Option<usize> {
    let prefix = format!("<{tag}");
    let mut from = 0;
    while let Some(offset) = text[from..].find(&prefix) {
        let start = from + offset;
        let after = text[start + prefix.len()..].chars().next();
        if matches!(after, Some('>') | Some('/')) || after.is_some_and(char::is_whitespace) {
            return Some(start);
        }
        from = start + prefix.len();
    }
    None
}

/// Whether a line opens or closes a fenced code block.
fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// The text with all scaffolding removed and outer blank lines trimmed.
///
/// Fenced code is content, whatever it contains, and passes through
/// verbatim: a block that shows an `<intent>` element, a line reading `vak`
/// or `Outcome: ok` is part of the answer, and removing it silently
/// corrupts code a person will copy. Only the prose between fences is
/// cleaned; an unterminated fence runs to the end of the text.
pub fn clean_scaffolding(text: &str) -> String {
    let had_trailing_newline = text.ends_with('\n');
    let mut lines: Vec<String> = Vec::new();
    let mut prose = String::new();
    let mut in_code = false;
    let flush = |prose: &mut String, lines: &mut Vec<String>| {
        let stripped = strip_control_blocks(prose);
        lines.extend(
            stripped
                .lines()
                .filter(|line| !is_scaffolding_line(line))
                .map(str::to_string),
        );
        prose.clear();
    };
    for line in text.lines() {
        if in_code {
            lines.push(line.to_string());
            if is_fence(line) {
                in_code = false;
            }
        } else if is_fence(line) {
            flush(&mut prose, &mut lines);
            lines.push(line.to_string());
            in_code = true;
        } else {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    flush(&mut prose, &mut lines);
    while let Some(first) = lines.first() {
        if first.trim().is_empty() {
            lines.remove(0);
        } else {
            break;
        }
    }
    while let Some(last) = lines.last() {
        if last.trim().is_empty() {
            lines.pop();
        } else {
            break;
        }
    }
    let mut out = lines.join("\n");
    if had_trailing_newline && !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_marker_is_unique_and_bracketed() {
        let mut markers: Vec<&str> = ControlKind::ALL
            .into_iter()
            .map(ControlKind::marker)
            .collect();
        markers.extend(inline_markers());
        let mut sorted = markers.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            markers.len(),
            "duplicate marker in {markers:?}"
        );
        assert!(
            markers
                .iter()
                .all(|m| m.starts_with('[') && m.ends_with(']'))
        );
    }

    /// Code a person will copy is never edited: inside a fence, a line that
    /// looks like runtime narration or a context tag is content.
    #[test]
    fn fenced_code_passes_through_verbatim() {
        let text = "Run this:\n```\nvak\nOutcome: ok\n<intent>\n```\nSurface: cli\nDone.";
        assert_eq!(
            clean_scaffolding(text),
            "Run this:\n```\nvak\nOutcome: ok\n<intent>\n```\nDone."
        );
        // An unterminated fence runs to the end, and the prose before it is
        // still cleaned.
        let open = "<intent>note</intent>Answer:\n~~~html\n<intent class=\"x\">";
        assert_eq!(
            clean_scaffolding(open),
            "Answer:\n~~~html\n<intent class=\"x\">"
        );
    }

    #[test]
    fn ordinary_text_is_never_a_control_line() {
        for text in [
            "how did the market do",
            "[ERROR]: it broke",
            "[note] remember this",
        ] {
            assert!(!is_control_line(text), "{text}");
        }
    }

    #[test]
    fn inline_hints_are_control_lines() {
        for hint in InlineHint::ALL {
            assert!(is_control_line(&format!("  {} something", hint.marker())));
        }
    }

    #[test]
    fn only_answer_retries_arm_the_card_supersede() {
        let armed: Vec<_> = ControlKind::ALL
            .into_iter()
            .filter(|k| k.retries_answer())
            .collect();
        assert_eq!(
            armed,
            vec![
                ControlKind::GroundingCheck,
                ControlKind::FenceCheck,
                ControlKind::DuplicateCardCheck,
                ControlKind::PresentationCheck,
                ControlKind::FreshnessCheck,
                ControlKind::EmptyStep,
                ControlKind::TopicMismatchCheck,
            ]
        );
    }

    #[test]
    fn serialises_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ControlKind::DuplicateCardCheck).unwrap(),
            "\"duplicate_card_check\""
        );
    }

    #[test]
    fn inline_hints_are_scaffolding_and_clean_to_nothing() {
        for marker in inline_markers() {
            let line = format!("{marker}: something the runtime said");
            assert!(is_scaffolding_line(&line), "{line}");
            assert_eq!(clean_scaffolding(&line), "", "{line}");
        }
    }

    #[test]
    fn real_answer_text_survives_around_embedded_hints() {
        let text = "<intent>select</intent><context_packet>d</context_packet>Final result.";
        assert_eq!(clean_scaffolding(text), "Final result.");
        let text = "Answer body.\n[recovery] retry the failing call";
        assert_eq!(clean_scaffolding(text), "Answer body.");
        let text = "Tool output\n[post-tool-use hook]: blocked\nMore output";
        assert_eq!(clean_scaffolding(text), "Tool output\nMore output");
    }
}
