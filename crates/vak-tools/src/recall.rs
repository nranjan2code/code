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
    /// `{ presentation }`: a `Presentation` ledger-entry id.
    Presentation(String),
    /// `{ id, range? }`: an evidence id (tool_use_id), optionally sliced to
    /// a 1-based inclusive line range.
    Id {
        id: String,
        range: Option<(u64, u64)>,
    },
}

/// Validates a `recall` call's arguments: exactly one of `query`, `turn_id`,
/// `turn`, `presentation`, or `id` must be present; `range` is only meaningful with
/// `id` but is not rejected when present alongside another field (the
/// resolver simply ignores it) — the one-of check is what actually matters.
pub fn parse_recall_args(args: &Value) -> Result<RecallRequest, String> {
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
    if let Some(query) = query {
        if turn.is_some() || presentation.is_some() || id.is_some() {
            return Err("query cannot be combined with a recall target".into());
        }
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
            Ok(RecallRequest::Id { id, range })
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
                range: None
            }
        );
        assert_eq!(
            parse_recall_args(&json!({"id": "ev1", "range": {"start": 2, "end": 5}})).unwrap(),
            RecallRequest::Id {
                id: "ev1".into(),
                range: Some((2, 5))
            }
        );
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
        assert!(parse_recall_args(&json!({"query": "weather", "turn": 1})).is_err());
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
                range: None
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
