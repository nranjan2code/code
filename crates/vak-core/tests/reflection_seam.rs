//! `Core::reflect_after_turn` — the background reflection seam shared by
//! every surface (docs/design/29 P1). Contract under test: best-effort by
//! design; flag-off / budget-denied / concurrent / failing paths all skip
//! cleanly and never dispatch outside their allowed envelope.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio_util::sync::CancellationToken;

use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

/// Provider stub that counts dispatches, optionally fails, and optionally
/// holds the response open until released (to force real overlap between
/// two concurrent reflection passes).
struct Counting {
    calls: Arc<AtomicUsize>,
    reply: &'static str,
    fail: bool,
    hold: Option<Arc<tokio::sync::Notify>>,
}

fn msg(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(text)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

#[async_trait::async_trait]
impl Provider for Counting {
    fn name(&self) -> &str {
        "counting"
    }

    async fn stream(
        &self,
        _req: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hold) = &self.hold {
            hold.notified().await;
        }
        let (mut sink, rx) = stream::channel(8);
        if self.fail {
            sink.close_error(LlmError::Parse("reflection boom".into()))
                .await;
        } else {
            sink.push(stream::StreamEvent::Start {
                partial: msg(self.reply),
            });
            sink.close_message(msg(self.reply)).await;
        }
        Ok(rx)
    }
}

const GOOD_REPLY: &str = r#"{"notes":[{"note":"the release pipeline pauses before every rollback window","kind":"decision","tag":"releases"}],"skill":{"name":"ship-guarded","description":"Ship with rollbacks guarded","instructions":"Run scripts/ship.sh after checks"}}"#;

fn core_in(dir: &tempfile::TempDir, project_config: &str) -> vak_core::Core {
    let project = dir.path().join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("config.toml"), project_config).unwrap();
    let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core
}

#[tokio::test]
async fn reflection_disabled_skips_without_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    // Project layer pins reflection off so a developer's global config
    // cannot leak into this hermetic test.
    let core = core_in(&dir, "[memory]\nreflection = false\n");
    let calls = Arc::new(AtomicUsize::new(0));
    core.set_provider_instance(Arc::new(Counting {
        calls: calls.clone(),
        reply: GOOD_REPLY,
        fail: false,
        hold: None,
    }));
    let session = core.start_session().await.unwrap();

    let out = core.reflect_after_turn(&session, "all done").await;

    assert_eq!(
        out,
        vak_core::reflection::ReflectionOutcome::Skipped {
            reason: "reflection-disabled"
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no provider dispatch");
    assert!(vak_core::memory::list_notes(&core.sessions_home(), core.cwd()).is_empty());
}

#[tokio::test]
async fn memory_writes_disabled_skips_without_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(&dir, "[memory]\nreflection = true\nwrite_enabled = false\n");
    let calls = Arc::new(AtomicUsize::new(0));
    core.set_provider_instance(Arc::new(Counting {
        calls: calls.clone(),
        reply: GOOD_REPLY,
        fail: false,
        hold: None,
    }));
    let session = core.start_session().await.unwrap();

    let out = core.reflect_after_turn(&session, "all done").await;

    assert_eq!(
        out,
        vak_core::reflection::ReflectionOutcome::Skipped {
            reason: "memory-writes-disabled"
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn budget_denied_skips_before_any_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    // Priced test model + a day cap already blown by seeded spend: the
    // gate must deny BEFORE the provider is ever touched.
    let core = core_in(
        &dir,
        "[memory]\nreflection = true\n[finops]\nmax_day_usd = 1.0\n\n[finops.price_overrides.test-model]\ninput = 1.25\noutput = 6.0\n",
    );
    core.set_model("test-model".into());
    let ledger = vak_core::finops::FinOpsLedger::new(&home);
    ledger
        .append(&vak_core::finops::CostRow {
            ts: chrono::Utc::now(),
            model: "test-model".into(),
            provider: "counting".into(),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_input_tokens: None,
            usd: Some(50.0),
            source: "test-seed".into(),
            session_id: "seed".into(),
        })
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    core.set_provider_instance(Arc::new(Counting {
        calls: calls.clone(),
        reply: GOOD_REPLY,
        fail: false,
        hold: None,
    }));
    let session = core.start_session().await.unwrap();

    let out = core.reflect_after_turn(&session, "all done").await;

    assert_eq!(
        out,
        vak_core::reflection::ReflectionOutcome::Skipped { reason: "budget" }
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "denial must precede dispatch"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_turns_reflect_once_and_never_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    // Explicit generous cap keeps the gate deterministic even when the
    // developer's global config carries tight finops knobs.
    let core = core_in(
        &dir,
        "[memory]\nreflection = true\n[finops]\nmax_day_usd = 1000.0\n\n[finops.price_overrides.test-model]\ninput = 1.25\noutput = 6.0\n",
    );
    core.set_model("test-model".into());
    let calls = Arc::new(AtomicUsize::new(0));
    let hold = Arc::new(tokio::sync::Notify::new());
    core.set_provider_instance(Arc::new(Counting {
        calls: calls.clone(),
        reply: GOOD_REPLY,
        fail: false,
        hold: Some(hold.clone()),
    }));
    let session = core.start_session().await.unwrap();
    let header_b = session.header().unwrap().clone();

    // A starts first and parks inside the aux call while holding the
    // in-flight marker (dispatch happens strictly after acquisition).
    let core_a = core.clone();
    let ha = tokio::spawn(async move { core_a.reflect_after_turn(&session, "all done").await });
    for _ in 0..200 {
        if calls.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "A dispatched exactly once");

    // B races in on the same session id while A is mid-flight. Ledger
    // handles are exclusively locked, so this second surface carries its
    // own log file cloned from A's header — legal here because a skipped
    // pass exits at the marker before touching any content or disk.
    let s_b =
        vak_session::SessionLog::create(dir.path().join("second-surface.jsonl"), header_b).unwrap();
    let core_b = core.clone();
    let hb = tokio::spawn(async move { core_b.reflect_after_turn(&s_b, "all done").await });
    // ...and must be skipped without its own dispatch.
    let b = hb.await.unwrap();
    assert_eq!(
        b,
        vak_core::reflection::ReflectionOutcome::Skipped {
            reason: "already-in-flight"
        }
    );

    // Release A; it completes as the single reflected pass.
    hold.notify_one();
    let a = ha.await.unwrap();
    assert_eq!(
        a,
        vak_core::reflection::ReflectionOutcome::Reflected {
            notes_added: 1,
            skills_proposed: true
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no second dispatch ever");
    let notes = vak_core::memory::list_notes(&core.sessions_home(), core.cwd());
    assert_eq!(notes.len(), 1, "one note total — no duplicates");
}

#[tokio::test]
async fn provider_error_yields_skipped_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(&dir, "[memory]\nreflection = true\n");
    let calls = Arc::new(AtomicUsize::new(0));
    core.set_provider_instance(Arc::new(Counting {
        calls: calls.clone(),
        reply: GOOD_REPLY,
        fail: true,
        hold: None,
    }));
    let session = core.start_session().await.unwrap();

    let out = core.reflect_after_turn(&session, "all done").await;

    assert_eq!(calls.load(Ordering::SeqCst), 1, "dispatch attempted");
    assert_eq!(
        out,
        vak_core::reflection::ReflectionOutcome::Skipped {
            reason: "reflect-call-failed"
        }
    );
    assert!(vak_core::memory::list_notes(&core.sessions_home(), core.cwd()).is_empty());
}
