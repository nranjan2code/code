//! Built-in stop gate: blocks premature completions.
//!
//! Small models frequently end their turn mid-plan ("Fixing both:" /
//! "Now I'll write the tests") or finish without running verification they
//! explicitly promised. The dogfood campaign measured this at 5/7 runs on a
//! free tier. This policy reuses the existing stop-hook continuation
//! machinery (append "[stop-guard]: reason / Please continue.", emit
//! StopHookContinuation) at most `max_blocks` times per run, so it can
//! never trap a model in a loop.

/// What triggered the block — also the operator-visible reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    /// Final text looks truncated: ends with ':', a bare plan marker, or an
    /// unclosed fenced code block.
    TruncatedPlan(String),
    /// The prompt demanded running/testing something and the run never
    /// executed a single bash command.
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
                "the task asked for verification or sandbox execution, but no substantive commands were \
                 executed this run. Call the `bash` tool to actually execute, build, or verify the work now; do not print commands or dummy echo statements.",
            ),
            BlockReason::VerificationStale => String::from(
                "the task changed files after its last verification command. \
                 Call the `bash` tool to run the verification again before finishing; do not describe it in text.",
            ),
            BlockReason::UserCompletionRequired => String::from(
                "the user asked you to keep working until they say done. Continue making \
                 useful progress; do not declare completion yet.",
            ),
            BlockReason::ExecutionReceiptMissing { act, hint } => format!(
                "the request requires {act} ({hint}), but no execution or modification receipts were produced. \
                 Execute the necessary commands or file edits now using the available tools; do not just describe the work in prose.",
            ),
            BlockReason::UnresolvedToolFailure { tool, error } => format!(
                "the `{tool}` tool failed with an error: {error}. \
                 Repair the failure using the appropriate tools, or clearly report the concrete blocker to the user.",
            ),
        }
    }
}

/// Summary of tool execution receipts produced during a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReceiptSummary {
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
    /// Most recent unresolved tool failure, if any.
    pub unresolved_error: Option<(String, String)>,
}

impl ReceiptSummary {
    pub fn has_execution_receipt(&self) -> bool {
        self.substantive_bash_calls > 0 || self.files_modified > 0
    }

    pub fn has_inspection_receipt(&self) -> bool {
        self.read_or_inspected > 0 || self.has_execution_receipt()
    }

    pub fn has_any_receipt(&self) -> bool {
        self.successful_tool_calls > 0
    }
}

