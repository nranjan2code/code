# 03 — Agent loop (vak-agent)
Status: implemented in 2.0.0; request assembly and completion gates extended
in 3.5.0 (docs/design/68-context-engine.md)

## Shape

```
run_message(prompt, steering, cancel, events) -> TurnOutcome
  outcome = run_message_inner(...)      // the loop below
  if outcome is a system-authored Completed (drift/stale-data/card-repeat
  exhaustion) and not already the ledger's last message: log it
  close_turn(outcome)                   // builds and appends this turn's
                                         // TurnCard once it has closed
  return outcome

run_message_inner(prompt, steering, cancel, events) -> TurnOutcome
  append user message
  loop turn in 0..max_turns:
    drain steering queue → user messages
    index = TurnIndex::from_log(session); index.ensure_cards(estimator)
    plan  = planner::plan(profile, index, directive, reading, prefix_tokens,
                           tail_tokens, current_turn_tokens)
    messages = session.derive_with_plan(plan)   // Full/Card/Packet per turn
    tail  = compose_tail(temporal, stance, intent, work_contract, thread)
    attach_tail(messages, tail, directive_index)  // one block, before the
                                         // user's own words, on the turn's
                                         // directive message; plan/tail/
                                         // tools are resolved once per turn
                                         // and reused every step
    request = { model, system: prefix, messages, tools: core+deferred/index,
                cache: CacheHints{ session_key, breakpoints } }
    stream = provider.stream(request, cancel)
    forward StreamEvents (delta+snapshot) to consumer
    response = stream.result()          // errors are values
    record_capacity_usage_feedback(request, usage, first_token_latency)
    append assistant message (+meta)
    calls = extract_tool_calls(response)  // structured tool_use blocks, plus
                                           // a text fallback for an explicit
                                           // <tool_call>/```tool_call/
                                           // ```tool_use envelope naming a
                                           // loaded tool -- never a bare
                                           // prose code fence
    if model drift detected: steer, maybe end the turn (see below)
    if calls.is_empty():
      if response has neither text nor a tool call: one [empty-step] redo
      if directive wants live data and nothing retrieved: gate/redo/fail-closed
      if stop_reason != ToolUse -> Completed
    gate any emit_*_card call against the same freshness rule, before execution
    execute tool batch (parallel by default, source-order results)
    breaker: N consecutive all-repeat card batches -> close on that card
    append user message of ToolResult blocks
