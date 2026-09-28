# vak — Agent Engineering Contract

Rules for every AI agent (and human) working in this repository.

## Identity

The public product name is **Vakyartha** (https://vakyartha.com), approved on
2026-09-25. The `vak` command, crates, identifiers and data paths keep their
internal names. Brand assets and platform exceptions are in `docs/brand/README.md`.

vak is a Rust general-purpose agent harness. Thesis: strong safety, transparent
operation, extensible integrations, and a simple core.
When a feature request conflicts with simplicity, resolve it as an extension,
not core.

The workspace `Cargo.toml` version is the only authoritative version stamp —
`scripts/check-version.sh` verifies every copy of it, and this document
deliberately does not carry a second one to drift. The supported baseline is
`2.0.0` (invariant 29); nothing below it is read, repaired, or migrated. Two
earlier version lines are retired: the original `0.1.0`–`0.11.51`, and the
short `0.2.0`–`0.2.4` line created by the reset in `fc9c78f`. `1.0.0` sorts
above both, so version ordering is meaningful again. CHANGELOG.md is the
release record; this section does not restate it.

### Reading the design docs

`docs/design/` is the architecture record, and **every document carries its own
`Status:` line — read it before assuming a document describes shipped behaviour
rather than a proposal.** A few numbers are used twice (`30-output-engineering`
and `30-render-architecture`, `51-feed-system` and `51-retired-tools`, the two
`62-universal-delegation-*` files); cite the full filename, never the number
alone.

Core layers, one document each: 01 LLM · 02 sessions · 03 agent loop ·
04 tools · 05 config · 07 prompt · 08 permissions · 09 extensibility ·
10 flows · 11 planner · 12 evals · 13 server · 14 checkpoints · 68 context
(17 is superseded by it).
00-roadmap.md holds the phase history.

Read before changing behaviour in these areas:

- **Failure handling** — `15-reliability.md` (the failure-handling matrix).
- **Security** — `24-agent-security.md` (threat model and priority order). All
  security work follows it.
- **Release** — `32-release-engineering.md`. Never move a version backwards.
- **Always-on platform** — `22-gateway.md` (gateway/approvals), `23-memory.md`
  (recall), `25-docker-sandbox.md` and `54-task-environments-and-promotion.md`
  (execution backends, candidate promotion), `26-learning.md` (learning loop),
  `28-operations.md` with `docs/hosting.md` (durable services),
  `31-network-resilience.md` (the four-plane network contract: loopback-only
  local, crash-only channels, ladder+endurance inference, store-and-forward
  delivery), `53-distributed-bus.md` (`crates/vak-bus`).
- **Surfaces** — `33-admin-console.md` (admin), `48-web-client.md` (browser and
  the `[server]` exposure rules),
  `55-rich-terminal-surface.md` (`vak term`), `34-channel-onboarding.md`
  (channels, multi-bot identity), `38-voice-personality.md` and
  `49-live-voice.md` (voice), `29-personal-os.md` (the personal-use surface).
- **Output and presentation** — `30-output-engineering.md` (the delivery
  contract), `30-render-architecture.md` (cross-surface rendering),
  `57-adaptive-presentation-runtime.md` (packs),
  `67-presentation-renderer-guide.md` (the step-by-step contributor guide, and
  the one to start from when adding a renderer).
- **Decision and capability layers** — `41-capability-registry.md`,
  `42-managed-work-contracts.md`, `45-prompt-layers.md`,
  `47-commitment-kernel.md` (the intent kernel — strands, resolver tiers,
  authority, the control plane; it subsumes skill intent-discovery),
  `50-call-and-evidence-contract.md`,
  `52-outcome-directed-runtime.md`, `43-governed-self-evolution.md`,
  `44-shared-config.md`, `39-plugin-ecosystem.md`, `40-harness-engineering.md`.
- **Install and onboarding** — `46-stabilization-install-and-onboarding.md`,
  `36-first-run-onboarding.md`, `37-distribution.md`, `35-tavily.md`.
- **Experience direction** — `61-adaptive-assistant-experience.md` (Phase 1
  shipped, later journeys phased), `65-universal-adaptive-platform.md` and
  `66-immersive-artifact-canvas.md` (both implemented and audited),
  `70-calm-agent-experience-implementation.md` (the current four-screen visual
  reference and implementation ledger; in progress),
  `58-admin-ia-and-ui-goal.md` (goal and guidance),
  `59-reference-ui-acceptance.md` (an acceptance backlog, not a description of
  the build), `51-feed-system.md`, `51-retired-tools.md`.
- **Superseded — read only for history** — `60-modern-presentation-system-2026.md`
  (by 61), `62-universal-delegation-experience.md` and
  `62-universal-delegation-prototype.md` (by 64),
  `63-agent-first-conversations.md` (by 64).
- **Office documents** — `72-openxml-documents.md` (the file-in, cite,
  redline, review, file-out loop is shipped; its ledger lists what is
  deferred and why).
- **PDF documents** — `77-pdf-documents.md` (the reader and writer in
  `crates/vak-pdf`, carried by the Office loop's own surfaces: `doc_read`,
  `office_apply`, Review with choices and shared drafts; its deferred list
  says what is not built and why).
- **Proposals, not behaviour** — `56-personal-multi-machine-system.md`,
  `73-data-architecture-and-lifecycle.md` and
  `74-lifecycle-and-data-administration.md` (the pending data architecture
  refactor; see "Pending" below),
  `75-visual-refresh.md` (the pending visual refresh: readable type, plain
  words, technical detail on request and the Ink and Saffron brand; see
  "Pending: the visual refresh" below),
  `76-intake-and-knowledge.md` (the pending redesign of the feed system into
  one intake path feeding the catalog, agent-reachable, with shared
  lifecycle; supersedes the target model of `51-feed-system.md` and depends on
  the data-architecture catalog at M6.5. Its §9 is a standalone security fix
  for the shipped pipeline).

### What is authoritative

**`64-agent-owned-platform.md` is the authoritative product model.** Agents own
conversations, private state under `<data home>/agents/<agent_id>/`
(`vak_config::paths::agent_home`: session ledgers, memory), their own
workspace (`vak_config::paths::agent_workspace`: the base workspace for the
built-in `vak`, `<workspace>/.vak/agents/<agent_id>/workspace/` for any
other), lifecycle, channel targets, scheduled work, request admission, and
delivery provenance. Bots are transport identities and channels are
endpoints; internal tasks are implementation details behind the Agent
conversation. Global infrastructure (gateway, operations, FinOps, tasks, the
archive and the trash) stays shared at the top of the data home via
`Core::shared_data_home()`; the FTS index is in the cache home. Doc 64's
topology section draws the tree. Invariant 37 states the enforceable half of
this.

Parked, and not to be assumed shipped: release supply-chain hardening
(SBOM/signing) and the capacity-exhaustion Ask type. The personal-use
completion pass is shipped per `29-personal-os.md`, with enterprise deferred.

### Pending: the data architecture refactor (M0 done 2026-09-25; M1 next)

The maintainer started it on 2026-09-25 with M0, which is done. Nothing
after M0 is behaviour yet, and no session starts a later milestone unasked.

**What it is.** One architecture for everything Vak writes:
- typed ids and a trace key on every record;
- a content-addressed, encrypted storage substrate;
- a tenant/space layout, cut as a new **5.0.0** baseline;
- a Run record for every trigger;
- structured telemetry that carries no content;
- one catalog for search and lineage;
- a lifecycle reconciler with retention labels, legal hold and
  crypto-shred erasure;
- versioned artifacts with inherited sharing;
- admin and user screens for all of it;
- a cloud remote.

**Read, in this order:**
1. `docs/plans/data-architecture-plan.md`: the locked decisions L1–L5,
   milestones M0–M9, exit tests, and the AGENTS.md changes each milestone
   makes.
2. `docs/design/73-data-architecture-and-lifecycle.md`: the model, and the
   audit of today (defects D1–D24).
3. `docs/design/74-lifecycle-and-data-administration.md`: lifecycles,
   policies, erasure, screens, API.
4. `docs/plans/data-architecture-review.md`: why revision 2 differs from
   the first draft.
5. `docs/plans/data-architecture-blast-radius.md`: what each milestone
   touches.

**How to pick it up:**
1. Confirm the maintainer has said to start, and with which milestone.
2. Re-run the blast-radius scans (§0 of that doc). Its counts and file
   locations were taken at `767db1d0` and will have drifted.
3. Follow the order M0 → M1 → M2 → M3a → M3b (5.0.0) → M4 → M6 → M7 → M8 →
   M9, with M5 in parallel after M1.
   - M3b never starts before the M3a refactor (no behaviour change) has
     merged.
   - Erasure (M7) never starts before the catalog (M6), because it needs
     lineage.
4. Each milestone ships whole: code, tests, docs, its AGENTS.md changes and
   its screens. The replaced path goes in the same change (invariant 30).
   There is no compatibility code, because there are no users.
5. A milestone is done when its named exit tests pass and its blast-radius
   section names nothing left unchanged. Then update this section and the
   plan's status line.

**M0, done.** Landed, each with its exit test:
- Bus secrets live in the secret store and apply live
  (`bus_credentials_never_written_to_a_file`).
- `vak self uninstall --purge` removes the logs root (`purge_includes_logs`).
- Feed scripts write only where the server says, under `VAK_HOME`
  (`feeds_write_under_overridden_home`).
- `AgentSchedule` and `agents_runs.jsonl` are deleted; `TaskDef` is the one
  schedule model.
- A routine that cannot run leaves a `routine_failed` inbox entry, run ids
  are full UUIDv7, a cron slot is spent only by a run that started, a run's
  handle is its ledger id, and a child run's home is not nested
  (`fire_task_records_refusal`, `non_git_space_routine_is_refused_loudly`,
  `two_tasks_due_same_tick_both_fire`, `cron_slot_not_lost_on_failure`,
  `scheduled_run_resolves_after_restart`, `child_core_home_is_not_nested`).
- An unchanged capability binding is written by reference
  (`unchanged_capabilities_not_rewritten`).
- The trash is honoured everywhere: a trashed session is gone from every
  list and search (the model's `session_search` included), its transcript,
  export and digest, and nothing reopens it; it can be restored.
  `vak_core::trash` is the one source (`trashed_session_absent_from_every_search`).
- Doc 64's topology section and "What is authoritative" describe the real
  4.x tree.

**Known gaps M0 leaves for later milestones:**
- A non-git space's routine is refused, not run: running it needs M4's
  copy environment.
- The trash hides; it never erases. Erasure is M7.

**Until the next milestone lands, don't deepen the debt:**
- Build no second schedule model: scheduled work is a `TaskDef`.
- Declare every new durable file in `vak_core::state::REGISTRY`.
- Resolve every new path through `vak_config::paths` and the `Core` home
  accessors.
- Give new records full UUIDv7 ids, never clock-derived or truncated ones.
- Keep conversation content out of logs.
- Read a session for a person or the model through a path that honours the
  trash (`open_historical_session`, `Core::open_session`, or
  `vak_core::trash`), never by opening its ledger file directly.

### Pending: the visual refresh (V1, V2, V3 and V4.4 done; V4 in progress)

The maintainer approved the direction and locked all five of its decisions
on 2026-09-25, then asked for the logo and platform-icon correction (V4.1)
and for implementation to continue stage by stage. V1 and V2 are done; V3 is
in progress, and the rest of V4 follows it.

**What it is.** One visual system for the shared client
(`crates/vak-client-ui`), so for both the desktop and the web app:
- readable type: 16px conversation text and 14px controls at 100%, nothing
  under 12px, with system sans as the default and Newsreader as an option;
- plain words in place of engineering terms in every everyday string;
- one Show technical details setting, off for new installs, which replaces
  Transcript detail and hides detail, never capability or safety state;
- the **Ink and Saffron** brand: indigo actions, the Songbird mark's saffron as the one
  accent for live states, neutral light surfaces, a charcoal dark theme, and
  four theme choices;
- fixes for fifteen visible defects found in the 4.0.2 review.

**Read, in this order:**
1. `docs/plans/visual-refresh-plan.md`: the decisions, stages V0 to V4, and
   the checklist with each item's "done when". It is the tracker: tick items
   there and add a progress-log line as they land.
2. `docs/design/75-visual-refresh.md`: the findings, the system, the glossary
   and the disclosure table.
3. `docs/assets/visual-refresh-2026/visual-refresh.html`: before-and-after
   mockups and the review screenshots.

**How to pick it up:**
1. Confirm the maintainer has said to start, and with which stage or item.
2. Re-find each defect in the current tree first. The review was taken at
   `acca0c88`, and its line numbers will drift.
3. Follow V1, then V2, then V3, then V4. V1 items are independent and may
   ship one at a time; no V3 screen starts before the V2 tokens have merged.
4. Each change replaces what it supersedes in the same commit (invariant
   30): old tokens, CSS overrides, strings and components go with it.
5. An item is done when its "done when" holds in the running app in a
   browser at 1440 × 900 and 390 × 844, light and dark, with screenshots
   saved as the plan says. A build or typecheck alone is not done.

**Until it starts, don't deepen the debt:**
- `DESIGN.md` describes the target system (V2.1); the stylesheet moves onto
  it in V2.2 to V2.4. Build new work to `DESIGN.md`, not to the old values
  still in `styles.css`.
- Name things in user-facing strings by what people recognise (doc 75 §7).
- Add no font size under 12px and no hex literal outside a theme block.
- Edit a selector's existing rule instead of adding an override later in
  `styles.css`.
- Keep IDs, paths, byte counts and hashes out of everyday screens.
- Follow the generated platform exports in `docs/brand/README.md`. Preserve
  the Songbird silhouette and complete app tile across themes; light, dark and
  one-ink lockups come from the SVG master. Only the macOS menu-bar template
  uses the approved monochrome, tile-free geometry.

## Non-negotiable invariants

1. **Model-visible means logged.** Anything that reaches a model request must
   be reconstructable from the session JSONL via `derive_messages()`. New
   model-visible input ⇒ new session entry type. Tests enforce this.
2. **Append-only sessions.** Never rewrite or delete session entries.
   Branching = new entry with `parent_id`. Compaction = an entry, never deletion.
3. **Errors are values.** Tools return `is_error` outputs; providers push typed
   errors into streams; the loop returns `TurnOutcome`. Library code never
   panics on bad input; `unwrap`/`expect`/`panic!` are forbidden outside tests.
4. **Every in-process streaming event carries delta AND snapshot.** Consumers
   inside the runtime choose their abstraction level; never force
   re-derivation. The client wire is a projection: `client_events::project`
   (vak-server) sends deltas only and only events a person is meant to see,
   and presentation frames carry a snapshot only when a stream opens, a run
   settles, or a consumer resyncs; the snapshot endpoints are authoritative.
   Under backpressure events coalesce (`StreamEvent::try_merge`), never drop.
5. **Abort preserves partial output.** Cancellation tokens thread through every
   async call; partial results survive.
6. **Unsafe is denied** workspace-wide except process-group kill in
   `vak-tools/src/bash.rs` (annotated).
7. **Transient provider failures retry within the turn's route ladder,
   which is planned fresh on every turn.** The turn ladder is assembled
   at dispatch time from `Core::plan_route_ladder()` using the live
   evidence ledger, session belief state, warm discovery cache, and the
   operator's current `effective_route()` — never from a session-frozen
   snapshot. Walking the turn ladder never changes the effective route.
   Retries honor `Retry-After` under a per-step watchdog deadline;
   run-level endurance re-attempts the same turn after cancel-aware
   backoff when nothing was committed. Informed transience (429 with
   Retry-After, explicit overload) feeds endurance but does not trip
   the shared circuit breaker; blind failures (network loss, deadlines,
   truncated/malformed streams) do. An open breaker fails fast;
   endurance paces its waits to the remaining cooldown so the half-close
   probe gets through. Never retry user aborts. The session header's
   `FrozenContract.route_ladder` / `.provider` / `.model` fields are the
   **initial admission snapshot** for audit only; `WorkReceipt` is the
   authoritative per-turn dispatch record.
8. **Secrets never enter git.** API keys live in the project secret scope or
   the canonical shared secret scope named by `vak_config::user_env_path`
   — resolved through `vak_config::credentials` to an OS-native secret
   service or an encrypted-file fallback, never a plaintext file, so there
   is nothing here for git to accidentally pick up. Point lookups go
   through `vak_config::read_env_file_var/get_var`. Real environment
   variables take precedence over a stored secret. Never hardcode, echo,
   or commit keys. No API writes a secret to a file: a secret an endpoint
   accepts (a provider key, a bot token, bus credentials) goes to the
   credential store (`bus_credentials_never_written_to_a_file`). Keys are
   user-supplied and user-revocable: `Core::set_provider_key` /
   `remove_provider_key` own the whole lifecycle, and both invalidate the
   cached provider client and the discovered-model cache.
9. **Model catalogues are discovered, never hardcoded.** The set of models a
   provider offers is a property of the user's key, not of our source tree —
   a baked-in list hides models shipped yesterday and offers ones the key
   cannot reach. `vak_llm::models::list_models` asks the provider
   (`GET /models`, paginated where the provider pages) and
   `Core::discover_models` memoises it for 5 minutes. When discovery fails,
   surface the reason; never substitute a static list. Endpoint *hosts* are
   configuration and may have defaults; model *ids* may not.
   AWS Bedrock additionally reports native control-plane status for each
   discovered model: agreement, authorization, entitlement, and regional
   availability. Only a fully available model is invokable in the admin
   selector. Mantle inference uses the scoped `AWS_BEARER_TOKEN_BEDROCK`,
   while native checks use the standard AWS SDK credential chain so headless
   AWS deployments can use IAM roles, SSO, web identity, or environment
   credentials. Failed checks remain explicitly unknown/unavailable.
10. **Restricted filesystem access is workspace-rooted.** In read-only and
    workspace-write modes, automatic read/glob/grep access must resolve inside
    the canonical session workspace; traversal and symlink escapes fail
    closed. Any exception requires an explicit scoped rule or FullAccess.
    Never weaken this with string-prefix checks or check only one file tool.
11. **Permission changes revoke old capability.** A runtime mode change must
    cancel in-flight main and side runs and reject pending approvals before the
    new mode is reported. Never let an agent continue with a stale FullAccess
    or sandbox snapshot.
12. **Secrets are not ambient tool state.** Bash and MCP subprocesses receive a
    small operational environment allowlist, not the parent environment.
    Provider, gateway, and connector credentials require explicit,
    recipient-scoped injection; never restore blanket environment passthrough.
13. **FullAccess is an explicit human trust decision.** It is unsandboxed by
    design and must never be selected automatically after a denial, tool
    failure, retry, prompt request, or model recommendation. Restricted-mode
    network denial and approval prompts are independent layers; an approval
    never silently disables the OS sandbox. The one network allowance in a
    restricted mode is a dev-server preview the user configured
    (`.vak/launch.toml`): its worker may listen on a port
    (`Sandbox::listening_variant`), and outbound connections stay denied.
    Seatbelt cannot limit listening to loopback, so a preview server that
    binds every interface is reachable from the LAN, as it would be run by
    hand; agent commands never get the allowance. Landlock keeps previews
    closed until the same rule is implemented and verified on Linux.
14. **Model tools cross a broker boundary.** Built-in filesystem and Bash tools
    execute through the versioned `__tool_worker` protocol in a disposable
    process group; MCP servers execute as separately sandboxed workers. Raw
    built-in tool construction is reserved for the worker implementation and
    deterministic unit fixtures. Task orchestration and session search are
    narrow broker-owned capabilities: never give a worker the policy engine,
    approvals, provider credentials, session store, or control-plane handles.
    A missing worker or unavailable restricted sandbox fails closed. A
    command-scoped backend such as Docker must be applied by the broker to the
    validated Bash command; never try to execute a host worker binary inside
    an image that does not contain the pinned worker artifact. A parser of
    untrusted file formats never runs in the server, desktop or gateway
    process: `doc_read` and `office_apply` are worker tools, and every target
    verifier and every Review of an Office file or PDF runs in a worker task
    (`VerifyTargets`, `OfficeReview`) under a read-only, network-denied
    sandbox rooted at the files being read, whatever the session's mode,
    within a deadline; `OfficeNarrow`, which writes a narrower version of a
    draft, may write only that version's fresh staging directory, and
    `OfficeApply` (`vak office apply`) only the directory of the new file it
    writes. A worker that cannot answer fails every planned check;
    a check never passes because verification could not run.
15. **Unattended surfaces fail closed.** The gateway ships disabled, cannot be
    enabled by untrusted project config, and chat-driven turns auto-deny
    escalations unless an explicitly configured approver surface answers a
    forwarded gate inside its window — silence, timeout, or missing delivery
    credential always means no, and a late reply resolves nothing. Never turn
    a denial into permission, forward gates through ambient state, or let
    verdict-shaped chatter from non-approver chats resolve anything. An empty
    `gateway.chat_allowlist` fails closed (rejects every chat) unless the
    operator explicitly sets `chat_allowlist_open = true`; never restore
    "empty means allow all" for compatibility. The live source of truth is
    `<sessions_home>/gateway/allowlist.json` (docs/design/34), seeded once
    from `gateway.chat_allowlist` — an unknown chat lands as a reviewable
    `pending` entry rather than a flat, dead-end rejection; a `denied` entry
    stays sticky and never re-prompts. Inbound channel bridges (Telegram,
    Discord, Slack) build their payload through `InboundChannel` +
    `InboundRequest::new` (`vak-server/src/gateway.rs`), which rejects an
    empty or placeholder `chat`/`sender` — a bridge that reuses one fixed
    identity for every remote user would otherwise defeat the allowlist
    silently. A pooled per-workspace `Core` (multi-tenant CorePool,
    docs/design/34 Phase 2) goes through the exact same trust/permission
    resolution as a local `vak` run in that workspace — pooling never
    grants a channel more access than approval already gated it into.
16. **Every execution path authorizes before dispatch.** Agent turns, task
    children, static flows, dynamic plans, evals, server runs, desktop runs,
    and browser-driven runs must use the same permission decision and
    brokered registry. A direct flow
    node or convenience SDK path may not call an effectful tool before
    evaluating `PermissionEngine` and resolving `Ask` through its approver.
17. **Configuration has one contract across every surface.** Persistent
    workspace preferences changed through authenticated `/config` are written
    atomically to `.vak/config.toml` before the live Core override is
    applied. `/health`, `/config`, `/providers`, admin snapshots, and session
    transcripts must report effective values and their source where relevant.
    CLI flags, task pins, heartbeat pins, and worker pins are scoped
    overrides and must remain visibly non-global. Session provider/model
    contracts freeze at session creation; changing workspace defaults never
    rewrites an existing session or silently changes its dispatch. Provider
    and model are one atomic route at every read/write boundary. New-session
    admission refreshes persisted defaults across processes unless an explicit
    scoped pin is present. Gateway bindings expose route provenance and stale
    reasons; a changed effective route rotates to a new frozen session while
    preserving the old append-only ledger. Persisted max-turns, theme, MCP,
    hooks, and permission mode use the same cross-process refresh; permission
    changes revoke active capabilities before apply — true today for
    `state.sessions` (`apply_permission_mode`, vak-server/src/lib.rs; a
    gateway-registered handle is in that same map, so the *current* turn on
    a channel is reached too) and for the gateway's own default workspace
    (its `CorePool` entry is `state.core` by shared `Arc` identity, not a
    copy). This also holds for other workspaces served by the gateway: warm
    pool entries recheck their persisted security ceiling on cache hits and
    cancel the old permission lease before narrowing.
18. **Durable services use the canonical default workspace.** Generated
    launchd/systemd units for the gateway and channel bridges must execute from
    `vak_config::paths::default_workspace()` (`~/vak-home` for each account),
    never the directory where `self services-sync` happened to run. Per-chat
    workspace overrides remain explicit gateway state. Units must also carry
    the invoking user's non-secret `HOME`; a sanitized manager environment may
    otherwise resolve the workspace as the platform data home and migrate its
    `.vak` directory away. Canonical path resolution must never use the
    current working directory as a missing-home fallback; GUI launches use the
    OS account home and otherwise fail closed to an absolute path.
19. **`doctor` diagnoses; `doctor --repair` only acts on checks with a known
    mechanical fix**, then re-collects and re-prints the report. A check
    with no mechanical fix (provider auth, config warnings) is left for the
    operator — `--repair` must never guess at those or paper over a failure
    it can't actually resolve. `scripts/vak.sh <verb>` is a thin dispatcher
    over `scripts/build.sh` / `scripts/release.sh` / `vak self <verb>` /
    `vak doctor` — it routes, it never reimplements a verb's logic.
20. **Channel capability overlays are restrictive and execution-scoped.** An
    allowlist entry may inherit or narrow built-in tools, MCP server/tool
    patterns, visible skills, and hooks; `None` inherits, `Some([])` blocks
    that category, and deny patterns win. The overlay is persisted with the
    binding, included in the `CorePool` identity, filtered from advertised
    surfaces, and enforced again at MCP call time. Skills are visibility
    controls, not a replacement for permission rules. Secrets remain owned by
    the workspace integration and are never assigned to a channel overlay.
21. **A GET that seeds a same-shape PUT must report only the layer that PUT
    writes, never a merged/effective view.** `GET /config/hooks` and `GET
    /config/mcp` report the *project* `.vak/config.toml` alone, not
    `state.core`'s global+project merge — because their `PUT` always
    replaces the whole project-layer list/map with whatever the matching
    `GET` last returned. Reporting the merged view would silently write an
    inherited global entry into the project file on the very next save. For
    a name-keyed map (MCP servers) that only forks a copy; for a bare `Vec`
    merged by `extend` with no dedup (hooks, before `HookConfig::enabled`
    and this rule existed) it duplicated without bound and re-fired a
    side-effecting command once per copy. Any new config surface with this
    read-then-full-replace shape must follow the same rule. A disabled hook
    is data, not an absence — it stays in the file (`enabled = false`),
    because deleting it on disable is indistinguishable from deleting it on
    purpose, and undoing "disable" must not require retyping the hook.
22. **A workspace/session-scoped mutation and its bulk form must share one
    boundary check.** `sessions_home`-level sidecars (`archive.json`,
    `deleted.json`, the gateway allowlist) are keyed globally by id, not
    per-workspace, but a session's ledger file — and therefore what a
    *single* mutation on it may reach — lives only under this process's own
    `sessions_dir(sessions_home, cwd)`. A bulk endpoint over the same
    sidecar (e.g. delete-all-archived) must re-derive and apply that same
    per-item existence check, not iterate the sidecar's full keyspace on
    the assumption that every key belongs to this workspace.
23. **A bot is its own identity, not a synonym for its surface.**
    Multi-bot-per-channel (docs/design/34 Phase 5) means `telegram`/
    `discord`/`slack` name a *transport*, not a credential slot — more than
    one `Bot` (`vak-server/src/gateway.rs`) can share a surface, each with
    its own token, policy, permission_mode, and route. Capability/permission
    resolution is a three-tier chain, bot → chat → workspace, composed with
    the same override-or-inherit convention every other tier here already
    uses: `ChannelPolicy::merge` for allow/deny (chat's `_allow` wins when
    set else falls back to the bot's; `_deny` lists concatenate, never
    override, since they only remove access), `PermissionMode::capped_by`
    chained twice (chat capped by bot capped by workspace — never
    escalates), and route falling through chat → bot → legacy binding →
    workspace default. `AllowlistEntry.inherit_bot_policy` is the literal
    "break inheritance" switch: `false` skips the bot tier entirely and the
    chat resolves purely against the workspace, same as before bots
    existed. A `Bot`'s token is never returned by the admin API once set,
    matching the legacy per-surface token's own never-shown-again property.
24. **One physical chat served by several bots gets one identity per bot,
    never one shared identity.** A key is `surface:chat` for a legacy/
    single-bot chat, or `surface:chat:bot_id` once a bridge's inbound
    payload names its own bot id (`gateway_inbound`, `vak-server/src/
    gateway.rs`) — the bot id is a third segment, not folded into `chat`,
    so `legacy_key_for` can strip it back off. Two bots in the same
    physical chat therefore get two independent `AllowlistEntry` rows, two
    independent sessions/bindings, and two independently-resolved policy
    chains (rule 23) — never one shared conversation governed by whichever
    bot's row happens to exist. `allowlist_resolve_inbound` is the one
    place a bot-scoped key is minted: seen for the first time, it inherits
    an already-`Allowed` legacy `surface:chat` row's
    workspace/policy/route (a chat approved before bots were scoped into
    the key, or a `chat_allowlist` config row, must not need a needless
    re-approval the moment a bot id starts arriving) rather than starting
    a fresh pending review — this is a real map mutation, not merely a
    read, so it must persist the allowlist file like any other write.
    Outbound delivery mirrors the same rule: `AdapterRegistry`
    (`vak-server/src/delivery.rs`) resolves a bot-scoped target through a
    `(surface, bot_id)`-keyed adapter map, never the single per-surface
    fallback, so a reply is never sent under a different bot's token than
    the one the chat is actually bound to — a target naming an
    unconfigured bot must fail loudly, not silently fall back to some
    other bot's identity. Any new code that builds or parses a delivery
    target or allowlist key must preserve this three-segment shape rather
    than assuming exactly `surface:address`.
25. **Optional integrations never gate turn admission, and service managers
    own recovery.** A configured MCP server is advertised by name during
    prompt construction but is spawned and discovered only when the lazy
    MCP meta-tool is invoked — never by the capability registry, admission,
    or prompt assembly. The per-`Core` pool (`vak_mcp::McpManager`) reuses a
    connection across sessions, shuts it down after `IDLE_TTL` unused, and
    never respawns without demand; what demand observed (catalog, last
    failure) reaches the registry as declared data. A missing or wedged
    integration must not delay recording the user message or dispatching
    the provider, and abandoned MCP clients must terminate their child
    process. For durable services,
    launchd/systemd PID state is process truth and `KeepAlive` is the sole
    automatic restart owner. Health probes describe readiness; a transient
    timeout may notify but must never authorize a competing restart.
26. **The Operations Center is an evidence projection, never a synthetic
    dashboard.** `/ops/center` derives resource state from live session
    handles, gateway/CorePool snapshots, service-manager probes, durable task,
    outbox, security, and health data. Missing or unavailable state remains
    explicit; operational metrics must never be mock, sample, or placeholder
    values. Operations URLs preserve `workspace` and `time` query context, and
    every internal drill-down link carries that context forward. Resource
    detail views stop at raw session JSONL, provider work receipts, durable
    outbox records, incident evidence, or manager state. Incident fingerprints
    reconcile open, resolved, and reopened records in
    `<sessions_home>/operations/incidents.jsonl`; service and delivery actions
    append before/after verification receipts to
    `<sessions_home>/operations/actions.jsonl`. Blocking service probes such
    as `reqwest::blocking` must run on a blocking worker, never inside an async
    request handler. Desktop and tray Operations Center links use the same
    bound listener port and bearer/cookie authentication as the admin console.
27. **User configuration is the base layer; narrower scopes store only
    intent.** The Shared layer (`~/vak-home/.vak/config.toml` plus its
    shared secret scope and other capability stores) is inherited by every
    workspace. A project layer
    may inherit, replace a same-named item, add an item, or explicitly disable
    inheritance; it must never receive a copied snapshot of effective user
    values. Session, task, bot, chat, and worker pins resolve after the
    workspace and remain scoped. Admin, Desktop, CLI, and server APIs use the
    same explicit `user`/`project` vocabulary and expose provenance. A GET used
    to seed a write returns that exact layer, never the merged projection.
    Secrets follow the same lookup chain but stay outside TOML: Agent private
    secret scope → project secret scope → Shared platform secret scope →
    process environment. Secret scopes are never literal `.env` files — they
    resolve through `vak_config::credentials` to an OS-native secret service
    (macOS Keychain / Windows Credential Manager / Linux Secret Service) or,
    when none is reachable (the common headless case), an encrypted-file
    fallback; no plaintext secret file is ever written
    (docs/design/44-shared-config.md, "Secrets Chain"). An Agent or project secret
    must never enter a process-global override map where another pooled
    workspace or agent could observe it. A config write from one process must
    reach an already-running `Core`/UI in another without a restart
    (docs/design/44-shared-config.md, "Liveness"). Curated integrations are
    executable definitions backed by real packages; the product must not
    advertise mock, placeholder, or TODO capabilities.
28. **The prompt is layered; its safety floor only grows.** The shipped
    prompt is a seed, not a constant (docs/design/45). `identity`,
    `operating_rules`, `guardrails`, and `surface_note` are user-editable and
    resolve through the same seed → Shared → project → surface → bot → chat →
    agent-role chain everything else uses. The capability contract, the
    `Surface:` line, and the skill/MCP inventories are **code-owned**:
    `PromptBlock` cannot name them, so no API can accept an edit to one — they
    describe the callable interface as it actually is, and a user who could
    edit them could only make the model wrong about its own tools.
    `identity`/`operating_rules` fall through narrowest-wins like `route`;
    `guardrails`/`surface_note` concatenate and de-duplicate like
    `ChannelPolicy` deny lists, so nothing narrower can remove what a wider
    layer said, and a surface note appends to the generated line rather than
    replacing it. File presence is the inheritance switch: there is
    deliberately no way to spell `guardrails.inherit = false`. A project layer
    is untrusted config until the workspace is trusted — its identity, rules,
    and surface notes are demoted exactly like `hooks`/`allow`/`mcp.servers`,
    while its **guardrails still apply**, because a guardrail can only narrow.
    Guardrail text instructs and never enforces; `PermissionEngine`, the
    broker, and the sandbox are the boundary, and no surface may word it
    otherwise. Every winning contribution is recorded in
    `FrozenContract.prompt_layers` with a digest, ordered by layer breadth,
    never alphabetically. On resume an implicit binding (a gateway chat)
    rotates and records the drift; a session the user named by id fails closed
    until `--accept-drift`. An empty frozen list means *unknown baseline*, not
    *everything changed*. An inbound message may never write a prompt layer.
29. **2.0.0 is the supported baseline.** No code may accept, migrate, or
    special-case state written by an earlier version. A data home, install
    manifest, gateway store, or config file that predates the baseline is
    refused by the one shared message — which names the file, says the
    install predates the baseline, and gives the single command that
    resolves it (`vak self uninstall --purge`, then install and run setup).
    It is never partially read, never repaired in place, and never migrated.
    The update feed offers no version below the baseline, so `self update`
    from an older line reports the baseline rather than resolving. A
    compatibility branch for a pre-baseline shape is a review failure, not a
    kindness: the tolerances this replaces grew back one sympathetic commit
    at a time, and that is exactly what this invariant exists to stop. From
    the baseline forward the contract is additive-only within a major
    version — a field may be added, never removed, renamed, retyped, or
    redefined; readers ignore unknown fields; a writer that parses, mutates,
    and rewrites a file preserves fields it did not understand; a file whose
    schema exceeds the supported version is refused loudly rather than
    half-read; and ledgers stay append-only forever (invariants 1 and 2), so
    new information is a new entry type and never a changed one. Within a
    major version no migration exists because none can be needed. See
    `docs/design/46-stabilization-install-and-onboarding.md` Part VII.
30. **One canonical way per capability.** A second way to set a token,
    address a chat, name a workspace, or write a setting is not a
    convenience — it is two contracts that must agree forever, and the
    record shows they do not. A new mechanism *replaces* the one it
    supersedes in the same change that introduces it; it never joins it.
    "Kept for compatibility" is not a justification a review accepts.
    Removing the older path is part of shipping the newer one, including
    the tests that pinned it and the doc paragraphs that described it.
31. **Capability changes never require a restart or a session rotation**
    (docs/design/41-capability-registry.md). vak is a daemon with sessions
    that live for weeks, so every capability mechanism must be
    *level-triggered*: one idempotent reconcile loop moves observed state
    toward desired state, and events are hints that make it run sooner, never
    the source of truth. A dropped hint costs one tick of latency and never
    costs correctness. Additions take effect at the next turn boundary of
    every live session via a published epoch; revocations take effect
    immediately and fail closed. Two corollaries a review enforces: an
    observed failure is a reason with a retry, never data — it must never
    introduce a callable name and never prevent the next demand's retry — and a
    capability the operator installed must be reachable without editing the
    harness, which means the host defines a vocabulary and capabilities are
    data classified against it, never a table of instance names.
32. **Intent narrows, never widens** (docs/design/47-commitment-kernel.md).
    A resolved engagement may lower a budget, lower a permission ceiling, or
    *raise* an approval floor. It may never grant a tool, touch the route
    ladder, cap the turn budget or delegation, raise a cap, or lower a floor:
    those caps only ever removed capacity from requests the reader got wrong.
    This is a property of the types rather than a rule to remember: `Limits`
    is a meet semilattice whose top element reproduces pre-kernel behaviour,
    `Limits::meet` is the only composition operator offered, `is_at_most`
    states the invariant as a predicate, and there is deliberately no `join`
    to reach for by accident. Intent never gates the permission engine —
    permission is evaluated exactly as before and intent may only add a
    requirement on top, so a resolution bug cannot authorize anything.
    `DomainSet::All` means everything and only a *disabled* kernel produces
    it; an uncertain reading resolves to the explicit *orienting* engagement
    (general posture, orientation-floor domains). A reading never removes a
    capability at all: admission is policy only, and the reading decides
    which admitted tools are *loaded* — the rest are one `find_tools` call
    away, so being wrong never removes a tool the turn cannot get back. A request resolves
    to its **strands** (one reading per part, with relations and cross-turn
    lineage); the turn's engagement meets every authority-bearing limit
    across them and unions their domains. Resolver tiers 2/3 may raise
    stakes or evidence and never lower either; their limits meet the free
    tier's. A weak reading keeps the risk its words stated: it is never too
    weak to raise caution. The tier-1 lexicon is pinned to `RESOLVER_VERSION`
    by a digest test, and nothing in `resolve`/`derive` reads a clock. An
    envelope is pre-authorization *within* existing authority, never a grant
    of new authority: it narrows the strands that serve its commitment and
    covers actions one gate at a time, read fresh at each, and irreversible
    work and an operator's ask rule reach a human whatever was delegated.
    Control authority comes from the
    channel, never from text: `ControlSource` is stamped by the transport,
    human free text is always steering, and only an explicit command
    (`/stop`, `/pause`, `/goal replace …`, a bare `stop`) is control.
33. **The runtime evaluates satisfaction; the model never does.** The model
    may propose criteria; it may not mark one passed. A criterion's
    evidentiary strength comes from how it was established — a command the
    runtime ran is `Observed`, an external receipt is `Attested`, the model's
    own judgement is `Asserted` however emphatically phrased. A commitment
    may not close `fulfilled` below the strength its `evidence` axis demands,
    and the ledger refuses the event at append time: a ledger that can record
    a lie is not an audit trail. Failure verdicts are deliberately
    unconstrained, so the record can always tell the truth about work that
    went wrong.
34. **Network exposure is explicit, never inferred**
    (docs/design/48-web-client.md). The server binds loopback and pins the
    `Host` header to loopback names; reaching it by a real hostname requires
    that name in `[server] trusted_hosts`, and a non-loopback `bind` with an
    empty list REFUSES TO START rather than booting and then rejecting every
    request with a 421 nobody can diagnose. `trusted_hosts` takes exact
    names — a wildcard there is a DNS-rebinding hole with extra steps, and
    entries containing one are dropped with a warning. `[server]` is
    privileged in full: an untrusted project cannot choose an interface,
    relax the host check, or open a shell. A cookie alone never authorizes a
    mutation — a state-changing request carrying one must also carry an
    `Origin` we recognise, while a request with no `Origin` at all (curl, the
    CLI, a bridge) must carry a real header token. `?token=` is
    loopback-only: it exists for the one client that cannot set a header on
    an in-process connection, and on anything reachable by a hostname it is
    just a credential in an access log. The web terminal is off by default
    and loopback-pinned when on, because every other effect the client can
    reach is permission-gated and a shell is not.
35. **Workspace execution works in the workspace, keeps its runtime state in
    `.vak/scratch/<agent_id>/`, and is observable in Workbench.** `bash` runs
    in the canonical workspace (or a `cwd` inside it) — the same view
    `read`/`write`/`edit` address, so what one tool writes the next can read.
    A per-execution empty scratch cwd was shipped and removed: a file the
    shell wrote was invisible to `read`, and a small model looped rewriting
    it. Execution processes run with scrubbed environments (`env_clear`),
    passing only minimal operational paths (`PATH`, `HOME`, virtual
    environment paths) and zero parent credentials or model API keys. Runtime
    state never lands in the project tree: temp files (`TMPDIR`) go to
    `.vak/scratch/<agent_id>/<execution-id>/tmp`, and tool caches and
    bytecode (`XDG_CACHE_HOME`, `PYTHONPYCACHEPREFIX`, the pip and npm caches)
    to `.vak/scratch/<agent_id>/cache`. Files a command creates or changes in
    the workspace are the work itself and are reported as Workbench
    artifacts; a candidate is exported only from an execution that ran inside
    `.vak/scratch/`. All executions stream live stdout, stderr, package
    detection events, and status directly to the Workbench panel for full
    operator observability. Egress and permissions follow the broker security
    model, failing closed when unapproved; read-only mode still denies every
    write. Frontend client preview frames must be sandboxed
    (`sandbox="allow-scripts"`) within safe error boundaries to protect the
    client host from untrusted script execution.
36. **Context is measured, never assumed, and nothing model-visible is cut
    blind** (docs/design/68-context-engine.md). Every number that shapes a
    request — window, usable instruction horizon, tokens per char, prefill
    rate, cache behaviour — comes from a per-model `CapacityProfile` probed
    at bind time (majority-of-three samples per rung, over filler shaped
    like real turns) and revised from every receipt; feedback never widens a
    horizon. The turn is the unit: a turn is never split, the open turn is
    verbatim, a closed turn projects at `Full` (its real `tool_use`/
    `tool_result` pairs, thinking dropped, results replaced by a
    schema-driven digest and cards by their short ack — never a
    prose-narrated trace, which a small model imitates as prose), `Card`
    (one `TurnCard` line) or `Packet` fidelity chosen by the
    `WorkingSetPlanner` against the measured budget by descending
    `max(recency, relevance, anaphora)` value, never a reserved share. A
    compaction packet is a cache keyed by the turn range it summarises,
    never a boundary: the projection renders it only when the current plan
    asks for exactly that range, so a packet written under a small model
    hides nothing from a larger model bound later and every request is a
    function of (ledger, bound model's profile) alone; the reset-with-
    handoff entry is the one true boundary. Every
    tool result stays reachable through `recall` by evidence id,
    presentation id or turn number. No character-count truncation anywhere.
    The system prefix (identity, contract, card catalogue, tool index) is
    byte-stable across turns; everything per-turn (temporal context, intent,
    stance, work contract, conversation thread, nudges) rides in one tail
    block attached once to the turn's directive message — placed *before*
    the user's own words, and never restating the directive, both found
    live to derail a small model — and stays there byte-identical for every
    later step of the turn: the working-set plan and tools array are each
    resolved once per turn too, re-planned only by an explicit over-length
    rejection or incremental compaction, so a message the turn already sent
    never silently changes shape underneath a replayed thinking block
    (required for provider-specific preserved-thinking checks and for cache hits).
    Only a handoff reset clears the frozen plan. Cache breakpoints/keys are
    rendered per provider. Presentations and TurnCards are hash-linked
    ledger entries, never rebuilt from tool arguments. A directive with
    temporal deixis ("current", "right now") that gets no retrieval this
    run is a `[freshness-check]` redo, then fails closed with an honest
    last-known statement rather than presenting a carried-over figure as
    current; a thinking-only step is one `[empty-step]` redo unless a card
    already answered; three consecutive steps that repeat a prior turn's
    answer verbatim end the turn via the same degraded outcome as
    tool-repair exhaustion.
    Vendor and topic names (a search provider, a weather API) are never
    behaviour keys; `vak-eval`'s banned-token gate enforces it.
37. **Agent ownership is mandatory for new work and isolates workspaces,
    memory, and execution.** Every newly admitted session, request, scheduled
    run, child/delegated run, and channel delivery has one resolved Agent
    identity plus its ConversationKey, audience, origin, and configuration
    revision. Each top-level agent (the built-in `vak` and user-defined custom
    agents) owns private state under `<data home>/agents/<agent_id>/`
    (`vak_config::paths::agent_home`), encompassing private append-only
    session ledgers under `sessions/<cwd-hash>/` and private memory under
    `memory/`, and its own workspace (`vak_config::paths::agent_workspace`),
    with quarantined execution scratch partitioned under
    `<workspace>/.vak/scratch/<agent_id>/`. Cross-agent infrastructure (the
    gateway allowlist, bots, operations incidents, actions receipts, FinOps
    ledger, scheduled tasks, the archive and the trash) remains shared at the
    top of the data home via `Core::shared_data_home()`. Agent identity is resolved
    at admission and cannot be supplied by untrusted client text. Bots
    identify transport credentials; channels identify endpoints; neither is an
    Agent. Paused, archived, or revoked Agents and endpoints fail closed,
    cancel affected work, and never fall back to Vak. Internal tasks, tools,
    flows, and workers inherit or explicitly freeze Agent ownership and are
    projected back only through authorized Agent conversations
    (docs/design/64-agent-owned-platform.md). On the client presentation
    layer, Workbench execution telemetry and artifacts are partitioned per
    conversation session in `sessionWorkbenchMap`, guaranteeing that
    executions in background agents are never dropped or cross-contaminated
    when switching active chats, with serialized admission gates preventing
    dropped requests.
38. **Universal outcome presentation, polyglot document ingestion, and domain
    specialist delegation are platform-level contracts.** vak is a universal
    assistant operating with domain neutrality across engineering, research,
    writing, operations, and quantitative analysis.
    - **Document ingestion (`doc_read`)** provides structured, token-bounded,
      token-efficient extraction across Markdown, plain text, CSV, TSV, JSON,
      YAML, TOML, INI, ENV, HTML/XML, and the Open XML family (Word, Excel,
      PowerPoint and Visio, with their template and macro-enabled variants,
      subject to invariant 39) and PDF (parsed and written only by
      `crates/vak-pdf`: invisible, white, tiny and off-page text is
      labelled, and JavaScript, actions, attachments and links are flagged,
      never run or followed; a PDF changes only through an `office_apply`
      draft and Review, like an Office file, and every write is a clean
      rewrite confirmed by a re-read; docs/design/77-pdf-documents.md),
      strictly confined to the canonical workspace root (Invariant 10).
    - **Outcome presentation** treats all tables and datasets as living
      interactive surfaces: every markdown table generated in conversation
      provides client-side column sorting, search filtering, and instant CSV
      export (`InteractiveTable`).
    - **Domain archetypes (`researcher`, `writer`, `operator`, `analyst`)**
      define standard behavioral expectations, epistemic stances, and
      specialist prompts without fragmenting core execution. Workers spawned
      via `task` receive specialist archetype instructions and capabilities
      according to assigned roles.
    - **Scheduled work** is a task (`vak_core::tasks::TaskDef`), the one
      schedule model, owned by the Agent named in its `agent_id`. A task that
      cannot run says why in the inbox (`RoutineFailed`), never silently.
39. **Office documents are hostile, lossless, labelled and self-sufficient**
    (docs/design/72-openxml-documents.md, O1–O10; this invariant states the
    part the tree enforces and grows with each phase). Open XML packages are
    parsed only by `crates/vak-ooxml`, only in the broker worker (invariant
    14), under its own bounds on entries, per-part and total bytes (counted
    as inflated, never as the ZIP directory declares them), compression
    ratio, XML depth and attributes, because the worker may have no OS
    sandbox under FullAccess. A `DOCTYPE`, a traversing, absolute or
    case-insensitively duplicate part name, and a relationship target that
    escapes the package are refused. The format is detected from the main
    part's content type, never from the extension, and a package whose
    content type contradicts its name fails verification. A write copies
    every part it did not edit as its raw compressed bytes. Document content
    is data: hidden, deleted, white, off-slide, notes and comment content is
    labelled for what it is, and external relationships are recorded, never
    followed. No macro, DDE or include field, OLE object, ActiveX control,
    Excel 4.0 macro sheet or external data connection is ever executed,
    activated or refreshed; Vak has no macro runtime and does not build one:
    macros are read, explained and flagged, and an Agent may rebuild what
    one does as a Vak automation under review. No external office application (Microsoft
    Office, LibreOffice, .NET) is ever a runtime dependency; such tools may
    serve only as CI test oracles, and their results are never shown to users
    as checks. A non-text attachment, from a channel or dropped in a client,
    never enters a prompt as bytes: it is saved to `inbox/` in the Agent
    workspace by the one `save_to_inbox` path and named, and the message
    records it as a typed attachment (`MessageMeta::attachments`) so a client
    draws the file, never the note. Every change to
    a document goes through the one `vak_ooxml::edit` engine and its typed
    ops, and `office_apply` is its only tool: an op names an anchor from a
    read of the exact file (`base_digest`), splices only the elements it
    changes, and is confirmed by re-reading the written package, or nothing
    is written. `office_apply` never writes the workspace file: its result is
    a draft in its execution's `.vak/scratch/` directory, and the one Review
    path (candidate, worker verification, semantic diff, atomic promotion
    with undo) is how a change reaches the workspace. The agent loop refuses
    a shell command that names a draft the turn delivered for review, and no
    command or script is the way to change an Office file; FullAccess can
    still do it, which is what FullAccess means (invariant 13). A Word edit to an existing document
    is a tracked change authored by the runtime's Agent id, never a name the
    model supplies, and marks only the words that change: every run it does
    not split, and everything the reader does not show (footnote marks,
    images, field codes, bookmarks, hidden text, other authors' deletions),
    is kept byte for byte, and a change to a field's result is refused,
    never written; another author's tracked insertion is struck by a
    deletion nested inside it and split around new text, never rewritten.
    A new document (a file not yet in the workspace, made from a template or
    from scratch) is written clean. A file created from scratch starts from
    Vakyartha's own blank for its format (`vak_ooxml::blank`), never from bytes
    a model or a client supplies; `office_apply` takes that start only for a
    path that does not exist and a call with neither `source` nor
    `base_digest`, so a digest naming a missing file is refused as a
    mistake, and Review replays the draft from the same blank. Viewing a
    file never waits on a shared workspace. A changed Excel input or formula sets `fullCalcOnLoad`,
    and cached values read as stale until Excel recalculates. An op never
    adds, enables or strips macros: the output keeps the source's macro
    state. Shared Office workspaces are a built-in client surface over this
    same engine, never a plugin or parser. Each room names an exact session,
    candidate and path; edits are typed ops against an expected branch head,
    applied in the broker worker and frozen as immutable candidate revisions.
    The room snapshot contains metadata and candidate references only. Edit
    grants permit editing and branching, not acceptance; promotion still uses
    the owner-controlled Review path. A stale head is a conflict. Branch
    merges must detect overlapping operations and conservatively refuse
    positional Word merges when inserts/deletes could move anchors. Agent
    candidates enter a room only through verified lineage from the same base.

## Code rules

- Edition 2024, stable toolchain. `cargo fmt` + `cargo clippy -D warnings` must pass.
- No comments unless semantics are non-obvious; doc-comment public API items
  whose contracts aren't clear from names (invariants especially).
- No new dependencies without exact versions pinned in the workspace manifest
  and a one-line justification in the PR.
- Output presentation is schema-v2 and semantic: `vak-delivery` owns the typed
  `OutputTimeline`/`PresentationDocument` contract, exact Markdown fallback,
  capability projection, and deterministic degradation. Desktop/native clients
  render the closed AST registry; raw HTML and untrusted model-authored UI are
  never mounted. Presentation snapshot/SSE endpoints are reconnectable
  projections over the ledger and live events; legacy transcript, `AgentEvent`,
  webhook, and channel text paths remain compatibility surfaces.
  Outcome cards follow a uniform contract and there are no per-type card
  components. Every `semantic_type` key in the single `STRUCTURED_RENDERERS`
  registry (`vak-client-ui/src/components/PresentationRenderer.tsx`) routes
  through ONE declarative, surface-aware renderer,
  `vak-client-ui/src/components/presentation/GenericSpecRenderer.tsx`, as a
  `buildXSpec(payload) -> AdaptiveRenderNode` adapter plus a `renderX()`
  primitive renderer. The node shape is the same `primitive`/`props`/
  `children` tree the host emits (`vak_presentation::RenderNode`), so a
  registry entry is a routing decision, never a new component. Adding a
  `semantic_type` that reuses an existing primitive is a one-line registry
  entry; only a genuinely new PRIMITIVE is a code change across host and
  client. Recipes, research citations, charts, diffs, maps, calendars,
  boards, entities, evidence, documents, graphs, forms, transactions, alerts,
  conversations, and sandboxed previews are first-class primitives backed by
  real validated payloads. Universal semantic shapes compose across domains;
  they are not a coding-only card taxonomy. The step-by-step contributor
  guide is `docs/design/67-presentation-renderer-guide.md`. A model emits a
  card preferentially by calling one of the twelve `emit_*_card` tools
  (`vak-core/src/presentation_tools.rs`, one per payload shape, not per
  `semantic_type`) rather than writing an inline ` ```vak ` fence — measured
  far more reliable against the small local models this app targets (see
  `docs/design/07-prompt.md` v3.4.5, `docs/design/30-render-architecture.md`
  §30.1). The fence path is the fallback when no matching tool is present. The
  card travels in the call's *arguments* (recorded untruncated in the ledger);
  the tool result is only a short ack or a repairable validation error, and
  `vak-server`'s projection rebuilds the card from the call
  (`vak_core::presentation_tools::card_output_from_call`) — never from result
  text, of which an over-long result's request carries only a window. A
  worker's cards reach the conversation that delegated to it: the worker
  records them in its own ledger, `task` hands them up beside its text
  (`ToolOutput::delegated`), and the parent loop records each as a
  `PresentationSource::Delegated` presentation of the `task` call and lists
  its id in the result, so the user sees it and the parent can recall it. A
  projected card carries `OutputKind::Card`, never `Information`/`Progress`/
  `Retry`, which chat views fold away (doc 30-output-engineering). When a
  model answers in prose what the app's own signal/recipe detection says is a
  card, the agent loop gives it one
  bounded `[presentation-check]` nudge (`AgentConfig::presentation_check`,
  supplied by `Core` from `RecipeCatalog::intended_outputs`); like the other
  repair nudges it is scaffolding and never shown as a user message.
  Runtime-authored traffic is **typed, never sniffed**: every user-role
  message the agent loop appends (repair nudges, stop guards) is created with
  `MessageRecord::control(kind, body)`, which sets `MessageMeta::control` to a
  `vak_intent::control::ControlKind`, and that tag is the only way any layer
  recognises one. `vak_intent::control` is the single vocabulary (the kinds,
  the inline hints, the derived context-block tags) and the single
  implementation of `clean_scaffolding`, shared by `vak-server`, `vak-delivery`
  and every channel. The tag drives the projection, the transcript API, session
  search, exports, compaction packets, reflection, and the per-turn state
  the runtime resets on a user message; none of them count a nudge as a user
  turn. Clients receive only what a person can see (`/transcript` omits nudges,
  context blocks and the frozen contract), each message with its ledger
  `entry_id`, and the chat pairs a turn with the projection by that id, never
  by position. An answer the runtime sent back for a redo is an internal draft
  and is not projected. A channel, webhook, inbox or routine summary gets the
  turn's cards (their deterministic text form) ahead of the narration, via
  `projection::text_with_run_cards`, because a card emitted through a tool is
  not in the model's final text. The two lists the desktop client keeps
  (`INLINE_HINT_MARKERS`, `CONTEXT_BLOCK_TAGS` in `structured.ts`) are checked
  for exact equality with the Rust vocabulary by
  `vak-server/tests/control_vocabulary_sync.rs`.
  Identifiers that name a lockable resource are unique by construction, not by
  clock: a child worker's session id carries a process-wide sequence number
  (`task.rs::next_child_session_id`), because ids that were the clock's
  nanoseconds alone collided when tasks launched in the same wave. The chat
  pairs turns through `vak-client-ui/src/turnPairing.ts`. What remains
  judgement (the keyword signals behind the presentation check, which card the
  model picks, inline tool-result hints) is listed in
  `docs/design/30-output-engineering.md`. Whether a call is *retrieval* is
  never guessed from a tool's name or output: `AgentConfig::retrieval_check`
  is built by `Core` from each capability's declared `serves` domains
  (`capability::provider::call_retrieves_external`), and a tool declares that it
  presents cards with `Tool::presents_cards`. For an MCP server, declare
  `serves = ["web"]` (or `documents`, etc.); a server that declares nothing
  inherits the `mcp` broker's web/live-data claim. A tool names the file a
  successful call delivers for review with `Tool::delivered_file`
  (`office_apply`'s draft): the presentation check stands down for the run,
  an identical call gets the first call's result instead of a second draft,
  and a card whose `artifact_path` previews that file is not shown.
- The shipped prompt seed stays under 1500 tokens and carries its
  `<!-- block: -->` markers; changes require a diff note in
  `docs/design/07-prompt.md`. Layer composition, trust, and the editing
  surfaces are `docs/design/45-prompt-layers.md`.
- A committed or shipped frontend bundle must match its source. `npm run
  build` writes `dist/.src-manifest`; `vak-server`'s and `vak-desktop`'s build
  scripts re-verify it and fail the build naming the stale file. Never
  weaken that check to get a build through — regenerate the bundle.
- Config keys unknown to this version are ignored with a warning, never fatal.

## Outcome-directed runtime contract

The outcome-directed runtime is the canonical interpretation-to-delivery path
for every surface, not a coding-only feature. A request is recorded as an
`OutcomeSpec` during admission and carried through capability discovery,
permission-bounded execution, child runs, evidence evaluation, and delivery.
The specification is an interpretation and completion contract; it never
grants authority. Permission, sandbox, broker, route, and approval contracts
remain authoritative and may only narrow the admitted outcome.

The durable chain is:

`request → IntentRecord/OutcomeSpec → admitted limits → results/evidence → evaluation → OutputTimeline`

`IntentRecord` is model-visible only through its recorded `model_visible`
projection. Audit-only records never enter `derive_messages()`. Sessions are
append-only: `GoalUpdate` records collaborative changes, `IntentRecord` records
outcome revisions, and `ActivityRecord`/receipts record lifecycle and evidence.
Replaying the ledger must reconstruct the model-visible input exactly.

Each meaningful result is independent. A result may be complete, partial,
failed, cancelled, or unknown; a typed renderer cannot upgrade an unsupported
claim into observed evidence. Evidence is scoped to the result and requirement,
and its strength, freshness, identity, and applicability are evaluated
separately. Missing or stale evidence must remain visible as partial/unknown.
Specialised presentation recipes are eligible only when the required typed
result and evidence are present; otherwise deterministic Markdown fallback is
used with a diagnostic. No surface may independently classify a result.

A collaborative message adds to the active goal unless it is an explicit
command: `/goal fix …` corrects (keeping the additions), `/goal replace …`
replaces, `/pause` / `/resume` / `/stop` / `/status` change control state.
There is no text classifier for this — one read "replace the deprecated API
call" as a goal replacement and wiped the additions. The `GoalState`
projection exposes the current objective, additions, superseded revisions,
and control state. Replan/reprioritise/add/remove commands use the
intervention path under the `ControlSource` matrix (humans: queued as a new
outcome revision; agents: only their own children, scope changes require a
human; systems: observe only) and queue work at a safe boundary. Human
approval is required whenever the change would exceed the current authority
or affect an irreversible action.

`OutputTimeline` is the cross-surface contract for web, desktop, and channel
delivery. It carries result outcome metadata, evidence references, diagnostics,
and the projected goal state. Clients render the closed semantic AST or the
exact fallback; they do not mount model-authored HTML or invent status. The
desktop/web goal panel is observational, while the existing run-control bar is
the single action surface for pause/resume/cancel and plan changes.

New capabilities (renderer, skill, evidence adapter, hook, tool, evaluator, or
environment provision) are workspace-scoped proposals until validated and
promoted. Presentation/skill/evidence/evaluator/environment contributions may
be auto-admitted only inside an existing envelope; hooks/tools and anything
networked, secret-bearing, or externally effectful require human review.
Promotion never silently activates a proposal, and revocation takes effect
immediately without a restart.

The evaluator must measure both usefulness and cost: pass rate, completion
verdict, evidence state, human-review rate, latency, token use, and baseline vs
outcome-directed deltas. Repository suites prove deterministic behavior;
representative live workloads are operational validation and must not be
presented as completed from fixture results alone.

## Layout

Every path in the left column is verified by
`python3 scripts/check_doc_paths.py`, so a crate rename that misses this map
fails CI rather than leaving the contract doc quietly wrong. Keep the column
format: the checker parses it (path, spaces, prose), which is why these are
not backticked.

```
crates/vak-llm       unified provider API (anthropic / openai-responses /
                     openai-completions / google / native ollama), SSE,
                     delta+snapshot events, live model discovery
                     (models.rs, incl. quantisation from Ollama's
                     `/api/show`), work receipts + dispatch ceiling
                     (work.rs), frozen-ladder ordering: demand-scored
                     objectives, belief demotion, cross-model fallbacks
                     (route.rs) -- docs/design/42-managed-work-contracts.md
                     Phases A+B+R. Which ids at different services name
                     one model (model_identity.rs): confirmed
                     `[route] same_model` groups, and suggestions read
                     from id spelling alone, used only once confirmed. `ChatRequest.cache` (session key +
                     per-message breakpoints) and `ToolDefinition.defer`
                     render per provider: Anthropic `cache_control` (<=4
                     breakpoints) + `defer_loading` + the server-side tool
                     search tool with opaque `ContentBlock::Provider`
                     round-tripping, OpenAI `prompt_cache_key` +
                     `previous_response_id` chaining, OpenRouter
                     `session_id`; `turn.rs::current_turn_boundary` decides
                     what thinking gets replayed. Native Ollama
                     (`ollama.rs`) speaks `/api/chat` directly with
                     `keep_alive`/`num_ctx`, fills prefill/load timing into
                     `Usage`, never replays thinking, and honours
                     `ChatRequest.think` (`Some(false)` on the structured
                     side-dispatches: classify, compaction, handoff, plan --
                     measured live, a thinking model spent its whole budget
                     deliberating and returned no JSON; the completion judge
                     keeps the default because thinking made its verdicts
                     parse 6/6 instead of 3/6) (docs/design/
                     68-context-engine.md §8, §10, §11)
                     + Gemini Live voice synthesis
                     (google_live.rs): BidiGenerateContent WebSocket
                     session, wall-clock timeout + input-length cap since
                     no upstream deadline exists otherwise, WAV wrapping,
                     WorkPurpose::VoiceSynthesis receipts (docs/design/
                     38-voice-personality.md)
crates/vak-voice     the voice vocabulary: the closed `VoiceProvider` set and
                     its catalogue, the direction-typed `/voice/session`
                     protocol (a client can never author a transcript),
                     server-side speech evidence measured against each
                     utterance's own quiet floor, WAV framing, and the
                     offline engine executables. The HTTP/socket surface is
                     crates/vak-server/src/voice.rs
                     (docs/design/49-live-voice.md)
crates/vak-session   append-only JSONL trees, frozen contract, projection,
                     receipt, presentation and turn-card entries (audit-only,
                     projection-neutral), TurnIndex and fidelity projections
                     (docs/design/68-context-engine.md),
                     dependency-free cross-session search w/ mtime-indexed
                     cache + cross-project search_all (docs/design/
                     23-memory.md)
crates/vak-store     SQLite FTS5 rebuildable index over session JSONL:
                     BM25 full-text search (all content blocks incl. tool
                     calls/results/thinking), structured metadata queries,
                     idempotent import, WAL mode — docs/design/23 +
                     33 (JSONL stays source of truth)
crates/vak-delivery  schema-v2 channel-neutral semantic output contract,
                     delivery posture (cadence x urgency) deciding WHEN a
                     packet goes out and never what it says; an approval and
                     an interrupt-urgency packet are never batched, because a
                     held gate is a stopped run
                     (docs/design/47-commitment-kernel.md),
                     closed AST/compiler, capability projection, safe templates,
                     exact Markdown fallback, ordered chunks, Telegram HTML,
                     isolated renderer worker, and append-only retry outbox
                     (docs/design/30-output-engineering.md)
crates/vak-presentation  the CLOSED primitive vocabulary (Primitive enum in
                     lib.rs) plus the bounded declarative spec, JSON-path
                     binding validator, deterministic compiler, content
                     digest, coverage accounting, and explicit fallback.
                     PACKS (StoredPresentation/PresentationSpec, PresentationOrigin,
                     LibraryScope) are runtime-pluggable and compose existing
                     primitives with ZERO code change; seeds.rs is the worked
                     example (75 disabled starter definitions). Adding a
                     primitive is the only part that needs a code change
                     (docs/design/57-adaptive-presentation-runtime.md,
                     docs/design/67-presentation-renderer-guide.md)
crates/vak-ooxml     the Open XML package engine, with NO vak dependencies so
                     it is testable and fuzzable alone: L0 bounded OPC reader
                     (content types, relationship graph, detection by main-
                     part content type, security inspection) and raw-copy
                     writer; L1 bounded XML walk that refuses DOCTYPE and a
                     span-aware splice editor; L2 anchored read projections
                     for Word, Excel, PowerPoint and Visio with hidden-content
                     labels; L3 the typed op engine (`edit`) behind
                     `office_apply`; and the built-in blank packages a
                     file created from scratch starts from (`blank`, parts
                     as plain XML in `blank/`). Every vak call
                     site runs it in the broker worker (invariants 14, 39;
                     docs/design/72-openxml-documents.md)
crates/vak-pdf       the PDF engine, written from ISO 32000-2 with NO PDF
                     library beneath it and NO vak dependencies: L0 lexer,
                     objects and bounded stream filters, the cross-
                     reference chain (tables, streams, hybrids), object
                     streams and a rebuild by scanning; L1 encodings,
                     ToUnicode CMaps, fonts and a content interpreter that
                     places and labels text (invisible, white, tiny,
                     off-page); L2 the anchored read projection
                     (`page:<n>/line:<m>`), the security inspection and the
                     client projection; L3 the typed op engine (`edit`)
                     behind `office_apply`, a clean whole-file writer and
                     a Helvetica layout engine for new content; L4/L5 the
                     semantic diff and review choices. Every vak call site
                     runs it in the broker worker (invariant 14;
                     docs/design/77-pdf-documents.md)
crates/vak-sandbox   the isolated-execution contract, deliberately ignorant
                     of models, prompts, sessions, approvals and
                     presentation: environment lifecycle + state machine,
                     the Seatbelt/Landlock/Docker backends, and the
                     reviewable boundary between a task candidate and its
                     destination (docs/design/25-docker-sandbox.md,
                     54-task-environments-and-promotion.md)
crates/vak-tools     read/write/edit/bash/glob/grep/webfetch/browse/
                     find_tools/recall behind Tool trait, versioned
                     broker-worker protocol, bounded subprocess
                     environment, resource claims, sandbox backends
                     (Seatbelt/Landlock). `find_tools` ranks the deferred
                     set by name/description/argument match and returns
                     full schemas; `recall` is intercepted in the agent
                     loop (never dispatched to a worker) and answers from
                     the session by turn number, presentation id, or
                     evidence id (docs/design/68-context-engine.md §3, §5)
crates/vak-permission rule engine: modes × rules -> Allow/Ask/Deny. Deny/Ask
                     match existentially (one bad effect gates the call);
                     Allow is UNIVERSAL — every segment of a compound shell
                     command must be covered, or the call falls through to
                     the mode default. Quote-aware splitting; redirection to
                     a path and command substitution make coverage
                     unprovable. tests/escapes.rs is the standing
                     adversarial corpus and every fix lands a case there.
crates/vak-plugin    plugin packages: manifest parsing, capability
                     declaration (skills/commands/hooks/MCP manifests),
                     workspace- and user-scoped stores, enable/disable/
                     rollback, invocation records, Ed25519 catalog-signature
                     verification with key revocation
                     (docs/design/39-plugin-ecosystem.md)
crates/vak-intent    the decision layer (docs/design/47-commitment-kernel.md):
                     seven behavioural axes, deterministic signal extraction,
                     clause segmentation into strands (strand.rs), the
                     resolution cascade with the classifier prompt/parse for
                     tiers 2/3, the autonomy/envelope model, the control
                     plane (`ControlSource`, `parse_command`,
                     `evaluate_intervention` in outcome.rs) and the
                     narrowing lattice. NO vak dependencies -- a pure
                     decision layer, unit-testable without a network, a model,
                     or a config file. `Limits` is a meet semilattice whose top
                     element reproduces pre-kernel behaviour; `meet` is the only
                     composition operator and there is deliberately no `join`.
                     Temporal deixis ("current", "right now", "today",
                     "latest") is a signal that sets the `live-data` domain
                     without raising the evidence standard -- the vote alone,
                     tried and measured, stalled a small local model into
                     silence (docs/design/68-context-engine.md §7).
                     `control.rs` is the single `ControlKind` vocabulary for
                     every runtime-authored nudge, including
                     `FreshnessCheck`/`EmptyStep`/`SteeringDrift`.
crates/vak-commit    durable commitments (docs/design/47): lifecycle, the
                     satisfaction lattice, the append-only ledger + projection,
                     and the deterministic portfolio scheduler. Reuses
                     vak-session's CriterionKind/EvidenceRef/WorkOwner rather
                     than defining a second vocabulary for the same idea.
crates/vak-hooks     lifecycle hooks: pre/post-tool-use, stop, session-start
crates/vak-mcp       MCP stdio client behind a lazy meta-tool; the on-demand
                     connection pool (no warm-up, idle eviction, failure
                     backoff) and its observations
crates/vak-bus       distributed event + message fabric for agent swarms:
                     CloudEvents envelopes, W3C trace context + Merkle
                     causal lineage, AES-256-GCM payload encryption,
                     subject algebra with role-based ACLs, dead-letter
                     queues, and NATS Core + JetStream alongside an
                     in-memory engine (docs/design/53-distributed-bus.md)
crates/vak-agent     loop, steering queues (full user messages: text +
                     image blocks), parallel tool execution w/
                     resource-claim waves, retries + watchdog + circuit
                     breaker + stop gate (premature-completion guard),
                     spend-gate seam (docs/design/15-reliability.md), frozen-ladder
                     leg walk (docs/design/15-reliability.md), goal mode + audited
                     completion + regression obligations + handoff reset
                     (docs/design/42-managed-work-contracts.md), workers (task tool) +
                     parent-scoped WorkerRegistry. The loop orchestrates
                     the context engine (`vak-context`) per step -- plan,
                     assemble, dispatch, write the receipt and capacity
                     feedback back -- and owns the summariser call behind an
                     incremental compaction. Turn-close hook builds and
                     appends each turn's `TurnCard`; the freshness, empty-
                     step, steering-drift and card-repeat gates all live in
                     the loop itself (docs/design/03-agent-loop.md,
                     docs/design/68-context-engine.md)
crates/vak-context   the context engine, pure functions over the ledger
                     with no I/O (docs/design/68-context-engine.md).
                     `capacity.rs`: the `CapacityProfile` type, the
                     bind-time probe ladder (turn-shaped filler,
                     majority-of-three samples per rung) and usage/latency
                     feedback -- Core drives the probe and records it, this
                     crate owns the math. `planner.rs`: the pure
                     `WorkingSetPlan` -- per closed turn, `Full`/`Card` fill
                     by descending max(recency, relevance, anaphora) value
                     against the measured budget, overflow collapses into
                     one packet, never a reserved share; `plan_for_session`
                     is the one entry point the loop, `/compact` and the
                     eval gate all plan through. `assemble.rs`: the
                     byte-stable prefix and its digest, the per-turn tail
                     on the last user message, cache breakpoints, the char
                     accounting that feeds the profile, and the summariser
                     request. The ledger it reads (`TurnIndex`, cards,
                     range-keyed packets, `derive_with_plan`, `recall`) stays
                     in `vak-session`.
crates/vak-flow      static flow DAGs + dynamic planner (bounded replan)
crates/vak-eval      deterministic eval suite + live-model mode +
                     context-engine gate and banned-token gate
                     (docs/design/68-context-engine.md)
crates/vak-config    layered TOML config + atomic persisted workspace
                     preferences + credential-store secret loading (OS
                     keychain or encrypted-file fallback) + canonical filesystem
                     paths (paths.rs: data_home, cache_home,
                     logs_dir) + [finops]
                     caps/pricing + [goal] policy + [route] ladder
                     preferences (docs/design/42-managed-work-contracts.md Phases D+H+R) +
                     [automation]/[update]/[tools] (docs/design/29) +
                     [server] bind/trusted_hosts/public_url/
                     session_ttl_hours/workspace_roots/[server.web] --
                     network exposure, PRIVILEGED in full
                     (docs/design/48-web-client.md) +
                     [providers.ollama] keep_alive/num_ctx (Go-duration
                     validated) threaded to the native Ollama provider +
                     [probe] hosted = "none"|"full" gating whether a paid
                     provider's model gets the full horizon-ladder probe
                     (docs/design/68-context-engine.md §1)
crates/vak-core      SDK facade, system prompt, checkpoints, worktrees,
                     capability/ (the registry for all five extension kinds:
                     domain.rs is the vocabulary capabilities classify
                     themselves against so the harness never enumerates
                     instances, resolution.rs is usability -- available or
                     retired, with nothing probed,
                     snapshot.rs publishes immutable epochs a *turn* binds,
                     registry.rs is the level-triggered reconcile loop plus
                     the immediate revocation channel, provider.rs turns
                     Core's five discovery paths into one declaration set,
                     report.rs is the single projection the model, doctor and
                     the console all render),
                     capability/turn.rs (TurnCapabilities: the single admission
                     pipeline for all capability kinds -- channel → reach,
                     policy only; the reading never removes a capability),
                     capability/surface.rs (ToolSurface: which admitted tools
                     are loaded vs deferred, from each tool's own
                     `Tool::always_loaded`/`serves`/`presents_cards`, plus the
                     reading-independent "More tools" catalogue -- a
                     low-confidence reading loads NO domain tools rather than
                     all of them, and reaches the rest through `find_tools`;
                     docs/design/68-context-engine.md §5),
                     capacity_profile_for (the per-request bind: returns the
                     newer of the ledger-recorded profile and the in-process
                     cache by probed-at timestamp, synchronously, from
                     metadata alone when neither exists yet -- never blocks a
                     turn on a probe; `maybe_start_capacity_probe` runs the
                     horizon-ladder probe as a background task once the turn
                     is done, only when eligible (local, or hosted with
                     `[probe] hosted = "full"`), the cache is missing/stale,
                     and no probe for that key is already in flight),
                     prompts.rs (`Resolution{text, tail}` -- the stable
                     prefix and the per-turn tail are two separate strings
                     since 3.5.0; see docs/design/07-prompt.md),
                     mcp_config_section (server names + tool names only, no
                     descriptions or schemas -- MCP is reached only through
                     the `mcp` broker),
                     intent.rs (the seam: gathers facts, runs the cascade,
                     projects the engagement onto runtime knobs -- every
                     function takes a baseline and returns something no
                     wider; `open_threads` for strand lineage,
                     `DeferringApprover` for the `Defer` HIL mode; the
                     tier-2/3 `Classify` dispatch lives on `Core` as
                     `resolve_turn_intent_with_escalation`, spend-gated,
                     watchdogged, fail-open), commitments.rs (opens or
                     continues one commitment per durable thread,
                     brackets an episode per strand, classifies what it achieved,
                     evaluates workspace criteria, and runs the zero-token
                     upkeep pass: schedule wakes, predicate wakes, escalation
                     policies, explicit expiry), tools_commitments.rs (the
                     read-only `commitments` capability), misread.rs (intent
                     evidence: a tool the reading left deferred that the model
                     then used is a MEASURED misread, not a suspected one --
                     deferring is what makes the loop closeable),
                     session_search injection w/ profile tier, memory/
                     skill-proposal tools + duplicate screening
                     (docs/design/26-learning.md, 29 P5), sandbox
                     selection incl. Docker exec backend + runtime backend
                     override (docs/design/25-docker-sandbox.md), manual
                     compaction (compact_session_now), runtime MCP table
                     hot-apply, cost ledger + budget admission gate +
                     alert rows (docs/design/15-reliability.md), task store +
                     cron engine, health report, backup export/import,
                     digest, shared transcript_md renderer
                     (docs/design/29-personal-os.md) + AgentDefinition store,
                     Agent/ConversationKey admission and lifecycle resolution,
                     scoped ownership/provenance for sessions, schedules,
                     child runs, and delivery (docs/design/64-agent-owned-platform.md)
crates/vak-server/site
                     The public website at `/`, `/surfaces`, `/security`,
                     `/install` — src/ built by build.py into a COMMITTED
                     dist/ embedded with include_dir!. motion.dev vendored,
                     not CDN-loaded: the box may have no public route
                     (crates/vak-server/site/README.md,
                     docs/design/48-web-client.md)
crates/vak-server    HTTP+SSE wrapper (sessions/runs/approvals/transcripts/
                     worker steer-stop/MCP management/memory
                     CRUD/search-all/transcript.md/doctor/backup/
                     digest/inbox-ack endpoints) + always-on gateway:
                     chat-surface routing, persisted bindings,
                     cron+script-watchdog scheduler w/ catch-up and model
                     pinning, heartbeat proactive check-ins, budget-alert
                     delivery-to-surface, approval forwarding with
                     addressed-gate resolution (Telegram forwards render as
                     inline-keyboard buttons via TelegramAdapter, resolved
                     through the same verdict-text path a typed yes/no
                     uses), semantic adapter registry, Telegram document
                     attachments (small text inlined, everything else
                     saved to the workspace inbox and named; image
                     attachments stay vision content), Telegram/webhook
                     transports, and
                     outbox replay
                     semantic presentation snapshots/SSE alongside legacy
                     AgentEvent and transcript endpoints
                     (docs/design/22-gateway.md, 28-operations.md,
                     29-personal-os.md, 30-output-engineering.md) +
                     Agent-owned endpoint routing, request receipts, audience
                     isolation, lifecycle-gated admission, and bot-scoped
                     delivery provenance (docs/design/64-agent-owned-platform.md) +
                     multi-bot-per-channel: `Bot` identities independent of
                     surface, bot->chat->workspace policy/permission/route
                     resolution chain, bots.json store, /gateway/bots CRUD
                     (docs/design/34 Phase 5) + per-bot/per-chat voice
                     override riding the same inheritance chain and
                     `inherit_bot_policy` switch as route/permission_mode,
                     POST /voice/speak (docs/design/38-voice-personality.md) +
                     admin console: global event hub + SSE,
                     /admin/api/* data plane, embedded SolidJS SPA at
                     /admin (docs/design/33-admin-console.md), including
                     memory CRUD/cleanup, live parent-scoped worker
                     controls, and the evidence-backed Operations Center
                     (`/ops/center`, durable incidents, action receipts, and
                     bookmarkable resource drill-downs) +
                     the BROWSER SURFACE (web.rs, docs/design/48-web-client.md):
                     ONE login for every browser client
                     (/auth/login|logout|session, HttpOnly SameSite=Strict,
                     Secure only behind real TLS -- a Secure cookie over
                     plain http is silently discarded and the session then
                     never persists); /host + /stream standing in for
                     the desktop shell's backend_info; /workspaces{,/open}
                     over CorePool; /fs/dirs (folder names only, rooted at
                     [server] workspace_roots); WS /pty, off by default and
                     loopback-pinned when on. Host-header pinning is
                     `host_is_trusted` (loopback + exact [server]
                     trusted_hosts; wildcards refused), cross-origin
                     mutations are rejected before routing, and ?token= is
                     loopback-only. events.rs adds EventBus: every session
                     event carries a seq, the last 1024 are retained, and a
                     Last-Event-ID reconnect gets exactly the gap -- or an
                     explicit `resync` frame when the gap is wider than the
                     ring. embedded_ui.rs holds the ONE asset-serving path
                     shared by /admin and /app (recursive registration; the
                     non-recursive version shipped broken three times)
crates/vak-client-ui SolidJS + Vite WORKSPACE client, shared by the Tauri
                     shell and the browser (docs/design/48-web-client.md).
                     src/host/ is the seam: a `Host` port with a Tauri and a
                     web implementation, selected by a BUILD-TIME alias
                     (#host-impl, vite.config.ts) so the web bundle never
                     carries the Tauri IPC layer. No component imports
                     @tauri-apps or sniffs for window.__TAURI__; they ask
                     host.can(...) and render DIFFERENTLY, not brokenly -- an
                     absent capability is a design decision, not an error
                     state. Builds twice from one source: dist/ (base "./",
                     shipped by vak-desktop, gitignored) and dist-web/ (base
                     "/app/", embedded by vak-server, COMMITTED so a headless
                     box builds the server without node). The primary navigation
                     is Agent-first: one durable conversation per Agent, with
                     channels, history, and internal work behind settings/details
                     (docs/design/64-agent-owned-platform.md)
crates/vak-admin-ui  SolidJS + Vite admin console source; the commitment
                     portfolio (#/commitments) with the evidence meter --
                     the satisfaction lattice drawn, achieved as fill and
                     required as a rule beneath, shortfall in the accent
                     (docs/design/47-commitment-kernel.md, DESIGN.md); built dist is
                     committed so cargo builds need no node — observation,
                     operation, and interaction views per docs/design/
                     33-admin-console.md, including the six-area navigation,
                     persistent Operations context bar, and raw-evidence
                     drill-downs, plus a VoiceConfigEditor
                     (inherit-toggle + live Preview button) on the per-bot
                     and per-chat panels (docs/design/38-voice-personality.md)
crates/vak-terminal  the rich terminal surface behind `vak term`: a ratatui
                     app that is a CLIENT of a real vak server over HTTP/SSE,
                     never a second runtime -- health, sessions, routes, MCP
                     inventory, approvals and telemetry are fetched live and
                     kept fresh by background watchers
                     (docs/design/55-rich-terminal-surface.md)
crates/vak-desktop   Tauri 2 SHELL over an embedded secured_router. The UI
                     itself lives in crates/vak-client-ui (above); this crate
                     is the native half -- window/tray lifecycle,
                     single-instance handover, workspace trust gate, PTY
                     commands (spawn/write/resize/CLOSE -- the close is what
                     scopes a shell's life to its pane), and the native save
                     dialog. Renders the schema-v2 outcome-first semantic
                     timeline/AST registry: sessions, split view, approvals, diff
                     review, workers tab, MCP manager, image attachments,
                     best-of-N, tasks (cron/script/pin), side chats,
                     memory tier editor, global search, diagnostics,
                     backup/digest cards, budget banner
                     (29-personal-os.md) +
                     voice narration of turn completions and approval
                     prompts via a shared `<audio>` element and
                     `/voice/speak` (no native audio crate — the webview
                     plays the returned WAV Blob), Settings toggle + voice
                     picker (docs/design/38-voice-personality.md)
crates/vak-ops       service-control layer over launchd/systemd — status,
                     start/stop/restart, install/uninstall shared by tray,
                     CLI and desktop (docs/design/28-operations.md)
crates/vak-tray      menu-bar controller: colour-coded service dot,
                     start/stop/restart/install/uninstall, logs, watchdog
                     with auto-restart + notifications
crates/vak           binary. Run: exec / plan / flow / eval /
                     serve [--host] [--gateway] /
                     telegram|discord|slack [--bot-id <id>] /
                     term (the rich terminal surface, crates/vak-terminal) /
                     open [app|admin]. Inspect and steer: sessions /
                     export / checkpoints / memory / entities / agents /
                     tasks / inbox / intent (explain, show) /
                     commit (list, show, close, supersede, attest) /
                     grant / revoke / office (read, apply, diff, verify,
                     for Office files and PDFs: JSON over the same worker
                     tasks as the Agent's tools, docs/design/72-openxml-
                     documents.md, 77-pdf-documents.md). Configure: config /
                     prompts / skills /
                     skills-review / plugins / setup / doctor / backup /
                     digest / self (state, install, reinstall, verify,
                     update, uninstall, status, services-sync), plus the
                     first-run wizard and the opt-in update check.
                     `user` and `project` are SCOPE arguments on config,
                     prompts and plugins, never subcommands. There is no
                     `tui` subcommand: the inline TUI shipped in v0.1.x and
                     was withdrawn; `term` is its replacement and is a
                     client of a running server, not a second runtime.
docs/design/         architecture decisions — update with behavior changes;
                     security boundaries and roadmap in 24-agent-security.md
scripts/             dev utilities (mock servers, PTY/HTTP smoke drivers)
```

Two same-named module sets that are not duplication: `vak-server/src/surfaces/`
holds the chat-surface TRANSPORT adapters (long-poll and webhook bridges into
`POST /gateway/inbound`), while `vak-delivery`'s modules of the same names own
the MARKUP projection for each surface. Both once sat at `src/telegram.rs` in
their own crate and read as a copy until you opened both.

## Acceptance-workspace contract

Real acceptance runs must use a disposable workspace outside the vak source
checkout (for example, a fresh directory under `/tmp`). The checkout is the
implementation workspace and must not become the destination for generated
dashboards, documents, package caches, local servers, or scratch artifacts.

For generated rich output, an acceptance run is incomplete until it has all of
the following evidence:

1. An explicit output scope (for example `--write-path dashboard.html`) was
   admitted before the tool call.
2. The runtime successfully wrote the output and read the exact path back.
3. Any local server or renderer actually started; a command that prints a
   success message while its child exits is a failed verification.
4. Browser-facing output was inspected in the real browser when the result is
   interactive or visual. A syntax check or model assertion cannot substitute
   for this.
5. The candidate remains quarantined until a user reviews and promotes it.

Tool errors and failed verification are authoritative. Model prose must never
turn a rejected write, missing file, failed server, or failed browser check
into a successful outcome. See
`docs/audits/acceptance-dashboard-2026-09-08.md` for the reproducible dashboard
scenario and residual limitations.

## Verification before every commit

```
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
scripts/check-version.sh && python3 scripts/check_doc_paths.py
```

**Touched a frontend or the public site? Rebuild it, or the binary ships
the previous one.**

```
cd crates/vak-client-ui && npm run build   # BOTH bundles: dist/ + dist-web/
cd crates/vak-admin-ui  && npm run build
python3 crates/vak-server/site/build.py    # the public site at /
```

`vak-client-ui` builds twice from one source — `dist/` for the Tauri shell
and `dist-web/` for the copy `vak-server` embeds at `/app` — and only
`dist-web/` and the admin bundle are committed, because a headless box must
build the server without node. The public site is the third embedded bundle
and the only one needing no npm; its `dist/` is committed for the same
reason, so a clone needs no Python either. All three refuse to compile
against a bundle whose `.src-manifest` no longer matches `src/`, so this is
a build error rather than a silently stale UI; `scripts/release.sh` and CI
additionally fail on a committed-bundle diff, which a local build cannot
see. `.gitignore` has to keep negating each of them out of the blanket
`dist/` rule, or a whole bundle silently stops being committed.

**A test that builds a `Core` must isolate its home first.** Call
`vak_config::paths::isolate_home_for_tests()` (or pin a specific one with
`set_home_override`) before `Core::new`/`Core::new_with_trust`. Without it,
`load_with_trust` reads the operator's real Shared layer —
`~/vak-home/.vak/config.toml` and the real Shared secret scope — so the suite
exercises whatever that machine happens to have configured. Twenty-one test
files did exactly that: a real MCP server was advertised inside tests, and a
personal provider key could make an "unconfigured" case pass on one machine
and fail in CI. Neither
`std::env::set_var` nor `unsafe` is needed for this; the override map sits
above the real environment in `get_var`'s precedence. That home is shared by
every test in the binary, and every `Core::new` reads its Shared config, so a
test that writes `vak_config::global_path()` goes in a test binary of its own,
on a private home per test held under one lock, as
`crates/vak-server/tests/shared_config_layer.rs` does. Two such tests once
sat beside three hundred readers, and removing the file between another
test's existence check and its read failed `Core::new` under full-suite load.

`check-version.sh` covers the stamps a user reads (README badge, CHANGELOG,
git tags), not just the ones the build system reads, and fails on a version
that moves BACKWARDS unless the abandoned line is declared in
`scripts/.version-reset` — a decreasing version makes every install on the
higher line permanently un-updatable, and walks into tag space the earlier
line already used. Both happened; see the release-discipline rules in
`docs/design/32-release-engineering.md` before cutting a release, and never
move a version line backwards. `check_doc_paths.py` fails on a design
doc citing a path that no longer exists; docs whose `Status:` line says
"proposal" are skipped, because their paths are targets rather than
citations.

Live checks (needs a provider key set via the Settings UI, `PUT /config/key`,
or an environment variable):

```
target/debug/vak eval                    # deterministic suite, ~100ms
target/debug/vak eval --live             # real model benchmark
```

## Live development runs

`docs/development.md` is the step-by-step guide to building, component
harnesses, and running a dev build live against a real model in the
browser. Read it before your first live run; the rules below are the ones
that cost a session time to rediscover.

- Build both binaries: `cargo build -p vak -p vak-server --bins`. The
  worker binary is what runs tools, verifiers and Office reviews.
- After a client change, rebuild the bundle, then the server binary, then
  restart the server; it serves the bundle it was compiled with.
- Run live in a disposable `/tmp` workspace on a port other than the
  installed service's (`8901`), against the default data home. A copy of
  the data home does not carry the provider connection. Never pass
  `--gateway` for a dev run.
- Sign in with a throwaway `VAK_GATEWAY_TOKEN` pinned for that server
  process, and get the link from `vak open app --print --port <port>`.
- Never open credential files or read ledgers in the real data home;
  inspect through the app and its HTTP API with the bearer token.
- Keep one browser tab per server origin: several tabs exhaust the
  browser's per-host connections and requests hang silently.
- Stop the server and delete any data-home copy when done.

## Parallel agents

Only touch files you changed in this session. Sessions are per-cwd-hashed;
never edit another session's files under the data home.

Two homes, easily confused, and `crates/vak-config/src/paths.rs` is the only
place that decides either:

- **The data home** — application-managed state (sessions, index, install
  manifest). `~/Library/Application Support/vak` on macOS,
  `~/.local/share/vak` on Linux, overridden wholesale by `VAK_HOME`. Never a
  dotdir; a test asserts it.
- **The default workspace** — `~/vak-home`, a plain directory a person can
  `cd` into. It is what durable services run from (invariant 18) and what the
  Shared config layer and Agent workspaces live under (invariants 27 and 37).
  It must never collide with the data home.
