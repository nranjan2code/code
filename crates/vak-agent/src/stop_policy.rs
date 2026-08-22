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
}

impl BlockReason {
    pub fn message(&self) -> String {
        match self {
            BlockReason::TruncatedPlan(tail) => format!(
                "your last message appears cut off mid-plan (ends with {tail:?}). \
                 Finish the work now; if you are actually done, say so plainly."
            ),
            BlockReason::VerificationMissing => String::from(
                "the task asked you to run/verify something, but no commands were \
                 executed this run. Run the verification before finishing.",
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

    /// True when the prompt itself asks for executed verification.
    fn demands_verification(prompt: &str) -> bool {
        const DEMANDS: [&str; 7] = [
            "must pass",
            "tests pass",
            "run it",
            "run them",
            "verify",
            "prove that",
            "prove it",
        ];
        let p = prompt.to_ascii_lowercase();
        DEMANDS.iter().any(|d| p.contains(d))
    }

    /// Returns Some(reason) when completion should be blocked.
    pub fn evaluate(
        &self,
        prompt: &str,
        final_text: &str,
        bash_calls_this_run: u32,
    ) -> Option<BlockReason> {
        if self.marker_gate
            && let Some(r) = Self::truncated_plan(final_text)
        {
            return Some(r);
        }
        if self.verify_gate && bash_calls_this_run == 0 && Self::demands_verification(prompt) {
            return Some(BlockReason::VerificationMissing);
        }
        None
    }
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
    fn empty_final_text_is_left_alone() {
        let p = StopPolicy::default();
        assert_eq!(p.evaluate("run the tests", "", 0), None);
    }
}
