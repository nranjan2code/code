//! A planned change that read the file it fixes rests on the runtime's own
//! receipts, not on retrieved material: no claim-support requirement is
//! inferred for it (found live 2026-10-19: such a coding turn was marked a
//! partial result). Run with the real worker, so the read and the edit run.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn message(content: Vec<ContentBlock>, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason: stop,
        usage: Usage {
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
        applied: None,
    }
}

fn call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    message(
        vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        StopReason::ToolUse,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_planned_change_that_read_its_file_needs_no_source_support() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("calc.py"), "def add(a, b):\n    return a - b\n").unwrap();
    // The stop gate is off: a scripted provider cannot write the check it
    // would ask for, and this test is about the outcome evaluation.
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
    vak_config::spaces::bind(&cwd).unwrap();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    core.set_shared_scope(vak_config::scope::SharedScope::new(home));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    crate::support::set_provider_without_automatic_topics(
        &core,
        Arc::new(Scripted {
            capacity_key: crate::support::CapacityKey::default(),
            responses: Mutex::new(VecDeque::from(vec![
                call(
                    "plan-a",
                    "plan",
                    serde_json::json!({"deliverables": [{"name": "fix add", "form": "change"}]}),
                ),
                call("read-a", "read", serde_json::json!({"path": "calc.py"})),
                call(
                    "edit-a",
                    "edit",
                    serde_json::json!({"path": "calc.py",
                    "edits": [{"old_text": "a - b", "new_text": "a + b"}]}),
                ),
                message(vec![ContentBlock::text("Fixed add.")], StopReason::EndTurn),
            ])),
        }),
    );
    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(256);
    let (outcome, session) = core
        .run_turn_with(
            session,
            "Fix the bug in calc.py.",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, vak_agent::TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert!(
        std::fs::read_to_string(cwd.join("calc.py"))
            .unwrap()
            .contains("a + b"),
        "the read and the edit ran: {:?}",
        session.derive_messages()
    );
    let evaluation = session
        .chain_to_root()
        .into_iter()
        .rev()
        .find_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Activity(activity)
                if activity.label == "Outcome evaluation" =>
            {
                Some(activity.clone())
            }
            _ => None,
        })
        .unwrap();
    let evaluations: Vec<vak_intent::RequirementEvaluation> =
        serde_json::from_str(&evaluation.data["evaluation"]).unwrap();
    assert!(
        !evaluations
            .iter()
            .any(|row| row.requirement_id == "retrieved-source-support"),
        "{evaluations:?}"
    );
    assert_eq!(
        evaluation.data.get("evidence_state").map(String::as_str),
        Some("fresh"),
        "the read is still a retrieval: {evaluation:?}"
    );
}
