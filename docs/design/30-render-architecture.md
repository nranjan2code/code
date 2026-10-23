# 30-render-architecture — Cross-surface rendering architecture

Status: **implemented contract, with explicitly listed future extensions** (checked against the code on 2026-10-14). The
schema-v2 semantic timeline, result-scoped eligibility/evidence metadata,
collaborative goal projection, deterministic fallback, and cross-surface
delivery path are shipped in 3.0.10. The open questions at the end are not
claims about missing core behavior.

> The 2026-10-23 consolidation is recorded in
> [the rich response contract](104-rich-response-contract.md): ordered answer
> selection is `vak_delivery::answer`, the stream reducer is in
> `vak_delivery::presentation`, and surface consumers carry original accepted
> items. Old split prose/card handoffs and separate transcript renderers are removed.

> Native channel form rendering is specified in docs/design/102-channel-cards.md;
> the non-native card-to-Markdown path drawn below is its historical predecessor.
> The card vocabulary, the client registry and the signal and recipe
> planning described here are replaced by docs/design/92-card-forms.md
> (2026-10-14): eighteen forms, drawn by block, nothing decided by words.


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
2. **Surface capability divergence.** Native cards and ordinary text have
   different protocol contracts. Telegram rich messages support headings,
   tables and disclosure; Slack has native rich-text and table blocks;
   Discord has embeds. Basic narration codecs retain their channel-specific
   text constraints. The form-aware channel compiler and current limits are
   specified in [Native channel cards](102-channel-cards.md).
3. **Durability without duplication.** The session ledger is the sole
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
│  Session Ledger — the immutable source of truth             │
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

**Preferred emission path: card tool calls.** The model shows each card by
calling the card tool that owns its kind (docs/design/87-composed-answers.md
§2), one of `vak-core/src/presentation_tools.rs`'s card tools, one per
payload *shape*, by its own name: its `build` turns the call's arguments
into the stored card, validates it against `SkillRegistry`, and a failure is
a repairable error naming the field. The card is written as a
`Presentation` ledger entry (`PresentationSource::ToolCall`) and never
carried in result text, of
which an over-long result's request carries only a window
(docs/design/68-context-engine.md §3). A call is the *preferred* path over
writing a ```vak fence directly (`docs/design/07-prompt.md` v3.4.5):
measured against gemma4:e2b-mlx via Ollama, a free-text fence parsed as
valid JSON only ~20% of the time. The fence path remains the fallback for a
turn where no card tool is admitted, and a fence carries exactly the
arguments the block would take.

Before schema validation, the shared tool-call boundary restores an array or
object that a provider returned as a JSON string when the admitted schema
requires that collection type. Tool canonicalizers then normalize their
documented provider-compatible forms, and authorization validates the final
arguments against the original admitted schema before dispatch. This recovers
representation drift without widening the schema or bypassing validation.

**The card argument contract.** A tool call is not constrained by its schema
everywhere: Ollama samples a local model's call syntax freely and drops a call
it cannot parse without telling the client, so the argument shape itself has
to be one every model writes reliably. Measured 2026-10-07 against
gemma4:e2b-mlx (20 runs per tool and per shape; the model wrote the same
nested data as plain JSON in a reply 14 times in 15, so the weak point is the
tool-call encoding, not JSON or the size of the data), every card block's
arguments (its card tool's schema) follow eight rules, checked by
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
   name settles it; such a tool fills a missing type before the call is
   validated (`Tool::fill_defaults`), and a refusal for a missing choice
   lists the allowed values. Live, the universal card was refused for a
   missing type with nothing telling the model what the types were. A
   call's first argument sets its syntax: a short enumerated value written
   first is written in the model's own string delimiter, and the long values
   after it follow. With `semantic_type` optional the model opened with the
   long value and malformed it.
6. **No value is asked to start with `|`.** Gemma's string delimiter is
   `<|"|>`; a Markdown table opening with `|` fused into the single token
   `="|` (p = 0.86 at that position, against 0.04 for the delimiter), which
   Ollama's parser drops. A table is a list of row strings, header first,
   cells separated by `|` (`Name | Country`), parsed into the stored columns
   and rows. The schema describes each row as one complete string with the
   same number and order of cells as the header. For a budget, its Amount,
   Allocation, Cost or Price column carries currency values, and its final
   Total row reconciles with them. An example in the field description makes
   that alignment explicit; runtime validation still checks the submitted
   rows. The metric emitter is for observations and related measurements; an
   itemized cost, expense or budget allocation belongs in a table with
   `semantic_type: budget`, even when it also has a total. The metric tool's
   first sentence states this boundary because it is also what a deferred
   emitter catalogue shows the model. In the ordered timeline shape,
   `schedule` requires a duration for
   each phase. `timeline` requires a duration for each phase when an overall
   duration is stated (also when it appears unambiguously in the card title),
   and runtime checks their sum; without a total, each timeline item needs a
   supplied time or duration. `milestones` and `checklist` remain untimed.

