# Canvas as an extensible working surface — architecture review

Status: historical. Review of the working tree on 2026-10-22, including the
uncommitted client work present at session start. Findings describe that tree;
the proposed host contract below is not implemented or an approved runtime change.

## Verdict

Canvas has the beginnings of a general working surface, but its extension
boundary is still a built-in viewer interface. A package can contribute
presentation packs today; it cannot install a new Canvas workspace. Treating
those two capabilities as equivalent would promise something the runtime does
not provide.

The maintainer's intended use is broader than previews: a plugin might provide
an entire 3D, medical-imaging, engineering or other working environment. The
right abstraction is a domain-independent workspace host. Preserve the small
core: extensions own their domain, while Vakyartha owns identity, access,
state admission, lifecycle, model context, review and effects. Do not add a
`medical`, `engineering`, or `3d` case to the frame for each new application.

This review proposes a direction and acceptance gates, not a new schedule for
implementation. In particular, executable plugin UI needs a separately designed
and approved isolation contract; ordinary HTML page-code permission is not one.

## Maintainer constraint: clean replacement, no compatibility layer

The maintainer stated on 2026-10-22 that there is no user base to preserve and
requested clean design and fixes without stale code, migration paths or
backward-compatibility machinery. Build the end state directly in the common
web/desktop core and client. Remove superseded paths in the same change;
do not add dual readers/writers, legacy subject adapters, old-schema converters
or parallel runtimes to keep the prior Canvas working. Existing legacy Canvas
conversion code is not a pattern to retain in the replacement.

Protocol/state schema identifiers may validate the current contract; they do
not promise support for earlier contracts. Reject unsupported input explicitly
and atomically rather than partially reading it and saving a reduced result.
Plugin disable/removal and temporarily unavailable capabilities are ordinary
states of the current design: preserve the work's canonical resource identity
and show an unavailable surface, without keeping an obsolete renderer alive.
Artifact versions and parent lineage remain domain data, not compatibility code.
This review does not authorize deleting real data or resetting an installation.

## What exists and should be preserved

| Layer | Current implementation | Assessment |
| --- | --- | --- |
| Entry points | Conversation previews, answer expansion, files, Library pages, Today, routines and live previews call `openArtifactCanvas` | A shared destination already serves different kinds of work |
| Subject identity | `CanvasSubject`, `subjectKey`, `subjectReader` and `previewSource` | Scoped readers distinguish workspace, run and saved-version bytes; no fallback to a different source |
| Frame | `ArtifactCanvas.tsx` | Shared tabs, split/focused layout, view controls, errors and discussion; there are still draft/live-server special cases |
| Viewer capabilities | `canvasViewers.ts`, `components/canvas/viewers.ts`, `ViewerProps` | Useful internal seam, compiled into the application |
| Persistence | `canvasStack.ts`, `canvasSync.ts`, `canvasSyncState.ts`, `canvasRebase.ts`, `canvasRecovery.ts`, server `canvas.rs` | Operations, CAS, conflict handling, encrypted recovery and conversation-sealed Agent Documents are valuable foundations |
| Content and interaction | Answer viewer reuses the presentation renderer; page viewer reads Library bytes; mail/calendar viewer uses governed APIs | Reuse the presentation, artifact and provider systems rather than introduce another one |
| Preview execution | `HtmlPreview.tsx`, `safeUrl.ts`, server `preview.rs` | Static previews and explicit temporary page-code execution have distinct policies; neither is a general plugin bridge |
| Runtime resources | `createLoader.ts`, `ServerViewer.tsx` | Overtaken results are disposed; live servers have shared leases |

Sources: [Canvas design](../design/66-immersive-artifact-canvas.md),
[Canvas plan](../plans/canvas-plan.md), [plugin ecosystem](../design/39-plugin-ecosystem.md),
[presentation capabilities](../design/91-presentation-capabilities.md),
[answers beyond one card](../design/98-answers-beyond-one-card.md).

## Findings against the current implementation

### C1 — Unknown subjects can be lost on a subsequent save

**Confirmed by a direct production-function probe; no production incident established.**
`fromStored` skips a tab when `isCanvasSubject` does not recognise its kind.
`toStored` then serialises only the retained tabs. Opening another known file
and saving sends a snapshot without the unknown tab and its note. The server's
opaque entry storage does not recover what the client omitted. This applies to
mixed client versions introducing a subject without an envelope-schema bump,
and would apply to unavailable extensions under the same design.

Probe: receive `file:s:known.txt` plus `extension:scene:1` with a note; restore,
open `other.txt`, serialise. The next write contains only the two known files.

