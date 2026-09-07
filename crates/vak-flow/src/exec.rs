/// Run a node's done-contract: every entry executes as brokered bash and
/// must exit 0. First failure rejects the node with the failing check.
async fn verify_accept(
    node: &NodeDef,
    deps: &ExecutorDeps,
    cancel: &CancellationToken,
) -> Result<(), String> {
    if node.accept.is_empty() {
        return Ok(());
    }
    let tool = deps
        .tools
        .iter()
        .find(|t| t.name() == "bash")
        .ok_or_else(|| "accept requires bash tool".to_string())?;
    for check in &node.accept {
        let command = check.trim_start_matches("verify:").trim();
        let ctx = ToolContext {
            cwd: deps.cwd.clone(),
            cancel: cancel.child_token(),
            limits: Default::default(),
            sandbox: deps.sandbox.clone(),
            sandbox_sink: None,
        };
        let args = serde_json::json!({"command": command});
        authorize_flow_tool("bash", &args, deps).await?;
        let out = tool.execute(&args, &ctx).await;
        if out.is_error {
            return Err(format!(
                "done-contract failed: `{command}`\n{}",
                out.content
            ));
        }
        let _ = cancel.child_token();
    }
    Ok(())
}

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, ApprovalMode, Approver};
use vak_llm::Provider;
use vak_permission::{Decision, Mode, PermissionEngine};
use vak_session::{SessionLog, SessionPath};
use vak_tools::sandbox::Sandbox;
use vak_tools::{Tool, ToolContext};

use crate::parse::layers;
use crate::types::{FlowDef, FlowState, NodeDef, NodeResult, NodeStatus};

#[derive(Clone)]
pub struct ExecutorDeps {
    pub provider: Arc<dyn Provider>,
    pub system_prompt: String,
    pub model: String,
    pub tools: Vec<Arc<dyn Tool>>,
    pub read_only_tools: Vec<Arc<dyn Tool>>,
    pub max_turns: usize,
    pub outcome: Option<vak_intent::OutcomeSpec>,
    pub max_retries: u32,
    pub retry_base_backoff_ms: u64,
    pub request_timeout: Option<std::time::Duration>,
    pub circuit_breaker: Option<Arc<vak_agent::CircuitBreaker>>,
    pub run_retry_attempts: u32,
    pub run_retry_base_backoff_ms: u64,
    pub dispatch_ceiling: u32,
    pub spend_gate: Option<Arc<dyn vak_agent::SpendGate>>,
    pub permission: Option<Arc<PermissionEngine>>,
    pub mode: Mode,
    pub approval_mode: ApprovalMode,
    pub approver: Option<Arc<dyn Approver>>,
    pub sandbox: Option<Arc<dyn Sandbox>>,
    pub cwd: PathBuf,
    pub sessions_home: PathBuf,
    pub parent_session_id: String,
    /// Where the run-state ledger is persisted.
    pub state_path: PathBuf,
    pub work: Option<FlowWorkContext>,
}

#[derive(Clone)]
pub struct FlowWorkContext {
    pub session: Arc<tokio::sync::Mutex<SessionLog>>,
    pub contract_id: String,
    pub work_item_id: String,
}

#[derive(Debug)]
pub enum FlowOutcome {
    Completed {
        outputs: BTreeMap<String, String>,
    },
    Failed {
        node: String,
        reason: String,
        outputs: BTreeMap<String, String>,
    },
    Aborted,
}

pub struct Executor {
    deps: Arc<ExecutorDeps>,
}

impl Executor {
    pub fn new(deps: ExecutorDeps) -> Self {
        Executor {
            deps: Arc::new(deps),
        }
    }

