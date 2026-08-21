# 12 — Eval harness

The Binding-Constraint Thesis (harness variance beats model variance) made
operational: `vakcoder eval` runs a deterministic suite that exercises the
production agent loop, tools, permissions, and session ledger — only the
provider is scripted.

## Design

- **In-process**: no HTTP, no external processes for the model; each case's
  trajectory is a `Vec<ScriptedTurn>` served by an `EvalProvider`. Runs in
  ~100ms total; safe for every CI run.
- **Real workspace per case**: files are materialized into a tempdir; the
  agent loop executes real tool calls against them.
- **Verification is a bash command** run with the production BashTool in the
  same workspace after the loop finishes. Exit 0 = pass.
- **Metrics on every case**: tokens in/out (from the session ledger),
  duration, and any loop error (aborted / failed / max-turns) are recorded
  even when verification passes.
- **`--report <path>`** writes JSON (`passed`, `total`, token totals,
  per-case details) for trend tracking; exit code is non-zero when any case
  fails, so regressions block releases.

## Built-in suite

| case | capability exercised |
|---|---|
| write-file | write tool + exact content |
| edit-file | atomic edit on setup file |
| bash-pipeline | multi-step bash + artifact |
| parallel-writes | batched parallel tool execution |
| permission-denial-adapts | full-access path sanity |

The suite also runs as a `cargo test --workspace` integration test
(`builtin_suite_fully_green`) so any harness regression fails CI directly.

## Live-model evals

`vakcoder eval --live` runs `live_suite()` (create-file, sort-lines,
json-edit — small, verifiable, environment-independent) against the
configured provider with no scripted trajectory: the model must genuinely
solve each task. Same report format, same token/cost accounting, so nightly
runs track real model performance through the harness. Requires API keys in
the environment; respects provider/model config and flags.
