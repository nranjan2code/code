//! vak-agent: the agent loop.
//!
//! Errors are values: run() never panics; every failure mode is a typed
//! TurnOutcome. Model context is always projected from the session log
//! (model-visible means logged).

pub mod circuit;
pub mod context;
pub mod steering;
pub mod task;

pub use circuit::{CircuitBreaker, CircuitBreakerConfig, CircuitOpen};
pub use task::{TaskDeps, TaskTool};

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use vak_llm::{
    AssistantMessage, ChatRequest, ContentBlock, LlmError, Message, Provider, Role, StopReason,
    StreamEvent, Usage,
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
    },
    ToolCallEnd {
        id: String,
        name: String,
        is_error: bool,
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
    },
    StreamOpened,
    ApprovalRequested {
        id: String,
        tool: String,
        reason: String,
    },
    RunFinished {
        summary: String,
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
    /// Long-horizon context policy (window, reserve, compaction trigger).
    pub context_policy: context::ContextPolicy,
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
            context_policy: Default::default(),
        }
    }
}

#[async_trait::async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, tool: &str, reason: &str) -> bool;
}

pub struct AutoApprove;

#[async_trait::async_trait]
impl Approver for AutoApprove {
    async fn approve(&self, _tool: &str, _reason: &str) -> bool {
        true
    }
}

pub struct AutoDeny;

