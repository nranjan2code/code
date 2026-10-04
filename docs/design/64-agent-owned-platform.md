# 64 — Agent-owned platform

Status: **implemented and audited — September 13, 2026; Agent/channel admission, lifecycle-gated ownership, default local session ownership, audience scope, idempotent request receipts, bot-scoped delivery provenance, scheduled-task Agent ownership, and browser verification landed. Optional live-provider/NATS tests remain environment-dependent.**

The product presents a small, calm model:

```text
Choose an Agent → talk → receive a useful result
```

An Agent is the durable owner of work. A user may create an Agent, talk to
it, connect it to a channel, and inspect its conversations and work. Bots,
external addresses, permissions, sessions, tool workers, and delivery packets
remain implementation details in Settings and the Operations Center.

This is a deliberately breaking replacement of the profile-only and
route-only models, shipped in a new major version with updated baseline guards
and invariant 29. Existing 3.x fields must not be redefined in place. No old-shape
runtime reader or migration is required. Unsupported stores are identified
before dispatch or mutation, with an explicit fresh-start procedure. Refusal
must leave old ledgers untouched: no automatic purge, rewrite, or inferred
deletion. An operator may export/backup using the prior release and select a
fresh data location or explicitly reset. The schema, API, UI, and all admission
paths are now cut over; this document is the authoritative contract. Document
63 is retained only as historical context and is not a runtime contract.

## Product vocabulary

| User-facing term | Meaning | Runtime owner |
| --- | --- | --- |
| Agent | A persistent specialist that owns instructions, conversations, memory boundary, restricted policy, and optional automations/channels. | `AgentDefinition` |
| Conversation | A private ongoing exchange with one Agent. | `ConversationKey` + root session lineage |
| Channel | An optional way to reach an Agent or receive its outcome. | Endpoint access/configuration and runtime binding |
| Bot | A configured transport credential, such as a Telegram bot. It is not an Agent. | `Bot` |

The interface never asks a normal user to choose a model, session, sandbox,
flow, task, bot token, or permission engine. Those are Details, Settings, or
Admin concerns.

## Runtime model

```text
Inbound surface
  → Bot (transport credential)
  → Endpoint authorization (address + trust + Agent target)
  → Authenticated sender and authorized audience
  → ConversationKey (workspace, Agent, audience, conversation)
  → root session / append-only ledger
  → Core admission + Agent loop
  → typed output projection
  → frozen origin destination, subject to current access checks
```

Desktop, web, and voice derive actors from authenticated server context, never
client-supplied owner IDs. Account linking requires verification of both
accounts and proves identity only. History sharing, conversation sharing, and
execution authority are separate grants. A remote endpoint never inherits
desktop permissions through linking. Groups have an audience identity and
separate individual sender attribution. Membership revocation/unlinking removes
access immediately, including cached grants; already disclosed data cannot be
recalled. Sharing an Agent does not imply sharing a conversation or its history.

### Surface conversation ownership

The authenticated local client conversation is shared by desktop and web:
both use the canonical `ConversationKey` for the Agent and local audience, so
switching between these two clients resumes the same ledger. Remote channels
such as Telegram, Discord, and Slack use a distinct conversation per
authorized audience (and bot where bot identity is part of the channel key).
Attaching a channel to an Agent grants it access to that Agent's identity and
admitted capabilities; it does not merge channel messages into the local
conversation or expose local history. Any future cross-surface history sharing
must be an explicit audience grant enforced at admission, reads, search,
memory, and delivery.

Web and desktop may disconnect while a local turn continues on the server.
The session remains the durable home for its result and pending local HIL
gates; reconnecting restores the transcript and any still-live approval cards.
The admin approval-forwarding policy remains explicit for connected-chat
runs, which can route a gate to an assigned, authorized channel.

Session discovery returns the Agent, `ConversationKey`, audience, and origin
as one summary. The local Agent picker may resume only the canonical local
conversation, never the most recently updated channel conversation. Channel
history remains available through the authorized history surface with its
transport and audience provenance. Every reply follows the immutable
request-origin destination recorded at admission.

## Durable records

All identifiers are stable UUID/slug values, not display names. New stores are
versioned and additive within the new major baseline. The shapes below describe
logical responsibilities, not a requirement to create a database for each one.
Reuse the session ledger, capability registry, scheduler, and delivery outbox.

