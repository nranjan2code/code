# Record reference

Generated from the record types by `records_reference_is_current`
(`crates/vak-server/src/traced_rows_tests.rs`). Do not edit by hand:
change the type, then run
`VAK_UPDATE_RECORDS=1 cargo test -p vak-server records_reference_is_current`.

Every record below carries an optional `trace` (the run's trace key) and
an optional `actor` (the principal that acted), both additive and omitted
when unknown (docs/design/73 section 4). Within a major version a field is
added, never removed, renamed or retyped, and readers ignore unknown
fields. A `ledger` is append-only JSONL; a `record` is one file per entity.

## action_receipt

Type `ActionReceipt` in `crates/vak-server/src/operations.rs`. Class: ledger. Stored in: `operations/actions.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `receipt_id` | `String` | yes |  |
| `service` | `String` | yes |  |
| `action` | `String` | yes |  |
| `requested_at` | `DateTime<Utc>` | yes |  |
| `completed_at` | `DateTime<Utc>` | yes |  |
| `succeeded` | `bool` | yes |  |
| `verification` | `ActionVerification` | yes |  |
| `persisted` | `bool` | yes |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## activity_row

Type `ActivityRow` in `crates/vak-core/src/finops.rs`. Class: ledger. Stored in: `activity-log.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `ts` | `chrono::DateTime<chrono::Utc>` | yes |  |
| `kind` | `String` | yes |  |
| `name` | `String` | yes |  |
| `success` | `bool` | yes |  |
| `duration_ms` | `Option<u64>` | no |  |
| `session_id` | `Option<String>` | no |  |
| `plugin` | `Option<String>` | no |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## audience_grant

Type `AudienceGrant` in `crates/vak-server/src/coworking.rs`. Class: ledger. Stored in: `coworking/grants.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `grant_id` | `String` | yes |  |
| `principal_id` | `String` | yes |  |
| `display_name` | `String` | yes |  |
| `conversation_id` | `String` | yes |  |
| `audience_id` | `String` | yes |  |
| `capabilities` | `Vec<String>` | yes |  |
| `token_hash` | `String` | yes |  |
| `created_at` | `String` | yes |  |
| `expires_at` | `String` | yes |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## budget_alert_row

Type `BudgetAlertRow` in `crates/vak-core/src/finops.rs`. Class: ledger. Stored in: `budget-alerts.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `ts` | `chrono::DateTime<chrono::Utc>` | yes | (written as `kind`) |
| `level` | `AlertLevel` | yes |  |
| `day_total_usd` | `f64` | yes | Day spend as observed when the alert fired. |
| `session_id` | `String` | yes |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## candidate_record

Type `CandidateRecord` in `crates/vak-sandbox/src/lib.rs`. Class: ledger. Stored in: `sandbox/records.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `record_id` | `String` | yes |  |
| `session_id` | `String` | yes |  |
| `turn_id` | `String` | yes |  |
| `result_id` | `String` | yes |  |
| `execution_id` | `String` | yes |  |
| `environment_id` | `String` | yes |  |
| `candidate_digest` | `String` | yes |  |
| `candidate` | `CandidateManifest` | yes |  |
| `verified` | `bool` | yes |  |
| `draft_checks` | `Vec<TargetCheckResult>` | no | Format evidence observed from the frozen draft bytes. Acceptance runs |
| `updated_at` | `String` | yes |  |
| `parent_candidate_id` | `Option<String>` | no | The saved version used as input for a human-requested revision. |
| `revision_session_id` | `Option<String>` | no | Durable child session whose tool receipts produced this version. |
| `narrowed` | `Option<NarrowedDraft>` | no | Set when a person kept only some of an Office draft's changes: this |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## commitment_event

Type `Event` in `crates/vak-commit/src/ledger.rs`. Class: ledger. Stored in: `commitments.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `event_id` | `String` | yes |  |
| `commitment_id` | `String` | yes |  |
| `ts` | `chrono::DateTime<chrono::Utc>` | yes |  |
| `kind` | `EventKind` | yes | (flattened into the row) |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## cost_row

Type `CostRow` in `crates/vak-core/src/finops.rs`. Class: ledger. Stored in: `cost-log.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `ts` | `chrono::DateTime<chrono::Utc>` | yes |  |
| `model` | `String` | yes |  |
| `provider` | `String` | no | Serving provider of the frozen-ladder leg (Phase R per-provider |
| `input_tokens` | `u64` | yes |  |
| `output_tokens` | `u64` | yes |  |
| `cache_read_input_tokens` | `Option<u64>` | no |  |
| `usd` | `Option<f64>` | no | Estimated USD. `None` when the model is unpriced — absent is |
| `source` | `String` | yes | Always "estimated" today; reserved for providers that return real |
| `session_id` | `String` | yes |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## environment_record

Type `EnvironmentRecord` in `crates/vak-sandbox/src/lib.rs`. Class: ledger. Stored in: `sandbox/records.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `record_id` | `String` | yes |  |
| `environment_id` | `String` | yes |  |
| `state` | `EnvironmentState` | yes |  |
| `plan` | `EnvironmentPlan` | yes |  |
| `updated_at` | `String` | yes |  |
| `detail` | `Option<String>` | no |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## evidence_row

Type `EvidenceRow` in `crates/vak-core/src/routing.rs`. Class: ledger. Stored in: `routing-evidence.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `ts` | `chrono::DateTime<chrono::Utc>` | yes |  |
| `provider` | `String` | yes |  |
| `model` | `String` | yes |  |
| `outcome` | `String` | yes | success \| failure \| unknown |
| `latency_ms` | `u64` | yes |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## inbox_entry

Type `Entry` in `crates/vak-core/src/inbox.rs`. Class: ledger. Stored in: `inbox.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `id` | `String` | yes |  |
| `ts` | `DateTime<Utc>` | yes |  |
| `kind` | `Kind` | yes |  |
| `title` | `String` | yes |  |
| `body` | `String` | yes |  |
| `session_id` | `Option<String>` | no |  |
| `task_id` | `Option<String>` | no |  |
| `result_id` | `Option<String>` | no |  |
| `dedupe_key` | `Option<String>` | no |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## incident_record

Type `IncidentRecord` in `crates/vak-server/src/operations.rs`. Class: ledger. Stored in: `operations/incidents.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `id` | `String` | yes |  |
| `fingerprint` | `String` | yes |  |
| `severity` | `String` | yes |  |
| `status` | `String` | yes |  |
| `source` | `String` | yes |  |
| `title` | `String` | yes |  |
| `detail` | `String` | yes |  |
| `first_seen` | `DateTime<Utc>` | yes |  |
| `last_seen` | `DateTime<Utc>` | yes |  |
| `occurrences` | `u64` | yes |  |
| `workspace` | `Option<String>` | no |  |
| `evidence` | `Vec<String>` | yes |  |
| `resolution` | `Option<String>` | no |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## misread_row

Type `MisreadRow` in `crates/vak-core/src/misread.rs`. Class: ledger. Stored in: `intent-evidence.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `ts` | `chrono::DateTime<chrono::Utc>` | yes |  |
| `act` | `String` | yes | The act that was read, so accuracy can be reported per cell. |
| `stakes` | `String` | yes |  |
| `tier` | `String` | yes | Which tier produced the reading; a lexicon miss and a classifier miss |
| `resolver_version` | `u32` | yes |  |
| `outcome` | `String` | yes |  |
| `wanted` | `Option<String>` | no | The tool the model used after the reading left it deferred. Present |
| `sliced` | `bool` | no | Whether the reading was confident enough to decide what was loaded. |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## outbox_record

Type `OutboxRecord` in `crates/vak-delivery/src/outbox.rs`. Class: record. Stored in: `gateway outbox, one file per delivery job`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `schema_version` | `u16` | yes |  |
| `job` | `DeliveryJob` | yes |  |
| `state` | `OutboxState` | yes |  |
| `attempts` | `u32` | yes |  |
| `created_at_ms` | `u64` | yes |  |
| `updated_at_ms` | `u64` | yes |  |
| `packet` | `Option<DeliveryPacket>` | no |  |
| `last_error` | `Option<String>` | no |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## preview_preparation_record

Type `PreviewPreparationRecord` in `crates/vak-sandbox/src/lib.rs`. Class: ledger. Stored in: `sandbox/records.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `record_id` | `String` | yes |  |
| `session_id` | `String` | yes |  |
| `result_id` | `String` | yes |  |
| `candidate_id` | `String` | yes |  |
| `candidate_digest` | `String` | yes |  |
| `environment_id` | `String` | yes |  |
| `state` | `EnvironmentState` | yes |  |
| `command` | `String` | yes |  |
| `evidence` | `String` | yes |  |
| `updated_at` | `String` | yes |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## promotion_record

Type `PromotionRecord` in `crates/vak-sandbox/src/lib.rs`. Class: ledger. Stored in: `sandbox/records.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `record_id` | `String` | yes |  |
| `session_id` | `String` | yes |  |
| `result_id` | `String` | yes |  |
| `candidate_digest` | `String` | yes |  |
| `candidate_id` | `String` | yes |  |
| `receipt` | `PromotionReceipt` | yes |  |
| `workspace_checks` | `Vec<WorkspaceCheckPlan>` | no |  |
| `updated_at` | `String` | yes |  |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |

## security_event

Type `SecurityEvent` in `crates/vak-core/src/security_events.rs`. Class: ledger. Stored in: `security-events.jsonl`. Version: 1.

| Field | Type | Required | Notes |
|---|---|---|---|
| `ts` | `DateTime<Utc>` | yes |  |
| `kind` | `EventKind` | yes |  |
| `label` | `String` | yes | Short human-readable label (e.g. "rate_limit", "auth_failure"). |
| `detail` | `String` | yes | Structured detail — freeform JSON string. |
| `ip` | `Option<String>` | no | Source IP when available. |
| `trace` | `Option<vak_session::trace::TraceKey>` | no |  |
| `actor` | `Option<vak_session::ids::PrincipalId>` | no |  |
