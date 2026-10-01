//! The model-facing `workers` tool: a parent Agent looking at, talking to,
//! waiting on and stopping its own background workers
//! (docs/design/84-worker-questions-and-control.md §5.3).
//!
//! Authority is the control matrix of AGENTS.md invariant 32: an Agent acts
//! on its own children and nothing else. Every action filters by the parent
//! session this tool was built for, and an id that is not that session's is
//! reported as unknown, never as forbidden, so nothing leaks about another
//! session's workers. A message carries no authority: it is steering text, and
//! the worker's permissions are unchanged.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use vak_tools::{Tool, ToolContext, ToolOutput};

use crate::task::{ActiveWorker, FinishedWorker, WorkerRegistry};

const DEFAULT_WAIT_SECS: u64 = 60;
const MAX_WAIT_SECS: u64 = 600;
const MAX_MESSAGE_CHARS: usize = 2_000;
/// How often a waiter re-checks, so a wake-up it missed costs one tick.
const WAIT_TICK: Duration = Duration::from_millis(250);

pub struct WorkersTool {
    registry: Arc<WorkerRegistry>,
    parent_session_id: String,
}

impl WorkersTool {
    pub const SERVES: &'static [&'static str] = &["orchestration"];

    pub const DESCRIPTION: &'static str = "Look at, talk to, wait for and stop the background workers you started with task. Actions: list; status {id}; message {id, text} sends steering text to a worker; reply {id, text} answers a question a worker is waiting on; wait {ids?, timeout_secs?} blocks until workers finish and returns their results; result {id} reads a finished worker's result; stop {id}. Only your own workers are visible. A message or an answer is information, not permission: the worker's own permissions do not change.";

    pub fn new(registry: Arc<WorkerRegistry>, parent_session_id: String) -> Self {
        WorkersTool {
            registry,
            parent_session_id,
        }
    }

    fn describe_live(&self, worker: &ActiveWorker) -> String {
        let mut line = format!(
            "{}  {}  running ({}s, {} steps",
            worker.id, worker.label, worker.elapsed_secs, worker.steps
        );
        if !worker.last_tools.is_empty() {
            line.push_str(&format!(", last: {}", worker.last_tools.join(", ")));
        }
        line.push(')');
        if let Some(question) = self.registry.questions().for_worker(&worker.id) {
            line.push_str(&format!(
                "  WAITING FOR AN ANSWER: \"{}\"",
                question.question
            ));
            if !question.options.is_empty() {
                line.push_str(&format!(" [{}]", question.options.join(" | ")));
            }
        }
        line
    }

    fn describe_finished(worker: &FinishedWorker) -> String {
        format!(
            "{}  {}  {} ({}s)",
            worker.id,
            worker.label,
            if worker.is_error {
                "failed or cancelled"
            } else {
                "finished"
            },
            worker.elapsed_secs
        )
    }

    fn list(&self) -> ToolOutput {
        let live = self.registry.active_for(&self.parent_session_id);
        let finished = self.registry.finished_for(&self.parent_session_id);
        if live.is_empty() && finished.is_empty() {
            return ToolOutput::ok("No workers.");
        }
        let mut lines: Vec<String> = live.iter().map(|w| self.describe_live(w)).collect();
        lines.extend(finished.iter().map(Self::describe_finished));
        ToolOutput::ok(lines.join("\n"))
    }

    fn status(&self, id: &str) -> ToolOutput {
        if let Some(worker) = self.registry.live_child(&self.parent_session_id, id) {
            let mut text = self.describe_live(&worker);
            text.push_str(&format!(
                "\ntokens: {} in, {} out",
                worker.input_tokens, worker.output_tokens
            ));
            return ToolOutput::ok(text);
        }
        if let Some(worker) = self.registry.finished_child(&self.parent_session_id, id) {
            return ToolOutput::ok(format!(
                "{}\nRead its result with result {{id}}.",
                Self::describe_finished(&worker)
            ));
        }
        unknown(id)
    }

    fn message(&self, id: &str, text: &str) -> ToolOutput {
        let text = text.trim();
        if text.is_empty() {
            return ToolOutput::error("text is required");
        }
        if text.chars().count() > MAX_MESSAGE_CHARS {
            return ToolOutput::error(format!(
                "a message is at most {MAX_MESSAGE_CHARS} characters"
            ));
        }
        if self
            .registry
            .live_child(&self.parent_session_id, id)
            .is_none()
        {
            return unknown(id);
        }
        // Prefixed so the worker can tell where it came from.
        let steered = self.registry.steer(
            id,
            &format!("Message from the agent that started you: {text}"),
        );
        if steered {
            ToolOutput::ok("Message queued; the worker reads it at its next step.")
        } else {
            unknown(id)
        }
    }

    fn reply(&self, id: &str, text: &str) -> ToolOutput {
        if self
            .registry
            .live_child(&self.parent_session_id, id)
            .is_none()
        {
            return unknown(id);
        }
        let Some(question) = self.registry.questions().for_worker(id) else {
            return ToolOutput::error("that worker is not waiting for an answer");
        };
        match self.registry.questions().answer(
            &self.parent_session_id,
            &question.id,
            text,
            "the agent that started you",
        ) {
            Ok(_) => ToolOutput::ok("Answered."),
            Err(error) => ToolOutput::error(error.to_string()),
        }
    }

    fn result(&self, id: &str) -> ToolOutput {
        if let Some(worker) = self.registry.finished_child(&self.parent_session_id, id) {
            return if worker.is_error {
                ToolOutput::error(worker.text)
            } else {
                ToolOutput::ok(worker.text)
            };
        }
        if self
            .registry
            .live_child(&self.parent_session_id, id)
            .is_some()
        {
            return ToolOutput::ok(
                "That worker is still running. Use wait to block until it finishes.",
            );
        }
        unknown(id)
    }

    fn stop(&self, id: &str) -> ToolOutput {
        if self
            .registry
            .live_child(&self.parent_session_id, id)
            .is_some()
            && self.registry.stop(id)
        {
            return ToolOutput::ok("Stopping the worker; its partial output is kept.");
        }
        unknown(id)
    }

    async fn wait(&self, ids: Vec<String>, timeout: Duration, ctx: &ToolContext) -> ToolOutput {
        let ids: Vec<String> = if ids.is_empty() {
            let mut all: Vec<String> = self
                .registry
                .active_for(&self.parent_session_id)
                .into_iter()
                .filter(|w| w.background)
                .map(|w| w.id)
                .collect();
            all.extend(
                self.registry
                    .finished_for(&self.parent_session_id)
                    .into_iter()
                    .map(|w| w.id),
            );
            all
        } else {
            ids
        };
        if ids.is_empty() {
            return ToolOutput::ok("No workers to wait for.");
        }
        for id in &ids {
            let known = self
                .registry
                .live_child(&self.parent_session_id, id)
                .is_some()
                || self
                    .registry
                    .finished_child(&self.parent_session_id, id)
                    .is_some();
            if !known {
                return unknown(id);
            }
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let waiting: Vec<&String> = ids
                .iter()
                .filter(|id| {
                    self.registry
                        .live_child(&self.parent_session_id, id)
                        .is_some()
                })
                .collect();
            // A worker that asks a question is not going to finish by
            // itself; stop waiting and say so, so the parent can reply.
            let asking: Vec<String> = waiting
                .iter()
                .filter_map(|id| self.registry.questions().for_worker(id))
                .map(|q| {
                    format!(
                        "{} is waiting for an answer: \"{}\"",
                        q.worker_id, q.question
                    )
                })
                .collect();
            if waiting.is_empty() || !asking.is_empty() || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::select! {
                _ = tokio::time::timeout(WAIT_TICK, self.registry.changed()) => {}
                _ = ctx.cancel.cancelled() => {
                    return ToolOutput::error("cancelled while waiting for workers");
                }
            }
        }
        let mut sections = Vec::new();
        for id in &ids {
            match self.registry.finished_child(&self.parent_session_id, id) {
                Some(worker) => sections.push(format!(
                    "{} ({}): {}\n{}",
                    worker.id,
                    worker.label,
                    if worker.is_error {
                        "failed"
                    } else {
                        "finished"
                    },
                    worker.text
                )),
                None => match self.registry.live_child(&self.parent_session_id, id) {
                    Some(worker) => sections.push(self.describe_live(&worker)),
                    None => sections.push(format!("{id}: no longer running")),
                },
            }
        }
        ToolOutput::ok(sections.join("\n\n"))
    }
}

