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
/// `AgentEvent::RunFinished` carries a typed runtime outcome. Its summary is
/// internal bookkeeping that can contain provider error details, so it is
/// discarded here rather than parsed or sent to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum RunOutcome {
    Completed,
    Stopped,
    MaxTurns,
    Failed,
}

/// Project the runtime's typed outcome without examining internal summary
/// text.
pub(crate) fn classify_run_outcome(outcome: vak_agent::AgentRunOutcome) -> RunOutcome {
    match outcome {
        vak_agent::AgentRunOutcome::Completed => RunOutcome::Completed,
        vak_agent::AgentRunOutcome::Stopped => RunOutcome::Stopped,
        vak_agent::AgentRunOutcome::MaxTurns => RunOutcome::MaxTurns,
        vak_agent::AgentRunOutcome::Failed => RunOutcome::Failed,
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
            "Vakyartha reached this run's step limit. Saved work is available; choose Continue to finish this task."
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
        directive_id: Option<String>,
        request_id: Option<String>,
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
    /// A card call has started; the client draws an empty shell of `block`
    /// until the call ends (docs/design/96-answer-projection-gaps.md, Q2).
    CardDrafting {
        id: String,
        block: String,
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
    /// A worker is waiting on an answer; answer it at
    /// `POST /sessions/{id}/questions/{id}`.
    WorkerQuestion {
        id: String,
        label: String,
        question: String,
        options: Vec<String>,
    },
    WorkerQuestionAnswered {
        id: String,
    },
    /// The wait ended with no answer (expired, skipped, withdrawn).
    WorkerQuestionClosed {
        id: String,
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
        AgentEvent::TurnStart {
            turn,
            directive_id,
            request_id,
        } => Some(ClientEvent::TurnStart {
            turn,
            directive_id,
            request_id,
        }),
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
        AgentEvent::CardDrafting { id, block } => Some(ClientEvent::CardDrafting { id, block }),
        // Recorded cards reach a client through the presentation stream.
        AgentEvent::CardRecorded { .. } | AgentEvent::ProgressUpdated { .. } => None,
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
        AgentEvent::WorkerQuestion {
            id,
            label,
            question,
            options,
        } => Some(ClientEvent::WorkerQuestion {
            id,
            label,
            question,
            options,
        }),
        AgentEvent::WorkerQuestionAnswered { id, .. } => {
            Some(ClientEvent::WorkerQuestionAnswered { id })
        }
        AgentEvent::WorkerQuestionClosed { id, .. } => {
            Some(ClientEvent::WorkerQuestionClosed { id })
        }
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
        AgentEvent::RunFinished { outcome, .. } => {
            let outcome = classify_run_outcome(outcome);
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
        kept(AgentEvent::TurnStart {
            turn: 1,
            directive_id: Some("directive-1".into()),
            request_id: Some("request-1".into()),
        });
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
        kept(AgentEvent::CardDrafting {
            id: "t2".into(),
            block: "table".into(),
        });
        dropped(AgentEvent::CardRecorded {
            tool_use_id: "t2".into(),
            presentation_id: "p2".into(),
            record: Box::new(vak_session::types::PresentationRecord {
                placement: Default::default(),
                turn_id: "u1".into(),
                source: vak_session::types::PresentationSource::ToolCall {
                    tool_use_id: "t2".into(),
                },
                semantic_type: "table".into(),
                skill_id: String::new(),
                skill_version: String::new(),
                schema_version: 2,
                payload: serde_json::json!({}),
                payload_digest: String::new(),
                derived_from: Vec::new(),
                title: String::new(),
                identity_digest: String::new(),
                block: None,
                data_call_id: None,
            }),
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
            outcome: vak_agent::AgentRunOutcome::Completed,
            summary: "completed".into(),
        });
        kept(AgentEvent::Sandbox(vak_tools::SandboxEvent::Stdout {
            execution_id: "e1".into(),
            chunk: "hi".into(),
        }));
        kept(AgentEvent::DraftDiscarded { turn: 2 });
    }

    #[test]
    fn a_drafting_card_projects_its_call_and_form_and_nothing_else() {
        let json = serde_json::to_value(
            project(AgentEvent::CardDrafting {
                id: "call-7".into(),
                block: "table".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"CardDrafting": {"id": "call-7", "block": "table"}})
        );
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
        for (outcome, summary) in [
            (
                vak_agent::AgentRunOutcome::Failed,
                "failed: connection reset by peer at 10.0.0.1:443",
            ),
            (
                vak_agent::AgentRunOutcome::Failed,
                "failed: dispatch ceiling of 28 exhausted",
            ),
            (vak_agent::AgentRunOutcome::Stopped, "arbitrary stop detail"),
            (
                vak_agent::AgentRunOutcome::MaxTurns,
                "internal ceiling detail",
            ),
            (vak_agent::AgentRunOutcome::Completed, "completed"),
        ] {
            let event = AgentEvent::RunFinished {
                outcome,
                summary: summary.to_string(),
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
    fn typed_run_outcomes_project_to_the_closed_client_vocabulary() {
        for (agent, client, encoded) in [
            (
                vak_agent::AgentRunOutcome::Completed,
                RunOutcome::Completed,
                "\"Completed\"",
            ),
            (
                vak_agent::AgentRunOutcome::Stopped,
                RunOutcome::Stopped,
                "\"Stopped\"",
            ),
            (
                vak_agent::AgentRunOutcome::MaxTurns,
                RunOutcome::MaxTurns,
                "\"MaxTurns\"",
            ),
            (
                vak_agent::AgentRunOutcome::Failed,
                RunOutcome::Failed,
                "\"Failed\"",
            ),
        ] {
            assert_eq!(classify_run_outcome(agent), client);
            assert_eq!(serde_json::to_string(&client).unwrap(), encoded);
        }
    }

    #[test]
    fn serialized_client_events_match_the_typescript_contract_fixture() {
        use vak_tools::SandboxEvent;

        fn assert_variant_is_covered(event: &ClientEvent) {
            match event {
                ClientEvent::StreamOpened
                | ClientEvent::TurnStart { .. }
                | ClientEvent::TextDelta { .. }
                | ClientEvent::ThinkingDelta { .. }
                | ClientEvent::ToolCallStart { .. }
                | ClientEvent::CardDrafting { .. }
                | ClientEvent::ToolCallEnd { .. }
                | ClientEvent::ApprovalRequested { .. }
                | ClientEvent::WorkerStarted { .. }
                | ClientEvent::WorkerToolCall { .. }
                | ClientEvent::WorkerFinished { .. }
                | ClientEvent::WorkerQuestion { .. }
                | ClientEvent::WorkerQuestionAnswered { .. }
                | ClientEvent::WorkerQuestionClosed { .. }
                | ClientEvent::Retrying
                | ClientEvent::Sandbox(_)
                | ClientEvent::DraftDiscarded { .. }
                | ClientEvent::RunFinished { .. } => {}
            }
        }

        let events = vec![
            ClientEvent::StreamOpened,
            ClientEvent::TurnStart {
                turn: 1,
                directive_id: Some("directive-1".into()),
                request_id: Some("request-1".into()),
            },
            ClientEvent::TextDelta {
                delta: "text".into(),
            },
            ClientEvent::ThinkingDelta {
                delta: "thinking".into(),
            },
            ClientEvent::ToolCallStart {
                id: "call".into(),
                name: "read".into(),
                args_json: "{}".into(),
            },
            ClientEvent::CardDrafting {
                id: "call".into(),
                block: "table".into(),
            },
            ClientEvent::ToolCallEnd {
                id: "call".into(),
                name: "read".into(),
                is_error: false,
                result_preview: None,
            },
            ClientEvent::ApprovalRequested {
                id: "approval".into(),
                tool: "bash".into(),
                args_json: "{}".into(),
                reason: "review".into(),
            },
            ClientEvent::WorkerStarted {
                label: "worker".into(),
            },
            ClientEvent::WorkerToolCall {
                label: "worker".into(),
                name: "read".into(),
                is_error: false,
            },
            ClientEvent::WorkerFinished {
                label: "worker".into(),
                is_error: false,
                elapsed_ms: 1,
            },
            ClientEvent::WorkerQuestion {
                id: "question".into(),
                label: "worker".into(),
                question: "Choose".into(),
                options: vec!["yes".into()],
            },
            ClientEvent::WorkerQuestionAnswered {
                id: "question".into(),
            },
            ClientEvent::WorkerQuestionClosed {
                id: "question".into(),
            },
            ClientEvent::Retrying,
            ClientEvent::Sandbox(SandboxEvent::Stdout {
                execution_id: "exec".into(),
                chunk: "output".into(),
            }),
            ClientEvent::DraftDiscarded { turn: 1 },
            ClientEvent::RunFinished {
                outcome: RunOutcome::MaxTurns,
                message: "Stopped at the step limit.".into(),
            },
        ];
        for event in &events {
            assert_variant_is_covered(event);
        }

        let serialized: Vec<serde_json::Value> = events
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect();
        let fixture: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../vak-client-ui/tests/fixtures/client-events.json"
        ))
        .unwrap();
        assert_eq!(serialized, fixture);
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
