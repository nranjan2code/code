# UI completion checklist

Status: dated review ledger. A row's recorded proof is evidence for that
review, not a fresh verification of the current client.

Review started: 2026-09-10
Scope: shared client (`vak-client-ui`), desktop/browser hosts, and admin console.

This is an evidence ledger, not a claim of completion. A row moves to **verified**
only after current source, runtime behavior, and the appropriate focused test agree.

## Baseline

| Area | Current evidence | Status | Next proof |
|---|---|---|---|
| Everyday/Advanced preference | Persisted client signal and header toggle in `store.ts` and `WorkspaceHeader.tsx` | partial | Verify idle, streaming, approval, failure, replay, and authority parity |
| Shared result contract | `PresentationRenderer.tsx`, adaptive presentation runtime, preserved fallback; fixed live pre-existing presentation-store failure by versioning changed built-in seed definitions instead of weakening immutable digest validation; healthy Settings runtime now loads both persisted revision generations; client hydration now preserves the last-known presentation when an optional snapshot read fails instead of clearing a valid result; shared markdown and research-card copy actions now distinguish clipboard failure from success | partial | Cross-surface fixture matrix and historical replay |
| Everyday navigation | Friendly labels and recent/earlier task grouping in `Sidebar.tsx`; duplicate unsupported “My tasks” affordance removed and scheduled-task destination renamed | partial | Define a real My tasks destination from session history, if product intent requires it |
| Everyday context rail | `EverydayContextRail.tsx` with files/notes/next steps counts | partial | Make each section actionable and verify empty/loading/error states |
| Everyday first-run guidance | Authenticated browser render at 390×844 shows the real prompt-insertion examples for research, writing, analysis, and planning; each example populates the Task prompt textarea with its intended starter text; the single-click path returns focus to the textarea; narrow layout has no horizontal overflow and the mode controls remain reachable; authenticated desktop render at 1440×1000 exposes all four examples and the full Everyday task controls without overflow; fresh rebuilt-server runtime confirms current examples and controls; setup-status read failure remains visible with a retry action instead of hiding the setup banner | partial | Repeat the same path in desktop/Tauri |
| Advanced workspace | Workbench, changes, terminal, editor, preview, receipts, subagents, commitments; sandbox telemetry failures remain visible without blocking transcript load; authenticated desktop render at 1440×1000 exposes the Advanced workspace, Activity/Changes/Terminal controls, Workbench views, and accessible task controls; fresh rebuilt-server runtime confirms Everyday→Advanced switching and dock controls; fresh isolated browser runtime switched to Advanced, opened Activity/Changes/Terminal, and closed the workspace pane while preserving the task composer and mode controls | partial | Verify all controls and lifecycle/error paths |
| Admin IA | Navigation now uses Overview/Work/Operate/Configure/System groups; Knowledge owns Memory, Feeds, and Search in one expandable area; Model & providers names the existing layered model/defaults screen; direct `#/feeds` and `#/search` routes still render their own screens and keep Knowledge active; admin build and authenticated entry verified; fresh isolated current server runtime shows grouped canonical nav, Knowledge expansion, and direct Feeds/Search deep links with Knowledge active; fresh rebuilt-server runtime confirms current grouped nav, Knowledge children, Memory screen, and layered Settings screen; Home preserves auxiliary probe failures and visibly marks telemetry incomplete rather than treating failed probes as a clean empty dashboard; Sessions surfaces unavailable workspace ownership data and hides local-session actions until it can verify scope | partial | Split remaining canonical configuration/runtime ownership and add missing target destinations |
| Configuration scope | Shared/workspace controls and provenance exist in multiple screens; setup, Gateway, channel capability, and voice discovery preserve failures; client Settings no longer turns inherited MCP/hooks/plugins failures into empty data; Admin MCP/hooks use layer-specific GET/PUT endpoints and render explicit read errors with retry instead of treating failures as empty configuration; Admin Settings surfaces effective/layer/provider load failures with retry; Event Bus surfaces unavailable reads with retry; fresh authenticated runtime confirms Global→Workspace scope switching and preservation into Extensions; Settings mutations await effective-config and edited-layer read-back before completing; multi-bot deletion now propagates HTTP failures, and plugin source/key mutations await authoritative refetches while surfacing errors | partial | Audit every remaining editor for layer-correct GET/PUT, atomic ownership, and stale reload |
| Accessibility | ARIA and responsive rules present in source; unauthenticated client and Admin sign-in render at 390px with no horizontal overflow; both token fields have explicit accessible labels and client keyboard submission is verified; authenticated Everyday↔Advanced mode switching works at 390×844 with no horizontal overflow and the header remains reachable above phone overlays; authenticated new-task controls expose accessible names in the rendered snapshot; authenticated 200% text-size stress check preserves the prompt and mode controls with no horizontal overflow; authenticated Admin Home at 390px has explicit scope/action labels and no horizontal overflow; Admin keyboard traversal reaches the scope selector and Home link with visible solid focus outlines; rendered client reduced-motion mode collapses animation and transition durations to 0.01ms; healthy authenticated Settings opens without the presentation error, moves focus into the dialog, and restores focus to the trigger on Escape; Advanced dock tabs expose explicit button and pressed-state semantics; split-pane/header action buttons explicitly use `type=button`; refreshed authenticated 390×844 runtime confirms current Everyday guidance, reachable mode controls, and client/Admin body width equal to viewport; current-artifact keyboard traversal reaches primary navigation, both mode controls, Everyday examples, prompt, workspace and permission selectors, with visible focus treatment; Memory governance toggles expose their visible titles as accessible names; rebuilt-server accessibility snapshot confirms all four governance checkbox names; Admin provider/model and Memory note-scope controls now expose explicit accessible names; shared result renderer controls now declare explicit non-submit button semantics; fresh release-server browser run at 390×844 reports body width equal to viewport in Everyday and Advanced, with mode controls reachable | partial | Screen reader, client keyboard traversal, and task-pane checks |
| Performance | Advanced dock panels and Settings now lazy-load; web build emits separate panel chunks; measured current web entry assets are 283,678 bytes JavaScript (88,255 gzip) plus 159,765 bytes CSS (29,261 gzip), while Mermaid, Cynefin, WASM, and C++ grammar chunks remain 622–797 kB uncompressed and are not part of the initial entry; fresh embedded-server browser measurement records ~120 ms DOMContentLoaded, ~139 ms load, ~88.6 kB JS and ~29.3 kB CSS transferred initially, and ~6.2 kB transferred when Advanced first loads Workbench | partial | Measure interaction latency across real panel actions; split or defer remaining heavy renderer dependencies where runtime traces show they enter the initial path |
| Verification | Client/Admin typechecks and builds, shell acceptance, formatting, `cargo check --workspace --all-targets`, strict workspace Clippy, and full `cargo test --workspace` pass; both client hosts and Admin shipped bundles were refreshed; unauthenticated `/app/` and `/admin/` browser entries render sign-in without error overlays; client at 390px has body width equal to viewport; authenticated 390px mode switching passes in both directions; rendered client status transitions to Offline and then Reconnecting on browser network events; idle health polling now drives Offline/Live without overriding active stream states; focused server event/replay tests pass after fixing the SSE reconnect handoff to stop discarding the first subscribed broadcast event; manual client-created replacement streams now preserve and transmit the latest event cursor, with server support for both query and native `Last-Event-ID` resume; provider-local `server_smoke.py` deliberately disconnects after the initial SSE cursor and passes a complete resumed run (TurnStart, tool call, RunFinished, transcript count 4), including against the rebuilt `vak` binary; the smoke harness now tolerates bounded slow release startup while failing loudly on early process exit; the smoke harness also verifies the presentation stream emits an initial cursor; active-turn replay-window exhaustion remains visibly `resyncing` until terminal hydration rather than falsely reporting `live`; latest Tauri and web client builds pass after the resync-state change; native `cargo tauri build --no-bundle` and `cargo check -p vak-desktop` pass after refreshing the Tauri bundle; Admin Home preserves auxiliary probe failures and marks telemetry incomplete; presentation SSE now accepts the shared cursor header/query contract and emits an ID-bearing authoritative snapshot, while historical presentation delta replay remains unimplemented | partial | Authenticated desktop/browser matrix, replay-window exhaustion/resync behavior, presentation-stream delta replay, and remaining release checks; access token is required |

