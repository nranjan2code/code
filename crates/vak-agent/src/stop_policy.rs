//! Built-in stop gate: blocks premature completions.
//!
//! Small models frequently end their turn mid-plan ("Fixing both:" /
//! "Now I'll write the tests") or finish without doing the work the request
//! asked for. The dogfood campaign measured this at 5/7 runs on a free tier.
//! This policy reuses the existing stop-hook continuation machinery (append
//! a `[stop-guard]` control message, emit StopHookContinuation) at most
//! `max_blocks` times per run, so it can never trap a model in a loop.
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
    pub fn message(&self) -> String {
        match self {
            BlockReason::TruncatedPlan(tail) => format!(
                "your last message appears cut off ({tail}). Finish it now; if you are \
                 actually done, say so plainly."
            ),
            BlockReason::VerificationStale => String::from(
                "files changed after the last check ran. Run the check again before finishing, \
                 or say plainly why it cannot be run.",
            ),
            BlockReason::UserCompletionRequired => String::from(
                "the user asked you to keep working until they say done. Continue making \
                 useful progress; do not declare completion yet.",
            ),
            BlockReason::UnresolvedToolFailure { tool, error } => format!(
                "the `{tool}` call failed: {error}. Fix the cause and try again, or tell the \
                 person plainly that it failed and why, quoting the error.",
            ),
        }
    }
}

/// The control message a stop-guard block appends: the reason alone. Every
/// reason ends with what to do and the way out ("say plainly why"), so a
/// fixed "Please continue." after it only repeated the first half of that
/// and contradicted the second.
pub fn guard_message(reason: &str) -> String {
    format!("[stop-guard]: {reason}")
}

/// The control message a stop hook's demand appends. A hook's reason is
/// its own text, so the message gives the instruction and the way out.
pub fn hook_message(reason: &str) -> String {
    format!(
        "[stop-hook]: a stop hook asked for more work: {}. Do it now, or say plainly why you cannot.",
        reason.trim().trim_end_matches('.')
    )
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

/// Whether a call that neither ran a command nor changed a file looked
/// something up for the request. Every tool does, except the one that only
/// finds other tools and one that shows a card: which tool is the right
/// place to look is the tool's business, never a list of names here. A list
/// counted `read` and `grep` and left out `automations`, `workers` and
/// `doc_read`, so "list my automations", answered correctly from
/// `automations`, was sent back as not yet looked at (measured 2026-10-08:
/// 5 of 5 turns, three to five extra requests each).
pub(crate) fn looks_something_up(tool: &str, presents_cards: bool) -> bool {
    tool != "find_tools" && !presents_cards
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
/// pair of adjacent words ("disk full", "read-only mode", "exit code"), or
/// the tool's name with a distinctive word of the error ("bash denied").
/// Keywords ("no issues", "error-free") read as a report and let a false
/// success through, and they only work in English; the error's words are
/// the same whatever language the rest of the answer is written in.
fn reports_blocker(text: &str, tool: &str, error: &str) -> bool {
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
    // The tool named together with a distinctive word of its error ("bash
    // denied"). Words any success claim can carry do not count.
    const GENERIC: [&str; 6] = ["error", "errors", "failed", "failure", "issue", "problem"];
    let tool = tool.to_lowercase();
    let named_with_cause = answer.contains(&tool)
        && error.iter().any(|word| {
            word.chars().count() >= 5
                && *word != tool
                && !GENERIC.contains(&word.as_str())
                && answer.contains(word)
        });
    identifier
        || named_with_cause
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
    /// after the last check ran. What the request asked for is the model's
    /// declared plan, checked against receipts by `plan_check`
    /// (docs/design/86-model-plans-runtime-checks.md); a guessed reading of
    /// the request never holds a turn open.
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
        if let Some((tool, error)) = &receipts.unresolved_error
            && !reports_blocker(final_text, tool, error)
        {
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
            substantive_bash_calls: bash_calls_this_run,
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
    #[test]
    fn a_continuation_ends_with_its_way_out_not_a_bare_imperative() {
        let reason = BlockReason::UnresolvedToolFailure {
            tool: "bash".into(),
            error: "read-only mode denies 'bash'".into(),
        }
        .message();
        for message in [
            super::guard_message(&reason),
            super::hook_message("tests fail"),
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
                    p.evaluate_receipts("save my notes", claim, &receipts, false),
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
                p.evaluate_receipts("save my notes", report, &receipts, false),
                None,
                "{report}"
            );
        }
    }

    #[test]
    fn a_denial_is_reported_by_naming_the_tool_and_its_cause() {
        let p = StopPolicy::default();
        let receipts = ReceiptSummary {
            total_tool_calls: 1,
            failed_tool_calls: 1,
            unresolved_error: Some(("bash".into(), "denied by user: bash needs approval".into())),
            ..Default::default()
        };
        assert_eq!(
            p.evaluate_receipts(
                "run it",
                "bash denied: moving on without it",
                &receipts,
                false
            ),
            None
        );
        assert!(matches!(
            p.evaluate_receipts("run it", "Ran bash without any error.", &receipts, false),
            Some(BlockReason::UnresolvedToolFailure { .. })
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
}
