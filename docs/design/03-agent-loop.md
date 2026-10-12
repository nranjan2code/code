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
  authorization, sandbox, hook, repeated-call, turn, and dispatch-ceiling checks.
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

- **`[empty-step]`**: the response carried neither text nor a tool call (a
  thinking-only completion). One redo, unless a card was already emitted
  this run — a card-only turn is a complete answer. A provider can drop a
  call whose arguments do not parse (Ollama's gemma4 parser logs "tool call
  parsing failed" and returns nothing), so the nudge says a call may have
  been lost and asks for the same call again with valid JSON, rather than
  telling the model it made none — told that, it planned afresh and once
  fetched a web page instead of re-sending the card it had written. The
  finding carries two observed facts (`Finding::EmptyStep { thought, done }`):
  when thinking arrived, the nudge says the thinking reached no one, and
  asks for the answer as text or the call it planned; when a call already
  succeeded this turn, it says that result stands and is not made again.
  Found live 2026-10-13: told first to "make the same call again", a model
  whose answer sat in its thinking repeated a finished `write` to the step
  limit.
- **`[steering-drift]`**: a step ends with no tool call and its final text
  repeats a prior turn's `TurnCard` narration verbatim instead of addressing
  the current directive — domain no longer enters the check. The nudge
  points at the user's latest message and never quotes it — an echo reads as
  the user asking again. It is sent once and the answer redone; an answer
  that drifts again straight after it stands as the model's own, with a
  `model-drift` diagnostic. No count of events ends a turn.
- **Identical-card repeat breaker**: an `emit_*_card` call identical to one
  already shown this run is acknowledged as a no-op (not an error). A step
  that only repeats shown cards again, straight after that acknowledgement,
  closes the turn on that card as its answer (`card_repeat_outcome()`).
- **Removed at control-plane C5** (docs/design/88-control-plane.md §5):
  `[freshness-check]`, `[grounding-check]` and `[topic-mismatch]`, which read
  the request's or the answer's words, and the runtime endings that replaced
  the model's answer with a statement of its own. Every remaining correction
  is a typed `Finding` rendered by `vak_control::render` and paced by the one
  `Ladder`: told once with the evidence, then the frame changes (a tool is
  withdrawn) or the answer stands.

Every one of these was found by replaying a real failing session against a
live local model (`gemma4:e2b-mlx`) six times per fix and reading the
resulting ledgers — see docs/design/68-context-engine.md for the measured
before/after tables.

## Stop gate (built-in premature-completion policy)

Small models frequently end their turn mid-plan. The loop consults an
internal `StopPolicy` (vak-agent) at every completion point, BEFORE
returning `Completed`:

- **marker gate**: for work whose reading needs a tool, final text ends
  with a bare plan marker, a non-heading trailing `:` line, or an unclosed
  fenced code block → looks truncated. A letter or an agenda is never
  checked: its last line is the deliverable;
- **verify gate**: what completion needs comes from the admitted
  `OutcomeSpec`, never from phrases in the request. A `verification` stop
  profile needs an execution receipt, and changed code must have been run;
  an effectful act or a named file deliverable needs an execution receipt;
  a `locate`/`verify` act needs an inspection or tool receipt unless the
  answer is substantive (the material came with the request); and a check
  that ran before code changed must run again. The phrase lists this
  replaced read "run a quick grammar check" as a demand for a shell command
  and only spoke English;
- **unresolved failure**: a call failed and no later call of its tool
  succeeded. It is recorded as a decision and the answer stands: whether an
  answer owns up to a failure is not something its words can establish, and
  the turn's outcome carries the failure as a typed fact. (Until
  control-plane C5 the answer was sent back unless it quoted the error.)

On a hit, each reason goes through the correction ladder
(`vak_control::Ladder`, docs/design/88-control-plane.md §5): told once as
`[stop-guard]: <reason>` (a typed `Finding` rendered by
`vak_control::render`; model-visible ⇒ logged), with `StopHookContinuation`
emitted, and continued within max_turns. An answer that still leaves the
reason standing is the model's own and ends the turn, with a
`Correct { rung: end }` decision saying so. No count bounds the gate.
Config (`[stop_policy]`): `enabled/marker_gate/verify_gate`; unknown keys
warn, `enabled = false` turns the gate off.
External Stop hooks still run first and keep their own `[stop-hook]`
prefix, so operator logs can tell them apart; a hook's reason is its own
text, so its message adds the instruction and the way out itself.

The explicit `/until-done …` command (`vak_intent::Command::UntilDone`)
uses a separate user-completion gate. It is the only way to hold completion
for the person: control comes from commands, never from phrases in free
text (invariant 32), so "keep working until the tests pass" names a
condition and is not a hold. It continues across ordinary model
completion claims, told each time, until
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
