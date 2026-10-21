# HTML preview permissions and Review layout — 2026-10-21

Status: historical. Development acceptance of the shared client; browser tests
use synthetic content and local loopback sinks. Native WebKit acceptance and
installation are not claimed.

## Changes

The maintainer reported generated HTML with an empty chart and a blanket
scripts-blocked notice, plus Review extending off the right of the window.

`HtmlPreview.tsx` is shared by Canvas and Library. It shows paused/running
permission status and separates embedded scripts, inline actions, local script
files and external references. Locations do not imply authorship or trust.
Scripts start paused. A person may run embedded code and, in Canvas, relative
assets bundled through the selected subject reader. External script downloads,
ordinary fetch, frames and form submissions remain blocked. An opaque frame
keeps the app's origin and data inaccessible. WebRTC can still connect, so the
permission disclosure and running footer explicitly say network access is
possible. Permission is transient; stop, reload, changing documents and closing
the viewer revoke it, including a pending preparation. Source/Preview navigation
preserves the current document. Separate windows and thumbnails remain paused.
Library uses only saved version bytes, with local linked files marked unavailable.

Review's shared `Sheet` mounts through a Portal outside the Details panel's CSS
size container and transformed/clipping ancestors. The full-window sheet's close
and decision controls stay visible, its body scrolls vertically, and its text
wraps at phone width. Its focus trap, Escape handling and focus restoration are
retained. No acceptance/promotion backend changed.

## Verification

At each of 1440×900 and 390×844, light and dark:

- New `preview-permissions.html`: 19 checks per cell (76), including real
  `HtmlViewer`, `HtmlPreview`, `Sheet` and `OfficeChangeList` components,
  paused execution, code classification, isolated execution, effective footer,
  stop/reload/library-change revocation, selected-reader local scripts, source navigation,
  Review geometry/scrolling and focus/keyboard behavior.
- Existing Library fixture: 29 checks per cell (116). Its stale pre-hardening
  automatic-execution assertions now require an explicit Run; selecting an
  older version revokes permission. Other file previews/actions still pass.
- Existing Canvas fixture: 39 desktop and 38 phone checks (154). The static
  sandbox assertion reflects the paused default; its split-view assertion now
  accounts for the existing narrow-screen focused-only behavior.

The real `WorkbenchPanel` fixture `review-formats.html` adds 164 assertions per
cell (656): HTML, Markdown, plain text, JavaScript, CSV, JSON, Word, Excel,
PowerPoint, PDF, image/audio/video/archive fallbacks, deletion, refused reads,
long filenames and a seven-file mixed draft. It checks standard and focused
review, semantic changes, technical anchors, before/after comparison, reachable
comments, omission/acceptance gating, selected-file scope and pinned controls.
Every fixture mutation is refused; no real acceptance is exercised. It found
and fixed long technical-anchor and nested file-list grid overflow. Binary
reads now explain the saved-version inspect/download route instead of staying
on a loading placeholder.

1,002 browser assertions passed, plus the standalone Office-review suite at all
four matrix cells. Review and preview screenshots were inspected.
Evidence: `/tmp/vak-preview-review-acceptance/`, including `results.json` and
`review-*.png` / `preview-*.png`; `permission-results.json`, `format-results.json` and `format-*.png`
cover the full Review matrix.

A real Chrome network probe uses a loopback HTTP sink and UDP STUN sink:

| Mode | HTTP requests | STUN packets | Code result |
|---|---:|---:|---|
| Default static | 0 | 0 | No document code ran |
| Explicit interactive | 0 | 5 | Embedded code ran; fetch rejected; parent access refused |

The HTTP sink covered external script, image and fetch attempts. This verifies
blocked external downloads without making an offline claim about interactive
mode. The probe does not certify every browser/network API. `network.json` owns
the local results; the prior security acceptance's static policies remain intact.

App and Library/preview-fixture typechecks passed. Both desktop and web client
production bundles rebuilt successfully (existing large-chunk warnings remain).
Focused Node suites passed: preview isolation, canvas subject, delimited preview
and result card. Version, record and doc-path checks are recorded in the handover.

## Real local-workspace check

Using the authorised `/Users/nisheethranjan/vak-home`, an installed backend on
8932 served the real records to the rebuilt production web client through a
temporary loopback frontend on 8933. The saved HTML from the maintainer's
screenshot starts paused and draws its chart after Run page code. The actual
pending workbook Review fills 1440×900, has no horizontal body overflow, and
keeps both footer decisions visible inside the window. Screenshots:
`live-html-running.png` and `live-review.png`. No model run was started and no
pending changes were accepted. This checks the rebuilt web client with the real
backend, not the installed native WebKit frontend.

## Limits

The standalone Office fixture had two stale assumptions: a deleted span expected
a period absent from its input, and a technical anchor was asserted while the
technical-details setting was off. Those fixture assumptions are corrected;
all four standalone runs now pass. An exploratory older connected-mail Review
fixture could not reach its fake account after Settings navigation changed;
that workflow is not certified by this file-review acceptance. The synthetic
Office/PDF review data exercise the client flow and do not certify parsing
fidelity.

No full Rust gate, native desktop installation or release is claimed. Fully
offline arbitrary JavaScript still needs an enforceable browser network boundary.
Module dependency graphs are not bundled; external libraries are not fetched.
