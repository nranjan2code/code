//! Run records (plan M4.2): every cause opens a run before its work and
//! settles it, a run that cannot be opened is not admitted, and a run whose
//! holder stopped is recorded abandoned.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;
use vak_core::Core;
use vak_core::admission::RunAdmission;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::runs::{RunStatus, Runs};
use vak_session::trace::Cause;

/// Answers "ok"; when `delegate` is set, its first answer instead asks for
/// one worker through the `task` tool.
struct Scripted {
    capacity_key: crate::support::CapacityKey,
    delegate: bool,
    calls: AtomicUsize,
}

impl Scripted {
    fn new(delegate: bool) -> Arc<Self> {
        Arc::new(Self {
            capacity_key: crate::support::CapacityKey::default(),
            delegate,
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let (content, stop_reason) = if self.delegate && first {
            (
                vec![ContentBlock::ToolUse {
                    id: "call-1".into(),
                    name: "task".into(),
                    input: serde_json::json!({"prompt": "say ok", "label": "helper"}),
                }],
                StopReason::ToolUse,
            )
        } else {
            (vec![ContentBlock::text("ok")], StopReason::EndTurn)
        };
        let m = AssistantMessage {
            content,
            stop_reason,
            usage: Usage::default(),
            model: "test-model".into(),
            response_id: None,
        };
        let (mut sink, rx) = stream::channel(64);
        sink.push(stream::StreamEvent::Start { partial: m.clone() });
        sink.close_message(m).await;
        Ok(rx)
    }
}

fn workspace(dir: &std::path::Path) -> std::path::PathBuf {
    let cwd = dir.join("work");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    cwd
}

fn core_at(cwd: &std::path::Path, home: &std::path::Path, delegate: bool) -> Core {
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.to_path_buf(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.to_path_buf()));
    core.set_provider_instance(Scripted::new(delegate));
    core
}

async fn one_turn(core: &Core) -> String {
    let session = core.start_session().await.unwrap();
    let sid = session.header().unwrap().session_id.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    // Allows the delegating turn's worker, which asks.
    let approver: Arc<dyn vak_agent::Approver> = Arc::new(vak_agent::AutoApprove);
    core.run_turn_with(
        session,
        "go",
        CancellationToken::new(),
        Some(approver),
        None,
        None,
        tx,
    )
    .await
    .unwrap();
    sid
}

fn cause_name(cause: &Cause) -> &'static str {
    match cause {
        Cause::User { .. } => "user",
        Cause::Channel { .. } => "channel",
        Cause::Schedule { .. } => "schedule",
        Cause::Delegation { .. } => "delegation",
        Cause::Revision { .. } => "revision",
        Cause::Trigger { .. } => "trigger",
        Cause::Heartbeat => "heartbeat",
        Cause::System { .. } => "system",
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_cause_writes_run() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = workspace(dir.path());
    let home = dir.path().join("home");

    // A person's turn, and a turn that delegates to a worker.
    let user = core_at(&cwd, &home, false);
    let user_session = one_turn(&user).await;
    let delegating = core_at(&cwd, &home, true);
    one_turn(&delegating).await;

    // Causes stamped by the surface that starts the work.
    for cause in [
        Cause::Heartbeat,
        Cause::Schedule {
            schedule: "task-1".into(),
            slot: chrono::Utc::now().to_rfc3339(),
        },
        Cause::Revision {
            candidate: "cand-1".into(),
        },
        Cause::Trigger {
            trigger: vak_session::ids::TriggerId::derived("task-1"),
            request_id: "manual-1".into(),
        },
    ] {
        let core =
            core_at(&cwd, &home, false).with_run_admission(RunAdmission::default().cause(cause));
        one_turn(&core).await;
    }

    // A channel request is admitted, and its run opened, before the turn,
    // as the gateway does.
    let channel = core_at(&cwd, &home, false);
    let admission = RunAdmission::default().cause(Cause::Channel {
        endpoint: "telegram:42".into(),
        request_id: "req-1".into(),
    });
    let channel = channel.with_run_admission(admission.clone());
    let admitted = channel.mint_trace(None);
    let channel = channel.with_run_admission(admission.trace(admitted.clone()));
    let mut open_run = channel.runs().begin(&admitted, None, None).unwrap();
    let session = channel.start_session().await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let (outcome, _) = channel
        .run_turn_with(
            session,
            "hi",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    open_run.end_with(vak_session::runs::RunEnd::Settled(outcome.run_outcome()));
    drop(open_run);

    let runs = user.runs().list().unwrap();
    let mut seen: Vec<&str> = runs
        .iter()
        .map(|run| cause_name(&run.trace.as_ref().expect("an opened run has its key").cause))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen,
        [
            "channel",
            "delegation",
            "heartbeat",
            "revision",
            "schedule",
            "trigger",
            "user"
        ],
        "{runs:#?}"
    );
    for run in &runs {
        assert_eq!(run.status, RunStatus::Completed, "{run:#?}");
        assert!(!run.sessions.is_empty(), "a run names its ledger: {run:#?}");
        assert_eq!(run.holder, Some(vak_session::fence::process()));
        assert!(run.settled_at.unwrap() >= run.opened_at);
    }
    let user_run = runs
        .iter()
        .find(|run| run.sessions.contains(&user_session))
        .unwrap();
    assert!(
        user_run.result_id.is_some(),
        "a completed turn names its answer"
    );
    // The delegated run is the child of a run of its own parent.
    let child = runs
        .iter()
        .find(|run| matches!(run.trace.as_ref().unwrap().cause, Cause::Delegation { .. }))
        .unwrap();
    let Cause::Delegation { parent_run, .. } = &child.trace.as_ref().unwrap().cause else {
        unreachable!()
    };
    assert!(runs.iter().any(|run| run.id == *parent_run));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_open_failure_refuses_admission() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = workspace(dir.path());
    let home = dir.path().join("home");
    let core = core_at(&cwd, &home, false);
    // The run records cannot be written: a file stands where they go.
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(core.shared_scope().runs(), b"not a directory").unwrap();

    let session = core.start_session().await.unwrap();
    let before = session.len();
    let sid = session.header().unwrap().session_id.clone();
    let (tx, _rx) = tokio::sync::mpsc::channel(256);
    let refused = core
        .run_turn_with(
            session,
            "go",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await;
    assert!(
        refused.is_err(),
        "a run that cannot be opened is not admitted"
    );
    // Nothing of the turn reached its ledger.
    let reopened = core.open_session_read_only(&sid).await.unwrap();
    assert_eq!(reopened.len(), before);
}

#[test]
fn abandoned_run_is_recorded() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let tenant_home = vak_config::paths::local_tenant_home();
    let runs = Runs::at(dir.path().join("runs"), &tenant_home);

    let key = |agent: &str| {
        vak_core::Core::new_with_trust(dir.path().to_path_buf(), true)
            .unwrap()
            .with_run_admission(RunAdmission::default().cause(Cause::System {
                job: agent.to_string(),
            }))
            .mint_trace(None)
    };
    // A run held by a process that is gone: it never renewed its liveness.
    let gone = vak_session::ids::ProcessId::new();
    let orphan = key("orphan");
    vak_session::chain::RecordChain::at(runs.path())
        .append(&vak_session::runs::RunEvent {
            run: orphan.run,
            at: chrono::Utc::now(),
            trace: Some(orphan.clone()),
            actor: orphan.actor,
            step: vak_session::runs::RunStep::Opened {
                work: None,
                trigger: None,
                slot: None,
                attempt: 1,
                holder: gone,
            },
        })
        .unwrap();
    // A run this live process holds.
    let mine = key("mine");
    runs.open(&mine, None, None, 1).unwrap();

    let abandoned = runs.sweep_abandoned(chrono::Utc::now()).unwrap();
    assert_eq!(abandoned, vec![orphan.run]);
    let record = runs.get(orphan.run).unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Abandoned);
    assert_eq!(record.holder, Some(gone));
    assert_eq!(record.noticed_by, Some(vak_session::fence::process()));
    assert!(runs.get(mine.run).unwrap().unwrap().is_open());
    // Swept once: a second pass finds nothing more.
    assert!(runs.sweep_abandoned(chrono::Utc::now()).unwrap().is_empty());
}

