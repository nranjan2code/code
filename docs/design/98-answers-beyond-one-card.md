# 98 — Answers beyond one card

Status: implemented (2026-10-22): retained controls, bindings and saved-page
resources; model-authored layout and ability table/chart tools are retired.
Runtime cards and Markdown replace the removed tool surface. Remaining
interactive acceptance work stays in docs/plans/open-items.md.

The obsolete model-plan parser and answer-outline projection/wire types were
removed on 2026-10-22 at the maintainer’s request. Current answers do not
read old plans or retain a compatibility path for them.

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

Earlier status: implemented (2026-10-17): accepted by the maintainer; S1 to S5
built and checked live (2026-10-17); what S5 left is in
docs/plans/open-items.md. Written at
the maintainer's direction ("we cannot build a card for every scenario";
"make this the best out there"). Builds on docs/design/92-card-forms.md
(forms), docs/design/87-composed-answers.md (composition),
docs/design/91-presentation-capabilities.md (packs),
docs/design/66-immersive-artifact-canvas.md (the Canvas) and
docs/design/96-answer-projection-gaps.md (P1 controls, P2 living pages,
Q1 preview tokens).

## Current resource and ability snapshot contract

Saved-page versions carry an immutable map from referenced file paths to
recorded artifact/version pairs, resolved in the source session's workspace.
The viewer reads only those captured references through the page's current
audience. It does not consult an unrelated active conversation. A missing
exact version never substitutes the artifact head; historical pages without
captured references explicitly lack that resource context.

Ability table/chart tools name both the data-producing tool (`from`) and its
recorded invocation (`call_id`). The runtime uses that exact completed
successful result, with its full evidence body, and records `data_call_id` on
the presentation. A later failure cannot select an earlier success by name.
An explicitly chosen earlier success is a stable snapshot. Refresh requires a
newly admitted ability call and a newly selected snapshot. Identical values
from different selected invocations retain distinct provenance.

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
  publishing a name already taken by an active card is refused with the
  name. An unused source can replace itself with the same names; replacing
  a source that active consumers still bind is explicitly refused until
  those dependencies are changed (doc 87, Placement and revision).
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
  Nothing runs but the closed language. Views of the same session and turn
  share current input text as well as computed values; mounting another view
  does not reset its inputs. Each view owns its visible source IDs; removing
  one view does not clear sources another view still owns. Missing sources
  and invalid inputs use the host-recorded field values, and closing the
  final view releases the temporary scope. Saved pages use a separate scope
  per immutable Library version, shared only by views of that version in
  this client. Page edits stay unsaved; another version starts fresh.
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
full size through the same source-order row composer, keeps the answer's value scope live, and lets the person sort,
filter, export and edit where the form allows. Nothing is a second
renderer: the form draws at two sizes.

Canvas and conversation use identical anchored groups (doc 87, 2026-10-21).
Wider space may let a pair fit, but does not reinterpret reading order as CSS
columns. A form's reading width determines fit; topic and history age do not.

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
- **S5 Living pages.** Doc 96 P2. Choices the maintainer made
  (2026-10-17): a Library page stores the answer as its own items, not
  Markdown; an automation names a page in one field (`Trigger::page`),
  made by the owner's Keep current or by the Agent's automation tool; a
  page's runs go to the page alone; a viewer's control values are their
  own and never saved. Done when a page made by a routine on the dev
  route gains a version per run, keeps the old ones, opens in the Canvas,
  and a guest with a grant reads it and nothing else.

Each stage is its own commit with the full gate and a live turn.

## 7. S1 as built (2026-10-17)

- A stat follows published values through its own optional field,
  `follows` (an expression); its `value` is optional when `follows` is
  given. The `= expression` marker inside `value` was tried first and
  failed live in both tries (the figure written as prose, then as fixed
  numbers): one field for two things, as doc 30 §30.1 rule 9 measured. A
  table cell keeps the `= expression` marker (cells have no sibling
  field), so far unmeasured live. The builder turns `follows` into the
  same binding.
