//! `WorkingSetPlanner` verification harness (docs/design/68-context-engine.md
//! "Verification", "Probe harness" and "No-cut invariant"): a fixture ledger
//! of 30 closed turns with mixed prose/tool/card answers, plus one open
//! turn, planned against a synthetic 13k-token horizon. Zero model calls —
//! this is deterministic, structural verification of the planner and the
//! projection it drives, not of what a summarizer would choose to keep.

use std::path::Path;

use vak_context::capacity::{CacheBehaviour, CapacityProfile, Horizon, ProbeProvenance};
use vak_context::planner::{self, Fidelity};
use vak_llm::{ContentBlock, Message};
use vak_session::types::{
    FrozenContract, MessageRecord, PresentationRecord, PresentationSource, SessionHeader,
    TurnCardRecord,
};
use vak_session::{SessionLog, SessionPath, TurnIndex, WorkingSetPlan};

use crate::scorecard::{ContextScorecard, MetricDirection, QualityMetric};

/// The synthetic horizon this harness plans against (docs/design/68 §"Probe
/// harness"): small enough that a 30-turn fixture cannot all fit at `Full`,
/// so the planner's Card/Packet tiers are actually exercised.
const HORIZON_TOKENS: u64 = 13_000;

/// The "small model" horizon for the packet replay: too tight for the
/// 30-turn fixture's cards, so a stored packet would show here if the
/// projection ever treated one as a boundary.
const REPLAY_SMALL_HORIZON_TOKENS: u64 = 1_500;

fn header(cwd: &Path, session_id: &str) -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
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
                block: None,
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
    log.append_message(user_text(
        "open turn: please help with the task directive and the status",
    ))
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
        Some(HORIZON_TOKENS),
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
            quantisation: None,
        },
    )
}

/// Plans once against the fixture's current chain state, through the same
/// entry point the agent loop and `/compact` use.
fn plan_now(log: &SessionLog, profile: &CapacityProfile) -> WorkingSetPlan {
    planner::plan_for_session(log, profile, 400, 100)
}

/// Every selected turn must be a complete, addressable closed turn; packets
/// must describe a valid contiguous range and may not overlap selected turns.
/// Other history is deliberately absent and remains available through recall.
fn selection_is_bounded_and_non_overlapping(index: &TurnIndex, plan: &WorkingSetPlan) -> bool {
    let position: std::collections::HashMap<&str, usize> = index
        .turns
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    let packet_bounds = match &plan.packet_range {
        Some((first, last)) => {
            let (Some(lo), Some(hi)) = (position.get(first.as_str()), position.get(last.as_str()))
            else {
                return false;
            };
            if lo > hi
                || index.turns[*lo..=*hi]
                    .iter()
                    .any(|turn| !turn.closed || turn.card.is_none())
            {
                return false;
            }
            Some((*lo, *hi))
        }
        None => None,
    };
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (id, _) in &plan.per_turn {
        if !seen.insert(id.as_str()) {
            return false; // duplicate: split/double-counted turn
        }
        if !position
            .get(id.as_str())
            .is_some_and(|&at| index.turns[at].closed && index.turns[at].card.is_some())
        {
            return false;
        }
    }
    !seen.iter().any(|id| {
        packet_bounds.is_some_and(|(lo, hi)| {
            let at = position[id];
            at >= lo && at <= hi
        })
    })
}

/// Selected tool results are complete and reversible: open turns stay
/// verbatim, selected Full turns carry evidence digests, selected Cards name
/// evidence ids, and packets cover their declared range. Omitted closed turns
/// remain in the ledger and are addressable through recall.
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
                    let tag = vak_session::turns::evidence_tag_start(&trace.evidence_id);
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
                    return false; // omitted/packeted turns were reintroduced
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
    let digest_k = vak_context::assemble::prefix_digest(system_prefix, &tools);

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
    let digest_k1 = vak_context::assemble::prefix_digest(system_prefix, &tools);

    if digest_k != digest_k1 {
        return Ok(false);
    }
    if messages_k1.len() < messages_k.len() {
        return Ok(false);
    }
    // Every block of every earlier message, not just its text: a tool call,
    // a tool result or a provider block changing between steps is exactly
    // what breaks the cache and a replayed thinking block.
    Ok(messages_k
        .iter()
        .zip(messages_k1.iter())
        .all(|(a, b)| serde_json::to_value(a).ok() == serde_json::to_value(b).ok()))
}

/// A profile with the given usable horizon, for the packet replay.
fn profile_with_horizon(horizon: u64) -> CapacityProfile {
    CapacityProfile::from_probe(
        Some(horizon),
        None,
        Horizon {
            tokens: horizon,
            confidence: 0.9,
            last_confirmed: std::time::SystemTime::now(),
        },
        CacheBehaviour::Unknown,
        512,
        ProbeProvenance {
            probed_at: std::time::SystemTime::now(),
            rungs: Vec::new(),
            signals: Vec::new(),
            metadata_digest: format!("context-engine-gate-{horizon}"),
            quantisation: None,
        },
    )
}

/// The packet replay (docs/design/68-context-engine.md, principle 1 and
/// §4): the projection is a function of `(ledger, plan)` and nothing else,
/// and a stored packet is a cache keyed by its turn range, never a
/// boundary. The standard planner selects no packet, so the range here is
/// asked for by a plan built for it, as an over-length rejection would. A
/// packet written under that plan must then be absent from every planner
/// projection, whatever the bound model's horizon, present only for the
/// exact range it summarises, and reused without a second summariser call.
/// Nothing in the ledger is ever cut; only the projection changes.
///
/// Returns one flag per property so the scorecard can name which one
/// broke.
struct ReplayVerdict {
    /// The planner's own projection, under a small and a large horizon,
    /// carries neither the stored packet nor any `<context_summary>`.
    stored_packet_is_not_a_boundary: bool,
    /// A plan naming another range does not render the stored packet and
    /// still needs its own summariser call.
    packet_renders_only_for_its_range: bool,
    /// A plan naming the stored range again needs no compaction and
    /// projects the same messages byte for byte.
    stored_packet_is_reused: bool,
}

