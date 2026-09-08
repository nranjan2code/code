# Changelog

**Supported history begins at 2.0.0.** Every version before it is
unsupported and cannot be upgraded in place — see
`docs/design/46-stabilization-install-and-onboarding.md` Part VII.1. Entries
for those releases were removed from this file; `git log` holds them.

## 3.0.31 — 2026-09-08

### Rich Modern Terminal Surface (`vak term`)

- **Ultra-Dense Cinematic Command Cockpit.** Greenfield interactive console surface in `crates/vak-terminal`, invoked via `vak term` or `Surface::Terminal`. Preserves the headless Unix-composable CLI (`vak exec`, `vak plan`, etc.) intact.
- **Screen 1: Dual-Deck Studio (60/40).** Left deck with 24-bit TrueColor halfblock wireframe (`▀▄█`), external browser launcher (`[o]`), collapsible markdown card (`Space`), and live bash worker card with animated Braille spinner (`⠋⠙⠹`). Right HUD with CPU/RSS gauges, 360° rotating radar sweep (`6°/tick`), token burn-rate waveform, and git diff inspector.
- **Screen 2: Remote Observability & Operations.** Top KPI bar (FinOps spend with budget cap meter, 100% Healthy circuit breaker, 3/3 online services, DLQ). Left deck with real-time incident feed, audit receipts with SHA-256 fingerprints, and Merkle causal DAG. Right HUD with network traffic waveform and provider latency meters.
- **Screen 3: Settings & Remote Admin.** CorePool multi-tenant directory tree displaying Shared Base Layer and active workspace, interactive security engine (`WorkspaceWrite`, `ReadOnly`, `FullAccess`, `Ask`, `AutoApprove`), MCP server inventory (`tavily`, `docker`, `github`), bot gateways, and pending authorization queue with live `[Approve]` and `[Deny]` action states.
- **Screen 4: Attention Inbox & Memory.** Prioritized attention inbox with dynamic unread badge, budget alert ack (`[a]`), watchdog service recovery (`[r]`), self-evolved skill candidate queue with promotion (`[p]`), and cron automation countdown timers.
- **Human-in-the-Loop (HIL) Dialog.** Floating modal with in-place interactive shell command editor (`[e]`), risk assessment, execution directory, cost impact, and fast decision keys (`[y]`, `[a]`, `[d]`, `[Esc]`).
- **5 TrueColor Themes & Minimalist Unicode.** Vak Warm, Vak Slate, Vak Paper, Vak Contrast, and Tokyo Night cyclable via `F2` or `/theme`. Strict geometric Unicode glyphs with zero emojis workspace-wide.

## 3.0.30 — 2026-09-08

### Intent-Driven Active Skill Inlining & Progressive Disclosure

- **Zero-Round-Trip Active Skill Inlining.** Transformed skill handling from voluntary
  tool-calling ceremony to intent-driven active context inlining. Surviving skills with
  the highest domain affinity to the turn (e.g. `software-development` for code authoring/modification)
  are automatically promoted to active status and inlined into the system prompt under
  `## Active Skill Guidelines` on Turn 1.
- **Progressive Skill Disclosure.** Remaining catalog skills are concisely indexed for on-demand
  loading via `skill({"name": "..."})`, preventing context window bloat while ensuring
  specialized procedures remain accessible.
- **Starter Skill Domain Classification.** Added domain classifications for built-in starter skills
  (`software-development`, `debugging`, `code-review`, `data-and-spreadsheets`, etc.) in
  `crates/vak-core/src/capability/provider.rs`.
- **Full React Benchmark Verified Across Models.** Validated full production Vite + React builds
  with `npm install` and `npm run build` across both frontier (`gpt-5.6-luna`) and local small
  models (`gemma4:e2b-mlx` 2B) in both single-agent and multi-agent subagent configurations.

## 3.0.29 — 2026-09-08

### Intent Pre-Flight HUD & Multi-Turn Context

- **Lossless Realtime Intent Pre-Flight HUD.** Upgraded Composer Intent Pre-Flight
  HUD to expose complete 6-axis commitment projections (`Risk`, `Pacing`,
  `Scope`, `Autonomy`, `Budget`, `Verification`), capability bounds, and raw
  signal weight telemetry.
- **Session-Grounded History Facts.** Connected real-time intent explanation to
  active session state via `session_id`, supplying multi-turn conversation
  depth, previous conversational acts, and active commitment status to the
  deterministic intent classification kernel.
- **Strict Theme Adaptation.** Full compliance with Vak core theme tokens
  across Light, Dark, Warm, and High Contrast palettes with zero hardcoded
  colors and native collapse/expand HUD controls.

## 3.0.28 — 2026-09-08

### Admin Console Bus + Sandbox Coverage

- Add Operations Center Bus status panel (backend, connection, encryption,
  metrics grid with byte formatting).
- Add `#/operations/sandbox` section with Environments, Candidates (with
  Promote-to-workspace action), and Promotion ledger sub-views.
- Add Event bus panel to Settings with NATS URL, JWT, NKey seed, and
  workspace secret env var fields; credential-never-returned semantics.
- Show per-session sandbox execution events in RunDetail.
- Add sandbox topology node to the system map inspector.

## 3.0.27 — 2026-09-08

### Fresh Startup & Workspace Panel Controls

- Start web and desktop with a fresh task canvas instead of reopening the
  latest persisted task.
- Keep the workspace More menus visible above the header and add a shared
  right-panel expand/collapse control.

## 3.0.26 — 2026-09-08

### Sandbox Acceptance & Documentation

- **Sandbox crate unifies execution backends.** Added `crates/vak-sandbox`
  as a workspace member, consolidating the Seatbelt (macOS) and Landlock
  (Linux) sandbox implementations behind a single trait surface used by
  the broker, the execution engine, and the runtime permission model.
- **Full-stack release artifact matrix.** Release and install runbooks
  (`docs/release-and-install.md`, `docs/design/32-release-engineering.md`)
  verified across the macOS native bundle (`.app` + DMG), the headless
  Linux Docker image, and the web surface, with end-to-end checks on both
  platforms.

## 3.0.25 — 2026-09-08

### Retired Tool Lifecycle & Plugin Cleanup

- **Retired tools registry.** Added `crates/vak-tools/src/retired.rs` — a
  compiled-in, append-only list of tool names that no longer exist
  (`python_eval`, `react_preview`). Each entry carries the retirement
  version and the replacement instruction (`bash`). This is the single
  source of truth for tool retirements; there is no second list.
- **Plugin retirement detection.** `PluginStore::retired_plugins()` scans
  installed plugin packages for `SKILL.md` files referencing retired tool
  names. A startup warning fires in `Core::new_with_trust`; `vak doctor`
  flags them as a failed check; `vak doctor --repair` and `vak setup seed`
  remove them automatically.
- **Skill validation at discovery.** `skills::validate()` rejects any
  `SKILL.md` whose description or body text backtick-references a retired
  tool name, so stale skills never enter the model's capability contract.
- **Tool name aliasing.** `canonical_tool_name()` redirects retired tool
  names to `bash`, so a model that still reaches for `python_eval` gets a
  `Correctable` error (missing `command` arg) instead of `unknown_capability`
  and can self-repair.
- **Config scrubbing.** `vak_config::prune_plugins_network_allow()` removes
  stale entries from `[plugins] network_allow` when a retired plugin is
  removed.
- **Admin API.** `GET /plugins/retired` lists retired plugins;
  `DELETE /plugins/retired` removes them all and prunes config.
- **Local install remediation.** Removed `python-sandbox` and
  `react-sandbox` plugin packages (v1.0.0 and v1.0.1) that survived the
  3.0.21 sandbox unification and still instructed the model to call
  `python_eval`/`react_preview`. Fixed `permission_mode` from `full-access`
  to `workspace-write` to restore sandbox enforcement.

## 3.0.24 — 2026-09-08

### Sandboxed Broker Wrapping, Env Scrub Security & Auto-Approval Verification

