# 07 — System prompt

Current prompt: `crates/vak-core/src/system-prompt.md` (~250 tokens).
Override per project via `.vak/SYSTEM.md`.

## Diff notes

- v0.1.0: initial six-tool kernel prompt. Rules emphasize read-before-edit,
  small verifiable steps, error-driven fixing, workspace containment.
- v0.1.1: documented the dynamically advertised task, memory, web, and MCP
  tools; explicitly separated skill guidance from executable tools; added a
  verification/no-false-completion rule after live Ollama prompt testing found
  ambiguity around `task` and discovered skill names.
- v0.1.2: made the skill boundary imperative by explicitly prohibiting skill
  names in tool calls after live-model retries showed that a descriptive skill
  name could still be selected as an executable tool.
- v0.1.3: instructed the agent to inspect file-backed requirements immediately
  rather than asking the user to repeat them; this closes a live Ollama case
  where the model stopped before reading README.md.

## Policy

The prompt stays under 1500 tokens. Every change ships with a diff note here
and passes the nightly eval suite before release (Phase 7). Prompt churn is a
bug class, not a feature — changes are reviewable events.
