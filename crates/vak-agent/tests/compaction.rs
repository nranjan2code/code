#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::bash::BashTool;

/// Routes by system-prompt marker: the compaction call carries
/// COMPACTION_SYSTEM; normal steps get the step script.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overflow_triggers_compaction_then_run_completes() {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        session_id: "compact".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            system_prompt: "sys".into(),
            tools: vec!["bash".into()],
            permission_mode: "full-access".into(),
            skills: Vec::new(),
        },
    };
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();

    // Pre-seed the ledger with 6 big messages (~9K tokens) so the first
    // step already exceeds the tiny 2K-token window's trigger point.
    for i in 0..6 {
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(format!("seed-{i}:{}", "y".repeat(6000))),
            meta: None,
        })
        .unwrap();
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(TaggedScripted {
        steps: Mutex::new(VecDeque::from(vec![
            text("step one done"),
            text("final answer"),
        ])),
        summaries: Mutex::new(VecDeque::from(vec![text(
            "Task: continue seeded work. State: seeds processed. Open: finish.",
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
            "finish it",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(256).0,
        )
        .await;

    match outcome {
        TurnOutcome::Completed { response } => {
            // Single post-compaction step: no tool calls => turn ends here.
            assert_eq!(response.text_content(), "step one done");
        }
        other => panic!("expected completed, got {other:?}"),
    }

    // A compaction call must have been made.
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|s| s.contains("context compactor")),
        "compaction request expected"
    );

    // The ledger now carries a compaction entry; projection reflects it.
    let path = {
        let session = agent.session.lock().await;
        let msgs = session.derive_messages();
        let summary_present = msgs.iter().any(|m| {
            m.text_content().contains("<context_summary>") && m.text_content().contains("Task:")
        });
        assert!(summary_present, "summary must be in projection");

        // Recent verbatim turns survive (the kept tail incl. the live prompt).
        assert!(
            msgs.iter()
                .any(|m| m.text_content().starts_with("finish it")),
            "recent turns stay verbatim"
        );
        assert!(
            msgs.iter().any(|m| m.text_content() == "step one done"),
            "post-compaction exchange stays verbatim"
        );
        session.path().to_path_buf()
    };

    // Compaction entry persisted on disk. The agent holds an exclusive
    // lock on the ledger for its lifetime, so drop it before reopening.
    drop(agent);
    let reopened = SessionLog::open(path).unwrap();
    assert!(
        reopened
            .derive_messages()
            .iter()
            .any(|m| { m.text_content().contains("<context_summary>") })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn still_over_after_compaction_fails_closed() {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        session_id: "over".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            system_prompt: "sys".into(),
            tools: vec![],
            permission_mode: "full-access".into(),
            skills: Vec::new(),
        },
    };
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();

    // keep_recent=2 keeps two ~1500-token messages => still over a 1K budget.
    for i in 0..4 {
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(format!("seed-{i}:{}", "y".repeat(6000))),
            meta: None,
        })
        .unwrap();
    }

    let provider = Arc::new(TaggedScripted {
        steps: Mutex::new(VecDeque::from(vec![text("never reached")])),
        summaries: Mutex::new(VecDeque::from(vec![text("short summary")])),
        seen_systems: Arc::new(Mutex::new(Vec::new())),
    });

    let mut cfg = AgentConfig::new("sys");
    cfg.context_policy.context_window = 1_000;
    cfg.context_policy.max_output = 100;
    cfg.context_policy.keep_recent = 2;
    // Phase H adds a one-shot reset-with-handoff rescue on this path; this
    // test pins the underlying fail-closed contract with the rescue off
    // (the rescue has its own e2e in goal_mode.rs).
    cfg.handoff_reset = false;
    cfg.run_retry_attempts = 0;
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Failed { error } => {
            assert!(
                error.to_string().contains("still over budget"),
                "got: {error}"
            );
        }
        other => panic!("expected fail-closed, got {other:?}"),
    }
}

mod pair_boundary_and_reuse {
    use super::*;
    use std::collections::HashMap;

