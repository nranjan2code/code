# 12 — Eval harness
Status: implemented in 2.0.0

The Binding-Constraint Thesis (harness variance beats model variance) made
operational: `vak eval` runs a deterministic suite that exercises the
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

## General-purpose suite (non-coding)

`general_suite()` proves the harness is a general agent, not a code-only
one: the same six-tool kernel driven through research, data-analysis,
writing, conversion, and inventory scenarios. Also runs as an integration
test (`general_suite_fully_green`) and in the default `vak eval`.

| case | scenario domain exercised |
|---|---|
| general-research-synthesis | multi-source read → synthesized summary artifact |
| general-csv-analysis | bash arithmetic over tabular data → report |
| general-writing-draft | structured writing with mechanical constraints |
| general-doc-conversion | free-form notes → machine-readable JSON |
| general-inventory-index | glob+grep discovery across non-code records |
| general-error-adapts-noncode | failed lookup → error-driven correction |

Beyond the scripted suites, harness behavior in general flows is covered by
`crates/vak-agent/tests/general_flows.rs`: mid-run steering, abort
preserving partial artifacts, the stop gate blocking unverified reports,
compaction during long research sessions (append-only asserted against the
raw JSONL), MCP meta-tool lookups, and read-only mode denials.

## Live-model evals

`vak eval --live` runs `live_suite()` (create-file, sort-lines,
json-edit — small, verifiable, environment-independent) against the
configured provider with no scripted trajectory: the model must genuinely
solve each task. Same report format, same token/cost accounting, so nightly
runs track real model performance through the harness. Requires API keys in
the environment; respects provider/model config and flags.
