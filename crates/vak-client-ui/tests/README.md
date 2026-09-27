# Presentation lifecycle regression

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
