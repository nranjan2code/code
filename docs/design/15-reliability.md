# 15 — Reliability: QoS, crash, recovery, start/stop/resume
Status: implemented in 2.0.0

## Tool and MCP error recovery

Tool and MCP failures are append-only, model-visible values rather than
terminal exceptions. When a failure is plausibly correctable, Vak appends a
recovery hint to that result so the next LLM turn can inspect the failure and
retry with corrected arguments or choose an alternative capability. This is a
bounded repair loop, not blind replay: the model must produce the new call,
identical arguments must not be repeated, and every new call is authorized and
executed through the normal broker.

Recovery hints are deliberately omitted for user cancellation, permission or
approval denial, revocation, authentication, and rate-limit failures. Those
conditions require an external state change or human decision. Existing turn
limits, doom-loop detection, provider dispatch ceilings, and append-only
session recording remain the upper bounds for the repair loop.

### Per-run repair budget (escalating, system-authored)

A recovery hint alone is a nudge to the model; it is not a backstop. The loop
tracks consecutive turns that end with an unresolved classifiable
(`vak_tools::ToolErrorKind::is_correctable()`) tool failure and escalates:

1. **First** correctable failure this run — the per-call `[recovery]` hint is
   appended to the result (as above; the model still produces the next call).
2. **Second consecutive** turn failing on a correctable fault — the loop
   injects an authoritative, schema-resurfacing directive, a tagged
   `ControlKind::RepairDirective` message (`[repair-directive] …`) that no
   client shows as the person's words: it re-surfaces the exact admitted `Tool::schema()`
   for each rejected tool (or, for an unknown name, the full admitted inventory)
   so the repair is no longer a guess. This is issued by the runtime, not the
   model.
3. **Third consecutive** turn still failing — the budget is exhausted: the loop
   stops re-dispatching and returns a degraded, system-authored
   `TurnOutcome::Completed` that states the failure instead of inventing an
   answer. A `Diagnostic` activity is appended to the ledger
   (`tool-repair-exhausted`).

The budget is per `Agent` run (`RepairState`, reset on every `run`) and is
general across all tool-error classes — it does not special-case the `mcp`
broker; `ToolErrorKind::classify` is the single classification authority.

### Outcome linkage (no false `Produced`)

The outcome gate (`vak_core` → `vak_intent::evaluate_response_with_failures`)
inspects the latest turn: when it ends with classifiable correctable failures
and zero successful receipts, the run is downgraded to `Unknown` rather than
`Produced` — a non-empty fallback answer is not evidence. Combined with the
budget above, a run that cannot repair its tool calls fails honest instead of
signing off a fabricated result.

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
- **Truncated SSE streams on OpenAI-compatible proxies**: some free-tier endpoints
  end the body after the last content chunk without
  `[DONE]`/`finish_reason`. The openai-completions adapter treats a clean
  close *with content* as de facto completion (`EndTurn`); a close with no
  content still fails closed as `Parse`.
- **Responses stream settlement**: `response.completed` and
  `response.incomplete` settle the step immediately; the adapter does not wait
  for the HTTP body to close after a terminal response. Streamed
  `response.failed` and `error` events preserve the provider's error code and
  message as a typed error, so context rejections can replan, overload and
  rate limits can retry, and exhausted quota is terminal. A body that closes
  without a terminal event still fails closed, with the last event type in
  the diagnostic (no response content).
- **Artifact completion**: a successful tool that delivers a reviewable file
  satisfies the empty-answer check, just as a displayed card does. When the
  model ends the next step without prose, the loop accepts that completion
  through the remaining outcome and safety gates instead of requesting more
  work solely to repeat the delivered file in text.

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
from a dead provider. Shared via Core across all runs of a process, with one
state machine per provider endpoint and credential fingerprint:

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
- A turn's ladder checks each leg's circuit independently, so an open primary
  fails fast while a healthy fallback can still serve. Run-level endurance
  uses normal cancel-aware backoff; each leg check prevents no-op calls until
  that leg's cooldown expires.

Config keys: `circuit_breaker_threshold`,
`circuit_breaker_cooldown_secs` (`0` cooldown disables opening).

## Route ladder history

