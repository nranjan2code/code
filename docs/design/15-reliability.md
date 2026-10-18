# 15 — Reliability: QoS, crash, recovery, start/stop/resume
Status: implemented in 2.0.0

## Required assistant recording and compaction failures

An assistant message must be appended successfully before any tool call
from that message executes. A failed append settles the turn as failed;
streamed output already delivered remains available to consumers.
System-authored completions also require their assistant append before
reporting success. Diagnostics remain separate best-effort records.
Steering messages must be recorded before reaching the model or releasing
user-controlled completion. Live outcome revisions are recorded before the
runtime publishes their changed contract; an append failure preserves the
previous runtime contract. Abort preserves partial output even when its
append fails and adds a visible persistence-failure marker. Failed fence-card
appends refuse successful delivery. A failed child terminal record returns an
error with partial output and withholds its cards. Call effects must also be recorded before the next model request or delegated
card import; a failure stops the turn. Account attribution is registered before
effect queues are drained so both result windows and complete evidence bodies
use the account scope, including provider error results. Other causal writes
remain under the hardening plan's A03 audit.

Incremental compaction drops its session lock before recording a failed
write diagnostic, since activity recording takes that same lock. A write
failure cannot hold the turn waiting on its own lock (hardening A03/A06).

## Verified budget admission

Budget admission reads a verified cost rollup and reserves through a fenced
shared Document CAS. Day totals and unresolved reservations survive restart
and coordinate independent gates. The existing `max_run_usd` policy remains
session lifetime across turns and delegated children; the cached gate binds
its owning session and each admission captures that owner immutably. Actual
child session and trace stay unchanged in the cost row. Recreating the gate
does not reset settled spend. This does not introduce a RunId-tree call cap.

A settlement appends its cost row before removing the reservation. An exact
reservation id correlates appended rows with surviving pending reservations,
counting the larger estimate or actual once after a crash. Duplicate settlement
ids, malformed history, unknown prices and unattributed legacy liabilities
refuse admission when a cap cannot be verified. Old rollup cache shapes rebuild
from the verified ledger. Unknown settlements also survive UTC rollover for
the session cap. Old delegated rows without budget ownership conservatively
block session-cap admission; no attribution is invented.

Approving a cap increase still requires successful second admission; day caps
and history errors remain enforced. The existing process-local waiver lifetime
is unchanged. Predispatch prices are estimates, so actual cost can exceed a
reservation before settlement. If both the cost append and reservation update
fail, the larger/unknown actual cannot be promised recoverable after a crash.
Unmatched abandoned reservations are retained; no invented expiry clears them.
Read-only financial summaries remain tolerant. The wider hardening plan stays
open for owner reconciliation and an explicit aggregate model-call policy.

## Circuit probe ownership

A provider leg owns its circuit admission permit until it settles or exits.
Dropping an unsettled half-open permit releases its reservation without
counting a user abort as a provider failure. The expired cooldown marker
remains, so another caller can probe. Reservations carry monotonically
issued tokens: dropping or settling an old probe cannot release or change
a newer probe. Normal success/failure and same-leg retry accounting retain
their existing semantics. The real Agent abort path and stale/concurrent
permit cases are covered by circuit tests (hardening finding A05).

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
limits, the repeated-call rule, provider dispatch ceilings, and append-only
session recording remain the upper bounds for the repair loop.

A registered capability called under the wrong kind is a correctable routing
error. Its typed error carries the proper invocation (for example, a skill
name called as a tool carries the `skill` loader call); recovery surfaces that
exact invocation before the staged repair below applies. It does not
authorize a capability that was not admitted.

When a local inference provider accepts a tool-bearing request but rejects
the generated call envelope before it reaches the broker, whether in an HTTP
error or an in-band streaming error, the provider adapter classifies it as a
generated tool-call format failure. The agent records the failure receipt and
adds one system-authored repair message to the same conversation, asking the
model to regenerate valid arguments from the admitted schemas. This is a new
model response, not a blind replay of the malformed output. No tool effect has
run at that point. A second format failure settles normally as a failed run;
it cannot spin indefinitely.

### Staged repair (system-authored, no counter)

