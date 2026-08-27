# 23 — Memory and search

Memory is Runtime-owned data, not a client-side note system. `vak-services`
stores memory records in `state.db`; `vak-runtime` applies scope, project, and
deletion rules; every client uses the same CRUD endpoints.

## Scopes and layout

Each record has an ID, scope (`workspace` or `profile`), optional project ID,
kind, tag, text, timestamps, and a soft-deletion timestamp. The authoritative
record is in SQLite and its mutation is audited. Document indexes are separate
projections and may be rebuilt without changing memory.

## Model-visible recall

The Runtime injects selected memory into an admitted run's context. The selected
record IDs and query are recorded in the session ledger, so a provider request
can be reconstructed. The current session is excluded from recall while it is
being written to prevent self-referential results.

## CRUD

CLI, TUI, desktop, and admin use `GET /memory`, `POST /memory`, `PATCH
/memory/:id`, and `DELETE /memory/:id`. Delete is a durable tombstone; it does
not rewrite prior transcripts. Scope and project ownership are validated by
Runtime.

## Safety and verification

Memory text is data, never executable instruction. It cannot grant permission,
change the provider contract, alter credentials, or bypass the broker. Tests
verify scope isolation, deletion, ledger logging, and consistent results across
all clients.
