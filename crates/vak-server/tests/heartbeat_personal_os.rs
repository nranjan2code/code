//! Heartbeat behaviors (docs/design/29-personal-os.md P7): a "nothing"
//! reply records nothing anywhere, findings park exactly one inbox entry
//! with zero deliveries, an URGENT marker adds the delivery leg through the
//! gateway chokepoint, a breached day-cap denies the dispatch entirely, and
//! a disabled flag never schedules.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use chrono::Timelike;
use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};

/// Provider whose final text is settable per-phase and which counts every
/// dispatch, mirroring the counting mock the scheduler tests use.
struct Scripted {
    dispatches: Arc<AtomicUsize>,
    reply: Arc<Mutex<String>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        let text = self
            .reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let (mut sink, rx) = stream::channel(8);
        let done = AssistantMessage {
            content: vec![ContentBlock::text(text)],
            stop_reason: vak_llm::types::StopReason::EndTurn,
            usage: Usage {
                input_tokens: 9,
                output_tokens: 1,
                ..Default::default()
            },
            model: "scripted-model".into(),
        };
        sink.push(stream::StreamEvent::Start {
            partial: done.clone(),
        });
        sink.close_message(done).await;
        Ok(rx)
    }
}

struct Fixture {
    home: PathBuf,
    ws: PathBuf,
    dispatches: Arc<AtomicUsize>,
    _dir: Arc<tempfile::TempDir>,
}

/// Hermetic workspace + home with the FULL stack started (bearer auth +
/// background scheduler, heartbeat loop included when enabled). `seed`
/// runs against the home dir before any Core exists — the injection point
/// for cost-ledger rows that must predate the first beat.
async fn spawn_full_seeded(
    config_toml: &str,
    reply_text: &str,
    seed: Option<&dyn Fn(&Path)>,
) -> Fixture {
    let dir = Arc::new(tempfile::tempdir().unwrap());
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join(".vakcoder")).unwrap();
    std::fs::write(
        ws.join(".vakcoder/config.toml"),
        format!("[memory]\nreflection = false\n{config_toml}"),
    )
    .unwrap();

    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    if let Some(seed) = seed {
        seed(&home);
    }
    let dispatches = Arc::new(AtomicUsize::new(0));
    let reply = Arc::new(Mutex::new(reply_text.to_string()));
    let core = Core::new_with_trust(ws.clone(), true).unwrap();
    core.set_sessions_home(home.clone());
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));
    core.set_provider_instance(Arc::new(Scripted {
        dispatches: dispatches.clone(),
        reply: reply.clone(),
    }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, _token) = vak_server::secured_router_with(core, true);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let _ = addr;

    Fixture {
        home,
        ws,
        dispatches,
        _dir: dir,
    }
}

async fn spawn_full(config_toml: &str, reply_text: &str) -> Fixture {
    spawn_full_seeded(config_toml, reply_text, None).await
}

fn heartbeat_entries(home: &Path) -> Vec<vak_core::inbox::Entry> {
    vak_core::inbox::list_scanned(home, 100)
        .entries
        .into_iter()
        .filter(|e| e.kind == vak_core::inbox::Kind::Heartbeat)
        .collect()
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

fn heartbeat_ledger(fx: &Fixture) -> PathBuf {
    vak_session::SessionPath::new_session_file(&fx.home, &fx.ws, "heartbeat")
}

async fn wait_for_dispatch(fx: &Fixture, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if fx.dispatches.load(Ordering::SeqCst) > 0 {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    fx.dispatches.load(Ordering::SeqCst) > 0
}

async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
}

// ---- Nothing-to-report costs tokens, zero noise ---------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_reply_records_nothing_anywhere() {
    let fx = spawn_full("[heartbeat]\nenabled = true\n", "nothing").await;

    assert!(
        wait_for_dispatch(&fx, 15).await,
        "enabled heartbeat must dispatch its first beat"
    );
    settle().await;

    assert!(
        heartbeat_entries(&fx.home).is_empty(),
        "a nothing-reply must never enter the inbox"
    );
    assert!(
        delivery_lines(&fx.home).is_empty(),
        "a nothing-reply must never be delivered"
    );
    assert!(
        heartbeat_ledger(&fx).is_file(),
        "the dedicated persistent session must exist on disk"
    );
}

