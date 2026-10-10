//! `recall`: reopen a past turn, presentation, or evidence result by id.
//!
//! Part of docs/design/68-context-engine.md §3/§7/§10's evidence store: past
//! turns are digested (or reduced to a card line) in the request, and this
//! tool is how the model reverses that — full turn record, canonical
//! presentation payload, or full tool-result content, optionally sliced by
//! line range.
//!
//! `RecallTool` only carries the definition (name/schema/description): it
//! has no session access (`ToolContext` carries none — AGENTS.md invariant
//! 14's worker/broker boundary), so the agent loop intercepts `recall` calls
//! by name before dispatch and answers them from the session directly,
//! exactly as `emit_*_card` results are rewritten after `execute_batch`, but
//! earlier — `recall` is never sent to a worker. `execute()` here exists
//! only as a defensive fallback and must never be reachable in production.
//! [`parse_recall_args`] and [`apply_range`] are the pure logic the loop
//! reuses to validate arguments and slice evidence content.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{Tool, ToolContext, ToolOutput};

/// One parsed, validated `recall` call. Exactly one of the supported request
/// shapes is ever produced by [`parse_recall_args`].
#[derive(Debug, Clone, PartialEq)]
pub enum RecallRequest {
    /// `{ turn }`: the 1-based turn number (`TurnIndex` order).
    Turn(u64),
    /// Stable directive entry ID, independent of display numbering.
    TurnId(String),
    /// Search compact records in the current conversation before reopening one.
    Search { query: String, limit: usize },
    /// A search or a turn reopened in another conversation: one named by
    /// an artifact attached in this conversation (plan M8.3b).
    Elsewhere {
        conversation: String,
        request: Box<RecallRequest>,
    },
    /// `{ presentation }`: a `Presentation` ledger-entry id.
    Presentation(String),
    /// `{ id, range? }`: an evidence id (tool_use_id), optionally sliced to
    /// a 1-based inclusive line range.
    Id {
        id: String,
        range: Option<(u64, u64)>,
        /// A 0-based character range `[start, end)` over the whole result,
        /// for reaching inside one line longer than a window shows.
        chars: Option<(u64, u64)>,
    },
}

/// The request a `recall` call's arguments name. `conversation` turns a
/// search or a `turn_id` reopen into one of another conversation.
pub fn parse_recall_args(args: &Value) -> Result<RecallRequest, String> {
    let request = parse_target(args)?;
    let Some(conversation) = args
        .get("conversation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(request);
    };
    if conversation.len() > 128 || conversation.contains(['/', '\\']) {
        return Err("conversation is not a conversation id".into());
    }
    match request {
        RecallRequest::Search { .. } | RecallRequest::TurnId(_) => Ok(RecallRequest::Elsewhere {
            conversation: conversation.to_string(),
            request: Box::new(request),
        }),
        _ => Err("conversation works with query or turn_id only".into()),
    }
}

