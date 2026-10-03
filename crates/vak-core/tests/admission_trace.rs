//! Admission mints the trace key (docs/design/73 §4, plan M1): the cause a
//! session records names the surface that made it, and the work the run
//! writes carries that run's key and its actor.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use vak_core::admission::RunAdmission;
use vak_core::{Core, Surface};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::ConversationContext;
use vak_session::ids::TriggerId;
use vak_session::trace::{Cause, local};

struct Echo {
    capacity_key: crate::support::CapacityKey,
}

#[async_trait::async_trait]
impl Provider for Echo {
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
        let m = AssistantMessage {
            content: vec![ContentBlock::text("ok")],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 2,
                ..Default::default()
            },
            model: "test-model".into(),
            response_id: None,
        };
        let (mut sink, rx) = stream::channel(64);
        sink.push(stream::StreamEvent::Start { partial: m.clone() });
        sink.close_message(m).await;
        Ok(rx)
    }
}

fn core(dir: &tempfile::TempDir) -> Core {
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd, true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_provider_instance(Arc::new(Echo {
        capacity_key: crate::support::CapacityKey::default(),
    }));
    core
}

async fn header_of(core: Core) -> vak_session::types::SessionHeader {
    core.start_session()
        .await
        .unwrap()
        .header()
        .unwrap()
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_header_names_cause_for_each_surface() {
    let dir = tempfile::tempdir().unwrap();
    let base = core(&dir);

    for surface in [Surface::Cli, Surface::Web] {
        let header = header_of(base.clone().with_surface(surface)).await;
        assert!(
            matches!(header.cause, Some(Cause::User { .. })),
            "{:?}",
            header.cause
        );
        assert!(header.run.is_some());
        assert_eq!(header.space, None, "no Space id exists yet");
    }

    let channel = base
        .clone()
        .with_surface(Surface::Chat {
            channel: "telegram".into(),
        })
        .with_conversation_context(Some(ConversationContext {
            conversation_id: "telegram:42".into(),
            audience_id: "telegram:42".into(),
            origin: Some(vak_session::ConversationOrigin {
                surface: "telegram".into(),
                address: "42".into(),
                bot_id: Some("bot1".into()),
            }),
        }));
    let header = header_of(channel).await;
    match header.cause {
        Some(Cause::Channel { endpoint, .. }) => assert_eq!(endpoint, "telegram:42:bot1"),
        other => panic!("{other:?}"),
    }

    let schedule = base
        .clone()
        .with_surface(Surface::Background)
        .with_run_admission(RunAdmission::default().cause(Cause::Schedule {
            schedule: "task-1".into(),
            slot: "2026-10-02T09:00:00Z".into(),
        }));
    let header = header_of(schedule).await;
    assert_eq!(
        header.cause,
        Some(Cause::Schedule {
            schedule: "task-1".into(),
            slot: "2026-10-02T09:00:00Z".into()
        })
    );

    let manual = base
        .clone()
        .with_surface(Surface::Background)
        .with_run_admission(RunAdmission::default().cause(Cause::Trigger {
            trigger: TriggerId::derived("task-1"),
            request_id: "manual:1".into(),
        }));
    assert!(matches!(
        header_of(manual).await.cause,
        Some(Cause::Trigger { .. })
    ));

    let heartbeat = header_of(base.clone().with_surface(Surface::Background)).await;
    assert_eq!(heartbeat.cause, Some(Cause::Heartbeat));

    let revision = base
        .clone()
        .with_surface(Surface::Background)
        .with_run_admission(RunAdmission::default().cause(Cause::Revision {
            candidate: "cand-1".into(),
        }));
    assert_eq!(
        header_of(revision).await.cause,
        Some(Cause::Revision {
            candidate: "cand-1".into()
        })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_actor_is_the_real_actor() {
    let dir = tempfile::tempdir().unwrap();
    let base = core(&dir);

    let person = base
        .clone()
        .with_surface(Surface::Cli)
        .mint_trace(Some("r1"));
    assert_eq!(person.actor, Some(local::local_owner()));
    assert_eq!(person.on_behalf_of, None);
    assert_eq!(person.agent, local::agent("vak"));

    let sender = local::channel_sender("telegram", "42", "alice");
    let channel = base
        .clone()
        .with_surface(Surface::Chat {
            channel: "telegram".into(),
        })
        .with_run_admission(RunAdmission::default().actor(sender))
        .mint_trace(Some("r2"));
    assert_eq!(channel.actor, Some(sender));

    let routine = base
        .clone()
        .with_surface(Surface::Background)
        .mint_trace(None);
    assert_eq!(routine.actor, Some(local::agent_principal("vak")));
    assert_eq!(
        routine.on_behalf_of,
        Some(local::local_owner()),
        "Agent work is done on behalf of the owner"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_stamps_its_run_on_what_it_writes() {
    let dir = tempfile::tempdir().unwrap();
    let core = core(&dir).with_surface(Surface::Cli);
    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    core.run_turn_with_message(
        session,
        vak_session::MessageRecord {
            message: vak_llm::Message::user_text("hello"),
            meta: Some(vak_session::types::MessageMeta {
                request_id: Some("req-42".into()),
                ..Default::default()
            }),
        },
        CancellationToken::new(),
        None,
        None,
        None,
        tx,
    )
    .await
    .unwrap();

    let cost = vak_core::finops::FinOpsLedger::new(&core.shared_scope().into_root()).all_rows();
    let row = cost.last().expect("the turn wrote a cost row");
    let trace = row.trace.as_ref().expect("the cost row carries the run");
    assert_eq!(
        trace.cause,
        Cause::User {
            request_id: "req-42".into()
        }
    );
    assert_eq!(row.actor, Some(local::local_owner()));

    let activity =
        vak_core::finops::ActivityLedger::new(&core.shared_scope().into_root()).all_rows();
    let provider = activity
        .iter()
        .find(|row| row.kind == "provider")
        .expect("the provider dispatch is recorded");
    assert_eq!(
        provider.trace.as_ref().map(|t| t.run),
        Some(trace.run),
        "the cost row and the activity row of one turn are one trace"
    );
}
