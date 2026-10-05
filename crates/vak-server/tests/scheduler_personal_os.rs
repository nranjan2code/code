//! Scheduler behaviors (docs/design/29-personal-os.md P2, plan M4.3): cron
//! automations with startup catch-up, zero-token watchdog scripts over the
//! brokered bash path, model pinning verified through work receipts, and
//! once-per-window budget alerts. What a run did is read from its run
//! record, never from the automation.

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
/// auth + scheduler). `seed` receives the workspace BEFORE the server
/// starts; its automations are stored first, which is how downtime is
/// simulated: the scheduler finds them due at its first tick.
async fn spawn_full(
    config_toml: &str,
    seed: impl FnOnce(&Path) -> Vec<vak_core::triggers::Trigger>,
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
    // The data home is fixed before an automation names its space in it.
    vak_config::paths::isolate_home_for_tests();
    std::fs::create_dir_all(&home).unwrap();
    let shared = vak_config::scope::SharedScope::new(&home);
    for trigger in seed(&ws) {
        vak_core::triggers::create(&shared, &trigger).unwrap();
    }

    let dispatches = Arc::new(AtomicUsize::new(0));
    let core = Core::new_with_trust(ws.clone(), true).unwrap();
    core.set_shared_scope(shared);
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

fn no_seed(_: &Path) -> Vec<vak_core::triggers::Trigger> {
    Vec::new()
}

fn delivery_lines(home: &Path) -> Vec<(String, String)> {
    vak_session::chain::RecordChain::at(home.join("gateway").join("deliveries"))
        .read::<serde_json::Value>()
        .into_iter()
        .map(|v| {
            (
                v["target"].as_str().unwrap_or_default().to_string(),
                v["text"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
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

/// The automation's last run, as the API reads it from the run records.
async fn last_run(srv: &Srv, id: &str) -> serde_json::Value {
    srv.get_json(&format!("/triggers/{id}")).await["last_run"].clone()
}

/// Waits until the automation's last run settles and matches `pred`.
async fn wait_run(
    srv: &Srv,
    id: &str,
    secs: u64,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let run = last_run(srv, id).await;
        if run["status"] != "running" && !run.is_null() && pred(&run) {
            return run;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "automation '{id}' never reached the expected run: {run}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

/// A watchdog on cron `* * * * *`, created two hours ago and never run:
/// every slot since then was missed while the server was down.
fn stale_script(
    name: &str,
    ws: &Path,
    deliver_to: &str,
    script: &str,
) -> vak_core::triggers::Trigger {
    vak_core::triggers::Trigger {
        id: vak_session::ids::TriggerId::new(),
        name: name.into(),
        agent: "vak".into(),
        agent_revision: None,
        space: vak_config::spaces::bind(ws).unwrap(),
        enabled: true,
        kind: vak_core::triggers::TriggerKind::Schedule {
            schedule: vak_core::triggers::Schedule::Cron {
                expr: "* * * * *".into(),
                timezone: None,
            },
        },
        action: vak_core::triggers::TriggerAction::Script {
            command: script.into(),
        },
        deliver_to: Some(deliver_to.into()),
        on_crash: vak_core::triggers::OnCrash::Skip,
        scope: None,
        created_at: chrono::Utc::now() - chrono::Duration::hours(2),
        created_by: None,
    }
}

/// A `POST /triggers` body for a script on a cron that almost never fires
/// (Feb 29 at midnight), so only run-now runs it.
fn rare_script(name: &str, command: &str, deliver_to: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "name": name,
        "kind": { "kind": "schedule", "schedule": { "kind": "cron", "expr": "0 0 29 2 *" } },
        "action": { "kind": "script", "command": command },
    });
    if let Some(target) = deliver_to {
        body["deliver_to"] = target.into();
    }
    body
}

// ---- Catch-up -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_fires_missed_cron_slot_exactly_once_with_zero_dispatch() {
    let mut id = String::new();
    let fx = spawn_full("", |ws| {
        let trigger = stale_script("nightly-watch", ws, "log:cu", "echo caught-up-42");
        id = trigger.id.to_string();
        vec![trigger]
    })
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

    // The catch-up is a run record, opened just now.
    let run = wait_run(&fx.srv, &id, 5, |run| run["status"] == "completed").await;
    let opened = chrono::DateTime::parse_from_rfc3339(run["opened_at"].as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(chrono::Utc::now() - opened < chrono::Duration::seconds(30));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_disabled_waits_for_next_slot() {
    let fx = spawn_full("[automation]\ncatch_up_missed = false\n", |ws| {
        vec![stale_script(
            "quiet-watch",
            ws,
            "log:cuoff",
            "echo not-caught-up-99",
        )]
    })
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

async fn create_trigger(srv: &Srv, body: serde_json::Value) -> String {
    let res = srv
        .client()
        .post(format!("{}/triggers", srv.base))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201, "create failed: {}", res.status());
    let created: serde_json::Value = res.json().await.unwrap();
    created["id"].as_str().unwrap().to_string()
}

async fn run_now(srv: &Srv, id: &str) -> reqwest::StatusCode {
    srv.client()
        .post(format!("{}/triggers/{id}/run", srv.base))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn script_watchdog_matrix_silent_stdout_failure_and_delivery() {
    let fx = spawn_full("", no_seed).await;
    let srv = &fx.srv;

    let ok_id = create_trigger(
        srv,
        rare_script("ok-watch", "echo watchdog-hello", Some("log:w-ok")),
    )
    .await;
    let silent_id = create_trigger(
        srv,
        rare_script("silent-watch", "true", Some("log:w-silent")),
    )
    .await;
    let fail_id = create_trigger(
        srv,
        rare_script("fail-watch", "echo broken >&2; exit 3", None),
    )
    .await;

    // Success delivers trimmed stdout verbatim.
    assert_eq!(run_now(srv, &ok_id).await, 202);
    wait_run(srv, &ok_id, 15, |run| run["status"] == "completed").await;
    assert!(
        wait_until(10, || delivery_lines(&srv.home)
            .iter()
            .any(|(t, x)| t == "log:w-ok" && x == "watchdog-hello"))
        .await,
        "delivery never landed: {:?}",
        delivery_lines(&srv.home)
    );

    // Empty stdout is a silent tick: a completed run, never delivered.
    assert_eq!(run_now(srv, &silent_id).await, 202);
    wait_run(srv, &silent_id, 15, |run| run["status"] == "completed").await;
    assert!(
        !delivery_lines(&srv.home)
            .iter()
            .any(|(t, _)| t == "log:w-silent"),
        "silent tick must not deliver"
    );

    // Failure delivers an error alert EVEN without a configured target
    // (fallback log surface), carrying exit-code detail, and its run says
    // it failed.
    assert_eq!(run_now(srv, &fail_id).await, 202);
    assert!(
        wait_until(15, || delivery_lines(&srv.home).iter().any(|(_, x)| {
            x.contains("watchdog 'fail-watch' alert") && x.contains("exit code: 3")
        }))
        .await,
        "failing watchdog must deliver a typed error alert"
    );
    let failed = wait_run(srv, &fail_id, 10, |run| run["status"] == "failed").await;
    assert!(
        failed["reason"]
            .as_str()
            .unwrap()
            .starts_with("script failed:"),
        "{failed}"
    );

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

    let created: serde_json::Value = client
        .post(format!("{base}/triggers"))
        .json(&{
            let mut body = support::script_trigger("brokerless", "echo hi", 3600);
            body["deliver_to"] = "log:bw".into();
            body
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let res = client
        .post(format!("{base}/triggers/{id}/run"))
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
    let fx = spawn_full("", no_seed).await;
    let srv = &fx.srv;
    let client = srv.client();

    let tid = create_trigger(
        srv,
        serde_json::json!({
            "name": "pinned-nightly",
            "kind": { "kind": "manual" },
            "action": {
                "kind": "prompt",
                "text": "summarize the tree",
                "model_pin": "counting/pinned-model-x",
            },
        }),
    )
    .await;
    assert_eq!(run_now(srv, &tid).await, 202);

    let run = wait_run(srv, &tid, 25, |run| run["status"] == "completed").await;
    let child = run["sessions"][0].as_str().unwrap().to_string();
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
}

// ---- Budget alerts ---------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budget_alert_fires_once_per_window_then_stops() {
    let fx = spawn_full("[finops]\nmax_day_usd = 10.0\n", no_seed).await;
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

    let alerts_path = srv.home.join("budget-alerts");
    let tid = create_trigger(srv, rare_script("budget-probe", "true", Some("log:budget"))).await;

    // First fire crosses the threshold: one audit row, one delivery.
    assert_eq!(run_now(srv, &tid).await, 202);
    assert!(
        wait_until(15, || vak_session::chain::RecordChain::at(&alerts_path)
            .text()
            .lines()
            .count()
            >= 1)
        .await,
        "no budget alert row recorded"
    );
    let rows = vak_session::chain::RecordChain::at(&alerts_path).text();
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
    let rows = vak_session::chain::RecordChain::at(&alerts_path).text();
    assert_eq!(rows.lines().count(), 1, "same-level alert must not refire");
    let deliveries = delivery_lines(&srv.home)
        .iter()
        .filter(|(_, x)| x.contains("budget alert [eighty]"))
        .count();
    assert_eq!(deliveries, 1, "same-level alert must not redeliver");
}

// ---- Automation API validation ----------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn automation_api_validates_schedule_action_and_ownership() {
    let fx = spawn_full("", no_seed).await;
    let srv = &fx.srv;
    let client = srv.client();
    let post = |body: serde_json::Value| {
        let client = client.clone();
        let base = srv.base.clone();
        async move {
            let res = client
                .post(format!("{base}/triggers"))
                .json(&body)
                .send()
                .await
                .unwrap();
            let status = res.status();
            (
                status,
                res.json::<serde_json::Value>().await.unwrap_or_default(),
            )
        }
    };

    // An action must say what to do.
    let (status, body) = post(rare_script("empty", "  ", None)).await;
    assert_eq!(status, 400, "{body}");
    let (status, _) = post(serde_json::json!({
        "name": "neither",
        "kind": { "kind": "manual" },
    }))
    .await;
    assert!(status.is_client_error());

    // Bad cron grammar is rejected with a typed message.
    let (status, body) = post(serde_json::json!({
        "name": "badcron",
        "kind": { "kind": "schedule", "schedule": { "kind": "cron", "expr": "99 * * * *" } },
        "action": { "kind": "script", "command": "true" },
    }))
    .await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("99"), "{body}");

    // A valid cron watchdog is accepted, owned by the built-in Agent.
    let (status, good) = post(serde_json::json!({
        "name": "goodcron",
        "kind": { "kind": "schedule", "schedule": { "kind": "cron", "expr": "*/5 * * * *" } },
        "action": { "kind": "script", "command": "true" },
    }))
    .await;
    assert_eq!(status, 201, "{good}");
    assert_eq!(good["agent"], "vak");
    let id = good["id"].as_str().unwrap().to_string();
    let put = |body: serde_json::Value| {
        let client = client.clone();
        let url = format!("{}/triggers/{id}", srv.base);
        async move { client.put(url).json(&body).send().await.unwrap().status() }
    };

    // Replacing it with an invalid schedule is refused and changes nothing.
    let mut invalid = good.clone();
    invalid["kind"]["schedule"]["expr"] = "* * * *".into();
    assert_eq!(put(invalid).await, 400);
    let stored = srv.get_json(&format!("/triggers/{id}")).await;
    assert_eq!(stored["kind"]["schedule"]["expr"], "*/5 * * * *");

    // A script becomes a prompt in one replacement.
    let mut prompt = stored.clone();
    prompt["action"] = serde_json::json!({ "kind": "prompt", "text": "now an agent turn" });
    assert_eq!(put(prompt).await, 200);
    let stored = srv.get_json(&format!("/triggers/{id}")).await;
    assert_eq!(stored["action"]["text"], "now an agent turn");
    assert!(stored["last_run"].is_null(), "no run state lives on it");

    // Its next slots are listed in order.
    let slots = srv.get_json(&format!("/triggers/{id}/slots?count=3")).await;
    assert_eq!(slots["slots"].as_array().unwrap().len(), 3);

    // An unknown automation is a typed 404.
    let res = client
        .put(format!(
            "{}/triggers/trg_00000000-0000-7000-8000-000000000000",
            srv.base
        ))
        .json(&stored)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}