**Fix:** the current contract must validate the complete Canvas atomically;
reject invalid or unsupported state without rewriting a reduced snapshot.
Represent a disabled or missing plugin explicitly through the current workspace
subject and canonical resource identity, with an unavailable-surface view.
Do not add support for unknown legacy subject shapes or mixed client versions.

Evidence: `canvasStack.ts` `fromStored`/`toStored`, `canvasSubject.ts`
`isCanvasSubject`, `canvasSync.ts` `validShape`, `vak-server/src/canvas.rs`
`check`. Current-schema validation should remain explicit; existing legacy
conversion is not part of the proposed replacement.

### C2 — Subject identity can alias different inline documents

**Confirmed by a direct production-function probe; no production incident established.**
Inline identity uses title, base path and a small rolling digest of HTML, but
omits its session. `<p>Aa</p>` and `<p>B@</p>` with title `Same` and no base path
both produce `inline::Same:1pyf3u2`. Reopening replaces the subject under the
same key; `HtmlViewer` loads by that key, so the old loaded content can remain
while the subject names different bytes. Identical markup with different
session-scoped relative assets also shares an inline key.

Answer-subset identity uses the same digest helper. It has the same class of
collision risk; this review reproduced the inline case only.

**Fix:** carry canonical producer identity and resource scope, with an explicit
content revision. Use an immutable recorded id or a collision-resistant content
identity where content identity is needed. Do not treat a short UI hash as a
resource or revision identity.

Evidence: `canvasSubject.ts` `subjectKey`/`digest`, `canvasStack.ts` `openEntry`,
`components/canvas/HtmlViewer.tsx` loader source.

### C3 — Editing state has no host lifecycle contract

**Confirmed interface gap, with reachable state-loss paths identified in source.**
The frame keys the mounted viewer by the active entry. Switching entries
unmounts it. `CanvasEntry` preserves view, selection and the frame's discussion
note, but `ViewerHandle` exposes only popout and safety text. No shared dirty
state, checkpoint, suspend/restore, close negotiation or edit transaction exists.
Eviction protects only `entry.draft`.

This is already narrower than the built-in working surfaces need:
`MailCalendarDraftWorkspace.tsx` holds unsaved recipient/body/event edits in
component signals and saves through an explicit `save` action. Those edits are
not the frame's discussion note. This review did not run a full-frame
mail-editor tab-switch reproduction, so the observed guarantee is the source
path and missing contract, not a completed browser loss test.

**Fix:** checkpoint workspace edits before suspension; protect dirty/pending
work independently of discussion notes. Separate durable domain revisions,
recoverable edits and disposable view preferences. An expensive renderer can
release GPU resources without discarding work. Do not keep every iframe alive
as the persistence strategy.

Evidence: `ArtifactCanvas.tsx` keyed `Show`, `canvasStack.ts` `CanvasEntry` and
`openEntry`, `components/canvas/types.ts`, `MailCalendarDraftWorkspace.tsx`.

### C4 — The registry is not a plugin workspace API

**Confirmed capability boundary, not a defect in declarative presentation packs.**
`ArtifactDisplayType`, `CanvasSubject`, validation, identity, readers, capability
specifications and component mappings are compiled tables/switches. A new
first-party subject requires coordinated source changes. `vak-plugin::Components`
contains presentation and assets, but no workspace declaration; presentation
packs compile into the closed primitive set and execute no code.

**Fix:** a versioned, namespaced workspace contribution, resolved through the
existing plugin inventory, digest, activation and permission machinery. Keep
package UI out of the privileged Solid component registry. Declarative packs
remain the appropriate route whenever existing primitives can express the UI.

Evidence: `canvasSubject.ts`, `canvasViewers.ts`, `canvas/viewers.ts`,
`vak-plugin/src/lib.rs` `Components`, docs 39 and 91.

### C5 — A full workspace needs richer interaction and resource contracts

**Confirmed interface limitations; future domain examples are design tests.**
Selection is only lines or an anchor string; the viewer handle has no commands,
resource access, document editing or model-context interface. Viewers import
application APIs and global store functions directly. The shared loader can
ignore/dispose superseded results but gives no `AbortSignal` to cancel work.
This is workable for built-ins, but cannot be the authority boundary for
untrusted installed UI.

File display dispatch is extension-based. There is no negotiated media type,
registered format adapter or deterministic viewer conflict resolution. Large
binary resources have no Canvas-wide range/chunk/stream contract; the current
Canvas snapshot is deliberately bounded and unsuitable for scene or volume
payloads.

**Fix:** inject scoped host services, typed version-bound selections, validated
commands and bounded resource handles. Resolve specialised formats using
registered metadata and brokered parsing, never domain keywords or arbitrary
frontend parsing. Keep heavy content in tenant objects/Library versions.

