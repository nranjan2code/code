#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use vak_llm::{
    EventStream, LlmError, Provider, stream,
    types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage},
};
use vak_session::{Entry, EntryPayload, SessionPath};

#[derive(Default)]
struct Capture(Mutex<Vec<ChatRequest>>);
#[async_trait::async_trait]
impl Provider for Capture {
    fn name(&self) -> &str {
        "capture"
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.0.lock().unwrap().push(request);
        let (mut sink, rx) = stream::channel(8);
        let message = AssistantMessage {
            content: vec![ContentBlock::text("Your request is complete.")],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
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

async fn call(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn profile(id: &str, name: &str) -> Value {
    json!({"id":id,"revision":1,"name":name,"character":"vak","personality":"Use the phrase identity-marker.","behaviour":"Answer concisely.","responsibilities":"Research news", "animation":"off","voice":"default"})
}

async fn run(app: &Router, sid: &str, prompt: &str) {
    let (status, body) = call(
        app,
        "POST",
        &format!("/sessions/{sid}/run"),
        json!({"prompt":prompt,"request_id":uuid::Uuid::now_v7().to_string()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    for _ in 0..200 {
        let (_, sessions) = call(app, "GET", "/sessions", json!({})).await;
        if sessions["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["session_id"] == sid && s["running"] == false)
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("agent did not settle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_identity_survives_clients_restart_and_followups_without_cross_talk() {
    vak_config::paths::isolate_home_for_tests();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n",
    )
    .unwrap();
    let core = vak_core::Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(temp.path().join("sessions-home"));
    let capture = Arc::new(Capture::default());
    core.set_provider_instance(capture.clone());
    let app = vak_server::router(core.clone());
    let profiles =
        json!({"agents":[profile("newsy","Newsy"),profile("other","Other")],"scope":"workspace"});
    assert_eq!(
        call(&app, "PUT", "/config/agents", profiles).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/agents/missing/open", json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "POST", "/agents/vak/open", json!({})).await.0,
        StatusCode::OK
    );
    let (_, a) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    let sid = a["session_id"].as_str().unwrap().to_owned();
    // Newsy, a user-created agent, is resolved through a freshly-constructed
    // Core rooted at its own workspace (see agent_chats::open), but that
    // Core is repointed at the same sessions/data-home root the test's Vak
    // Core already uses (`set_sessions_home` above), just under its own
    // agent-scoped subdirectory — every agent's data lives under one root.
    let newsy_home = vak_config::paths::agent_home_at(&core.shared_data_home(), "newsy");
    let newsy_cwd = vak_config::paths::agent_workspace(&cwd, "newsy");
    assert_ne!(
        newsy_cwd, cwd,
        "a user-created agent must not share Vak's project workspace"
    );
    assert_eq!(
        std::fs::canonicalize(a["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(&newsy_cwd).unwrap()
    );
    let resolved_newsy_cwd = std::path::PathBuf::from(a["cwd"].as_str().unwrap());
    let ledger = SessionPath::new_session_file(&newsy_home, &resolved_newsy_cwd, &sid);
    assert!(
        !SessionPath::new_session_file(&core.sessions_home(), &cwd, &sid).exists(),
        "Newsy session must not be in Vak workspace"
    );
    let first = std::fs::read_to_string(ledger)
        .unwrap()
        .lines()
        .next()
        .map(|line| serde_json::from_str::<Entry>(line).unwrap())
        .unwrap();
    let EntryPayload::Header(header) = first.payload else {
        panic!("agent admission ledger must begin with a header");
    };
    let context = header
        .conversation
        .expect("Agent admission must stamp conversation context");
    assert_eq!(context.conversation_id, "agent:newsy:local");
    assert_eq!(context.audience_id, "local");
    assert_eq!(
        context
            .origin
            .as_ref()
            .map(|origin| origin.surface.as_str()),
        Some("desktop")
    );
    let (_, again) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    assert_eq!(
        again["session_id"], sid,
        "even header-only conversations must reopen"
    );
    let (_, sessions_list) = call(&app, "GET", "/sessions", json!({})).await;
    let found = sessions_list["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["session_id"] == sid);
    assert!(
        found,
        "newly opened active agent session must be present in list_sessions even if header-only"
    );
    let (_, b) = call(&app, "POST", "/agents/other/open", json!({})).await;
    assert_ne!(b["session_id"], sid);
    run(&app, &sid, "Remember private-newsy-marker").await;
    run(
        &app,
        b["session_id"].as_str().unwrap(),
        "A different request",
    )
    .await;
    {
        let requests = capture.0.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|r| r.system.as_deref().unwrap_or("").contains("You are Newsy."))
        );
        let other = requests
            .iter()
            .find(|r| r.system.as_deref().unwrap_or("").contains("You are Other."))
            .unwrap();
        assert!(
            !other
                .messages
                .iter()
                .any(|m| m.text_content().contains("private-newsy-marker"))
        );
    }
    drop(app);
    let app = vak_server::router(core.clone());
    let (_, reopened) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    assert_eq!(reopened["session_id"], sid);
    run(&app, &sid, "What did I ask you to remember?").await;
    {
        let requests = capture.0.lock().unwrap();
        let last = requests.last().unwrap();
        assert!(
            last.system
                .as_deref()
                .unwrap_or("")
                .contains("identity-marker")
        );
        assert!(
            last.messages
                .iter()
                .any(|m| m.text_content().contains("private-newsy-marker"))
        );
        assert!(
            !last
                .messages
                .iter()
                .any(|m| m.text_content().contains("Use my Agent"))
        );
    }
    let (_, transcript) = call(
        &app,
        "GET",
        &format!("/sessions/{sid}/transcript"),
        json!({}),
    )
    .await;
    assert!(transcript.to_string().contains("private-newsy-marker"));
    // Editing the catalogue never rewrites an admitted conversation's identity.
    call(
        &app,
        "PUT",
        "/config/agents",
        json!({"agents":[profile("newsy","Renamed"),profile("other","Other")]}),
    )
    .await;
    let (_, frozen) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    assert_eq!(frozen["agent"]["name"], "Newsy");
    assert_eq!(frozen["agent"]["revision"], 1);
    // Shared writes are not copied into the project layer; effective reads merge.
    call(
        &app,
        "PUT",
        "/config/agents",
        json!({"scope":"user","agents":[profile("shared-helper","Shared helper")]}),
    )
    .await;
    let (_, project_layer) = call(&app, "GET", "/config/agents?scope=workspace", json!({})).await;
    assert!(
        project_layer
            .get("agents")
            .and_then(|v| v.as_array())
            .is_some()
    );
    assert!(!project_layer.to_string().contains("shared-helper"));
    let (_, effective) = call(&app, "GET", "/agents", json!({})).await;
    assert!(effective.get("agents").and_then(|v| v.as_array()).is_some());
    assert!(effective.to_string().contains("shared-helper"));
    // Same identity in another workspace has a separate ledger.
    let other_dir = temp.path().join("other-workspace");
    std::fs::create_dir_all(&other_dir).unwrap();
    let other_core = vak_core::Core::new_with_trust(other_dir, true).unwrap();
    other_core.set_sessions_home(core.shared_data_home());
    other_core.set_provider_instance(capture);
    let other_app = vak_server::router(other_core);
    call(
        &other_app,
        "PUT",
        "/config/agents",
        json!({"agents":[profile("newsy","Newsy")]}),
    )
    .await;
    let (_, isolated) = call(&other_app, "POST", "/agents/newsy/open", json!({})).await;
    assert_ne!(isolated["session_id"], sid);

    // Concurrent process simulation: a second server instance sharing the same workspace
    // and sessions home (e.g. gateway service when desktop application holds the writer lock).
    let concurrent_core = vak_core::Core::new_with_trust(cwd.clone(), true).unwrap();
    concurrent_core.set_sessions_home(core.shared_data_home());
    concurrent_core.set_provider_instance(Arc::new(Capture::default()));
    let concurrent_app = vak_server::router(concurrent_core);

    let (status, opened) = call(&concurrent_app, "POST", "/agents/newsy/open", json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "concurrent open must succeed read-only when locked by desktop"
    );
    assert_eq!(
        opened["session_id"], sid,
        "must match the canonical session id"
    );

    let (transcript_status, transcript) = call(
        &concurrent_app,
        "GET",
        &format!("/sessions/{sid}/transcript"),
        json!({}),
    )
    .await;
    assert_eq!(
        transcript_status,
        StatusCode::OK,
        "transcript must be readable when session is locked"
    );
    assert!(transcript["messages"].as_array().is_some());
}

/// Memory notes, learning proposals, and skill proposals are Agent-scoped
/// (docs/design/23-memory.md: "Per-agent data home: sessions, memory, and
/// agent-specific config") — a note or proposal written for one Agent must
/// never appear for, or be mutated by, a different Agent's `?agent=` view.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn memory_and_proposals_are_scoped_per_agent() {
    vak_config::paths::isolate_home_for_tests();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let core = vak_core::Core::new_with_trust(cwd, true).unwrap();
    core.set_sessions_home(temp.path().join("sessions-home"));
    let app = vak_server::router(core);
    assert_eq!(
        call(
            &app,
            "PUT",
            "/config/agents",
            json!({"agents":[profile("newsy","Newsy"),profile("other","Other")],"scope":"workspace"}),
        )
        .await
        .0,
        StatusCode::OK
    );

    // Each agent writes a note only it should ever see.
    for (agent, marker) in [
        ("newsy", "newsy-private-note"),
        ("other", "other-private-note"),
    ] {
        let (status, _) = call(
            &app,
            "POST",
            "/memory",
            json!({"text": marker, "agent": agent}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "append for {agent}");
    }

    let (_, newsy_notes) = call(&app, "GET", "/memory?agent=newsy", json!({})).await;
    let newsy_text = newsy_notes.to_string();
    assert!(newsy_text.contains("newsy-private-note"));
    assert!(!newsy_text.contains("other-private-note"));

    let (_, other_notes) = call(&app, "GET", "/memory?agent=other", json!({})).await;
    let other_text = other_notes.to_string();
    assert!(other_text.contains("other-private-note"));
    assert!(!other_text.contains("newsy-private-note"));

    // The default (built-in) agent, and an unscoped call, see neither.
    let (_, default_notes) = call(&app, "GET", "/memory", json!({})).await;
    let default_text = default_notes.to_string();
    assert!(!default_text.contains("newsy-private-note"));
    assert!(!default_text.contains("other-private-note"));

    // Forgetting newsy's note must not touch other's.
    let newsy_note_id = newsy_notes["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["text"] == "newsy-private-note")
        .and_then(|n| n["id"].as_str())
        .unwrap()
        .to_string();
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/memory/{newsy_note_id}?agent=newsy"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, other_notes_after) = call(&app, "GET", "/memory?agent=other", json!({})).await;
    assert!(
        other_notes_after.to_string().contains("other-private-note"),
        "deleting newsy's note must not affect other's"
    );
}

/// Set up two custom Agents (mirroring `memory_and_proposals_are_scoped_per_agent`)
/// and return `(app, core)`.
async fn two_agent_app() -> (Router, vak_core::Core, tempfile::TempDir) {
    vak_config::paths::isolate_home_for_tests();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let core = vak_core::Core::new_with_trust(cwd, true).unwrap();
    core.set_sessions_home(temp.path().join("sessions-home"));
    let app = vak_server::router(core.clone());
    assert_eq!(
        call(
            &app,
            "PUT",
            "/config/agents",
            json!({"agents":[profile("newsy","Newsy"),profile("other","Other")],"scope":"workspace"}),
        )
        .await
        .0,
        StatusCode::OK
    );
    // The caller must hold onto the returned `TempDir` for the rest of the
    // test — dropping it here (as a purely local variable would be, once
    // this function returns) deletes the on-disk workspace out from under
    // every subsequent request, which is exactly what produced the
    // spurious "This agent is no longer available" 404s while this helper
    // was being written.
    (app, core, temp)
}

/// `GET /config/prompts/effective` assembles the merged prompt off whichever
/// `Core` the request resolves to — it must resolve per-`?agent=` exactly
/// like its sibling `GET /config/prompts` layer endpoint, not fall back to
/// the default "vak" Agent regardless of the query (the gap this test
/// closes: `get_prompt_effective` was left reading `state.core` directly
/// when the rest of the prompt endpoints were generalized to
/// `resolve_scoped_core`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prompt_effective_is_scoped_per_agent() {
    let (app, _core, _temp) = two_agent_app().await;

    let (status, _) = call(
        &app,
        "PUT",
        "/config/prompts",
        json!({
            "scope": "workspace",
            "block": "guardrails",
            "text": "newsy-only-guardrail-line",
            "agent": "newsy",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, newsy_effective) = call(
        &app,
        "GET",
        "/config/prompts/effective?agent=newsy",
        json!({}),
    )
    .await;
    assert!(
        newsy_effective["text"]
            .as_str()
            .unwrap_or_default()
            .contains("newsy-only-guardrail-line"),
        "newsy's own effective prompt must include its guardrails override: {newsy_effective}"
    );

    let (_, default_effective) = call(&app, "GET", "/config/prompts/effective", json!({})).await;
    assert!(
        !default_effective["text"]
            .as_str()
            .unwrap_or_default()
            .contains("newsy-only-guardrail-line"),
        "the default agent must not see newsy's effective prompt: {default_effective}"
    );
}

/// `PUT /config/permissions` writes one Agent's own workspace rule layer —
/// same isolation contract as hooks/MCP, checked here for permission rules.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn permission_rules_are_scoped_per_agent() {
    let (app, _core, _temp) = two_agent_app().await;

    let (status, _) = call(
        &app,
        "PUT",
        "/config/permissions",
        json!({
            "scope": "workspace",
            "deny": ["bash(rm -rf /)"],
            "agent": "newsy",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, newsy_rules) = call(&app, "GET", "/config/permissions?agent=newsy", json!({})).await;
    assert!(
        newsy_rules["layer"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "bash(rm -rf /)"),
        "newsy's own rule layer must include its deny rule: {newsy_rules}"
    );

    let (_, default_rules) = call(&app, "GET", "/config/permissions", json!({})).await;
    assert!(
        !default_rules["layer"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "bash(rm -rf /)"),
        "the default agent must not see newsy's deny rule: {default_rules}"
    );
}

/// `PATCH /finops` sets budget caps in one Agent's own `.vak/config.toml` —
/// same isolation contract as hooks/MCP, checked here for FinOps caps.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finops_caps_are_scoped_per_agent() {
    let (app, _core, _temp) = two_agent_app().await;

    let (status, _) = call(
        &app,
        "PATCH",
        "/finops",
        json!({
            "max_run_usd": 1.5,
            "agent": "newsy",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, newsy_status) = call(&app, "GET", "/finops?agent=newsy", json!({})).await;
    assert_eq!(newsy_status["run_cap_usd"].as_f64(), Some(1.5));

    let (_, default_status) = call(&app, "GET", "/finops", json!({})).await;
    assert_ne!(
        default_status["run_cap_usd"].as_f64(),
        Some(1.5),
        "the default agent must not see newsy's run cap: {default_status}"
    );
}

/// `GET /sessions/{id}/checkpoints` must resolve a *closed* session's real
/// owning Agent via `?agent=` rather than silently falling back to the
/// default Agent once no in-memory session handle exists for it — the exact
/// gap `resolve_scoped_core`'s session-id-first, agent-id-fallback path
/// exists to close.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checkpoints_resolve_the_owning_agent_once_the_session_is_closed() {
    let (app, core, _temp) = two_agent_app().await;

    // Seed a checkpoint directly under Newsy's isolated workspace/home,
    // exactly as a real run would have via `vak_core::checkpoints::capture`
    // — no session handle is ever registered for `sid`, simulating a
    // session that closed (or a server restart) before this request.
    let newsy_cwd = vak_config::paths::agent_workspace(core.cwd(), "newsy");
    std::fs::create_dir_all(&newsy_cwd).unwrap();
    let newsy_home = vak_config::paths::agent_home_at(&core.shared_data_home(), "newsy");
    let sid = "closed-newsy-session";
    let cp = vak_core::checkpoints::capture(&newsy_cwd, sid, 1, "seed").unwrap();
    vak_core::checkpoints::store(&newsy_home, &cp).unwrap();

    let (status, newsy_checkpoints) = call(
        &app,
        "GET",
        &format!("/sessions/{sid}/checkpoints?agent=newsy"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{newsy_checkpoints}");
    assert_eq!(
        newsy_checkpoints["checkpoints"].as_array().unwrap().len(),
        1,
        "newsy's own checkpoint must be found via ?agent=newsy: {newsy_checkpoints}"
    );

    let (_, default_checkpoints) = call(
        &app,
        "GET",
        &format!("/sessions/{sid}/checkpoints"),
        json!({}),
    )
    .await;
    assert!(
        default_checkpoints["checkpoints"]
            .as_array()
            .unwrap()
            .is_empty(),
        "the default agent must not see newsy's checkpoint: {default_checkpoints}"
    );
}

/// A user-created Agent's isolated workspace is never separately visited or
/// trust-prompted, so without `agents::save` carrying the creating context's
/// own trust decision forward onto it (`vak_core::trust::mark_trusted`), its
/// own privileged config — `permission_mode`, `hooks`, `mcp.servers`, ... —
/// gets silently stripped by `CorePool::resolve_at` forever, no matter how
/// correctly it's scoped per-Agent. This is the live-tested gap that
/// surfaced *after* every other per-Agent scoping fix in this file: a write
/// through `/config/mode?agent=newsy` persisted correctly to newsy's own
/// `config.toml`, but read back as the untouched default because the
/// directory it lived in had no trust marker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_agents_workspace_is_trusted_when_its_creator_is() {
    let (app, _core, _temp) = two_agent_app().await;

    let (status, _) = call(
        &app,
        "POST",
        "/config/mode",
        json!({"mode": "ReadOnly", "agent": "newsy"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, newsy_config) = call(&app, "GET", "/config?agent=newsy", json!({})).await;
    assert_eq!(
        newsy_config["permission_mode"], "ReadOnly",
        "newsy's own permission mode must actually take effect once its \
         workspace is trusted, not silently stay at the untrusted default: \
         {newsy_config}"
    );
}

/// `PUT /config/hooks` writes the project's own `[[hooks]]` array
/// (docs/design/45-prompt-layers.md-adjacent config-layer semantics) — a
/// hook saved for one Agent must not appear in, or be overwritten by, a
/// different Agent's `?agent=` view, since each Agent has its own isolated
/// workspace file (see commit 15c9c256's memory/proposals precedent).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hooks_are_scoped_per_agent() {
    let (app, _core, _temp) = two_agent_app().await;

    let (status, body) = call(
        &app,
        "PUT",
        "/config/hooks",
        json!({
            "hooks": [{"event": "session_start", "command": "echo newsy-hook", "enabled": true}],
            "agent": "newsy",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, newsy_hooks) = call(&app, "GET", "/config/hooks?agent=newsy", json!({})).await;
    assert!(newsy_hooks.to_string().contains("newsy-hook"));

    let (_, other_hooks) = call(&app, "GET", "/config/hooks?agent=other", json!({})).await;
    assert!(
        !other_hooks.to_string().contains("newsy-hook"),
        "other agent must not see newsy's hook"
    );

    let (_, default_hooks) = call(&app, "GET", "/config/hooks", json!({})).await;
    assert!(
        !default_hooks.to_string().contains("newsy-hook"),
        "the default agent must not see newsy's hook either"
    );
}

/// `PUT /config/mcp` writes the project's own MCP server map — same
/// isolation contract as hooks above, checked here for the MCP config
/// layer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_servers_are_scoped_per_agent() {
    let (app, _core, _temp) = two_agent_app().await;

    let (status, _) = call(
        &app,
        "PUT",
        "/config/mcp",
        json!({
            "servers": {"newsy-server": {"command": "true", "args": [], "env": {}}},
            "agent": "newsy",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, newsy_servers) = call(&app, "GET", "/config/mcp?agent=newsy", json!({})).await;
    assert!(newsy_servers.to_string().contains("newsy-server"));

    let (_, other_servers) = call(&app, "GET", "/config/mcp?agent=other", json!({})).await;
    assert!(
        !other_servers.to_string().contains("newsy-server"),
        "other agent must not see newsy's MCP server"
    );

    let (_, default_servers) = call(&app, "GET", "/config/mcp", json!({})).await;
    assert!(
        !default_servers.to_string().contains("newsy-server"),
        "the default agent must not see newsy's MCP server either"
    );
}

/// The plugin store (retired-plugin sweep, catalog sources, key
/// revocation, install/enable/disable/rollback/remove) is rooted at
/// `<Agent's own cwd>/.vak` (`plugin_store`) — revoking a signing key for
/// one Agent's workspace-scoped plugin store must not touch a different
/// Agent's, or the default Agent's, own store.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plugin_key_revocation_is_scoped_per_agent() {
    let (app, _core, _temp) = two_agent_app().await;

    let (status, _) = call(
        &app,
        "POST",
        "/plugins/keys/test-signing-key/revoke?agent=newsy&scope=workspace",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Read each Agent's own plugin registry file directly — there is no
    // dedicated "list revoked keys" endpoint, and the registry file is
    // exactly what every plugin_store-backed endpoint (install, enable,
    // disable, rollback, remove, sources) reads and writes.
    let newsy_root = vak_config::paths::agent_workspace(_core.cwd(), "newsy").join(".vak");
    let other_root = vak_config::paths::agent_workspace(_core.cwd(), "other").join(".vak");
    let default_root = _core.cwd().join(".vak");

    let newsy_registry = vak_plugin::PluginStore::new(newsy_root)
        .load_sources()
        .unwrap();
    assert!(
        newsy_registry.revoked_keys.contains("test-signing-key"),
        "newsy's own store must record the revocation"
    );

    let other_registry = vak_plugin::PluginStore::new(other_root)
        .load_sources()
        .unwrap();
    assert!(
        !other_registry.revoked_keys.contains("test-signing-key"),
        "other agent's plugin store must not see newsy's key revocation"
    );

    let default_registry = vak_plugin::PluginStore::new(default_root)
        .load_sources()
        .unwrap();
    assert!(
        !default_registry.revoked_keys.contains("test-signing-key"),
        "the default agent's plugin store must not see newsy's key revocation either"
    );
}

/// Isolated, credential-free browser fixture. Never reads the operator's home.
/// Run with: cargo test -p vak-server --test agent_chats browser_fixture -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual browser verification server; stops after ten minutes"]
async fn browser_fixture() {
    vak_config::paths::isolate_home_for_tests();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n",
    )
    .unwrap();
    let core = vak_core::Core::new_with_trust(cwd, true).unwrap();
    core.set_sessions_home(temp.path().join("sessions"));
    core.set_provider_instance(Arc::new(Capture::default()));
    let app = vak_server::router(core);
    call(
        &app,
        "PUT",
        "/config/agents",
        json!({"agents":[profile("newsy","Newsy"),profile("other","Other")]}),
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!(
        "AGENT_BROWSER_URL=http://{}/app",
        listener.local_addr().unwrap()
    );
    let server = axum::serve(listener, app).with_graceful_shutdown(async {
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
    });
    server.await.unwrap();
}
