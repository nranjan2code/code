//! Corrections (docs/design/88-control-plane.md §5): what a failed check
//! observed, the one ladder that decides what follows, and the one place a
//! correction's wording for the model is written.

use std::collections::BTreeSet;

use crate::{Cause, Rung};

/// A tool whose schema is shown again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownTool {
    pub name: String,
    pub description: String,
    /// The tool's input schema, as JSON.
    pub schema: String,
}

/// What a failed check observed: the facts a correction is rendered from.
/// Each is something the runtime saw, never a reading of words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// The provider could not parse the tool call the model generated.
    ToolCallFormat { reason: String },
    /// The response carried neither text nor a call.
    EmptyStep,
    /// A card fence in the answer does not parse.
    Fence { error: String },
    /// The declared plan has deliverables with no receipt.
    Plan {
        /// Planned cards no card's `answers` names.
        cards_unnamed: Vec<String>,
        /// Cards shown that name no planned card.
        cards_unattributed: usize,
        /// Planned cards with no card at all.
        cards_short: usize,
        /// Every other deliverable with no receipt, with its form.
        others: Vec<(String, String)>,
        /// The tool the whole answer is given with, when the turn offers
        /// one: giving it again replaces the answer already shown.
        answer: Option<String>,
    },
    /// Planned cards are still not shown, and the next step is framed to
    /// give the answer as its one document.
    AnswerRequired { tool: String, missing: Vec<String> },
    /// A stop hook the operator configured asked for more work.
    StopHook { reason: String },
    /// A completion gate with its own reason (goal mode, managed work).
    StopGuard { reason: String },
    /// The answer ends inside an unclosed block.
    Truncated { what: String },
    /// Files changed after the last check ran.
    VerificationStale,
    /// The person asked, by command, for work to go on until they say done.
    KeepWorking,
    /// A call failed and no later call of its tool succeeded.
    UnresolvedFailure { tool: String, error: String },
    /// Background workers the turn started are still running.
    WorkersRunning { workers: Vec<String> },
    /// Calls of these tools failed in two steps running: their schemas, and
    /// for a name that is no tool, the tools there are.
    SchemaShown {
        tools: Vec<ShownTool>,
        unknown: Vec<String>,
        offered: Vec<String>,
    },
    /// Calls of these tools failed again after their schema was shown, and
    /// the tools are out of the turn.
    Withdrawn { tools: Vec<String> },
}

impl Finding {
    /// The check this finding belongs to.
    pub fn cause(&self) -> Cause {
        match self {
            Self::ToolCallFormat { .. } => Cause::ToolCallRepair,
            Self::EmptyStep => Cause::EmptyStep,
            Self::Fence { .. } => Cause::Fence,
            Self::Plan { .. } | Self::AnswerRequired { .. } => Cause::Plan,
            Self::StopHook { .. } => Cause::StopHook,
            Self::StopGuard { .. } => Cause::StopGuard,
            Self::Truncated { .. } => Cause::Truncated,
            Self::VerificationStale => Cause::VerificationStale,
            Self::KeepWorking => Cause::KeepWorking,
            Self::UnresolvedFailure { .. } => Cause::UnresolvedFailure,
            Self::WorkersRunning { .. } => Cause::WorkersRunning,
            Self::SchemaShown { .. } | Self::Withdrawn { .. } => Cause::Repair,
        }
    }
}

/// The correction ladder for one turn. A check that fails is told once,
/// with the evidence; if it fails again the frame changes where it can, and
/// otherwise the turn goes on with what exists. No count decides a rung:
/// only whether this check was already told.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ladder {
    told: BTreeSet<Cause>,
}

impl Ladder {
    /// The rung for a check that just failed. `can_reframe` says whether
    /// the next step's frame can be changed for it (a tool withdrawn, the
    /// output form constrained).
    pub fn step(&mut self, cause: Cause, can_reframe: bool) -> Rung {
        if self.told.insert(cause) {
            Rung::Tell
        } else if can_reframe {
            Rung::Reframe
        } else {
            Rung::End
        }
    }

    /// The check passed: a later failure of it is a new one, told again.
    pub fn passed(&mut self, cause: Cause) {
        self.told.remove(&cause);
    }