```rust
AgentDefinition {
    id, identity_revision, policy_revision, name, lifecycle,
    identity,                    // personality, behaviour, responsibilities
    policy_ceiling,              // narrowing-only
    memory_scope,
    automation_policy,
}

EndpointAccess {
    id, status, bot_id?, transport, external_address,
    observed_sender, review_evidence,
    configuration?,              // required and validated before activation
}

EndpointConfiguration {
    workspace_id, agent_selection, // explicit override or inherit
    audience_id, route_override?, policy_ceiling?, delivery_preferences,
}

ConversationKey {
    workspace_id, agent_id, audience_id, conversation_id,
}
```

Desired routing configuration has one canonical owner. Pending endpoints can
exist before workspace/Agent/audience assignment; approval validates and writes
those assignments atomically. Runtime session pointers and admission locks are
separate responsibilities and hold no competing desired overrides.

Agents retain Shared/user → trusted project layering. Project records store
intent only, edit GETs return exactly the layer their PUT replaces, and effective
reads report provenance. Untrusted project definitions cannot be admitted.
Secrets remain recipient-scoped credential references outside Agent data.

`SessionHeader.agent` is mandatory for every newly admitted session. Vak has a
reserved built-in ID and default definition; its effective instructions still
honor editable prompt layers and the growing guardrail floor. Every newly
admitted session header
also carries its `ConversationKey` and origin; a run can therefore be audited
without reverse-engineering a gateway binding. Each admitted request also logs
its own sender, audience, endpoint revision, origin, and policy provenance:
one session can receive requests through different authorized endpoints.

## Authority and privacy

An Agent policy can only narrow the workspace baseline: remove a capability,
lower budget/turn/dispatch limits, constrain a route ladder to a prefix, or
raise approval requirements. It cannot add a tool, credential, model,
workspace, budget, or permission.

Memory is explicitly scoped:

1. `conversation_private` — that conversation only.
2. `agent_private` — the same Agent AND authorized owner/audience, plus an
   explicitly authorized child; not every person using that Agent.
3. `workspace_shared` — deliberately published knowledge with authorized
   recipient audiences, not automatic access for every external caller.
4. `user_global` — user-owned preferences with explicit audience/surface grants;
   account linking alone does not expose them.
5. `delegation_packet` — a bounded, logged handoff to a child.

Private records carry owner/audience and workspace labels as well as their
Agent/conversation scope. Search defaults to the first two scopes within the
caller's authorized audience. A result from another Agent is
unavailable unless it was explicitly promoted to `workspace_shared` or placed
in a delegation packet. Publishing private data requires authority to disclose
it; relabeling cannot bypass access checks. Cross-Agent and cross-principal
sharing are separate permissions.

One source access predicate covers transcript APIs, indexes/search, memory
writes, reflection, compaction/retrieval, delegation, artifacts, exports,
commitment views, UI/SSE, and delivery. Authorization precedes model context
assembly, and admitted context remains logged. Indexes cannot weaken source
authorization. Filesystem/Bash/MCP must not bypass these controls: private
ledgers/artifacts cannot sit in a shared tool-readable workspace while search
filters claim to isolate them. Separate audiences need enforced storage and
execution boundaries. FullAccess/shared filesystem grants remain explicit human
trust decisions; do not promise confidentiality those grants defeat.

An Agent may create an ephemeral child run under its frozen limits. It may not
create a persistent Agent, attach a Bot, approve an endpoint, link external
identities, widen its authority, or re-route a person to another Agent. Those
are user/operator actions.

## Agent workspace and execution scratch topology

This section describes the 4.x tree as built. The planned tenant/space layout
replaces it in the data architecture refactor (docs/plans/data-architecture-plan.md,
M3b); until then these are the real paths. `vak_config::paths` is the only
place that resolves them, and `vak_core::state::REGISTRY` declares every
durable file.

Two roots hold an Agent's things, and they are different directories:

- **The data home** (`vak_config::paths::data_home`: `~/Library/Application
  Support/vak` on macOS, `~/.local/share/vak` on Linux, `VAK_HOME` when set)
  holds application state. An Agent's private state lives under
  `<data home>/agents/<agent_id>/` (`vak_config::paths::agent_home`); what
  every Agent shares sits at the top of the data home
  (`Core::shared_data_home()`).
