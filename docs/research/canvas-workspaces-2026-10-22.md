# Canvas workspaces: lessons from other systems, 2026-10-22

Status: **historical — research and design decisions for the shared Canvas foundation; this is not evidence that external plugin execution or domain engines have shipped.**

## Our agenda

Vakyartha's Canvas is a place to work with the Agent, from a small artifact to
an entire specialized environment. It is not synonymous with an HTML canvas,
a whiteboard, a 3D engine, or a document editor. The common host owns resources,
layout, lifecycle, recoverable work, selection and conversation connection.
An extension owns the domain model, renderer, tools and domain-specific panels.
One client implementation serves browser and desktop; platform adapters expose
specific capabilities. There is no second desktop Canvas architecture.

The maintainer requires a clean current contract, with no compatibility readers
or migrations. This research informs our design; it does not introduce another
agent UI SDK, runtime, state store or permission system.

## What we learned

| System and primary source | Useful lesson | Consequence for our Canvas |
| --- | --- | --- |
| [OpenAI extensions](https://developers.openai.com/plugins/build/extensions) and [ChatGPT UI](https://developers.openai.com/plugins/build/chatgpt-ui) | Tools can have full working surfaces; host capabilities and platform availability differ. Temporary view state differs from authoritative business state. | A workspace is a first-class subject. Detect the capability actually available, rather than assuming desktop or web implies it. Saving a tab is not committing a business action. |
| [LibreChat artifacts](https://www.librechat.ai/docs/features/artifacts), [smooth streaming](https://www.librechat.ai/docs/features/smooth_streaming), [resumable streams](https://www.librechat.ai/docs/features/resumable_streams) | Artifacts need enough room. Rendering polish need not delay content or replay entrance animation on reconnect. Bundler dependencies can impose external network requirements. | Keep focused/split modes, independent renderer lifecycle and level reads. Do not make CDN reachability or decorative animation a prerequisite for work. |
| [Open WebUI artifacts](https://docs.openwebui.com/features/chat-conversations/chat-features/code-execution/artifacts/) and [rich UI](https://docs.openwebui.com/features/extensibility/plugin/development/rich-ui/) | HTML artifacts and tool UIs are easy entry points, but iframe sizing and sandbox/CSP policy become architectural concerns. | Keep our existing owned preview isolation. Do not copy same-origin troubleshooting settings into an extension privilege model. A generic postMessage bridge is not authority. |
| [OpenHands Canvas](https://www.openhands.dev/product/canvas) and [architecture](https://github.com/OpenHands/OpenHands/blob/main/docs/architecture.md) | A workspace can encompass applications and ongoing work. Execution belongs behind the agent-server boundary. | A renderer never becomes an alternate route to filesystem, provider credentials or execution. Keep Vakyartha's existing broker and effects. |
| [tldraw agent kit](https://tldraw.dev/starter-kits/agent) and [persistence](https://tldraw.dev/docs/persistence) | Structured actions/context make graphical work agent-addressable. Document state and camera/session state are different. | Extend domain selections and commands with typed addresses. Persist resources separately from view state. Do not replay pointer motion as durable document edits. |
| [assistant-ui architecture](https://www.assistant-ui.com/docs/architecture) and [CopilotKit shared state](https://docs.copilotkit.ai/a2a/shared-state) | Runtime state, UI projections, adapters and persistence have distinct owners; context and writable state should be explicit. | Canvas uses existing authoritative resources. Give the Agent a bounded, revisioned context projection, not arbitrary renderer memory. These are references, not dependencies. |
| [json-render catalogs](https://json-render.dev/docs/catalog) and [A2UI renderer development](https://a2ui.org/guides/renderer-development/) | Bounded registered vocabularies are easier to validate than arbitrary generated executable UI. Platform renderers need a common semantic contract. | Retain our own presentation vocabulary and compiled renderer registry. Separate declarative UI contribution from privileged executable contribution. |
| [VS Code custom editors](https://code.visualstudio.com/api/extension-guides/custom-editors) | Multiple views may share one model. A hidden renderer should not be the sole keeper of unsaved work. Keeping hidden contexts alive costs memory. | Host checkpoints outlive mounted viewers. Unmounting a tab releases renderer resources; remount restores edits. Saving is a resource operation independent of panel visibility. |
| [Dockview](https://dockview.dev/docs/core/overview/) | Panel composition includes serialization, focus and keyboard behavior, not just movable rectangles. | Keep one shared layout/focus contract. Do not add an elaborate docking library before a real workspace needs it. |
| [LangChain Open Canvas](https://github.com/langchain-ai/open-canvas) | Useful historical document-editing example, but the repository is archived. | Do not choose it as a current platform foundation. |
| [Tauri WebView versions](https://v2.tauri.app/reference/webview-versions/) | Desktop uses OS WebView engines, not one bundled Chromium version. | Test native WebKit separately. Browser emulation does not certify Windows, Linux, iOS or Android. |

These observations concern public documentation available on 2026-10-22.
Some APIs have tier, preview or platform restrictions; a page describing a
feature is not evidence that every installed product version supports it.
Open WebUI Computer is a separate ecosystem project, not proof that base
Open WebUI artifacts provide the same workspace host.

## The common contract and the boundaries

The built foundation has a namespaced workspace type, scoped resource identity
and explicit revision. A compiled contribution supplies a renderer and frame
capabilities through `registerCanvasWorkspace`. An absent contribution produces
an unavailable view while preserving the subject and its edits. Unknown or
malformed current-contract snapshots are rejected atomically, never filtered
into a smaller saved Canvas.

The host owns small JSON editing checkpoints with schema and human summary.
The boundary copies values and gives each mounted renderer a revocable editing
lease, so old async callbacks cannot alter a replaced view.
They survive viewer remount, participate in the existing encrypted Document
CAS/recovery path, block automatic eviction and require an explicit discard on
close. Conflict resolution compares pending work. These are recovery values,
not saved artifact versions, simulation snapshots or provider-side drafts.
Large domain data must use existing resource/artifact APIs; Canvas is not a
second storage engine. A renderer clears a checkpoint only after the actual
save succeeds, retaining newer edits made during that save.

Viewports expose CSS dimensions and physical backing dimensions. The shared
drawing helper measures resize/DPI changes, coalesces invalidations with
requestAnimationFrame and stops when hidden, offscreen, disposed or context-lost.
Static surfaces draw on demand; animation uses elapsed time and display cadence,
not an assumed 60 Hz timer. The renderer owns its GPU resources and restores
its graphics state after context restoration. Expensive computation belongs in
an appropriate worker or execution backend, not the input/render loop.

## Work still required for installable extensions

The compiled registry is not an executable package loader. Before packages may
contribute code, define one versioned contribution manifest and validate it at
install/activation. It must declare renderer identity, resource schemas,
commands, selection/context projections and required host capabilities. Wire it
through the existing plugin registry and permission revocation. An arbitrary
script must never run in the host origin because it declares a workspace type.

Design the isolated execution boundary explicitly: origin/process ownership,
message schemas, lifetime/revocation, access to resource handles, blocked
operations and crash recovery. Declarative presentation contributions can use
our existing capability model. Privileged executable contributions need their
own reviewed boundary, rather than a permissive generic bridge. A disabled
plugin keeps data and shows unavailable; it does not silently map to HTML.

Agent interaction needs domain-addressable selections, revisioned context and
typed commands with validation, undo/redo and existing approval/effect receipts
where appropriate. The host must not infer scientific semantics from pixels.
An input device/camera change is view state; changing a component or starting a
simulation is a domain command. Multi-panel workspace layout and command/undo
services remain open; do not advertise the checkpoint API as the complete SDK.

## Real 3D and scientific work

The acceptance renderer uses WebGL2 triangles, normals, depth, perspective and
lighting, plus wireframe, animation, pointer orbit, wheel zoom, ray selection
and a checkpointed material edit. It exercises the actual registered workspace
path. It is a test fixture, not a scientific solver or a shipped 3D plugin.

A proper 3D extension can own mesh/material/scene resources, animation tracks,
picking and domain panels. Engineering, biology, chemistry, physics and games
can use the same host while choosing their own rendering/compute engines.
Units, constraints, numerical accuracy, domain validation and reproducibility
belong to those extensions. A game loop, a medical volume viewer and a symbolic
math editor must not be forced into one scene format or one rendering engine.

## Qualification gates

Measure interaction latency, frame intervals and memory under representative
scenes, not just the empty frame. Include 60/90/120/144 Hz displays, fractional
DPI, mouse/trackpad/touch/pen, resize, background/resume, context loss, long
sessions, low-memory/GPU devices and CPU contention. Treat animation, numerical
simulation and playback clocks independently. Respect reduced motion.

Browser fixtures establish shared-client behavior at emulated viewport sizes.
Actual WebKit probing provides engine evidence on this Mac; it does not close
installed Tauri-shell, Windows/Linux or physical phone/tablet qualification.
The dated acceptance record states precisely what was measured, including
callback timing versus actual presented frames and CPU versus GPU timing.