- The controls card's acknowledgement names what it publishes and shows a
  reading built from those names, so the next step knows what it can
  follow.
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
  assignment.
- Then, on the same request: the dev route answered with one controls
  card holding every result three times (each a fair answer; nothing may
  read the request to insist on a second card), and once wrote "Follows"
  into `value`. The installed route (gpt-6-luna) drew a controls card and
  a stats card whose readings use `follows`, beside each other, on the
  first try. In the browser, changing the rent moved the stats card from
  35,000 to 30,000 a month. Found and fixed on the way: `FormView` passed
  the form a plain props object, so a bound payload never redrew (now a
  getter), and the client cloned a store proxy with `structuredClone`,
  which threw. Whole computed numbers are stored as integers.


## 8. S2 as built (2026-10-17)

- A series is a controls output with `vary` (one of the card's inputs),
  `from`, `to` and `step` (numbers or expressions over the inputs): its
  expression is computed across that range, earlier outputs recomputed at
  each point (`vak_delivery::controls::series`; client `seriesOf`, held to
  the same answer by a test). No list type was added to the language. A
  range is bounded by the same 10,000 a card's lists are.
- A chart follows series through its own `follows` (names, comma
  separated); `data` is optional then. The builder adds each as a series
  marked `follows` with no points, which a validator accepts, and binding
  fills its points and records `{path, series, from}` in `bound`.
- The controls acknowledgement names its series for a later chart.
- Live, installed route: run 1 drew the controls and then a following
  chart that Vak refused (`points` must be non-empty: my validator, fixed,
  with a test); run 2 sent fixed `data` without a series (static, valid);
  run 3 drew a series and a following chart, and in the browser a growth
  rate of 15% redrew the curve and rescaled its axis.

## 9. From the audit (2026-10-17)

- The series point cap is the operator's: `[control.limits]
  series_points` (shipped 10,000), shown in Settings under Limits as a
  technical row. When a controls card is recorded the host computes its
  series under it and stores it on the card (`max_points`), so the client
  recomputes under the same limit. The validator checks a range's
  structure and no longer computes it. Seen in Settings → the Agent's
  shared defaults → Limits → Technical details, default 10,000 points.
- Every example in the card tools' descriptions and refusals is neutral
  (no groceries, weather, units, places or bookings); domain examples
  remain only in test fixtures.
- What may follow published values is declared by each card tool
  (`Tool::followable_fields`, from its shape: the stats card's
  `readings.*.value`, the table's `rows.*.*`); binding walks whatever a
  tool declares, with `*` for every index. A new form or pack declares
  its own; nothing in the binding names a form.

## 10. S3 as built (2026-10-17)

> **Historical record:** the first S3 opened single cards in the Canvas
> (a `card` subject and "Open in Canvas" on every card). The maintainer
> set the unit right the same day: an answer is seen whole. It was
> replaced, and the per-card subject, viewer and button removed.

- The Canvas opens an answer: `{kind: "answer", sessionId, turnId}`
  (`canvasSubject.ts`, `canvasViewers.ts`, `AnswerViewer`). The viewer
  draws the turn with the conversation's own `PresentationTimelineView`,
  given only that turn's items, so text, cards and files appear in the
  model's order and placement, and everything works as in the
  conversation: inputs, actions, and the values cards publish and follow.
- Layout is CSS alone: inside `.answer-in-canvas` the reading-width caps
  are lifted and the answer's material flows into columns of at least
  24rem, as many as fit; a card never splits across columns. One column
  on a phone, two at a wide window. The viewer scrolls.
- "Open in Canvas" sits with the answer's actions, beside Save to
  Library; inside the Canvas it is not drawn (`InCanvasContext`).
- Checked in the browser on a support-operations dashboard of four cards
  (controls, stats, a chart following a series, a table), built by the
  installed route: in the Canvas at a wide window the cards stood in two
  columns; raising agents on shift from 4 to 6 in the Canvas moved the
  stats (throughput 128 to 144, utilisation 112.5% to 75%), redrew the
  chart and changed the table; at 390 px it was one scrolling column.
- In the conversation an answer stays one reading column, and the model
  may set cards side by side with `beside_previous`; the Canvas is where
  an answer takes the width.
- The same request, asked for "a dashboard" without saying cards, was
  answered with an HTML page (the free-form path, §5), which drew in the
  Canvas in its own colours: S4's work.

## 11. S4 as built (2026-10-17)

- One stylesheet, built by the client from the app's tokens
  (`safeUrl.ts` `previewThemeCss`): eighteen `--vak-*` variables (surfaces,
  text, accent, rule, good/warn/bad, four series colours, fonts, radius)
  and a base for plain elements and the classes `vak-card`, `vak-grid`,
  `vak-muted` and `vak-primary`, all at zero specificity (`:where`), so a
  page's own rules still win. Markup previews carry it first, as before.
- A page on a preview origin is cross-origin, so the preview server puts
  a fixed slot in every HTML page it serves (`preview.rs` `THEME_SLOT`: an
  empty style and a listener that accepts only its parent's message) and
  the Canvas posts the stylesheet into it on load and on every theme
  change (`sendPreviewTheme`). The theme has one source, the client.
- Where the model meets it: the `write` tool and both preview card tools
  say a page is shown in Vakyartha's look and name the variables, each
  written with a fallback so the file still works outside Vakyartha. The
  first live page ignored the preview card's note, because the model wrote
  the file with `write` before any card; the note now sits in `write`.
- Live, installed route, a fresh folder: the dashboard page used the
  variables 30 times and had no colour of its own (all 34 hex values are
  fallbacks). In the Canvas it showed in the product's dark theme, and
  switching the app to light re-themed the open page at once.

## 12. S5 as built (2026-10-17)

- A page is `vak_delivery::page::Page`: the answer's turn as the
  conversation drew it (its items, its outline when its model planned)
  plus a text form (the cards' text, then the reply) for search, export
  and anything that draws no cards. Save to Library keeps every answer as
  a page (`projection::answer_page`, `library::record_page`); the
  Markdown save it replaces is gone, and the never-written
  `ArtifactKind::Card` became `ArtifactKind::Page`.
- An automation keeps a page current through one field,
  `Trigger::page`. The owner sets it with Keep current on a page in the
  Library (a schedule; the prompt is the page's request), which posts
  `/triggers` with `page`; the Agent sets it with the automation tool's
  `result_to`, which says where each run's answer goes: `conversation`
  (the default), `new_page` (a new page named like the automation,
  declared at once, its first version the first run) or an existing
  page's id. It replaced an optional `as_page` flag a live turn left out;
  the next live turn set `result_to: new_page`, and a run made the page's
  first version (2026-10-17).
  `Trigger::page_refusal` is the one check: only a prompt automation, and
  only a Library page outside the trash.
- A page automation's run ends in `automations::keep_page`: its answer
  becomes the page's next version, parented on the newest, saved so no
  retention rule removes it. Nothing goes to a conversation or the inbox's
  summaries; a `deliver_to` the automation names is still sent. A run that
  failed or answered nothing leaves the page as it was and the inbox says
  why (`RoutineFailed`).
- One view draws a page wherever it is shown (`PageView`): the
  conversation's own timeline view over the page's items, in the Canvas's
  columns, so cards, controls and shared values work as in the answer and a
  viewer's changes stay theirs. It is used by the Library's preview, by
  the Canvas subject `page` (`PageViewer`, always the newest version, so a
  reload shows the latest run) and by a guest's shared view, which draws
  the newest version it was shown beside the version list.
- A page leaves the Library as its text form (Download, for the owner and
  a guest), and Continue working or Make another gives the Agent that text
  and asks it to answer again, never to edit the page's stored file.
