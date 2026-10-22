//! Built-in stop gate: blocks premature completions.
//!
//! Small models frequently end their turn mid-plan ("Fixing both:" /
//! "Now I'll write the tests") or finish without doing the work the request
//! asked for. The dogfood campaign measured this at 5/7 runs on a free tier.
//! Each reason is told once through the correction ladder
//! (`vak_control::Ladder`); an answer that still leaves it standing is the
//! model's own, so it can never trap a model in a loop.
//!
//! What completion needs comes from the admitted reading (`OutcomeSpec`) and
//! the run's receipts, never from phrases in the request: a phrase list read
//! "run a quick grammar check" as a demand for a shell command and "I'll see
//! you on Sunday" at the end of a letter as an unfinished plan, and it only
//! ever spoke English.

/// What triggered the block — also the operator-visible reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    /// The final text ends inside an unclosed fenced code block.
    TruncatedPlan(String),
    /// A file-changing tool ran after the last verification command.
    VerificationStale,
    /// The user explicitly asked the agent to continue until a later user
    /// message authorizes completion.
    UserCompletionRequired,
    /// A tool failed with an error and the model neither repaired it nor reported the blocker.
    UnresolvedToolFailure { tool: String, error: String },
}

impl BlockReason {
    /// The finding a correction is rendered from.
    pub fn finding(&self) -> vak_control::Finding {
        use vak_control::Finding;
        match self {
            BlockReason::TruncatedPlan(what) => Finding::Truncated { what: what.clone() },
            BlockReason::VerificationStale => Finding::VerificationStale,
            BlockReason::UserCompletionRequired => Finding::KeepWorking,
            BlockReason::UnresolvedToolFailure { tool, error } => Finding::UnresolvedFailure {
                tool: tool.clone(),
                error: error.clone(),
            },
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
    /// Successful tool results that produced a declared user-facing artifact.
    pub artifact_deliveries: u32,
    /// Number of tool calls that completed with ToolRunOutput::Err.
    pub failed_tool_calls: u32,
    /// Number of calls that ran a command (`Tool::command`).
    pub commands_run: u32,
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
    /// The last broker-owned Topics call failed, including a refused call.
    /// Organisation failure never holds the requested answer open.
    pub topics_failed: bool,
    /// Most recent unresolved tool failure, if any.
    pub unresolved_error: Option<(String, String)>,
}

impl ReceiptSummary {
    pub fn has_execution_receipt(&self) -> bool {
        self.commands_run > 0
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

#[derive(Debug, Clone)]
pub struct StopPolicy {
    /// Gate on truncated-looking final messages.
    pub marker_gate: bool,
    /// Gate on promised-but-never-run verification.
    pub verify_gate: bool,
}

impl Default for StopPolicy {
    fn default() -> Self {
        StopPolicy {
            marker_gate: true,
            verify_gate: true,
        }
    }
}

impl StopPolicy {
    /// The request hands completion to the person with the explicit
    /// `/until-done …` command: work continues until they release it with a
    /// `done`/`stop` message. Control comes from a command, never from
    /// phrases in free text (invariant 32); a phrase list only ever spoke
    /// English and read "keep working until the tests pass" as a hold.
    pub fn requires_user_completion(prompt: &str) -> bool {
        matches!(
            vak_intent::parse_command(prompt),
            Some(vak_intent::Command::UntilDone { .. })
        )
    }

    pub fn is_done_message(message: &str) -> bool {
        let normalized = message.trim().to_lowercase();
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

    /// Whether this turn may end, judged only from what the run observed:
    /// the explicit hold, an answer cut off inside a code fence, a tool
    /// failure the answer neither repaired nor reports, and code changed
    /// after the last check ran. Optional progress is checked separately from
    /// its declared states, with one reminder. A guessed reading of the
    /// request never holds a turn open.
    pub fn evaluate_receipts(
        &self,
        prompt: &str,
        final_text: &str,
        receipts: &ReceiptSummary,
        verification_stale: bool,
    ) -> Option<BlockReason> {
        if final_text.trim().is_empty() {
            return None;
        }
        if Self::requires_user_completion(prompt) {
            return Some(BlockReason::UserCompletionRequired);
        }
        if self.marker_gate && final_text.matches("```").count() % 2 == 1 {
            return Some(BlockReason::TruncatedPlan("unclosed code fence".into()));
        }
        if let Some((tool, error)) = &receipts.unresolved_error {
            return Some(BlockReason::UnresolvedToolFailure {
                tool: tool.clone(),
                error: error.clone(),
            });
        }
        (self.verify_gate && verification_stale && receipts.code_files_modified > 0)
            .then_some(BlockReason::VerificationStale)
    }

    /// Returns Some(reason) when completion should be blocked.
    pub fn evaluate(
        &self,
        prompt: &str,
        final_text: &str,
        bash_calls_this_run: u32,
    ) -> Option<BlockReason> {
        let receipts = ReceiptSummary {
            commands_run: bash_calls_this_run,
            successful_tool_calls: if bash_calls_this_run > 0 {
                bash_calls_this_run
            } else {
                0
            },
            ..Default::default()
        };
        self.evaluate_receipts(prompt, final_text, &receipts, false)
    }

    pub fn evaluate_with_state(
        &self,
        prompt: &str,
        final_text: &str,
        bash_calls_this_run: u32,
        verification_stale: bool,
    ) -> Option<BlockReason> {
        let receipts = ReceiptSummary {
            commands_run: bash_calls_this_run,
            code_files_modified: if verification_stale { 1 } else { 0 },
            files_modified: if verification_stale { 1 } else { 0 },
            successful_tool_calls: if bash_calls_this_run > 0 {
                bash_calls_this_run
            } else {
                0
            },
            ..Default::default()
        };
        self.evaluate_receipts(prompt, final_text, &receipts, verification_stale)
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

#[cfg(test)]
mod tests {
    #[test]
    fn a_continuation_ends_with_its_way_out_not_a_bare_imperative() {
        let reason = BlockReason::UnresolvedToolFailure {
            tool: "bash".into(),
            error: "read-only mode denies 'bash'".into(),
        }
        .finding();
        for message in [
            vak_control::render(&reason, "[stop-guard]"),
            vak_control::render(
                &vak_control::Finding::StopHook {
                    reason: "tests fail".into(),
                },
                "[stop-hook]",
            ),
        ] {
            assert!(!message.contains("Please continue"), "{message}");
            assert!(message.contains("plainly"), "{message}");
        }
    }

    use super::*;

    /// Found live (2026-10-11): a turn read as "verify" from a sentence that
    /// described the person's habit was held open until it looked something
    /// up, so the model called `recall {}` eighteen times. What was asked is
    /// the model's plan now; nothing here reads the request's acts.
    #[test]
    fn an_answer_is_never_held_open_by_a_reading_of_the_request() {
        let p = StopPolicy::default();
        let receipts = ReceiptSummary::default();
        for answer in ["Done.", "Here is the routine and the comparison."] {
            assert_eq!(
                p.evaluate_receipts("check it", answer, &receipts, false),
                None
            );
        }
    }

    #[test]
    fn an_answer_cut_off_inside_a_code_fence_is_held() {
        let p = StopPolicy::default();
        assert!(matches!(
            p.evaluate("", "here is the patch:\n```rust\nfn a() {}", 3),
            Some(BlockReason::TruncatedPlan(_))
        ));
        assert_eq!(p.evaluate("", "Options:", 0), None);
    }

    #[test]
    fn explicit_until_done_request_requires_user_release() {
        let p = StopPolicy::default();
        assert!(matches!(
            p.evaluate("/until-done keep improving the project", "Improved it.", 1),
            Some(BlockReason::UserCompletionRequired)
        ));
        // A phrase is not the command, in any language.
        assert!(!StopPolicy::requires_user_completion(
            "Keep improving the project until I say done."
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
    fn test_is_code_path_accurately_classifies_code_vs_doc_paths() {
        assert!(is_code_path("src/main.rs"));
        assert!(is_code_path("backend/app.py"));
        assert!(is_code_path("web/index.ts"));
        assert!(is_code_path("scripts/deploy.sh"));
        assert!(!is_code_path("README.md"));
        assert!(!is_code_path("recipes/sourdough.txt"));
        assert!(!is_code_path("data/analysis.csv"));
    }

    /// Whether an answer owns up to a failure is not read from its words:
    /// the failure is a reason whatever the answer says, and the loop
    /// records it and lets the answer stand.
    #[test]
    fn an_unrepaired_failure_is_a_reason_whatever_the_answer_says() {
        let receipts = ReceiptSummary {
            unresolved_error: Some(("bash".into(), "read-only mode denies 'bash'".into())),
            ..Default::default()
        };
        for answer in [
            "All done, everything worked.",
            "The bash call failed: read-only mode denies 'bash'.",
        ] {
            assert!(matches!(
                StopPolicy::default().evaluate_receipts("do it", answer, &receipts, false),
                Some(BlockReason::UnresolvedToolFailure { .. })
            ));
        }
    }
}
