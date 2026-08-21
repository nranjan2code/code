//! Dynamic planning for open-ended tasks: a planner model authors a
//! candidate flow DAG (same schema as static flows), which is validated
//! before execution. Invalid plans fail closed as `planning_failed` — no
//! fallback plan is ever substituted. Execution failure grants exactly one
//! replan, seeded with settled node outputs and the failure reason.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use vak_llm::{ChatRequest, Message};

use crate::exec::{Executor, ExecutorDeps, FlowOutcome};
use crate::parse::parse_flow;
use crate::types::FlowState;

pub const PLANNER_SYSTEM: &str = "\
You are a task planner. Convert the user's task into a small workflow DAG in TOML.

Output EXACTLY one fenced toml block and nothing else. Schema:

[flow]
name = \"short-name\"

[[nodes]]
id = \"unique_id\"
type = \"bash\" | \"agent\" | \"approval\" | \"merge\"
command = \"...\"        # bash only
prompt = \"...\"         # agent only; may reference {{other_node_id}} outputs
message = \"...\"        # approval only
deps = [\"other_id\"]     # optional; {{refs}} imply deps automatically
required = true          # optional, default true
readonly = false         # agent only: read-only exploration child

Rules:
- Prefer 2-5 nodes. Every node must be reachable and useful.
- Use bash for deterministic commands (build/test/inspect). Use agent nodes
  only for work needing judgment.
- End with a merge node collecting the final results.
- Never invent node types or fields.";

#[derive(Debug, Clone)]
pub struct ToolCatalogEntry {
    pub name: String,
    pub description: String,
}

pub fn build_planner_prompt(task: &str, catalog: &[ToolCatalogEntry]) -> String {
    let mut p = String::from("Available execution tools:\n");
    for e in catalog {
        let desc: String = e.description.chars().take(100).collect();
        p.push_str(&format!("- {}: {desc}\n", e.name));
    }
    p.push_str(&format!(
        "\nTask:\n{task}\n\nProduce the TOML workflow now."
    ));
    p
}

/// Extracts the candidate TOML from a planner response: fenced ```toml
/// first, then any fence, then a raw `[flow]` document heuristic.
pub fn extract_toml(response: &str) -> Option<String> {
    if let Some(start) = response.find("```toml") {
        let after = &response[start + 7..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    if let Some(start) = response.find("```") {
        let after = &response[start + 3..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    let trimmed = response.trim();
    if trimmed.starts_with("[flow]") {
        return Some(trimmed.to_string());
    }
    None
}

#[derive(Debug)]
pub enum PlanOutcome {
    Completed {
        outputs: std::collections::BTreeMap<String, String>,
        attempts: usize,
    },
    /// The planner could not produce a valid plan. Fail closed.
    PlanningFailed {
        reason: String,
    },
    /// Plans were valid but execution failed (replan budget exhausted).
    Failed {
        node: String,
        reason: String,
    },
    Aborted,
}

const MAX_ATTEMPTS: usize = 2;

async fn complete_text(
    deps: &ExecutorDeps,
    system: &str,
    prompt: &str,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let request = ChatRequest {
        model: deps.model.clone(),
        system: Some(system.to_string()),
        messages: vec![Message::user_text(prompt)],
        tools: Vec::new(),
        max_tokens: 4096,
        temperature: None,
    };
    let stream = deps
        .provider
        .stream(request, cancel.clone())
        .await
        .map_err(|e| e.to_string())?;
    let response = stream.result().await.map_err(|e| e.to_string())?;
    Ok(response.text_content())
}

fn seed_with_settled(task: &str, state: &FlowState, failed_node: &str, reason: &str) -> String {
    let mut s = format!("{task}\n\n## Previous attempt context\n");
    s.push_str(&format!(
        "The previous plan failed at node '{failed_node}': {reason}\n"
    ));
    s.push_str("Settled node outputs you may reuse (do not redo this work):\n");
    for (id, r) in &state.nodes {
        if r.status == crate::types::NodeStatus::Completed {
            let out: String = r.output.chars().take(400).collect();
            s.push_str(&format!("- {id}: {out}\n"));
        } else {
            s.push_str(&format!(
                "- {id}: {:?} — do not repeat this node\n",
                r.status
            ));
        }
    }
    s
}

pub async fn plan_and_run(
    deps: Arc<ExecutorDeps>,
    task: &str,
    cancel: CancellationToken,
    events: tokio::sync::mpsc::Sender<String>,
) -> PlanOutcome {
    let catalog: Vec<ToolCatalogEntry> = deps
        .tools
        .iter()
        .map(|t| ToolCatalogEntry {
            name: t.name().to_string(),
            description: t.description().to_string(),
        })
        .collect();

    let mut current_task = task.to_string();
    let mut last_failure: Option<(String, String)> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        if cancel.is_cancelled() {
            return PlanOutcome::Aborted;
        }

        let prompt = build_planner_prompt(&current_task, &catalog);
        let _ = events
            .send(format!("◌ planning (attempt {attempt})…"))
            .await;
        let response = match complete_text(&deps, PLANNER_SYSTEM, &prompt, &cancel).await {
            Ok(r) => r,
            Err(e) => {
                return PlanOutcome::PlanningFailed {
                    reason: format!("planner call failed: {e}"),
                };
            }
        };

        let Some(toml_str) = extract_toml(&response) else {
            return PlanOutcome::PlanningFailed {
                reason: format!(
                    "planner returned no TOML plan. Response head: {}",
                    response.chars().take(200).collect::<String>()
                ),
            };
        };

        let flow = match parse_flow(&toml_str) {
            Ok(f) => f,
            Err(e) => {
                return PlanOutcome::PlanningFailed {
                    reason: format!("planned DAG invalid: {e}"),
                };
            }
        };
        let _ = events
            .send(format!(
                "▸ plan accepted: {} ({} nodes)",
                flow.name,
                flow.nodes.len()
            ))
            .await;

        // Fresh ledger per attempt; definition frozen from the planner output.
        let run_id = format!(
            "plan-{}-{attempt}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let state_path = deps
            .sessions_home
            .join("flow-runs")
            .join(format!("{}.json", run_id));
        let mut state = FlowState {
            run_id,
            flow_name: flow.name.clone(),
            definition_toml: toml_str.clone(),
            started_at: chrono::Utc::now(),
            nodes: Default::default(),
        };

        let mut attempt_deps = (*deps).clone();
        attempt_deps.state_path = state_path.clone();
        let executor = Executor::new(attempt_deps);
        let outcome = executor
            .run(&flow, &mut state, cancel.clone(), events.clone())
            .await;

        match outcome {
            FlowOutcome::Completed { outputs } => {
                return PlanOutcome::Completed {
                    outputs,
                    attempts: attempt,
                };
            }
            FlowOutcome::Aborted => return PlanOutcome::Aborted,
            FlowOutcome::Failed { node, reason, .. } => {
                if attempt < MAX_ATTEMPTS {
                    let _ = events
                        .send(format!(
                            "✗ '{node}' failed — replanning once with settled context"
                        ))
                        .await;
                    last_failure = Some((node.clone(), reason.clone()));
                    current_task = seed_with_settled(task, &state, &node, &reason);
                }
            }
        }
    }

    match last_failure {
        Some((node, reason)) => PlanOutcome::Failed { node, reason },
        None => PlanOutcome::PlanningFailed {
            reason: "planning exhausted".into(),
        },
    }
}
