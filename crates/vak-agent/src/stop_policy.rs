//! Built-in stop gate: blocks premature completions.
//!
//! Small models frequently end their turn mid-plan ("Fixing both:" /
//! "Now I'll write the tests") or finish without doing the work the request
//! asked for. The dogfood campaign measured this at 5/7 runs on a free tier.
//! This policy reuses the existing stop-hook continuation machinery (append
//! "[stop-guard]: reason / Please continue.", emit StopHookContinuation) at
//! most `max_blocks` times per run, so it can never trap a model in a loop.
//!
//! What completion needs comes from the admitted reading (`OutcomeSpec`) and
//! the run's receipts, never from phrases in the request: a phrase list read
//! "run a quick grammar check" as a demand for a shell command and "I'll see
//! you on Sunday" at the end of a letter as an unfinished plan, and it only
//! ever spoke English.

/// What triggered the block — also the operator-visible reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    /// Final text of effect work looks truncated: ends with ':', a bare plan
    /// marker, or an unclosed fenced code block.
    TruncatedPlan(String),
    /// The reading demands proof, code changed, and nothing ran it.
    VerificationMissing,
    /// A file-changing tool ran after the last verification command.
    VerificationStale,
    /// The user explicitly asked the agent to continue until a later user
    /// message authorizes completion.
    UserCompletionRequired,
    /// The request requires execution or tool receipts, but none were produced.
    ExecutionReceiptMissing { act: String, hint: String },
    /// A tool failed with an error and the model neither repaired it nor reported the blocker.
    UnresolvedToolFailure { tool: String, error: String },
}

impl BlockReason {
    pub fn message(&self) -> String {
        match self {
            BlockReason::TruncatedPlan(tail) => format!(
                "your last message appears cut off mid-plan (ends with {tail:?}). \
                 Finish the work now; if you are actually done, say so plainly."
            ),
            BlockReason::VerificationMissing => String::from(
                "the request asks for this to be proven, and code changed without being run. \
                 Run the changed code or its checks now; if it cannot be run here, say plainly why.",
            ),
            BlockReason::VerificationStale => String::from(
                "files changed after the last check ran. Run the check again before finishing, \
                 or say plainly why it cannot be run.",
            ),
            BlockReason::UserCompletionRequired => String::from(
                "the user asked you to keep working until they say done. Continue making \
                 useful progress; do not declare completion yet.",
            ),
            BlockReason::ExecutionReceiptMissing { act, hint } => format!(
                "the request asks you to {act} ({hint}), but nothing this turn has done it yet. \
                 Do it now with the tools you have; if you cannot, say plainly what is missing.",
            ),
            BlockReason::UnresolvedToolFailure { tool, error } => format!(
                "the `{tool}` call failed: {error}. Fix the cause and try again, or tell the \
                 person plainly that it failed and why, quoting the error.",
            ),
        }
    }
}

/// Summary of tool execution receipts produced during a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReceiptSummary {
    /// Proven saved file from the same intent thread before a step-limit
    /// continuation. Only counts with a successful inspection in this turn.
    pub continued_saved_file: bool,
    /// Total tool invocations attempted.
    pub total_tool_calls: u32,
    /// Number of tool calls that completed with ToolRunOutput::Ok.
    pub successful_tool_calls: u32,
    /// Number of tool calls that completed with ToolRunOutput::Err.
    pub failed_tool_calls: u32,
    /// Number of substantive bash invocations.
    pub substantive_bash_calls: u32,
    /// Number of files modified/written (via edit, write, etc.).
    pub files_modified: u32,
    /// Number of code files modified/written.
    pub code_files_modified: u32,
    /// Number of documentation/content/non-code files modified/written.
    pub doc_files_modified: u32,
    /// Number of inspection/read tool calls (read_file, glob, grep, etc.).
    pub read_or_inspected: u32,
    /// Inspections that actually returned successfully. A prior saved file
    /// only counts after one of these in the continuation turn.
    pub successful_inspections: u32,
    /// Work that succeeded outside the built-in tools: an integration's tool
    /// invoked through `mcp` (`action = "call"`) or a delegated `task`,
    /// whose worker keeps its own receipts. Counted on success only.
    pub external_effects: u32,
    /// Most recent unresolved tool failure, if any.
    pub unresolved_error: Option<(String, String)>,
}

