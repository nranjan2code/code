//! M6.5 exit test `feed_pipeline_is_gone` (data-architecture plan M6.5c):
//! intake replaced the Python feed pipeline in the same change (invariant
//! 30). No script, route, config key, registry entry, install step or
//! frontend call of it is left, so nothing can run it again.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

/// Source files under `dir` (Rust, TypeScript, Python, shell, Docker),
/// leaving out built bundles and dependencies.
fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(
                name.as_str(),
                "target" | "node_modules" | "dist" | "dist-web" | ".git"
            ) {
                sources(&path, found);
            }
        } else if [".rs", ".ts", ".tsx", ".py", ".sh", ".toml", "Dockerfile"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            found.push(path);
        }
    }
}

#[tokio::test]
async fn feed_pipeline_is_gone() {
    let root = workspace_root();
    assert!(
        !root.join("scripts/feeds").exists(),
        "scripts/feeds is deleted"
    );

    let mut files = Vec::new();
    for dir in ["crates", "scripts", "docker"] {
        sources(&root.join(dir), &mut files);
    }
    let this = Path::new(file!()).file_name().unwrap();
    let banned = [
        "feed_ingest",
        "feed_mcp",
        "feed_search",
        "feeds_dir",
        "FeedSettings",
        "feeds.toml",
        "\"/feeds/",
        "`/feeds/",
        "scripts/feeds",
        "duckdb",
    ];
    let mut left = Vec::new();
    for file in &files {
        if file.file_name() == Some(this) {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap_or_default();
        for word in banned {
            if text.contains(word) {
                left.push(format!("{} names {word}", file.display()));
            }
        }
    }
    assert!(
        left.is_empty(),
        "the feed pipeline is still named: {left:#?}"
    );

    // No registry entry declares the pipeline's store or its config.
    assert!(
        vak_core::state::REGISTRY
            .iter()
            .all(|entry| !entry.path.contains("feeds")),
        "a registry entry still names the feed store"
    );

    // The server answers none of its routes.
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    for (method, path) in [
        (reqwest::Method::GET, "/feeds/items"),
        (reqwest::Method::GET, "/feeds/search?q=x"),
        (reqwest::Method::GET, "/feeds/sources"),
        (reqwest::Method::POST, "/feeds/ingest"),
    ] {
        let status = client
            .request(method, format!("http://{addr}{path}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 404, "{path}");
    }
    // Its replacement is there.
    let status = client
        .get(format!("http://{addr}/intake/sources"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);
}
