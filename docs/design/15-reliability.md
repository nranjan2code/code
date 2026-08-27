# 15 — Reliability and recovery

Reliability is a Runtime contract: state transitions are durable, cancellation
is explicit, and every failure is returned as typed data. The Runtime does not
hide provider retries or create a second recovery state machine.

## Provider and tool failures

`vak-llm` maps transport, authentication, rate-limit, overload, parse, and
abort conditions to `LlmError`. `vak-agent` returns those errors to Runtime;
the session retains any output already recorded. Brokered tools return a typed
error result and never turn a failure into an implicit allow.

Provider retry and route selection are not performed by the current Runtime
request path. A failed provider step therefore produces one failed run with
its recorded error; cancellation is never retried or replaced with another
provider.

## Durable state and crash recovery

- Session JSONL is append-only. A complete line is either present or absent;
  Runtime reconstructs model context from the ledger rather than a live cache.
- SQLite transactions protect control-plane records and enforce one terminal
  run transition. Duplicate completion or cancellation is idempotent.
- Checkpoint manifests are stored as content-addressed blobs and restored only
  through Runtime after project and permission checks.
- Delivery jobs retain their source payload and status in Runtime state so an
  external adapter can inspect and acknowledge them without mutating a session.

## Start, stop, and resume

| Operation | Contract |
|---|---|
| start | `exec`, TUI, desktop, or `flow run` submits a Runtime run |
| stop (user) | cancellation token reaches the provider and broker; partial output is retained |
| stop (server) | `POST /runs/:id/cancel` targets the live run ID |
| resume (session) | `exec --session <id>` reuses the existing append-only ledger |
| flow run | validates a project definition, then submits a normal run |
| shutdown | gateway performs graceful shutdown and removes its runtime receipt |

Cancellation is scoped to an admitted run, not an idle session. Runtime emits
one terminal status and persists one terminal outcome even when cancellation
and completion race.

## Capability revocation

Changing permission or sandbox policy advances the capability epoch, cancels
old-epoch work, waits for the revocation barrier, rejects stale approvals, and
only then admits new work. A worker that presents a stale epoch is denied by
the broker.

## Network boundary

The gateway writes its authenticated loopback endpoint to
`runtime/gateway.json`. Native surfaces resolve that receipt through the one
`vak-client` connection manager, which validates PID, endpoint, authenticated
`/version`, and authenticated `/health` before reporting Ready. Desktop keeps
the selected project and retries that shared handshake after a disconnect;
TUI and CLI re-resolve it for each invocation. Provider network errors remain
typed failures. Delivery outages leave the durable job pending or retryable
for the owning external adapter.

## Verification

The workspace tests cover stream parsing, typed provider failures, cancellation
with partial output, idempotent terminal transitions, capability revocation,
broker containment, checkpoint/blob integrity, and delivery state transitions.
