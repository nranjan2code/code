# Cards in answers: why answers went to prose (2026-10-11)

Status: historical. As written: Steps 1 to 4 done 2026-10-11; measured and decided 2026-10-12
(see "Decision: each card is its own call"). A fifth
finding, Markdown separator rows stored in a table, was a misreading:
the table builder skips them, and the stored card held only data rows.
Maintainer's direction 2026-10-11: keep the one
`answer` tool (doc 87 §2) and fix its contract, and make every card type
in the registry reachable through it, not only the 31 block kinds. Each
finding below is a fault in Vak's contract, read from the ledger and from
the request Vak sent (captured with a logging proxy in front of Ollama on
the dev route, `qwen3.5:4b-mlx`).

## Findings

### F1. 66 card types can no longer be produced

`d0875af47` (doc 87 stage 1) replaced the twelve shape tools, which
declared 97 `semantic_type`s, with sixteen block tools declaring 31. Since
then the model cannot produce `map`, `calendar`, `board`, `timeline`,
`itinerary`, `schedule`, `agenda`, `meal_plan`, `shopping_list`,
`budget`, `invoice_summary`, `inventory`, `weather`, `metric`, `faq`,
`entity`, `graph`, `form`, `alert`, `decision_matrix`, `pros_cons`,
`travel_options` and the rest. The client still renders all of them
(`STRUCTURED_RENDERERS`, about 113 keys) and the 87 starter packs in
`vak-presentation/src/seeds.rs` still target them, so the packs can only
apply to the 15 or so types still produced. Doc 87 §1.2 found that a topic
label alone changed nothing on screen. That holds for the generic builders,
but a type also selects a dedicated renderer, a pack and
type-specific validation, and none of those are reachable while the label
cannot be chosen.

### F2. The `answer` schema drops the type of two fields

`answer` merges every block tool's arguments into one object schema
(`answer::definition`). Where two tools use one name for different
shapes, the merge keeps neither: `items` is a list of step records for
`steps` and a list of strings for `checklist`; `points` is a list of
strings for `lead` and a list of point records for `chart`. Both reach the
model as a bare `"type": "array"` with no item schema, and their
descriptions are two descriptions joined by "Or, in another kind of
block". The whole tool is 11.6k characters with 41 fields per block.
This breaks the card contract's own rule that the same idea has the same
name on every block, read the other way: one name, one shape.

Observed with it: since 11:54 UTC most `answer` calls in the original
Agent conversation sent `blocks` as one string of JSON with one `}` too
many before its last `]` (`ses_01a123d6…`: 33 calls, 21 refused; 
`ses_01a12b91…`: 3 of 3 refused, then `answer` withdrawn and the answer
given as prose). Before `993b5fa0d` a withdrawn `answer` stayed callable,
so retries sometimes got through; since that fix, the third refusal ends in
prose, which is why every answer looked like prose.

The measurement in doc 88 'Open in C6' used the same block fields both
ways, not the merged schema, so it did not see F2.

### F3. A failed turn's note reads as unfinished work

The `<turns>` line for the previous turn (`TurnCard::line`) omits the
card's `outcome`. A turn that failed (here: provider circuit open) reads
"did: nothing → \"\"". In run `run_01a12bc1…` the model planned the new
request's deliverables, then answered the failed turn's request instead.

### F4. Cards are not tied to the plan

The plan check counts cards against planned cards; a card that names no
deliverable still counts as one. Two cards with no `answers` satisfied a
plan of two named deliverables while answering a different request.

### Decision: no topic is a card type of its own

`weather` had a renderer, a pack and a recipe chosen by the words
"temperature" and "forecast". At the maintainer's direction (2026-10-11)
it is removed from every layer: readings are a `details` or `stats` card
like any other, and the banned-token gate allows the word nowhere in
production code. The client draws 112 types and the blocks produce the
same 112.

### Decision: each card is its own call (2026-10-12)

Restoring the types and giving every name one shape (plan steps 1 and 2)
did not make `answer` reliable. Replaying Vak's real captured requests on
the dev route (`qwen3.5:4b-mlx`), eight runs each of two answer steps, the
system prompt and tools exactly as sent:

| Form | routine step good | note-taking step good |
|---|---|---|
| nested `answer`, before the schema fix | 6 of 8 | 2 of 8 |
| nested `answer`, after it | 5 of 8 | 2 of 8 |
| `answer`, one block per call (the union schema, no list) | 0 of 8 | 1 of 8 |
| separate card tools, each its own precise schema | 8 of 8 | 6 of 8 |

The fault is one long nested value written as text: a stray `}`, a raw
newline inside a string, characters after the end. A loose union schema
fails too, nested or not. A precise schema per card, called on its own, is
what holds, as measured on 2026-09-18. At the maintainer's choice each card
is again its own call of the card tool that owns its kind; the outline and
the layout of the answer (docs/design/87-composed-answers.md §2) stay, built
from each card's `answers`. F4: a planned card counts as shown only when a
card names it (an unnamed card covers none), a card naming a deliverable the
plan does not have is refused with the plan's names, and the reframe rung
requires the card tool of the kind the plan named. Refusing every unnamed
card was tried first and contradicted the prompt's own summary card (live,
`run_01a12d8b-f0b1-76c3-8849-0d468db9cb2c`).

### Next: a design, not more fixes

Separate card calls still failed often enough on the dev route to need
several refused calls per answer. The maintainer asked for a design from how
other systems solve this, for every provider Vak supports:
docs/design/89-constrained-tool-arguments.md (constrained decoding as a
capability per wire and model, regeneration of unreadable arguments under
the tool's schema, a ladder by fault class). The constrained measurement
there, 16 of 16, is from this investigation.

## Plan

1. **One name, one shape (F2).** Rename where one name has two shapes:
   the steps block's step records become `steps`; the recipe's method
   becomes `method`; the chart's point records become `data`. Stored card
   payloads keep their keys, so renderers and records are unchanged. A
   test over whatever card tools exist fails when two tools give one
   argument name two shapes, so `answer` never drops a type again.
2. **Every registry type is a kind of `answer` (F1).** Each topic type is
   declared by the one block tool whose shape draws it: plans and
   schedules by `steps`, lists by `checklist`, tabular data by `table`,
   readings by `stats`, labelled facts by `details`, alternatives by
   `comparison`, notes by `callout`, series by `chart`, sources by
   `research`, recipes by `recipe`. The stored card keeps the chosen type,
   so its dedicated renderer, its pack and its validation apply. Each
   renderer reads the payload of the block that builds its type. Tests: every
   type the client registers is declared by a card tool (or is listed as
   history-only), and every declared kind's built payload passes
   `SkillRegistry::validate` and draws through its renderer. The `answer`
   description lists a block's other kinds in one line, not one line each.
3. **A failed turn says so (F3).** `TurnCard::line` names the ending of a
   turn that did not complete (`ended: failed`).
4. **Cards answer the plan (F4).** When a plan lists card deliverables, an
   `answer` that leaves one of them unnamed by every block's `answers` is
   refused once with the deliverables it left unnamed, through the one
   correction ladder.
5. **Measure.** `scripts/measure_answer_form.py` run with the real
   `answer` schema on the dev route before and after, and live turns in
   the original Agent conversation for the doc 87 cases plus one rich type
   per block.
