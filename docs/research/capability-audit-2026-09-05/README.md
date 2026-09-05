# Turn capability assembly audit

Date: 2026-09-05. Source baseline: `6a58f33` (workspace version 3.0.0).
Status: implementation pass completed for the findings below, including the
standalone CLI/eval/heartbeat prepared-turn migration.

## Implementation status

The working tree now fixes R3–R9 at their source and makes the turn plan own
ordinary tool filtering and prompt descriptor selection. It also propagates
the selected hook/MCP projections into child agents. `cargo check --workspace`
passes; the vak-core lifecycle/scope suites and the full vak-agent test suite
pass. The original nine temporary reproductions were deliberately not kept as
failing tests; equivalent regression coverage now lives in the capability turn,
registry, custom-command, lifecycle and scope test modules.

R1/R2 are addressed at the turn boundary by admitting later registry epochs
and rebuilding prompt descriptors from the live selected set, which removes
missing capabilities from prose. Each turn now appends a
`TurnCapabilitiesBound` ledger entry containing the selected/excluded IDs,
epoch, system prompt and tool schemas. Revocation is checked again at tool
authorization, including after an approval wait. Standalone CLI flows,
evaluation runs, and heartbeat admission now consume the public
`Core::prepare_turn`/`PreparedTurn` projection instead of assembling prompt
and tool state independently.

## Conclusion

vak has a capability registry, but does not yet have one authoritative turn capability contract. It has several independently evaluated representations: effective config, declarations, the published registry, a session-frozen inventory, prompt rendering, the MCP manager's catalog, turn filters, actual tool objects, and agent-added schemas. Their disagreement is sufficient to explain “enabled, but the model does not get/call/use it” without assuming a model defect.

Nine isolated contract tests reproduce defects, including two that capture actual provider requests. The existing lifecycle and scope suites pass all ten tests. That contrast matters: current tests mostly prove the registry or discovery layer in isolation, not their composition into a model request and dispatch.

The right replacement is one capability authority that resolves source inheritance and publishes a complete, immutable turn plan. Prompt, schemas, loaders, hooks, commands, dispatch targets and diagnostics must all be projections of that plan. Factories may remain in separate modules; selection and precedence must not.

## Scope and evidence

Read the config merge, plugin/skill/command discovery, registry and resolution state machines, reach checks, turn assembly, prompt construction, agent schema/dispatch assembly, MCP manager/tool, child-agent construction, managed-flow dependencies, and gateway bot/chat/CorePool seams. Reviewed design 39, 41 and 47 and the repository invariants. Design 41 states the intended live-session behavior; implementation evidence takes precedence over its claims of completion.

No real provider, credential, personal configuration, external MCP server or production session was used. Tests use temporary homes, synthetic declarations, and an in-process capture provider. The audit established deterministic harness defects; the subsequent implementation pass changed runtime assembly and does not measure how often any particular live model chooses the wrong tool.

- [Reproduction source](reproductions.rs): nine expected-behavior assertions captured against the pre-fix baseline; it is a historical fixture, not part of the current test build.
- [Observed reproduction output](reproductions.log): 0 passed, 9 failed. Line numbers refer to the source before formatting.
- [Existing test output](existing-tests.log): lifecycle 9 passed; scope matrix 1 passed.

The current regression coverage is in the source modules listed above. The historical fixture is kept outside the normal test suite so the audit does not install permanent failing tests or encode bugs as desired behavior. No new dependency is required.

## What actually happens