Evidence: `canvasSelection.ts`, `canvasSubject.ts` `displayType`,
`canvas/types.ts`, `createLoader.ts`, `artifactPreview.ts`, server `canvas.rs`.

### C6 — Shared chrome must grow by capability without overwhelming the stage

**Partly confirmed.** Split/focused modes, mobile document defaults, themed
controls and frame errors are sound. However, the frame has a fixed set of
preview-oriented controls and no tools/inspector/status slots. Focused Canvas
sets `aria-modal=true` without the shared `trapFocus` directive or an equivalent
focus lifecycle in this component. That differs from Settings and Sheet. This
review did not qualify the installed native shell or complete a four-case
responsive/accessibility matrix.

**Fix:** common navigation and trust/save/job status in host chrome; optional
workspace commands, tools and inspectors inside a bounded contribution model.
Let the stage own its internal arrangement. Focused mode needs keyboard entry,
containment and return; split mode must preserve access to the conversation.
Do not attach the existing unconditional focus trap to split mode.

Evidence: `ArtifactCanvas.tsx`, `focusTrap.ts`, `styles.css` Canvas rules.

## Proposed workspace host contract

The following are responsibilities, not a final wire schema. Introduce the
smallest common primitives that support genuinely different workspaces.

| Contract | Host owns | Extension owns |
| --- | --- | --- |
| Registration | Namespaced type, current contract/state schema, package digest, availability, priority/explicit viewer choice, policy admission | Format declarations and capability requests |
| Subject | Principal/Agent/space/resource identity, pinned version or explicit live-head mode, provenance | Domain object references inside the admitted resource |
| Surface | Tabs, conversation relationship, focused/split layout, theme tokens, accessible host chrome, unavailable/crashed state | Stage, domain tools, inspectors, internal panes and specialised gestures |
| State | Validated bounded checkpoints, revision/CAS, conflicts, encryption, recovery, erasure and sharing | Domain edit model and state schema; camera/slice/viewport preferences |
| Lifecycle | Open/activate/suspend/resume/close, cancellation, cleanup, quotas and plugin revocation | Resource allocation/disposal and resumable view reconstruction |
| Commands | Schema validation, current permissions, revision preconditions, review, effect/run admission and receipts | Named domain operations and undo semantics |
| Agent context | Explicit admission, bounded payloads/references, provenance and ledger reconstruction | Meaningful descriptions of the selected object, region, constraint or measurement |

A subject identifies **the work**. A renderer identifies **how to work on it**.
A live viewer instance identifies **this mounted view**. A saved artifact
version identifies **reviewable bytes**. Keep these separate so switching a
viewer or updating a plugin does not change the work's identity.

### One shared core for web and desktop

The maintainer reiterated on 2026-10-22 that web and desktop already share a
common core. This is a constraint on this proposal: build one workspace
contract in the existing Rust core/server and one host/frame implementation in
`crates/vak-client-ui`, consumed by both web and desktop. Plugin registration,
resource identity, commands, permissions, state/recovery, Agent context and
lifecycle semantics must not fork by platform. A workspace package targets
this shared contract rather than implementing separate web and desktop apps.

Keep platform differences behind narrow adapters with explicit capability
negotiation: browser/GPU features, native integration, resource transport and
enforceable execution isolation. Desktop privileges must not leak into a
workspace through direct native calls; web must not silently offer weaker
permission checks. Unsupported capabilities produce an explicit fallback or
unavailable state. Local-only previews remain an existing deployment limit,
not an architectural template for desktop-only workspaces.

Acceptance must exercise the same plugin package and resource revisions in
both web and native WebKit, including cross-surface reopen, concurrent edits,
permission revocation and unavailable capabilities. Layout and disposable view
preferences may differ; domain meaning and authority must remain the same.

### State must have separate lifetimes

- Domain work: immutable Library versions and existing review threads, or
  Agent-authored Documents for non-deliverable structured state. Mutations name
  their actual parent/version and preserve conflicts.
- Recoverable editing: encrypted checkpoints with a base revision, schema and
  plugin provenance; never a blob of executable state. A clear saved/pending/
  conflict status must cover domain edits as well as frame notes.
- View state: camera, zoom, scroll, active tool and panel arrangement, with an
  explicit local/shared policy. Do not broadcast pointer movement or camera
  frames through the conversation Document on every animation frame.
- Runtime state: GPU objects, subscriptions, decoder workers and sockets;
  disposable and rebuilt after suspension or crash.

Do not invent a scene database, job queue, grant list, trigger engine or file
store beside the existing data architecture. A server job uses recorded runs;
external actions use effects; repeated work uses triggers; revocation uses the
plugin/permission lifecycle. Closing a view releases its subscription, while
stopping a durable job is a distinct governed operation.

