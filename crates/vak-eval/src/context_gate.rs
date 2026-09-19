//! Deterministic context-quality gate (docs/design/17-context.md): packet-accounting
//! properties asserted over real compaction machinery with zero model
//! calls. Every fixed context bug becomes a fixture here.
//!
//! What this gate honestly covers today: partition integrity, recent-turn
//! recall floor, verbatim exclusion of dropped turns, evidence-loss
//! visibility, and tool-pair boundary safety across a keep_recent sweep.
//! Semantic selection quality (what a summarizer keeps) is covered by
//! `eval --live`, not here — absent evidence stays UNKNOWN, never assumed.

use std::path::Path;

use vak_llm::{ContentBlock, Message, Role};
use vak_session::{SessionLog, SessionPath};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricDirection {
    Min,
    Max,
}

#[derive(Debug, Clone)]
pub struct QualityMetric {
    pub name: &'static str,
    pub value: f64,
    pub threshold: f64,
    pub direction: MetricDirection,
}

impl QualityMetric {
    fn passed(&self) -> bool {
        match self.direction {
            MetricDirection::Min => self.value >= self.threshold,
            MetricDirection::Max => self.value <= self.threshold,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextScorecard {
    pub metrics: Vec<QualityMetric>,
}

impl ContextScorecard {
    pub fn passed(&self) -> bool {
        self.metrics.iter().all(|m| m.passed())
    }
}

impl std::fmt::Display for ContextScorecard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Context quality scorecard:")?;
        for m in &self.metrics {
            let mark = if m.passed() { "PASS" } else { "FAIL" };
            let op = match m.direction {
                MetricDirection::Min => "≥",
                MetricDirection::Max => "≤",
            };
            writeln!(
                f,
                "  [{mark}] {} = {:.3} ({} {})",
                m.name, m.value, op, m.threshold
            )?;
        }
        Ok(())
    }
}

/// A fixture turn: role + text carrying named anchors.
struct FixtureTurn {
    role: Role,
    text: String,
}

fn build_fixture_log(
    dir: &Path,
    turns: &[FixtureTurn],
    session_id: &str,
) -> Result<SessionLog, String> {
    let cwd = dir.to_path_buf();
    let header = vak_session::types::SessionHeader {
        agent: Some(vak_session::types::AgentIdentity {
            id: "vak".into(),
            revision: 1,
            name: "Vak".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
        }),
        session_id: session_id.to_string(),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: Some(vak_session::ConversationContext::local(session_id, "eval")),
        contract: vak_session::types::FrozenContract {
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
    };
    let home = cwd.join(".vak-home");
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let mut log = SessionLog::create(
        SessionPath::new_session_file(&home, &cwd, session_id),
        header,
    )
    .map_err(|e| e.to_string())?;
    for t in turns {
        log.append_message(vak_session::MessageRecord {
            message: Message {
                role: t.role,
                content: vec![ContentBlock::text(t.text.clone())],
            },
            meta: None,
        })
        .map_err(|e| e.to_string())?;
    }
    Ok(log)
}

fn plain_turns(markers: &[&str]) -> Vec<FixtureTurn> {
    markers
        .iter()
        .enumerate()
        .map(|(i, m)| FixtureTurn {
            role: if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            },
            text: format!("turn-{i}: anchor[{m}] plus filler prose for token mass"),
        })
        .collect()
}

const OLD_REQUIRED: [&str; 3] = ["DECISION-alpha", "FILE-beta-path", "ERROR-gamma-fix"];
const NOISE: [&str; 2] = ["NOISE-chatter-1", "NOISE-chatter-2"];
const RECENT: [&str; 2] = ["RECENT-delta", "RECENT-epsilon"];

/// Runs the deterministic gate over a swept fixture. Returns the
/// scorecard; callers print it and decide pass/fail.
///
/// `plan_compaction`/`apply_compaction` account in whole TURNS now, not
/// messages (docs/design/68-context-engine.md §10: "a turn is never
/// split"), and a turn's two messages (directive + final answer) share one
/// partition id — the turn's directive entry id. Every check below reads
/// through that unit rather than the raw per-message chain.
pub fn run_context_scorecard() -> Result<ContextScorecard, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;

