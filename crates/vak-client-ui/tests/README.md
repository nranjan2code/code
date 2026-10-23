# Presentation lifecycle regression

`/app/tests/composer-commands.html?run` mounts the real composer and chat pane
over a settled server projection. It checks visible, once-only `/status` and
idle `/pause`, `/resume`, `/stop` feedback, quoted file selection (spaces,
non-ASCII names and embedded quotes), and a two-skill/custom-command/file chain.
Use `&theme=dark` for dark mode. Run at 1440 × 900 and 390 × 844; the report
includes overflow detection. Pure mention syntax tests run with
`node crates/vak-client-ui/tests/file-mentions.mjs` from the repository root.

Run the real Solid components with isolated fixture data (no provider calls):

```sh
npm run dev:web -- --port 1421
agent-browser --session vak-render-check open http://localhost:1421/app/tests/presentation.html
agent-browser --session vak-render-check wait --load networkidle
agent-browser --session vak-render-check eval 'window.runChecks()'
```

The browser returns the passed assertions, or rejects on the first failure.
The fixture covers saved/live/durable cards, transport fences, completion refresh,
contextual task details, stable message nodes, scroll position, background task
isolation, and authenticated relative-asset loading. The sandbox check executes a script
inside an opaque-origin iframe and verifies its stylesheet using a test-only
postMessage receipt. `window.showArtifact()` opens the Workbench fixture.

These are deterministic component and transport tests. They do not claim that a
live model created a valid deliverable, that every renderer schema was exercised,
or that Tauri's installed binary has been updated. Production builds do not
include this test entry point.

# Canvas

`canvas.html` mounts the real `ArtifactCanvas` over a stub server whose file
routes (workspace, run, saved version) each answer with their own marker and
log every request. Run `window.runChecks()` as above. It shows that each kind of
subject is read through exactly its own route, an unreadable file is an error
and never a stand-in page, a bare path opens a run's file only on an identical
path from exactly one run, and a preview frame has the client-owned sandbox
and no-network policy. It also covers pointing at lines (which must not reload the viewer), Comment and Ask Agent on a saved version, its Activity and Changes, a newer version being announced without taking over, preview origins (framed from one when a page has files
behind it, closed with the Canvas, none for conversation markup), tabs, a Canvas per conversation, a
document opening on the whole viewport, and a viewer that fails. Run it in a
browser at 1440 px wide: below 1100 px the layout switch is hidden by design.
The pure rules are in `canvas-subject.mjs`, `canvas-stack.mjs` and
`preview-isolation.mjs` (`node tests/<file>`).

# Office presentation

`office-presentation.html` mounts the real Office view with a synthetic
two-slide PowerPoint projection. It checks the actual Canvas and presenter
components without a provider call or an on-disk document: the first slide has
an image and a chart with cached values, and the second has a table. In a web
dev server, open `/app/tests/office-presentation.html` and run
`window.runChecks()` in the browser console. It checks that presenter view
keeps the image, populated chart and table, and that the slide frame stays
16:9.

# Daily mail and calendar Canvas

Open `/app/tests/mail-calendar-daily.html?run` in the Vite web dev server. This
mounts the real daily Canvas viewer over local HTTP fixtures: nine synthetic
accounts, 72 mail rows, 300 events, and 24 free/busy intervals. Loading it with
`?run` runs `window.runChecks()` and prints the assertions. It checks that the
account request carries the owning Agent ID, each provider uses only its
granted mail/calendar operation, source labels survive aggregation, free/busy
shows no event titles, and provider requests stay within a two-account
concurrency limit. It has no provider credentials, provider traffic, or session
writes.

`/app/tests/mail-calendar-demo.html?run` exercises the product's explicit
**Use synthetic demo data** mode. It checks sample accounts and previews,
browser-local draft save/reopen, hard refusal of OAuth/provider effects, and
that no mail/calendar API request reaches `fetch`. It is loopback-only and
uses no credentials or session data.

`/app/tests/mail-calendar-connected-review.html?run` mounts the real Settings
panel against a same-origin fake connected Google account. It verifies that
Settings offers a direct entry to the daily Canvas, contains no duplicate inbox,
calendar, or draft workspace, and still supports Agent-scoped account and
routine administration. Its synthetic event-trigger routine remains paused
until explicitly resumed, exposes run history, and can be paused while running.
It asserts that no provider effect route or external request was made. The daily
Canvas fixture separately covers mail/calendar browsing, paging, draft editing,
and local Review preparation. These fixtures use fabricated data and do not
replace live provider conformance checks.

# Result card

