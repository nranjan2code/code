//! vak-agent: the agent loop.
//!
//! Errors are values: run() never panics; every failure mode is a typed
//! TurnOutcome. Model context is always projected from the session log
//! (model-visible means logged).

pub mod steering;

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use vak_llm::{
    AssistantMessage, ChatRequest, ContentBlock, LlmError, Message, Provider, Role, StopReason,
    StreamEvent, Usage,
};
use vak_session::{MessageMeta, MessageRecord, SessionLog};
use vak_tools::Tool;

pub use steering::{DrainMode, SteeringQueues};

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TurnStart {
        turn: usize,
    },
    Stream(StreamEvent),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallEnd {
        id: String,
        name: String,
        is_error: bool,
    },
    TurnEnd {
        usage: Usage,
    },
}

#[derive(Debug)]
pub enum TurnOutcome {
    Completed { response: AssistantMessage },
    Aborted { partial: Option<AssistantMessage> },
    Failed { error: LlmError },
    MaxTurnsReached,
}

pub struct AgentConfig {
    pub system_prompt: String,
    pub tools: Vec<Arc<dyn Tool>>,
    pub max_turns: usize,
    pub parallel_tools: bool,
}

impl AgentConfig {
    pub fn new(system_prompt: impl Into<String>) -> Self {
        AgentConfig {
            system_prompt: system_prompt.into(),
            tools: Vec::new(),
            max_turns: 40,
            parallel_tools: true,
        }
    }
}

struct PendingToolCall {
    id: String,
    name: String,
    input: Value,
}

pub struct Agent {
    provider: Arc<dyn Provider>,
    pub session: Mutex<SessionLog>,
    pub config: AgentConfig,
    pub steering: SteeringQueues,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, session: SessionLog, config: AgentConfig) -> Self {
        Agent {
            provider,
            session: Mutex::new(session),
            config,
            steering: SteeringQueues::default(),
        }
    }

    pub async fn run(
        &mut self,
        prompt: &str,
        cancel: CancellationToken,
        events: mpsc::Sender<AgentEvent>,
    ) -> TurnOutcome {
        if let Err(e) = self
            .session
            .lock()
            .await
            .append_message(MessageRecord {
                message: Message::user_text(prompt),
                meta: None,
            })
            .map_err(|e| LlmError::Network(format!("session write failed: {e}")))
        {
            return TurnOutcome::Failed { error: e };
        }

        let mut turn = 0usize;
        loop {
            if cancel.is_cancelled() {
                return TurnOutcome::Aborted { partial: None };
            }
            if turn >= self.config.max_turns {
                return TurnOutcome::MaxTurnsReached;
            }

            {
                let mut session = self.session.lock().await;
                for text in self.steering.drain(DrainMode::OneAtATime) {
                    let _ = session.append_message(MessageRecord {
                        message: Message::user_text(text),
                        meta: None,
                    });
                }
            }

            let _ = events.send(AgentEvent::TurnStart { turn }).await;

            let request = {
                let session = self.session.lock().await;
                ChatRequest {
                    model: session
                        .header()
                        .map(|h| h.contract.model.clone())
                        .unwrap_or_default(),
                    system: Some(self.config.system_prompt.clone()),
                    messages: session.derive_messages(),
                    tools: vak_tools::definitions(&self.config.tools),
                    max_tokens: 8192,
                    temperature: None,
                }
            };

            let mut stream = match self.provider.stream(request, cancel.clone()).await {
                Ok(s) => s,
                Err(e) => return llm_error_outcome(e),
            };

            while let Some(ev) = futures::StreamExt::next(&mut stream).await {
                if events.send(AgentEvent::Stream(ev)).await.is_err() {
                    cancel.cancel();
                }
            }

            let response = match stream.result().await {
                Ok(r) => r,
                Err(LlmError::Aborted { partial }) => {
                    if let Some(p) = &partial {
                        self.append_assistant(p).await;
                    }
                    return TurnOutcome::Aborted { partial };
                }
                Err(e) => return TurnOutcome::Failed { error: e },
            };

            let usage = response.usage.clone();
            self.append_assistant(&response).await;
            let _ = events.send(AgentEvent::TurnEnd { usage }).await;

            if response.stop_reason != StopReason::ToolUse {
                return TurnOutcome::Completed { response };
            }

            let calls = extract_tool_calls(&response);
            if calls.is_empty() {
                return TurnOutcome::Completed { response };
            }

            let results = self.execute_batch(calls, &cancel, &events).await;
            let blocks = results
                .into_iter()
                .map(|(id, out)| match out {
                    ToolRunOutput::Ok(content) => ContentBlock::tool_result(id, content),
                    ToolRunOutput::Err(content) => ContentBlock::tool_error(id, content),
                })
                .collect();

            if let Err(e) = self
                .session
                .lock()
                .await
                .append_message(MessageRecord {
                    message: Message {
                        role: Role::User,
                        content: blocks,
                    },
                    meta: None,
                })
                .map_err(|e| LlmError::Network(format!("session write failed: {e}")))
            {
                return TurnOutcome::Failed { error: e };
            }

            turn += 1;
        }
    }

    async fn append_assistant(&self, response: &AssistantMessage) {
        let mut session = self.session.lock().await;
        let _ = session.append_message(MessageRecord {
            message: response.clone().into_message(),
            meta: Some(MessageMeta {
                model: Some(response.model.clone()),
                stop_reason: Some(format!("{:?}", response.stop_reason).to_lowercase()),
                usage: Some(response.usage.clone()),
            }),
        });
    }

    async fn execute_batch(
        &self,
        calls: Vec<PendingToolCall>,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Vec<(String, ToolRunOutput)> {
        let n = calls.len();

        if !self.config.parallel_tools || n == 1 {
            let mut out = Vec::with_capacity(n);
            for call in calls {
                if cancel.is_cancelled() {
                    out.push((call.id, ToolRunOutput::Err("cancelled".into())));
                    continue;
                }
                out.push(execute_one(call, &self.config.tools, cancel, events).await);
            }
            return out;
        }

        let mut join = tokio::task::JoinSet::new();
        for (idx, call) in calls.into_iter().enumerate() {
            let tools = self.config.tools.clone();
            let cancel = cancel.clone();
            let events = events.clone();
            join.spawn(async move {
                let r = execute_one(call, &tools, &cancel, &events).await;
                (idx, r)
            });
        }
        let mut ordered: Vec<Option<(String, ToolRunOutput)>> = (0..n).map(|_| None).collect();
        while let Some(res) = join.join_next().await {
            if let Ok((idx, pair)) = res {
                ordered[idx] = Some(pair);
            }
        }
        ordered.into_iter().flatten().collect()
    }
}

