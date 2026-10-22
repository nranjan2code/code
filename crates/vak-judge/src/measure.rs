//! Measurements over real turns and the proposals they support
//! (docs/design/88-control-plane.md §7, stage C7). Pure: the caller gathers
//! each turn's facts from the ledger.

use std::collections::{BTreeMap, BTreeSet};

use vak_control::{CallVerdict, Cause, End, Fault, Pattern, Proposal, Refusal};

/// What one finished turn's decisions recorded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnFacts {
    pub turn: String,
    pub model: String,
    /// Steps framed (`FrameStep` decisions).
    pub steps: u32,
    /// From the turn's admission to its ending, when both were recorded.
    pub seconds: Option<f64>,
    /// Each call the runtime ruled on, by tool name.
    pub calls: Vec<(String, CallVerdict)>,
    /// Each call that ran and failed, by tool name.
    pub failures: Vec<(String, Fault)>,
    pub corrections: Vec<vak_control::Correction>,
    pub ending: Option<End>,
}

/// One model's measured behaviour over its turns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelMeasure {
    pub model: String,
    pub turns: u64,
    pub answered: u64,
    pub failed: u64,
    pub aborted: u64,
    pub step_limit: u64,
    pub calls: u64,
    pub refused_calls: u64,
    pub failed_calls: u64,
    pub corrections: u64,
    /// Median over turns; `None` when no turn recorded it.
    pub median_steps: Option<f64>,
    pub median_seconds: Option<f64>,
}

/// One `ModelMeasure` per model, ordered by model name.
pub fn measure(turns: &[TurnFacts]) -> Vec<ModelMeasure> {
    group(turns)
        .into_iter()
        .map(|(model, ts)| {
            let mut m = ModelMeasure {
                model: model.to_string(),
                turns: count(ts.len()),
                ..ModelMeasure::default()
            };
            for t in &ts {
                match t.ending {
                    Some(End::Answered) => m.answered += 1,
                    Some(End::Failed) => m.failed += 1,
                    Some(End::Aborted) => m.aborted += 1,
                    Some(End::StepLimit) => m.step_limit += 1,
                    None => {}
                }
                m.calls += count(t.calls.len());
                m.refused_calls += count(
                    t.calls
                        .iter()
                        .filter(|(_, v)| matches!(v, CallVerdict::Refused(_)))
                        .count(),
                );
                m.failed_calls += count(t.failures.len());
                m.corrections += count(t.corrections.len());
            }
            m.median_steps = median(ts.iter().map(|t| f64::from(t.steps)).collect());
            m.median_seconds = median(ts.iter().filter_map(|t| t.seconds).collect());
            m
        })
        .collect()
}

/// Every contract pattern observed, one proposal per (model, pattern),
/// most frequent first (ties: by model, then pattern). No threshold: a
/// pattern seen once is offered, and a person decides.
pub fn promote(turns: &[TurnFacts]) -> Vec<Proposal> {
    let mut out = Vec::new();
    for (model, ts) in group(turns) {
        let mut calls_of: BTreeMap<&str, u64> = BTreeMap::new();
        for t in &ts {
            for (tool, _) in &t.calls {
                *calls_of.entry(tool.as_str()).or_default() += 1;
            }
        }

        let mut tallies: BTreeMap<Pattern, (u64, Vec<String>)> = BTreeMap::new();
        for t in &ts {
            let mut seen = |pattern: Pattern| {
                let entry = tallies.entry(pattern).or_default();
                entry.0 += 1;
                if !entry.1.contains(&t.turn) {
                    entry.1.push(t.turn.clone());
                }
            };
            for (tool, verdict) in &t.calls {
                if let CallVerdict::Refused(refusal) = verdict
                    && refusal_is_contract_fault(*refusal)
                {
                    seen(Pattern::ToolRefused {
                        tool: tool.clone(),
                        refusal: *refusal,
                    });
                }
            }
            for (tool, fault) in &t.failures {
                if fault_is_contract_fault(*fault) {
                    seen(Pattern::ToolFailed {
                        tool: tool.clone(),
                        fault: *fault,
                    });
                }
            }
            let causes: BTreeSet<Cause> = t.corrections.iter().map(|c| c.cause).collect();
            for cause in causes {
                seen(Pattern::Corrected { cause });
            }
            if t.ending == Some(End::StepLimit) {
                seen(Pattern::StepLimit);
            }
        }

        let turn_count = count(ts.len());
        for (pattern, (count_seen, turns_seen)) in tallies {
            let of = match &pattern {
                Pattern::ToolRefused { tool, .. } | Pattern::ToolFailed { tool, .. } => calls_of
                    .get(tool.as_str())
                    .copied()
                    .unwrap_or(0)
                    .max(count_seen),
                Pattern::Corrected { .. } | Pattern::StepLimit => turn_count,
            };
            out.push(Proposal {
                key: format!("{model}|{}", pattern_key(&pattern)),
                model: model.to_string(),
                pattern,
                count: count_seen,
                of,
                turns: turns_seen,
            });
        }
    }
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.model.cmp(&b.model))
            .then_with(|| a.pattern.cmp(&b.pattern))
    });
    out
}

