# 87 — Composed answers

Status: in progress: direction approved by the maintainer on 2026-10-11;
stage 1 done,
stage 2 built: the outline and its layout, with each card its own call of
the card tool that owns its kind (§2; the maintainer's choice on
2026-10-12, after one nested `answer` call of every block was measured as
broken JSON on 7 of 16 captured steps against none for separate calls,
docs/audits/cards-in-answers-2026-10-11.md). Stage 3 is not started. Builds on docs/design/86-model-plans-runtime-checks.md (the plan
and its receipt check). Replaces the model-facing card vocabulary of
doc 30-render-architecture §30.1 (twelve `emit_*_card` tools over 109
domain-labelled `semantic_type`s); the rendering contract of
`vak-presentation` and doc 67 stays.

## 1. What is wrong

Measured on 2026-10-11 against the live client and the original Agent
conversation:

1. **No answer has a design of its own.** The unit is a card, produced as
   a side effect of a tool call; "the answer" is cards and free text in the
   order the model emitted them. Nothing gives it a headline, a lead,
   sections or an arrangement. In the pantry run the model treated its
   text as the real answer and restated both cards in Markdown under them.
2. **The vocabulary is topic labels, not visual blocks.** 109 semantic
   types reach about ten client builders: 31 render as the vertical
   timeline, 22 as one labelled-details box with a different heading word
   (map, calendar, board, graph, form, alert…), 18 as a table, including
   `comparison` and `pros_cons`. The client's own `comparison` primitive
   is drawn by `renderTable`. The model spends its choice on a label
   (`habit_plan` or `checklist`) that changes nothing on screen.
3. **The table is the catch-all.** A routine became a one-column table; a
   herb guide put paragraphs in table cells. A table is the most
   Markdown-like thing on screen.
4. **What makes an answer good to look at is missing or unreachable:** a
   lead, steps with emphasis, options side by side with a verdict, a tip
   or warning, a row of key numbers.
5. **The 88 starter packs are seeded disabled,** so every card renders
   through the generic look.

## 2. The answer is one document

A turn's answer is composed by the runtime, not left to emit order:

- **Outline.** The model's `plan` (doc 86) is the outline: its
  deliverables, in order, are the answer's sections. A turn with no plan
  is one section.
- **Blocks.** The model shows each part of its answer that has a card
  shape by calling the card tool that owns its kind (`Tool::card_kinds`),
  by that tool's own name and precise schema, and writes the rest as the
  reply's text. A card call is the loop's own display channel, in every
  permission mode: it is checked against its tool's schema, the plan, the
  sources the conversation retrieved and the tool's rules, then recorded
  as a `Presentation` of the call (`PresentationSource::ToolCall`). A card
  identical to one already shown this turn is not shown twice; a card that
  previews a file the run delivered for review is not shown. A card that
  fails is refused with what to fix and goes through the repair stages like
  any tool (docs/design/15-reliability.md): its schema is shown, then it is
  withdrawn, and the part is given as text.
  A card names its section with `deliverable`. A card that names a deliverable
  the plan does not have is refused with the plan's names; a card that
  names none (a tip beside the answer) is in no section and is shown after
  the sections in emit order. A planned card counts as shown only when a
  card names it: a count of cards once let two cards answering an earlier
  request stand for a plan of two. A `plan` deliverable whose form names
  a kind of card is a card of that kind (`Deliverable::kind`), so a step
  can be required to call that kind's tool when the card is still missing.
  The reply's text is in the one text section when the plan has exactly
  one. The projection publishes this as the answer's outline
  (`OutputTimeline.answers`), reading each card call's own `deliverable`, and
  the client lays the answer out from it. Ledgers written while the answer
  was one nested call keep their `PresentationSource::Answer` records,
  which the projection still reads.
- **Layout.** The client lays out the answer from the blocks' shapes,
  deterministically: a summary block leads as the headline; a comparison
  renders its options as columns; steps as a numbered flow; stats as one
  row of figures; a callout as an accented note; small neighbouring blocks
  (stats beside a callout) share a row on a wide screen and stack on a
  narrow one. One type scale, one spacing rhythm, one entrance motion for
  the whole answer, so it reads as one thing rather than a pile of boxes.

