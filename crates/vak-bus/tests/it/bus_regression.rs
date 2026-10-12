//! Comprehensive regression test suite for `vak-bus`.
//!
//! Validates zero-trust cryptography, Merkle causal chaining, W3C trace context,
//! subject ACLs, pub/sub fan-out, competing consumer work queues, and Dead-Letter Queue (DLQ).

use std::time::Duration;

use vak_bus::bus::{EventPublisher, EventSubscriber, InMemoryBus, WorkQueue};
use vak_bus::crypto::{CryptoError, decrypt_envelope, encrypt_envelope};
use vak_bus::envelope::{MessageEnvelope, TraceContext};
use vak_bus::subjects::{AclPolicy, Subject};

#[tokio::test]
async fn test_trace_context_propagation() {
    let root = TraceContext::new_root();
    assert_eq!(root.trace_id().len(), 32);
    assert_eq!(root.span_id().len(), 16);

    let child = root.child_span();
    assert_eq!(child.trace_id(), root.trace_id());
    assert_ne!(child.span_id(), root.span_id());
}

#[tokio::test]
async fn test_envelope_merkle_chaining() {
    let env1 = MessageEnvelope::new(
        "vak://agent/coder-1",
        "vak.work.task.started",
        "ws_alpha",
        "agent_01",
        1,
        MessageEnvelope::GENESIS_HASH,
        b"step 1 data".to_vec(),
        None,
    );

    let env1_hash = env1.compute_hash();

    // Valid successor in causal chain
    let env2 = MessageEnvelope::new(
        "vak://agent/coder-1",
        "vak.work.task.progress",
        "ws_alpha",
        "agent_01",
        2,
        &env1_hash,
        b"step 2 data".to_vec(),
        Some(env1.trace.child_span()),
    );

    assert!(MessageEnvelope::verify_merkle_link(&env1, &env2));

    // Invalid sequence number breaks link
    let env2_bad_seq = MessageEnvelope::new(
        "vak://agent/coder-1",
        "vak.work.task.progress",
        "ws_alpha",
        "agent_01",
        3, // Should be 2!
        &env1_hash,
        b"step 2 data".to_vec(),
        None,
    );
    assert!(!MessageEnvelope::verify_merkle_link(&env1, &env2_bad_seq));

    // Forged predecessor hash breaks link
    let env2_bad_hash = MessageEnvelope::new(
        "vak://agent/coder-1",
        "vak.work.task.progress",
        "ws_alpha",
        "agent_01",
        2,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        b"step 2 data".to_vec(),
        None,
    );
    assert!(!MessageEnvelope::verify_merkle_link(&env1, &env2_bad_hash));
}

#[tokio::test]
async fn test_crypto_envelope_round_trip_and_tampering() {
    let secret = b"super-strong-workspace-secret-key-32b!";
    let original_payload = b"sensitive proprietary agent output".to_vec();

    let mut env = MessageEnvelope::new(
        "vak://agent/secure",
        "vak.work.output",
        "ws_prod",
        "agent_sec",
        1,
        MessageEnvelope::GENESIS_HASH,
        original_payload.clone(),
        None,
    );

    // Encrypt
    encrypt_envelope(&mut env, secret, "v1").expect("encryption succeeds");
    assert!(env.encrypted);
    assert_ne!(env.payload, original_payload);

    // Decrypt
    decrypt_envelope(&mut env, secret).expect("decryption succeeds");
    assert!(!env.encrypted);
    assert_eq!(env.payload, original_payload);

    // Re-encrypt and tamper with payload
    encrypt_envelope(&mut env, secret, "v1").unwrap();
    env.payload[0] ^= 0x42; // Corrupt ciphertext
    let err = decrypt_envelope(&mut env, secret).unwrap_err();
    assert!(matches!(err, CryptoError::DecryptionFailed));
}

#[tokio::test]
async fn test_subjects_and_acl_enforcement() {
    let subj = Subject::EventsTokens {
        workspace_id: "ws1".into(),
        session_id: "sess1".into(),
    };
    assert_eq!(subj.to_subject_string(), "vak.events.ws1.sess1.tokens");

    let parsed = Subject::parse("vak.work.ws1.coder.task").unwrap();
    assert_eq!(
        parsed,
        Subject::WorkTask {
            workspace_id: "ws1".into(),
            role: "coder".into(),
        }
    );

    let acl = AclPolicy::for_worker("ws1", "sess1", "coder_agent", "coder");
    assert!(acl.can_publish("vak.events.ws1.sess1.tokens"));
    assert!(acl.can_publish("vak.events.ws1.sess1.terminal"));
    assert!(!acl.can_publish("vak.work.ws1.coder.task")); // Workers cannot forge work
    assert!(acl.can_subscribe("vak.work.ws1.coder.task"));
    assert!(acl.can_subscribe("vak.agent.ws1.coder_agent.inbox"));
    assert!(!acl.can_subscribe("vak.approvals.ws1.sess1.request")); // Cannot snoop approvals
}

