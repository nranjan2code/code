# ChatJimmy response trace — 2026-10-19

Status: reference: read-only installed-home investigation, with an actual-source
hydration probe; no runtime repair, installation or new research was performed.

## Observed failure

The owner's request, “what is impact of chatjimmy.ai study deep and its
parent company and get all latest details and all”, ran in the Vakyartha
conversation `01a13fe9-002e-7f43-a4e3-80a88b4a0312`, run
`run_01a152b3-5c6d-7302-837e-7af21bb1329f`. It opened at 11:17:28 IST
and settled at 11:23:29 IST. Installed health reports 8.12.0.

Inspection used authenticated local HTTP APIs (sessions, admin transcript,
presentation, runs and health), never credential-file or private-ledger reads.
The screenshot and the running desktop accessibility tree both show only
“Scope and confidence” after the request. The running tree also shows that
older requests have “Set this turn aside”, while this request lacks it.
The owner had independently started a follow-up by the time of UI inspection;
this investigation did not interrupt it or submit another turn.

## What the record contains

The admin transcript records 26 model steps. The presentation snapshot has
76 tool-result progress items for turn-27, three complete structured cards
and the closing prose. Google/Bing search fetches account for 48 calls
(45 Bing, three Google); DuckDuckGo was tried too.

At 11:23:01 the model replanned into key findings, sector implications,
sources and confidence. At 11:23:24, three card tools succeeded:

- Summary: presentation `01a152b8-c888-7d12-8ec6-8273c16a19b9`.
- Table: presentation `01a152b8-c88b-7a00-8fd8-cb935931636d`.
- Sources: presentation `01a152b8-c894-7810-816e-e5fa14e21e93`.

All three acknowledgements say “Shown to the user”. All three cards exist
in the authoritative `/sessions/{id}/presentation` snapshot and its answer
outline. At 11:23:29 the model wrote the confidence paragraph. Therefore
this is not a model that wrote only that paragraph, nor lost server output.
The acknowledgement establishes recording, not actual client visibility.

## Client delivery failure

`crates/vak-client-ui/src/components/ChatPane.tsx` selects the settled
projection through `serverTurnFor`, which requires the user message's
saved `entryId` (`src/turnPairing.ts`). An optimistic live user message
has none until transcript hydration. Without that match, ChatPane draws
its legacy stream fallback. `LiveAnswer` in `PresentationRenderer.tsx`
only draws cards with `provenance.source == live_event`, excluding settled
cards. Consequently a settled snapshot can contain every card while the
fallback shows only the streamed closing prose. The missing history action
is consistent with the absent entry id; the app's private JS store was not
inspected, so that inference is distinguished from direct API evidence.

A concrete defect exists in `src/App.tsx::hydrate`: `lastHydrated.set(id,
read)` happens before the running-state guard which may skip
`hydrateFromTranscript`. A later identical settled read then returns as
“unchanged”, despite never applying the earlier read. A probe transpiled
the actual function from this checkout and supplied a running read followed
by two identical settled reads: three reads, zero transcript applications,
cache marked applied. This proves the failure schedule is reachable in the
function; the exact original event interleaving was not captured.

Repair scope: cache only the transcript/projection actually applied, retain
an explicit pending application across running-state transitions, and test
the running-to-settled path with an optimistic user lacking an entry id and
settled cards already present. Do not pair turns by position or words.
Verify visibility in the running desktop and both browser sizes/themes.

## Research and completion failure

Google returned an unusual-traffic challenge. DuckDuckGo returned almost
no readable content. Many Bing results were unrelated to the query:
bike comparisons, YouTube help, school sites and insurance exams. The
recorded page titles contain the requested search terms, but the result
bodies do not substantiate them. The renderer uses separate throwaway
browser profiles (`crates/vak-tools/src/webbrowse.rs`); these records do
not establish a shared-tab race or why the upstream bodies were unrelated.

The model eventually read ChatJimmy's model/health endpoints, Taalas's
site, terms and privacy policy, and RDAP. Its output ties the service to
Taalas, reports the advertised hardware efficiency as a company claim,
and offers mostly generic sector possibilities. Those are claims in the
original answer, not independently reverified facts of this investigation.
There is no adequate retrieved evidence here for a comprehensive current
company history, funding, competitive position or measured disruption.
Failed retrieval does not establish that public evidence does not exist.

The final outcome's only requirement says “response content exists” and
is marked met; the turn is labelled produced/complete. `vak-judge` checks
that structural condition in `src/outcome.rs`. It establishes output,
not that the requested research depth, coverage or actual client display
was achieved. Search capability and research sufficiency therefore need
follow-up alongside the client fix; a card-rendering repair alone will not
make this a deep disruption study.

## Local evidence

Private API snapshots and the source probe are under
`/tmp/vak-chatjimmy-trace/`; no tokens were saved. `recovered-answer.md`
contains all three original card fallbacks and the closing paragraph.
The evidence directory is temporary and is not part of the repository.
No source change, service restart, release or installation was made.

## Implementation follow-through

The original observations above are the investigation record. The later
[conversation/evidence contract repair](conversation-evidence-contract-2026-10-19.md)
records local implementation, live runs, tests and remaining acceptance limits.
It does not alter this original conversation or install a new app.
