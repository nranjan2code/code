//! `recall`: the one way to reach anything from before this turn
//! (docs/design/68-context-engine.md §3, docs/design/23-memory.md).
//!
//! Two verbs. `query` searches everything: this conversation's earlier
//! turns first, then, where the runtime reaches them, other conversations,
//! saved notes, the person's profile, followed sources and deliverables,
//! each hit saying where it is from and, when it can be reopened, the exact
//! `open` value that reopens it. `open` reopens one reference in a form the
//! context already shows: `#3` (a turn of `<turns>`), `pres:<id>`,
//! `ev:<id>`, `turn:<turn_id>` from a hit, or
//! `conversation:<id>/turn:<turn_id>` for a turn of another conversation an
//! attached artifact came from (plan M8.3b).
//!
//! One tool replaced two (`recall` and `session_search`, 2026-10-16): asked
//! for a detail three turns back, a small model chose the tool that cannot
//! search this conversation 13 times in 16 (docs/audits/
//! history-lookup-2026-10-16.md). A scope the model must pick is a mistake
//! it can make; one search it cannot.
//!
//! `RecallTool` only carries the definition: it has no session access
//! (`ToolContext` carries none, AGENTS.md invariant 14), so the agent loop
//! answers `recall` calls before dispatch. [`parse_recall_args`],
//! [`apply_range`] and [`apply_chars`] are the pure logic it uses.

use crate::Fault;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{Tool, ToolContext, ToolOutput};

/// Hits a search returns when the call names no limit.
pub const DEFAULT_LIMIT: usize = 8;
/// Most hits one search returns.
pub const MAX_LIMIT: usize = 20;

/// One parsed `recall` call.
#[derive(Debug, Clone, PartialEq)]
pub enum RecallRequest {
    /// `query`: search everything the runtime reaches.
    Search { query: String, limit: usize },
    /// `open: "#3"`: a turn of this conversation by its `<turns>` number.
    Turn(u64),
    /// `open: "turn:<turn_id>"`: a turn of this conversation by its stable id.
    TurnId(String),
    /// `open: "pres:<id>"`: a presentation's canonical payload.
    Presentation(String),
    /// `open: "ev:<id>"`: a tool result in full, or the `lines` or `chars`
    /// of it named.
    Evidence {
        id: String,
        /// A 1-based inclusive line range.
        lines: Option<(u64, u64)>,
        /// A 0-based character range `[start, end)`, for reaching inside
        /// one line longer than a window shows.
        chars: Option<(u64, u64)>,
    },
    /// `open: "conversation:<id>/turn:<turn_id>"`: a turn of another
    /// conversation, reachable only when an artifact attached here came
    /// from it (the loop checks; Core checks its trash, Agent and audience).
    Elsewhere {
        conversation: String,
        turn_id: String,
    },
}

/// What `open` accepts, as the model is told when it sends something else.
const OPEN_FORMS: &str = "open takes one reference as the context shows it: #3 (a turn), \
pres:<id> (a card), ev:<id> (a tool result), turn:<turn_id> or \
conversation:<id>/turn:<turn_id> (from a search hit)";

/// The request a `recall` call's arguments name. `open` wins over `query`
/// when both are sent: the query is then the reason for the reopening.
pub fn parse_recall_args(args: &Value) -> Result<RecallRequest, String> {
    let open = text_arg(args, "open");
    let query = text_arg(args, "query");
    if let Some(open) = open {
        return parse_open(open, args);
    }
    let Some(query) = query else {
        return Err(
            "send query (what to find from before this turn) or open (a reference to reopen); \
             a request that does not refer back needs no recall"
                .into(),
        );
    };
    if query.chars().count() > 2048 {
        return Err("query must be at most 2048 characters".into());
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| (n as usize).clamp(1, MAX_LIMIT));
    Ok(RecallRequest::Search {
        query: query.into(),
        limit,
    })
}

/// A non-empty, trimmed string argument. Some providers send every optional
/// property with an empty default; those count as absent.
fn text_arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse_open(open: &str, args: &Value) -> Result<RecallRequest, String> {
    let id = |s: &str| -> Result<String, String> {
        let s = s.trim();
        if s.is_empty() || s.len() > 128 || s.contains(char::is_whitespace) {
            Err(OPEN_FORMS.into())
        } else {
            Ok(s.to_string())
        }
    };
    let number = open.strip_prefix('#').unwrap_or(open);
    if let Ok(n) = number.parse::<u64>() {
        return if n > 0 {
            Ok(RecallRequest::Turn(n))
        } else {
            Err(OPEN_FORMS.into())
        };
    }
    if let Some(rest) = open.strip_prefix("ev:") {
        return Ok(RecallRequest::Evidence {
            id: id(rest)?,
            lines: range_arg(args, "lines", |start, end| start > 0 && end >= start)?,
            chars: range_arg(args, "chars", |start, end| end > start)?,
        })
        .and_then(|request| match &request {
            RecallRequest::Evidence {
                lines: Some(_),
                chars: Some(_),
                ..
            } => Err("lines and chars cannot be combined".into()),
            _ => Ok(request),
        });
    }
    if let Some(rest) = open.strip_prefix("pres:") {
        return Ok(RecallRequest::Presentation(id(rest)?));
    }
    if let Some(rest) = open.strip_prefix("conversation:") {
        let (conversation, turn) = rest.split_once("/turn:").ok_or(OPEN_FORMS)?;
        if conversation.contains(['/', '\\']) {
            return Err(OPEN_FORMS.into());
        }
        return Ok(RecallRequest::Elsewhere {
            conversation: id(conversation)?,
            turn_id: id(turn)?,
        });
    }
    if let Some(rest) = open.strip_prefix("turn:") {
        return match rest.trim().parse::<u64>() {
            Ok(n) if n > 0 => Ok(RecallRequest::Turn(n)),
            _ => Ok(RecallRequest::TurnId(id(rest)?)),
        };
    }
    Err(OPEN_FORMS.into())
}

