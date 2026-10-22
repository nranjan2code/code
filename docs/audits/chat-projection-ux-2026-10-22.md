# Chat and projection UX review — 22 October 2026

Status: historical. Live review and proposed direction, not an implemented
redesign. Inspected the installed local-home conversation, Library and Canvas,
the native app, and the current source's twenty-form gallery. No product code,
permission, stored conversation or installed service was changed.

The later approved implementation and its qualification limits are recorded
in [the follow-through](chat-projection-ux-implementation-2026-10-22.md).

## Finding

One piece of work is visually fragmented into many peer objects. The user has
to reconstruct the relationship between an answer, its files, observations,
checks and next action. Rounded rectangles alone cannot fix that relationship.

The maintainer explicitly clarified that code is only an example. The solution
must apply to any subject and must not infer presentation from domain words,
file extensions, tool names or a special coding mode.

The recommended direction is **a continuous answer with deliberate object
boundaries**. Rich forms remain available. A form is a way to express part of an
answer; it does not inherently deserve a separate enclosing card. An artifact,
an independently usable control, a human decision or an isolated preview may
need a boundary. Headings, alignment and spacing usually suffice for narrative
sections. One request may still have several independent deliverables.

## What was actually inspected

- Installed browser client at loopback port 8901, using the actual `vak-home`
  Agent conversation and existing saved work; no new model turn was needed.
- Conversation with interleaved prose, summary/details/sources, diff, terminal,
  test results, and an answer containing eight separate file cards.
- Request Details, Activity, Library grid, saved meal-planner detail, version
  selector, paused preview and an answer opened in mobile Canvas.
- Native `dev.vak.desktop`: accessibility tree and screenshot of the same
  conversation. The native app used a different selected appearance and showed
  different file-state wording from the browser; this is not a current-build
  native parity qualification.
- Source gallery at `/app/tests/presentation.html`: all twenty registered
  forms. Desktop light (1440 × 900) and phone dark (390 × 844) were visually
  inspected, alongside the live browser's desktop/phone dark views and native
  light appearance. This is not all forms in all four size/theme combinations.
- The gallery's `runChecks()` progressed through the size checks, then failed
  at `Sandboxed iframe executes the relative script and applies its stylesheet`.
  The suite is not passing; no permission was enabled to change that outcome.
  The gallery contains placeholder external media, not qualified real media.

The gallery was exposed and scrolled with temporary developer-only DOM changes
in its own tab. The tab and development server were closed afterward. Browser
viewport overrides were reset. No raw ledgers or credential files were read.

## Prioritized observations and changes

| Priority | Observed | Proposed change | Acceptance evidence |
|---|---|---|---|
| P1 | One answer exposes a test panel, terminal panel, then eight file cards with repeated Open/Download/history actions. | Present related artifact references as a compact collection within that result's existing file area. Keep each artifact's identity, version, state and individual actions. Expand the object the person chooses. | Same membership and actions, substantially less repeated framing; keyboard and phone access to every item; no grouping by directory or extension. |
| P1 | Narrative details, lists and source sections have nearly the same enclosing surface as independently usable objects. Comparison nests option boxes inside another box. | Separate form rendering from enclosing chrome. In composed answers use open narrative sections, subtle row separators and bounded interactive/independent objects. | The same content and semantic headings survive; the composition reads as one answer rather than a stack of unrelated tiles. |
| P1 | The inspected result has pills for Ask for changes / Open in Canvas / Save to Library, a separate copy row, and another request-details row separated by considerable space. | One compact result toolbar with explicit labels for the main action and accessible secondary controls; per-object actions stay local. | No duplicated result-level actions; a person can still copy, save, revise, inspect and open the exact result. Multiple independent results retain separate scopes. |
| P1 | Generic outcome warning and a later Partial result label repeat without naming the missing requirement. | One authoritative visible status summary at the relevant result, with the concrete unresolved requirement when available. Details explains receipts. | Failure, denied action, unverified outcome and stale evidence remain visible. No status is inferred from text, a green test ring or successful file creation. |
| P1 | Needs you shows nine cross-workspace file chips above every composer, mixing draft review and missing files, including other Agents' work. | Keep the global queue reachable in Library/navigation, with counts. Show the current request's actionable items beside its result. Distinguish draft review from file-location notices using the authoritative `needs`. | All existing notices remain discoverable. No client-created urgency, dropped safety state or independent file-state interpretation. |
| P1 | A saved interactive planner opens with empty dynamic sections; Page code paused is separate from the object's promise of automatic behavior. | Give the preview an honest, prominent readiness explanation and a plain-language next step under the existing authority model. | No blank shell presented as a working result; no automatic script/network permission. Runtime admission remains the separate kept-work workstream. |
| P2 | With Details open at 1440px, the result is 531px wide while the dock is 520px; label/value details become narrow prose columns. | Make layout respond to available container width: stack labels above long values, let the answer keep a useful reading measure, offer a deliberate expanded view for dense support. | No narrow value columns or clipped controls at any dock width; semantic order and chosen request remain stable. |
| P2 | On the phone, artifact description, toolbar, content/history/comments, version and preview/source controls push the actual preview well below the top. | Keep object title, real state, version and main action compact; disclose secondary metadata. Preserve deliberate history selection. | Useful content appears earlier; history, comments, permissions and source remain reachable; no automatic latest-version takeover. |
| P2 | Library is an equal-weight grid of file-like and finished objects, including long descriptions and implementation files. | Offer a compact list and a preview-led grid using existing artifact metadata. Use explicit result/collection relationships if a higher-level grouping is needed. | No inferred project grouping, suppressed artifacts or new artifact identity; filtering and versions still work. |
| P2 | Top chrome shows global navigation, a traffic dot, centered character and several workspace controls; the current request is visually weak. | Give agent/request context a clear text anchor. Keep character identity, with less competition from repeated boxes and metadata. | Long request titles wrap/truncate accessibly; traffic and safety details remain available; native drag regions and controls stay intact. |
| P2 | A two-line diff occupies a much taller panel than its content. A one-test report carries a ring, percentage, totals, filters and a row. | Size inspection surfaces to content; use a concise check summary with expandable detail when supporting work is explicitly identified. | Inspection remains available; failures are visible; a requested diff, log or test report remains primary instead of being hidden by form type. |
| P2 | Map renders precise positions on a plain field with straight-line connectors. | State schematic/straight-line meaning visibly; offer permitted geographic context when available, preserving offline operation. | No road-route implication, invented geometry or silent tile/network permission. |

