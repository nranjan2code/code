#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration coverage for docs/design/34-channel-onboarding.md Phase 2:
//! an allowlist entry approved for a workspace other than the gateway's
//! own default actually runs a session rooted at that other workspace,
//! not just its provider/model.
//!
//! `VAK_HOME` is set for the whole process below so pooled Cores started
//! for the non-default workspace resolve their sessions_home the normal
//! way (`Core::new_with_trust`, unmodified) inside a hermetic tempdir
//! instead of the real user's data home. This file has exactly one test
//! so that process-wide env mutation cannot race a sibling test.

mod support;

use std::sync::Arc;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct NoCred {
    capacity_key: crate::support::CapacityKey,
}

#[async_trait::async_trait]
impl Provider for NoCred {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }
    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(64);
        let m = AssistantMessage {
            content: vec![ContentBlock::text("unused")],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "test-model".into(),
            response_id: None,
        };
        sink.close_message(m).await;
        Ok(rx)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approved_entry_routes_to_its_own_workspace_core() {
    let vak_home = tempfile::tempdir().unwrap();
    vak_config::paths::set_home_override(vak_home.path());

    let default_dir = tempfile::tempdir().unwrap();
    let other_dir = tempfile::tempdir().unwrap();
    let _ = std::fs::create_dir_all(other_dir.path());

    let default_cwd = default_dir.path().to_path_buf();
    let _ = std::fs::create_dir_all(default_cwd.join(".vak"));
    let _ = std::fs::write(
        default_cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n[gateway]\nchat_allowlist_open = false\n",
    );
    let core = Core::new_with_trust(default_cwd.clone(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(
        vak_home.path().join("default-home"),
    ));
    core.set_provider_instance(Arc::new(NoCred {
        capacity_key: crate::support::CapacityKey::default(),
    }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with(core, true);
    let _server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");

    let client = reqwest::ClientBuilder::new()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            h
        })
        .build()
        .unwrap();

    // First message: unknown key becomes pending, not silently allowed.
    let msg = serde_json::json!({
        "surface": "webhook", "chat": "ci-other-ws", "sender": "bot",
        "text": "hello", "wait": false,
    });
    let res = client
        .post(format!("{base}/gateway/inbound"))
        .json(&msg)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "pending");

    // Operator approves, explicitly naming a workspace other than the
    // gateway's own default — the Phase 1 fix for silent inheritance.
    let approve_res = client
        .post(format!(
            "{base}/admin/api/gateway/allowlist/webhook%3Aci-other-ws/approve"
        ))
        .json(&serde_json::json!({ "workspace": other_dir.path() }))
        .send()
        .await
        .unwrap();
    assert_eq!(approve_res.status(), 200, "approve must succeed");

    // Second message: now allowed. Whether or not the turn can reach a
    // provider, session creation happens first, so the pooled Core for
    // `other_dir` must already have started and minted a session there by
    // the time we inspect it.
    let res = client
        .post(format!("{base}/gateway/inbound"))
        .json(&msg)
        .send()
        .await
        .unwrap();
    // Admitted either way: 202 when the route is keyless (the default
    // local provider starts the run), 503 when a keyed provider has no
    // credential. Session creation precedes both, which is what the checks
    // below are about.
    let status = res.status();
    assert!(
        status == reqwest::StatusCode::ACCEPTED
            || status == reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "an approved chat must be admitted to the other workspace's pooled Core, got {status}"
    );

    // Confirm: the pool now reports the other workspace as warm.
    let status: serde_json::Value = client
        .get(format!("{base}/admin/api/gateway/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pool_entries = status["core_pool"]["entries"].as_array().unwrap();
    let other_canonical = other_dir.path().canonicalize().unwrap();
    assert!(
        pool_entries
            .iter()
            .any(|e| e["workspace"].as_str() == Some(other_canonical.to_str().unwrap())),
        "expected {other_canonical:?} to appear warm in the core pool, got {pool_entries:?}"
    );

    // Confirm: the session actually minted lives under the OTHER
    // workspace's sessions_home (the normal, unmodified `Core::new_with_trust`
    // resolution under VAK_HOME), with a ledger header whose cwd is that
    // workspace — not the gateway's own default cwd.
    let mut found_header_cwd: Option<String> = None;
    for entry in walkdir::WalkDir::new(vak_home.path()).into_iter().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Some(first_line) = raw.lines().next()
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(first_line)
            && let Some(cwd) = v.get("cwd").and_then(|c| c.as_str())
        {
            found_header_cwd = Some(cwd.to_string());
        }
    }
    assert_eq!(
        found_header_cwd.as_deref(),
        Some(other_canonical.to_str().unwrap()),
        "session ledger header cwd must be the approved entry's own workspace, \
         not the gateway's default"
    );

    // Drop the pin so a later test in this binary resolves normally.
    vak_config::clear_override("VAK_HOME");
}
