#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{
    Agent, AgentConfig, ApprovalMode, AutoApprove, SteeringQueues, TurnOutcome, WorkMode,
};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider, WorkPurpose};
use vak_session::SessionLog;
use vak_session::types::{
    CapabilityDescriptor, CapabilityInvocation, CapabilityKind, FrozenContract, SessionHeader,
};
use vak_tools::Tool;
use vak_tools::bash::BashTool;
use vak_tools::write::WriteTool;

enum ScriptedResponse {
    Message(AssistantMessage),
    Error(LlmError),
}

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(request);
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(ScriptedResponse::Message(m)) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            Some(ScriptedResponse::Error(e)) => sink.close_error(e).await,
            None => {
                sink.close_error(LlmError::Parse("script exhausted".into()))
                    .await
            }
        }
        Ok(rx)
    }
}
fn assistant_text(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(text)],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
    }
}

fn tool_call_msg(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

struct Harness {
    agent: Agent,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    events_tx: mpsc::Sender<vak_agent::AgentEvent>,
    _events_rx: mpsc::Receiver<vak_agent::AgentEvent>,
}

fn harness(responses: Vec<ScriptedResponse>, tools: Vec<Arc<dyn Tool>>) -> Harness {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-test".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: vec![CapabilityDescriptor {
                name: "code-task".into(),
                kind: CapabilityKind::Skill,
                invocation: CapabilityInvocation::SkillLoader,
                description: "test skill".into(),
                source: None,
                digest: None,
                provenance: None,
                configuration: serde_json::Value::Null,
            }],
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = tools;
    let (tx, rx) = mpsc::channel(4096);
    let requests: Arc<Mutex<Vec<ChatRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::capacity_key(),
        responses: Mutex::new(responses.into_iter().collect()),
        requests: requests.clone(),
    });
    std::mem::forget(dir);
    Harness {
        agent: Agent::new(provider, log, cfg),
        requests,
        events_tx: tx,
        _events_rx: rx,
    }
}

fn next_tool_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static C: AtomicU32 = AtomicU32::new(0);
    format!("t{}", C.fetch_add(1, Ordering::Relaxed))
}

#[tokio::test]
async fn single_turn_no_tools_completes() {
    let mut h = harness(
        vec![ScriptedResponse::Message(assistant_text("done"))],
        vec![],
    );
    let outcome = h
        .agent
        .run(
            "hello",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => assert_eq!(response.text_content(), "done"),
        other => panic!("expected completed, got {other:?}"),
    }
    let session = h.agent.session.lock().await;
    assert_eq!(session.derive_messages().len(), 2);
}

