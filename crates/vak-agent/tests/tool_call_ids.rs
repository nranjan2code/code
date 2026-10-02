#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Tool-call ids are unique within a session whatever the provider sent
//! (docs/design/68-context-engine.md §3): an omitted id and an id an adapter
//! restarts on every response (`call_0`, `gemini-call-0`) must not make two
//! calls share one id, or `recall` and the closed-turn digests read the wrong
//! result.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vak_agent::{Agent, AgentConfig, ApprovalMode, AutoApprove, SteeringQueues, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::bash::BashTool;

struct Script(Mutex<VecDeque<AssistantMessage>>);

#[async_trait::async_trait]
impl Provider for Script {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn stream(
        &self,
        _r: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.0.lock().unwrap().pop_front();
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

fn message(content: Vec<ContentBlock>, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason: stop,
        usage: Usage::default(),
        model: "m".into(),
        response_id: None,
    }
}

fn call(id: &str, text: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": format!("printf {text}")}),
    }
}

async fn run(steps: Vec<Vec<ContentBlock>>) -> SessionLog {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-ids".into(),
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
    let mut script: VecDeque<AssistantMessage> = steps
        .into_iter()
        .map(|c| message(c, StopReason::ToolUse))
        .collect();
    script.push_back(message(
        vec![ContentBlock::text("done")],
        StopReason::EndTurn,
    ));
    let mut agent = Agent::new(Arc::new(Script(Mutex::new(script))), log, cfg);
    agent.config.mode = vak_permission::Mode::FullAccess;
    agent.config.approval_mode = ApprovalMode::AutoApprove;
    agent.config.approver = Some(Arc::new(AutoApprove));
    let (tx, _rx) = mpsc::channel(4096);
    let outcome = agent
        .run("go", &SteeringQueues::new(), CancellationToken::new(), tx)
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    std::mem::forget(dir);
    let session = agent.session.lock().await;
    SessionLog::open_read_only(session.path().to_path_buf()).unwrap()
}

/// (use ids in order, result ids in order, result texts in order)
fn recorded(log: &SessionLog) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut uses, mut results, mut texts) = (vec![], vec![], vec![]);
    for (_, m) in log.message_chain() {
        for b in m.content {
            match b {
                ContentBlock::ToolUse { id, .. } => uses.push(id),
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    results.push(tool_use_id);
                    texts.push(content);
                }
                _ => {}
            }
        }
    }
    (uses, results, texts)
}

#[tokio::test]
async fn a_counter_that_restarts_each_response_gets_distinct_ids() {
    let log = run(vec![
        vec![call("call_0", "first")],
        vec![call("call_0", "second")],
    ])
    .await;
    let (uses, results, texts) = recorded(&log);
    assert_eq!(uses.len(), 2);
    assert_eq!(uses.iter().collect::<HashSet<_>>().len(), 2, "{uses:?}");
    assert_eq!(uses, results, "each result pairs with its own call");
    assert!(texts[0].contains("first") && texts[1].contains("second"));
    assert_eq!(
        log.evidence(&uses[0]).unwrap().content.trim(),
        texts[0].trim()
    );
    assert_eq!(
        log.evidence(&uses[1]).unwrap().content.trim(),
        texts[1].trim()
    );
}

#[tokio::test]
async fn omitted_ids_are_filled_in_and_distinct() {
    let log = run(vec![vec![call("", "a"), call("", "b")]]).await;
    let (uses, results, texts) = recorded(&log);
    assert_eq!(uses.len(), 2);
    assert!(uses.iter().all(|id| !id.is_empty()));
    assert_ne!(uses[0], uses[1]);
    assert_eq!(uses, results);
    assert!(texts[0].contains('a') && texts[1].contains('b'));
}
