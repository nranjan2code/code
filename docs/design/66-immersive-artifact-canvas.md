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

Each conversation has its own Canvas (`canvasStack.ts`): the subjects opened in it are tabs, reopening one brings it forward and keeps the view, selection and unsent note the reader left in it (the same subject is not read again), and at most eight are kept. Automatic eviction skips every tab with an
unsent note. If all eight have notes, opening another subject leaves the
Canvas unchanged and explains that a note must be finished first; reopening
an existing subject remains allowed. Only an explicit close can remove a
tab that still has a note. This protects tab visibility changes, not remote
sync failures or reload durability; those remain open under hardening P06. Leaving a conversation hides its Canvas and coming back finds it as it was. Documents open on the whole viewport and can be placed beside the conversation; a phone always uses the whole viewport.

The Canvas is the same on every surface showing the conversation: the desktop app and the web app at once, or one after the other. The server keeps it as an Agent-owned Document named `agents/<agent>/canvas/<session>.json` (`vak-server/src/canvas.rs`), with the entire Canvas content encrypted under the conversation's scope key inside each Document version. This name is a Document identity, not a plaintext file. Document CAS and the held writer epoch enforce the revision; a change is an operation (`CanvasOp`) applied at once and written from the revision it was made on, and when another surface wrote first the operations are applied again on the newer Canvas (`canvasSync.ts`). A write is announced on the stream as a `canvas` hint; reading is level-triggered (opening a conversation, the hint, the window coming back). Only the layout (beside or whole window) is the device's own. Escape belongs to the app's one chain (menus, dialogs, Settings, inbox, side panel, Canvas, then stopping a run); it never closes the Canvas from a text field inside it and never stops a run while the Canvas is open.

A preview opened in a window of its own is a sandboxed frame inside a wrapper with no script (`previewWindowDocument`), never the page itself: a `blob:` page takes the app's origin.

Canvas save failures retain the pending operations in memory until a write
acknowledges them. The frame shows saving or a recoverable failure and a
Retry saving action. Authentication, missing conversations, invalid/newer
formats and oversized writes pause automatic retries; network, server and
throttling failures retry after five seconds. Focus, returning to a visible
window, a new local edit or explicit retry can retry paused work; stream
hints only read and cannot overwrite pending work. An in-flight write only
acknowledges the prefix it sent. A delayed read cannot lower the observed
revision. While any conversation has pending work, the client requests the
standard before-unload warning. Supported browsers also keep encrypted
checkpoints for reload in the same tab, as described below. Stale writes replay only when local pending notes can be preserved. A
remotely closed tab with a pending local note is restored through the same
eight-tab rules, which may evict only clean tabs. Competing edits to the
same note (including a remote clear), or insufficient safe room, leave the
entire local Canvas unchanged and pause saving with a conflict notice and
Copy unsent notes. The base revision advances only after a safe rebase;
retry cannot bypass the conflict by writing directly at the remote head.
Unrelated remote notes and ordinary reads continue to update normally.
For competing note edits, the notice previews both exact versions and offers
Keep my changes or Use saved copy for each note, then Save these choices.
Choices are scoped to a preview token and exact local snapshot; a later edit
invalidates the preview. The resolved snapshot is checkpointed before CAS
advances. A new server conflict pauses again; acknowledgment retains edits
made while a choice was saving. Structural/capacity conflicts retain the
Copy unsent notes and retry path. Note previews wrap at narrow widths and
stay outside live screen-reader announcements. Browser acceptance remains
open; source queue/rebase/recovery regressions are not a visual claim. Saved state uses encrypted, fenced Documents; pending
client operations are checkpointed locally when recovery is available. The status never claims that
a failed or conflicted write was saved.

Unacknowledged operations use `canvasRecovery.ts`: IndexedDB contains an
AES-GCM envelope with fresh random IV and authenticated origin, conversation,
writer and immutable server identity. The encrypted checkpoint holds the
actual CAS base/baseline, pending operations and local snapshot. Neither note
text nor keys are stored in browser local/session storage; session storage
holds only a full UUIDv7 writer identifier. An origin Web Lock reserves that
writer, so a duplicate live tab receives another identifier. A reload uses
the original identifier after the previous document releases its lock.

The owner-only `POST /sessions/{id}/canvas/recovery-key` returns a separate
random key and immutable UUIDv7 identity with `Cache-Control: no-store`.
The conversation storage key is never exported. Recovery material is an
Agent-owned, conversation-sealed Document, named through
`AgentScope::canvas_recovery_key`, registered at
`agents/{agent}/canvas-recovery`. Its normal Document CAS, writer fence,
retention and conversation-key erasure apply. Blocking storage work runs in
a blocking task. Participant authentication passes through the real bearer
and audience layers and cannot obtain this owner key.

