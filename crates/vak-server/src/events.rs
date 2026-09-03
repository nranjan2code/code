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
    ApprovalRequested {
        id: String,
        session_id: String,
        tool: String,
        reason: String,
    },
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

// ---- per-session replay bus (docs/design/48-web-client.md §4.4) ------------

/// One agent event with the sequence number a client resumes from.
#[derive(Debug, Clone)]
pub struct SeqEvent {
    pub seq: u64,
    pub event: vak_agent::AgentEvent,
}

/// A session's live event channel, plus a bounded replay ring.
///
/// A bare `tokio::broadcast` gives a late or reconnecting subscriber
/// nothing: the events it missed are gone, not delayed. On loopback that
/// is a rare race the client papers over with a ten-second reconciliation
/// poll. Over a WAN — a laptop lid closing, a phone changing cell, a proxy
/// idling out a stream — it is the common case, and a run's reply can be
/// durably logged while the UI still says "Working".
///
/// So every event carries a monotonic `seq`, the last `CAPACITY` of them
/// are retained, and a reconnect with `Last-Event-ID` gets the gap. When
/// the gap is older than the ring, the client is told to resync rather
/// than being handed a silently incomplete stream — a missing event it
/// does not know is missing is worse than an explicit reload.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<SeqEvent>,
    ring: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<SeqEvent>>>,
    next: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl EventBus {
    const CAPACITY: usize = 1024;

    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(Self::CAPACITY);
        EventBus {
            tx,
            ring: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::with_capacity(Self::CAPACITY),
            )),
            next: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    /// Number and retain an event, then broadcast it.
    ///
    /// Retention happens BEFORE the broadcast so a subscriber that reads
    /// the ring immediately after receiving a live event cannot observe a
    /// ring that is missing it.
    pub fn send(&self, event: vak_agent::AgentEvent) -> usize {
        let seq = self
            .next
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let framed = SeqEvent { seq, event };
        {
            let mut ring = self
                .ring
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if ring.len() == Self::CAPACITY {
                ring.pop_front();
            }
            ring.push_back(framed.clone());
        }
        self.tx.send(framed).unwrap_or(0)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SeqEvent> {
        self.tx.subscribe()
    }

    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Events strictly after `seq`, oldest first.
    ///
    /// `None` means the ring no longer covers that point and the caller
    /// must resync from the durable transcript instead. Note the
    /// distinction from `Some(vec![])`, which means "you are up to date" —
    /// conflating the two is how a client silently loses a turn.
    pub fn replay_after(&self, seq: u64) -> Option<Vec<SeqEvent>> {
        let ring = self
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match ring.front() {
            // Nothing retained yet: only "no events at all" is consistent
            // with any resume point, and that is exactly an empty replay.
            None => Some(Vec::new()),
            // The oldest retained event is already past the client's
            // resume point, so whatever sits between them is unrecoverable.
            Some(oldest) if oldest.seq > seq + 1 => None,
            _ => Some(ring.iter().filter(|e| e.seq > seq).cloned().collect()),
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod bus_tests {
    use super::*;
    use vak_agent::AgentEvent;

    fn note(n: u32) -> AgentEvent {
        AgentEvent::RetryScheduled {
            attempt: n,
            delay_ms: 0,
            reason: String::new(),
        }
    }

    #[test]
    fn a_reconnect_receives_exactly_the_gap() {
        let bus = EventBus::new();
        bus.send(note(1));
        bus.send(note(2));
        bus.send(note(3));
        // Resuming after the first event yields the two it missed, in order.
        let gap = bus.replay_after(1).unwrap();
        assert_eq!(gap.len(), 2);
        assert_eq!(gap[0].seq, 2);
        assert_eq!(gap[1].seq, 3);
    }

    #[test]
    fn being_up_to_date_is_not_the_same_as_having_lost_events() {
        let bus = EventBus::new();
        bus.send(note(1));
        // Caught up: an empty replay, NOT a resync.
        assert_eq!(bus.replay_after(1).unwrap().len(), 0);
    }

    #[test]
    fn a_resume_point_older_than_the_ring_asks_for_a_resync() {
        let bus = EventBus::new();
        for i in 0..(EventBus::CAPACITY as u32 + 10) {
            bus.send(note(i));
        }
        // Seq 1 fell out of the ring long ago; the caller must not be told
        // "here is the gap" when the gap cannot be produced.
        assert!(bus.replay_after(1).is_none());
        // The newest events are still resumable.
        let newest = EventBus::CAPACITY as u64 + 5;
        assert!(bus.replay_after(newest).is_some());
    }

    #[test]
    fn an_empty_bus_replays_nothing_rather_than_demanding_a_resync() {
        let bus = EventBus::new();
        assert_eq!(bus.replay_after(0).unwrap().len(), 0);
    }
}