#[tokio::test]
async fn outcome_turn_cap_counts_model_calls_not_tool_round_trips() {
    let mut h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg(
                "tool-1",
                "bash",
                serde_json::json!({"command": "printf tool"}),
            )),
            ScriptedResponse::Message(assistant_text("done")),
        ],
        vec![Arc::new(BashTool)],
    );
    h.agent.config.mode = vak_permission::Mode::FullAccess;
    h.agent.config.approval_mode = ApprovalMode::AutoApprove;
    h.agent.config.approver = Some(Arc::new(AutoApprove));
    h.agent.config.outcome = Some(vak_intent::OutcomeSpec {
        schema_version: 1,
        revision: 0,
        objective: "complete one tool-assisted answer".into(),
        assumptions: Vec::new(),
        requirements: Vec::new(),
        resolver_version: 1,
        evidence_max_age_secs: None,
        acts: Default::default(),
        stop: Default::default(),
        max_turns: Some(2),
    });

    let outcome = h
        .agent
        .run(
            "run the tool",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;

    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(h.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn queued_outcome_revision_is_applied_and_logged_before_dispatch() {
    let mut h = harness(
        vec![ScriptedResponse::Message(assistant_text("done"))],
        vec![],
    );
    let steering = SteeringQueues::new();
    let intent = vak_intent::Intent::general(1);
    steering.push_outcome_update(vak_session::types::IntentRecord {
        reading: intent.reading,
        engagement: intent.engagement,
        provenance: intent.provenance,
        outcome: Some(vak_intent::OutcomeSpec {
            schema_version: 1,
            revision: 7,
            objective: "updated objective".into(),
            assumptions: Vec::new(),
            requirements: Vec::new(),
            resolver_version: 1,
            evidence_max_age_secs: None,
            acts: Default::default(),
            stop: Default::default(),
            max_turns: Some(1),
        }),
        model_visible: None,
        commitment_id: None,
        strands: Vec::new(),
        strand_commitments: Default::default(),
    });

    let outcome = h
        .agent
        .run(
            "continue",
            &steering,
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = h.agent.session.lock().await;
    let revision = session.chain_to_root().iter().rev().find_map(|entry| {
        if let vak_session::EntryPayload::Intent(record) = &entry.payload {
            record.outcome.as_ref().map(|outcome| outcome.revision)
        } else {
            None
        }
    });
    assert_eq!(revision, Some(7));
}

#[tokio::test]
async fn managed_turn_authors_and_persists_validated_contract() {
    let authored = serde_json::json!({
        "objective": "make the change",
        "constraints": [],
        "assumptions": [],
        "criteria": [{
            "criterion_id": "change",
            "statement": "the change tool completed",
            "kind": {"kind": "tool_succeeded", "tool": "bash"},
            "required": true
        }],
        "items": [{
            "item_id": "change",
            "title": "Make the change",
            "instructions": "make it",
            "dependencies": [],
            "owner": "parent_agent",
            "required": true,
            "readonly": false,
            "path_claims": [],
            "criterion_ids": ["change"]
        }]
    });
    let mut h = harness(
        vec![
            ScriptedResponse::Message(assistant_text(&authored.to_string())),
            ScriptedResponse::Message(tool_call_msg(
                "b1",
                "bash",
                serde_json::json!({"command": "true"}),
            )),
            ScriptedResponse::Message(assistant_text("completed")),
        ],
        vec![Arc::new(BashTool)],
    );
    h.agent.config.work_mode = WorkMode::Managed;
    let outcome = h
        .agent
        .run(
            "please make the change",
            &SteeringQueues::new(),
            CancellationToken::new(),
            h.events_tx,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = h.agent.into_session().await;
    let projection = session.work_projection().unwrap().unwrap();
    assert_eq!(projection.contract.objective, "make the change");
    assert_eq!(
        projection.status,
        vak_session::types::WorkContractStatus::Completed
    );
    assert_eq!(
        projection.items["change"].status,
        vak_session::types::WorkItemStatus::Succeeded
    );
    assert_eq!(session.receipts()[0].purpose, WorkPurpose::Plan);
}

#[tokio::test]
async fn managed_semantic_criterion_uses_the_independent_judge() {
    let authored = serde_json::json!({
        "objective": "make the change",
        "constraints": [],
        "assumptions": [],
        "criteria": [{
            "criterion_id": "review",
            "statement": "the requested change is complete",
            "kind": {"kind": "semantic"},
            "required": true
        }],
        "items": [{
            "item_id": "change",
            "title": "Make the change",
            "instructions": "make it",
            "dependencies": [],
            "owner": "parent_agent",
            "required": true,
            "readonly": false,
            "path_claims": [],
            "criterion_ids": ["review"]
        }]
    });
    let judge = serde_json::json!({
        "results": [{
            "criterion": "[review] the requested change is complete",
            "verdict": "pass",
            "evidence": "the tool result and final response agree"
        }]
    });
    let mut h = harness(
        vec![
            ScriptedResponse::Message(assistant_text(&authored.to_string())),
            ScriptedResponse::Message(tool_call_msg(
                "b1",
                "bash",
                serde_json::json!({"command": "true"}),
            )),
            ScriptedResponse::Message(assistant_text("done")),
            ScriptedResponse::Message(assistant_text(&judge.to_string())),
        ],
        vec![Arc::new(BashTool)],
    );
    h.agent.config.work_mode = WorkMode::Managed;
    let outcome = h
        .agent
        .run(
            "please make the change",
            &SteeringQueues::new(),
            CancellationToken::new(),
            h.events_tx,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = h.agent.into_session().await;
    let projection = session.work_projection().unwrap().unwrap();
    assert_eq!(
        projection.criteria["review"],
        vak_session::types::CriterionResult::Passed {
            evidence: "the tool result and final response agree".into()
        }
    );
    assert_eq!(
        projection.status,
        vak_session::types::WorkContractStatus::Completed
    );
}

#[tokio::test]
async fn normalizes_steering_before_it_reaches_the_ledger_or_provider() {
    let mut h = harness(
        vec![ScriptedResponse::Message(assistant_text("done"))],
        vec![],
    );
    h.agent.config.input_normalizer = Some(Arc::new(|mut message| {
        if let Some(ContentBlock::Text { text }) = message
            .content
            .iter_mut()
            .find(|block| matches!(block, ContentBlock::Text { .. }))
        {
            *text = format!("normalized: {text}");
        }
        Ok(message)
    }));
    let steering = vak_agent::SteeringQueues::new();
    steering.push_steering("queued command");
    let outcome = h
        .agent
        .run(
            "initial",
            &steering,
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let request = h.requests.lock().unwrap().first().cloned().unwrap();
    assert!(
        request
            .messages
            .iter()
            .any(|message| message.text_content() == "normalized: queued command")
    );
    let session = h.agent.session.lock().await;
    assert!(
        session
            .derive_messages()
            .iter()
            .any(|message| message.text_content() == "normalized: queued command")
    );
}

#[tokio::test]
async fn tool_roundtrip_executes_and_feeds_result_back() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg(
                &next_tool_id(),
                "bash",
                serde_json::json!({"command": "echo ran"}),
            )),
            ScriptedResponse::Message(assistant_text("finished")),
        ],
        vec![Arc::new(BashTool)],
    );
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "run it",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let (req_count, has_tool_result) = {
        let reqs = h.requests.lock().unwrap();
        let hit = reqs.get(1).map(|r| {
            r.messages.iter().any(|m| {
                m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolResult { content, is_error: false, .. } if content.contains("ran")))
            })
        }).unwrap_or(false);
        (reqs.len(), hit)
    };
    assert_eq!(req_count, 2);
    assert!(
        has_tool_result,
        "second request must contain the tool result"
    );

    let session = agent.session.lock().await;
    // Four raw entries on the ledger (directive, tool_use, tool_result,
    // final answer); the turn is now closed, so the model-visible
    // projection is its full record: the same four messages with the
    // result digested and thinking dropped
    // (docs/design/68-context-engine.md §10).
    assert_eq!(session.message_chain().len(), 4);
    let projected = session.derive_messages();
    assert_eq!(projected.len(), 4);
    assert!(projected.iter().any(|m| m.content.iter().any(|b| matches!(
        b,
        ContentBlock::ToolResult { content, .. } if content.contains("[evidence:")
    ))));
}