The browser imports the key as non-extractable and retains it only in this
document's memory; an initial connected read warms it for later offline
edits. Reload requires a successful authenticated key retrieval. Failed
unlock, corrupt ciphertext, changed server identity or unsafe restoration
preserves ciphertext and blocks canonical reads/writes. New edits made
while unlocking cannot silently replace recovered notes. Queue checkpoints
are serialized; writes capture their sent prefix and snapshot together,
retain edits arriving in flight, and clear only the acknowledged writer's
record. A failed acknowledgment cleanup stays visible until retry succeeds.

Limits: the first offline edit before key provisioning is not durable;
recovery depends on IndexedDB, Web Crypto, Web Locks and retained tab session
identity. Closing a tab, losing its session identity, clearing browser data
or browser storage eviction can prevent automatic recovery. Unsupported
browsers retain the memory-only save path and show recovery unavailable.
Storage quota/format failures remain explicit. The checkpoint limit is six
MiB, allowing a Canvas and its baseline; larger pending history fails local
checkpointing without preventing canonical saves. A live unlocked tab may
still hold readable content, as the existing Canvas does; erasure blocks
future retrieval and reload, not a claim to wipe a person's browser memory.
This does not provide offline unlocking or recovery across arbitrary new
tabs/machines, and does not close the wider S5 authorization/acceptance work.

Existing plaintext Canvas files from the supported data baseline are
converted once under their historical writer lock. The converter validates
the source, creates an Agent-owned Document containing conversation-key ciphertext,
reads that copy back, checks fencing, then removes the identical plaintext
source. A conflict or failed conversion preserves the source and refuses
rather than treating the Canvas as empty. Ordinary reads and writes use
Documents only, without a separate Canvas lock or retry model. Conversation
holds and erasure act through the existing conversation key; Document
collection remains confined to its Agent scope, separate from conversation
ledger objects. The ciphertext is owned by the Document version, so
history pruning releases old snapshots through ordinary Document collection.
Unknown fields stay inside the sealed content.

Browser acceptance on 2026-10-18 exercises 39 Canvas interactions at
1440 × 900 and 390 × 844, light and dark. The fixture reads saved content
and comments through artifact/version routes and retains the existing
local preview-origin contract. This verifies mocked-provider UI behavior;
it does not establish forced-reload durability or shared-page authorization.

### 0b. Pointing, discussion and new versions

A viewer says what a reader can point at in each of its views (`ViewerSpec.selects`): source lines, or a place in a document (the reader's own address, shown only under Show technical details). A preview page or an image has no place a comment could name, so nothing is selected there and a comment is about the whole file. Pointing opens the discussion; the discussion (`CanvasContext`) offers **Comment** (saved on the version being read, asks nobody) and **Ask Agent** (saves it and asks for a revision, or for a file that is not a saved draft, sends a request that names the place). A comment's place can be shown again with Show where. Activity lists only what the sandbox records and comments show (versions saved, the Agent revising, comments), and Changes lists the files this version adds, changes or removes.

A newer version of the draft being read is announced (`NewVersionNotice`) and never replaces it: reading it opens another tab, comments stay on the version they were written on, and dismissing the notice keeps it away until a still newer one arrives. Not built: a pointing mode for interactive pages, regions of an image, and handing work over to another person or Agent (collaboration stage C3).

### 0c. Subjects that are not files

A subject need not be a file: a scheduled routine is `{ kind: "automation", taskId }` (`AutomationViewer`). It has no path, conversation or comments, so its registry entry says `feedback: "none"` and `selects: () => null`, and the frame draws no comment area for it. The viewer reads the task list, keeps itself current every ten seconds, and offers what the task list offers for one routine (run now, pause or resume, retry a waiting delivery, see the last run); the words for cadence, status and delivery are shared with the task list (`taskWords.ts`). The task list's "open" opens one. A routine that has been deleted says so. A new non-file subject kind (a mail thread, a calendar view) is the same three steps: a `CanvasSubject` variant, an entry in `canvasViewers.ts`, and a viewer in `components/canvas/`.

The read-only daily mail/calendar view is `{ kind: "daily_mail_calendar" }` (`DailyMailCalendarViewer`). A **Today** action stays available in each Agent conversation. The viewer reads only connected accounts with available credentials using the owner-authenticated mail/calendar preview APIs; it does not open an Agent read tool or send provider content to the model. It presents the local day's calendar events and free/busy intervals, recent inbox messages, per-source refresh failures, and links supported messages to the existing account conversation preview. It loads at most two accounts concurrently, bounding provider request bursts while still aggregating all linked accounts. Data loads on open and explicit refresh. Like other non-file views it has no comments or file path.

### Restoring subjects

Persisted Canvas subjects validate all supported kinds, including composed
answers (session and turn identity) and saved pages (artifact identity).
All nine current kinds round-trip through stack opening, serialization
and restoration in source tests. Missing identity fields or a mismatched
stored identity are refused. This covers subject restoration; draft
retention, pending-operation reload recovery and saved-page resource
authorization remain separate work in the architecture and presentation hardening plan.

### 0. What the Canvas shows: a subject

The Canvas opens a `CanvasSubject` (`src/canvasSubject.ts`), which names what is shown by identity, not by path: a workspace `file`, an `execution_artifact` (a run's scratch file), a `draft_file` (one saved version, read through hash-checked candidate routes), `inline` markup from the conversation, or a `live_server`. Each kind is read through the one route its identity names, and there is no fallback between them: an unreadable file is an error, never replaced by text found elsewhere. `openArtifactFile(path, origin?)` is the entry point for files. A bare reference (a path in the Agent's text) is matched to a run only by an identical path made by exactly one run; if several runs made it, the reader is asked instead of a file being chosen.

### A. SolidJS State Atoms (`store.ts`)
Earlier iterations coupled layout modes and active artifact payloads inside a single compound signal. Toggling view modes caused the compound object to change identity, triggering unwanted iframe reloads and resetting runtime application state.

The state is cleanly decoupled:
- `canvasStacks`: each conversation's open tabs (`canvasEntries`), with the one in front (`canvasEntry`); a Canvas belongs to its conversation (§0a).
- `canvasSubject` and `canvasMode` (`"split" | "focused"`): read from the entry in front.
- `canvasDevice`: `"desktop" | "tablet" | "mobile"` responsive viewport simulation, one signal for the device.
- `canvasOpen`: whether the active conversation has an entry in front.

`createEffect` in `ArtifactCanvas` tracks `canvasSubject()` strictly, ensuring that toggling between split/focused modes or switching between 100%/768px/375px viewports never reloads the underlying iframe.

### B. Asynchronous Generation Guarding
Rapid navigation between artifacts or dev-server startup introduces potential race conditions. Every viewer loads through one loader (`components/canvas/createLoader.ts`): a load overtaken by a newer one, or by the viewer closing, is discarded and its result disposed, so a slow read never replaces a newer subject's content and nothing it allocated (a blob URL, a preview, a dev server's lease) outlives it. A reload keeps what is shown until the fresh content is ready.

