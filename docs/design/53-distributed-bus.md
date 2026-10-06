# 53 — Distributed Event & Message Fabric (`crates/vak-bus`)

Status: **implemented in 3.0.22** (`crates/vak-bus`).

## Mission

Provide a zero-compromise, production-grade distributed event and message fabric
engineered for **thousands of concurrent and coordinated agents** operating across
local macOS, headless Linux nodes, Docker containers, hybrid environments, and
multi-node cloud clusters.

Given complex, massive multi-agent execution, the fabric guarantees:

1. **Decoupled Asynchronous Swarms**: Agents dispatch and claim work asynchronously via competing consumer queues rather than synchronously blocking Tokio call stacks.
2. **Zero-Trust Defense-in-Depth Security**: Payloads are encrypted with authenticated AES-256-GCM bound to envelope metadata via AAD, with role-based subject ACLs and NKey/Ed25519 identity.
3. **Cryptographic Causal Traceability**: Every event links into an append-only Merkle hash chain (`prev_event_hash`) and propagates standard W3C Distributed Tracing context (`traceparent`, `tracestate`).
4. **Complete Operational Observability**: Real-time metrics counters, latencies, worker queue lag, and poison-pill Dead-Letter Queue (DLQ) isolation.
5. **Dual-Engine Operation**: Real NATS Core + JetStream client (`async-nats`) for distributed cloud/hybrid topologies, paired with an embedded zero-dependency concurrency engine (`InMemoryBus`) for standalone local runs and deterministic CI testing without mocks or stubs.

---

## Data-architecture amendments (plan M5b, 2026-10-06)

- **The bus carries references only** (review R22). The server publishes a
  `SystemEvent` as its kind and the ids a subscriber needs (session, entry,
  approval, question, provider and model names), never its summary,
  preview, reason, label or error text
  (`vak_server::bus::reference`, test `bus_payload_carries_references_only`).
  A subscriber that needs more reads it through the authenticated API.
- **Work streams age out.** `NatsBus` creates each work queue's stream
  itself (`work_stream_config`): work-queue retention and a `max_age` of
  `WORK_QUEUE_MAX_AGE` (24 hours), with a durable explicit-ack consumer, so
  an unclaimed task does not outlive its usefulness and the stream is never
  an archive.

## 1. Architectural Topology

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Distributed Agent Swarm                            │
│           (Thousands of Agents on Mac Desktop / Cloud Pods / Headless)       │
└──────────────────────┬───────────────────────────────┬──────────────────────┘
                       │                               │
       Ephemeral Telemetry & Logs              Durable Work & Receipts
                       │                               │
                       ▼                               ▼
