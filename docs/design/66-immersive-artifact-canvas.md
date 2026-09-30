# 66 — Immersive Polyglot Artifact Canvas

Status: **partially implemented — the viewer, split/focused modes, device switcher and polyglot formats shipped in 3.0.85; preview isolation (§3.1) is enforced from the K0 change. The dev-server path in the Canvas has no producer yet and the dock `PreviewPane` still exists; `docs/plans/canvas-plan.md` is the remaining work.**

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
- **Automated Dev-Server Lifecycle**: When an artifact requires a backend process, the canvas starts the server (`api.startLaunch`), discovers the active port, binds the preview iframe, and automatically shuts down the server (`api.stopLaunch`) upon dismissal or navigation.

---

## 2. Architecture & Reactive Decoupling

### 0a. The frame, the viewers and the conversation's Canvas

`ArtifactCanvas.tsx` is a frame: header, tab strip, comment area and footer. What each kind of subject can do (its views, whether it can be reloaded, shown at device widths or opened in its own window, where it opens, whether comments are always there) is data in `canvasViewers.ts`, and each kind is drawn by a viewer in `components/canvas/`. Each viewer reads its own content through `createLoader`, which discards a load that has been overtaken and releases what it allocated (a blob URL, a running dev server). A viewer that throws is contained by an error boundary and offers another go. A new subject kind is a registry entry and a viewer.

Each conversation has its own Canvas (`canvasStack.ts`): the subjects opened in it are tabs, reopening one brings it forward and keeps the view, selection and unsent note the reader left in it, and at most eight are kept. Leaving a conversation hides its Canvas and coming back finds it as it was. Documents open on the whole viewport and can be placed beside the conversation; a phone always uses the whole viewport.

A preview opened in a window of its own is a sandboxed frame inside a wrapper with no script (`previewWindowDocument`), never the page itself: a `blob:` page takes the app's origin.

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
| **HTML Prototypes** | `.html`, `.htm`, `.xhtml`, or inline HTML snippet without conflicting file path | Sandboxed `<iframe>` with `srcdoc` | `static` sandbox and no-network CSP (§3.1) |
| **Dev Servers** | `artifact.serverName` or `artifact.serverUrl` | Live `<iframe>` pointing to `http://127.0.0.1:{port}` with auto-lifecycle | `live_server` sandbox (§3.1) |
| **PDF Documents** | `.pdf` extension | Native browser PDF viewer in full-bleed `<iframe>` | Loaded via authenticated raw stream `api.readFileRaw()`, converted to managed blob URL, revoked on cleanup |
| **Images** | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`, `.ico`, `.bmp` | Responsive image viewport with checkerboard transparency grid | Blob URL streaming, containment sizing |
| **Code & Config** | `.rs`, `.ts`, `.tsx`, `.py`, `.json`, `.toml`, `.yaml`, `.md`, `.sh`, etc. | Formatted `<pre><code>` block with one-click clipboard copy | Text content display, no script execution |

### 3.1 Preview isolation

A preview runs someone else's markup, so its isolation is decided by the client and never by the data that describes the preview.

- **Sandbox.** `previewSandbox()` (`safeUrl.ts`) is the only source of an `iframe` `sandbox` value: `static` (`allow-scripts allow-forms`, opaque origin) for documents and saved drafts, `live_server` for a dev server that already has its own loopback origin. `allow-same-origin` on a `srcdoc` frame would give the previewed script the app's origin and its authenticated API, so it is never reachable from a card.
- **Network.** `sandboxedSrcdoc()` always writes `connect-src 'none'`. There is no card field, prop or argument that widens it, so the chrome's "Safe preview, offline" is what the frame enforces. A preview that needs network access is a separate, user-granted setting, not built.
- **CSP placement.** The policy is the first markup after the document's doctype, so no script (before `<head>`, or in a string or comment that mentions `<head>`) runs ahead of it.
- **Server side.** `emit_ui_preview_card` has a closed schema, and `normalize_payload` drops `sandbox` and `connect_src` if a model sends them anyway. The client ignores them regardless, because the fence path does not go through the tool.

Tests: `crates/vak-client-ui/tests/preview-isolation.mjs`, and `ui_preview_never_carries_isolation_settings_from_the_model` in `vak-core`.

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
