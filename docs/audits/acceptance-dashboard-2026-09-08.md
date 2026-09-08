# Dashboard acceptance audit — 2026-09-08

Status: verified in an isolated disposable workspace.

## Scope

The acceptance run used `/tmp/vak-acceptance-dashboard.aXLoI4`, separate from
the vak source checkout. A real `vak exec` run generated a self-contained HTML
dashboard, a static server served it, and the in-app browser loaded the page.
The source workspace was not used for generated dashboard files.

## Verified behavior

- The dashboard rendered in a browser with KPI cards, a canvas chart, date
  controls, a chart-type selector, and an accessible data table.
- The dashboard was served from the isolated workspace on a local HTTP port.
- The final artifact was read back from the exact absolute path after the write.
- The JavaScript was extracted and passed `node --check`.
- The generated dashboard was corrected after browser-oriented review exposed a
  table sorting bug; sorting now reads body-cell positions rather than looking
  for header cells inside body rows.

## Findings and contract changes

The first run declared `--write-path .`. The write tool rejected
`dashboard.html` as outside the declared scope, but the model then claimed the
file existed. The workspace remained unchanged. This is a model/output quality
failure, not a successful artifact.

The reliable workflow is:

1. Create a disposable task workspace outside the source checkout.
2. Declare explicit output paths, such as `--write-path dashboard.html`.
3. Require a read-back of the exact output path.
4. Start a real local server from that workspace when browser behavior matters.
5. Inspect the browser DOM and interactions; syntax checks alone are
   insufficient for generated UI.
6. Promote or copy an accepted artifact only after the user reviews the actual
   candidate.

The harness must preserve tool errors and filesystem evidence even when the
model's prose says that work succeeded. A failed write, failed read-back, or
failed server start cannot be summarized as a successful deliverable.

## Residual limitations

Generated dashboards are arbitrary user/model code. The harness can safely
quarantine, stream, preview, and review them, but it cannot prove visual or
interaction quality from a successful write alone. Browser verification remains
necessary for rich UI, and native PDF accessibility depends on the browser's
PDF implementation.