## Every current form: shaping direction

These are form affordances, not topic recipes. Whether a form is primary or
supporting must be explicit in the accepted answer structure; the renderer
must not decide that from the form name.

| Form | Direction |
|---|---|
| summary | Keep the existing open lead treatment; a lead is optional and must add information rather than repeat the following sections. |
| steps | One aligned sequence with readable time/detail; avoid another card around every choice. Preserve actionable choices. |
| list | Open list in narrative, bounded only when it is an independently usable checklist; clear pending/saved/error behavior for ticks. |
| details | Labels beside short facts, above longer prose when width demands it; one section heading. |
| table | Keep real table semantics and numeric fidelity; calm rules, clear sort state and contained horizontal overflow when necessary. Never convert it based on column words. |
| comparison | One comparison surface with aligned aspects; stack options at narrow widths while repeating aspect labels. Avoid nested enclosing rectangles. |
| stats | A shared aligned group of figures with clear units and context; no independent tile required for every value. |
| chart | Let the plot dominate; retain units, exact values and accessible numbers. Phone category truncation needs an accessible full-label interaction. |
| callout | A concise distinct signal at its accepted position; tone uses text/icon as well as color. Avoid duplicating the same warning in other regions. |
| sources | Compact finding-to-source rows. Supporting sources may disclose in place; sources requested as the answer remain expanded. Preserve provenance and freshness. |
| calendar | Aligned day/time structure; identify all-day events, date and timezone. Richer time-grid presentation is separate capability work. |
| map | Geographic or explicitly schematic display, legible pins and list; distinguish straight-line distance from travel distance. |
| controls | Bounded working area with inputs and computed output together; show invalid, stale and busy states without wiping entered values. |
| link | Compact actionable row with title and destination context, rather than another full-height content card. |
| media | Stable intrinsic aspect ratio, visible loading/failure state, accessible alternative and caption; never rely on a blank placeholder. |
| file_preview | One named object with honest readiness, one Open action, optional preview; avoid duplicate preview and artifact cards for the same exact identity. |
| markup_preview | Same visual object discipline as file preview, retaining isolated execution and explicit permissions. |
| diff | Content-sized inspection, exact file/version context and clear change totals; expanded if it is the requested result. |
| test_report | Concise named checks and real outcome, with failure visible; detailed results available without a decorative success dashboard. Passing checks do not establish completion of the user's task. |
| terminal | Readable output with explicit command/exit disclosure; primary when requested, supporting only when that relationship is recorded. |

## General composition contract

1. Preserve the existing turn/result and presentation identities. One coherent
   work unit may have several parts, but a turn may also answer independent
   asks; do not fuse everything in a turn into one artifact or one card.