1. `vak_config::load_with_trust` merges Shared/project TOML and demotes privileged untrusted config. Core stores that result plus many independent mutable overrides. Gateway composes bot/chat policy and permission pins before selecting a pooled Core (`gateway.rs:565`). Refreshing persisted preferences remains a caller responsibility (`vak-core/src/lib.rs:2108`); the capability ticker itself does not reload TOML.
2. `CapabilityProvider::declare` calls five separate discovery paths: `tool_names`, `skills`, `effective_mcp`, `effective_hooks`, `custom_commands` (`capability/provider.rs:126`). Several already apply channel filtering, which erases excluded candidates before the registry can explain them. Plugins are expanded separately in several of those paths.
3. Registry reconciliation probes MCP servers, stores resolution state, and publishes `CapabilitySet` (`capability/registry.rs:230`). Core also maintains an MCP manager/inventory cache and a background warmer (`lib.rs:1680`). These are distinct stores and lifecycle paths.
4. Session admission waits up to five seconds if the registry has no epoch. If still empty it returns direct declarations; otherwise it takes usable registry descriptors. The result is frozen into the session header (`lib.rs:1282`, `:3828`). Publishing waits for all due probes, so a slow optional server can delay the initial packet, including static capabilities.
5. At each user turn, `run_turn_inner` renders a system prompt from the old session inventory rebound by name to current descriptors. Then it resolves intent and runs `TurnCapabilities::build` (`lib.rs:4173`, `:4223`). Thus the system prompt predates final selection.
6. `TurnCapabilities` selects MCP servers/aliases, skills, hooks and a flow flag. Core separately constructs tool objects, creates a lazy MCP tool, builds child-agent dependencies, and performs another series of channel/reach/contract/domain filters (`lib.rs:4385–4636`). Ordinary tools are not returned by the purported single pipeline.
7. Agent builds each provider request from `cfg.system_prompt` and `tool_definitions()`. The latter appends work and mutable MCP alias schemas after Core's tool definitions (`vak-agent/src/lib.rs:535`, `:1045`). A catalog observer can change aliases during the same user turn.
8. Calls normalize aliases into `mcp`, evaluate permission/approval, and execute the selected object with pre/post hooks (`vak-agent/src/lib.rs:2640`, `:3254`, `:3372`). MCP applies another allow/deny filter. Child agents directly build `AgentConfig`; managed flows assemble `ExecutorDeps` independently.

A “turn” must be defined precisely in the replacement: user turn/episode, each model step within it, and each tool dispatch have different update boundaries today.

## Reproduced findings

### R1 — Existing sessions cannot acquire newly enabled capabilities (high)

`rebound_capabilities` iterates only the original contract, while both turn pipelines require original `(kind, name)` membership (`lib.rs:1314`, `:4600`; `capability/turn.rs:169`). Registry additions therefore do not grant existing sessions visibility. If a session originally had no skills, even the `skill` loader itself is absent from its frozen tool list.

Reproduction: create a session, add a skill, explicitly reconcile, run a turn with intent disabled, capture the request. There is no `skill` schema. A recovered MCP server omitted during initial admission has the same membership problem. This contradicts invariant 31 and design 41's no-rotation requirement.

### R2 — Removed capabilities remain in the actual model prompt (high)

When a current descriptor is absent, rebound falls back to `frozen.clone()` (`lib.rs:1327`). That includes removed skills and unavailable servers. The subsequent tool selection can exclude them, leaving the model instructed about a capability it cannot load.

Reproduction: create a session with a skill, remove its file, reconcile, capture a turn. Its name remains in the system prompt. Independently, domain slicing happens after prompt rendering, so even healthy capabilities can be advertised in prose while omitted from callable schemas.

### R3 — An unchanged reconcile discards the MCP catalog (high)

Probe reports are kept in the local `probed` map only. The next non-due pass carries `Resolution::Ready` forward but reconstructs `configuration` from the declaration, which is null for MCP (`capability/registry.rs:321`; `provider.rs:216`). The digest includes configuration, so this also produces a spurious epoch change.

Reproduction: successful probe returns a query tool; immediate unchanged reconciliation replaces its descriptor configuration with null. The separate Core inventory can still contain aliases while the registry-rendered prompt loses its catalog.

### R4 — Tool-specific MCP allowlists reject allowed servers (high)

The turn pipeline evaluates a fabricated `server/tool` string against the channel patterns (`capability/turn.rs:155`). `mcp_allow = ["search/query"]` does not match `search/tool`, so the whole server disappears even though `filter_mcp` retained it.

Reproduction: a usable `search` server and allowlist `search/query` produce zero admitted servers.

The subsequent constructor also replaces the original allow patterns with admitted `server/*` patterns (`lib.rs:4430`). This is a separate boundary defect: if an original pattern admits the fabricated name, its narrowness is lost at invocation. Restore an intersection with the original policy; do not fix server visibility alone and retain the wildcard replacement. No live unauthorized MCP invocation was attempted in this audit.

