//! A worker's question on the terminal
//! (docs/design/84-worker-questions-and-control.md §4.5).
//!
//! `vak exec` auto-approves or auto-denies gates and never prompts for them.
//! A question is different: nothing can answer it but a person, so when one is
//! at the terminal the question is printed and a line is read from the
//! keyboard. Attended means both stdin and stderr are terminals; a piped or
//! scripted run is unattended and the worker is told at once that nobody can
//! answer.

use std::io::Write;
use std::sync::{Arc, Mutex};

use vak_agent::{Approver, PendingQuestion, WorkerRegistry};

/// Reads one line of the person's reply. `None` means end of input.
pub(crate) type ReadLine = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// Wraps the run's gate approver (`AutoApprove` or `AutoDeny`, unchanged) and
/// adds the ability to ask the person at the terminal.
pub(crate) struct CliApprover {
    gates: Arc<dyn Approver>,
    attended: bool,
    read_line: ReadLine,
    /// One prompt at a time: several workers may ask at once, and two threads
    /// reading the keyboard would split a reply between them.
    prompting: Arc<Mutex<()>>,
}

impl CliApprover {
    pub(crate) fn new(gates: Arc<dyn Approver>) -> Self {
        Self::with(gates, terminal_is_attended(), Arc::new(read_stdin_line))
    }

    pub(crate) fn with(gates: Arc<dyn Approver>, attended: bool, read_line: ReadLine) -> Self {
        CliApprover {
            gates,
            attended,
            read_line,
            prompting: Arc::new(Mutex::new(())),
        }
    }
}

fn terminal_is_attended() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn read_stdin_line() -> Option<String> {
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

/// What the person typed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    /// Leave it unanswered: the worker is told no answer arrived.
    Skip,
    Answer(String),
}

/// A number picks that option; anything else is the answer as typed; an
/// empty line skips.
pub(crate) fn interpret(question: &PendingQuestion, line: &str) -> Reply {
    let line = line.trim();
    if line.is_empty() {
        return Reply::Skip;
    }
    if let Ok(number) = line.parse::<usize>()
        && (1..=question.options.len()).contains(&number)
    {
        return Reply::Answer(question.options[number - 1].clone());
    }
    Reply::Answer(line.to_string())
}

fn render(question: &PendingQuestion) -> String {
    let mut out = format!("\n? {} asks: {}\n", question.label, question.question);
    for (index, option) in question.options.iter().enumerate() {
        out.push_str(&format!("  {}) {option}\n", index + 1));
    }
    out.push_str(if question.options.is_empty() {
        "  Your answer (Enter to skip; it does not approve any action): "
    } else {
        "  Your answer, a number above or your own words (Enter to skip; it does not approve any action): "
    });
    out
}

#[async_trait::async_trait]
impl Approver for CliApprover {
    async fn approve(
        &self,
        tool: &str,
        args_json: &str,
        reason: &str,
        call_id: Option<&str>,
    ) -> bool {
        self.gates.approve(tool, args_json, reason, call_id).await
    }

    fn answerable(&self) -> bool {
        self.gates.answerable()
    }

    fn answers_questions(&self) -> bool {
        self.attended
    }

