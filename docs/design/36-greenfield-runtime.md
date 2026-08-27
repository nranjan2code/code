# 36 — Greenfield runtime ownership

This document defines the current architecture. The implementation has one
runtime, one on-disk layout, and one client protocol.

## Ownership

`vak-runtime` is the sole mutable application authority. HTTP, CLI, TUI,
desktop, admin, flows, and evals are adapters that submit
typed commands and consume typed queries/events. They do not open state files,
construct agents, or spawn effectful processes directly. `vak-domain` defines
the shared IDs and contracts; `vak-services` implements CRUD and audit; and
`vak-storage` persists the state.

`vak-storage` owns transactional control state, append-only operational audit,
session-ledger access, and content-addressed blobs. `vak-agent` is an execution
engine over an immutable `RunContext`; it does not select global cwd or load
mutable application configuration. `vak-server` and `vak-client` are the
authenticated HTTP/SSE adapter pair.

## Identity and context

Projects, sessions, runs, tasks, approvals, deliveries, and capabilities have
stable IDs. A session references a registered canonical project root. Every run
receives a complete immutable context containing project root, frozen session
contract, cancellation scope, workspace lease, and capability epoch.

No execution decision may consult process cwd or a global Runtime cwd.

## Run and revocation contract

Cancellation is owned by a live run, never by an idle session handle. A cancel
request marks the run cancelling, propagates through the provider and brokered
tools, waits for bounded teardown, persists one terminal outcome, and emits
one terminal event. Cancelling an idle session is an idempotent no-op.

Permission changes stop admission, advance the capability epoch, cancel and
join old-epoch work, reject old approvals, persist the new mode, then reopen
admission. The effect broker rejects stale-epoch dispatches.

## Storage layout

```text
data_home/
├── config.toml
├── .env
├── state.db
├── sessions/<project-id>/<session-id>.jsonl
├── blobs/<sha256>
├── audit/operations.jsonl
├── runtime/gateway.json
├── locks/runtime.lock
└── logs/

cache_home/
└── search.db
```

Session JSONL remains append-only and is authoritative for model-visible
history. SQLite is the transactional projection for operational state. The
search database is disposable. Checkpoints reference blobs through manifests;
unchanged file content is deduplicated.

## Effect boundary

All effectful operations pass through the broker and workspace lease manager.
This includes built-in tools, scripts, filesystem APIs, flows, evals, and
checkpoint restore. The broker evaluates permission
and capability epoch immediately before dispatch.

## Cutover

This build has one greenfield contract: callers use `vak-runtime` through its
typed server/client adapters. No surface may create an alternate authority,
read an unregistered state path, or speak an unversioned protocol.
