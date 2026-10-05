//! Exit tests of M3b slice 2 (docs/plans/data-architecture-plan.md):
//! a turn grows its ledger by at most 20 KB mean and costs at most six
//! record syncs for its ledger and side ledgers, plus the two its run
//! record adds at M4.2 (opened with its ledger named, then settled; a
//! settle lost to a power cut would read a finished run as abandoned). Turns are real Core turns with a tool call and an answer;
//! the first turn, which binds the capability interface in full, is
//! excluded from the mean.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;
use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

const TURNS: u64 = 5;
const BYTES_PER_TURN: u64 = 20 * 1024;
const SYNCS_PER_TURN: u64 = 8;

/// Each turn reads a file, then answers.
struct ReadThenAnswer {
    calls: AtomicUsize,
    capacity_key: crate::support::CapacityKey,
}

#[async_trait::async_trait]
impl Provider for ReadThenAnswer {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let answered = request.messages.last().is_some_and(|message| {
            message
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
        });
        let (content, stop_reason) = if !answered {
            (
                vec![ContentBlock::ToolUse {
                    id: format!("read-{call}"),
                    name: "read".into(),
                    input: serde_json::json!({ "path": named_file(&request) }),
                }],
                StopReason::ToolUse,
            )
        } else {
            (
                vec![ContentBlock::text("The notes list three errands.")],
                StopReason::EndTurn,
            )
        };
        let m = AssistantMessage {
            content,
            stop_reason,
            usage: Usage::default(),
            model: "test-model".into(),
            response_id: None,
        };
        let (mut sink, rx) = stream::channel(64);
        sink.push(stream::StreamEvent::Start { partial: m.clone() });
        sink.close_message(m).await;
        Ok(rx)
    }
}

/// The `notes-<n>.txt` the turn's directive names.
fn named_file(request: &ChatRequest) -> String {
    request
        .messages
        .iter()
        .rev()
        .flat_map(|message| message.content.iter())
        .find_map(|block| match block {
            ContentBlock::Text { text, .. } => text
                .split_whitespace()
                .find(|word| word.starts_with("notes-"))
                .map(|word| word.trim_end_matches('?').to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "notes-0.txt".into())
}

struct Measured {
    bytes_per_turn: u64,
    syncs_per_turn: u64,
}

/// The two budgets share one process-wide sync counter, so they share one
/// measurement.
async fn measure() -> &'static Measured {
    static MEASURED: tokio::sync::OnceCell<Measured> = tokio::sync::OnceCell::const_new();
    MEASURED.get_or_init(run_turns).await
}

async fn run_turns() -> Measured {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    for n in 0..=TURNS {
        std::fs::write(
            cwd.join(format!("notes-{n}.txt")),
            format!("errand {n}\nstamps\n"),
        )
        .unwrap();
    }
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_provider_instance(Arc::new(ReadThenAnswer {
        calls: AtomicUsize::new(0),
        capacity_key: crate::support::CapacityKey::default(),
    }));

    let mut session = core.start_session().await.unwrap();
    let ledger = session.path().to_path_buf();
    let size = || {
        vak_session::SessionLog::segment_files(&ledger)
            .iter()
            .map(|segment| std::fs::metadata(segment).unwrap().len())
            .sum::<u64>()
    };
    let turn = |session, prompt: String| {
        let core = core.clone();
        async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(256);
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
            let (_, session) = core
                .run_turn_with(
                    session,
                    &prompt,
                    CancellationToken::new(),
                    None,
                    None,
                    None,
                    tx,
                )
                .await
                .unwrap();
            session
        }
    };

    session = turn(session, "what is in notes-0.txt?".into()).await;
    let (bytes, syncs) = (size(), vak_storage::records::syncs());
    for n in 1..=TURNS {
        session = turn(session, format!("and what does notes-{n}.txt say?")).await;
    }
    let measured = Measured {
        bytes_per_turn: (size() - bytes) / TURNS,
        syncs_per_turn: (vak_storage::records::syncs() - syncs) / TURNS,
    };
    drop(session);
    println!(
        "per turn: {} bytes, {} record syncs",
        measured.bytes_per_turn, measured.syncs_per_turn
    );
    measured
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bytes_per_turn_budget() {
    let measured = measure().await;
    assert!(
        measured.bytes_per_turn <= BYTES_PER_TURN,
        "a turn grew the ledger by {} bytes",
        measured.bytes_per_turn
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fsyncs_per_turn_budget() {
    let measured = measure().await;
    assert!(
        measured.syncs_per_turn <= SYNCS_PER_TURN,
        "a turn cost {} record syncs",
        measured.syncs_per_turn
    );
}
