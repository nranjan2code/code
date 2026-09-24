//! The one server-side projection from `vak_agent::AgentEvent` (the internal
//! agent-loop event stream) to `ClientEvent` (what `/sessions/:id/events` and
//! `/sessions/:id/side/events` are allowed to send). Both `seq_frame` (live)
//! and replay run every event through `project` here, so a client can never
//! observe a difference between what it missed and what it is seeing live.
//!
//! `AgentEvent` carries a lot that is legitimately useful to the agent loop
//! and to Workbench/observability surfaces but was never meant for a chat
//! reader: retry backoff, frozen-ladder route legs, context compaction,
//! stop-hook continuations, worker token counts, and — critically — raw
//! provider/internal error text riding in `RunFinished.summary` or a tool's
//! `result_preview`. Per AGENTS.md ("Runtime-authored traffic is typed,
//! never sniffed") and docs/design/30-output-engineering.md, the client
//! receives only what it deliberately renders, and a run's outcome is always
//! reduced to a small typed set of human sentences rather than forwarded
//! text.
//!
//! A `TextDelta`/`ThinkingDelta` frame here carries only the delta, never the
//! provider's full `partial` snapshot `AgentEvent::Stream` carries — the
//! client already accumulates deltas itself (invariant 4: consumers choose
//! delta or snapshot, never both forced together on the same frame).

use vak_agent::AgentEvent;

/// A run's outcome, reduced to the handful of states a person may see.
/// `AgentEvent::RunFinished` carries only `summary: String` + `is_error:
/// bool`; `summary` is internal bookkeeping that can legitimately be a raw
/// provider error (`format!("failed: {error}")`) or an internal ceiling
/// message. Classification below trusts only `is_error` plus the small set
/// of literal sentinels the runtime is documented to emit for a clean stop
/// (`"aborted"`) or a step-limit stop (`"max_turns"`) — never the rest of
/// the string, which is discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum RunOutcome {
    Completed,
    Stopped,
    MaxTurns,
    Failed,
}

/// Classify a raw `(summary, is_error)` pair into a `RunOutcome` without
/// ever exposing `summary` itself to a caller.
///
/// `"cancelled by client"` is `cancel_run`'s own synthetic `RunFinished`
/// (`vak-server/src/lib.rs`), which predates the run's real terminal event
/// using the documented `"aborted"` sentinel; both are recognized here as
/// `Stopped` until that synthetic send is retired.
pub(crate) fn classify_run_outcome(summary: &str, is_error: bool) -> RunOutcome {
    match summary {
        "aborted" | "cancelled by client" => RunOutcome::Stopped,
        "max_turns" => RunOutcome::MaxTurns,
        _ if is_error => RunOutcome::Failed,
        _ => RunOutcome::Completed,
    }
}

/// The one human sentence for each outcome. Never raw error text — see
/// `RunOutcome`'s doc comment. Shared by the live `/events` projection
/// (`ClientEvent::RunFinished`) and the gateway's channel reply text
/// (`gateway::outcome_text`), so a Telegram/Slack/Discord user and a client
/// tab see the same words for the same outcome.
pub(crate) fn run_outcome_message(outcome: RunOutcome) -> &'static str {
    match outcome {
        RunOutcome::Completed => "Completed.",
        RunOutcome::Stopped => "Stopped.",
        RunOutcome::MaxTurns => {
            "Vak reached this run's step limit. Saved work is available; choose Continue to finish this task."
        }
        RunOutcome::Failed => "This run failed. Check the transcript for details.",
    }
}

