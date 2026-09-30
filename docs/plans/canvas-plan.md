# Plan — the Canvas as a multi-surface workspace

Status: **K0 and K1 done (2026-09-30); K2–K5 proposed, none started.** Review of the
Canvas at `61393a37` against `66-immersive-artifact-canvas.md`,
`80-mail-and-calendar.md` ("Preview and working area", on
`codex/mail-calendar`), `72-openxml-documents.md` and
`plans/collaboration-and-shared-work-plan.md` (§7, journeys 3 and 7).

## Findings that drive the stages

1. A card could set the preview's `sandbox` and `connect_src`; `allow-same-origin`
   on a `srcdoc` frame exposes the app's origin. **Fixed in K0.**
2. The CSP meta was inserted at the first `<head>` match, after any earlier
   script. **Fixed in K0.**
3. Nothing opens a `live_server` subject, so the Canvas dev-server path is
   unreachable; real dev servers run in the dock `PreviewPane`. (K3)
4. ~~`cleanupServer` read the subject after it had changed.~~ **Fixed in K1:**
   the server is stopped under the identity it was started with.
5. ~~Loose filename matching and a code-block fallback could show content that
   is not the named file.~~ **Fixed in K1.**
6. `127.0.0.1:{port}` is hardcoded; it points at the viewer's machine when the
   browser reaches a headless server.
7. Previews are one `srcdoc`: relative fetches, modules and multi-page sites
   fail, inlining is unbounded, and an inlining failure is silent.
8. The viewer is chosen by file extension and there is one global slot that
   closes on conversation switch; mail, calendar and automation subjects have
   no place in it.
9. Documents are forced full-screen; feedback on a non-draft closes the Canvas.
10. Two preview surfaces exist (Canvas, dock) and `/canvas/preview` is dead.
11. Selection anchors are source line numbers only.

## Stages

| Stage | Ships | Exit test |
| --- | --- | --- |
| K0 — isolation (done) | Client-owned sandbox set, no-network CSP placed first, closed card schema | `tests/preview-isolation.mjs`; `ui_preview_never_carries_isolation_settings_from_the_model` |
| K1 — identity and honesty (done) | `CanvasSubject` (`src/canvasSubject.ts`) carrying full identity; `openArtifactFile` replaces `openArtifactPathInCanvas`; loose matching, the code-block fallback, `serverUrl` and the card-borne `id`/`timestamp` are gone; server shutdown keyed by the subject; a failed asset load is shown as a warning | `tests/canvas-subject.mjs`; `tests/canvas.html` (`window.runChecks()`) |
| K2 — frame and viewer registry | Frame (header, context area, feedback) plus one viewer per subject kind, each declaring modes, source view, selection kinds and cleanup; per-conversation stack; error boundary; split view for documents | Office beside the conversation at 1440 × 900; stacked at 390 × 844; switching conversations keeps the Canvas |
| K3 — preview origin | Isolated origin serving saved drafts and scratch files, live-server proxy, dock `PreviewPane` and `/canvas/preview` removed | A multi-file app and a multi-page site preview on the desktop and from a remote browser |
| K4 — selection and context panel | Selection → Comment / Ask Agent per viewer; Changes, Discussion and Activity panel; new-version notice | Collaboration journeys 3 and 7 |
| K5 — subjects that are not files | Mail thread, calendar and automation viewers on the registry | Doc 80's source → edit → preview → Review → receipt in the running browser |

Open decisions before K3: a second loopback port versus a host-based
sub-origin for the preview origin (recommended: second port), and removing the
dock preview outright (recommended: yes, invariant 30). K4 needs the
maintainer to authorize collaboration stage C0 or later.