#[tokio::test]
async fn test_ephemeral_pub_sub_fanout() {
    let bus = InMemoryBus::new();

    let mut rx1 = bus
        .subscribe("vak.events.ws_test.*.telemetry")
        .await
        .expect("sub 1");
    let mut rx2 = bus.subscribe("vak.events.ws_test.>").await.expect("sub 2");

    let env = MessageEnvelope::new(
        "vak://test",
        "vak.events.ws_test.sess_a.telemetry",
        "ws_test",
        "agent_a",
        1,
        MessageEnvelope::GENESIS_HASH,
        b"{\"rss_bytes\": 45000000}".to_vec(),
        None,
    );

    bus.publish("vak.events.ws_test.sess_a.telemetry", env.clone())
        .await
        .expect("publish succeeds");

    // Both subscribers should receive the envelope
    let received1 = tokio::time::timeout(Duration::from_millis(500), rx1.recv())
        .await
        .expect("timeout")
        .expect("received");
    assert_eq!(received1.id, env.id);

    let received2 = tokio::time::timeout(Duration::from_millis(500), rx2.recv())
        .await
        .expect("timeout")
        .expect("received");
    assert_eq!(received2.id, env.id);
}

#[tokio::test]
async fn test_durable_work_queue_claim_and_ack() {
    let bus = InMemoryBus::new();
    let queue = "code_review";

    let env1 = MessageEnvelope::new(
        "vak://lead",
        "vak.work.task",
        "ws_test",
        "lead_agent",
        1,
        MessageEnvelope::GENESIS_HASH,
        b"task 1: review pull request".to_vec(),
        None,
    );
    let env2 = MessageEnvelope::new(
        "vak://lead",
        "vak.work.task",
        "ws_test",
        "lead_agent",
        2,
        env1.compute_hash(),
        b"task 2: run integration tests".to_vec(),
        None,
    );

    let id1 = bus.enqueue(queue, env1).await.expect("enqueue 1");
    let id2 = bus.enqueue(queue, env2).await.expect("enqueue 2");

    // Worker 1 claims first task
    let claim1 = bus
        .claim(queue, "worker_1", Duration::from_millis(500))
        .await
        .expect("claim 1")
        .expect("task available");
    assert_eq!(claim1.task_id, id1);
    claim1.ack_handle.ack().await.expect("ack 1 succeeds");

    // Worker 2 claims second task
    let claim2 = bus
        .claim(queue, "worker_2", Duration::from_millis(500))
        .await
        .expect("claim 2")
        .expect("task available");
    assert_eq!(claim2.task_id, id2);
    claim2.ack_handle.ack().await.expect("ack 2 succeeds");

    // Queue is now empty
    let claim3 = bus
        .claim(queue, "worker_1", Duration::from_millis(100))
        .await
        .expect("claim empty");
    assert!(claim3.is_none());
}

#[tokio::test]
async fn test_dead_letter_queue_on_max_retries() {
    let bus = InMemoryBus::new();
    let queue = "fragile_work";

    let env = MessageEnvelope::new(
        "vak://lead",
        "vak.work.task",
        "ws_test",
        "lead",
        1,
        MessageEnvelope::GENESIS_HASH,
        b"poison task".to_vec(),
        None,
    );

    let task_id = bus.enqueue(queue, env).await.expect("enqueue");

    // Attempt 1: Nack with retry
    let claim1 = bus
        .claim(queue, "w1", Duration::from_millis(500))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim1.attempts, 1);
    claim1.ack_handle.nack(true).await.expect("nack 1");

    // Attempt 2: Nack with retry
    let claim2 = bus
        .claim(queue, "w2", Duration::from_millis(500))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim2.attempts, 2);
    claim2.ack_handle.nack(true).await.expect("nack 2");

    // Attempt 3: Exhausted! Reaches max_retries (3) and diverts to DLQ
    let claim3 = bus
        .claim(queue, "w3", Duration::from_millis(500))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim3.attempts, 3);
    claim3.ack_handle.nack(true).await.expect("nack 3 (dlq)");

    // Queue is now empty (poison pill removed)
    let empty = bus
        .claim(queue, "w1", Duration::from_millis(100))
        .await
        .unwrap();
    assert!(empty.is_none());

    // Verified in DLQ
    let dlq = bus.dead_letters();
    assert_eq!(dlq.len(), 1);
    assert_eq!(dlq[0].envelope_id, task_id);
    assert_eq!(dlq[0].attempts, 3);
    assert!(dlq[0].failure_reason.contains("exhausted"));

    // Metrics verified
    let metrics = bus.metrics().snapshot();
    assert_eq!(metrics.dead_letter_count, 1);
}
