# 03 — Agent engine

`vak-agent` is a stateless turn engine. `vak-runtime` admits the run and
supplies an immutable `RunContext`, provider adapter, tool dispatcher,
recorder, cancellation token, and capability guard.

## Turn

```text
Runtime admission → provider response → brokered tool waves →
record tool outputs → provider continuation → terminal outcome
```

The engine cannot load configuration, choose a project root, read the data
home, or bypass the broker. Every provider/tool call checks cancellation and
the capability epoch. Tool results are values with an error flag; malformed
requests become typed failures rather than process panics.

## Streaming and cancellation

Each visible event carries a delta and complete snapshot. Cancellation is
propagated through the provider and brokered tools. The engine
returns the partial response and a terminal cancelled outcome after bounded
teardown. A second cancellation is idempotent.

## Admission contract

The Runtime freezes provider/model route ladder, permission, sandbox, budget,
system prompt, and tool catalogue in the session contract. Retries stay inside
that frozen contract. The engine never switches providers or permissions on its
own.
