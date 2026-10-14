# 51 — Retired Tool and Plugin Lifecycle

Status: implemented; checked against the code on 2026-10-14.

## Problem

When a tool is removed from vak's callable interface (e.g. `python_eval`
and `react_preview` were retired in 3.0.21 when the sandbox unified on
the neutral `bash` execution engine), associated plugin packages may
survive on the user's disk. Their `SKILL.md` descriptions still instruct
the model to call the retired tool name, which produces
`unknown_capability` errors and — worse — model hallucinations where the
agent fabricates execution results for a tool that never ran.

The root cause is that **plugin retirement was not part of the tool
retirement lifecycle**. Removing source files (`python_runner.rs`,
`react_runner.rs`) did not clean up the installed plugin packages, the
plugin store registry, or the `[plugins] network_allow` config entries.

## Design

### 1. Retired Tools Registry (`vak-tools/src/retired.rs`)

A compiled-in, append-only list of retired tool names. Each entry
record:

- `name` — the exact tool name the model might try to call
- `retired_in` — the version when the tool was removed
- `replacement` — what the model should use instead (e.g. `bash`)

This list lives in `vak-tools` because that crate owns the tool
definitions. Adding a new retirement entry is the single mechanism for
registering a tool as retired.

### 2. Plugin Validation at Load Time (`vak-plugin`)

`PluginStore::retired_plugins()` walks all installed plugin packages
and scans every `SKILL.md` file for backtick-quoted references to
retired tool names. Returns `(plugin_name, [retired_tool_names])`
pairs.

This is a **read-only** check — it never removes or disables plugins.
Removal is an intentional operator action (see §4).

### 3. Skill Validation at Discovery Time (`vak-core::skills`)

`skills::validate()` now rejects any `SKILL.md` that references a
retired tool name in its description or body. The skill is excluded
from the capability contract sent to the model (the system prompt's
"Available skills" list). A `SkillDiagnostic` explains why it was
dropped.

This ensures that even if a retired plugin survives cleanup, its stale
skills never reach the model's tool inventory.

### 4. Automatic Cleanup at Setup/Update (`vak-core::seed`)

`seed_shared_capabilities()` now calls `cleanup_retired_plugins()`,
which:

1. Queries `PluginStore::retired_plugins()` for both the shared and
   workspace plugin stores.
2. Removes each flagged plugin via `PluginStore::remove()`.
3. Prunes stale entries from `[plugins] network_allow` in the config
   via `vak_config::prune_plugins_network_allow()`.

This runs during `vak setup seed`, `vak self services-sync`, and
`vak self update` (which calls `services-sync`).

### 5. Startup Warning (`vak-core::Core::new_with_trust`)

At every Core initialization, `warn_retired_plugins()` scans the
capability roots and logs a content-free `tracing` warning for each
retired plugin found, with the count of retired tools it references and
the repair command (`vak setup seed`); library code never writes to a
person's stderr. The plugin's name is in `vak doctor` (§6), which a
person reads.

### 6. Doctor Check (`vak-core::health`)

A new `retired_plugins_check` is added to the `/doctor` health report.
It fails (showing as ✗ in `vak doctor`) when retired plugins are
detected. `vak doctor --repair` runs the cleanup pass automatically.

### 7. Admin API (`vak-server`)

- `GET /plugins/retired` — list retired plugins and the tool names
  they reference.
- `DELETE /plugins/retired` — remove all retired plugins and prune
  their config entries.

### 8. A call by a retired or unknown name

A call is matched to a tool by its exact name. A call naming a retired
tool, or any name no admitted tool has, is answered with
`unknown_capability` and the tools that are available, and the model
calls one of those. A name in `RETIRED_TOOLS` is also told what replaced
it (`"retired": true`, `"use_instead"`), from that registry. No name stands for another tool: the alias table
that rewrote `python_eval`, `read_file`, `shell` and the like to a
built-in tool (`canonical_tool_name`) was removed on 2026-10-14, because
it rewrote a call to an admitted tool of that very name (a tool named
`shell` was called as `bash` and refused).

## Lifecycle Summary

| Stage | Action | Component |
|-------|--------|-----------|
| Tool retired | Add to `RETIRED_TOOLS` | `vak-tools/src/retired.rs` |
| Plugin persists | Detected by `retired_plugins()` | `vak-plugin` |
| Skill loaded | Rejected by `validate()` | `vak-core::skills` |
| Core starts | Warning to operator | `vak-core::Core` |
| Setup/Update | Auto-removed | `vak-core::seed` |
| Doctor | Health check + repair | `vak-core::health` / `vak::doctor` |
| Admin | List + delete | `vak-server` |
| Model calls retired name | `unknown_capability` with the available tools | the agent loop |

## AGNS Invariants Satisfied

- **Invariant 9**: Model catalogues are discovered, never hardcoded.
  Retired tools are compiled-in, discovered at runtime.
- **Invariant 13**: FullAccess is an explicit human trust decision.
  The retired-plugin check runs regardless of permission mode.
- **Invariant 29**: Pre-baseline state is refused, not migrated.
  Retired tools are refused (rejected at validation, removed at cleanup).
- **Invariant 30**: One canonical way per capability. The retired
  tools registry is the single source of truth for what's retired;
  there is no second list.