- **The workspace** is the directory an Agent's file and shell tools work in
  (`vak_config::paths::agent_workspace`). The built-in `vak` Agent works in
  the base workspace itself (`~/vak-home` for services, or the directory a
  person opened); any other Agent works in
  `<workspace>/.vak/agents/<agent_id>/workspace/`, so Agents never see each
  other's files.

```text
<data home>/
├── agents/
│   └── <agent_id>/                    # vak_config::paths::agent_home; vak included
│       ├── sessions/<cwd-hash>/<session-id>/   # a ledger: record segments
│       ├── sandbox/                   # records, candidates, executions (D25)
│       └── coworking/                 # grants
├── tenants/<tenant>/
│   ├── store/                         # objects and refs: ledger payloads and
│   │                                  # Documents (memory, entities, skill
│   │                                  # proposals, Office rooms, presentations)
│   ├── keys/                          # revocations; the KEKs are in the credential store
│   ├── auth/                          # the owner's passkeys and recovery digests
│   └── feeds/, feeds.toml
├── gateway/                           # allowlist.json, bots.json, bindings, deliveries
├── operations/                        # incidents, actions (record chains)
├── cost-log/                          # the FinOps ledger (a record chain)
├── inbox/                             # a record chain
├── tasks.json                         # scheduled work: vak_core::tasks::TaskDef
├── archive.json                       # archived sessions (hidden from the sidebar)
└── deleted.json                       # the trash (vak_core::trash)
<cache home>/store.db                  # the rebuildable FTS index over every ledger

<workspace>/
├── .vak/config.toml                   # the project layer
├── .vak/agents.json                   # Agent definitions for this workspace
├── .vak/agents/<agent_id>/workspace/  # a non-built-in Agent's workspace
└── .vak/scratch/<agent_id>/           # execution runtime state (invariant 35)
```

1. **Private Agent state (`vak_config::paths::agent_home`)**:
   - `Core::sessions_home()` resolves through `self.agent_identity` to `<data home>/agents/<agent_id>/`.
   - Each Agent's session logs, memory notes (`append_note`), reflection entries and skill proposals stay isolated.
   - Admission locks are acquired per-agent and per-session, ensuring that turns running on one Agent never block or stall turns running on another Agent.

2. **Execution Scratch (`.vak/scratch/<agent_id>/`)**:
   - `BashTool` works in the workspace; its runtime state is partitioned per Agent: temp files under `<cwd>/.vak/scratch/<agent_id>/<execution-id>/tmp`, caches under `<cwd>/.vak/scratch/<agent_id>/cache` (AGENTS.md invariant 35).
   - Concurrent tasks spawned by different Agents in the same workspace never share temp files or caches.
   - Promotion manifests (`CandidateManifest`) and diff viewers review candidates only from an execution that ran inside the agent-scoped scratch path.

3. **Global Shared Infrastructure (`Core::shared_data_home()`)**:
   - Cross-agent services use the top of the data home through `shared_data_home()`.
   - This encompasses channel routing and transport credentials (`gateway/allowlist.json`, `gateway/bots.json`), operational receipts (`operations/`), the FinOps ledger (`cost-log/`), the inbox, scheduled tasks (`tasks.json`), the archive and the trash. The full-text search index is in the cache home.

4. **Client Presentation and Workbench Isolation (`vak-client-ui`)**:
   - The desktop and web UI maintains `sessionWorkbenchMap: Record<string, WorkbenchExecution[]>`, keying execution events by session ID rather than a single flat global array.
   - Live SSE execution events for background agents continue streaming and populating their respective views when the user switches tabs or focuses another conversation.
   - `openAgentChat` serializes concurrent admission requests with a mutual exclusion gate, guaranteeing no user request is dropped or answered with `null` during agent switching.

## Resolution and rotation

An endpoint's target resolves exactly once per admission:

```text
endpoint Agent override → bot default Agent → workspace default Agent (Vak)
```

Resolve the Agent in the destination workspace, never the active UI workspace.
Missing/paused explicit targets fail visibly without fallback to Vak. Preserve
inheritance choices and resolution provenance rather than copying defaults.

Identity and instructions are read every turn, so an edit to a saved Agent
reaches the next turn of every conversation it owns, with no rotation and no
re-admission; a turn already running keeps what it started with. The session
header keeps the identity as admitted, for display and audit, and each turn's
`TurnCapabilitiesBound` records the prompt that turn sent. The revision rises
when any prompt-bearing field changes, instructions included.

