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
- ~15 keys total to know; everything else is discoverable via
  `vakcoder config dump` (boot-tree introspection, DeepSeek-Harness pattern).
- `VAKCODER_HOME` relocates session storage (default `~/.vakcoder`).

## Current keys

provider, model, max_tokens, max_turns, permission_mode, profile, profiles.*,
anthropic_base_url, allow/ask/deny, subagents, hooks.*, mcp.servers.*,
max_retries, retry_base_backoff_ms, request_timeout_secs,
circuit_breaker_threshold, circuit_breaker_cooldown_secs, context_window
(min 16384; smaller values warn and fall back to the default).

## Later

- permission rules block (Phase 3)
- hooks/skills/MCP registration blocks (Phase 5)
- minimal|standard runtime profiles as eval baseline (Phase 7)