```

`TurnOutcome = Completed | Aborted{partial} | Failed{error} | MaxTurnsReached`.
No panics; session-write failures become `Failed`.

## Principles

- **Errors are values** end to end: unknown tools and tool panics become
  `is_error` ToolResults the model can self-correct from; a single tool failure
  is not an automatic blind replay. For correctable failures (malformed
  arguments, unknown capability names, schema mismatches, and recoverable
  tool/MCP transport faults), the result includes a logged recovery contract:
  inspect the error and admitted inventory, issue at most one corrected or
  alternative call, and do not repeat identical arguments. Cancellation,
  permission/approval denial, revocation, authentication, and rate limiting
  do not receive recovery advice. The corrected call still passes the normal
  authorization, sandbox, hook, doom-loop, turn, and dispatch-ceiling checks.
  When the model fails to repair a correctable fault across consecutive turns,
  the loop escalates past the hint into a system-authored, schema-resurfacing
  directive and finally a bounded stop that degrades the outcome (see
  `docs/design/15-reliability.md`). Never kills the run.
- **Parallel by default**, results re-ordered into assistant source order.
- **Steering + follow-up are two queues**: steering drains before the next
  model call; follow-up is exposed for callers after natural stops.
- **Cancellation threads everywhere**: checked between turns, before each
  sequential tool, inside every tool via child tokens.
- **Projection invariant**: requests are built only from `derive_messages()`
  / `derive_with_plan()` (docs/design/02-sessions.md invariant 3).

## Completion gates added in 3.5.0

Five bounded, system-authored checks run inside the loop, each with its own
`ControlKind` marker so the nudge is visible on the ledger
(docs/design/68-context-engine.md §7, §10):

- **`[freshness-check]`**: the directive's reading carries the `live-data`
  domain (temporal deixis — "current", "right now", "today") and nothing was
  retrieved this run. A card call is intercepted *before execution* and
  answered with an error value; a final text answer gets one redo nudge. An
  explicit "no live data" is accepted. After one repair still nothing
  retrieved (a second stale card, or another empty step), the turn **fails
  closed**: `stale_data_outcome()` states plainly that nothing current was
  retrieved and names the last figure this conversation recorded and when —
  never a carried-over figure presented as current.
- **`[empty-step]`**: the response carried neither text nor a tool call (a
  thinking-only completion). One redo, unless a card was already emitted
  this run — a card-only turn is a complete answer.
- **`[steering-drift]`**: a step ends with no tool call and its final text
  repeats a prior turn's `TurnCard` narration verbatim instead of addressing
  the current directive — domain no longer enters the check. The nudge
  points at the user's latest message and never quotes it — an echo reads as
  the user asking again. Three consecutive events end the turn via
  `degraded_drift_outcome()`, the same shape as the tool-repair exhaustion
  below.
- **Identical-card repeat breaker**: an `emit_*_card` call identical to one
  already shown this run is acknowledged as a no-op (not an error). After
  three consecutive all-repeat batches, `card_repeat_outcome()` closes the
  turn on that card as its answer instead of spending the turn budget on
  acknowledgements.
- **`[grounding-check]`**: the step right after a retrieval gave an answer
  that shows no sign of using it. "Uses" is judged from the result, not the
  answer's format: the answer repeats a host or a figure the retrieval
  returned, or says plainly that it found nothing (`admits_no_data`, the one
  phrase list the freshness check shares). A card emitted alongside the
  retrieval clears the check, and a result with nothing checkable in it never
  triggers one. It used to pass only answers containing a `vak` fence, which
  sent correct cited prose back for a redo.
- The `[fence-check]`, `[duplicate-card-check]` and `[presentation-check]`
  nudges are unchanged, except that a presentation check loads the card tool
  it names for the redo; all `ControlKind` variants are enumerated in
  `vak-intent/src/control.rs`.

Every one of these was found by replaying a real failing session against a
live local model (`gemma4:e2b-mlx`) six times per fix and reading the
resulting ledgers — see docs/design/68-context-engine.md for the measured
before/after tables.

## Stop gate (built-in premature-completion policy)

Small models frequently end their turn mid-plan. The loop consults an
internal `StopPolicy` (vak-agent) at every completion point, BEFORE
returning `Completed`:

- **marker gate**: final text ends with a bare plan marker, a non-heading
  trailing `:` line, or an unclosed fenced code block → looks truncated;
- **verify gate**: the run's initial prompt demanded executed verification
  ("must pass", "run it", "prove that"…) and ZERO bash commands ran all run;
  it also fires whenever the resolved outcome's `requires_execution()` is
  true (an effectful act, `Verify`, or a request that names a file
  deliverable — `Author` alone never demands a receipt) and no execution
  receipt exists, independent of the keyword heuristic.

On a hit, the gate reuses the stop-hook continuation machinery: emits
`StopHookContinuation`, appends `[stop-guard]: <reason> / Please continue.`
as a user message (model-visible ⇒ logged), and continues within max_turns.
Hard cap `stop_policy.max_blocks` (default 2) per run — the gate can nudge,
never trap. Config (`[stop_policy]`): `enabled/marker_gate/verify_gate/
max_blocks`; unknown keys warn, `enabled = false` restores old behavior.
External Stop hooks still run first and keep their own `[stop-hook]`
prefix, so operator logs can tell them apart.

An explicit continuation request such as “keep improving until I say done”
uses a separate user-completion gate. It continues across ordinary model
completion claims without consuming the diagnostic `max_blocks` budget, until
an exact user steering message such as `done` or `stop` releases it. The
configured `max_turns` remains a hard safety ceiling; reaching it returns
`MaxTurnsReached`, never `Completed`.

## Where completion authority actually lives

The stop gate above guards *this turn*. It is a heuristic over the
transcript, and it was for a long time the only thing standing between "the
model stopped talking" and "the work is done" — which is why it needed
marker and verify sub-gates at all.

With the intent kernel (`docs/design/47-commitment-kernel.md`) that is no
longer the authoritative signal for durable work. A commitment closes only
when the **runtime** has evaluated its criteria against the world at the
strength its `evidence` axis demands; the model may propose criteria and may
never mark one passed. The stop gate's job narrows accordingly, to "did this
episode terminate cleanly" rather than "is the objective met".

The turn's reading also supplies a `stop` profile — `message`, `inspection`,
`effect`, or `verification`. An act that changes something and produced no
effect did not finish, whatever the final message says.

## Workers and their cards

A `task` worker runs its own loop and ledger. When it ends, `task` returns its
final text and, beside it, every card the worker showed
(`ToolOutput::delegated`). The parent loop records each card as a
presentation of the `task` call (`PresentationSource::Delegated`, carrying the
worker's session id) and appends the list to the call's result — `pres:<id>`,
type and title — so the user sees the cards in the parent conversation and
the parent can `recall` one to review or fix it. A worker that builds, reviews
or fixes cards therefore needs no text re-encoding of them. The worker's text
is a tool result like any other: recorded whole and windowed in the request
when over-long (docs/design/68-context-engine.md §3).

## Later phases

All three former items shipped (permission gate, `task` workers, resource-
claim scheduler) — see 00-roadmap.md and 08-permissions.md. Remaining open
ideas: stop-gate extension to catch fabricated verification (currently
omission-only), per-worker token attribution in the ledger itself.
