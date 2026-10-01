//! A worker asking its parent a question (docs/design/84-worker-questions-
//! and-control.md §4).
//!
//! The board is a small seam inside [`crate::WorkerRegistry`]: one pending
//! question per worker, a oneshot channel back to the blocked `ask_parent`
//! call, and an expiry. Whoever can answer (a person through a surface, or
//! the parent model through `workers reply`) calls [`QuestionBoard::answer`];
//! the first answer wins. An answer is information, never permission: it
//! changes what the worker knows, and every gated call still reaches the
//! approver and the permission engine as before.

use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicU32, Ordering},
};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::oneshot;
use vak_tools::{Tool, ToolContext, ToolOutput};

pub const MAX_QUESTION_CHARS: usize = 500;
pub const MAX_OPTIONS: usize = 6;
pub const MAX_OPTION_CHARS: usize = 80;
pub const MAX_ANSWER_CHARS: usize = 2_000;
pub const MAX_QUESTIONS_PER_WORKER: u32 = 3;

/// How long a question waits for an answer before the worker is told none
/// arrived. The same bound as an HTTP-surfaced approval gate.
pub const QUESTION_TIMEOUT: Duration = Duration::from_secs(900);

/// What the worker is told when nobody can answer, instead of blocking.
const NOBODY_AVAILABLE: &str = "Nobody is available to answer right now. Decide from your task and state the assumption you made, or stop and say what you need.";

/// A question as a surface or the parent model sees it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PendingQuestion {
    pub id: String,
    /// The worker's session id.
    pub worker_id: String,
    pub label: String,
    pub parent_session_id: String,
    pub question: String,
    pub options: Vec<String>,
    pub asked_at: chrono::DateTime<chrono::Utc>,
}

/// An answer and where it came from (`person:<name>`, `agent:<id>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerError {
    /// No such pending question for this parent: it was answered, expired,
    /// cancelled, or belongs to another session. Never distinguishes which.
    Unknown,
    Empty,
    TooLong {
        max: usize,
    },
}

impl std::fmt::Display for AnswerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnswerError::Unknown => write!(f, "no such pending question"),
            AnswerError::Empty => write!(f, "an answer cannot be empty"),
            AnswerError::TooLong { max } => write!(f, "an answer is at most {max} characters"),
        }
    }
}

struct Slot {
    info: PendingQuestion,
    tx: Option<oneshot::Sender<Answer>>,
}

/// Pending worker questions, keyed by question id. Interior-mutable and
/// shared: surfaces and the parent's `workers` tool hold the same board.
#[derive(Default)]
pub struct QuestionBoard {
    inner: Mutex<BTreeMap<String, Slot>>,
}

impl std::fmt::Debug for QuestionBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuestionBoard")
            .field("pending", &self.lock().len())
            .finish()
    }
}

impl QuestionBoard {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Slot>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Register a question. `None` when this worker already has one pending:
    /// a worker asks one at a time.
    pub fn open(&self, info: PendingQuestion) -> Option<oneshot::Receiver<Answer>> {
        let mut map = self.lock();
        if map
            .values()
            .any(|slot| slot.info.worker_id == info.worker_id)
        {
            return None;
        }
        let (tx, rx) = oneshot::channel();
        map.insert(info.id.clone(), Slot { info, tx: Some(tx) });
        Some(rx)
    }

