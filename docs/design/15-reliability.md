# 15 — Reliability: QoS, crash, recovery, start/stop/resume

The standard failure matrix and where each case is handled.

## Transient failures (QoS)

- **Retry with backoff**: every model step (connect + stream + collect) is
  retried up to `max_retries` (default 3) when the error is retryable
  (429 / 529 / network). Delay = `retry_base_backoff_ms * 2^(n-1)` with ±50%
  jitter, capped at 30s; server-advised `Retry-After` overrides. User aborts
  and partial-output aborts are never retried.
- **Watchdog**: each step runs under `request_timeout` (default 600s,
  `request_timeout_secs = 0` disables). A hung stream becomes a retryable
  deadline error instead of hanging forever.
- Surfaced as `AgentEvent::RetryScheduled{attempt, delay_ms, reason}` so all
  UIs show the wait; config keys `max_retries`, `retry_base_backoff_ms`,
  `request_timeout_secs`.
- **Run-level endurance** (`run_retry_attempts`, default 6;
  `run_retry_base_backoff_ms`, default 2s): a fault window can outlast one
  step's retry budget — a sustained 429 window or slow/hung upstream used to
  kill the whole run on first step exhaustion (found in the live chaos
  campaign). Now, when a step exhausts its retries with a *transient* error
  (429/529/network/truncated stream), the loop backs off cancel-aware and
  re-attempts the same turn: nothing was committed to the ledger, so the
  re-attempt is exact and turn budgets are not consumed by infrastructure
  pain. An OPEN circuit breaker still fails fast — fresh evidence of a dead
  provider must not cost the user minutes of waiting. Permanent errors
  (auth/bad request/api) are never endured.
- **Planner calls retry too** (found in live testing): `plan`'s model calls
  bypass the agent loop, so they carry their own bounded retry (3 attempts,
  exponential backoff, `Retry-After` honored, cancel-aware). A transient
  failure mid-planning no longer fails the whole run closed.
- **Truncated SSE streams on OpenAI-compatible proxies**: some endpoints
  (OpenCode Zen free tier) end the body after the last content chunk without
  `[DONE]`/`finish_reason`. The openai-completions adapter treats a clean
  close *with content* as de facto completion (`EndTurn`); a close with no
  content still fails closed as `Parse`.

## Crash & recovery

- **Session ledgers are append-only JSONL** — a crash mid-write loses at most
  the trailing partial line, which `SessionLog::open` skips; nothing else is
  rewritten. Model context is always *derived* from the log, so a crashed run
  resumes with full history.
- **Checkpoint ledgers** write via tmp-file + rename (atomic).
- **Flow state ledgers** persist after every node completion; `flow run
  --resume` replays only non-completed nodes.
- **Terminal restore**: TUI raw mode is restored by a Drop guard even on
  panic.

## Start / stop / resume

| operation | path |
|---|---|
| start | `exec`, TUI, `serve`, `flow run`, `plan` |
| stop (user) | Ctrl-C → cancel token → `Aborted{partial}`; partial assistant output is persisted |
| stop (server) | `POST /sessions/:id/cancel` → same path + `RunFinished{cancelled}` |
| resume (session) | `exec --session <id>` reopens the ledger; projection includes all history |
| resume (flow) | `flow run <name> --resume` skips completed nodes |
| shutdown (server) | ctrl_c → graceful drain (`with_graceful_shutdown`) |

## Circuit breaker (cross-run QoS)

Per-step retries protect one run; the **circuit breaker** protects every run
from a dead provider. Shared via Core across all runs of a process:

- Only **blind failures** count: network loss, watchdog deadlines, truncated
  or malformed streams. Informed transience — 429 with `Retry-After`,
  explicit 503/529 overload — is the server saying "try again later"; it
  feeds endurance and must not open the circuit mid-window (found in the
  live chaos campaign: an opened breaker killed runs the window would have
  released seconds later).
- `circuit_breaker_threshold` consecutive failures (default 5) open the
  circuit; while open, steps fail fast with the remaining cooldown instead
  of burning their retry budget.
- After `circuit_breaker_cooldown_secs` (default 60) the circuit half-closes:
  one probe gets through, and any success resets the counter.
- Run-level endurance paces its waits to the breaker's remaining cooldown,
  so a run caught on the wrong side of an open circuit waits for the probe
  instead of exhausting its budget on instant no-op failures.

Config keys: `circuit_breaker_threshold`,
`circuit_breaker_cooldown_secs` (`0` cooldown disables opening).

## Frozen route ladder (landed — Phase B, router-grade ordering in Phase R)

`27-vakyartha-adoption.md` Phases A–B are live: at session admission an
ordered candidate ladder is computed (primary + warm-discovery fallbacks
only — no invented ids, no network) and frozen INTO the contract header.
Dispatch walks legs top-down on typed failure domains; the first dispatch
of each next leg is receipted `route-fallback` and surfaced as a
`RouteFallback` event. Ceiling, receipts, and endurance budget are shared
across all legs, so walking the ladder is contract execution, never
mid-contract switching (invariant 7 above carries the new wording).

Phase R (vakrouter adoption) upgrades the ordering machinery:

- **Attribution is real**: every attempt records the `(provider, model)`
  leg that actually served or failed; evidence rows land keyed correctly
  in `routing-evidence.jsonl` with true p50 latency.
- **Demand-scored objectives** (`order_ladder_v2`): request difficulty
  picks utility/balanced/quality-critical ordering; `[route].objective`
  overrides; `[route].quality_hints` replaces hardcoded model-name bands
  (invariant 9). v1 stays only for replaying old contracts.
- **Cross-model fallbacks are opt-in**: `[route].fallback_models` allowlist
  ∩ warm discovery; the user's primary never loses the head position.
- **Diversity caps + annotations**: ⌈max_total/3⌉ seats per provider;
  thin-chain/dominant-domain/unreachable warnings frozen into the header,
  visible in TUI introspection.
- **Beliefs**: domain-weighted doubt demotes flaky legs below trusted
  peers until one success clears them; governance failures are not
  evidence.

Evidence rows land in `routing-evidence.jsonl`; unknown settlements shrink
confidence without punishing direction. FinOps attribution follows the
serving leg per dispatch (`CostRow.provider`).

## Invariants

- Retry never changes the frozen contract: same model, same request body.
- Retries are unbounded by wall-clock but bounded by count and cancel token.
- A denied/failed tool result is data, not an exception — the loop continues;
  only provider-step exhaustion or required-node failure ends a run.
