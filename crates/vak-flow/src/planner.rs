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
- All strings are single-line TOML basic strings: write newlines as \\n and
  escape embedded double quotes as \\\" .
- Never invent node types or fields.";

#[derive(Debug, Clone)]
pub struct ToolCatalogEntry {
    pub name: String,
    pub description: String,
}

pub fn build_planner_prompt(task: &str, catalog: &[ToolCatalogEntry]) -> String {
    let mut p = String::from("Available execution tools:\n");
    for e in catalog {
        p.push_str(&format!("- {}: {}\n", e.name, e.description));
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

/// Structural repair for the two invalidities models actually emit:
/// raw newlines and nested raw double quotes inside single-line TOML basic
/// strings (shell commands like `[ "$(grep …)" ]` are written verbatim).
/// A `"` is treated as the closing quote only when followed (after optional
/// whitespace) by valid TOML continuation — newline, `,`, `]`, `}`, `#`, or
/// EOF; anything else is escaped as `\"`. Raw newlines become `\n`.
/// Multi-line (`"""`/`'''`) strings, literal strings, and comments pass
/// through untouched; anything still invalid after this fails closed.
pub fn sanitize_basic_string_newlines(doc: &str) -> String {
    #[derive(PartialEq)]
    enum St {
        Normal,
        Basic,
        Literal,
        MultiBasic,
        MultiLiteral,
    }
    let mut out = String::with_capacity(doc.len());
    let mut chars = doc.chars().peekable();
    let mut st = St::Normal;
    while let Some(c) = chars.next() {
        match st {
            St::Normal => match c {
                '#' => {
                    out.push(c);
                    while let Some(n) = chars.next_if(|&n| n != '\n') {
                        out.push(n);
                    }
                }
                '"' | '\'' => {
                    let q = c;
                    if chars.peek() == Some(&q) {
                        chars.next();
                        if chars.peek() == Some(&q) {
                            chars.next();
                            out.push(q);
                            out.push(q);
                            out.push(q);
                            st = if q == '"' {
                                St::MultiBasic
                            } else {
                                St::MultiLiteral
                            };
                        } else {
                            out.push(q);
                            out.push(q);
                        }
                    } else {
                        out.push(q);
                        st = if q == '"' { St::Basic } else { St::Literal };
                    }
                }
                _ => out.push(c),
            },
            St::Basic => match c {
                '\\' => {
                    out.push(c);
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                }
                '\n' => out.push_str("\\n"),
                '\r' if chars.peek() == Some(&'\n') => {
                    chars.next();
                    out.push_str("\\n");
                }
                '"' => {
                    if closes_basic_string(&mut chars) {
                        out.push('"');
                        st = St::Normal;
                    } else {
                        out.push_str("\\\"");
                    }
                }
                _ => out.push(c),
            },
            St::Literal => {
                out.push(c);
                if c == '\'' {
                    st = St::Normal;
                }
            }
            St::MultiBasic => {
                out.push(c);
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                } else if c == '"' && chars.peek() == Some(&'"') {
                    chars.next();
                    out.push('"');
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        out.push('"');
                        st = St::Normal;
                    }
                }
            }
            St::MultiLiteral => {
                out.push(c);
                if c == '\'' && chars.peek() == Some(&'\'') {
                    chars.next();
                    out.push('\'');
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                        out.push('\'');
                        st = St::Normal;
                    }
                }
            }
        }
    }
    out
}

/// A `"` closes a basic string only if the next non-whitespace char is valid
/// TOML continuation (newline, `,`, `]`, `}`, `#`) or the document ends.
fn closes_basic_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    let mut lookahead = chars.clone();
    loop {
        match lookahead.next() {
            None => return true,
            Some('\n') | Some('\r') | Some(',') | Some(']') | Some('}') | Some('#') => {
                return true;
            }
            Some(' ') | Some('\t') => continue,
            Some(_) => return false,
        }
    }
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

/// Bounded retries for the planner's own model calls. The planner bypasses
/// the agent loop, so without this a single transient provider failure
/// (rate limit, overload, network drop) would fail-closed the whole run.
const PLANNER_CALL_ATTEMPTS: usize = 3;
const PLANNER_RETRY_BACKOFF_MS: u64 = 500;

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
        cache: None,
        previous_response_id: None,
        // A TOML DAG, not a deliberation: measured live, thinking made no
        // difference to whether the plan parsed and cost up to 5x the
        // latency.
        think: Some(false),
    };
    let mut backoff_ms = PLANNER_RETRY_BACKOFF_MS;
    for attempt in 1..=PLANNER_CALL_ATTEMPTS {
        let last = attempt == PLANNER_CALL_ATTEMPTS;
        match deps.provider.stream(request.clone(), cancel.clone()).await {
            Ok(stream) => match stream.result().await {
                Ok(response) => return Ok(response.text_content()),
                Err(e) if !last && e.is_retryable() => {
                    if !sleep_backoff(e.retry_after_secs(), backoff_ms, cancel).await {
                        return Err("cancelled".into());
                    }
                }
                Err(e) => return Err(e.to_string()),
            },
            Err(e) if !last && e.is_retryable() => {
                if !sleep_backoff(e.retry_after_secs(), backoff_ms, cancel).await {
                    return Err("cancelled".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
        backoff_ms = backoff_ms.saturating_mul(2);
    }
    Err("planner call exhausted retries".into())
}

/// Waits `secs` (or the exponential default); returns false if cancelled.
async fn sleep_backoff(
    retry_after_secs: Option<u64>,
    default_ms: u64,
    cancel: &CancellationToken,
) -> bool {
    let d = std::time::Duration::from_millis(
        retry_after_secs.map_or(default_ms, |s| s.saturating_mul(1000)),
    );
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(d) => true,
    }
}

fn seed_with_settled(task: &str, state: &FlowState, failed_node: &str, reason: &str) -> String {
    let mut s = format!("{task}\n\n## Previous attempt context\n");
    s.push_str(&format!(
        "The previous plan failed at node '{failed_node}': {reason}\n"
    ));
    s.push_str("Settled node outputs you may reuse (do not redo this work):\n");
    for (id, r) in &state.nodes {
        if r.status == crate::types::NodeStatus::Completed {
            s.push_str(&format!("- {id}: {}\n", r.output));
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

        let Some(raw_toml) = extract_toml(&response) else {
            return PlanOutcome::PlanningFailed {
                reason: format!(
                    "planner returned no TOML plan. Response head: {}",
                    response.chars().take(200).collect::<String>()
                ),
            };
        };
        let toml_str = sanitize_basic_string_newlines(&raw_toml);

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
            outcome: None,
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