/// Refusals that lie in the call or the contract. The others are the
/// system working: a revoked capability, a rule, a person declining, or no
/// approver to ask.
fn refusal_is_contract_fault(refusal: Refusal) -> bool {
    match refusal {
        Refusal::Unknown | Refusal::Schema | Refusal::Tool | Refusal::Repeated => true,
        Refusal::Revoked | Refusal::Policy | Refusal::Person | Refusal::Unattended => false,
    }
}

/// Faults that lie in the call's shape, its target, the tool or the limit.
/// A refusal by a rule or a person, and a cancellation, are not patterns.
fn fault_is_contract_fault(fault: Fault) -> bool {
    match fault {
        Fault::Shape | Fault::Target | Fault::Tool | Fault::Limit => true,
        Fault::Refused | Fault::Cancelled => false,
    }
}

fn pattern_key(pattern: &Pattern) -> String {
    match pattern {
        Pattern::ToolRefused { tool, refusal } => {
            format!("tool_refused:{tool}:{}", snake(&format!("{refusal:?}")))
        }
        Pattern::ToolFailed { tool, fault } => {
            format!("tool_failed:{tool}:{}", snake(&format!("{fault:?}")))
        }
        Pattern::Corrected { cause } => format!("corrected:{}", snake(&format!("{cause:?}"))),
        Pattern::StepLimit => "step_limit".to_string(),
    }
}

