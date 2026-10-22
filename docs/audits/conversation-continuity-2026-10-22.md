# Conversation continuity and prompt provenance

Status: historical. Investigation and correction on 2026-10-22. Verification
results below describe the measured cases, not arbitrary answer quality.

## Evidence and design review

The installed market conversation emitted structured forms, including a chart;
this was not a global loss of card events. Its short continuation after a
capability question instead produced an unrelated earthquake report. A recorded
Research Analyst interface also described its consuming surface as a generic
HTTP API, while the built-in Agent's interface described the rich web client.
These observations do not establish that a surface label caused topic drift.

The context engine supplies one recent settled TurnCard. Its compact reference
contains the latest request and the first presentation title or a short opening
of its narration. A clarification or capability exchange can therefore displace
the substantive subject. Existing recall could search a specific subject or
open an exact address, but could not discover preceding exchanges chronologically.
Requiring the omitted subject as a search query is a circular dependency.

An Astra design review confirmed this mechanism and recommended chronological
discovery and main-model referent resolution as the smallest coherent correction.
It advised against a fixed recent-turn count, inferred active-task state, domain
routing, a mandatory secondary reviewer or another renderer. Its implementation
review checked branch visibility, provenance and surface-pool isolation and
identified the initial-browse and synthetic-ordinal edges addressed below.

## Static and dynamic instruction inventory

| Owner | What it contributes | Result of audit |
|---|---|---|
| Shipped seed and editable identity/rules | General work guidance; an Agent can replace identity and operating style | Identity/style cannot restore information omitted from context. Referent resolution belongs in the shared code-owned history contract. |
| Code-owned capability/history contract | Current tools, source discovery, history resolution and Topics | Extend existing history navigation without choosing a subject or reading current request words in the runtime. |
| Code-owned presentation contract | Form choice from actual information relationships and admitted capabilities | Preserved, including no repeated findings and no invented data. No topic recipes or card quotas added. |
| Runtime surface and workspace | Where an answer is consumed and where tools work | Pooling an Agent/workspace discarded the caller's surface. Preserve it on the resolved Core clone, for rich and non-rich callers alike. |
| Runtime inventories and tool schemas | Admitted capabilities, deferred catalogue, skill names and callable arguments | New navigation is an intrinsic recall operation. No arbitrary tool-name map, provider branch or preferred domain tool sequence. |
| Loaded skills and companion wrapper | On-demand procedures and nonexclusive related tools | Existing coordinated procedure correction remains. No additional compulsory answer layout. |
| Per-turn context and controls | Clock, recorded intent/work state, current directive, historical candidate, tool results and correction messages | Current meaning may depend on earlier work through interludes; old approval remains historical. Tool results record every discovered/opened record. |
| Main-model plan and accepted receipts | Requested outcomes, evidence questions, delivered semantic forms | Existing structural checks preserved. Neither delivery nor navigation proves evidential support or semantic completeness. |

The selected Research Analyst identity describes its perspective and evidence
style. It does not impose a financial domain. The correction applies equally to
user-created Agents and the built-in Agent; the current message and verified
conversation evidence determine the subject.

## Implemented contract

A request carries the open turn verbatim and the newest visible settled
exchanges whole while they fit the measured budget, each under a
`<historical_exchange>` header with its stable address, outcome and unmet
state. What no longer fits is one stored summary that starts at the oldest
visible exchange: the planner reuses the one that leaves the most exchanges
whole beside it, and otherwise asks for a summary of everything but the newest
exchange, which the loop writes before the step. Between two summaries the
request only grows at its end. Summaries read each turn's request and answer
whole beside its card line. A summary that could not be written is left out of
that request, never recorded as sent. Recorded selections replay the summary
from the packet stored on that chain at the time.

Automatic context uses one visibility iterator: active branch, latest
set-aside/restored marks, empty stopped exchanges excluded, reset-with-handoff
boundary respected. Planning measures exchanges newest first only until the
window is full. No word, inferred thread, file name, graph score or picked
number chooses a turn. `recall` keeps query and open for anything older.

### Interim attempt, withdrawn unshipped

The first implementation kept one adjacent exchange at Full fidelity and added
chronological browsing to `recall` (`before: "current"` or a turn id, newest
first, with a `next` cursor). It failed the live sequence below and was
replaced by the filled window before any release. Its historical dialogue
renderer and visibility iterator are kept.

## Verification

Focused checks cover address grammar, first-page discovery, paging, invalid and
abandoned cursors, page exhaustion, empty stopped turns, set-aside/restoration,
handoff boundaries, actual dispatched tool-result content and durable logging.
Surface tests cycle through Web, Desktop, Server, Chat and Background using the
same pooled workspace, checking that consuming-surface state does not leak.

