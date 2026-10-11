# 87 — Composed answers

Status: direction approved by the maintainer on 2026-10-11; stage 1 in
progress. Builds on docs/design/86-model-plans-runtime-checks.md (the plan
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
- **Blocks.** Every card and the final text are blocks. A card names its
  section with `answers`; one that names none goes to the next section
  still waiting for one of its form, as the plan check reads it. Prose is
  a block too: it renders where its section is, never as a trailing essay
  under the cards.
- **Layout.** The client lays out the answer from the blocks' shapes,
  deterministically: a summary block leads as the headline; a comparison
  renders its options as columns; steps as a numbered flow; stats as one
  row of figures; a callout as an accented note; small neighbouring blocks
  (stats beside a callout) share a row on a wide screen and stack on a
  narrow one. One type scale, one spacing rhythm, one entrance motion for
  the whole answer, so it reads as one thing rather than a pile of boxes.

## 3. Blocks

Each block is its own flat card tool (doc 30 §30.1 measured per-shape tools
as the reliable path; schemas keep its portability rules). No tool carries
a topic enum: `semantic_type` is the block's own name, and an optional
`label` holds the model's own words for what it is ("Weekly routine"),
shown as the block's eyebrow.

| Block | Shape | Absorbs |
|---|---|---|
| `summary` | headline, one to three sentences, up to five key points | overview, summary, detail lead |
| `steps` | ordered steps: label, optional detail, optional `when` or duration, optional total | timeline, schedule, itinerary, agenda, milestones, steps, routines, plans |
| `checklist` | unordered things to bring or do, each checkable | checklist, shopping, packing, reading lists |
| `comparison` | options (2–6) each with a name, a short verdict and its strengths and weaknesses; aspects compared across options; an optional recommendation | comparison, pros/cons, decision, trade-off |
| `table` | real tabular data: header and rows, numeric columns detected | data grid, inventory, budget, scorecard |
| `stats` | one to six figures: label, value, unit, optional change | metric, weather, KPI, status readings |
| `chart` | series over a category or time axis | chart types as today |
| `callout` | tone (tip, note, warning, important) and text | alert, caveat |
| `details` | labelled facts or question-and-answer pairs | FAQ, entity, document facts |
| `sources` | findings, each tied to a retrieved source | research, news |
| `recipe` | ingredients, steps, timings | recipe |
| `media`, `ui_preview`, `diff`, `test_report`, `terminal` | as today | as today |

Builders check structure only (a table's rows have the header's cell
count; a comparison has at least two options; a step has a label). Domain
rules leave the builders: a budget's Total row and currency words are not
the table's business.

## 4. Packs

A pack accepts block types, not topic types. A starter pack that only
renamed a topic for an existing block is removed (invariant 30); one that
draws a block better (a richer comparison, a stat row) is enabled.

## 5. History

Ledgers written before this change keep their semantic types, and the
client keeps rendering them. Only the model-facing tools change: no
existing record is rewritten (invariants 2 and 29).

## 6. Stages

1. **Blocks.** The new tools with flat schemas and structural builders;
   client renderers for `summary`, `steps`, `checklist`, a real
   side-by-side `comparison`, `stats`, `callout` and `details`; the old
   topic enums removed from the model-facing tools; prompt and tool
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