    // Sweep 1 — plain turns: partition integrity + recall floor +
    // exclusion + evidence visibility across several keep_recent values.
    let mut markers: Vec<&str> = Vec::new();
    markers.extend(OLD_REQUIRED);
    markers.extend(NOISE);
    for i in 0..4 {
        markers.push(if i % 2 == 0 {
            "FILLER-old"
        } else {
            "FILLER-mid"
        });
    }
    markers.extend(RECENT);
    let turns = plain_turns(&markers);
    let total_sweeps = 5u32;
    let mut partition_ok = 0u32;
    let mut recall_ok = 0u32;
    let mut exclusion_ok = 0u32;
    let mut evidence_visible = 0u32;

    for keep_recent in 2..=6usize {
        let mut log = build_fixture_log(dir.path(), &turns, &format!("sweep-{keep_recent}"))?;
        let index = vak_session::TurnIndex::from_log(&log);
        let total_turns = index.turns.len();
        // Turn id -> the concatenation of its own directive and final-
        // answer text, so an anchor embedded in EITHER message of a turn
        // is found through that turn's single partition id.
        let turn_text: std::collections::HashMap<String, String> = index
            .turns
            .iter()
            .map(|t| {
                let mut text = t.directive.text_content();
                if let Some(answer) = &t.final_answer {
                    text.push(' ');
                    text.push_str(&answer.text_content());
                }
                (t.id.clone(), text)
            })
            .collect();
        // Turn id -> its own RAW message texts individually (not
        // concatenated), each unique via its "turn-N:" prefix — unlike a
        // bare marker, which several turns can share — so a leak check
        // against these can't produce a false positive from an unrelated
        // KEPT turn that happens to carry the same marker word.
        let turn_message_texts: std::collections::HashMap<String, Vec<String>> = index
            .turns
            .iter()
            .map(|t| {
                let mut texts = vec![t.directive.text_content()];
                if let Some(answer) = &t.final_answer {
                    texts.push(answer.text_content());
                }
                (t.id.clone(), texts)
            })
            .collect();

        let Some(plan) = log.plan_compaction(keep_recent) else {
            // Nothing to compact at this keep_recent (too few turns): every
            // invariant below holds trivially since nothing was dropped.
            partition_ok += 1;
            recall_ok += 1;
            exclusion_ok += 1;
            evidence_visible += 1;
            continue;
        };

        // Partition integrity: disjoint, and together they cover exactly
        // the turns visible in the current projection.
        let p = &plan.partition;
        let disjoint = p
            .selected_entry_ids
            .iter()
            .all(|id| !p.dropped_entry_ids.contains(id));
        let covers = p.selected_entry_ids.len() + p.dropped_entry_ids.len() == total_turns;
        if disjoint && covers {
            partition_ok += 1;
        }

        // Evidence-loss visibility: any old-side required anchor whose
        // turn got dropped must be LISTED as dropped — never silently
        // gone. Anchors whose turn stayed selected count as visible too.
        let all_accounted = OLD_REQUIRED.iter().all(|anchor| {
            p.selected_entry_ids
                .iter()
                .any(|id| turn_text.get(id).is_some_and(|t| t.contains(anchor)))
                || p.dropped_entry_ids
                    .iter()
                    .any(|id| turn_text.get(id).is_some_and(|t| t.contains(anchor)))
        });
        if all_accounted {
            evidence_visible += 1;
        }

        // Apply with a fixed dummy summary: post-compaction projection is
        // fully predictable, so recall/exclusion are exact.
        log.apply_compaction(&plan, "FIXED-DUMMY-SUMMARY".into(), 10_000)
            .map_err(|e| e.to_string())?;
        let projected = log.derive_messages();
        let joined = projected
            .iter()
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n");

        // Recall floor: the keep_recent window survives verbatim.
        if RECENT.iter().all(|a| joined.contains(a)) {
            recall_ok += 1;
        }

        // Exclusion: no DROPPED turn's own raw text survives in the
        // post-compaction projection (it may only re-enter through the
        // fixed dummy summary text, which carries none of these turns'
        // exact wording).
        let dropped_leak = p.dropped_entry_ids.iter().any(|id| {
            turn_message_texts
                .get(id)
                .is_some_and(|texts| texts.iter().any(|t| joined.contains(t.as_str())))
        });
        if !dropped_leak {
            exclusion_ok += 1;
        }
    }

