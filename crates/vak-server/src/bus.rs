//! Distributed event fabric bridge (docs/design/53-distributed-bus.md).
//!
//! Bridges the server's local `SystemEvent` model to vak-bus's distributed
//! messaging fabric. When NATS is configured (`[server.bus]` section), the
//! `ServerBus` publishes system events to JetStream subjects with Merkle causal
//! chaining, AES-256-GCM envelope encryption, and role-based ACL enforcement.
//! When NATS is absent, the `InMemoryBus` backend provides the same semantic
//! contract for local operation and CI.
//!
//! The bus is **additive**: the existing local `tokio::broadcast` hub keeps
//! working exactly as before. `ServerBus` is only consulted when present, and
//! a missing/unavailable backend fails closed (local broadcast continues).

#![allow(dead_code)]

use std::sync::Arc;

use tokio::sync::Mutex;
use vak_bus::BusMetricsSnapshot;
use vak_bus::bus::{EventPublisher, EventSubscriber, InMemoryBus, NatsBus, NatsConfig};
use vak_bus::envelope::{MessageEnvelope, TraceContext};
use vak_bus::subjects::{AclPolicy, Subject};

use crate::events::SystemEvent;

/// The backend engine backing `ServerBus`.
#[allow(clippy::large_enum_variant)]
enum Backend {
    InMemory(InMemoryBus),
    Distributed(NatsBus),
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::InMemory(_) => write!(f, "Backend::InMemory"),
            Backend::Distributed(_) => write!(f, "Backend::Distributed"),
        }
    }
}

/// The NATS secrets, resolved through the secrets chain at the moment a
/// bus is built (`Core::bus_secret`), never read from TOML.
#[derive(Default)]
pub struct BusCredentials {
    pub jwt: Option<String>,
    pub nkey_seed: Option<String>,
}

impl BusCredentials {
    pub fn of(core: &vak_core::Core) -> Self {
        Self {
            jwt: core.bus_secret(vak_config::BUS_NATS_JWT_VAR),
            nkey_seed: core.bus_secret(vak_config::BUS_NATS_NKEY_SEED_VAR),
        }
    }
}

/// Distributed event bus for the vak server.
///
/// Wraps a vak-bus backend (`InMemoryBus` for local operation, `NatsBus` for
/// distributed fan-out) and translates between the server's `SystemEvent`
/// model and vak-bus's `MessageEnvelope` wire format.
pub struct ServerBus {
    backend: Backend,
    workspace_id: String,
    /// Optional per-workspace encryption key. When set, payloads are
    /// AES-256-GCM encrypted before publishing.
    workspace_secret: Option<Arc<[u8]>>,
    /// Monotonic sequence number for causal chaining on the event plane.
    /// The last published sequence number and the hash of that envelope, so
    /// each envelope's `prev_event_hash` is the real hash of its predecessor.
    chain: Arc<Mutex<(u64, String)>>,
}

impl std::fmt::Debug for ServerBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerBus")
            .field("backend", &self.backend)
            .field("workspace_id", &self.workspace_id)
            .finish_non_exhaustive()
    }
}

impl ServerBus {
    /// Create a local-only bus backed by `InMemoryBus`.
    pub fn local(workspace_id: impl Into<String>) -> Self {
        Self {
            backend: Backend::InMemory(InMemoryBus::new()),
            workspace_id: workspace_id.into(),
            workspace_secret: None,
            chain: Arc::new(Mutex::new((0, MessageEnvelope::GENESIS_HASH.to_string()))),
        }
    }

    /// Create a distributed bus backed by NATS Core + JetStream.
    pub async fn distributed(
        workspace_id: impl Into<String>,
        config: &vak_config::BusResolved,
        credentials: &BusCredentials,
    ) -> Result<Self, vak_bus::BusError> {
        let nats_config = NatsConfig {
            url: config
                .nats_url
                .clone()
                .unwrap_or_else(|| NatsConfig::default().url),
            credentials_jwt: credentials.jwt.clone(),
            nkey_seed: credentials.nkey_seed.clone(),
            ..NatsConfig::default()
        };
        let bus = NatsBus::connect(nats_config).await?;
        Ok(Self {
            backend: Backend::Distributed(bus),
            workspace_id: workspace_id.into(),
            workspace_secret: config.workspace_secret.as_deref().map(Arc::from),
            chain: Arc::new(Mutex::new((0, MessageEnvelope::GENESIS_HASH.to_string()))),
        })
    }

