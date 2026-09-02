# 10 — Flows (static DAGs)

## Format

`.vak/flows/<name>.toml` (project) or `data_home()/flows/` (user):

```toml
[flow]
name = "migrate-and-test"

[[nodes]]
id = "explore"
type = "agent"
readonly = true
prompt = "Find all call sites of the legacy API"

[[nodes]]
id = "refactor"
type = "agent"
paths = ["src/**"]
prompt = "Migrate the sites from {{explore}} to the new API"

[[nodes]]
id = "test"
type = "bash"
command = "cargo test"
deps = ["refactor"]
```

Node types: `agent` (prompt; readonly/paths like task), `bash` (command,
timeout_ms), `approval` (message → human gate via Approver), `merge`
(concatenates upstream outputs; always runs, emitting a partial-outcome
report when upstream failed).

## Semantics

- **Validate before run**: unique ids, known types, required fields per type,
  unknown/self deps, cycles (Kahn). `{{dep}}` template references imply
  dependencies automatically; unknown references are validation errors.
- **Layered execution**: topological layers run sequentially; nodes within a
  layer run concurrently (JoinSet). Agent nodes spawn child sessions with
  `parent_session_id` lineage — same narrowing rules as subagents.
- **Typed failure policy**: `required` (default true) — failure fails the
  flow and marks every transitive dependent `skipped` in the ledger;
  `required = false` — node fails, dependents are skipped, but merge nodes
  still produce partial-outcome reports.
- **Frozen definition + resume**: each run persists a state ledger JSON
  (`data_home()/flow-runs/<flow>/<run>.json`) containing the raw TOML frozen
  at first run plus per-node status/output. `flow run <name> --resume`
  replays only non-completed nodes.

## CLI

```
vak flow list
vak flow check <name>     # validate + print layers
vak flow run <name> [--resume] [--yes]
```

Ctrl-C aborts cleanly; the ledger allows resuming later.

## Deliberately out of scope (per research)

Dynamic LLM-authored planning is a separate mechanism (planner slice); flows
are deterministic, hand/GPT-authored files validated before execution.

## Later

Runs → flows adoption (`flows adopt --from <session>` with provider/model
taken only from work receipts), deterministic run-vs-run diff, and typed
recovery audits are live (demand-backed 2026-08-23: a completed run manually
rerun by hand).