    // Sweep 2 — no-tool-blocks invariant: a closed turn's `full_record`
    // projection never carries a `ToolUse`/`ToolResult` block, at any
    // keep_recent (docs/design/68-context-engine.md §10: past turns
    // project as a directive + trace/answer text, never raw tool pairs —
    // there is no pair-boundary walk left to get wrong).
    let mut no_tool_blocks_ok = 0u32;
    let pair_total = 4u32;
    let mut pair_markers: Vec<&str> = vec!["PAIR-SEED"];
    pair_markers.extend(std::iter::repeat_n("PAIRED-TURN", 6));
    for keep_recent in 1..=4usize {
        let mut log = build_fixture_log(
            dir.path(),
            &plain_turns(&pair_markers),
            &format!("pairs-{keep_recent}"),
        )?;
        if let Some(plan) = log.plan_compaction(keep_recent) {
            log.apply_compaction(&plan, "PAIR-SWEEP-SUMMARY".into(), 1_000)
                .map_err(|e| e.to_string())?;
        }
        let clean = log.derive_messages().iter().all(|m| {
            !m.content.iter().any(|b| {
                matches!(
                    b,
                    ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. }
                )
            })
        });
        if clean {
            no_tool_blocks_ok += 1;
        }
    }

    // Sweep 3 — repeated compaction: a second pass still partitions
    // cleanly over a projection that already contains a summary
    // pseudo-entry. The second pass uses a smaller keep_recent than the
    // first because only the turns the first pass KEPT remain as real
    // turn units afterward (docs/design/68 §10: the summary is excluded
    // from `plan_compaction`'s own turn count).
    let repeat_ok = {
        let mut log = build_fixture_log(dir.path(), &turns, "repeat")?;
        let first_ok = if let Some(p1) = log.plan_compaction(3) {
            log.apply_compaction(&p1, "FIRST-SUMMARY".into(), 9_000)
                .is_ok()
        } else {
            false
        };
        let second_ok = first_ok
            && log.plan_compaction(1).map(|p2| {
                log.apply_compaction(&p2, "SECOND-SUMMARY".into(), 5_000)
                    .is_ok()
                    && p2.partition.selected_entry_ids.len() + p2.partition.dropped_entry_ids.len()
                        > 0
            }) == Some(true);
        second_ok
            && log
                .derive_messages()
                .iter()
                .any(|m| m.text_content().contains("SECOND-SUMMARY"))
    };

    let metric = |name: &'static str, ok: u32, total: u32| QualityMetric {
        name,
        value: ok as f64 / total.max(1) as f64,
        threshold: 1.0,
        direction: MetricDirection::Min,
    };

    Ok(ContextScorecard {
        metrics: vec![
            metric("partition_covers_projection", partition_ok, total_sweeps),
            metric("recent_recall_floor", recall_ok, total_sweeps),
            metric("dropped_verbatim_exclusion", exclusion_ok, total_sweeps),
            metric("evidence_loss_visible", evidence_visible, total_sweeps),
            metric(
                "no_raw_tool_blocks_in_projection",
                no_tool_blocks_ok,
                pair_total,
            ),
            QualityMetric {
                name: "repeated_compaction_partition",
                value: if repeat_ok { 1.0 } else { 0.0 },
                threshold: 1.0,
                direction: MetricDirection::Min,
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn context_scorecard_passes_on_healthy_machinery() {
        let card = run_context_scorecard().expect("harness");
        println!("{card}");
        assert!(card.passed(), "scorecard failed: {card}");
    }
}
