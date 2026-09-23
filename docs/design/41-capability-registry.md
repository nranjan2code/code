# Capability registry
Status: implemented in 2.0.0

VAK treats every extension as a typed capability: tool, skill, MCP server,
hook, command. One registry owns all five, one reconcile loop keeps them
current, and one projection describes them to the model, the operator, and
the admin console.

The design constraint that shapes everything here is **uptime**. vak is not a
process that runs one turn and exits; it is a daemon with gateways attached,
and a session can stay alive for weeks while skills are edited, plugins are
toggled, and MCP servers come and go. Two things follow, and they are
requirements rather than preferences:

* **Nothing may require a process restart.** A server that was down at boot
  must rejoin on its own; a skill added today must be usable today.
* **No session is ever rotated.** A conversation that has been running for
  three weeks picks up new capabilities in place, keeping its ledger and its
  history.

## Why the previous design failed

The old registry was correct for a process-per-session lifetime and was never
re-derived when vak grew into a daemon. Its every mechanism was
**edge-triggered**: discovery was warmed once, a prompt was frozen once, a
connection was opened once, and a catalog was cached once — with a retry
guard (`inventory.is_none()`) written so that caching a *failure* counted as
having succeeded. There was no loop behind any of it, so a single missed edge
was permanent, and uptime converted small races into dead integrations.

The concrete failure it produced: a configured, connected search server was
invisible to a question about the weather. The prompt froze before anything
triggered discovery, so the session advertised server names with no catalog;
a failed probe was stored as a catalog containing a tool literally named
`error`, which then blocked every retry; and the per-turn slice consulted a
static table of built-in *tool names* that could not mention a capability the
operator had installed. Nothing errored. The agent answered from memory and
sounded certain, and `doctor` was green throughout, because the diagnostics
explaining the failure were rendered only into the system prompt.

The old escape hatch for long-running surfaces — "gateway bindings compare
the frozen packet and rotate stale sessions" — was documented here but never
built, and rotation is ruled out regardless.

## The model

A capability has an identity (`kind:name`), an origin, a declaration, a
resolution, and a standing. Admission is a **ceiling**, not a floor: three
layers narrow it per turn and none may add — channel visibility, `reach`, and
the intent kernel's capability slice.

| Kind | Invocation | Runtime boundary |
| --- | --- | --- |
| Tool | model tool call | broker, permission engine, hooks, sandbox |
| Skill | `skill({name})` or `/skill:name` | read-only loader, admitted digest |
| MCP server | `mcp` broker | pooled connection, server/tool policy |
| Hook | automatic lifecycle event | isolated hook runner and failure mode |
| Command | user slash command | host expansion before model dispatch |

A skill is still a document, and `skill` is the only model-callable loader; a
skill name is never registered as a tool name. Hooks and commands are not
tool-shaped. Plugin contributions use the same kinds and carry plugin
provenance rather than creating a parallel execution path.

## Declare: a capability says what it serves

Each capability classifies itself against a shared vocabulary of **domains** —
`live-data`, `web`, `filesystem`, `code-exec`, `memory`, `messaging`,
`documents`, `orchestration`, `vcs`, `observability`, plus `Custom(String)`
for a name this build has never heard of.

Built-in tools declare domains beside their schema, skills in `serves:`
frontmatter, MCP servers in `serves` under `[mcp.servers.<name>]`, plugins per
contribution. The intent kernel derives *required domains* from the reading's
own axes and the slice is a set intersection.

The distinction from the table this replaces is the one that matters: the
host defines a **vocabulary**, and capabilities are **data** classified
against it. Adding an integration never edits the harness. A table of
instance names could never satisfy that, which is why every new integration
used to need a code change, granted only after a user reported a wrong
answer.

Two rules keep it honest:

1. **Undeclared fails open.** A capability that declares nothing is never
   narrowed away. Slicing saves context; it does not enforce policy — `reach`
   and the permission engine do that — so the failure modes are asymmetric:
   an extra capability costs a little context, a missing one costs the task.
2. **Domains are never inferred.** An MCP server with no `serves` stays
   undeclared rather than being guessed at from its tool names. A keyword
   table in the harness would reintroduce exactly the coupling this design
   deletes.

## Resolve: declaration is offline; MCP is on demand

```
Available ──► Retired{reason}      (revoked)
```

A capability is usable when declared and not revoked. Nothing in the
registry talks to anything: a pass is a filesystem walk at worst, so turn
admission reconciles whenever the declared world changed and never waits.