- **Brokered execution command wrapping for containerized sandboxes.**
  Added transparent command wrapping for `SandboxTarget::ToolCommand` targets in `crates/vak-tools/src/broker.rs`. Host-level workers remain protocol adapters while bash commands inside tool arguments are securely encapsulated for Docker-style execution sandboxes.
- **Defense-in-depth subprocess environment scrubbing.**
  Hardened operational variable allowlist predicate in `crates/vak-tools/src/bash.rs`. Verified complete rejection of provider credentials, cloud API tokens, proxy credentials, and authentication keys, backed by comprehensive end-to-end unit tests.
- **Fail-closed DenySandbox and Seatbelt read-only traits.**
  Verified `DenySandbox` exit 126 error reporting with full command redaction, and enforced invariant that `read_only_variant()` on Seatbelt profiles strips all `file-write*` entitlements.
- **Comprehensive auto-approval policy matrix verification.**
  Implemented regression tests in `crates/vak-agent` proving that `AskSource::Rule` and `AskSource::CircuitBreaker` cannot be bypassed by `AutoApprove`, and that `ApproveSafe` requires active container or Seatbelt sandbox enforcement before authorizing bash execution.

## 3.0.23 — 2026-09-07

### Distributed Event & Message Fabric (`crates/vak-bus`, Doc 53)

- **Green-field distributed event fabric for multi-agent fleets.**
  Added `crates/vak-bus` providing high-throughput, zero-trust event distribution and durable
  asynchronous work queues for thousands of concurrent agents across local, headless Linux, hybrid,
  and multi-cloud environments.
- **Universal CloudEvents 1.0 with Merkle Causal Lineage & W3C Tracing.**
  Standard CloudEvents 1.0 envelope (`id`, `source`, `type`, `time`, `subject`, `data`) augmented with
  W3C Distributed Tracing (`traceparent`, `tracestate`, `child_span`) and SHA-256 Merkle causal hash chaining
  for cryptographic provenance and causal verification across multi-agent workflows.
- **Defense-in-depth zero-trust envelope security.**
  End-to-end authenticated AES-256-GCM encryption with HKDF-SHA256 key derivation and 12-byte random nonces.
  Binds event ID, type, workspace, and sequence via Authenticated Additional Data (AAD), failing fast on tampering.
- **Deterministic hierarchical subject algebra & least-privilege ACLs.**
  Structured taxonomy for broadcast events (`vak.events.<ws>.<sess>.*`), durable work queues
  (`vak.work.<ws>.<role>.task`), direct inboxes (`vak.agent.<ws>.<agent>.inbox`), and dead letters
  (`vak.dlq.<ws>.<agent>.failed`). Governed by role-based `AclPolicy` rules.
- **Lock-free telemetry, W3C context propagation & Dead-Letter Queues.**
  Atomic performance counters (`BusMetrics`), automated cross-process W3C trace propagation, and
  dead-letter event routing with poison-pill quarantine and diagnostic logs.
- **Production NATS Core + JetStream engine and zero-dependency InMemoryBus.**
  Native asynchronous `NatsBus` with pull-consumer work claiming, ACK/NAK semantics, alongside a
  real, zero-dependency `InMemoryBus` for local development and CI testing. Zero mocks or stubs.

## 3.0.22 — 2026-09-07

### 2026 State-of-the-Art Sandboxed Execution Engine across CLI, Desktop, and Headless Linux

- **High-fidelity ANSI terminal & progress bar streaming.**
  Added streaming line folding for carriage returns (`\r`) in `crates/vak-tools` and `crates/vak` so
  progress bars (`npm`, `pip`, `cargo`, `docker`, `curl`) update smoothly in-place without log noise.
  Implemented a lightweight 16-color ANSI-to-HTML parser in the SolidJS Workbench panel with auto-scrolling
  to bottom during streaming.
- **Sub-second real-time process telemetry.**
  Added `SandboxEvent::ProcessTelemetry` emitting elapsed execution time (ms) and resident set size (RSS)
  memory every 500ms during execution. Probes `/proc/<pid>/statm` directly on Linux and Docker containers,
  and queries `ps -o rss=` on macOS. Displayed live as RAM and timer badges in the Workbench header and CLI
  execution box footer.
- **Quarantined scratch artifact auto-detection & in-situ preview.**
  Intermediate scripts, assets, and data files are isolated in `.vak/scratch/`. Automatically detects newly
  generated scratch files upon command completion, classifies MIME types, and provides in-situ visualizers:
  sandboxed HTML/web apps in isolated `<iframe>` with `srcdoc`, image inspection with click-to-zoom modal,
  and syntax-formatted code and data viewers.
- **Interactive process controls.**
  Added one-click "Stop Process" in Desktop and Web Workbench calling `api.cancelRun()` to cleanly terminate
  commands via process group signaling (`SIGTERM` -> `SIGKILL`), along with one-click "Copy Output".
- **Cross-surface parity.**
  Full feature parity verified across CLI terminal box, Desktop App (`/Applications/Vak.app`), Web UI,
  and Headless Linux on Docker (`vak:local`).

## 3.0.21 — 2026-09-07

### Unified Bash Sandbox with Workbench Live Observability

- **Unified neutral `bash` sandbox execution.** Replaced fragmented language-specific runners
  (`python_eval`, `react_preview`) with a single universal, stack-neutral `bash` execution engine.
  Any programming stack (Python, Node/TypeScript, Rust, Go, shell scripts) can be run and installed directly.
- **Real-time execution streaming in Workbench.** Added live event streaming (`SandboxEvent::Stdout`,
  `SandboxEvent::Stderr`, `ExecutionStarted`, `ExecutionFinished`) to `vak-tools` and `vak-agent`,
  broadcasting real-time execution telemetry directly to the client Workbench panel.
- **Package installation tracking & scratch isolation.** Automatically tracks package installations
  (`pip`, `npm`, `cargo`) and provides complete visibility into execution lifecycle, duration, and
  quarantined intermediate artifacts in `.vak/scratch/`.
- **Complete legacy cleanup.** Thoroughly removed obsolete tools, runtimes, plugin seeds, and
  presentation adapters across permissions, config, core, delivery, server, and client UI.

## 3.0.20 — 2026-09-07

### Sandbox package installation robustness & Docker build reproducibility

