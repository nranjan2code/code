# 01 — LLM layer (vak-llm)

## Decisions

1. **Raw provider APIs, no meta-SDK.** pi's lesson: Vercel-AI-style
   lowest-common-denominator abstractions leak. We own request/response shape
   per provider family. Cost: ~2K LOC per family; acceptable.
2. **Every streaming event carries delta AND accumulated snapshot**
   (`StreamEvent::TextDelta { delta, partial }`). Consumers render incrementally
   or re-render from snapshot; never forced to re-derive.
3. **Streams are async-iterable AND awaitable**: iterate events for UI, then
   `stream.result()` for the final `AssistantMessage` or typed error.
4. **Errors are values.** Connect-time failures return `Err(LlmError)`.
   Mid-stream failures terminate the stream and surface via `result()`.
   Nothing panics into consumers.
5. **Abort is first-class.** `CancellationToken` gates both the HTTP send and
   the body read (`tokio::select!`). On abort: `LlmError::Aborted { partial }`
   preserves everything received so far.
6. **Lazy provider registry.** Factories registered by name; providers built on
   first use and cached. Adding a provider = one factory + one adapter module.
7. **SSE parsing is hand-rolled** (`SseDecoder`): spec-correct frames, CRLF,
   comments, multi-line data, split-chunk safe. No eventsource dependency.
8. **Tool-input JSON accumulates raw per block index** and parses at
   `content_block_stop`; intermediate deltas best-effort parse for snapshots.

## Error taxonomy

`Auth | RateLimit{retry_after} | Overloaded | InvalidRequest | Api{status} |
Network | Parse | Aborted{partial}` — `is_retryable()` covers the first three.
Retries are a caller policy (loop/CLI), never hidden inside the provider.

## Neutral vocabulary

`Message { role, content: Vec<ContentBlock> }`,
blocks: `Text | Thinking{signature} | ToolUse{id,name,input} | ToolResult{tool_use_id,content,is_error}`.
Anthropic maps 1:1; OpenAI-responses/Google adapters translate in Phase 4 with
cross-provider transforms (thinking→text, tool-id remap, orphan-result repair).

## Testing

Fixture SSE streams replayed against a local TCP mock: full-turn conversion,
snapshot consistency while streaming, abort-preserves-partial, HTTP status →
typed error mapping, decoder edge cases. No network in CI.
