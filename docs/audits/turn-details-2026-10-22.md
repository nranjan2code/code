# Request-focused Details — 2026-10-22

Status: historical. Shared-client implementation and browser acceptance;
installed native WebKit qualification remains open.

## Change and evidence boundary

The maintainer approved the turn-focused right-panel proposal. The previous
panel stacked all conversation steps over a separately scrolling command
history, hid command identity behind repeated completed-step labels, and did
not share a selected request across activity and files.

Details now follows current work or pins a recorded request. Activity, Files
and Evidence read the same request; history and commands without a recorded
link are explicit. The selector, follow control and connection state remain
outside one scrolling body. The existing Workbench is a deliberate command
inspector; file Review remains the existing version-bound sheet, with checks,
comments, decisions and Undo. The misleading local clear-history control was
removed. Broader panels state their actual feed/workspace/conversation scope.

No new durable state, runtime event schema, model calls, permissions or
provider dispatch were added. Selection is per conversation in client memory.
No domain classifier or inferred topic/time join is used. Per-model-step live
frames attach to the active directive through the live projection boundary;
recorded calls supply explicit durable joins. Missing completion events and
unreadable records never establish success. Worker UI state now distinguishes
an observed finish from whether a disclosure is open, and interleaved worker
updates match their event label.

## Verification

- `npm --prefix crates/vak-client-ui run typecheck` and the Details component
  fixture typecheck pass. The focused Node check covers identical request
  text with different call ids, unlinked commands, live-frame reconciliation,
  admission through a shared call, interrupted work and source-link safety.
- All 44 client Node checks pass (the first run found a plain-word violation;
  its labels were corrected and that check rerun successfully).
- The real `DetailsPanel`, store, file cards and Workbench run in Vite at
  1440 × 900 and 390 × 844, light and dark. The new browser fixture passes
  28 assertions per combination (112 total), with no page errors or horizontal
  overflow. Tests cover live/default selection, pinned selectors and content
  during updates and settlement, scoped files/checks/links, partial results,
  unavailable Library state, failed/unreadable checks, Review, command details,
  unlinked commands, single scrolling ownership, reading-position preservation,
  explicit tail jumping, interleaved worker finishes, offline status,
  unfinished-call honesty and absence of write requests during inspection.
- The existing Review-formats suite passes 235 assertions per combination
  (940 total), with no page errors, for desktop/phone and light/dark.
- Additional desktop light and phone dark checks at 125% text size pass:
  selector and footer fit, there is no horizontal overflow, visible text stays
  above the 12px floor, and the phone uses coarse-pointer control geometry.
  Live and failed-check views were visually inspected.

The fixture uses synthetic recorded requests, file-state answers and checks.
It exercises the shared production components and client event path; it does
not prove live-provider output, Office parsing fidelity or installed WebKit.
Browser logs and screenshots are under `/tmp/vak-turn-details-acceptance/`.
Both client production bundles and the development CLI/server binaries build.
The full `scripts/gate.sh` passes: formatting (4s), workspace Clippy with
warnings denied (63s), workspace tests including doc-tests (785s), version,
document paths, records and advisories (7s). The memory load-matrix test
completed successfully in the full run; no test failure required a retry.
The user requested committing all work after browser acceptance. Implementation,
tests, records and the rebuilt web bundle are committed together; generated
blueprint views follow in their own commit. No release/install or real file
acceptance was performed.