## Required work order

1. Establish source/API ownership map for every Everyday and Advanced control.
2. Fix Everyday navigation and context actions without weakening Advanced access.
3. Verify shared task state, approvals, failures, cancellation, reconnect, and replay.
4. Complete Advanced disclosure and technical-surface behavior.
5. Migrate admin navigation and canonical capability ownership.
6. Audit configuration scope, provenance, persistence, and stale-data handling.
7. Run accessibility, responsive, theme, and reduced-motion verification.
8. Reduce initial bundle cost through measured lazy loading.
9. Add or repair focused acceptance fixtures, then run broad repository checks.
10. Record every discovered defect, including pre-existing defects, with evidence.

## Done conditions

- Everyday and Advanced render the same underlying session, result, approval,
  outcome, and artifacts while changing disclosure only.
- Every visible action has a real success, validation, authorization, transport,
  cancellation, and stale-data outcome.
- Every capability has one canonical admin home and existing deep links remain usable.
- Configuration writes the intended layer atomically and shows effective value plus source.
- No unsupported state is represented as an empty successful state.
- Desktop, browser, narrow browser, transcript export, and supported channel
  projections preserve material result facts and safety state.
- Focus, keyboard, responsive, theme, contrast, text-size, and reduced-motion
  behavior are verified from rendered behavior, not source inspection alone.
