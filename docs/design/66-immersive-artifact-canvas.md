# 66 — Immersive Polyglot Artifact Canvas

Status: **implemented for a Canvas on this computer — the viewer, split/focused modes, device switcher and polyglot formats shipped in 3.0.85; preview isolation (§3.1), the frame and per-conversation Canvas (§0a), subject identity (§0) and preview origins (§3.2) landed after it. Previewing several files or a dev server from a browser on another machine is not built; `docs/plans/canvas-plan.md` has the remaining work.**

## 1. Product Decision & Thesis

When agents generate deliverables — web applications, prototypes, diagrams, reports, data models, or running dev servers — the traditional interface alternatives fall into two extremes:
1. **Passive link cards / geeky sidebars**: Asking the operator to "click a link and check a new tab", or cramming rich outputs into a narrow fixed right-hand dock panel where neither the conversation nor the application has enough breathing room.
2. **Aggressive hijackers**: Forcibly opening previews on tool completion, disrupting ongoing reading or typing.

### Core Decisions
- **User-Activated Only (Design-61 compliant)**: The canvas never opens automatically on tool execution or file generation. It is summoned explicitly via the prominent "Open Canvas" action on inline preview cards or Workbench items.
- **Dual Display Modes**:
  - **Split View (`65%` screen width, min `440px`)**: The canvas slides in smoothly from the right with spring bezier timing. Crucially, the backdrop has `pointer-events: none`, meaning the chat remains completely interactive and scrollable side-by-side. The operator can converse with the assistant while viewing and interacting with the live artifact.
  - **Focused View (`100%` screen width)**: Takes over the full viewport for distraction-free deep work, testing, and presentation. The backdrop becomes active (`pointer-events: auto`), and clicking outside closes the focused view.
- **Polyglot Content Engine**: Seamlessly presents HTML prototypes, live localhost dev servers, PDFs via authenticated streaming blobs, responsive raster/vector images, and formatted syntax-highlighted code.
- **Shared Dev-Server Lifecycle**: A view starts a dev server, or joins it when it is running, and holds a lease it renews (`/launch/lease`). Letting go (`/launch/release`) leaves it running briefly so moving between tabs, conversations or surfaces never restarts it; the server stops it after the last view lets go or stops renewing. One started from the Live preview list runs until it is stopped there.

---

## 2. Architecture & Reactive Decoupling

### 0a. The frame, the viewers and the conversation's Canvas

`ArtifactCanvas.tsx` is a frame: header, tab strip, comment area and footer. What each kind of subject can do (its views, whether it can be reloaded, shown at device widths or opened in its own window, where it opens, whether comments are always there) is data in `canvasViewers.ts`, and each kind is drawn by a viewer in `components/canvas/`. Each viewer reads its own content through `createLoader`, which discards a load that has been overtaken and releases what it allocated (a blob URL, a running dev server). A viewer that throws is contained by an error boundary and offers another go. A new subject kind is a registry entry and a viewer.

Each conversation has its own Canvas (`canvasStack.ts`): the subjects opened in it are tabs, reopening one brings it forward and keeps the view, selection and unsent note the reader left in it (the same subject is not read again), and at most eight are kept. Leaving a conversation hides its Canvas and coming back finds it as it was. Documents open on the whole viewport and can be placed beside the conversation; a phone always uses the whole viewport.

The Canvas is the same on every surface showing the conversation: the desktop app and the web app at once, or one after the other. The server keeps it with the conversation's Agent (`agents/<agent>/canvas/<session>.json`, `vak-server/src/canvas.rs`) at a revision; a change is an operation (`CanvasOp`) applied at once and written from the revision it was made on, and when another surface wrote first the operations are applied again on the newer Canvas (`canvasSync.ts`). A write is announced on the stream as a `canvas` hint; reading is level-triggered (opening a conversation, the hint, the window coming back). Only the layout (beside or whole window) is the device's own. Escape belongs to the app's one chain (menus, dialogs, Settings, inbox, side panel, Canvas, then stopping a run); it never closes the Canvas from a text field inside it and never stops a run while the Canvas is open.

A preview opened in a window of its own is a sandboxed frame inside a wrapper with no script (`previewWindowDocument`), never the page itself: a `blob:` page takes the app's origin.

### 0b. Pointing, discussion and new versions