    pub async fn run(
        &self,
        flow: &FlowDef,
        state: &mut FlowState,
        cancel: CancellationToken,
        events: tokio::sync::mpsc::Sender<String>,
    ) -> FlowOutcome {
        if state.outcome.is_none() {
            state.outcome = self.deps.outcome.clone();
        }
        let Ok(layer_list) = layers(flow) else {
            return FlowOutcome::Failed {
                node: "<flow>".into(),
                reason: "invalid graph".into(),
                outputs: BTreeMap::new(),
            };
        };

        for id in &layer_list.iter().flatten().cloned().collect::<Vec<_>>() {
            state.nodes.entry(id.clone()).or_insert(NodeResult {
                status: NodeStatus::Pending,
                output: String::new(),
            });
        }

        let by_id: BTreeMap<String, NodeDef> = flow
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();

        if let Err(reason) = self.start_work_item().await {
            return FlowOutcome::Failed {
                node: "<work>".into(),
                reason,
                outputs: BTreeMap::new(),
            };
        }

        for layer in &layer_list {
            let mut runnable = Vec::new();
            for id in layer {
                let status = state
                    .nodes
                    .get(id)
                    .map(|r| r.status)
                    .unwrap_or(NodeStatus::Pending);
                match status {
                    NodeStatus::Completed => continue,
                    NodeStatus::Failed | NodeStatus::Skipped => continue,
                    NodeStatus::Pending | NodeStatus::Running => {}
                }
                let Some(node) = by_id.get(id).cloned() else {
                    continue;
                };
                let blocked_dep = node.deps.iter().any(|d| {
                    matches!(
                        state.nodes.get(d).map(|r| r.status),
                        Some(NodeStatus::Failed) | Some(NodeStatus::Skipped)
                    )
                });
                if blocked_dep && node.r#type != "merge" {
                    state.nodes.insert(
                        id.clone(),
                        NodeResult {
                            status: NodeStatus::Skipped,
                            output: "upstream dependency failed or was skipped".into(),
                        },
                    );
                    let _ = events
                        .send(format!("⊘ skipped {id} (upstream failure)"))
                        .await;
                    continue;
                }
                runnable.push(node);
            }

            let mut join = tokio::task::JoinSet::new();
            for node in runnable {
                let deps = self.deps.clone();
                let cancel = cancel.clone();
                let events = events.clone();
                let dep_outputs: BTreeMap<String, String> = node
                    .deps
                    .iter()
                    .filter_map(|d| {
                        state
                            .nodes
                            .get(d)
                            .filter(|r| r.status == NodeStatus::Completed)
                            .map(|r| (d.clone(), r.output.clone()))
                    })
                    .collect();
                join.spawn(async move {
                    let id = node.id.clone();
                    let mut res = execute_node(&node, &dep_outputs, &deps, &cancel, &events).await;
                    if res.is_ok() && !node.accept.is_empty() {
                        let _ = events
                            .send(format!(
                                "⍗ accept {} ({} check(s))",
                                node.id,
                                node.accept.len()
                            ))
                            .await;
                        res = verify_accept(&node, &deps, &cancel)
                            .await
                            .map(|_| res.unwrap_or_default());
                    }
                    (id, res)
                });
            }

            while let Some(res) = join.join_next().await {
                let Ok((id, result)) = res else {
                    continue;
                };
                let (status, output) = match result {
                    Ok(text) => (NodeStatus::Completed, text),
                    Err(reason) => (NodeStatus::Failed, reason),
                };
                let failed = status == NodeStatus::Failed;
                state.nodes.insert(
                    id.clone(),
                    NodeResult {
                        status,
                        output: output.clone(),
                    },
                );
                let _ = events
                    .send(format!("{} {}", if failed { "✗" } else { "✓" }, id))
                    .await;
                self.persist(state);

                if status == NodeStatus::Completed
                    && let Err(reason) = self
                        .record_work_evidence(&id, &state.run_id, &flow.name)
                        .await
                {
                    let _ = self
                        .finish_work_item(vak_session::types::WorkItemStatus::Failed)
                        .await;
                    return FlowOutcome::Failed {
                        node: id,
                        reason,
                        outputs: collect_outputs(state),
                    };
                }

                if failed && by_id.get(&id).is_some_and(|n| n.required) {
                    // Mark every transitive dependent skipped so the ledger
                    // reflects why they never ran.
                    let mut stack: Vec<String> = vec![id.clone()];
                    while let Some(failed_id) = stack.pop() {
                        for n in flow.nodes.iter() {
                            if n.deps.contains(&failed_id)
                                && matches!(
                                    state.nodes.get(&n.id).map(|r| r.status),
                                    None | Some(NodeStatus::Pending)
                                )
                            {
                                state.nodes.insert(
                                    n.id.clone(),
                                    NodeResult {
                                        status: NodeStatus::Skipped,
                                        output: format!("upstream '{failed_id}' failed"),
                                    },
                                );
                                stack.push(n.id.clone());
                            }
                        }
                    }
                    self.persist(state);
                    let _ = self
                        .finish_work_item(vak_session::types::WorkItemStatus::Failed)
                        .await;
                    return FlowOutcome::Failed {
                        node: id,
                        reason: output,
                        outputs: collect_outputs(state),
                    };
                }
            }

            if cancel.is_cancelled() {
                let _ = self
                    .finish_work_item(vak_session::types::WorkItemStatus::Interrupted)
                    .await;
                return FlowOutcome::Aborted;
            }
        }