- Build warnings are either resolved or documented with measured impact.
- Final report identifies completed work, remaining blockers, and external access needed.

## Incremental audit notes

- 2026-09-10: Hardened the shared event-bus replay boundary against a maximum
  resume cursor (`saturating_add`) and added a regression test. Presentation SSE
  serialization now emits a structured error payload instead of silently
  producing an empty frame. Focused `vak-server` event tests (10) and strict
  server clippy pass. Historical presentation delta replay is intentionally
  still open: the endpoint sends an authoritative snapshot and live projection
  frames, and must not fabricate deltas without a durable historical projection
  baseline.
- 2026-09-10: Workspace `cargo fmt --all -- --check` remains red on pre-existing
  formatting drift in `crates/vak-delivery/src/lib.rs` and an import-order change
  in the already modified `crates/vak-presentation/src/seeds.rs`; no unrelated
  formatting rewrite was applied.
- 2026-09-10: Rebuilt `target/release/vak` after the UI and SSE changes. The
  release smoke harness passed with disposable loopback ports: unauthenticated
  access was rejected, the presentation snapshot emitted cursor `0`, the
  primary stream resumed from cursor `1`, and the complete run produced
  `TurnStart`, tool, terminal, `RunFinished`, and four transcript entries.
- 2026-09-10: Admin plugin-source registration now waits for the authoritative
  source refetch before clearing the form or showing success; refetch failures
  remain on the mutation error path. Admin build, UI acceptance, and embedded
  asset integrity tests pass after the change.
- 2026-09-10: Admin plugin install/update now waits for source and catalog
  read-back before clearing input or reporting success. Memory governance
  toggles now await both effective-config and selected-layer provenance
  refreshes. Admin build, UI acceptance, embedded asset integrity, and diff
  checks pass.
- 2026-09-10: Client failure audit found only intentional JSON-parse fallbacks
  and last-known connectivity preservation in the remaining empty catches. The
  message-action and server-directory-picker controls now explicitly use
  `type="button"`; client typecheck, web build, UI acceptance, and diff checks
  pass. The web build continues to document large deferred renderer chunks.
- 2026-09-10: Permission `RuleEditor` now waits for both the selected-layer
  rules read-back and the parent config refresh before showing success. This
  closes a stale-state window in the configuration ownership contract. Admin
  build, UI acceptance, embedded asset tests, and diff checks pass.
- 2026-09-10: Feed/admin mutations now await projection read-back before
  announcing success: source enable/disable/edit/remove, alert deletion,
  ingestion, and quarantine release. Production Admin build, UI acceptance,
  and diff checks pass after the change.
- 2026-09-10: Replay architecture review confirmed that the server has no
  durable historical presentation-frame ledger—only the append-only session
  ledger plus a bounded raw-event ring. Presentation reconnect therefore
  remains authoritative-snapshot based; implementing historical presentation
  deltas requires a new append-only projection history and must not be faked
  from an unknown baseline. After the latest Admin changes, the rebuilt release
  binary passed the complete HTTP/SSE smoke run again.
- 2026-09-10: Feed alert creation and source-wizard creation now await parent
  projection refreshes before closing their forms or reporting success. Admin
  production build, UI acceptance, embedded asset tests, and diff checks pass.
- 2026-09-10: Operations Center outbox replay and sandbox promotion now await
  live snapshot/read-back before showing success. Sandbox refresh now reconciles
  both its records and the shared operations projection. Admin build, UI
  acceptance, embedded asset tests, and diff checks pass.
- 2026-09-10: Workspace selection controls in `DirectoryPicker` and
  `WorkspaceGate` now explicitly declare non-submit button behavior, including
  browse, choose, retry, and fallback actions. Both client production hosts
  were rebuilt; TypeScript, UI acceptance, and diff checks pass. The build’s
  existing large deferred-renderer warning remains documented.
- 2026-09-10: Client Settings voice, hook, and MCP mutations now wait for
  authoritative effective/layer read-back instead of relying on optimistic
  local state. Both client hosts were rebuilt; TypeScript, UI acceptance, asset
  integrity, and diff checks pass.
- 2026-09-10: Client capability settings now surface failures from plugin
  source enable/disable and signing-key revoke/restore actions; previously
  those inline async handlers had cleanup but no catch. Both client hosts were
  rebuilt after the source change; TypeScript, UI acceptance, and diff checks
  pass.
- 2026-09-10: Added acceptance assertions for generated Markdown copy-button
  semantics and the server-directory picker’s explicit button type. The
  client renderer import graph was also reviewed: Mermaid, terminal, and
  syntax-rendering dependencies are dynamically loaded; the remaining large
  chunks are deferred renderer dependencies rather than initial Everyday
  payload. UI acceptance and diff checks pass.
- 2026-09-10: Admin skill proposal, hook, and scheduled-task mutations now
  await their authoritative list refresh before showing success; this closes
  a stale-projection gap found during the detached-refetch sweep. Admin
  TypeScript check, production build, UI acceptance, and diff checks pass.