The stored card, and every renderer, pack, digest and channel that reads it,
is unchanged by the contract: `build` is the only place the two meet.

7. **One text field per record, and no record ends in a number.** A
   research card is its sources, each with the one-sentence `finding` it
   supports; `build` turns each finding into a takeaway citing its source.
   A separate takeaway list numbered by source ended each record in an
   integer, and the model left that record unclosed (`…<|"|>,source:5]`,
   7 of 12 dropped calls); with `snippet` and `takeaway` side by side it
   filled only one. On a captured request after a 50k-character page,
   3 of 8 calls arrived before and 8 of 8 after.

8. **A list item is a string where the card allows it.** Records inside a
   list are where the call syntax slips (a value closed with `"` instead
   of the delimiter, a `}` left out after a number). A table's rows are
   complete, aligned strings such as `A | B`, a universal card's details `Label: value` (split at the first
   colon), a recipe's ingredients and steps plain text, which the stored
   recipe already accepts. Measured on captured requests on 2026-10-07: an
   overview card 8 of 12 as records, 12 of 12 as strings; a recipe 9 of 12
   and 12 of 12. Timeline items and chart points, measured since, stay
   records (rule 10).
9. **One field for one thing.** A card never asks for the same fact twice
   or offers two fields the same value fits. The media card asked for its
   kind as `semantic_type` and again as `media_type`, and for an address
   as `url` (a link preview), `source` (media) and `image_url` (a
   preview's thumbnail). Measured on captured requests on 2026-10-07, 12
   runs each: image, video and audio cards were valid 0 of 12 each (the
   kind went in `media_type` and `semantic_type` was left out in 19 of
   36; the address went in `url` or `image_url`). Without `media_type`:
   1, 2 and 3 of 12. With one `url`: video 12, audio 11, image 2 (the
   address in `image_url`). With the thumbnail named `thumbnail_url`:
   image, video, audio and link preview each 12 of 12 with their type and
   `url`. The tool now takes
   `semantic_type`, `url`, `alt`, `title`, `description`, `site_name` and
   `thumbnail_url`; the stored card's `source`, `media_type` and
   `image_url` are derived. What a media card shows is its `alt`, else
   its title or description, else the file's name (5 of 12 image calls
   gave an address alone); a link preview with no title (6 of 12 before,
   2 of 12 after) is titled by its site.

10. **Ask only for what the model's data does not already say.** A
   required field the model leaves out costs a refused call. Measured on
   captured requests on 2026-10-07, 12 runs each: 20 of 22 chart calls
   left out the required `accessible_summary`, with every point and the
   `chart_type` right in all 22. The sentence is now optional, and a chart
   without one gets it from its own points (each series' first and last
   point); a chart with neither points nor a sentence is refused. Measured in the same run and
   left as they are: timeline items as records, 35 of 36 valid, and chart
   points as records, valid in every chart call, so rule 8's strings are
   not needed there. A single metric came without its `label` in 5 of 24
   calls; with the field described as "Give it with every value", 1 of 24.
   One reading written in `readings` is the single metric: as a grid of
   one it was refused by the stored card's rule with a message that named
   no field (live 2026-10-08, 3 of 5 calls; 6 of 6 shown once built as
   label, value and unit).

Live follow-up, 2026-10-10: Ollama returned timeline `items` as
`"Day 1: Check soil"` strings on three repair attempts, despite the tool
advertising object items. `vak-agent` now canonicalizes those strings to
`{label, detail}` at the provider-response boundary before the existing
schema validator runs; object-form items pass through unchanged. This is a
narrow compatibility repair for the observed wire-shape drift. The stored
presentation contract and renderer still receive validated timeline records.

