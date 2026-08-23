//! vak-agent: the agent loop.
//!
//! Errors are values: run() never panics; every failure mode is a typed
//! TurnOutcome. Model context is always projected from the session log
//! (model-visible means logged).

pub mod circuit;
pub mod context;
pub mod spend;
pub mod steering;
pub mod stop_policy;
pub mod task;

pub use circuit::{CircuitBreaker, CircuitBreakerConfig, CircuitOpen};
pub use spend::{SpendCheck, SpendGate};
pub use stop_policy::{BlockReason, StopPolicy};
pub use task::{ActiveSubagent, SubagentHandle, SubagentRegistry, TaskDeps, TaskTool};

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use vak_llm::{
    AssistantMessage, ChatRequest, ContentBlock, LlmError, Message, Provider, Role, StopReason,
    StreamEvent, Usage,
    work::{AttemptReason, FailureDomain, Settlement, StepLedger, WorkPurpose},
};
use vak_permission::{Decision, Mode, PermissionEngine};
use vak_session::{MessageMeta, MessageRecord, SessionLog};
use vak_tools::Tool;

pub use steering::{DrainMode, SteeringQueues};

pub use async_trait;

#[derive(Debug, Clone, serde::Serialize)]
pub enum AgentEvent {
    TurnStart {
        turn: usize,
    },
    Stream(StreamEvent),
    ToolCallStart {
        id: String,
        name: String,
        args_json: String,
    },
    ToolCallEnd {
        id: String,
        name: String,
        is_error: bool,
        result_preview: Option<String>,
    },
    TurnEnd {
        usage: Usage,
    },
    StopHookContinuation {
        reason: String,
    },
    RetryScheduled {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
    ContextCompacting {
        estimated_tokens: u64,
    },
    ContextCompacted {
        before_tokens: u64,
        after_tokens: u64,
        summarized_messages: usize,
        /// Packet accounting (doc 27 Phase C): how many visible message
        /// entries stayed verbatim vs became summary material.
        selected_messages: usize,
        dropped_messages: usize,
    },
    StreamOpened,
    ApprovalRequested {
        id: String,
        tool: String,
        args_json: String,
        reason: String,
    },
    SubagentStarted {
        label: String,
    },
    SubagentToolCall {
        label: String,
        name: String,
        is_error: bool,
    },
    SubagentUsage {
        label: String,
        input_tokens: u64,
        output_tokens: u64,
    },
    SubagentFinished {
        label: String,
        is_error: bool,
        elapsed_ms: u64,
    },
    RunFinished {
        summary: String,
        /// Whether the run ended badly. Consumers must not have to sniff
        /// `summary` for a "failed:" prefix to know something broke —
        /// errors are values (see SubagentFinished above).
        is_error: bool,
    },
}

#[derive(Debug)]
pub enum TurnOutcome {
    Completed { response: AssistantMessage },
    Aborted { partial: Option<AssistantMessage> },
    Failed { error: LlmError },
    MaxTurnsReached,
}

pub struct AgentConfig {
    pub system_prompt: String,
    pub model: String,
    pub tools: Vec<Arc<dyn Tool>>,
    pub max_turns: usize,
    pub parallel_tools: bool,
    pub permission: Option<Arc<PermissionEngine>>,
    pub mode: Mode,
    pub approver: Option<Arc<dyn Approver>>,
    pub sandbox: Option<Arc<dyn vak_tools::sandbox::Sandbox>>,
    pub hooks: Option<Arc<Vec<vak_hooks::HookDef>>>,
    /// Retries for transient provider errors (429/529/network) per step.
    pub max_retries: u32,
    /// Exponential backoff base: delay = base * 2^(attempt-1), jittered.
    pub retry_base_backoff_ms: u64,
    /// Whole-step deadline (connect + stream + collect). None disables.
    pub request_timeout: Option<std::time::Duration>,
    /// Shared cross-run provider-health breaker. None disables.
    pub circuit_breaker: Option<Arc<CircuitBreaker>>,
    /// Run-level endurance: when a model step exhausts its retry budget with
    /// a transient error (rate limit / overload / network / truncated
    /// stream), back off and re-attempt the same turn this many times
    /// before failing the run. Nothing has been committed to the ledger at
    /// that point, so the re-attempt is exact. 0 disables (fail on first
    /// step exhaustion).
    pub run_retry_attempts: u32,
    /// Exponential backoff base for run-level endurance, capped at 30s.
    pub run_retry_base_backoff_ms: u64,
    /// Hard cap on provider dispatches for one unit of work (doc 27 Phase
    /// A). Exhaustion fails closed before another paid call goes out. The
    /// single-ladder default codifies today's worst case:
    /// `(max_retries + 1) * (run_retry_attempts + 1)`; the frozen ladder
    /// (Phase B) tightens this to `ladder + repair allowance`.
    pub dispatch_ceiling: u32,
    /// Long-horizon context policy (window, reserve, compaction trigger).
    pub context_policy: context::ContextPolicy,
    /// Built-in premature-completion gate. None disables entirely.
    pub stop_policy: Option<StopPolicy>,
    /// Pre-dispatch budget admission (docs/design/27 Phase D). None
    /// disables spend gating entirely.
    pub spend_gate: Option<Arc<dyn SpendGate>>,
}

impl AgentConfig {
    pub fn new(system_prompt: impl Into<String>) -> Self {
        AgentConfig {
            system_prompt: system_prompt.into(),
            model: String::new(),
            tools: Vec::new(),
            max_turns: 40,
            parallel_tools: true,
            permission: None,
            mode: Mode::WorkspaceWrite,
            approver: None,
            sandbox: None,
            hooks: None,
            max_retries: 3,
            retry_base_backoff_ms: 500,
            request_timeout: Some(std::time::Duration::from_secs(600)),
            circuit_breaker: None,
            run_retry_attempts: 6,
            run_retry_base_backoff_ms: 2_000,
            dispatch_ceiling: (3 + 1) * (6 + 1),
            context_policy: Default::default(),
            stop_policy: Some(StopPolicy::default()),
            spend_gate: None,
        }
    }
}

#[async_trait::async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, tool: &str, args_json: &str, reason: &str) -> bool;
}