// ---- Findings park once, push never ----------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn findings_record_exactly_one_entry_with_zero_deliveries() {
    let fx = spawn_full(
        "[heartbeat]\nenabled = true\n",
        "check the flaky cron\nclose stale PR #4",
    )
    .await;

    assert!(wait_for_dispatch(&fx, 15).await);
    settle().await;

    let entries = heartbeat_entries(&fx.home);
    assert_eq!(entries.len(), 1, "findings park exactly one entry");
    assert_eq!(entries[0].title, "heartbeat: 2 findings");
    assert_eq!(entries[0].body, "check the flaky cron\nclose stale PR #4");
    assert_eq!(entries[0].session_id.as_deref(), Some("heartbeat"));
    assert!(
        delivery_lines(&fx.home).is_empty(),
        "non-urgent findings must never ride a transport"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn urgent_finding_adds_exactly_one_delivery_and_strips_markers() {
    let fx = spawn_full(
        "[heartbeat]\nenabled = true\n",
        "URGENT: disk almost full\nminor lint debt",
    )
    .await;

    // No tasks configured, so the urgent beat falls back to log:vakcoder.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "urgent finding was never delivered"
        );
        if delivery_lines(&fx.home)
            .iter()
            .any(|(_, x)| x.contains("disk almost full"))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    settle().await;

    let deliveries = delivery_lines(&fx.home)
        .iter()
        .filter(|(_, x)| x.contains("disk almost full"))
        .count();
    assert_eq!(deliveries, 1, "urgent beat delivers once");

    let entries = heartbeat_entries(&fx.home);
    assert!(!entries.is_empty(), "findings still park an inbox entry");
    assert!(
        entries.iter().all(|e| !e.body.contains("URGENT:")),
        "markers are stripped from recorded bodies: {:?}",
        entries.iter().map(|e| &e.body).collect::<Vec<_>>()
    );
    assert!(
        entries.iter().any(|e| e.body.contains("disk almost full")),
        "the finding text survives the strip"
    );
}

// ---- Budget gate ------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn breached_day_cap_denies_the_dispatch_silently() {
    // Seed the cost ledger BEFORE the server exists: the first beat fires
    // immediately, so the cap must already be breached by then.
    let seed = |home: &Path| {
        vak_core::finops::FinOpsLedger::new(home)
            .append(&vak_core::finops::CostRow {
                ts: chrono::Utc::now(),
                model: "claude-sonnet".into(),
                provider: "anthropic".into(),
                input_tokens: 1000,
                output_tokens: 500,
                cache_read_input_tokens: None,
                usd: Some(5.0),
                source: "estimated".into(),
                session_id: "seed".into(),
            })
            .unwrap();
    };
    let fx = spawn_full_seeded(
        "[heartbeat]\nenabled = true\n\n[finops]\nmax_day_usd = 1.0\n",
        "would-be findings",
        Some(&seed),
    )
    .await;

    settle().await;
    settle().await;
    assert_eq!(
        fx.dispatches.load(Ordering::SeqCst),
        0,
        "budget-denied cycle must dispatch zero"
    );
    assert!(
        !heartbeat_ledger(&fx).exists(),
        "denied cycle must not even open the heartbeat session"
    );
}

// ---- Quiet hours --------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quiet_hours_window_skips_the_cycle() {
    // Window covering the current local minute plus the next two, so the
    // test's whole lifetime sits inside quiet hours regardless of where
    // within the minute the process starts.
    let now = chrono::Local::now();
    let m = now.hour() * 60 + now.minute();
    let end = (m + 3) % (24 * 60);
    let quiet = format!(
        "[heartbeat]\nenabled = true\nquiet_hours = \"{:02}:{:02}-{:02}:{:02}\"\n",
        m / 60,
        m % 60,
        end / 60,
        end % 60
    );
    let fx = spawn_full(&quiet, "would-be findings").await;

    settle().await;
    settle().await;
    assert_eq!(
        fx.dispatches.load(Ordering::SeqCst),
        0,
        "quiet-hours cycle must not dispatch"
    );
    assert!(!heartbeat_ledger(&fx).exists());
}

// ---- Disabled ------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_flag_never_schedules() {
    let fx = spawn_full("", "findings that must never happen").await;

    settle().await;
    settle().await;
    assert_eq!(fx.dispatches.load(Ordering::SeqCst), 0);
    assert!(!heartbeat_ledger(&fx).exists());
    assert!(heartbeat_entries(&fx.home).is_empty());
}
