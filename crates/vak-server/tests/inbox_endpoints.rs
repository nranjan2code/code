//! Inbox chokepoint + endpoints (docs/design/29-personal-os.md P6): every
//! delivery has a durable pull-side twin, watchdog output survives with
//! zero transports configured, and ack/read-state is idempotent over HTTP.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::BufRead;
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

struct Counting {
    dispatches: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for Counting {
    fn name(&self) -> &str {
        "counting"
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
    _dir: Arc<tempfile::TempDir>,
    client: reqwest::Client,
}

/// Spawn the FULL secured stack (bearer auth + scheduler) over a hermetic
/// workspace, exactly like the desktop shell does.
async fn spawn_server(config_toml: &str) -> Server {
    let dir = Arc::new(tempfile::tempdir().unwrap());
    let cwd = dir.path().join("ws");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        format!("[memory]\nreflection = false\n{config_toml}"),
    )
    .unwrap();

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd, true).unwrap();
    let home = dir.path().join("home");
    core.set_sessions_home(home.clone());
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    // A REAL worker executable: the harness cannot speak the broker
    // protocol, and watchdog scripts run through it.
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));
    core.set_provider_instance(Arc::new(Counting {
        dispatches: Arc::new(AtomicUsize::new(0)),
    }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
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
    Server {
        base: format!("http://{addr}"),
        home,
        _dir: dir,
        client,
    }
}

impl Server {
    async fn get_json(&self, path: &str) -> serde_json::Value {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn post_status(&self, path: &str) -> reqwest::StatusCode {
        self.client
            .post(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn post_json(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        let res = self
            .client
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json::<serde_json::Value>().await.unwrap())
    }

    async fn create_task(&self, body: serde_json::Value) -> String {
        let (status, _) = self.post_json("/tasks", body).await;
        assert_eq!(status, 200, "create failed");
        let list = self.get_json("/tasks").await;
        list["tasks"][0]["id"].as_str().unwrap().to_string()
    }

    async fn run_now(&self, id: &str) -> reqwest::StatusCode {
        self.post_status(&format!("/tasks/{id}/run-now")).await
    }

    async fn inbox(&self) -> serde_json::Value {
        self.get_json("/inbox").await
    }
}

fn delivery_lines(home: &Path) -> Vec<(String, String)> {
    let Ok(f) = std::fs::File::open(home.join("gateway").join("deliveries.jsonl")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            out.push((
                v["target"].as_str().unwrap_or_default().to_string(),
                v["text"].as_str().unwrap_or_default().to_string(),
            ));
        }
    }
    out
}

async fn wait_until(secs: u64, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if pred() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    pred()
}

/// Synchronous ledger read for poll loops (async endpoints cannot be
/// awaited inside a plain closure).
fn ledger_entries(home: &Path) -> Vec<vak_core::inbox::Entry> {
    vak_core::inbox::list(home, vak_core::inbox::MAX_SCAN)
}

// ---- P6 exit criterion ------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watchdog_summary_lands_in_inbox_with_zero_transports() {
    let srv = spawn_server("").await;

    // No deliver_to anywhere: no transport can carry this output.
    let tid = srv
        .create_task(serde_json::json!({
            "name": "quiet-watch", "script": "echo inbox-signal-42",
            "interval_secs": 3600
        }))
        .await;
    assert_eq!(srv.run_now(&tid).await, 202);

    assert!(
        wait_until(15, || {
            ledger_entries(&srv.home).iter().any(|e| {
                e.kind == vak_core::inbox::Kind::TaskSummary
                    && e.title == "watchdog 'quiet-watch'"
                    && e.body.contains("inbox-signal-42")
                    && e.task_id.as_deref() == Some(tid.as_str())
            })
        })
        .await,
        "watchdog summary never landed in the inbox"
    );
    // The durable ledger itself exists on disk — not just an endpoint view.
    assert!(srv.home.join("inbox.jsonl").is_file());
    // Zero transports configured: nothing was delivered anywhere else.
    assert!(
        !wait_until(2, || !delivery_lines(&srv.home).is_empty()).await,
        "no delivery may happen without a configured transport"
    );
}

// ---- Ack idempotence + unread math ------------------------------------------

async fn ack_entry(srv: &Server, id: &str) -> (reqwest::StatusCode, serde_json::Value) {
    srv.post_json(&format!("/inbox/{id}/ack"), serde_json::json!({}))
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ack_is_idempotent_over_http_and_404s_unknown_ids() {
    let srv = spawn_server("").await;
    let a = vak_core::inbox::record(
        &srv.home,
        vak_core::inbox::Kind::Digest,
        "daily",
        "",
        None,
        None,
    )
    .unwrap();
    let b = vak_core::inbox::record(
        &srv.home,
        vak_core::inbox::Kind::TaskSummary,
        "run",
        "body",
        None,
        None,
    )
    .unwrap();

    let view = srv.inbox().await;
    assert_eq!(view["entries"].as_array().unwrap().len(), 2);
    assert_eq!(view["unread_count"], 2);

    let (status, body) = ack_entry(&srv, &a.id).await;
    assert_eq!(status, 200);
    assert_eq!(body["acked"], true, "{body}");
    let (status, body) = ack_entry(&srv, &a.id).await;
    assert_eq!(status, 200);
    assert_eq!(body["acked"], false, "re-ack is a no-op: {body}");

    let (status, _) = ack_entry(&srv, "deadbeef0000").await;
    assert_eq!(status, 404, "unknown id must 404, not report acked:false");

    assert_eq!(srv.get_json("/inbox/unread_count").await["count"], 1);
    let unread = srv.get_json("/inbox?unread=true").await;
    let left = unread["entries"].as_array().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["id"], b.id.as_str());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unread_count_matches_entries_and_limit_bounds_only_the_list() {
    let srv = spawn_server("").await;
    for i in 0..3 {
        vak_core::inbox::record(
            &srv.home,
            vak_core::inbox::Kind::Heartbeat,
            &format!("beat-{i}"),
            "",
            None,
            None,
        )
        .unwrap();
    }

    let view = srv.get_json("/inbox").await;
    assert_eq!(view["entries"].as_array().unwrap().len(), 3);
    assert_eq!(view["unread_count"], 3);

    // limit truncates the window without touching the count.
    let capped = srv.get_json("/inbox?limit=2").await;
    assert_eq!(capped["entries"].as_array().unwrap().len(), 2);
    assert_eq!(capped["unread_count"], 3);

    // Ack everything through the endpoint; unread drains to zero while the
    // plain list keeps every entry (tombstones never delete).
    for e in view["entries"].as_array().unwrap() {
        let id = e["id"].as_str().unwrap();
        assert_eq!(ack_entry(&srv, id).await.1["acked"], true);
    }
    assert_eq!(srv.get_json("/inbox/unread_count").await["count"], 0);
    assert!(
        srv.get_json("/inbox?unread=true").await["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(srv.inbox().await["entries"].as_array().unwrap().len(), 3);
}

// ---- Budget alert twin --------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budget_alert_recorded_once_per_window_alongside_delivery() {
    let srv = spawn_server("[finops]\nmax_day_usd = 10.0\n").await;

    // Day spend already past the 80% threshold of the $10 cap.
    let ledger = vak_core::finops::FinOpsLedger::new(&srv.home);
    ledger
        .append(&vak_core::finops::CostRow {
            ts: chrono::Utc::now(),
            model: "claude-sonnet".into(),
            provider: "anthropic".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_input_tokens: None,
            usd: Some(9.0),
            source: "estimated".into(),
            session_id: "seed".into(),
        })
        .unwrap();

    let tid = srv
        .create_task(serde_json::json!({
            "name": "budget-probe", "script": "true",
            "interval_secs": 3600, "deliver_to": "log:budget"
        }))
        .await;

    // First fire crosses the threshold: one delivery AND one inbox entry.
    assert_eq!(srv.run_now(&tid).await, 202);
    assert!(
        wait_until(15, || {
            delivery_lines(&srv.home)
                .iter()
                .any(|(t, x)| t == "log:budget" && x.contains("budget alert [eighty]"))
        })
        .await,
        "budget alert never delivered"
    );
    assert!(
        wait_until(5, || {
            ledger_entries(&srv.home).iter().any(|e| {
                e.kind == vak_core::inbox::Kind::BudgetAlert
                    && e.session_id.as_deref() == Some(tid.as_str())
                    && e.body.contains("budget alert [eighty]")
            })
        })
        .await,
        "budget alert never recorded in the inbox"
    );
    let budget_entries = |view: &serde_json::Value| {
        view["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "budget_alert")
            .count()
    };
    assert_eq!(budget_entries(&srv.inbox().await), 1);

    // Second fire inside the same day window: no redelivery, no new entry.
    assert_eq!(srv.run_now(&tid).await, 202);
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let deliveries = delivery_lines(&srv.home)
        .iter()
        .filter(|(_, x)| x.contains("budget alert [eighty]"))
        .count();
    assert_eq!(deliveries, 1, "same-level alert must not redeliver");
    assert_eq!(
        budget_entries(&srv.inbox().await),
        1,
        "same-level alert must not re-record"
    );
}
