# 30-render-architecture — Cross-surface rendering architecture

Status: **implemented contract, with explicitly listed future extensions**. The
schema-v2 semantic timeline, result-scoped eligibility/evidence metadata,
collaborative goal projection, deterministic fallback, and cross-surface
delivery path are shipped in 3.0.10. The open questions at the end are not
claims about missing core behavior.

This document specifies the rendering architecture for vak's output. Result
eligibility, evidence state, and collaborative goal state come from the shared
outcome-directed runtime; this document owns only semantic projection and
surface rendering.

Scope boundary: everything below is ledger → projection → delivery, ending at
the `DeliveryPacket` / `OutputTimeline` the surface receives. What the native
(desktop/web) client does with a `Structured` or `Adaptive` item after that is
`docs/design/57-adaptive-presentation-runtime.md` — the closed primitive
vocabulary and runtime-pluggable packs — and the contributor walkthrough for
adding or changing a renderer is
`docs/design/67-presentation-renderer-guide.md`. There are no per-semantic-type
client components; one generic renderer consumes the primitive tree.
It is the design contract that the implementation follows,
replacing the ad-hoc wiring that left `project_structured_fences` as dead code
and `DeliveryPacket.presentation` as inert data.

## Problem statement

Three forces pull in different directions:

1. **Provider/model diversity.** Each turn walks a freshly planned route
   ladder across Anthropic, OpenAI, Google, and local providers. Turn-by-turn, the model
   that answered is not the model that will answer next. Raw provider JSON
   shapes (`choices[0].message.content` vs `content[0].text` vs
   `candidates[0].content`) are irreconcilable per-provider. Writing N adapters
   for N providers is an unbounded maintenance nightmare.
2. **Surface capability divergence.** Telegram accepts 7 HTML tags, 4096 chars,
   no headings/tables. Slack sends mrkdwn with `*bold*` / `_italic_` /
   `<url|text>`, no headings/tables, ~4000 chars. Discord passes GFM but no
   hyperlinks in plain messages, no tables, 2000 chars. Desktop renders a
   schema-v2 semantic AST with SolidJS components. Each surface's
   protocol limits are immutable walls, not styling preferences.
3. **Durability without duplication.** The session ledger (JSONL) is the sole
   source of truth. Formatting must not become a second source of truth.
   Every rendering decision must be loss-accounted (native / fallback), and
   every rendered output must carry its `fallback_markdown` so a renderer bug
   never loses data.

## Design principles

- **Self-declaration over inference.** Structured data enters the pipeline
  through a normalized envelope (`semantic_type` + `payload`). The envelope is
  model-agnostic and provider-agnostic — any model can emit it because the
  format is the same regardless of which provider wrapped the text content.
- **One projection path.** The model output → `AnswerDraft` → `DeliveryPacket`
  pipeline produces channel-specific text (chunks) AND an optional semantic
  timeline (presentation), but the text projection is always derived from the
  same source through one function: `project_structured_fences()`.
- **Coverage everywhere.** Every block in the compiled document carries a
  `Coverage` disposition: `Native` (rendered with full surface semantics) or
  `Fallback` (downgraded to text). Nothing is silently dropped.
- **The host defines the vocabulary.** `DocumentBlock` (15 variants),
  `InlineNode` (9 variants), and `OutputContent` (9 variants) are closed enums
  owned by the host. Skills and plugins classify their data against this
  vocabulary — they never extend it.
- **Capabilities are data classified against vocabulary.** Recipes, signal
  matchers, and renderer bindings are plugin-contributed data, not hardcoded
  tables. The recipe catalog accepts registrations from any installed plugin;
  more specific recipes shadow less specific ones (longest-signal-prefix
  wins, like routing tables).
- **Level-triggered reconciliation.** Capability changes (new plugins, skill
  updates, renderer bindings) propagate at the next turn boundary via an
  immutable epoch published by the snapshot layer. Revocations take effect
  immediately and fail closed.

## Architecture: the rendering stack