### C. Graceful Exit Animation & Reopening Safety
Closing the last tab slides the Canvas away (`canvasSlideOut`, `EXIT_MS = 220` in `ArtifactCanvas.tsx`); closing one of several brings the next forward at once. If another tab comes forward or the conversation changes during the slide, the close finishes at once (`finishClose`) and the new subject renders without interruption.

---

## 3. Polyglot Format Engine

| Format | Detection Strategy | Presentation Implementation | Security & Sandboxing |
|---|---|---|---|
| **HTML Prototypes** | `.html`, `.htm`, `.xhtml`, or inline HTML snippet without conflicting file path | A preview origin (§3.2) when files sit behind the page, otherwise a sandboxed `<iframe>` with `srcdoc` | `origin` sandbox on a preview origin; `static` sandbox and no-network CSP for `srcdoc` (§3.1) |
| **Dev Servers** | a `live_server` subject, shown from the Live preview list in the dock | Live `<iframe>` on the server's own port, framed under the other loopback name than the app's, started and stopped by the viewer | `origin` sandbox (§3.1) |
| **PDF Documents** | `.pdf` extension | Native browser PDF viewer in full-bleed `<iframe>` | Loaded via authenticated raw stream `api.readFileRaw()`, converted to managed blob URL, revoked on cleanup |
| **Images** | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`, `.ico`, `.bmp` | Responsive image viewport with checkerboard transparency grid | Blob URL streaming, containment sizing |
| **Office files** | a Word, Excel, PowerPoint or Visio package by content type | `OfficeViewer`: the read projection drawn by the client, with the document pane for text | Parsed only in the broker worker (invariant 39); nothing in the package runs |
| **Tables** | `.csv`, `.tsv` and similar | `TableViewer`: a sortable, searchable grid | Text content, no script execution |
| **Code & Config** | `.rs`, `.ts`, `.tsx`, `.py`, `.json`, `.toml`, `.yaml`, `.md`, `.sh`, etc. | Formatted `<pre><code>` block with one-click clipboard copy | Text content display, no script execution |

### 3.1 Preview isolation

A preview runs someone else's markup, so its isolation is decided by the client and never by the data that describes the preview.

- **Sandbox.** `previewSandbox()` (`safeUrl.ts`) is the only source of an `iframe` `sandbox` value: `static` (`allow-forms`, opaque origin; scripts blocked) for markup from the conversation and for a page shown as one document, `origin` for a page served from an origin of its own (§3.2), which is not the app's origin and so may keep its own storage and open windows. `allow-same-origin` on a `srcdoc` frame would give the previewed script the app's origin and its authenticated API, so it is never reachable from a card.
- **Navigation.** A frame's own policy cannot stop it navigating itself away (a script, a link or a refresh), so the page that frames it does: the web app answers with `frame-src 'self' blob: data: http://127.0.0.1:* http://localhost:*` (`embedded_ui.rs`, `the_app_page_confines_frames_to_the_app_and_loopback`) and the desktop shell sets the same in `tauri.conf.json`. A preview cannot be sent to an internet address, whatever it runs. A preview on an origin of its own is still allowed to open a window (`allow-popups`).
- **Network.** `sandboxedSrcdoc()` always writes `connect-src 'none'` and `script-src 'none'`. CSP fetch directives do not block WebRTC, so document scripts cannot run. There is no card field, prop or argument that widens it, so the chrome's "Safe preview, offline" is what the frame enforces. A preview that needs network access is a separate, user-granted setting, not built.
- **Theme.** After the policy and before the markup, `sandboxedSrcdoc()` writes one `<style data-vak-theme>` block and nothing else: the active theme's tokens as CSS variables on `:root` (`--vak-bg`, `--vak-fg`, `--vak-muted`, `--vak-accent`, `--vak-rule`, `--vak-font`, `--vak-font-mono`, `--vak-radius`), read from the app's computed root style when the document is built, with `color-scheme` and `body { font-family: var(--vak-font) }`. A value cannot close the block. A theme switch rewrites that leading block in the open preview's document (`withCurrentTheme`, `previewTheme.ts`) and reads no file again; the sandbox value and the policy do not change. `emit_markup_preview_card`'s description names the variables (docs/design/96-answer-projection-gaps.md, Q1).
- **CSP placement.** The policy is the first markup after the document's doctype, so no script (before `<head>`, or in a string or comment that mentions `<head>`) runs ahead of it.
- **Server side.** `emit_file_preview_card` (a file, `ui.preview`, `artifact_path` required) and `emit_markup_preview_card` (inline markup, `ui.markup`, `html` required) have closed schemas, and `normalize_payload` drops `sandbox` and `connect_src` if a model sends them anyway. The client ignores them regardless, because the fence path does not go through the tool.