- **Default sandbox network seeding for package installation.**
  `seed_shared_capabilities` now automatically seeds `[plugins] network_allow = ["python-sandbox", "react-sandbox"]`
  into the Shared layer (`~/vak-home/.vak/config.toml`) via `seed_plugins_network_allow_if_empty`
  and `seed_global_plugins_network_allow_if_empty` in `vak-config`. The LLM can dynamically install
  any required Python packages (via `python_eval`'s `install_packages`) and load React preview assets
  out-of-the-box without manual configuration, while preserving strict operator governance via `network_deny`
  and channel policies.
- **Robust pip argument quoting and diagnostics.**
  `python_runner` now properly shell-quotes each package argument in `pip install` individually
  (handling specifiers like `pandas>=2.0` without shell redirects) and falls back to stdout if stderr is
  empty to guarantee complete diagnostic reporting.
- **Reproducible Docker container builds.**
  The Dockerfile cargo build now enforces `--locked` dependency resolution and receives `VAK_GIT_SHA`
  as a build argument, guaranteeing container binaries match the workspace lockfile and carry commit provenance.

## 3.0.19 — 2026-09-07

### Connected sandbox runtimes: python_eval and react_preview

- **Connected write→execute→debug→result loop.** The system prompt now
  establishes `python_eval` and `react_preview` as first-class sandbox
  runtimes with an explicit write→execute→debug→result cycle, and documents
  the shared scratch-space connection between them: Python processes data and
  generates figures, and React components consume that data via props or
  scratch files to render interactive visualizers inside the chat and the
  Preview Dock.
- **Dual-artifact scratch management for react_preview.** Components are
  written as `.tsx` source and compiled into an isolated `.html` preview under
  `.vak/scratch/previews/`, with `component_path` support. Window `error` and
  `unhandledrejection` diagnostics are injected into the preview HTML so
  client-side failures surface inside the sandbox instead of vanishing into a
  blank frame.
- **Headless Python + Matplotlib.** `python_eval` runs headless (Agg) and
  auto-captures figures into `.vak/scratch/python/`; `python3 -m pip install`
  is supported with cross-platform package management, and `ModuleNotFoundError`
  now carries targeted debug hints. All artifacts are quarantined to
  `.vak/scratch/python/` and never contaminate the project source tree.
- **Quarantined, scrubbed-execution runtimes.** Both runtimes execute in
  disconnected process groups with fully scrubbed environments (`env_clear`),
  zero parent API keys or secrets leaked, and a strict CSP
  (`connect-src 'none'`) on previews — `pip install` is network-gated by the
  same privileged network model as MCP (invariant 35).
- **Broker-level tool normalization + canonical resolution.** `vak-agent`
  normalizes tool calls through a single normalization seam, and `vak-tools`
  resolves built-in runtime tool names canonically so the tool vector, the
  derived name packet, and the per-channel filter all describe the same
  executable surface (invariant 30).
- **Permission + delivery signals.** Runtime admission now carries
  permission descriptions in `vak-permission` and the corresponding
  presentation signals in `vak-delivery`.

## 3.0.18 — 2026-09-07

### Plugin runtime admission unifies on the store, and network knobs speak one language

- **One admission decision point, one tool set.** `built_in_runtime_plugin_tools`
  is now the single place the built-in `python_eval`/`react_preview` runtime
  tools are decided; the plugin-tier and tool-name-tier filter sets agree, so
  the tool vector, the derived name packet, and the per-channel filter all
  describe the same executable surface (invariant 30).
- **Network is a strict lattice, decided once.** Config-level `network_deny`
  beats `network_allow` (entry wins, `*` wins), an absent allowlist denies by
  default, and the channel overlay can only narrow egress — never grant it.
  The Settings toggle persists its grant atomically and the server refuses a
  non-empty grant for an untrusted project layer.
- **The plugin store owns package lifecycle, and the runtime follows it.**
  Enable/Disable/Remove on a runtime package now changes the executable
  surface, not just the listing: the runtimes live and die with their owning
  plugin. A store that was never written is vacuous (config alone decides),
  so pre-setup homes and synthetic test homes stay deterministic. `[plugins]`
  remains the privileged overlay above.
- **`react.preview` becomes a real typed result.** The preview tool emits a
  semantic envelope, the exact single-render invariant holds (projection emits
  one structured item), and `count` equals the model-visible projection length.

## 3.0.17 — 2026-09-06

### Multi-turn continuity, conversational drift handling, and local context discovery

- **Dynamic `<conversation_thread>` projection**: When a session has multiple turns (`revision > 1`), `derive_messages()` projects the chronological request timeline across turns alongside explicit rules directing the model to follow intent across conversational drifts without complaint or resistance, resolve references against earlier turns, and prohibiting clarification as an exception-handling escape hatch.
- **Historical tool result pruning**: Tool execution results prior to the active turn exceeding 600 characters are safely pruned down to 300 characters plus summary metadata in runtime memory projections, preventing past search dumps and stack traces from crowding out context while keeping on-disk JSONL ledgers 100% verbatim.
- **System prompt operating rules**: Updated core `operating_rules` in `system-prompt.md` to universally enforce drift adaptability, reference resolution, and prohibit using demands for manual input/URLs as an excuse to avoid using tools.
- **Provider model context discovery for Ollama**: Added native `/api/show` model context discovery in `vak-llm` inspecting `num_ctx`, `details.context_length`, and `model_info`. Local models default conservatively to 8,192 tokens instead of 128,000, ensuring context compaction triggers reliably on local hardware.

## 3.0.16 — 2026-09-06

### Sandboxed Python & React runtime plugins with chat presentation and preview dock integration

- **Sandboxed Python execution (`python_eval`)**: Implemented `PythonTool` running in isolated process groups with fully scrubbed environments (`env_clear`, zero parent API keys or secrets leaked). All generated scripts and pip installations are quarantined in `.vak/scratch/python/site-packages/` and never pollute regular project files.
- **Sandboxed React component preview (`react_preview`)**: Implemented `ReactPreviewTool` compiling self-contained, offline HTML preview bundles with React 18, Babel Standalone, and Tailwind CSS. Quarantined in `.vak/scratch/previews/` with strict Content Security Policy (`connect-src 'none'`) and built-in ErrorBoundary.
- **Multi-tier network governance**: Follows the privileged MCP network model (`network = true | false`). When network access is disabled, `pip install` package management is blocked with a typed error and React preview CSP strictly enforces `connect-src 'none'`. Channel overlays (`plugins_network_deny`, `plugins_deny`) enable remote bot lockdown.
- **Shared capability seeding**: Automatically seeds `python-sandbox` and `react-sandbox` plugins along with their skills (`python-execution`, `react-component-preview`) into `~/vak-home/.vak/plugins/packages/` so every new workspace inherits them seamlessly.
- **Interactive chat rendering**: Created `UIPreviewCard` rendering live sandboxed component previews directly inside chat turns with reload, popout, source inspection, and a direct "Right Bar" open action.
- **Dual-mode Right Bar Preview Dock**: Upgraded `PreviewPane` to seamlessly toggle between live sandboxed Component Previews and local dev servers from `.vak/launch.toml`.

## 3.0.15 — 2026-09-06

### Reconfirmation modals, workspace removal safety, and composer layout stability

- **Accidental removal protection with `ConfirmModal`**: Added a dedicated confirmation modal with clear action semantics, danger badges, and explicit assurances that local project files and git commits remain intact on disk before forgetting a workspace, archiving a task, or permanently deleting ledger history.
- **Graceful in-flight cancellation**: Removing a workspace or archiving a task automatically and safely halts any active agent runs in that workspace or session, recording partial outputs cleanly without data corruption.
- **Automatic canvas clearing on removal**: Archiving an active task or removing an active workspace immediately switches the canvas to the next available task or opens a fresh clean session rather than leaving stale chats rendered.
- **Active workspace removal support**: Updated server and desktop host ports to seamlessly switch the active workspace to an available workspace or default home when removing the active directory, clearing both desktop and web recent lists.
- **Workspace and session context menus**: Right-click context menus on sidebar workspace rows ("Switch to workspace", "Copy folder path", "Remove from list") and session rows ("Open task", "View transcript", "Copy task ID", "Archive task", "Delete permanently").
- **Composer layout stability**: Eliminated composer toolbar distortion when sending prompts by enforcing non-wrapping horizontal alignment, moving detail density controls to the status bar (`st-density`), compacting token meters into a tight pill, and morphing the Send button directly into a Stop button within the exact same 28x28 footprint.
- **Discoverable slash palette and skills button**: Added a dedicated `[/] skills` button to the composer toolbar alongside a unified slash palette supporting built-in commands (`/clear`, `/btw`, `/compact`, `/diff`, `/terminal`, `/files`, `/help`) and registered skills with full keyboard navigation.

## 3.0.14 — 2026-09-06

### Composer toolbar alignment and control streamlining

- **Streamlined composer controls**: Removed the standalone spark/goal control from the primary input bar to keep focus on direct turn entry.
- **Precision vertical alignment**: Standardized all bottom toolbar interactive elements (workspace selector, permission mode, model picker, file mention, file attachment, density selector, token counts, context ring, and send button) to a uniform 26px baseline plane with centered flex distribution.
- **Attachment buttons**: Clean, dedicated `@ files` mention and `+ Attach` file picker buttons with consistent styling.

## 3.0.13 — 2026-09-06

### Chat surface UX overhaul and web workspace management

- **Progressive streaming markdown**: Dynamic unclosed code block detection renders headings, bold, and code blocks in real time with debounced Shiki highlighting, preventing post-turn DOM swap and layout reflow.
- **Collapsible tool cards**: Completed tool calls collapse by default to preserve transcript scannability, with full outputs expandable on click.
- **Message action bar**: Floating hover bar provides 1-click response copying with checkmark feedback and prompt editing for user messages.
- **Composer ergonomics**: Prompt history navigation (`↑`/`↓`), multi-file drag-and-drop code attachments, image preview thumbnails, and mobile virtual keyboard Enter handling.
- **Quick model switcher**: Inline dropdown in composer populated dynamically from discovered provider models.
- **Web workspace management**: Added `WorkspacePickerModal` backed by `DirectoryPicker` allowing web users to browse server directories and add/switch workspaces directly from the sidebar.
- **Workspace lifecycle clarity**: Workspace removal is strictly a presentation decision ("forget") that never deletes local files or directories on disk.

## 3.0.12 — 2026-09-06

### System-driven tool-failure recovery (no more silent `produced`)

- Add a typed `ToolErrorKind` classification for tool failures (single
  authority shared by the recovery hint, the repair budget, and the outcome
  gate). Webfetch byte-cap and MCP/mcp broker omissions now classify
  `Correctable`.
- `McpTool` schema is contract-honest: `oneOf` list/call branches with
  `additionalProperties: false`, so the model no longer omits `server`/`tool`
  by trusting an incomplete `required` list.
- The schema validators (`vak-tools` contract and `vak-mcp`) now enforce
  `oneOf` (exactly one branch) and `additionalProperties: false`.
- Per-run repair budget in the agent turn loop: failed attempt -> hint;
  second consecutive correctable failure -> system-authored, schema-resurfacing
  directive; third -> bounded degraded stop (honest "could not repair" answer),
  instead of spinning on a fault or signing off a fabricated result.
- Outcome linkage: a turn that ends with unresolved correctable tool failures
  and no successful receipts is downgraded `Produced` -> `Unknown`.

## 3.0.11 — 2026-09-06

### Provider and runtime contract audit

- Harden provider routing, fallback identity, watchdog cancellation, discovery freshness, child/flow policy propagation, and outcome evaluation evidence.
- Regenerate shipped frontend bundles and add focused regression coverage for the audited runtime paths.

## 3.0.10 — 2026-09-06

### Admin feed workflow UX

- Fix feed source add/edit/delete flows to use stable source identities and
  responsive, keyboard-accessible controls.
- Improve wizard hierarchy, field alignment, mobile behavior, and close/cancel
  affordances.

## 3.0.9 — 2026-09-05

### Scoped feed system

- Add global and workspace feed scopes with stable source identities,
  permission checks, provenance, quarantine, search, alerts, and delivery
  receipts across the server, pipeline, MCP, scheduler, and UIs.
- Rebuild shipped frontend bundles and add feed contract/concurrency coverage.

## 3.0.8 — 2026-09-05

### Harness lifecycle hardening

- Revoke active permission leases immediately when a session's effective mode changes, including warm pooled gateway cores.
- Deny forwarded approval gates during revocation and reject late replies.
- Refuse dollar-capped dispatch for models whose pricing is unknown instead of allowing an unenforceable budget.
- Persist child-run terminal markers and recover completed children into verification after a process restart.
- Recover orphaned commitment episodes, wake commitments after fulfilled dependencies, and classify tool-only or empty turns as stalled.
- Add regression coverage across permissions, gateway approvals, budgets, commitments, child sessions, subagents, flows, and full workspace execution.

## 3.0.7 — 2026-09-05

### Stream lifecycle management and multi-task scale

- Prune inactive Server-Sent Events (SSE) connections when switching between tasks or dismissing panes.
- Prevent HTTP/1.1 connection pool exhaustion in desktop WebKit and browser runtimes, allowing seamless navigation and creation across $N$ chats.
- Reconcile background running streams and disconnect completion listeners on task completion.

## 3.0.6 — 2026-09-05

### Enhanced renderer visualization and canvas harmony

- Integrate native Mermaid SVG diagram generation into Markdown and presentation views.
- Expand multi-theme syntax highlighting support across code canvas blocks.
- Refine canvas harmony and layout transitions for rich outcome-first desktop and web presentations.

## 3.0.5 — 2026-09-05

### Capability-driven presentation planning

- Separate recipe composition intent from per-output renderer resolution.
- Record surface-aware renderer decisions and mixed-output audit details.
- Make recipe signal/type matching deterministic and reject ambiguous semantic
  type ownership.

## 3.0.4 — 2026-09-05

### Universal call-contract admission

- Validate every model-proposed tool call against its admitted schema before
  authorization and dispatch, independent of tool or domain.
- Record malformed calls as non-dispatching error values rather than treating
  them as provider or network failures.

## 3.0.3 — 2026-09-05

### Bounded tool context and MCP artifacts

- Bound model-visible tool input and output so oversized results cannot consume
  the entire turn context.
- Persist large MCP results as redacted, bounded artifacts with offset/limit
  reads, allowing follow-up inspection without replaying the full payload.
- Reject malformed MCP action requests before authorization and dispatch.

## 3.0.2 — 2026-09-05

### Endpoint capability routing

- Frozen provider routes now include their API dialect. Agentic OpenAI and
  OpenRouter turns select Responses before dispatch, so function tools and
  reasoning do not accidentally travel through Chat Completions.
- Primary and fallback legs preserve the admitted dialect, preventing retries
  from silently changing request semantics.

## 3.0.1 — 2026-09-05

### Turn capability assembly

- Unified live capability selection, prompt projection, tool schemas, hooks,
  MCP, skills, plugins, and child execution around one turn contract.
- Added append-only per-turn capability bindings and immediate revocation
  checks across authorization and approval waits.
- Routed standalone CLI flows, eval runs, and heartbeat admission through the
  public prepared-turn API.

## 3.0.0 — 2026-09-05

### Capability assembly consolidation

- **Root cause fix:** MCP aliases, hooks, skills, and `flow`/`work` tool
  admission previously assembled through four divergent code paths in
  `vak-agent` and `vak-core`, each skipping different filter stages.
  MCP aliases bypassed reach + domain-slice entirely; the `work` tool
  definition was appended *after* all filters had already run; hooks from
  plugin capabilities were never filtered by domain slice.

  A single `TurnCapabilities::build()` pipeline in `crates/vak-core/src/
  capability/turn.rs` is now the **only** assembly point for all four
  capability kinds. Every kind passes through the same four stages —
  channel policy → reach standings → frozen contract → domain slice —
  before reaching the agent or the model.

- `Agent::tool_definitions()` in `vak-agent` no longer appends `work`
  unconditionally when `work_mode == Managed`; it now checks
  `flow_dispatcher.is_some()`, which is only set when the `flow`
  capability survived the domain slice.

- Removed dead code: `Core::mcp_aliases_for_session`, the old
  `mcp_aliases_from_inventory` free function, and
  `Core::hooks_from_capabilities`. These are superseded by
  `turn::mcp_aliases_from_inventory` and `TurnCapabilities::build`.

- Added 10 end-to-end tests covering all four capability kinds through
  the full pipeline (tool, MCP, skill, hook) — both survive and blocked
  cases, plus a combined greeting test and a live-data query test.

### Rendering system fix

Structured ```vak fences in assistant answers and tool results are now
projected to readable text on all chat surfaces (Telegram, Slack, Discord),
not shown as raw JSON. The worker validates each fence against a merged skill
registry (builtins + plugin-contributed) and renders a deterministic text
projection with an inspectable JSON appendix. Plugin-contributed semantic
types are recognized through `SkillRegistry::find_by_type` and optional JSON
Schema validation.

`DeliveryPosture` (cadence × urgency) is now wired into the delivery path.
Packets with `HoldUntilComplete` or `HoldForDigest` disposition are enqueued
to the outbox without immediate delivery; the replay loop skips held packets
until their posture resolves to `Send`. Approvals and interrupt-urgency
packets always send.

The SSE snapshot endpoint now uses the Core's merged presentation planner
(builtins + plugin recipes) for live sessions.

## 2.3.1 — 2026-09-05

A review of the schema-v2 rendering pipeline (2.3.0) turned out to be
auditing an earlier snapshot of it — its five findings about AST
flattening, recipe validation, chart integrity, and delivery-surface
formatting no longer matched what shipped in 2.3.0. Verifying that
surfaced one real, currently-failing regression, plus the actual gap
behind "rendering doesn't do enough": nothing connected a tool's own
result to the recipe/renderer vocabulary that already existed.

### Recipe selection was rejecting almost everything

`RecipeCatalog::choose_for_types` had a guard that rejected every
signal-matched recipe whenever no structured candidates had already
been validated — exactly the state of plain-text classification (no
`\`\`\`vak` fence, no tool JSON). That silently broke selection of
`research.synthesis`, `weather.forecast`, `coding.diff_inspector`,
`coding.test_report`, `terminal.session`, `lifestyle.culinary_recipe`,
`data.spreadsheet_grid`, and `data.multi_chart` for most plain-text
answers, falling back to `answer.basic` instead. Two existing unit
tests were already red on `main` because of it; the redundant guard is
removed.