- 2026-09-10: Client feed view toggles, modal close/search-result actions, and
  plugin install controls now explicitly declare button behavior, preventing
  accidental form submission as these controls are composed into larger
  surfaces. The client package has no `check` script; its supported
  `typecheck`/build path was inspected and the full build plus UI acceptance
  passed. Existing deferred-chunk size warnings remain informational.
- 2026-09-10: Responsive audit found that the client mobile modal rule kept a
  360px minimum width even below 360px viewports, which could force horizontal
  overflow. The mobile rule now resets `min-width: 0`; client typecheck,
  production build, UI acceptance, and diff checks pass.
- 2026-09-10: Re-audited primary and presentation stream reconnect handling.
  Client cursors, server replay/resync, active-turn terminal hydration, and
  Admin authentication probing remain explicit and covered by source/tests;
  no additional safe defect was found in this pass. The complete workspace
  test suite passed (all executed tests green; only environment-dependent
  browser/network and real-terminal tests remain ignored by their declared
  prerequisites).
- 2026-09-10: Native parity verification passed `cargo check -p vak-desktop`,
  the client Tauri-host build, and the client web-host build after the
  responsive and button-semantics changes. The build still reports the known
  intentional App/BudgetBanner dynamic-import cycle warning and large deferred
  renderer chunks; neither enters the measured initial web payload.
- 2026-09-10: Accessibility dialog audit found the Advanced approval
  rule-preview overlay missing the shared focus trap. It now uses the same
  initial-focus, Tab containment, and focus-restoration directive as the other
  client dialogs. Client typecheck, web build, UI acceptance, and diff checks
  pass. The separate Admin doctor dialog remains an explicit follow-up because
  the Admin bundle has no shared focus-trap utility yet.
- 2026-09-10: Added the Admin-local focus-trap directive and attached it to
  the Operations Center “Ask doctor” dialog, including explicit close-button
  semantics. Admin typecheck/build, embedded asset integrity tests, and diff
  checks pass; the dialog now has an explicit keyboard containment and focus
  restoration path rather than relying on mouse dismissal.
- 2026-09-10: Side-chat stream audit found that `/btw` had no EventSource
  error callback, leaving a dropped secondary stream unexplained. The client
  now surfaces a reconnecting notice while retaining browser-managed retry.
  Client typecheck, web build, UI acceptance, and diff checks pass.
- 2026-09-10: Follow-up found that a permanently closed side-chat EventSource
  could remain registered and block future reopening. Closed side streams now
  leave the registry, cancel on visibility changes, and receive a guarded
  replacement after backoff. Client typecheck, web build, UI acceptance, and
  diff checks pass.
- 2026-09-10: Everyday context-rail audit confirmed Files, Notes, and Next
  steps are actionable buttons mapped to the canonical Editor, Workbench, and
  Commitments dock tabs. The UI acceptance gate now asserts those labels and
  the shared dock-navigation handler, plus the Admin doctor focus trap;
  acceptance and diff checks pass.
- 2026-09-10: Configuration ownership sweep fixed read-after-write ordering
  for Admin MCP server edits, memory add/forget, workspace naming, gateway
  workspace selection, Event Bus credentials, provider-key saves, and the
  generic effective/layer settings guard. These paths now read back before
  success is announced. Admin check/build, server embedded-asset tests, UI
  acceptance, and diff checks pass.
- 2026-09-10: Session/admin lifecycle sweep fixed read-after-write ordering
  for archive/unarchive, session deletion, bulk archived deletion, Best-of-N
  keep/discard, subagent stop, and checkpoint restore. Their visible lists or
  evidence views now refresh before success is shown. Admin check/build,
  embedded asset tests, UI acceptance, and diff checks pass.
- 2026-09-10: Gateway ownership sweep fixed read-after-write ordering for
  allowlist approval/denial, bot defaults and credentials, bot creation,
  and manual channel binding registration. Gateway projections now refresh
  before success is announced. Admin check/build, embedded asset tests, UI
  acceptance, and diff checks pass.
- 2026-09-10: Remaining gateway editor sweep fixed the generic binding action
  helper and channel policy editor so binding mutations read back their
  projection before success. Admin check/build, UI acceptance, and diff checks
  pass.
- 2026-09-10: Final detached-refresh sweep fixed memory amendment, inbox
  acknowledgement, provider-key revocation, service activation, and approval
  forwarding. These mutations now refresh the visible projection before
  success or completion state is presented. Admin check/build, embedded asset
  tests, UI acceptance, and diff checks pass.
- 2026-09-10: The lightweight UI acceptance gate now protects the secondary
  Advanced side-chat stream lifecycle as well: it asserts the visible lost-
  connection message, EventSource error callback, registry cleanup, and
  guarded reconnect timer. The acceptance gate and diff checks pass.