For a plan with a requested overall duration, `emit_timeline_card` now accepts
`total_duration_minutes` plus each item's `duration_minutes`. The tool refuses
an incomplete or mismatched sum with the calculated total in its error so the
model can correct the schedule through the normal tool-repair path. Validated
durations are projected into each timeline item's existing `time` field. The
tool schema also tells the model that the renderer shows labels and durations
separately, so item details should not repeat them.

Live Ollama checks on 2026-10-10 found two additional presentation-boundary
issues. A natural question with supplied weather readings first selected the
general-purpose overview emitter despite the registered `weather` metric type;
clarifying both emitter descriptions and the selection rule led the same
prompt to select the metric emitter on the next run. A model-supplied weather
condition label (`Conditions`) and unit (`Condition`) rendered as “Cloudy
Condition”; weather normalization now recognizes the same condition aliases as
the renderer and removes a redundant category label. The same run also exposed
structured final text after a successful card. The completion check now treats
two or more ordered or unordered list items as structured content, in addition
to headings and tables. Its focused tests pass; a live retry should verify that
a numbered Markdown answer is removed or replaced after the control nudge.

Live follow-up on 2026-10-11 with `qwen3.5:4b-mlx` exposed a tool-selection
ambiguity: for a request with a schedule and itemized budget, the model used the
deferred metric emitter for the costs, first guessing string readings and then
repeating a disallowed `title` field after the schema error. The request's
predicted card set included both the timeline and table emitters. This was a
tool-description/discovery issue, before rendering: the metric catalogue
sentence described related readings broadly enough to invite an allocation
breakdown. Its first sentence now directs itemized costs and budgets to the
table emitter, while the budget validator and schema make currency syntax
explicit. Focused tests cover the prediction and both tool contracts; a live
post-change retry is still needed to confirm local-model selection.

Live cross-layer audit, 2026-10-10: the tool, skill and client registries
contained three gaps: the supported social research types had no research
tool route; `coding.search` belonged to the universal detail tool although
its renderer only accepts table or research payloads; and media `url` /
`thumbnail_url` plus title-cased weather readings did not reach the client
fields their renderers read. The research types now use `emit_research_card`,
`coding.search` uses `emit_table_card`, the media adapter accepts the tool's
field names, and weather readings are normalized to the weather renderer's
field names. A renderer coverage test now compares all 109 tool types against
the UI registry and fixture routes.

A live garden-schedule run on 2026-10-10 supplied `time_minutes` for each
timeline item, which the canonical schema rejected; Ollama then fell back to
`data.grid`, rendering the schedule as a three-column table. A second run
supplied `duration`, then corrected itself to `duration_minutes` and emitted
the timeline. The timeline tool now normalizes the observed spellings
`duration`, `time_minutes` and `time_in_minutes` to `duration_minutes` before
schema validation, while keeping the canonical schema and stored payload
unchanged. A rerun on the rebuilt server should verify the alias path.

### Diagnose the complete card path

A card that looks wrong is not automatically a model failure. Follow one call
through these boundaries, using the turn's recorded request and ledgers as the
source of evidence:

1. **Turn and context.** The user directive is the turn anchor. `vak-context`
   keeps the ledger complete while assembling a request from the current turn
   and the one compact TurnCard of the turn it follows. It does
   not fill spare context with related or unrelated history. Older details are
   searched and reopened with `recall` only when needed. The
   prompt resolver adds only the admitted presentation tools and their schemas
   for this turn. Check the assembled request before judging whether the model
   lacked context or a card capability. A selected historical card is a
   compact referent, not new output. A new unrelated request has no historical
   card to re-emit.
2. **Provider response.** The adapter decodes tool calls. Before the assistant
   response is appended, `normalize_response_tool_uses` reads the arguments with the one
   reader every tool shares (`vak_tools::read_arguments`,
   docs/design/04-tools.md), then settles wrappers, card ownership and the
   tool's own `fill_defaults`. This is where provider-specific output drift is
   made canonical; do not silently mutate the saved presentation later.
3. **Admission.** `authorize` validates the normalized arguments against the
   admitted tool schema and checks permissions. A call rejected here never
   reaches `execute`, so builder-only recovery cannot repair it. Keep any safe
   alias or shape normalization before this gate and test the complete
   normalize → schema-validate path.
4. **Execution and semantic validation.** The card tool's shape builder turns
   flat arguments into the renderer payload; `SkillRegistry::validate` checks
   the `semantic_type` and payload. A rejected call returns a field-level
   repair error. A successful call writes a `PresentationRecord` tied to the
   user turn and `tool_use_id`; its tool result is a short acknowledgement.
