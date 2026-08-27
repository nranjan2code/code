# 31 — Network resilience

Network behavior is separated by ownership. Local clients use the loopback
gateway; provider calls return typed failures; delivery records are persisted
before an external adapter is invoked.

## Local plane

`vak-server` binds the configured loopback address and writes its authenticated
endpoint to `runtime/gateway.json`. Native TUI, desktop, CLI, and tray resolve
that receipt exclusively through `vak-client`; no adapter guesses a port or
parses a private connection format of its own. The browser Admin console uses
the same origin and establishes an HttpOnly cookie through `/auth/login`.
DHCP changes do not alter a loopback connection. Remote exposure is an
explicit operator decision outside the Runtime process.

## Inference plane

`vak-llm` classifies provider failures into typed values. Runtime records the
failure and preserves partial output; the current request path does not add a
hidden retry or provider-switching loop. Cancellation stops the attempt and
preserves the partial result.

## Delivery plane

`vak-services` stores delivery jobs and state transitions transactionally. A
projection or external adapter can claim a pending job, mark it delivered, or
schedule a retry with an error and next-attempt timestamp. The source payload
is retained for deterministic replay and the inbox record remains available
when no external adapter accepts the packet. Delivery retry decisions belong to
the adapter that owns the destination.

## Failure contract

No network failure mutates session history outside an append-only event or
silently drops an answer. Every failed request returns a typed error and leaves
enough state for inspection by the owning Runtime or adapter.
