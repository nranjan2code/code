//! What the decisions recorded in real ledgers measure, and the proposals
//! they support (docs/design/88-control-plane.md §7, stage C7). The
//! arithmetic is `vak_judge::measure`; this gathers each finished turn's
//! facts from the ledgers a person can see, the trash honoured.

use std::collections::HashMap;

use vak_judge::measure::TurnFacts;
use vak_session::{EntryPayload, SessionLog};

/// Every finished turn's facts, and how many ledgers could not be read
/// (reported, never silently dropped).
#[derive(Debug, Default)]
pub struct LedgerTurns {
    pub turns: Vec<TurnFacts>,
    pub sessions: u64,
    pub unreadable: u64,
}

/// The facts of every finished turn in the sessions the catalog lists.
pub fn ledger_turns(core: &crate::Core) -> Result<LedgerTurns, crate::CoreError> {
    let catalog = core.catalog()?;
    catalog.catch_up()?;
    let audience = vak_catalog::Audience {
        exclude_sessions: crate::trash::trashed(&core.shared_scope()),
        ..Default::default()
    };
    let mut gathered = LedgerTurns::default();
    for node in catalog.list("session", &audience, usize::MAX)? {
        gathered.sessions += 1;
        let Some(log) = node
            .locator
            .and_then(|locator| SessionLog::open_read_only(std::path::PathBuf::from(locator)).ok())
        else {
            gathered.unreadable += 1;
            continue;
        };
        gathered.turns.extend(turn_facts(&log));
    }
    Ok(gathered)
}

/// The facts of each finished turn of one ledger, in order. A call is named
/// by the tool the model called it; a turn with no ending is still open.
pub fn turn_facts(log: &SessionLog) -> Vec<TurnFacts> {
    let entries = log.chain_to_root();
    let names: HashMap<&str, &str> = entries
        .iter()
        .filter_map(|entry| match &entry.payload {
            EntryPayload::Message(record) => Some(&record.message.content),
            _ => None,
        })
        .flatten()
        .filter_map(|block| match block {
            vak_llm::ContentBlock::ToolUse { id, name, .. } => Some((id.as_str(), name.as_str())),
            _ => None,
        })
        .collect();
    let tool = |call: &str| {
        names
            .get(call)
            .map_or_else(String::new, |name| (*name).to_string())
    };
    // A refused call never ran: its failed effect is the refusal itself,
    // already counted as one.
    let refused: std::collections::HashSet<&str> = entries
        .iter()
        .filter_map(|entry| match &entry.payload {
            EntryPayload::Decision(decision) => match decision.choice {
                vak_control::Choice::AcceptCall(vak_control::CallVerdict::Refused(_)) => {
                    Some(&decision.evidence)
                }
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .filter_map(|evidence| match evidence {
            vak_control::Evidence::Call(call) => Some(call.as_str()),
            _ => None,
        })
        .collect();

    let mut order: Vec<String> = Vec::new();
    let mut turns: HashMap<String, (TurnFacts, Option<chrono::DateTime<chrono::Utc>>)> =
        HashMap::new();
    for entry in &entries {
        let Some(turn) = entry.at_turn.as_deref() else {
            continue;
        };
        let (facts, admitted) = turns.entry(turn.to_string()).or_insert_with(|| {
            order.push(turn.to_string());
            (
                TurnFacts {
                    turn: turn.to_string(),
                    ..Default::default()
                },
                None,
            )
        });
        match &entry.payload {
            EntryPayload::Decision(decision) => match &decision.choice {
                vak_control::Choice::AdmitTurn(admission) => {
                    facts.model.clone_from(&admission.model);
                    *admitted = Some(entry.ts);
                }
                vak_control::Choice::FrameStep(_) => facts.steps += 1,
                vak_control::Choice::AcceptCall(verdict) => {
                    for evidence in &decision.evidence {
                        if let vak_control::Evidence::Call(call) = evidence {
                            facts.calls.push((tool(call), *verdict));
                        }
                    }
                }
                vak_control::Choice::Correct(correction) => facts.corrections.push(*correction),
                vak_control::Choice::EndTurn(ending) => {
                    facts.ending = Some(ending.end);
                    facts.seconds =
                        admitted.map(|start| (entry.ts - start).num_milliseconds() as f64 / 1000.0);
                }
                vak_control::Choice::AcceptAnswer(_) | vak_control::Choice::Promote(_) => {}
            },
            EntryPayload::CallEffect(record) => {
                if let vak_session::types::CallEffect::Failed { fault } = record.effect
                    && !refused.contains(record.tool_use_id.as_str())
                {
                    facts.failures.push((tool(&record.tool_use_id), fault));
                }
            }
            _ => {}
        }
    }
    order
        .into_iter()
        .filter_map(|turn| turns.remove(&turn))
        .map(|(facts, _)| facts)
        .filter(|facts| facts.ending.is_some())
        .collect()
}

/// The measurement and its proposals as one report, the shape `vak eval
/// --ledgers --json` prints and the admin console reads.
pub fn report(core: &crate::Core) -> Result<serde_json::Value, crate::CoreError> {
    let gathered = ledger_turns(core)?;
    let models: Vec<serde_json::Value> = vak_judge::measure::measure(&gathered.turns)
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "model": m.model,
                "turns": m.turns,
                "answered": m.answered,
                "failed": m.failed,
                "aborted": m.aborted,
                "step_limit": m.step_limit,
                "calls": m.calls,
                "refused_calls": m.refused_calls,
                "failed_calls": m.failed_calls,
                "corrections": m.corrections,
                "median_steps": m.median_steps,
                "median_seconds": m.median_seconds,
            })
        })
        .collect();
    let reviews = reviews(core)?;
    let proposals: Vec<serde_json::Value> = vak_judge::measure::promote(&gathered.turns)
        .into_iter()
        .map(|proposal| {
            let mut value = serde_json::to_value(&proposal).unwrap_or_default();
            value["review"] = serde_json::to_value(reviews.get(&proposal.key)).unwrap_or_default();
            value
        })
        .collect();
    Ok(serde_json::json!({
        "sessions": gathered.sessions,
        "unreadable": gathered.unreadable,
        "turns": gathered.turns.len(),
        "models": models,
        "proposals": proposals,
    }))
}