5. **Ledger and projection.** The append-only ledger is authoritative. The
   server reads that `PresentationRecord` and projects an `OutputKind::Card`
   by `tool_use_id`; it does not recreate a card from tool-result text or
   arguments. A successful acknowledgement with no projected card points to
   this boundary, not to generation.
6. **Client render and delivery.** The client routes `semantic_type` through
   `STRUCTURED_RENDERERS` into a generic primitive renderer. Inspect the
   projection payload, renderer coverage and actual layout before blaming the
   model. Other surfaces use their deterministic capability projection and
   fallback rules in §30.3.
7. **Final answer and turn state.** The short tool acknowledgement tells the
   model which requested distinct cards remain; final narration is checked for
   duplicated card data. Cards retain tool-emission order. Duplicate suppression
   is turn-local; exact repeated cards within one answer collapse, while
   different same-type cards survive. A repair nudge arms replacement of an
   earlier same-type attempt for that answer. A new direct user request resets
   this state; typed control messages do not create turns. The prompt tells the
   model to search indexed history for an explicit older reference no matter
   how many turns back it is, and to preserve the user's requested card order.
   Live Ollama testing found unrelated weather and dashboard cards copied into
   a cleanup request; the historical context was valid recall material, but the
   model needed a clear current-turn output boundary.

This order gives concrete diagnosis: no admitted tool means inspect prompt
capability and tool routing; a call absent from the ledger means inspect
provider decoding, normalization and authorization; a successful ledger record
missing from the snapshot means inspect projection; a valid snapshot rendered
poorly means inspect the type registry, payload adapter and UI. Existing
conversation history remains in the ledger; the context planner decides which
whole-turn summaries and evidence the next request needs (doc 68 §§3–10).

The same live pass found a chart-schema ambiguity: Ollama put `line` in both
`chart_type` (the visual style) and `semantic_type` (the renderer category),
which validation rejected although the request was a valid line chart. The
schema now describes those fields separately, and the chart tool safely
normalizes `semantic_type` values `line`, `bar` or `area` to the default
`chart` category. The visual style remains in `chart_type`.

**A card is identified by the tool it calls.** (Historical: until the
cards became one tool per block, the loop routed a call by its
`semantic_type` to the card tool owning that type, `normalize_card_type`.
Now a stray `semantic_type` in a call's arguments never sends it to another
block, a call is matched by its exact tool name, and only a fence resolves
its block from `semantic_type`; AGENTS.md "Code rules".) The measurement
that led to the routing: Measured on captured requests: a form card was written in full
under `emit_ui_preview_card` in 7 of 8 runs and a transaction card under an
invented `emit_transaction_card` in 7 of 8, with the right `semantic_type`
almost every time; rewording the tool description and the prompt changed
neither. The owner's schema then validates the call, and a refusal names
the item a field is missing from (`arguments.sources[0] is missing required
`title``), which read before as the card's own field. A wrapper object
named for the called tool (`{"metric_card": {...}}`) is removed whatever it
holds.

**Which card owns a type.** A type belongs to the card whose shape fits
its content. `overview`, `summary`, `detail` (one subject described) and
`coding.architecture` (components and how they relate) are universal cards,
labelled details; they were timeline cards, a list of items, and in a live
run on gemma4 every request for one was answered with an entity, document or
graph card instead (decided 2026-10-07). Types whose content is an ordered
list keep the timeline card (`progress` and `status` render items with their
status), and rows and columns keep the table. `faq` is a universal card
too, each detail written `Question? Answer`. Measured on a captured request
on gemma4, 12 runs each: as a timeline card 0 of 12 (the model reasoned
that an FAQ is the general-purpose card, which was not loaded, and stopped
or called a card it was not offered); as a universal card with `Label:
value` details 2 of 12, and 1 of 12 when the description said `Question:
answer` (the string ended at the question mark); as a table 1 of 12; with
`Question? Answer`, all 12 chose the card and the type, 8 as written, 2 as
`Question:` and `Answer:` fields in turn, which the builder pairs, and 2
with questions alone, which the refusal names. A question in one field with
its answer in the next is paired the same way. The refusal of questions
alone quotes the field and shows a pair: replayed on the captured repair
step 12 times each (2026-10-08), naming the form alone got an answered
card 4 times and the same bare questions again 7 times; quoting the field
with an example pair got 8. Rewording the `fields` description with the
same example changed nothing on the first call (7 of 12 against 6).

