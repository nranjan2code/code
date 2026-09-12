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
mode switches, stable message nodes, scroll position, background task isolation,
and authenticated relative-asset loading. The sandbox check executes a script
inside an opaque-origin iframe and verifies its stylesheet using a test-only
postMessage receipt. `window.showArtifact()` opens the Workbench fixture.

These are deterministic component and transport tests. They do not claim that a
live model created a valid deliverable, that every renderer schema was exercised,
or that Tauri's installed binary has been updated. Production builds do not
include this test entry point.