- 2026-09-10: Cross-surface presentation audit completed. The client rich
  renderer consumes the shared schema-v2 `OutputTimeline` snapshots, while
  Admin intentionally presents session ledger, receipts, operations, and
  configuration evidence rather than duplicating rich rendering. The Rust
  presentation contract passed all 22 unit tests; server presentation
  filtering ran cleanly (no matching unit tests). Presentation SSE remains
  snapshot-authoritative on reconnect; durable historical presentation-delta
  replay is still an explicit open item, not silently counted as complete.
- 2026-09-10: Extended `server_smoke.py` to reconnect the presentation SSE
  after a completed run and validate the returned cursor, schema-v2 timeline,
  session id, and item array. The rebuilt release binary passed the complete
  smoke flow, including the new presentation reconnect assertion.
- 2026-09-10: Native parity audit found the Tauri bundle stale after recent
  client edits; rebuilding both `dist/` and `dist-web/` repaired the shipped
  artifacts. `npm run build` and `cargo check -p vak-desktop` now pass, and
  the desktop freshness guard is satisfied again. The build still reports
  expected large deferred renderer chunks and an existing Vite dynamic/static
  import warning; neither blocks the native build.
- 2026-09-10: Everyday context rail state audit found loading, empty, and
  optional-projection failure were previously indistinguishable. Added an
  explicit presentation-error signal and rendered accessible loading and
  unavailable states while preserving the last-known projection on optional
  read failure. Client typecheck, both artifact builds, UI acceptance, and
  diff checks pass.
- 2026-09-10: Fresh embedded-server accessibility verification at 390×844
  found the Advanced dock's visible tab labels were hidden by mobile CSS and
  had no accessible names. Added explicit `aria-label` values for the primary
  and overflow dock tabs, rebuilt the embedded client, and verified Activity,
  Changes, and Terminal in the accessibility tree. The acceptance gate now
  asserts both label paths; it and diff checks pass.
- 2026-09-10: Rendered Everyday verification found a no-active-task edge case
  where `null === null` made the rail report perpetual loading. The loading
  predicate now requires a real active task before comparing hydration ids.
  Client typecheck, both artifact builds, UI acceptance, and diff checks pass.
- 2026-09-10: Rebuilt the release server after the rail fix and repeated the
  authenticated 390×844 browser check against the freshly embedded artifact.
  With no active task, the rail now reaches its genuine empty state and keeps
  Files, Notes, and Next steps accessible; the loading message is gone.
- 2026-09-10: Full `cargo test --workspace` regression checkpoint passed with
  all executed workspace tests green, including server HTTP/SSE lifecycle,
  Admin endpoint coverage, client bundle freshness, presentation/delivery,
  permission/security, desktop, and end-to-end workflow suites. Nine tests
  requiring external NATS were correctly ignored. The first attempt was
  interrupted because a PTY made an intentional non-interactive confirmation
  test wait for input; the clean non-PTY rerun passed that test and the full
  suite.
- 2026-09-10: Fresh 390×844 browser inspection found the Advanced overflow
  disclosure also lost its accessible name when mobile CSS hid its visible
  label. Added `More workspace views`, asserted it in the acceptance gate,
  and verified client typecheck, web artifact rebuild, acceptance, and diff
  checks. The Tauri artifact was rebuilt as well and `cargo check -p
  vak-desktop` passed.
- 2026-09-10: The subsequent full workspace regression completed with all
  executed tests passing. A PTY-based invocation had appeared to hang only
  because the install confirmation test correctly waits when stdin is a
  terminal; the non-PTY invocation exercised its refusal path and passed the
  complete suite.
- 2026-09-10: Fresh authenticated Admin browser audit at 390×844 verified the
  canonical navigation, explicit Global/Workspace scope selector, real health
  and queue evidence, and visible setup/configuration actions. Direct DOM
  measurement reported body width equal to viewport width (390/390), with no
  horizontal overflow; switching the scope selector to Workspace preserved
  the explicit scope wording.
- 2026-09-10: After rebuilding the Admin bundle and release server, the direct
  `#/feeds` route was verified against the current embedded artifact. The DOM
  reports Knowledge active with Memory, Feeds, and Search sub-links, and Feeds
  is marked active; the previous accessibility snapshot omission was a tree
  truncation/representation issue, not missing rendered navigation.
- 2026-09-10: The exact current release binary passed the provider-local HTTP/SSE
  smoke flow. It verified unauthenticated rejection, session admission,
  presentation cursoring, run completion, event-stream resume, and presentation
  reconnect with a schema-v2 snapshot containing the expected session and item
  data; the full loop transcript remained intact.
- 2026-09-10: Added a scoped `Retry details` action to the Everyday context
  rail's presentation-unavailable state, reusing the existing session hydration
  path and avoiding a misleading permanent error. Client typecheck, Tauri/web
  production builds, UI acceptance, diff checks, and a rebuilt release binary's
  HTTP/SSE/presentation reconnect smoke all pass.
