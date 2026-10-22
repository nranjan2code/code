# 87 — Composed answers

Status: superseded (2026-10-22): model-authored outlines and layout are
replaced by Markdown answers and runtime composition. The retained rendering
and ledger contracts are described below; the current form registry is doc 92.

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

Earlier status: in progress: direction approved by the maintainer on 2026-10-11;
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

## Accepted placement identity, 2026-10-21

`PresentationRecord.placement` records the named part and exact beside/replaced
Presentation entry ids when a card is accepted. Resolution uses the then-active
sequence, respecting prose and part boundaries; it never chooses a new neighbour
after replacement removes an endpoint. Both the settled OutputItem and
CardRecorded live delta carry that accepted metadata. Their item identity is the
same tool-call/structured-output identity. Live server and client stores remove
only the addressed replaced presentation, and the shared row composer matches
beside anchors by presentation identity before any outline arrives. Outlines
project the same endpoints rather than infer them again from visible neighbours.

HTTP, gateway and scheduled turns first drain the model-event forwarder, then
publish the durable projection with a separate `covered_sequence`. The background projector and
client discard events already covered by that snapshot. A delayed live card
cannot overwrite a settled card or resurrect a replaced one; events from the
next turn have later sequence numbers and remain admitted. The sequence is a
stream authority boundary, not the ledger cursor or a second durable record.
Reconnect and gap baselines preserve it; replaying RunFinished rebases from the
canonical handle snapshot, and resync frames carry authoritative snapshot
semantics rather than an additive live cursor.

Channel replies carry metadata projected from their own settled turn alongside
their text and cards. Delivery never reads a mutable live presentation to decide
a reply's outcome: that view may still be streaming or already belong to the
next turn. A completed outcome with evidence state `none` needs no retrieval;
that state alone is not an unverified result. Stale or unknown evidence,
incomplete outcomes and required human review still produce notices.

## Shared order and geometry, 2026-10-21

The ledger's answer order is semantic order: the client never lifts a summary
or moves cards across another section or prose. The plan's sections attribute
blocks; they do not authorize reordering. The host resolves `beside_previous`
in original visible order to exact anchor/item ids in `AnswerOutline.beside`.
Only adjacent cards of the same named part can pair. A removed or replaced
anchor never silently binds another predecessor.

LiveAnswer, settled composition and Canvas use the same row composer. Each
form has a minimum reading width; grouped blocks share a wrapping flex row
only when those widths fit, with unequal allocation where appropriate. Canvas
adds available space through these same rules, without an independent CSS
column flow. Prose, result ids and turn ids remain composition boundaries.

Prompt and plan guidance request the smallest complete answer, with optional
summary and no repetition of the same finding across forms. Tools and schemas
remain available; there is no domain-specific layout recipe or card-count cap.

## Continuous answer surface (2026-10-22)

The shared web/desktop renderer gives composed form content the answer's open
surface, while standalone forms retain their shell. Compact collections in the
existing result file area preserve each artifact's own status, Open, Download,
history and review action. A single file retains its preview. Copy and request
details share the result toolbar; independent result identities retain separate
toolbars. Non-success outcomes have one visible typed status rather than a
second repeated footer. Long fact values stack at narrow container widths.

Nothing selects a layout by domain or hides checks, logs or sources by form.
No new primary/support metadata, stored collection, route or execution authority
was introduced. Artifact focus continues through the existing exact-subject
Canvas; its answer view uses the same renderer. Browser acceptance and native
qualification limits are in
[the implementation record](../audits/chat-projection-ux-implementation-2026-10-22.md).

## Answer design authority (2026-10-22)

The code-owned presentation contract is the shared source for how the model
chooses forms. Selection follows relationships in the available information
and what the person needs to understand or do, never domain names. Procedure
skills guide evidence, execution and verification without compulsory answer
recipes. Their related-tool lists are nonexclusive hints; they do not narrow
the admitted interface. Rich surface descriptions acknowledge typed forms and
Markdown prose together. Literal, creative and direct prose remain valid.

The initial plan names requested outcomes and can be refined as evidence makes
useful parts and forms clear. A generic card permits several appropriate forms;
a specifically named form promises that form. Receipt validation retains the
actual semantic type/base form and accepted replacement addresses; another form
cannot satisfy that promise merely by sharing its deliverable name. Structural
delivery is separate from evidence sufficiency and quality of composition.

The coordinated correction, frozen acceptance scenarios and completion gates
are in [the answer design plan](../plans/answer-design-contract-plan.md), with
the installed evidence in [the investigation](../audits/answer-design-contract-2026-10-22.md).
The Continuous answer renderer and recent display optimizations are preserved.

