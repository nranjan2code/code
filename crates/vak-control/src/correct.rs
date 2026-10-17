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
    EmptyStep {
        /// What the step thought, as it arrived; empty when nothing did.
        /// Quoted back, because not every wire shows a model its own
        /// thinking again.
        thinking: String,
        /// A call already succeeded this turn: its result stands.
        done: bool,
    },
    /// A card fence in the answer does not parse.
    Fence { error: String },
    /// The declared plan has deliverables with no receipt.
    Plan {
        /// Planned cards no card's `deliverable` names.
        cards_unnamed: Vec<String>,
        /// Cards shown that name no planned card.
        cards_unattributed: usize,
        /// Planned cards with no card at all.
        cards_short: usize,
        /// Every other deliverable with no receipt, with its form.
        others: Vec<(String, String)>,
    },
    /// A planned card is still not shown, and the next step is framed to
    /// call the card tool of the kind the plan named for it.
    CardRequired { name: String, tool: String },
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
    /// The person declined a call of each of these tools, and the tools
    /// are out of the turn.
    Declined { tools: Vec<String> },
}

impl Finding {
    /// The check this finding belongs to.
    pub fn cause(&self) -> Cause {
        match self {
            Self::ToolCallFormat { .. } => Cause::ToolCallRepair,
            Self::EmptyStep { .. } => Cause::EmptyStep,
            Self::Fence { .. } => Cause::Fence,
            Self::Plan { .. } | Self::CardRequired { .. } => Cause::Plan,
            Self::StopHook { .. } => Cause::StopHook,
            Self::StopGuard { .. } => Cause::StopGuard,
            Self::Truncated { .. } => Cause::Truncated,
            Self::VerificationStale => Cause::VerificationStale,
            Self::KeepWorking => Cause::KeepWorking,
            Self::UnresolvedFailure { .. } => Cause::UnresolvedFailure,
            Self::WorkersRunning { .. } => Cause::WorkersRunning,
            Self::SchemaShown { .. } | Self::Withdrawn { .. } | Self::Declined { .. } => {
                Cause::Repair
            }
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

/// The result of a call to a name this turn does not offer: what may be
/// called instead, and how an MCP capability is reached when the `mcp`
/// broker is offered.
/// A name Vak itself retired is told what replaced it (`retired`), never
/// rewritten into another tool.
pub fn unknown_tool(name: &str, offered: &[&str], via_mcp: bool, retired: Option<&str>) -> String {
    let quoted = serde_json::to_string(name).unwrap_or_else(|_| "\"invalid\"".into());
    let available = serde_json::to_string(offered).unwrap_or_else(|_| "[]".into());
    let advice = if let Some(replacement) = retired {
        let replacement = serde_json::to_string(replacement).unwrap_or_else(|_| "\"\"".into());
        format!(r#","retired":true,"use_instead":{replacement}"#)
    } else if via_mcp {
        format!(
            r#","recovery_advice":"Tool '{name}' is an external MCP capability. Invoke it via the 'mcp' tool: mcp(action: \"call\", server: \"<server_name>\", tool: \"{name}\", arguments: {{ ... }})""#
        )
    } else {
        String::new()
    };
    format!(
        r#"{{"type":"unknown_capability","requested_kind":"tool","name":{quoted},"available_tools":{available}{advice}}}{RECOVERY_NOTE}"#
    )
}

/// The result of calling a skill as if it were a tool: the invocation that
/// reaches it.
pub fn skill_not_tool(name: &str, skill: &str) -> String {
    let quoted = serde_json::to_string(name).unwrap_or_else(|_| "\"invalid\"".into());
    let arguments = serde_json::json!({ "name": skill });
    format!(
        r#"{{"type":"capability_kind_mismatch","name":{quoted},"actual_kind":"skill","invocation":{{"tool":"skill","arguments":{arguments}}}}}
[recovery] `{name}` is registered as a skill, not a callable tool. The runtime supplied the correct invocation: call `skill` with arguments `{arguments}` if that capability is needed. Otherwise continue with the admitted tools. Do not retry `{name}` as a tool."#
    )
}

/// Why a card is not shown: it cites source URLs nothing in the
/// conversation retrieved or was given.
pub fn uncited_source(urls: &[String]) -> String {
    format!(
        "[uncited-source]: not shown — this card cites {} which nothing in this conversation \
         retrieved or was given. A card cites only the exact URLs of pages a tool fetched here. \
         Fetch the page first. If it cannot be fetched, give the link in your text, say what \
         could not be verified, and call `plan` again with that deliverable's form as text.",
        urls.iter()
            .map(|url| format!("`{url}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// What the step that writes a call's arguments again is asked: the call as
/// the model wrote it is in front of it, and only its encoding is to change
/// (docs/design/89-constrained-tool-arguments.md §4.3).
pub fn regenerate_arguments(tool: &str, fault: &str) -> String {
    format!(
        "[arguments]: The arguments of that `{tool}` call could not be read: {fault}. Write the \
         same call's arguments again as JSON that follows the tool's schema, keeping every value \
         you meant to send. This is not a message from the person."
    )
}

/// What a card is told when its `semantic_type` is a kind another card tool
/// shows.
pub fn card_kind_elsewhere(kind: &str, owner: &str) -> String {
    format!(
        "[card-kind]: not shown — `{kind}` is a kind of `{owner}`, not of this card. Call \
         `{owner}` with `semantic_type` `{kind}` and that tool's own fields."
    )
}

/// What a card that previews a file is told when no such file exists: in
/// the workspace, or among this Agent's drafts. Found live 2026-10-13: a
/// preview was asked for markup no file held, and a path was supplied anyway.
pub fn card_file_missing(path: &str) -> String {
    format!(
        "Not shown: `{path}` is not a file in the workspace or among your drafts. \
         A file preview shows a file that exists; write it first, or show the content \
         another way."
    )
}

/// What a card is told when it shows the output of a command no call in
/// the conversation ran.
/// A card that takes its data from an ability named one no call in the
/// conversation answered (docs/design/93 R5).
pub fn card_ability_not_answered(ability: &str, answered: &[String]) -> String {
    let named = if answered.is_empty() {
        "No ability has answered in this conversation yet: call it first.".to_string()
    } else {
        let list: Vec<String> = answered.iter().map(|name| format!("`{name}`")).collect();
        format!(
            "The abilities that answered are {}: name one of them exactly, or call this one first.",
            list.join(", ")
        )
    };
    format!(
        "Not shown: no call in this conversation to `{ability}` answered. This card takes its \
         data from the latest answer of the ability it names. {named}"
    )
}

/// A card that takes its data from an ability's answer named a place in
/// it that holds no list of records.
pub fn card_data_not_found(ability: &str, list: &str, keys: &[String]) -> String {
    let at = if list.is_empty() {
        "the answer itself".to_string()
    } else {
        format!("`{list}`")
    };
    let keys = if keys.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = keys.iter().map(|key| format!("`{key}`")).collect();
        format!(" Its answer's keys are {}.", list.join(", "))
    };
    format!(
        "Not shown: {at} in `{ability}`'s latest answer is not a list of records. Name where \
         the list is with `list` (a key, or keys joined by dots).{keys}"
    )
}

pub fn card_command_not_run(command: &str, ran: &[String]) -> String {
    let named = if ran.is_empty() {
        "No command has run in this turn yet: run it first.".to_string()
    } else {
        let list: Vec<String> = ran.iter().map(|command| format!("`{command}`")).collect();
        format!(
            "The commands that ran in this turn are {}: name one of them exactly, or run this one first.",
            list.join(", ")
        )
    };
    format!(
        "Not shown: no call in this conversation ran `{command}`. This card takes what a \
         command printed and how it ended from the call that ran it. {named}"
    )
}

/// What a card is told when its `deliverable` names a deliverable the plan
/// does not list: the card is not shown. The plan's cards are given in two
/// lists, those still to show and those already shown, which stay shown.
/// Found live 2026-10-14: given every planned card under "the cards it
/// lists", a model sent the ones already shown again.
pub fn card_unplanned(answers: &str, open: &[String], shown: &[String]) -> String {
    let quoted = |names: &[String]| {
        names
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let open = if open.is_empty() && shown.is_empty() {
        "Your plan lists no card.".to_string()
    } else if open.is_empty() {
        "Every card in your plan is shown already.".to_string()
    } else {
        format!(
            "Your plan's cards not shown yet are {}: if this card is one of them, call it \
             again with `deliverable` set to that name.",
            quoted(open)
        )
    };
    let shown = if shown.is_empty() {
        String::new()
    } else {
        format!(
            " {} are shown already and stay shown: do not send them again.",
            quoted(shown)
        )
    };
    format!(
        "[card-plan]: not shown — `deliverable` is \"{answers}\", which is not a card in your \
         plan. {open}{shown} A card your plan does not list is called with `deliverable` left out."
    )
}

/// What a card is told when it names a `deliverable` and the answer has no
/// plan: there is nothing for it to name.
pub fn card_without_plan(answers: &str) -> String {
    format!(
        "[card-plan]: not shown — `deliverable` is \"{answers}\", but this answer has no plan. \
         Call it again with `deliverable` left out."
    )
}

/// What a card is told when its `replaces` names no card shown in this
/// answer: the card is not shown, and the answer's cards are given by id.
pub fn card_replaces_unknown(named: &str, shown: &[String]) -> String {
    let ids = if shown.is_empty() {
        "No card is shown in this answer yet, so leave `replaces` out.".to_string()
    } else {
        format!(
            "The cards shown in this answer are {}: name one of them, or leave `replaces` out \
             for a new card.",
            shown
                .iter()
                .map(|id| format!("`{id}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "[card-plan]: not shown — `replaces` is `{named}`, which is no card in this answer. {ids}"
    )
}

/// What a call stopped at its time limit is told: below the operator's
/// longest limit it may ask for more; at it, asking again changes nothing.
pub fn limit_reached(named: u64, ceiling: u64) -> String {
    if named >= ceiling {
        format!(
            "\n[limit]: {ceiling}ms is the longest the operator allows a command here, and \
             the call already ran that long; it did not finish. Do less at a time, or say \
             that it could not finish within the limit."
        )
    } else {
        format!(
            "\n[limit]: the call may name a longer limit, up to {ceiling}ms (the operator's \
             longest), or do less at a time."
        )
    }
}

/// A card that names no deliverable while the plan's cards are not all
/// shown: the plan check pairs a card with its deliverable only by name.
pub fn card_unnamed(open: &[String]) -> String {
    let names = open
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "[card-plan]: not shown — this card sets no `deliverable`, and your plan's cards \
         not shown yet are {names}. Call it again with `deliverable` set to the one this card \
         delivers. A card your plan does not list goes after them."
    )
}

/// What the model is told about a failed check, signed with the check's
/// marker. It says who is speaking, because a model reads an unsigned
/// reminder as the person's.
pub fn render(finding: &Finding, marker: &str) -> String {
    match finding {
        Finding::ToolCallFormat { reason } => format!(
            "{marker}: The inference provider could not parse the structured tool call it generated ({reason}). This was an output-format failure before any tool ran. Continue the same request from the admitted tool schemas and regenerate valid structured arguments. Do not repeat the malformed envelope. If the request needs several outputs, complete them with the admitted tools; if no tool call is needed, answer directly."
        ),
        Finding::EmptyStep { thinking, done } => {
            // Found live 2026-10-13: told to act on thinking the Ollama
            // wire never shows back, a model redid the request from its
            // start, rewriting a file it had already written, to the step
            // limit. The thinking is quoted as the evidence.
            let next = if thinking.trim().is_empty() {
                "Nothing arrived from your last response: no visible answer and no \
                 tool call. A tool call whose arguments are not valid JSON can be \
                 dropped before it reaches Vak, so if you made one, make it again with \
                 valid JSON arguments that follow its schema. Or write the answer as \
                 text."
                    .to_string()
            } else {
                format!(
                    "Your last response held only thinking. Thinking reaches neither \
                     the person nor Vak, and it may not be shown to you again, so here \
                     it is:\n<thinking>\n{}\n</thinking>\nIf it holds your answer, \
                     write that answer as text. If it planned a call, make that call.",
                    thinking.trim()
                )
            };
            let stands = if *done {
                " The calls that already succeeded in this turn stand; do not make \
                 them again."
            } else {
                ""
            };
            format!(
                "{marker}: {next}{stands} This message is from Vak; it is not a new \
                 request."
            )
        }
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
                    "Planned as cards and named by no card's `deliverable`: {named}. Cards shown without \
                     a planned name in `deliverable`: {cards_unattributed}. So {cards_short} planned card(s) are not shown yet."
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
            parts.push(
                "Show each missing card with its card tool, `deliverable` set to its planned name, \
                 or finish and say plainly why it is not needed. Keep everything already shown \
                 and do not plan again."
                    .to_string(),
            );
            parts.join(" ")
        }
        Finding::CardRequired { name, tool } => format!(
            "{marker}: Your plan lists \"{name}\" as a card and no card shows it. This step \
             can only call `{tool}`: show it now with `deliverable` set to \"{name}\". This is not \
             a message from the person."
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
            "{marker}: The person started this request with /until-done, so it stays open until \
             they send the message \"done\". Your answer above was shown to them and stands. Go on with the next \
             useful part of the same request; if nothing useful is left, say so in one line \
             without calling a tool, and the request waits for them, still open. Anything they \
             send reaches you between steps. This is not a message from the person."
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
                "{marker} The same tool call failed twice. Do not repeat the failing call; send \
                 exactly the arguments the schema below asks for, naming only what exists, or \
                 deliver this part another way."
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
        Finding::Declined { tools } => {
            let gone = tools
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{marker} {gone} is withdrawn for the rest of this turn: the person declined a \
                 call of it. Do not try it again. Answer with what you already have, or say \
                 what you could not do without it, and keep everything you have already \
                 produced."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retired_name_is_told_what_replaced_it_and_lists_what_exists() {
        let told = unknown_tool(
            "python_eval",
            &["bash", "read"],
            false,
            Some("bash — run Python"),
        );
        assert!(told.contains(r#""retired":true"#), "{told}");
        assert!(
            told.contains(r#""use_instead":"bash — run Python""#),
            "{told}"
        );
        assert!(
            told.contains(r#""available_tools":["bash","read"]"#),
            "{told}"
        );
        let unknown = unknown_tool("read_file", &["read"], false, None);
        assert!(!unknown.contains("retired"), "{unknown}");
    }

    #[test]
    fn a_limit_below_the_maximum_may_ask_for_more_and_one_at_it_may_not() {
        assert!(limit_reached(5_000, 600_000).contains("up to 600000ms"));
        assert!(limit_reached(5_000, 5_000).contains("already ran that long"));
    }

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
            told.contains("without a planned name in `deliverable`: 1"),
            "{told}"
        );
        assert!(told.contains("\"fix\" (change)"), "{told}");
    }

    /// The thinking a thinking-only step wrote is quoted back whole, and
    /// a step with none is told a call may have been dropped.
    #[test]
    fn an_empty_step_quotes_the_thinking_it_had() {
        let told = render(
            &Finding::EmptyStep {
                thinking: "The file exists. I should tell the person.".into(),
                done: true,
            },
            "[empty-step]",
        );
        assert!(
            told.contains("<thinking>\nThe file exists. I should tell the person.\n</thinking>")
        );
        assert!(told.contains("do not make them again"), "{told}");
        let told = render(
            &Finding::EmptyStep {
                thinking: " ".into(),
                done: false,
            },
            "[empty-step]",
        );
        assert!(told.contains("dropped before it reaches Vak"), "{told}");
        assert!(!told.contains("<thinking>"));
    }

    #[test]
    fn every_finding_is_signed_with_its_marker() {
        let findings = [
            Finding::ToolCallFormat { reason: "x".into() },
            Finding::EmptyStep {
                thinking: "x".into(),
                done: true,
            },
            Finding::EmptyStep {
                thinking: String::new(),
                done: false,
            },
            Finding::Fence { error: "x".into() },
            Finding::CardRequired {
                name: "x".into(),
                tool: "x".into(),
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
            Finding::Declined {
                tools: vec!["x".into()],
            },
        ];
        for finding in findings {
            assert!(render(&finding, "[m]").starts_with("[m]"), "{finding:?}");
        }
    }
}