#[tokio::test]
async fn malformed_tool_input_is_rejected_by_the_admitted_schema() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg("t-schema", "write", serde_json::json!({}))),
            ScriptedResponse::Message(assistant_text("recovered")),
        ],
        vec![Arc::new(WriteTool)],
    );
    let mut agent = h.agent;
    // About what the failed call recorded, not whether "recovered" may end
    // the turn: the stop gate is tested in stop_guard.rs.
    agent.config.stop_policy = None;
    let outcome = agent
        .run(
            "write it",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = agent.session.lock().await;
    // Raw ledger, not the model-visible projection: once the turn closes,
    // a closed turn's tool results collapse into trace lines
    // (docs/design/68-context-engine.md §10) — the mechanics under test
    // here are what got recorded, not how a later turn would see it.
    let result = session
        .message_chain()
        .iter()
        .flat_map(|(_, message)| message.content.iter())
        .find_map(|block| match block {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("schema rejection must be recorded as a tool result");
    assert!(result.contains("required parameter `path` was omitted"));
}

#[tokio::test]
async fn unknown_tool_becomes_error_value_not_crash() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg("t1", "nonexistent", serde_json::json!({}))),
            ScriptedResponse::Message(assistant_text("recovered")),
        ],
        vec![Arc::new(BashTool)],
    );
    let mut agent = h.agent;
    agent.config.stop_policy = None;
    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = agent.session.lock().await;
    // Raw ledger: the closed turn's tool result is a trace line in the
    // projection now, not a raw block (docs/design/68-context-engine.md
    // §10) — this test is about what the tool actually recorded.
    let msgs = session.message_chain();
    let results_block = msgs
        .iter()
        .rev()
        .flat_map(|(_, m)| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("a tool result must exist");
    assert!(results_block.contains("unknown_capability"));
    assert!(results_block.contains("available_tools"));
}

