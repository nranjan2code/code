#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::AutoApprove;
use vak_flow::{
    ExecutorDeps, PLANNER_SYSTEM, PlanOutcome, ToolCatalogEntry, build_planner_prompt,
    extract_toml, plan_and_run, sanitize_basic_string_newlines,
};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_tools::bash::BashTool;

#[test]
fn planner_prompt_includes_task_and_catalog() {
    let catalog = vec![
        ToolCatalogEntry {
            name: "bash".into(),
            description: "Execute a shell command".into(),
        },
        ToolCatalogEntry {
            name: "task".into(),
            description: "Delegate to a subagent".into(),
        },
    ];
    let prompt = build_planner_prompt("migrate the config module", &catalog);
    assert!(prompt.contains("migrate the config module"));
    assert!(prompt.contains("- bash: Execute a shell command"));
    assert!(prompt.contains("- task: Delegate to a subagent"));
}

#[test]
fn extract_toml_handles_fenced_raw_and_garbage() {
    let fenced = "Here is the plan:\n```toml\n[flow]\nname = \"x\"\n[[nodes]]\nid=\"a\"\ntype=\"bash\"\ncommand=\"echo hi\"\n```\ndone";
    let extracted = extract_toml(fenced).unwrap();
    assert!(extracted.contains("[flow]"));
    assert!(!extracted.contains("```"));

    let raw = "[flow]\nname = \"y\"\n\n[[nodes]]\nid = \"a\"\ntype = \"bash\"\ncommand = \"echo\"";
    assert_eq!(extract_toml(raw).as_deref(), Some(raw));

    assert!(extract_toml("I cannot plan this task, sorry.").is_none());
}

struct ScriptedPlanner {
    /// Responses consumed in order across ALL requests (planner + children).
    responses: Mutex<VecDeque<ScriptedResponse>>,
}

enum ScriptedResponse {
    Text(String),
    Error(LlmError),
}