A viewer says what a reader can point at in each of its views (`ViewerSpec.selects`): source lines, or a place in a document (the reader's own address, shown only under Show technical details). A preview page or an image has no place a comment could name, so nothing is selected there and a comment is about the whole file. Pointing opens the discussion; the discussion (`CanvasContext`) offers **Comment** (saved on the version being read, asks nobody) and **Ask Agent** (saves it and asks for a revision, or for a file that is not a saved draft, sends a request that names the place). A comment's place can be shown again with Show where. Activity lists only what the sandbox records and comments show (versions saved, the Agent revising, comments), and Changes lists the files this version adds, changes or removes.

A newer version of the draft being read is announced (`NewVersionNotice`) and never replaces it: reading it opens another tab, comments stay on the version they were written on, and dismissing the notice keeps it away until a still newer one arrives. Not built: a pointing mode for interactive pages, regions of an image, and handing work over to another person or Agent (collaboration stage C3).

### 0c. Subjects that are not files

A subject need not be a file: a scheduled routine is `{ kind: "automation", taskId }` (`AutomationViewer`). It has no path, conversation or comments, so its registry entry says `feedback: "none"` and `selects: () => null`, and the frame draws no comment area for it. The viewer reads the task list, keeps itself current every ten seconds, and offers what the task list offers for one routine (run now, pause or resume, retry a waiting delivery, see the last run); the words for cadence, status and delivery are shared with the task list (`taskWords.ts`). The task list's "open" opens one. A routine that has been deleted says so. A new non-file subject kind (a mail thread, a calendar view) is the same three steps: a `CanvasSubject` variant, an entry in `canvasViewers.ts`, and a viewer in `components/canvas/`.

The read-only daily mail/calendar view is `{ kind: "daily_mail_calendar" }` (`DailyMailCalendarViewer`). A **Today** action stays available in each Agent conversation. The viewer reads only connected accounts with available credentials using the owner-authenticated mail/calendar preview APIs; it does not open an Agent read tool or send provider content to the model. It presents the local day's calendar events and free/busy intervals, recent inbox messages, per-source refresh failures, and links supported messages to the existing account conversation preview. It loads at most two accounts concurrently, bounding provider request bursts while still aggregating all linked accounts. Data loads on open and explicit refresh. Like other non-file views it has no comments or file path.

### 0. What the Canvas shows: a subject

The Canvas opens a `CanvasSubject` (`src/canvasSubject.ts`), which names what is shown by identity, not by path: a workspace `file`, an `execution_artifact` (a run's scratch file), a `draft_file` (one saved version, read through hash-checked candidate routes), `inline` markup from the conversation, or a `live_server`. Each kind is read through the one route its identity names, and there is no fallback between them: an unreadable file is an error, never replaced by text found elsewhere. `openArtifactFile(path, origin?)` is the entry point for files. A bare reference (a path in the Agent's text) is matched to a run only by an identical path made by exactly one run; if several runs made it, the reader is asked instead of a file being chosen.

### A. SolidJS State Atoms (`store.ts`)
Earlier iterations coupled layout modes and active artifact payloads inside a single compound signal. Toggling view modes caused the compound object to change identity, triggering unwanted iframe reloads and resetting runtime application state.

The state is cleanly decoupled into discrete atoms:
- `canvasSubject`: The active `CanvasSubject` (or `null` when closed).
- `canvasMode`: `"split" | "focused"` layout signal.
- `canvasDevice`: `"desktop" | "tablet" | "mobile"` responsive viewport simulation.
- `canvasOpen`: Derived memo checking `canvasSubject() !== null`.

`createEffect` in `ArtifactCanvas` tracks `canvasSubject()` strictly, ensuring that toggling between split/focused modes or switching between 100%/768px/375px viewports never reloads the underlying iframe.

### B. Asynchronous Generation Guarding
Rapid navigation between artifacts or dev-server startup introduces potential race conditions. Every asynchronous loading pipeline increments a local `request` counter (`const generation = ++request`). State assignments (`setActiveServerPort`, `startedServerName`, `setHtml`, `setRawText`) and DOM mutations are guarded by `if (generation !== request) return;`.

### C. Graceful Exit Animation & Reopening Safety
Closing uses a 220ms CSS slide-out animation via `isClosing()`. If an operator closes one artifact and rapidly selects another within 200ms, the pending dismissal timeout is cleared immediately, resetting `isClosing(false)` and rendering the new artifact without interruption.

---

## 3. Polyglot Format Engine

| Format | Detection Strategy | Presentation Implementation | Security & Sandboxing |
|---|---|---|---|
| **HTML Prototypes** | `.html`, `.htm`, `.xhtml`, or inline HTML snippet without conflicting file path | A preview origin (§3.2) when files sit behind the page, otherwise a sandboxed `<iframe>` with `srcdoc` | `origin` sandbox on a preview origin; `static` sandbox and no-network CSP for `srcdoc` (§3.1) |
| **Dev Servers** | a `live_server` subject, shown from the Live preview list in the dock | Live `<iframe>` on the server's own port, framed under the other loopback name than the app's, started and stopped by the viewer | `origin` sandbox (§3.1) |
| **PDF Documents** | `.pdf` extension | Native browser PDF viewer in full-bleed `<iframe>` | Loaded via authenticated raw stream `api.readFileRaw()`, converted to managed blob URL, revoked on cleanup |
| **Images** | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`, `.ico`, `.bmp` | Responsive image viewport with checkerboard transparency grid | Blob URL streaming, containment sizing |
| **Code & Config** | `.rs`, `.ts`, `.tsx`, `.py`, `.json`, `.toml`, `.yaml`, `.md`, `.sh`, etc. | Formatted `<pre><code>` block with one-click clipboard copy | Text content display, no script execution |

### 3.1 Preview isolation

A preview runs someone else's markup, so its isolation is decided by the client and never by the data that describes the preview.

- **Sandbox.** `previewSandbox()` (`safeUrl.ts`) is the only source of an `iframe` `sandbox` value: `static` (`allow-scripts allow-forms`, opaque origin) for markup from the conversation and for a page shown as one document, `origin` for a page served from an origin of its own (§3.2), which is not the app's origin and so may keep its own storage and open windows. `allow-same-origin` on a `srcdoc` frame would give the previewed script the app's origin and its authenticated API, so it is never reachable from a card.
- **Navigation.** A frame's own policy cannot stop it navigating itself away (a script, a link or a refresh), so the page that frames it does: the web app answers with `frame-src 'self' blob: data: http://127.0.0.1:* http://localhost:*` (`embedded_ui.rs`, `the_app_page_confines_frames_to_the_app_and_loopback`) and the desktop shell sets the same in `tauri.conf.json`. A preview cannot be sent to an internet address, whatever it runs. A preview on an origin of its own is still allowed to open a window (`allow-popups`).
- **Network.** `sandboxedSrcdoc()` always writes `connect-src 'none'`. There is no card field, prop or argument that widens it, so the chrome's "Safe preview, offline" is what the frame enforces. A preview that needs network access is a separate, user-granted setting, not built.
- **CSP placement.** The policy is the first markup after the document's doctype, so no script (before `<head>`, or in a string or comment that mentions `<head>`) runs ahead of it.
- **Server side.** `emit_ui_preview_card` has a closed schema, and `normalize_payload` drops `sandbox` and `connect_src` if a model sends them anyway. The client ignores them regardless, because the fence path does not go through the tool.

Tests: `crates/vak-client-ui/tests/preview-isolation.mjs`, and `ui_preview_never_carries_isolation_settings_from_the_model` in `vak-core`.

### 3.2 Preview origins

A page with files behind it (a saved draft's site, a run's output, a page in the workspace) is not poured into a `srcdoc` frame, where relative stylesheets, scripts, modules, `fetch` and links cannot work. `POST /previews` (`crates/vak-server/src/preview.rs`) opens a preview of one file under a scope (`candidate`, `execution` or `workspace`) and answers with the URL to frame.

- **An origin of its own.** Each preview listens on a fresh loopback port, so it is a browser origin of its own, and it is framed under the *other* loopback name than the one the app is reached by (`127.0.0.1` and `localhost` are two sites). It shares no cookie, storage or `document.parent` access with the app, and the app's routes, credentials and cookies never reach it.
- **Scope.** The listener reads only through the same functions that serve the file to the owner: a saved version's manifest files (hash-checked), a run's scratch directory, or the conversation's own workspace (its Agent's folder) below the opened page's directory, exactly, with dotfiles refused and nothing looked for elsewhere. `..`, backslashes and NUL are refused before any read. The path starts with an unguessable token; a wrong or missing one is a plain 404.
- **Policy.** The page may load its own files and run its own script and may reach no other origin (`connect-src 'self'`, no external images, fonts or frames); only the app's loopback origins and the desktop shell may embed it. `GET` and `HEAD` only, and a request whose `Host` is not a loopback name is refused (DNS rebinding).
- **Lifetime.** The viewer closes the preview (`DELETE /previews/{id}`) when it lets go; at most 16 are open and the oldest goes first; none outlives twelve hours.
- **Loopback only.** A loopback port cannot be reached from a browser on another machine. A request that did not arrive by a loopback name is answered `409 not_local`, and the page is shown as one document as before; a dev server reports that a live preview needs the app on this computer. Serving previews to a remote browser needs a distinct preview host name and a fixed port range; it is not built.
- **Limits.** Links written as absolute paths (`/style.css`) resolve against the preview origin's root, not the page's directory, and are not found; relative links work. A dev server is framed directly on its own port; it is not proxied, so its hot-reload socket works as it does in a browser tab.

The Live preview list in the dock (`LivePreviewPanel.tsx`) is the one place dev servers are started, stopped and read; "show" opens one in the Canvas. The dock's own preview pane, its URL bar and `/fs/preview` were removed (invariant 30).

---

## 4. Visual Aesthetics & Ergonomics

- **Spring-Based Transitions**: Custom cubic-bezier curve (`cubic-bezier(0.22, 1.0, 0.36, 1.0)`) for physical slide-in and slide-out.
- **Split-Screen Frosted Glow**: Subtle linear accent wash along the left border indicating canvas separation.
- **Responsive Viewport Controls**: Quick switcher in the canvas header for HTML and dev-server previews:
  - `100%` (Desktop)
  - `768px` (Tablet, with subtle device curvature and shadow)
  - `375px` (Mobile, with mobile device frame styling)
- **Theme Awareness**: Dynamic canvas background styling conforming to Dark Obsidian, Quiet Sage, Soft Paper, Mist, Dawn, and High Contrast palettes.
- **Keyboard Navigation**: Pressing `Escape` closes the canvas.
