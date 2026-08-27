# vakcoder — Engineering Contract

This repository is a greenfield Runtime system. There is one active authority
graph and no migration or backward-compatibility contract. Do not reintroduce
parallel state owners, compatibility adapters, or legacy entry points.

## Authority graph

`vak-runtime` is the only mutable application authority. It owns project
registration, sessions, runs, cancellation, capability epochs, permissions,
tasks, approvals, memory, skills, checkpoints, backups, flows, evals, secrets,
and audit. Every surface is an adapter:

```text
TUI ───────┐
Desktop ───┤
Admin UI ──┤── vak-client ── vak-server ── vak-runtime
CLI ───────┘                                  │
                                              ├─ vak-storage (SQLite + blobs + audit)
                                              ├─ vak-session (append-only JSONL)
                                              ├─ vak-config (TOML + .env)
                                              ├─ vak-agent (execution engine)
                                              └─ vak-services (tasks, approvals, memory, delivery)
```

`vak-llm` provides provider protocols and live model discovery. `vak-tools`
provides brokered workers, filesystem tools, Bash, webfetch/browse, claims,
and sandbox backends. These are called by Runtime-owned execution only.

The Runtime graph is the only graph. There are no parallel core, session,
server, client, permission, flow, or eval authorities. If a feature is needed,
implement it in Runtime and expose it through the typed client instead of
creating a second state path.

## Persistent layout

```text
data_home/
├── config.toml                    # global configuration layer
├── .env                           # user secrets; never committed
├── state.db                       # Runtime control state and projections
├── sessions/<project>/<id>.jsonl  # append-only model-visible ledger
├── blobs/<sha256>                 # content-addressed checkpoint/data blobs
├── audit/operations.jsonl         # append-only operational audit
├── runtime/gateway.json           # live gateway endpoint/token/pid
├── locks/                         # Runtime/gateway/process locks
└── logs/

project/.vakcoder/project.toml    # project config layer
cache_home/search.db              # disposable derived search index
```

JSONL is the source of truth for model-visible history. SQLite and search are
rebuildable/transactional projections. Config writes are revisioned, locked,
atomic, and layered global-then-project. Environment variables override file
secrets; secrets never enter logs, git, prompts, or worker environments.

## Non-negotiable invariants

1. Anything sent to a model is reconstructable from that session JSONL through
   `derive_messages()`; new visible input requires a ledger entry.
2. Sessions are append-only. Branching adds a parent-linked entry; compaction
   adds an entry and never deletes history.
3. Errors are values. Providers, tools, hooks, MCP, flows, and Runtime return
   typed errors/results. Production library code must not panic or use
   `unwrap`/`expect`.
4. Every streaming event contains both delta and snapshot state.
5. Cancellation tokens reach every async provider/tool/hook/MCP call; partial
   output is retained and exactly one terminal run outcome is persisted.
6. Every effectful dispatch passes permission, capability-epoch, workspace
   lease, and broker authorization immediately before execution.
7. Permission changes revoke old capabilities, cancel/join old-epoch work, and
   reject stale approvals before the new mode is visible.
8. Restricted filesystem operations resolve beneath the canonical registered
   project root; traversal and symlink escapes fail closed.
9. Model catalogues are discovered from the configured provider API. Never
   hardcode model IDs or silently substitute a static catalogue.
10. Restricted workers receive an explicit operational environment allowlist;
    provider, gateway, and connector credentials are recipient-scoped.
11. FullAccess is explicit human authorization and is never selected by a
    retry, denial, failure, or model suggestion.
12. Gateway, scheduler, replay, rebuild, outbox, and delivery duties are
    singleton-controlled and never run from a plain request-only process.

## Surface contracts

- `vak-server` is the authenticated HTTP/SSE transport. It owns no state.
- `vak-client` is the typed client used by TUI, desktop, admin, and CLI.
- `vak-tui` is a thin terminal adapter; it does not open Runtime files.
- `vak-desktop` is a Tauri shell over the same secured Runtime API. It does
  not start a competing server, shell, scheduler, or session store.
- `vak-admin-ui` is the embedded SolidJS observation/operation console.
- `vakcoder` is the command-line adapter. CRUD/config/task/memory/checkpoint/
  backup/flow/eval operations must resolve through Runtime contracts.
- Channels, delivery, tray, and service-control code may transport or manage
  the Runtime process but must not own agent state.

## Code rules

- Rust edition 2024, stable toolchain.
- Pin new dependencies in the workspace manifest with a one-line reason.
- Keep public contracts documented when their safety semantics are not obvious.
- Keep system prompts below 1500 tokens and update `docs/design/07-prompt.md`
  with prompt behavior changes.
- Unknown config keys are warnings, never fatal errors.
- Use `apply_patch` for source edits. Preserve unrelated user changes.

## Verification

Before every commit:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-release-version.sh
```

Also build both committed web surfaces:

```text
(cd crates/vak-admin-ui && npm run build)
(cd crates/vak-desktop/ui && npm run build)
```

The only handwritten version is `[workspace.package].version` in
`Cargo.toml`. Use `scripts/bump-version.sh` for releases. Never point managed
services at a build tree or copy binaries by hand.
