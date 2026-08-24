//! Work receipts: typed audit records for provider dispatches (doc 27
//! Phase A). Receipts are ledger data, never model-visible input.

use crate::{LlmError, Usage};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Why this work exists. Extends as call sites adopt receipts; every
/// variant must map to a caller-visible purpose, never a hidden retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPurpose {
    /// A main agent-loop model step.
    Execute,
    /// The compaction summarizer call.
    Summarize,
    /// Completion-audit judge call (Phase H).
    Verify,
}

/// Why a dispatch was made. `Retry` covers same-candidate transient
/// retries; route-level reasons arrive with the frozen ladder (Phase B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptReason {
    Initial,
    Retry,
    /// First dispatch of the NEXT frozen-ladder candidate after typed
    // failure of the previous one (Phase B).
    RouteFallback,
    EnduranceRetry,
}

/// Which slice of the world failed. Consumed by breaker/endurance
/// classification instead of error-string matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureDomain {
    /// Credentials, quota, rate limits: account-scoped facts.
    Account,
    /// The provider service itself (overload, outage).
    Provider,
    /// The model produced unusable output (malformed stream).
    Model,
    /// Our request was rejected before generation (bad request, auth).
    Request,
    /// Transport-level failure.
    Network,
    /// Watchdog deadline.
    Deadline,
    /// Unclassified.
    Unknown,
}

