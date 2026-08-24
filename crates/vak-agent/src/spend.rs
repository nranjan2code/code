//! Spend-admission seam (docs/design/27 Phase D). The agent loop asks the
//! gate before every paid dispatch and reports settled usage after; the
//! implementation lives above (vak-core) where the ledger, caps, and
//! pricing config exist. Budget denial is a typed Ask through the normal
//! approver channel — unattended surfaces auto-deny, silence means no.

use vak_llm::Usage;

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

    /// A settled dispatch: record it against run/day windows. `provider`
    /// attributes the spend to the serving leg for per-provider FinOps
    /// rollups.
    fn record_settled(&self, provider: &str, model: &str, session_id: &str, usage: &Usage);

    /// The approver accepted the budget Ask: lift the cap for the REST of
    /// this run (raise-cap-once semantics, doc 27 Phase D). Default no-op
    /// for gates without run state.
    fn on_budget_approved(&self) {}
}