A recovery hint alone is a nudge to the model; it is not a backstop. The loop
moves a tool whose calls keep failing with a fault of shape
(`vak_control::Fault::Shape`: not a tool, or not its arguments), and a card
tool whose calls keep failing with a fault of target (`Fault::Target`: a
preview of a file that does not exist, a source nothing retrieved), named
where the call failed and never read from the error's wording
(`vak_judge::call::keeps_failing`, from the tool's own `presents_cards`),
one stage on at each failing step (`reconcile_repair`). There is no repair
count to tune and no turn is ended for it. Card target faults joined on
2026-10-15: before, a card refused for a missing file was told the same
thing at every step and a turn ran until it was stopped. Another tool's
target fault (a page that is gone, text that does not match) stays ordinary
work, since a later call of it may succeed and withdrawing it would take a
working tool out of the turn:

1. **The failed call** carries the fault in its result: a shape fault the
   per-call `[recovery]` hint (as above), a target fault the tool's own
   account of what is missing (the model still produces the next call).
2. **A second failing step** for the same tool, with no call of it having
   succeeded in between: the loop appends a
   tagged `ControlKind::RepairDirective` message (`[repair-directive] …`)
   that no client shows as the person's words. It re-surfaces the exact
   admitted `Tool::schema()` (or, for an unknown name, the admitted
   inventory), so the repair is no longer a guess.
3. **A failure after its schema was shown**: the tool is withdrawn for the
   rest of the turn. It leaves the admitted set and the tools the next
   request offers, a second
   `[repair-directive]` tells the model to deliver that part another way
   (another tool, or plain text) and to keep what it has produced, and a
   `Tool withdrawn` diagnostic is recorded. A later call of it is answered
   as an unknown capability. Nothing already produced is discarded, and the
   turn ends with the model's own answer. Withdrawn tools return at the
   next run. The only bound on a model that keeps calling a withdrawn tool
   is the operator's own turn limit.

A card tool is withdrawn the same way. Once no card tool is left, the plan
check reads a planned card as text, because no card can be shown, and the
outcome evaluation still records that the planned card was not shown.

A tool leaves its stage when a call of it succeeds, not when a step
happens not to call it. The stages are per `Agent` run
(`RepairState`) and general across all tool-error classes; they do not
special-case the `mcp` broker. Found live on 2026-10-11: the earlier
three-strike budget ended a turn with a canned refusal after three failed
card calls and threw away a complete answer.

Found live later that day (dev build of `f0ae9e5aa`, local route): eight
`answer` calls in one turn were refused with the same words and the ladder
never moved. Four causes, all Vak's:

- `answer` is no tool in the admitted set, so withdrawing it by name
  removed nothing, and the stage stayed at "schema shown" for every later
  failure.
- A step that called another tool (`plan`) cleared the stages, so the
  ladder could start again.
- A withdrawn tool was still in the tools each request offered: the turn's
  definitions are frozen at its first step, and withdrawal did not reach
  them.
- The refusal said `blocks` "was sent as text, not a list" and gave a
  column in a 2,700-character line. The model had not sent text: on that
  wire every argument travels as text and the provider reads it, and text
  it cannot read as JSON is passed on as sent. The model's own reasoning
  after the refusal was that it must "send a list, not raw text", and it
  sent the same broken JSON again. The refusal now says where the JSON
  breaks, inside which block and field, and quotes the text on both sides
  (`vak_tools::json_fault`, docs/design/04-tools.md).

After the fix, three live turns with the same request on the same route:
in each the refusals named the place (`expected , or ] in
blocks[1].rows, after … and before …`), the schema was shown at the second
failure, `answer` was withdrawn at the third, and the model wrote the
summary, table and checklist as Markdown. Still open: on this route the
model's `blocks` JSON was malformed in 9 of 11 `answer` calls (a list not
closed, an extra `}`, a `\$` escape), also when told the exact place, so
most turns end in text rather than cards. One slip anywhere in the one
deep argument voided the whole call. Decided on 2026-10-12: each card is
its own call again, whose arguments are shallow
(docs/audits/cards-in-answers-2026-10-11.md).

### Repeated calls (an observed state, no count)

The loop remembers what each call of a run last returned, by tool and exact
arguments (`SeenCall`). A call that runs again and returns the same result
is marked unchanged, and from then on is refused (`Refusal::Repeated`) with
the reason: it would return the same thing. An accepted call that may
change the workspace clears what other calls returned. Until control-plane
C5 two counts did this (three identical calls asked a person; five
identical reads were denied).