/// `StopHook` to `stop_hook`.
fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn group(turns: &[TurnFacts]) -> BTreeMap<&str, Vec<&TurnFacts>> {
    let mut by_model: BTreeMap<&str, Vec<&TurnFacts>> = BTreeMap::new();
    for t in turns {
        by_model.entry(t.model.as_str()).or_default().push(t);
    }
    by_model
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn median(mut xs: Vec<f64>) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    let mid = n / 2;
    if n % 2 == 1 {
        xs.get(mid).copied()
    } else {
        let lo = xs.get(mid - 1).copied()?;
        let hi = xs.get(mid).copied()?;
        Some((lo + hi) / 2.0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use vak_control::{Correction, Rung};

    fn turn(id: &str, model: &str) -> TurnFacts {
        TurnFacts {
            turn: id.to_string(),
            model: model.to_string(),
            steps: 1,
            ..TurnFacts::default()
        }
    }

    fn refused(tool: &str, r: Refusal) -> (String, CallVerdict) {
        (tool.to_string(), CallVerdict::Refused(r))
    }

    fn accepted(tool: &str) -> (String, CallVerdict) {
        (tool.to_string(), CallVerdict::Accepted)
    }

    fn correction(cause: Cause) -> Correction {
        Correction {
            cause,
            rung: Rung::Tell,
        }
    }

    #[test]
    fn measure_counts_endings_and_medians_per_model() {
        let mut a1 = turn("a1", "m1");
        a1.steps = 1;
        a1.seconds = Some(2.0);
        a1.ending = Some(End::Answered);
        let mut a2 = turn("a2", "m1");
        a2.steps = 3;
        a2.seconds = Some(4.0);
        a2.ending = Some(End::Failed);
        let mut a3 = turn("a3", "m1");
        a3.steps = 8;
        a3.ending = Some(End::StepLimit);
        a3.calls = vec![refused("read", Refusal::Policy), accepted("read")];
        a3.failures = vec![("bash".into(), Fault::Target)];
        a3.corrections = vec![correction(Cause::Grounding)];
        let mut b1 = turn("b1", "m2");
        b1.steps = 2;
        b1.ending = Some(End::Aborted);

        let got = measure(&[a1, a2, a3, b1]);
        assert_eq!(got.len(), 2);
        let m1 = got.first().unwrap();
        assert_eq!(m1.model, "m1");
        assert_eq!(m1.turns, 3);
        assert_eq!(
            (m1.answered, m1.failed, m1.aborted, m1.step_limit),
            (1, 1, 0, 1)
        );
        assert_eq!(m1.calls, 2);
        assert_eq!(m1.refused_calls, 1);
        assert_eq!(m1.failed_calls, 1);
        assert_eq!(m1.corrections, 1);
        // Steps 1, 3, 8: odd count, the middle value.
        assert_eq!(m1.median_steps, Some(3.0));
        // Seconds 2, 4: even count, the mean of the middle two.
        assert_eq!(m1.median_seconds, Some(3.0));

        let m2 = got.get(1).unwrap();
        assert_eq!(m2.model, "m2");
        assert_eq!(m2.turns, 1);
        assert_eq!(m2.aborted, 1);
        assert_eq!(m2.median_steps, Some(2.0));
        assert_eq!(m2.median_seconds, None);
    }

    #[test]
    fn measure_of_nothing_is_empty() {
        assert!(measure(&[]).is_empty());
        assert!(promote(&[]).is_empty());
    }

    #[test]
    fn promote_counts_schema_refusals_against_calls_of_the_tool() {
        let mut t1 = turn("t1", "m1");
        t1.calls = vec![refused("show_table", Refusal::Schema), accepted("read")];
        let mut t2 = turn("t2", "m1");
        t2.calls = vec![
            refused("show_table", Refusal::Schema),
            accepted("show_table"),
        ];
        // A person declining and a revoked capability are the system working.
        let mut t3 = turn("t3", "m1");
        t3.calls = vec![
            refused("bash", Refusal::Person),
            refused("bash", Refusal::Revoked),
        ];
        t3.failures = vec![("bash".into(), Fault::Cancelled)];

        let got = promote(&[t1, t2, t3]);
        assert_eq!(got.len(), 1);
        let p = got.first().unwrap();
        assert_eq!(p.key, "m1|tool_refused:show_table:schema");
        assert_eq!(p.model, "m1");
        assert_eq!(p.count, 2);
        assert_eq!(p.of, 3);
        assert_eq!(p.turns, vec!["t1".to_string(), "t2".to_string()]);
        assert_eq!(
            p.pattern,
            Pattern::ToolRefused {
                tool: "show_table".into(),
                refusal: Refusal::Schema,
            }
        );
    }

    #[test]
    fn promote_counts_a_correction_once_per_turn() {
        let mut t1 = turn("t1", "m1");
        t1.corrections = vec![correction(Cause::Grounding), correction(Cause::Grounding)];
        let mut t2 = turn("t2", "m1");
        t2.corrections = vec![correction(Cause::Grounding)];
        let got = promote(&[t1, t2]);
        assert_eq!(got.len(), 1);
        let p = got.first().unwrap();
        assert_eq!(p.key, "m1|corrected:grounding");
        assert_eq!(p.count, 2);
        assert_eq!(p.of, 2);
        assert_eq!(p.turns.len(), 2);
    }

    #[test]
    fn promote_offers_step_limit_only_where_a_turn_ended_there() {
        let mut t1 = turn("t1", "m1");
        t1.ending = Some(End::Answered);
        let got = promote(&[t1.clone()]);
        assert!(got.is_empty());

        let mut t2 = turn("t2", "m1");
        t2.ending = Some(End::StepLimit);
        let got = promote(&[t1, t2]);
        assert_eq!(got.len(), 1);
        let p = got.first().unwrap();
        assert_eq!(p.key, "m1|step_limit");
        assert_eq!((p.count, p.of), (1, 2));
    }

    #[test]
    fn promote_orders_by_count_then_model_then_pattern() {
        let mut t1 = turn("t1", "m2");
        t1.calls = vec![refused("read", Refusal::Unknown)];
        let mut t2 = turn("t2", "m1");
        t2.calls = vec![refused("read", Refusal::Unknown)];
        let mut t3 = turn("t3", "m1");
        t3.calls = vec![refused("read", Refusal::Unknown)];
        t3.ending = Some(End::StepLimit);
        let got = promote(&[t1, t2, t3]);
        let keys: Vec<&str> = got.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "m1|tool_refused:read:unknown",
                "m1|step_limit",
                "m2|tool_refused:read:unknown",
            ]
        );
        let counts: Vec<u64> = got.iter().map(|p| p.count).collect();
        assert_eq!(counts, vec![2, 1, 1]);
    }

    #[test]
    fn snake_names_match_the_wire_names() {
        assert_eq!(snake("StopHook"), "stop_hook");
        assert_eq!(snake("Plan"), "plan");
        assert_eq!(snake("CardCompleteness"), "card_completeness");
    }
}