impl ReceiptSummary {
    pub fn has_execution_receipt(&self) -> bool {
        self.substantive_bash_calls > 0
            || self.files_modified > 0
            || self.external_effects > 0
            || (self.continued_saved_file && self.successful_inspections > 0)
    }

    pub fn has_inspection_receipt(&self) -> bool {
        self.read_or_inspected > 0 || self.has_execution_receipt()
    }

    pub fn has_any_receipt(&self) -> bool {
        self.successful_tool_calls > 0
    }
}

/// Whether the answer names the failure it leaves standing, by quoting the
/// error's own words: a distinctive identifier (`unknown_capability`) or a
/// pair of adjacent words ("disk full", "read-only mode", "exit code").
/// Keywords ("no issues", "error-free") read as a report and let a false
/// success through, and they only work in English; the error's words are
/// the same whatever language the rest of the answer is written in.
fn reports_blocker(text: &str, error: &str) -> bool {
    fn words(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    }
    let answer = words(text);
    let joined = format!(" {} ", answer.join(" "));
    let error: String = error.chars().take(600).collect();
    let error = words(&error);
    let identifier = error
        .iter()
        .any(|word| (word.contains('_') || word.chars().count() >= 12) && answer.contains(word));
    identifier
        || error.windows(2).any(|pair| {
            pair[0].chars().count() + pair[1].chars().count() >= 7
                && pair.iter().any(|word| word.chars().count() >= 4)
                && joined.contains(&format!(" {} {} ", pair[0], pair[1]))
        })
}

#[derive(Debug, Clone)]
pub struct StopPolicy {
    /// Gate on truncated-looking final messages.
    pub marker_gate: bool,
    /// Gate on promised-but-never-run verification.
    pub verify_gate: bool,
    /// Hard cap of guard continuations per run.
    pub max_blocks: u32,
}

impl Default for StopPolicy {
    fn default() -> Self {
        StopPolicy {
            marker_gate: true,
            verify_gate: true,
            max_blocks: 2,
        }
    }
}

impl StopPolicy {
    /// The request hands completion to the person: work continues until
    /// they release it with a `done`/`stop` message. Only phrases that name
    /// the person as the one who ends it count — "keep working until the
    /// tests pass" names a condition the runtime evaluates, and reading it
    /// as a user hold kept such turns running to `max_turns`.
    pub fn requires_user_completion(prompt: &str) -> bool {
        let p = prompt.to_lowercase();
        [
            "until i say done",
            "until i say so",
            "until i say stop",
            "until i tell you to stop",
            "until i tell you you're done",
            "until i tell you i'm done",
        ]
        .iter()
        .any(|marker| p.contains(marker))
    }

    pub fn is_done_message(message: &str) -> bool {
        let normalized = message.trim().to_ascii_lowercase();
        [
            "done",
            "stop",
            "you can stop",
            "that's enough",
            "that’s enough",
        ]
        .iter()
        .any(|marker| normalized == *marker)
    }

    /// Conservative trailing-intent patterns, only fired on line-final
    /// markers so normal prose summaries never match. Applied only to work
    /// whose reading needs a tool: in a letter or an agenda the last line is
    /// the deliverable, not a plan.
    fn truncated_plan(final_text: &str) -> Option<BlockReason> {
        let trimmed = final_text.trim_end();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.matches("```").count() % 2 == 1 {
            return Some(BlockReason::TruncatedPlan("unclosed code fence".into()));
        }
        let last_line = trimmed.lines().next_back()?.trim_end();
        if last_line.ends_with(':') && !last_line.starts_with('#') && last_line.len() < 200 {
            return Some(BlockReason::TruncatedPlan(format!(
                "'{}'",
                last_line
                    .chars()
                    .rev()
                    .take(40)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>()
            )));
        }
        const MARKERS: [&str; 8] = [
            "now i'll",
            "now let me",
            "let me ",
            "i will ",
            "i'll ",
            "going to ",
            "next,",
            "then,",
        ];
        let lower = last_line.to_ascii_lowercase();
        if lower.len() < 300
            && MARKERS
                .iter()
                .any(|m| lower.starts_with(m) || lower.contains(m))
            && !lower.ends_with('.')
            && !lower.ends_with('!')
            && !lower.ends_with('?')
        {
            return Some(BlockReason::TruncatedPlan("a plan marker".into()));
        }
        None
    }