### R5 — Reach filtering loses instance identity (high)

`blocked_ids` groups standings by the generic tool string and constructs `McpServer("mcp")` or `Skill("skill")`, not the actual server/skill identity (`capability/turn.rs:120`). With mixed standings it also requires all instances to be blocked before emitting anything. Almost no real server or skill ID can match that result.

Reproduction: a blocked standing for server `search` leaves that server admitted. The separately implemented whole-tool filter can remove `mcp` while aliases remain eligible for later insertion. Permission still evaluates real calls; this finding establishes incorrect exposure, not a bypass of that permission engine.

### R6 — Accepted hook event spellings silently disappear (high)

The public hook builder accepts `pre-tool-use`, `post-tool-use`, `session-start`, snake_case forms and `start` (`lib.rs:5914`). Turn materialization instead deserializes directly into a snake_case enum and silently drops failures (`capability/turn.rs:268`). Existing fixtures use hyphenated events.

Reproduction: a `pre-tool-use` hook present in the capability set yields no turn hook. The same builder also converts an invalid matcher into `None` with `.ok()`, potentially turning an intended selective hook into an all-tools hook. Validation must produce a typed error before publishing readiness, never silently skip or broaden a hook.

### R7 — A project-disabled inherited hook remains enabled (high)

Config merges hooks by full-value equality, not stable identity (`vak-config/src/lib.rs:3256`). Shared enabled and project disabled versions differ, so both survive. Provider declaration later removes disabled rows and keeps the enabled inherited row.

Reproduction: same `stop/true` hook in Shared with `enabled=true` and project with `enabled=false`; effective hooks still contain an enabled hook. Category-wide `inherit_hooks=false` works, but it is not a substitute for disabling one inherited item.

### R8 — Command discovery bypasses installed/enabled plugin state (medium)

`custom_commands::discover_with_plugins` scans `.vak/plugins/*/commands` directly before consulting supplied enabled package roots (`custom_commands.rs:29`). An unmanaged directory contributes commands even when the enabled plugin list is empty.

Reproduction: create such a directory without a registry entry; its command is discovered. This is another source contract alongside the installed plugin store, contrary to “one canonical way.”

### R9 — Plugin commands beat project commands on collisions (medium)

Command discovery inserts plugin entries first, project entries later, then stable-sorts and keeps the first equal name (`custom_commands.rs:54–70`). Its stated project precedence is reversed for this collision.

Reproduction: enabled package and project both define `review`; the surviving source is the plugin.

## Additional source-confirmed integration gaps

These were traced in code but were not exercised end-to-end with external integrations.

