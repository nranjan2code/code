# 17 — Context construction

Context is built by `vak-runtime` at run admission and passed to `vak-agent` as
part of the immutable `RunContext`.

## Packet

The packet contains the frozen session contract, system prompt, project/session
identity, selected memory IDs, prior ledger messages, tool catalogue revision,
route ladder, budget, permission, sandbox, and capability epoch. The selection
decision is recorded in the session ledger before the provider request.

## Compaction

Compaction appends a ledger entry describing retained and dropped partitions. It
never deletes prior entries. The next provider request is derived from the
compaction entry and the retained packet, so model-visible history remains
reconstructable.

## Limits and errors

Runtime rejects an over-limit packet with a typed error and preserves the prior
ledger. It never silently drops tool/result pairs, approval context, or user
messages. Cancellation during construction preserves already-recorded entries.

## Verification

Tests cover packet accounting, repeated compaction, tool-pair boundaries,
memory exclusion, prompt visibility, and reconstruction through
`derive_messages()`.
