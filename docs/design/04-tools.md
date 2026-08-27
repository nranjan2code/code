# 04 — Effect broker and tools

Tools are effects requested by `vak-agent` and executed by the Runtime broker.
The broker is the only path to filesystem, shell, web, and other external
effects.

## Dispatch contract

Before dispatch the broker validates the tool name, project-root confinement,
permission mode, sandbox, capability epoch, resource claims, and cancellation
token. It then runs the versioned worker protocol and records the request and
result in the session ledger and operation audit.

## Built-ins

The built-in catalogue covers read, write, edit, glob, grep, bash, web fetch,
and headless browse. Workers receive bounded operational environments. Provider
credentials and gateway tokens are injected only into their intended recipient.

## Failure behavior

Unknown tools, invalid arguments, denied permissions, missing containment, worker
failure, and cancellation return typed error values. They never become an
implicit allow and never panic the Runtime. Partial output is retained when a
running effect is cancelled.