- 2026-09-10: Native parity checkpoint completed against the refreshed client
  bundle: `cargo tauri build --no-bundle` and `cargo check -p vak-desktop`
  completed successfully. This verifies desktop compilation and host wiring;
  OS-level dialog, PTY, notification, and tray behavior still require an actual
  desktop runtime rather than being inferred from compilation.
- 2026-09-10: Configuration/accessibility audit found four Admin Voice model
  inputs whose visual labels were not programmatically associated. Added stable
  label/input associations for provider, transcription, synthesis, and realtime
  models. The acceptance harness now checks the Admin path explicitly; its
  missing `ADMIN` variable was fixed as part of the harness correction. Admin
  production build, UI acceptance, and diff checks pass.
- 2026-09-10: Extended the acceptance assertions to cover all four Voice label
  and input IDs, not only the initially found provider field. The Admin bundle
  was rebuilt after the complete association fix; production build, acceptance,
  and diff checks pass.
- 2026-09-10: The isolated macOS end-to-end harness passed. It verified clean
  install behavior, setup refusal and completion, CLI↔HTTP projection parity,
  authenticated onboarding/Admin serving, first-task ReadOnly capping from a
  FullAccess workspace, MCP registration/configuration, update preservation,
  app bundle architecture/seal/executable parity, DMG contents, and a genuine
  purge cleanup. No real provider model or external channel credentials were
  used.
- 2026-09-10: Re-audited presentation replay against the live server contract.
  The bounded event ring can replay live gaps, while historical presentation
  deltas have no durable ledger and therefore remain intentionally unavailable;
  reconnect uses an authoritative snapshot plus an explicit resync diagnostic
  rather than fabricated deltas. Existing event-ring tests (4/4) and the
  provider-local presentation reconnect smoke both pass, so this remains an
  explicit architectural follow-up rather than an untested failure.
- 2026-09-10: Compound offline regression initially exposed a machine setup
  prerequisite: no skills existed at the product's conventional macOS global
  path. Rerunning with the repository's bundled skill source
  completed all 10 cases successfully: skills validation, deterministic eval,
  feed concurrency/load, hooks/MCP, plugin lifecycle/runtime, memory
  add/amend/forget provenance, scheduler/heartbeat/learning/MCP endpoints, and
  the scenario matrix. The original failure did not indicate a product defect.
- 2026-09-10: Added `--skills-root` to `scripts/compound_regression.py`, making
  the global-skill prerequisite explicit and independent of `VAK_HOME`. Python
  syntax validation and a full 10-case offline run with that option passed.
