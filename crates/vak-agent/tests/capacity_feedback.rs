#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Capacity feedback (docs/design/68-context-engine.md §1 "Feedback"): a
//! provider that reports no prefill duration of its own (everyone but
//! Ollama) must still feed `prefill_tps`, from the wall-clock time between
//! sending the request and the first `StreamEvent` off the wire, measured
//! by `Agent::complete_with_reliability`'s own stream-consuming loop and
//! threaded through `StepLedger::last_first_token_ms`.

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, AutoApprove, SteeringQueues, TurnOutcome};
use vak_context::capacity::{
    CacheBehaviour, CapacityProfile, Horizon, ProbeProvenance, ProfileKey,
};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};

/// Answers after a deliberate delay so the first-token latency the harness
/// measures is unambiguously non-zero, and reports no `prefill_ms` of its
/// own (unlike Ollama) so the only way `prefill_tps` gets a sample is via
/// the caller-measured wall-clock latency.
struct SlowNoPrefillMs {
    capacity_key: String,
}

#[async_trait::async_trait]
impl Provider for SlowNoPrefillMs {
    fn name(&self) -> &str {
        "slow-no-prefill"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(64);
        tokio::time::sleep(Duration::from_millis(30)).await;
        let message = AssistantMessage {
            content: vec![ContentBlock::text("done")],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 500,
                output_tokens: 3,
                ..Default::default()
            },
            model: "test-model".into(),
            response_id: None,
        };
        sink.push(stream::StreamEvent::Start {
            partial: message.clone(),
        });
        sink.close_message(message).await;
        Ok(rx)
    }
}

fn session_paths(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let cwd = dir.to_path_buf();
    let home = cwd.join(".vak-home");
    (
        home.clone(),
        SessionPath::new_session_file(&home, &cwd, "capacity-feedback"),
    )
}

fn flat_profile(declared_window: u64) -> CapacityProfile {
    CapacityProfile::from_probe(
        Some(declared_window),
        None,
        Horizon {
            tokens: declared_window,
            confidence: 0.9,
            last_confirmed: std::time::SystemTime::now(),
        },
        CacheBehaviour::Unknown,
        1_024,
        ProbeProvenance {
            probed_at: std::time::SystemTime::now(),
            rungs: Vec::new(),
            signals: Vec::new(),
            metadata_digest: "digest".into(),
            quantisation: None,
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_ollama_provider_still_gets_a_prefill_tps_sample_from_wall_clock_latency() {
    let dir = tempdir().unwrap();
    let (home, path) = session_paths(dir.path());
    std::fs::create_dir_all(&home).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(SlowNoPrefillMs {
        capacity_key: crate::support::capacity_key(),
    });
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "capacity-feedback".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: provider.name().to_string(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(path, header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(AutoApprove));
    cfg.capacity = Some(flat_profile(128_000));

    let mut agent = Agent::new(provider, log, cfg);

    let (ev_tx, mut ev_rx) = mpsc::channel(256);
    tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
    let cancel = CancellationToken::new();
    let steering = SteeringQueues::new();
    let outcome = agent.run("hello", &steering, cancel, ev_tx).await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.into_session().await;
    let key = ProfileKey {
        provider: "slow-no-prefill".into(),
        model: "test-model".into(),
        quantisation: None,
    };
    let profile: CapacityProfile = session
        .latest_capacity_profile(&key)
        .expect("capacity feedback activity recorded with a matching profile");
    assert_eq!(
        profile.prefill_tps.samples, 1,
        "wall-clock first-token latency must feed prefill_tps even though \
         this provider never reports usage.prefill_ms itself"
    );
    assert!(
        profile.prefill_tps.value > 0.0,
        "prefill_tps value should be positive: {}",
        profile.prefill_tps.value
    );
}
