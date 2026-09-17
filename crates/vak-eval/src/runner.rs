use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::bash::BashTool;
use vak_tools::{Tool, ToolContext};

/// One scripted assistant turn: a plain text reply or a batch of tool calls
/// (the loop executes them and requests the next turn).
#[derive(Debug, Clone)]
pub enum ScriptedTurn {
    Text(String),
    ToolCalls(Vec<(String, String, serde_json::Value)>),
}

impl ScriptedTurn {
    pub fn tool(name: &str, args: serde_json::Value) -> Self {
        ScriptedTurn::ToolCalls(vec![(format!("t-{}", uuid_like()), name.to_string(), args)])
    }

    pub fn tool_calls(calls: Vec<(&str, serde_json::Value)>) -> Self {
        ScriptedTurn::ToolCalls(
            calls
                .into_iter()
                .map(|(name, args)| (format!("t-{}", uuid_like()), name.to_string(), args))
                .collect(),
        )
    }

    fn to_message(&self) -> AssistantMessage {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        };
        match self {
            ScriptedTurn::Text(t) => AssistantMessage {
                content: vec![ContentBlock::text(t.clone())],
                stop_reason: StopReason::EndTurn,
                usage,
                model: "eval-model".into(),
            },
            ScriptedTurn::ToolCalls(calls) => AssistantMessage {
                content: calls
                    .iter()
                    .map(|(id, name, input)| ContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    })
                    .collect(),
                stop_reason: StopReason::ToolUse,
                usage,
                model: "eval-model".into(),
            },
        }
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static C: AtomicU32 = AtomicU32::new(0);
    format!("u{}", C.fetch_add(1, Ordering::Relaxed))
}

