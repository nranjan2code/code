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

## Provider families

| family | adapter | covers |
|---|---|---|
| anthropic-messages | `anthropic.rs` | Anthropic |
| openai-completions | `openai.rs` | OpenRouter, Ollama, Groq, Together, vLLM, any `/v1/chat/completions` endpoint that supports the requested feature set |
| openai-responses | `openai_responses.rs` | OpenAI and OpenRouter (`/v1/responses`, for models/features that require the Responses dialect) |
| google-generative-ai | `google.rs` | Gemini (`streamGenerateContent?alt=sse`) |
| openai-completions (zen) | `openai.rs` | OpenCode Zen (`opencode.ai/zen/v1`) |

Registry names: `anthropic`, `openai`, `openai-responses`, `openrouter`,
`openrouter-responses`, `opencode-zen`, `ollama`, `google` (lazy-built,
cached). Auth via
`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY` /
`GEMINI_API_KEY` / `OPENCODE_API_KEY`; Ollama needs no key. Base-URL
overrides: `VAK_{ANTHROPIC,OPENAI,OPENROUTER,OLLAMA,GOOGLE,
OPENCODE_ZEN}_BASE_URL`.

The adapters intentionally implement these API dialects, rather than infer
features from a model-name prefix:

| Route | Contract | Tooling compatibility rule |
|---|---|---|
| `anthropic` | Anthropic Messages + SSE | Native tool-use blocks. |
| `openai-responses` | OpenAI Responses + named SSE events | Preferred for native OpenAI reasoning/tool combinations. |
| `openai` | OpenAI-compatible Chat Completions + delta SSE | The endpoint must accept the requested model and features. |
| `openrouter-responses` | OpenRouter Responses | Use when OpenRouter's selected model/feature combination requires Responses. |
| `openrouter`, `opencode-zen`, `ollama` | OpenAI-compatible Chat Completions | Compatibility is endpoint- and model-specific; reject live mismatches rather than assuming support. |
| `google` | Gemini `streamGenerateContent` + SSE | Client function declarations/results are supported; Google recommends its newer Interactions API for new agent integrations, but this route remains a distinct supported wire contract. |

## Model discovery

There is no hardcoded model catalogue. Which models exist is a property of
the user's key, so `models.rs` asks the provider and
`Core::discover_models` memoises the answer for 5 minutes (invalidated
whenever a key is stored or revoked).

| shape | providers | request | response |
|---|---|---|---|
| OpenAI listing | `openai`, `openai-responses`, `openrouter`, `openrouter-responses`, `opencode-zen`, `ollama` | `GET {base}/models`, bearer | `{ data: [{ id }] }` |
| Anthropic | `anthropic` | `GET {base}/v1/models`, `x-api-key` + `anthropic-version` | `{ data: [{ id }], has_more, last_id }` |
| Google | `google` | `GET {base}/models?key=…` | `{ models: [{ name: "models/x" }], nextPageToken }` |

Anthropic (default 20/page) and Google (default 50/page) **paginate** — both
are followed to exhaustion (`after_id` / `pageToken`, capped at 20 pages) or
the list silently truncates. Google ids are returned fully qualified and are
stripped of the `models/` prefix. Ids are sorted and de-duplicated.

Failures surface as themselves: HTTP status maps onto the same `LlmError`
taxonomy the chat paths use (401/403 → `Auth`, 429 → `RateLimit`, …), so a
UI can say "this key is invalid" rather than offering models the key cannot
reach. Discovery never falls back to a static list. Endpoint *hosts* keep
defaults (`default_base_url`) because those are configuration; model *ids*
are always live.

Keys are user-supplied and user-revocable. `Core::set_provider_key` writes
`~/vak-home/.env` (0600); `remove_provider_key` strips the entry, clears the
runtime override and the loaded-dotenv copy, and reports `shadowed_by_env`
when the variable is *also* exported in the real environment — that copy
cannot be unset from inside the app, and the provider stays authenticated.
Both paths drop the cached provider client and the discovered-model cache.

### Endpoint capability negotiation

A usable route is `credential + provider + endpoint dialect + model +
requested capabilities`, not merely a provider/model pair. `RouteLeg` freezes
the dialect as well as the credential and model. This means a later retry or
fallback cannot accidentally change a request from a native Messages,
Responses, GenerateContent, or Chat Completions contract.

The admission selector owns provider-to-dialect choice and contains no model
name table. Native Anthropic and Google routes use their native contracts;
agentic OpenAI and OpenRouter routes choose Responses, while local and generic
OpenAI-compatible routes retain Chat Completions unless their endpoint has an
explicit Responses contract. A provider/model catalogue is still live data:
where a provider publishes feature metadata it must be retained and checked at
admission; where it does not, an explicit bounded compatibility probe or a
live rejection is evidence. Unknown is not compatibility. A rejection is a
permanent endpoint-capability mismatch, never a transient retry and never an
unannounced post-admission route switch.

Providers may also expose a pool through a plural environment variable:
`ANTHROPIC_API_KEYS`, `GEMINI_API_KEYS`, `OPENAI_API_KEYS`,
`OPENROUTER_API_KEYS`, or `OPENCODE_API_KEYS`. Values are comma- or
newline-separated; the singular variable remains the primary credential.
VAK fingerprints each credential without storing or returning the secret,
discovers models separately per credential, and freezes the selected
fingerprint into each session route leg. Pool identity is distinct from
provider quota identity: provider-reported organization, project, workspace,
model-class, and account limits remain separate observations and are never
assumed to be per-key.

Secrets live in `.env` (project) or `~/vak-home/.env` (Shared) — both are
gitignored by convention and loaded at startup; real environment variables
always take precedence. Never commit keys.

Gemini specifics: roles are `user`/`model`; tool args are JSON objects;
function responses ride in a user turn keyed by function NAME — the adapter
resolves ids→names by walking prior assistant `functionCall` parts; parts
preserve assistant source order; any `functionCall` part forces
`stop_reason = ToolUse`.

Responses-API specifics: system prompt rides in `instructions`; history is
typed items (`input_text`/`output_text`, `function_call`,
`function_call_output`); SSE deltas are keyed by event name
(`response.output_text.delta`, `response.function_call_arguments.delta`);
any emitted `function_call` item forces `stop_reason = ToolUse` so the loop
continues even though `response.completed` terminates the stream.

Cross-provider history conversion happens in the request builders (stateless,
from the neutral log): OpenAI-family drops `Thinking` blocks, re-emits
assistant `ToolUse` as `tool_calls` with **original ids preserved** (so a
session that starts on Anthropic continues cleanly on OpenAI and vice versa),
and splits user `ToolResult` blocks into one `role:"tool"` message each.
OpenAI tool-call argument fragments accumulate raw per call index (same
lesson as Anthropic's `input_json_delta`: partial JSON never round-trips
through a parsed Value).

## Testing

Fixture SSE streams replayed against a local TCP mock: full-turn conversion,
snapshot consistency while streaming, abort-preserves-partial, HTTP status →
typed error mapping, decoder edge cases. No network in CI.