- **Revocation is not wired to dispatch.** `CapabilityRegistry::revoke/revocation/revoked_ids` exist, but workspace-wide call-site search finds no production dispatch consumer of the revocation queries and no production caller of this registry's revoke operation. AgentConfig carries no such authority. Already-created tools and hooks retain old state. Permission-mode cancellation is a separate mechanism and does not prove capability revocation works.
- **The promised shared projection is not connected.** `Binding`, `CapabilityReport::build`, and `standing_section` are exported and tested, but their consumers found in the Rust tree are tests, not turn binding, doctor, or admin wiring. Existing diagnostics instead re-read Core discovery/reach. The registry also omits disabled and shadowed entries before reporting, losing their reasons and provenance.
- **MCP notifications do not force reprobe.** Core clears its inventory and emits `ServerAnnounced`; the reconcile loop consumes hints only as wakeups and never invokes `mark_due`. A ready resolution can therefore skip the notified probe until TTL. `provider::probe` infers `announces_changes` from connectivity, not the negotiated listChanged capability. Changing server configuration under the same ID also carries old readiness/backoff, because resolution state is keyed only by ID.
- **Children and flows bypass full assembly.** Task dependencies are cloned before the parent's final tool filtering (`lib.rs:4542`). Child AgentConfig omits hooks and MCP aliases (`vak-agent/src/task.rs:411`), while its prompt/skill/MCP descriptors derive from the original contract. Managed flow dependencies use `agent_tools()` with `system_prompt()` (`vak-core/src/lib.rs:189`), which can advertise optional integrations absent from its tools. Their desired narrower authority should be resolved explicitly by the same plan builder, not emerge from missing fields.
- **Skill invocation has two temporal contracts.** The model's skill loader uses current selected skill digests, but explicit `/skill:` expansion closes over the old session descriptors (`lib.rs:4251`, `:5897`). Editing a skill can make explicit invocation fail with “start a new session” while the ordinary loader uses the new version. Command expansion similarly reads old templates and does not consume a turn-selected command set.
- **Plugin inheritance is package-inconsistent.** MCP uses first-wins per qualified server key; skills de-duplicate by skill name; commands follow their own precedence; hooks append every enabled package's hooks from both roots. The same package name in Shared and project is not resolved once as a unit. Disabling a project package does not supply a tombstone suppressing its enabled Shared counterpart. `inherit_mcp/hooks/skills/commands` mainly controls standalone sources; inherited plugin contributions are instead controlled through `inherit_plugins`. These semantics need an explicit product contract and a matrix test, rather than an assumption that all switches act alike.
- **Provenance is inaccurate.** MCP declarations label non-plugin servers Workspace and plugin servers workspace-scoped even when inherited. Hooks are always Workspace. Standalone skill discovery does not populate user/project provenance, so the fallback becomes Workspace. A reliable “why active?” explanation cannot be recovered after these distinctions are discarded.
- **MCP discovery is broad and changes schemas mid-turn.** `McpTool::list` connects to every manager server before filtering returned tools (`vak-mcp/src/tool.rs:130`), including turn-excluded servers. Its observer mutates alias definitions. The broker description says never call direct names while Agent explicitly advertises them as aliases. This is a contradictory interface and exposes the model to two invocation routes.
- **No persisted exact per-turn capability request contract was found.** The session header stores initial prompt/capabilities, while Agent dispatches a freshly rendered system prompt and potentially changing alias schemas. No capability-epoch/request-schema entry was found in `EntryPayload`. Work receipts record provider work, not a reconstructable copy of this changing capability interface. Exact model-visible replay needs a persisted turn/step capability record.

## Greenfield replacement: one authority, several projections

Introduce one owner under `vak-core::capability` with one admission API:

```rust
prepare_turn(TurnContext) -> Result<PreparedTurn, AdmissionError>
```

`TurnContext` carries workspace/trust, surface/bot/chat/role, session ID, scoped pins, actual approver, parent delegation ceiling and current request intent. `PreparedTurn` contains the immutable capability plan plus constructed execution handles. Callers cannot append schemas, hooks or loaders afterward. Agent accepts this prepared interface rather than independent optional lists.

### Resolve source intent before capability selection

Collect raw declarations with stable IDs, package generation, source layer and explicit enabled/disabled state. Keep invalid, disabled and shadowed candidates as diagnostic rows. Resolve Shared → project once, then expand the winning enabled plugin generation once. A disabled narrower item must remain a tombstone; removing it restores inheritance. Do not let a disabled project plugin fall through to the Shared copy.

Use typed category-specific merge operators owned by this resolver: replacement/tombstones for named items; explicit stable hook identity and order; absent allowlist inherits, empty allowlist blocks; deny lists accumulate. Preserve the existing documented bot/chat allow override convention and `inherit_bot_policy` semantics deliberately rather than silently changing them into intersection. Permission caps and delegated execution ceilings still intersect and cannot widen authority. If the product instead wants bot allowlists to be immutable ceilings, make that an explicit contract change.

Define category switches uniformly: recommended semantics are that `inherit_<kind>=false` excludes all broader contributions of that kind, including plugin contributions; `inherit_plugins=false` excludes all broader packages. This differs from current behavior and requires a deliberate schema/version decision, not a reinterpretation of existing fields within the current major.

### Separate state dimensions

One `usable` boolean is insufficient. Each resolved item needs separate fields for source enablement, validity, readiness, policy eligibility, turn selection, advertisement and invocation route, plus a reason/provenance chain. An enabled MCP server waiting for a probe is discoverable as pending, not callable as a made-up tool. A domain-withheld tool is available for safe disclosure, not permission-denied. A hook is an automatically triggered execution plan, not a function the model should choose to call. A plugin owns contributions; it is not itself a callable tool.

