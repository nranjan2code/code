# 66 — Immersive Polyglot Artifact Canvas

Status: **implemented and audited — September 14, 2026; slide-over overlay canvas, dual split/focused view modes, responsive viewport device switcher, polyglot format engine (HTML, Dev Server, PDF, Images, Code), automated dev-server lifecycle, and reactive atom decoupling landed in 3.0.85.**

## 1. Product Decision & Thesis

When agents generate deliverables — web applications, prototypes, diagrams, reports, data models, or running dev servers — the traditional interface alternatives fall into two extremes:
1. **Passive link cards / geeky sidebars**: Asking the operator to "click a link and check a new tab", or cramming rich outputs into a narrow fixed right-hand dock panel where neither the conversation nor the application has enough breathing room.
2. **Aggressive hijackers**: Forcibly opening previews on tool completion, disrupting ongoing reading or typing.

### Core Decisions
- **User-Activated Only (Design-61 compliant)**: The canvas never opens automatically on tool execution or file generation. It is summoned explicitly via the prominent "Open Canvas" action on inline preview cards or Workbench items.
- **Dual Display Modes**:
  - **Split View (`68%` screen width, min `480px`)**: The canvas slides in smoothly from the right with spring bezier timing. Crucially, the backdrop has `pointer-events: none`, meaning the chat remains completely interactive and scrollable side-by-side. The operator can converse with the assistant while viewing and interacting with the live artifact.
  - **Focused View (`100%` screen width)**: Takes over the full viewport for distraction-free deep work, testing, and presentation. The backdrop becomes active (`pointer-events: auto`), and clicking outside closes the focused view.
- **Polyglot Content Engine**: Seamlessly presents HTML prototypes, live localhost dev servers, PDFs via authenticated streaming blobs, responsive raster/vector images, and formatted syntax-highlighted code.
- **Automated Dev-Server Lifecycle**: When an artifact requires a backend process, the canvas starts the server (`api.startLaunch`), discovers the active port, binds the preview iframe, and automatically shuts down the server (`api.stopLaunch`) upon dismissal or navigation.

---

## 2. Architecture & Reactive Decoupling

### A. SolidJS State Atoms (`store.ts`)
Earlier iterations coupled layout modes and active artifact payloads inside a single compound signal. Toggling view modes caused the compound object to change identity, triggering unwanted iframe reloads and resetting runtime application state.

The state is cleanly decoupled into discrete atoms:
- `canvasArtifact`: The active `ActiveComponentPreview` payload (or `null` when closed).
- `canvasMode`: `"split" | "focused"` layout signal.
- `canvasDevice`: `"desktop" | "tablet" | "mobile"` responsive viewport simulation.
- `canvasOpen`: Derived memo checking `canvasArtifact() !== null`.

`createEffect` in `ArtifactCanvas` tracks `canvasArtifact()` strictly, ensuring that toggling between split/focused modes or switching between 100%/768px/375px viewports never reloads the underlying iframe.

### B. Asynchronous Generation Guarding
Rapid navigation between artifacts or dev-server startup introduces potential race conditions. Every asynchronous loading pipeline increments a local `request` counter (`const generation = ++request`). State assignments (`setActiveServerPort`, `startedServerName`, `setHtml`, `setRawText`) and DOM mutations are guarded by `if (generation !== request) return;`.

### C. Graceful Exit Animation & Reopening Safety
Closing uses a 220ms CSS slide-out animation via `isClosing()`. If an operator closes one artifact and rapidly selects another within 200ms, the pending dismissal timeout is cleared immediately, resetting `isClosing(false)` and rendering the new artifact without interruption.

---

## 3. Polyglot Format Engine

| Format | Detection Strategy | Presentation Implementation | Security & Sandboxing |
|---|---|---|---|
| **HTML Prototypes** | `.html`, `.htm`, `.xhtml`, or inline HTML snippet without conflicting file path | Sandboxed `<iframe>` with `srcdoc` | Injected strict CSP meta (`sandboxedSrcdoc`), `sandbox="allow-scripts allow-forms"`, origin isolation |
| **Dev Servers** | `artifact.serverName` or `artifact.serverUrl` | Live `<iframe>` pointing to `http://127.0.0.1:{port}` with auto-lifecycle | `sandbox="allow-scripts allow-same-origin allow-forms allow-popups"` |
| **PDF Documents** | `.pdf` extension | Native browser PDF viewer in full-bleed `<iframe>` | Loaded via authenticated raw stream `api.readFileRaw()`, converted to managed blob URL, revoked on cleanup |
| **Images** | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`, `.ico`, `.bmp` | Responsive image viewport with checkerboard transparency grid | Blob URL streaming, containment sizing |
| **Code & Config** | `.rs`, `.ts`, `.tsx`, `.py`, `.json`, `.toml`, `.yaml`, `.md`, `.sh`, etc. | Formatted `<pre><code>` block with one-click clipboard copy | Text content display, no script execution |

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
