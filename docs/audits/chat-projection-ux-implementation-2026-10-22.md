# Continuous answers and compact work — implementation acceptance

Status: historical. Shared-client source implementation and browser acceptance
on 22 October 2026, following the maintainer's approval of
[the UX review](chat-projection-ux-2026-10-22.md). Both desktop and web bundles
were rebuilt. No release, installed-app update or native qualification is
claimed.

## Result

Composed forms now inherit an open answer surface. Standalone forms retain
their shared shell; files, isolated previews and caution signals keep deliberate
boundaries. Related file references in the existing result file area become
rows inside one collection. A single file retains its thumbnail. Every file
keeps its own identity, status, Open, download, history and review actions.
Names lead the rows; secondary file kinds align alongside on wider containers.
No artifact or version is created by visual grouping.

Copy, revision, Canvas, Save and request Details share one result toolbar when
applicable. Independent result identities retain independent action scopes.
Single-file revision remains on the file. The old title/form-name heuristic
for switching Ask for changes to Adjust plan was removed. Denied, cancelled,
failed and partial outcomes retain one visible typed status with the caution,
rather than repeating a second status footer.

Comparison options and step choices use separators instead of nested boxes.
Long fact values and calendar events stack according to their actual container
width, including beside an open Details pane. Short diffs no longer reserve a
260px empty workspace. Check reports present recorded pass/fail counts without
the decorative success ring or repeated percentage/count summary; filters and
individual outcomes remain available.

Global Needs you retains its visible count and expands into the same individually
actionable file notices. Navigation rows are quieter. The agent switcher has a
text name and participates in header layout rather than floating over controls.
Library defaults to a compact list, with the existing grid available explicitly;
the two views contain the same artifacts. An artifact description is available
under About this file, leaving more room for content. Paused HTML previews
explain that interactive features are paused and point to the existing permission
disclosure. No script, network or storage permission changed.

This remains general-purpose: no topic classification, code mode, file-path
grouping, magic card limit, new stored role or source-order rewrite. Forms are
not hidden because they are logs, tests, sources or any other type. The selected
Continuous answer direction is the conversation surface; deliberately opened
objects continue to use the existing subject-scoped workspace/Canvas.

## Actual local-home inspection

A temporary loopback Vite server on port 1420 served current client source and
proxied API requests to the already-running local gateway at 8901. The gateway
and installed app were not restarted. The source client used the actual
`vak-home` conversation through the normal authenticated API, with no raw ledger
or credential reads and no model request or outbound channel message.

Observed in real content:

- Twelve result groups each had one copy action. The existing eight-file and
  five-file results became two compact collections containing all their files.
- A collection row opened the addressed `market-hours.mjs` in the existing
  file/command workspace, preserving its content and Open action.
- Opening Details left the conversation's long facts 531px wide; labels stacked
  above values instead of forming narrow side-by-side prose columns.
- Library displayed all 42 items, switched between list and grid, and opened the
  saved planner. Phone view showed the compact description disclosure and the
  explicit paused-code explanation. Selecting Version 1 remained deliberate.
- The Needs you queue's count remained visible and it started collapsed.
  The final expanded-queue phone action was refused when local browser access
  was disabled in Work settings. Do not claim that interaction was qualified.

Before/after and fixture screenshots are task-local conversation evidence in
the thread's visualization directory; the final actual-home collection capture
is `ux-files-desktop.png`. They were not copied into a second product catalog.

## Deterministic browser acceptance

Each matrix row below ran at 1440 × 900 and 390 × 844, light and dark.
Fixtures use the actual Solid components with synthetic HTTP responses and
do not mutate the install's artifacts or permissions.

| Fixture / function | Checks per size/theme | What it establishes |
|---|---|---|
| `tests/answer-composition.html` / `runAnswerChecks` | 12 | Interleaved prose order, no duplicate content, shared enclosure, toolbar, responsive pair geometry and Canvas layout reuse. |
| Same / `runLivePlacementChecks` | 3 | Accepted live anchors, exact replacement and no rebinding when an anchor disappears. |
| Same / `runResultStateChecks` | 3 | Partial/denied state remains visible with content; independent result identities keep separate toolbars. |
| `tests/result-card.html` / `runChecks` | 32 | All ten exact files, isolated thumbnails, static expanded previews, lazy loading, individual actions, collection grouping and review deduplication. |
| `tests/presentation.html` / `runChecks` | 26 | Twenty current forms, 260/293/358px fit, saved/live/durable handoff, default paused scripts, explicitly interactive synthetic execution and reader scroll behavior. |
| `tests/library-preview.html` / `runChecks` | 32 | List/grid parity, exact versions, history, comments, comparison, preview permission revocation, media/Office viewers and linked page-control retention. |

The phone dark `tests/chat-stability.html` automatic run passed 22 checks:
run start/settlement and hydration preserve reading position, mounted results,
prompt controls and durable identities without duplicating answers.

Two pre-existing fixture expectations were stale against the shipped static
preview policy: result-card expected `allow-scripts` in a thumbnail and an
interactive inline preview; presentation tried to execute a page returned with
`script-src 'none'`. Expectations now assert the actual paused policy. The
execution receipt is tested separately using explicitly interactive synthetic
markup. No production isolation setting was loosened to make a test pass.

The broader phone dark `tests/failed-answer.html` fixture failed at
`an mcp approval says its server has no internet only when its own setting says
so`. Earlier assertions in that run passed, including failed-answer prose and
approval command visibility. The MCP-network disclosure assertion is unresolved;
no full failure-suite pass is claimed and its security behavior was not changed.

## Source and build checks

- Both production bundles rebuilt with `npm run build`; TypeScript passed.
- Focused Node checks passed: answer composition, result cards, Canvas subject,
  preview isolation/policy/theme/popout, and request Details.
- Documentation records/index, paths, version stamps and whitespace were checked
  for the final change. Blueprint views were regenerated without needs-review
  findings.
- No Rust source, schema, projection wire or backend behavior changed. No full
  workspace Rust gate, commit, release or installation is claimed.

## Remaining qualification

Installed native WebKit, native drag/full-screen controls and the expanded queue
on a phone remain unqualified for this build. Browser access was disabled during
the final real-content check; the last temporary viewport was 390 × 844 and
could not be reset, and its review tab could not be closed through the disabled
browser API. The temporary development server was stopped through its own
terminal session. No attempt was made to bypass the setting.

The kept-work runtime, preview capability admission, Library live metadata
reconciliation, richer presentation capabilities and the unresolved MCP-network
disclosure fixture remain separate work. This change does not infer or persist
primary/support relationships or hide supporting outputs automatically.