Lifecycle is enforced on every turn as well as at admission. A turn for an
Agent whose saved definition is paused or archived is refused before it
does anything (`Core::refuse_inactive_agent`, error `AgentUnavailable`),
and the message says to set the Agent active again; doing so resumes the same
conversation. Enforcement is at the turn boundary only: a turn already
running is never cancelled, edited or re-identified by a change that arrives
while it runs, and the next turn sees it. Reasons: a turn's request is
append-only and byte-stable so the provider cache and replayed thinking stay
valid (invariant 36); checking the definition on every step of every running
turn would put a disk read on the hot path of each; and a pause is a
decision about the next piece of work, not a reason to discard half-done
work. A deleted Agent, or one defined in a layer the turn's `Core` cannot
read, is not refused, since that cannot be told from a deletion. An
immediate, push-based "stop what this Agent is doing" (cancel the Agent's
live handles when it is paused) would fit the server's session map without
polling; it is not built, and would be an explicit pause option, not a side
effect of saving.

Policy is live. Effective authority intersects workspace, Agent, Bot/endpoint,
audience, and run/delegation ceilings; no layer restores authority another
applicable ceiling removed. Restrictions cancel affected main/child runs and
reject pending gates before being reported effective. Capability additions
reconcile at the next turn boundary; revocations fail closed immediately.
Capability changes require neither restart nor session rotation, per invariant
31. Per-turn records log the actual policy, inventory, prompt, and epoch used.

Workspace/Agent/audience reassignment cannot transfer old private history.
Cancel/drain affected work under its old contract and separately admit future
input. Route changes create new frozen execution sessions while preserving the
visible conversation only within its authorized audience. Atomic cross-process
admission prevents duplicate roots and competing writers.

Agent pause cancels its runs, schedules, and descendants; endpoint pause affects
work authorized through that endpoint. Resume never automatically replays
cancelled effects. Denial remains sticky. Archive disables admission and retains
evidence. Removal requires explicit disposition of endpoint, schedule, and
outbox references; dangling references fail visibly.

There is one resolver used by desktop, CLI, TUI, voice, gateway, schedules,
heartbeats, flows, best-of-N, and workers:

```text
resolve_agent(workspace, agent_id, endpoint?)
  → effective narrowing policy
  → identity + prompt layers
  → admitted route/capability/budget contract
```

No caller may inject an Agent's instructions directly into a system prompt.
Resolution feeds existing authorization, budget, and broker machinery; it does
not replace those checks. Ordinary internal children carry parent Agent
ownership; explicit specialist delegation freezes the chosen Agent identity.
Both receive bounded authorized context and intersect parent/child limits.
Delegation cannot reset shared spend accounting. Schedules record Agent,
workspace, audience, and approved recipients. System maintenance has an explicit
system identity. Background results enter a visible Agent conversation only
through an authorized result projection.

## Request durability and delivery

Record request ID, Agent/conversation/audience, sender, configuration revision,
and exact origin durably before acknowledging acceptance. Duplicate IDs return
the original admission. Busy input is queued durably or explicitly rejected;
a new request must never be acknowledged and discarded. Preserve ordering and
ingress restrictions through steering/follow-ups: requests with different
authority cannot share a broader running lease silently.

Each response carries a frozen destination envelope: transport, external
address, bot identity, audience, and request correlation. Binding edits cannot
redirect in-flight answers or retries. Recheck current access/revocation before
sending; hold denied delivery with an actionable reason. Resolve credentials
only for the designated bot; never fall back to another bot on failure.

Reuse the durable outbox, ordered chunks, degradation diagnostics, and receipts.
Track execution completion separately from delivery completion. Recover accepted
input and pending output after restart; reconcile uncertain effects before
replay. Do not promise exactly-once effects where transports lack idempotency.
Cancellation retains partial output, and failed/limited/unsatisfied work must
not become successful merely because the model produced text.

## UX and administration

The sidebar centers on Agents. Opening one resumes the user's continuous
conversation for the workspace, idempotently across clients. Separate external
conversations and internal runs remain under history/details. Do not introduce
“New conversation” as a primary action. Creating an Agent offers an optional
next action: connect a Channel. One gateway serves multiple Agents and Bots;
connecting an Agent creates no separate gateway process.