    /// Whether this turn may end, judged from the admitted reading and the
    /// run's receipts. `prompt` is read only for the explicit user hold.
    pub fn evaluate_receipts(
        &self,
        prompt: &str,
        final_text: &str,
        outcome: Option<&vak_intent::OutcomeSpec>,
        receipts: &ReceiptSummary,
        verification_stale: bool,
    ) -> Option<BlockReason> {
        use vak_intent::StopProfile;
        if final_text.trim().is_empty() {
            return None;
        }
        if Self::requires_user_completion(prompt) {
            return Some(BlockReason::UserCompletionRequired);
        }
        let effect_work = outcome.is_none_or(|spec| spec.requires_tool());
        if self.marker_gate
            && effect_work
            && let Some(reason) = Self::truncated_plan(final_text)
        {
            return Some(reason);
        }
        if let Some((tool, error)) = &receipts.unresolved_error
            && !reports_blocker(final_text, error)
        {
            return Some(BlockReason::UnresolvedToolFailure {
                tool: tool.clone(),
                error: error.clone(),
            });
        }
        let stale_code = self.verify_gate && verification_stale && receipts.code_files_modified > 0;
        let Some(spec) = outcome else {
            // No reading: only the structural check survives — a check ran,
            // then code changed after it.
            return stale_code.then_some(BlockReason::VerificationStale);
        };
        if spec.stop == StopProfile::Verification {
            if !receipts.has_execution_receipt() {
                return Some(BlockReason::ExecutionReceiptMissing {
                    act: spec.deliverable_act().unwrap_or("verify").to_string(),
                    hint: "produce the check that proves it".into(),
                });
            }
            if self.verify_gate
                && receipts.code_files_modified > 0
                && receipts.substantive_bash_calls == 0
            {
                return Some(BlockReason::VerificationMissing);
            }
            return stale_code.then_some(BlockReason::VerificationStale);
        }
        if spec.stop == StopProfile::Effect || spec.requires_execution() {
            if !receipts.has_execution_receipt() {
                return Some(BlockReason::ExecutionReceiptMissing {
                    act: spec
                        .deliverable_act()
                        .unwrap_or("make the change")
                        .to_string(),
                    hint: "make the change itself, not a description of it".into(),
                });
            }
            return stale_code.then_some(BlockReason::VerificationStale);
        }
        // Material the request carried (an attachment, pasted text) leaves
        // no receipt, so a substantive direct answer satisfies a reading that
        // only needed something looked at.
        let direct_answer = final_text.trim().chars().count() >= 80;
        if spec.requires_inspection() && !receipts.has_inspection_receipt() && !direct_answer {
            return Some(BlockReason::ExecutionReceiptMissing {
                act: spec.deliverable_act().unwrap_or("look it up").to_string(),
                hint: "look at what it refers to".into(),
            });
        }
        if spec.requires_tool() && !receipts.has_any_receipt() && !direct_answer {
            return Some(BlockReason::ExecutionReceiptMissing {
                act: spec.deliverable_act().unwrap_or("use a tool").to_string(),
                hint: "use the tool it needs".into(),
            });
        }
        None
    }