fn unknown(id: &str) -> ToolOutput {
    ToolOutput::error(format!("unknown worker '{id}'"))
}

#[async_trait::async_trait]
impl Tool for WorkersTool {
    fn name(&self) -> &str {
        "workers"
    }

    fn serves(&self) -> &'static [&'static str] {
        Self::SERVES
    }

    fn description(&self) -> &str {
        Self::DESCRIPTION
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "status", "message", "reply", "wait", "result", "stop"]
                },
                "id": {"type": "string", "description": "A worker id from list"},
                "ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "wait: the workers to wait for; omit for all of them"
                },
                "text": {"type": "string", "description": "message or reply text"},
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_WAIT_SECS,
                    "description": "wait: give up after this long and report what is still running"
                }
            },
            "required": ["action"]
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
        let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
        let text = args.get("text").and_then(Value::as_str).unwrap_or_default();
        let needs_id = |id: &str| -> Option<ToolOutput> {
            id.is_empty().then(|| ToolOutput::error("id is required"))
        };
        match args.get("action").and_then(Value::as_str) {
            Some("list") => self.list(),
            Some("status") => needs_id(id).unwrap_or_else(|| self.status(id)),
            Some("message") => needs_id(id).unwrap_or_else(|| self.message(id, text)),
            Some("reply") => needs_id(id).unwrap_or_else(|| self.reply(id, text)),
            Some("result") => needs_id(id).unwrap_or_else(|| self.result(id)),
            Some("stop") => needs_id(id).unwrap_or_else(|| self.stop(id)),
            Some("wait") => {
                let ids = args
                    .get("ids")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let secs = args
                    .get("timeout_secs")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_WAIT_SECS)
                    .clamp(1, MAX_WAIT_SECS);
                self.wait(ids, Duration::from_secs(secs), ctx).await
            }
            _ => ToolOutput::error(
                "action must be one of: list, status, message, reply, wait, result, stop",
            ),
        }
    }
}