The following describes the original session-admission design. The shipped
per-turn behavior is specified in [Turn-Level Routing](#turn-level-routing)
below and implemented in `vak-core/src/lib.rs` by `run_turn_inner`.

In the original design, session admission computed an ordered candidate
ladder (primary + warm-discovery fallbacks only — no invented ids, no network)
and froze it into the contract header. The current header keeps this as an
audit snapshot. Dispatch walks the per-turn ladder on typed failure domains.
The first dispatch of each next leg is receipted `route-fallback` and surfaced as a
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
  The difficulty facts themselves come from the intent kernel
  (`docs/design/47-commitment-kernel.md`): `reasoning_required`,
  `evidence_required` and `structured_output` are derived from the turn's
  reading. Until that landed, `plan_route_ladder` passed all three as
  `false` with `estimated_input_tokens: 0`, so every session scored
  identical demand and this ordering function never actually varied.
  A turn may also restrict its planned ladder to a **prefix** — never
  reordering or extending it — which keeps dispatch inside the admitted
  turn route and leaves replay exact.
- **Cross-model fallbacks are opt-in**: `[route].fallback_models` allowlist
  ∩ warm discovery; the user's primary never loses the head position.
- **Same model at other services** (`crates/vak-llm/src/model_identity.rs`,
  `routing::stand_in_legs`): one model has a different id at each service
  (a bare name at its maker, `vendor/name` at an aggregator,
  `region.vendor.name-v1:0` on Bedrock), so an id is evidence of identity
  only within one account: another key for the primary's provider, or a
  provider name sharing its credential fingerprint (one account, two wire
  protocols). Across services a leg stands in for the primary only as a
  member of a confirmed `[route] same_model` group, spelled `provider/model`
  as `[intent] classify_model` is; groups that share a member merge, and every
  layer's groups apply. The admin portal's Backup models panel proposes groups
  from live catalogues (`GET /config/route/suggestions`) by reading how ids
  are spelled, never what a model is (invariant 9), and a proposal is used
  only once a person confirms it, because a wrong match would quietly serve a
  different model. Stand-ins are seated before any cross-model alternate: the
  same model elsewhere is a like-for-like swap, another model changes
  behaviour. `[route]` applies live: every route refresh re-reads it, so a
  confirmation reaches a running session at its next turn.
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

## Network events (docs/design/31-network-resilience.md)

| Event | Handling | Proof |
|---|---|---|
| DHCP change / network switch | local plane loopback-immune; outbound reconnectors own recovery | `telegram_bridge::bridge_survives_outage_window_and_resumes_cursor` |
| Multi-minute outage on a channel | bridge never exits; capped backoff, cursor resumes gap-free via ownership probe | same regression |
| Inference outage window | keyed ladder circuits + endurance ride it; healthy fallback legs remain eligible while a dead leg cools down | fault_proxy scenario (`scripts/fault_proxy.py`) |
| Hibernation / wake | tokio timers collapse across sleep; watchdog bounds dead sockets; scheduler per-tick evaluation fires each missed slot once | scheduler catch-up tests |
| Full restart | sessions append-only + resume; gateway bindings + task store persisted; telegram cursor re-synced by probe | existing resume/bindings suites |
| Delivery while channel down | inbox chokepoint stores durably; transports best-effort | P6 zero-transports test |

## §Turn-Level Routing

**Status: shipped.**

### Problem with session-frozen ladders

The original Phase R design froze `provider`, `model`, and `route_ladder` into the
`FrozenContract` at session creation (`start_session_with_route_for`). Every subsequent
turn in that session read the frozen values unconditionally, which caused:

- A user changing provider in Settings had zero effect on open sessions.
- The auto-routing algorithm (`plan_route_ladder`: evidence, beliefs, v2 ordering,
  demand-scoring) ran exactly once at admission, then was inert for the session's life.
- If a provider failed mid-session the frozen ladder was always tried first, even if
  the user had switched away from it.
- Workers inherited the parent session's frozen route rather than the current
  effective route at spawn time.

### Design change

**Route planning is now per-turn, not per-session.**

`run_turn_inner` (`vak-core/src/lib.rs`) now:

1. Resolves `(provider, model)` from `Core::effective_route()` — always the live
   operator selection, never the session header's initial snapshot.
2. Calls `Core::plan_route_ladder(turn_primary_leg, Some(engagement.posture.demand))`
   after intent resolution to assemble a fresh ladder using:
   - Current evidence ledger (`routing-evidence.jsonl`, 30-day TTL)
   - Session belief state (domain-weighted doubt, clears on success)
   - Warm discovery: the last successful model catalogue per credential,
     kept fresh in the background (see [How discovery stays
     warm](#how-discovery-stays-warm))
   - Demand facts from this turn's intent reading (not hardcoded constants)
3. Uses the fresh `turn_plan.ladder` to populate `cfg.ladder` (fallback legs),
   not `session_contract.route_ladder`.

### How discovery stays warm

A fallback leg is the primary's model on another keyed credential, or an
exact id from `[route].fallback_models`, and planning admits one only where
discovery shows a configured credential reaches that model. Planning never
talks to a provider (invariants 7 and 25), so discovery has to be warm before
the turn that reads it. It used to be written only when a person opened a
model list, and only on the Core that served the list, so every other Core,
each pooled gateway Core included, and any Core five minutes after its last
picker read planned a primary-only ladder annotated as a single point of
failure.

The mechanism is `vak-core/src/discovery.rs`, level-triggered in the sense of
invariant 31 (docs/design/41-capability-registry.md), in a loop of its own
because a capability pass is offline by construction and this one is not.

- **One store.** `ModelCatalogues` holds the last successful catalogue per
  `(provider, credential_id)`, with the reason and retry time of the latest
  failure beside it. Planning and the model pickers (`Core::discover_models`)
  read and write the same store; a picker still serves a catalogue younger
  than five minutes (`DISCOVERY_TTL`) without asking again.
- **Shared across the pool.** A credential id fingerprints the base URL and
  the key together, so a catalogue does not depend on which workspace asked.
  `CorePool` (`vak-server/src/core_pool.rs`) hands every Core it builds the
  default Core's store, and a channel Core built for its first message in an
  hour plans from what the pool already knows. A Core makes legs only from
  entries for credentials it resolves itself, so sharing never widens what a
  workspace or an Agent can reach.
- **Demand.** Every ladder plan, at session admission and on every turn,
  notes that the turn's Agent is planning on this Core. Agents are tracked
  separately because an Agent-private secret scope can hold keys of its own.
- **The loop.** `Core::start_model_discovery` starts one refresh loop per
  Core. The server starts it for its own Core and the pool for every Core it
  builds, from `secured_router_with_port`, so `vak serve` and the desktop
  shell behave alike. A pass is idempotent: for each Agent that planned a turn
  in the last 30 minutes (`ACTIVE_WINDOW`) it resolves every keyed provider's
  credential pool and fetches, concurrently, each catalogue that is missing or
  older than four minutes (`REFRESH_AFTER`), recording each as it lands. The
  loop ticks every 30 seconds (`RECONCILE_INTERVAL`), so a renewal lands
  before the five-minute TTL. Hints, from the first turn after a quiet spell
  and from a key change, only make it run sooner; a lost hint costs one tick.
- **A failure is a reason with a retry, never data.** A failed listing never
  replaces or empties a catalogue. Its reason is recorded beside it and it is
  retried with capped backoff, 15 seconds doubling to five minutes, without
  end. A picker asks straight away whatever the backoff, and reports the
  reason rather than a list. The loop logs the first failure of a streak and
  the recovery, and `/doctor` shows each credential's state in its "route
  discovery" fact.
- **Planning keeps the last good catalogue for 24 hours**
  (`PLAN_STALE_BOUND`). A model withdrawn meanwhile costs one fast failed
  dispatch on a fallback leg, reached only after the primary has failed, and
  the walk moves on; dropping the catalogue would cost the whole fallback.
  Nothing ever substitutes a static list (invariant 9).
- **A quiet Core stops asking.** Past the active window its loop keeps
  ticking but asks no provider, so an idle or evicted pooled Core does not
  poll forever. Its next turn plans from the last catalogue and wakes the
  loop. The loop holds its Core only for the length of a pass, so dropping
  the Core ends the loop.

What this does not cover: a process with no loop, such as a one-shot
`vak exec`, plans only from what it discovered itself, which for a fresh
process is nothing, so its ladder is primary-only unless a picker ran in it.
The first turn after a server starts can race the first pass the same way.
Catalogues are not persisted across processes, and closing that gap would
need them to be.

### What the FrozenContract still governs (immutable)

| Field | Still frozen | Reason |
|---|---|---|
| `capabilities` | Yes | Security/audit boundary — which tools are admitted |
| `permission_mode` | Yes | Security boundary — cannot escalate permissions mid-session |
| `system_prompt` / `prompt_layers` | Yes | Drift detection and prompt attribution |
| `app_version` | Yes | Audit context |

### What changed to initial snapshot only

| Field | Was | Now |
|---|---|---|
| `provider` | Dispatch authority | Admission snapshot for audit |
| `model` | Dispatch authority | Admission snapshot for audit |
| `route_ladder` | Dispatch authority | Admission snapshot for audit |

### Audit preservation

`WorkReceipt` records the exact provider, model, and attempt outcome for every
dispatch, per turn. Per-turn audit is strictly richer than the previous
session-level frozen record — you can now reconstruct which provider handled
turn 3 even if the user switched providers between turn 2 and turn 3.

### Affected call sites

- `vak-core/src/lib.rs` — `run_turn_inner`: provider/model resolution + ladder assembly
- `vak-agent/src/lib.rs` — `run_turn`, `run_judge`, `write_handoff`, managed-work
  authoring: all read `self.config.model` (set per-turn by `run_turn_inner`)
- `vak-server/src/lib.rs` — `configuration_mismatch` field: always `false` (concept retired)
- `vak-server/src/admin.rs` — `stale_reasons`: `provider_changed`/`model_changed` removed
- `vak-server/src/gateway.rs` — `session_matches_route`: no longer checks initial contract
- `vak-server/src/agent_chats.rs` — candidate sessions no longer filtered by initial route
- `vak-session/src/types.rs` — `FrozenContract` doc comments updated
- `AGENTS.md` — invariant 7 updated