- 2026-09-10: The upgrade gate initially reported lost permission only because
  its fixture omitted the required provider/model answers and setup exited
  before persisting posture. The fixture now supplies the explicit offline-safe
  Ollama route; the full gate passes, including settled posture/capabilities,
  edited-seed byte preservation, registry state verification, and downgrade
  readability. This proves install/update preservation for the current seed
  generation, but not a cross-version changed-seed delta (still open because
  the current seed implementation now records digest metadata, while a
  cross-version changed-seed fixture remains useful future coverage.
- 2026-09-10: Implemented Shared skill seed provenance with
  `.vak/.seed-manifest.json`. New standard skills are recorded by SHA-256;
  unchanged shipped bytes advance on future seed runs, while edited or
  previously untracked files are preserved. Added the manifest to the durable
  state registry and a focused regression covering all standard skills plus an
  operator edit. Cargo-format, `vak-core` clippy, the focused test, and the
  upgrade gate pass. Starter plugin seed provenance/update remains to be wired
  through the same contract.
- 2026-09-10: Extended the seed manifest contract to the two standard starter
  plugins. A plugin advances only when its installed digest matches the prior
  Vak-shipped digest; an independently installed or modified package is left
  untouched. The manifest now records six standard skills plus two plugins,
  and the focused seed regression, `vak-core` clippy, formatting, and diff
  checks pass.
- 2026-09-10: Rebuilt and reran the installed-binary upgrade gate after the
  plugin provenance change. It passed state preservation, settled setup state,
  edited skill preservation, downgrade readability, and registry verification.
- 2026-09-10: Tightened starter-plugin updates to verify the installed package
  directory itself still matches the recorded registry digest before updating.
  Directly edited plugin packages are now preserved instead of being silently
  replaced. The focused seed regression covers both edited skills and an edited
  starter plugin; `vak-core` tests, Clippy, formatting, and diff checks pass.
- 2026-09-10: Seed failures are no longer silently reported as successful.
  Shared seeding now returns an explicit error; setup and service activation
  fail closed, the onboarding API returns HTTP 500, and doctor records a
  diagnostic instead of claiming cleanup completed. Focused `vak-core`/
  `vak-server` Clippy, seed regression, formatting, and diff checks pass.
### Seed/update contract follow-up (2026-09-10)

- `seed_shared_capabilities()` now returns an explicit `Result`; setup, doctor,
  services-sync, and onboarding surface failures instead of silently treating a
  partial seed as success.
- Standard skills and starter plugins use `.vak/.seed-manifest.json` to add new
  content and advance untouched shipped content while preserving edited or
  independently installed content. Hooks and plugin network defaults remain
  one-time empty-layer defaults by design.
- The release update path now reconciles Shared capabilities after a real
  upgrade and on a non-dry-run up-to-date check, so standard seeds are not
  stranded behind the install path.
- Fresh client web and Tauri builds, the admin production build, and the UI
  acceptance hook gate pass. The client build still reports large deferred
  renderer chunks (Mermaid/Cynefin/WASM/C++); these remain a performance
  follow-up rather than an initial-entry regression.
- The isolated `scripts/macos-check.sh` run passes against the current build:
  install/setup, onboarding/admin authentication, first-task ReadOnly capping,
  MCP configuration, same-version update state preservation, app-bundle and
  DMG verification, and purge all complete. This is host-lifecycle evidence;
  interactive desktop/Tauri task-pane coverage remains open.
- The Advanced dock now exposes a named `complementary` region for the active
  pane (Activity, Changes, Terminal, or another workspace view), improving
  screen-reader orientation beyond the existing labelled controls. Client
  TypeScript verification passes.
- Fresh embedded-server browser verification now confirms the shipped artifact:
  switching to Advanced exposes `Advanced workspace: Changes`, closing it and
  returning to Everyday restores the context rail, and the rail's `Open Notes`
  action opens the Workbench pane while retaining the task shell.
- A fresh browser session first exposed the expected “Open a workspace” gate;
  after the normal workspace became ready, the live Advanced matrix switched
  Changes to `diff` and Terminal to `terminal` through the dock, with the task
  shell preserved. This separates onboarding gating from pane behavior.
- Full Chromium accessibility-tree verification exposes the dock as
  `complementary "Advanced workspace: Activity"`, with Activity, Changes,
  Terminal, and the More workspace disclosure beneath it. This closes the
  headless screen-reader structure check for the Advanced dock.
- Fresh embedded-server Admin browser verification confirms Global scope,
  Knowledge expansion to Memory/Feeds/Search, accessible Memory governance
  controls, and the Global→Workspace scope transition (`project` wire value)
  while preserving the `#/memory` deep link.
- A fresh authenticated browser pass at 390×844 reports body width equal to
  the viewport in both Everyday and Advanced, with the Everyday rail and
  Advanced dock rendered. Direct `/admin#/feeds` and `/admin#/search` routes
  render their intended screens, controls, role filtering, scope selector, and
  canonical navigation shell.
- A live keyboard pass at 390×844 reaches the Advanced mode controls and the
  permission selector; the selector has an intentional focused border/background
  treatment. Full screen-reader announcement testing remains outside the
  available headless browser evidence.
- A deterministic live browser event check confirms the rendered status region
  transitions `Connection: Live` → `Connection: Offline` → `Connection:
  Reconnecting` when the same page-level events used by the application are
  dispatched. Browser transport emulation itself did not dispatch those events,
  so that separate behavior remains unclaimed.
- Fresh `scripts/server_smoke.py` execution against `target/debug/vak` passes
  authentication rejection, session/run creation, event-stream cursor resume,
  terminal event delivery, full transcript count, and schema-v2 presentation
  reconnect with an authoritative snapshot cursor.
- Fresh `scripts/upgrade-gate.sh` passes after update-path reconciliation:
  declared state survives, setup remains settled, edited seed bytes remain
  identical, and the previous build can read the updated state.
- Native Tauri verification initially caught `crates/vak-client-ui/dist` stale
  relative to `src/App.tsx`; rebuilding the Tauri client bundle corrected the
  shipped-artifact mismatch, and `cargo tauri build --no-bundle` then passed,
  producing `target/release/vak-desktop`.
- Fresh embedded-server browser performance measurement records 71 ms
  DOMContentLoaded, 73 ms load, 119,304 bytes transferred for initial script/
  stylesheet resources, and a rendered Advanced dock by approximately 716 ms.
  Deferred heavy renderer chunks remain outside the initial request set.
- A stale design-contract row claiming update seed deltas were “offered, never
  applied” was corrected to describe the implemented digest-gated behavior.
  The stale-contract sweep and UI acceptance gate now pass.
- The UI acceptance gate now checks both generated client bundles (`dist` and
  `dist-web`) for the Advanced dock contract, preventing a source-only pass from
  hiding a stale shipped artifact. The strengthened gate passes.
- Seed reconciliation documentation now matches behavior: setup and update use
  the same Shared capability path. Retired-plugin removal and its network
  allowlist cleanup now fail the seed operation if either write cannot complete,
  instead of reporting success while stale capability state remains. The focused
  `vak-core` seed regression passes after this hardening.
- Approval actions in the client transcript and persistent-rule confirmation
  dialog now explicitly declare `type="button"`, preventing accidental form
  submission in embedded/native hosts. Client typecheck, web/Tauri bundle builds,
  and the UI acceptance gate pass after the change.
- Extended the same explicit-button contract to shared task controls in ChatPane
  and Sidebar (pause/cancel/change-plan, expandable tool rows, latest-scroll,
  task navigation, archive/history/delete, workspace list actions, and Settings).
  Client typecheck and the UI acceptance gate pass after the sweep.
- Focus-trap audit found loading/empty dialogs could receive no initial focus:
  the shared directive called `.focus()` on a non-focusable dialog root. It now
  temporarily supplies `tabindex="-1"` when needed and removes it on teardown,
  preserving focus restoration. Client typecheck and the UI acceptance gate pass.
- The shared Settings shell and scope controls now explicitly use
  `type="button"`, including the reusable capability switch and archived-task
  navigation. This prevents nested-host form submission while preserving their
  switch/pressed semantics. Client typecheck passes.
- Extended explicit button semantics to the Everyday reminders modal and the
  Advanced managed-work modal: run/diff/pause/delete/create/cancel/close/retry/
  resume actions can no longer submit an enclosing host form. Client typecheck
  and the UI acceptance gate pass.
- Advanced managed-work now distinguishes a failed ledger read from “no managed
  work” and exposes a retry action; retry/resume command failures render an
  explicit alert instead of becoming unhandled promise failures. Client
  typecheck and the UI acceptance gate pass.
- Advanced Diff comment submission now catches transport/authorization failures
  and keeps the visible diff intact while showing an inline alert. Refresh,
  review, and comment-send controls also use explicit non-submit button types.
  Client typecheck and the UI acceptance gate pass.
- Refreshed both shipped client hosts after the latest modal and Diff changes:
  `npm run build:web` and `npm run build:tauri` pass. The acceptance gate and
  `git diff --check` also pass; the existing large deferred-renderer chunk
  warning remains documented and outside the initial Everyday payload.
- Advanced Preview component/dev-server controls now explicitly use
  `type="button"` across both tabs (view switch, logs, refresh, reload, popout,
  and clear). Both web and Tauri client builds pass, followed by the UI
  acceptance gate and `git diff --check`.
- Preview server startup now stops immediately when the authoritative start
  response contains an error; it no longer refreshes stale state and presents a
  previously running server as newly started. Both client host builds pass,
  followed by the UI acceptance gate and `git diff --check`.
- Shared presentation outcome-review actions now catch persistence failures and
  show an inline alert without discarding the rendered result. Both client host
  builds pass, followed by the UI acceptance gate and `git diff --check`; the
  existing deferred-renderer chunk warnings remain unchanged.
- Admin Operations Center live-work, approval, navigation, and automation
  controls now explicitly declare `type="button"`, preventing accidental host
  form submission from changing operational state. Admin production build, UI
  acceptance gate, and `git diff --check` pass.
- Strengthened `scripts/ui-acceptance-check.sh` to assert the managed-work
  unavailable/update alerts, Diff comment failure preservation, and focus-trap
  fallback contract. The first run caught an overly escaped assertion; that
  gate assertion was corrected and the acceptance gate plus `git diff --check`
  now pass.
- Aligned semantic presentation approvals with transcript approvals: pending
  gates now expose an assertive live alert and resolved gates a polite status,
  with an atomic accessible label in both Everyday and Advanced renderings.
  Client typecheck, both production host builds, the strengthened acceptance
  gate, and `git diff --check` pass.
- Seed-contract audit found and corrected a stale onboarding reference to the
  retired `crates/vak/src/install/seed.rs`; the standard six-skill/two-plugin
  inventory is now documented against `crates/vak-core/src/seed.rs`. A stale
  path sweep, UI acceptance gate, and `git diff --check` pass.
- Full design-contract sweep confirms `docs/design/59-reference-ui-acceptance.md`
  still requires a real Everyday “My tasks” destination, while the shipped
  Sidebar deliberately removed the prior unsupported duplicate. This remains a
  genuine navigation gap requiring product definition (task-history view,
  scheduled-task view, or another canonical destination); no misleading alias
  was added.

## Stabilization checkpoint — 2026-09-10

This review is intentionally paused at a clean incremental checkpoint while
the weekly work budget is nearly exhausted. The current source and generated
web/Tauri/Admin artifacts pass `git diff --check` and
`scripts/ui-acceptance-check.sh`. No broad refactor or unverified release claim
is being made from this checkpoint.

Deferred for a later capacity window: full screen-reader and interactive Tauri
matrix; authenticated desktop/browser lifecycle matrix; durable historical
presentation-delta replay; complete cross-surface fixtures; measured interaction
latency across every panel; and final clean-release verification. The open
Everyday “My tasks” contract needs the product destination decision recorded
above. Generated bundle churn is expected from the refreshed shipped artifacts;
unrelated user changes remain untouched.
