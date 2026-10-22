//! A file version and a saved note name the turn that made them, and are
//! read back with that turn's record (docs/design/99-turns-that-do-not-poison.md
//! §4 E): nothing is withheld, everything is labelled.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;

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

fn message(content: Vec<ContentBlock>, stop: vak_llm::types::StopReason) -> AssistantMessage {
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

fn text(t: &str) -> AssistantMessage {
    message(
        vec![ContentBlock::text(t)],
        vak_llm::types::StopReason::EndTurn,
    )
}

fn calls(list: &[(&str, &str, serde_json::Value)]) -> AssistantMessage {
    message(
        list.iter()
            .map(|(id, name, input)| ContentBlock::ToolUse {
                id: (*id).into(),
                name: (*name).into(),
                input: input.clone(),
            })
            .collect(),
        vak_llm::types::StopReason::ToolUse,
    )
}

async fn turn(core: &Core, log: SessionLog, words: &str) -> SessionLog {
    let (events_tx, _events_rx) = tokio::sync::mpsc::channel(256);
    core.run_turn_with(
        log,
        words,
        CancellationToken::new(),
        None,
        None,
        None,
        events_tx,
    )
    .await
    .unwrap()
    .1
}

/// The result of the call `id` on the log's chain.
fn result_of(log: &SessionLog, id: &str) -> String {
    log.message_chain()
        .iter()
        .flat_map(|(_, m)| m.content.clone())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } if tool_use_id == id => Some(content),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result for {id}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_and_a_note_are_read_with_the_record_of_the_turn_that_made_them() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    vak_config::paths::isolate_home_for_tests();
    vak_config::spaces::bind(&cwd).unwrap();
    let core = Core::new(cwd.clone()).unwrap();
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.clone()));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    crate::support::set_provider_without_automatic_topics(
        &core,
        Arc::new(Scripted {
            capacity_key: crate::support::CapacityKey::default(),
            responses: Mutex::new(VecDeque::from(vec![
                // The turn that goes wrong: a lookup fails, then a file and a
                // note are written anyway.
                calls(&[("c1", "read", serde_json::json!({"path": "missing.txt"}))]),
                calls(&[
                    (
                        "c2",
                        "write",
                        serde_json::json!({"path": "points.txt", "content": "41.1, 29.0\n",
                        "title": "Points"}),
                    ),
                    (
                        "c3",
                        "remember",
                        serde_json::json!({"note": "the quarry points are 41.1, 29.0"}),
                    ),
                ]),
                text("Saved the points."),
                // A later turn reads the file back.
                calls(&[("c4", "read", serde_json::json!({"path": "points.txt"}))]),
                text("Read them."),
                // Another conversation finds the note.
                calls(&[(
                    "c5",
                    "recall",
                    serde_json::json!({"query": "quarry points"}),
                )]),
                text("Found the note."),
            ])),
        }),
    );
    std::mem::forget(dir);

    let log = core.start_session().await.unwrap();
    let first = log.header().unwrap().session_id.clone();
    let log = turn(&core, log, "where are the quarry points").await;
    let made_turn = vak_session::TurnIndex::from_log(&log)
        .turns
        .first()
        .and_then(|turn| turn.card.as_ref())
        .map(|card| card.turn_id.clone())
        .expect("the first turn closed with a card");

    let log = turn(&core, log, "show me the file").await;
    let read = result_of(&log, "c4");
    assert!(
        read.contains(&format!(
            "`points.txt` in the workspace was made in turn {made_turn} of this conversation: \
             calls read (failed), write, remember"
        )),
        "the read names the turn that wrote the file and how its calls ended: {read}"
    );
    drop(log);

    let other = core.start_session().await.unwrap();
    assert_ne!(other.header().unwrap().session_id, first);
    let other = turn(&core, other, "what about the quarry").await;
    let found: serde_json::Value = serde_json::from_str(&result_of(&other, "c5")).unwrap();
    let note = found["elsewhere"]
        .as_array()
        .and_then(|hits| hits.iter().find(|hit| hit["from"] == "saved note"))
        .unwrap_or_else(|| panic!("the note is found: {found}"));
    assert_eq!(note["made_in"]["turn"], made_turn.as_str(), "{note}");
    assert_eq!(note["made_in"]["conversation"], first.as_str(), "{note}");
    assert_eq!(
        note["made_in"]["calls"], "read (failed), write, remember",
        "{note}"
    );
}