## Conversation state handoff (2026-10-19)

The shared client hydrates from `/sessions/{id}/snapshot`: transcript
identities and output are projected from one immutable ledger revision.
An active runner exposes one committed prefix for both, marked `running`;
stream events continue the live view until a settled read can be applied.
The client applies message identity and presentation in one reactive batch.
Its hydration coordinator acknowledges applied state only, rejects older
in-flight reads, and invalidates reads at local admission and stream run
boundaries. A deferred running read can never suppress its identical settled
read. Turn pairing stays by durable entry id, never topic or position.

Cards are acknowledged as accepted for presentation. Acceptance/recording
is not proof a surface displayed them; no visibility receipt is fabricated.
The legacy stream renderer remains for an active answer before its durable
identity is available. This is a coordinated handoff, not a claim that all
legacy view storage has been removed.

## Delivery attribution and literal prose

Outline sections use the recorded card’s explicit deliverable name even
when the plan requested text. Their displayed form reflects actual card
delivery; unrelated prose is not guessed into several text deliverables.
User/assistant words and literal marker examples survive client cleanup.
Runtime control/authorship filtering belongs to the typed server projection,
not regular expressions over words such as `completed`, `Outcome`, or XML
examples. Typed `vak` fences retain their explicit card parsing contract.

## Projected prose and card boundaries

The client composes adjacent projected answer parts with one author header
and shared result actions. It preserves each assistant document and each
prose/card boundary in timeline order; it never selects only the final
assistant document or hides prose because a card exists. Adjacent cards may
use the outline's sections/layout, but a prose document is a boundary that
card layout cannot cross. Files remain in the result's file area. In the
everyday view, a successful `file_preview` with exactly one matching file
path (including its saved draft path) in that result is represented by the
file's compact result card and Open action. Ambiguous matches, previews in
other results and inline markup remain independent. Standalone file and
markup previews start collapsed and mount their isolated frame only when
expanded; source and reload controls appear with the expanded preview.
Open and Review keep short labels; secondary file actions and preview
controls use icons with accessible names and tooltips.

Known distinct result ids and non-result items split composition groups.
The server's typed PlanCheck/rejected-draft filters remain the authority
for removing runtime replies; the client's turn grouping does not classify
prose by its wording. Existing lexical envelope/scaffolding filters are a
separate remaining hardening concern. The legacy `card_followup` projection
is kept for older consumers; the shared client draws source documents once
rather than both the document and its redundant follow-up metadata.

The implementation has pure composition-order tests, a server source-order
regression and `tests/answer-composition.html` for live verification. Browser
acceptance remains pending while local browser access is disabled; this
is not completion of hardening P02 or the whole composed-answer design.

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
  deliverables name the answer's sections; source order remains authoritative. A turn with no plan
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
- **Layout.** One shared source-order row composer draws the blocks on every
  surface. Only exact addressed pairs share a row when their form reading
  widths fit; otherwise they stack. The summary remains where it was emitted.

### 2a. Cards end the answer (2026-10-16)

Every card says whether the answer goes on after it (`then`: `more` or
`done`), and a step of accepted cards whose last card said `done` ends the
turn when the plan, if any, has nothing left; no further step is asked
for. The design, the measurements on two routes and the provider survey
are docs/design/97-cards-end-the-answer.md.

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
| `chart` | series over a category or time axis | trend, timeseries, metric_chart, comparison_chart, telemetry.chart |
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

A pack accepts the types a block is offered as (§3): the 85 built-in packs
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

## Placement and revision (2026-10-14)

Decided by the maintainer: the model places its own cards. Every card
tool takes two optional typed arguments, read from the call and never
stored in the card. `beside_previous: true` puts the card beside the one
before it where the answer is wide enough (`AnswerOutline.beside`); a row
holds two, and a narrow screen stacks them. `replaces` names a card of
this answer by the presentation id its acknowledgement gave: the new card
takes that card's place in the answer, and the ledger keeps both; an id
that is no active card of this answer is refused with its active ids.
An unused controls card may be replaced with the same published names. A
source still named by an active card's binding is refused with a
`card_source_in_use` reason and the dependent presentation ids; automatic
dependency migration is not implemented. Retired cards do not supply new
bindings, count as current completion evidence, or transfer from workers.
Runtime validation and snapshot display use the same accepted replacement
view, while history and explicit recall retain every ledger entry. These
replace the client's fixed list of forms allowed to share a row and the
projection's supersede-by-type after a repair nudge.