## 3. Blocks

Each block kind is the arguments of the card tool that owns it, called by
its own name (§2); the flat-schema portability rules of
doc 30 §30.1 hold for every block. No block carries
a topic enum: `semantic_type` is the block's own name, and an optional
`label` holds the model's own words for what it is ("Weekly routine"),
shown as the block's eyebrow.

| Block | Shape | Also offered as (the same block, named for what it shows) |
|---|---|---|
| `lead` | headline, one to three sentences, up to five key points | |
| `steps` | ordered steps: label, optional detail, optional `time` or duration, optional total | timeline, plan.timeline, itinerary, schedule, agenda, milestones, progress, status, lesson, habit_plan, project_plan, home_project, care_plan, event_plan, meal_plan, decision, decision_analysis, coding.deployment, coding.incident, coding.release |
| `checklist` | unordered lines | shopping_list, reading_list, media_list, collection, notes, meeting_notes, follow_up, reminder, contact_log |
| `comparison` | options (2–6) each with a name, a short verdict and its strengths and weaknesses; aspects compared across options; an optional recommendation | |
| `table` | header and rows, numeric columns detected | data.grid, dataframe, comparison_table, pros_cons, decision_matrix, criteria_matrix, tradeoff_analysis, scorecard, inventory, budget, finance_summary, invoice_summary, travel_options, coding.benchmark, coding.dependencies |
| `stats` | one to six figures: label, value, unit, optional change | |
| `chart` | series over a category or time axis | trend, timeseries, bar_chart, metric_chart, comparison_chart, telemetry.chart |
| `callout` | tone (tip, note, warning, important) and text | |
| `details` | labelled facts or question-and-answer pairs | overview, summary, detail, faq, metric, telemetry.metric, map, route_map, calendar, availability, board, entity, evidence, document, graph, form, action, transaction, alert, conversation, progress_dashboard, simulation, search_results, coding.search, coding.architecture |
| `research.synthesis` | findings, each tied to a retrieved source | research_brief, news, the social briefs |
| `recipe` | ingredients, method, timings | recipe.card, recipe_summary, lifestyle.recipe, lifestyle.culinary_recipe |
| `media`, `ui.preview`, `coding.diff`, `test.report`, `terminal.view` | as today | |

A type is offered under the block whose stored payload its renderer and
its pack already read, so a type keeps its own renderer, its
pack and its validation. Every built-in pack's type is produced by some
block, and a test fails when one is not. Stage 1 first removed these
types; they came back on 2026-10-11 when that was found to have cut the
model off from 66 types and 75 of the 88 active packs
(docs/audits/cards-in-answers-2026-10-11.md).

Builders check structure only (a table's rows have the header's cell
count; a comparison has at least two options; a step has a label). Domain
rules leave the builders: a budget's Total row and currency words are not
the table's business.

## 4. Packs

A pack accepts the types a block is offered as (§3): the 86 built-in packs
keep their types, and each is reached through the block that builds its
type's payload. The block kinds of their own (`lead`, `stats`, `callout`,
`details`, the options `comparison`) have no pack yet and draw through
their client renderers; a pack for them is stage 3.

## 5. History

Ledgers written before this change keep their semantic types, and the
client keeps rendering them. Only the model-facing tools change: no
existing record is rewritten (invariants 2 and 29).

## 6. Stages

1. **Blocks.** The new tools with flat schemas and structural builders;
   client renderers for `summary`, `steps`, `checklist`, a real
   side-by-side `comparison`, `stats`, `callout` and `details`; each
   topic type offered as a kind of the block that builds its shape; prompt and tool
   descriptions rewritten around blocks.
2. **Composition.** The server projection groups a turn's blocks by the
   plan's sections; the client lays out the answer document with the
   layout rules above.
3. **Packs** by block type, enabled where they draw a block better.

Each stage is accepted the way doc 86 says: real turns on the configured
route through the web app in the original Agent conversation, read back
from the ledger and seen in the browser at 1440 × 900 and 390 × 844, light
and dark. The cases: the pantry request, the herb guide, a single
measurement, a decision between options, a budget, a trip day plan, a code
change with its check, a distant follow-up and a topic switch.
