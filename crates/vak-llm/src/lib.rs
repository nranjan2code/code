//! vak-llm: unified multi-provider LLM vocabulary and streaming.
//!
//! Design invariants (see docs/design/01-llm.md):
//! - every streaming event carries a delta AND the accumulated snapshot
//! - errors are values; streams never panic into consumers
//! - abort is first-class and preserves partial output

pub mod anthropic;
pub mod error;
pub mod gate;
pub mod google;
pub mod google_live;
pub mod model_identity;
pub mod models;
pub mod ollama;
pub mod openai;
pub mod openai_realtime;
pub mod openai_responses;
pub mod provider_status;
pub mod rate_limit;
pub mod registry;
pub mod route;
pub mod sse;
pub mod stream;
pub mod turn;
pub mod types;
pub mod work;

pub use error::LlmError;
pub use gate::credential_id;
pub use model_identity::ModelRef;
pub use rate_limit::RateLimitGate;
pub use registry::{ProviderAuth, ProviderRegistry};
pub use route::{
    BELIEF_FLOOR, BeliefMap, Demand, DemandBand, DemandInput, EndpointDialect, EvidenceSnapshot,
    ModelEvidence, QualityObjective, RouteLeg, order_ladder_v1, order_ladder_v2, score_demand,
};
pub use stream::{EventSink, EventStream, StreamEvent};
pub use turn::current_turn_boundary;
pub use types::{
    AssistantMessage, CacheBreakpoint, CacheHints, ChatRequest, ContentBlock, Effort, Message,
    Role, StopReason, ToolDefinition, Usage,
};
pub use work::{
    AttemptReason, DispatchAttempt, DispatchBudget, DispatchCeiling, FailureDomain, Settlement,
    StepLedger, WorkPurpose, WorkReceipt,
};

use tokio_util::sync::CancellationToken;

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;

    /// Stable in-process health identity. Implementations with independent
    /// credentials must include that credential's fingerprint so one bad key
    /// cannot open the circuit for another key.
    fn circuit_key(&self) -> String {
        self.name().to_string()
    }

    /// Identity of the provider capacity bucket shared by this route.
    /// Adapters using the same account over different wire protocols return
    /// the same opaque credential identity; the identity never contains a key.
    fn rate_limit_key(&self) -> String {
        self.circuit_key()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError>;
}