The first live implementation demonstrated subject recovery for supplied
measurements. It also exposed a real argument-contract issue: the model sent a
stable turn id without the prefix and the parser rejected a combined browse/open
call. Making the turn-address field accept both equivalent spellings corrected
that API mismatch; it did not reinterpret the user's words. Initial source tests
also corrected test assertions that mistakenly inspected a closed-turn digest
instead of the dispatched open-turn result. The workspace gate found an iterator
lint, corrected before the final checks.

The browse-only live candidate failed the original five-turn sequence: after
clarifications and a capability question it asked the person to pick a topic.
The missing subject was present after the first twelve words of the preceding
answer and absent from the compact card. This blocked release and motivated
measured adjacent-exchange fidelity rather than another semantic prompt patch.

Test coverage previously emphasized isolated presentation prompts and scripted
mechanics. Some assertions locked in compact-only admission. Those assertions
now check the intended one-source policy and measured fidelity; safety, branch,
rendering and receipt tests remain. New deterministic regressions check complete
answers, historical status/address, exact replay, same-source downgrade and
chronological discovery. Live multi-turn scenarios additionally check meaning;
a scripted provider cannot establish semantic correctness.

### Filled-window live acceptance, 2026-10-22

Run with the corpus runner against a development build of this change on the
installed route (`openai-responses`, `gpt-6-luna`), one fresh conversation per
case, each output read in full:

| Case | Result |
|---|---|
| observations | Pass. Riverwatch and every value kept through the capability question; +14 from R3 to R4 in every turn; the final turn drew only supplied values. |
| options | Pass. Three people, offline use and the unresolved tradeoff kept after the tools question; the independent request returned exactly "The lantern is blue." |
| original-followup | Pass on continuity. After the clarifying answer and the capability question every turn kept the Indian all-sector closing report, named its source and said it had no search tool. Its last turn was still fetching data on that subject after the runner's limit and was stopped by hand. |
| ambiguity | Pass. Asked whether North or South; revised neither. |
| long-continuation | Pass on recovery. The final request carried all thirteen earlier exchanges whole (recorded plan, budget 39,885 tokens) and the answer restored 13, 18 and 16 with +5 and −2; it did not state the overall +3 the criterion lists. Every side answer was exactly 5. |

No case needed `recall` to find its subject; models still reopened a turn by
the address its header gave. No case filled the window, so the live summary
path was exercised by the scripted loop tests only
(`open_turn_growth_makes_older_history_give_way_to_a_summary`,
`a_long_conversation_is_summarised_once_and_its_newest_exchange_rides_whole`,
`a_small_window_summarises_once_per_fill_and_keeps_the_newest_whole`).

Seen in every case and left for the answer-side design: each turn ended with a
`topics` call, and two short answers were held for one and written again.

The long live continuation exposed another invalid policy: two independent
arithmetic requests correctly answered `5`, but narration equality was treated
as proof of semantic drift and the answer was retried with a steering correction.
The normalizer test only tested its own string transformation, not the claim
that equality establishes relevance. That test and the faulty detector are
removed; a two-directive integration regression verifies identical correct
answers settle without correction. This also removes its whole-history fold.

Astra's implementation review found that legacy Full strips accepted card
fences from narration. The adjacent-dialogue renderer now additionally retains
canonical presentation payloads when no complete local call/result pair represents
them (fence, delegated, scheduled or dangling-call sources). It preserves exact
presentation addresses and original positions. The legacy Full renderer remains
unchanged for old replay. Focused tests cover those source shapes.

The frozen [multi-turn corpus](fixtures/conversation-continuity-scenarios-2026-10-22.json)
records the original chain, supplied measurements/options, explicit topic change,
ambiguity and a return across twelve independent interludes. Its criteria require
semantic inspection of all output and receipts; tool/card counts alone cannot pass.

Reproduce the corpus with the [API capture runner](../../scripts/scenarios/conversation_continuity.py)
against a development server already opened to a disposable trusted workspace:

```sh
python3 scripts/scenarios/conversation_continuity.py \
  --base-url http://127.0.0.1:8939 \
  --token-file /path/to/private-dev-token \
  --output /tmp/continuity-evidence.json
```

The runner uses the configured route, creates a fresh conversation for each
case, records every settled snapshot and its transcript, and leaves semantic
review explicitly pending. It refuses to overwrite earlier evidence. Review
criteria include retained scope/constraints, grounded source dates, appropriate
forms, exact independent replies and clarification rather than invented targets.
A successful HTTP run cannot mark those criteria passed.
