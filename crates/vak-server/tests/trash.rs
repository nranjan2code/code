//! The trash (docs/plans/data-architecture-plan.md, M0): a session moved to
//! the trash is hidden from every list, every search a person or the model
//! can run, its transcript and export, and the digest, and comes back whole
//! when restored.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use vak_core::Core;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::Tool;

fn header_for(id: &str, cwd: &Path) -> SessionHeader {
    SessionHeader {
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
            provider: "scripted".into(),
            model: "m".into(),
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

/// Written where the built-in Agent keeps its ledgers, as a real run would.
fn write_session(home: &Path, cwd: &Path, id: &str, text: &str) {
    let agent_home = vak_config::paths::agent_home_at(home, "vak");
    let path = SessionPath::new_session_file(&agent_home, cwd, id);
    let mut header = header_for(id, cwd);
    header.agent = Some(vak_core::vak_agent_identity());
    let mut log = SessionLog::create(path, header).unwrap();
    log.append_message(vak_session::types::MessageRecord {
        message: vak_llm::Message {
            role: vak_llm::Role::User,
            content: vec![vak_llm::types::ContentBlock::text(text)],
        },
        meta: None,
    })
    .unwrap();
}

fn client_with(token: &str) -> reqwest::Client {
    reqwest::ClientBuilder::new()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            h
        })
        .build()
        .unwrap()
}

fn mentions(body: &serde_json::Value, id: &str) -> bool {
    body.to_string().contains(id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trashed_session_absent_from_every_search() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let kept = uuid::Uuid::now_v7().to_string();
    let gone = uuid::Uuid::now_v7().to_string();
    write_session(&home, &cwd, &kept, "the zanzibar itinerary is in the notes");
    write_session(
        &home,
        &cwd,
        &gone,
        "the zanzibar passport number is private",
    );
    let now = chrono::Utc::now().to_rfc3339();
    let cost_rows: String = [&kept, &gone]
        .iter()
        .map(|sid| {
            format!(
                "{}\n",
                serde_json::json!({
                    "ts": now, "model": "m", "provider": "p",
                    "input_tokens": 10, "output_tokens": 5, "usd": 0.01,
                    "source": "estimated", "session_id": sid,
                })
            )
        })
        .collect();
    std::fs::write(home.join("cost-log.jsonl"), cost_rows).unwrap();

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(home.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = client_with(&token);
    let get = |path: String| {
        let client = client.clone();
        let base = base.clone();
        async move {
            let res = client.get(format!("{base}{path}")).send().await.unwrap();
            let status = res.status().as_u16();
            let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
            (status, body)
        }
    };

    let res = client
        .post(format!("{base}/admin/api/store/rebuild"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let search_tool = vak_core::session_search::SessionSearchTool {
        sessions_home: home.clone(),
        trash_home: home.clone(),
        cwd: cwd.clone(),
        exclude_session_id: String::new(),
        agent_id: None,
        audience_id: None,
    };
    let ctx = vak_tools::ToolContext {
        cwd: cwd.clone(),
        cancel: tokio_util::sync::CancellationToken::new(),
        sandbox: None,
        sandbox_sink: None,
        agent_id: None,
    };
    let tool_search = || async {
        search_tool
            .execute(&serde_json::json!({"query": "zanzibar"}), &ctx)
            .await
            .content
    };

    // Every reader sees the session before it is trashed, so an absence
    // afterwards is the trash at work and not a reader that never saw it.
    let readers = [
        "/sessions".to_string(),
        "/search?q=zanzibar".to_string(),
        "/search?q=zanzibar&all=true".to_string(),
        "/admin/api/sessions".to_string(),
        "/admin/api/search?q=zanzibar".to_string(),
        "/digest".to_string(),
    ];
    for reader in &readers {
        let (status, body) = get(reader.clone()).await;
        assert_eq!(status, 200, "{reader}: {body}");
        assert!(mentions(&body, &gone), "{reader} before trash: {body}");
        assert!(mentions(&body, &kept), "{reader} before trash: {body}");
    }
    assert!(tool_search().await.contains(&gone));

    // Only an archived session can go to the trash.
    let res = client
        .delete(format!("{base}/sessions/{gone}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    let res = client
        .post(format!("{base}/sessions/{gone}/archive"))
        .json(&serde_json::json!({ "archived": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let res = client
        .delete(format!("{base}/sessions/{gone}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["trashed"], gone.as_str());

    for reader in &readers {
        let (status, body) = get(reader.clone()).await;
        assert_eq!(status, 200, "{reader}: {body}");
        assert!(!mentions(&body, &gone), "{reader} after trash: {body}");
        assert!(mentions(&body, &kept), "{reader} after trash: {body}");
    }
    let found = tool_search().await;
    assert!(!found.contains(&gone), "session_search: {found}");
    assert!(found.contains(&kept), "session_search: {found}");

    for path in [
        format!("/sessions/{gone}/transcript"),
        format!("/sessions/{gone}/transcript.md"),
    ] {
        let (status, _) = get(path.clone()).await;
        assert_eq!(status, 404, "{path}");
    }
    let (_, body) = get(format!("/admin/api/sessions/{gone}/transcript")).await;
    assert!(body.get("entries").is_none(), "admin transcript: {body}");
    let res = client
        .post(format!("{base}/sessions/{gone}/attach"))
        .json(&serde_json::json!({ "session_id": gone }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404, "a trashed session cannot be reopened");

    // The trash itself lists it, and restoring brings it back archived.
    let (_, trash) = get("/sessions?trash=true".to_string()).await;
    assert!(mentions(&trash, &gone), "{trash}");
    assert!(!mentions(&trash, &kept), "{trash}");
    let res = client
        .post(format!("{base}/sessions/{gone}/restore"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let (_, listed) = get("/sessions".to_string()).await;
    let restored = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["session_id"] == gone.as_str())
        .expect("restored session is listed again");
    assert_eq!(restored["archived"], true);
    let (_, found) = get("/search?q=zanzibar".to_string()).await;
    assert!(mentions(&found, &gone), "{found}");
}
