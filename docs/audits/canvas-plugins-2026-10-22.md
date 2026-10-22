# Installed Canvas workspace v1 acceptance — 2026-10-22

Status: **historical — implementation and verification record for installed workspace v1; shared browser UI/worker checks passed later the same day; installed-device qualification remains open.**

The maintainer requested completing installable plugin support, isolation,
domain commands, undo and Agent context on the common web/desktop core. This
slice builds a defined v1 contract, not a general installed CAD/science UI.
The [authoring guide](../canvas-workspace-authoring.md) specifies its surface.
The [earlier rendering acceptance](canvas-foundation-2026-10-22.md) measures the
owned drawing/WebGL2 fixtures and temporary native WebKit probe; it does not
qualify this new package UI or actual browser Worker execution.

## Implemented boundaries

- Native package inspection validates namespaced workspace JSON, bounded
  definitions, closed supported schema keywords and inspected WASM asset paths.
  Installed packages start disabled. Definitions bind to digest/generation and
  existing project/shared plugin shadowing, effective config and channel policy.
- Only owned declarative rendering runs in the application. Pure numeric WASM
  runs in a fresh dedicated worker with no imports, bounded linear memory,
  tables, functions, module and input/output bytes. Abort/deadline disposes the
  worker. No arbitrary package JS, privileged URL fetch, credentials or native
  IPC is granted by this contract.
- Registry writers use an OS-held lock, replacing the expiring lock-file lease.
  Bounded resource commits share it and recheck activation; disable/update waits
  for an admitted commit, and stale commits after disable are refused.
- Conversation-sealed Documents hold revisioned resources/history. Commands,
  undo and redo have actor/run traces and CAS. Human routes and session-bound
  Agent tools use one core operation. Catalog entries inherit conversation
  audience and disappear while hidden, then return on restoration.
- Ask Agent freezes only selected saved declared fields at an exact revision,
  writes a typed WorkspaceContext entry with a conversation-object projection,
  and links the ordinary admitted message. Pending inputs are not projected.
- Shared launcher and fields view support unavailable plugins, refresh,
  checkpointed unsent values, partial saves, concurrent typing, conflict refusal,
  undo/redo and explicit context previews. Old poll responses cannot replace a
  newer resource revision. Disabling/replacing contributions aborts UI jobs.

## Verification

- All 44 Node test files pass, including real binary WASM ABI execution,
  malformed/import/shared/unbounded memory refusals, typed command input,
  prototype/address refusal, partial-save and concurrent-input preservation.
- Plugin, session and catalog library suites pass: 29 + 108 + 23 tests; two
  pre-existing session tests are ignored. This includes installed contribution
  enable/disable, closed schema/namespace checks, OS writer exclusion and sealed
  resource CAS/undo/redo/concurrent writers.
- Both production bundles and the Canvas fixture TypeScript build pass.
- Strict all-target Clippy passes for plugin, session, catalog, core and server.
- The real HTTP bearer/audience-stack integration passes installation, owner
  commands, guest denial, invalid authentication, stale revisions, foreign
  conversations, undo/redo, selected context reconstructed from the ledger and
  disable revocation. Follow-through also checks the typed context/message link
  and catalog hide/restore. The provider is a local test implementation.
- Session-bound Agent tool integration checks creation, command conflict,
  foreign-conversation refusal and disabled activation through the shared core
  service. Another 54 capability/admission tests pass. This is tool integration,
  not live-model UI acceptance.

## Initial browser blocker

> **Historical record** — the initial verification stopped here; the browser follow-through below supersedes this blocker.

Local browser control is disabled in Work settings, so no visual acceptance of
this new UI or actual browser Worker execution is claimed. The production-host
fixtures `tests/canvas-host.html?run` and `tests/canvas-installed.html?run`
typecheck and are ready to run at 1440 × 900 and 390 × 844, light and dark.
The installed fixture exercises real worker success and abort of an infinite
WASM function; it was authored but not run. Pure Node WASM results do not prove
worker termination, browser scheduling or native-shell behavior.

Installed Tauri, Windows/Linux engines, physical touch/pen, refresh-rate/input
latency, GPU memory/context loss and long-session fidelity remain broader gates.
The parameter package is not a solver. UI computation returns proposed command
arguments; server schema/CAS validation does not prove numerical correctness.
No real-home plugin was installed, no installed app was replaced, no release
was made and no backward-compatibility or migration layer was added.

## Browser follow-through — 2026-10-22

Browser access returned and the production shared components were exercised
through Vite's web host in the in-app browser. The authenticated API is a local
fixture stub; real server authority/admission is established by the separate
HTTP integration above. No live-model or installed native-shell acceptance is
inferred from these browser fixtures.

| Fixture | 1440 × 900 light | 1440 × 900 dark | 390 × 844 light | 390 × 844 dark |
|---|---|---|---|---|
| Host lifecycle | 15 passed | 15 passed | 15 passed | 15 passed |
| Installed contract + launcher + real worker | 18 passed | 18 passed | 18 passed | 18 passed |

The first lifecycle run found that conversation/view replacement could capture
an internal Canvas header as the opener. Closing then stranded focus on the
page body. The frame now retains the external opener across replacements and
restores it on final close. All four lifecycle runs passed after this fix.

Coverage includes launcher start/reopen, typed saves, checkpoints, undo/redo,
disable/re-enable with unsent values, remote-revision conflicts, explicit discard,
selected saved Agent context, isolated real Worker WASM success and abort of a
worker request containing an infinite WASM function. The abort assertion checks
the cancellation result specifically; a compilation/worker error cannot count
as cancellation success. Lifecycle checks include disposal/replacement, late
callbacks, conversation separation, discard confirmation and focus return.
Both layouts fit without horizontal overflow. Screenshots were inspected in
light/desktop and dark/phone; DOM dimensions were checked for every matrix run.

Evidence: [results](../assets/canvas-plugins-2026/browser-results.json),
[desktop](../assets/canvas-plugins-2026/desktop-light.jpg) and
[phone](../assets/canvas-plugins-2026/phone-dark.jpg).
Both production builds, fixture typechecking and all 44 Node test files pass
after the focus fix. Native Tauri/other engines, physical input devices and
performance/fidelity qualification remain open. These short UI/worker tests do
not measure presented frame rate, GPU time, solver accuracy or input-to-photon.
