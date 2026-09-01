# 03 — Agent loop (vak-agent)

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
  never kills the run.
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

## Later phases

All three former items shipped (permission gate, `task` subagents, resource-
claim scheduler) — see 00-roadmap.md and 08-permissions.md. Remaining open
ideas: stop-gate extension to catch fabricated verification (currently
omission-only), per-subagent token attribution in the ledger itself.