MCP servers are started **only by demand** — a model's `mcp` call — never by
the registry, admission, or prompt assembly (AGENTS.md invariant 25). The
per-`Core` pool (`vak_mcp::McpManager`, so one per workspace in the server's
`CorePool`) spawns a server when a call needs it, reuses the connection for
every session, and shuts it down after `IDLE_TTL` unused; eviction never
triggers a respawn. What demand teaches the pool — a server's catalog, or
why its last attempt failed — is recorded as a `ServerObservation`, declared
as that server's configuration, and published in the next epoch, so the
following turn's prompt names its tools with no restart. A failure is a
reason on a still-callable server, never a catalog entry and never a callable
name; repeated demand inside the pool's exponential backoff gets the recorded
reason back instead of a fresh spawn, and the next demand after it retries.
A server that announces `notifications/tools/list_changed` has its catalog
forgotten, to be re-learned on its next use.

The registry used to probe every configured server on a background timer —
spawning servers nobody had asked for, re-spawning them right after the pool
evicted them for idleness — and admission waited up to five seconds for
those probes. Both are gone.

## Reconcile: level-triggered, not edge-triggered

One loop owns all change. It selects over a ticker and a debounced hint
channel; on either, it re-scans, diffs, and publishes if anything moved.
Hints — a filesystem event, an MCP `notifications/tools/list_changed`, a
plugin toggle, a config write — only make it run *sooner*. The ticker makes
it run anyway.

Every pass is idempotent, so a dropped hint costs one tick of latency and
never costs correctness. This is the property that makes multi-week uptime
safe, and its absence is what made every defect above permanent.

## Admit: turn-atomic epochs replace rotation

The registry publishes immutable, versioned `CapabilitySet` snapshots. The
**turn** — not the session — binds one, and holds it start to finish, so a
plan formed in step one cannot have its tools change by step four.

Between turns a session re-binds to the current epoch and appends a
`CapabilityEpochChanged` entry naming the delta; the model is told, once, in
its standing section. This is what makes rotation unnecessary rather than
merely forbidden.

The epoch moves only when the world actually differs. The set digest covers
identity, usability, content, declared domains and configuration. An MCP
server's observation carries only its catalog and failure *reason*, never
attempt counts or times, so repeated failures for one cause do not churn
every live session's prompt.

Audit is strengthened, not weakened. Previously you could reconstruct what a
session was *born* with; now every turn records the epoch it ran at, and
every transition is a ledger entry, so you can reconstruct what each turn
actually saw.

## Narrow and widen: two channels, deliberately asymmetric

| Change | Direction | Applies |
| --- | --- | --- |
| Skill / server / plugin added | widens | next turn |
| Catalog or skill body updated | may widen | next turn |
| Removed at source | narrows | next turn, announced |
| Operator disable · trust withdrawn · policy deny | narrows | **immediately**, fail-closed |
| Server became unhealthy | reality | **immediately**, typed error |

Dispatch checks **availability** against the turn's bound epoch and
**authorization** against current policy. A revoked capability fails at the
gate even though the bound epoch still lists it. Revocation must not wait for
a long turn to finish; a newly added skill appearing halfway through a plan
would be a torn read.

## Running forever

Versioned snapshots leak if every version is kept, and over weeks of edits
that ends in an OOM — the one failure a no-restart system cannot absorb.

* Snapshots are `Arc`-refcounted, not retained: an old epoch lives exactly as
  long as the last turn holding it.
* Skill bodies are content-addressed and bounded, so an edited skill's old
  body stays resolvable for a turn still bound to its digest, then evicts.
* Retired capabilities are tombstones kept only while a live epoch references
  them.
* MCP connections carry a liveness check before reuse and an idle TTL, so a
  daemon neither answers through a dead subprocess nor accumulates every
  server it has ever touched.

The loop's own health is reported. "Capabilities look thin because
reconciliation has been failing for two hours" is a visible fact, not an
unexplained shortage of tools.

## Project: one value, three audiences

`registry.report()` returns every capability with kind, origin, declaration,
resolution, standing, and the reason it is or is not usable. The model's
standing section, `doctor`'s `capability health` check, and the admin console
render that same value.

This closes the gap that hid the original defect. `capability_diagnostics`
already knew a server was unreachable, but only the model was told; the
person who could repair the configuration saw nothing. Counts now come from
the effective set rather than raw config, so a plugin-contributed hook or
server is no longer invisible to the operator and present to the model.

## Invariants

1. The model may call only schemas from the tool set admitted by `Tool`
   entries in the bound epoch; one turn reuses one snapshot of those schemas.
2. Runtime objects not present in the bound epoch are filtered before a
   request is built.
3. Skill and MCP loaders may resolve only admitted entries.
4. Workers inherit the parent's bound epoch, narrowed to their actual tools
   and mode; they never rediscover a wider environment.
5. Unknown names and kind mismatches are structured error values. They never
   panic the loop and never trigger an effect before authorization.
6. **Capability changes take effect at the next turn boundary of every live
   session; revocations take effect immediately.** No session is rotated and
   no process is restarted to pick up a change. (This replaces the former
   "capability changes apply to new sessions", whose only escape hatches were
   restart and rotation.)
7. A host may add run-scoped direct-write paths. They are evaluated before a
   `write` or `edit` tool is dispatched and never depend on model compliance
   with a natural-language scope instruction.