```
┌─────────────────────────────────────────────────────────────┐
│  Session Ledger (JSONL) — the immutable source of truth     │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐  │
│  │ Message      │    │ Activity     │    │ ToolResult   │  │
│  │ (markdown)   │    │ (approval)   │    │ (structured) │  │
│  └──────────────┘    └──────────────┘    └──────────────┘  │
└─────────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────┐
│  Projection Layer (vak-server/src/projection.rs)              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ snapshot() / live_event() / project_frame()          │   │
│  │                                                      │   │
│  │ 1. compile_markdown(source) → PresentationDocument   │   │
│  │ 2. parse_fragment() on ```vak fences → Structured   │   │
│  │ 3. signals_from_text() / signals_from_context()    │   │
│  │ 4. PresentationPlanner::plan(signals) → recipe     │   │
│  │ 5. DocumentCoverage: Native / Fallback per block   │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────┐
│  Delivery Layer (vak-delivery)                               │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ render(job) → DeliveryPacket                         │   │
│  │                                                      │   │
│  │   AnswerDraft                                         │   │
│  │   ├── source_markdown (exact, preserved)              │   │
│  │   ├── document: PresentationDocument (semantic AST) │   │
│  │   └── metadata: recipe_id, signals, coverage        │   │
│  │                                                      │   │
│  │   render_content()                                    │   │
│  │   ├── Non-native surface:                             │   │
│  │   │   project_structured_fences(source) → text      │   │
│  │   │   → markup_to_html/mrkdwn/discord(text)        │   │
│  │   ├── Native surface (Json):                         │   │
│  │   │   output_document directly as JSON             │   │
│  │   └── Native surface (Plain):                        │   │
│  │       render_plain(document) → text projection     │   │
│  │                                                      │   │
│  │   chunk_text / split_html_chunks /                  │   │
│  │   chunk_markdown_preserving_fences                  │   │
│  │                                                      │   │
│  │   coverage: block_id → Rendered/Fallback           │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────┐
│  Surface Projections                                         │
│                                                              │
│  chat (telegram/s/l) → DeliveryPacket.chunks                  │
│     └─ sequential channel messages, actions on final chunk  │
│                                                              │
│  desktop/web       → /sessions/{id}/presentation SSE          │
│     └─ OutputTimeline with OutputContent (Document,          │
│        Structured, Artifact, Approval, etc.)                 │
│                                                              │
│  webhook           → DeliveryPacket (JSON payload)            │
│                                                              │
│  log               → DeliveryPacket.chunks (plain text)       │
└─────────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────┐
│  Durability Layer (vak-session/src/effects.rs, M4.5)         │
│  effects/ chain: prepared → dispatched → sent | failed |     │
│  unknown → reconciled; payload a tenant object; 10 attempts │
└─────────────────────────────────────────────────────────────┘
```

## Component specifications

### 30.1 The Normalized Output Envelope

```rust
// The only model-authored rich block envelope
pub struct StructuredOutput {
    pub semantic_type: String,   // must be declared by an installed skill
    pub schema_version: u16,     // PRESENTATION_SCHEMA_VERSION
    pub skill_id: String,         // who provided this type
    pub skill_version: String,    // content-addressed for determinism
    pub payload: Value,           // validated against the skill's schema
}
```

The envelope is recognized in three input forms:

| Form | Where | Extraction |
|---|---|---|
| ```vak fence | Model text | `project_structured_fences()` finds ```vak blocks |
| Bare JSON | Tool output / model text | `parse_fragment()` tries `serde_json::from_str` |
| Tool declared | Tool wrapper output | `structured_outputs_from_tool_result()` checks `semantic_type` field |

**Provider-agnostic invariant:** We never parse raw provider response JSON
(`choices.[0].message.content`). The provider wraps text content; we unwrap
one level to get text, then look for our envelope within that text. The
envelope format is identical regardless of provider.

**Preferred emission path: `emit_*_card` tool calls.** The "Tool declared"
row above is how `vak-core/src/presentation_tools.rs`'s twelve
`emit_*_card` tools reach this pipeline — one tool per payload *shape*
(not per `semantic_type`; siblings that share a shape, e.g. every
timeline-flavored type, share one tool; the twelve carry all 97 types).
`execute()` turns the call's arguments into the stored card (each shape's
`build`), validates it against `SkillRegistry`, and replies with a short ack
(or a repairable tool error naming the field); the card is written as a
`Presentation` ledger entry at that moment and never carried in result text,
of which an over-long result's request carries only a window
(docs/design/68-context-engine.md §3). A tool call is the *preferred* path
over writing a ```vak fence directly (`system-prompt.md`,
`docs/design/07-prompt.md` v3.4.5): measured against gemma4:e2b-mlx via
Ollama, a free-text fence parsed as valid JSON only ~20% of the time. The
fence path remains the fallback for a turn where no matching tool is present,
and a fence carries exactly the arguments the tool would take.

