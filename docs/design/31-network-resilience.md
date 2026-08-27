# 31 — Network resilience

Network behavior is separated by ownership. Local clients use the loopback
gateway; provider calls use the LLM retry/breaker contract; delivery records
are persisted before an external adapter is invoked.

## Local plane

`vak-server` binds the configured loopback address and writes its authenticated
endpoint to `runtime/gateway.json`. TUI, desktop, admin, and CLI use that
endpoint and bearer/cookie authentication. DHCP changes do not alter a
loopback connection. Remote exposure is an explicit operator decision outside
the Runtime process.

## Inference plane

`vak-llm` classifies provider failures, retries only within the route ladder
captured in the session contract, honors retry delays under a watchdog, and
records attempt receipts. Blind network failures feed the circuit breaker;
informed overload responses are paced by the retry policy. Cancellation stops
the attempt and preserves the partial result.

## Delivery plane

`vak-services` stores delivery jobs and state transitions transactionally. A
projection or external adapter can claim a pending job, mark it delivered, or
schedule a retry with an error and next-attempt timestamp. The source payload
is retained for deterministic replay and the inbox record remains available
when no external adapter accepts the packet.

## Failure contract

No network failure mutates session history outside an append-only event, changes
the committed route ladder, or silently drops an answer. Every failed request
returns a typed error and leaves enough state for inspection or retry.
