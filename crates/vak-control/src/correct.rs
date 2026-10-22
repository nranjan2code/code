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
    reframed: BTreeSet<Cause>,
}

impl Ladder {
    /// The rung for a check that just failed. `can_reframe` says whether
    /// the next step's frame can be changed for it (a tool withdrawn, the
    /// output form constrained).
    pub fn step(&mut self, cause: Cause, can_reframe: bool) -> Rung {
        if self.told.insert(cause) {
            Rung::Tell
        } else if can_reframe {
            self.reframed.insert(cause);
            Rung::Reframe
        } else {
            Rung::End
        }
    }

    /// The check passed: a later failure of it is a new one, told again.
    pub fn passed(&mut self, cause: Cause) {
        self.told.remove(&cause);
        self.reframed.remove(&cause);
    }

    /// Whether this check already changed the frame this turn.
    pub fn reframed(&self, cause: Cause) -> bool {
        self.reframed.contains(&cause)
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

/// What a card that shows the files this turn changed is told when it
/// cannot: no file changed, or it named a file that did not. `changed` is
/// every file the turn changed, as the calls named them.
pub fn card_files_unchanged(asked: &[String], changed: &[String]) -> String {
    if changed.is_empty() {
        return "Not shown: no call in this turn changed a text file. This card takes what \
                changed from the files a call wrote; write or edit the file first."
            .to_string();
    }
    let quoted = |names: &[String]| {
        names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "Not shown: no call in this turn changed {}. The files it changed are {}: name them \
         in `paths`, or leave `paths` out to show them all.",
        quoted(asked),
        quoted(changed)
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
        assert!(ladder.reframed(Cause::Repair));
        ladder.passed(Cause::Repair);
        assert!(!ladder.reframed(Cause::Repair));
        assert_eq!(ladder.step(Cause::Plan, false), Rung::Tell);
        assert!(ladder.told(Cause::Plan) && !ladder.told(Cause::EmptyStep));
        ladder.passed(Cause::Fence);
        assert_eq!(ladder.step(Cause::Fence, false), Rung::Tell);
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
