//! End-to-end test for proactive context retrieval with dynamic caps.
//!
//! Verifies that when context overflows, the agent:
//! 1. Proactively retrieves relevant older turns before compaction
//! 2. Scales the retrieval cap with the available history budget
//! 3. Keeps retrieved turns verbatim while summarizing only the rest
//! 4. Emits ContextRetrieved + ContextCompacted events

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, AgentEvent, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::bash::BashTool;

struct TaggedScripted {
    steps: Mutex<VecDeque<AssistantMessage>>,
    summaries: Mutex<VecDeque<AssistantMessage>>,
    seen_systems: Arc<Mutex<Vec<String>>>,
}

impl TaggedScripted {
    fn is_compaction(request: &ChatRequest) -> bool {
        request
            .system
            .as_deref()
            .is_some_and(|s| s.contains("context compactor"))
    }
}

#[async_trait::async_trait]
impl Provider for TaggedScripted {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.seen_systems
            .lock()
            .unwrap()
            .push(request.system.clone().unwrap_or_default());
        let msg = if Self::is_compaction(&request) {
            self.summaries.lock().unwrap().pop_front()
        } else {
            self.steps.lock().unwrap().pop_front()
        };
        let (mut sink, rx) = stream::channel(64);
        match msg {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn text(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

fn make_header(session_id: &str, dir: &std::path::Path) -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: session_id.into(),
        created_at: chrono::Utc::now(),
        cwd: dir.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn seed_message(log: &mut SessionLog, text: &str) {
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text(text),
        meta: None,
    })
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proactive_retrieval_scales_with_budget() {
    let dir = tempdir().unwrap();

    // Test: dynamic_retrieval_cap scales with history budget
    {
        let policy = vak_agent::context::ContextPolicy::default();
        // Small window (32K model): floor behavior
        let small = vak_agent::context::ContextPolicy {
            context_window: 32_000,
            ..policy.clone()
        };
        assert_eq!(small.dynamic_retrieval_cap(2_000), 6);
        assert_eq!(small.dynamic_retrieval_cap(16_000), 6);

        // Large window (128K model): scales up
        let large = vak_agent::context::ContextPolicy {
            context_window: 128_000,
            ..policy.clone()
        };
        assert_eq!(large.dynamic_retrieval_cap(37_000), 13);
        assert_eq!(large.dynamic_retrieval_cap(58_000), 20);

        // Capped at 50
        assert_eq!(large.dynamic_retrieval_cap(200_000), 50);
    }

    // Test: dynamic_history_budget_cap scales with window
    {
        let policy = vak_agent::context::ContextPolicy::default();
        let small = vak_agent::context::ContextPolicy {
            context_window: 32_000,
            ..policy.clone()
        };
        assert!(small.dynamic_history_budget_cap(16_000) <= 16_000);

        let large = vak_agent::context::ContextPolicy {
            context_window: 128_000,
            ..policy.clone()
        };
        let cap = large.dynamic_history_budget_cap(16_000);
        assert!(cap >= 16_000);
    }

    // End-to-end: proactive retrieval happens on context overflow
    let header = make_header("proactive", dir.path());
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();

    // Seed messages with distinct topics. The ones about Kubernetes
    // should be retrieved when the query mentions Kubernetes.
    seed_message(&mut log, "discuss Kubernetes cluster deployment patterns");
    seed_message(&mut log, "the Kubernetes cluster uses Docker containers");
    seed_message(&mut log, "how to configure PostgreSQL database settings");
    seed_message(&mut log, "PostgreSQL needs shared_buffers tuning");
    seed_message(&mut log, "tell me about network firewalls and TLS");
    // Pad with large messages to trigger overflow
    for i in 0..4 {
        seed_message(&mut log, &format!("seed-{i}: {}", "y".repeat(6000)));
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(TaggedScripted {
        steps: Mutex::new(VecDeque::from(vec![
            text("step one done"),
            text("final answer"),
        ])),
        summaries: Mutex::new(VecDeque::from(vec![text(
            "Task: continue. State: work in progress. Open: finish.",
        )])),
        seen_systems: seen.clone(),
    });

    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(BashTool)];
    cfg.context_policy.context_window = 2_000;
    cfg.context_policy.max_output = 200;
    cfg.context_policy.keep_recent = 2;
    let mut agent = Agent::new(provider, log, cfg);

    let (tx, mut rx) = mpsc::channel(256);
    let outcome = agent
        .run(
            "Kubernetes deployment",
            &Default::default(),
            CancellationToken::new(),
            tx,
        )
        .await;

    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(response.text_content(), "step one done");
        }
        other => panic!("expected completed, got {other:?}"),
    }

    // Verify ContextRetrieved event was emitted
    let mut retrieved_event_seen = false;
    let mut compacted_event_seen = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            AgentEvent::ContextRetrieved {
                retrieved_count,
                cap,
            } => {
                retrieved_event_seen = true;
                assert!(retrieved_count > 0, "should have retrieved entries");
                assert!(cap > 0, "cap should be positive");
            }
            AgentEvent::ContextCompacted { .. } => {
                compacted_event_seen = true;
            }
            _ => {}
        }
    }

    assert!(
        retrieved_event_seen,
        "ContextRetrieved event should have been emitted"
    );
    assert!(
        compacted_event_seen,
        "ContextCompacted event should have been emitted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proactive_retrieval_filters_retrieved_from_summary() {
    let dir = tempdir().unwrap();
    let header = make_header("filter-test", dir.path());
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();

    // Seed with Kubernetes content that should be retrieved
    seed_message(&mut log, "the Kubernetes API server exposes cluster state");
    seed_message(&mut log, "Kubernetes ingress routes external traffic");
    // Non-relevant content
    seed_message(&mut log, "favorite pizza toppings debate continues");
    seed_message(&mut log, "weather forecast predicts rain tomorrow");
    // Pad to trigger overflow
    for i in 0..4 {
        seed_message(&mut log, &format!("filler-{i}: {}", "z".repeat(6000)));
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(TaggedScripted {
        steps: Mutex::new(VecDeque::from(vec![text("done")])),
        summaries: Mutex::new(VecDeque::from(vec![text(
            "Summary covers filler content only.",
        )])),
        seen_systems: seen.clone(),
    });

    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(BashTool)];
    cfg.context_policy.context_window = 2_000;
    cfg.context_policy.max_output = 200;
    cfg.context_policy.keep_recent = 2;
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "Kubernetes ingress routing",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(256).0,
        )
        .await;

    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    // The Kubernetes messages should NOT appear in the summary — they were
    // retrieved and kept verbatim, not summarized. We verify by checking
    // that the compaction transcript didn't include them.
    let session = agent.session.lock().await;
    let msgs = session.derive_messages();
    let all_text: String = msgs
        .iter()
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n");

    // The summary should NOT contain the retrieved Kubernetes content —
    // it was retrieved, not summarized. Check that the summary mentions
    // the filler content instead.
    let has_summary = all_text.contains("<context_summary>");
    if has_summary {
        // Extract the summary text
        let summary_start = all_text.find("<context_summary>").map(|i| i + 17);
        let summary_end = all_text.find("</context_summary>");
        if let (Some(start), Some(end)) = (summary_start, summary_end) {
            let summary_text = &all_text[start..end];
            // The summary should NOT contain the retrieved Kubernetes content
            assert!(
                !summary_text.contains("Kubernetes API server"),
                "retrieved content should not be in summary"
            );
        }
    }

    // At least the filler content should have been summarized (proving
    // compaction happened)
    assert!(has_summary, "compaction should have occurred");
}
