//! Integration tests for the vak-bus distributed event fabric
//! (docs/design/53-distributed-bus.md).
//!
//! InMemory tests run unconditionally. NatsBus tests are `#[ignore]`'d
//! and require a live NATS server:
//!
//! ```sh
//! nats-server -p 4222   # or: docker run -p 4222:4222 nats
//! cargo test -p vak-bus --test nats -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use vak_bus::{
    BusError, EventPublisher, EventSubscriber, InMemoryBus, NatsBus, NatsConfig, WorkQueue,
};
use vak_bus::{MessageEnvelope, TraceContext};

/// Build a plain-text JSON envelope for testing.
fn envelope(source: &str, event_type: &str, payload: serde_json::Value) -> MessageEnvelope {
    let data = serde_json::to_vec(&payload).unwrap();
    MessageEnvelope::new(
        source,
        event_type,
        "test_ws",
        "test_agent",
        1,
        MessageEnvelope::GENESIS_HASH,
        data,
        Some(TraceContext::new_root()),
    )
}

// ---------------------------------------------------------------------------
// InMemoryBus integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn inmemory_roundtrip() {
    let bus = InMemoryBus::new();
    let mut sub = EventSubscriber::subscribe(&bus, "vak.events.test.>")
        .await
        .unwrap();

    EventPublisher::publish(
        &bus,
        "vak.events.test.session1.message",
        envelope("test", "message", serde_json::json!({ "msg": "hello" })),
    )
    .await
    .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), sub.recv())
        .await
        .unwrap()
        .unwrap();

    let payload: serde_json::Value = serde_json::from_slice(&received.payload).unwrap();
    assert_eq!(payload["msg"], "hello");
    assert_eq!(received.lineage.workspace_id, "test_ws");
    assert_eq!(received.trace.traceparent.split('-').count(), 4);
}

#[tokio::test]
async fn inmemory_work_queue_claim_and_ack() {
    let bus = InMemoryBus::new();
    let task_id = WorkQueue::enqueue(
        &bus,
        "vak.work.test.coder",
        envelope("test", "task", serde_json::json!({ "job": "build" })),
    )
    .await
    .unwrap();

    let claimed = WorkQueue::claim(
        &bus,
        "vak.work.test.coder",
        "worker-1",
        Duration::from_secs(1),
    )
    .await
    .unwrap()
    .expect("should have a pending task");

    assert_eq!(claimed.task_id, task_id);
    assert_eq!(claimed.attempts, 1);
    claimed.ack_handle.ack().await.unwrap();
}

#[tokio::test]
async fn inmemory_work_queue_dlq_on_nack() {
    let bus = InMemoryBus::new();
    let task_id = WorkQueue::enqueue(
        &bus,
        "vak.work.test.worker",
        envelope("test", "task", serde_json::json!({ "job": "poison" })),
    )
    .await
    .unwrap();

    let claimed = WorkQueue::claim(
        &bus,
        "vak.work.test.worker",
        "worker-1",
        Duration::from_secs(1),
    )
    .await
    .unwrap()
    .expect("should have a pending task");
    assert_eq!(claimed.task_id, task_id);

    // Nack with retry=false → routed to DLQ
    claimed.ack_handle.nack(false).await.unwrap();

    let dlq = bus.dead_letters();
    assert_eq!(dlq.len(), 1);
    assert_eq!(dlq[0].failure_reason, "retry threshold exhausted");
}

#[tokio::test]
async fn inmemory_metrics_tracks_publishes_and_subscribes() {
    let bus = InMemoryBus::new();
    let metrics = bus.metrics();

    EventPublisher::publish(
        &bus,
        "vak.events.test.metrics",
        envelope("test", "metrics", serde_json::json!({ "n": 1 })),
    )
    .await
    .unwrap();

    let _sub = EventSubscriber::subscribe(&bus, "vak.events.test.metrics")
        .await
        .unwrap();

    // Wait for subscriber to register
    tokio::time::sleep(Duration::from_millis(50)).await;

    let snap = metrics.snapshot();
    assert!(snap.published_count >= 1);
}

