//! `vak-bus`: High-throughput distributed event & message fabric for massive agent swarms.
//!
//! Provides CloudEvents envelopes with W3C distributed tracing and Merkle causal hash chaining,
//! zero-trust AES-256-GCM envelope payload encryption, deterministic subject algebra with
//! role-based ACLs, and production NATS Core + JetStream as well as standalone concurrent engines.

pub mod bus;
pub mod crypto;
pub mod envelope;
pub mod subjects;
pub mod telemetry;

pub use bus::{
    BusError, ClaimedTask, EventPublisher, EventSubscriber, InMemoryBus, NatsBus, NatsConfig,
    TaskAckHandle, WorkQueue,
};
pub use crypto::{CryptoError, decrypt_envelope, derive_workspace_key, encrypt_envelope};
pub use envelope::{CausalLineage, EncryptionMeta, MessageEnvelope, TraceContext};
pub use subjects::{AclPolicy, Subject, SubjectError, matches_pattern};
pub use telemetry::{
    BusMetrics, BusMetricsSnapshot, DeadLetterEvent, extract_trace_context, inject_trace_context,
};