#[async_trait::async_trait]
impl Provider for ScriptedPlanner {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(ScriptedResponse::Text(t)) => {
                let msg = AssistantMessage {
                    content: vec![ContentBlock::text(t.clone())],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    model: "test-model".into(),
                };
                sink.push(stream::StreamEvent::Start {
                    partial: msg.clone(),
                });
                sink.close_message(msg).await;
            }
            Some(ScriptedResponse::Error(e)) => sink.close_error(e).await,
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

const GOOD_PLAN: &str = "\
[flow]
name = \"planned\"

[[nodes]]
id = \"probe\"
type = \"bash\"
command = \"echo planned-ok\"

[[nodes]]
id = \"report\"
type = \"merge\"
deps = [\"probe\"]
";

fn fenced(plan: &str) -> String {
    format!("Sure, here is the plan:\n```toml\n{plan}\n```\n")
}

fn make_deps(provider: Arc<ScriptedPlanner>) -> Arc<ExecutorDeps> {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::mem::forget(dir);
    Arc::new(ExecutorDeps {
        provider,
        system_prompt: "sys".into(),
        model: "test-model".into(),
        tools: vec![Arc::new(BashTool)],
        read_only_tools: vec![],
        max_turns: 4,
        outcome: None,
        max_retries: 0,
        retry_base_backoff_ms: 100,
        request_timeout: Some(std::time::Duration::from_secs(600)),
        circuit_breaker: None,
        run_retry_attempts: 0,
        run_retry_base_backoff_ms: 1000,
        dispatch_ceiling: 1,
        spend_gate: None,
        permission: Some(Arc::new(PermissionEngine::default())),
        mode: Mode::FullAccess,
        approval_mode: vak_agent::ApprovalMode::Ask,
        approver: Some(Arc::new(AutoApprove)),
        sandbox: None,
        cwd: std::env::temp_dir(),
        sessions_home: home.clone(),
        parent_session_id: "plan-parent".into(),
        state_path: home.join("flow-runs/plan"),
        work: None,
    })
}

async fn drain(f: impl std::future::Future<Output = PlanOutcome>) -> PlanOutcome {
    let (tx, mut rx) = mpsc::channel::<String>(256);
    let d = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    drop(tx);
    let out = f.await;
    d.await.unwrap();
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_plan_executes_to_completion() {
    let provider = Arc::new(ScriptedPlanner {
        responses: Mutex::new(VecDeque::from(vec![ScriptedResponse::Text(fenced(
            GOOD_PLAN,
        ))])),
    });
    let deps = make_deps(provider);

    let outcome = drain(plan_and_run(
        deps,
        "run the planned probe",
        CancellationToken::new(),
        mpsc::channel(64).0,
    ))
    .await;

    match outcome {
        PlanOutcome::Completed { outputs, attempts } => {
            assert_eq!(attempts, 1);
            assert!(
                outputs
                    .get("probe")
                    .is_some_and(|o| o.contains("planned-ok"))
            );
        }
        other => panic!("expected completed, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn garbage_plan_fails_closed_without_execution() {
    let provider = Arc::new(ScriptedPlanner {
        responses: Mutex::new(VecDeque::from(vec![ScriptedResponse::Text(
            "I'm sorry, I cannot produce a TOML plan for that.".into(),
        )])),
    });
    let deps = make_deps(provider);

    let outcome = drain(plan_and_run(
        deps,
        "impossible task",
        CancellationToken::new(),
        mpsc::channel(64).0,
    ))
    .await;

    match outcome {
        PlanOutcome::PlanningFailed { reason } => {
            assert!(reason.contains("no TOML plan"));
        }
        other => panic!("expected planning_failed, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structurally_invalid_plan_fails_closed() {
    // A cycle must be rejected by validation, never executed.
    let cyclic = "\
[flow]
name = \"cyclic\"

[[nodes]]
id = \"a\"
type = \"bash\"
command = \"echo a\"
deps = [\"b\"]

[[nodes]]
id = \"b\"
type = \"bash\"
command = \"echo b\"
deps = [\"a\"]
";
    let provider = Arc::new(ScriptedPlanner {
        responses: Mutex::new(VecDeque::from([ScriptedResponse::Text(fenced(cyclic))])),
    });
    let deps = make_deps(provider);

    let outcome = drain(plan_and_run(
        deps,
        "circular task",
        CancellationToken::new(),
        mpsc::channel(64).0,
    ))
    .await;

    match outcome {
        PlanOutcome::PlanningFailed { reason } => {
            assert!(reason.contains("cycle"), "got: {reason}");
        }
        other => panic!("expected planning_failed, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_execution_triggers_exactly_one_replan() {
    // Attempt 1: plan with a required node that fails at runtime.
    // Attempt 2 (replan): clean plan that succeeds.
    let failing_plan = "\
[flow]
name = \"attempt-one\"

[[nodes]]
id = \"boom\"
type = \"bash\"
command = \"exit 5\"
required = true

[[nodes]]
id = \"report\"
type = \"merge\"
deps = [\"boom\"]
";
    let provider = Arc::new(ScriptedPlanner {
        responses: Mutex::new(VecDeque::from(vec![
            ScriptedResponse::Text(fenced(failing_plan)),
            ScriptedResponse::Text(fenced(GOOD_PLAN)),
        ])),
    });
    let deps = make_deps(provider);

    let outcome = drain(plan_and_run(
        deps,
        "flaky task",
        CancellationToken::new(),
        mpsc::channel(256).0,
    ))
    .await;

    match outcome {
        PlanOutcome::Completed { attempts, .. } => {
            assert_eq!(attempts, 2, "exactly one replan allowed");
        }
        other => panic!("expected completed after replan, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replan_budget_is_bounded_at_one_retry() {
    let bad_plan = "\
[flow]
name = \"always-fails\"

[[nodes]]
id = \"boom\"
type = \"bash\"
command = \"exit 7\"
required = true
";
    let provider = Arc::new(ScriptedPlanner {
        responses: Mutex::new(VecDeque::from(vec![
            ScriptedResponse::Text(fenced(bad_plan)),
            ScriptedResponse::Text(fenced(bad_plan)),
            // A third plan would be accepted here if the budget were unbounded.
            ScriptedResponse::Text(fenced(GOOD_PLAN)),
        ])),
    });
    let deps = make_deps(provider);

    let outcome = drain(plan_and_run(
        deps,
        "doomed task",
        CancellationToken::new(),
        mpsc::channel(256).0,
    ))
    .await;

    match outcome {
        PlanOutcome::Failed { node, .. } => {
            assert_eq!(node, "boom");
        }
        other => panic!("expected failed after budget exhausted, got {other:?}"),
    }
}

#[test]
fn planner_system_prompt_is_compact() {
    // Prompt discipline: the planner system prompt stays small.
    let tokens_estimate = PLANNER_SYSTEM.len() / 4;
    assert!(
        tokens_estimate < 500,
        "planner system prompt too large (~{tokens_estimate} tokens)"
    );
}

#[test]
fn sanitizer_escapes_newlines_in_basic_strings_only() {
    let doc = "[flow]\nname = \"x\"\n\n[[nodes]]\nid = \"a\"\ntype = \"bash\"\ncommand = \"echo one\necho two\"\n";
    let fixed = sanitize_basic_string_newlines(doc);
    assert_eq!(
        fixed,
        "[flow]\nname = \"x\"\n\n[[nodes]]\nid = \"a\"\ntype = \"bash\"\ncommand = \"echo one\\necho two\"\n"
    );
    assert!(vak_flow::parse_flow(&fixed).is_ok());

    // Valid documents pass through byte-for-byte.
    assert_eq!(sanitize_basic_string_newlines(GOOD_PLAN), GOOD_PLAN);

    // Multi-line basic strings are untouched (already valid TOML).
    let multi = "prompt = \"\"\"a\nb\"\"\"\n";
    assert_eq!(sanitize_basic_string_newlines(multi), multi);

    // Literal strings and comments are untouched.
    let lit = "command = 'echo hi'\n# comment with \" quote\nname = \"z\"\n";
    assert_eq!(sanitize_basic_string_newlines(lit), lit);

    // Escaped quotes/backslashes inside basic strings survive.
    let escaped = "prompt = \"said \\\"hi\\\" then \\\\ broke\ninto two\"\n";
    assert_eq!(
        sanitize_basic_string_newlines(escaped),
        "prompt = \"said \\\"hi\\\" then \\\\ broke\\ninto two\"\n"
    );
}

#[test]
fn sanitizer_escapes_nested_quotes_but_keeps_closers() {
    // Nested shell quotes are escaped; real closers survive.
    let doc = "[flow]\nname = \"q\"\n\n[[nodes]]\nid = \"a\"\ntype = \"bash\"\ncommand = \"test -z \"$(grep x f)\" && echo \"done\"\"\n";
    let fixed = sanitize_basic_string_newlines(doc);
    assert!(
        vak_flow::parse_flow(&fixed).is_ok(),
        "sanitized doc must parse: {fixed}"
    );
    assert!(fixed.contains("\\\"$(grep x f)\\\""));
    assert!(fixed.contains("\\\"done\\\""));

    // Closing quotes followed by comment / whitespace+comma stay closers.
    let ok = "name = \"x\" # trailing\nnodes-note = [ \"a\" , \"b\" ]\n";
    assert_eq!(sanitize_basic_string_newlines(ok), ok);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiline_basic_string_plan_is_sanitized_and_executes() {
    // Regression: live planner runs emitted raw newlines inside
    // `prompt = "..."`, which is invalid TOML. The structural sanitizer must
    // repair it so the plan parses and executes instead of failing closed.
    let plan = "[flow]\nname = \"multiline\"\n\n[[nodes]]\nid = \"probe\"\ntype = \"bash\"\ncommand = \"echo line-one\necho line-two\"\n\n[[nodes]]\nid = \"report\"\ntype = \"merge\"\ndeps = [\"probe\"]\n";
    let provider = Arc::new(ScriptedPlanner {
        responses: Mutex::new(VecDeque::from([ScriptedResponse::Text(fenced(plan))])),
    });
    let deps = make_deps(provider);

    let outcome = drain(plan_and_run(
        deps,
        "multiline task",
        CancellationToken::new(),
        mpsc::channel(64).0,
    ))
    .await;

    match outcome {
        PlanOutcome::Completed { outputs, .. } => {
            let out = outputs.get("probe").unwrap();
            assert!(out.contains("line-one") && out.contains("line-two"));
        }
        other => panic!("expected completed after sanitize, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_planner_failure_is_retried() {
    // Invariant 7: the planner bypasses the loop's retry machinery, so it
    // must retry transient provider failures itself instead of failing the
    // whole run closed.
    let provider = Arc::new(ScriptedPlanner {
        responses: Mutex::new(VecDeque::from(vec![
            ScriptedResponse::Error(LlmError::Network("connection reset".into())),
            ScriptedResponse::Text(fenced(GOOD_PLAN)),
        ])),
    });
    let deps = make_deps(provider);

    let outcome = drain(plan_and_run(
        deps,
        "flaky provider task",
        CancellationToken::new(),
        mpsc::channel(64).0,
    ))
    .await;

    match outcome {
        PlanOutcome::Completed { attempts, .. } => assert_eq!(attempts, 1),
        other => panic!("expected completed after retry, got {other:?}"),
    }
}