async fn execute_one(
    call: PendingToolCall,
    tools: &[Arc<dyn Tool>],
    cancel: &CancellationToken,
    events: &mpsc::Sender<AgentEvent>,
) -> (String, ToolRunOutput) {
    let _ = events
        .send(AgentEvent::ToolCallStart {
            id: call.id.clone(),
            name: call.name.clone(),
        })
        .await;

    let tool = tools.iter().find(|t| t.name() == call.name);

    let output = match tool {
        None => ToolRunOutput::Err(format!(
            "unknown tool: {} (available: {})",
            call.name,
            tools
                .iter()
                .map(|t| t.name())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Some(tool) => {
            let ctx = vak_tools::ToolContext {
                cwd: std::env::current_dir().unwrap_or_else(|_| ".".into()),
                cancel: cancel.child_token(),
                limits: Default::default(),
            };
            let tool = tool.clone();
            let res = tokio::spawn(async move { tool.execute(&call.input, &ctx).await }).await;
            match res {
                Ok(out) if out.is_error => ToolRunOutput::Err(out.content),
                Ok(out) => ToolRunOutput::Ok(out.content),
                Err(join_err) => ToolRunOutput::Err(format!("tool task failed: {join_err}")),
            }
        }
    };

    let _ = events
        .send(AgentEvent::ToolCallEnd {
            id: call.id.clone(),
            name: call.name.clone(),
            is_error: matches!(output, ToolRunOutput::Err(_)),
        })
        .await;

    (call.id, output)
}

enum ToolRunOutput {
    Ok(String),
    Err(String),
}

fn extract_tool_calls(response: &AssistantMessage) -> Vec<PendingToolCall> {
    response
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => Some(PendingToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn llm_error_outcome(e: LlmError) -> TurnOutcome {
    match e {
        LlmError::Aborted { partial } => TurnOutcome::Aborted { partial },
        other => TurnOutcome::Failed { error: other },
    }
}