**Citations.** A card cites only what this conversation holds. Every `url`
a card carries (a media card's only when it is a web address, since it may
be a workspace path) must appear in
the person's own messages or in a successful non-card tool call's arguments
or result, its whole evidence body included (`SessionLog::evidence_text`);
a failed fetch's URL is not evidence. A card citing anything else is not
shown: the call is refused with which URL nothing retrieved
(`[uncited-source]`, `vak_control::uncited_source`), to fetch it or answer
in text, and goes through the one correction ladder like any refusal; no
runtime ending replaces the answer (docs/design/88-control-plane.md §5). Found live: with no search tool configured, a
research card's sources were written after a fetched search-results page
that contained none of them.

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

**Presentation check (removed in docs/design/88-control-plane.md C5; kept as history).** Prompt guidance is advisory, and a strong model can
still answer in prose what the app would have shown as a card. After the
turn-loop's other repair checks, `Agent::run` asks an optional
`AgentConfig::presentation_check` (set by `Core`) whether the final text reads
as a card: `signals_from_text` → `RecipeCatalog::intended_outputs` (the best
non-default recipe whose signals all match, independent of which outputs exist)
→ the offered `emit_*_card` tool that carries one of its primary types. If no
card was emitted this run and no inline fence is present, the model gets one
`[presentation-check]` nudge; it may decline by resending unchanged. No
nudge follows a run whose every retrieval failed (a fetch answering 403 is
an error value, never a page): the honest answer reports the failure, and a
card would have no data. No
per-type rules live in the loop.

**Card completion check (removed with the presentation check; kept as
history).** The model now plans its deliverables and the plan check names a
missing card (docs/design/86-model-plans-runtime-checks.md). As it was: after
a tool-emitted card, structured assistant
text (headings with lists, tables, or several sections) signals that requested
parts may still be in prose. The loop records one `[card-completion-check]` in
the same logical turn and asks the model to emit matching cards for remaining
parts and repair incomplete cards. Ordinary assistant text remains part of the
answer projection while the check runs. A textual prefix has no control
meaning; the projection preserves assistant text as written.

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

**Plugin extension point:** an installed plugin contributes recipe files,
merged with `RecipeCatalog::merge_recipe_files` (each through
`RecipeCatalog::register`). Matching requires every declared
signal and every required primary type, then ranks candidates by an explicit
specificity/priority tuple with stable id/version tie-breaking. Renderer
bindings are never supplied by a recipe. The built-in catalog is the default
register; plugin recipes are additive.

**Signals are not extensible.** The signal vocabulary is the built-in one in
`vak_delivery::skills`; a plugin adds recipes over it, not signals. (A
`SignalRegistry` was designed and not built.) Planning recipes from words
is an open question in `docs/plans/open-items.md`.

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

3. **Delivery posture integration** (Gap 7). Removed on 2026-10-09: no held
   delivery could be released, and none could occur
   (`30-output-engineering.md`, "When a packet goes out").

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
| Structured projection | `tool_result_with_vak_fence_is_projected`, `malformed_vak_fence_is_preserved_as_code_block`, `structured_fence_projected_on_telegram_html`, `structured_fence_projected_on_slack_mrkdwn`, `structured_fence_projected_on_discord_markdown`, `structured_fence_passed_through_as_json_for_native_surface`, `red_team_code_fence_abuse` |
| Markup converters | Telegram: `converts_and_escapes_supported_markdown`, `renders_aligned_table`, `preserves_fence_language_as_code_class`; Slack: `converts_bold_and_italic_conventions`, `converts_links_to_angle_pipe_form`, `renders_table_as_aligned_code_block`; Discord: `passes_through_native_markdown_unchanged`, `keeps_masked_links_for_discord_to_render` |
| Chunking | `chunking_is_unicode_safe_and_prefers_line_boundaries`, `chunker_closes_and_reopens_tags`, `no_cards_means_no_chunks_and_never_a_blank_message` |
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
- **Terminal projector:** A terminal renderer for the `OutputTimeline`.
  `SurfaceCapabilities` lists `"tui"` as native, but no renderer exists:
  the inline TUI was withdrawn, and `vak term` (doc 55) draws the
  presentation items it receives in its own decks.
