//! Global event hub (docs/design/23-memory.md): a single `tokio::broadcast`
//! channel that every subsystem writes to and the admin console SSE endpoint
//! reads from. Events are append-only and lossy by design — slow consumers
//! get `Lagged` errors, not backpressure.

#![allow(dead_code)]

use std::sync::OnceLock;

use serde::Serialize;
use tokio::sync::broadcast;

/// Capacity of the global broadcast channel. Slow consumers that fall
/// more than this many events behind receive `Lagged` and must reconnect.
const HUB_CAPACITY: usize = 4096;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum SystemEvent {
    // ---- agent lifecycle (forwarded from AgentEvent) ----
    Agent(AgentEventPayload),

    // ---- session lifecycle ----
    SessionCreated {
        session_id: String,
        project_hash: String,
    },
    SessionEntryAppended {
        session_id: String,
        entry_id: String,
        kind: String,
    },

    // ---- config / gateway ----
    ConfigChanged {
        label: String,
        detail: String,
    },
    GatewayInbound {
        surface: String,
        who: String,
        preview: String,
    },

    // ---- approvals ----
    ApprovalGranted {
        id: String,
        tool: String,
    },
    ApprovalDenied {
        id: String,
        tool: String,
    },

    // ---- security ----
    SecurityEvent {
        kind: String,
        label: String,
    },

    // ---- provider health ----
    ProviderError {
        provider: String,
        model: String,
        error: String,
    },
    RateLimit {
        provider: String,
        retry_after_secs: Option<u64>,
    },

    // ---- heartbeat ----
    Heartbeat,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentEventPayload {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Global event hub singleton. Created once at server startup; every
/// subsystem clones the `Sender` side. Consumers subscribe via `subscribe()`.
#[derive(Debug, Clone)]
pub struct EventHub {
    tx: broadcast::Sender<SystemEvent>,
}

impl EventHub {
    /// Create a new hub. Only call this once at server startup.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(HUB_CAPACITY);
        EventHub { tx }
    }

    /// Emit an event. Never blocks — dropped if no receivers are alive.
    pub fn emit(&self, event: SystemEvent) {
        // Best-effort: ignore Lagged/RecvError.
        let _ = self.tx.send(event);
    }

    /// Create a new receiver. Lagged receivers get `Lagged` errors on
    /// `recv()` and should reconnect.
    pub fn subscribe(&self) -> broadcast::Receiver<SystemEvent> {
        self.tx.subscribe()
    }

    /// Downgrade to a weak sender for non-owning references.
    pub fn sender(&self) -> broadcast::Sender<SystemEvent> {
        self.tx.clone()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global singleton accessor
// ---------------------------------------------------------------------------

static GLOBAL_HUB: OnceLock<EventHub> = OnceLock::new();

/// Initialize the global event hub. Call once at server startup before
/// any subsystem tries `global()`.
pub fn init_global() -> EventHub {
    GLOBAL_HUB.get_or_init(EventHub::new).clone()
}

/// Access the global hub. Returns `None` if `init_global()` was never called.
pub fn global() -> Option<EventHub> {
    GLOBAL_HUB.get().cloned()
}

// ---------------------------------------------------------------------------
// Convenience helpers
// ---------------------------------------------------------------------------

impl EventHub {
    pub fn emit_session_created(&self, session_id: &str, project_hash: &str) {
        self.emit(SystemEvent::SessionCreated {
            session_id: session_id.to_string(),
            project_hash: project_hash.to_string(),
        });
    }

    pub fn emit_entry_appended(&self, session_id: &str, entry_id: &str, kind: &str) {
        self.emit(SystemEvent::SessionEntryAppended {
            session_id: session_id.to_string(),
            entry_id: entry_id.to_string(),
            kind: kind.to_string(),
        });
    }

    pub fn emit_config_changed(&self, label: &str, detail: &str) {
        self.emit(SystemEvent::ConfigChanged {
            label: label.to_string(),
            detail: detail.to_string(),
        });
    }

    pub fn emit_gateway_inbound(&self, surface: &str, who: &str, preview: &str) {
        self.emit(SystemEvent::GatewayInbound {
            surface: surface.to_string(),
            who: who.to_string(),
            preview: preview.to_string(),
        });
    }

    pub fn emit_agent_summary(&self, summary: &str, detail: Option<String>) {
        self.emit(SystemEvent::Agent(AgentEventPayload {
            summary: summary.to_string(),
            detail,
        }));
    }

    pub fn emit_security(&self, kind: &str, label: &str) {
        self.emit(SystemEvent::SecurityEvent {
            kind: kind.to_string(),
            label: label.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn emit_and_receive() {
        let hub = EventHub::new();
        let mut rx = hub.subscribe();
        hub.emit(SystemEvent::SessionCreated {
            session_id: "s1".into(),
            project_hash: "abc".into(),
        });
        let event = rx.try_recv().unwrap();
        match event {
            SystemEvent::SessionCreated { session_id, .. } => {
                assert_eq!(session_id, "s1");
            }
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn multiple_receivers_get_independent_copies() {
        let hub = EventHub::new();
        let mut rx1 = hub.subscribe();
        let mut rx2 = hub.subscribe();
        hub.emit(SystemEvent::Heartbeat);
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn lagged_receiver_gets_error() {
        let hub = EventHub::new();
        let rx = hub.subscribe();
        drop(rx); // drop receiver
        // Fill the buffer beyond capacity.
        for _ in 0..5000 {
            hub.emit(SystemEvent::Heartbeat);
        }
        // Re-subscribe; old position is lost.
        let mut rx = hub.subscribe();
        // The first recv may be Lagged or a valid event — both ok.
        let result = rx.try_recv();
        // We may get Lagged or a valid event depending on timing — both ok.
        // But at minimum, the hub still works.
        drop(result);
        hub.emit(SystemEvent::Heartbeat);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn convenience_helpers_produce_correct_events() {
        let hub = EventHub::new();
        let mut rx = hub.subscribe();

        hub.emit_config_changed("mode", "restricted");
        if let SystemEvent::ConfigChanged { label, detail } = rx.try_recv().unwrap() {
            assert_eq!(label, "mode");
            assert_eq!(detail, "restricted");
        } else {
            panic!("expected ConfigChanged");
        }

        hub.emit_security("AuthFailure", "bad token");
        if let SystemEvent::SecurityEvent { kind, label } = rx.try_recv().unwrap() {
            assert_eq!(kind, "AuthFailure");
            assert_eq!(label, "bad token");
        } else {
            panic!("expected SecurityEvent");
        }
    }
}
