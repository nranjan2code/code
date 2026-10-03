//! Scheduler upgrade behaviors (docs/design/29-personal-os.md P2): cron
//! schedules with startup catch-up, zero-token watchdog script tasks over
//! the brokered bash path, per-task model pinning verified through work
//! receipts, and once-per-window budget alerts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

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
                input_tokens: 9,
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

struct Srv {
    base: String,
    token: String,
    home: PathBuf,
    dispatches: Arc<AtomicUsize>,
}

impl Srv {
    fn client(&self) -> reqwest::Client {
        reqwest::ClientBuilder::new()
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", self.token).parse().unwrap(),
                );
                h
            })
            .build()
            .unwrap()
    }

    async fn get_json(&self, path: &str) -> serde_json::Value {
        self.client()
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

fn git_seed(cwd: &Path) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["init", "-q"]);
    std::fs::write(cwd.join("README.md"), "seed\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "seed"]);
}

struct Fixture {
    srv: Srv,
    #[allow(dead_code)]
    dir: Arc<tempfile::TempDir>,
}

/// Build a hermetic git workspace + home and start the FULL stack (bearer
/// auth + scheduler). `seed_tasks` receives `(home, ws)` BEFORE the server
/// starts — writing `<home>/tasks.json` there is the simulated-downtime
/// injection point, since the scheduler loads it once at startup.
async fn spawn_full(
    config_toml: &str,
    seed_tasks: Option<impl for<'a> FnOnce(&'a Path, &'a Path) -> serde_json::Value>,
) -> Fixture {
    let dir = Arc::new(tempfile::tempdir().unwrap());
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join(".vak")).unwrap();
    std::fs::write(
        ws.join(".vak/config.toml"),
        format!("[memory]\nreflection = false\n{config_toml}"),
    )
    .unwrap();
    git_seed(&ws);

    let home = dir.path().join("home");
    if let Some(build) = seed_tasks {
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            vak_core::tasks::tasks_file(&home),
            serde_json::to_string_pretty(&build(&home, &ws)).unwrap(),
        )
        .unwrap();
    }

    let dispatches = Arc::new(AtomicUsize::new(0));
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(ws.clone(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.clone()));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    // A REAL worker executable: a cargo test harness cannot speak the
    // broker protocol.
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));
    core.set_provider_instance(Arc::new(Counting {
        capacity_key: crate::support::CapacityKey::default(),
        dispatches: dispatches.clone(),
    }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with(core, false);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture {
        srv: Srv {
            base: format!("http://{addr}"),
            token,
            home,
            dispatches,
        },
        dir,
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

async fn task_field(srv: &Srv, id: &str, field: &str) -> serde_json::Value {
    let body = srv.get_json("/tasks").await;
    body["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .map(|t| t[field].clone())
        .unwrap_or(serde_json::Value::Null)
}

async fn wait_summary(srv: &Srv, id: &str, secs: u64, pred: impl Fn(&str) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "task '{id}' summary never matched"
        );
        let v = task_field(srv, id, "last_summary").await;
        let matched = match v.as_str() {
            Some(s) => pred(s),
            None => false,
        };
        if matched {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

fn stale_script_task(
    id: &str,
    name: &str,
    ws: &Path,
    deliver_to: &str,
    script: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": name,
        "prompt": "",
        "interval_secs": 3600,
        "enabled": true,
        "cwd": ws.display().to_string(),
        "created_at": chrono::Utc::now().to_rfc3339(),
        // Two hours of downtime: every "* * * * *" slot since this instant
        // was missed.
        "last_run_at": (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339(),
        "last_session_id": null,
        "last_summary": null,
        "last_wt": null,
        "deliver_to": deliver_to,
        "schedule": "* * * * *",
        "script": script,
        "model_pin": null
    })
}

// ---- Catch-up -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_fires_missed_cron_slot_exactly_once_with_zero_dispatch() {
    let fx = spawn_full(
        "",
        Some(|_home: &Path, ws: &Path| {
            serde_json::json!([stale_script_task(
                "cu-1",
                "nightly-watch",
                ws,
                "log:cu",
                "echo caught-up-42"
            )])
        }),
    )
    .await;

    // The missed slot fires immediately at startup, stdout verbatim.
    assert!(
        wait_until(10, || delivery_lines(&fx.srv.home)
            .iter()
            .any(|(t, x)| t == "log:cu" && x.contains("caught-up-42")))
        .await,
        "catch-up never delivered"
    );
    // Exactly once: no duplicate delivery after things settle.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let hits = delivery_lines(&fx.srv.home)
        .iter()
        .filter(|(_, x)| x.contains("caught-up-42"))
        .count();
    assert_eq!(hits, 1, "catch-up must fire exactly once");

    // Watchdogs never touch the LLM: zero provider dispatches overall.
    assert_eq!(fx.srv.dispatches.load(Ordering::SeqCst), 0);

    // The record moved forward (no longer two hours stale).
    wait_summary(&fx.srv, "cu-1", 5, |s| s.contains("caught-up-42")).await;
    let last = task_field(&fx.srv, "cu-1", "last_run_at").await;
    let last = chrono::DateTime::parse_from_rfc3339(last.as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(
        chrono::Utc::now() - last < chrono::Duration::seconds(30),
        "last_run_at should be refreshed by the catch-up fire"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_disabled_waits_for_next_slot() {
    let fx = spawn_full(
        "[automation]\ncatch_up_missed = false\n",
        Some(|_home: &Path, ws: &Path| {
            serde_json::json!([stale_script_task(
                "cu-off",
                "quiet-watch",
                ws,
                "log:cuoff",
                "echo not-caught-up-99"
            )])
        }),
    )
    .await;

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert!(
        !delivery_lines(&fx.srv.home)
            .iter()
            .any(|(_, x)| x.contains("not-caught-up-99")),
        "disabled catch-up must not fire missed slots"
    );
    assert_eq!(fx.srv.dispatches.load(Ordering::SeqCst), 0);
}

// ---- Watchdog script matrix -----------------------------------------------------

async fn create_task(srv: &Srv, body: serde_json::Value) -> String {
    let res = srv
        .client()
        .post(format!("{}/tasks", srv.base))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "create failed: {}", res.status());
    let list = srv.get_json("/tasks").await;
    list["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == body["name"])
        .map(|t| t["id"].as_str().unwrap().to_string())
        .unwrap()
}

async fn run_now(srv: &Srv, id: &str) -> reqwest::StatusCode {
    srv.client()
        .post(format!("{}/tasks/{id}/run-now", srv.base))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn script_watchdog_matrix_silent_stdout_failure_and_delivery() {
    let fx = spawn_full("", None::<fn(&Path, &Path) -> serde_json::Value>).await;
    let srv = &fx.srv;

    let ok_id = create_task(
        srv,
        serde_json::json!({
            "name": "ok-watch", "script": "echo watchdog-hello",
            "schedule": "0 0 29 2 *", "deliver_to": "log:w-ok"
        }),
    )
    .await;
    let silent_id = create_task(
        srv,
        serde_json::json!({
            "name": "silent-watch", "script": "true",
            "schedule": "0 0 29 2 *", "deliver_to": "log:w-silent"
        }),
    )
    .await;
    let fail_id = create_task(
        srv,
        serde_json::json!({
            "name": "fail-watch", "script": "echo broken >&2; exit 3",
            "schedule": "0 0 29 2 *"
        }),
    )
    .await;

    // Success delivers trimmed stdout verbatim.
    assert_eq!(run_now(srv, &ok_id).await, 202);
    wait_summary(srv, &ok_id, 15, |s| s == "watchdog-hello").await;
    if !wait_until(10, || {
        delivery_lines(&srv.home)
            .iter()
            .any(|(t, x)| t == "log:w-ok" && x == "watchdog-hello")
    })
    .await
    {
        eprintln!(
            "[DBG-test] deliveries={:?} raw={:?} summary={:?}",
            delivery_lines(&srv.home),
            std::fs::read(srv.home.join("gateway").join("deliveries.jsonl")),
            task_field(srv, &ok_id, "last_summary").await
        );
        panic!("delivery never landed");
    }

    // Empty stdout is a silent tick: recorded locally, never delivered.
    assert_eq!(run_now(srv, &silent_id).await, 202);
    wait_summary(srv, &silent_id, 15, |s| s == "(silent tick)").await;
    assert!(
        !delivery_lines(&srv.home)
            .iter()
            .any(|(t, _)| t == "log:w-silent"),
        "silent tick must not deliver"
    );

    // Failure delivers an error alert EVEN without a configured target
    // (fallback log surface), carrying exit-code detail.
    assert_eq!(run_now(srv, &fail_id).await, 202);
    assert!(
        wait_until(15, || delivery_lines(&srv.home).iter().any(|(_, x)| {
            x.contains("watchdog 'fail-watch' alert") && x.contains("exit code: 3")
        }))
        .await,
        "failing watchdog must deliver a typed error alert"
    );
    wait_summary(srv, &fail_id, 10, |s| s.starts_with("script failed:")).await;

    // Zero tokens by construction AND observed: no provider dispatch ever.
    assert_eq!(fx.srv.dispatches.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_worker_delivers_typed_error_not_silence() {
    // Same fixture minus the real worker exe: the broker fails closed and
    // the failure must land as a DELIVERED alert, never silence.
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("ws");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(cwd.join(".vak/config.toml"), "").unwrap();
    let home = dir.path().join("home");
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd, true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.clone()));
    core.set_provider_instance(Arc::new(Counting {
        capacity_key: crate::support::CapacityKey::default(),
        dispatches: Arc::new(AtomicUsize::new(0)),
    }));
    // Deliberately point the worker at something unusable.
    core.set_tool_worker_exe(dir.path().join("no-worker-here"));
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    client
        .post(format!("{base}/tasks"))
        .json(&serde_json::json!({
            "name": "brokerless", "script": "echo hi",
            "interval_secs": 3600, "deliver_to": "log:bw"
        }))
        .send()
        .await
        .unwrap();
    let list: serde_json::Value = client
        .get(format!("{base}/tasks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = list["tasks"][0]["id"].as_str().unwrap().to_string();
    let res = client
        .post(format!("{base}/tasks/{id}/run-now"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "broker failure was silent; expected delivered alert"
        );
        if delivery_lines(&home)
            .iter()
            .any(|(_, x)| x.contains("watchdog 'brokerless' alert"))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

// ---- Model pin -------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn model_pin_receipts_show_pinned_model_only() {
    let fx = spawn_full("", None::<fn(&Path, &Path) -> serde_json::Value>).await;
    let srv = &fx.srv;
    let client = srv.client();

    let res = client
        .post(format!("{}/tasks", srv.base))
        .json(&serde_json::json!({
            "name": "pinned-nightly",
            "prompt": "summarize the tree",
            "interval_secs": 3600,
            "model_pin": "counting/pinned-model-x"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let list = srv.get_json("/tasks").await;
    let tid = list["tasks"][0]["id"].as_str().unwrap().to_string();

    // A never-run interval task is due immediately, so the SCHEDULER fires
    // it on its first tick; wait for the child session id it records.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    let child = loop {
        assert!(
            std::time::Instant::now() < deadline,
            "task never recorded a child session"
        );
        if let Some(c) = task_field(srv, &tid, "last_session_id")
            .await
            .as_str()
            .map(String::from)
        {
            break c;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    };
    let receipts: serde_json::Value = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "receipts never appeared for {child}"
            );
            match client
                .get(format!("{}/sessions/{child}/receipts", srv.base))
                .send()
                .await
            {
                Ok(res) if res.status() == 200 => {
                    let body: serde_json::Value = res.json().await.unwrap();
                    if body.as_array().is_some_and(|a| !a.is_empty()) {
                        break body;
                    }
                }
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    };
    for r in receipts.as_array().unwrap() {
        assert_eq!(
            r["model"], "pinned-model-x",
            "every receipt carries the pin"
        );
        assert_eq!(r["provider"], "counting");
    }

    // And the run itself completed normally.
    wait_summary(srv, &tid, 15, |s| s == "done").await;
}

// ---- Budget alerts ---------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budget_alert_fires_once_per_window_then_stops() {
    let fx = spawn_full(
        "[finops]\nmax_day_usd = 10.0\n",
        None::<fn(&Path, &Path) -> serde_json::Value>,
    )
    .await;
    let srv = &fx.srv;

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
            trace: None,
            actor: None,
        })
        .unwrap();

    let alerts_path = srv.home.join("budget-alerts.jsonl");

    let tid = create_task(
        srv,
        serde_json::json!({
            "name": "budget-probe", "script": "true",
            "schedule": "0 0 29 2 *", "deliver_to": "log:budget"
        }),
    )
    .await;

    // First fire crosses the threshold: one audit row, one delivery.
    assert_eq!(run_now(srv, &tid).await, 202);
    assert!(
        wait_until(15, || std::fs::read_to_string(&alerts_path)
            .map(|c| c.lines().count())
            .unwrap_or(0)
            >= 1)
        .await,
        "no budget alert row recorded"
    );
    let rows = std::fs::read_to_string(&alerts_path).unwrap();
    assert_eq!(rows.lines().count(), 1, "{rows}");
    assert!(rows.contains("\"level\":\"eighty\""), "{rows}");
    assert!(
        wait_until(10, || delivery_lines(&srv.home).iter().any(|(t, x)| t
            == "log:budget"
            && x.contains("budget alert [eighty]")))
        .await
    );

    // Second fire inside the same day window: no new row, no redelivery.
    assert_eq!(run_now(srv, &tid).await, 202);
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let rows = std::fs::read_to_string(&alerts_path).unwrap();
    assert_eq!(rows.lines().count(), 1, "same-level alert must not refire");
    let deliveries = delivery_lines(&srv.home)
        .iter()
        .filter(|(_, x)| x.contains("budget alert [eighty]"))
        .count();
    assert_eq!(deliveries, 1, "same-level alert must not redeliver");
}

// ---- Task CRUD validation ---------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_api_validates_schedule_script_and_pin_fields() {
    let fx = spawn_full("", None::<fn(&Path, &Path) -> serde_json::Value>).await;
    let srv = &fx.srv;
    let client = srv.client();

    // prompt XOR script.
    let res = client
        .post(format!("{}/tasks", srv.base))
        .json(&serde_json::json!({
            "name": "both", "prompt": "x", "script": "y", "interval_secs": 3600
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("exactly one"));

    let res = client
        .post(format!("{}/tasks", srv.base))
        .json(&serde_json::json!({ "name": "neither", "interval_secs": 3600 }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);

    // Bad cron grammar is rejected with a typed message.
    let res = client
        .post(format!("{}/tasks", srv.base))
        .json(&serde_json::json!({
            "name": "badcron", "script": "true",
            "interval_secs": 3600, "schedule": "99 * * * *"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("99"), "{body}");

    // A valid cron+script watchdog is accepted; prompt stays optional.
    let res = client
        .post(format!("{}/tasks", srv.base))
        .json(&serde_json::json!({
            "name": "goodcron", "script": "true",
            "interval_secs": 3600, "schedule": "*/5 * * * *",
            "agent_id": "vak", "agent_revision": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let list = srv.get_json("/tasks").await;
    let good = &list["tasks"][0];
    assert_eq!(good["schedule"], "*/5 * * * *");
    assert_eq!(good["agent_id"], "vak");
    assert_eq!(good["agent_revision"], 1);

    // PATCH to an invalid schedule is rejected and leaves state untouched.
    let res = client
        .patch(format!(
            "{}/tasks/{}",
            srv.base,
            good["id"].as_str().unwrap()
        ))
        .json(&serde_json::json!({ "schedule": "* * * *" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    let list = srv.get_json("/tasks").await;
    assert_eq!(
        list["tasks"][0]["schedule"], "*/5 * * * *",
        "invalid patch ignored"
    );

    // Clearing script then setting prompt keeps validation green.
    let res = client
        .patch(format!(
            "{}/tasks/{}",
            srv.base,
            good["id"].as_str().unwrap()
        ))
        .json(&serde_json::json!({ "script": null, "prompt": "now an agent task" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let list = srv.get_json("/tasks").await;
    assert_eq!(list["tasks"][0]["prompt"], "now an agent task");
    assert!(list["tasks"][0]["script"].is_null());

    // Agent ownership is patchable and clearing it also clears its revision.
    let res = client
        .patch(format!(
            "{}/tasks/{}",
            srv.base,
            good["id"].as_str().unwrap()
        ))
        .json(&serde_json::json!({ "agent_id": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let list = srv.get_json("/tasks").await;
    assert!(list["tasks"][0]["agent_id"].is_null());
    assert!(list["tasks"][0]["agent_revision"].is_null());

    // Unknown task id on PATCH is a typed 404.
    let res = client
        .patch(format!("{}/tasks/nope", srv.base))
        .json(&serde_json::json!({ "name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}