/// Errors worth surviving at run level: sustained fault windows, hung or
/// truncated streams. Permanent errors (auth, bad request, non-2xx api,
/// aborts) are excluded — retrying them cannot help.
fn is_transient_step_error(e: &LlmError) -> bool {
    matches!(
        e,
        LlmError::RateLimit { .. }
            | LlmError::Overloaded(_)
            | LlmError::Network(_)
            | LlmError::Parse(_)
    )
}

/// The breaker protects against a DEAD provider: blind failures with no
/// server guidance (network loss, watchdog deadlines, truncated or malformed
/// streams). Informed transience — 429 with Retry-After, explicit 503/529
/// overload — is the server saying "try again later"; endurance handles it
/// by waiting, and it must not open the circuit mid-window (found in the
/// live chaos campaign: an opened breaker killed runs the window would have
/// released seconds later).
fn trips_breaker(e: &LlmError) -> bool {
    matches!(e, LlmError::Network(_) | LlmError::Parse(_))
}

pub struct AutoApprove;

#[async_trait::async_trait]
impl Approver for AutoApprove {
    async fn approve(&self, _tool: &str, _args_json: &str, _reason: &str) -> bool {
        true
    }
}

pub struct AutoDeny;

#[async_trait::async_trait]
impl Approver for AutoDeny {
    async fn approve(&self, _tool: &str, _args_json: &str, _reason: &str) -> bool {
        false
    }
}

/// Compact JSON preview of a tool input; capped so UI surfaces never
/// absorb unbounded payloads.
fn args_preview(input: &Value) -> String {
    let json = serde_json::to_string(input).unwrap_or_else(|_| "{}".into());
    truncate_chars(&json, ARGS_PREVIEW_LIMIT)
}