    /// Answer a pending question of `parent`'s. The first answer wins; a
    /// second, or one for an expired question, resolves nothing.
    pub fn answer(
        &self,
        parent_session_id: &str,
        question_id: &str,
        text: &str,
        source: &str,
    ) -> Result<PendingQuestion, AnswerError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(AnswerError::Empty);
        }
        if text.chars().count() > MAX_ANSWER_CHARS {
            return Err(AnswerError::TooLong {
                max: MAX_ANSWER_CHARS,
            });
        }
        let mut map = self.lock();
        let owned = map
            .get(question_id)
            .is_some_and(|slot| slot.info.parent_session_id == parent_session_id);
        if !owned {
            return Err(AnswerError::Unknown);
        }
        let Some(mut slot) = map.remove(question_id) else {
            return Err(AnswerError::Unknown);
        };
        if let Some(tx) = slot.tx.take() {
            let _ = tx.send(Answer {
                text: text.to_string(),
                source: source.to_string(),
            });
        }
        Ok(slot.info)
    }

    /// Withdraw a question without answering (expiry, cancel).
    pub fn close(&self, question_id: &str) {
        self.lock().remove(question_id);
    }

    /// The questions waiting on `parent_session_id`'s workers, oldest first.
    pub fn pending(&self, parent_session_id: &str) -> Vec<PendingQuestion> {
        let mut out: Vec<PendingQuestion> = self
            .lock()
            .values()
            .filter(|slot| slot.info.parent_session_id == parent_session_id)
            .map(|slot| slot.info.clone())
            .collect();
        out.sort_by_key(|question| question.asked_at);
        out
    }

    /// The pending question for one worker, if any.
    pub fn for_worker(&self, worker_id: &str) -> Option<PendingQuestion> {
        self.lock()
            .values()
            .find(|slot| slot.info.worker_id == worker_id)
            .map(|slot| slot.info.clone())
    }

    /// Fail every question of `parent_session_id` closed (pause, permission
    /// change): each blocked worker is told no answer arrived. Returns how
    /// many were withdrawn.
    pub fn deny_all(&self, parent_session_id: &str) -> usize {
        let mut map = self.lock();
        let ids: Vec<String> = map
            .iter()
            .filter(|(_, slot)| slot.info.parent_session_id == parent_session_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            map.remove(id);
        }
        ids.len()
    }
}

/// The child-only `ask_parent` tool.
pub struct AskParentTool {
    board: Arc<crate::task::WorkerRegistry>,
    worker_id: String,
    label: String,
    parent_session_id: String,
    /// Whether anyone could answer: the parent's approver reaches somebody.
    /// `false` ends the call at once instead of blocking a worker forever.
    answerable: bool,
    asked: AtomicU32,
    events: Option<tokio::sync::mpsc::Sender<crate::AgentEvent>>,
    timeout: Duration,
}

impl AskParentTool {
    pub const DESCRIPTION: &'static str = "Ask the person (or the agent that started you) one specific question when you cannot continue without a decision only they can make. Do not ask what you can decide from your task or look up yourself. An answer is information, not permission: it does not approve any action your tools would otherwise ask about. At most 3 questions per task, one at a time.";

