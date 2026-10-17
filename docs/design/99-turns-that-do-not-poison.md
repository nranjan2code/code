# 99 — Turns that do not poison later turns

Status: in progress (2026-10-17): stages A to F and A2 to A4 built; A to
D, A2 and F checked live on the installed route (B and C seen in the
model's own reading); the poison replay (§5) is a repeatable check; retry
from a turn and the rest are open in docs/plans/open-items.md. Builds on docs/design/68-context-engine.md (the planner and
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
| Deferred tools | `tools_for_leg` | Never sent on any wire, so on a wire that only returns calls to tools it was sent, a deferred tool (`office_apply`, `mail_calendar`, …) could not be called at all. A turn asked for a Word file said the document tool was not available and wrote HTML. |
| Loaded skills | `SkillTool` | A skill that serves documents never named the turn's tools that serve them; the model looked for the document tool through `recall` and the shell. |
| Reopened turns | `read_session_turn` | A time, count and size cap picked in code made a long turn read back as "history is warming". |

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

### A2. Every admitted tool is callable on every wire

Each adapter declares whether its wire hands back a call to a tool it was
not sent (`ModelCapabilities::unlisted_calls`, through `Provider::reach`):
the local wire does; the Responses, Messages and Gemini wires do not; an
OpenAI-compatible server is not known. Only where it does is a deferred
tool kept off the wire until called by name. Everywhere else every
admitted tool is sent.

### A3. A loaded skill names the tools for its work

A skill's result ends with the turn's tools whose `serves` meets the
skill's own `serves` (`<skill-tools>`), read from each tool's
declaration: no tool is named in any skill's text, and a tool an
integration brings that declares what it serves is named the same way.

### A4. A turn reads back whole

Reopening a turn walks its own entries from its closing record to its
directive; only a cancel ends the walk early. "Still being indexed" is
said only when the turn is not indexed yet; any other failure says what
it was.

### E. Files and notes are read with the turn that made them

A file version recorded for a call (`vak_core::artifacts`) already named
its turn (doc 90 §3.5); a memory note now does too: `remember` writes the
turn of its call's trace key. Reading either back names that turn's
record, `TurnCard::made_in` (`vak_session::MadeIn`: the turn, its
conversation when it is another, its calls with how many failed, the
planned deliverables it ended without, how it ended), read through
`Core::turn_card`, the same scoped, trash-checked path as `recall`:

- a call that touches a workspace file whose current copy a turn made
  (`FileState::workspace_made`) carries, in the file note of doc 90 §3.2,
  `` `findings.md` in the workspace was made in turn … of this
  conversation: calls read (failed), webfetch (failed), write, remember ``;
- a `recall` hit on a saved note carries `made_in` with the same record.

Every such file and note is labelled, whether its turn went well or not:
the label is the record, never a judgement of it. Nothing is withheld. A
turn still open has no card and adds nothing; a file no version holds (a
`write` with no title to a path that is not yet an artifact, a file a
command made) has no turn to name: open in docs/plans/open-items.md.

### F. Setting a turn aside

A person can remove a turn from what later turns see without deleting
anything: invariants 2 and 42 keep the ledger append-only. Setting a turn
aside appends a `TurnSetAside` entry naming the turn's directive; Undo
appends `TurnRestored` (`vak_session::turn_marks::TurnMarkRecord`, with
the owner as actor). The last mark of a turn on the chain decides
(`set_aside_turns`); `TurnIndex` marks it `set_aside`, and
`Turn::out_of_view` (behind a reset, or set aside) is what every reader
of history skips:

- the planner never carries it, so a request follows the turn before it,
  and `derive_with_plan` never sends it even when a plan names it;
- `recall` neither finds nor reopens it: `turn:`, `#n`, `pres:` and `ev:`
  of a set-aside turn answer `set_aside`, the in-memory search skips it,
  and the catalog keeps the marks (`turn_marks`, branch-aware, for the
  turn search; `turns_aside` for every other search, so its calls, and
  a memory note naming it, are found by no one);
- citation evidence (`SessionLog::evidence_text`) counts nothing it
  received;
- what is measured and offered for promotion from turns
  (`vak_core::measure::turn_facts`) leaves it out.

A turn still going cannot be set aside. `PUT /sessions/{id}/turns/{turn}/aside`
(`{"aside": true|false}`, owner only) records the mark and answers the
turns set aside now; `/transcript` names them in `set_aside`. The client
offers Set aside on a person's message once its turn has ended and draws
a set-aside turn folded, with Show and Undo. "Retry from here" (a new
directive whose parent is the turn before) is not built: it is open in
docs/plans/open-items.md. Erasing bytes stays `Core::erase_conversation`.

## 5. How to find more of these

- **Poison replay** (`scripts/poison_replay.sh --provider … --model …`,
  `vak eval --poison-replay`). In a disposable data home and workspace,
  one request is asked twice in a clean conversation, and in a poisoned
  one asked, then answered once by a scripted turn whose `read` and
  `webfetch` fail and which writes an unchecked claim to `findings.md` and
  a note, then asked a third time, every live turn on the same provider
  instance. The scripted turn's request is the same words, so nothing the
  person said carries over: only what that turn did (the 2026-10-17 run
  was confounded because the poisoned turn's request named its approach).
  The last answers are compared by receipts alone: calls and failures,
  refusals, cards shown, unmet deliverables, corrections, cited links,
  the ending, and any tool only the poisoned turn had called. A difference
  exits 1 and is a lead to read in the ledger: a path in §3 that still
  carries something unlabelled, or the model's own variance.
- **Repeat audit.** Over a conversation's turn records, pair turns that
  carry the same request and compare their counts and plan verdicts; a
  later turn doing worse is a lead.
- Every new path from one turn to another (a rollup, a projection, a
  learned pattern) is added to §3 with the facts it carries.
