//! At its quota an install refuses new work and removes no record
//! (plan M7a-d part 5, docs/design/74 §3.3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use vak_core::lifecycle::QuotaState;
use vak_core::{Core, CoreError};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Answers {
    capacity_key: crate::support::CapacityKey,
}

#[async_trait::async_trait]
impl Provider for Answers {
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
            content: vec![ContentBlock::text("Noted.")],
            stop_reason: StopReason::EndTurn,
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

fn core(cwd: &std::path::Path, config: &str) -> Core {
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        format!("[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n{config}"),
    )
    .unwrap();
    let core = Core::new_with_trust(cwd.to_path_buf(), true).unwrap();
    core.set_provider_instance(Arc::new(Answers {
        capacity_key: crate::support::CapacityKey::default(),
    }));
    core
}

async fn turn(
    core: &Core,
    session: vak_session::SessionLog,
    prompt: &str,
) -> Result<vak_session::SessionLog, CoreError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    core.run_turn_with(
        session,
        prompt,
        CancellationToken::new(),
        None,
        None,
        None,
        tx,
    )
    .await
    .map(|(_, session)| session)
}

#[tokio::test]
async fn quota_refuses_admission_not_records() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();

    // With no limit set, work is admitted and nothing is measured against.
    let free = core(work.path(), "");
    assert_eq!(free.quota().state, QuotaState::None);
    let session = free.start_session().await.unwrap();
    let ledger = session.path().to_path_buf();
    let id = session.header().unwrap().session_id.clone();
    let session = turn(&free, session, "remember the launch is on the fourteenth")
        .await
        .unwrap();
    drop(session);
    let kept = || {
        vak_session::SessionLog::open_read_only(ledger.clone())
            .unwrap()
            .len()
    };
    let before = kept();
    assert!(before > 1);

    // A limit far below what the install already keeps: the next turn is
    // refused, with what is kept and the limit, and no record is removed.
    let full = core(
        work.path(),
        "\n[lifecycle]\nquota_gb = 0.000001\nmode = \"commit\"\n",
    );
    full.lifecycle_tick(true);
    let quota = full.quota();
    assert_eq!(quota.state, QuotaState::Hard);
    assert!(quota.kept_bytes >= quota.limit_bytes.unwrap());
    let reopened = full.open_session(&id).await.unwrap();
    let refused = match turn(&full, reopened, "one more thing").await {
        Err(error) => error,
        Ok(_) => panic!("work was admitted over the limit"),
    };
    assert!(matches!(refused, CoreError::OverQuota { .. }), "{refused}");
    assert!(refused.to_string().contains("Nothing was removed"));
    assert_eq!(
        kept(),
        before,
        "a refused turn writes nothing and removes nothing"
    );
    assert!(
        vak_session::SessionLog::text(&ledger).contains("the fourteenth"),
        "what was recorded is still readable"
    );

    // Raising the limit admits work again.
    let roomy = core(work.path(), "\n[lifecycle]\nquota_gb = 50\n");
    roomy.measure_storage();
    assert_eq!(roomy.quota().state, QuotaState::Ok);
    let reopened = roomy.open_session(&id).await.unwrap();
    turn(&roomy, reopened, "one more thing").await.unwrap();
    assert!(kept() > before);
}
