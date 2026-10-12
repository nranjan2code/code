//! `PUT /config/bus` keeps the bus's secrets in the secret store, never in a
//! file (docs/plans/data-architecture-plan.md, M0): the NATS URL goes to the
//! project config, the credentials JWT and nkey seed to the project secret
//! scope, GET never returns them, and DELETE removes both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use vak_core::Core;

fn files_containing(root: &Path, needle: &str) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if std::fs::read(&path).is_ok_and(|bytes| {
                bytes
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes())
            }) {
                found.push(path);
            }
        }
    }
    found
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bus_credentials_never_written_to_a_file() {
    let isolated = vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let (ws, home) = (dir.path().join("ws"), dir.path().join("home"));
    std::fs::create_dir_all(&ws).unwrap();
    let core = Core::new_with_trust(ws.clone(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::router(core.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let jwt = format!("jwt-secret-{}", uuid::Uuid::now_v7().simple());
    let seed = format!("SUSEED{}", uuid::Uuid::now_v7().simple());
    let response = client
        .put(format!("{base}/config/bus"))
        .json(&serde_json::json!({
            "nats_url": "nats://127.0.0.1:1",
            "nats_credentials_jwt": jwt,
            "nats_nkey_seed": seed,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());

    for secret in [&jwt, &seed] {
        for root in [&ws, &home, &isolated] {
            assert!(
                files_containing(root, secret).is_empty(),
                "a secret was written to a file under {}",
                root.display()
            );
        }
    }
    assert_eq!(
        core.bus_secret(vak_config::BUS_NATS_JWT_VAR),
        Some(jwt.clone())
    );
    assert_eq!(
        core.bus_secret(vak_config::BUS_NATS_NKEY_SEED_VAR),
        Some(seed.clone())
    );
    let settings = std::fs::read_to_string(ws.join(".vak/config.toml")).unwrap();
    assert!(settings.contains("nats://127.0.0.1:1"), "{settings}");
    assert!(!ws.join(".vak/env").exists());

    let shown = client
        .get(format!("{base}/config/bus"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!shown.contains(&jwt) && !shown.contains(&seed), "{shown}");

    let removed = client
        .delete(format!("{base}/config/bus"))
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 200);
    assert_eq!(core.bus_secret(vak_config::BUS_NATS_JWT_VAR), None);
    assert_eq!(core.bus_secret(vak_config::BUS_NATS_NKEY_SEED_VAR), None);
    let settings = std::fs::read_to_string(ws.join(".vak/config.toml")).unwrap();
    assert!(!settings.contains("nats_url"), "{settings}");
}
