//! `WorkingSetPlanner` verification harness (docs/design/68-context-engine.md
//! "Verification", "Probe harness" and "No-cut invariant"): a fixture ledger
//! of 30 closed turns with mixed prose/tool/card answers, plus one open
//! turn, planned against a synthetic 13k-token horizon. Zero model calls —
//! this is deterministic, structural verification of the planner and the
//! projection it drives, not of what a summarizer would choose to keep.

use std::path::Path;

use vak_agent::capacity::{CacheBehaviour, CapacityProfile, Horizon, ProbeProvenance};
use vak_agent::planner::{self, Fidelity, PlanInput};
use vak_llm::{ContentBlock, Message};
use vak_session::types::{
    FrozenContract, MessageRecord, PresentationRecord, PresentationSource, SessionHeader,
    TurnCardRecord,
};
use vak_session::{SessionLog, SessionPath, TurnIndex, WorkingSetPlan};

use crate::context_gate::{ContextScorecard, MetricDirection, QualityMetric};

/// The synthetic horizon this harness plans against (docs/design/68 §"Probe
/// harness"): small enough that a 30-turn fixture cannot all fit at `Full`,
/// so the planner's Card/Packet tiers are actually exercised.
const HORIZON_TOKENS: u64 = 13_000;

fn header(cwd: &Path, session_id: &str) -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: session_id.into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "fixture".into(),
            model: "fixture-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn user_text(text: impl Into<String>) -> MessageRecord {
    MessageRecord {
        message: Message::user_text(text),
        meta: None,
    }
}

fn assistant_text(text: impl Into<String>) -> MessageRecord {
    MessageRecord {
        message: Message::assistant(vec![ContentBlock::text(text.into())]),
        meta: None,
    }
}

fn assistant_tool_call(id: &str, name: &str, input: serde_json::Value) -> MessageRecord {
    MessageRecord {
        message: Message::assistant(vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }]),
        meta: None,
    }
}

fn tool_result(id: &str, content: impl Into<String>) -> MessageRecord {
    MessageRecord {
        message: Message {
            role: vak_llm::Role::User,
            content: vec![ContentBlock::tool_result(id, content.into())],
        },
        meta: None,
    }
}

/// The three answer shapes mixed across the fixture's 30 closed turns.
enum TurnKind {
    /// Plain Q&A, no tool calls at all.
    Prose,
    /// One evidence-producing tool call, then a prose answer that cites it.
    Tool,
    /// One evidence-producing tool call, then an emitted card.
    Card,
}