2. Reuse the existing ordered composition and accepted placement anchors.
   Never move a summary to the top, reorder sources, or pull material across
   interleaved prose to make a preferred layout. A supporting disclosure stays
   at its accepted place. If the desired role/group is not represented, extend
   the typed answer contract first; do not guess it in CSS or client text code.
3. Add explicit primary/support relationships only where existing structure
   cannot express them. The main model authors semantic relationships; the
   runtime validates references and preserves the receipt. No domain list,
   keyword routing, file-path grouping, count threshold or magic card cap.
4. Separate the meaning of a form from its visual enclosure. Use the same
   renderer content in chat and Canvas; a new wrapper must not create a second
   projection or destroy each block's interaction, selection or provenance.
5. Supporting work is still available in place. Required decisions, errors,
   scope limitations and safety state cannot disappear inside a collapsed
   section. A code block, chart or source list can itself be the main answer.
6. Keep persisted identity separate from display grouping. Grouped file rows
   still open exact artifact/version routes and use authoritative `now` and
   `needs`; nothing creates a bundle manifest or second artifact catalog.
7. Live, settled, replayed, Library-saved and Canvas views keep the same order,
   grouping and action targets. User-opened disclosures, focus, unsent edits
   and scroll anchors survive streaming and settlement where identity remains.

This builds on the already-shipped shared author header, result grouping,
borderless summary, exact beside anchors and width-aware row composer. It is
not a proposal to replace those with another layout engine.

## Visual language

Use `DESIGN.md` and the existing Ink and Saffron tokens. Reading text stays
16px, controls 14px, secondary labels 12–13px. Keep prose around 65–75ch;
dense visual material can use a wider container. Base geometry is 7px controls,
12px object boundaries and 16px sheets. Pills indicate a compact state or
filter rather than becoming the default action shape.

Create hierarchy through spacing: related rows close together, sections further
apart, requests clearly separated. Use one aligned leading edge; avoid a chain
of nested padding that consumes phone reading width. Keep surfaces flat at
rest; the composer may float. Indigo is the action color; saffron remains for
live state, not decorative attention. Preserve the approved Songbird assets.

The companion mockups compare continuous reading, an explicitly bounded work
group, and an intentionally opened artifact workspace. They are alternative
visual treatments, not automatic modes selected by topic. Content is synthetic;
buttons only change the local demonstration. The installed UI is unchanged.

## Suggested implementation sequence

| Stage | Scope | Exit condition |
|---|---|---|
| A — shared enclosure | Audit existing result/part ownership; separate enclosure from form content; consolidate result actions and spacing. | Same ordered content and actions in live, settled and Canvas; no form-specific hiding. |
| B — collections | Compact exact artifact references inside their existing result file area, with states and individual actions. | Eight references remain eight real objects but do not require eight large peer cards; standalone objects still work. |
| C — authored relationships | Only if needed, extend typed answer metadata for primary/support roles and named groups. | Validation rejects bad refs; missing metadata uses faithful expanded order; roles survive acceptance/live/replay/save. |
| D — surrounding surfaces | Request context, Details sizing, queue placement, Library density and preview metadata. | Keyboard/phone access, no lost version selection, and no safety or authority regression. |
| E — qualification | Varied work across every state and form. | All four target size/theme combinations in shared web UI, followed by actual native WebKit and native window controls. |

Acceptance must include a short text-only answer, one rich visual, several
independent deliverables, mixed prose and forms, an eight-artifact result,
large tables, long labels, a required decision, partial failure, denied action,
stale sources, script-paused preview, changed/missing files, live replacement,
reconnect, old-version review and unsent Canvas edits. Vary the subject while
keeping equivalent structural cases: the same renderer must handle them.

## Source entry points

- `crates/vak-client-ui/src/components/PresentationRenderer.tsx`: existing
  ResultFiles, ResultActions, ResultEvidence, LiveAnswer and settled composition.
- `crates/vak-client-ui/src/answerComposition.ts`: shared ordered layout.
- `crates/vak-client-ui/src/components/cards/shared.tsx`: form shell.
- `crates/vak-client-ui/src/components/cards/forms.tsx`: narrative/data forms.
- `crates/vak-client-ui/src/components/NeedsYou.tsx`: global queue presentation.
- `crates/vak-client-ui/src/styles.css`: result, form, Library and queue geometry.
- `crates/vak-client-ui/tests/answer-composition.tsx` and
  `crates/vak-client-ui/tests/presentation.tsx`: existing composition/gallery
  fixtures; preserve their semantic checks while adding visual acceptance.

No implementation stage, release or local reinstall was performed in this
review. The known kept-work runtime gaps remain separate open work.