    /// Build a `ServerBus` from resolved config: distributed if NATS is
    /// configured, local otherwise. Never fails for a missing configuration —
    /// returns a local bus so the server always has an event fabric.
    ///
    /// A NATS connection failure degrades to `InMemoryBus` with a warning,
    /// preserving local observability.
    pub async fn from_resolved(
        workspace_id: impl Into<String>,
        config: &vak_config::BusResolved,
        credentials: &BusCredentials,
    ) -> Self {
        let ws = workspace_id.into();
        if config.nats_url.is_some() {
            match Self::distributed(&ws, config, credentials).await {
                Ok(bus) => bus,
                Err(e) => {
                    tracing::warn!(
                        error_kind = %vak_telemetry::error_kind(&e),
                        "the NATS bus did not connect; using the in-process bus"
                    );
                    Self::local(&ws)
                }
            }
        } else {
            Self::local(&ws)
        }
    }

    /// Publish a `SystemEvent` to the distributed fabric as a `MessageEnvelope`.
    ///
    /// Maps each event variant to an appropriate vak-bus `Subject`, attaches
    /// the workspace/session context, computes the Merkle causal link from
    /// the previous event's hash, and optionally encrypts the payload.
    pub async fn emit(
        &self,
        event: &SystemEvent,
        session_id: Option<&str>,
    ) -> Result<(), vak_bus::BusError> {
        self.emit_traced(event, session_id, None).await
    }

    /// As [`ServerBus::emit`], carrying the run's trace key: the envelope's
    /// trace context is that run's W3C trace-id and span, never a fresh root.
    pub async fn emit_traced(
        &self,
        event: &SystemEvent,
        session_id: Option<&str>,
        trace: Option<&vak_session::trace::TraceKey>,
    ) -> Result<(), vak_bus::BusError> {
        let subject = self.subject_for(event, session_id);
        let payload = serde_json::to_vec(event).unwrap_or_else(|_| b"{}".to_vec());

        let mut chain = self.chain.lock().await;
        let seq = chain.0 + 1;
        let mut envelope = MessageEnvelope::new(
            format!("vak://server/{}", self.workspace_id),
            subject_event_type(&subject),
            &self.workspace_id,
            "server",
            seq,
            &chain.1,
            payload,
            trace.map(|t| TraceContext {
                traceparent: t.traceparent(),
                tracestate: None,
            }),
        );
        *chain = (seq, envelope.compute_hash());
        drop(chain);

        if let Some(sid) = session_id {
            envelope = envelope.with_session(sid);
        }

        if let Some(secret) = &self.workspace_secret {
            let _ = vak_bus::encrypt_envelope(&mut envelope, secret, "v1");
        }

        let subject_str = subject.to_subject_string();
        match &self.backend {
            Backend::InMemory(b) => EventPublisher::publish(b, &subject_str, envelope).await,
            Backend::Distributed(b) => EventPublisher::publish(b, &subject_str, envelope).await,
        }
    }

