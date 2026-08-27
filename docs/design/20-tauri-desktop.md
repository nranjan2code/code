# 20 — Desktop client

The Tauri 2 application is a native shell around the secured Runtime API. It
does not embed a server, agent loop, shell, session store, or credential store.

## Boundary

```text
SolidJS UI → Tauri commands → vak-client → vak-server → vak-runtime
                                              │
                                      state + broker + audit
```

The desktop receives its Runtime URL and bearer token only through
`vak-client`'s canonical gateway receipt resolver. That resolver verifies the
gateway process plus authenticated `/version` and `/health` before the UI is
marked ready. The shell retains the selected project, not a durable copy of
connection credentials: while the Runtime is unavailable it shows a visible
disconnected state, clears webview credentials, and retries the shared
handshake. A fresh receipt is adopted automatically when the gateway returns.

## Desktop responsibilities

- render sessions, transcripts, run status, deltas, snapshots, and errors;
- submit prompts, cancellation, and approval responses;
- show project-scoped memory, tasks, skills, checkpoints, and diagnostics;
- edit Runtime configuration through revision-checked API requests;
- display Runtime diagnostics, checkpoints, backup/restore, and task status;
- provide native notifications and window lifecycle only.

File reads, writes, command execution, checkpoint restore, and provider calls
are Runtime operations. The desktop cannot bypass the permission engine or
workspace lease. UI state is disposable and is never a source of truth.

## Run presentation

The run view subscribes to the authenticated SSE stream. It renders each event
from its supplied delta and snapshot, preserves partial output after cancel,
and refreshes from the Runtime snapshot after reconnect. A run ID is never
reused for a new request, and a cancel action targets a live run rather than an
idle session.

## Workspace and approvals

The selected project is registered by its canonical path before session
creation. Approval cards show the Runtime request, scope, and expiry; responses
are sent to the Runtime and rejected when the capability epoch is stale. Mode
changes close old approvals and cancel old-epoch work before the UI reports the
new mode.

## Build and test

```bash
cd crates/vak-desktop/ui
npm ci
npm run build
cd ../../..
cargo test -p vak-desktop -p vak-server -p vak-client
```

The desktop test path uses a disposable Runtime and verifies project and
session creation, streaming, cancellation, memory/tasks CRUD, approvals,
configuration, checkpoints, backup/restore, editor access, and reconnect
behavior.