    /// Returns Some(reason) when completion should be blocked.
    pub fn evaluate(
        &self,
        prompt: &str,
        final_text: &str,
        bash_calls_this_run: u32,
    ) -> Option<BlockReason> {
        let receipts = ReceiptSummary {
            substantive_bash_calls: bash_calls_this_run,
            successful_tool_calls: if bash_calls_this_run > 0 {
                bash_calls_this_run
            } else {
                0
            },
            ..Default::default()
        };
        self.evaluate_receipts(prompt, final_text, None, &receipts, false)
    }

    pub fn evaluate_with_state(
        &self,
        prompt: &str,
        final_text: &str,
        bash_calls_this_run: u32,
        verification_stale: bool,
    ) -> Option<BlockReason> {
        let receipts = ReceiptSummary {
            substantive_bash_calls: bash_calls_this_run,
            code_files_modified: if verification_stale { 1 } else { 0 },
            files_modified: if verification_stale { 1 } else { 0 },
            successful_tool_calls: if bash_calls_this_run > 0 {
                bash_calls_this_run
            } else {
                0
            },
            ..Default::default()
        };
        self.evaluate_receipts(prompt, final_text, None, &receipts, verification_stale)
    }
}

/// Checks whether a file path points to an executable, compilable, or script source file
/// (as opposed to documentation, notes, recipes, data, or content assets).
pub fn is_code_path(path: &str) -> bool {
    let p = std::path::Path::new(path);
    match p
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
    {
        Some(ext) => matches!(
            ext.as_str(),
            "rs" | "py"
                | "js"
                | "mjs"
                | "cjs"
                | "ts"
                | "tsx"
                | "jsx"
                | "c"
                | "cpp"
                | "cc"
                | "cxx"
                | "h"
                | "hpp"
                | "go"
                | "java"
                | "kt"
                | "kts"
                | "rb"
                | "php"
                | "swift"
                | "scala"
                | "sh"
                | "bash"
                | "zsh"
                | "fish"
                | "ps1"
                | "bat"
                | "cmd"
                | "lua"
                | "pl"
                | "pm"
                | "r"
                | "jl"
                | "dart"
                | "zig"
                | "nim"
                | "sql"
        ),
        None => false,
    }
}