### A tool's own result can now render richly, with nothing hardcoded to it

Only an assistant's final text was ever scanned for structured data
(inline `\`\`\`vak` fences, bare URLs). A tool's raw result — the thing
most likely to actually carry a chart, a metric, a grid, a test report
— was only ever shown as opaque progress text, except for
`write`/`edit`/`apply_patch`/`imagegen`, matched by literal tool name
for artifacts. Extending that per-name pattern to every domain would
mean a hardcoded branch per tool, forever.

Instead, `structured_outputs_from_text` — the same self-declared,
schema-validated contract a model already uses inline — now also
recognizes a tool result that is itself a bare
`{"semantic_type": ..., "payload": ...}` envelope, with no Markdown
fence required (a tool's result is rarely Markdown to begin with).
Nothing here inspects a tool's name, and no type is ever guessed from
a payload's field names; a result with no declared `semantic_type`
still renders as plain text, same as before.

For the much larger set of tools we don't control — a weather API, a
ticketing system, any third-party MCP server — nothing can make them
adopt our envelope, and rewriting their actual output to force it
would reach past our own boundary into the same value the ledger
records and a later turn's model reads. So a new `ResultAdapter` /
`AdapterRegistry` (`vak-delivery::adapters`) puts provider-shape
translation at render composition instead: an adapter recognizes one
provider's specific, known response shape and projects a candidate for
that one rendering pass only, still subject to the same
`SkillRegistry::validate` as everything else. `built_in_adapters()`
ships empty — no real third-party provider is wired into this
codebase yet, and inventing one to demo would fabricate a shape
nothing actually returns.

Covered end-to-end across five personas' tools — a general user's
weather metric, a developer's test report, a knowledge worker's
research synthesis, a data analyst's grid, and a chart consumer's
telemetry — run through `snapshot()` with tool names the pipeline has
never seen, proving the mechanism is domain-neutral rather than
demonstrating it only at the unit level.

## 2.3.0 — 2026-09-05

- Rebuilt rendering and delivery around the schema-v2 semantic AST.
- Added deterministic recipe validation, stream snapshots, safe Mermaid source
  preservation, richer charts/grids/recipes, and regenerated web bundles.
- Added release and installation verification for macOS, Linux, and Docker.

## 2.2.1 — 2026-09-04

2.2.0 rebuilt the capability subsystem and still answered "I do not have a
tool that can provide real-time weather information" on the desktop, with a
connected, admitted Tavily server attached. This is that fix.

### Admission decides when a prompt may freeze, not each surface

The admitted packet was correct all along — 26 capabilities, `mcp` present,
`tavily` present, the per-turn slice removing nothing. What was wrong was the
**prompt**: it froze the name-only MCP line with no catalog behind it, and a
model handed a server name and no tools reasonably concludes it cannot reach
anything.

The cause was that "when may a prompt be frozen?" was answered in each
surface's startup code, three different ways:

| Surface | Waited for discovery? |
| --- | --- |
| CLI (`exec`, `flow exec`, `plan`) | `warm_mcp_bounded(2s)` — yes |
| `vak serve` (gateway, web `/app`) | `warm_mcp()` — fired, never waited |
| Desktop | nothing at all |

So the same question produced a different packet depending on where it was
asked, and on the desktop it was not a race but a certainty: every session
froze name-only, permanently, because nothing re-renders a frozen contract.

Three changes, each **removing** a second way to do one thing rather than
adding a fourth (invariant 30):

- **`Core::admitted_capabilities` is the only wait.** All three per-surface
  warm calls are deleted. Bounded by `ADMISSION_BUDGET`, so an optional
  integration still cannot block a turn indefinitely.
- **The catalog renders from the admitted packet**, where the registry's
  probe puts it, instead of from a parallel `mcp_cache` inventory that could
  — and did — disagree with the packet beside it.
- **`rebound_capabilities` re-renders a live session's admitted set** at each
  turn boundary, matching on typed `(kind, name)` identity. Admission is
  unchanged; only the description of an already-admitted capability is
  refreshed. This is doc 41 invariant 6 in practice: no restart, no rotation,
  and a session admitted during discovery stops being degraded for life.

`capability_descriptors` also stopped hand-building descriptors a second
time. The two constructions had already drifted — the same built-in tool came
out stamped `"vak-core"` one way and `"builtin"` the other — which a gateway
test caught as a contract mismatch. It now projects from
`CapabilityProvider::declare`, like the registry does.

### `doctor` no longer cries wolf

The `capability health` check added in 2.2.0 reported every diagnostic as a
failure, and on a real machine the first thing it did was go red for a hook
the operator had deliberately disabled. "Configured but not usable" covers
two different things, and reporting a chosen state as a failure trains people
to scroll past the check — which costs more than the noise saves, because a
silently unreachable MCP server is exactly what it exists to surface.
Diagnostics now carry `deliberate`, and the check fails only on breakage
while still counting the rest.

## 2.2.0 — 2026-09-04

Capabilities become a live subsystem, and the release pipeline starts proving
what it ships.

### One capability registry, no restarts, no rotation (`docs/design/41-capability-registry.md`)

vak was running a **process-per-session lifetime model inside a daemon**.
Every mechanism in the capability subsystem was edge-triggered — discovery
warmed once, a prompt frozen once, a connection opened once, a catalog cached
once, with a retry guard written so that caching a *failure* counted as
success. There was no loop behind any of it, so a missed edge was permanent
and uptime turned small races into dead integrations.

The visible symptom: a configured, connected search server was unreachable
for an ordinary question about the weather. Nothing errored. The agent
answered from memory and sounded certain, and `doctor` stayed green
throughout, because the diagnostics that explained the failure were rendered
only into the system prompt — the model was told, and the person who could
repair the configuration was not.

All five kinds (tool, skill, MCP server, hook, command) now share one
registry, one reconcile loop, and one projection:

- **Capabilities declare what they serve.** A `serves` vocabulary
  (`live-data`, `web`, `filesystem`, …) replaces the static table that mapped
  each act to built-in *tool names* — a shape that could never mention a
  capability the operator had installed, so every integration needed a
  harness edit and only got one after somebody reported a wrong answer.
  Adding an integration now edits nothing. Undeclared capabilities are never
  narrowed away, and domains are never guessed from tool names.
- **Usability is a state machine, not data.** A failed probe carries a
  reason, a remedy and a `retry_at` with exponential backoff. It is never a
  catalog entry — the old code stored it as a tool literally named `error` —
  and never blocks its own retry, so a server that was down at boot rejoins
  on its own.
- **One level-triggered reconcile loop** replaces every warm, cache and
  per-turn filesystem walk. Hints (filesystem, config, an MCP
  `notifications/tools/list_changed` that the transport used to discard) make
  it run sooner; a ticker makes it run anyway. A dropped hint costs one tick
  of latency and never costs correctness.
- **Turn-atomic epochs replace session rotation.** The registry publishes
  immutable versioned snapshots and a *turn* binds one for its whole
  duration, so a session alive for weeks picks up a skill added today at its
  next turn — with no restart and no rotation. Audit strengthens: every turn
  records the epoch it ran at.
- **Additions land at the next turn boundary; revocations land immediately**
  and fail closed. Dispatch checks availability against the bound epoch and
  authorization against current policy.
- **One report** behind the model's standing section, `doctor`'s new
  `capability health` check, and the console. `doctor`'s extension counts move
  from raw config to the effective set, so plugin-contributed hooks and
  servers stop being invisible to the operator.

MCP transport also pools connections behind a liveness check with an idle
TTL, and probes concurrently under a 10s budget so N unreachable servers cost
the max rather than the sum.

### The release pipeline proves its own artifacts

Every gate verified the *tree* and nothing verified the *binary*.

- **The tag-triggered release workflow could never succeed.** It fires on
  `push: tags: v*`, checkout materialises that tag, and `release.sh` then
  refused to build because the tag existed — the exact tag it was asked to
  build. It now refuses only a tag pointing at a *different* commit.
- **A release now runs what it is about to publish.** `vak --version` must
  report this version and this commit, or nothing ships. This is the
  permanent answer to a release bundling a stale binary.
- **`--no-build` collected from the developer's `target/release`**, entirely
  unrelated to the gates that had just passed. It now requires an explicit
  directory and still faces the provenance gate.
- **Nothing was built `--locked` except the Docker image**, so the released
  binaries and the container could resolve different dependency graphs than
  `Cargo.lock` records.
- **The Docker image was never built or tested anywhere.** CI now builds it,
  serves it, and checks `/health` and `/app` — which is what proves the
  committed bundles actually reached the binary.
- `install.sh` documented `VAK_VERSION` from the start and never implemented
  it, so pinning a version silently installed whatever the feed served.

## 2.1.0 — 2026-09-04

The web client: the workspace surface stops being desktop-only.

### One client, three hosts (`docs/design/48-web-client.md`)

`vak serve` now serves the **full workspace client at `/app`** — the same
client the desktop app ships, in a browser. A headless Linux box is a place
to *use* vak, not only to host it.

It is one source tree, not a second client. The workspace UI moved out of
`crates/vak-desktop` into `crates/vak-client-ui` and sits behind a small
`Host` port with a Tauri and a
web implementation, chosen by a build-time alias so the web bundle never
carries the Tauri IPC layer. No component knows which host it got; they ask
`host.can(...)`, and a capability the host cannot provide is **absent rather
than broken** — there is no Terminal tab on a host without a terminal, rather
than a disabled one that cannot explain itself.

Three things blocked this and are fixed:

- **The server answered only to loopback hostnames.** `[server]
  trusted_hosts` now names the hostnames it will accept, keeping the
  DNS-rebinding defence that pinning provided. Wildcards are refused: one
  there reopens exactly the hole the list closes.
- **`vak serve --host` did not exist**, though `docs/hosting.md` had
  documented it for releases. It exists, and **refuses to start** on a
  non-loopback bind with no `trusted_hosts` — naming the setting, and
  offering the SSH tunnel that needs none of it — rather than starting and
  then rejecting every request with a 421 nobody can diagnose.
- **A dropped event stream lost the gap permanently.** Events now carry a
  sequence number, the last 1024 are retained, and a reconnect replays
  exactly what was missed — or says `resync` when the gap is wider than the
  ring, so a client is never handed a stream with a hole it cannot see. A
  phone that slept, or a closed laptop lid, is now survivable.

Auth is one login for every browser surface: `/auth/login|logout|session`
replace `/admin/login|logout`, which were **removed**, not kept alongside —
two endpoints against one cookie is two contracts that must agree forever.
The cookie is `HttpOnly; SameSite=Strict`, and `Secure` only behind real TLS,
because a `Secure` cookie over plain http is silently discarded and the
session then never persists. Cross-origin mutations are refused even holding
a valid cookie, and `?token=` is now loopback-only.

The terminal is a real shell, so it is **off by default** (`[server.web]
terminal`) and loopback-pinned even when on: every other effect the client
can reach is permission-gated, and a shell is not.

### The surface earns a phone

- A **light theme**, and `system` as the new default. All three previous
  themes were dark. Light is built warm rather than inverted, and its accent
  *darkens* — Burnt Terracotta is 2.4:1 on white and unreadable as a button
  label. Contrast is now measured in CI for every theme, not asserted.
- **Responsive to 375px.** Below 900px the sidebar and dock become overlays;
  below 600px it is a single column. The narrow target is deliberate: read,
  review, **answer an approval**, steer. An approval is a run that has
  stopped and is waiting on a person, which is the one thing that cannot
  wait for someone to reach a laptop — so it notifies, and the notification
  deep-links to the card rather than to the app.
- A **connection indicator** with four states, because "Working" over a
  network is a claim about a round trip that may not have happened.
- Installable as a PWA.

### Also

- **The commitment portfolio reached the workspace client.** The kernel
  shipped it in the admin console only; a durable obligation the runtime
  verifies is workspace information, and there was no way to see or close one
  without leaving to a second application.
- A **second, cold palette** across the presentation components (indigo,
  emerald, rose, amber, sky, slate on near-black grounds) is gone, along with
  ~90 one-off literals. Everything resolves to a token or a `color-mix` of
  tokens.
- Desktop fixes worth naming on their own: the default transcript density
  rendered **nothing at all** while a run was working; the header could say
  "Retrying" forever after one transient failure; seven CSS custom properties
  were used and never defined (removing a focus outline, a border, and a
  chart series); the integrated terminal **leaked a shell process per session
  switch**; sidebar row actions were mouse-only; no modal trapped focus; and
  one render exception blanked the entire application.

### The commitment kernel (docs/design/47-commitment-kernel.md)

Vak learns what it was asked, and what "done" means.

### The intent kernel (`docs/design/47-commitment-kernel.md`)

vak decided a great deal before a turn ran — provider, permission mode,
capability packet, budget — and made every one of those decisions without any
model of what the user was trying to do. Three consequences, all now fixed:

- **The only intent classifier was a keyword hack.** `is_managed_work_request`
  looked for one of thirteen English verbs plus two conjunctions, so "explain
  what this and that mean" read as durable multi-step work. It is **deleted**;
  managed admission now follows from the reading's `horizon` axis. An explicit
  run-scoped `work_mode` still overrides it.
- **Demand-driven routing was wired but fed zeros.** `plan_route_ladder`
  passed `reasoning_required: false, evidence_required: false,
  structured_output: false, estimated_input_tokens: 0`, so every session
  scored identical demand and `order_ladder_v2` never actually varied. Those
  facts now come from the turn's reading.
- **Every tool was advertised on every turn.** A greeting carried the whole
  toolbox. Capability slicing narrows the admitted packet to what the reading
  plausibly needs — `vak exec "hi"` now sees zero tools and one ladder leg.

New `crates/vak-intent`: seven behavioural axes (`act`, `horizon`, `stakes`,
`evidence`, `clarity`, `modality`, `attendance`), deterministic signal
extraction, a cheap-first resolution cascade, the autonomy/envelope model, and
a narrowing lattice. No dependencies on other vak crates — a pure decision
layer, testable without a network, a model, or a config file.

- **It only ever narrows** (`AGENTS.md` invariant 31). `Limits` is a meet
  semilattice whose top element reproduces the previous behaviour exactly;
  `meet` is the only composition operator and there is deliberately no `join`.
  A property test proves no engagement derived from any reading in the
  reachable space widens the baseline.
- **Intent never gates the permission engine.** It supplies a *ceiling* on
  approval permissiveness, so an irreversible turn reaches a human even under
  `auto-approve` — and nothing it concludes can skip a gate the operator
  wanted. A resolution bug can make vak more cautious; it cannot authorize.
- **Uncertainty resolves to the general engagement**, byte-for-byte the
  behaviour before the kernel existed. Being unsure never removes a tool.
- Confidence is **per-axis**: capability slicing gates on `act`, commitment
  promotion on `horizon`. A single scalar let an unsignalled horizon suppress
  slicing the act reading was certain about.
- Ordered axes (`stakes`, `horizon`, `evidence`) take the highest supported
  level rather than an argmax over rivals. A lower level *corroborates* a
  higher one; scoring them as competitors made a dirty working tree's
  `reversible` vote argue against a request's own `irreversible`.

### Durable commitments

New `crates/vak-commit`: work outliving a session becomes a commitment in its
own append-only ledger, rather than the session being the unit of identity and
"the model stopped talking" being the completion signal.

- **The satisfaction lattice** — `asserted < cited < observed < attested`.
  Strength comes from *how* a criterion was established: a command the runtime
  ran is `observed`, an external receipt is `attested`, the model's own
  judgement is `asserted` however emphatically phrased.
- **The closure invariant** (`AGENTS.md` invariant 32). A commitment cannot
  close `fulfilled` below the strength its `evidence` axis demands, and the
  ledger refuses the event at append time. Failure verdicts are deliberately
  unconstrained, so the record can always tell the truth about work that went
  wrong — including an honest `unknown`.
- **Waiting is not failing.** Unattended durable work that needs a human
  suspends on a `Human` wake condition and queues the question, where a
  one-shot turn still fails closed. Every deferred question carries an
  escalation policy, and `assume-conservative` is refused above `costly`.
- **Progress versus motion.** Episodes end with `advanced`, `learned`,
  `blocked`, or `stalled`; consecutive stalls trip a breaker. `learned` exists
  so genuine exploration is not punished.
- **Lifetime economics.** Budget exhaustion *holds* a commitment rather than
  failing it, and expiry produces an explicit `expired` verdict — never a
  silent deletion. Supersession records lineage instead of orphaning work.
- A deterministic, inspectable portfolio scheduler: every priority decomposes
  into named components, and a user pin dominates all of them.

### Surfaces

- `vak intent explain "<prompt>"` — the reading, every signal with the weight
  it carried, the engagement diff against doing nothing, and the exact text
  the model would additionally be told. Costs nothing, dispatches nothing.
- `vak intent show`, and `vak commit list|show|close|supersede|attest`.
- `GET /intent/explain`, `GET /intent/policy`, `GET /commitments`,
  `GET /commitments/{id}`, `POST /commitments/{id}/close` (409 on a refused
  closure — the request was fine; the evidence does not support the claim).
- New session entry type `intent`, carrying the exact model-visible text so a
  replay reproduces the prompt rather than re-deriving it (invariant 1). Only
  the newest applies, emitted immediately before the turn it governs.
- `[intent]` and `[commitment]` config. `intent.autonomy` and
  `intent.escalate = "cloud"` are privileged and stripped for an untrusted
  project: a cloned repository must not grant itself the right to act without
  asking, nor spend the user's credentials classifying.

### Fixed

- `Economics::default()` produced `stall_limit: 0`, so every commitment was
  born already stalled — `#[serde(default = "…")]` does not feed
  `Default::default()`.
- `vak intent explain --surface cron` derived attendance from the calling
  process's surface rather than the one being asked about, reporting an
  unattended cron run as interactive.
- The verb lexicon matched only bare stems, so "before deploying to
  production" contributed no act signal at all.

### Episodes and surfaces

- A durable turn now opens a commitment, brackets an **episode** around the
  work, and records what that episode achieved. `Learned` and `Stalled` are
  deliberately different: a turn that answered substantively but moved no
  criterion reduced uncertainty and must not count against the stall breaker,
  while exhausting the turn budget is the textbook motion-without-progress
  case the breaker exists to catch.
- A weak `horizon` reading opens nothing. A stray recurrence-ish word must not
  leave a month-long obligation behind.
- Seeded criteria are never stronger than `asserted`. The runtime may only
  propose what it could also check, and guessing a test command would
  manufacture `observed` evidence out of a guess — so a commitment held to
  `verified` stays visibly open until a checkable criterion or a human
  attestation arrives, rather than closing itself on a placeholder.
- **Admin console**: the commitment portfolio at `#/commitments`, built as a
  ledger of rows rather than cards. Its signature element is the evidence
  meter — the satisfaction lattice drawn, with the achieved level as fill and
  the required level as a rule beneath the track, the shortfall in the accent.
  Work closed on the model's own say-so and work closed on a check the runtime
  ran are not the same claim, and in an ordinary status column they look
  identical. Scheduler priority decomposes into its named components on click.
- **Desktop**: a composer intent strip, quiet in proportion to consequence.
  Chrome for ordinary work; only irreversible, deferred or must-ask readings
  take the accent and state the reason without needing a click.
- **Every surface**: a read-only `commitments` tool, so "what are you working
  on" is answerable on Telegram, the desktop and a cron check-in alike without
  a gateway slash-command layer that would serve one transport and add a
  second dispatch path. It has no write verb — the model may discuss a
  commitment and may never mark a criterion passed.

### Authority, upkeep, and the loop closing

- **Envelopes are live.** `vak grant` delegates authority to one commitment
  for a bounded time and scope; `vak revoke` withdraws it. A grant reaches the
  running turn's authority, and revocation is honoured on read rather than
  remembered, so it lands at the next authority check rather than the next
  session. No grant, at any autonomy level, lets an irreversible action past
  without a human — proven exhaustively.
- `--on-silence assume` is refused for irreversible work. A default nobody
  confirmed cannot stand in for consent there.
- **Commitment upkeep runs on its own tick**, deliberately not gated on
  `heartbeat.enabled`: heartbeat is an opt-in model pass that costs tokens,
  this is clock and filesystem work that costs none, and tying durable work's
  upkeep to an opt-in prober would mean a commitment stopped being durable the
  moment somebody switched the prober off. It wakes scheduled commitments,
  evaluates predicate suspensions for free (so "watch X, tell me when Y" costs
  nothing at all while Y stays false), applies escalation policies, and closes
  lapsed work as `expired`. A question with no policy waits forever by design.
- **Per-channel autonomy ceiling** joins the existing restrictive-only
  `ChannelPolicy` chain: a chat may cap delegation below what the workspace
  granted and may never raise it.
- **Delivery posture** decides when a packet goes out, never what it says. An
  unattended overnight run rolls its chatter into a digest instead of sending
  forty notifications; an approval and an interrupt-urgency packet are never
  batched, because a held gate is a stopped run and batching it would turn a
  question into a hang.
- **The loop closes.** When an engagement withholds a capability and the model
  then asks for that exact tool, the reading was *measurably* wrong — and the
  row names the capability to put back. Slicing does not merely improve the
  turn; it is what makes misclassification observable at all. Per-cell
  accuracy uses the routing ledger's epistemics: held / contradicted /
  unknown, Laplace-shrunk, 30-day TTL, absence a neutral prior. Abandonment
  deliberately does not count against a reading — a user who walked away told
  us the turn ended, not that it was misread.

### Notes

- `AGENTS.md` invariants 31 and 32 are **appended**, not inserted. Roughly
  twenty code comments cite invariants by number; inserting in the middle
  would have silently invalidated every one of them.
- Phases I5–I8 (envelope wiring into live dispatch, episodes and the portfolio
  scheduler against the real scheduler, admin/desktop/gateway/delivery
  surfaces, and the misread evidence loop) are specified in doc 47 and not yet
  wired. The doc's `Status:` line says so.

## 2.0.1 — 2026-09-03

Universal output engineering, tool-provenance signal engine, and desktop presentation suite.

### Presentation and UI

- **Universal recipe catalog and tool-provenance signal engine:** Expanded built-in presentation recipes across common life and work domains (`research.synthesis`, `coding.diff_inspector`, `coding.change_summary`, `coding.test_report`, `terminal.session`, `data.multi_chart`, `data.spreadsheet_grid`, `lifestyle.culinary_recipe`). Introduced `SignalContext` and `signals_from_context()`, allowing tool names (`bash`, `write`, `edit`, `websearch`), CLI commands, exit codes, and output patterns to drive presentation recipe selection without manual user tagging.
- **Desktop presentation canvas suite:** Added 7 native, outcome-first renderers embedded in the continuous chat stream without external navigation (`ResearchCards`, `DiffInspector`, `TestMatrix`, `UniversalChart`, `DataGrid`, `TerminalConsole`, `RecipeCard`).
- **AST routing & promotion:** Wired `PresentationRenderer` to route diff blocks to `DiffInspector`, promote markdown tables with $\ge 3$ rows to interactive `DataGrid`, and dispatch specialized recipes.
- **Theme tokens and responsive layout:** Normalized `:root` color tokens (`--emerald-bright`, `--rose-bright`, `--text-main`, `--text-muted`) to dynamically cascade with theme changes, and added responsive stacking for compact split panes and mobile viewports.

## 2.0.0 — 2026-09-03

The runtime was finished before anyone outside the project could install it.
2.0.0 closes exactly that, and establishes the contract that keeps every
later release non-destructive.

### Baseline

- 2.0.0 is the supported baseline. State written by an earlier version is
  refused with an explicit message and the one command that resolves it; it
  is never partially read and never migrated.
- The update feed carries no version below 2.0.0.
- `AGENTS.md` invariant 29 forbids accepting, migrating, or special-casing
  pre-baseline state, and invariant 30 requires one canonical way per
  capability.

### Admin console

- Rebuilt the console's first screen as **Home**. It answers four questions
  in order — is the system healthy, what is waiting on me, what is running,
  what is it costing — where the previous Overview showed four stat cards,
  three hardcoded attention tiles, and a key/value dump.
  - A **readiness ring** draws one arc per subsystem from that subsystem's
    own probe. A probe that did not answer reads `unknown` and is excluded
    from the healthy count instead of being counted as healthy; a subsystem
    switched off is excluded too and the count says so. State is carried by
    colour, stroke pattern, and a printed word, so the ring stays readable
    without colour vision.
  - An **attention queue** merges every blocking, asking, or drifting signal
    the product already knew about but never surfaced on the first screen:
    failed doctor checks, open incidents, dead-lettered and queued
    deliveries, chats knocking at the allowlist, a stopped gateway unit,
    drifted bindings, overdue scheduled jobs, provider errors and rate
    limits, skill proposals, unread inbox, and unfinished setup steps —
    each with its own repair as the row's detail. Incidents whose
    fingerprint Home already states in its own words are dropped, so one
    stuck outbox no longer reads as three separate problems. An empty queue
    names how many probes produced it and when, rather than rendering a
    decorative green tick.
  - **Approval gates** are answered on Home rather than linked to; a gate is
    a run that has already stopped.
  - **Pulse** reads a new 30-minute ring buffer of hub arrivals kept apart
    from the 60-item display feed, so the event rate no longer pins itself
    exactly when the system gets busy. The rate names the window it covers
    and never divides by less than a minute.
  - **Spend today** adds burn rate, when the cap lands at that rate, top
    model and provider, a 14-day trend, budget alerts, and the count of
    dispatches carrying no price — which makes the headline figure a floor,
    and says so.
- Extracted the console's shared presentation vocabulary into
  `crates/vak-admin-ui/src/display.tsx`: provider labels, permission-mode
  wording, security-event kinds, hub-event labels, path truncation, and the
  chat-surface catalog. One spelling of each, imported by every view.

### Permissions and capability reach

- **Configured capability is now reconciled against reachable capability.**
  The system prompt advertised what configuration declared while dispatch
  enforced what the composed policy permitted — permission mode, rules,
  channel overlay, approval mode, and the hosting surface's approver — and
  nothing compared the two. A chat-gateway turn was told it had an MCP
  server, spent four tool calls discovering that every call to it was
  refused by an approver that was never going to answer, and reported the
  capability as simply missing. `vak_core::reach` runs the real permission
  engine against each advertised capability and returns its standing;
  unreachable capabilities leave the capability set and the tool registry,
  and the prompt names them, says why, and gives the operator the fix. It
  is strictly subtractive: it never grants, and unattended surfaces still
  fail closed (invariant 15).
- **A channel overlay can no longer escalate.** `tools_allow` and
  `mcp_allow` were compiled into blanket `+` allow rules at two separate
  call sites, so `tools_allow = ["bash"]` — written to *narrow* a chat to
  one tool — handed that chat unattended shell execution with its approval
  gate removed, and `mcp_allow` did the same for MCP. Overlays are
  visibility narrowings, enforced by the registry filter and the MCP glob;
  the single shared translation now emits restrictive rules only
  (invariant 20).
- **`approval_mode = "auto-approve"` now reaches `webfetch` and `browse`.**
  Their restricted-mode gate was injected as a synthetic `?webfetch` rule,
  indistinguishable from one an operator typed, and `auto_approve`
  correctly refuses rule-sourced asks — so auto-approve silently worked for
  every tool except those two. The classification moved into
  `PermissionEngine`'s mode arms, where a mode default is sourced as a mode
  default. A deliberate `?webfetch` still outranks auto-approve.
- Collapsed `build_engine_for_mode` into `build_engine_with`. With the
  injection gone they were identical, and two constructors for the object
  that decides access is how these layers drifted apart.
- An approver declares whether a gate reaches anyone (`Approver::
  answerable`). A refusal from an unattended surface no longer reports
  itself as "denied by user" when no user was asked.
- `doctor` gained a **capability reach** check, and a blocked capability
  records a `capability_unreachable` security event — previously the only
  evidence was a denied tool call inside a session transcript.

### Universal presentation and output engineering

- **Universal recipe catalog and tool-provenance signal engine.**
  Expanded built-in presentation recipes across common life and work domains:
  `research.synthesis`, `coding.diff_inspector`, `coding.change_summary`,
  `coding.test_report`, `terminal.session`, `data.multi_chart`,
  `data.spreadsheet_grid`, and `lifestyle.culinary_recipe`. Added
  `SignalContext` and `signals_from_context()`, allowing tool names (`bash`,
  `write`, `edit`, `websearch`), CLI commands, exit codes, and output patterns
  to drive presentation recipe selection without manual user tagging.
- **Desktop presentation canvas suite.** Added 7 native, outcome-first
  renderers embedded in the continuous chat stream without external navigation:
  - `ResearchCards`: numbered takeaway rows, superscript citation chips with
    hover popovers displaying quoted snippets and source tags, and verified
    source link tiles.
  - `DiffInspector`: multi-file drawer with additions/deletions counts,
    unified vs. side-by-side mode toggle, line gutter numbering, and
    one-click host editor opening.
  - `TestMatrix`: SVG circular pass-rate ring, filter chips (`All` vs `Failed
    Only`), and collapsible traceback drawers.
  - `UniversalChart`: KPI metric pods with delta trends, multi-series SVG
    curves with area gradients, live mouse-tracking crosshair line with data
    bubble, and one-click CSV export.
  - `DataGrid`: interactive table with numeric-aware column sorting, real-time
    search filtering, and CSV export. Standard markdown tables with $\ge 3$
    rows automatically promote to this grid.
  - `TerminalConsole`: dark terminal container with command prompt, exit code
    badge, execution duration, and formatted monospace output.
  - `RecipeCard`: dynamic servings stepper (`-` 2 `+`) that recalculates
    ingredient measurements, paired with live countdown step timers.
- **Theme tokens and responsive layout.** Normalized `:root` color tokens
  (`--emerald-bright`, `--rose-bright`, `--text-main`, `--text-muted`) to
  dynamically cascade with theme changes, and added responsive stacking for
  compact split panes and mobile viewports.

### Documentation

- Deleted eight design documents that described pre-baseline behavior or
  recorded provenance rather than contract: research notes, achievements,
  the vakyartha adoption study, the Tavily integration doc, the first-run
  onboarding and distribution proposals, the competitive landscape, and the
  unimplemented self-evolving-agent proposal.
- Added `docs/design/46-stabilization-install-and-onboarding.md`: bundling,
  release, install, first-run onboarding, the configuration and inheritance
  contract, and the forward-compatibility contract.
- Rewrote `docs/design/00-roadmap.md` against the baseline; it states what
  is true now and what is planned, not what was shipped.
- Repointed every code and doc citation of the deleted study at the document
  that actually owns each contract.
- `README.md` documents the canonical secret path correctly:
  `~/vak-home/.env`, not `data_home()/.env`.
