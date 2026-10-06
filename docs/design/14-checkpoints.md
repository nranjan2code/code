# 14 — Checkpoints & worktree isolation
Status: implemented in 2.0.0

## Checkpoints

State-based workspace snapshots captured automatically at the start of every
run (`run_turn_with`). A small per-checkpoint **manifest**
(`<session_id>/<seq>.json` under the Agent's checkpoints root — path, object
id, size and mtime per file, plus the observed set) names file contents held
as tenant objects (`tenants/<t>/objects`, sealed per object) granted to the
conversation, so equal contents are stored once (data-architecture plan, M3b
slice 2). A manifest that does not deserialize is simply not read (AGENTS.md
invariant 29): never partially read, never migrated.

- **Incremental capture**: a file whose `(size, mtime)` match its entry in
  the immediately preceding manifest is assumed unchanged and its hash is
  reused without being re-read (the standard rsync/make-style
  fast path) — a multi-thousand-file workspace's per-turn capture becomes a
  handful of stats plus however many files actually changed. `CaptureStats`
  (`files_observed`/`files_reused`/`files_read`) lets callers and tests
  observe the fast path directly.
- **Scope**: every regular file under cwd, excluding `.git`, `target`,
  `node_modules`, `.vak`, `dist`, and the rebuildable data catalog's
  files (`catalog.db`/`-wal`/`-shm`); per-file cap 8MB, total cap 64MB. Capture
  also skips secret paths (`.env*`, `*.pem`, `*.key`, `id_rsa*`,
  `id_ed25519*`, `credentials.json`) and honors a gitignore subset (root +
  nested `.gitignore`, dir-only rules, negation; last match wins).
- **Restore semantics**: rewrite all snapshotted files (recreating deleted
  ones) and delete only files that are present now and absent from the
  manifest's `observed` set — a path the capture walk never saw (over
  budget, unreadable, secret, gitignored) is left untouched. Comparisons use
  relative paths — immune to symlinked roots (`/tmp` vs `/private/tmp`). A
  legacy checkpoint with no manifest deletes nothing.
- **Why state-based, not operation-based**: bash mutations are not invertible;
  snapshots cover them by construction. Cost is bounded by the caps above.
- **Retention**: `store` keeps only the newest 20 checkpoints per session.
  Contents a pruned manifest names lose the conversation's grant unless a
  surviving manifest of that session still names them, and objects no scope
  holds are then collected.

CLI:

```
vak checkpoints list [--session <id>]
vak checkpoints restore <session> <seq>
```

## Worktree isolation

`exec --worktree` / `plan --worktree` create
`tenants/<tenant>/environments/<run-id>` in the data home
(`vak_config::paths::environment_dir`) on branch `vak/<run-id>` off HEAD and run
there. The main checkout is never touched; on success the worktree is kept
for inspection (path printed), on failure it is removed. Non-repos fail fast
with a typed error. Cleanup: `git worktree remove --force` + branch delete.

## Invariants

- Checkpoint ledgers are append-only artifacts outside the session JSONL
  (they are workspace state, never model-visible context).
- Restore deletes only within the tracked scope and only files absent from
  the snapshot — untracked-but-ignored paths are never touched. A rewind can
  lose in-session changes; it must never destroy files it knows nothing
  about.
