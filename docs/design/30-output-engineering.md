# 30 — Output engineering and channel delivery

Status: contract, isolated worker, trusted templates, Telegram projection,
semantic webhook envelope, adapter registry, ordered multi-message output, and
durable retry outbox implemented. Native desktop/TUI block widgets remain a
presentation-layer extension; existing event and HIL controls are unchanged.

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

User and trusted-project configuration can select a template by ID and replace
it without changing Rust code. Files are loaded from `<home>/output.toml` and
trusted `<cwd>/.vakcoder/output.toml`; user definitions win by ID:

```toml
[templates.compact-result]
revision = 2
format = "{title}\n\n{body}"

[channels.telegram]
template = "compact-result"
max_chars = 3500
```

Allowed slots are `{title}`, `{body}`, `{source_markdown}`, `{block_count}` and
`{metadata.KEY}`. Templates are schema- and size-bounded data: no scripts,
includes, tools, filesystem lookup, network access, or expression language.
An untrusted project output file is ignored. Agent proposals remain inactive
until explicitly activated.

Every block receives an ID and a coverage disposition. Unknown structures stay
in the exact fallback even when the target cannot render them richly. Silent
loss is a protocol error.

Templates apply only to outward-facing answer-like messages. An approval keeps
its action IDs, verbs, data, and expiry as separate fields even when its text
fallback is rendered. Progress and tool results retain their typed payloads.
This prevents a template from accidentally turning a human-in-the-loop gate,
system message, or tool event into ordinary prose.

## Process boundary and load

Production CLI and desktop binaries host a private `__delivery_worker`
subcommand. The server keeps one empty-environment child alive per sessions
home and exchanges versioned JSON lines. The renderer receives no provider
keys, channel credentials, tools, session handles, or network access. A
five-second watchdog restarts a broken process once; a deterministic in-process
fallback adds a diagnostic rather than suppressing output. Rendering calls no
LLM and has bounded payload/template work.

Before push, the server creates an append-only job-state JSONL under
`<home>/delivery/jobs/`. It records pending, delivered, failed-attempt, and
dead-letter snapshots without deleting the exact job. Writes are synced; Unix
also syncs the containing directory on creation. Replay scans at most 100 jobs
every 30 seconds and stops after ten attempts. The inbox copy is written before
transport, so missing credentials or remote failure cannot erase the signal.

Attached TUI and desktop clients should normally render semantic events locally:
they know terminal width, theme, accessibility mode, and window state. The
worker is primarily for Telegram, webhooks, scheduled tasks, and unattended
surfaces. A small plain-text emergency path may remain in the server for
critical failure alerts.

## Multi-message rule

When one message cannot satisfy a channel limit, `DeliveryPacket.chunks` holds
every ordered part. Adapters must send all chunks sequentially; truncation is
not normal delivery. Telegram closes and reopens HTML tags at boundaries so
each chunk parses independently. Actions appear once, normally on the final
chunk. A partial send returns an error and retains the same job ID for replay.
`fallback_markdown` always carries the exact unsplit answer.

Chunking is Unicode-scalar safe, but not yet grapheme-cluster aware; it can
split a visible emoji sequence while remaining valid UTF-8. That limitation is
explicit until grapheme golden tests land.

## Adapter boundary

Native outbound adapters implement three operations: routing `scheme`, hard
`DeliveryProfile`, and async `send`. The registry currently contains `log:`
and `webhook:`. Credentials are resolved only inside the adapter. Generic
webhooks receive `{target,text,ts,job_id,delivery}` plus `Idempotency-Key`, so a
relay can deduplicate and consume semantics while old receivers keep using
`text`.

Telegram is a sidecar adapter: `/gateway/inbound` returns the semantic packet
and the bridge sends every chunk. Future Slack, Discord, Teams, or Matrix
sidecars can post optional `capabilities` (`markup`, `max_chars`, tables, code,
links, actions) and consume the same packet without changing the agent loop.
Unknown surfaces start conservative; declared limits are capped at 100,000.

For context, Telegram text is limited to 4096 characters, Slack recommends
4000 top-level characters and imposes per-block limits, and Discord message
content is limited to 2000 characters. Discord Components V2 also changes
whether traditional content and embeds may coexist. These are distinct
protocol contracts, not styling preferences.

Primary references:

- [CloudEvents core specification](https://github.com/cloudevents/spec/blob/main/cloudevents/spec.md)
- [Telegram Bot API](https://core.telegram.org/bots/api#sendmessage)
- [Slack `chat.postMessage`](https://api.slack.com/methods/chat.postMessage)
- [Slack Block Kit limits](https://api.slack.com/reference/block-kit/blocks)
- [Discord message resource](https://docs.discord.com/developers/resources/message)
- [Discord components](https://docs.discord.com/developers/components/overview)

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

## Integration status

1. Durable outbox records and a persistent worker supervisor: complete.
2. Telegram conversion behind the worker with tag-safe chunking: complete.
3. Semantic webhook envelope plus `text` fallback: complete.
4. Stable semantic packet returned to sidecars and available to native clients:
   complete. Native desktop/TUI block widgets remain future presentation work.
5. Golden fixtures for tables, diffs, long Unicode, links, code, errors,
   approvals, accessibility/plain mode, and unsupported features.
