# 14 — Checkpoints & worktree isolation

## Checkpoints

State-based workspace snapshots captured automatically at the start of every
run (`run_turn_with`), stored under
`~/.vakcoder/checkpoints/<session-id>/<seq>.json`.

- **Scope**: every regular file under cwd, excluding `.git`, `target`,
  `node_modules`, `.vakcoder`, `dist`; per-file cap 8MB, total cap 64MB.
  Contents are base64 in the ledger JSON.
- **Restore semantics**: rewrite all snapshotted files (recreating deleted
  ones) and delete any tracked file that did not exist at capture time.
  Comparisons use relative paths — immune to symlinked roots
  (`/tmp` vs `/private/tmp`).
- **Why state-based, not operation-based**: bash mutations are not invertible;
  snapshots cover them by construction. Cost is bounded by the caps above.

CLI:

```
vakcoder checkpoints list [--session <id>]
vakcoder checkpoints restore <session> <seq>
```

## Worktree isolation

`exec --worktree` / `plan --worktree` create
`.vakcoder/worktrees/<run-id>` on branch `vakcoder/<run-id>` off HEAD and run
there. The main checkout is never touched; on success the worktree is kept
for inspection (path printed), on failure it is removed. Non-repos fail fast
with a typed error. Cleanup: `git worktree remove --force` + branch delete.

## Invariants

- Checkpoint ledgers are append-only artifacts outside the session JSONL
  (they are workspace state, never model-visible context).
- Restore deletes only within the tracked scope and only files absent from
  the snapshot — untracked-but-ignored paths are never touched.
