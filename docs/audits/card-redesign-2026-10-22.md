# Markdown answers and runtime cards

Status: historical. Implementation and live acceptance on 2026-10-22,
before the 8.28.0 release. These results cover this corpus and route.

## Change and verification

Step 2 (`acaa6734d`) offers only chart, stats, map, calendar and controls.
Every form builder remains available for history, inline fences and runtime
records. Markdown is the ordinary answer contract, including interactive
tables, lists, quotes, Mermaid and diff code. Shipped answer skills follow it.
The blueprint now measures offered tools separately from retained builders.

Step 3 (`9037a9c4d`) records sources from successful `fetched()` calls and
actual file diffs through `with_changes_made` with no paths. These are
`PresentationSource::Derived { from }` records, projected at their ledger
positions after the answer and omitted from historical dialogue's missing
presentation supplement. No model call builds them. The old command-output
and ability-data filling functions and their card-invocation tests are gone.

The initial full gate passed. The first step-2 gates exposed stale prompt
wording assertions and scheduled fixtures invoking the retired callout tool;
those fixtures now test the current contract. Focused checks passed for all
agent tests, derived serde/history, projection ordering and all eleven
scheduled-run cases. The final combined full gate passed formatting, Clippy,
workspace tests (823 seconds), versions, doc paths, records and advisories.
No gate was waived.

## Live method

Built both development binaries and used the `vak-window-live` recipe on
port 8941, workspace `/tmp/vak-live/window`, default data home, no gateway.
Health reported the existing `openai-responses / gpt-6-luna` route. No route,
permission setting or installed service was changed during acceptance.

Ran `scripts/scenarios/conversation_continuity.py` over the complete frozen
corpus. Every one of the thirty settled answers and every card was read.
The [captured outputs and fetch receipts](fixtures/card-redesign-acceptance-2026-10-22.json)
come from authenticated snapshot/transcript APIs, not raw ledgers or secrets.
The full repeated snapshots remain at `/tmp/vak-live/card-redesign-continuity.json`.

| Case | Read result |
|---|---|
| observations, four turns | Riverwatch stays R1=10, R2=12, R3=11, R4=25, R5=26. Largest step remains +14, R3→R4. Charts preserve the supplied values and observation order; missing timestamps/units are explicit. |
| options, four turns | Neither Cedar nor Birch meets both three-person capacity and offline operation. The final independent answer is exactly `The lantern is blue.` |
| original-followup, five turns | Preserves Indian NSE/BSE and the 22 October close through the capability interlude. Reports failed retrievals, then retrieves Yahoo data and qualifies the provisional report, missing breadth/drivers and inconsistent provider fields. No replacement subject appears. |
| ambiguity, three turns | Preserves North=`Meet at 9` and South=`Meet at 10`; asks which to change before making either `Meet at 11`. |
| long-continuation, fourteen turns | Each of twelve arithmetic interludes returns exactly `5`. The final answer recovers Aster 13,18,16 and the correct +5/−2 changes. Net +3 is not explicitly restated; every original observation and the requested largest change are present. |

Only the current chart and stats tools were called (three chart calls, one
stats call); no retired card tool or layout plan was called. There were
thirty webfetch calls, with successful and failed results preserved.

The runtime produced two source cards, after the answers in market turns
four and five. They contain respectively fourteen and five URLs. For each
card, its URL set equals exactly that turn's successful webfetch receipts;
failed/time-limited/403/404 calls supply no source row. Neither card has a
model tool-call identity. Source rows state retrieval, never synthesize a
finding. Reported market prices and changes were checked against the captured
JSON metadata; this is not independent certification of the provider's data.

## Limits and remaining work

Riverwatch's detailed answer repeats findings across charts, stats and prose.
The market answer initially overstated its inability to retrieve known URLs;
it later corrected that claim after successful retrieval. These remain
semantic quality observations, not grounds for a word list, card quota,
forced tool call or model blame. No source-discovery capability was added.

The corpus forbids file creation, so live diff delivery is not claimed. The
runtime test exercises actual writes to an existing and a new file, real
hunks/counts, no model card call and no carry-over into the next turn. The
server test exercises ledger-position ordering through the real projection.
No new form renderer or frontend bundle changed. This run does not qualify
all viewport/theme/native combinations, arbitrary answer quality, interactive
artifact revision, or the unfilled live summary path.