### Commands and context are the common language

A 3D surface might expose `select object`, `set transform` and `render`; an
engineering surface might expose `set constraint` and `solve`. The host should
understand the command envelope, expected revision, capability and receipt.
It should not contain the physics, geometry or medical semantics.

Selections need a versioned locator with a registered namespace and validated
payload: object/face, page/region, series/frame/region, or sheet/cell. Measurements
must retain units and coordinate frames when those matter. Opaque locators
alone are insufficient for the Agent: provide a concise semantic description
and resolvable evidence references. Do not automatically send an entire scene,
image series, private record or screenshot because its workspace was opened.
Any selected state admitted to a model must be reconstructable from the ledger.

### Executable surfaces require an explicit execution boundary

The existing rule says Vakyartha owns the presentation runtime and prohibits
third-party agent UI runtimes. Preserve it. Plugin-defined workspaces must not
be raw components mounted with application credentials or imported into the
host process. Declarative packs need no new execution privilege.

A future executable workspace requires a Vakyartha-owned isolated host and a
small versioned bridge. Bind its messages to a live instance and current
plugin digest/permission generation; validate schemas, sequence and size;
revoke pending calls and resource handles when disabled or replaced. Supply
only scoped resource operations and admitted commands, never bearer tokens,
ambient filesystem access, provider secrets or a generic privileged `fetch`.
The server/broker remains the authority even when UI claims a human gesture.

An iframe alone is not a no-network guarantee. The recorded WebRTC probe and
current page-code disclosure make that explicit. WebGL/WebGPU, worker support,
binary transfer, network confinement, frame ancestry and native-WebKit parity
must be qualified for the chosen host. Script-powered 3D is not enabled merely
by declaring another `ViewerSpec`. Unsupported capabilities should yield a
truthful fallback, not a weakened sandbox.

### Test the abstraction with different full workspaces

| Workspace | What it tests |
| --- | --- |
| 3D scene editor | Object/face selection, camera state, mesh streaming, GPU disposal/context loss, transforms, render runs, export versions |
| Medical-imaging workspace | Series/frame identity, region annotations, calibrated coordinates, bounded volume reads, access/revocation and explicit context sharing |
| Engineering workbench | Assemblies, dimensions/units, constraint edits, version-bound solve inputs, cancellable simulation runs and result lineage |
| Existing mail/calendar workspace | Forms, dirty edits, provider authority, review/effects, account revocation and drafts across tab changes |

These are contract tests, not a commitment to build all four products or a
claim of clinical or engineering validation. An extension carries its own
domain validation responsibilities. The first engineering prototype can be
small: a real editable scene plus the existing forms workspace is a better
boundary test than several static previews of different file extensions.

## Suggested order and exit gates

1. Replace partial subject restoration with atomic current-contract validation
   and explicit unavailable-plugin state; fix inline identity. Test refusal
   without reduced writes, plugin disable/removal and identity collisions.
2. Define subject/resource identity and viewer-independent dirty/checkpoint
   lifecycle. Exercise tab switch, eviction, reload, crash, concurrent edits
   and plugin removal; never silently drop work.
3. Add scoped service injection and versioned command/context contracts to
   built-ins. Demonstrate a workspace using only those services.
4. Decide and qualify the isolated execution model before exposing executable
   workspace packages. Prove that forged/replayed messages, disabled plugins,
   changed permissions and unavailable renderers fail safely.
5. Add workspace declarations to the existing plugin lifecycle; install,
   update, rollback, disable and reopen against the current shared contract;
   reject incompatible packages explicitly, without compatibility adapters.
6. Qualify a real 3D/editing prototype and the existing forms workspace in the
   running web client at 1440×900 and 390×844, light/dark, then native WebKit.
   Include large data, memory pressure, keyboard operation, cancellation and
   remote deployment. Remote multi-file/live previews remain separately open
   in the existing Canvas plan.

## Verification and limits

- Passed the seven existing Node test files: Canvas subject, stack, selection,
  rebase, sync-state, recovery and preview isolation.
- Ran the two production-function probes reported under C1 and C2.
- Inspected the real mail/calendar viewer using its isolated browser fixtures.
  The synthetic demo reported 14 checks passed; the daily fixture completed
  its checks through bounded provider fan-out. These use synthetic data and
  fake HTTP/provider routes, not private accounts or real effects.
- No runtime code changed; no full gate, installed-app qualification, new
  plugin implementation or complete responsive Canvas matrix is claimed.
- Existing uncommitted runtime/UI work was retained. The review does not
  certify unrelated changes in that working tree.