/// Builds one closed turn of the given kind and returns its directive
/// entry id, closing it with a real `TurnCard` (as the turn-close hook
/// would) so the planner has real `tokens_full`/`tokens_card` to plan
/// against.
fn build_closed_turn(log: &mut SessionLog, i: usize, kind: TurnKind) -> Result<String, String> {
    let turn_id = log
        .append_message(user_text(format!(
            "turn {i} directive: please help with task number {i}"
        )))
        .map_err(|e| e.to_string())?
        .id;
    let narration = match kind {
        TurnKind::Prose => {
            let text = format!("turn {i} prose answer with a bit of explanation");
            log.append_message(assistant_text(text.clone()))
                .map_err(|e| e.to_string())?;
            text
        }
        TurnKind::Tool => {
            let call_id = format!("call-{i}");
            log.append_message(assistant_tool_call(
                &call_id,
                "search",
                serde_json::json!({"query": format!("topic {i}")}),
            ))
            .map_err(|e| e.to_string())?;
            log.append_message(tool_result(
                &call_id,
                format!(r#"[{{"title":"Result {i}","url":"https://example.com/{i}"}}]"#),
            ))
            .map_err(|e| e.to_string())?;
            let text = format!("turn {i} answer citing the search result");
            log.append_message(assistant_text(text.clone()))
                .map_err(|e| e.to_string())?;
            text
        }
        TurnKind::Card => {
            let call_id = format!("call-{i}");
            log.append_message(assistant_tool_call(
                &call_id,
                "search",
                serde_json::json!({"query": format!("topic {i}")}),
            ))
            .map_err(|e| e.to_string())?;
            log.append_message(tool_result(
                &call_id,
                format!(r#"[{{"title":"Result {i}","url":"https://example.com/{i}"}}]"#),
            ))
            .map_err(|e| e.to_string())?;
            let card_id = format!("card-{i}");
            log.append_message(assistant_tool_call(
                &card_id,
                "emit_research_card",
                serde_json::json!({"semantic_type": "research.synthesis"}),
            ))
            .map_err(|e| e.to_string())?;
            log.append_message(tool_result(
                &card_id,
                format!(r#"{{"presentation":"pres-{i}","ok":true}}"#),
            ))
            .map_err(|e| e.to_string())?;
            let text = format!("turn {i} narration around the card");
            log.append_message(assistant_text(text.clone()))
                .map_err(|e| e.to_string())?;
            log.append_presentation(PresentationRecord {
                turn_id: turn_id.clone(),
                source: PresentationSource::ToolCall {
                    tool_use_id: card_id,
                },
                semantic_type: "research.synthesis".into(),
                skill_id: "skill".into(),
                skill_version: "1".into(),
                schema_version: 1,
                payload: serde_json::json!({"takeaways": [format!("finding {i}")]}),
                payload_digest: format!("digest-{i}"),
                derived_from: vec![call_id],
                title: format!("Card {i}"),
                identity_digest: format!("Card {i}: finding {i}"),
            })
            .map_err(|e| e.to_string())?;
            text
        }
    };
    let card = TurnIndex::from_log(log)
        .turn_by_id(&turn_id)
        .ok_or("turn missing from index right after being written")?
        .build_card("completed", narration, &|s| s.len() as u64 / 4);
    log.append_turn_card(TurnCardRecord {
        turn_id: turn_id.clone(),
        card,
    })
    .map_err(|e| e.to_string())?;
    Ok(turn_id)
}

/// Builds the 30-closed-turn-plus-one-open fixture and returns the log plus
/// the open turn's tool-result content (for the no-cut assertion).
fn build_fixture(dir: &Path, session_id: &str) -> Result<(SessionLog, String), String> {
    let cwd = dir.to_path_buf();
    let home = cwd.join(".vak-home");
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let mut log = SessionLog::create(
        SessionPath::new_session_file(&home, &cwd, session_id),
        header(&cwd, session_id),
    )
    .map_err(|e| e.to_string())?;

    for i in 0..30 {
        let kind = match i % 3 {
            0 => TurnKind::Prose,
            1 => TurnKind::Tool,
            _ => TurnKind::Card,
        };
        build_closed_turn(&mut log, i, kind)?;
    }

    // The still-open 31st turn: a directive plus one dispatched tool call
    // and its result, no final answer yet — always verbatim, never planned.
    log.append_message(user_text("open turn: please check the current status"))
        .map_err(|e| e.to_string())?;
    log.append_message(assistant_tool_call(
        "open-call",
        "bash",
        serde_json::json!({"command": "echo status"}),
    ))
    .map_err(|e| e.to_string())?;
    let open_tool_result_content = "status: all clear, exit code: 0".to_string();
    log.append_message(tool_result("open-call", open_tool_result_content.clone()))
        .map_err(|e| e.to_string())?;

    Ok((log, open_tool_result_content))
}

fn synthetic_profile() -> CapacityProfile {
    CapacityProfile::from_probe(
        HORIZON_TOKENS,
        None,
        Horizon {
            tokens: HORIZON_TOKENS,
            confidence: 0.9,
            last_confirmed: std::time::SystemTime::now(),
        },
        CacheBehaviour::Unknown,
        512,
        ProbeProvenance {
            probed_at: std::time::SystemTime::now(),
            rungs: Vec::new(),
            signals: Vec::new(),
            metadata_digest: "context-engine-gate".into(),
        },
    )
}

/// Plans once against the fixture's current chain state, mirroring what
/// `Agent::build_working_set_plan` does per step.
fn plan_now(log: &SessionLog, profile: &CapacityProfile) -> WorkingSetPlan {
    let index = TurnIndex::from_log(log);
    let directive = index
        .turns
        .last()
        .map(|t| t.directive.text_content())
        .unwrap_or_default();
    let reading = log.latest_reading();
    let current_turn_tokens = profile.estimate_tokens(
        log.open_turn_verbatim()
            .iter()
            .map(|m| m.text_content().len() as u64)
            .sum(),
    );
    planner::plan(PlanInput {
        profile,
        index: &index,
        directive: &directive,
        reading: reading.as_ref(),
        prefix_tokens: 400,
        tail_tokens: 100,
        current_turn_tokens,
    })
}

/// Every non-open turn with a card must appear in EXACTLY one place: either
/// `per_turn` (Full or Card) or inside `packet_range` — never both, never
/// neither ("never splits a turn").
fn every_turn_accounted_exactly_once(index: &TurnIndex, plan: &WorkingSetPlan) -> bool {
    let closed_carded: Vec<&str> = index
        .turns
        .iter()
        .filter(|t| t.closed && t.card.is_some())
        .map(|t| t.id.as_str())
        .collect();
    let position: std::collections::HashMap<&str, usize> = index
        .turns
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    let packet_bounds = plan.packet_range.as_ref().map(|(first, last)| {
        (
            position.get(first.as_str()).copied().unwrap_or(usize::MAX),
            position.get(last.as_str()).copied().unwrap_or(0),
        )
    });
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (id, _) in &plan.per_turn {
        if !seen.insert(id.as_str()) {
            return false; // duplicate: split/double-counted turn
        }
    }
    closed_carded.iter().all(|id| {
        let in_per_turn = seen.contains(id);
        let in_packet = packet_bounds.is_some_and(|(lo, hi)| {
            let pos = position[id];
            pos >= lo && pos <= hi
        });
        in_per_turn ^ in_packet
    })
}

/// The no-cut invariant (docs/design/68 "No-cut invariant"): every tool
/// result in the ledger is either verbatim (the open turn), a digest
/// carrying its evidence id in its paired `tool_result` (a `Full` turn), or
/// named in a card/packet — never silently absent and never a raw replay.
fn no_cut_invariant_holds(log: &SessionLog, index: &TurnIndex, plan: &WorkingSetPlan) -> bool {
    let messages = log.derive_with_plan(plan);
    let joined: String = messages
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");

    // `text_content()` strips non-Text blocks, so the verbatim check for a
    // tool result (a ToolResult block, never Text) has to scan the derived
    // messages' raw content directly rather than through `joined`.
    let derived_tool_results: Vec<&str> = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    let open_result_verbatim = log
        .open_turn_verbatim()
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .all(|content| derived_tool_results.contains(&content));
    if !open_result_verbatim {
        return false;
    }

    let fidelity_of: std::collections::HashMap<&str, Fidelity> = plan
        .per_turn
        .iter()
        .map(|(id, f)| (id.as_str(), *f))
        .collect();

    for turn in index.turns.iter().filter(|t| t.closed && t.card.is_some()) {
        let card = turn.card.as_ref().unwrap_or_else(|| unreachable!());
        match fidelity_of.get(turn.id.as_str()) {
            Some(Fidelity::Full) => {
                // The pair stays; the result is the digest tagged with the
                // evidence id, never the raw content of a closed turn.
                for trace in &card.did {
                    let tag = format!("[evidence:{}", trace.evidence_id);
                    let digested = derived_tool_results
                        .iter()
                        .any(|content| content.contains(&tag));
                    let raw = turn
                        .evidence_for(&trace.evidence_id)
                        .map(|evidence| evidence.content);
                    let leaked_raw = raw
                        .as_deref()
                        .map(|raw| derived_tool_results.contains(&raw))
                        .unwrap_or(false);
                    if !digested || leaked_raw {
                        return false;
                    }
                }
            }
            Some(Fidelity::Card) => {
                let line = card.line(0);
                for trace in &card.did {
                    if !line.contains(&trace.evidence_id) {
                        return false;
                    }
                }
            }
            Some(Fidelity::Packet) | None => {
                if joined.contains(&turn.directive.text_content()) {
                    return false; // leaked raw despite being packeted
                }
            }
        }
    }
    true
}

/// Two consecutive steps within the same (still open) turn must share a
/// prefix digest and step k+1's messages must start with step k's, verbatim
/// (append-only, docs/design/68 §6/§10).
fn steps_are_append_only_with_a_stable_prefix(dir: &Path) -> Result<bool, String> {
    let (mut log, _open_result) = build_fixture(dir, "gate-append-only")?;
    let profile = synthetic_profile();
    let system_prefix = "You are vak. Fixed system prompt.";
    let tools: Vec<vak_llm::ToolDefinition> = vec![vak_llm::ToolDefinition::new(
        "bash",
        "Run a shell command.",
        serde_json::json!({"type": "object"}),
    )];

    let plan_k = plan_now(&log, &profile);
    let messages_k = log.derive_with_plan(&plan_k);
    let digest_k = vak_agent::context::prefix_digest(system_prefix, &tools);

    log.append_message(assistant_tool_call(
        "open-call-2",
        "bash",
        serde_json::json!({"command": "echo more"}),
    ))
    .map_err(|e| e.to_string())?;
    log.append_message(tool_result("open-call-2", "more: ok"))
        .map_err(|e| e.to_string())?;

    let plan_k1 = plan_now(&log, &profile);
    let messages_k1 = log.derive_with_plan(&plan_k1);
    let digest_k1 = vak_agent::context::prefix_digest(system_prefix, &tools);

    if digest_k != digest_k1 {
        return Ok(false);
    }
    if messages_k1.len() < messages_k.len() {
        return Ok(false);
    }
    Ok(messages_k
        .iter()
        .zip(messages_k1.iter())
        .all(|(a, b)| a.text_content() == b.text_content()))
}

/// Runs the planner-verification gate. Zero model calls: every property is
/// structural, over a fixed 30-turn-plus-open fixture and a synthetic
/// 13k-token profile (docs/design/68-context-engine.md "Verification").
pub fn run_context_engine_scorecard() -> Result<ContextScorecard, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let (log, _open_result) = build_fixture(dir.path(), "gate-budget")?;
    let profile = synthetic_profile();
    let plan = plan_now(&log, &profile);
    let index = TurnIndex::from_log(&log);

    let budget_ok = plan.spent <= plan.budget;
    let accounted_ok = every_turn_accounted_exactly_once(&index, &plan);
    let no_cut_ok = no_cut_invariant_holds(&log, &index, &plan);
    let append_only_ok = steps_are_append_only_with_a_stable_prefix(dir.path())?;

    let metric = |name: &'static str, ok: bool| QualityMetric {
        name,
        value: if ok { 1.0 } else { 0.0 },
        threshold: 1.0,
        direction: MetricDirection::Min,
    };

    Ok(ContextScorecard {
        metrics: vec![
            metric("plan_never_exceeds_budget", budget_ok),
            metric("every_turn_accounted_exactly_once", accounted_ok),
            metric("no_cut_invariant_holds", no_cut_ok),
            metric("steps_append_only_with_stable_prefix", append_only_ok),
        ],
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn context_engine_scorecard_passes_on_healthy_machinery() {
        let card = run_context_engine_scorecard().expect("harness");
        println!("{card}");
        assert!(card.passed(), "scorecard failed: {card}");
    }
}
