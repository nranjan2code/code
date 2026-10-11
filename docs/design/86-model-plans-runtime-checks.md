# 86 — The model plans, the runtime checks

Status: direction approved by the maintainer on 2026-10-11; implementation
in progress (phase 1). Supersedes the parts of doc 47 that let a word-list
reading drive behaviour (stop gates, model-visible part notes, commitments
opened from a guessed horizon) and the card-completeness self-audit in doc
30-render-architecture §30.1. Doc 47's safety floor (intent narrows, never
widens; invariant 32) and its control plane stay as they are.

## 1. Why

A natural two-part request in the original Agent conversation (run
`run_01a12871-98b8-7112-859c-561174faa4c9`, local Ollama route, 2026-10-11)
took 37 model calls and 374 seconds and ended as a partial result of
header-only tables. Each step of the failure was Vak's contract, not the
model:

1. **The fixed reader turned description into work.** "We restock on
   Saturdays and check expiry dates on the first Sunday of each month" read
   as an instruction to *verify*, durable, because `check` after `and` was
   taken as a new imperative. The turn became a managed commitment whose
   objective was the user's habit, and its id was shown to the model.
2. **Ungrounded notes.** The model was told "Part 3 uses the result of part
   2" and never shown what the parts were; it went looking for earlier work.
3. **Two rules that cannot both hold.** The card contract says end with no
   text when the cards answer; the stop gate blocks an inspection-act turn
   whose final text is under 80 characters and which looked nothing up. The
   only way out was to look something up, so the model called `recall {}`
   eighteen times.
4. **A malformed call was waved through.** Every `recall` failure was
   treated as optional missing history, so the repair controller never saw
   the malformed calls.
5. **The table tool refused the best answer.** The first call was a
   complete comparison table sent as one Markdown string; schema admission
   refused it before the builder, which already reads one row per line.
6. **The model was asked to audit itself in the open.** The
   card-completeness check discarded a correct text routine and asked the
   model to compare every requested outcome with its cards; the model
   answered that question in its final text (the "I need to check whether
   all requested outcomes are covered" prose seen in the herb run), and the
   check re-armed after every new card.

Every patch so far added to a word list or a threshold. A reader built from
word lists cannot cover the requests people make; every fix grows a list and
the next request misses it. A classifier model call per turn would make
fewer wrong guesses, but a guess would still force behaviour, and it costs a
dispatch on every turn.

## 2. Three roles

| Role | Decides | Never decides |
|---|---|---|
| Fixed reader (`vak-intent` tier 1) | Facts that must not depend on a model: risk words that raise stakes or the approval floor, explicit control commands (`/stop`, `/goal …`), surface and attachment facts. It only narrows (invariant 32). | What the deliverables are, when a turn may stop, whether earlier work exists, whether to open a commitment. |
| The model | Its plan: the deliverables it will produce for this request, each with its form. | Whether a deliverable is satisfied (invariant 33). |
| The runtime | Whether each declared deliverable was produced, from receipts it observed: cards recorded, final text, files delivered, changes and checks run. | What the user meant. |

### 2.1 The plan is data

The model declares its plan with the broker-owned `plan` tool, intercepted
in the agent loop like `recall` (it needs the session, never a worker). Its
one argument is `deliverables`, a list of records `{name, form}`. When a
planned card is still missing after the model was told, one step is
required to call `answer` (docs/design/87-composed-answers.md §2;
docs/design/88-control-plane.md §5, the reframe rung):

- `name` — the deliverable in the model's own words ("weekly routine",
  "shelf vs meal comparison").
- `form` — how it is delivered: `card`, `text`, `file`, `change` (a change
  made in the workspace or outside Vak) or `check` (something run to prove
  a result).

The plan's record is the `plan` call itself: its arguments are ledgered
untruncated with its result, so it is model-visible and `derive_messages()`
reconstructs it (invariant 1), and nothing else records the same fact. It
is not a second `IntentRecord`: a mid-turn intent record would change the
per-turn tail, which stays byte-identical for every step of a turn (doc 68
§10). A later `plan` call in the same turn replaces the earlier one; the
latest is the one checked. Each declared deliverable is an
`OutcomeRequirement` of kind `Deliverable`, origin `Declared`, with its
`DeliverableForm`.