#[async_trait::async_trait]
impl Approver for AutoDeny {
    async fn approve(&self, _tool: &str, _reason: &str) -> bool {
        false
    }
}

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
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, session: SessionLog, config: AgentConfig) -> Self {
        Agent {
            provider,
            session: Mutex::new(session),
            config,
        }
    }

    pub async fn run(
        &mut self,
        prompt: &str,
        steering: &SteeringQueues,
        cancel: CancellationToken,
        events: mpsc::Sender<AgentEvent>,
    ) -> TurnOutcome {
        if let Err(e) = self
            .session
            .lock()
            .await
            .append_message(MessageRecord {
                message: Message::user_text(prompt),
                meta: None,
            })
            .map_err(|e| LlmError::Network(format!("session write failed: {e}")))
        {
            return TurnOutcome::Failed { error: e };
        }

        let mut turn = 0usize;
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

            let base_request = {
                let mut session = self.session.lock().await;
                let model = if self.config.model.is_empty() {
                    session
                        .header()
                        .map(|h| h.contract.model.clone())
                        .unwrap_or_default()
                } else {
                    self.config.model.clone()
                };
                // Long-horizon guard: estimate the projection; on overflow,
                // summarize older turns into a compaction entry and retry
                // the same contract. Still over afterwards, or no progress,
                // => fail closed. The session lock is NOT held across the
                // summarizer network call.
                let policy = &self.config.context_policy;
                let system = self.config.system_prompt.clone();
                let mut derived = session.derive_messages();
                let tool_defs = vak_tools::definitions(&self.config.tools);
                let mut est = context::estimate_tokens(&derived, Some(&system), &tool_defs);
                if est > policy.trigger_at() {
                    let Some(plan) = session.plan_compaction(policy.keep_recent) else {
                        return TurnOutcome::Failed {
                            error: LlmError::Network(
                                "context over budget but too short to compact".into(),
                            ),
                        };
                    };
                    let transcript = context::render_transcript(&plan.older);
                    let _ = events
                        .send(AgentEvent::ContextCompacting {
                            estimated_tokens: est,
                        })
                        .await;

                    let req = context::compaction_request(&model, &transcript);
                    let summary_msg = match self
                        .complete_with_reliability(&req, &cancel, &events, false)
                        .await
                    {
                        Ok(LlmAbortOr::Message(m)) => m,
                        Ok(LlmAbortOr::Aborted) => {
                            return TurnOutcome::Aborted { partial: None };
                        }
                        Err(e) => {
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

                    let tokens_before = est;
                    if let Err(e) = session.apply_compaction(&plan, summary, tokens_before) {
                        return TurnOutcome::Failed {
                            error: LlmError::Network(format!("compaction write failed: {e}")),
                        };
                    }
                    derived = session.derive_messages();
                    est = context::estimate_tokens(&derived, Some(&system), &tool_defs);
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
                        })
                        .await;
                    if est > policy.input_budget() {
                        return TurnOutcome::Failed {
                            error: LlmError::Network(format!(
                                "context still over budget after compaction (~{est} > {} tokens)",
                                policy.input_budget()
                            )),
                        };
                    }
                }
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

            // One model step = connect + stream + collect, wrapped with the
            // full reliability machinery (watchdog, retries+backoff,
            // circuit breaker). User aborts and partial-output aborts are
            // never retried; they propagate for caller handling.
            let response = match self
                .complete_with_reliability(&request, &cancel, &events, true)
                .await
            {
                Ok(LlmAbortOr::Message(r)) => r,
                Ok(LlmAbortOr::Aborted) => unreachable!("helper maps aborts to Err"),
                Err(LlmError::Aborted { partial }) => {
                    if let Some(p) = &partial {
                        self.append_assistant(p).await;
                    }
                    return TurnOutcome::Aborted { partial };
                }
                Err(e) => return TurnOutcome::Failed { error: e },
            };

            let usage = response.usage.clone();
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
                return TurnOutcome::Completed { response };
            }

            let calls = extract_tool_calls(&response);
            if calls.is_empty() {
                return TurnOutcome::Completed { response };
            }

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
    /// Retry-After), and circuit breaker. When `forward` is true, stream
    /// deltas are forwarded to `events`; otherwise they are drained.
    async fn complete_with_reliability(
        &self,
        request: &ChatRequest,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
        forward: bool,
    ) -> Result<LlmAbortOr, LlmError> {
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

            let outcome = match self.config.request_timeout {
                Some(t) => match tokio::time::timeout(t, step).await {
                    Ok(r) => r,
                    Err(_) => Err(LlmError::Network(format!(
                        "model step exceeded deadline of {}s",
                        t.as_secs()
                    ))),
                },
                None => step.await,
            };

            match outcome {
                Ok(r) => {
                    if let Some(breaker) = &self.config.circuit_breaker {
                        breaker.record_success();
                    }
                    return Ok(LlmAbortOr::Message(r));
                }
                Err(e @ LlmError::Aborted { .. }) => return Err(e),
                Err(e) if e.is_retryable() && attempt < self.config.max_retries => {
                    if let Some(breaker) = &self.config.circuit_breaker {
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
                }
                Err(e) => {
                    if let Some(breaker) = &self.config.circuit_breaker
                        && e.is_retryable()
                    {
                        breaker.record_failure();
                    }
                    return Err(e);
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
            authz.push(authorize(&self.config, call, &cwd).await);
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
            let _ = events
                .send(AgentEvent::ToolCallEnd {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    is_error: true,
                })
                .await;
            return (
                call.id,
                ToolRunOutput::Err(format!("blocked by hook: {reason}")),
            );
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

    let _ = events
        .send(AgentEvent::ToolCallEnd {
            id: call.id.clone(),
            name: call.name.clone(),
            is_error: matches!(output, ToolRunOutput::Err(_)),
        })
        .await;

    (call.id, output)
}

async fn authorize(
    config: &AgentConfig,
    call: &PendingToolCall,
    cwd: &std::path::Path,
) -> Result<(), String> {
    let Some(engine) = &config.permission else {
        return Ok(());
    };
    match engine.evaluate(&call.name, &call.input, config.mode, cwd) {
        Decision::Allow => Ok(()),
        Decision::Deny { reason } => Err(reason),
        Decision::Ask { reason } => match &config.approver {
            Some(a) if a.approve(&call.name, &reason).await => Ok(()),
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

enum LlmAbortOr {
    Message(vak_llm::AssistantMessage),
    #[allow(dead_code)]
    Aborted,
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
