//! vak-llm: unified multi-provider LLM vocabulary and streaming.
//!
//! Design invariants (see docs/design/01-llm.md):
//! - every streaming event carries a delta AND the accumulated snapshot
//! - errors are values; streams never panic into consumers
//! - abort is first-class and preserves partial output

pub mod anthropic;
pub mod error;
pub mod google;
pub mod openai;
pub mod openai_responses;
pub mod registry;
pub mod sse;
pub mod stream;
pub mod types;

pub use error::LlmError;
pub use registry::{ProviderAuth, ProviderRegistry};
pub use stream::{EventSink, EventStream, StreamEvent};
pub use types::{
    AssistantMessage, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition, Usage,
};

use tokio_util::sync::CancellationToken;

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError>;
}