**The card argument contract.** A tool call is not constrained by its schema
everywhere: Ollama samples a local model's call syntax freely and drops a call
it cannot parse without telling the client, so the argument shape itself has
to be one every model writes reliably. Measured 2026-10-07 against
gemma4:e2b-mlx (20 runs per tool and per shape; the model wrote the same
nested data as plain JSON in a reply 14 times in 15, so the weak point is the
tool-call encoding, not JSON or the size of the data), every card tool's
arguments follow six rules, checked by
`every_card_schema_is_flat_and_portable`:

1. **No envelope.** The card's fields sit at the top level beside
   `semantic_type`, which a tool whose name settles the type lets the model
   leave out. 13 of 16 dropped calls had left a `payload` wrapper open; with
   the wrapper gone, a UI preview went from 1 to 20 calls in 20 and a recipe
   from 7 to 19.
2. **Only the keywords every provider carries:** `type` (one type, never a
   union), `properties`, `items`, `required`, `enum`, `description`. Ollama's
   tool schema type drops `oneOf`, `additionalProperties` and bounds, so a
   shape stated only through them reached the model as a bare object; bounds
   and union shapes are checked by `build`, with a message naming the field.
3. **No data in keys.** Labelled details and metric readings are lists of
   `{label, value}`; a label such as `Year Built` is not a key every call
   syntax can write.
4. **No list of records inside a record in a list.** Nesting is expressed as
   flat lists joined by a name, as a database would: a timeline's options are
   their own list naming their step, a chart's points name their series. A
   timeline went from 12 of 20 (options inside items) to 16 of 20 valid with
   none dropped. A table is its rows (rule 6); a nested value carried as one
   JSON string was no better (9 of 15).
5. **`semantic_type` is required and comes first**, even where the tool's
   name settles it (`build` still fills the default when it is missing). A
   call's first argument sets its syntax: a short enumerated value written
   first is written in the model's own string delimiter, and the long values
   after it follow. With `semantic_type` optional the model opened with the
   long value and malformed it.
6. **No value is asked to start with `|`.** Gemma's string delimiter is
   `<|"|>`; a Markdown table opening with `|` fused into the single token
   `="|` (p = 0.86 at that position, against 0.04 for the delimiter), which
   Ollama's parser drops. A table is a list of row strings, header first,
   cells separated by `|` (`Name | Country`), parsed into the stored columns
   and rows.

The stored card, and every renderer, pack, digest and channel that reads it,
is unchanged by the contract: `build` is the only place the two meet.

**How rules 5 and 6 were found.** One captured request ("three famous
lighthouses … as a table card", Vak's full 12.5k-character system prompt, 17
tools) got a valid `emit_table_card` call in 4 of 20 runs on gemma4:e2b-mlx;
the rest were written `table="| …`, which Ollama drops (its log: `gemma4
tool call parsing failed`). Removing prompt blocks one at a time changed
nothing reliably, and padding a short prompt with harmless text broke it, so
the prompt's length only tipped a balance. Token log-probabilities at the
decision point showed the balance: after `call:emit_table_card{table` the
model chose `="|` with p = 0.86. With a one-line prompt it had opened with
`semantic_type` instead, and its correct delimiter carried into `table`.
Measured 2026-10-07, 20 runs each on the full request: `semantic_type`
required, 15 of 20; rows as a list of strings, 18 of 20; both, 20 of 20;
the shipped schema and description, 30 of 32 (2 refused by a check stricter
than `build`).

