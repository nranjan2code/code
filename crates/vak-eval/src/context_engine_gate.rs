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

/// The "small model" horizon for the two-model replay: tight enough that
/// the 30-turn fixture cannot even fit every card, so the plan packets the
/// oldest turns and writes a `Compaction` entry — the state the replay
/// then re-projects under a large model.
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
            quantisation: None,
        },
    )
}

/// Plans once against the fixture's current chain state, through the same
/// entry point the agent loop and `/compact` use.
fn plan_now(log: &SessionLog, profile: &CapacityProfile) -> WorkingSetPlan {
    planner::plan_for_session(log, profile, 400, 100)
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

/// A profile with the given usable horizon, for the two-model replay.
fn profile_with_horizon(horizon: u64) -> CapacityProfile {
    CapacityProfile::from_probe(
        horizon,
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

/// The two-model replay (docs/design/68-context-engine.md, principle 1 and
/// §4): the projection is a function of `(ledger, the bound model's
/// profile)` and nothing else. A session that ran on a small model —
/// whose plan packeted the oldest turns and wrote a `Compaction` entry for
/// them — is then bound to a model with a horizon large enough to hold
/// every turn at `Full`. The large model's plan says `Full` for the turns
/// the small model packeted, so its projection must carry those turns'
/// real records, not the small model's packet summary. Then bound back to
/// the small model, the stored packet is reused (no second summariser
/// call) and the projection is the same as before the switch. Nothing in
/// the ledger is ever cut; only the projection changes with the model.
///
/// Returns one flag per property so the scorecard can name which one
/// broke.
struct ReplayVerdict {
    /// The large model's plan puts the small model's packeted turns at
    /// `Full` (the planner is model-driven, not boundary-driven).
    large_plan_promotes_packeted_turns: bool,
    /// The large model's projection carries those turns' directives
    /// verbatim and no `<context_summary>` at all.
    large_projection_follows_its_plan: bool,
    /// Bound back to the small model, the packet already stored is reused:
    /// no compaction is needed and the projection matches the pre-switch
    /// one byte for byte.
    small_model_reuses_its_packet: bool,
}

fn two_model_replay(dir: &Path) -> Result<ReplayVerdict, String> {
    let (mut log, _open_result) = build_fixture(dir, "gate-two-model-replay")?;
    let small = profile_with_horizon(REPLAY_SMALL_HORIZON_TOKENS);
    // Large enough that every closed turn fits at Full with room to spare.
    let large = profile_with_horizon(2_000_000);

    // 1) The small model runs: its plan packets the oldest turns, and the
    //    agent loop writes the packet (a stand-in summary here; the
    //    summariser's wording is irrelevant to the property).
    let small_plan = plan_now(&log, &small);
    let Some((first_packeted, last_packeted)) = small_plan.packet_range.clone() else {
        return Err(
            "the small profile must packet at least one turn for the replay to mean anything"
                .into(),
        );
    };
    if !log.packet_needs_compaction(&first_packeted, &last_packeted) {
        return Err("a fresh ledger cannot already carry a packet".into());
    }
    log.append_incremental_compaction(
        &first_packeted,
        &last_packeted,
        "small-model",
        "SMALL-MODEL-PACKET".into(),
        1_000,
    )
    .map_err(|e| e.to_string())?;
    let small_messages_before = log.derive_with_plan(&plan_now(&log, &small));
    let small_joined_before = small_messages_before
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    if !small_joined_before.contains("SMALL-MODEL-PACKET") {
        return Err("the small model's own projection must carry its packet".into());
    }

    let index = TurnIndex::from_log(&log);
    let position: std::collections::HashMap<&str, usize> = index
        .turns
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    let lo = position[first_packeted.as_str()];
    let hi = position[last_packeted.as_str()];
    let packeted: Vec<&vak_session::Turn> = index.turns[lo..=hi].iter().collect();

    // 2) The same session, now bound to the large model.
    let large_plan = plan_now(&log, &large);
    let large_fidelity: std::collections::HashMap<&str, Fidelity> = large_plan
        .per_turn
        .iter()
        .map(|(id, f)| (id.as_str(), *f))
        .collect();
    let large_plan_promotes_packeted_turns = large_plan.packet_range.is_none()
        && packeted
            .iter()
            .all(|t| large_fidelity.get(t.id.as_str()) == Some(&Fidelity::Full));

    let large_messages = log.derive_with_plan(&large_plan);
    let large_joined = large_messages
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    let large_projection_follows_its_plan = !large_joined.contains("<context_summary>")
        && !large_joined.contains("SMALL-MODEL-PACKET")
        && packeted
            .iter()
            .all(|t| large_joined.contains(&t.directive.text_content()));

    // 3) Back to the small model: the stored packet covers exactly the
    //    range its plan needs again, so nothing is re-summarised and the
    //    projection is unchanged.
    let small_plan_after = plan_now(&log, &small);
    let reuses = small_plan_after
        .packet_range
        .as_ref()
        .is_some_and(|(first, last)| !log.packet_needs_compaction(first, last));
    let small_messages_after = log.derive_with_plan(&small_plan_after);
    let same_projection = small_messages_before.len() == small_messages_after.len()
        && small_messages_before
            .iter()
            .zip(small_messages_after.iter())
            .all(|(a, b)| a.text_content() == b.text_content());
    let small_model_reuses_its_packet = reuses && same_projection;

    Ok(ReplayVerdict {
        large_plan_promotes_packeted_turns,
        large_projection_follows_its_plan,
        small_model_reuses_its_packet,
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
    let accounted_ok = every_turn_accounted_exactly_once(&index, &plan);
    let no_cut_ok = no_cut_invariant_holds(&log, &index, &plan);
    let append_only_ok = steps_are_append_only_with_a_stable_prefix(dir.path())?;
    let replay = two_model_replay(dir.path())?;

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
            metric(
                "replay_large_plan_promotes_packeted_turns",
                replay.large_plan_promotes_packeted_turns,
            ),
            metric(
                "replay_large_projection_follows_its_plan",
                replay.large_projection_follows_its_plan,
            ),
            metric(
                "replay_small_model_reuses_its_packet",
                replay.small_model_reuses_its_packet,
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

    /// The two-model replay on its own, so a regression names the exact
    /// property instead of failing the whole scorecard.
    #[test]
    fn projection_is_a_function_of_the_bound_model_not_of_earlier_models() {
        let dir = tempfile::tempdir().unwrap();
        let verdict = two_model_replay(dir.path()).expect("replay harness");
        assert!(
            verdict.large_plan_promotes_packeted_turns,
            "the large model's plan must put the small model's packeted turns at Full"
        );
        assert!(
            verdict.large_projection_follows_its_plan,
            "the large model's projection must carry the packeted turns' records, not the small model's packet"
        );
        assert!(
            verdict.small_model_reuses_its_packet,
            "bound back to the small model, the stored packet must be reused and the projection unchanged"
        );
    }
}
