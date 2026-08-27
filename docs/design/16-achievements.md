# 16 — Verification record

This document records the current greenfield acceptance surface. It is not a
second roadmap or an alternate architecture.

## Runtime acceptance

- projects, sessions, runs, tasks, approvals, memory, skills, checkpoints,
  backups, flow definitions, and evaluations use Runtime-owned IDs and CRUD;
- session ledgers are append-only and reconstruct every model-visible input;
- provider streams emit delta plus snapshot and preserve partial output on
  cancellation;
- route ladders, budgets, permission modes, sandboxes, and capability epochs are
  frozen at admission;
- every effect is brokered, authorized, leased, audited, and cancel-aware;
- gateway singleton, delivery jobs, and approval handling share the same
  Runtime authority; external adapters keep only transport-local state.

## Surface acceptance

CLI, TUI, desktop, and admin authenticate to the same `vak-server` contract.
Each can observe a common project/session/run, submit a prompt, render
streaming output, cancel a run, and report typed errors. The interactive TUI and
desktop also resolve approval requests. External delivery adapters consume
Runtime packets and never create local state when the Runtime is unavailable.

## Release gate

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd crates/vak-admin-ui && npm run build)
(cd crates/vak-desktop/ui && npm run build)
scripts/check-release-version.sh
```

Artifact smoke tests repeat the acceptance path in a disposable data home and
assert that exactly one Runtime process owns the lock and all state changes.
