# 01 — LLM layer (vak-llm)
Status: implemented in 2.0.0

## Decisions

1. **Raw provider APIs, no meta-SDK or provider SDK.** Broad lowest-common-denominator
   abstractions leak. We own request/response shape
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
| openai-completions (compatible gateway) | `openai.rs` | OpenAI-compatible gateway |

Registry names: `anthropic`, `openai`, `openai-responses`, `openrouter`,
`openrouter-responses`, compatible gateways, `ollama`, `google` (lazy-built,
cached). Auth via
`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY` /
`GEMINI_API_KEY`; Ollama needs no key. Base-URL
overrides: `VAK_{ANTHROPIC,OPENAI,OPENROUTER,OLLAMA,GOOGLE,
compatible gateways).

The adapters intentionally implement these API dialects, rather than infer
features from a model-name prefix:

| Route | Contract | Tooling compatibility rule |
|---|---|---|
| `anthropic` | Anthropic Messages + SSE | Native tool-use blocks. |
| `openai-responses` | OpenAI Responses + named SSE events | Preferred for native OpenAI reasoning/tool combinations. |
| `openai` | OpenAI-compatible Chat Completions + delta SSE | The endpoint must accept the requested model and features. |
| `openrouter-responses` | OpenRouter Responses | Use when OpenRouter's selected model/feature combination requires Responses. |
| `openrouter`, compatible gateways, `ollama` | OpenAI-compatible Chat Completions | Compatibility is endpoint- and model-specific; reject live mismatches rather than assuming support. |
| `google` | Gemini `streamGenerateContent` + SSE | Client function declarations/results are supported; Google recommends its newer Interactions API for new agent integrations, but this route remains a distinct supported wire contract. |

### Reasoning effort

`ChatRequest.effort` is one dial, rendered by each adapter in its own
wire's words:

| Wire | Field |
|---|---|
| Anthropic | `output_config.effort` |
| OpenAI Responses | `reasoning.effort` |
| OpenAI chat completions | `reasoning_effort` (`reasoning.effort` through OpenRouter) |
| Gemini | `generationConfig.thinkingConfig.thinkingLevel` |
| Ollama (native) | never sent |

A wire with three levels sends anything above `high` as `high`. A turn's
own steps carry an effort only when its reading is a direct reply
(`DemandHint::direct_reply`: every part of the turn is conversation, or an
answer expected in one reply, read confidently, with no cited evidence
asked for), and then only `low`: a reading may ask for less reasoning than
the provider's default, never more (invariant 32,
docs/design/47-commitment-kernel.md). With no such reading nothing is
sent and the provider's default stands.

A plain factual question is not a direct reply, and stays that way.
Measured on 2026-10-08 with `vak intent explain`: 20 of 20 questions got
the same reading (an answer, within the turn, no reasoning demanded): 8
single facts ("What is the capital of Australia?"), 10 that need working
out ("Which is larger, 2^100 or 3^63, and by what factor?") and 2 that
need a tool ("What is in my notes.txt file?"). Nothing in the reading
tells them apart, so lowering effort for that reading would lower it for
the ten that need it. Only a reading the words themselves make immediate
(conversation, or an answer expected in one reply) is lowered.

No table says which models take the setting. A 400 that names it
(`models::rejects_effort`) is retried once without it, and that model on
that endpoint is remembered for the life of the process
(`models::mark_effort_unsupported`), so later requests skip it.

Ollama never receives it. Measured on gemma4 (2026-10-07): with thinking
off, a card call failed 20 of 20, so on that wire the model's own default
is the only setting.

### Deferred tools and tool search

Whether a wire can keep a deferred tool's schema out of the request is the
adapter's answer, `Provider::defers_tools(model)`, never a provider name
in the agent loop. A leg whose adapter says yes is sent the loaded and the
deferred tools; any other leg is sent the loaded tools only, and the model
reaches the rest through `find_tools`
(docs/design/68-context-engine.md §5).

| Wire | Deferred tool | Search entry | What comes back |
|---|---|---|---|
| Anthropic | `defer_loading: true` | `tool_search_tool_regex_20251119` | `server_tool_use`, `tool_search_tool_result` blocks |
| OpenAI Responses | `defer_loading: true` | `{"type": "tool_search"}` | `tool_search_call`, `tool_search_output` items |
| every other wire | not sent | none | nothing |

What comes back is kept as an opaque `ContentBlock::Provider` block in the
ledger and sent again only on the wire that wrote it. The Responses
adapter sends its search items in order, ahead of the call they loaded a
tool for, and without their `id`: an item sent by id is refused unless the
reasoning item before it comes too, and reasoning is not replayed there.
They go only while the request still carries a deferred tool.

No table says which models take tool search. A 400 that names it
(`models::rejects_tool_search`) is retried once with the loaded tools
only, and that model on that endpoint is remembered for the life of the
process (`models::mark_tool_search_unsupported`).

Measured on `gpt-6-luna` through the server (2026-10-08): the entry and
18 to 22 deferred tools were accepted on every request. Asked to use its
own search, the model did on 10 of 10 turns, and all 37 later requests
that carried the search items were accepted. Left to choose, with
`find_tools` loaded and named in the prompt, it called `find_tools` on 10
of 10 turns and its own search on none. A deferred tool still costs its
name and description in every request: about 2,500 input tokens for 20
tools.

### Anthropic: reasoning depth and fast mode

The adapter never sends `thinking: {type: "disabled"}` or an explicit
`budget_tokens` — current models either reject it outright or silently
degrade tool-call reliability under it — so `thinking` is simply omitted,
which runs adaptive reasoning on every current model. `effort` is the one
supported dial: `ChatRequest.effort` renders as `output_config.effort`, and
`think == Some(false)` (a side-dispatch that wants a fast, cheap answer —
classify, compaction, handoff, plan) maps onto its lowest level only when
the caller has not already asked for a specific one. An `output_config`
400 on a model that does not support it is a one-time, per-model fallback:
the request is retried once without `effort`, and the model is marked
unsupported so later requests skip straight to the fallback. Opt-in
`[providers.anthropic] fast_mode` sends `speed: "fast"` on models discovered
to support it (support is looked up once per model id, in the background,
never blocking the request that triggered it); a 429 while `speed` is set is
retried once without it, and a further failure returns the mapped error
normally.

## Model discovery

The built-in configuration selects no provider or model. First run and the
in-app service picker ask the operator to choose a registered provider, add a
key when that provider requires one, discover its live catalogue, and choose
the model; the same flow remains available from Settings.

There is no hardcoded model catalogue. Which models exist is a property of
the user's key, so `models.rs` asks the provider and
`Core::discover_models` memoises the answer for 5 minutes (invalidated
whenever a key is stored or revoked).

| shape | providers | request | response |
|---|---|---|---|
| OpenAI listing | `openai`, `openai-responses`, `openrouter`, `openrouter-responses`, compatible gateways, `ollama` | `GET {base}/models`, bearer | `{ data: [{ id }] }` |
| Anthropic | `anthropic` | `GET {base}/v1/models`, `x-api-key` + `anthropic-version` | `{ data: [{ id }], has_more, last_id }` |
| Google | `google` | `GET {base}/models?key=…` | `{ models: [{ name: "models/x" }], nextPageToken }` |
| OpenAI-compatible gateway (including Bedrock Mantle) | configured endpoint | `GET {base}/models`, bearer | `{ data: [{ id }] }` |

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
into the canonical shared secret scope, resolved through
`vak_config::credentials` to an OS-native secret service or an
encrypted-file fallback (docs/design/44-shared-config.md, "Secrets
Chain") — never a plaintext file; `remove_provider_key` strips the entry,
clears the runtime override and the loaded process-env cache, and reports
`shadowed_by_env` when the variable is *also* exported in the real
environment — that copy cannot be unset from inside the app, and the
provider stays authenticated. Both paths drop the cached provider client
and the discovered-model cache.

### Endpoint capability negotiation

A usable route is `credential + provider + endpoint dialect + model +
requested capabilities`, not merely a provider/model pair. `RouteLeg` fixes
the dialect, credential and model for a turn. A later retry or fallback
cannot accidentally change a request from a native Messages,
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
`OPENROUTER_API_KEYS`, or a configured gateway-key variable. Values are comma- or
newline-separated; the singular variable remains the primary credential.
VAK fingerprints each credential without storing or returning the secret,
discovers models separately per credential, and records the selected
fingerprint for each dispatched turn leg. Pool identity is distinct from
provider quota identity: provider-reported organization, project, workspace,
model-class, and account limits remain separate observations and are never
assumed to be per-key.

Secrets live in the project or Shared secret scope, never in a plaintext
file on disk — resolved through `vak_config::credentials` (OS-native
secret service, or an encrypted-file fallback where none is reachable);
real environment variables always take precedence. Never commit keys.

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
