# Plan — the Canvas as a multi-surface workspace

Status: **K0, K1 and K2 done (2026-09-30); K3–K5 proposed, none started.** Review of the
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
8. ~~The viewer was chosen by file extension and there was one global slot that
   closed on conversation switch.~~ **Fixed in K2:** a viewer registry and a
   Canvas per conversation with tabs. Mail, calendar and automation viewers are
   K5.
9. ~~Documents were forced full-screen.~~ **Fixed in K2:** they open on the whole
   viewport and can be put beside the conversation. (Feedback on a non-draft
   still closes the Canvas so its live controls are visible.)
10. Two preview surfaces exist (Canvas, dock) and `/canvas/preview` is dead.
11. Selection anchors are source line numbers only.

## Stages

| Stage | Ships | Exit test |
| --- | --- | --- |
| K0 — isolation (done) | Client-owned sandbox set, no-network CSP placed first, closed card schema | `tests/preview-isolation.mjs`; `ui_preview_never_carries_isolation_settings_from_the_model` |
| K1 — identity and honesty (done) | `CanvasSubject` (`src/canvasSubject.ts`) carrying full identity; `openArtifactFile` replaces `openArtifactPathInCanvas`; loose matching, the code-block fallback, `serverUrl` and the card-borne `id`/`timestamp` are gone; server shutdown keyed by the subject; a failed asset load is shown as a warning | `tests/canvas-subject.mjs`; `tests/canvas.html` (`window.runChecks()`) |
| K2 — frame and viewer registry (done) | Frame (`ArtifactCanvas.tsx`) plus one viewer per kind in `components/canvas/`; what each can do is data in `canvasViewers.ts`; a Canvas per conversation with tabs (`canvasStack.ts`); an error boundary; documents can sit beside the conversation; a preview opened in its own window is a sandboxed frame in a bare wrapper | `tests/canvas-stack.mjs`; `tests/canvas.html` (21 checks at 1440 px); a phone width opens full width with no toggle and no sideways scroll |
| K3 — preview origin | Isolated origin serving saved drafts and scratch files, live-server proxy, dock `PreviewPane` and `/canvas/preview` removed | A multi-file app and a multi-page site preview on the desktop and from a remote browser |
| K4 — selection and context panel | Selection → Comment / Ask Agent per viewer; Changes, Discussion and Activity panel; new-version notice | Collaboration journeys 3 and 7 |
| K5 — subjects that are not files | Mail thread, calendar and automation viewers on the registry | Doc 80's source → edit → preview → Review → receipt in the running browser |

Open decisions before K3: a second loopback port versus a host-based
sub-origin for the preview origin (recommended: second port), and removing the
dock preview outright (recommended: yes, invariant 30). K4 needs the
maintainer to authorize collaboration stage C0 or later.