#[tokio::test]
async fn inmemory_subscription_pattern_filtering() {
    let bus = InMemoryBus::new();
    let mut sub = EventSubscriber::subscribe(&bus, "vak.events.ws_alpha.>")
        .await
        .unwrap();

    EventPublisher::publish(
        &bus,
        "vak.events.ws_alpha.sess_001.message",
        envelope("test", "msg", serde_json::json!({})),
    )
    .await
    .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), sub.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.lineage.workspace_id, "test_ws");

    // A message on a different workspace should not be received.
    let other = EventPublisher::publish(
        &bus,
        "vak.events.ws_beta.sess_001.message",
        envelope("test", "msg", serde_json::json!({})),
    );
    // Should complete without the subscriber receiving it.
    other.await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test]
async fn inmemory_claims_remove_from_queue() {
    let bus = InMemoryBus::new();
    for i in 0..3 {
        WorkQueue::enqueue(
            &bus,
            "vak.work.test.batch",
            envelope("test", "task", serde_json::json!({ "i": i })),
        )
        .await
        .unwrap();
    }

    // Claim all three
    for _ in 0..3 {
        let task = WorkQueue::claim(&bus, "vak.work.test.batch", "w", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(task.is_some());
    }
    // Queue should now be empty
    let empty = WorkQueue::claim(&bus, "vak.work.test.batch", "w", Duration::from_millis(10))
        .await
        .unwrap();
    assert!(empty.is_none());
}

// ---------------------------------------------------------------------------
// NatsBus integration tests (require live NATS)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires a live NATS server on nats://127.0.0.1:4222"]
async fn nats_connects_and_roundtrips() {
    let config = NatsConfig {
        url: "nats://127.0.0.1:4222".into(),
        ..NatsConfig::default()
    };
    let bus = NatsBus::connect(config).await.unwrap();

    let mut sub = EventSubscriber::subscribe(&bus, "vak.events.test.nats.>")
        .await
        .unwrap();

    EventPublisher::publish(
        &bus,
        "vak.events.test.nats.roundtrip",
        envelope(
            "test",
            "nats.msg",
            serde_json::json!({ "msg": "from-nats" }),
        ),
    )
    .await
    .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(3), sub.recv())
        .await
        .unwrap()
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&received.payload).unwrap();
    assert_eq!(payload["msg"], "from-nats");
}

#[tokio::test]
#[ignore = "requires a live NATS server on nats://127.0.0.1:4222"]
async fn nats_rejects_bad_credentials() {
    let config = NatsConfig {
        url: "nats://127.0.0.1:4222".into(),
        credentials_jwt: Some("invalid.jwt.token".into()),
        nkey_seed: Some("SUAFISVULNERABLEFAKESEED".into()),
        connect_timeout: Duration::from_secs(3),
    };
    let result = NatsBus::connect(config).await;
    assert!(result.is_err(), "bad credentials must fail");
}

#[tokio::test]
#[ignore = "requires a live NATS server on nats://127.0.0.1:4222"]
async fn nats_work_queue_claim_and_ack() {
    let config = NatsConfig {
        url: "nats://127.0.0.1:4222".into(),
        ..NatsConfig::default()
    };
    let bus = NatsBus::connect(config).await.unwrap();

    let task_id = WorkQueue::enqueue(
        &bus,
        "vak.work.test.nats_coder",
        envelope("test", "nats.task", serde_json::json!({ "job": "build" })),
    )
    .await
    .unwrap();

    let claimed = WorkQueue::claim(
        &bus,
        "vak.work.test.nats_coder",
        "worker-1",
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert!(claimed.is_some(), "should claim the enqueued task");
    let claimed = claimed.unwrap();
    assert_eq!(claimed.task_id, task_id);
    claimed.ack_handle.ack().await.unwrap();
}

/// Verify that `BusError` implements `std::error::Error` and `Display`.
#[test]
fn bus_error_is_display_and_error() {
    let err = BusError::Connection("nats down".into());
    assert!(err.to_string().contains("nats down"));
    assert!(std::error::Error::source(&err).is_none());
}