Agent settings provide definitions, channel setup/access review, memory sharing,
and automations. Credentials and advanced policy are progressively disclosed.
Admin offers the same canonical APIs with richer fleet/diagnostic views;
personal users need not visit Admin for routine setup. The Operations Center projects Agent, conversation,
endpoint, Bot, and run provenance from real records; it never manufactures a
dashboard-only state.

**Voice follows the same ownership rule.** A conversation's Agent `Core`
provides its provider, discovered transcription model and discovered
synthesis model. A connected Channel can narrow its voice settings for that
endpoint, and a preview can supply a temporary audition override. The session
Core remains authoritative for the Agent and workspace; a client changing its
currently selected Agent cannot retarget a live voice conversation. Voice
provider keys are shared account credentials in the canonical credential
store, not copied into Agent data. Settings and Admin expose the same discovered
model choices and voice route, while the shared FinOps ledger records hosted
voice requests with unknown cost when providers return no rated usage.

## Replacement plan (completed)

1. Inventory every ingress/store and define core records, access predicates,
   layering, lifecycle transitions, and new-major refusal guards, with tests.
2. Make Agent and ConversationKey mandatory in new session headers; update
   every admission path in the same change.
3. Replace duplicated desired routing in `AllowlistEntry`/`ChannelBinding`,
   retaining access-review and runtime-binding responsibilities. Add atomic
   admission, immutable request origin, and frozen delivery destinations.
4. Partition memory/index/query APIs by scope before enabling multi-Agent
   endpoint routing.
5. Convert schedules, heartbeat, flow, best-of-N, and delegated runs to named
   Agent ownership; remove prompt-only helper injection.
6. Update sidebar, Agent settings, history, channel setup, CLI/TUI, Admin, and
   Operations against the same APIs; build bundled frontends with the server.
7. Remove superseded schemas, endpoints, resolver branches, and labels at the
   coordinated release cutover. Transfer useful regression assertions to
   replacement tests and mark historical documents superseded. Do not delete
   evidence or retain runtime aliases/old-file readers.
8. Verify fresh installation and unsupported-store refusal without mutation;
   update baseline/release documentation. No partially converted path ships.

## Required end-to-end tests

- Desktop, Telegram, and voice reach the chosen Agent with correct destination
  workspace/layer provenance. Linking accounts alone shares no history or authority.
- Two different principals using the same Bot and Agent receive different
  conversations and cannot retrieve one another's private memory.
- A binding target change rotates only that binding's conversation and leaves
  its former ledger immutable.
- An Agent policy can narrow every admitted surface but cannot widen any
  workspace, channel, or session authority.
- A paused/revoked Agent or endpoint stops active work and rejects new input.
- Scheduled, child, flow, and gateway runs record the same mandatory Agent
  identity and provenance fields.
- Each response returns through the origin binding with a durable delivery
  receipt, including partial/failed outcomes.

Additional release gates require code/test evidence for each boundary:

| Boundary | Required cases |
| --- | --- |
| Privacy | Same Agent/different audiences; group sender attribution; unlink/member revocation; transcript, raw-file, artifact, export, reflection, and SSE isolation |
| Live policy | Immediate main/child/gate revocation; additions next turn without restart/rotation; no widening across any ceiling |
| Admission | Racing opens, duplicate retries, ordered busy input, restart recovery, mixed-ingress steering authority |
| Delivery | Binding edit during execution/retry, bot-specific credentials, audience revocation, chunks, timeout/disconnect, failed and partial outcomes |
| Execution | Permission engine, broker, sanitized credentials, sandbox/workspace restrictions, hooks/MCP and forwarded approvals on every path |
| Reliability and FinOps | Frozen ladder, circuit breaker/endurance, cancellation, dispatch limits, aggregate parent/child/schedule accounting |
| Durable work | Runtime criteria/commitments, checkpoint deltas, tool receipts, partial preservation, uncertain-effect reconciliation |
| Background/distributed | Ownership, Agent pause/archive, descendant cancellation, restart recovery, authorized result projection |
| Presentation/voice | Agent switching, attachments, reconnect/hydration, typed outcomes, no internal scaffolding in ordinary prose |
| Release | Workspace compilation/tests, production frontend builds, fresh-install smoke test, unsupported files unchanged on refusal |

Run complete ingress → authorization → execution → evidence → delivery scenarios
for local chat, external chat, voice, scheduled work, and delegation, including
denied/interrupted cases. Browser verification covers Agent opening, settings,
channel review, and results. Missing external dependencies remain explicit
unverified items, never recorded as passed.
