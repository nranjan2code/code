# 92 — Card forms

Status: implemented. The shared registry offers summary, steps, list, details,
table, comparison, stats, chart, callout, calendar, map and controls for
structured answer content. Remaining forms retain runtime builders. Live and saved answers use the same
typed renderer. Inline-card fences and the retired whole-answer protocol are
removed without compatibility readers; fenced code is ordinary code.
The 24 October correction restores structured authorship after the maintainer
reported prose/table answers instead of rich forms. No topic classification,
mandatory card count or separate layout planner selects the output.

> **Historical record** — the 22 October authoring policy below was replaced
> on 24 October; its runtime-derived sources and diff contract still applies.

## Runtime composition, 2026-10-22

The model authors Markdown and uses the five precise interactive card tools
when useful. All other form builders remain for stored history, inline
`vak` fences and runtime cards. At turn close, successful calls whose tools
declare `fetched()` supply a sources card of retrieval receipts, without
synthesized findings. Files captured before the turn wrote them supply the
diff card's actual hunks and counts through `with_changes_made` with no paths.

These are append-only `PresentationSource::Derived { from }` records naming
their source calls. The second projection pass draws them at the entry's
position after the answer. They are excluded from historical dialogue's
missing-presentations supplement: the recorded tool results already carry
the underlying evidence. No extra model call or layout plan is involved.

> **Historical record** — the earlier design and implementation follow.

