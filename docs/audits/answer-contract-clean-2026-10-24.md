# One typed answer path — 24 October 2026

Status: historical. Implementation and non-browser acceptance of the maintainer's
clean replacement request. Source is uncommitted and the installed 8.31.2 app
has not been replaced. Final expanded browser/native qualification remains open
because local browser access was disabled and the maintainer explicitly chose
“Continue non-browser checks.”

## Contract and implementation

Rust's existing session ledger is still the authority. Typed card tools validate
arguments, convert them to the form's stored payload, validate that payload, and
record accepted content. Runtime sources/diffs and file writers keep their own
provenance and authorization. Argument schemas are not interchangeable with
stored-payload schemas; existing conformance tests exercise their real conversion.

`AgentEvent::TurnStart` carries the actual recorded directive ID and original
submission key. The client matches its optimistic request by that key. Neither
surface nor server takes the last request from a stale snapshot as authority.
Typed provenance distinguishes live content from recorded content; an ID prefix
is not a lifecycle state.

`PresentationTimelineView` now renders both in-flight and durable prose, cards
and files. Saved answers remain mounted while a new run starts. Empty running
placeholders cannot produce finished-result warnings. Draft removal updates the
whole indexed list. Cancellation retains content with its actual stopped state;
settlement replaces live items. Settled-result actions are unavailable on live
material. Existing file review and card-action services remain the writers.

Removed implementations and protocols:

- The transcript assistant/card renderer and its independent file attachment
  reconstruction, card grouping, and latest-assistant-only selection.
- Client inline-card parsing from fences, paragraphs and raw Markdown fallbacks.
- The agent's inline-fence presentation writer, fence parser and repair loop.
- Server interpretation of prose as card candidates or adaptive payloads.
- Delivery text/fence projection helpers, the unused inline skill-registry field,
  and its plugin registry loader in the server delivery path.
- Retired `PresentationSource::Fence` and `PresentationSource::Answer` readers.

Code examples mentioning `semantic_type`, including `vak`-tagged code and malformed
JSON examples, remain document code. They cannot create a card or trigger a card
repair. There is no migration/compatibility implementation for the retired
protocols. Delivery jobs reject removed fields; the worker protocol advances to 2.
No domain classifier, vocabulary trigger, mandatory plan, card quota, separate
model presentation pass or new dependency was added.

Existing summary, steps, list, details, table, comparison and callout tools are
admitted alongside stats, chart, calendar, map and controls. The model chooses
by information shape. Labelled collections use typed forms; prose explains them
or gives an unstructured/text-only answer. Accepted-card receipts explicitly
state that the card is answer content and that completion needs no repeated prose.

## Executed evidence

The [captured accepted payloads and identity receipt](fixtures/answer-contract-clean-2026-10-24.json)
record the exact development CLI/worker hashes, UI manifest, dirty runtime-source
digest and baseline Git SHA. These are not release artifacts. The final live run
used the configured `openai-responses / gpt-6-luna` route in a disposable workspace
using frozen CLI/worker images on port 8938; no route or installed service was changed.

- Observations: a typed summary preserves R1=10, R2=12, R3=11, R4=25, R5=26
  in order and identifies +14 from R3 to R4. The independent
  follow-up returns exactly `The lantern is blue.`
- Options: a typed comparison preserves cost, capacity and offline constraints;
  neither option meets both requirements. No full second prose answer follows.
  The independent follow-up returns exactly `The lantern is blue.`
- Labelled measurements: a typed summary retains 22.2°C, 24.8°C, 81% and 4.7 km/h
  and correctly calculates the 2.6°C difference. No second prose answer follows.
  The next arithmetic answer is exactly `14`. **Strict semantic quality fails:**
  it adds the qualifier “air” although the supplied reading did not say what
  the temperature measured. This is preserved as an unresolved quality finding,
  not counted as a completely grounded answer.
- An actual SSE run associates its new live item with the eventual recorded
  directive. All **7** preceding projected items remain byte-equivalent as
  JSON values after the next request; the reply is exactly `17`.

This is finite transport/authorship acceptance with explicit semantic limits,
not universal accuracy or a complete semantic-quality pass. Other runs earlier
in this session used a table with a concise interpretation, or produced more
redundant summaries/metrics. No runtime text matching
or repeated prompt tuning was added to suppress particular sentences.

## Verification boundary

Both frontend bundles build and source TypeScript checks pass. All **47** Node
regression scripts pass. The modified ChatPane, presentation, composition and
wire-contract fixtures pass strict TypeScript checking. Workspace Clippy passes
with warnings denied. The final `cargo test --workspace` exits successfully:
**3,913 passed, zero failed, 25 ignored**, across 144 test/doc-test suites.
Ignored tests are the existing external-service, manual browser/performance,
real-server and documentation cases; none were newly skipped for this change.
Formatting, diff whitespace, documentation paths, record consistency, version
stamps and dependency advisory checks pass.

The original browser regression reproduced the screenshot's false warning and
passed its initial 26 checks after the narrow fix. The expanded unified-renderer
fixture reached 27 successful assertions before an incorrect `.card-summary`
selector failed; the real summary uses `.answer-summary`. That fixture selector
is corrected and the expanded fixture typechecks. **It has not been rerun in a
browser after the final cleanup.** No final browser/native visual or viewport
acceptance is claimed. The user explicitly chose non-browser checks.

Logs/captures are retained under `/tmp/vak-answer-clean-20261024/` and in
`/tmp/vak-response-final-ui.log`, `/tmp/vak-response-final-build.log`,
`/tmp/vak-response-final-clippy-verified.log`,
`/tmp/vak-response-workspace-final.log` and
`/tmp/vak-response-final-node-tests.log`.