**Presentation check.** Prompt guidance is advisory, and a strong model can
still answer in prose what the app would have shown as a card. After the
turn-loop's other repair checks, `Agent::run` asks an optional
`AgentConfig::presentation_check` (set by `Core`) whether the final text reads
as a card: `signals_from_text` → `RecipeCatalog::intended_outputs` (the best
non-default recipe whose signals all match, independent of which outputs exist)
→ the offered `emit_*_card` tool that carries one of its primary types. If no
card was emitted this run and no inline fence is present, the model gets one
`[presentation-check]` nudge; it may decline by resending unchanged. No
per-type rules live in the loop.

### 30.2 The Semantic Compiler

`compile_markdown()` in `presentation.rs` uses `pulldown-cmark` with
tables + strikethrough + tasklists + footnotes. It produces a closed
`Node` tree, then projects to `DocumentBlock` variants:

```
Markdown → pulldown-cmark Node → DocumentBlock (15 variants)
                                    └─ Structured { output: StructuredOutput }
                                        created when a ```vak fence yields
                                        a validated StructuredOutput
```

The compiler preserves exact source markdown in
`PresentationDocument.source_markdown` and records coverage:
- `Native` — the block renders with full surface semantics
- `Fallback` — the block degrades to text (e.g., a table on Telegram becomes
  a monospace grid inside `<pre>`)

`unsafe_links_and_html_remain_inert` test confirms `<script>` and
`javascript:` URLs are preserved but flagged `safe: false`.

### 30.3 Structured Fence Projection

THIS IS THE CORE FIX. `project_structured_fences()` (skills.rs) replaces
validated ```vak fences with their deterministic text projection
(`structured_markdown()`). It is called on the source markdown **before**
markup conversion, but **only for non-native surfaces** (Telegram, Slack,
Discord — not desktop, tui, admin, or webhook/json).

```
render_content(job):
  rendered = render_answer(answer, markup)
  if !surface_capabilities(markup).structured_blocks:
      rendered = project_structured_fences(&rendered)
  chunks = chunk(rendered, max_chars)
```

