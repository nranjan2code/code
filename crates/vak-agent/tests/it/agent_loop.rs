#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
    readonly_on_stream: Mutex<Option<(std::path::PathBuf, Arc<tokio::sync::Mutex<SessionLog>>)>>,
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
        let readonly_on_stream = self.readonly_on_stream.lock().unwrap().take();
        if let Some((path, session)) = readonly_on_stream {
            *session.lock().await = SessionLog::open_read_only(path).unwrap();
        }
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
        applied: None,
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
        applied: None,
    }
}

struct Harness {
    agent: Agent,
    provider: Arc<Scripted>,
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
                serves: Vec::new(),
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
        readonly_on_stream: Mutex::new(None),
    });
    std::mem::forget(dir);
    Harness {
        agent: Agent::new(provider.clone(), log, cfg),
        provider,
        requests,
        events_tx: tx,
        _events_rx: rx,
    }
}

struct DispatchProbe(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl Tool for DispatchProbe {
    fn name(&self) -> &str {
        "probe"
    }

    fn description(&self) -> &str {
        "A test tool that records whether dispatch occurred."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &vak_tools::ToolContext,
    ) -> vak_tools::ToolOutput {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        vak_tools::ToolOutput::default()
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
async fn required_session_record_failure_stops_tool_dispatch() {
    let probe = Arc::new(DispatchProbe(std::sync::atomic::AtomicUsize::new(0)));
    let mut h = harness(
        vec![ScriptedResponse::Message(tool_call_msg(
            "probe-1",
            "probe",
            serde_json::json!({}),
        ))],
        vec![probe.clone()],
    );
    let session_path = h.agent.session.lock().await.path().to_path_buf();
    *h.provider.readonly_on_stream.lock().unwrap() = Some((session_path, h.agent.session.clone()));

    let outcome = h
        .agent
        .run(
            "call probe",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;

    assert!(
        matches!(&outcome, TurnOutcome::Failed { error } if error.to_string().contains("write failed")),
        "a required session-record failure must fail the turn: {outcome:?}"
    );
    assert_eq!(probe.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(h.requests.lock().unwrap().len(), 1);
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
        ContentBlock::ToolResult { content, .. } if content.contains("[evidence ev:")
    ))));
}

#[tokio::test]
async fn malformed_tool_input_is_rejected_by_the_admitted_schema() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg("t-schema", "write", serde_json::json!({}))),
            // The regeneration under the schema is refused too, so the
            // call runs as written and the admitted schema rejects it.
            ScriptedResponse::Message(assistant_text("{}")),
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
    assert!(result.contains("arguments is missing required `path`"));
}

