# 07 — System prompt

Current prompt: `crates/vak-core/src/system-prompt.md` (~120 tokens).
Override per project via `.vak/SYSTEM.md`.

## Diff notes

- v0.1.0: initial six-tool kernel prompt. Rules emphasize read-before-edit,
  small verifiable steps, error-driven fixing, workspace containment.

## Policy

The prompt stays under 1500 tokens. Every change ships with a diff note here
and passes the nightly eval suite before release (Phase 7). Prompt churn is a
bug class, not a feature — changes are reviewable events.