    /// Whether this check was already told this turn.
    pub fn told(&self, cause: Cause) -> bool {
        self.told.contains(&cause)
    }
}

/// The note a failed call's result carries when another call can repair it.
pub const RECOVERY_NOTE: &str = "\n[recovery] Treat this as a failed attempt. Inspect the error and the admitted tool/schema inventory, then make at most one corrected or alternative call. Do not repeat identical arguments. If the failure is environmental or the corrected call is unsafe, explain the blocker instead.";

/// What the model is told about a failed check, signed with the check's
/// marker. It says who is speaking, because a model reads an unsigned
/// reminder as the person's.
pub fn render(finding: &Finding, marker: &str) -> String {
    match finding {
        Finding::ToolCallFormat { reason } => format!(
            "{marker}: The inference provider could not parse the structured tool call it generated ({reason}). This was an output-format failure before any tool ran. Continue the same request from the admitted tool schemas and regenerate valid structured arguments. Do not repeat the malformed envelope. If the request needs several outputs, complete them with the admitted tools; if no tool call is needed, answer directly."
        ),
        Finding::EmptyStep => format!(
            "{marker}: Nothing arrived from your last response: no visible answer \
             and no tool call. A tool call whose arguments are not valid JSON can be \
             dropped before it reaches Vak, so if you made one, make the same call again \
             with valid JSON arguments that follow its schema. This is the request in \
             the person's message that opened this turn, not a new one. Or write the \
             answer as text."
        ),
        Finding::Fence { error } => format!(
            "{marker}: The ```vak card block in your last answer has invalid JSON and failed to parse \
             ({error}). Resend the same answer with a syntactically valid JSON body this time — \
             double-check every object/array is closed and every key is quoted. If you can't produce \
             valid JSON for it, drop the fence and answer in plain prose instead."
        ),
        Finding::Plan {
            cards_unnamed,
            cards_unattributed,
            cards_short,
            others,
            answer,
        } => {
            let mut parts = vec![format!(
                "{marker}: Vakyartha's runtime compared your plan with what this turn \
                 produced. This is not a message from the person, and it is sent once."
            )];
            if *cards_short > 0 {
                let named = cards_unnamed
                    .iter()
                    .map(|name| format!("\"{name}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                parts.push(format!(
                    "Planned as cards and named by no card's `answers`: {named}. Cards shown without \
                     a planned name in `answers`: {cards_unattributed}. So {cards_short} planned card(s) are not shown yet."
                ));
            }
            if !others.is_empty() {
                let listed = others
                    .iter()
                    .map(|(name, form)| format!("\"{name}\" ({form})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                parts.push(format!("Nothing produced yet for: {listed}."));
            }
            parts.push(match answer {
                Some(tool) => format!(
                    "Give the whole answer again with `{tool}`: every block it had and what is \
                     missing, each block's `answers` set to its planned name. It replaces the \
                     answer already shown. Or finish and say plainly why it is not needed. Do \
                     not plan again."
                ),
                None => "Produce what is missing, or finish and say plainly why it is not \
                         needed. Keep everything already produced and do not plan again."
                    .to_string(),
            });
            parts.join(" ")
        }
        Finding::AnswerRequired { tool, missing } => format!(
            "{marker}: Your plan lists {} as cards and none shows them. This step can only \
             call `{tool}`: give your whole answer with it now, one block for each, with \
             `answers` set to its planned name. This is not a message from the person.",
            missing
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Finding::StopHook { reason } => format!(
            "{marker}: a stop hook asked for more work: {}. Do it now, or say plainly why you cannot.",
            reason.trim().trim_end_matches('.')
        ),
        Finding::StopGuard { reason } => format!("{marker}: {reason}"),
        Finding::Truncated { what } => format!(
            "{marker}: your last message appears cut off ({what}). Finish it now; if you are \
             actually done, say so plainly."
        ),
        Finding::VerificationStale => format!(
            "{marker}: files changed after the last check ran. Run the check again before \
             finishing, or say plainly why it cannot be run."
        ),
        Finding::KeepWorking => format!(
            "{marker}: the user asked you to keep working until they say done. Continue making \
             useful progress; do not declare completion yet."
        ),
        Finding::UnresolvedFailure { tool, error } => format!(
            "{marker}: the `{tool}` call failed: {error}. Fix the cause and try again, or tell the \
             person plainly that it failed and why, quoting the error."
        ),
        Finding::WorkersRunning { workers } => format!(
            "{marker}: workers you started are still running: {}. Wait for them with the workers \
             tool (action wait) and use their results, or stop them. They are cancelled if you \
             finish now.",
            workers.join(", ")
        ),
        Finding::SchemaShown {
            tools,
            unknown,
            offered,
        } => {
            let mut parts = vec![format!(
                "{marker} The same tool call failed twice. Do not repeat the failing call shape; \
                 send exactly the arguments the schema below asks for, or deliver this part \
                 another way."
            )];
            for tool in tools {
                parts.push(format!(
                    "\nAdmitted tool `{}` (re-surfaced verbatim):\ndescription: {}\ninput_schema: {}",
                    tool.name, tool.description, tool.schema
                ));
            }
            for name in unknown {
                parts.push(format!(
                    "\nTool `{name}` is not in the admitted set for this turn. Admitted \
                     tools: {}. Re-issue using an admitted tool.",
                    offered.join(", ")
                ));
            }
            parts.join("\n\n")
        }
        Finding::Withdrawn { tools } => {
            let gone = tools
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{marker} {gone} is withdrawn for the rest of this turn: its calls kept failing \
                 after its schema was shown. Do not call it again. Deliver that part another \
                 way, with another tool or as plain text in your reply, and keep everything \
                 you have already produced."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_is_told_once_then_reframed_or_left_to_stand() {
        let mut ladder = Ladder::default();
        assert_eq!(ladder.step(Cause::Fence, false), Rung::Tell);
        assert_eq!(ladder.step(Cause::Fence, false), Rung::End);
        assert_eq!(ladder.step(Cause::Repair, true), Rung::Tell);
        assert_eq!(ladder.step(Cause::Repair, true), Rung::Reframe);
        assert_eq!(ladder.step(Cause::Plan, false), Rung::Tell);
        assert!(ladder.told(Cause::Plan) && !ladder.told(Cause::EmptyStep));
        ladder.passed(Cause::Fence);
        assert_eq!(ladder.step(Cause::Fence, false), Rung::Tell);
    }

    #[test]
    fn a_plan_correction_names_what_was_seen_and_who_is_speaking() {
        let told = render(
            &Finding::Plan {
                cards_unnamed: vec!["comparison".into(), "plan".into()],
                cards_unattributed: 1,
                cards_short: 1,
                others: vec![("fix".into(), "change".into())],
                answer: Some("answer".into()),
            },
            "[plan-check]",
        );
        assert!(
            told.starts_with("[plan-check]: Vakyartha's runtime"),
            "{told}"
        );
        assert!(told.contains("not a message from the person"), "{told}");
        assert!(told.contains("\"comparison\", \"plan\""), "{told}");
        assert!(
            told.contains("without a planned name in `answers`: 1"),
            "{told}"
        );
        assert!(told.contains("\"fix\" (change)"), "{told}");
    }

    #[test]
    fn every_finding_is_signed_with_its_marker() {
        let findings = [
            Finding::ToolCallFormat { reason: "x".into() },
            Finding::EmptyStep,
            Finding::Fence { error: "x".into() },
            Finding::AnswerRequired {
                tool: "x".into(),
                missing: vec!["x".into()],
            },
            Finding::StopHook {
                reason: "x.".into(),
            },
            Finding::StopGuard { reason: "x".into() },
            Finding::Truncated { what: "x".into() },
            Finding::VerificationStale,
            Finding::KeepWorking,
            Finding::UnresolvedFailure {
                tool: "x".into(),
                error: "x".into(),
            },
            Finding::WorkersRunning {
                workers: vec!["x".into()],
            },
            Finding::SchemaShown {
                tools: Vec::new(),
                unknown: vec!["x".into()],
                offered: vec!["read".into()],
            },
            Finding::Withdrawn {
                tools: vec!["x".into()],
            },
        ];
        for finding in findings {
            assert!(render(&finding, "[m]").starts_with("[m]"), "{finding:?}");
        }
    }
}