This ensures that whether the ```vak fence was produced by GPT-4o via
OpenAI's API, Anthropic's API, or Gemini via Google's API,
the chat surface sees the same readable text projection — because the
envelop format is the same regardless of provider.

`structured_markdown()` projects by semantic type:
- `metric` → `label: value unit`
- `link.preview` → `title\nurl`
- `chart` → `accessible_summary` (the textual alternative, never the raw series data)
- `terminal.view` → `Command: ...\nExit: ...`
- All types → a ```` json appendix with the full validated payload (inspectable, never lost)

### 30.4 The Recipe / Signal System

`signals_from_text()` performs deterministic pattern matching on lowercase
text and tool provenance context. It returns a `SignalContext`:

```rust
pub struct SignalContext {
    pub text_signals: Vec<String>,      // "diff", "tests", "temperature", ...
    pub tool_signals: Vec<String>,      // "edit" → ["diff", "files_changed"], ...
    pub provenance_signals: Vec<String>, // workspace, session, surface
}
```

`PresentationPlanner::plan()` matches signals against `RecipeCatalog`:

```rust
pub struct PresentationRecipe {
    pub id: String,
    pub version: String,
    pub match_signals: Vec<String>, // every declared signal must be present
    pub primary_types: Vec<String>, // required semantic types for this composition
    pub surfaces: Vec<String>,      // eligible targets; never renderer bindings
}
```

Renderer resolution is a separate per-block operation. A recipe chooses
composition intent; the validated skill registry chooses a renderer for each
structured block against the target surface. The persisted plan therefore
contains one `RendererDecision` per block plus a summary (`markdown:native`, a
single renderer id, or `mixed`). `builtin:generic` is an explicit surface
fallback, never a recipe default.

**Plugin extension point:** `RecipeCatalog::register(plugin_recipe)` allows
any installed plugin to contribute recipes. Matching requires every declared
signal and every required primary type, then ranks candidates by an explicit
specificity/priority tuple with stable id/version tie-breaking. Renderer
bindings are never supplied by a recipe. The built-in catalog is the default
register; plugin recipes are additive.

**Signal extensibility:** Plugins contribute signal definitions (keyword
patterns, tool-name mappings) to the `SignalRegistry`. The core `signals_from_text()`
merges builtin + plugin signals on each call. This is level-triggered — a new
plugin's signals appear on the next `snapshot()` call.

### 30.5 Surface Capability Profiles

Each surface declares its `SurfaceCapabilities`:

| Surface | structured_blocks | tables | code | links | actions | media | color | interactive | max_chars |
|---|---|---|---|---|---|---|---|---|---|
| telegram | false | false | true | true | yes* | false | false | false | 4096 |
| discord | false | false | true | true | false | false | false | false | 1900 |
| slack | false | false | true | true | false | false | false | false | 3900 |
| desktop | true | true | true | true | true | true | true | true | None |
| tui | true | true | true | false | true | false | true | true | None |
| admin | true | true | true | true | true | true | true | true | None |
| webhook(json) | true | true | true | true | true | true | true | true | None |
| log (plain) | false | false | true | false | false | false | false | false | None |

\* Telegram's `supports_actions` is `false` for per-turn replies (the bridge
handles inline keyboards directly) but `true` for proactive `TelegramAdapter`
delivery. This split is intentional and documented here.

### 30.6 The Delivery Packet

```rust
pub struct DeliveryPacket {
    pub schema_version: u16,
    pub job_id: String,
    pub target: String,        // surface:address[:bot_id]
    pub surface: String,
    pub kind: DeliveryKind,
    pub payload: DeliveryPayload,
    pub fallback_markdown: String,  // exact source, never lost
    pub chunks: Vec<String>,       // channel-formatted chunks
    pub actions: Vec<DeliveryAction>,  // inline keyboard buttons / typed prompts
    pub coverage: Vec<Coverage>,   // block_id → Rendered/Fallback
    pub diagnostics: Vec<String>,  // loss-accounting notes
    pub presentation: Option<OutputTimeline>, // semantic timeline (desktop)
}
```

**Key invariant:** `presentation` is computed for every packet, but only
native surface consumers (desktop/web SSE) actually use it. Chat surfaces
consume `chunks` — the projected text with structured fences replaced by
`structured_markdown()` output. This is by design, not a bug: chat surfaces
can't render semantic AST nodes, so the presentation timeline for them is
purely for auditability.

### 30.7 The Durability Layer

Every delivery is an effect (`vak_session::effects`, plan M4.5, AGENTS.md
invariant 41):
- An `effects/` record chain in the shared scope; the job is a tenant object.
- Steps: `prepared` (with an optional `hold`) → `dispatched` → `accepted` |
  `confirmed` | `failed { proven_not_sent }` | `unknown` → `reconciled` or
  `superseded`.
- One dispatcher per attempt: the process that moves `eff/<key>` under the
  writer epoch.
- `dispatchable()` returns oldest-first (no starvation); replay takes 100.
- Replay interval: 30s. Retry budget: 10 attempts for failures proven not
  sent; an unknown outcome is never retried by itself.
- A row the reader cannot decode is an error, never skipped.
- `fallback_markdown` is always in the packet, so a delivery failure never
  loses the exact source.

### 30.8 The Worker Isolation Boundary

The `vak-delivery-worker` subprocess:
- Runs the **current executable** with `__delivery_worker` as argv[1]
- `env_clear()` — zero environment variables, no transport credentials,
  no network
- Communicates via line-oriented JSON (protocol v1) over stdin/stdout
- 5s timeout per job, one retry with a fresh process
- Falls back to in-process rendering with a diagnostic flag if unavailable
- Receives only the `DeliveryJob` (serialized JSON); no Core handles, no
  policy engine, no provider credentials

This ensures the rendering worker cannot leak secrets or make network
requests, regardless of which model or provider produced the original output.

## Changes from the previous implementation

1. **`project_structured_fences()` now called for all non-native markup** (Gap 1, Gap 3).
   Previously dead code — now wired into `render_content()` before markup
   conversion. Chat surfaces receive readable text projections of structured
   outputs instead of raw JSON envelopes.

2. **`PresentationPlanner` accepts plugin-contributed recipes** (Gap 6).
   Previously hardcoded builtins; now merges builtin + plugin recipes via
   `RecipeCatalog::register()`. Signal extraction merges builtin + plugin
   signal matchers.

3. **Delivery posture integration** (Gap 7).
   `DeliveryPosture` (cadence × urgency) now consulted by `render_response()`
   before it prepares or sends, holding the effect when appropriate.

4. **`DeliveryContent::{Text, Progress, ToolResult}` go through markup conversion** (Gap 5).
   Previously these bypassed the markup converters entirely. Now all content
   kinds pass through the same `project_structured_fences` → markup pipeline.

5. **Telegram `supports_actions` split is documented** (Gap 4).
   The difference between `render_response` (per-turn reply, no actions) and
   `TelegramAdapter` (proactive push, with actions) is now an explicit,
   documented design decision, not an inconsistency.

## Testing strategy

| Layer | Tests |
|---|---|
| Compiler | `compiles_commonmark_without_losing_source`, `tight_lists_keep_inline_runs_together`, `unsafe_links_and_html_remain_inert`, `fenced_diff_receives_a_native_block`, `mermaid_is_preserved_as_a_safe_diagram_source` |
| Structured projection | `project_structured_fences_replaces_vak_blocks`, `project_structured_fences_leaves_plain_code_intact`, `structured_markdown_renders_all_known_types`, `project_structured_fences_never_panics_on_malformed` |
| Markup converters | Telegram: `converts_links_to_angle_pipe_form`, `rewrites_links_with_a_visible_suppressed_url`, `split_html_chunks_balances_tags`, `passes_through_native_markdown`; Slack: `bold_and_italic_converted`, `links_as_angle_pipe`, `tables_as_code_block`; Discord: `links_wrap_url`, `tables_as_code_block` |
| Chunking | `chunking_is_unicode_safe_and_prefers_line_boundaries`, `split_html_chunks_closes_and_reopens_tags`, `chunk_markdown_preserving_fences_reopens_fences` |
| Posture | `approval_always_sends`, `interrupt_overrides_all_cadences`, `alerts_never_held_for_digest`, `on_completion_holds_progress` |
| Integration | `persistent_worker_renders_multiple_jobs` (worker process test), golden fixtures for cross-surface rendering of a representative document |
| Coverage | Every `project_structured_fences` call adds `Coverage::Rendered` entries for the replaced blocks |

## Open questions / future work

- **Cross-surface golden fixtures:** A test harness that renders the same
  `AnswerDraft` on all chat surfaces and asserts semantic equivalence (same
  text content, different formatting). This is an additional regression
  harness, not a prerequisite for the shipped Telegram action split.
- **Streaming to chat surfaces:** Push `TextDelta` events to chat surfaces
  during model generation, not just final-state delivery. Requires a
  streaming delivery posture and per-channel connection state.
- **Grapheme-cluster-aware chunking:** `unicode-segmentation` dependency to
  avoid splitting emoji sequences. Documented limitation, not yet prioritized.
- **TUI projector:** A terminal renderer for the `OutputTimeline`.
  `SurfaceCapabilities` already lists `"tui"` as native; the renderer is
  planned (design/30 item 8).
