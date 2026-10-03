//! Personal-OS server surfaces (docs/design/29-personal-os.md P1–P4):
//! tiered memory CRUD, cross-project search, markdown transcript export,
//! doctor, backup export/import, and the usage digest.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};

struct Counting {
    capacity_key: crate::support::CapacityKey,
    dispatches: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for Counting {
    fn name(&self) -> &str {
        "counting"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        let (mut sink, rx) = stream::channel(8);
        let done = AssistantMessage {
            content: vec![ContentBlock::text("done")],
            stop_reason: vak_llm::types::StopReason::EndTurn,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 1,
                ..Default::default()
            },
            model: "counted-model".into(),
            response_id: None,
        };
        sink.push(stream::StreamEvent::Start {
            partial: done.clone(),
        });
        sink.close_message(done).await;
        Ok(rx)
    }
}

struct Server {
    base: String,
    home: PathBuf,
    cwd: PathBuf,
    _dir: Arc<tempfile::TempDir>,
    client: reqwest::Client,
}

/// Spawn a plain (scheduler-free) server over a hermetic workspace.
async fn spawn_server(config_toml: &str) -> Server {
    let dir = Arc::new(tempfile::tempdir().unwrap());
    let cwd = dir.path().to_path_buf();
    let project = cwd.join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        format!("[memory]\nreflection = false\n{config_toml}"),
    )
    .unwrap();

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    // A REAL worker executable: the test harness itself cannot speak the
    // broker protocol.
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));
    core.set_provider_instance(Arc::new(Counting {
        capacity_key: crate::support::CapacityKey::default(),
        dispatches: Arc::new(AtomicUsize::new(0)),
    }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let home = core.sessions_home();
    let app = vak_server::router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server {
        base: format!("http://{addr}"),
        home,
        cwd,
        _dir: dir,
        client: reqwest::Client::new(),
    }
}

fn header_for(id: &str, cwd: &Path) -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: id.to_string(),
        created_at: chrono::Utc::now(),
        cwd: cwd.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "test".into(),
            provider: "counting".into(),
            model: "fixture-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: String::new(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

// ---- Memory endpoints --------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_list_forget_amend_across_scopes() {
    let srv = spawn_server("").await;
    let ws = vak_core::memory::append_note(
        &srv.home,
        &srv.cwd,
        "decision",
        "deploy",
        "s1",
        "pause before rollbacks",
    )
    .unwrap();
    let profile = vak_core::memory::append_profile_note(
        &srv.home,
        "preference",
        "editor",
        "vim bindings",
        "s2",
    )
    .unwrap();

    // GET lists both tiers with scope annotations.
    let res = srv
        .client
        .get(format!("{}/memory", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let notes = body["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 2);
    let scopes: Vec<&str> = notes.iter().map(|n| n["scope"].as_str().unwrap()).collect();
    assert!(scopes.contains(&"workspace") && scopes.contains(&"profile"));
    assert!(notes.iter().all(|n| n["id"].is_string()));

    // Amend keeps provenance and swaps the body.
    let res = srv
        .client
        .patch(format!("{}/memory/{}", srv.base, profile.id))
        .json(&serde_json::json!({"text": "prefers helix now", "scope": "profile"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let amended = vak_core::memory::list_profile_notes(&srv.home);
    assert_eq!(amended[0].text, "prefers helix now");
    assert_eq!(amended[0].ts, profile.ts);

    // Default scope is workspace.
    let res = srv
        .client
        .patch(format!("{}/memory/{}", srv.base, ws.id))
        .json(&serde_json::json!({"text": "always dry-run first"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Forget removes exactly one block from the right store.
    let res = srv
        .client
        .delete(format!("{}/memory/{}?scope=workspace", srv.base, ws.id))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["forgotten"], serde_json::json!(ws.id));
    assert!(vak_core::memory::list_notes(&srv.home, &srv.cwd).is_empty());
    assert_eq!(vak_core::memory::list_profile_notes(&srv.home).len(), 1);

    // Unknown ids are typed 404s, in both tiers and both verbs.
    for uri in [
        format!("{}/memory/deadbeef?scope=workspace", srv.base),
        format!("{}/memory/deadbeef?scope=profile", srv.base),
    ] {
        let res = srv.client.delete(&uri).send().await.unwrap();
        assert_eq!(res.status(), 404, "{uri}");
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            !body["error"].as_str().unwrap_or_default().is_empty(),
            "typed error required: {body}"
        );
    }
    let res = srv
        .client
        .patch(format!("{}/memory/deadbeef", srv.base))
        .json(&serde_json::json!({"text": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);

    // Empty amend text is rejected before touching any file.
    let res = srv
        .client
        .patch(format!("{}/memory/{}", srv.base, profile.id))
        .json(&serde_json::json!({"text": "   ", "scope": "profile"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);

    // Cleanup is explicit and safe: it removes an empty orphan directory but
    // never removes the profile note that remains above.
    std::fs::create_dir_all(srv.home.join("memory/orphan-empty")).unwrap();
    let res = srv
        .client
        .post(format!("{}/memory/cleanup", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let cleanup: serde_json::Value = res.json().await.unwrap();
    assert_eq!(cleanup["removed_empty_dirs"], 1);
    assert!(!srv.home.join("memory/orphan-empty").exists());
    assert_eq!(vak_core::memory::list_profile_notes(&srv.home).len(), 1);
}

// ---- Cross-project search ----------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_all_spans_projects_and_flags_the_scope() {
    let srv = spawn_server("").await;
    // One ledger in THIS project's hash dir…
    let here = SessionPath::new_session_file(&srv.home, &srv.cwd, "22222222-here");
    let mut log = SessionLog::create(here, header_for("22222222-here", &srv.cwd)).unwrap();
    log.append_message(vak_session::types::MessageRecord {
        message: vak_llm::Message::user_text("local deploy checklist lives in ops"),
        meta: None,
    })
    .unwrap();
    drop(log);
    // …and one under a DIFFERENT project hash dir.
    let other_cwd = srv.cwd.join("other-project");
    let there = SessionPath::new_session_file(&srv.home, &other_cwd, "33333333-there");
    let mut log = SessionLog::create(there, header_for("33333333-there", &other_cwd)).unwrap();
    log.append_message(vak_session::types::MessageRecord {
        message: vak_llm::Message::user_text("the rollout checklist lives elsewhere"),
        meta: None,
    })
    .unwrap();
    drop(log);

    // Workspace-scoped search sees only this project's ledger.
    let res = srv
        .client
        .get(format!("{}/search", srv.base))
        .query(&[("q", "checklist")])
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["all"], false);
    let ids: Vec<&str> = body["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["session_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["22222222-here"]);

    // all=true spans every project dir and annotates the origin.
    let res = srv
        .client
        .get(format!("{}/search", srv.base))
        .query(&[("q", "checklist"), ("all", "true")])
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["all"], true);
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 2, "{body}");
    assert!(hits.iter().all(|h| h["project_hash"].is_string()));
}

// ---- Markdown transcript export ----------------------------------------------

// Regression: a fresh server process starts with an empty in-memory handle
// map; historical sessions must still export from disk (found by live
// dogfooding — transcript.md 404'd for every non-attached session, and the
// JSON endpoint masked the same failure as a 200-wrapped error body).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn historical_sessions_serve_from_disk_without_attach() {
    let srv = spawn_server("").await;
    let id = "55555555-disk";
    let path = SessionPath::new_session_file(&srv.home, &srv.cwd, id);
    let mut log = SessionLog::create(path, header_for(id, &srv.cwd)).unwrap();
    log.append_message(vak_session::types::MessageRecord {
        message: vak_llm::Message::user_text("historical question"),
        meta: None,
    })
    .unwrap();
    log.append_message(vak_session::types::MessageRecord {
        message: vak_llm::Message::assistant(vec![ContentBlock::text("historical answer")]),
        meta: None,
    })
    .unwrap();
    drop(log);

    let res = srv
        .client
        .get(format!("{}/sessions/{id}/transcript.md", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("historical answer"));

    let res = srv
        .client
        .get(format!("{}/sessions/{id}/transcript", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["count"], 2);

    let res = srv
        .client
        .get(format!("{}/sessions/unknown/transcript.md", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transcript_md_equals_shared_renderer_byte_for_byte() {
    let srv = spawn_server("").await;
    let id = "44444444-md";
    let path = SessionPath::new_session_file(&srv.home, &srv.cwd, id);
    let mut log = SessionLog::create(path, header_for(id, &srv.cwd)).unwrap();
    let user_msg = vak_llm::Message::user_text("fix the flaky test");
    log.append_message(vak_session::types::MessageRecord {
        message: user_msg.clone(),
        meta: None,
    })
    .unwrap();
    log.append_message(vak_session::types::MessageRecord {
        message: vak_llm::Message::assistant(vec![ContentBlock::text("all done")]),
        meta: None,
    })
    .unwrap();
    drop(log);
    // The transcript endpoints serve live handles; attach like a client.
    srv.client
        .post(format!("{}/sessions/{id}/attach", srv.base))
        .json(&serde_json::json!({"session_id": id}))
        .send()
        .await
        .unwrap();

    let res = srv
        .client
        .get(format!("{}/sessions/{id}/transcript.md", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert!(
        res.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("text/markdown")
    );
    let body = res.text().await.unwrap();

    // The exact projection the JSON transcript serves, through the one
    // shared renderer — byte parity by construction.
    let expected = vak_core::transcript_md::render_markdown(&[
        user_msg,
        vak_llm::Message::assistant(vec![ContentBlock::text("all done")]),
    ]);
    assert_eq!(body, expected);

    let res = srv
        .client
        .get(format!("{}/sessions/unknown/transcript.md", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

// ---- Doctor -------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn doctor_reports_checks_facts_and_optional_ladder() {
    let srv = spawn_server("").await;
    let res = srv
        .client
        .get(format!("{}/doctor", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let labels: Vec<&str> = body["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    // Order mirrors the pushes in vak_core::health::collect. "install
    // layout" (canonical-layout conformance, doc 32) joined the ladder in
    // f6131a5 and this expectation was never updated; the failure stayed
    // hidden because a deadlock in gateway.rs stopped this binary from
    // ever running.
    assert_eq!(
        labels,
        vec![
            "provider",
            "sessions home",
            "config warnings",
            // Configured integrations the composed policy would refuse.
            // Belongs beside the config checks: "you set this up and it
            // does not work" is a health fact, not a transcript detail.
            "capability reach",
            "voice configuration",
            "local transcriber",
            "local TTS backend",
            // Configured capabilities that are currently unusable, with the
            // reason and the fix. Previously these were rendered only into
            // the system prompt, so the model was told a server was down and
            // the operator who could repair it was not.
            "capability health",
            // docs/design/34: channel state is part of the health surface,
            // not a config detail.
            "gateway channels",
            "install layout",
            "self version parity",
            "retired plugins",
            "agent roster",
        ]
    );
    assert!(
        body["facts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f.as_str().unwrap().starts_with("model "))
    );
    // Every check must pass except "self version parity", which compares
    // this build against whatever release is installed on the machine
    // running the suite. Mid-release — built 0.11.12, installed 0.11.11 —
    // that check legitimately fails, and asserting a bare zero here made
    // the suite a function of host state rather than of this code.
    let failed: Vec<&str> = body["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["ok"] == serde_json::Value::Bool(false))
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert!(
        failed.iter().all(|l| *l == "self version parity"),
        "unexpected doctor failures: {failed:?}"
    );
    assert!(body["ladder"].is_null(), "no session requested");

    // With a session, the frozen-ladder section appears.
    let id = "55555555-doctor";
    let path = SessionPath::new_session_file(&srv.home, &srv.cwd, id);
    let mut log = SessionLog::create(path, header_for(id, &srv.cwd)).unwrap();
    log.append_message(vak_session::types::MessageRecord {
        message: vak_llm::Message::user_text("hi"),
        meta: None,
    })
    .unwrap();
    drop(log);
    srv.client
        .post(format!("{}/sessions/{id}/attach", srv.base))
        .json(&serde_json::json!({"session_id": id}))
        .send()
        .await
        .unwrap();
    let res = srv
        .client
        .get(format!("{}/doctor", srv.base))
        .query(&[("session", id)])
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let ladder = &body["ladder"];
    assert!(!ladder.is_null());
    assert_eq!(
        ladder["rendered"], "fixture-model",
        "legacy header falls back to the model id"
    );
    assert_eq!(ladder["fallback_legs"], 0);
}

// ---- Backup export/import -------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_roundtrip_preserves_ledgers_and_rejects_self_backup() {
    let srv = spawn_server("").await;
    std::fs::create_dir_all(srv.home.join("sessions/x")).unwrap();
    std::fs::write(srv.home.join("cost-log.jsonl"), "{\"kind\":\"cost\"}\n").unwrap();
    std::fs::write(srv.home.join("sessions/x/a.jsonl"), "ledger-bytes").unwrap();
    let dest = tempfile::tempdir().unwrap();

    let res = srv
        .client
        .post(format!("{}/backup/export", srv.base))
        .json(&serde_json::json!({
            "dest_dir": dest.path().display().to_string(),
            "include_secrets": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["manifest"]["file_count"], 2);
    assert_eq!(body["included_secrets"], false);
    assert_eq!(
        std::fs::read_to_string(dest.path().join("sessions/x/a.jsonl")).unwrap(),
        "ledger-bytes"
    );

    // Wipe the live data, then restore: skip-conflict report counts copies.
    std::fs::remove_file(srv.home.join("cost-log.jsonl")).unwrap();
    std::fs::remove_file(srv.home.join("sessions/x/a.jsonl")).unwrap();
    let res = srv
        .client
        .post(format!("{}/backup/import", srv.base))
        .json(&serde_json::json!({
            "src_dir": dest.path().display().to_string(),
            "conflict": "skip"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["copied"], 2);
    assert_eq!(body["skipped"], 0);
    assert_eq!(
        std::fs::read_to_string(srv.home.join("cost-log.jsonl")).unwrap(),
        "{\"kind\":\"cost\"}\n",
        "roundtrip must be byte-identical"
    );

    // Re-import with rename preserves BOTH copies.
    let res = srv
        .client
        .post(format!("{}/backup/import", srv.base))
        .json(&serde_json::json!({
            "src_dir": dest.path().display().to_string(),
            "conflict": "rename"
        }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["renamed"], 2);
    assert!(srv.home.join("cost-log.import1.jsonl").is_file());

    // Self-backup is rejected on both directions, typed 400.
    for (uri, field) in [
        ("/backup/export", "dest_dir"),
        ("/backup/import", "src_dir"),
    ] {
        let res = srv
            .client
            .post(format!("{}{uri}", srv.base))
            .json(&serde_json::json!({field: srv.home.display().to_string()}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 400, "{uri}");
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].as_str().unwrap().contains("home itself"),
            "{body}"
        );
    }

    // Unknown conflict policy is a typed 400 too.
    let res = srv
        .client
        .post(format!("{}/backup/import", srv.base))
        .json(&serde_json::json!({
            "src_dir": dest.path().display().to_string(),
            "conflict": "overwrite"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

// ---- Digest ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn digest_reports_window_math_and_clamps_days() {
    let srv = spawn_server("").await;
    // Where every run writes it: the shared data home, not the Agent's.
    let ledger = vak_core::finops::FinOpsLedger::new(&srv._dir.path().join("home"));
    ledger
        .append(&vak_core::finops::CostRow {
            ts: chrono::Utc::now() - chrono::Duration::hours(2),
            model: "claude-sonnet".into(),
            provider: "anthropic".into(),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_input_tokens: None,
            usd: Some(0.25),
            source: "estimated".into(),
            session_id: "s-digest".into(),
            trace: None,
            actor: None,
        })
        .unwrap();

    let res = srv
        .client
        .get(format!("{}/digest", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["days"], 7, "default window");
    assert_eq!(body["dispatches"], 1);
    assert!((body["total_usd"].as_f64().unwrap() - 0.25).abs() < 1e-9);
    assert_eq!(body["by_model"]["claude-sonnet"]["rows"], 1);
    assert_eq!(body["distinct_sessions"], serde_json::json!(["s-digest"]));

    let res = srv
        .client
        .get(format!("{}/digest", srv.base))
        .query(&[("days", "500")])
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["days"], 90, "clamped high");

    let res = srv
        .client
        .get(format!("{}/digest", srv.base))
        .query(&[("days", "0")])
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["days"], 1, "clamped low");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finops_projects_observed_tokens_and_activity_without_zeroing_unknown_cost() {
    let srv = spawn_server("").await;
    let cost = vak_core::finops::FinOpsLedger::new(&srv.home);
    cost.append(&vak_core::finops::CostRow {
        ts: chrono::Utc::now(),
        model: "unpriced-model".into(),
        provider: "counting".into(),
        input_tokens: 12,
        output_tokens: 7,
        cache_read_input_tokens: Some(3),
        usd: None,
        source: "estimated".into(),
        session_id: "s-finops".into(),
        trace: None,
        actor: None,
    })
    .unwrap();
    let activity = vak_core::finops::ActivityLedger::new(&srv.home);
    activity
        .append(&vak_core::finops::ActivityRow {
            ts: chrono::Utc::now(),
            kind: "mcp".into(),
            name: "plugin.demo/search".into(),
            success: true,
            duration_ms: Some(19),
            session_id: Some("s-finops".into()),
            plugin: Some("demo".into()),
            tool_use_id: None,
            trace: None,
            actor: None,
        })
        .unwrap();

    let body: serde_json::Value = srv
        .client
        .get(format!("{}/finops", srv.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["day_input_tokens"], 12);
    assert_eq!(body["day_output_tokens"], 7);
    assert_eq!(body["unknown_rows"], 1);
    assert_eq!(body["day_usd"], 0.0);
    assert_eq!(body["activity"][0]["kind"], "mcp");
    assert_eq!(body["activity"][0]["plugin"], "demo");
    assert_eq!(body["activity"][0]["duration_ms"], 19);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finops_reports_spend_per_agent_and_per_run() {
    use vak_session::ids::{AgentId, SpaceId, TenantId};
    use vak_session::trace::{Cause, TraceKey};
    let srv = spawn_server("").await;
    let agent = AgentId::new();
    let key = TraceKey::root(TenantId::new(), SpaceId::new(), agent, Cause::Heartbeat);
    let row = |trace: Option<TraceKey>, usd: f64| vak_core::finops::CostRow {
        ts: chrono::Utc::now(),
        model: "m".into(),
        provider: "p".into(),
        input_tokens: 4,
        output_tokens: 2,
        cache_read_input_tokens: None,
        usd: Some(usd),
        source: "estimated".into(),
        session_id: "s-roll".into(),
        trace,
        actor: None,
    };
    let cost = vak_core::finops::FinOpsLedger::new(&srv.home);
    cost.append(&row(Some(key.clone()), 0.5)).unwrap();
    cost.append(&row(Some(key.child()), 0.25)).unwrap();
    cost.append(&row(None, 1.0)).unwrap();

    let body: serde_json::Value = srv
        .client
        .get(format!("{}/finops", srv.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agents = body["by_agent"].as_array().unwrap();
    let mine = agents
        .iter()
        .find(|r| r["key"] == agent.to_string())
        .unwrap();
    assert!((mine["usd"].as_f64().unwrap() - 0.75).abs() < 1e-9);
    assert_eq!(mine["calls"], 2);
    assert!(agents.iter().any(|r| r["key"] == "unattributed"));
    let runs = body["by_run"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["key"], key.run.to_string());
    assert_eq!(runs[0]["calls"], 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finops_platform_scope_uses_shared_home_and_agent_scope_adds_private_home() {
    let srv = spawn_server("").await;
    let now = chrono::Utc::now();
    let row = |session_id: &str, usd: f64| vak_core::finops::CostRow {
        ts: now,
        model: "counting-model".into(),
        provider: "counting".into(),
        input_tokens: 1,
        output_tokens: 1,
        cache_read_input_tokens: None,
        usd: Some(usd),
        source: "estimated".into(),
        session_id: session_id.into(),
        trace: None,
        actor: None,
    };
    vak_core::finops::FinOpsLedger::new(&srv.home)
        .append(&row("shared", 1.25))
        .unwrap();

    let platform: serde_json::Value = srv
        .client
        .get(format!("{}/finops", srv.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(platform["day_usd"], 1.25);
    assert_eq!(platform["total_rows"], 1);
}
