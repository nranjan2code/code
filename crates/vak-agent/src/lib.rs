//! Greenfield turn engine boundary.
//!
//! This crate defines the engine contract that Runtime owns: every
//! provider/tool effect is preceded by cancellation and capability checks,
//! and every visible output is emitted as a delta plus its complete snapshot.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vak_domain::{CapabilityEpoch, DomainError, Event, RunContext};
use vak_llm::{ContentBlock, Message, Role};
use vak_session::{MessageRecord, SessionLog};

#[async_trait]
pub trait TranscriptRecorder: Send + Sync {
    async fn append(&self, message: Message) -> Result<(), EngineError>;
}

pub struct SessionRecorder(pub Arc<tokio::sync::Mutex<SessionLog>>);

#[async_trait]
impl TranscriptRecorder for SessionRecorder {
    async fn append(&self, message: Message) -> Result<(), EngineError> {
        self.0
            .lock()
            .await
            .append_message(MessageRecord { message })
            .map(|_| ())
            .map_err(|e| EngineError::Provider(format!("session append: {e}")))
    }
}

/// Adapter for the production provider protocol. The Runtime may inject this
/// without giving the engine access to provider configuration or credentials.
pub struct LlmProviderAdapter {
    provider: Arc<dyn vak_llm::Provider>,
}

impl LlmProviderAdapter {
    pub fn new(provider: Arc<dyn vak_llm::Provider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Provider for LlmProviderAdapter {
    async fn respond(
        &self,
        context: &RunContext,
        transcript: &str,
        cancel: CancellationToken,
    ) -> Result<ModelResponse, EngineError> {
        let mut request = vak_llm::ChatRequest::new(context.contract.model.clone());
        request.system = Some(context.contract.system_prompt.clone());
        request
            .messages
            .push(vak_llm::Message::user_text(transcript));
        let stream = self
            .provider
            .stream(request, cancel)
            .await
            .map_err(|error| EngineError::Provider(error.to_string()))?;
        let message = stream
            .result()
            .await
            .map_err(|error| EngineError::Provider(error.to_string()))?;
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for block in message.content {
            match block {
                vak_llm::ContentBlock::Text { text: value } => text.push_str(&value),
                vak_llm::ContentBlock::ToolUse { name, input, .. } => {
                    tool_calls.push(ToolCall {
                        id: uuid::Uuid::now_v7().to_string(),
                        name,
                        arguments: input,
                    });
                }
                _ => {}
            }
        }
        Ok(ModelResponse { text, tool_calls })
    }

    async fn respond_messages(
        &self,
        context: &RunContext,
        messages: &[Message],
        cancel: CancellationToken,
    ) -> Result<ModelResponse, EngineError> {
        self.respond_messages_with_tools(context, messages, &[], cancel)
            .await
    }

    async fn respond_messages_with_tools(
        &self,
        context: &RunContext,
        messages: &[Message],
        tools: &[vak_llm::ToolDefinition],
        cancel: CancellationToken,
    ) -> Result<ModelResponse, EngineError> {
        let mut request = vak_llm::ChatRequest::new(context.contract.model.clone());
        request.system = Some(context.contract.system_prompt.clone());
        request.messages.extend_from_slice(messages);
        request.tools.extend_from_slice(tools);
        let stream = tokio::select! {
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
            result = self.provider.stream(request, cancel.clone()) => result.map_err(|error| EngineError::Provider(error.to_string()))?,
        };
        let message = tokio::select! {
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
            result = stream.result() => result.map_err(|error| EngineError::Provider(error.to_string()))?,
        };
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for block in message.content {
            match block {
                ContentBlock::Text { text: value } => text.push_str(&value),
                ContentBlock::ToolUse { id, name, input } => tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments: input,
                }),
                _ => {}
            }
        }
        Ok(ModelResponse { text, tool_calls })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn respond(
        &self,
        context: &RunContext,
        transcript: &str,
        cancel: CancellationToken,
    ) -> Result<ModelResponse, EngineError>;

    async fn respond_messages(
        &self,
        context: &RunContext,
        messages: &[Message],
        cancel: CancellationToken,
    ) -> Result<ModelResponse, EngineError> {
        let transcript = messages
            .iter()
            .map(Message::text_content)
            .collect::<Vec<_>>()
            .join("\n");
        self.respond(context, &transcript, cancel).await
    }

    async fn respond_messages_with_tools(
        &self,
        context: &RunContext,
        messages: &[Message],
        tools: &[vak_llm::ToolDefinition],
        cancel: CancellationToken,
    ) -> Result<ModelResponse, EngineError> {
        let _ = tools;
        self.respond_messages(context, messages, cancel).await
    }
}

#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    fn definitions(&self) -> Vec<vak_llm::ToolDefinition> {
        Vec::new()
    }

    async fn dispatch(
        &self,
        context: &RunContext,
        call: &ToolCall,
        cancel: CancellationToken,
    ) -> Result<ToolResult, EngineError>;
}