    pub fn new(
        board: Arc<crate::task::WorkerRegistry>,
        worker_id: String,
        label: String,
        parent_session_id: String,
        answerable: bool,
        events: Option<tokio::sync::mpsc::Sender<crate::AgentEvent>>,
    ) -> Self {
        AskParentTool {
            board,
            worker_id,
            label,
            parent_session_id,
            answerable,
            asked: AtomicU32::new(0),
            events,
            timeout: QUESTION_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait::async_trait]
impl Tool for AskParentTool {
    fn name(&self) -> &str {
        "ask_parent"
    }

    fn description(&self) -> &str {
        Self::DESCRIPTION
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "One specific question, in the person's language",
                    "maxLength": MAX_QUESTION_CHARS
                },
                "options": {
                    "type": "array",
                    "items": {"type": "string", "maxLength": MAX_OPTION_CHARS},
                    "maxItems": MAX_OPTIONS,
                    "description": "Optional short choices the answer can pick from"
                }
            },
            "required": ["question"]
        })
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: false,
            read_only: true,
            paths: Vec::new(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let question = args
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if question.is_empty() {
            return ToolOutput::error("missing required parameter: question");
        }
        if question.chars().count() > MAX_QUESTION_CHARS {
            return ToolOutput::error(format!(
                "the question is too long: at most {MAX_QUESTION_CHARS} characters. Ask one specific question."
            ));
        }
        let options: Vec<String> = args
            .get("options")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if options.len() > MAX_OPTIONS
            || options
                .iter()
                .any(|option| option.chars().count() > MAX_OPTION_CHARS)
        {
            return ToolOutput::error(format!(
                "at most {MAX_OPTIONS} options of {MAX_OPTION_CHARS} characters each"
            ));
        }
        if self.asked.load(Ordering::SeqCst) >= MAX_QUESTIONS_PER_WORKER {
            return ToolOutput::error(format!(
                "you have used your {MAX_QUESTIONS_PER_WORKER} questions. Decide from what you know and state your assumption."
            ));
        }
        if !self.answerable {
            return ToolOutput::ok(NOBODY_AVAILABLE);
        }
        let info = PendingQuestion {
            id: uuid::Uuid::now_v7().to_string(),
            worker_id: self.worker_id.clone(),
            label: self.label.clone(),
            parent_session_id: self.parent_session_id.clone(),
            question: question.to_string(),
            options: options.clone(),
            asked_at: chrono::Utc::now(),
        };
        let Some(rx) = self.board.questions().open(info.clone()) else {
            return ToolOutput::error(
                "you already have a question waiting for an answer; wait for it first",
            );
        };
        self.asked.fetch_add(1, Ordering::SeqCst);
        if let Some(events) = &self.events {
            let _ = events
                .send(crate::AgentEvent::WorkerQuestion {
                    id: info.id.clone(),
                    label: info.label.clone(),
                    question: info.question.clone(),
                    options,
                })
                .await;
        }
        let outcome = tokio::select! {
            answer = rx => answer.ok(),
            _ = tokio::time::sleep(self.timeout) => None,
            _ = ctx.cancel.cancelled() => {
                self.board.questions().close(&info.id);
                return ToolOutput::error("cancelled while waiting for an answer");
            }
        };
        // Dropping the entry on expiry means a reply that arrives later
        // resolves nothing (AGENTS.md invariant 15).
        self.board.questions().close(&info.id);
        let Some(answer) = outcome else {
            return ToolOutput::ok(
                "No answer arrived. Decide from your task and state the assumption you made, or stop and say what you need.",
            );
        };
        if let Some(events) = &self.events {
            let _ = events
                .send(crate::AgentEvent::WorkerQuestionAnswered {
                    id: info.id.clone(),
                    label: info.label.clone(),
                    source: answer.source.clone(),
                })
                .await;
        }
        ToolOutput::ok(format!(
            "Answer from {}: {}\n(This is information, not permission.)",
            answer.source, answer.text
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::task::WorkerRegistry;

    fn question(worker: &str, parent: &str) -> PendingQuestion {
        PendingQuestion {
            id: format!("q-{worker}"),
            worker_id: worker.into(),
            label: "Researcher".into(),
            parent_session_id: parent.into(),
            question: "Which year?".into(),
            options: vec![],
            asked_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn the_first_answer_wins_and_a_second_resolves_nothing() {
        let board = QuestionBoard::default();
        let mut rx = board.open(question("w1", "p1")).unwrap();
        assert!(board.answer("p1", "q-w1", "2026", "person:you").is_ok());
        assert_eq!(rx.try_recv().unwrap().text, "2026");
        assert_eq!(
            board.answer("p1", "q-w1", "2025", "agent:x"),
            Err(AnswerError::Unknown)
        );
    }

    #[test]
    fn a_question_is_answerable_only_by_its_own_parent() {
        let board = QuestionBoard::default();
        let _rx = board.open(question("w1", "p1")).unwrap();
        assert_eq!(
            board.answer("other", "q-w1", "x", "person:you"),
            Err(AnswerError::Unknown)
        );
        assert!(board.pending("other").is_empty());
        assert_eq!(board.pending("p1").len(), 1);
    }

    #[test]
    fn a_worker_asks_one_at_a_time() {
        let board = QuestionBoard::default();
        let _rx = board.open(question("w1", "p1")).unwrap();
        let mut second = question("w1", "p1");
        second.id = "q-other".into();
        assert!(board.open(second).is_none());
    }

    #[test]
    fn answers_are_bounded() {
        let board = QuestionBoard::default();
        let _rx = board.open(question("w1", "p1")).unwrap();
        assert_eq!(
            board.answer("p1", "q-w1", "  ", "person:you"),
            Err(AnswerError::Empty)
        );
        let long = "x".repeat(MAX_ANSWER_CHARS + 1);
        assert_eq!(
            board.answer("p1", "q-w1", &long, "person:you"),
            Err(AnswerError::TooLong {
                max: MAX_ANSWER_CHARS
            })
        );
        assert_eq!(
            board.pending("p1").len(),
            1,
            "a refused answer keeps it open"
        );
    }

    #[test]
    fn denying_a_parents_questions_leaves_other_parents_alone() {
        let board = QuestionBoard::default();
        let mut rx = board.open(question("w1", "p1")).unwrap();
        let _other = board.open(question("w2", "p2")).unwrap();
        assert_eq!(board.deny_all("p1"), 1);
        assert!(rx.try_recv().is_err());
        assert_eq!(board.pending("p2").len(), 1);
    }

    fn tool(answerable: bool, timeout: Duration) -> (AskParentTool, Arc<WorkerRegistry>) {
        let registry = Arc::new(WorkerRegistry::new());
        let tool = AskParentTool::new(
            registry.clone(),
            "w1".into(),
            "Researcher".into(),
            "p1".into(),
            answerable,
            None,
        )
        .with_timeout(timeout);
        (tool, registry)
    }

    fn context() -> ToolContext {
        ToolContext::new(std::env::temp_dir())
    }

    #[tokio::test]
    async fn an_unanswerable_surface_ends_the_call_at_once() {
        let (tool, registry) = tool(false, Duration::from_secs(60));
        let out = tool
            .execute(&json!({"question": "Which year?"}), &context())
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("Nobody is available"));
        assert!(registry.questions().pending("p1").is_empty());
    }

    #[tokio::test]
    async fn a_person_answer_reaches_the_worker_as_information() {
        let (tool, registry) = tool(true, Duration::from_secs(60));
        let ctx = context();
        let args = json!({"question": "Which year?", "options": ["2025", "2026"]});
        let ask = tool.execute(&args, &ctx);
        let answer = async {
            let pending = loop {
                let pending = registry.questions().pending("p1");
                if let Some(first) = pending.into_iter().next() {
                    break first;
                }
                tokio::task::yield_now().await;
            };
            assert_eq!(pending.options, vec!["2025", "2026"]);
            registry
                .questions()
                .answer("p1", &pending.id, "2026", "person:you")
                .unwrap();
        };
        let (out, ()) = tokio::join!(ask, answer);
        assert!(!out.is_error);
        assert!(
            out.content.contains("Answer from person:you: 2026"),
            "{}",
            out.content
        );
        assert!(out.content.contains("not permission"));
    }

    #[tokio::test]
    async fn a_late_answer_resolves_nothing() {
        let (tool, registry) = tool(true, Duration::from_millis(50));
        let out = tool
            .execute(&json!({"question": "Which year?"}), &context())
            .await;
        assert!(out.content.contains("No answer arrived"));
        assert!(registry.questions().pending("p1").is_empty());
        assert_eq!(
            registry
                .questions()
                .answer("p1", "anything", "late", "person:you"),
            Err(AnswerError::Unknown)
        );
    }

    #[tokio::test]
    async fn the_fourth_question_is_refused() {
        let (tool, _registry) = tool(false, Duration::from_secs(1));
        // Unanswerable calls do not count; use an answerable tool that times out.
        drop(tool);
        let (tool, _registry) = self::tool(true, Duration::from_millis(20));
        let ctx = context();
        for _ in 0..MAX_QUESTIONS_PER_WORKER {
            let out = tool.execute(&json!({"question": "again?"}), &ctx).await;
            assert!(out.content.contains("No answer arrived"));
        }
        let out = tool.execute(&json!({"question": "once more?"}), &ctx).await;
        assert!(out.is_error);
        assert!(out.content.contains("used your 3 questions"));
    }

    #[tokio::test]
    async fn over_long_input_is_refused_not_cut() {
        let (tool, _registry) = tool(true, Duration::from_millis(20));
        let out = tool
            .execute(
                &json!({"question": "x".repeat(MAX_QUESTION_CHARS + 1)}),
                &context(),
            )
            .await;
        assert!(out.is_error);
        let out = tool
            .execute(
                &json!({"question": "ok?", "options": ["a","b","c","d","e","f","g"]}),
                &context(),
            )
            .await;
        assert!(out.is_error);
    }
}
