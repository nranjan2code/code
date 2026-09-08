# 14 — Checkpoints & worktree isolation
Status: implemented in 2.0.0

## Checkpoints

State-based workspace snapshots captured automatically at the start of every
run (`run_turn_with`), stored under
`data_home()/checkpoints/<session-id>/<seq>.json`.

- **Scope**: every regular file under cwd, excluding `.git`, `target`,
  `node_modules`, `.vak`, `dist`; per-file cap 8MB, total cap 64MB.
  Contents are base64 in the ledger JSON.
- **Restore semantics**: rewrite all snapshotted files (recreating deleted
  ones) and delete any tracked file that did not exist at capture time.
  Comparisons use relative paths — immune to symlinked roots
  (`/tmp` vs `/private/tmp`).
- **Why state-based, not operation-based**: bash mutations are not invertible;
  snapshots cover them by construction. Cost is bounded by the caps above.

CLI:

```
vak checkpoints list [--session <id>]
vak checkpoints restore <session> <seq>
```

## Worktree isolation

`exec --worktree` / `plan --worktree` create
`.vak/worktrees/<run-id>` on branch `vak/<run-id>` off HEAD and run
there. The main checkout is never touched; on success the worktree is kept
for inspection (path printed), on failure it is removed. Non-repos fail fast
with a typed error. Cleanup: `git worktree remove --force` + branch delete.

## Invariants

- Checkpoint ledgers are append-only artifacts outside the session JSONL
  (they are workspace state, never model-visible context).
- Restore deletes only within the tracked scope and only files absent from
  the snapshot — untracked-but-ignored paths are never touched.

## Diff note — restore-safety rework (this change)

`capture` now records an `observed` manifest: every regular-file path the
walk saw, including files whose content was NOT stored (oversized, unreadable,
secret, gitignored, beyond-budget). `restore` deletes only files that are
present now and absent from `observed` — anything capture could not vouch
for is left untouched. Legacy checkpoints without a manifest delete nothing.
Capture additionally skips secret paths (`.env*`, `*.pem`, `*.key`,
`id_rsa*`, `id_ed25519*`, `credentials.json`) and honors a gitignore subset
(root + nested `.gitignore`, dir-only rules, negation; last match wins).
`store` prunes to the newest 20 checkpoints per session. Invariant: rewind
can lose in-session changes; it must never destroy files it knows nothing
about.
