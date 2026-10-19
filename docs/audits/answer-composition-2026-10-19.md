# Answer composition: before and after (2026-10-19)

Status: reference: the measurement behind the 2026-10-19 plan and card
wording change; open work is in `docs/plans/open-items.md`.

What was changed, and what the same requests did before and after, on the
installed route (OpenAI Responses, `gpt-6-luna`, strict tools), one run
each. One run per case is a sample, not a rate: read it as what the
contract now allows, not as a measured improvement.

## Why

The installed session's dollar-analysis turn (2026-10-19) answered "a
detailed analysis across sectors" with three cards, one of them a table of
long sentences. Its records showed three causes in Vak's contract:

1. The runtime puts several cards under one plan entry
   (`AnswerSection.items`), but the model read `deliverable` as "the name of
   the card in your plan" and the plan as "each distinct thing the person
   asked for", so one part became one card.
2. A plan's `form` could not name a card form: `plan_forms` and
   `Deliverable::kind` read one, but the schema's enum held only the five
   delivery words, so a wire that enforces the schema could never send it,
   and the reframe rung (doc 86 §2.1) could not name a tool.
3. Receipts said `reasoned: false` on every step: the Responses wire returns
   no reasoning text unless asked, and its reasoning token count was not read.

## What changed

- `plan`: one entry per part of the answer, in reading order; an entry shown
  as cards takes as many cards as its content has parts. `form` lists the
  card forms (`vak_delivery::FORMS`); one whose tool the turn withdrew is a
  card with no kind.
- Every card's `deliverable`: the plan entry it is part of; an entry may take
  several cards. The table's description asks for short cell values. The
  seed says `deliverable` names "the plan entry it belongs to" (doc 07 diff
  note).
- `Usage::reasoning_tokens` from each wire that reports it; a step reasoned
  when the response carried reasoning or its usage counts reasoning tokens.

## Results

| Request | Before | After |
|---|---|---|
| Dollar collapse, then "detailed analysis across sectors" | plan of 2 cards, no kinds; summary, table, comparison | plan of 3 named kinds (summary, table, list); summary, table (short first-round column, prose channels column), list |
| Spanish, French, Japanese | plan of 1; summary, comparison; 5 steps | plan of 1; summary, comparison, sources; 17 steps, 13 guessed URLs (6 failed) |
| One day in Kyoto | plan of 1; summary, steps; 4 steps | plan of 5 named kinds (summary, stats, steps, list, callout), re-planned with sources; summary, stats, steps, list, callout, sources; 15 steps |
| How vaccines train the immune system | no plan; summary, details, callout | plan of 2; summary and two details, sources; 5 steps |

- Plans named a card kind in 2 of 4 after (0 before). Several cards under one
  entry: 2 of 4 after, both before as well (the runtime always allowed it).
- Every after receipt but the empty steps records `reasoning_tokens`
  (0 to 1,034); before, every receipt said `reasoned: false`.
- After, three of four turns looked sources up and showed a sources card.
  With no search tool they guessed URLs with `webfetch`; most of the extra
  steps and time are those fetches (languages 109 s of model time against
  34 s). Search as a stated capability is the next item
  (`docs/plans/open-items.md`, "Answer composition").
- One refusal: a sources card named its entry with different words than the
  plan; the retry without `deliverable` stood.

The rendered answers were captured at 1440 wide from the web app of each
build. The ledgers are in the data home under the `/tmp/vak-live/m-before`
and `/tmp/vak-live/m-after` spaces.
