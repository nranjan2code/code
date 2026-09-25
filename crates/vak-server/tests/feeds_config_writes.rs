//! Concurrent edits to `.vak/feeds.toml` all land, and every reader sees a
//! whole document: each edit is a read-modify-write held under one lock and
//! published through a temporary file no other write shares.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use vak_core::Core;

fn source_ids(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let document: toml::Table = toml::from_str(&text)
        .unwrap_or_else(|error| panic!("feeds.toml does not parse: {error}\n{text}"));
    document
        .get("sources")
        .and_then(toml::Value::as_array)
        .map(|sources| {
            sources
                .iter()
                .filter_map(|source| source.get("id").and_then(toml::Value::as_str))
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_feed_source_adds_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("ws");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    let config = cwd.join(".vak").join("feeds.toml");
    // A key no feed handler writes; every edit must carry it through.
    std::fs::write(&config, "# kept\n[general]\nowner_note = \"mine\"\n").unwrap();

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).expect("core");
    core.set_sessions_home(dir.path().join("home"));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    const ROUNDS: usize = 20;
    const WRITERS: usize = 8;
    let mut expected = Vec::new();
    for round in 0..ROUNDS {
        let mut requests = Vec::new();
        for writer in 0..WRITERS {
            let id = format!("src-{round}-{writer}");
            expected.push(id.clone());
            let client = client.clone();
            let url = format!("{base}/feeds/sources");
            requests.push(tokio::spawn(async move {
                client
                    .post(url)
                    .json(&serde_json::json!({
                        "id": id,
                        "name": format!("Feed {id}"),
                        "type": "rss",
                        "url": "https://example.com/feed.xml",
                    }))
                    .send()
                    .await
                    .unwrap()
                    .status()
            }));
        }
        for request in requests {
            assert_eq!(request.await.unwrap(), 200);
        }
        let ids = source_ids(&config);
        for id in &expected {
            assert!(ids.contains(id), "round {round}: {id} was lost");
        }
        assert_eq!(ids.len(), expected.len(), "round {round}: {ids:?}");
    }

    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("owner_note = \"mine\""));
    let strays: Vec<_> = std::fs::read_dir(cwd.join(".vak"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(strays.is_empty(), "temporary files left behind: {strays:?}");
}