### Outcome linkage (no false `Produced`)

The outcome gate (`vak_core` → `vak_judge::outcome::evaluate_response_with_failures`)
inspects the latest turn: when a call failed and no later call of the same
tool succeeded (`vak_judge::call::Failures`), the run is downgraded to
`Unknown` rather than `Produced` — a non-empty fallback answer is not
evidence. Each failed call's fault is a typed record
(`CallEffect::Failed { fault }`), written by the loop from what the tool or
the refusal named. A call whose shape was wrong (not a tool, or not the
tool's arguments) never ran and is not such a failure, nor is a cancelled
one (`vak_judge::call::counts`), nor a card that was not shown: the stop
guard asks the model to report none of them.

The standard failure matrix and where each case is handled.

## Transient failures (QoS)

- **Retry with backoff**: every model step (connect + stream + collect) is
  retried up to `max_retries` (default 3) when the error is retryable
  (429 / 529 / network). Delay = `retry_base_backoff_ms * 2^(n-1)` with ±50%
  jitter, capped at 30s; server-advised `Retry-After` overrides. User aborts
  and partial-output aborts are never retried.
- **Watchdog**: `request_timeout` (default 600s, `request_timeout_secs = 0`
  disables) is how long a step may stay silent: it bounds the wait to
  connect, for each stream event and for the collected result, never the
  whole step, so an answer that is still arriving is never cut
  (docs/design/88-control-plane.md §5a). A silent stream becomes a
  retryable deadline error instead of hanging forever.
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
  work solely to repeat the delivered file in text. Conversely, an authoring
  outcome cannot be marked complete by a prose claim alone: a successful
  result from a tool that declares artifact production must be receipted in
  the same run. Tool declarations are the extension point; the runtime does
  not maintain a domain or file-type list. When that receipt is missing,
  the plan check names the missing deliverable once
  (docs/design/86-model-plans-runtime-checks.md); what is left then stands,
  and the outcome records the file as not delivered rather than accepting
  the prose claim.

## Crash & recovery

- **Session ledgers are append-only record segments** — a crash mid-write
  loses at most the frame being written, and a crash while a segment is
  sealed leaves a readable prefix that the next open finishes or discards
  (`crates/vak-storage/src/segments.rs`); nothing else is rewritten. Model context is always *derived* from the log, so a crashed run
  resumes with full history.
- **Checkpoint ledgers** write via tmp-file + rename (atomic).
- **Flow state ledgers** persist after every node completion; `flow run
  --resume` replays only non-completed nodes.
- **Terminal restore**: `vak term`'s raw mode and mouse capture are
  restored by a Drop guard (`TerminalGuard`) even on panic.

## Start / stop / resume

| operation | path |
|---|---|
| start | `exec`, `serve`, `flow run`, `plan`; `vak term` is a client of `serve` |
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
  in the `routing-evidence` chain with true p50 latency.
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

Evidence rows land in the `routing-evidence` chain; unknown settlements shrink
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
   - Current evidence ledger (the `routing-evidence` chain, 30-day TTL in
     whole UTC days), read through its rollup Document
     (`routing-evidence-rollup`: per day, per leg, counts and up to 512
     latencies) with only the rows appended since folded in, so a turn
     never replays the chain (data-architecture plan M6.3)
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

## Verified charge recovery and required causal records

Admission decisions, framing, authorization and successful WorkReceipts must
persist before dependent work; successful ending/acceptance persistence is
required before reporting a successful turn. Original abort/failure outcomes
remain authoritative if their diagnostic ending append also fails. Card
acceptance decisions written after a card append remain a separate audit gap.

FinOps retains uncertain reservations across restart and UTC rollover. Exact
priced settlements allow interrupted pending-marker cleanup on admission.
The owner-only reservation API and admin panel accept a finite nonnegative
actual USD charge only with explicit provider verification. Corrections use
the original reservation identity and budget owner, append owner-attributed
evidence, and support identical retries; conflicts refuse. An unknown ledger
row with a reservation ID can be corrected even without its pending marker.
Older unknown rows without that ID are reported separately and remain blocked.
The server refuses active local work and durably live traced runs before
recovery. It invalidates known local cached gates after correction; another
process can remain conservatively blocked until its gate is recreated.
No unknown charge expires or is silently dismissed.