/// An optional `{start, end}` range argument; `{0, 0}` (a provider's empty
/// default) counts as absent.
fn range_arg(
    args: &Value,
    name: &str,
    valid: fn(u64, u64) -> bool,
) -> Result<Option<(u64, u64)>, String> {
    let Some(range) = args.get(name).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let start = range.get("start").and_then(Value::as_u64);
    let end = range.get("end").and_then(Value::as_u64);
    match (start, end) {
        (Some(0), Some(0)) => Ok(None),
        (Some(start), Some(end)) if valid(start, end) => Ok(Some((start, end))),
        _ => Err(format!(
            "{name} takes integer start and end, end not before start"
        )),
    }
}

/// Applies an optional 1-based inclusive line range to `content`. Pure: no
/// I/O, no session access. Out-of-range bounds clamp rather than error, so a
/// model guessing at a range still gets whatever exists.
pub fn apply_range(content: &str, range: Option<(u64, u64)>) -> String {
    let Some((start, end)) = range else {
        return content.to_string();
    };
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let start_idx = start.max(1) as usize - 1;
    let end_idx = end.max(start) as usize;
    if start_idx >= lines.len() {
        return String::new();
    }
    lines[start_idx..end_idx.min(lines.len())].join("\n")
}

/// Most characters one `chars` recall returns, so the answer fits a request
/// whole and is never windowed again.
pub const CHAR_RANGE_MAX: u64 = 29_000;

/// A 0-based character range `[start, end)` of tool result `id`'s
/// `content`, for the one line a window could not show. At most
/// [`CHAR_RANGE_MAX`] characters; when more remains, a closing line names
/// the call that continues.
pub fn apply_chars(id: &str, content: &str, start: u64, end: u64) -> String {
    let total = content.chars().count() as u64;
    if start >= total {
        return format!("[the result has {total} chars; nothing at {start}]");
    }
    let end = end.min(total).min(start.saturating_add(CHAR_RANGE_MAX));
    let slice: String = content
        .chars()
        .skip(start as usize)
        .take((end - start) as usize)
        .collect();
    if end < total {
        format!(
            "{slice}\n[chars {start}-{end} of {total}; recall {{\"open\": \"ev:{id}\", \"chars\": {{\"start\": {end}, \"end\": {total}}}}} continues]"
        )
    } else {
        slice
    }
}

pub struct RecallTool;

