//! Spend-admission seam (docs/design/15-reliability.md). The agent loop asks the
//! gate before every paid dispatch and reports settled usage after; the
//! implementation lives above (vak-core) where the ledger, caps, and
//! pricing config exist. Budget denial is a typed Ask through the normal
//! approver channel — unattended surfaces auto-deny, silence means no.

use vak_llm::Usage;

/// Opaque identity for one budget admission. The generated value carries no
/// provider, model, session, or credential data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpendReservationId(uuid::Uuid);

impl SpendReservationId {
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}

pub struct SpendCheck<'a> {
    pub model: &'a str,
    /// Serving provider of the current frozen-ladder leg. Empty when the
    /// caller cannot know it (legacy fixtures).
    pub provider: &'a str,
    pub session_id: &'a str,
    /// Planning estimate of the next call's input size.
    pub est_input_tokens: u64,
    /// Reserved completion size (context policy max output).
    pub planned_output_tokens: u64,
}

#[async_trait::async_trait]
pub trait SpendGate: Send + Sync {
    /// Err(reason) denies the dispatch. Denial routes through the approver
    /// once as a budget Ask before the run fails closed.
    async fn authorize(&self, check: &SpendCheck<'_>) -> Result<(), String>;

    /// Reserve one dispatch and return its identity when supported. The
    /// default preserves custom gates that implement usage-only accounting.
    async fn reserve_dispatch(
        &self,
        check: &SpendCheck<'_>,
    ) -> Result<Option<SpendReservationId>, String> {
        self.authorize(check).await?;
        Ok(None)
    }

    /// Settle an exact reservation; legacy gates fall back to usage-only
    /// accounting.
    fn settle_dispatch(
        &self,
        _id: SpendReservationId,
        provider: &str,
        model: &str,
        session_id: &str,
        usage: &Usage,
        latency_ms: u64,
    ) {
        self.record_settled_with_latency(provider, model, session_id, usage, latency_ms);
    }

    /// Release a reservation when the provider definitively did not consume
    /// capacity. Uncertain dispatches remain reserved conservatively.
    fn release_dispatch(&self, _id: SpendReservationId) {}

    /// A settled dispatch: record it against run/day windows. `provider`
    /// attributes the spend to the serving leg for per-provider FinOps
    /// rollups.
    fn record_settled(&self, provider: &str, model: &str, session_id: &str, usage: &Usage);

    fn record_settled_with_latency(
        &self,
        provider: &str,
        model: &str,
        session_id: &str,
        usage: &Usage,
        _latency_ms: u64,
    ) {
        self.record_settled(provider, model, session_id, usage);
    }

    /// The approver accepted the budget Ask: lift the cap for the REST of
    /// this run (raise-cap-once semantics, docs/design/15-reliability.md). Default no-op
    /// for gates without run state.
    fn on_budget_approved(&self) {}
}