/// What a client may receive from the live event stream. Every variant here
/// is one the client deliberately renders; anything else `project` drops.
#[derive(Debug, Clone, serde::Serialize)]
pub enum ClientEvent {
    StreamOpened,
    TurnStart {
        turn: usize,
    },
    TextDelta {
        delta: String,
    },
    ThinkingDelta {
        delta: String,
    },
    ToolCallStart {
        id: String,
        name: String,
        args_json: String,
    },
    ToolCallEnd {
        id: String,
        name: String,
        is_error: bool,
        result_preview: Option<String>,
    },
    ApprovalRequested {
        id: String,
        tool: String,
        args_json: String,
        reason: String,
    },
    WorkerStarted {
        label: String,
    },
    WorkerToolCall {
        label: String,
        name: String,
        is_error: bool,
    },
    WorkerFinished {
        label: String,
        is_error: bool,
        elapsed_ms: u64,
    },
    /// A long provider-side backoff is happening. No attempt count, delay or
    /// raw reason: the header shows a neutral "Retrying" state and nothing
    /// more (docs/audits Finding 1 — the previous wire sent the raw provider
    /// error text on every attempt).
    Retrying,
    Sandbox(vak_tools::SandboxEvent),
    /// The runtime sent a text answer back for a redo; the client drops the
    /// discarded draft bubble for this turn rather than showing it as an
    /// answer the runtime itself rejected.
    DraftDiscarded {
        turn: usize,
    },
    RunFinished {
        outcome: RunOutcome,
        message: String,
    },
}