A card tool takes an optional `answers` field naming the deliverable it
delivers, by the name the plan gave it.

### 2.2 The runtime checks receipts

When the model ends the turn, the runtime reads the latest plan revision and
the turn's receipts:

- `card`: a card recorded this turn whose `answers` names it. A card that
  names no planned deliverable is counted, never attributed: the planned
  cards no card names are compared with those cards by number alone, and
  the runtime does not say which one a card was.
  Once the turn can show no card (the `answer` tool was withdrawn,
  docs/design/15-reliability.md), a planned card is checked as text.
- `text`: the final text is non-empty.
- `file`: a delivered-file receipt.
- `change`: an execution or edit receipt.
- `check`: a command run after the last file change.

A deliverable with no receipt is missing. The runtime sends one control
message for the turn, whatever changes afterwards, and keeps what was
already produced: the draft is not discarded and the model is not asked to
audit anything. The message says it comes from the runtime and not the
person, lists what the plan names and what was observed (the planned cards
no card names, how many cards were shown without a planned name, and each
other deliverable with no receipt), and asks for what is missing or a plain
reason. If the next step still leaves one missing, the turn ends as it
stands rather than looping.

Found live on 2026-10-11 (run_01a1293b-45c3-7f21-8fbf-bff3e2f9d3c6), before
this wording: a card that named nothing was attributed to the first planned
card by emit order, so the model was told the wrong one was missing; the
check was sent again when the missing set changed; and the model took the
unsigned message for the person, planned three more times and invented a
`submit_plan` tool.

The outcome evaluation reads the same receipts
(`vak_judge::outcome::PlannedReceipts`): a turn whose plan lists more
cards than were recorded, or a file no call delivered, is not `produced`,
whatever its final text says. Found live on 2026-10-11: after every
`answer` call had been refused, a reply saying the table "has been
delivered" stood as a produced outcome.

A turn with no plan is not gated on guesses. What the runtime observed still
gates it: an unresolved tool failure, and files changed after the last check.

### 2.3 Cards describe shapes

A card tool's description says what shape it draws and nothing else: no
rules about other tools, no topic examples, no patch history. Its builder
checks structure (a table's rows have the header's cell count; a timeline's
items have labels), never topic rules. The acknowledgement of a recorded
card is one line naming its presentation id; it instructs nothing.

## 3. What is removed

- Model-visible numbered part notes (`strand_note` relation and per-part
  lines). Relations stay in the intent record for audit.
- Stop gates keyed on guessed acts (`requires_inspection`, `requires_tool`,
  `requires_execution`, verification from a guessed `verify`) and the
  80-character "direct answer" threshold; replaced by §2.2.
- The `[card-completeness-check]` self-audit and its draft discard;
  replaced by §2.2.
- `CARD_REPEAT_ACK`'s behavioural text; a repeated card gets the same
  one-line acknowledgement.
- Commitments opened from a guessed durable horizon (phase 2: a commitment
  is opened by the model's plan or an explicit request, never by the
  reader).
- Domain validators in card builders (budget column words, a required Total
  row) and domain example lists in card descriptions (phase 3).
- Domain-labelled `semantic_type` enums the model must choose from, of which
  22 universal types differ only in a heading word (phase 3; pack selection
  moves to shape plus an optional label first, so no pack is stranded).

## 4. Phases

1. **Plan and check.** `plan` tool, `answers` on cards, the receipt check,
   removal of the self-audit, guessed-act stop gates, the 80-character rule
   and the part notes. Already landed beside it: an array field sent as one
   string (JSON text or one item per line) is read as its items for every
   card tool; malformed `recall` calls go through the repair controller,
   while a lookup that finds nothing stays optional history; a verb joined
   by `and`/`or` shares its statement's subject.
2. **Commitments** opened only from a plan or an explicit request.
3. **Card surface**: shape-only descriptions, structural builders, the
   prompt's presentation block rewritten to principles, pack selection by
   shape plus label, and the domain enums removed.

Each phase is accepted by real turns on the local Ollama route through the
web app in the original Agent conversation: the pantry request, the herb
guide, a single-card request, a request with text and cards, a code change
with a check, a distant follow-up and a topic switch. Each is read back
from the ledger (calls, results, control records, receipts) and seen in
the browser. Fixture tests prove contracts; they do not stand in for these
runs.