### Publish consistent observations and bind them

The reconciler owns validated declarations and the last valid catalog together. Key probe state by identity plus configuration/generation fingerprint. Preserve successful catalogs between probes; store failures separately with retry and freshness. Publish static declarations immediately and probe optional integrations outside the turn-admission critical path. Serialize publication or compare source revisions so an older concurrent pass cannot overwrite a newer one. Hints invalidate the named observation; the periodic scan remains the correctness backstop.

Bind the latest eligible snapshot at each user-turn boundary. The session route/security contract stays separate from the live capability inventory. New authorized additions appear without rotating or rewriting the session. Immediate revocation is a live broker check shared by built-ins, MCP, skills, commands and hooks; it also cancels affected pending/in-flight work where applicable. It is rechecked after an approval wait.

### Produce the entire model interface once

From the bound snapshot, apply scope policy, actual execution authority, parent ceiling and intent narrowing. Compute the selected typed IDs once. Materialize tool schemas and routes, skill loader/name list, command templates, hook plan and prompt sections from those same IDs. Structural dependencies are explicit: admitting a skill requires its loader; admitting an MCP tool requires its broker route. No raw-config reads or second policy assembly are permitted in these projections.

For MCP choose one canonical model API. Recommended: a stable discovery/call broker with server-qualified tool identities and exact schemas returned on demand; delete compatibility aliases. Keep a concise eligible catalog so the model knows relevant integrations exist. Discovery searches only authorized candidates and retains original per-tool policy at call time.

Intent should primarily rank initial disclosure. A stable discovery tool may disclose additional eligible capabilities within the same authority ceiling when a request evolves. Record any new disclosure before the next model step. A classifier error should never require the model to guess the name of a tool that was hidden completely. Explicitly requested skills/commands resolve through this same plan, with the same generation and authorization as automatic invocation.

Persist an additive `TurnCapabilitiesBound` record and, if disclosure changes, a step-scoped update before dispatch. Include epoch, source revisions, selected/excluded IDs and reasons, schema/content hashes and the exact rendered model-visible content or durable content-addressed references. Secret values are excluded. Replay must reconstruct actual system text and tool schemas, not just the old header.

Doctor/admin explain the same resolved inventory; a selected turn view explains its additional narrowing. Display the deciding source and distinguish “disabled,” “shadowed,” “waiting for discovery,” “denied,” “approval unavailable,” and “not disclosed for this request.”

## Delivery sequence and acceptance criteria

1. Convert the nine reproductions into normal regression tests while fixing the immediate failures. Add coverage for the wildcard replacement before changing MCP server admission.
2. Introduce the typed source resolver, stable provenance and dependency edges; remove standalone competing discovery/merge implementations in the same changes.
3. Make PreparedTurn own every schema and execution path; move prompt rendering after selection. Route child agents and flow/eval paths through it with explicit scoped ceilings.
4. Wire live invalidation/revocation and append-only turn binding; separate frozen routes from live capability generations without redefining existing stored fields. Respect the additive-major-version contract; use new entries/types and reject unsupported schemas.
5. Delete obsolete alias assembly, old skill/command expansion, direct descriptor fallback, unused Binding/report alternatives and independent hook parser. Keeping them as parallel compatibility routes would recreate the problem.

Completion requires provider-capture and real broker tests across fresh/resumed sessions, Shared/project collisions and tombstones, trusted/untrusted projects, bots/chats, children/flows, enabled/disabled items, MCP cold/ready/failed/recovered states, same-ID configuration changes, catalog notifications, cancellation and mid-turn revocation. Include two different workspaces sharing the daemon; preserve the known CorePool busy-steering regression test while resolving refresh.

For every request assert that advertised callable identities equal the bound dispatch identities; excluded entries have stable reasons; authorized discovery is the only way to grow disclosure; hooks execute from the same plan; replay reproduces the request. Then use a controlled live-model task suite to measure discovery, skill loading and successful tool use. A live model failing to select a correctly exposed tool is a separate diagnosis from a harness that never sent it the tool.
