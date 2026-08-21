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

## Later phases

- permission gate between extract_tool_calls and execute_batch (Phase 3)
- subagent `task` tool spawning child Agents with narrowed contracts (Phase 5)
- resource-claim scheduler for parallel fan-out (Phase 6)