        if let Err(reason) = self
            .record_work_evidence("__flow_completed__", &state.run_id, &flow.name)
            .await
        {
            let _ = self
                .finish_work_item(vak_session::types::WorkItemStatus::Failed)
                .await;
            return FlowOutcome::Failed {
                node: "<work>".into(),
                reason,
                outputs: collect_outputs(state),
            };
        }
        if let Err(reason) = self
            .finish_work_item(vak_session::types::WorkItemStatus::ReadyForVerification)
            .await
        {
            return FlowOutcome::Failed {
                node: "<work>".into(),
                reason,
                outputs: collect_outputs(state),
            };
        }
        FlowOutcome::Completed {
            outputs: collect_outputs(state),
        }
    }

    fn persist(&self, state: &FlowState) {
        if let Ok(json) = serde_json::to_string_pretty(state) {
            if let Some(parent) = self.deps.state_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&self.deps.state_path, json);
        }
    }

    async fn record_work_evidence(
        &self,
        node_id: &str,
        run_id: &str,
        flow_name: &str,
    ) -> Result<(), String> {
        let Some(work) = &self.deps.work else {
            return Ok(());
        };
        let mut session = work.session.lock().await;
        let Some(projection) = session
            .work_projection()
            .map_err(|error| format!("flow work projection is invalid: {error}"))?
        else {
            return Err("flow work context has no contract".into());
        };
        if projection.contract.contract_id != work.contract_id {
            return Err("flow work context targets a different contract".into());
        }
        session
            .append_work(vak_session::types::WorkEvent {
                contract_id: work.contract_id.clone(),
                revision: projection.contract.revision,
                kind: vak_session::types::WorkEventKind::EvidenceAttached {
                    item_id: work.work_item_id.clone(),
                    evidence: vak_session::types::EvidenceRef::FlowNode {
                        flow: flow_name.into(),
                        run_id: run_id.into(),
                        node_id: node_id.into(),
                    },
                },
            })
            .map_err(|error| format!("flow evidence write failed: {error}"))?;
        Ok(())
    }

    async fn start_work_item(&self) -> Result<(), String> {
        let Some(work) = &self.deps.work else {
            return Ok(());
        };
        let mut session = work.session.lock().await;
        let Some(projection) = session.work_projection().map_err(|e| e.to_string())? else {
            return Err("flow work context has no contract".into());
        };
        let Some(state) = projection.items.get(&work.work_item_id) else {
            return Err("flow work context has no work item".into());
        };
        if state.status == vak_session::types::WorkItemStatus::Ready {
            session
                .append_work(vak_session::types::WorkEvent {
                    contract_id: work.contract_id.clone(),
                    revision: projection.contract.revision,
                    kind: vak_session::types::WorkEventKind::ItemStatusChanged {
                        item_id: work.work_item_id.clone(),
                        from: vak_session::types::WorkItemStatus::Ready,
                        to: vak_session::types::WorkItemStatus::Running,
                        attempt: state.attempt.saturating_add(1),
                        reason: "flow execution started".into(),
                    },
                })
                .map_err(|e| e.to_string())?;
        } else if state.status != vak_session::types::WorkItemStatus::Running {
            return Err(format!(
                "flow work item is {:?}, not ready or running",
                state.status
            ));
        }
        Ok(())
    }

    async fn finish_work_item(
        &self,
        status: vak_session::types::WorkItemStatus,
    ) -> Result<(), String> {
        let Some(work) = &self.deps.work else {
            return Ok(());
        };
        let mut session = work.session.lock().await;
        let Some(projection) = session.work_projection().map_err(|e| e.to_string())? else {
            return Err("flow work context has no contract".into());
        };
        let Some(state) = projection.items.get(&work.work_item_id) else {
            return Err("flow work context has no work item".into());
        };
        if state.status != vak_session::types::WorkItemStatus::Running {
            return Ok(());
        }
        session
            .append_work(vak_session::types::WorkEvent {
                contract_id: work.contract_id.clone(),
                revision: projection.contract.revision,
                kind: vak_session::types::WorkEventKind::ItemStatusChanged {
                    item_id: work.work_item_id.clone(),
                    from: vak_session::types::WorkItemStatus::Running,
                    to: status,
                    attempt: state.attempt,
                    reason: "flow execution returned".into(),
                },
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn collect_outputs(state: &FlowState) -> BTreeMap<String, String> {
    state
        .nodes
        .iter()
        .filter(|(_, r)| r.status == NodeStatus::Completed)
        .map(|(id, r)| (id.clone(), r.output.clone()))
        .collect()
}

fn render(template: &str, dep_outputs: &BTreeMap<String, String>) -> String {
    let mut out = template.to_string();
    for (id, output) in dep_outputs {
        out = out.replace(&format!("{{{{{id}}}}}"), output);
    }
    out
}

async fn execute_node(
    node: &NodeDef,
    dep_outputs: &BTreeMap<String, String>,
    deps: &ExecutorDeps,
    cancel: &CancellationToken,
    events: &tokio::sync::mpsc::Sender<String>,
) -> Result<String, String> {
    match node.r#type.as_str() {
        "bash" => {
            let command = render(node.command.as_deref().unwrap_or_default(), dep_outputs);
            let ctx = ToolContext {
                cwd: deps.cwd.clone(),
                cancel: cancel.child_token(),
                limits: Default::default(),
                sandbox: deps.sandbox.clone(),
                sandbox_sink: None,
            };
            let tool = deps
                .tools
                .iter()
                .find(|tool| tool.name() == "bash")
                .ok_or_else(|| "bash tool unavailable".to_string())?;
            let args = match node.timeout_ms {
                Some(t) => serde_json::json!({"command": command, "timeout_ms": t}),
                None => serde_json::json!({"command": command}),
            };
            authorize_flow_tool("bash", &args, deps).await?;
            let out = tool.execute(&args, &ctx).await;
            if out.is_error {
                Err(out.content)
            } else {
                Ok(out.content)
            }
        }
        "agent" => {
            let prompt = render(node.prompt.as_deref().unwrap_or_default(), dep_outputs);
            let readonly = node.readonly;
            let tools = if readonly {
                deps.read_only_tools.clone()
            } else {
                deps.tools.clone()
            };
            let mode = if readonly { Mode::ReadOnly } else { deps.mode };

            let session_id = format!(
                "flow-{}-{}",
                node.id,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            let header = vak_session::types::SessionHeader {
                session_id: session_id.clone(),
                created_at: chrono::Utc::now(),
                cwd: deps.cwd.clone(),
                parent_session_id: Some(deps.parent_session_id.clone()),
                contract_id: None,
                work_item_id: None,
                contract: vak_session::types::FrozenContract {
                    app_version: env!("CARGO_PKG_VERSION").into(),
                    provider: deps.provider.name().into(),
                    model: deps.model.clone(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: deps.system_prompt.clone(),
                    permission_mode: match mode {
                        Mode::ReadOnly => "read-only",
                        Mode::WorkspaceWrite => "workspace-write",
                        Mode::FullAccess => "full-access",
                    }
                    .into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            };
            let path = SessionPath::new_session_file(&deps.sessions_home, &deps.cwd, &session_id);
            let log = SessionLog::create(path, header)
                .map_err(|e| format!("cannot create node session: {e}"))?;

            let mut cfg = AgentConfig::new(deps.system_prompt.clone());
            cfg.outcome = deps.outcome.clone();
            cfg.max_retries = deps.max_retries;
            cfg.retry_base_backoff_ms = deps.retry_base_backoff_ms;
            cfg.request_timeout = deps.request_timeout;
            cfg.circuit_breaker = deps.circuit_breaker.clone();
            cfg.run_retry_attempts = deps.run_retry_attempts;
            cfg.run_retry_base_backoff_ms = deps.run_retry_base_backoff_ms;
            cfg.dispatch_ceiling = deps.dispatch_ceiling;
            cfg.spend_gate = deps.spend_gate.clone();
            cfg.model = deps.model.clone();
            cfg.tools = tools;
            cfg.max_turns = deps
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.max_turns)
                .map_or(deps.max_turns, |cap| deps.max_turns.min(cap));
            cfg.permission = deps.permission.clone();
            cfg.mode = mode;
            cfg.approval_mode = deps.approval_mode;
            cfg.approver = deps.approver.clone();
            cfg.sandbox = deps.sandbox.clone();

            let mut agent = Agent::new(deps.provider.clone(), log, cfg);
            let steering = vak_agent::SteeringQueues::new();
            let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<vak_agent::AgentEvent>(256);
            let pump = tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
            let outcome = agent
                .run(&prompt, &steering, cancel.child_token(), ev_tx)
                .await;
            let _ = pump.await;

            match outcome {
                vak_agent::TurnOutcome::Completed { response } => {
                    let text = response.text_content();
                    if text.is_empty() {
                        Ok(format!("(node '{}' completed without output)", node.id))
                    } else {
                        Ok(text)
                    }
                }
                vak_agent::TurnOutcome::Aborted { partial } => Err(format!(
                    "agent node aborted. Partial:\n{}",
                    partial.map(|p| p.text_content()).unwrap_or_default()
                )),
                vak_agent::TurnOutcome::Failed { error } => Err(error.to_string()),
                vak_agent::TurnOutcome::MaxTurnsReached => {
                    Err("agent node hit its turn limit".into())
                }
            }
        }
        "approval" => {
            let message = render(node.message.as_deref().unwrap_or_default(), dep_outputs);
            let _ = events.send(format!("⏸ approval needed: {message}")).await;
            match &deps.approver {
                Some(a) => {
                    if a.approve("approval", "", &message).await {
                        Ok("approved".into())
                    } else {
                        Err("denied by user".into())
                    }
                }
                None => Err("no approver available for approval node".into()),
            }
        }
        "merge" => {
            let mut report = String::from("<merge-report>\n");
            for dep in &node.deps {
                match dep_outputs.get(dep) {
                    Some(out) => report.push_str(&format!("[{dep}] ok\n{out}\n")),
                    None => report.push_str(&format!(
                        "[{dep}] unavailable (failed or skipped upstream)\n"
                    )),
                }
            }
            report.push_str("</merge-report>");
            Ok(report)
        }
        other => Err(format!("unknown node type '{other}'")),
    }
}

async fn authorize_flow_tool(
    tool: &str,
    args: &serde_json::Value,
    deps: &ExecutorDeps,
) -> Result<(), String> {
    let Some(engine) = &deps.permission else {
        return Ok(());
    };
    match engine.evaluate(tool, args, deps.mode, &deps.cwd) {
        Decision::Allow => Ok(()),
        Decision::Deny { reason } => Err(reason),
        Decision::Ask { reason, source } => {
            if vak_agent::auto_approve(
                deps.approval_mode,
                source,
                tool,
                args,
                deps.mode,
                deps.sandbox.is_some(),
                &deps.cwd,
            ) {
                return Ok(());
            }
            match &deps.approver {
                Some(approver) if approver.approve(tool, &args.to_string(), &reason).await => {
                    Ok(())
                }
                Some(_) => Err(format!("denied by user: {reason}")),
                None => Err(format!("{reason} (no approver available)")),
            }
        }
    }
}