#[tokio::test]
async fn provider_tool_call_format_failure_gets_one_same_turn_repair() {
    let h = harness(
        vec![
            ScriptedResponse::Error(LlmError::ToolCallFormat(
                "XML syntax error in generated tool call".into(),
            )),
            ScriptedResponse::Message(assistant_text("recovered answer")),
        ],
        vec![Arc::new(BashTool)],
    );
    let mut agent = h.agent;
    agent.config.stop_policy = None;
    let outcome = agent
        .run(
            "answer this request",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;

    assert!(matches!(
        outcome,
        TurnOutcome::Completed { response } if response.text_content() == "recovered answer"
    ));
    {
        let requests = h.requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "one repair dispatch follows the error");
        assert_eq!(requests[1].messages.len(), requests[0].messages.len() + 1);
        assert!(
            requests[1].messages.iter().any(|message| {
                message.content.iter().any(|block| {
                matches!(block, ContentBlock::Text { text } if text.contains("[tool-call-repair]"))
            })
            }),
            "repair directive must be included in the second request"
        );
    }

    let session = agent.session.lock().await;
    let records = session.message_chain();
    assert!(records.iter().any(|(_, message)| {
        message.content.iter().any(|block| {
            matches!(block, ContentBlock::Text { text } if text.contains("[tool-call-repair]"))
        })
    }));
    assert!(
        !records.iter().any(|(_, message)| message
            .content
            .iter()
            .any(|block| { matches!(block, ContentBlock::ToolResult { .. }) })),
        "the malformed envelope must not execute a tool"
    );
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
    assert!(result.contains("registered as a skill, not a callable tool"));
    assert!(result.contains("call `skill` with arguments `{\"name\":\"code-task\"}`"));
    assert!(result.contains("Do not retry `code-task` as a tool"));
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
        applied: None,
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
async fn abort_partial_survives_an_unwritable_history_with_a_visible_marker() {
    let mut h = harness(
        vec![ScriptedResponse::Error(LlmError::Aborted {
            partial: Some(Box::new(assistant_text("partial work"))),
        })],
        vec![],
    );
    let session_path = h.agent.session.lock().await.path().to_path_buf();
    *h.provider.readonly_on_stream.lock().unwrap() = Some((session_path, h.agent.session.clone()));

    let outcome = h
        .agent
        .run(
            "start",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    let TurnOutcome::Aborted {
        partial: Some(partial),
    } = outcome
    else {
        panic!("abort should retain partial output: {outcome:?}");
    };
    let text = partial.text_content();
    assert!(text.contains("partial work"), "{text}");
    assert!(
        text.contains("could not be saved to conversation history"),
        "{text}"
    );
    let session = h.agent.session.lock().await;
    assert!(
        !session
            .derive_messages()
            .iter()
            .any(|message| message.text_content().contains("partial work")),
        "failed history append must not claim the partial was persisted"
    );
}

#[tokio::test]
async fn successful_step_receipt_failure_withholds_assistant_success() {
    let mut h = harness(
        vec![ScriptedResponse::Message(assistant_text(
            "answer not recorded",
        ))],
        vec![],
    );
    let session_path = h.agent.session.lock().await.path().to_path_buf();
    *h.provider.readonly_on_stream.lock().unwrap() = Some((session_path, h.agent.session.clone()));

    let outcome = h
        .agent
        .run(
            "answer this",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;

    assert!(
        matches!(&outcome, TurnOutcome::Failed { error } if error.to_string().contains("work receipt write failed")),
        "a step without its work receipt must not complete: {outcome:?}"
    );
    assert!(
        !h.agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .any(|(_, message)| message.text_content().contains("answer not recorded")),
        "the assistant success must not be appended after its receipt failed"
    );
}

#[tokio::test]
async fn aggregate_model_call_limit_stops_retries_without_endurance_spinning() {
    let h = harness(
        vec![ScriptedResponse::Error(LlmError::Network(
            "temporary".into(),
        ))],
        vec![],
    );
    let mut agent = h.agent;
    agent.config.max_retries = 3;
    agent.config.run_retry_attempts = 5;
    agent.config.model_call_budget = Some(Arc::new(vak_agent::ModelCallBudget::new(
        vak_session::ids::RunId::new(),
        1,
    )));

    let outcome = agent
        .run(
            "make one call",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;

    assert!(
        matches!(&outcome, TurnOutcome::Failed { error } if error.to_string().contains("aggregate model-call limit of 1 exhausted")),
        "budget exhaustion is permanent and stops retries: {outcome:?}"
    );
    assert_eq!(h.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn fence_card_write_failure_does_not_deliver_a_successful_turn() {
    let fence =
        "Here's the chart.\n\n```vak\n{\"semantic_type\":\"chart\",\"title\":\"Evidence\"}\n```";
    let mut h = harness(
        vec![ScriptedResponse::Message(assistant_text(fence))],
        vec![],
    );
    let path = h.agent.session.lock().await.path().to_path_buf();
    let session = h.agent.session.clone();
    h.agent.config.presentation_rebuild = Some(Arc::new(move |_name, input| {
        if let Ok(mut log) = session.try_lock() {
            *log = SessionLog::open_read_only(path.clone()).ok()?;
        }
        Some(vak_tools::PresentationCard {
            presentation_id: None,
            semantic_type: "chart".into(),
            skill_id: "test".into(),
            skill_version: "1".into(),
            schema_version: 1,
            payload: input.clone(),
            title: "Evidence".into(),
            identity_digest: "evidence".into(),
            block: Some("chart".into()),
            data_call_id: None,
        })
    }));

    let outcome = h
        .agent
        .run(
            "show evidence",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Failed { error } if error.to_string().contains("session write failed")),
        "a failed presentation append must fail the turn: {outcome:?}"
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

/// Invariant 1: the host-supplied tail (the clock) reaches the model
/// only after its exact bytes are in the ledger.
#[tokio::test]
async fn the_turn_context_the_model_reads_is_recorded_first() {
    let mut h = harness(
        vec![ScriptedResponse::Message(assistant_text("hi"))],
        vec![],
    );
    h.agent.config.tail = vak_agent::TailInput {
        temporal: "Current time: 2026-09-30 10:00 UTC.".into(),
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
    assert_eq!(recorded.data.get("stance"), None);
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
    agent.config.mode = vak_permission::Mode::FullAccess;
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