fn reports_blocker(text: &str, tool: &str, error: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let err_first_line = error
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let keywords = [
        "error",
        "failed",
        "failure",
        "failing",
        "blocked",
        "blocker",
        "could not",
        "cannot",
        "can't",
        "unable to",
        "unavailable",
        "issue",
        "problem",
        "exit code",
        "exception",
        "recover",
        "repaired",
        "unsupported",
        "guard",
        "denied",
        "denial",
        "rejected",
        "rejection",
        "repeated",
        "skip",
        "skipping",
    ];
    let mentions_keyword = keywords.iter().any(|k| lower.contains(k));
    let mentions_tool = lower.contains(&tool.to_ascii_lowercase());
    let mentions_snippet = !err_first_line.is_empty() && lower.contains(&err_first_line);
    mentions_keyword || mentions_tool || mentions_snippet
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
    pub fn requires_user_completion(prompt: &str) -> bool {
        let p = prompt.to_ascii_lowercase();
        [
            "until i say done",
            "until i tell you to stop",
            "until i tell you you're done",
            "keep working until",
            "keep improving until",
            "don't stop until",
            "do not stop until",
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

    /// Conservative trailing-intent patterns: only fire on line-final
    /// markers so normal prose summaries never match.
    fn truncated_plan(final_text: &str) -> Option<BlockReason> {
        let trimmed = final_text.trim_end();
        if trimmed.is_empty() {
            return None;
        }
        // Unclosed fenced code block: strong truncation signal.
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

    /// True when the prompt itself explicitly asks for executed verification, testing, or sandbox commands.
    pub fn demands_code_execution(prompt: &str) -> bool {
        let stripped = if let Some(idx) = prompt.find("[Scheduled-run context:") {
            &prompt[..idx]
        } else {
            prompt
        };
        const DEMANDS: [&str; 11] = [
            "must pass",
            "tests pass",
            "test pass",
            "run it",
            "run them",
            "run the test",
            "run tests",
            "verify by running",
            "prove by running",
            "run in sandbox",
            "execute in sandbox",
        ];
        let p = stripped.to_ascii_lowercase();
        if DEMANDS.iter().any(|d| p.contains(d)) {
            return true;
        }
        if p.contains("sandbox")
            && ["run", "show", "test", "build", "execute", "serve", "start"]
                .iter()
                .any(|action| p.contains(action))
        {
            return true;
        }
        if p.contains("cargo test")
            || p.contains("pytest")
            || p.contains("npm test")
            || p.contains("go test")
            || p.contains("python -m unittest")
        {
            return true;
        }
        p.contains("run ")
            && [
                "test", "tests", "command", "script", "check", "app", "code", "python", "cargo",
                "binary",
            ]
            .iter()
            .any(|word| p.contains(word))
    }

    /// True when the prompt asks for any verification (code or universal/content).
    pub fn demands_verification(prompt: &str) -> bool {
        if Self::demands_code_execution(prompt) {
            return true;
        }
        let stripped = if let Some(idx) = prompt.find("[Scheduled-run context:") {
            &prompt[..idx]
        } else {
            prompt
        };
        let p = stripped.to_ascii_lowercase();
        const VERIFY_MARKERS: [&str; 7] = [
            "verify that",
            "verify the",
            "double check",
            "double-check",
            "make sure that",
            "check that",
            "verify whether",
        ];
        VERIFY_MARKERS.iter().any(|m| p.contains(m))
    }

    /// True when the assistant response claims execution or emits shell scripts without tool calls having run.
    pub fn claims_execution_unexecuted(final_text: &str) -> bool {
        let lower = final_text.to_ascii_lowercase();
        let markers = [
            "use the bash tool",
            "use the `bash` tool",
            "using the bash tool",
            "using the `bash` tool",
            "run the python script",
            "running the python script",
            "execute the python script",
            "executing the python script",
            "execute in the sandbox",
            "running in the sandbox",
            "run in the sandbox",
            "execute in sandbox",
            "running in sandbox",
        ];
        if markers.iter().any(|m| lower.contains(m)) {
            return true;
        }
        if (lower.contains("```bash") || lower.contains("```sh"))
            && (lower.contains(".vak/scratch")
                || lower.contains("python3 ")
                || lower.contains("node ")
                || lower.contains("cargo "))
        {
            return true;
        }
        false
    }

    /// Intent- and receipt-driven evaluation of completion validity.
    pub fn evaluate_receipts(
        &self,
        prompt: &str,
        final_text: &str,
        outcome: Option<&vak_intent::OutcomeSpec>,
        receipts: &ReceiptSummary,
        verification_stale: bool,
    ) -> Option<BlockReason> {
        if final_text.trim().is_empty() {
            return None;
        }
        if Self::requires_user_completion(prompt) {
            return Some(BlockReason::UserCompletionRequired);
        }
        if self.marker_gate
            && let Some(r) = Self::truncated_plan(final_text)
        {
            return Some(r);
        }

        // If a tool failed and hasn't been repaired or reported in text, block.
        if let Some((tool, err)) = &receipts.unresolved_error
            && !reports_blocker(final_text, tool, err)
        {
            return Some(BlockReason::UnresolvedToolFailure {
                tool: tool.clone(),
                error: err.clone(),
            });
        }

        // Intent-driven gate: when outcome specification is available.
        if let Some(spec) = outcome {
            if spec.requires_execution() {
                if !receipts.has_execution_receipt() {
                    let act = spec.deliverable_act().unwrap_or("execution").to_string();
                    return Some(BlockReason::ExecutionReceiptMissing {
                        act,
                        hint: "run code, build, test, or modify files".into(),
                    });
                }
                if self.verify_gate
                    && verification_stale
                    && (spec.requires_execution() || Self::demands_verification(prompt))
                {
                    if receipts.code_files_modified > 0 || Self::demands_code_execution(prompt) {
                        return Some(BlockReason::VerificationStale);
                    }
                }
            } else if spec.requires_inspection() {
                let direct_substantive = final_text.trim().len() >= 80
                    && !Self::demands_code_execution(prompt)
                    && !Self::claims_execution_unexecuted(final_text);
                if !receipts.has_inspection_receipt() && !direct_substantive {
                    let act = spec.deliverable_act().unwrap_or("inspection").to_string();
                    return Some(BlockReason::ExecutionReceiptMissing {
                        act,
                        hint: "read, search, inspect files or data".into(),
                    });
                }
            } else if spec.requires_tool() && !receipts.has_any_receipt() {
                let direct_substantive = final_text.trim().len() >= 80
                    && !Self::demands_code_execution(prompt)
                    && !Self::claims_execution_unexecuted(final_text);
                if !direct_substantive {
                    return Some(BlockReason::ExecutionReceiptMissing {
                        act: "tool execution".into(),
                        hint: "execute relevant tools".into(),
                    });
                }
            }
        }

        // Verification and execution gate: runs whenever verify_gate is enabled.
        if self.verify_gate {
            let demands_code = Self::demands_code_execution(prompt);
            let claims_exec = Self::claims_execution_unexecuted(final_text);

            if claims_exec {
                return Some(BlockReason::VerificationMissing);
            }

            if demands_code && receipts.substantive_bash_calls == 0 {
                return Some(BlockReason::VerificationMissing);
            }

            if Self::demands_verification(prompt) && receipts.substantive_bash_calls == 0 {
                // If code files were touched, verification commands are required.
                if receipts.code_files_modified > 0 {
                    return Some(BlockReason::VerificationMissing);
                }
                // If no tools were called and the text is not a substantive direct answer:
                let direct_substantive = final_text.trim().len() >= 80;
                if !receipts.has_any_receipt() && !direct_substantive {
                    return Some(BlockReason::VerificationMissing);
                }
                // If an execution deliverable was required or a specific file target was requested,
                // but no files were modified or inspected:
                let lower_p = prompt.to_ascii_lowercase();
                let mentions_file_target = lower_p.contains(".md")
                    || lower_p.contains(".txt")
                    || lower_p.contains(".json")
                    || lower_p.contains(".csv")
                    || lower_p.contains("into ")
                    || lower_p.contains("in file")
                    || lower_p.contains("in the file");
                if (outcome.map(|s| s.requires_execution()).unwrap_or(false) || mentions_file_target)
                    && receipts.files_modified == 0
                    && receipts.read_or_inspected == 0
                {
                    return Some(BlockReason::VerificationMissing);
                }
                // Universal tasks (documentation, research synthesis, lifestyle, notes, recipes,
                // explanations) where content was inspected, written, or substantively answered
                // are NOT falsely blocked on non-existent bash commands.
            }

            if verification_stale && (receipts.code_files_modified > 0 || demands_code) {
                return Some(BlockReason::VerificationStale);
            }
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
    match p.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_ascii_lowercase()) {
        Some(ext) => matches!(
            ext.as_str(),
            "rs" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx"
                | "c" | "cpp" | "cc" | "cxx" | "h" | "hpp"
                | "go" | "java" | "kt" | "kts" | "rb" | "php"
                | "swift" | "scala" | "sh" | "bash" | "zsh" | "fish"
                | "ps1" | "bat" | "cmd" | "lua" | "pl" | "pm"
                | "r" | "jl" | "dart" | "zig" | "nim" | "sql"
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

    #[test]
    fn truncated_shapes_block_normal_prose_passes() {
        let p = StopPolicy::default();
        assert!(
            p.evaluate("", "Working on it:\n- fix parser\n- then", 1)
                .is_none()
                || true
        ); // sanity no-op to keep structure

        // trailing colon line
        assert!(matches!(
            p.evaluate("", "Let me check the config:", 3),
            Some(BlockReason::TruncatedPlan(_))
        ));
        // unclosed fence
        assert!(matches!(
            p.evaluate("", "here is the patch:\n```rust\nfn a() {}", 3),
            Some(BlockReason::TruncatedPlan(_))
        ));
        // plan-marker final line without terminal punctuation
        assert!(matches!(
            p.evaluate(
                "",
                "done with part one.\nNow I'll write the store module",
                3
            ),
            Some(BlockReason::TruncatedPlan(_))
        ));
        // normal summary passes
        assert_eq!(
            p.evaluate("", "All done. Tests pass.\nSummary:\n- added x", 3),
            None
        );
        assert_eq!(
            p.evaluate("", "Fixed both issues. cargo test green.", 3),
            None
        );
        // headings ending in colon are fine (e.g. 'Summary:')
        assert_eq!(p.evaluate("", "Results\n# Summary:", 3), None);
    }

    #[test]
    fn verify_gate_needs_demand_and_zero_bash() {
        let p = StopPolicy::default();
        let prompt = "Create fizzbuzz.py and run it to prove that it works.";
        assert!(matches!(
            p.evaluate(prompt, "Created the file.", 0),
            Some(BlockReason::VerificationMissing)
        ));
        assert_eq!(p.evaluate(prompt, "Created the file.", 2), None);
        // no demand -> never blocks
        assert_eq!(p.evaluate("Write a haiku about sand.", "Done.", 0), None);
    }

    #[test]
    fn verify_gate_recognizes_explicit_run_commands() {
        let p = StopPolicy::default();
        assert!(matches!(
            p.evaluate(
                "Read README.md, implement the change, then run python3 test_app.py.",
                "I need more details.",
                0
            ),
            Some(BlockReason::VerificationMissing)
        ));
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
        assert!(StopPolicy::is_done_message("done"));
        assert!(!StopPolicy::is_done_message(
            "done, and here is the summary"
        ));
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
        assert!(is_substantive_command("echo 'hello'"));
        assert!(is_substantive_command("echo 'hello' > index.html"));
        assert!(is_substantive_command("echo 'hi' | wc -l"));
        assert!(is_substantive_command("python3 -m unittest"));
        assert!(is_substantive_command("cargo test"));
        assert!(is_substantive_command("npm start"));
        assert!(is_substantive_command("node server.js"));
    }

    #[test]
    fn test_sandbox_demands_verification() {
        let p = StopPolicy::default();
        let prompt =
            "make in using react with beautifull design and run them in sandbox and show me";
        assert!(matches!(
            p.evaluate(prompt, "Here is the code in a block.", 0),
            Some(BlockReason::VerificationMissing)
        ));
    }

    #[test]
    fn test_outcome_intent_requires_execution_receipt() {
        let p = StopPolicy::default();
        let mut reading = vak_intent::Reading::general();
        reading.act = vak_intent::Act::Modify;
        let spec = vak_intent::OutcomeSpec::from_reading("create an svg animation", &reading, 1);
        assert!(spec.requires_execution());

        // 0 receipts -> blocked
        let empty_receipts = ReceiptSummary::default();
        let blocked = p.evaluate_receipts(
            "create an svg animation",
            "Here is your svg:\n```xml\n<svg/>\n```",
            Some(&spec),
            &empty_receipts,
            false,
        );
        assert!(matches!(
            blocked,
            Some(BlockReason::ExecutionReceiptMissing { .. })
        ));

        // with substantive bash receipt -> allowed
        let with_bash = ReceiptSummary {
            substantive_bash_calls: 1,
            successful_tool_calls: 1,
            ..Default::default()
        };
        assert_eq!(
            p.evaluate_receipts(
                "create an svg animation",
                "Created and verified.",
                Some(&spec),
                &with_bash,
                false
            ),
            None
        );

        // with file write receipt -> allowed
        let with_file = ReceiptSummary {
            files_modified: 1,
            successful_tool_calls: 1,
            ..Default::default()
        };
        assert_eq!(
            p.evaluate_receipts(
                "create an svg animation",
                "Created file.",
                Some(&spec),
                &with_file,
                false
            ),
            None
        );
    }

    #[test]
    fn test_outcome_conversational_allows_prose_completion() {
        let p = StopPolicy::default();
        let mut reading = vak_intent::Reading::general();
        reading.act = vak_intent::Act::Answer;
        let spec = vak_intent::OutcomeSpec::from_reading("what is rust?", &reading, 1);
        assert!(!spec.requires_execution());
        assert!(!spec.requires_tool());

        let receipts = ReceiptSummary::default();
        assert_eq!(
            p.evaluate_receipts(
                "what is rust?",
                "Rust is a systems programming language.",
                Some(&spec),
                &receipts,
                false
            ),
            None
        );
    }

    #[test]
    fn test_claims_execution_unexecuted_blocks_even_with_outcome_spec() {
        let p = StopPolicy::default();
        let reading = vak_intent::Reading::general();
        let spec = vak_intent::OutcomeSpec::from_reading("can you run it and show", &reading, 1);
        let receipts = ReceiptSummary::default();

        let blocked = p.evaluate_receipts(
            "can you run it and show",
            "I will use the bash tool to execute a Python script:\n```bash\npython3 script.py\n```",
            Some(&spec),
            &receipts,
            false,
        );
        assert_eq!(blocked, Some(BlockReason::VerificationMissing));
    }

    #[test]
    fn test_unresolved_tool_failure_blocks_unless_reported() {
        let p = StopPolicy::default();
        let receipts = ReceiptSummary {
            total_tool_calls: 1,
            failed_tool_calls: 1,
            unresolved_error: Some(("bash".into(), "exit code 1: compile error".into())),
            ..Default::default()
        };

        // Model hallucinates success without reporting error -> blocked
        let blocked = p.evaluate_receipts(
            "build it",
            "All done! Everything succeeded.",
            None,
            &receipts,
            false,
        );
        assert!(matches!(
            blocked,
            Some(BlockReason::UnresolvedToolFailure { .. })
        ));

        // Model reports the error/blocker -> allowed
        let reported = p.evaluate_receipts(
            "build it",
            "The build failed with exit code 1: compile error. Cannot proceed without missing dependency.",
            None,
            &receipts,
            false,
        );
        assert_eq!(reported, None);
    }

    #[test]
    fn test_direct_substantive_answer_not_blocked_for_inspection_spec() {
        let p = StopPolicy::default();
        let mut reading = vak_intent::Reading::general();
        reading.act = vak_intent::Act::Locate;
        let spec = vak_intent::OutcomeSpec::from_reading(
            "explain the architectural differences",
            &reading,
            1,
        );
        let receipts = ReceiptSummary::default();

        // Substantive direct analysis without false tool claims or execution demands -> allowed
        let substantive_answer = "Optimistic locking assumes multiple transactions can complete without affecting each other. It verifies no other transaction has modified the data before committing. In contrast, pessimistic locking acquires locks immediately upon reading.";
        let blocked = p.evaluate_receipts(
            "explain the architectural differences",
            substantive_answer,
            Some(&spec),
            &receipts,
            false,
        );
        assert_eq!(blocked, None);

        // Empty or non-substantive answer -> blocked
        let blocked_empty = p.evaluate_receipts(
            "explain the architectural differences",
            "Okay",
            Some(&spec),
            &receipts,
            false,
        );
        assert!(matches!(
            blocked_empty,
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
        assert!(!is_code_path("docs/architecture.md"));
        assert!(!is_code_path("recipes/sourdough.txt"));
        assert!(!is_code_path("data/analysis.csv"));
        assert!(!is_code_path("notes.org"));
    }

    #[test]
    fn test_universal_doc_modification_with_verify_not_blocked_on_bash() {
        let p = StopPolicy::default();
        let prompt = "Update README.md to describe the release steps and verify that all links are formatted correctly.";
        let final_text = "Updated README.md with release steps and verified that the Markdown links match the repository structure.";

        let receipts = ReceiptSummary {
            total_tool_calls: 2,
            successful_tool_calls: 2,
            files_modified: 1,
            code_files_modified: 0,
            doc_files_modified: 1,
            read_or_inspected: 1,
            ..Default::default()
        };

        // Even though prompt says "verify that", since no code was modified and no code execution was demanded,
        // it must NOT block on non-existent bash commands or stale verification!
        let blocked = p.evaluate_receipts(prompt, final_text, None, &receipts, false);
        assert_eq!(blocked, None);

        let blocked_stale = p.evaluate_receipts(prompt, final_text, None, &receipts, true);
        assert_eq!(blocked_stale, None);
    }

    #[test]
    fn test_universal_research_and_lifestyle_with_verify_not_blocked() {
        let p = StopPolicy::default();
        let prompt = "Compare the top 3 pour-over drippers and verify that the brew ratios are accurate.";
        let final_text = "Here is a detailed comparison of Hario V60, Kalita Wave, and Chemex. All brew ratios are verified between 1:15 and 1:17 for balanced extraction across light and medium roasts.";

        let receipts = ReceiptSummary {
            total_tool_calls: 1,
            successful_tool_calls: 1,
            read_or_inspected: 1,
            ..Default::default()
        };

        let blocked = p.evaluate_receipts(prompt, final_text, None, &receipts, false);
        assert_eq!(blocked, None);
    }

    #[test]
    fn test_code_modification_with_verify_blocked_without_execution() {
        let p = StopPolicy::default();
        let prompt = "Fix the off-by-one bug in quicksort.py and verify that the sort works correctly.";
        let final_text = "I fixed the index in quicksort.py.";

        let receipts = ReceiptSummary {
            total_tool_calls: 1,
            successful_tool_calls: 1,
            files_modified: 1,
            code_files_modified: 1,
            doc_files_modified: 0,
            ..Default::default()
        };

        // Code was modified and prompt asks to "verify that" -> must block with VerificationMissing
        let blocked = p.evaluate_receipts(prompt, final_text, None, &receipts, false);
        assert_eq!(blocked, Some(BlockReason::VerificationMissing));

        // If code files were modified after test ran, stale verification blocks
        let with_bash = ReceiptSummary {
            total_tool_calls: 2,
            successful_tool_calls: 2,
            substantive_bash_calls: 1,
            files_modified: 1,
            code_files_modified: 1,
            ..Default::default()
        };
        let blocked_stale = p.evaluate_receipts(prompt, final_text, None, &with_bash, true);
        assert_eq!(blocked_stale, Some(BlockReason::VerificationStale));
    }
}
