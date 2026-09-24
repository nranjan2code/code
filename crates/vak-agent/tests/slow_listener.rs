#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A slow listener never paces the model: stream pieces it has no room for
//! are dropped (each carries the message so far), so a long reasoning reply
//! finishes at the provider's speed, not the listener's.
//!
//! Found live: the eval runner handed the loop an event channel nobody read,
//! and a long reasoning reply waited on it until the step watchdog although
//! the provider had finished.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};

/// Streams two thousand reasoning pieces, then the answer.
struct LongReasoning;

#[async_trait]
impl Provider for LongReasoning {
    fn name(&self) -> &str {
        "long-reasoning"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(4096);
        tokio::spawn(async move {
            let mut partial = AssistantMessage::empty("test-model");
            for n in 0..2000 {
                let delta = format!("step {n} ");
                partial.content = vec![ContentBlock::Thinking {
                    text: format!(
                        "{}{delta}",
                        match partial.content.first() {
                            Some(ContentBlock::Thinking { text, .. }) => text.clone(),
                            _ => String::new(),
                        }
                    ),
                    signature: None,
                }];
                sink.push(stream::StreamEvent::ThinkingDelta {
                    delta,
                    partial: partial.clone(),
                });
            }
            sink.close_message(AssistantMessage {
                content: vec![ContentBlock::text("done")],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                    ..Default::default()
                },
                model: "test-model".into(),
                response_id: None,
            })
            .await;
        });
        Ok(rx)
    }
}

#[tokio::test]
async fn a_slow_listener_does_not_pace_a_long_reply() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let header = SessionHeader {
        agent: None,
        session_id: "stalled".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "long-reasoning".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "read-only".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "stalled"),
        header,
    )
    .unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    let mut agent = Agent::new(Arc::new(LongReasoning), log, cfg);

    // 20ms per event: pacing the two thousand pieces would take 40s.
    let (events, mut rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        agent.run(
            "think it through",
            &Default::default(),
            CancellationToken::new(),
            events,
        ),
    )
    .await
    .expect("the reply is not paced by the listener");
    match outcome {
        TurnOutcome::Completed { response } => assert_eq!(response.text_content(), "done"),
        other => panic!("expected completed, got {other:?}"),
    }
}
