# Library preview and workflow review — 2026-10-20

Status: historical. Checkout acceptance for the shared desktop/web client;
synthetic fixtures, not installed-app or live-provider acceptance.

## Result

Library entries show their saved content instead of HTML source by default.
Content, History and Comments have separate views, a shared version picker,
and the existing action toolbar. File types are named in plain words. HTML
source and page navigation preserve the preview's current interactions.

`tests/library-preview.html` mounts the real Library over fake immutable
version routes. Its 28 assertions cover opaque-origin HTML execution, source
view, old-version selection, comparison, comment submission, sharing, formatted
Markdown, quoted CSV values, malformed CSV/source recovery, image drawing,
audio/video controls, source code, unsupported files, refused reads, saved
page controls and bound values, state across comment navigation, and the
Office/PDF viewer integration. No mutable workspace/session file route is used.
Two further browser assertions click the HTML scenario control and confirm its
state after Source/Preview and History/Content navigation.

All 30 assertions passed at each of 1440×900 and 390×844 in light and dark
(120 total). Each matrix cell also opened HTML, Markdown, CSV, image, saved
page, Word, Excel, PowerPoint, PDF, audio, video, unsupported and code examples;
neither the page nor Library container overflowed horizontally. The four HTML
screenshots were visually reviewed. Local evidence: `/tmp/vak-library-*.png`
and `/tmp/vak-library-check-results.json`.

## Other validation

- App and `tsconfig.library-fixture.json` typechecks passed.
- Both desktop and web production bundles built; the usual large-chunk
  warnings remain.
- Focused Node suites passed: `delimited-preview.mjs`, `preview-isolation.mjs`,
  `canvas-subject.mjs`, `result-card.mjs`.
- Existing result-card browser fixture passed all 30 assertions, and the
  Office-presentation fixture passed its image/chart/table/slide-ratio checks.

## Limits

Office/PDF fixtures exercise viewer integration with a synthetic projection;
they do not certify parsing fidelity. Audio/video tests check controls and blob
sources; the dummy payloads do not certify playback or codecs. Saved-page
linked controls are exercised; routines, sharing grants, uploads and Agent
continuation were not run against a real service. The change reuses those
existing actions. HTML uses only the selected saved file, the existing sandbox
and no-network policy. Linked image/script/stylesheet references show a notice;
there is no fallback to today's workspace resources. Unsupported files retain
Download. Text previews cap at 2 MiB and tables at the existing 200-row preview
limit. No release/install or full Rust gate is claimed.