/// Runs opened at once from many threads all open: each renews this
/// process's liveness, and a lost race on that ref is retried, never a
/// refused run.
#[test]
fn runs_opened_together_all_open() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let runs = Runs::at(
        dir.path().join("runs"),
        vak_config::paths::local_tenant_home(),
    );
    let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    let keys: Vec<_> = (0..16)
        .map(|n| {
            core.clone()
                .with_run_admission(RunAdmission::default().cause(Cause::System {
                    job: format!("job-{n}"),
                }))
                .mint_trace(None)
        })
        .collect();
    std::thread::scope(|scope| {
        for key in &keys {
            let runs = runs.clone();
            scope.spawn(move || runs.open(key, None, None, 1).unwrap());
        }
    });
    assert_eq!(runs.list().unwrap().len(), keys.len());
}

/// Exit test (plan M4.3): an automation's last run is a query over the run
/// records that name it; running it never writes to the automation.
#[test]
fn last_run_is_a_query() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let shared = vak_config::scope::SharedScope::new(dir.path());
    let runs = Runs::at(shared.runs(), vak_config::paths::local_tenant_home());
    let trigger = vak_core::triggers::Trigger {
        id: vak_session::ids::TriggerId::new(),
        name: "nightly".into(),
        agent: "vak".into(),
        agent_revision: None,
        space: "spc_test".into(),
        enabled: true,
        kind: vak_core::triggers::TriggerKind::Manual,
        action: vak_core::triggers::TriggerAction::Script {
            command: "true".into(),
        },
        deliver_to: None,
        on_crash: vak_core::triggers::OnCrash::Skip,
        scope: None,
        created_at: chrono::Utc::now(),
        created_by: None,
    };
    vak_core::triggers::create(&shared, &trigger).unwrap();
    assert!(
        vak_core::triggers::last_run(&runs, &trigger.id)
            .unwrap()
            .is_none()
    );

    let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    let key = || {
        core.clone()
            .with_run_admission(RunAdmission::default().cause(Cause::Trigger {
                trigger: trigger.id,
                request_id: uuid::Uuid::now_v7().to_string(),
            }))
            .mint_trace(None)
    };
    let first = key();
    runs.open(&first, Some(trigger.id), None, 1).unwrap();
    runs.settle(first.run, vak_session::runs::RunOutcome::Completed, None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = key();
    runs.open(&second, Some(trigger.id), None, 1).unwrap();
    // Another trigger's run is not this one's.
    let other = key();
    runs.open(&other, Some(vak_session::ids::TriggerId::new()), None, 1)
        .unwrap();

    let last = vak_core::triggers::last_run(&runs, &trigger.id)
        .unwrap()
        .unwrap();
    assert_eq!(last.id, second.run);
    assert!(last.is_open());
    assert_eq!(runs.of_trigger(&trigger.id).unwrap().len(), 2);
    // The automation itself never changed: one version, as created.
    assert_eq!(
        vak_session::documents::version_count(&shared.triggers().join(trigger.id.to_string())),
        1
    );
    assert_eq!(
        vak_core::triggers::get(&shared, &trigger.id.to_string()).unwrap(),
        Some(trigger)
    );
}
