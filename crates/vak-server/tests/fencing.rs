//! Fencing (plan M4.1): a server whose tenant store was restored after it
//! opened it stops its scheduler and dispatch, refuses new turns, and says
//! so in `/health`. Its own test binary, because being fenced is
//! process-wide and sticky.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use vak_core::Core;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fenced_process_stops_background_work() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().canonicalize().unwrap();
    vak_config::spaces::bind(&cwd).unwrap();
    let core = Core::new(cwd.clone()).unwrap();

    // The process holds the epoch it read at open and is alive.
    core.renew_liveness(chrono::Duration::seconds(60)).unwrap();
    let tenant_home = vak_config::paths::local_tenant_home();
    let process = vak_session::fence::process();
    let renewed = vak_session::fence::liveness(&tenant_home, &process)
        .unwrap()
        .unwrap();

    // The store is restored under it.
    let tenant = vak_session::objects::TenantObjects::for_tenant(&tenant_home).unwrap();
    tenant.store().restore().unwrap();

    // The scheduler's first tick fires as the router starts.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let health: serde_json::Value = client
        .get(format!("{base}/health"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["status"], "fenced", "{health}");
    assert_eq!(health["fenced"], true);
    assert_eq!(health["posture"], "fenced");
    assert_eq!(health["process"], process.to_string());
    // The scheduler never ran a tick, and liveness was never renewed, so
    // this process's leases lapse.
    assert_eq!(health["automation_scheduler"]["status"], "starting");
    assert!(health["automation_scheduler"]["last_tick_at"].is_null());
    assert_eq!(
        vak_session::fence::liveness(&tenant_home, &process)
            .unwrap()
            .unwrap(),
        renewed
    );

    // A new conversation is refused.
    let created = client
        .post(format!("{base}/sessions"))
        .bearer_auth(&token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    let status = created.status();
    let body = created.text().await.unwrap();
    assert!(!status.is_success(), "{status} {body}");
    assert!(body.contains("fenced"), "{body}");
}