Tests: `crates/vak-client-ui/tests/preview-isolation.mjs`, and `ui_preview_never_carries_isolation_settings_from_the_model` in `vak-core`.

### 3.2 Preview origins

A page with files behind it (a saved draft's site, a run's output, a page in the workspace) is not poured into a `srcdoc` frame, where relative stylesheets, scripts, modules, `fetch` and links cannot work. `POST /previews` (`crates/vak-server/src/preview.rs`) opens a preview of one file under a scope (`candidate`, `execution` or `workspace`) and answers with the URL to frame.

- **An origin of its own.** Each preview listens on a fresh loopback port, so it is a browser origin of its own, and it is framed under the *other* loopback name than the one the app is reached by (`127.0.0.1` and `localhost` are two sites). It shares no cookie, storage or `document.parent` access with the app, and the app's routes, credentials and cookies never reach it.
- **Scope.** The listener reads only through the same functions that serve the file to the owner: a saved version's manifest files (hash-checked), a run's scratch directory, or the conversation's own workspace (its Agent's folder) below the opened page's directory, exactly, with dotfiles refused and nothing looked for elsewhere. `..`, backslashes and NUL are refused before any read. The path starts with an unguessable token; a wrong or missing one is a plain 404.
- **Policy.** The page may load its own files but runs only the fixed, hash-authorized theme bridge; document scripts and event handlers are blocked. It may reach no other origin (`connect-src 'self'`, no external images, fonts or frames); only the app's loopback origins and the desktop shell may embed it. `GET` and `HEAD` only, and a request whose `Host` is not a loopback name is refused (DNS rebinding).
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
- **Theme Awareness**: the canvas follows the client's themes (`theme.ts`: match system, light, dark, high contrast, frosted light, frosted dark; docs/design/75-visual-refresh.md).
- **Keyboard Navigation**: Pressing `Escape` closes the canvas.

## Preview script restriction — 2026-10-20

A real Chrome probe sent five loopback STUN packets from an opaque-origin
`srcdoc` carrying `connect-src 'none'`. Automatic document previews therefore
block arbitrary scripts: static CSP uses `script-src 'none'`, and the isolated
preview server permits only the exact SHA-256 of its fixed theme bridge.
Origin previews retain relative styles/images and native form controls, but
script-powered controls, modules and document fetch do not run. Result cards
and Workbench frames also omit `allow-scripts`. Popouts retain a sandboxed
wrapper rather than opening origin documents directly. Configured live dev
servers remain a separate explicitly started capability; they are not described
as offline document previews. See the
[acceptance record](../audits/security-hardening-2026-10-20.md).