/// The one projection. `None` means: internal bookkeeping, drop it —
/// produces no frame on either the live or the replay path.
pub(crate) fn project(event: AgentEvent) -> Option<ClientEvent> {
    match event {
        AgentEvent::StreamOpened => Some(ClientEvent::StreamOpened),
        AgentEvent::TurnStart { turn } => Some(ClientEvent::TurnStart { turn }),
        AgentEvent::Stream(vak_llm::StreamEvent::TextDelta { delta, .. }) => {
            Some(ClientEvent::TextDelta { delta })
        }
        AgentEvent::Stream(vak_llm::StreamEvent::ThinkingDelta { delta, .. }) => {
            Some(ClientEvent::ThinkingDelta { delta })
        }
        // Start/ToolUseStart/ToolInputDelta/End are provider-stream
        // bookkeeping superseded by ToolCallStart/ToolCallEnd below.
        AgentEvent::Stream(_) => None,
        AgentEvent::ToolCallStart {
            id,
            name,
            args_json,
        } => Some(ClientEvent::ToolCallStart {
            id,
            name,
            args_json,
        }),
        AgentEvent::ToolCallEnd {
            id,
            name,
            is_error,
            result_preview,
        } => Some(ClientEvent::ToolCallEnd {
            id,
            name,
            is_error,
            result_preview,
        }),
        // Session-total usage is not rendered from the live stream (only
        // from the durable transcript's own `usage` field); the loop's own
        // per-step accounting has no client-visible meaning.
        AgentEvent::TurnEnd { .. } => None,
        // Internal continuation bookkeeping for the stop-hook gate.
        AgentEvent::StopHookContinuation { .. } => None,
        AgentEvent::RetryScheduled { .. } => Some(ClientEvent::Retrying),
        // Frozen-ladder leg changes, context compaction and reset-with-
        // handoff are dispatch-contract internals; the run is still working
        // and the header already reflects that via "Working"/"Retrying".
        AgentEvent::RouteFallback { .. } => None,
        AgentEvent::ContextCompacting { .. } => None,
        AgentEvent::ContextCompacted { .. } => None,
        AgentEvent::HandoffReset { .. } => None,
        AgentEvent::ApprovalRequested {
            id,
            tool,
            args_json,
            reason,
        } => Some(ClientEvent::ApprovalRequested {
            id,
            tool,
            args_json,
            reason,
        }),
        AgentEvent::WorkerStarted { label } => Some(ClientEvent::WorkerStarted { label }),
        AgentEvent::WorkerToolCall {
            label,
            name,
            is_error,
        } => Some(ClientEvent::WorkerToolCall {
            label,
            name,
            is_error,
        }),
        // Per-call worker token counts are debug detail, not something the
        // Workbench worker card renders.
        AgentEvent::WorkerUsage { .. } => None,
        AgentEvent::WorkerFinished {
            label,
            is_error,
            elapsed_ms,
        } => Some(ClientEvent::WorkerFinished {
            label,
            is_error,
            elapsed_ms,
        }),
        // Managed-work state is not rendered from the live event stream
        // today (the work panel reads `GET /sessions/:id/work` instead).
        AgentEvent::WorkState { .. } => None,
        AgentEvent::RunFinished { summary, is_error } => {
            let outcome = classify_run_outcome(&summary, is_error);
            Some(ClientEvent::RunFinished {
                outcome,
                message: run_outcome_message(outcome).into(),
            })
        }
        AgentEvent::Sandbox(sandbox_event) => Some(ClientEvent::Sandbox(sandbox_event)),
        AgentEvent::DraftDiscarded { turn } => Some(ClientEvent::DraftDiscarded { turn }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use vak_llm::{AssistantMessage, StreamEvent, Usage};

    fn blank_message() -> AssistantMessage {
        AssistantMessage::empty("test-model")
    }

    /// Every `AgentEvent` variant must be classified exactly once: kept
    /// (with the shape the client expects), or explicitly dropped. This
    /// pins the "typed, never sniffed" contract so a new variant added to
    /// `AgentEvent` cannot silently leak internal traffic to a client
    /// through a catch-all arm.
    #[test]
    fn every_variant_is_kept_or_dropped_on_purpose() {
        let kept = |event: AgentEvent| assert!(project(event).is_some());
        let dropped = |event: AgentEvent| assert!(project(event).is_none());

        kept(AgentEvent::StreamOpened);
        kept(AgentEvent::TurnStart { turn: 1 });
        kept(AgentEvent::Stream(StreamEvent::TextDelta {
            delta: "hi".into(),
            partial: blank_message(),
        }));
        kept(AgentEvent::Stream(StreamEvent::ThinkingDelta {
            delta: "hmm".into(),
            partial: blank_message(),
        }));
        dropped(AgentEvent::Stream(StreamEvent::Start {
            partial: blank_message(),
        }));
        dropped(AgentEvent::Stream(StreamEvent::ToolUseStart {
            index: 0,
            id: "t1".into(),
            name: "read".into(),
            partial: blank_message(),
        }));
        dropped(AgentEvent::Stream(StreamEvent::ToolInputDelta {
            index: 0,
            delta: "{}".into(),
            partial: blank_message(),
        }));
        dropped(AgentEvent::Stream(StreamEvent::End {
            message: blank_message(),
        }));
        kept(AgentEvent::ToolCallStart {
            id: "t1".into(),
            name: "read".into(),
            args_json: "{}".into(),
        });
        kept(AgentEvent::ToolCallEnd {
            id: "t1".into(),
            name: "read".into(),
            is_error: false,
            result_preview: None,
        });
        dropped(AgentEvent::TurnEnd {
            usage: Usage::default(),
        });
        dropped(AgentEvent::StopHookContinuation {
            reason: "internal".into(),
        });
        kept(AgentEvent::RetryScheduled {
            attempt: 1,
            delay_ms: 100,
            reason: "429 from provider".into(),
        });
        dropped(AgentEvent::RouteFallback {
            to_provider: "anthropic".into(),
            to_model: "m".into(),
        });
        dropped(AgentEvent::ContextCompacting {
            estimated_tokens: 10,
        });
        dropped(AgentEvent::ContextCompacted {
            before_tokens: 10,
            after_tokens: 5,
            summarized_turns: 1,
        });
        dropped(AgentEvent::HandoffReset { before_tokens: 10 });
        kept(AgentEvent::ApprovalRequested {
            id: "a1".into(),
            tool: "bash".into(),
            args_json: "{}".into(),
            reason: "writes outside workspace".into(),
        });
        kept(AgentEvent::WorkerStarted {
            label: "worker-1".into(),
        });
        kept(AgentEvent::WorkerToolCall {
            label: "worker-1".into(),
            name: "read".into(),
            is_error: false,
        });
        dropped(AgentEvent::WorkerUsage {
            label: "worker-1".into(),
            input_tokens: 10,
            output_tokens: 5,
        });
        kept(AgentEvent::WorkerFinished {
            label: "worker-1".into(),
            is_error: false,
            elapsed_ms: 10,
        });
        dropped(AgentEvent::WorkState {
            projection: vak_session::work::WorkProjection {
                contract: vak_session::types::WorkContract {
                    contract_id: "c1".into(),
                    revision: 1,
                    source_entry_id: "e1".into(),
                    objective: "test".into(),
                    constraints: Vec::new(),
                    assumptions: Vec::new(),
                    criteria: Vec::new(),
                    items: Vec::new(),
                },
                status: vak_session::types::WorkContractStatus::Active,
                items: Default::default(),
                criteria: Default::default(),
            },
        });
        kept(AgentEvent::RunFinished {
            summary: "completed".into(),
            is_error: false,
        });
        kept(AgentEvent::Sandbox(vak_tools::SandboxEvent::Stdout {
            execution_id: "e1".into(),
            chunk: "hi".into(),
        }));
        kept(AgentEvent::DraftDiscarded { turn: 2 });
    }

    #[test]
    fn text_and_thinking_deltas_never_carry_the_provider_snapshot() {
        let event = AgentEvent::Stream(StreamEvent::TextDelta {
            delta: "partial answer".into(),
            partial: blank_message(),
        });
        let json = serde_json::to_string(&project(event).unwrap()).unwrap();
        assert!(json.contains("partial answer"));
        // The snapshot's model-visible marker must not ride along.
        assert!(!json.contains("test-model"));
        assert!(!json.contains("end_turn"));
    }

    #[test]
    fn run_finished_never_carries_raw_summary_text() {
        for (summary, is_error) in [
            (
                "failed: connection reset by peer at 10.0.0.1:443".to_string(),
                true,
            ),
            ("failed: dispatch ceiling of 28 exhausted".to_string(), true),
            ("aborted".to_string(), false),
            ("max_turns".to_string(), true),
            ("completed".to_string(), false),
        ] {
            let event = AgentEvent::RunFinished {
                summary: summary.clone(),
                is_error,
            };
            let ClientEvent::RunFinished { message, .. } = project(event).unwrap() else {
                panic!("expected RunFinished");
            };
            assert!(
                !message.contains("connection reset")
                    && !message.contains("dispatch ceiling")
                    && !message.contains("10.0.0.1"),
                "leaked raw summary text into {message:?}"
            );
        }
    }

    #[test]
    fn classify_run_outcome_uses_only_documented_sentinels() {
        assert_eq!(classify_run_outcome("aborted", false), RunOutcome::Stopped);
        assert_eq!(
            classify_run_outcome("max_turns", true),
            RunOutcome::MaxTurns
        );
        assert_eq!(
            classify_run_outcome("failed: anything at all", true),
            RunOutcome::Failed
        );
        assert_eq!(
            classify_run_outcome("completed", false),
            RunOutcome::Completed
        );
        // An unrecognised string with is_error unset degrades to Completed
        // rather than fabricating a false-negative failure state.
        assert_eq!(
            classify_run_outcome("some future sentinel", false),
            RunOutcome::Completed
        );
    }

    #[test]
    fn retry_carries_no_attempt_count_delay_or_reason() {
        let event = AgentEvent::RetryScheduled {
            attempt: 7,
            delay_ms: 60_000,
            reason: "internal provider detail".into(),
        };
        let json = serde_json::to_string(&project(event).unwrap()).unwrap();
        assert_eq!(json, "\"Retrying\"");
    }
}
