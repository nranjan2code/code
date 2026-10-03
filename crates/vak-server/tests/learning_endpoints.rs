#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;
use std::sync::Arc;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::ChatRequest;
use vak_llm::{EventStream, LlmError, Provider};

struct Empty {
    capacity_key: crate::support::CapacityKey,
}

#[async_trait::async_trait]
impl Provider for Empty {
    fn name(&self) -> &str {
        "empty"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(8);
        sink.close_error(LlmError::Parse("unused".into())).await;
        Ok(rx)
    }
}

fn header_for(cwd: &Path) -> String {
    let _ = cwd;
    String::new()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn memory_and_proposal_endpoints() {
    let _ = header_for;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().to_path_buf();

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(cwd.clone()).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.clone()));
    core.set_provider_instance(Arc::new(Empty {
        capacity_key: crate::support::CapacityKey::default(),
    }));
    std::mem::forget(dir);

    // Seed one durable note and one pending proposal through the same APIs
    // the tools use.
    vak_core::memory::append_note(
        &home,
        &cwd,
        "preference",
        "style",
        "seed-session",
        "prefer small PRs",
    )
    .unwrap();
    let pdir = home
        .join("skill-proposals")
        .join(vak_core::memory::hash_cwd(&cwd));
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(
        pdir.join("aabbccddeeff11223344556677889900112233445566778899aabbccddeeff00.md"),
        "---\nname: \"test-skill\"\ndescription: \"A queued skill\"\n---\n\nDo the thing.\n",
    )
    .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::router(core))
            .await
            .unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Memory listing.
    let res = client.get(format!("{base}/memory")).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let notes = body["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["kind"], "preference");
    assert_eq!(notes[0]["tag"], "style");

    // Proposal queue → promote → gone from queue, present in discovery.
    let res = client
        .get(format!("{base}/skills/proposals"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let proposals = body["proposals"].as_array().unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0]["name"], "test-skill");
    let id = proposals[0]["id"].as_str().unwrap().to_string();

    let res = client
        .post(format!("{base}/skills/proposals/{id}/promote"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["promoted"], "test-skill");

    let res = client
        .get(format!("{base}/skills/proposals"))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["proposals"].as_array().unwrap().len(), 0);
    assert!(home.join("skills/test-skill/SKILL.md").exists());

    // Unknown id answers 404-shaped error, not a panic.
    let res = client
        .post(format!("{base}/skills/proposals/nope/reject"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}
