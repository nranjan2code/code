# Focused Review and shared control density — 2026-10-21

Status: **historical** — browser acceptance of the shared client and Admin
control changes; native WebKit acceptance remains open.

## Scope and cause

The maintainer broadened the clipped Review report to every file type, then
requested a focused review experience and smaller, modern controls throughout
the product. Reading-sized inherited labels, inconsistent per-surface geometry
and a narrow-container rule forcing 44px buttons on mouse desktops contributed
to the oversized appearance.

Shared client and Admin now agree on normal 32px, compact 28px and prominent
36px minimum heights and 7px radii. Shared control text is explicitly 14px;
reading content stays 16px. Coarse pointers get larger 40–44px targets and
larger text can grow its control. Secondary actions are quiet at rest, with
visible hover/focus; primary, selected, danger and disabled remain distinct.
Settings, navigation, Topics, Library, file actions, Canvas, Office tabs and
form controls use the shared geometry. Content-rich permission choices keep
their explanatory space.

Review gives one file a centred reading column and multiple files a separate
navigation column that stacks on phones. Open preview stays visible; Focus,
Library and Star are in More. Existing text/code compares once. Focus retains
the document, while addressed comments reveal their composer. Failed checks
and unverified warnings stay visible outside disclosures. The decision footer
remains pinned and explains unread, empty and omitted-edit gates. Library
navigation closes Review and preserves the selected artifact binding.

## Browser verification

Evidence is under `/tmp/vak-preview-review-acceptance/` on the development
machine. Fixtures mount real shared components against synthetic immutable
records; mutation routes throw, so these checks do not accept user changes.

- `controls-system`: 39 assertions in each of 20 scenarios (780 total): five
  rendered palettes, desktop and narrow mouse, narrow touch, and desktop at
  125% text. Checks cover density, explicit typography, narrow companion panes,
  primary/secondary distinction, native fields, type floor, switch/segmented
  state, keyboard focus and horizontal overflow.
- Admin controls: 24 assertions in each of 15 scenarios (360 total), the same
  five palettes at desktop, narrow mouse and narrow touch. Its six Node tests
  also pass, including token, type floor, focus binding and error contracts.
- Shared geometry parity and preview isolation Node checks pass. Both client
  production bundles and the Admin bundle build; application and fixture
  TypeScript checks pass. Existing large-chunk build warnings remain.

Cross-format Review: 235 assertions per desktop/phone, light/dark scenario
(940 total), plus four standalone Office suite passes. Preview permission,
Library and Canvas regression scenarios add 346 assertions. Together the
browser suites pass 2,426 assertions. Review covers HTML, Markdown, text, code,
CSV/JSON, DOCX/XLSX/PPTX, PDF, images/audio/video/archive fallbacks, deletions,
failed reads, long names and mixed drafts. Screenshots include `controls-1440-light.png`,
`admin-controls-1440-light.png` and the format gallery.

## Real local-workspace verification

The rebuilt production web client was served against the installed backend
using the authorised local `vak-home`. The saved market HTML begins paused
and draws its chart after explicit Run page code. The actual pending workbook
Review occupies 1440×900 without horizontal overflow; both decision buttons
are visible in the pinned footer. Real Settings also has no horizontal
overflow and its visible action buttons measure 32px. `live-review.png` and
`live-html-running.png` show those results. No model run, acceptance, promotion
or setting change was triggered.

## Limits

These changes are uncommitted and are not installed into the native desktop
app. Browser acceptance does not certify native WebKit. No full Rust gate or
release is claimed. Synthetic Office/PDF review records exercise the client
flow rather than parsing fidelity. The older connected-mail Review fixture
limitation remains in the earlier preview acceptance record. Arbitrary page
code still has the explicitly disclosed possible WebRTC network access; this
control work does not claim fully offline JavaScript.
