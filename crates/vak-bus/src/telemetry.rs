//! Observability, metrics collection, distributed trace propagation, and Dead-Letter Queue (DLQ) tracking.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::envelope::{MessageEnvelope, TraceContext};

/// Snapshot of bus operational metrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusMetricsSnapshot {
    pub published_count: u64,
    pub received_count: u64,
    pub bytes_published: u64,
    pub bytes_received: u64,
    pub dead_letter_count: u64,
    pub active_queue_lag: u64,
}

/// Thread-safe lock-free metrics counters for the messaging fabric.
#[derive(Debug, Default)]
pub struct BusMetrics {
    published_count: AtomicU64,
    received_count: AtomicU64,
    bytes_published: AtomicU64,
    bytes_received: AtomicU64,
    dead_letter_count: AtomicU64,
    active_queue_lag: AtomicU64,
}

impl BusMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record_published(&self, bytes: usize) {
        self.published_count.fetch_add(1, Ordering::Relaxed);
        self.bytes_published
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_received(&self, bytes: usize) {
        self.received_count.fetch_add(1, Ordering::Relaxed);
        self.bytes_received
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_dead_letter(&self) {
        self.dead_letter_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn update_queue_lag(&self, lag: u64) {
        self.active_queue_lag.store(lag, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> BusMetricsSnapshot {
        BusMetricsSnapshot {
            published_count: self.published_count.load(Ordering::Relaxed),
            received_count: self.received_count.load(Ordering::Relaxed),
            bytes_published: self.bytes_published.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            dead_letter_count: self.dead_letter_count.load(Ordering::Relaxed),
            active_queue_lag: self.active_queue_lag.load(Ordering::Relaxed),
        }
    }
}

/// Dead-Letter Queue event representation capturing poison pills and exhausted retries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLetterEvent {
    pub dlq_id: String,
    pub envelope_id: String,
    pub subject: String,
    pub worker_id: String,
    pub attempts: u32,
    pub failure_reason: String,
    pub failed_at: DateTime<Utc>,
    pub envelope: MessageEnvelope,
}

impl DeadLetterEvent {
    pub fn new(
        envelope: MessageEnvelope,
        subject: impl Into<String>,
        worker_id: impl Into<String>,
        attempts: u32,
        failure_reason: impl Into<String>,
    ) -> Self {
        Self {
            dlq_id: uuid::Uuid::now_v7().to_string(),
            envelope_id: envelope.id.clone(),
            subject: subject.into(),
            worker_id: worker_id.into(),
            attempts,
            failure_reason: failure_reason.into(),
            failed_at: Utc::now(),
            envelope,
        }
    }
}

/// Inject or propagate W3C distributed trace context across execution boundaries.
pub fn inject_trace_context(envelope: &mut MessageEnvelope, parent_context: Option<&TraceContext>) {
    if let Some(parent) = parent_context {
        envelope.trace = parent.child_span();
    }
}

/// Extract trace context from an incoming message envelope.
pub fn extract_trace_context(envelope: &MessageEnvelope) -> TraceContext {
    envelope.trace.clone()
}
