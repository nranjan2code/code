# 05 — Config (vak-config)

## Layering

```
defaults  <  ~/.config/vakcoder/config.toml  <  .vakcoder/config.toml  <  env
```

Env: `VAKCODER_MODEL`, `VAKCODER_PROVIDER` (CLI flags override all).
Profiles: named override blocks selected via `profile = "ci"` — for dev/CI
splits without duplicating files.

## Rules

- Unknown keys are ignored with a warning, never fatal (forward compatibility).
- The supported surface is grouped by concern and discoverable through
  `vakcoder config dump`: core agent/provider settings, profiles, frozen route
  policy, retries, permissions, sandbox/tools, UI, hooks, MCP, FinOps, goals,
  memory/learning, gateway, automation, and update checks.
- `VAKCODER_HOME` relocates the data home and nests `cache/` and `logs/` under it (default: `~/Library/Application Support/vakcoder` on macOS, `~/.local/share/vakcoder` on Linux).

## Current contract

`provider` and `model` form one atomic route. They are never persisted or
hot-applied independently. Provider model ids come from live discovery, never
source-code catalogues. The remaining major groups are `profiles.*`,
`[route]`, retry/watchdog/circuit-breaker controls, `allow`/`ask`/`deny`,
`[sandbox]`, `[tools]`, `[ui]`, `[hooks]`, `[mcp.servers]`, `[finops]`,
`[goal]`, `[memory]`, `[learning]`, `[gateway]`, `[automation]`, and `[update]`.
Unknown keys warn and remain forward-compatible.

## Diff note — workspace trust + unknown keys (this change)

Project-layer privileged keys (`permission_mode`, `allow`, `hooks`,
`anthropic_base_url`, `mcp.servers`) are ignored unless the workspace is
trusted: `Core::new_with_trust(cwd, trust)` / CLI `--trust` / per-directory
prompt marker under `~/.vakcoder/trusted/`. The project `.env` is likewise
only loaded when trusted (it can inject `VAKCODER_*_BASE_URL`). Restrictive
keys (`deny`, `ask`) still apply from untrusted projects. Unknown config
keys are diffed against the schema and surfaced as warnings — a typo'd key
is visible, never silently dead.

## Diff note — interface keys (doc 21 close-out)

New `[ui]` surfaces, all cosmetic-tier: unknown values warn and fall back
(`composer` → emacs; `theme` → dark unless defined under `[ui.themes]`),
never fatal. `ui.themes` color tables deserialize through raw TOML so a
non-string entry warns instead of failing the whole config. `osc52`
defaults false — clipboard mutation stays opt-in even though every use is
an explicit Alt-Y / `/copy`. Accessibility flags render-only; they never
alter protocol behavior or session content.

## Diff note — persisted runtime preferences and frozen sessions

Authenticated `/config` mutations for provider, model, max turns, permission
mode, and theme now persist changed fields atomically in the workspace
`.vakcoder/config.toml` before applying the live Core override. Health,
provider discovery, and admin snapshots expose effective values, provenance,
and the route revision. New-session admission in other local processes
refreshes persisted provider/model, max turns, theme, MCP, hooks, and permission
mode; permission changes revoke live capabilities before apply. CLI, task,
heartbeat, and subagent pins remain intentionally transient. Sessions continue
to freeze their provider/model contract at creation; gateway bindings rotate to
a new frozen session on mismatch while preserving the old append-only ledger.
