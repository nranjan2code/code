# 13 — Server mode

The client/server bet from the original architecture: one headless agent
core, many surfaces. `vakcoder serve --port 8901` exposes vak-core over
HTTP+SSE; the TUI, web clients, IDE extensions, and curl are all equal
consumers.

## Endpoints

| method | path | purpose |
|---|---|---|
| GET | `/health` | liveness |
| POST | `/sessions` | create session → `{session_id}` |
| POST | `/sessions/:id/run` `{prompt}` | 202; events stream on SSE |
| POST | `/sessions/:id/steering` `{text}` | queue mid-run input |
| POST | `/sessions/:id/approvals/:rid` `{approve}` | resolve a permission gate |
| GET | `/sessions/:id/events` | SSE stream of `AgentEvent` JSON |
| GET | `/sessions/:id/transcript` | derived messages + usage |

## Semantics

- **Stream-open handshake**: the SSE handler subscribes to the broadcast
  ring *before* notifying, then publishes a `StreamOpened` marker. Clients
  fire `/run` only after seeing it — no subscribe/publish races, verified by
  both the rust e2e test and the python smoke driver.
- **Approvals over HTTP**: permission asks publish `ApprovalRequested{id,
  tool, reason}` and park on a oneshot; `POST /approvals/:rid` resolves it.
  Grants are consumed exactly once (same invariant as the TUI approver).
- **Session ledger returns** after each run (`run_turn_with` now yields
  `(TurnOutcome, SessionLog)`), so transcripts stay queryable between runs.
- Second concurrent `/run` on a live session → `409 Conflict`.

## Implementation notes

- axum + broadcast ring; a single mpsc→broadcast bridge per run.
- The bridge forwards into the **broadcast** channel — an earlier draft fed
  the mpsc back into itself, producing an infinite echo loop. The e2e test's
  event-kind assertions now pin this class of bug.
- SDK seams added to Core for embedding/tests: `set_provider_instance`,
  `set_sessions_home`.
