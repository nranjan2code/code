Status: implemented

# 51 — Retired Tool and Plugin Lifecycle

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

This runs during `vak setup seed`, `vak services-sync`, and
`vak self update` (which calls `services-sync`).

### 5. Startup Warning (`vak-core::Core::new_with_trust`)

At every Core initialization, `warn_retired_plugins()` scans the
capability roots and emits an `eprintln!` warning for each retired
plugin found. This gives operators immediate feedback even if they
haven't run setup/update.

The warning message names the plugin, the retired tools it references,
and the repair command (`vak setup seed` or `vak plugins remove <name>`).

### 6. Doctor Check (`vak-core::health`)

A new `retired_plugins_check` is added to the `/doctor` health report.
It fails (showing as ✗ in `vak doctor`) when retired plugins are
detected. `vak doctor --repair` runs the cleanup pass automatically.

### 7. Admin API (`vak-server`)

- `GET /plugins/retired` — list retired plugins and the tool names
  they reference.
- `DELETE /plugins/retired` — remove all retired plugins and prune
  their config entries.

### 8. Tool Name Aliasing (`vak-tools::canonical_tool_name`)

The existing `canonical_tool_name()` function (which already remaps
common model hallucinations like `read_file` → `read`) now also
redirects retired tool names to their replacement. When the model
calls `python_eval`, the name is normalized to `bash` by
`normalize_tool_call()` in the agent loop. The resulting error
(`missing required parameter: command`) is classified as `Correctable`,
enabling model self-repair rather than a hard `unknown_capability`
failure.

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
| Model calls retired name | Redirected to `bash` | `vak-tools::canonical_tool_name` |

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
