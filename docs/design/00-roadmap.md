# 00 — Runtime architecture map

This document is the entry point for the current implementation. The project
is greenfield: it has one protocol and one state owner, with no alternate
layout import. The complete contract is described in
[36-greenfield-runtime.md](36-greenfield-runtime.md).

## Authority graph

```text
surfaces: CLI · TUI · desktop · admin
                           │ vak-client / typed commands
                           ▼
                      vak-server
                           │ authenticated adapter
                           ▼
                      vak-runtime
          ┌────────────────┼────────────────┐
          ▼                ▼                ▼
    vak-domain       vak-agent       vak-services
    IDs/contracts    turn engine       CRUD/audit
          │                │                │
          └─────────────── vak-storage ─────┘
                    SQLite · JSONL · blobs
```

`vak-runtime` owns admission, run lifetime, cancellation, capability epochs,
workspace leases, provider selection, and all mutations. `vak-agent` executes
one admitted run using an immutable `RunContext`. `vak-server` is transport
only. A surface never opens a database, reads a session file, or executes a
tool directly.

## Data ownership

| Data | Authority | Mutation |
|---|---|---|
| Projects, sessions, runs, tasks, approvals | `vak-storage` via Runtime | transactional command |
| Model-visible history | session JSONL via `vak-session` | append only |
| File/checkpoint content | `vak-storage` blob store | content addressed |
| Configuration and secrets | `vak-config` via Runtime | revision-checked atomic write |
| Operation receipts | Runtime audit JSONL | append only |
| Search/index/cache | rebuildable projection | disposable |
| UI state | client-local presentation state | never authoritative |

## Surface contract

All surfaces authenticate to the Runtime endpoint, complete `/version`, submit
typed commands, and consume the same run/event/status IDs. The TUI and desktop
are not reduced copies of an agent; they are presentations of Runtime state.
Delivery adapters use the same typed packet contract and cannot bypass
Runtime authorization.

## Reliability and security

Admission freezes the route ladder, permission mode, sandbox, and capability
epoch. Every effect passes through the broker after an authorization check.
Cancellation preserves committed and partial output. A permission change
revokes the old epoch, cancels and joins old work, rejects stale approvals, and
only then admits new work. No provider, credential, or tool state is global to a
client process.

## Verification

The repository gate is:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The client/UI smoke path proves that CLI, TUI, desktop, and admin observe one
session and one run, including cancellation, approval resolution,
memory/tasks CRUD, checkpoint/backup operations, and diagnostics.

Detailed contracts live in docs 01–36. Those documents describe the current
Runtime implementation and must not introduce a second ownership model.