/// Serves the case's turns in order; records every request for audit.
pub struct EvalProvider {
    turns: Mutex<VecDeque<AssistantMessage>>,
    pub requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl EvalProvider {
    pub fn new(turns: Vec<ScriptedTurn>) -> Self {
        EvalProvider {
            turns: Mutex::new(turns.into_iter().map(|t| t.to_message()).collect()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl Provider for EvalProvider {
    fn name(&self) -> &str {
        "eval-scripted"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        let next = self
            .turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => {
                sink.close_error(LlmError::Parse("eval script exhausted".into()))
                    .await;
            }
        }
        Ok(rx)
    }
}

#[derive(Debug, Clone)]
pub struct EvalCase {
    pub id: String,
    pub description: String,
    /// Files written into the sandbox workspace before the run.
    pub files: Vec<(String, String)>,
    pub prompt: String,
    pub script: Vec<ScriptedTurn>,
    /// Optional admitted contract for this case. When absent, the evaluator
    /// uses the general-purpose baseline for compatibility with simple cases.
    pub outcome: Option<vak_intent::OutcomeSpec>,
    /// Bash command run in the workspace after the agent finishes; exit 0 = pass.
    pub verify: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalReport {
    pub task_id: String,
    pub passed: bool,
    pub verify_exit: Option<i32>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub duration_ms: u128,
    pub outcome_status: String,
    pub completion_verdict: String,
    pub human_review: String,
    pub verification_evidence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregate measurement for a corpus. This is deliberately computed from
/// per-case production-loop reports so a green corpus cannot hide a failed
/// case behind an average.
#[derive(Debug, Clone, Serialize)]
pub struct EvalSuiteReport {
    pub cases: Vec<EvalReport>,
    pub total: usize,
    pub passed: usize,
    pub pass_rate: f64,
    pub mean_duration_ms: f64,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub partial_or_unknown: usize,
    pub human_review_recommended: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalComparisonReport {
    pub task_id: String,
    pub baseline: EvalReport,
    pub outcome_directed: EvalReport,
    pub latency_delta_ms: i128,
    pub token_delta: i128,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalComparisonSuiteReport {
    pub baseline: EvalSuiteReport,
    pub outcome_directed: EvalSuiteReport,
    pub pass_rate_delta: f64,
    pub mean_duration_delta_ms: f64,
    pub token_delta: i128,
}

pub async fn compare_case(case: &EvalCase) -> EvalComparisonReport {
    let mut baseline_case = case.clone();
    baseline_case.outcome = None;
    let baseline = run_case(&baseline_case).await;
    let outcome_directed = run_case(case).await;
    EvalComparisonReport {
        task_id: case.id.clone(),
        latency_delta_ms: outcome_directed.duration_ms as i128 - baseline.duration_ms as i128,
        token_delta: (outcome_directed.tokens_in + outcome_directed.tokens_out) as i128
            - (baseline.tokens_in + baseline.tokens_out) as i128,
        baseline,
        outcome_directed,
    }
}

pub async fn compare_suite(cases: &[EvalCase]) -> EvalComparisonSuiteReport {
    let mut baseline_cases = Vec::with_capacity(cases.len());
    for case in cases {
        let mut baseline = case.clone();
        baseline.outcome = None;
        baseline_cases.push(baseline);
    }
    let baseline = run_suite(&baseline_cases).await;
    let outcome_directed = run_suite(cases).await;
    EvalComparisonSuiteReport {
        pass_rate_delta: outcome_directed.pass_rate - baseline.pass_rate,
        mean_duration_delta_ms: outcome_directed.mean_duration_ms - baseline.mean_duration_ms,
        token_delta: (outcome_directed.total_tokens_in + outcome_directed.total_tokens_out) as i128
            - (baseline.total_tokens_in + baseline.total_tokens_out) as i128,
        baseline,
        outcome_directed,
    }
}

impl EvalSuiteReport {
    fn from_cases(cases: Vec<EvalReport>) -> Self {
        let total = cases.len();
        let passed = cases.iter().filter(|case| case.passed).count();
        let mean_duration_ms = if total == 0 {
            0.0
        } else {
            cases
                .iter()
                .map(|case| case.duration_ms as f64)
                .sum::<f64>()
                / total as f64
        };
        EvalSuiteReport {
            total,
            passed,
            pass_rate: if total == 0 {
                0.0
            } else {
                passed as f64 / total as f64
            },
            mean_duration_ms,
            total_tokens_in: cases.iter().map(|case| case.tokens_in).sum(),
            total_tokens_out: cases.iter().map(|case| case.tokens_out).sum(),
            partial_or_unknown: cases
                .iter()
                .filter(|case| matches!(case.completion_verdict.as_str(), "partial" | "unknown"))
                .count(),
            human_review_recommended: cases
                .iter()
                .filter(|case| case.human_review == "recommended")
                .count(),
            cases,
        }
    }
}

/// Run a corpus and retain every case report for inspection and comparison.
pub async fn run_suite(cases: &[EvalCase]) -> EvalSuiteReport {
    let mut reports = Vec::with_capacity(cases.len());
    for case in cases {
        reports.push(run_case(case).await);
    }
    EvalSuiteReport::from_cases(reports)
}

/// Runs one case with the built-in scripted provider (deterministic).
pub async fn run_case(case: &EvalCase) -> EvalReport {
    let provider = Arc::new(EvalProvider::new(case.script.clone()));
    run_case_with_provider(case, provider, "eval-model").await
}

pub async fn run_case_brokered(case: &EvalCase, worker_exe: std::path::PathBuf) -> EvalReport {
    let provider = Arc::new(EvalProvider::new(case.script.clone()));
    run_case_with_provider_brokered(case, provider, "eval-model", worker_exe).await
}

/// Runs one case against ANY provider — the live-model path. The `script`
/// field is ignored; the model must genuinely solve the task.
pub async fn run_case_with_provider(
    case: &EvalCase,
    provider: Arc<dyn Provider>,
    model: &str,
) -> EvalReport {
    run_case_with_tools(case, provider, model, vak_tools::default_tools()).await
}

pub async fn run_case_with_provider_brokered(
    case: &EvalCase,
    provider: Arc<dyn Provider>,
    model: &str,
    worker_exe: std::path::PathBuf,
) -> EvalReport {
    run_case_with_tools(
        case,
        provider,
        model,
        vak_tools::brokered_default_tools(worker_exe),
    )
    .await
}

async fn run_case_with_tools(
    case: &EvalCase,
    provider: Arc<dyn Provider>,
    model: &str,
    tools: Vec<Arc<dyn Tool>>,
) -> EvalReport {
    let deterministic = provider.name() == "eval-scripted";
    let start = Instant::now();
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return EvalReport {
                task_id: case.id.clone(),
                passed: false,
                verify_exit: None,
                tokens_in: 0,
                tokens_out: 0,
                duration_ms: start.elapsed().as_millis(),
                outcome_status: "unknown".into(),
                completion_verdict: "failed".into(),
                human_review: "required_for_recovery".into(),
                verification_evidence: "none".into(),
                error: Some(format!("workspace setup failed: {e}")),
            };
        }
    };
    let cwd = dir.path().to_path_buf();
    for (rel, content) in &case.files {
        let p = cwd.join(rel);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&p, content).is_err() {
            return fail(&case.id, "setup file write failed", &start, None);
        }
    }

    let header = SessionHeader {
        agent: Some(vak_session::types::AgentIdentity {
            id: "vak".into(),
            revision: 1,
            name: "Vak".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
        }),
        session_id: format!("eval-{}", case.id),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: Some(vak_session::ConversationContext::local(
            format!("eval-{}", case.id),
            "eval",
        )),
        contract: FrozenContract {
            app_version: env!("CARGO_PKG_VERSION").into(),
            provider: "eval-scripted".into(),
            model: model.to_string(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "eval".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let sessions_home = cwd.join(".vak-home");
    let log = match SessionLog::create(
        vak_session::SessionPath::new_session_file(&sessions_home, &cwd, &header.session_id),
        header,
    ) {
        Ok(l) => l,
        Err(e) => {
            return fail(
                &case.id,
                &format!("session setup failed: {e}"),
                &start,
                None,
            );
        }
    };

    let prepared = vak_core::PreparedTurn::from_parts("eval", tools, Vec::new());
    let mut cfg = AgentConfig::new(prepared.system_prompt);
    cfg.model = model.to_string();
    cfg.tools = prepared.tools;
    cfg.outcome = case.outcome.clone();
    cfg.max_turns = 12;
    if deterministic {
        cfg.max_retries = 0;
        cfg.run_retry_attempts = 0;
    }
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.mode = Mode::FullAccess;
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            &case.prompt,
            &Default::default(),
            CancellationToken::new(),
            tokio::sync::mpsc::channel(256).0,
        )
        .await;

    let (tokens_in, tokens_out, loop_error) = {
        let session = agent.session.lock().await;
        let u = session.total_usage();
        let err = match outcome {
            TurnOutcome::Completed { .. } => None,
            TurnOutcome::Aborted { .. } => Some("aborted".to_string()),
            TurnOutcome::Failed { ref error } => Some(error.to_string()),
            TurnOutcome::MaxTurnsReached => Some("max turns reached".to_string()),
        };
        (u.input_tokens, u.output_tokens, err)
    };

    // Verification runs in the same workspace with the production bash tool.
    let verify_ctx = ToolContext {
        cwd: cwd.clone(),
        cancel: CancellationToken::new(),
        limits: Default::default(),
        sandbox: None,
        sandbox_sink: None,
        agent_id: None,
    };
    promote_scratch_artifacts(&cwd);
    let verify_tool = BashTool;
    let verify_command = format!(
        "(cd {} && ({})) || (cd {} && ({}))",
        shell_quote(&cwd),
        case.verify,
        shell_quote(&cwd.join(".vak").join("scratch")),
        case.verify
    );
    let verify_out = verify_tool
        .execute(&serde_json::json!({"command": verify_command}), &verify_ctx)
        .await;

    let report = EvalReport {
        task_id: case.id.clone(),
        passed: !verify_out.is_error,
        verify_exit: Some(if verify_out.is_error { 1 } else { 0 }),
        tokens_in,
        tokens_out,
        duration_ms: start.elapsed().as_millis(),
        outcome_status: format!(
            "{:?}",
            vak_intent::evaluate_response(
                match &outcome {
                    TurnOutcome::Completed { response } => Some(response.text_content()),
                    TurnOutcome::Aborted { partial } => {
                        partial.as_ref().map(|message| message.text_content())
                    }
                    TurnOutcome::Failed { .. } | TurnOutcome::MaxTurnsReached => None,
                }
                .as_deref(),
                loop_error.is_some(),
                matches!(outcome, TurnOutcome::Aborted { .. }),
            )
        )
        .to_ascii_lowercase(),
        completion_verdict: {
            let response = match &outcome {
                TurnOutcome::Completed { response } => Some(response.text_content()),
                TurnOutcome::Aborted { partial } => {
                    partial.as_ref().map(|message| message.text_content())
                }
                TurnOutcome::Failed { .. } | TurnOutcome::MaxTurnsReached => None,
            };
            let spec = case.outcome.clone().unwrap_or_else(|| {
                vak_intent::OutcomeSpec::from_reading(
                    &case.prompt,
                    &vak_intent::Reading::general(),
                    vak_intent::RESOLVER_VERSION,
                )
            });
            let status = vak_intent::evaluate_response(
                response.as_deref(),
                loop_error.is_some(),
                matches!(outcome, TurnOutcome::Aborted { .. }),
            );
            let evaluations = vak_intent::evaluate_requirements(&spec, response.as_deref());
            let verdict = vak_intent::evaluate_completion(status, &evaluations, &spec);
            if verify_out.is_error && matches!(verdict, vak_intent::CompletionVerdict::Complete) {
                "partial".into()
            } else {
                format!("{verdict:?}").to_ascii_lowercase()
            }
        },
        human_review: if verify_out.is_error
            || case.outcome.as_ref().is_some_and(|outcome| {
                outcome.requirements.iter().any(|requirement| {
                    matches!(requirement.kind, vak_intent::RequirementKind::Evidence)
                })
            }) {
            "recommended".into()
        } else {
            "not_required".into()
        },
        verification_evidence: if verify_out.is_error {
            "observed_failure".into()
        } else {
            "observed_success".into()
        },
        error: loop_error.or_else(|| {
            if verify_out.is_error {
                Some(format!("verify failed: {}", verify_out.content))
            } else {
                None
            }
        }),
    };
    keep_workspace(&cwd, &report);
    report
}

fn fail(id: &str, msg: &str, start: &Instant, verify_exit: Option<i32>) -> EvalReport {
    EvalReport {
        task_id: id.to_string(),
        passed: false,
        verify_exit,
        tokens_in: 0,
        tokens_out: 0,
        duration_ms: start.elapsed().as_millis(),
        outcome_status: "unknown".into(),
        completion_verdict: "failed".into(),
        human_review: "required_for_recovery".into(),
        verification_evidence: "none".into(),
        error: Some(msg.to_string()),
    }
}

/// Keep failing workspaces under target/ for post-mortem; drop passing ones.
fn keep_workspace(_cwd: &Path, report: &EvalReport) {
    let _ = report;
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace("'", "'\\''"))
}

fn promote_scratch_artifacts(cwd: &Path) {
    let scratch = cwd.join(".vak").join("scratch");
    fn visit(dir: &Path, cwd: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, cwd);
            } else if let Some(name) = path.file_name() {
                let _ = std::fs::copy(&path, cwd.join(name));
            }
        }
    }
    visit(&scratch, cwd);
}
