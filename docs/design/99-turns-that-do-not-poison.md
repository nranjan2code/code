# 99 — Turns that do not poison later turns

Status: in progress (2026-10-17): stages A to D built and checked live on
the installed route (B and C seen in the model's own reading); E and F
designed, open in docs/plans/open-items.md. Builds on docs/design/68-context-engine.md (the planner and
recall), docs/design/85-turn-graph.md (turn ids), docs/design/86-model-plans-runtime-checks.md
(the plan check) and docs/design/30-render-architecture.md §30.1
(citations).

## 1. What happened

One conversation on 2026-10-17 asked the same thing four times: a bike
route between two places, with a map. The first time, a model looked
both places up, fetched a road route and showed a summary and a map card.
Then a few turns on another route went badly: maps pages refused (403),
lookups failed, coordinates were made up and written to a workspace file.
After that the first model, given the same request again, failed three
times: it copied the failed turns' approach, reopened its own refused card
as "what we answered earlier", and fell back to prose.

Nothing in the model changed between the first answer and the failures.
What changed was what Vak handed it about earlier turns. Every way one
turn feeds a later one carried the bad turn's output without saying it
was refused, failed or never checked.

## 2. The rule

**Anything a turn produced reaches a later turn with the typed facts
of how it was established.** A later turn may see a failed or refused
result; it must never see it as an ordinary one. The facts are receipts
(a call failed, a card was refused, the plan was left short), never a
reading of words, and never a judgement of quality.

## 3. The paths a turn feeds later turns, and what each lacked

| Path | Where | What was missing |
|---|---|---|
| Citation evidence | `SessionLog::evidence_text` | Counted every non-card call's *arguments* (written by the model) and `recall` results (which re-quote failed and refused results) as retrieved. A URL the model typed, or one quoted in a 403, became citable. |
| Carried turn card | `TurnCard::line`, `planner.rs` | `did: webfetch×6` read the same whether all six succeeded or all failed; refused cards were invisible; `outcome` was `completed`. |
| Recall | `vak_tools::recall`, `pres:`/`turn:` | A reopened card or turn carried no word on how its turn ended. |
| Plan check | `card_unplanned`, `vak_judge::plan` | A card naming a deliverable the plan listed as text was refused ("Your plan lists no card"), so the content fell to prose. |
| Workspace files | shell and `write` | A file written in a turn whose calls failed carries nothing to say so. |
| Memory notes | `remember` | Same as files: a note from a failed turn reads like any other. |

## 4. Stages

### A. Evidence is what was retrieved

A card cites what the person said and what a call that succeeded
returned. Not evidence: any call's arguments, a failed call's result, a
card call, and `recall`'s results (they reopen evidence that is already
counted from where it was first received, or was not). A URL is citable
only where a result that succeeded or a person's message names it.

### B. A turn card says how its calls ended

Each `TraceLine` gains `failed`, taken from its result's `is_error`, and
`TurnCard` gains `unmet`, both additive with serde defaults. The card
line shows them: `did: webfetch×3 (2 failed), emit_link_card (failed)`;
a refused card is a failed card call. The card also records the plan's
deliverables the turn ended without (`unmet`, from the plan check the
loop already runs): `→ not delivered: "route map"`. The planner's choice
of turn is unchanged; what changes is that a failed turn reads as one.

### C. Recall says how the turn ended

`recall` `open` of a presentation, a turn or evidence names the turn it
came from and that turn's counts, so "what we answered before" is read
with its record: a card as `{card, from_turn}`, a turn with its `record`
line, a failed result as `{failed: true, result}`. Search hits carry
`Calls:` and `Not delivered:` lines. `turn:#n` is turn number n, and a
turn id the conversation does not have is not found (it was reported as
history still warming).

### D. A card may deliver a deliverable planned as text

The plan is the model's own declaration. A card whose `deliverable`
names a planned deliverable of form text delivers it: shown, and covering
it in the plan check. A name the plan does not have is still refused.

### E. Files and notes made in a turn that failed (designed)

A file version recorded for a turn (`vak_core::artifacts`) and a memory
note record the turn id they came from; reading either back names that
turn's counts. Nothing is withheld; it is labelled.

### F. Setting a turn aside (designed)

A person can remove a turn from what later turns see without deleting
anything: invariants 2 and 42 keep the ledger append-only. Setting a turn
aside appends a `TurnSetAside { turn }` entry (undo appends
`TurnRestored`). A set-aside turn is skipped by the planner, by recall,
by citation evidence and by memory writes derived from it, exactly as
`behind_reset` turns are; the transcript shows it folded, with Undo.
"Retry from here" is a branch (a new directive whose parent is the turn
before), already the ledger's shape. Erasing bytes stays
`Core::erase_conversation`.

## 5. How to find more of these

- **Poison replay.** In a scratch data home, take a conversation where a
  request succeeded, append one turn of failed calls and an unchecked
  claim, then ask the same request again on the same route. Compare the
  plan check, card refusals and citations with the clean history. A
  difference is a path in §3 that still carries something unlabelled.
- **Repeat audit.** Over a conversation's turn records, pair turns that
  carry the same request and compare their counts and plan verdicts; a
  later turn doing worse is a lead.
- Every new path from one turn to another (a rollup, a projection, a
  learned pattern) is added to §3 with the facts it carries.
