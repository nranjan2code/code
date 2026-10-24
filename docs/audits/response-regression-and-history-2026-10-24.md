# Response regression and two-month history — 24 October 2026

Status: historical. GitHub history inventory and focused debugging of the
installed 8.31.2 conversation. Source corrections are uncommitted and are not
an installed release. Rich-answer repetition remains unresolved.

## Assessment

The complaint about repeated changes without reliable everyday behavior is
supported by the response history and today's reproduced regression. This
does not establish that every architectural change was wrong, or establish
any contributor's intent. The concrete failure is a new request's live
placeholder being attributed to the previous saved request. The Markdown
answer is separately explained by the deliberately shipped authoring policy.

Yesterday's architecture review already identified repeated ownership and
acceptance problems. Another architecture proposal is not the missing work:
the product sequence must be verified against the actual stream/store/UI.

## Coverage and evidence

- Enumerated all **2,517 GitHub commits** returned for 24 August through
  24 October UTC in `nranjan2code/code`, in 26 paginated API responses.
  Every returned SHA is reachable locally. GitHub `main` and the checkout
  both name `4d8ffeea1a9665c9f9cfdbc0d86960f444d05067`.
- Committer-month counts are August 254, September 1,211, October 1,052.
  These include releases, generated assets and documentation, and are not
  counts of defects or architectural rewrites. Ordinary `git log --since`
  returned a different population; the GitHub inventory is the denominator.
- Reviewed source history and selected diffs along the response contract,
  prompt, form tools, server projection, client hydration and acceptance
  records. This is **not a line-by-line review of all 2,517 commits**, all
  branches, or every subsystem's security and persistence behavior.
- The installed authenticated API reports `8.31.2 / 049f8dbcb`. Its latest
  vak-home snapshot/transcript were inspected read-only through HTTP; no
  credential files or real ledgers were opened or edited.
- The complete metadata/link inventory is retained at
  `/tmp/vak-github-history-2026-08-24-to-2026-10-24.md`; raw GitHub pages are
  `/tmp/vak-github-two-month-commits.json`. These contain no API token.

## Verified findings

### 1. New live steps contaminate the previous answer