/// Runtime-owned capability state. Implementations normally delegate to the
/// Runtime run supervisor's epoch check; keeping it a trait avoids a runtime
/// dependency cycle and makes the engine deterministic in tests.
pub trait CapabilityGuard: Send + Sync {
    fn check(&self, expected: CapabilityEpoch) -> Result<(), DomainError>;
}

#[derive(Clone, Default)]
pub struct StaticCapabilityGuard {
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl StaticCapabilityGuard {
    pub fn new(epoch: CapabilityEpoch) -> Self {
        Self {
            epoch: Arc::new(std::sync::atomic::AtomicU64::new(epoch.0)),
        }
    }

    pub fn revoke(&self) -> CapabilityEpoch {
        CapabilityEpoch(self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1)
    }
}

impl CapabilityGuard for StaticCapabilityGuard {
    fn check(&self, expected: CapabilityEpoch) -> Result<(), DomainError> {
        let actual = CapabilityEpoch(self.epoch.load(std::sync::atomic::Ordering::Acquire));
        (actual == expected)
            .then_some(())
            .ok_or(DomainError::CapabilityRevoked { expected, actual })
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("cancelled")]
    Cancelled,
    #[error("cancelled after partial output")]
    CancelledWithOutput(String),
    #[error("capability revoked: {0}")]
    Capability(#[from] DomainError),
    #[error("provider: {0}")]
    Provider(String),
    #[error("tool {name}: {message}")]
    Tool { name: String, message: String },
    #[error("maximum turns reached")]
    MaxTurns,
}

impl EngineError {
    fn check(
        cancel: &CancellationToken,
        guard: &dyn CapabilityGuard,
        epoch: CapabilityEpoch,
    ) -> Result<(), Self> {
        if cancel.is_cancelled() {
            return Err(Self::Cancelled);
        }
        guard.check(epoch).map_err(Self::Capability)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunResult {
    pub output: String,
    pub tool_calls: usize,
}

pub struct AgentEngine {
    provider: Arc<dyn Provider>,
    tools: Arc<dyn ToolDispatcher>,
    guard: Arc<dyn CapabilityGuard>,
    max_turns: usize,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests_extra {
    use super::*;
    use tokio_util::sync::CancellationToken;
    use vak_domain::{
        PermissionMode, ProjectContext, ProjectId, RouteLeg, RunId, SandboxMode, SessionContract,
        SessionId,
    };

    struct FakeProvider;
    #[async_trait]
    impl Provider for FakeProvider {
        async fn respond(
            &self,
            _context: &RunContext,
            _transcript: &str,
            _cancel: CancellationToken,
        ) -> Result<ModelResponse, EngineError> {
            Ok(ModelResponse {
                text: "ok".into(),
                tool_calls: Vec::new(),
            })
        }
    }

    struct NoopTools;
    #[async_trait]
    impl ToolDispatcher for NoopTools {
        async fn dispatch(
            &self,
            _context: &RunContext,
            call: &ToolCall,
            _cancel: CancellationToken,
        ) -> Result<ToolResult, EngineError> {
            Ok(ToolResult {
                output: call.name.clone(),
                is_error: false,
            })
        }
    }

    fn context() -> RunContext {
        RunContext {
            run_id: RunId::new(),
            session_id: SessionId::new(),
            project: ProjectContext {
                id: ProjectId::new(),
                root: "/tmp/project".into(),
                display_name: None,
            },
            contract: SessionContract {
                provider: "test".into(),
                model: "test".into(),
                route_ladder: vec![RouteLeg {
                    provider: "test".into(),
                    model: "test".into(),
                }],
                system_prompt: "test".into(),
                permission_mode: PermissionMode::ReadOnly,
                sandbox: SandboxMode::None,
                tool_catalogue_revision: "1".into(),
                context_limit: 1000,
                budget_ceiling: None,
            },
            capability_epoch: CapabilityEpoch(0),
        }
    }

    #[tokio::test]
    async fn emits_delta_and_snapshot_for_visible_output() {
        let guard = Arc::new(StaticCapabilityGuard::new(CapabilityEpoch(0)));
        let engine = AgentEngine::new(Arc::new(FakeProvider), Arc::new(NoopTools), guard);
        let (events, mut received) = mpsc::channel(4);
        let result = engine
            .run(&context(), "hello", CancellationToken::new(), events)
            .await
            .expect("run");
        assert_eq!(result.output, "ok");
        match received.recv().await.expect("event") {
            Event::RunOutput {
                delta, snapshot, ..
            } => assert_eq!((delta, snapshot), ("ok".to_owned(), "ok".to_owned())),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn revoked_capability_stops_before_provider_dispatch() {
        let guard = Arc::new(StaticCapabilityGuard::new(CapabilityEpoch(0)));
        guard.revoke();
        let engine = AgentEngine::new(Arc::new(FakeProvider), Arc::new(NoopTools), guard);
        let (events, _) = mpsc::channel(1);
        assert!(matches!(
            engine
                .run(&context(), "hello", CancellationToken::new(), events)
                .await,
            Err(EngineError::Capability(_))
        ));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use vak_domain::{
        PermissionMode, ProjectContext, ProjectId, RouteLeg, RunId, SandboxMode, SessionContract,
        SessionId,
    };

    fn context(epoch: u64) -> RunContext {
        RunContext {
            run_id: RunId::new(),
            session_id: SessionId::new(),
            project: ProjectContext {
                id: ProjectId::new(),
                root: "/tmp/project".into(),
                display_name: None,
            },
            contract: SessionContract {
                provider: "fake".into(),
                model: "fake-model".into(),
                route_ladder: vec![RouteLeg {
                    provider: "fake".into(),
                    model: "fake-model".into(),
                }],
                system_prompt: "test".into(),
                permission_mode: PermissionMode::ReadOnly,
                sandbox: SandboxMode::None,
                tool_catalogue_revision: "test".into(),
                context_limit: 1000,
                budget_ceiling: None,
            },
            capability_epoch: CapabilityEpoch(epoch),
        }
    }

    struct FakeProvider {
        responses: Mutex<Vec<ModelResponse>>,
    }

    #[async_trait]
    impl Provider for FakeProvider {
        async fn respond(
            &self,
            _: &RunContext,
            _: &str,
            _: CancellationToken,
        ) -> Result<ModelResponse, EngineError> {
            self.responses
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| EngineError::Provider("no response".into()))
        }
    }

    struct FakeTools;

    #[async_trait]
    impl ToolDispatcher for FakeTools {
        async fn dispatch(
            &self,
            _: &RunContext,
            call: &ToolCall,
            _: CancellationToken,
        ) -> Result<ToolResult, EngineError> {
            Ok(ToolResult {
                output: format!("{} ok", call.name),
                is_error: false,
            })
        }
    }

    #[tokio::test]
    async fn emits_delta_and_snapshot_and_dispatches_after_epoch_check() {
        let provider = Arc::new(FakeProvider {
            responses: Mutex::new(vec![
                ModelResponse {
                    text: "done".into(),
                    tool_calls: vec![],
                },
                ModelResponse {
                    text: "working".into(),
                    tool_calls: vec![ToolCall {
                        id: "tool-1".into(),
                        name: "read".into(),
                        arguments: serde_json::json!({}),
                    }],
                },
            ]),
        });
        let guard = Arc::new(StaticCapabilityGuard::new(CapabilityEpoch(0)));
        let engine = AgentEngine::new(provider, Arc::new(FakeTools), guard);
        let ctx = context(0);
        let (tx, mut rx) = mpsc::channel(8);
        let result = engine
            .run(&ctx, "hello", CancellationToken::new(), tx)
            .await
            .expect("run");
        assert_eq!(result.output, "workingdone");
        assert_eq!(result.tool_calls, 1);
        let Event::RunOutput {
            delta, snapshot, ..
        } = rx.recv().await.expect("event")
        else {
            panic!("output event")
        };
        assert_eq!(
            (delta, snapshot),
            ("working".to_owned(), "working".to_owned())
        );
        let Event::RunOutput {
            delta, snapshot, ..
        } = rx.recv().await.expect("event")
        else {
            panic!("output event")
        };
        assert_eq!(
            (delta, snapshot),
            ("done".to_owned(), "workingdone".to_owned())
        );
    }

    #[tokio::test]
    async fn revoked_epoch_prevents_provider_dispatch() {
        let provider = Arc::new(FakeProvider {
            responses: Mutex::new(vec![]),
        });
        let guard = Arc::new(StaticCapabilityGuard::new(CapabilityEpoch(1)));
        let engine = AgentEngine::new(provider, Arc::new(FakeTools), guard);
        let (tx, _) = mpsc::channel(1);
        assert!(matches!(
            engine
                .run(&context(0), "hello", CancellationToken::new(), tx)
                .await,
            Err(EngineError::Capability(_))
        ));
    }
}

impl AgentEngine {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Arc<dyn ToolDispatcher>,
        guard: Arc<dyn CapabilityGuard>,
    ) -> Self {
        Self {
            provider,
            tools,
            guard,
            max_turns: 40,
        }
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// Execute one immutable-context run. The engine never chooses a project,
    /// loads configuration, or persists state; the Runtime supplies all of it.
    pub async fn run(
        &self,
        context: &RunContext,
        input: impl Into<String>,
        cancel: CancellationToken,
        events: mpsc::Sender<Event>,
    ) -> Result<RunResult, EngineError> {
        self.run_with_recorder(context, input, cancel, events, None)
            .await
    }

    pub async fn run_with_recorder(
        &self,
        context: &RunContext,
        input: impl Into<String>,
        cancel: CancellationToken,
        events: mpsc::Sender<Event>,
        recorder: Option<Arc<dyn TranscriptRecorder>>,
    ) -> Result<RunResult, EngineError> {
        let input = input.into();
        if let Some(recorder) = &recorder {
            recorder.append(Message::user_text(&input)).await?;
        }
        let mut messages = vec![Message::user_text(&input)];
        let mut snapshot = String::new();
        let mut tool_calls = 0;

        for _ in 0..self.max_turns {
            EngineError::check(&cancel, self.guard.as_ref(), context.capability_epoch)?;
            let tool_definitions = self.tools.definitions();
            let response = tokio::select! {
                _ = cancel.cancelled() => return Err(EngineError::CancelledWithOutput(snapshot)),
                result = self.provider.respond_messages_with_tools(context, &messages, &tool_definitions, cancel.clone()) => result?,
            };
            EngineError::check(&cancel, self.guard.as_ref(), context.capability_epoch)?;

            if !response.text.is_empty() {
                let delta = response.text.clone();
                snapshot.push_str(&delta);
                let _ = events
                    .send(Event::RunOutput {
                        run_id: context.run_id.clone(),
                        delta,
                        snapshot: snapshot.clone(),
                    })
                    .await;
            }

            let assistant_content = response
                .tool_calls
                .iter()
                .map(|call| ContentBlock::ToolUse {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    input: call.arguments.clone(),
                })
                .chain(
                    (!response.text.is_empty()).then(|| ContentBlock::text(response.text.clone())),
                )
                .collect::<Vec<_>>();
            if !assistant_content.is_empty() {
                let assistant = Message::assistant(assistant_content);
                if let Some(recorder) = &recorder {
                    recorder.append(assistant.clone()).await?;
                }
                messages.push(assistant);
            }

            if response.tool_calls.is_empty() {
                return Ok(RunResult {
                    output: snapshot,
                    tool_calls,
                });
            }

            for call in response.tool_calls {
                EngineError::check(&cancel, self.guard.as_ref(), context.capability_epoch)?;
                let result = tokio::select! {
                    _ = cancel.cancelled() => return Err(EngineError::CancelledWithOutput(snapshot)),
                    result = self.tools.dispatch(context, &call, cancel.clone()) => result?,
                };
                EngineError::check(&cancel, self.guard.as_ref(), context.capability_epoch)?;
                tool_calls += 1;
                let tool_message = Message {
                    role: Role::User,
                    content: vec![if result.is_error {
                        ContentBlock::tool_error(&call.id, &result.output)
                    } else {
                        ContentBlock::tool_result(&call.id, &result.output)
                    }],
                };
                if let Some(recorder) = &recorder {
                    recorder.append(tool_message.clone()).await?;
                }
                messages.push(tool_message);
            }
        }

        Err(EngineError::MaxTurns)
    }
}