/// Validates a `recall` call's arguments: one of `query`, `turn_id`, `turn`,
/// `presentation`, or `id` names what to recall; `range` is only meaningful
/// with `id` but is not rejected when present alongside another field (the
/// resolver simply ignores it). A `query` beside a precise target (`id`,
/// `presentation`, `turn`) is the reason for the recall, not a second
/// target: the target is recalled. Refusing it left a model that named the
/// evidence it wanted, and why, with nothing (found live, 2026-10-10).
fn parse_target(args: &Value) -> Result<RecallRequest, String> {
    // Some providers materialize every optional schema property with an
    // empty default. Treat those placeholders as absent while retaining the
    // exactly-one-target rule for meaningful values.
    let turn = args.get("turn").and_then(Value::as_u64).filter(|n| *n > 0);
    let presentation = args
        .get("presentation")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let turn_id = args
        .get("turn_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(turn_id) = turn_id {
        if turn.is_some() || presentation.is_some() || id.is_some() || query.is_some() {
            return Err("turn_id cannot be combined with another recall target".into());
        }
        if turn_id.len() > 128 {
            return Err("turn_id is too long".into());
        }
        return Ok(RecallRequest::TurnId(turn_id.into()));
    }
    let precise = turn.is_some() || presentation.is_some() || id.is_some();
    if let Some(query) = query.filter(|_| !precise) {
        if query.chars().count() > 2048 {
            return Err("query must be at most 2048 characters".into());
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(8)
            .clamp(1, 20) as usize;
        return Ok(RecallRequest::Search {
            query: query.into(),
            limit,
        });
    }
    match (turn, presentation, id) {
        (Some(turn), None, None) => Ok(RecallRequest::Turn(turn)),
        (None, Some(presentation), None) => Ok(RecallRequest::Presentation(presentation)),
        (None, None, Some(id)) => {
            let range = match args.get("range") {
                None | Some(Value::Null) => None,
                Some(range) => {
                    let start = range.get("start").and_then(Value::as_u64);
                    let end = range.get("end").and_then(Value::as_u64);
                    match (start, end) {
                        (Some(0), Some(0)) => None,
                        (Some(start), Some(end)) if start > 0 && end > 0 => Some((start, end)),
                        _ => {
                            return Err("'range' requires integer 'start' and 'end'".to_string());
                        }
                    }
                }
            };
            let chars = match args.get("chars") {
                None | Some(Value::Null) => None,
                Some(chars) => {
                    let start = chars.get("start").and_then(Value::as_u64);
                    let end = chars.get("end").and_then(Value::as_u64);
                    match (start, end) {
                        (Some(0), Some(0)) => None,
                        (Some(start), Some(end)) if end > start => Some((start, end)),
                        _ => {
                            return Err(
                                "'chars' requires integer 'start' and a larger 'end'".to_string()
                            );
                        }
                    }
                }
            };
            if range.is_some() && chars.is_some() {
                return Err("'range' and 'chars' cannot be combined".to_string());
            }
            Ok(RecallRequest::Id { id, range, chars })
        }
        _ => Err(if args.get("query").is_some() {
            "'query' must be a non-empty description of the past subject or result to find; omit recall for a fresh or unrelated request, and use an exact turn_id, turn, presentation, or evidence id when available".to_string()
        } else {
            "exactly one non-empty target is required: query, turn_id, turn, presentation, or id; omit recall for a fresh or unrelated request".to_string()
        }),
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

/// A 0-based character range `[start, end)` of `content`, for the one line a
/// window could not show. At most [`CHAR_RANGE_MAX`] characters; when more
/// remains, a closing line names where to continue.
pub fn apply_chars(content: &str, start: u64, end: u64) -> String {
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
            "{slice}\n[chars {start}-{end} of {total}; recall the same id with chars start {end} for the rest]"
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

    fn serves(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn always_loaded(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Use only when the current request needs missing information from an earlier turn. Do not \
         call for a fresh or unrelated request, and never call with an empty query. Search this \
         conversation's past turns with a specific natural-language subject/referent query, then reopen a matching turn's full \
         record, a presentation's canonical payload, or an evidence result (optionally by line \
         range). Exactly one of query, turn_id, turn, presentation, or id. Old records describe what \
         happened then; use fresh evidence for current conditions."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "turn_id": {"type": "string", "description": "Stable turn_id from a search result or selected reference; reopens that turn even when display numbering changes."},
                "conversation": {"type": "string", "description": "With query or turn_id: a conversation named by an attached Library artifact, to search or reopen its turns instead of this conversation's."},
                "query": {"type": "string", "minLength": 1, "description": "Required only when searching history: give a non-empty natural-language subject, entity, or artifact from an earlier turn. Never send an empty string. Do not call recall for fresh or unrelated requests. Returns compact candidate references, not verified facts."},
                "limit": {"type": "integer", "description": "Search result count, default 8, maximum 20."},
                "turn": {
                    "type": "integer",
                    "description": "Turn number (as shown in a <turns> card line) to reopen as its full record."
                },
                "presentation": {
                    "type": "string",
                    "description": "Presentation id (as shown in a card line's pres:<id>) to return the canonical payload for."
                },
                "id": {
                    "type": "string",
                    "description": "Evidence id (as shown in a card line's ev:<id>) to return the full tool result for."
                },
                "chars": {
                    "type": "object",
                    "description": "Optional 0-based character range [start, end), only meaningful with 'id'; reaches inside one very long line a window cut. At most 29000 characters per call.",
                    "properties": {
                        "start": {"type": "integer"},
                        "end": {"type": "integer"}
                    },
                    "required": ["start", "end"]
                },
                "range": {
                    "type": "object",
                    "description": "Optional 1-based inclusive line range, only meaningful with 'id'.",
                    "properties": {
                        "start": {"type": "integer"},
                        "end": {"type": "integer"}
                    },
                    "required": ["start", "end"]
                }
            }
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        // `recall` is answered by the agent loop before dispatch (it needs
        // session access this tool object never has); reaching here is a
        // wiring bug, not a user-correctable error, but it is still
        // reported as one so a stray dispatch fails loudly instead of
        // silently returning nothing.
        match parse_recall_args(args) {
            Ok(_) => ToolOutput::error(
                r#"{"type":"internal","message":"recall must be answered by the agent loop, not dispatched directly"}"#,
            ),
            Err(message) => ToolOutput::error(format!(
                r#"{{"type":"invalid_arguments","message":"{message}"}}"#
            )),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_request_shape() {
        assert_eq!(
            parse_recall_args(&json!({"turn": 3})).unwrap(),
            RecallRequest::Turn(3)
        );
        assert_eq!(
            parse_recall_args(&json!({"presentation": "p1"})).unwrap(),
            RecallRequest::Presentation("p1".into())
        );
        assert_eq!(
            parse_recall_args(&json!({"id": "ev1"})).unwrap(),
            RecallRequest::Id {
                id: "ev1".into(),
                range: None,
                chars: None
            }
        );
        assert_eq!(
            parse_recall_args(&json!({"id": "ev1", "range": {"start": 2, "end": 5}})).unwrap(),
            RecallRequest::Id {
                id: "ev1".into(),
                range: Some((2, 5)),
                chars: None
            }
        );
    }

    /// The call a model made on a deployed host: every property filled,
    /// a real evidence id and a query saying what it wanted from it.
    #[test]
    fn a_query_beside_an_evidence_id_recalls_the_evidence() {
        let call = json!({
            "chars": {"end": 10000, "start": 0}, "conversation": "", "id": "call_GX",
            "limit": 8, "presentation": "", "query": "NIFTY Bank daily OHLC values",
            "range": {"end": 0, "start": 0}, "turn": 0, "turn_id": ""
        });
        match parse_recall_args(&call).unwrap() {
            RecallRequest::Id { id, .. } => assert_eq!(id, "call_GX"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_character_range_reaches_inside_one_long_line() {
        assert_eq!(
            parse_recall_args(&json!({"id": "ev1", "chars": {"start": 2000, "end": 9000}}))
                .unwrap(),
            RecallRequest::Id {
                id: "ev1".into(),
                range: None,
                chars: Some((2_000, 9_000))
            }
        );
        assert!(parse_recall_args(&json!({"id": "e", "chars": {"start": 5, "end": 5}})).is_err());
        assert!(
            parse_recall_args(&json!({"id": "e", "range": {"start": 1, "end": 2},
                "chars": {"start": 0, "end": 5}}))
            .is_err()
        );
        let line: String = ('a'..='z').cycle().take(100_000).collect();
        let first = apply_chars(&line, 2_000, 90_000);
        assert!(first.starts_with(&line[2_000..2_010]));
        assert!(first.contains(&format!("chars 2000-{} of 100000", 2_000 + CHAR_RANGE_MAX)));
        let last = apply_chars(&line, 99_990, 200_000);
        assert_eq!(last, &line[99_990..]);
        assert!(apply_chars("héllo wörld", 1, 4).starts_with("éll\n[chars 1-4 of 11"));
        assert_eq!(apply_chars("héllo wörld", 6, 11), "wörld");
    }

    #[test]
    fn stable_turn_targets_are_exclusive() {
        assert_eq!(
            parse_recall_args(&json!({"turn_id": "stable", "turn": 0, "query": ""})).unwrap(),
            RecallRequest::TurnId("stable".into())
        );
        assert!(parse_recall_args(&json!({"turn_id": "stable", "query": "topic"})).is_err());
        assert!(parse_recall_args(&json!({"turn_id": "stable", "id": "evidence"})).is_err());
    }

    #[test]
    fn search_is_bounded_and_exclusive() {
        assert_eq!(
            parse_recall_args(&json!({"query": " Noida forecast ", "limit": 1000})).unwrap(),
            RecallRequest::Search {
                query: "Noida forecast".into(),
                limit: 20
            }
        );
        // A query beside a precise target is its reason, not a second target.
        assert_eq!(
            parse_recall_args(&json!({"query": "weather", "turn": 1})).unwrap(),
            RecallRequest::Turn(1)
        );
        assert!(parse_recall_args(&json!({"query": "x".repeat(2049)})).is_err());
        assert!(parse_recall_args(&json!({"query": " "})).is_err());
    }

    #[test]
    fn rejects_zero_or_multiple_fields() {
        assert!(parse_recall_args(&json!({})).is_err());
        assert!(parse_recall_args(&json!({"turn": 1, "id": "x"})).is_err());
        assert!(parse_recall_args(&json!({"id": "x", "range": {"start": 1}})).is_err());
    }

    #[test]
    fn empty_search_returns_actionable_guidance() {
        let error = parse_recall_args(&json!({"query": "  "})).unwrap_err();
        assert!(error.contains("non-empty description"));
        assert!(error.contains("fresh or unrelated request"));

        let error = parse_recall_args(&json!({})).unwrap_err();
        assert!(error.contains("omit recall"));
    }

    #[test]
    fn ignores_provider_filled_optional_placeholders() {
        let placeholders =
            json!({"turn": 0, "presentation": "", "id": "", "range": {"start": 0, "end": 0}});
        assert!(parse_recall_args(&placeholders).is_err());
        let mut id_request = placeholders.clone();
        id_request["id"] = json!("ev1");
        assert_eq!(
            parse_recall_args(&id_request).unwrap(),
            RecallRequest::Id {
                id: "ev1".into(),
                range: None,
                chars: None
            }
        );
        let mut turn_request = placeholders;
        turn_request["turn"] = json!(2);
        assert_eq!(
            parse_recall_args(&turn_request).unwrap(),
            RecallRequest::Turn(2)
        );
    }

    #[test]
    fn range_slices_by_line_and_clamps() {
        let content = "a\nb\nc\nd\ne";
        assert_eq!(apply_range(content, None), content);
        assert_eq!(apply_range(content, Some((2, 3))), "b\nc");
        assert_eq!(apply_range(content, Some((4, 100))), "d\ne");
        assert_eq!(apply_range(content, Some((100, 200))), "");
    }

    #[tokio::test]
    async fn execute_is_never_the_real_answer() {
        let tool = RecallTool;
        let ctx = ToolContext::new(std::path::PathBuf::from("."));
        let out = tool.execute(&json!({"turn": 1}), &ctx).await;
        assert!(out.is_error, "execute() must never be a live recall path");
    }
}