fn result_preview(content: &str) -> Option<String> {
    if content.is_empty() {
        None
    } else {
        Some(truncate_chars(content, RESULT_PREVIEW_LIMIT))
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

const ARGS_PREVIEW_LIMIT: usize = 400;
const RESULT_PREVIEW_LIMIT: usize = 2000;

#[derive(Clone)]
struct PendingToolCall {
    id: String,
    name: String,
    input: Value,
}

pub struct Agent {
    provider: Arc<dyn Provider>,
    pub session: Mutex<SessionLog>,
    pub config: AgentConfig,
    /// Identical-call detector for the doom-loop guard, reset per run.
    run_call_counts: std::sync::Mutex<HashMap<String, u32>>,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, session: SessionLog, config: AgentConfig) -> Self {
        Agent {
            provider,
            session: Mutex::new(session),
            config,
            run_call_counts: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub async fn run(
        &mut self,
        prompt: &str,
        steering: &SteeringQueues,
        cancel: CancellationToken,
        events: mpsc::Sender<AgentEvent>,
    ) -> TurnOutcome {
        self.run_message(Message::user_text(prompt), steering, cancel, events)
            .await
    }

    /// Run with a prebuilt prompt `Message` — the seam for multimodal
    /// (image) input; the ledger stores exactly what the model sees.
    pub async fn run_message(
        &mut self,
        prompt: Message,
        steering: &SteeringQueues,
        cancel: CancellationToken,
        events: mpsc::Sender<AgentEvent>,
    ) -> TurnOutcome {
        let prompt_owned = prompt.text_content();
        if let Err(e) = self
            .session
            .lock()
            .await
            .append_message(MessageRecord {
                message: prompt,
                meta: None,
            })
            .map_err(|e| LlmError::Network(format!("session write failed: {e}")))
        {
            return TurnOutcome::Failed { error: e };
        }

        let mut turn = 0usize;
        let mut bash_calls_this_run: u32 = 0;
        self.run_call_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let mut stop_blocks_left = self
            .config
            .stop_policy
            .as_ref()
            .map(|p| p.max_blocks)
            .unwrap_or(0);
        loop {
            if cancel.is_cancelled() {
                return TurnOutcome::Aborted { partial: None };
            }
            if turn >= self.config.max_turns {
                return TurnOutcome::MaxTurnsReached;
            }

            {
                let mut session = self.session.lock().await;
                for text in steering.drain(DrainMode::OneAtATime) {
                    let _ = session.append_message(MessageRecord {
                        message: Message::user_text(text),
                        meta: None,
                    });
                }
            }

            let _ = events.send(AgentEvent::TurnStart { turn }).await;

            let model = {
                let session = self.session.lock().await;
                if self.config.model.is_empty() {
                    session
                        .header()
                        .map(|h| h.contract.model.clone())
                        .unwrap_or_default()
                } else {
                    self.config.model.clone()
                }
            };

            // Long-horizon guard: estimate the projection; on overflow,
            // summarize older turns into a compaction entry and retry
            // the same contract. Still over afterwards, or no progress,
            // => fail closed. The lock is taken only for short read /
            // plan / apply phases and is NEVER held across the summarizer
            // network call below.
            enum CompactionNeed {
                None,
                Plan(vak_session::types::CompactionPlan, u64),
                TooShortToCompact,
            }
            let need = {
                let session = self.session.lock().await;
                let policy = &self.config.context_policy;
                let system = self.config.system_prompt.as_str();
                let tool_defs = vak_tools::definitions(&self.config.tools);
                let est =
                    context::estimate_tokens(&session.derive_messages(), Some(system), &tool_defs);
                if est <= policy.trigger_at() {
                    CompactionNeed::None
                } else {
                    match session.plan_compaction(policy.keep_recent) {
                        Some(plan) => CompactionNeed::Plan(plan, est),
                        None => CompactionNeed::TooShortToCompact,
                    }
                }
            };
            match need {
                CompactionNeed::TooShortToCompact => {
                    return TurnOutcome::Failed {
                        error: LlmError::Network(
                            "context over budget but too short to compact".into(),
                        ),
                    };
                }
                CompactionNeed::Plan(plan, tokens_before) => {
                    let transcript = context::render_transcript(&plan.older);
                    let _ = events
                        .send(AgentEvent::ContextCompacting {
                            estimated_tokens: tokens_before,
                        })
                        .await;

                    // Network call runs with no session lock held.
                    let req = context::compaction_request(&model, &transcript);
                    let mut ledger = StepLedger::new(
                        WorkPurpose::Summarize,
                        &model,
                        self.config.dispatch_ceiling,
                    );
                    let summary_msg = match self
                        .complete_with_reliability(&req, &cancel, &events, false, &mut ledger)
                        .await
                    {
                        Ok(m) => {
                            let sid = {
                                let mut session = self.session.lock().await;
                                let sid = session
                                    .header()
                                    .map(|h| h.session_id.clone())
                                    .unwrap_or_default();
                                let _ = session.append_receipt(ledger.take_receipt());
                                sid
                            };
                            if let Some(gate) = &self.config.spend_gate {
                                gate.record_settled(&m.model, &sid, &m.usage);
                            }
                            m
                        }
                        Err(LlmError::Aborted { .. }) => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Aborted { partial: None };
                        }
                        Err(e) => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Failed {
                                error: LlmError::Network(format!("compaction call failed: {e}")),
                            };
                        }
                    };
                    let summary = summary_msg.text_content();
                    if summary.trim().is_empty() {
                        return TurnOutcome::Failed {
                            error: LlmError::Network("compaction produced an empty summary".into()),
                        };
                    }

                    {
                        let mut session = self.session.lock().await;
                        if let Err(e) = session.apply_compaction(&plan, summary, tokens_before) {
                            return TurnOutcome::Failed {
                                error: LlmError::Network(format!("compaction write failed: {e}")),
                            };
                        }
                    }

                    let est = {
                        let session = self.session.lock().await;
                        let system = self.config.system_prompt.as_str();
                        let tool_defs = vak_tools::definitions(&self.config.tools);
                        context::estimate_tokens(
                            &session.derive_messages(),
                            Some(system),
                            &tool_defs,
                        )
                    };
                    if est >= tokens_before {
                        return TurnOutcome::Failed {
                            error: LlmError::Network(format!(
                                "compaction made no progress (~{tokens_before} -> ~{est} tokens)"
                            )),
                        };
                    }
                    let _ = events
                        .send(AgentEvent::ContextCompacted {
                            before_tokens: tokens_before,
                            after_tokens: est,
                            summarized_messages: plan.older.len(),
                            selected_messages: plan.partition.selected_entry_ids.len(),
                            dropped_messages: plan.partition.dropped_entry_ids.len(),
                        })
                        .await;
                    if est > self.config.context_policy.input_budget() {
                        return TurnOutcome::Failed {
                            error: LlmError::Network(format!(
                                "context still over budget after compaction (~{est} > {} tokens)",
                                self.config.context_policy.input_budget()
                            )),
                        };
                    }
                }
                CompactionNeed::None => {}
            }

            // One model step = connect + stream + collect, wrapped with the
            // full reliability machinery (watchdog, retries+backoff,
            // circuit breaker, dispatch ceiling). Every dispatch is recorded
            // into the work receipt, which lands in the ledger on every
            // exit path. User aborts and partial-output aborts are never
            // retried; they propagate for caller handling.
            let mut ledger =
                StepLedger::new(WorkPurpose::Execute, &model, self.config.dispatch_ceiling);
            let base_request = {
                let session = self.session.lock().await;
                ChatRequest {
                    model,
                    system: Some(self.config.system_prompt.clone()),
                    messages: session.derive_messages(),
                    tools: vak_tools::definitions(&self.config.tools),
                    max_tokens: self.config.context_policy.max_output as u32,
                    temperature: None,
                }
            };
            let request = base_request.clone();

            let response = {
                // Run-level endurance: a sustained fault window (rate-limit
                // burst, slow/hung upstream, truncating proxy) can outlast
                // one step's retry budget. The ledger has not been touched,
                // so re-attempting the whole turn is exact. Aborts, permanent
                // errors, and ceiling exhaustion still fail/abort immediately.
                let mut run_attempt: u32 = 0;
                let mut backoff_ms = self.config.run_retry_base_backoff_ms.max(1);
                loop {
                    match self
                        .complete_with_reliability(&request, &cancel, &events, true, &mut ledger)
                        .await
                    {
                        Ok(r) => break r,
                        Err(LlmError::Aborted { partial }) => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            if let Some(p) = &partial {
                                self.append_assistant(p).await;
                            }
                            return TurnOutcome::Aborted { partial };
                        }
                        Err(e) if ledger.budget.remaining() == 0 => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Failed {
                                error: LlmError::Network(format!(
                                    "dispatch ceiling of {} exhausted for this step; last error: {e}",
                                    self.config.dispatch_ceiling
                                )),
                            };
                        }
                        Err(e)
                            if run_attempt < self.config.run_retry_attempts
                                && is_transient_step_error(&e) =>
                        {
                            run_attempt += 1;
                            let delay = backoff_ms.min(30_000);
                            // An open breaker fails every attempt instantly
                            // until its cooldown elapses; pacing the wait to
                            // the remaining cooldown lets the half-close probe
                            // through instead of burning the budget on no-op
                            // failures.
                            let cooldown_ms = self
                                .config
                                .circuit_breaker
                                .as_ref()
                                .and_then(|b| b.check().err())
                                .map(|open| open.remaining_secs * 1000 + 250)
                                .unwrap_or(0);
                            let delay = delay.max(cooldown_ms);
                            let _ = events
                                .send(AgentEvent::RetryScheduled {
                                    attempt: run_attempt,
                                    delay_ms: delay,
                                    reason: format!(
                                        "step exhausted ({e}); run-level re-attempt {run_attempt}/{}",
                                        self.config.run_retry_attempts
                                    ),
                                })
                                .await;
                            if tokio::select! {
                                _ = cancel.cancelled() => false,
                                _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => true,
                            } {
                                backoff_ms = backoff_ms.saturating_mul(2);
                                continue;
                            }
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Aborted { partial: None };
                        }
                        Err(e) => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Failed { error: e };
                        }
                    }
                }
            };

            let usage = response.usage.clone();
            let settled_session_id = {
                let mut session = self.session.lock().await;
                let sid = session
                    .header()
                    .map(|h| h.session_id.clone())
                    .unwrap_or_default();
                let _ = session.append_receipt(ledger.take_receipt());
                sid
            };
            if let Some(gate) = &self.config.spend_gate {
                gate.record_settled(&response.model, &settled_session_id, &usage);
            }
            self.append_assistant(&response).await;
            let _ = events.send(AgentEvent::TurnEnd { usage }).await;

            if response.stop_reason != StopReason::ToolUse {
                if let Some(hooks) = &self.config.hooks {
                    let session_id = self
                        .session
                        .lock()
                        .await
                        .header()
                        .map(|h| h.session_id.clone())
                        .unwrap_or_default();
                    let cwd = self
                        .session
                        .lock()
                        .await
                        .header()
                        .map(|h| h.contract_cwd())
                        .unwrap_or_else(|| ".".into());
                    let stop = vak_hooks::run_hooks(
                        hooks.clone(),
                        vak_hooks::HookEvent::Stop,
                        &session_id,
                        &cwd,
                        None,
                        Some(&response.text_content()),
                        &cancel,
                    )
                    .await;
                    if stop.blocked && turn + 1 < self.config.max_turns {
                        let reason = stop
                            .reason
                            .unwrap_or_else(|| "continue required by hook".into());
                        let _ = events
                            .send(AgentEvent::StopHookContinuation {
                                reason: reason.clone(),
                            })
                            .await;
                        let _ = self.session.lock().await.append_message(MessageRecord {
                            message: Message::user_text(format!(
                                "[stop-hook]: {reason}\nPlease continue."
                            )),
                            meta: None,
                        });
                        turn += 1;
                        continue;
                    }
                }
                if let Some(reason) = self
                    .stop_gate(
                        &prompt_owned,
                        &response,
                        bash_calls_this_run,
                        &mut stop_blocks_left,
                    )
                    .await
                    && self.guard_continue(reason, &events, turn).await
                {
                    turn += 1;
                    continue;
                }
                return TurnOutcome::Completed { response };
            }

            let calls = extract_tool_calls(&response);
            if calls.is_empty() {
                if let Some(reason) = self
                    .stop_gate(
                        &prompt_owned,
                        &response,
                        bash_calls_this_run,
                        &mut stop_blocks_left,
                    )
                    .await
                    && self.guard_continue(reason, &events, turn).await
                {
                    turn += 1;
                    continue;
                }
                return TurnOutcome::Completed { response };
            }

            bash_calls_this_run += calls.iter().filter(|c| c.name == "bash").count() as u32;
            let results = self.execute_batch(calls, &cancel, &events).await;
            let blocks = results
                .into_iter()
                .map(|(id, out)| match out {
                    ToolRunOutput::Ok(content) => ContentBlock::tool_result(id, content),
                    ToolRunOutput::Err(content) => ContentBlock::tool_error(id, content),
                })
                .collect();

            if let Err(e) = self
                .session
                .lock()
                .await
                .append_message(MessageRecord {
                    message: Message {
                        role: Role::User,
                        content: blocks,
                    },
                    meta: None,
                })
                .map_err(|e| LlmError::Network(format!("session write failed: {e}")))
            {
                return TurnOutcome::Failed { error: e };
            }

            turn += 1;
        }
    }

    /// Internal premature-completion gate. Returns a continuation reason
    /// when the stop policy fires and budget remains.
    async fn stop_gate(
        &self,
        prompt: &str,
        response: &AssistantMessage,
        bash_calls_this_run: u32,
        blocks_left: &mut u32,
    ) -> Option<String> {
        let policy = self.config.stop_policy.as_ref()?;
        if *blocks_left == 0 {
            return None;
        }
        let reason = policy.evaluate(prompt, &response.text_content(), bash_calls_this_run)?;
        *blocks_left -= 1;
        Some(reason.message())
    }

    /// Appends the continue nudge (model-visible => logged) and reports
    /// whether the loop may continue within max_turns.
    async fn guard_continue(
        &mut self,
        reason: String,
        events: &mpsc::Sender<AgentEvent>,
        turn: usize,
    ) -> bool {
        if turn + 1 >= self.config.max_turns {
            return false;
        }
        let _ = events
            .send(AgentEvent::StopHookContinuation {
                reason: reason.clone(),
            })
            .await;
        let _ = self.session.lock().await.append_message(MessageRecord {
            message: Message::user_text(format!("[stop-guard]: {reason}\nPlease continue.")),
            meta: None,
        });
        true
    }

    /// Recovers the session ledger after a run (server/API consumers).
    pub async fn into_session(self) -> SessionLog {
        self.session.into_inner()
    }

    async fn append_assistant(&self, response: &AssistantMessage) {
        let mut session = self.session.lock().await;
        let _ = session.append_message(MessageRecord {
            message: response.clone().into_message(),
            meta: Some(MessageMeta {
                model: Some(response.model.clone()),
                stop_reason: Some(format!("{:?}", response.stop_reason).to_lowercase()),
                usage: Some(response.usage.clone()),
            }),
        });
    }

    /// One provider completion with watchdog, retry/backoff (honoring
    /// Retry-After), circuit breaker, dispatch-ceiling enforcement, and
    /// per-attempt receipt recording. When `forward` is true, stream
    /// deltas are forwarded to `events`; otherwise they are drained.
    async fn complete_with_reliability(
        &self,
        request: &ChatRequest,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
        forward: bool,
        ledger: &mut StepLedger,
    ) -> Result<AssistantMessage, LlmError> {
        if let Some(breaker) = &self.config.circuit_breaker {
            breaker
                .check()
                .map_err(|open| LlmError::Network(open.to_string()))?;
        }
        let mut attempt: u32 = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(LlmError::Aborted { partial: None });
            }
            // Budget admission precedes every paid dispatch (Phase D). A
            // denial becomes one bounded budget Ask; refusal — or no
            // approver, which is the unattended case — fails the step
            // permanently (never retried, never breaker-tripping).
            if let Some(gate) = &self.config.spend_gate {
                let session_id = self
                    .session
                    .lock()
                    .await
                    .header()
                    .map(|h| h.session_id.clone())
                    .unwrap_or_default();
                let est_input = context::estimate_tokens(
                    &request.messages,
                    request.system.as_deref(),
                    &request.tools,
                );
                let check = SpendCheck {
                    model: &request.model,
                    session_id: &session_id,
                    est_input_tokens: est_input,
                    planned_output_tokens: self.config.context_policy.max_output,
                };
                if let Err(reason) = gate.authorize(&check).await {
                    let approved = match &self.config.approver {
                        Some(a) => {
                            a.approve(
                                "finops-budget",
                                &args_preview(&serde_json::json!({
                                    "model": request.model,
                                    "reason": reason,
                                })),
                                &reason,
                            )
                            .await
                        }
                        None => false,
                    };
                    if !approved {
                        return Err(LlmError::InvalidRequest(format!(
                            "budget admission denied: {reason}"
                        )));
                    }
                }
            }
            // Ceiling check happens before every paid dispatch; exhaustion
            // surfaces as a plain error that the endurance loop treats as
            // fail-closed (never transient).
            if let Err(c) = ledger.budget.consume() {
                return Err(LlmError::Network(c.to_string()));
            }
            let reason = if attempt == 0 {
                AttemptReason::Initial
            } else {
                AttemptReason::Retry
            };
            let started = std::time::Instant::now();

            let step = async {
                let mut stream = self
                    .provider
                    .stream(request.clone(), cancel.clone())
                    .await?;
                while let Some(ev) = futures::StreamExt::next(&mut stream).await {
                    if forward && events.send(AgentEvent::Stream(ev)).await.is_err() {
                        cancel.cancel();
                    }
                }
                stream.result().await
            };

            let mut domain_override: Option<FailureDomain> = None;
            let outcome = match self.config.request_timeout {
                Some(t) => match tokio::time::timeout(t, step).await {
                    Ok(r) => r,
                    Err(_) => {
                        domain_override = Some(FailureDomain::Deadline);
                        Err(LlmError::Network(format!(
                            "model step exceeded deadline of {}s",
                            t.as_secs()
                        )))
                    }
                },
                None => step.await,
            };
            let elapsed_ms = started.elapsed().as_millis() as u64;

            match outcome {
                Ok(r) => {
                    if let Some(breaker) = &self.config.circuit_breaker {
                        breaker.record_success();
                    }
                    ledger.receipt.record(
                        reason,
                        FailureDomain::Unknown,
                        Settlement::Ok,
                        elapsed_ms,
                        Some(r.usage.clone()),
                        None,
                    );
                    return Ok(r);
                }
                Err(e @ LlmError::Aborted { .. }) => {
                    ledger.receipt.record(
                        reason,
                        FailureDomain::Unknown,
                        Settlement::Cancelled,
                        elapsed_ms,
                        None,
                        None,
                    );
                    return Err(e);
                }
                Err(e) => {
                    let (domain, settlement) = vak_llm::work::classify_error(&e);
                    ledger.receipt.record(
                        reason,
                        domain_override.unwrap_or(domain),
                        settlement,
                        elapsed_ms,
                        None,
                        Some(e.to_string()),
                    );
                    if e.is_retryable() && attempt < self.config.max_retries {
                        if trips_breaker(&e)
                            && let Some(breaker) = &self.config.circuit_breaker
                        {
                            breaker.record_failure();
                        }
                        attempt += 1;
                        let delay = backoff_delay(
                            attempt,
                            e.retry_after_secs(),
                            self.config.retry_base_backoff_ms,
                        );
                        let _ = events
                            .send(AgentEvent::RetryScheduled {
                                attempt,
                                delay_ms: delay.as_millis() as u64,
                                reason: e.to_string(),
                            })
                            .await;
                        tokio::select! {
                            _ = cancel.cancelled() => {
                                return Err(LlmError::Aborted { partial: None });
                            }
                            _ = tokio::time::sleep(delay) => {}
                        }
                    } else {
                        if trips_breaker(&e)
                            && let Some(breaker) = &self.config.circuit_breaker
                        {
                            breaker.record_failure();
                        }
                        return Err(e);
                    }
                }
            }
        }
    }

    async fn execute_batch(
        &self,
        calls: Vec<PendingToolCall>,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Vec<(String, ToolRunOutput)> {
        let n = calls.len();
        let cwd = self
            .session
            .lock()
            .await
            .header()
            .map(|h| h.contract_cwd())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        let sandbox = self.config.sandbox.clone();
        let hooks = self.config.hooks.clone();
        let session_id = self
            .session
            .lock()
            .await
            .header()
            .map(|h| h.session_id.clone())
            .unwrap_or_default();

        let mut authz: Vec<Result<(), String>> = Vec::with_capacity(n);
        for call in &calls {
            authz.push(authorize(&self.config, call, &cwd, &self.run_call_counts).await);
        }
        let ids: Vec<String> = calls.iter().map(|c| c.id.clone()).collect();

        if !self.config.parallel_tools || n == 1 {
            let mut out = Vec::with_capacity(n);
            for (call, verdict) in calls.into_iter().zip(authz) {
                match verdict {
                    Err(reason) => out.push((call.id, ToolRunOutput::Err(reason))),
                    Ok(()) => {
                        if cancel.is_cancelled() {
                            out.push((call.id, ToolRunOutput::Err("cancelled".into())));
                            continue;
                        }
                        out.push(
                            execute_one(
                                call,
                                &self.config.tools,
                                &cwd,
                                &session_id,
                                hooks.as_ref(),
                                sandbox.as_ref(),
                                cancel,
                                events,
                            )
                            .await,
                        );
                    }
                }
            }
            return out;
        }

        // Greedy wave scheduling: claimed calls that conflict are placed in
        // separate waves; unclaimed and read-only calls share wave 0.
        let claims_for = |call: &PendingToolCall| -> vak_tools::ResourceClaims {
            self.config
                .tools
                .iter()
                .find(|t| t.name() == call.name)
                .map(|t| t.claims(&call.input))
                .unwrap_or_default()
        };
        let mut waves: Vec<Vec<usize>> = vec![Vec::new()];
        for (idx, call) in calls.iter().enumerate() {
            let claims = claims_for(call);
            if claims.is_unclaimed() {
                waves[0].push(idx);
                continue;
            }
            let mut placed = false;
            for wave in waves.iter_mut() {
                let compatible = wave.iter().all(|j| {
                    let other = claims_for(&calls[*j]);
                    !other.conflicts(&claims) && !claims.conflicts(&other)
                });
                if compatible {
                    wave.push(idx);
                    placed = true;
                    break;
                }
            }
            if !placed {
                waves.push(vec![idx]);
            }
        }

        let mut ordered: Vec<Option<(String, ToolRunOutput)>> = (0..n).map(|_| None).collect();
        for wave in waves {
            let mut join = tokio::task::JoinSet::new();
            for &idx in &wave {
                if authz[idx].is_err() {
                    continue;
                }
                let call = match calls.get(idx) {
                    Some(c) => c.clone(),
                    None => continue,
                };
                let tools = self.config.tools.clone();
                let cancel = cancel.clone();
                let events = events.clone();
                let cwd = cwd.clone();
                let sandbox = sandbox.clone();
                let session_id = session_id.clone();
                let hooks = hooks.clone();
                join.spawn(async move {
                    let r = execute_one(
                        call,
                        &tools,
                        &cwd,
                        &session_id,
                        hooks.as_ref(),
                        sandbox.as_ref(),
                        &cancel,
                        &events,
                    )
                    .await;
                    (idx, r)
                });
            }
            while let Some(res) = join.join_next().await {
                if let Ok((idx, pair)) = res {
                    ordered[idx] = Some(pair);
                }
            }
        }
        for (idx, verdict) in authz.into_iter().enumerate() {
            if let Err(reason) = verdict {
                ordered[idx] = Some((ids[idx].clone(), ToolRunOutput::Err(reason)));
            }
        }
        ordered.into_iter().flatten().collect()
    }
}
#[allow(clippy::too_many_arguments)]
async fn execute_one(
    call: PendingToolCall,
    tools: &[Arc<dyn Tool>],
    cwd: &std::path::Path,
    session_id: &str,
    hooks: Option<&Arc<Vec<vak_hooks::HookDef>>>,
    sandbox: Option<&Arc<dyn vak_tools::sandbox::Sandbox>>,
    cancel: &CancellationToken,
    events: &mpsc::Sender<AgentEvent>,
) -> (String, ToolRunOutput) {
    let _ = events
        .send(AgentEvent::ToolCallStart {
            id: call.id.clone(),
            name: call.name.clone(),
            args_json: args_preview(&call.input),
        })
        .await;

    if let Some(hooks) = hooks {
        let pre = vak_hooks::run_hooks(
            hooks.clone(),
            vak_hooks::HookEvent::PreToolUse,
            session_id,
            cwd,
            Some((&call.name, &call.input)),
            None,
            cancel,
        )
        .await;
        if pre.blocked {
            let reason = pre.reason.unwrap_or_else(|| "blocked by hook".into());
            let content = format!("blocked by hook: {reason}");
            let _ = events
                .send(AgentEvent::ToolCallEnd {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    is_error: true,
                    result_preview: result_preview(&content),
                })
                .await;
            return (call.id, ToolRunOutput::Err(content));
        }
    }

    let tool = tools.iter().find(|t| t.name() == call.name);
    let hook_name = call.name.clone();
    let hook_input = call.input.clone();

    let mut output = match tool {
        None => ToolRunOutput::Err(format!(
            "unknown tool: {} (available: {})",
            call.name,
            tools
                .iter()
                .map(|t| t.name())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Some(tool) => {
            let ctx = vak_tools::ToolContext {
                cwd: cwd.to_path_buf(),
                cancel: cancel.child_token(),
                limits: Default::default(),
                sandbox: sandbox.cloned(),
            };
            let tool = tool.clone();
            let res = tokio::spawn(async move { tool.execute(&call.input, &ctx).await }).await;
            match res {
                Ok(out) if out.is_error => ToolRunOutput::Err(out.content),
                Ok(out) => ToolRunOutput::Ok(out.content),
                Err(join_err) => ToolRunOutput::Err(format!("tool task failed: {join_err}")),
            }
        }
    };

    if let Some(hooks) = hooks {
        let post_reason = match &output {
            ToolRunOutput::Ok(content) => content.as_str(),
            ToolRunOutput::Err(content) => content.as_str(),
        };
        let post = vak_hooks::run_hooks(
            hooks.clone(),
            vak_hooks::HookEvent::PostToolUse,
            session_id,
            cwd,
            Some((&hook_name, &hook_input)),
            Some(post_reason),
            cancel,
        )
        .await;
        if post.blocked && matches!(output, ToolRunOutput::Ok(_)) {
            let reason = post.reason.unwrap_or_else(|| "flagged by hook".into());
            output = ToolRunOutput::Err(format!("{}\n[post-tool-use hook]: {reason}", post_reason));
        } else if post.blocked {
            let reason = post.reason.unwrap_or_else(|| "flagged by hook".into());
            output = ToolRunOutput::Err(format!("{post_reason}\n[post-tool-use hook]: {reason}"));
        }
    }

    let result_content = match &output {
        ToolRunOutput::Ok(content) | ToolRunOutput::Err(content) => content.as_str(),
    };
    let _ = events
        .send(AgentEvent::ToolCallEnd {
            id: call.id.clone(),
            name: call.name.clone(),
            is_error: matches!(output, ToolRunOutput::Err(_)),
            result_preview: result_preview(result_content),
        })
        .await;

    (call.id, output)
}

/// Doom-loop threshold: the Nth identical (tool, args) call in one run is
/// re-routed through approval instead of silently repeating.
const DOOM_LOOP_THRESHOLD: u32 = 3;

async fn authorize(
    config: &AgentConfig,
    call: &PendingToolCall,
    cwd: &std::path::Path,
    run_call_counts: &std::sync::Mutex<HashMap<String, u32>>,
) -> Result<(), String> {
    let Some(engine) = &config.permission else {
        return Ok(());
    };
    let mut decision = engine.evaluate(&call.name, &call.input, config.mode, cwd);
    if matches!(decision, Decision::Allow) {
        let key = format!(
            "{}\u{0}{}",
            call.name,
            serde_json::to_string(&call.input).unwrap_or_default()
        );
        let mut counts = run_call_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = counts.entry(key).or_insert(0);
        *n += 1;
        if n.is_multiple_of(DOOM_LOOP_THRESHOLD) {
            decision = Decision::Ask {
                reason: format!("identical {} call repeated ×{n} this run", call.name),
            };
        }
    }
    match decision {
        Decision::Allow => Ok(()),
        Decision::Deny { reason } => Err(reason),
        Decision::Ask { reason } => match &config.approver {
            Some(a)
                if a.approve(&call.name, &args_preview(&call.input), &reason)
                    .await =>
            {
                Ok(())
            }
            Some(_) => Err(format!("denied by user: {reason}")),
            None => Err(format!("{reason} (no approver available)")),
        },
    }
}

enum ToolRunOutput {
    Ok(String),
    Err(String),
}

fn extract_tool_calls(response: &AssistantMessage) -> Vec<PendingToolCall> {
    response
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => Some(PendingToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn backoff_delay(attempt: u32, retry_after_secs: Option<u64>, base_ms: u64) -> std::time::Duration {
    if let Some(secs) = retry_after_secs {
        return std::time::Duration::from_secs(secs.max(1));
    }
    let exp = base_ms.saturating_mul(1u64 << (attempt - 1).min(6));
    let jitter = rand_jitter(exp);
    std::time::Duration::from_millis((exp / 2).max(1).saturating_add(jitter).min(30_000))
}

fn rand_jitter(ms: u64) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let x = STATE
        .fetch_add(0x9E3779B97F4A7C15, Ordering::Relaxed)
        .wrapping_add(0x9E3779B97F4A7C15);
    (x >> 33) % ms.max(2)
}