#[tokio::test]
async fn skill_name_tool_call_returns_typed_loader_recovery() {
    let mut h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg("t1", "code-task", serde_json::json!({}))),
            ScriptedResponse::Message(assistant_text("recovered")),
        ],
        vec![Arc::new(BashTool)],
    );
    h.agent.config.stop_policy = None;
    let outcome = h
        .agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = h.agent.session.lock().await;
    // Raw ledger: see the comment on `unknown_tool_becomes_error_value_not_crash`.
    let result = session
        .message_chain()
        .iter()
        .flat_map(|(_, m)| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("a tool result must exist");
    assert!(result.contains("capability_kind_mismatch"));
    assert!(result.contains("\"tool\":\"skill\""));
    assert!(result.contains("\"name\":\"code-task\""));
}

#[tokio::test]
async fn parallel_tools_preserve_source_order() {
    let msg = AssistantMessage {
        content: vec![
            ContentBlock::ToolUse {
                id: "a".into(),
                name: "write".into(),
                input: serde_json::json!({"path": "a.txt", "content": "A"}),
            },
            ContentBlock::ToolUse {
                id: "b".into(),
                name: "write".into(),
                input: serde_json::json!({"path": "b.txt", "content": "B"}),
            },
        ],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    };
    let h = harness(
        vec![
            ScriptedResponse::Message(msg),
            ScriptedResponse::Message(assistant_text("both done")),
        ],
        vec![Arc::new(WriteTool), Arc::new(BashTool)],
    );
    let mut h = h;
    h.agent.config.parallel_tools = true;
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "fan out",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.session.lock().await;
    // Raw ledger: the closed turn's results are trace lines in the
    // projection now (docs/design/68-context-engine.md §10); source order
    // is a property of what was recorded, checked here against the raw
    // entries.
    let msgs: Vec<vak_llm::types::Message> = session
        .message_chain()
        .into_iter()
        .map(|(_, m)| m)
        .collect();
    let results_msg = msgs
        .iter()
        .rev()
        .find(|m| matches!(m.content.first(), Some(ContentBlock::ToolResult { .. })))
        .expect("tool results message must exist");
    assert_eq!(results_msg.content.len(), 2);
    if let ContentBlock::ToolResult { tool_use_id, .. } = &results_msg.content[0] {
        assert_eq!(
            tool_use_id, "a",
            "results must follow assistant source order"
        );
    } else {
        panic!("expected tool result block");
    }
}

#[tokio::test]
async fn abort_mid_stream_returns_partial_and_persists_it() {
    let h = harness(
        vec![ScriptedResponse::Error(LlmError::Aborted {
            partial: Some(Box::new(assistant_text("partial work"))),
        })],
        vec![],
    );
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "start",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    match outcome {
        TurnOutcome::Aborted { partial } => {
            assert_eq!(partial.unwrap().text_content(), "partial work");
        }
        other => panic!("expected aborted, got {other:?}"),
    }
    let session = agent.session.lock().await;
    assert_eq!(
        session.derive_messages().last().unwrap().text_content(),
        "partial work"
    );
}

#[tokio::test]
async fn max_turns_guard_stops_runaway_loops() {
    let responses: Vec<_> = (0..6)
        .map(|_| {
            ScriptedResponse::Message(tool_call_msg(
                &next_tool_id(),
                "bash",
                serde_json::json!({"command": "true"}),
            ))
        })
        .collect();
    let mut h = harness(responses, vec![Arc::new(BashTool)]);
    h.agent.config.max_turns = 3;
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "loop forever",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::MaxTurnsReached));
}

#[tokio::test]
async fn projection_invariant_every_request_message_is_logged() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg(
                &next_tool_id(),
                "bash",
                serde_json::json!({"command": "echo x"}),
            )),
            ScriptedResponse::Message(assistant_text("ok")),
        ],
        vec![Arc::new(BashTool)],
    );
    let mut agent = h.agent;
    let _ = agent
        .run(
            "invariant check",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;

    // Raw ledger, not the model-visible projection: once this turn closes,
    // `derive_messages()` collapses it to its two-message full record
    // (docs/design/68-context-engine.md §10) and no longer contains the
    // raw tool_use/tool_result blocks a mid-turn request carried — those
    // are still reconstructable (a trace line + `SessionLog::evidence`),
    // just not via byte-identical containment in the CURRENT projection.
    // Invariant 1 ("model-visible means logged") is checked here against
    // what was actually appended to the ledger.
    let raw_msgs: Vec<vak_llm::types::Message> = agent
        .session
        .lock()
        .await
        .message_chain()
        .into_iter()
        .map(|(_, m)| m)
        .collect();
    for req in h.requests.lock().unwrap().iter() {
        for msg in &req.messages {
            assert!(
                raw_msgs.contains(msg),
                "model-visible means logged: request message missing from the ledger"
            );
        }
    }
}

