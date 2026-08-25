# 30 — Output engineering and channel delivery

Status: foundation implemented; server/outbox integration follows after the
contract and worker protocol have stabilized.

## Problem

The assistant's Markdown is currently close to the delivery contract. That is
safe for export but weak for channels: terminals, desktop, Telegram, Slack,
Discord, and generic webhooks have different widths, markup, interaction
models, accessibility needs, and hard payload limits.

Formatting must not become a second source of truth. The session ledger keeps
the complete assistant output. Delivery is a derived projection.

## Contract

`vak-delivery` defines six layers:

1. `DeliveryKind`/`DeliveryContent` identify what is being delivered. An
   assistant answer, task summary, alert, approval, progress update, and tool
   result are not interchangeable. System/developer/internal messages are
   control-plane data and are rejected by the external delivery renderer.
2. `AnswerDraft` contains the exact source Markdown and a conservative block
   projection. Unknown structures become `RawMarkdown`; they are never
   dropped.
3. `DeliveryProfile` declares the target's accepted markup and constraints.
4. `TemplateSpec`/`TemplateRegistry` provide replaceable, declarative layouts.
   Templates contain literal text, approved slots, and bounded conditionals;
   they cannot execute code or access tools. Built-in, project, and user
   templates can be active. Agent-created templates are proposals until an
   explicit activation operation approves them. Higher-precedence active
   templates replace lower-precedence ones by ID.
5. `DeliveryPacket` contains the rendered payload, exact Markdown fallback,
   bounded chunks, block coverage, and diagnostics.
6. `vak-delivery-worker` is a separate line-oriented process. It renders jobs
   but does not own transport credentials, sessions, permissions, or the
   canonical ledger.

The eventual user/project configuration can select a template by ID and
replace it without changing Rust code, for example:

```toml
[output]
template = "compact-result"

[output.templates.compact-result]
revision = 2
origin = "user"
format = "plain"
```

The exact config syntax is intentionally deferred until the server outbox is
integrated; the typed registry is the stable boundary underneath it.

Every block receives an ID. A later validator must require every ID to be
rendered, preserved in fallback, attached as an artifact, or explicitly
reported as degraded. Silent loss is a protocol error.

Templates apply only to outward-facing answer-like messages. An approval keeps
its action IDs, verbs, data, and expiry as separate fields even when its text
fallback is rendered. Progress and tool results retain their typed payloads.
This prevents a template from accidentally turning a human-in-the-loop gate,
system message, or tool event into ordinary prose.

## Process boundary

The server should eventually append a `DeliveryJob` to a durable outbox after
the run is committed. A bounded worker consumes the outbox, renders the job,
validates the target payload, and invokes the channel adapter. The job needs an
idempotency key (`run + target + revision`) so worker restart and retries cannot
duplicate delivery accidentally.

Attached TUI and desktop clients should normally render semantic events locally:
they know terminal width, theme, accessibility mode, and window state. The
worker is primarily for Telegram, webhooks, scheduled tasks, and unattended
surfaces. A small plain-text emergency path may remain in the server for
critical failure alerts.

## Safety and performance rules

- The worker is deterministic by default; no LLM editorial pass is in the
  delivery critical path.
- The server does not wait on remote rendering or transport calls after a
  committed run; the outbox is the handoff point.
- Worker concurrency, payload size, and per-target retry budgets are bounded.
- Template files are data, not code: they are schema-validated, size-bounded,
  and resolved by explicit precedence.
- Credentials are injected only into the transport adapter that needs them.
- The original answer remains available if a worker is unavailable or a target
  rejects its projection.
- Rendering is versioned. A packet records the renderer/schema version and
  degradation decisions for debugging and replay.

## Planned integration slices

1. Add durable outbox records and a worker supervisor without changing current
   gateway behavior.
2. Move Telegram conversion behind the worker, retaining byte-compatible
   fallback behavior and adding limit-aware chunking.
3. Change generic webhooks to receive a semantic JSON envelope plus `text`
   compatibility fallback.
4. Expose the same semantic event stream to TUI and desktop renderers.
5. Add golden fixtures for tables, diffs, long Unicode, links, code, errors,
   approvals, accessibility/plain mode, and unsupported features.

The current crate is intentionally only the contract and pure renderer. This
keeps the first change reviewable while other server work is active.
