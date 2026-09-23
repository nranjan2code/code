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

/// One parsed, validated `recall` call. Exactly one of the three request
/// shapes is ever produced by [`parse_recall_args`].
#[derive(Debug, Clone, PartialEq)]
pub enum RecallRequest {
    /// `{ turn }`: the 1-based turn number (`TurnIndex` order).
    Turn(u64),
    /// `{ presentation }`: a `Presentation` ledger-entry id.
    Presentation(String),
    /// `{ id, range? }`: an evidence id (tool_use_id), optionally sliced to
    /// a 1-based inclusive line range.
    Id {
        id: String,
        range: Option<(u64, u64)>,
    },
}

/// Validates a `recall` call's arguments: exactly one of `turn`,
/// `presentation`, or `id` must be present; `range` is only meaningful with
/// `id` but is not rejected when present alongside another field (the
/// resolver simply ignores it) — the one-of check is what actually matters.
pub fn parse_recall_args(args: &Value) -> Result<RecallRequest, String> {
    let turn = args.get("turn").and_then(Value::as_u64);
    let presentation = args
        .get("presentation")
        .and_then(Value::as_str)
        .map(str::to_string);
    let id = args.get("id").and_then(Value::as_str).map(str::to_string);
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
                        (Some(start), Some(end)) => Some((start, end)),
                        _ => {
                            return Err("'range' requires integer 'start' and 'end'".to_string());
                        }
                    }
                }
            };
            Ok(RecallRequest::Id { id, range })
        }
        _ => Err("exactly one of 'turn', 'presentation', or 'id' is required".to_string()),
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
        "Reopen a past turn's full record, a presentation's canonical payload, or an evidence \
         result's full content (optionally by line range). Exactly one of turn, presentation, \
         or id."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
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
    fn rejects_zero_or_multiple_fields() {
        assert!(parse_recall_args(&json!({})).is_err());
        assert!(parse_recall_args(&json!({"turn": 1, "id": "x"})).is_err());
        assert!(parse_recall_args(&json!({"id": "x", "range": {"start": 1}})).is_err());
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