    /// Subscribe to events matching a subject pattern.
    pub async fn subscribe(
        &self,
        pattern: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<MessageEnvelope>, vak_bus::BusError> {
        match &self.backend {
            Backend::InMemory(b) => EventSubscriber::subscribe(b, pattern).await,
            Backend::Distributed(b) => EventSubscriber::subscribe(b, pattern).await,
        }
    }

    /// Subscribe to all system events for this workspace.
    pub async fn subscribe_workspace_events(
        &self,
    ) -> Result<tokio::sync::mpsc::Receiver<MessageEnvelope>, vak_bus::BusError> {
        self.subscribe(&format!("vak.events.{}.*", self.workspace_id))
            .await
    }

    /// Current bus operational metrics.
    pub fn metrics(&self) -> BusMetricsSnapshot {
        match &self.backend {
            Backend::InMemory(b) => b.metrics().snapshot(),
            Backend::Distributed(b) => b.metrics().snapshot(),
        }
    }

    /// Whether the bus is backed by a distributed NATS connection.
    pub fn is_distributed(&self) -> bool {
        matches!(self.backend, Backend::Distributed(_))
    }

    /// The workspace ID this bus is scoped to.
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    /// Operational status snapshot for the admin console / ops center.
    pub fn status(&self) -> serde_json::Value {
        let m = self.metrics();
        serde_json::json!({
            "workspace_id": self.workspace_id,
            "backend": if self.is_distributed() { "nats" } else { "memory" },
            "connected": self.is_distributed(),
            "encrypted": self.workspace_secret.is_some(),
            "metrics": {
                "published_count": m.published_count,
                "received_count": m.received_count,
                "bytes_published": m.bytes_published,
                "bytes_received": m.bytes_received,
                "dead_letter_count": m.dead_letter_count,
                "active_queue_lag": m.active_queue_lag,
            },
        })
    }

    /// ACL policy for the workspace's event plane.
    pub fn acl_for(&self, session_id: &str, agent_id: &str, role: &str) -> AclPolicy {
        AclPolicy::for_worker(&self.workspace_id, session_id, agent_id, role)
    }

    fn subject_for(&self, event: &SystemEvent, session_id: Option<&str>) -> Subject {
        let sess = session_id.unwrap_or("");
        match event {
            SystemEvent::SessionCreated { .. } => Subject::Custom(format!(
                "vak.events.{}.{}.session.created",
                self.workspace_id, sess
            )),
            SystemEvent::SessionEntryAppended {
                session_id: sid, ..
            } => Subject::Custom(format!("vak.events.{}.{}.entry", self.workspace_id, sid)),
            SystemEvent::ConfigChanged { .. } => Subject::Custom(format!(
                "vak.events.{}.{}.config.changed",
                self.workspace_id, sess
            )),
            SystemEvent::GatewayInbound { .. } => Subject::Custom(format!(
                "vak.events.{}.{}.gateway.inbound",
                self.workspace_id, sess
            )),
            SystemEvent::ApprovalRequested { session_id, .. } => Subject::Approvals {
                workspace_id: self.workspace_id.clone(),
                session_id: session_id.clone(),
            },
            SystemEvent::ApprovalGranted { .. } | SystemEvent::ApprovalDenied { .. } => {
                Subject::Custom(format!(
                    "vak.events.{}.{}.approval.{}",
                    self.workspace_id,
                    sess,
                    if matches!(event, SystemEvent::ApprovalGranted { .. }) {
                        "granted"
                    } else {
                        "denied"
                    }
                ))
            }
            SystemEvent::WorkerQuestion { .. } => Subject::Custom(format!(
                "vak.events.{}.{}.worker.question",
                self.workspace_id, sess
            )),
            SystemEvent::WorkerQuestionClosed { .. } => Subject::Custom(format!(
                "vak.events.{}.{}.worker.question.closed",
                self.workspace_id, sess
            )),
            SystemEvent::SecurityEvent { .. } => Subject::Custom(format!(
                "vak.events.{}.{}.security",
                self.workspace_id, sess
            )),
            SystemEvent::ProviderError { .. } => Subject::Custom(format!(
                "vak.events.{}.{}.provider.error",
                self.workspace_id, sess
            )),
            SystemEvent::RateLimit { .. } => Subject::Custom(format!(
                "vak.events.{}.{}.rate_limit",
                self.workspace_id, sess
            )),
            SystemEvent::Heartbeat => Subject::Custom(format!(
                "vak.events.{}.{}.heartbeat",
                self.workspace_id, sess
            )),
            SystemEvent::Agent(_) => {
                Subject::Custom(format!("vak.events.{}.{}.agent", self.workspace_id, sess))
            }
        }
    }
}

/// Human-readable event type string for the bus subject.
fn subject_event_type(subject: &Subject) -> &str {
    match subject {
        Subject::EventsTokens { .. } => "vak.events.tokens",
        Subject::EventsTerminal { .. } => "vak.events.terminal",
        Subject::EventsTelemetry { .. } => "vak.events.telemetry",
        Subject::WorkTask { .. } => "vak.work.task",
        Subject::AgentInbox { .. } => "vak.agent.inbox",
        Subject::Receipts { .. } => "vak.receipts.completed",
        Subject::Approvals { .. } => "vak.approvals.request",
        Subject::DeadLetter { .. } => "vak.dlq.failed",
        Subject::Custom(s) => s,
    }
}

/// Decrypt a received envelope if it carries encryption metadata.
pub fn try_decrypt(
    envelope: &mut MessageEnvelope,
    workspace_secret: &[u8],
) -> Result<(), vak_bus::BusError> {
    if !envelope.encrypted {
        return Ok(());
    }
    vak_bus::decrypt_envelope(envelope, workspace_secret)
        .map_err(|e| vak_bus::BusError::Serialization(e.to_string()))
}

// ---- tests ----

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn local_bus_publishes_and_receives_system_event() {
        let bus = ServerBus::local("ws_test");

        // Subscribe before emitting: broadcast channels don't deliver to
        // late joiners.
        let mut rx = bus
            .subscribe("vak.events.ws_test.>")
            .await
            .expect("subscribe");

        // Yield to let the forwarding task start before we publish.
        tokio::task::yield_now().await;

        let event = SystemEvent::Heartbeat;
        bus.emit(&event, Some("sess1")).await.expect("publish");

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("timeout")
            .expect("received envelope");

        let payload: serde_json::Value =
            serde_json::from_slice(&received.payload).expect("decode payload");
        assert_eq!(payload["type"], "Heartbeat");
    }

