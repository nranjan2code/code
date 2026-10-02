#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The handoff rescue mid-turn: the turn outgrows a small horizon, the reset
//! replaces its earlier steps with a summary, and the turn carries on from its
//! directive (docs/design/68-context-engine.md §4).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vak_agent::{Agent, AgentConfig, ApprovalMode, AutoApprove, SteeringQueues, TurnOutcome};
use vak_context::capacity::{CacheBehaviour, CapacityProfile, Horizon, ProbeProvenance};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::bash::BashTool;

struct Script {
    replies: Mutex<VecDeque<AssistantMessage>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Script {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn stream(&self, r: ChatRequest, _c: CancellationToken) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(r);
        let next = self.replies.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => {
                sink.close_error(LlmError::Parse("script exhausted".into()))
                    .await
            }
        }
        Ok(rx)
    }
}

fn reply(content: Vec<ContentBlock>, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason: stop,
        usage: Usage::default(),
        model: "m".into(),
        response_id: None,
    }
}

#[tokio::test]
async fn the_turn_continues_from_its_directive_after_a_mid_turn_reset() {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-reset-mid".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "scripted".into(),
            model: "m".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(BashTool)];
    // A 3k-token ceiling: a 14k-char tool result (~3.5k tokens) outgrows it.
    cfg.capacity = Some(CapacityProfile::from_probe(
        32_768,
        None,
        Horizon {
            tokens: 3_000,
            confidence: 0.9,
            last_confirmed: std::time::SystemTime::now(),
        },
        CacheBehaviour::Unknown,
        1_024,
        ProbeProvenance {
            probed_at: std::time::SystemTime::now(),
            rungs: Vec::new(),
            signals: Vec::new(),
            metadata_digest: "d".into(),
            quantisation: None,
        },
    ));
    cfg.handoff_reset = true;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(Script {
        replies: Mutex::new(
            vec![
                reply(
                    vec![ContentBlock::ToolUse {
                        id: "t1".into(),
                        name: "bash".into(),
                        input: serde_json::json!({
                            "command": "head -c 14000 /dev/zero | tr '\\0' x"
                        }),
                    }],
                    StopReason::ToolUse,
                ),
                reply(
                    vec![ContentBlock::text("HANDOFF SUMMARY")],
                    StopReason::EndTurn,
                ),
                reply(vec![ContentBlock::text("all done")], StopReason::EndTurn),
            ]
            .into(),
        ),
        requests: requests.clone(),
    });
    let mut agent = Agent::new(provider, log, cfg);
    agent.config.mode = vak_permission::Mode::FullAccess;
    agent.config.approval_mode = ApprovalMode::AutoApprove;
    agent.config.approver = Some(Arc::new(AutoApprove));
    let (tx, _rx) = mpsc::channel(4096);
    let outcome = agent
        .run(
            "build the quarterly report",
            &SteeringQueues::new(),
            CancellationToken::new(),
            tx,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3, "step, handoff, post-reset step");
    let last = &requests[2];
    let blocks = |pred: &dyn Fn(&ContentBlock) -> bool| {
        last.messages
            .iter()
            .flat_map(|m| m.content.iter())
            .any(pred)
    };
    assert!(blocks(
        &|b| matches!(b, ContentBlock::Text { text } if text.contains("HANDOFF SUMMARY"))
    ));
    assert!(
        blocks(
            &|b| matches!(b, ContentBlock::Text { text } if text.contains("build the quarterly report"))
        ),
        "the directive must reach the model after the reset"
    );
    assert!(
        !blocks(&|b| matches!(b, ContentBlock::ToolResult { .. })),
        "the oversized result was replaced by the summary"
    );
}
