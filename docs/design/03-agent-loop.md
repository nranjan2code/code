# 03 — Agent loop (vak-agent)
Status: implemented in 2.0.0

## Shape

```
run(prompt, cancel, events) -> TurnOutcome
  append user message
  loop turn in 0..max_turns:
    drain steering queue → user messages
    request = { model: contract.model, system, messages: session.derive_messages(), tools }
    stream = provider.stream(request, cancel)
    forward StreamEvents (delta+snapshot) to consumer
    response = stream.result()          // errors are values
    append assistant message (+meta)
    if stop_reason != ToolUse -> Completed
    execute tool batch (parallel by default, source-order results)
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
- **Projection invariant**: requests are built only from `derive_messages()`.

## Stop gate (built-in premature-completion policy)

Small models frequently end their turn mid-plan. The loop consults an
internal `StopPolicy` (vak-agent) at every completion point, BEFORE
returning `Completed`:

- **marker gate**: final text ends with a bare plan marker, a non-heading
  trailing `:` line, or an unclosed fenced code block → looks truncated;
- **verify gate**: the run's initial prompt demanded executed verification
  ("must pass", "run it", "prove that"…) and ZERO bash commands ran all run.

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

## Later phases

All three former items shipped (permission gate, `task` subagents, resource-
claim scheduler) — see 00-roadmap.md and 08-permissions.md. Remaining open
ideas: stop-gate extension to catch fabricated verification (currently
omission-only), per-subagent token attribution in the ledger itself.
