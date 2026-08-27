# 07 — System prompt

The CLI default prompt is defined when it creates the immutable session
contract in `crates/vakcoder/src/main.rs` (~7 tokens). TUI, desktop, and other
clients provide their prompt as part of the same typed session contract; there
is no second prompt file or override path.

## Diff notes

- Current contract: the Runtime prompt emphasizes read-before-edit, small
  verifiable steps, error-driven fixing, workspace containment, and explicit
  authorization before effects.

## Policy

The prompt stays under 1500 tokens. Every change ships with a diff note here
and passes the nightly eval suite before release. Prompt churn is a
bug class, not a feature — changes are reviewable events.
