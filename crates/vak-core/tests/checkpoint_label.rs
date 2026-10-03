//! A checkpoint manifest is labelled with the turn id, never the prompt text:
//! the manifest is durable and must not carry conversation content.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checkpoint_label_has_no_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    let home = dir.path().join("home");
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.clone()));
    core.set_provider_instance(Arc::new(Echo {
        capacity_key: crate::support::CapacityKey::default(),
    }));

    let session = core.start_session().await.unwrap();
    let sid = session.header().unwrap().session_id.clone();
    let secret = "zebra-quokka-private-prompt";
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    core.run_turn_with(
        session,
        secret,
        CancellationToken::new(),
        None,
        None,
        None,
        tx,
    )
    .await
    .unwrap();

    let manifests = vak_core::checkpoints::list(&core.scope(), &sid).unwrap();
    assert!(!manifests.is_empty(), "the first turn always checkpoints");
    for m in manifests {
        assert!(
            !m.label.contains(secret),
            "label leaked the prompt: {}",
            m.label
        );
        assert!(m.label.starts_with("turn: "), "{}", m.label);
        assert!(m.label.len() > "turn: ".len());
    }
}