/// One row of the `proposal-reviews/` chain: a person's answer to a
/// proposal, by its stable key. The latest answer to a key holds.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReviewRow {
    pub key: String,
    pub review: vak_control::Review,
    pub at: chrono::DateTime<chrono::Utc>,
    pub actor: vak_session::ids::PrincipalId,
}

/// The latest answer to each proposal. A row that does not decode is an
/// error, never skipped: what a person decided is not guessed.
pub fn reviews(core: &crate::Core) -> Result<HashMap<String, ReviewRow>, crate::CoreError> {
    let chain = vak_session::chain::RecordChain::at(core.shared_scope().proposal_reviews());
    let mut latest = HashMap::new();
    for row in chain.read::<serde_json::Value>() {
        let row: ReviewRow = serde_json::from_value(row).map_err(|error| {
            crate::CoreError::Session(vak_session::SessionError::Corrupt {
                line: 0,
                message: format!("a proposal review does not decode: {error}"),
            })
        })?;
        latest.insert(row.key.clone(), row);
    }
    Ok(latest)
}

/// Records a person's answer to a proposal the measurement offers now.
/// Nothing in Vak changes: the answer is the record.
pub fn review(
    core: &crate::Core,
    key: &str,
    review: vak_control::Review,
    actor: vak_session::ids::PrincipalId,
) -> Result<ReviewRow, ReviewError> {
    let turns = ledger_turns(core).map_err(ReviewError::Core)?;
    if !vak_judge::measure::promote(&turns.turns)
        .iter()
        .any(|proposal| proposal.key == key)
    {
        return Err(ReviewError::NotOffered);
    }
    let row = ReviewRow {
        key: key.to_string(),
        review,
        at: chrono::Utc::now(),
        actor,
    };
    vak_session::chain::RecordChain::at(core.shared_scope().proposal_reviews())
        .append(&row)
        .map_err(|error| ReviewError::Core(crate::CoreError::Session(error)))?;
    Ok(row)
}

#[derive(Debug)]
pub enum ReviewError {
    /// No proposal with that key is offered by the current measurement.
    NotOffered,
    Core(crate::CoreError),
}