Introduced by [b9ada8ca3, 23 October](https://github.com/nranjan2code/code/commit/b9ada8ca3799f50ebe02798838b08bd92c9ab547).
At that revision, `crates/vak-server/src/projection.rs:2511` assigns a new
live assistant item the last user item's turn ID in the cached projection.
That projection still describes the preceding request. An empty placeholder
therefore enters the old turn, and the client draws its empty document as
“No result.” Each model step can add another placeholder, explaining the
multiple notices in the supplied screenshot. The saved answer itself is intact.

The real ChatPane regression failed after its original 22 checks with that
exact visible warning. The correction carries `latest_directive_entry_id()`
from the agent's recorded session in `AgentEvent::TurnStart`; the projector
no longer guesses it from cached user items. Settled ChatPane selection also
excludes `live_event` material. This retains the existing ledger authority.

### 2. The store ignores draft removal

In `crates/vak-client-ui/src/store.ts:1170` at the inspected revision,
`item_removed` goes through a single-item reducer, producing an empty list.
Reading `[0]` then yields undefined and the following guard performs no
store update. The pure reducer supports removal, but the real store does not
apply it. A rejected live draft can remain until authoritative hydration.

The correction routes removal through the existing whole-list reducer and
rebuilds its ID index. The component regression checks the actual store and
visible ChatPane, not only the pure reducer.

### 3. Markdown was a shipped decision, not a lost-card rendering failure

[acaa6734d, 22 October](https://github.com/nranjan2code/code/commit/acaa6734d84149f9dcf84e829bfa38d1eb509891)
explicitly made Markdown the answer contract and restricted offered forms to
chart, stats, map, calendar and controls. The latest saved market run contains
prose, a document table and a derived sources card; there is no emitted stats
or chart card for the renderer to recover. The weather run likewise contains
prose and retrieval receipts. Changing CSS cannot supply missing authored cards.

The working correction offers the existing summary, steps, list, details,
table, comparison and callout tools alongside the existing interactive tools.
Its prompt selects by information shape and permits short direct prose.
Sources and diffs retain their runtime evidence writer. No domain classifier,
word list, card quota, new layout planner or dependency was added.

### 4. Acceptance did not cover the failing product transition

The [23 October consolidation record](rich-response-consolidation-2026-10-23.md)
records broad automated checks and explicitly excludes installed WebKit and
live-model quality from its own qualification. The subsequent
[8.31.1 installation record](release-8.31.1-2026-10-23.md) adds existing-history
rendering and a fresh arithmetic answer. Those checks do not establish that
a new live request leaves the preceding rich answer intact. Today's failure
demonstrates that gap despite the reported passing gate. A valid binary digest
and one successful answer do not qualify this transition.

## Changes of direction visible in the history

| Date | Change | What it establishes |
| --- | --- | --- |
| 3–5 September | `42f020842`, `63be08231`: presentation suite, then replacement delivery pipeline | The output implementation was already being reorganized. |
| 10 September | `026b52282`: guidance and rendering for 52 seeded card types | A broad card vocabulary preceded the later form model. |
| 16–18 September | `b101dfd9e`, `719448888`, `de579aac7`: restore settled answers, diagnose invisible results, display empty-result fallback | Visibility failures and their symptoms were already a recurring concern. These are distinct from today's newly introduced identity bug. |
| 14 October | `3abc37045`: cards become eighteen forms | A substantial replacement: 148 files, 4,771 insertions and 14,769 deletions in that commit. Size alone does not prove it wrong. |
| 16–19 October | `e30a68764`, `357e8e0b2`, `56ad9d1e9`: card completion and plan/summary rules | Authoring and completion contracts continued changing. |
| 22 October | `9d09f9c24`, `acaa6734d`, `9037a9c4d`: remove layout plans, prefer Markdown, derive receipts | The visible answer policy changed again; this directly explains today's prose output. |
| 23 October | `b9ada8ca3`: consolidate accepted response selection across surfaces | Shared selection addressed real duplication, but its live identity assignment introduced today's regression. |

Source churn also concentrates in existing authorities: the interval's numstat
scan records 518 touches to `vak-server/src/lib.rs`, 258 to
`vak-agent/src/lib.rs`, 353 to `vak-core/src/lib.rs`, 125 to server
`projection.rs`, and 83 to `presentation_tools.rs`. These are change-pressure
indicators, not demonstrated bugs or a reason to split files by line count.

## Verification and remaining limits

- Browser baseline reproduced the false warning; corrected ChatPane passes
  **26 checks at desktop and 390-pixel width**, including preserving previous
  result DOM/card content and removing a discarded draft from the real store.
- Server projection suite: **47 passed, one existing measurement test ignored**.
- Core form tests: **35 passed**; cross-layer form registry test: **one passed**.
- Recorded-directive agent event test: **one passed**; prompt contract tests:
  **20 passed**. Targeted Clippy for agent/core/server/CLI and all their targets
  passes with warnings denied. Record, doc-path, version and whitespace checks pass.
- Both frontend bundles build/typecheck. Five focused Node scripts pass:
  conversation hydration, presentation hydration, history pages, turn pairing
  and answer composition.
- A development CLI/server/worker build completed. A fresh disposable workspace
  on port 8937 used the existing `openai-responses / gpt-6-luna` route. Two
  natural supplied-data requests produced accepted table/summary and comparison
  forms. Both independent follow-ups returned exactly `The lantern is blue.`
  Original values, the +14 change and both options' unresolved constraints
  were preserved. No domain-specific policy selected those forms.
- **Quality remains incomplete:** the observation answer repeats findings
  across table, summary and closing prose; the option answer also repeats its
  conclusion in prose. Card presence is a transport/authorship result, not a
  complete semantic-quality pass. No further prompt permutation was applied
  to disguise this result.
- Live captures are `/tmp/vak-response-20261024/answers.json`; deterministic
  logs are `/tmp/vak-turn-boundary-tests.log`,
  `/tmp/vak-rich-forms-tests.log`, `/tmp/vak-form-registry-tests.log` and
  `/tmp/vak-turn-boundary-ui-build.log`.
- The installed application still runs 8.31.2. No commit, push, release or
  installation was performed. The whole architecture, other domains,
  external channel clients and the installed native turn transition are
  **not qualified by these focused checks**.
