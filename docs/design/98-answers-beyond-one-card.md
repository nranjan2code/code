# 98 — Answers beyond one card

Status: in progress (2026-10-17): accepted by the maintainer; S1 built
(host and client, tested), not yet drawn linked in a live turn. Written at
the maintainer's direction ("we cannot build a card for every scenario";
"make this the best out there"). Builds on docs/design/92-card-forms.md
(forms), docs/design/87-composed-answers.md (composition),
docs/design/91-presentation-capabilities.md (packs),
docs/design/66-immersive-artifact-canvas.md (the Canvas) and
docs/design/96-answer-projection-gaps.md (P1 controls, P2 living pages,
Q1 preview tokens).

## 1. The problem

Every card stands alone. A person who moves a slider on a controls card
sees that card's results change and nothing else: a chart of the same
loan, a table of its schedule and a callout of its total do not follow.
Adding a card type for each combination is the trap: combinations are
unbounded, and a form is a shape, never a topic (doc 92).

What people ask for falls on a ladder of freedom:

| Need | Path |
|---|---|
| one interactive card | `controls` (built, doc 96 P1) |
| several cards moving together | shared values (§2) |
| a large result to read and work in | the Canvas (§3) |
| a dashboard that stays current | a living page (§4, doc 96 P2) |
| anything free-form | HTML in the preview sandbox, in the product's own look (§5) |

Each rung trades portability (channels get a text form) and reliability
(small models write flat schemas well) for freedom. Nothing on the ladder
needs a card built for its scenario.

## 2. Shared values within an answer

- A `controls` card publishes its inputs and outputs by name into the
  answer's scope. Names are unique within an answer; a second card
  publishing a name already taken is refused with the name.
- Any card field that holds a number (a stat's value, a chart point's y,
  a table cell, a callout's figure) may instead hold an expression over
  the answer's names, written `= expression` in that field, in the same
  closed language as controls (`vak_delivery::expression`). Text fields
  stay text.
- The host checks every expression when the card is recorded (it parses,
  names only what an earlier card of the answer published) and stores
  each field's value computed from the published starting values, so the
  card is complete without the client and its text form is right.
- The client keeps one value scope per answer: when a published input
  changes, every expression that reads it is recomputed and redrawn.
  Nothing runs but the closed language.
- "Ask about this" on any card of the answer sends the scope's current
  settings, computed again by the host, as Q3's values action does today.
- A chart over a range (a schedule by month) needs a list: a `controls`
  output may be a `series(start, end, step, expression)` that publishes
  points, which a chart or table reads by name. This is the one addition
  to the language and is held equal on both sides by the shared table.

Open: whether the `= expression` marker in a value field is clear to
small models, or whether each numeric field gets a sibling `*_from`
field. Measure both on captured requests before choosing (doc 89).

## 3. Large results in the Canvas

A card whose content is larger than the answer column (a long table, a
schedule, a document, a dashboard) shows an "Open in Canvas" action. The
Canvas (doc 66) draws the same payload with the same form renderer at
full size, keeps the answer's value scope live, and lets the person sort,
filter, export and edit where the form allows. Nothing is a second
renderer: the form draws at two sizes.

## 4. Living pages

Doc 96 P2: a Library artifact whose content is an answer's composed cards
and whose versions come from an automation (`Trigger`) whose run ends by
recording a new version. A dashboard is a living page; its controls and
shared values work as in an answer.

## 5. Free-form HTML in the product's look

Generated markup and workspace pages run in the preview sandbox
(invariant 35) and today receive the theme's tokens as `--vak-*` (doc 96
Q1). To make every page look like the product:

- the preview gets one generated stylesheet: the tokens, the type scale
  (16 px reading, 14 px controls, nothing under 12 px), spacing, radius,
  focus rings and default styles for headings, tables, buttons, inputs and
  charts' colours, light and dark following the app's theme;
- `emit_markup_preview_card` and the file preview describe the classes
  and tokens a page should use, so a page built from them matches without
  any colour of its own;
- a page that sets its own colours still works; nothing is rewritten.

## 6. Stages

- **S1 Shared values.** §2 without `series`. Done when a loan card's
  sliders move a stat and a table cell in another card of the same
  answer, live on the dev route, both sizes and themes, and the text form
  shows the starting values.
- **S2 Series.** `series` and a chart that reads it. Done when a loan
  schedule chart follows the sliders.
- **S3 Canvas.** §3. Done when a long table opens in the Canvas and keeps
  following the answer's sliders.
- **S4 Product look for HTML.** §5. Done when a generated dashboard page
  matches the app in both themes with no colour of its own.
- **S5 Living pages.** Doc 96 P2.

Each stage is its own commit with the full gate and a live turn.

## 7. S1 as built (2026-10-17)

- Marker: `= expression` in a stat's `value` or a table cell, both text
  fields, so no schema changed; the controls, stats and table tool
  descriptions say so.
- Host: `show_card` (vak-agent) collects what this turn's controls cards
  publish (`vak_delivery::controls::published`: inputs and computed
  outputs, by name, with the publishing presentation id), refuses a
  controls card that publishes a name already taken, and binds the new
  card (`controls::bind`): each `=` field is parsed, checked against the
  names, computed and stored as a number, and recorded in the card's
  `bound` list (`path`, `expression`, `from`). Every form may carry
  `bound`.
- Client: a controls card publishes its live values under its
  presentation id (`answerValues.ts`); `FormView` recomputes every bound
  field from them (`withBoundValues`), so any form follows, one path.
- Live on the dev route (qwen3.5:4b-mlx), two turns: the model answered
  with one controls card holding every result, and, asked for a separate
  table, wrote textbook names (`P`, `r`, `n`) and an assignment; both
  refusals now list the names to use and say an expression has no
  assignment. No turn has yet drawn two linked cards.

