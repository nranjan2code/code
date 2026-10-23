//! Organisation has its own admitted work and cannot hold the answer open.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use vak_core::Core;
use vak_llm::{AssistantMessage, ChatRequest, ContentBlock, EventStream, LlmError, Provider};
use vak_session::work_threads::TopicStore;

#[derive(Default)]
struct ProviderFixture {
    capacity: crate::support::CapacityKey,
    entered: Notify,
    release: Notify,
    requests: Mutex<Vec<ChatRequest>>,
}
#[async_trait::async_trait]
impl Provider for ProviderFixture {
    fn name(&self) -> &str {
        "scripted"
    }
    fn rate_limit_key(&self) -> String {
        self.capacity.0.clone()
    }
    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let organising = request
            .output_schema
            .as_ref()
            .is_some_and(|s| s.name == "turn_organisation");
        let topic_id = if organising {
            let input: serde_json::Value =
                serde_json::from_str(&request.messages[0].text_content()).unwrap();
            input["topics"][0]["id"].clone()
        } else {
            serde_json::Value::Null
        };
        let metadata = serde_json::json!({"title":"Indian markets dashboard","outcomes":[{"outcome":"Dashboard","topics":[{"id":topic_id,"title":"Indian markets"}]}]}).to_string();
        let model = request.model.clone();
        self.requests.lock().unwrap().push(request);
        if organising {
            self.entered.notify_one();
            tokio::select! { _ = self.release.notified() => {}, _ = cancel.cancelled() => return Err(LlmError::InvalidRequest("cancelled fixture".into())) }
        }
        let mut reply = AssistantMessage::empty(model);
        reply.content = vec![ContentBlock::text(if organising {
            &metadata
        } else {
            "Created the dashboard. This is a plain answer without a heading."
        })];
        let (mut sink, rx) = vak_llm::stream::channel(8);
        sink.close_message(reply).await;
        Ok(rx)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answer_returns_while_organisation_waits_and_names_arrive_without_an_answer_heading() {
    let home = vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let config = vak_config::scope::WorkspaceScope::new(dir.path()).config_file();
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "[stop_policy]\nenabled = false\n").unwrap();
    let core = Core::new_with_trust(dir.path().into(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(home));
    let provider = Arc::new(ProviderFixture::default());
    core.set_provider_instance(provider.clone());
    let session = core.start_session().await.unwrap();
    let sid = session.header().unwrap().session_id.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let (outcome, session) = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        core.run_turn_with(
            session,
            "Make an Indian markets dashboard",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(outcome, vak_agent::TurnOutcome::Completed { .. }));
    let before = TopicStore::new(&core.scope(), &sid)
        .unwrap()
        .snapshot()
        .unwrap();
    assert_eq!(before.turn_names.len(), 1, "organisation was not queued");
    let trigger = vak_core::triggers::get(
        &core.shared_scope(),
        &before.turn_names[0].trigger.to_string(),
    )
    .unwrap();
    assert!(
        trigger.is_some(),
        "metadata exists but Trigger creation failed"
    );
    let trigger = trigger.unwrap();
    assert!(matches!(
        trigger.action,
        vak_core::triggers::TriggerAction::OrganiseTurn { .. }
    ));
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        provider.entered.notified(),
    )
    .await
    .unwrap_or_else(|_| {
        panic!(
            "metadata dispatch never entered: {:?}",
            core.runs()
                .of_trigger(&before.turn_names[0].trigger)
                .unwrap()
        )
    });
    let store = TopicStore::new(&core.scope(), &sid).unwrap();
    let pending = store.snapshot().unwrap();
    assert!(pending.turn_names[0].title.is_none());
    let requests = session.organisation_requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].output_schema.as_ref().unwrap().name,
        "turn_organisation"
    );
    assert!(
        requests[0].messages[0]
            .text_content()
            .contains("Make an Indian markets dashboard")
    );
    assert!(
        !session
            .derive_messages()
            .iter()
            .any(|m| m.text_content().contains("turn_organisation"))
    );
    let job = pending.turn_names[0].trigger;
    assert_eq!(
        core.runs().of_trigger(&job).unwrap()[0].status,
        vak_session::runs::RunStatus::Running
    );
    assert!(
        core.runs().of_trigger(&job).unwrap()[0]
            .sessions
            .contains(&sid)
    );
    provider.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if core
                .runs()
                .of_trigger(&job)
                .unwrap()
                .first()
                .is_some_and(|run| run.status == vak_session::runs::RunStatus::Completed)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let ready = store.snapshot().unwrap();
    assert_eq!(
        ready.turn_names[0].title.as_deref(),
        Some("Indian markets dashboard")
    );
    assert_eq!(ready.topics[0].title, "Indian markets");
    assert_eq!(
        ready.placements[0].topic_ids,
        vec![ready.topics[0].id.clone()]
    );
    let main_messages = provider
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.output_schema.is_none())
        .count();
    assert_eq!(
        main_messages, 1,
        "metadata never asks the answer loop to call a tool"
    );
    assert!(ready.turn_names[0].response.is_some());
    drain.await.unwrap();

    // A follow-up's wording alone carries no subject. Naming must receive the
    // same admitted addressed history used by the main answer, not a Topic
    // graph walk or the whole transcript.
    let first_turn = ready.turn_names[0].turn_id.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let (followup_outcome, session) = core
        .run_turn_with(
            session,
            "Improve that",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    assert!(
        matches!(followup_outcome, vak_agent::TurnOutcome::Completed { .. }),
        "{followup_outcome:?}"
    );
    assert_eq!(
        store.snapshot().unwrap().turn_names.len(),
        2,
        "followup context: {:?}; selections {:?}",
        session.organisation_context(&session.current_turn_index().turns.last().unwrap().id),
        session
            .active_entries_rev()
            .filter_map(|e| match &e.payload {
                vak_session::types::EntryPayload::ContextSelection(r) => Some((
                    &e.id,
                    &e.parent_id,
                    &e.at_turn,
                    r,
                    session.replay_context_selection(&e.id).map(|m| m.len())
                )),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        provider.entered.notified(),
    )
    .await
    .unwrap();
    let requests = session.organisation_requests().unwrap();
    assert_eq!(requests.len(), 2);
    let input: serde_json::Value =
        serde_json::from_str(&requests[1].messages[0].text_content()).unwrap();
    assert!(input["context"]["selection_entry"].is_string());
    assert_eq!(input["context"]["current"]["asked"], "Improve that");
    assert_eq!(input["context"]["history"][0]["turn_id"], first_turn);
    assert!(
        input["context"]["history"][0]["asked"]
            .as_str()
            .unwrap()
            .contains("Indian markets")
    );
    provider.release.notify_one();
    let pending = store.snapshot().unwrap();
    let second_job = pending.turn_names[1].trigger;
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if core
                .runs()
                .of_trigger(&second_job)
                .unwrap()
                .first()
                .is_some_and(|run| run.status == vak_session::runs::RunStatus::Completed)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        store.snapshot().unwrap().topics.len(),
        1,
        "follow-up reuses the addressed active Topic"
    );
    let (earlier_context, _) = session.organisation_context(&first_turn).unwrap();
    assert_eq!(earlier_context["current"]["turn_id"], first_turn);
    assert!(earlier_context["history"].as_array().unwrap().is_empty());
    assert!(
        !earlier_context.to_string().contains("Improve that"),
        "backfill must not admit later dialogue"
    );
    drain.await.unwrap();
}
