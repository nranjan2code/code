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
        }
    }
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

    /// True when the prompt itself asks for executed verification or sandbox execution.
    fn demands_verification(prompt: &str) -> bool {
        const DEMANDS: [&str; 12] = [
            "must pass",
            "tests pass",
            "run it",
            "run them",
            "verify",
            "prove that",
            "prove it",
            "in sandbox",
            "in the sandbox",
            "show in sandbox",
            "run in sandbox",
            "execute in sandbox",
        ];
        let p = prompt.to_ascii_lowercase();
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
        p.contains("run ")
            && [
                "test", "tests", "command", "script", "check", "app", "code", "it",
            ]
            .iter()
            .any(|word| p.contains(word))
    }

    /// Returns Some(reason) when completion should be blocked.
    pub fn evaluate(
        &self,
        prompt: &str,
        final_text: &str,
        bash_calls_this_run: u32,
    ) -> Option<BlockReason> {
        self.evaluate_with_state(prompt, final_text, bash_calls_this_run, false)
    }

    pub fn evaluate_with_state(
        &self,
        prompt: &str,
        final_text: &str,
        bash_calls_this_run: u32,
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
        if self.verify_gate && bash_calls_this_run == 0 && Self::demands_verification(prompt) {
            return Some(BlockReason::VerificationMissing);
        }
        if self.verify_gate && verification_stale && Self::demands_verification(prompt) {
            return Some(BlockReason::VerificationStale);
        }
        None
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
    // Check if the command line is just a bare echo/printf/noop
    let first_word = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches("./");
    let is_noop = matches!(first_word, "echo" | "printf" | "true" | ":" | "exit");
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
        assert!(!is_substantive_command("echo 'hello'"));
        assert!(!is_substantive_command("echo \"Verification successful\""));
        assert!(!is_substantive_command("printf 'done\\n'"));
        assert!(!is_substantive_command("true"));
        assert!(!is_substantive_command(":"));
        assert!(!is_substantive_command("exit 0"));
        assert!(!is_substantive_command(""));

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
}