┌──────────────────────────────────────┐ ┌────────────────────────────────────┐
│      1. Ephemeral Event Plane        │ │       2. Durable Work Plane        │
│          (NATS Core Pub/Sub)         │ │          (NATS JetStream)          │
├──────────────────────────────────────┤ ├────────────────────────────────────┤
│ • Streaming tokens & thinking deltas │ │ • WorkQueue task distribution      │
│ • ANSI terminal stdout/stderr chunks │ │ • Direct agent mailboxes / inboxes │
│ • 500ms process RSS & CPU telemetry  │ │ • Verified signed work receipts    │
│ • Fan-out to Web/Desktop Workbench   │ │ • Dead-Letter Queues (DLQ)         │
└──────────────────────────────────────┘ └────────────────────────────────────┘
```

---

## 2. Deterministic Subject Namespace Algebra

Subjects follow a strict, non-colliding hierarchy:

```text
vak.<plane>.<workspace_id>.<entity>.<target_or_session>.<verb_or_type>
```

### Registered Subject Patterns:

| Subject | Plane | Purpose | Policy |
| :--- | :--- | :--- | :--- |
| `vak.events.<ws>.<sess>.tokens` | Ephemeral | Streaming assistant and thinking tokens | Fan-out pub/sub, drop-on-lag |
| `vak.events.<ws>.<sess>.terminal` | Ephemeral | Streaming ANSI stdout/stderr chunks + `\r` folding | Fan-out pub/sub, drop-on-lag |
| `vak.events.<ws>.<sess>.telemetry` | Ephemeral | 500ms process RSS memory and CPU probes | Latest-value window |
| `vak.work.<ws>.<role>.task` | Durable | Competing-consumer worker task queues | JetStream WorkQueue (single ack) |
| `vak.agent.<ws>.<agent>.inbox` | Durable | Direct agent steering, signals, and messages | JetStream Interest (point-to-point) |
| `vak.receipts.<ws>.<sess>.completed` | Durable | Verified work receipts & execution evidence | Append-only, audit-logged |
| `vak.approvals.<ws>.<sess>.request` | Durable | Elevated permission & security gate requests | Persisted until verdict |
| `vak.dlq.<ws>.<agent>.failed` | Durable | Poison pills and exhausted retries | Persisted for Ops Center alerts |

---

## 3. Defense-in-Depth Security

### 3.1 Authenticated AES-256-GCM Envelope Encryption
- Workspace secrets derive a 256-bit symmetric key via **HKDF-SHA256** using the salt `b"vak-bus-hkdf-salt-v1"` and info `b"vak-bus-aes-256-gcm-<key_id>"`.
- Payloads are encrypted in-place using authenticated AES-256-GCM with a 96-bit (12-byte) cryptographically secure random nonce generated via `ring::rand::SystemRandom`.
- **Additional Authenticated Data (AAD)** binds the envelope's ID, event type, workspace ID, and sequence number (`format!("{id}:{type}:{ws}:{seq}")`). Any attempt to tamper with headers, swap envelope targets, or alter ciphertext causes `CryptoError::DecryptionFailed` immediately.

### 3.1a Where the bus's settings and secrets live
- The NATS URL and the *name* of the variable that holds the workspace encryption secret are ordinary settings in `[server.bus]` (`nats_url`, `workspace_secret_env`). `[server]` is privileged, so an untrusted project's values are not used.
- The NATS credentials JWT and nkey seed are secrets. They live in the secrets chain under `VAK_BUS_NATS_CREDENTIALS_JWT` and `VAK_BUS_NATS_NKEY_SEED` (`vak_config::BUS_NATS_JWT_VAR`, `BUS_NATS_NKEY_SEED_VAR`), read with `Core::bus_secret`, and are never accepted in TOML.
- `PUT /config/bus` writes the settings to the project config and the secrets to the project secret scope through `vak_config::credentials`, then rebuilds the live bus (invariant 31); `DELETE /config/bus` removes both and returns the bus to local-only. `GET /config/bus` never returns a secret. `bus_credentials_never_written_to_a_file` (vak-server tests) checks that no file under the workspace or the data home ever holds one.

### 3.2 Subject-Level Role-Based ACLs
The `AclPolicy` enforces least-privilege access per agent role:
- **Worker Agents** are permitted to publish only to their assigned session events, receipts, and DLQ.
- **Worker Agents** are strictly denied from publishing to `vak.work.<ws>.>` (preventing task forgery) or snooping on approval channels `vak.approvals.<ws>.>`.
- Default-deny prevents any agent from subscribing to other agents' inboxes.

---

## 4. Cryptographic Causal Traceability

### 4.1 CloudEvents 1.0 + W3C Distributed Tracing
Every message envelope carries:
- CloudEvents headers (`id`, `source`, `specversion`, `type`, `time`, `datacontenttype`).
- W3C Distributed Tracing headers:
  - `traceparent`: `00-<trace_id_32_hex>-<span_id_16_hex>-01`
  - `tracestate`: vendor-specific state pairs.
- Spawning child tasks or workers calls `trace.child_span()`, preserving the global `trace_id` while generating a unique span ID.

### 4.2 Merkle Causal Hash Chaining
- Each envelope computes a canonical SHA-256 Merkle leaf:
  ```text
  SHA-256(specversion | id | source | type | time | ws | agent | seq | prev_event_hash | payload_hash)
  ```
- The successor envelope records `lineage.prev_event_hash = prev.compute_hash()` and `seq_num = prev.seq_num + 1`.
- `MessageEnvelope::verify_merkle_link(&prev, &current)` verifies the cryptographic link in constant time using `subtle::ConstantTimeEq`. Any reordering, omission, or injection invalidates the chain.

---

## 5. Observability & Dead-Letter Queue (DLQ)

### 5.1 Real-Time Metrics
`BusMetrics` maintains thread-safe lock-free atomic counters:
- `published_count`, `received_count`
- `bytes_published`, `bytes_received`
- `dead_letter_count`
- `active_queue_lag`

### 5.2 Dead-Letter Queue & Poison Pill Isolation
- When an agent execution fails or crashes, tasks are retried up to `max_retries: 3`.
- Upon exhaustion, the task is diverted to `vak.dlq.<ws>.<agent>.failed` with a `DeadLetterEvent` containing:
  - Envelope ID and original message payload.
  - Number of attempts.
  - Failure reason and timestamp.
  - Worker ID.
- The incident triggers an alert in the Operations Center (`/ops/center`).

---

## 6. Verification & Test Suite

The implementation is verified by unit and regression test suites in `crates/vak-bus`:
- `subjects::tests::subject_pattern_matching`: Wildcard matching (`*`, `>`).
- `subjects::tests::acl_worker_enforcement`: Least-privilege publish/subscribe ACL checks.
- `crypto::tests::round_trip_encryption_decryption`: AES-256-GCM round-trip.
- `crypto::tests::tamper_detection_ciphertext`: Corrupted ciphertext rejection.
- `crypto::tests::tamper_detection_metadata_aad`: Header alteration rejection via AAD.
- `tests/bus_regression.rs`:
  - `test_trace_context_propagation`: W3C parent/child span generation.
  - `test_envelope_merkle_chaining`: Valid link verification, sequence mismatch rejection, forged hash rejection.
  - `test_crypto_envelope_round_trip_and_tampering`: Tamper resistance.
  - `test_subjects_and_acl_enforcement`: Subject algebra and ACL enforcement.
  - `test_ephemeral_pub_sub_fanout`: Concurrent multi-subscriber broadcast.
  - `test_durable_work_queue_claim_and_ack`: Competing consumer task distribution and ACKs.
  - `test_dead_letter_queue_on_max_retries`: Automatic poison pill diversion to DLQ after 3 retries.