    #[tokio::test]
    async fn bus_envelope_trace_is_run_trace() {
        use vak_session::ids::{AgentId, SpaceId, TenantId};
        use vak_session::trace::{Cause, TraceKey};
        let bus = ServerBus::local("ws_trace");
        let mut rx = bus.subscribe("vak.events.ws_trace.>").await.expect("sub");
        tokio::task::yield_now().await;
        let key = TraceKey::root(
            TenantId::new(),
            SpaceId::new(),
            AgentId::new(),
            Cause::Heartbeat,
        );
        bus.emit_traced(&SystemEvent::Heartbeat, None, Some(&key))
            .await
            .expect("emit");
        let env = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("timeout")
            .expect("recv");
        assert_eq!(env.trace.trace_id(), key.w3c_trace_id());
        assert_eq!(env.trace.traceparent, key.traceparent());
    }

    #[tokio::test]
    async fn bus_prev_hash_is_hash() {
        let bus = ServerBus::local("ws_hash");
        let mut rx = bus.subscribe("vak.events.ws_hash.>").await.expect("sub");
        tokio::task::yield_now().await;
        for _ in 0..3 {
            bus.emit(&SystemEvent::Heartbeat, None).await.expect("emit");
        }
        let mut prev: Option<MessageEnvelope> = None;
        for _ in 0..3 {
            let env = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
                .await
                .expect("timeout")
                .expect("recv");
            match &prev {
                None => assert_eq!(env.lineage.prev_event_hash, MessageEnvelope::GENESIS_HASH),
                Some(p) => {
                    assert_eq!(env.lineage.prev_event_hash.len(), 64);
                    assert!(MessageEnvelope::verify_merkle_link(p, &env));
                }
            }
            prev = Some(env);
        }
    }

    #[tokio::test]
    async fn local_bus_chains_causal_sequence() {
        let bus = ServerBus::local("ws_seq");

        // Subscribe before emitting (broadcast channels don't backfill
        // to late joiners).
        let mut rx = bus.subscribe("vak.events.ws_seq.>").await.expect("sub");

        // Yield to let the forwarding task start before we publish.
        tokio::task::yield_now().await;

        bus.emit(&SystemEvent::Heartbeat, Some("sess1"))
            .await
            .expect("emit 1");
        bus.emit(&SystemEvent::Heartbeat, Some("sess1"))
            .await
            .expect("emit 2");

        let env1 = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("timeout 1")
            .expect("recv 1");
        let env2 = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("timeout 2")
            .expect("recv 2");

        assert!(env2.lineage.seq_num == env1.lineage.seq_num + 1);
    }

    #[tokio::test]
    async fn local_bus_metrics_track_publish() {
        let bus = ServerBus::local("ws_metrics");
        bus.emit(&SystemEvent::Heartbeat, None).await.expect("emit");
        let snap = bus.metrics();
        assert!(snap.published_count >= 1);
    }

    #[tokio::test]
    async fn distributed_bus_falls_back_when_nats_unreachable() {
        let cfg = vak_config::BusResolved {
            nats_url: Some("nats://127.0.0.1:1".to_string()),
            ..Default::default()
        };
        let bus = ServerBus::from_resolved("ws_fb", &cfg, &BusCredentials::default()).await;
        bus.emit(&SystemEvent::Heartbeat, None)
            .await
            .expect("emit on fallback");
    }

    #[test]
    fn approval_subject_uses_typed_variant() {
        let bus = ServerBus::local("ws_approvals");
        let event = SystemEvent::ApprovalRequested {
            id: "appr1".to_string(),
            session_id: "sess_x".to_string(),
            tool: "bash".to_string(),
            reason: "test".to_string(),
        };
        let subject = bus.subject_for(&event, Some("sess_x"));
        match subject {
            Subject::Approvals {
                workspace_id,
                session_id,
            } => {
                assert_eq!(workspace_id, "ws_approvals");
                assert_eq!(session_id, "sess_x");
            }
            other => panic!("expected Approvals subject, got {:?}", other),
        }
    }

    #[test]
    fn acl_policy_is_workspace_scoped() {
        let bus = ServerBus::local("ws_acl");
        let acl = bus.acl_for("sess1", "agent_a", "coder");
        assert!(acl.can_publish("vak.events.ws_acl.sess1.tokens"));
        assert!(!acl.can_publish("vak.events.ws_other.sess1.tokens"));
    }

    #[test]
    fn resolved_bus_config_defaults_to_local() {
        let cfg = vak_config::BusResolved::default();
        assert!(cfg.nats_url.is_none());
    }
}
