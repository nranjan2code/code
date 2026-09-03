//! The `commitments` tool: the agent can read its own portfolio.
//!
//! # Why a tool rather than a chat command
//!
//! "What are you working on?", "what's blocked?", "did that ever finish?" are
//! ordinary questions, and they arrive on every surface — a Telegram message,
//! a desktop turn, a cron check-in. Building a slash-command layer in the
//! gateway would answer them on exactly one surface and create a second
//! dispatch path beside the tool broker.
//!
//! As a capability it reaches all of them at once, crosses the same permission
//! boundary as everything else, and appears in the ledger like any other call.
//!
//! Strictly read-only. Opening, advancing and closing a commitment are effects
//! the runtime performs; the model may propose criteria but never marks one
//! passed (`AGENTS.md` invariant 32), so there is deliberately no write verb
//! here for it to reach for.

use serde_json::{Value, json};

use vak_commit::{CommitmentLedger, SchedulerContext, rank};

pub struct CommitmentsTool {
    pub sessions_home: std::path::PathBuf,
}

#[async_trait::async_trait]
impl vak_tools::Tool for CommitmentsTool {
    fn name(&self) -> &str {
        "commitments"
    }

    fn description(&self) -> &str {
        "Read the durable commitments this agent holds: long-running work that \
         outlives a single conversation, what evidence each one still needs \
         before it can be called done, what is waiting on a person, and what \
         already closed and how. Use when the user asks what you are working \
         on, what is blocked or outstanding, whether something ever finished, \
         or what you owe them. Read-only."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "include_closed": {
                    "type": "boolean",
                    "description": "Include commitments that have already closed (default false)"
                },
                "id": {
                    "type": "string",
                    "description": "Full id or unique prefix of one commitment to read in detail"
                }
            }
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let ledger = CommitmentLedger::new(&self.sessions_home);
        let include_closed = args
            .get("include_closed")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if let Some(id) = args.get("id").and_then(Value::as_str) {
            let all = ledger.all();
            let matches: Vec<_> = all
                .iter()
                .filter(|c| c.commitment_id == id || c.commitment_id.starts_with(id))
                .collect();
            return match matches.as_slice() {
                [] => vak_tools::ToolOutput::error(format!("no commitment matching '{id}'")),
                // An ambiguous prefix names nothing rather than whichever row
                // happened to sort first.
                [_, _, ..] => vak_tools::ToolOutput::error(format!(
                    "'{id}' matches {} commitments; use a longer prefix",
                    matches.len()
                )),
                [one] => vak_tools::ToolOutput::ok(detail(one)),
            };
        }

        let commitments = if include_closed {
            ledger.all()
        } else {
            ledger.open()
        };
        if commitments.is_empty() {
            return vak_tools::ToolOutput::ok(
                "No commitments. Nothing asked of this agent so far has needed to \
                 outlive its session.",
            );
        }
        // Ranked by the same scheduler the runtime uses, so what the model
        // reports as "next" is what would actually be worked next.
        let ranked = rank(&commitments, &SchedulerContext::default());
        let mut out = String::new();
        for priority in ranked {
            let Some(commitment) = commitments
                .iter()
                .find(|c| c.commitment_id == priority.commitment_id)
            else {
                continue;
            };
            let short = &commitment.commitment_id[..8.min(commitment.commitment_id.len())];
            out.push_str(&format!("[{short}] {}\n", commitment.summary()));
            match &priority.withheld {
                Some(reason) => out.push_str(&format!("    held: {reason}\n")),
                None => out.push_str(&format!("    priority {:.1}\n", priority.score)),
            }
            // The evidence gap is the fact worth volunteering: it is the
            // difference between work that is finished and work that merely
            // looks finished.
            if commitment.closure.is_none() {
                let achieved = commitment.achieved_strength();
                let required = commitment.spec.min_satisfaction;
                if !achieved.satisfies(required) {
                    out.push_str(&format!(
                        "    needs {} evidence to close; has {}\n",
                        required.as_str(),
                        achieved.as_str()
                    ));
                }
            }
        }
        out.push_str(
            "\nYou may report and discuss these. You cannot mark a criterion passed — \
             the runtime evaluates them.",
        );
        vak_tools::ToolOutput::ok(out)
    }
}