8. A turn's reading never removes a capability. It only chooses which
   admitted tools are loaded; the rest are listed and one `find_tools` call
   away, and a tool that declares no domains is always loaded.
9. A failure is never data. No observation may introduce a callable name,
   and no recorded failure may prevent the next demand from retrying once
   the pool's backoff has passed.
10. Nothing but demand starts an MCP server: not declaration, not
    reconciliation, not admission, not prompt assembly.

## Layout

```
crates/vak-core/src/capability/
  domain.rs      Domain vocabulary and Serves; the anti-hardcoding boundary
  resolution.rs  Resolution: available or retired
  snapshot.rs    Capability, CapabilitySet, Epoch, Binding, CapabilityDelta
  registry.rs    CapabilityRegistry, the reconcile loop, revocation channel
  provider.rs    Core as CapabilityProvider; five kinds → one declaration set
  report.rs      CapabilityReport and the model's standing section
  turn.rs        TurnCapabilities, TurnProbe, and the single build() pipeline
                  covering channel → reach (+ revocation) for every kind
  surface.rs     ToolSurface: which admitted tools are loaded vs deferred,
                  and the stable tool catalogue the prompt lists
```

Tests: `crates/vak-core/tests/capability_lifecycle.rs` pins the behaviours
that failed silently — a down server is named and never faked, a recovered
server becomes usable with no restart, capabilities can be added, edited and
removed while running, a quiet world never churns the epoch, and an
undeclared capability survives every slice.

### Turn capabilities (`turn.rs`)

`TurnCapabilities::build` is the single admission point for every kind. It
runs two stages, and both are **policy**:

1. **Channel policy** — `ChannelPolicy` allow/deny globs for tools, MCP,
   skills, and hooks.
2. **Reach** — a capability the composed permission policy fully blocks is
   removed, keyed by the typed `CapabilityId` each `reach::Standing` carries
   (never by parsing its display label); so is one revoked since the last
   published epoch.

Admission never reads the turn's intent reading. Two stages that used to sit
here are gone. The frozen-contract filter was a no-op in production (the turn
passed no contract, and invariant 31 forbids one from hiding a capability
added mid-session), and the domain slice made a *prediction* into a
*policy*: a capability a confident reading excluded was uncallable for the
turn. Prediction now lives only in the surface below, where being wrong costs
one `find_tools` call.

Outputs: `tool_names`, `descriptors` (the prompt and ledger projection),
`mcp_tool_index` (bare MCP tool name → owning server, so the loop can repair
a call addressed by the bare name into an `mcp` broker call; names ambiguous
across servers or shadowing a built-in are left out), `mcp_server_names`,
`hooks`, `frozen_skills`, and `flow_admitted`.

Hooks are read by `vak_core::hook_def`, the same reader config validation
uses. A hook whose definition cannot be read is reported by
`Core::capability_diagnostics`; if it was advisory it is dropped, and if it
was declared fail-closed it becomes a `HookDef` with a `refusal`, which
blocks every tool call with the reason — a guard that cannot be read must not
quietly stop guarding.

### Tool surface (`surface.rs`)

Every admitted tool is callable. The surface decides only which are *loaded*
(full schema in the request) this turn, from each tool's own declaration on
the `vak_tools::Tool` trait — the harness keeps no table of tool names:

- `always_loaded()` — the orientation and discovery primitives (`read`,
  `glob`, `grep`, `find_tools`, `recall`, `skill`, `mcp`, `session_search`,
  `commitments`) are loaded on every turn;
- `serves()` — a tool is loaded when it serves a domain the reading requires;
  an undeclared tool is always loaded (deferring saves context, it is not
  policy);
- `presents_cards()` — a card tool is loaded when the request itself reads as
  that card (`presentation_tools::predicted_card_tools`, the same recipe
  detection the presentation check runs over the answer); otherwise it waits
  for `find_tools` or for the presentation-check nudge, which loads the tool
  it names for the redo.

Everything else is deferred: named in the prompt's "More tools" catalogue
(`tool_catalogue`: name and first sentence, sorted, a function of the
admitted set only, so it is byte-stable in the cached prefix), and loaded by
one `find_tools` call. A tool `find_tools` returns is promoted to a loaded
definition on every provider (`vak_agent::load_discovered`); on Anthropic legs
deferred tools additionally ride `defer_loading`. With
`[intent] slice_capabilities = false` (or the kernel off) everything is
loaded and there is no catalogue.

A deferred tool the reading did not predict and the model then used is the
measured misread (`misread::escalated_capability` over
`ToolSurface::unpredicted`); card tools are excluded, since a card is an
output choice rather than a reading of what the request needs.

Skills are listed once, in the prompt (name and description); the `skill`
tool carries only the admitted names as its schema enum and loads a body
on demand after checking its admitted digest. No skill body is inlined into
the prompt.