    struct Tagged {
        routes: Mutex<HashMap<String, VecDeque<AssistantMessage>>>,
        seen_prompts: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Provider for Tagged {
        fn name(&self) -> &str {
            "tagged"
        }
        async fn stream(
            &self,
            request: ChatRequest,
            _c: CancellationToken,
        ) -> Result<EventStream, LlmError> {
            let key = if request
                .system
                .as_deref()
                .is_some_and(|s| s.contains("context compactor"))
            {
                "compact".to_string()
            } else {
                "step".to_string()
            };
            if key == "compact" {
                let transcript = request.messages[0].text_content();
                self.seen_prompts.lock().unwrap().push(transcript);
            }
            let msg = self
                .routes
                .lock()
                .unwrap()
                .get_mut(&key)
                .and_then(|d| d.pop_front());
            let (mut sink, rx) = stream::channel(64);
            match msg {
                Some(m) => {
                    sink.push(stream::StreamEvent::Start { partial: m.clone() });
                    sink.close_message(m).await;
                }
                None => {
                    sink.close_error(LlmError::Parse(format!("exhausted: {key}")))
                        .await
                }
            }
            Ok(rx)
        }
    }

    fn assistant_tool_use(id: &str) -> AssistantMessage {
        AssistantMessage {
            content: vec![ContentBlock::ToolUse {
                id: id.into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "x"}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
            model: "m".into(),
        }
    }

    fn user_tool_result(id: &str) -> vak_llm::Message {
        vak_llm::Message {
            role: vak_llm::types::Role::User,
            content: vec![ContentBlock::tool_result(id, "out")],
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn boundary_never_orphans_a_tool_result() {
        // Ledger: pairs (assistant tool_use, user tool_result) x3, then a
        // plain text. keep_recent=2 lands the naive boundary exactly
        // between pair 2's result and pair 3's assistant... but with 6
        // messages before the final text, the raw boundary falls ON pair-2's
        // result user message — the orphan case.
        let dir = tempdir().unwrap();
        let header = SessionHeader {
            session_id: "pairs".into(),
            created_at: chrono::Utc::now(),
            cwd: dir.path().to_path_buf(),
            parent_session_id: None,
            contract: FrozenContract {
                app_version: "0".into(),
                provider: "tagged".into(),
                model: "m".into(),
                system_prompt: "sys".into(),
                tools: vec![],
                permission_mode: "full-access".into(),
                skills: Vec::new(),
            },
        };
        let mut log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
        for i in 0..3 {
            let au = assistant_tool_use(&format!("t{i}"));
            log.append_message(vak_session::MessageRecord {
                message: au.into_message(),
                meta: None,
            })
            .unwrap();
            log.append_message(vak_session::MessageRecord {
                message: user_tool_result(&format!("t{i}")),
                meta: None,
            })
            .unwrap();
        }
        log.append_message(vak_session::MessageRecord {
            message: text("plain tail").into_message(),
            meta: None,
        })
        .unwrap();

        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(Tagged {
            routes: Mutex::new(HashMap::from([
                (
                    "compact".to_string(),
                    VecDeque::from(vec![text("summary of earlier pairs")]),
                ),
                ("step".to_string(), VecDeque::from(vec![text("ok")])),
            ])),
            seen_prompts: seen.clone(),
        });

        let mut cfg = AgentConfig::new("sys");
        cfg.context_policy.context_window = 1_200;
        cfg.context_policy.max_output = 100;
        cfg.context_policy.keep_recent = 2;
        let mut agent = Agent::new(provider, log, cfg);

        let outcome = agent
            .run(
                "go",
                &Default::default(),
                CancellationToken::new(),
                mpsc::channel(128).0,
            )
            .await;
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "got {outcome:?}"
        );

        // The projection must never start its kept region on an orphaned
        // tool_result: every ToolResult in derived messages must be
        // preceded somewhere by its matching ToolUse.
        let session = agent.session.lock().await;
        let msgs = session.derive_messages();
        let mut seen_uses = std::collections::HashSet::new();
        for m in &msgs {
            for b in &m.content {
                match b {
                    ContentBlock::ToolUse { id, .. } => {
                        seen_uses.insert(id.clone());
                    }
                    ContentBlock::ToolResult { tool_use_id, .. } => {
                        assert!(
                            seen_uses.contains(tool_use_id),
                            "orphaned tool_result {tool_use_id} after compaction"
                        );
                    }
                    _ => {}
                }
            }
        }

        // (Reuse-across-compactions is covered by the vak-session test
        // 'second_compaction_summarizes_the_prior_summary'.)
    }
}