/// Invariant 1: the host-supplied tail (clock and stance) reaches the model
/// only after its exact bytes are in the ledger.
#[tokio::test]
async fn the_turn_context_the_model_reads_is_recorded_first() {
    let mut h = harness(
        vec![ScriptedResponse::Message(assistant_text("hi"))],
        vec![],
    );
    h.agent.config.tail = vak_agent::TailInput {
        temporal: "Current time: 2026-09-30 10:00 UTC.".into(),
        stance: "Answer directly.".into(),
    };
    let outcome = h
        .agent
        .run(
            "hello",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = h.agent.session.lock().await;
    let recorded = session
        .chain_to_root()
        .iter()
        .find_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Activity(activity)
                if activity.data.get("section").map(String::as_str)
                    == Some(SessionLog::TURN_CONTEXT_SECTION) =>
            {
                Some(activity.clone())
            }
            _ => None,
        })
        .expect("turn context recorded");
    assert_eq!(
        recorded.detail.as_deref(),
        Some("Current time: 2026-09-30 10:00 UTC.")
    );
    assert_eq!(
        recorded.data.get("stance").map(String::as_str),
        Some("Answer directly.")
    );
    drop(session);
    let request = h.requests.lock().unwrap()[0].clone();
    let sent = request
        .messages
        .iter()
        .flat_map(|m| m.content.iter())
        .any(|block| matches!(block, ContentBlock::Text { text } if text.contains("Current time: 2026-09-30 10:00 UTC.")));
    assert!(sent, "the recorded text is what the model received");
}

/// A real turn's calls leave call effects naming the files they touched,
/// with digests and the turn (docs/design/85-turn-graph.md, G0): a declared
/// write and read, and a file bash created, found by its own scan.
#[tokio::test]
async fn a_turns_file_calls_record_their_effects() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg(
                "call-write",
                "write",
                serde_json::json!({"path": "notes.txt", "content": "hello"}),
            )),
            ScriptedResponse::Message(tool_call_msg(
                "call-read",
                "read",
                serde_json::json!({"path": "notes.txt"}),
            )),
            ScriptedResponse::Message(tool_call_msg(
                "call-bash",
                "bash",
                serde_json::json!({"command": "printf made > made.txt"}),
            )),
            ScriptedResponse::Message(assistant_text("done")),
        ],
        vec![
            Arc::new(WriteTool),
            Arc::new(vak_tools::read::ReadTool),
            Arc::new(BashTool),
        ],
    );
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "make the notes",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let turn = session.latest_directive_entry_id().unwrap();
    let effects: Vec<(String, vak_session::types::CallEffect, Option<String>)> = session
        .chain_to_root()
        .into_iter()
        .filter_map(|entry| match &entry.payload {
            vak_session::EntryPayload::CallEffect(record) => Some((
                record.tool_use_id.clone(),
                record.effect.clone(),
                entry.at_turn.clone(),
            )),
            _ => None,
        })
        .collect();
    use vak_session::types::CallEffect;
    let digest = |call: &str| {
        effects.iter().find_map(|(id, effect, _)| match effect {
            CallEffect::FileRead { digest, .. } | CallEffect::FileWrite { digest, .. }
                if id == call =>
            {
                digest.clone()
            }
            _ => None,
        })
    };
    assert!(effects.iter().any(|(id, effect, _)| id == "call-write"
        && matches!(effect, CallEffect::FileWrite { path, .. } if path == "notes.txt")));
    assert!(effects.iter().any(|(id, effect, _)| id == "call-read"
        && matches!(effect, CallEffect::FileRead { path, .. } if path == "notes.txt")));
    assert!(effects.iter().any(|(id, effect, _)| id == "call-bash"
        && matches!(effect, CallEffect::FileWrite { path, .. } if path == "made.txt")));
    assert!(digest("call-write").is_some());
    assert_eq!(
        digest("call-write"),
        digest("call-read"),
        "same content read back"
    );
    assert!(
        effects
            .iter()
            .all(|(_, _, at)| at.as_deref() == Some(turn.as_str())),
        "{effects:?}"
    );
}