#[async_trait]
impl Tool for RecallTool {
    fn name(&self) -> &str {
        "recall"
    }

    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::reach(vak_permission::Reach::ReadStore)
    }

    fn serves(&self) -> Vec<&str> {
        vec!["memory"]
    }

    fn always_loaded(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Find something from before this turn that is not in front of you: an earlier turn of \
         this conversation, another conversation, a note saved with remember, the person's \
         profile, a source you follow or a file you made. Search with query, a specific \
         subject; each hit says where it is from, and gives the open value that reopens it. \
         Reopen with open: #3 (a turn), pres:<id> (a card), ev:<id> (a tool result), or a \
         hit's turn:<id>. A request that does not refer back needs no recall. What a record \
         says was true then; current facts need fresh evidence."
    }

    fn schema(&self) -> Value {
        let range = |what: &str| {
            json!({
                "type": "object",
                "description": what,
                "properties": {
                    "start": {"type": "integer", "description": "Where the range starts."},
                    "end": {"type": "integer", "description": "Where the range ends."}
                },
                "required": ["start", "end"]
            })
        };
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "What to find, as a specific subject: a name, a thing, a file, what was said."},
                "open": {"type": "string", "description": "One reference to reopen, as the context or a hit shows it: #3, pres:<id>, ev:<id>, turn:<id> or conversation:<id>/turn:<id>."},
                "lines": range("Optional, with an ev: reference: the 1-based lines to return, inclusive."),
                "chars": range("Optional, with an ev: reference: the 0-based characters [start, end) to return, for one very long line; at most 29000."),
                "limit": {"type": "integer", "description": "Optional, with query: how many hits, default 8, at most 20."}
            }
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        // `recall` is answered by the agent loop before dispatch (it needs
        // session access this tool object never has); reaching here is a
        // wiring bug, still reported so a stray dispatch fails loudly.
        match parse_recall_args(args) {
            Ok(_) => ToolOutput::error(
                Fault::Tool,
                r#"{"type":"internal","message":"recall must be answered by the agent loop, not dispatched directly"}"#,
            ),
            Err(message) => ToolOutput::error(
                Fault::Shape,
                json!({"type": "invalid_arguments", "message": message}).to_string(),
            ),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn open(reference: &str) -> Result<RecallRequest, String> {
        parse_recall_args(&json!({ "open": reference }))
    }

    #[test]
    fn open_reads_every_reference_the_context_shows() {
        assert_eq!(open("#3").unwrap(), RecallRequest::Turn(3));
        assert_eq!(open("3").unwrap(), RecallRequest::Turn(3));
        assert_eq!(open("turn:3").unwrap(), RecallRequest::Turn(3));
        assert_eq!(
            open("turn:01a1-xyz").unwrap(),
            RecallRequest::TurnId("01a1-xyz".into())
        );
        assert_eq!(
            open("pres:p1").unwrap(),
            RecallRequest::Presentation("p1".into())
        );
        assert_eq!(
            open("ev:call_7").unwrap(),
            RecallRequest::Evidence {
                id: "call_7".into(),
                lines: None,
                chars: None
            }
        );
        assert_eq!(
            open("conversation:s9/turn:t2").unwrap(),
            RecallRequest::Elsewhere {
                conversation: "s9".into(),
                turn_id: "t2".into()
            }
        );
        for wrong in [
            "#0",
            "pres:",
            "ev:a b",
            "conversation:s9",
            "the Lisbon notes",
        ] {
            let refused = open(wrong).unwrap_err();
            assert!(refused.contains("#3"), "{wrong}: {refused}");
        }
    }

    #[test]
    fn a_query_beside_open_is_the_reason_for_the_reopening() {
        assert_eq!(
            parse_recall_args(&json!({"open": "#2", "query": "the Lisbon notes"})).unwrap(),
            RecallRequest::Turn(2)
        );
    }

    #[test]
    fn lines_and_chars_slice_a_tool_result() {
        assert_eq!(
            parse_recall_args(&json!({"open": "ev:e", "lines": {"start": 2, "end": 5}})).unwrap(),
            RecallRequest::Evidence {
                id: "e".into(),
                lines: Some((2, 5)),
                chars: None
            }
        );
        assert_eq!(
            parse_recall_args(&json!({"open": "ev:e", "chars": {"start": 2000, "end": 9000}}))
                .unwrap(),
            RecallRequest::Evidence {
                id: "e".into(),
                lines: None,
                chars: Some((2000, 9000))
            }
        );
        assert!(
            parse_recall_args(&json!({"open": "ev:e", "chars": {"start": 5, "end": 5}})).is_err()
        );
        assert!(
            parse_recall_args(&json!({"open": "ev:e", "lines": {"start": 1, "end": 2},
                "chars": {"start": 0, "end": 9}}))
            .is_err()
        );
        assert!(
            parse_recall_args(&json!({"open": "ev:e", "lines": {"start": 0, "end": 0}}))
                .is_ok_and(|r| matches!(r, RecallRequest::Evidence { lines: None, .. }))
        );
    }

    #[test]
    fn search_is_bounded() {
        assert_eq!(
            parse_recall_args(&json!({"query": " Noida forecast ", "limit": 1000})).unwrap(),
            RecallRequest::Search {
                query: "Noida forecast".into(),
                limit: MAX_LIMIT
            }
        );
        assert!(parse_recall_args(&json!({"query": "x".repeat(2049)})).is_err());
    }

    #[test]
    fn nothing_to_find_says_what_to_send() {
        for empty in [
            json!({}),
            json!({"query": " "}),
            json!({"query": "", "open": ""}),
        ] {
            let refused = parse_recall_args(&empty).unwrap_err();
            assert!(
                refused.contains("query") && refused.contains("open"),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_character_range_continues_by_naming_the_call() {
        let line = "x".repeat(100_000);
        let first = apply_chars("e1", &line, 2_000, 90_000);
        assert!(first.ends_with(
            "[chars 2000-31000 of 100000; recall {\"open\": \"ev:e1\", \"chars\": {\"start\": 31000, \"end\": 100000}} continues]"
        ));
        assert_eq!(apply_chars("e1", &line, 99_990, 200_000).len(), 10);
        assert!(apply_chars("e", "héllo wörld", 1, 4).starts_with("éll\n[chars 1-4 of 11"));
        assert_eq!(apply_chars("e", "héllo wörld", 6, 11), "wörld");
    }

    #[test]
    fn apply_range_clamps() {
        assert_eq!(apply_range("a\nb\nc", Some((2, 9))), "b\nc");
        assert_eq!(apply_range("a\nb\nc", Some((5, 9))), "");
        assert_eq!(apply_range("a\nb", None), "a\nb");
    }
}
