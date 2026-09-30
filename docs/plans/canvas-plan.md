# Plan — the Canvas as a multi-surface workspace

Status: **K0–K4 done (2026-09-30), K3 for a Canvas on this computer; K5 proposed, not started.** Review of the
Canvas at `61393a37` against `66-immersive-artifact-canvas.md`,
`80-mail-and-calendar.md` ("Preview and working area", on
`codex/mail-calendar`), `72-openxml-documents.md` and
`plans/collaboration-and-shared-work-plan.md` (§7, journeys 3 and 7).

## Findings that drive the stages

1. A card could set the preview's `sandbox` and `connect_src`; `allow-same-origin`
   on a `srcdoc` frame exposes the app's origin. **Fixed in K0.**
2. The CSP meta was inserted at the first `<head>` match, after any earlier
   script. **Fixed in K0.**
3. ~~Nothing opened a `live_server` subject.~~ **Fixed in K3:** the dock's Live
   preview list shows one in the Canvas.
4. ~~`cleanupServer` read the subject after it had changed.~~ **Fixed in K1:**
   the server is stopped under the identity it was started with.
5. ~~Loose filename matching and a code-block fallback could show content that
   is not the named file.~~ **Fixed in K1.**
6. ~~`127.0.0.1:{port}` was hardcoded.~~ **Reported honestly in K3:** a dev
   server is framed under the other loopback name, and a browser on another
   machine is told a live preview needs the app on this computer.
7. ~~Previews were one `srcdoc`.~~ **Fixed in K3 for a Canvas on this
   computer:** pages with files are served from a preview origin. (Inlining
   failure is shown as a warning since K1.)
8. ~~The viewer was chosen by file extension and there was one global slot that
   closed on conversation switch.~~ **Fixed in K2:** a viewer registry and a
   Canvas per conversation with tabs. Mail, calendar and automation viewers are
   K5.
9. ~~Documents were forced full-screen.~~ **Fixed in K2:** they open on the whole
   viewport and can be put beside the conversation. (Feedback on a non-draft
   still closes the Canvas so its live controls are visible.)
10. ~~Two preview surfaces (Canvas, dock) and a dead `/canvas/preview`.~~
    **Fixed in K3:** the dock keeps only the server list; `/canvas/preview` and
    `/fs/preview` are gone.
11. ~~Selection anchors were source line numbers only.~~ **Fixed in K4:** lines or a
    document place, per view.

## Stages

| Stage | Ships | Exit test |
| --- | --- | --- |
| K0 — isolation (done) | Client-owned sandbox set, no-network CSP placed first, closed card schema | `tests/preview-isolation.mjs`; `ui_preview_never_carries_isolation_settings_from_the_model` |
| K1 — identity and honesty (done) | `CanvasSubject` (`src/canvasSubject.ts`) carrying full identity; `openArtifactFile` replaces `openArtifactPathInCanvas`; loose matching, the code-block fallback, `serverUrl` and the card-borne `id`/`timestamp` are gone; server shutdown keyed by the subject; a failed asset load is shown as a warning | `tests/canvas-subject.mjs`; `tests/canvas.html` (`window.runChecks()`) |
| K2 — frame and viewer registry (done) | Frame (`ArtifactCanvas.tsx`) plus one viewer per kind in `components/canvas/`; what each can do is data in `canvasViewers.ts`; a Canvas per conversation with tabs (`canvasStack.ts`); an error boundary; documents can sit beside the conversation; a preview opened in its own window is a sandboxed frame in a bare wrapper | `tests/canvas-stack.mjs`; `tests/canvas.html` (21 checks at 1440 px); a phone width opens full width with no toggle and no sideways scroll |
| K3 — preview origin (done, loopback) | `POST /previews` opens an origin of its own on a loopback port for a saved version, a run or a workspace page (`preview.rs`); dev servers are framed directly under the other loopback name; the dock `PreviewPane`, `/canvas/preview` and `/fs/preview` are removed and the dock keeps a Live preview list | `preview::tests` (scope, traversal, dotfiles, token, Host, method, eviction, handler); `tests/canvas.html` origin checks; run against a real server: a module script, relative CSS and `fetch` load, `parent.document` and the app's API and cookie are unreachable from a real cross-origin frame, and a dev server starts, frames and stops with the Canvas |
| K3b — previews for a remote browser (not built) | A distinct preview host name and fixed port range (or a proxy with a wildcard preview domain) so a browser on another machine can reach a preview origin and a dev server | A multi-file app and a dev server preview from a browser on another machine |
| K4 — selection and context panel (done) | `canvasSelection.ts`, per-view `selects`; `CanvasContext` (Discussion with Comment / Ask Agent, Activity, Changes); `NewVersionNotice`; `draftVersions.ts` | `tests/canvas-selection.mjs`; 38 checks in `tests/canvas.html`; against a real server and model: a real page opens from a real result in the real Canvas, lines are picked, and Ask for revision carries the place to the real Agent, which edits the file. Comment/Activity/Changes/new-version on a saved draft ran only against the stub: no saved draft could be produced in this setup |
| K5 — subjects that are not files | Mail thread, calendar and automation viewers on the registry | Doc 80's source → edit → preview → Review → receipt in the running browser |

K3 took a loopback port per preview and removed the dock preview outright. K3b
needs a design decision on how a remote deployment names and reaches the
preview origin. K4 needs the maintainer to authorize collaboration stage C0 or
later.
