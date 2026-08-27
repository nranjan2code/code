# 07 — System prompt

Current prompt: `crates/vak-runtime/src/system-prompt.md` (~120 tokens).
Override per project via `.vakcoder/SYSTEM.md`.

## Diff notes

- Current contract: the Runtime prompt emphasizes read-before-edit, small
  verifiable steps, error-driven fixing, workspace containment, and explicit
  authorization before effects.

## Policy

The prompt stays under 1500 tokens. Every change ships with a diff note here
and passes the nightly eval suite before release. Prompt churn is a
bug class, not a feature — changes are reviewable events.