/// Checks if a shell command is substantive rather than a dummy echo/no-op evasion.
pub fn is_substantive_command(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    // If it writes to a file or pipes to another command, it has side effects or processing
    if trimmed.contains('>') || trimmed.contains('|') {
        return true;
    }
    // Check if the command line is an explicit evasion claiming verification without doing work
    let lower = trimmed.to_ascii_lowercase();
    if (lower.starts_with("echo ") || lower.starts_with("printf "))
        && (lower.contains("verification")
            || lower.contains("shell execution path is functional")
            || lower.contains("verified")
            || lower.contains("dummy"))
    {
        return false;
    }
    let first_word = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches("./");
    let is_noop = matches!(first_word, ":" | "true" | "false" | "exit");
    !is_noop
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(act: vak_intent::Act, request: &str) -> vak_intent::OutcomeSpec {
        let mut reading = vak_intent::Reading::general();
        reading.act = act;
        vak_intent::OutcomeSpec::from_reading(request, &reading, 1)
    }

    fn verified(act: vak_intent::Act, request: &str) -> vak_intent::OutcomeSpec {
        let mut reading = vak_intent::Reading::general();
        reading.act = act;
        reading.evidence = vak_intent::Evidence::Verified;
        let mut spec = vak_intent::OutcomeSpec::from_reading(request, &reading, 1);
        spec.stop = vak_intent::StopProfile::Verification;
        spec
    }

    #[test]
    fn truncated_shapes_block_effect_work_and_normal_prose_passes() {
        let p = StopPolicy::default();
        assert!(matches!(
            p.evaluate(
                "",
                "done with part one.\nNow I'll write the store module",
                3
            ),
            Some(BlockReason::TruncatedPlan(_))
        ));
        assert!(matches!(
            p.evaluate("", "Let me check the config:", 3),
            Some(BlockReason::TruncatedPlan(_))
        ));
        assert!(matches!(
            p.evaluate("", "here is the patch:\n```rust\nfn a() {}", 3),
            Some(BlockReason::TruncatedPlan(_))
        ));
        assert_eq!(
            p.evaluate("", "All done. Tests pass.\nSummary:\n- added x", 3),
            None
        );
        assert_eq!(
            p.evaluate("", "Fixed both issues. cargo test green.", 3),
            None
        );
        assert_eq!(p.evaluate("", "Results\n# Summary:", 3), None);
    }

    /// The misfires the 2026-09-30 prompt audit demonstrated. Each is an
    /// ordinary request whose reading needs no tool; none may be sent back.
    #[test]
    fn authored_and_answered_work_is_never_sent_back_for_its_wording() {
        let p = StopPolicy::default();
        let none = ReceiptSummary::default();
        let letter = spec(
            vak_intent::Act::Author,
            "Write a short note to my neighbour",
        );
        assert_eq!(
            p.evaluate_receipts(
                "Write a short note to my neighbour thanking them for the flowers",
                "Dear Asha,\n\nThank you for the flowers.\n\nI'll see you at the market on Sunday",
                Some(&letter),
                &none,
                false,
            ),
            None
        );
        let agenda = spec(vak_intent::Act::Author, "Draft an agenda");
        assert_eq!(
            p.evaluate_receipts(
                "Draft an agenda for tomorrow's family meeting",
                "Agenda\n\n1. Holiday plans\n2. Chores rota\n\nThings to bring:",
                Some(&agenda),
                &none,
                false,
            ),
            None
        );
        let grammar = spec(vak_intent::Act::Verify, "Run a quick grammar check");
        assert_eq!(
            p.evaluate_receipts(
                "Can you run a quick grammar check on this paragraph: 'Their going to the park.'",
                "Corrected: They're going to the park after lunch, and they will bring sandwiches for everyone.",
                Some(&grammar),
                &none,
                false,
            ),
            None
        );
        let translation = spec(vak_intent::Act::Converse, "double-check this translation");
        assert_eq!(
            p.evaluate_receipts(
                "Please double-check that this translation into French is accurate: 'Good morning'",
                "Yes, « Bonjour » is accurate.",
                Some(&translation),
                &none,
                false,
            ),
            None
        );
    }

    #[test]
    fn explicit_until_done_request_requires_user_release() {
        let p = StopPolicy::default();
        assert!(matches!(
            p.evaluate(
                "Keep improving the project until I say done.",
                "Improved it.",
                1
            ),
            Some(BlockReason::UserCompletionRequired)
        ));
        assert!(!StopPolicy::requires_user_completion(
            "keep working until the tests pass"
        ));
        assert!(!StopPolicy::requires_user_completion(
            "don't stop until you find a cheaper flight"
        ));
        assert!(StopPolicy::is_done_message("done"));
        assert!(!StopPolicy::is_done_message(
            "done, and here is the summary"
        ));
    }

    /// An edit the reading did not hold to a check owes none: authoring a
    /// function into a code file is done when the function is there.
    #[test]
    fn an_edit_owes_no_check_the_reading_did_not_ask_for() {
        let p = StopPolicy::default();
        let authoring = spec(
            vak_intent::Act::Author,
            "add a subtract function to calc.py",
        );
        let edited = ReceiptSummary {
            total_tool_calls: 1,
            successful_tool_calls: 1,
            files_modified: 1,
            code_files_modified: 1,
            ..Default::default()
        };
        assert_eq!(
            p.evaluate_receipts(
                "explain what calc.py does, then add a subtract function to it",
                "calc.py now defines add and subtract.",
                Some(&authoring),
                &edited,
                true,
            ),
            None
        );
        let fixing = spec(vak_intent::Act::Modify, "fix the off-by-one in calc.py");
        let fixed = ReceiptSummary {
            substantive_bash_calls: 1,
            ..edited
        };
        assert_eq!(
            p.evaluate_receipts(
                "fix the off-by-one in calc.py",
                "Fixed.",
                Some(&fixing),
                &fixed,
                true,
            ),
            Some(BlockReason::VerificationStale)
        );
    }

    #[test]
    fn stale_verification_blocks_after_a_file_change() {
        let p = StopPolicy::default();
        assert_eq!(
            p.evaluate_with_state("implement it and run the tests", "Done.", 1, true),
            Some(BlockReason::VerificationStale)
        );
        assert_eq!(
            p.evaluate_with_state("implement it and run the tests", "Done.", 1, false),
            None
        );
    }

    #[test]
    fn empty_final_text_is_left_alone() {
        let p = StopPolicy::default();
        assert_eq!(p.evaluate("run the tests", "", 0), None);
    }

    #[test]
    fn test_substantive_command_detection() {
        assert!(!is_substantive_command(
            "echo \"Verification successful: Shell execution path is functional.\""
        ));
        assert!(!is_substantive_command("echo 'verification passed'"));
        assert!(!is_substantive_command("printf 'verified\\n'"));
        assert!(!is_substantive_command("true"));
        assert!(!is_substantive_command(":"));
        assert!(!is_substantive_command("exit 0"));
        assert!(!is_substantive_command(""));

        assert!(is_substantive_command("echo ran"));
        assert!(is_substantive_command("echo 'hello' > index.html"));
        assert!(is_substantive_command("echo 'hi' | wc -l"));
        assert!(is_substantive_command("python3 -m unittest"));
        assert!(is_substantive_command("cargo test"));
    }

    #[test]
    fn test_outcome_intent_requires_execution_receipt() {
        let p = StopPolicy::default();
        let spec = spec(vak_intent::Act::Modify, "create an svg animation");
        assert!(spec.requires_execution());
        let blocked = p.evaluate_receipts(
            "create an svg animation",
            "Here is your svg:\n```xml\n<svg/>\n```",
            Some(&spec),
            &ReceiptSummary::default(),
            false,
        );
        assert!(matches!(
            blocked,
            Some(BlockReason::ExecutionReceiptMissing { .. })
        ));
        for receipts in [
            ReceiptSummary {
                substantive_bash_calls: 1,
                successful_tool_calls: 1,
                ..Default::default()
            },
            // Work done through an integration or a delegated worker is
            // execution too: an email an MCP server sent needs no shell.
            ReceiptSummary {
                external_effects: 1,
                successful_tool_calls: 1,
                ..Default::default()
            },
            ReceiptSummary {
                files_modified: 1,
                successful_tool_calls: 1,
                ..Default::default()
            },
        ] {
            assert_eq!(
                p.evaluate_receipts(
                    "create an svg animation",
                    "Created.",
                    Some(&spec),
                    &receipts,
                    false
                ),
                None
            );
        }
    }

    #[test]
    fn capped_continuation_counts_prior_saved_file_only_after_successful_inspection() {
        let policy = StopPolicy::default();
        let spec = spec(vak_intent::Act::Modify, "create report.csv");
        let prompt = "Continue the most recent unfinished task";
        let answer = "The saved report contains 60 minutes.";
        let prior_only = ReceiptSummary {
            continued_saved_file: true,
            read_or_inspected: 1,
            ..Default::default()
        };
        assert!(matches!(
            policy.evaluate_receipts(prompt, answer, Some(&spec), &prior_only, false),
            Some(BlockReason::ExecutionReceiptMissing { .. })
        ));
        let inspected = ReceiptSummary {
            successful_inspections: 1,
            ..prior_only
        };
        assert_eq!(
            policy.evaluate_receipts(prompt, answer, Some(&spec), &inspected, false),
            None
        );
    }

    #[test]
    fn test_outcome_conversational_allows_prose_completion() {
        let p = StopPolicy::default();
        let spec = spec(vak_intent::Act::Answer, "what is rust?");
        assert!(!spec.requires_execution());
        assert!(!spec.requires_tool());
        assert_eq!(
            p.evaluate_receipts(
                "what is rust?",
                "Rust is a systems programming language.",
                Some(&spec),
                &ReceiptSummary::default(),
                false
            ),
            None
        );
    }

    #[test]
    fn an_unresolved_failure_passes_only_when_the_answer_quotes_it() {
        let p = StopPolicy::default();
        let receipts = ReceiptSummary {
            total_tool_calls: 1,
            failed_tool_calls: 1,
            unresolved_error: Some((
                "write".into(),
                "write failed: disk full (os error 28)".into(),
            )),
            ..Default::default()
        };
        for claim in [
            "All done! Everything succeeded.",
            "Saved your notes to notes.md with no issues.",
            "Saved without any error or problem.",
        ] {
            assert!(
                matches!(
                    p.evaluate_receipts("save my notes", claim, None, &receipts, false),
                    Some(BlockReason::UnresolvedToolFailure { .. })
                ),
                "{claim}"
            );
        }
        for report in [
            "I could not save the notes: the disk is full (disk full).",
            "नोट्स सेव नहीं हुए — disk full.",
        ] {
            assert_eq!(
                p.evaluate_receipts("save my notes", report, None, &receipts, false),
                None,
                "{report}"
            );
        }
    }

    #[test]
    fn test_direct_substantive_answer_not_blocked_for_inspection_spec() {
        let p = StopPolicy::default();
        let spec = spec(
            vak_intent::Act::Locate,
            "explain the architectural differences",
        );
        let answer = "Optimistic locking assumes multiple transactions can complete without affecting each other. It verifies no other transaction has modified the data before committing.";
        assert_eq!(
            p.evaluate_receipts(
                "explain",
                answer,
                Some(&spec),
                &ReceiptSummary::default(),
                false
            ),
            None
        );
        assert!(matches!(
            p.evaluate_receipts(
                "explain",
                "Okay",
                Some(&spec),
                &ReceiptSummary::default(),
                false
            ),
            Some(BlockReason::ExecutionReceiptMissing { .. })
        ));
    }

    #[test]
    fn test_is_code_path_accurately_classifies_code_vs_doc_paths() {
        assert!(is_code_path("src/main.rs"));
        assert!(is_code_path("backend/app.py"));
        assert!(is_code_path("web/index.ts"));
        assert!(is_code_path("scripts/deploy.sh"));
        assert!(!is_code_path("README.md"));
        assert!(!is_code_path("recipes/sourdough.txt"));
        assert!(!is_code_path("data/analysis.csv"));
    }

    #[test]
    fn a_demanded_proof_of_changed_code_needs_it_run() {
        let p = StopPolicy::default();
        let spec = verified(
            vak_intent::Act::Modify,
            "Fix the off-by-one bug in quicksort.py and verify that the sort works correctly.",
        );
        let edited = ReceiptSummary {
            total_tool_calls: 1,
            successful_tool_calls: 1,
            files_modified: 1,
            code_files_modified: 1,
            ..Default::default()
        };
        assert_eq!(
            p.evaluate_receipts(
                "fix and verify",
                "I fixed the index.",
                Some(&spec),
                &edited,
                false
            ),
            Some(BlockReason::VerificationMissing)
        );
        let ran = ReceiptSummary {
            substantive_bash_calls: 1,
            successful_tool_calls: 2,
            ..edited.clone()
        };
        assert_eq!(
            p.evaluate_receipts(
                "fix and verify",
                "Fixed and ran it.",
                Some(&spec),
                &ran,
                false
            ),
            None
        );
        assert_eq!(
            p.evaluate_receipts(
                "fix and verify",
                "Fixed and ran it.",
                Some(&spec),
                &ran,
                true
            ),
            Some(BlockReason::VerificationStale)
        );
        // A document proven by reading it back owes no shell.
        let doc = ReceiptSummary {
            total_tool_calls: 2,
            successful_tool_calls: 2,
            files_modified: 1,
            doc_files_modified: 1,
            read_or_inspected: 1,
            ..Default::default()
        };
        let doc_spec = verified(
            vak_intent::Act::Modify,
            "update README.md and verify the links",
        );
        assert_eq!(
            p.evaluate_receipts(
                "update",
                "Updated and checked.",
                Some(&doc_spec),
                &doc,
                true
            ),
            None
        );
    }
}
