# 30 — Output projection

`vak-delivery` is the channel-neutral projection library. It is downstream of
Runtime and has no authority over sessions, runs, permissions, credentials, or
the filesystem ledger.

## Contract

The library converts an exact source Markdown answer into a `DeliveryPacket`:

1. typed content (`answer`, `summary`, `alert`, `approval`, progress, or tool
   result);
2. a conservative block projection with stable block IDs;
3. a rendered payload selected by a bounded `DeliveryProfile` and template;
4. an exact `fallback_markdown` copy; and
5. ordered, UTF-8-safe chunks plus diagnostics.

Unknown structures remain in the fallback and are reported as coverage
diagnostics. A template contains literal text, approved slots, and bounded
conditionals only. It cannot execute code, read files, call tools, or access a
secret. User and trusted-project template files are layered by ID; untrusted
project files are ignored.

```toml
[templates.compact-result]
revision = 2
format = "{title}\n\n{body}"

[channels.webhook]
template = "compact-result"
max_chars = 3500
```

Allowed slots are `{title}`, `{body}`, `{source_markdown}`, `{block_count}`,
and `{metadata.KEY}`. The source ledger remains the complete answer; a
projection can never replace it.

## Process boundary

The optional `vak-delivery-worker` process is line-oriented and deterministic.
It receives a serialized delivery job, returns a packet, and owns no Runtime
handles or credentials. The broker bounds payload size and worker lifetime;
failure returns a typed diagnostic and preserves the source Markdown.

Native TUI, desktop, and admin clients can render event snapshots directly.
External adapters consume the same packet and are responsible for their own
network credentials and delivery acknowledgements.

## Safety rules

- Every block has an ID and a coverage disposition.
- Chunking is Unicode-scalar safe and never silently truncates.
- Approval actions remain structured fields even when text is rendered.
- The original Markdown is always retained as the exact fallback.
- Renderer failures are values; they do not alter Runtime state.