fn detail(commitment: &vak_commit::Commitment) -> String {
    let mut out = format!(
        "{}\n  id {}\n  phase {}\n  opened {}\n  requires {} evidence; has {}\n  spend ${:.4}\n",
        commitment.spec.objective,
        commitment.commitment_id,
        commitment.phase.as_str(),
        commitment.opened_at.to_rfc3339(),
        commitment.spec.min_satisfaction.as_str(),
        commitment.achieved_strength().as_str(),
        commitment.spend_usd,
    );
    if let Some(suspension) = &commitment.suspension {
        out.push_str(&format!("  waiting: {}\n", suspension.describe()));
    }
    if let Some(blocker) = &commitment.blocker {
        out.push_str(&format!("  blocked: {blocker}\n"));
    }
    if commitment.is_stalled() {
        out.push_str(&format!(
            "  stalled: {} consecutive episodes made no progress\n",
            commitment.consecutive_stalls
        ));
    }
    if !commitment.criteria.is_empty() {
        out.push_str("  criteria:\n");
        for criterion in &commitment.criteria {
            out.push_str(&format!(
                "    [{}] {} ({})\n",
                if criterion.passed() { "pass" } else { "open" },
                criterion.statement,
                criterion
                    .strength
                    .map(|s| s.as_str())
                    .unwrap_or("not evaluated"),
            ));
        }
    }
    if let Some(closure) = &commitment.closure {
        out.push_str(&format!(
            "  closed {} on {} evidence — {}\n",
            closure.verdict.as_str(),
            closure.strength.as_str(),
            closure.note
        ));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use vak_commit::{Economics, Event, EventKind};
    use vak_intent::{Evidence, Horizon, Reading, Satisfaction};
    use vak_session::types::{CriterionKind, CriterionResult, WorkCriterion};
    use vak_tools::Tool;

    fn ctx() -> vak_tools::ToolContext {
        vak_tools::ToolContext::new(std::path::PathBuf::from("/tmp"))
    }

    fn open(dir: &std::path::Path, evidence: Evidence) -> String {
        let reading = Reading {
            horizon: Horizon::Durable,
            evidence,
            ..Reading::general()
        };
        CommitmentLedger::new(dir)
            .open_commitment(vak_commit::spec_from_reading(
                "migrate the billing schema",
                reading,
                vec![WorkCriterion {
                    criterion_id: "tests".into(),
                    statement: "cargo test passes".into(),
                    kind: CriterionKind::Shell {
                        command: "cargo test".into(),
                    },
                    required: true,
                }],
                dir.to_path_buf(),
                Economics::default(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn an_empty_portfolio_says_so_plainly() {
        let dir = tempfile::tempdir().unwrap();
        let tool = CommitmentsTool {
            sessions_home: dir.path().to_path_buf(),
        };
        let out = tool.execute(&json!({}), &ctx()).await;
        assert!(!out.is_error);
        assert!(out.content.contains("No commitments"));
    }

    /// The evidence gap is the fact worth volunteering: it separates work that
    /// is finished from work that merely looks finished.
    #[tokio::test]
    async fn the_listing_names_the_evidence_a_commitment_still_needs() {
        let dir = tempfile::tempdir().unwrap();
        open(dir.path(), Evidence::Verified);
        let tool = CommitmentsTool {
            sessions_home: dir.path().to_path_buf(),
        };
        let out = tool.execute(&json!({}), &ctx()).await;
        assert!(
            out.content.contains("needs observed evidence"),
            "{}",
            out.content
        );
        // And it tells the model, in the result, that it cannot close this
        // itself — the separation of powers is stated where it is relevant.
        assert!(out.content.contains("cannot mark a criterion passed"));
    }

    #[tokio::test]
    async fn detail_reports_criteria_and_their_strength() {
        let dir = tempfile::tempdir().unwrap();
        let id = open(dir.path(), Evidence::Verified);
        CommitmentLedger::new(dir.path())
            .append(&Event::new(
                &id,
                EventKind::CriterionEvaluated {
                    criterion_id: "tests".into(),
                    result: CriterionResult::Passed {
                        evidence: "exit 0".into(),
                    },
                    strength: Satisfaction::Observed,
                },
            ))
            .unwrap();
        let tool = CommitmentsTool {
            sessions_home: dir.path().to_path_buf(),
        };
        let out = tool.execute(&json!({ "id": &id[..8] }), &ctx()).await;
        assert!(out.content.contains("cargo test passes"));
        assert!(out.content.contains("observed"));
    }

    #[tokio::test]
    async fn an_ambiguous_prefix_names_nothing_rather_than_guessing() {
        let dir = tempfile::tempdir().unwrap();
        open(dir.path(), Evidence::None);
        open(dir.path(), Evidence::None);
        let tool = CommitmentsTool {
            sessions_home: dir.path().to_path_buf(),
        };
        // uuid v7 ids share a time-ordered prefix, so a one-character prefix
        // is genuinely ambiguous here.
        let out = tool.execute(&json!({ "id": "0" }), &ctx()).await;
        assert!(out.is_error);
        assert!(out.content.contains("longer prefix"));
    }

    #[tokio::test]
    async fn an_unknown_id_is_an_error_value_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let tool = CommitmentsTool {
            sessions_home: dir.path().to_path_buf(),
        };
        let out = tool.execute(&json!({ "id": "nope" }), &ctx()).await;
        assert!(out.is_error);
    }
}