Earlier status: implemented: F1 to F6 built and checked live 2026-10-14; direction from the maintainer on 2026-10-14 ("rebuild
correctly to be maintainable but yet effective for anything … completely
remove what's not wanted"; no users, so nothing is kept for compatibility).
Builds on docs/design/87-composed-answers.md (cards in answers),
docs/design/89-constrained-tool-arguments.md (one precise flat schema per
tool) and docs/design/91-presentation-capabilities.md (packs add kinds to a
block). Replaces the type registry of docs/design/30-render-architecture.md
§30.1 and the starter catalogue of docs/design/57-adaptive-presentation-runtime.md.

## 1. What is wrong

Read from the code and from live answers on 2026-10-14:

1. **A hundred names for eight drawings.** The client's
   `STRUCTURED_RENDERERS` maps 100 `semantic_type`s onto a handful of
   builders: 31 to the numbered timeline, 24 to a field list
   (`universal_card`), 17 to the table. A map, a calendar, a board, a form
   and an FAQ are each drawn as nested label/value lists. A shopping list,
   a reading list and notes are drawn as numbered steps joined by a line.
2. **The host defines what the client never draws.** `vak_presentation::Primitive`
   has 57 members; the client draws about twenty and sends `map`,
   `calendar`, `board`, `graph`, `form`, `entity`, `evidence`, `alert`,
   `conversation` and `transaction` to the same field list.
3. **The client draws by kind, not by block.** The record knows the block
   that built a card (`PresentationRecord.block`), but the wire does not
   carry it, so the client needs a table of every kind.
4. **A second decision layer reads words.** On every assistant message the
   projection extracts signals from the text (`signals_from_context`), runs
   a recipe planner over them, merges outcome requirements and chooses a
   pack. That is a reading of words deciding what a person sees.
5. **85 starter packs restate the aliases** (`seeds.rs`), and the renderer
   itself decides by words and counts: titles dropped when they read
   "overview", "plan", "timeline", "table"; a table becomes options when
   its first column is named "option"; "quiet" at five rows.
6. **Drawing defects** seen live: the chart uses a point scale (the first
   bar covers the axis labels), labels only its first and last category,
   scales its text with its width, and carries a range slider; tables
   scroll inside the conversation (280px cap); prose runs ~140 characters
   a line; card body text is three different sizes.

## 2. The model

**A card is a form.** A form is a shape of information, not a topic. The
built-in forms, each one card tool (`emit_<form>_card`) whose own kind is
its name:

| Form | Shape | Replaces |
|---|---|---|
| `summary` | the answer: a headline, one to three sentences, key points | `lead` |
| `steps` | items in order: a how-to, a routine, an itinerary, an agenda | timeline, itinerary, agenda, milestones, schedule, release, incident |
| `list` | items in no order, each with optional detail, facts, link; `checkable` when they are to be ticked | checklist, shopping/reading/media lists, notes, search results |
| `details` | labelled facts about one subject, or questions with answers | entity, place, contact, transaction, FAQ |
| `table` | records under shared columns | data grid, budget, inventory, invoice |
| `comparison` | two to four options side by side, with a recommendation | pros/cons, decision matrix, travel options |
| `stats` | a few headline figures | metric, scorecard |
| `chart` | numbers across a dimension | trend, timeseries, metric chart |
| `callout` | one thing not to miss | alert, tip, warning |
| `sources` | findings tied to the sources they came from | research, news, evidence |
| `calendar` | events placed on dates and times | calendar, availability |
| `map` | places on the earth: apart, a route in order, or an area (doc 96 P5) | places that were a list with links |
| `controls` | inputs a person changes and results the runtime computes from them (doc 96 P1) | calculators that were prose or a static table |
| `link` | a link with its title | link preview |
| `media` | an image, video or audio | media.* |
| `file_preview` | a workspace HTML/UI file | ui.preview |
| `markup_preview` | inline HTML | ui.markup |
| `diff` | code changes made | coding.diff |
| `test_report` | a test run a call executed | test.report |
| `terminal` | a command a call ran, with its real output | terminal.view |

Prose is not a card. A recipe, a lesson or a care plan is not a form: it
is a composition (`details` for its facts, `list` for what it needs,
`steps` for how), which doc 87 already lays out as one answer.

Rules:

- **One kind per block.** A block's built-in kind is its own name. Packs
  add kinds to a block (doc 91); nothing else does.
- **The wire carries the block.** `StructuredOutput.block` is set by the
  host for every card. The client draws by block; a kind only selects a
  pack on the host.
- **The primitives are the forms**, plus layout (`stack`, `row`, `group`,
  `section`) and `text`. A pack composes these; the host enum and the
  client's dispatcher are the same set, checked by a test.
- **Nothing decides by words or counts.** No signal extraction from text,
  no recipe planner, no title word lists, no row thresholds. A variant a
  card needs is a typed field of its schema.
- **One content shell.** Standalone forms have a shared border, padding and
  optional heading, with no hover unless they act. Inside a composed result,
  the same form content inherits an open section treatment from its enclosing
  answer; it does not add another peer box. This affects enclosure, never
  primary/support role, visibility, semantic order or interaction. Card body
  text is the conversation's reading size; prose is held to ~70ch.

Not built: a board, a graph and a form to fill in. Places go in a `map`
(added 2026-10-16, docs/design/96-answer-projection-gaps.md P5), and a
calculation a person adjusts in `controls` (added 2026-10-17, P1); a decision the person makes goes through
the existing approval and option flows. Each is added only when real
turns show a shape the forms above cannot carry (`docs/plans/open-items.md`).

## Channel rendering

Native channel rendering uses the same `block` and validated payload, through
one form projection and provider compilers shared by direct replies and
proactive pushes. See [Native channel cards](102-channel-cards.md). There are
no aliases, subject lists or guessed payload schemas in the channel path.

## 3. What is removed

- The 82 alias kinds, the client's `STRUCTURED_RENDERERS`, the
  `universal_card` field list and every alias builder.
- `emit_recipe_card`, `emit_checklist_card` (now `list`), the media
  shape's four kinds (now `link` and `media`).
- `PresentationRecipe`, `built_in_recipes`, `signals_from_text`,
  `signals_from_context` and the projection's planner over them.
- The 85 alias starter packs and the seed module: every form draws
  natively, so a built-in pack that redraws one adds nothing. Packs stay a
  capability (plugins, the owner's own); `packages/answer-layouts` is the
  one shipped. The social add-ons' twelve research layouts go with them.
  The data home is set up again (re-seeded) after the change.
- The delivery result adapters, the JSON sniffing of prose and of json
  fences on host and client, and the HTML exporter's own vak-table fence
  dialect: a vak fence is the one fallback.
- Primitives no form or layout uses.
- The compatibility branches for older saved pack trees in the client.

## 4. Stages

| Stage | Done when |
|---|---|
| F1 Host forms | `SHAPES` is the table above, one kind each; `StructuredOutput.block` on the wire; skills, digests, fixtures, fence path agree; card tests pass |
| F2 Remove the word layer | no `signals_*`, no recipes, no planner in the projection; pack selection by kind and activation alone |
| F3 Primitives and seeds | `Primitive` is the forms plus layout; seeds rebuilt; social add-on packs name their block |
| F4 Client | one renderer per form under `components/cards/`, drawn by block; one card shell; chart, table and measure fixes; old registry and renderers deleted; both bundles rebuilt |
| F5 Prompt and docs | prompt seed and tool descriptions name the forms (diff note in doc 07); docs 30, 57, 67, 87 point here |
| F6 Live | real turns on the dev route draw every form; checked at 1440×900 and 390×844, light and dark; data home re-seeded |

Progress:

- F1–F3, 2026-10-14: `SHAPES` is the eighteen forms; `vak_delivery::skills`
  is one validator and one text form per form, with `StructuredOutput.block`;
  the planner, recipes and signals are gone from delivery and the server
  projection; `Primitive` is the forms plus layout; seeds removed.
- F4, 2026-10-14: `components/cards/` (one shell, one renderer per form,
  the chart rebuilt with a band scale and round ticks); the registry,
  `GenericSpecRenderer`, `blocks.tsx` and the client's fence repair removed;
  the card CSS rewritten on DESIGN.md tokens.
- F5, 2026-10-14: the prompt seed says an answer uses as many cards as it
  has parts; AGENTS.md and docs 30, 67 and 68 point here.
- F6, 2026-10-14 (dev route): every form drawn live; checked at 1440×900
  and 390×844, light and dark. Fixed from what the runs showed: a plan is
  recorded before the cards in its step; a `[card-plan]` refusal lists the
  cards still to show apart from those shown (a model had re-sent shown
  ones); card prose renders as the conversation's markdown and wraps long
  words; Today moved into the top bar. Format-free requests (a fact, hidden
  numbers, two small asks, a bill split) stayed prose; a hidden plan drew
  summary, steps, callout and sources. Card revision and placement are the
  model's typed choices (doc 87, "Placement and revision").