/// What actually happened to a dispatch. Billing without a rated outcome
/// is `Unknown`, never `Ok` — absent is UNKNOWN, never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Settlement {
    Ok,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchAttempt {
    pub ordinal: u32,
    pub reason: AttemptReason,
    pub domain: FailureDomain,
    pub settlement: Settlement,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkReceipt {
    pub purpose: WorkPurpose,
    pub model: String,
    /// Ordinal of the attempt that produced committed output; None when no
    /// attempt succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winning_attempt: Option<u32>,
    pub attempts: Vec<DispatchAttempt>,
}

impl WorkReceipt {
    pub fn new(purpose: WorkPurpose, model: impl Into<String>) -> Self {
        WorkReceipt {
            purpose,
            model: model.into(),
            winning_attempt: None,
            attempts: Vec::new(),
        }
    }

    pub fn record(
        &mut self,
        reason: AttemptReason,
        domain: FailureDomain,
        settlement: Settlement,
        latency_ms: u64,
        usage: Option<Usage>,
        error: Option<String>,
    ) {
        let ordinal = self.attempts.len() as u32;
        if settlement == Settlement::Ok {
            self.winning_attempt = Some(ordinal);
        }
        self.attempts.push(DispatchAttempt {
            ordinal,
            reason,
            domain,
            settlement,
            latency_ms,
            usage,
            error,
        });
    }

    pub fn settle_cancelled(&mut self) {
        if let Some(last) = self.attempts.last_mut() {
            last.settlement = Settlement::Cancelled;
        }
    }
}

/// Classify an error into (domain, settlement). Pre-dispatch rejections
/// (auth, bad request, rate limit, overload) deterministically failed;
/// transport/mid-stream failures may have consumed provider compute, so
/// their settlement is Unknown — fail-closed against paid fallback later.
pub fn classify_error(e: &LlmError) -> (FailureDomain, Settlement) {
    match e {
        LlmError::Auth(_) => (FailureDomain::Account, Settlement::Failed),
        LlmError::RateLimit { .. } => (FailureDomain::Account, Settlement::Failed),
        LlmError::Overloaded(_) => (FailureDomain::Provider, Settlement::Failed),
        LlmError::InvalidRequest(_) => (FailureDomain::Request, Settlement::Failed),
        LlmError::Api { .. } => (FailureDomain::Request, Settlement::Unknown),
        LlmError::Network(_) => (FailureDomain::Network, Settlement::Unknown),
        LlmError::Parse(_) => (FailureDomain::Model, Settlement::Unknown),
        LlmError::Aborted { .. } => (FailureDomain::Unknown, Settlement::Cancelled),
    }
}

/// Hard cap on provider dispatches for one unit of work. Exhaustion fails
/// closed before another paid call goes out. Single-ladder default in the
/// agent codifies today's worst case:
/// `(max_retries + 1) * (run_retry_attempts + 1)`; Phase B tightens the
/// formula to `ladder.len() + repair_allowance` once ladders exist.
#[derive(Debug, Clone)]
pub struct DispatchBudget {
    limit: u32,
    used: u32,
}

impl DispatchBudget {
    pub fn new(limit: u32) -> Self {
        DispatchBudget { limit, used: 0 }
    }

    pub fn remaining(&self) -> u32 {
        self.limit.saturating_sub(self.used)
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// Consume one dispatch. Err when the ceiling is exhausted: callers
    /// must not dispatch past it.
    pub fn consume(&mut self) -> Result<(), DispatchCeiling> {
        if self.used >= self.limit {
            return Err(DispatchCeiling { limit: self.limit });
        }
        self.used += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchCeiling {
    pub limit: u32,
}

impl std::fmt::Display for DispatchCeiling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dispatch ceiling of {} exhausted", self.limit)
    }
}

/// Per-work accounting threaded through the reliability helper: the shared
/// ceiling plus the receipt under construction.
pub struct StepLedger {
    pub budget: DispatchBudget,
    pub receipt: WorkReceipt,
}

impl StepLedger {
    pub fn new(purpose: WorkPurpose, model: &str, ceiling: u32) -> Self {
        StepLedger {
            budget: DispatchBudget::new(ceiling),
            receipt: WorkReceipt::new(purpose, model),
        }
    }

    /// Hand the finished receipt to the caller, leaving an empty shell in
    /// place (used when a ledger outlives its first work unit's write).
    pub fn take_receipt(&mut self) -> WorkReceipt {
        let purpose = self.receipt.purpose;
        let model = self.receipt.model.clone();
        std::mem::replace(&mut self.receipt, WorkReceipt::new(purpose, model))
    }

    /// Time one dispatch and record its outcome from start to end.
    pub async fn timed(
        &mut self,
        reason: AttemptReason,
        f: impl std::future::Future<Output = Result<crate::AssistantMessage, LlmError>>,
    ) -> Result<crate::AssistantMessage, LlmError> {
        if let Err(c) = self.budget.consume() {
            return Err(LlmError::Network(c.to_string()));
        }
        let started = Instant::now();
        let outcome = f.await;
        match &outcome {
            Ok(value) => {
                self.receipt.record(
                    reason,
                    FailureDomain::Unknown,
                    Settlement::Ok,
                    started.elapsed().as_millis() as u64,
                    Some(value.usage.clone()),
                    None,
                );
            }
            Err(e) => {
                // Deadline classification happens at the watchdog site via
                // `record_deadline`; everything else classifies here.
                if !matches!(e, LlmError::Aborted { .. }) {
                    let (domain, settlement) = classify_error(e);
                    self.receipt.record(
                        reason,
                        domain,
                        settlement,
                        started.elapsed().as_millis() as u64,
                        None,
                        Some(e.to_string()),
                    );
                }
            }
        }
        outcome
    }

    /// Record a watchdog-deadline failure explicitly (the timeout wrapper
    /// erases the inner future's result, so classification must be manual).
    pub fn record_deadline(&mut self, reason: AttemptReason, secs: u64, err: &LlmError) {
        self.receipt.record(
            reason,
            FailureDomain::Deadline,
            Settlement::Unknown,
            secs * 1000,
            None,
            Some(err.to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn classify_maps_domains_and_settlements() {
        assert_eq!(
            classify_error(&LlmError::RateLimit {
                message: "slow down".into(),
                retry_after_secs: Some(3),
            }),
            (FailureDomain::Account, Settlement::Failed)
        );
        assert_eq!(
            classify_error(&LlmError::Parse("truncated".into())),
            (FailureDomain::Model, Settlement::Unknown)
        );
        assert_eq!(
            classify_error(&LlmError::Network("conn reset".into())),
            (FailureDomain::Network, Settlement::Unknown)
        );
        assert_eq!(
            classify_error(&LlmError::Auth("bad key".into())),
            (FailureDomain::Account, Settlement::Failed)
        );
    }

    #[test]
    fn budget_fails_closed_at_limit() {
        let mut b = DispatchBudget::new(2);
        assert_eq!(b.remaining(), 2);
        assert!(b.consume().is_ok());
        assert!(b.consume().is_ok());
        assert_eq!(b.remaining(), 0);
        let err = b.consume().unwrap_err();
        assert_eq!(err.limit, 2);
        assert_eq!(err.to_string(), "dispatch ceiling of 2 exhausted");
    }

    #[test]
    fn receipt_tracks_winning_ordinal_and_cancellation() {
        let mut r = WorkReceipt::new(WorkPurpose::Execute, "m");
        r.record(
            AttemptReason::Initial,
            FailureDomain::Account,
            Settlement::Failed,
            10,
            None,
            Some("429".into()),
        );
        r.record(
            AttemptReason::EnduranceRetry,
            FailureDomain::Deadline,
            Settlement::Unknown,
            20,
            None,
            Some("deadline".into()),
        );
        assert_eq!(r.attempts.len(), 2);
        assert!(r.winning_attempt.is_none());
        r.record(
            AttemptReason::EnduranceRetry,
            FailureDomain::Unknown,
            Settlement::Ok,
            30,
            Some(Usage::default()),
            None,
        );
        assert_eq!(r.winning_attempt, Some(2));
        r.settle_cancelled();
        assert_eq!(r.attempts[2].settlement, Settlement::Cancelled);
    }

    #[test]
    fn receipt_json_round_trip_preserves_everything() {
        let mut r = WorkReceipt::new(WorkPurpose::Summarize, "claude-x");
        r.record(
            AttemptReason::Initial,
            FailureDomain::Network,
            Settlement::Unknown,
            1234,
            None,
            Some("reset by peer".into()),
        );
        r.record(
            AttemptReason::Retry,
            FailureDomain::Unknown,
            Settlement::Ok,
            4321,
            Some(Usage {
                input_tokens: 11,
                output_tokens: 7,
                ..Default::default()
            }),
            None,
        );
        let json = serde_json::to_string(&r).unwrap();
        let back: WorkReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(back.purpose, WorkPurpose::Summarize);
        assert_eq!(back.model, "claude-x");
        assert_eq!(back.winning_attempt, Some(1));
        assert_eq!(back.attempts.len(), 2);
        assert_eq!(back.attempts[0].domain, FailureDomain::Network);
        assert_eq!(back.attempts[0].settlement, Settlement::Unknown);
        assert_eq!(
            back.attempts[1].usage.as_ref().map(|u| u.output_tokens),
            Some(7)
        );
    }
}