    async fn announce_question(&self, question: &PendingQuestion, board: &Arc<WorkerRegistry>) {
        if !self.attended {
            board.questions().close(&question.id);
            return;
        }
        let question = question.clone();
        let board = board.clone();
        let read_line = self.read_line.clone();
        let prompting = self.prompting.clone();
        // A plain OS thread, not a tokio blocking task: a prompt still waiting
        // on the keyboard when the run ends must never hold up the process.
        std::thread::spawn(move || {
            let _one_at_a_time = prompting
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // It may have been answered or have expired while queued.
            if !board.questions().is_open(&question.id) {
                return;
            }
            eprint!("{}", render(&question));
            let _ = std::io::stderr().flush();
            let reply = match read_line() {
                Some(line) => interpret(&question, &line),
                None => Reply::Skip,
            };
            match reply {
                Reply::Answer(text) => {
                    let _ = board.questions().answer(
                        &question.parent_session_id,
                        &question.id,
                        &text,
                        "the person",
                    );
                }
                Reply::Skip => board.questions().close(&question.id),
            }
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn question(options: &[&str]) -> PendingQuestion {
        PendingQuestion {
            id: "q1".into(),
            worker_id: "w1".into(),
            label: "Totals".into(),
            parent_session_id: "p1".into(),
            question: "Which year?".into(),
            options: options.iter().map(|o| o.to_string()).collect(),
            asked_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_number_picks_an_option_and_anything_else_is_the_answer() {
        let q = question(&["2025", "2026"]);
        assert_eq!(interpret(&q, "2\n"), Reply::Answer("2026".into()));
        assert_eq!(interpret(&q, " 1 "), Reply::Answer("2025".into()));
        assert_eq!(
            interpret(&q, "3"),
            Reply::Answer("3".into()),
            "a number outside the choices is the answer as typed"
        );
        assert_eq!(
            interpret(&q, "fiscal 2027"),
            Reply::Answer("fiscal 2027".into())
        );
        assert_eq!(
            interpret(&question(&[]), "2"),
            Reply::Answer("2".into()),
            "with no choices a number is just text"
        );
        assert_eq!(interpret(&q, "   \n"), Reply::Skip);
    }

    fn ask(attended: bool, typed: &'static str) -> (vak_tools::ToolOutput, Arc<WorkerRegistry>) {
        let gates: Arc<dyn Approver> = Arc::new(vak_agent::AutoDeny);
        let approver = Arc::new(CliApprover::with(
            gates,
            attended,
            Arc::new(move || Some(typed.to_string())),
        ));
        let registry = Arc::new(WorkerRegistry::new());
        let tool = vak_agent::AskParentTool::new(
            registry.clone(),
            "w1".into(),
            "Totals".into(),
            "p1".into(),
            approver.answers_questions(),
            None,
            Some(approver),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let args = serde_json::json!({"question": "Which year?", "options": ["2025", "2026"]});
        let out = runtime.block_on(async {
            use vak_tools::Tool;
            tool.execute(&args, &vak_tools::ToolContext::new(std::env::temp_dir()))
                .await
        });
        (out, registry)
    }

    #[test]
    fn a_person_at_the_terminal_answers_by_number() {
        let (out, registry) = ask(true, "2\n");
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("Answer from the person: 2026"),
            "{}",
            out.content
        );
        assert!(registry.questions().pending("p1").is_empty());
    }

    #[test]
    fn a_person_at_the_terminal_answers_in_their_own_words() {
        let (out, _) = ask(true, "use fiscal 2027\n");
        assert!(
            out.content
                .contains("Answer from the person: use fiscal 2027"),
            "{}",
            out.content
        );
    }

    #[test]
    fn pressing_enter_skips_and_the_worker_is_told_no_answer_arrived() {
        let (out, registry) = ask(true, "\n");
        assert!(out.content.contains("No answer arrived"), "{}", out.content);
        assert!(registry.questions().pending("p1").is_empty());
    }

    #[test]
    fn an_unattended_run_never_waits_for_a_keyboard() {
        let (out, _) = ask(false, "2\n");
        assert!(
            out.content.contains("Nobody is available to answer"),
            "{}",
            out.content
        );
    }

    #[test]
    fn gates_are_still_decided_by_the_wrapped_approver() {
        let approver = CliApprover::with(Arc::new(vak_agent::AutoDeny), true, Arc::new(|| None));
        assert!(
            !approver.answerable(),
            "a terminal that auto-denies gates is not a gate answerer"
        );
        assert!(
            approver.answers_questions(),
            "but it can still ask a question"
        );
    }

    #[test]
    fn the_prompt_numbers_the_choices_and_says_it_is_not_permission() {
        let text = render(&question(&["2025", "2026"]));
        assert!(text.contains("Totals asks: Which year?"));
        assert!(text.contains("1) 2025") && text.contains("2) 2026"));
        assert!(text.contains("does not approve any action"));
        assert!(!render(&question(&[])).contains("1)"));
    }
}