`result-card.html` renders the real timeline over file results shaped like
the server's projection: two drafts waiting for review (only the newer one's
Review changes is primary), an accepted image, a file saved straight to the
folder, and one result with two files from one run.

```sh
agent-browser --session vak-result-card open http://localhost:1421/app/tests/result-card.html
agent-browser --session vak-result-card eval 'window.runChecks()'
```

It checks the status words, the one primary action, that the preview draws
the newest saved version in a sandboxed frame, that no size shows until
technical details are on, where Ask for changes sits, and that the answer
comes before its file. The words and the newest-draft rule are
`tests/result-card.mjs`; the status itself is derived on the server
(`draft_status_follows_saved_versions_and_acceptance`).

Add `?run` to run the checks automatically and display the report. The
fixture also covers consolidation of matching draft and folder file
previews, independent collapsed markup/file previews, on-demand file
reads and sandboxed expansion/collapse. `?run&theme=dark` selects dark mode.

# Undo after a reload

`promotion-undo.html` mounts the real Workbench over a session's durable
sandbox records alone, as a page opened after the person accepted would see
it: nothing in memory, and the session's latest acceptance belonging to
another execution.

```sh
agent-browser --session vak-undo-check open http://localhost:1421/app/tests/promotion-undo.html
agent-browser --session vak-undo-check eval 'window.runChecks()'
```

It checks that each execution offers Undo for its own acceptance with the
same message shown after Accept, that Undo reverses that one, and that the
offer goes once undone. The derivation itself is `tests/candidate-versions.mjs`.

# Chat reading position and input tray

With the web dev server running, open `/app/tests/chat-stability.html`.
The fixture runs automatically; reload to repeat. Add `?theme=dark` for dark
appearance. It uses the real ChatPane, Composer and presentation renderer with
mocked control endpoints, without provider calls or saved conversation data.

The 16 checks cover retaining settled DOM during live frames, scroll position
across run boundaries and conversation switches, following a locally submitted
prompt, disengaging follow after a small upward scroll, one existing Stop
button, and Pause/Resume in the input tray with server readback. It also checks
horizontal overflow. Verified at 1440×900 and 390×844, in light and dark.
These component checks do not update or certify the installed desktop binary.

# AI service settings

Run the guided connect flow with provider and error fixtures (no real key is
written):

```sh
npm run dev:web -- --port 1421
agent-browser --session vak-ai-service open http://localhost:1421/app/tests/ai-service.html
agent-browser --session vak-ai-service eval 'window.runChecks()'
node tests/model-choices.mjs
```

The admin race and provider-neutral model discovery fixture is
`crates/vak-admin-ui/tests/model-settings.html`.

# Library previews and workflows

Open `/app/tests/library-preview.html` in the Vite web dev server and run
`window.runChecks()`. Add `?theme=dark` for dark mode. The fixture mounts the
real Library over synthetic artifact/version routes: HTML, Markdown, CSV,
images, audio/video controls, code, saved pages with linked controls,
Office/PDF viewers, malformed tables and refused reads. It exercises version
selection, comparison, comments and sharing, and checks that no preview reads
a mutable workspace file. `window.openLibraryFixture(id)` opens one example.
Typecheck with `tsc --noEmit -p tsconfig.library-fixture.json`.

The 2026-10-20 browser review also verifies HTML interaction state across
Source and History, and overflow at desktop/phone sizes in light/dark. Audio
and video fixtures certify controls only; Office fixtures certify integration
with the existing renderer, not parsing or full document fidelity. No provider
calls, installed binary changes, or live sharing effects are involved.

## Preview permission and Review containment

Open `tests/preview-permissions.html` and run `window.runChecks()`. It mounts
real `HtmlViewer`, `HtmlPreview`, `Sheet` and `OfficeChangeList` components,
including a Details-like transformed size container that used to clip Review.
Check 1440×900 and 390×844, light and dark (`?theme=dark`). The assertions cover
paused defaults, source classification, explicit isolated execution, stop/reload
revocation, local script bundling, footer status, unclipped Review controls,
vertical scrolling, focus trapping and Escape/focus restoration. The fixture
never accepts or writes a real file. Typecheck with the Library fixture config.

`tests/review-formats.html` mounts the real WorkbenchPanel against saved-version
responses in a confined Details-like container. Run `window.runChecks()` across
the same matrix for HTML, text/code/data, Office/PDF, media/binary fallbacks,
deletions, unavailable versions, long filenames and mixed drafts. It verifies
file/Office selection gates, comments, comparisons and technical-label wrapping,
and refuses every mutation. `window.openReviewFormat(path)` opens an individual
fixture; `window.setReviewTechnicalDetails(true)` exposes technical labels.