fn joined_text(messages: &[Message]) -> String {
    messages
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n")
}

fn packet_replay(dir: &Path) -> Result<ReplayVerdict, String> {
    let (mut log, _open_result) = build_fixture(dir, "gate-packet-replay")?;
    let small = profile_with_horizon(REPLAY_SMALL_HORIZON_TOKENS);
    let large = profile_with_horizon(2_000_000);

    let index = TurnIndex::from_log(&log);
    let carded: Vec<&vak_session::Turn> = index
        .turns
        .iter()
        .filter(|turn| turn.closed && turn.card.is_some())
        .collect();
    let [first, second, ..] = carded.as_slice() else {
        return Err("the fixture must hold closed turns with cards".into());
    };
    let Some(last) = carded.get(4) else {
        return Err("the fixture must hold at least five closed turns with cards".into());
    };
    let base = plan_now(&log, &small);
    let with_range = |first: &str, last: &str| WorkingSetPlan {
        packet_range: Some((first.to_string(), last.to_string())),
        ..base.clone()
    };

    // 1) A plan asks for a packet over the oldest five turns, and the agent
    //    loop writes it (a stand-in summary here; the summariser's wording
    //    is irrelevant to the property).
    let packeting = with_range(&first.id, &last.id);
    if !log.packet_needs_compaction(&first.id, &last.id) {
        return Err("a fresh ledger cannot already carry a packet".into());
    }
    let other = with_range(&second.id, &last.id);
    log.append_incremental_compaction(
        &first.id,
        &last.id,
        "small-model",
        "SMALL-MODEL-PACKET".into(),
        1_000,
    )
    .map_err(|e| e.to_string())?;
    let before = log.derive_with_plan(&packeting);
    if !joined_text(&before).contains("SMALL-MODEL-PACKET") {
        return Err("the plan that asked for the packet must project it".into());
    }

    // 2) The planner's own plan, under either model, never sees it.
    let stored_packet_is_not_a_boundary = [&small, &large].into_iter().all(|profile| {
        let plan = plan_now(&log, profile);
        let joined = joined_text(&log.derive_with_plan(&plan));
        plan.packet_range.is_none()
            && !joined.contains("<context_summary>")
            && !joined.contains("SMALL-MODEL-PACKET")
    });

    // 3) Another range is another packet.
    let packet_renders_only_for_its_range = log.packet_needs_compaction(&second.id, &last.id)
        && !joined_text(&log.derive_with_plan(&other)).contains("SMALL-MODEL-PACKET");

    // 4) The same range again: nothing is re-summarised and the projection
    //    is unchanged.
    let after = log.derive_with_plan(&with_range(&first.id, &last.id));
    let stored_packet_is_reused = !log.packet_needs_compaction(&first.id, &last.id)
        && serde_json::to_value(&before).ok() == serde_json::to_value(&after).ok();

    Ok(ReplayVerdict {
        stored_packet_is_not_a_boundary,
        packet_renders_only_for_its_range,
        stored_packet_is_reused,
    })
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
    let accounted_ok = selection_is_bounded_and_non_overlapping(&index, &plan);
    let no_cut_ok = no_cut_invariant_holds(&log, &index, &plan);
    let append_only_ok = steps_are_append_only_with_a_stable_prefix(dir.path())?;
    let replay = packet_replay(dir.path())?;

    let metric = |name: &'static str, ok: bool| QualityMetric {
        name,
        value: if ok { 1.0 } else { 0.0 },
        threshold: 1.0,
        direction: MetricDirection::Min,
    };

    Ok(ContextScorecard {
        metrics: vec![
            metric("plan_never_exceeds_budget", budget_ok),
            metric("selection_is_bounded_and_non_overlapping", accounted_ok),
            metric("no_cut_invariant_holds", no_cut_ok),
            metric("steps_append_only_with_stable_prefix", append_only_ok),
            metric(
                "replay_stored_packet_is_not_a_boundary",
                replay.stored_packet_is_not_a_boundary,
            ),
            metric(
                "replay_packet_renders_only_for_its_range",
                replay.packet_renders_only_for_its_range,
            ),
            metric(
                "replay_stored_packet_is_reused",
                replay.stored_packet_is_reused,
            ),
        ],
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;

    #[test]
    fn context_engine_scorecard_passes_on_healthy_machinery() {
        let card = run_context_engine_scorecard().expect("harness");
        println!("{card}");
        assert!(card.passed(), "scorecard failed: {card}");
    }

    /// The packet replay on its own, so a regression names the exact
    /// property instead of failing the whole scorecard.
    #[test]
    fn a_stored_packet_is_a_cache_for_its_range_never_a_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let verdict = packet_replay(dir.path()).expect("replay harness");
        assert!(
            verdict.stored_packet_is_not_a_boundary,
            "the planner's projection must not carry a packet its plan did not ask for"
        );
        assert!(
            verdict.packet_renders_only_for_its_range,
            "a plan naming another range must not render the stored packet"
        );
        assert!(
            verdict.stored_packet_is_reused,
            "a plan naming the stored range must reuse the packet and project the same messages"
        );
    }
}
