# 73 — Data architecture: information model, lifecycle, tracing, index and cloud

Status: **proposal, revision 2 (2026-09-25); decisions locked (§13); plan in
`docs/plans/data-architecture-plan.md`; review fixes applied from
`docs/plans/data-architecture-review.md`.** Nothing here is shipped. §2 is an audit of
what the tree does today, taken from the source and from sizes (never
contents) of a real development data home; each defect names the file that
causes it and says whether it was reproduced or read from code.

`79-private-headless-fleet.md` is the proposal for one dedicated 24/7 VM per
customer, operator-blind management and disaster recovery. It adds a hosted
deployment trust boundary without changing the locked local-first decisions
or claiming that a later data milestone has started.

## 0. Why this document exists

A single developer's machine, ten days into the 4.x line, holds 1.83 GB of
Vak state spread across six roots. 1.6 GB of it is checkpoint manifests the
current code cannot read and nothing will ever delete. That is one person,
three Agents, no channels and a handful of schedules.

A customer install multiplies every axis at once: Agents × channels × chats ×
schedules × months, plus people sharing and revising artifacts, plus code
being written and run in sandboxes. At that scale four questions must each
have exactly one answer, and today none does:

1. **What is this file, who owns it, and what caused it?** (identity, tracing)
2. **How long does it live, and what removes it?** (lifecycle)
3. **How do I find everything about X?** (catalog, index, search)
4. **How does it leave this machine?** (backup, sync, cloud handoff)

## 1. GitHub or SharePoint?

Both. They answer different questions, and choosing one for everything is
the mistake. A third model covers what neither handles.

**The storage layer works like git.** Content is immutable and addressed by its
hash. History is append-only. The only mutable things are small named
pointers (refs). Sync means "send the objects the other side is missing, then
move a ref with compare-and-swap". Vak already believes this: invariants 1
and 2 make session ledgers append-only, the checkpoint store is
content-addressed, and Office drafts are candidates promoted by an atomic
rename. This model gives dedupe, integrity, offline work, cheap backup, and
cloud handoff almost for free. It is the right storage layer for
*everything*, including documents.

**What people see is organised like SharePoint.** People don't navigate
hashes or branches. They navigate *spaces organised by audience and purpose*,
find things by *metadata* rather than folder paths, inherit permissions from
the space unless someone deliberately breaks inheritance, see *version
history* rather than commits, comment and co-author, and are governed by
*retention labels*. That is the right model for how Agents, conversations,
artifacts and sharing are presented, permissioned and retained.

**Runs work like CI plus observability** (GitHub Actions + OpenTelemetry).
Every trigger (a message, a schedule slot, a channel request, a delegation, a
revision) produces a **Run** with one trace id, attempts, a decision
(including "skipped, because…"), linked logs and outputs, and retention by
policy. Neither a document library nor a repository models this.

What we deliberately do **not** take:

| From | Reject | Because |
|---|---|---|
| git | branches and merges as a *user* concept for documents | nobody merges a `.docx`; concurrent edits become sibling versions reviewed through the existing redline/Review path (doc 72) |
| git | mutable working tree as the record | the working tree is a *projection*; the record is the objects + ledgers |
| SharePoint | edit-in-place files | every save is a new immutable version; "current" is a ref |
| SharePoint | folders as the information architecture | folders are one facet; metadata (trace key, kind, audience) is the IA |
| SharePoint | site sprawl | spaces are created by admission rules, not ad hoc |

Where the content really *is* code, promotion targets a real git repository,
so at the edge it becomes an actual commit, branch or PR. **Git where the
content is code; a library where the content is documents; the same object
store underneath both.**

| Vak thing | Model |
|---|---|
| session ledger, commitments, promotions, audit, cost | git-style append-only record |
| files produced, attachments, drafts, checkpoints, evidence bodies | git-style content-addressed objects |
| Agents, spaces, conversations, artifacts, sharing, retention | SharePoint-style IA over those objects |
| turns, schedules, delegations, revisions, deliveries | CI/OTel-style runs and traces |
| changes to a code repository | real git at promotion time |

## 2. Today (audit, 2026-09-25)

### 2.1 Roots

| Root | Resolved by | Holds |
|---|---|---|
| data home | `vak_config::paths::data_home` | shared state + `agents/<id>/` per-Agent state |
| cache | `paths::cache_home` | `store.db` FTS index (sessions only) |
| logs | `paths::logs_dir` | unstructured service logs |
| Shared layer | `paths::default_workspace` (`~/vak-home`) | config, skills, plugins, encrypted credentials |
| project `.vak/` | ad hoc `cwd.join(".vak")` in many crates | scratch, worktrees, agent workspaces, flows, prompts, agents.json, launch/permission files |
| system temp | `std::env::temp_dir` | provider gates, browser profiles, prompt edit files |

`Core::sessions_home()` (`crates/vak-core/src/lib.rs:3036`) appends
`agents/<agent_id>` for every admitted Agent. So everything keyed on "the
sessions home" (sessions, checkpoints, memory, entities, sandbox records,
presentations, flow runs, intent/routing/security evidence) is per-Agent,
while `shared_data_home()` callers are global. The split is decided per call
site, not by any declared data class.

The durable-state registry (`crates/vak-core/src/state.rs`) is the right idea,
but it covers only four roots (no project `.vak/`, no temp) and has drifted
(D10, D17, D24).

### 2.2 Defects

Reproduced on a real data home (by listing names and sizes):

- **D1. 1.6 GB of unreadable checkpoints.** Manifests from before `31c1bb9c`
  embedded base64 file content (single files up to 89 MB). The current reader
  refuses them (invariant 29, correctly), but pruning is per-session on the
  next `store`, so a finished session's files are never touched again. The
  shared `blobs/` store is 24 KB.
- **D2. Agent homes nest twice.** The data home contains
  `agents/vak/agents/vak/{sessions,checkpoints,…}`. Cause:
  `crates/vak-server/src/lib.rs:16528` seeds a child Core with
  `state.core.sessions_home()` (already agent-scoped) instead of
  `shared_data_home()`, which every other child-core site uses.
  `spawn_isolated_run` is the path for scheduled tasks and best-of-N.
- **D3. Retired writers leave debris.** `~/vak-home/.vak/mcp-artifacts/`
  (retired per CHANGELOG), a second `credentials.enc` + `.credential_key`
  pair in the data home that the registry declares only under the Shared
  root, and `.js` files written directly into `.vak/scratch/` where only
  per-Agent folders belong.
- **D4. Scratch is never collected.** One session left twelve
  `.vak/scratch/vak/call_*/tmp` directories. Nothing removes them.

Read from code, not yet reproduced live:

- **D5. Scheduled runs can be silently dropped.** In `fire_task`
  (`crates/vak-server/src/lib.rs:17472`):
  - `worktree::create(...).ok()?` returns without a record when the
    workspace is not a git repository. The canonical default workspace
    `~/vak-home` is not one, so an LLM task there never runs, and an
    interval task retries every 20 s forever with no trace.
  - The run id is `task-` + the first 8 hex digits of a UUIDv7 (the top 32
    bits of a millisecond clock, so it changes every 65.5 s). Two tasks due
    in one tick get the same worktree path and branch; the second fails
    `git worktree add`. For a cron task without a timezone,
    `advance_marker` then moves past the slot, so the slot is lost. AGENTS.md
    already forbids clock-derived lockable ids.
  - Script tasks run with `sandbox_sink: None`, so every run shares
    `scratch/<agent>/shell/tmp`.
- **D6. Scheduled runs are not traceable after a restart.**
  - The handle id stored in `TaskDef.last_session_id` and
    `inbox.session_id` is `<ledger id>-<rid>` (`lib.rs`, end of
    `spawn_isolated_run`), which is not the ledger's session id.
  - `find_session_on_disk` (`lib.rs:7988`) neither strips that suffix nor
    looks in the D2 nested path.
  - `SessionHeader` has no field naming what triggered the session, so the
    ledger cannot name its task.
  - Run history is only `last_*` fields that each fire overwrites.
- **D7. There are two scheduling systems, and one is dead.**
  `AgentSchedule` (`crates/vak-server/src/agents.rs:47`) is writable through
  `PUT` and `vak agents`. `scheduler_tick` reads only `tasks.json`, and
  `record_run` (`agents_runs.jsonl`) has no non-test caller. This violates
  invariant 30 (one canonical way), and invariant 38's description of
  `AgentRunRecord` receipts is not true of the build.
- **D8. Side ledgers can't be joined to the turn that caused them.**
  - `CostRow`: session only; no agent, turn or receipt entry.
  - `EvidenceRow` (routing): provider and model only.
  - `MisreadRow`: no session.
  - `SecurityEvent`: no session or agent.
  - Checkpoint manifests: session + seq, labelled with the raw prompt text;
    failures are discarded (`let _ = checkpoints::store`) and no ledger entry
    says a checkpoint was taken.
  - Scratch directories are keyed by provider tool-call id with no session
    in the path.
  - `CandidateRecord` is the exception, with session, turn, result, execution
    and environment ids, and is the model to copy.
- **D9. No structured telemetry.** There is no `tracing` crate in the
  workspace, and several hundred `eprintln!` calls across library and server
  code. Service logs have no timestamps, levels or ids.
  `vak-server/src/bus.rs::emit` passes `trace: None`, so every envelope
  starts a new root trace. Its `prev_hash` is the string `seq - 1`, not a
  hash, so the "causal lineage" is not one.
- **D10. The registry has drifted from the writers.**
  - It declares `jobs`, but the outbox writes `delivery/jobs/`, so backups
    miss it.
  - Undeclared: `commitments.jsonl`, `budget-alerts.jsonl`, top-level
    `activity-log.jsonl`, `workspaces.json`, `feeds/feeds.duckdb`,
    `credential_index.json`, `sandbox/promotions/`, `update-check.json`.
  - `learning` is declared but has no writer.
  - Doc 64 (marked "implemented and audited") shows
    `~/vak-home/agents/…`, `finops/ledger.jsonl` and `index/`, and none of
    those exist.
- **D11. Identity is derived from where you stand.** Sessions, entities and
  skill proposals are keyed by `hash_cwd(cwd)`. The same project opened from
  a different path or machine, or after a move, becomes a different
  "project". In the cloud there is no cwd.
- **D12. Lookup means scanning.** Finding a session by id walks up to four
  directory layouts. Nothing can answer "everything this run produced"
  without reading every ledger.

Measured earlier (`docs/architecture/write-paths-and-growth.html`, v3.5.1:
60 sessions, 1 386 turns):

- **D13. Ledgers carry bulk that should be referenced.** A turn averages 13
  entries, 13 fsyncs and 66 KB. 72% of ledger bytes are
  `TurnCapabilitiesBound`: about 48 KB rewritten whole every turn even when
  nothing changed. Tool results ride inside user-role messages.
  `sandbox/executions/<session>.jsonl` writes 2 rows/s for the whole life of
  a command, with no bound. Settled delivery job files are never removed.
- **D14. Reads grow with history.** Examples:
  - `plan_route_ladder` fully scans `routing-evidence.jsonl` on every turn.
  - `has_request_admission` scans the in-memory ledger.
  - Every commitment append replays the whole ledger.
  - `store.db` re-reads the whole JSONL after every run.
  - The activity log has no compaction.

Found in review (`docs/plans/data-architecture-review.md`), read from code:

- **D15. Plaintext secrets in the project tree.** `PUT /config/bus` writes
  the NATS JWT and NKey seed to `<project>/.vak/env`
  (`crates/vak-server/src/lib.rs:13470-13540`). Nothing reads that file
  back.
- **D16. "Delete" only hides.** The session id goes into `deleted.json`
  (`lib.rs:9140-9177`). Only the two session lists consult that map. The
  model's `session_search`, admin search, `search_all` and the FTS store
  still return the "deleted" conversation.
- **D17. Purge leaves data behind.** `purge_state` skips the Logs root
  (`crates/vak/src/install/mod.rs:1093`), and every undeclared file survives
  by design. That covers commitments, feeds, the credential index, the
  archive/deleted sidecars, delivery jobs and sandbox records, and project
  `.vak/` runtime state is never purged.
- **D18. A second path resolver.** The Python feeds pipeline resolves the
  data home itself (`scripts/feeds/feed_utils.py:38`) and ignores
  `VAK_HOME` and the `VAK_SESSIONS_HOME` it is passed.
- **D19. Retention is scattered and rewrites ledgers.** There are four
  hardcoded mechanisms: checkpoints (20 per session), cost-log (5 MB / 90 d
  rewrite), alerts (1 MB / 2 000 rows rewrite), and memory write debris
  (24 h). Two of them rewrite files the registry declares as ledgers.
- **D20. Environments have no backend.** The `EnvironmentBackend` trait
  (`crates/vak-sandbox/src/lib.rs:59`) has no implementation, so doc 54's
  task environments cannot yet be used by anything.
- **D21. Secret scopes are keyed by path.** `scope_key_for`
  (`crates/vak-config/src/credentials.rs:51`) hashes the canonical path, so
  moving a project, or keying spaces by id, orphans its stored secrets.
- **D22. Content copies escape their conversation.** Checkpoint labels
  embed the prompt text (`vak-core/src/lib.rs:6804`). Inbox bodies, delivery
  text, outbox payloads and commitment statements hold conversation content
  in shared ledgers. Memory, entities and skill proposals record no
  provenance.
- **D23. Some docs claim behaviour that isn't built:**
  - doc 64 (layout tree)
  - doc 65, marked implemented (`AgentSchedule`/`AgentRunRecord`)
  - doc 72 (`AgentSchedule` automations)
  - AGENTS.md invariant 38
  - invariant 37's "revoked" Agents: `AgentLifecycle` has no such state
- **D24. The registry test barely exercises anything.** The enforcement
  test (`crates/vak-core/tests/state_registry.rs`) starts one session and
  records one event, so most of D10 and D17 went unseen.

## 3. Information model

One object graph, with identity separate from location.

```
Tenant ─┬─ Space ─┬─ Agent ─┬─ Conversation ── Session ── Turn ── Step ── Call ── Execution
        │         │         └─ Memory, Entities, Commitments
        │         ├─ Schedule ── Run*
        │         ├─ Artifact ── Version* ── Object*
        │         └─ Endpoint (channel/bot) ── Delivery*
        └─ Audit, FinOps, Operations            (* = produced over time)
```

- **Tenant**: the isolation, encryption and billing boundary. Local installs
  have exactly one (`local`).
- **Space**: what "workspace" means to a person. It has a stable `spc_` id
  and one or more *bindings* to filesystem roots on particular machines. The
  cwd hash becomes a lookup from binding to space, never an identity (fixes
  D11).
- **Run**: any unit of work with a cause: a user turn, a schedule slot, a
  channel request, a delegation, a revision, a heartbeat. A Run owns zero or
  more Sessions and Turns.
- **Execution**: one sandboxed process invocation, with its environment,
  inputs (workspace snapshot digest), outputs (object refs), streams (as
  objects) and exit.
- **Artifact**: a named thing a person sees and can share: a document,
  dataset, dashboard, card, changeset, report. Versions are immutable.
  "Current" is a ref.

**Identifier rules.** Every id is `<prefix>_<full UUIDv7>` (`ten_ spc_ agt_
cnv_ ses_ trn_ run_ exe_ art_ ver_ sch_ dlv_`), or `obj_<hex>` for objects.
An object id is HMAC-SHA256 of the plaintext under a tenant id key (§7.3),
so dedupe works inside a tenant while a file's presence cannot be confirmed
across tenants. Ids are never truncated, never clock-only, and never derived
from a path or a display name. A provider's tool-call id is data *inside* a
Call, never a directory name.

## 4. The trace key

Every durable record, ledger row, object manifest, log line and bus envelope
carries one:

```rust
struct TraceKey {
    tenant: TenantId, space: SpaceId, agent: AgentId,
    conversation: Option<ConversationId>, session: Option<SessionId>,
    turn: Option<TurnId>,                       // directive entry id, as today
    run: RunId,                                 // == W3C trace-id
    span: SpanId, parent_span: Option<SpanId>,  // turn/step/call/execution
    cause: Cause,
}
enum Cause {
    User { request_id },            Channel { endpoint, request_id },
    Schedule { schedule, slot },    Delegation { parent_run, tool_use_id },
    Revision { candidate },         Heartbeat,     System { job },
}
```

- **Contract:** a writer that cannot fill `tenant/space/agent/run/span/cause`
  does not write. That is a review failure, just like a pre-baseline
  compatibility branch.
- **Constructors:** `TraceKey` has one owner (`vak-session`, next to
  `Entry`) and is propagated by value through `ToolContext`, `AgentConfig`,
  the broker protocol and delivery packets. It is never re-derived from
  ambient state.
- **W3C mapping:** `trace-id = run`, `span-id = span`. The same key goes to
  `tracing` spans, OTLP and bus envelopes (fixes D9's fresh root per event).

## 5. Data classes

Every path belongs to exactly one class, and the class decides everything
else. The registry in `state.rs` becomes a registry of *classes and
locations*, not of file names.

| Class | Examples | Authority | Mutability | Deleted by | Backup | Cloud |
|---|---|---|---|---|---|---|
| **Record** | session ledgers, runs, commitments, promotions, audit, cost, deliveries | source of truth | append-only, segmented | conversation content: crypto-shred per conversation (§7.3); non-content shared ledgers: expire by sealed segment | always | sealed segments |
| **Object** | file contents, attachments, drafts, checkpoint contents, evidence bodies, stdout/stderr, exports | content-addressed (keyed id) | immutable | GC when no key grant or ref remains | always | by id, deduped |
| **Document** | memory notes, entities, skills, skill proposals, prompt layers, presentation packs | named; a person or the runtime edits it | every save is an immutable version; "current" is a ref | forget = tombstone; history by retention; erased with its `derived_from` source | always | versions as objects |
| **Ref** | artifact current version, session head, schedule cursor, bindings | pointer | CAS-updated | with its owner | always | CAS sync |
| **Desired** | config layers, Agents, schedules, endpoints, bots, allowlist, prompt layers | operator intent | atomic replace, additive schema | explicit | always | sync (no secrets) |
| **Secret** | provider keys, bot tokens | credential store | operator only | explicit | opt-in | never plaintext; references only |
| **Workspace** | a space's working tree, Agent workspaces, worktrees, task environments | projection of objects + human edits | mutable | lifecycle of its Run/Space | via checkpoints → objects | via objects |
| **Derived** | catalog, FTS, embeddings, belief state, route aggregates, thumbnails | rebuilt from records/objects | overwritten | any time | never | rebuilt remotely |
| **Ephemeral** | scratch tmp, caches, locks, sockets, gates, browser profiles | none | anything | end of Execution/process, boot sweep | never | never |
| **Telemetry** | logs, spans, metrics | none (records win) | rotated | size/age | never | optional, allowlisted OTLP export; fleet receives no content (§11.2) |

Four rules follow:

- **Content is keyed to its conversation wherever it is written.** A content
  field in a shared ledger (inbox body, delivery text, outbox payload,
  commitment statement) is encrypted under its conversation's key. Ids,
  timestamps and states stay readable for audit and scheduling. This is
  what lets an erasure reach every copy (review R2).
- **Derived writes record `derived_from`.** Memory, entities, skill
  proposals, catalog text rows and embeddings name their source
  conversation and turn.
- **Ledgers never hold what an object should.** Big tool results, file
  contents and stdout go to the object store, and the ledger holds the hash.
  `EvidenceBodyRecord` already points this way. This is what keeps session
  ledgers small and syncable.
- **The project tree holds only project intent.** `<space>/.vak/` keeps
  committable configuration (prompt layers, launch, permissions, flows,
  commands). Runtime state (executions, worktrees, candidates, agent
  workspaces) moves under the data home, keyed by id. This changes invariant
  35 (decision Q3).

## 6. Physical layout

```
<data>/tenants/<ten>/
  catalog.db                          Derived   (§9) — rebuildable
  objects/ab/cdef…                    Object    keyed-hash id, zstd, per-object key
  keys/                               key grants (wrapped); conversation/space/artifact keys
  records/
    spaces/<spc>/agents/<agt>/sessions/<ses>/{HEAD, 000001.seg, 000002.open}
    runs/…                            Record    every Run, including skips
    commitments/…  deliveries/…  promotions/…  finops/…  inbox/…
    audit/{security,operations,lifecycle,erasure,grants}/…   no content, ever
  documents/<kind>/<id>               Document  current ref + versions as objects
  refs/                               Ref       one KV table (CAS)
  desired/{agents,schedules,endpoints,bots,labels,holds}, allowlist
  lifecycle/quarantine/…              staged destructive actions (doc 74 §5)
  executions/<exe>/{tmp,…}            Ephemeral; removed when the Execution settles
  environments/<run>/                 Workspace; worktrees & task envs by run id
<cache>/<ten>/                        Derived   caches (pip/npm per agent), fts, embeddings
<logs>/                               Telemetry vak-<service>.jsonl, rotated
<runtime>/                            Ephemeral locks, sockets, provider gates (wiped at boot)
~/vak-home/                           Shared layer (unchanged: config + credential store)
<space root>/.vak/                    project intent only
```

- **Segmented ledgers.**
  - A session ledger is a directory of segments. Each entry is a frame,
    encrypted under the conversation key when tenant policy is on.
  - The open segment only ever grows. A sealed segment is compressed,
    hashed, and chained to the previous one; the per-entry `prev_hash` chain
    that ledgers already carry (`vak-session/src/log.rs:225`) continues
    across segments.
  - Sealing is copy, verify (count, hashes, chain), atomic swap, then a seal
    entry. Entries are never rewritten; only their encoding changes, by a
    verified seal (the invariant 2 amendment).
  - `derive_messages()` reads through the segments.
- **One object store for everything.** Checkpoints, attachments, drafts,
  candidates and evidence bodies share it, so the same file captured by a
  checkpoint, saved as an attachment and promoted as a candidate is stored
  once.

## 7. Lifecycle

### 7.1 States

```
open ──seal──▶ sealed ──age──▶ cold ──retention──▶ expired(tombstone) ──▶ shredded/purged
                  ▲                                     │
                  └────────────── legal hold blocks ────┘
```

Every object in the catalog has a lifecycle state, the policy that governs
it, and `expires_at`. Transitions are records, so "why is this gone?" always
has an answer.

**Records expire per conversation, never per entry.** Dropping entries from
a hash-chained ledger would break `derive_messages()`. A conversation expires
as a whole (`last_activity + delete_after`) by crypto-shred. Shared ledgers
without content (cost, routing, activity) expire by whole sealed segment.
The per-entity state machines (tenant, space, Agent, conversation, run,
execution, environment, artifact, Document, checkpoint, delivery, endpoint,
object, key) are in doc 74 §2.

### 7.2 One reconciler

A single level-triggered `LifecycleReconciler` (invariant 31's pattern),
per tenant:

- seals idle sessions and runs
- compresses and cold-tiers sealed segments
- removes settled executions' scratch
- tears down reviewed or abandoned environments
- prunes checkpoints to policy
- mark-and-sweeps objects, with roots = records + refs + holds, and a grace
  period (as `checkpoints::GC_GRACE` does today)
- enforces quotas
- deletes debris that no class claims, after quarantining it for one cycle

Events (turn closed, run settled, candidate promoted) are hints. The periodic
tick is the truth. It runs in the server and the desktop shell. It ships
observe-only first (it plans and shows, and commits nothing), then commits one
action class at a time, and every destructive action is staged in quarantine
before commit. The loop, guards and pacing are in doc 74 §5. A CLI surface
exposes it: `vak data status | gc --dry-run | verify | export | hold`.
`doctor --repair` calls only its mechanical actions (invariant 19).

### 7.3 Keys, crypto-shredding and erasure

Append-only and "delete this" are reconciled by **crypto-shredding**. The
first draft keyed "exclusively owned objects" per conversation, which cannot
work: exclusivity is unknown at write time, and per-conversation keys defeat
dedupe (review R1). The design is:

- **Key hierarchy.**
  - The tenant KEK lives in the credential store
    (`vak_config::credentials`: OS keychain, or the encrypted-file fallback).
  - The KEK wraps **scope keys**: one per conversation (spanning its session
    rotations), one per space (checkpoints, promoted files), and one per
    artifact (shared versions).
  - AEAD comes from `ring`, already a workspace dependency.
- **Records are encrypted per entry** under their conversation key, so
  appends never rewrite anything.
- **Content fields in shared ledgers** (inbox body, delivery text, outbox
  payload, commitment statement) are field-encrypted under the
  conversation key they came from (§5).
- **Objects have their own random key.** Each scope that references an
  object stores that key wrapped under the scope key (a *key grant*). An
  object stays readable while any grant survives, so the same file in two
  conversations is stored once and outlives the erasure of either.
- **Object ids are keyed hashes** (HMAC-SHA256 under a tenant id key):
  dedupe within the tenant, no cross-tenant confirmation-of-file.
- **Erasure destroys the scope key**, removes derived plaintext (catalog
  rows, embeddings, Document versions derived only from the scope), GCs
  objects left with no grant, and writes a content-free audit tombstone plus
  a signed receipt. The steps, scopes, approvals and receipt contents are in
  doc 74 §3.4 and §4.
- **Derived plaintext is the limit.** The catalog's FTS and embedding
  stores are plaintext derived copies. Erasure deletes their rows with
  `secure_delete` and checkpoints the WAL. Locally, their at-rest
  protection relies on OS disk encryption; the cloud baseline uses managed
  encryption. For an operator-blind hosted customer, this is insufficient:
  the catalog and every derived copy must remain inside the customer VM's
  protected boundary, or receive equivalent customer-controlled encryption
  and attested key release (doc 79 §5). A fleet operator cannot receive its
  plaintext through a search or troubleshooting endpoint.
- **Backups** carry ciphertext and *wrapped* keys, never the KEK unless
  escrow is requested. Restoring re-applies every erasure tombstone before
  anything becomes readable. Fleet DR also needs a coherent record/ref/object
  manifest, off-VM copies, fencing and a measured restore drill (§11.1).
- **Transparency and honesty.**
  - Encryption at rest is a per-tenant policy, on by default because
    erasure depends on it.
  - `vak data cat | grep | export --plain` gives people the plain view the
    product promises.
  - On a headless host, the encrypted-file store keeps its key beside its
    data, so at-rest protection there is nominal. Crypto-shred still works,
    because it destroys the scope key. This describes the local baseline,
    not an operator-blind customer hosting solution. Ordinary EBS encryption,
    a customer key unlocked into operator-replaceable code, or a staff role
    that hides plaintext only in the panel cannot meet doc 79's privacy
    promise. Hosted key custody and approved-code measurements are a
    separate release gate, with no operator escrow by default.

### 7.4 Default policies

Retention labels (`retain_for`, `delete_after`, `on_expiry` per class)
attach to the tenant, a space, an Agent or a conversation. They inherit
downward, with explicit break points, like SharePoint retention labels. The
minimum keep is the longest `retain_for` in the chain. The maximum keep is
the shortest `delete_after`, never below the minimum. Holds suspend expiry.
The default label, holds, quotas and erasure scopes are in doc 74 §3.
Execution scratch is removed when the Execution settles, and telemetry is
capped at 14 days or 200 MB.

## 8. Runs and schedules

- **One schedule model** (fixes D7). `AgentSchedule` is removed in the same
  change: API, CLI, UI, tests and doc paragraphs. A schedule is Desired state
  owned by an Agent (`sch_` id, cron/interval/once, timezone, prompt or
  script, delivery target, policy pin).
- **Every slot produces a Run record**, whatever happens. It records
  `schedule`, `slot` (the intended instant), `fired_at`, `attempt`,
  `decision` = `fired | skipped{reason} | coalesced | failed{reason}`,
  `sessions[]`, `result_id`, `deliveries[]`, cost, and the full `TraceKey`.
  "No provider", "not a git repository", "previous run still going" and
  "lease held elsewhere" are all *records*, never `eprintln!` + `return None`
  (fixes D5).
- **At most one start per slot.** A run has side effects, so a crash
  between start and record cannot be made exactly-once (review R4). Instead:
  - Before any side effect, the slot `(schedule, slot)` is claimed with a
    CAS on a ref. Catch-up, restarts and a second server all go through the
    claim, so a slot is never started twice and never silently lost.
  - A run whose lease expires is recorded `abandoned`.
  - The schedule's `on_crash = skip | retry_once` decides whether that slot
    is retried.
- **Environments are named by the full run id.** A git worktree is used only
  when the space *is* a git repository. Otherwise the run gets a
  `CopyEnvironment`: the first real `EnvironmentBackend` (D20). It copies
  the space with ignore rules and size caps, and its changes come back as a
  candidate for Review. There is never a silent skip.
- **The ledger names its cause.** `SessionHeader` gains `run: RunId` and
  `cause: Cause`, an additive field (invariant 29). The handle id is the
  ledger id, with no suffix (fixes D6).
- **The Runs view** works like an Actions tab: per Agent and per schedule,
  showing status, duration, cost, outputs and deliveries, and drilling into
  the session, executions and artifacts through the catalog's edges.

## 9. Catalog, index and search

`catalog.db` is one Derived store per tenant. It replaces `store.db`, the
recall ledger cache, and every scanning lookup (`find_session_on_disk`,
`read_historical_header`, `find_session_in_cwd`). `workspaces.json`,
`workspace-names.json` and `trusted/` become the Spaces store (Desired).
`presentations.json` is the presentation-pack library and becomes a
Document store, not a projection (review R9).

Several processes (desktop, server, CLI, gateway) may ingest. Ingest is an
idempotent upsert keyed by `(chain, seq)`, using WAL with a busy timeout, so
no single process has to own it. The text and vector stores are plaintext
derived copies, and erasure removes their rows explicitly (§7.3).

- **`nodes`**: one row per addressable thing (space, agent, conversation,
  session, turn, run, execution, artifact, version, candidate, delivery,
  schedule, memory note, commitment). Columns: every `TraceKey` field, kind,
  class, lifecycle state, policy, `expires_at`, size, audience and ACL
  digest, created/sealed timestamps.
- **`edges`**: lineage (`produced_by`, `derived_from`, `version_of`,
  `promoted_to`, `delivered_as`, `caused`, `references`, `shared_with`). It
  answers "everything this run touched" and "where did this file come from"
  with one query.
- **`text`**: FTS5 over text projections (messages, tool digests, artifact
  text via `doc_read`, memory, run summaries). An optional vector index can
  sit behind the same query API.
- **ACL at query time.** Every search is filtered by the caller's audience
  and grants before ranking (invariant 37). Results never carry content the
  caller could not open directly.
- **Rebuildable.** Records, refs and object manifests are the only inputs. A
  deleted catalog is rebuilt, and a checksum over `(records HEADs, refs)`
  says when it is stale.

In the cloud the same schema runs on Postgres, with objects in an object
store and search on Postgres FTS plus pgvector (or OpenSearch when volume
demands it). The query API is identical, so every surface calls one
`search`/`lineage` interface wherever it runs.

## 10. Artifacts, sharing and collaboration

- **An Artifact is the unit people share.** A draft (written in an
  execution) becomes a *candidate* (reviewable, verified in a worker, per
  invariants 14 and 39), which is then *promoted* into a space's working
  tree, *shared* with an audience, or *published* externally. Each step is a
  record, and each resulting state is a new Version or a new ref, never an
  in-place edit.
- **Permissions work like SharePoint.** An Artifact inherits its Space's
  audience. A share is an explicit grant (viewer / commenter / editor) that
  breaks inheritance for that Artifact only. Coworking grants (doc 69)
  become artifact and conversation grants in the same table.
- **Concurrent edits produce sibling Versions.** Reconciling them is the
  existing Review path (redline for Office, semantic diff for data, file diff
  for code). There is no merge UI for documents.
- **Code is the exception at the edge.** An Artifact of kind `changeset`
  promotes into a git repository as a commit on a branch, and optionally a
  PR. Git is the destination format there, not Vak's storage model.

## 11. Cloud handoff

The cloud is a *remote*, as in git, not a different product:

- **Push** uploads missing objects by hash, then sealed segments, then moves
  refs with compare-and-swap. **Pull** is symmetric. Desired state syncs.
  Secrets never do; the cloud holds its own credentials, and records carry
  references (invariant 8).
- **One writer per session.** The session lock becomes a lease with a
  holder, an epoch and an expiry. Handing a live conversation to the cloud
  means releasing the lease at a turn boundary; the cloud acquires it and
  continues from the same HEAD. The four-plane network contract (doc 31)
  applies: a push that cannot complete is store-and-forward, never data
  loss.
- **Tenancy and residency.** A tenant pins a region. Keys follow the tenant
  (§7.3). Tombstones and retention decisions sync both ways, so an erasure
  performed on one side cannot be resurrected by the other.
- **Multi-machine** (doc 56) becomes the same mechanism with two personal
  machines as remotes of each other.

### 11.1 Dedicated fleet and disaster recovery

The hosted fleet in `79-private-headless-fleet.md` deploys one tenant to one
dedicated, continuously running VM. This deployment unit is distinct from
the `Remote` protocol and from `CorePool`'s workspace cache. A customer may
have several Agents inside its VM, but another customer's runtime or data is
never placed there. The fleet control plane may operate signed software and
observe content-free health; it is not a second data home or a route into
tenant records, secrets or detailed logs. The customer's stable HTTPS origin
survives VM replacement so owner passkeys continue to work (doc 78).
The fleet may prepare unused, running VMs to shorten signup, but customer
data, owner identity, keys and records begin only after an atomic one-customer
binding. A VM that reached binding is never recycled for another customer;
its data follows the governed offboarding and erasure path. Pool inventory
and readiness are infrastructure records, not tenant records. Doc 79 §3.1
owns the pool lifecycle, TLS and measured readiness contract.
Portal signup and billing identity are fleet metadata, not authorization to
read tenant records. First-owner claim, VM login and recovery follow doc 78;
the portal cannot mint a VM session or release a decryption key on its own.
Doc 79 §3.2 owns the hosted signup and separation-of-authority proposal.
Fleet inventory, infrastructure operation and billable-usage ledgers are
separate control-plane records, not a second copy of tenant Runs or objects.
The payment provider owns invoices/settlement, AWS owns resource and cost
evidence, and the fleet reconciles their references by opaque deployment id.
Doc 79 §§10–13 define the proposed dashboards, alerts, network paths, AI
operations boundary and bookkeeping; they do not change this document's
local-first record/object/ref authority.

A VM, EBS volume or snapshot is not the sole recovery record. A complete
recovery point binds sealed/open record-chain heads, refs, object inventory,
key grants, Desired state, software/data baseline and the latest erasure
watermark in one verified manifest. Copies live outside the VM's failure
domain as ciphertext under customer-controlled keys. A restore checks every
referenced object and chain before admission, replays erasure decisions before
readability, rebuilds Derived stores and obtains a new fenced writer epoch.
The fleet backup contract also covers the current canonical data home and
every registered durable state path until this model replaces them. Generated
OS/application images and caches are rebuilt; customer records and encrypted
credentials are preserved. A storage snapshot without a logical manifest is
not a complete restore point. Patch and replacement workflows record the
backup id, image/software baseline, attested key decision and post-restore
integrity result before admitting new work (doc 79 §§7–8).
For a later no-loss-of-acknowledged-writes mode, every durable mutation must
join an encrypted off-VM commit before its caller receives `saved`: objects
first, an ordered complete manifest/ref marker last, verified on restore.
Periodic backup alone supports only a measured nonzero RPO. This is a new
write-acknowledgement contract for all file, credential, owner, schedule and
delivery paths, not a property gained by adding S3 to the current tree.
Doc 79 §8.1 specifies the degraded-mode disclosure and failure tests.
The old VM must be unable to resume schedules, channel polling, writes or
delivery after the new epoch is active. Recovered external actions are
reconciled from receipts rather than replayed blindly. A restore without the
customer's key authority fails closed; the fleet cannot decrypt on behalf
of an absent customer.

The initial design targets are RPO ≤15 minutes and RTO ≤60 minutes for a
single VM/availability-zone loss when the key authority and AWS region are
available. They are **not** current service guarantees. Cross-region and
customer-key outages need separate targets and consent to any residency
change. Scheduled isolated restore drills measure the actual gap and time,
and verify `derive_messages()`, leases, passkey sign-in, delivery dedupe and
erasure non-resurrection. Doc 79 §8 owns the failure matrix and tests.

### 11.2 Fleet privacy and telemetry

The remote and catalog contracts may be backed by local SQLite, object
storage and Postgres as the plan states, but a hosted fleet implementation
cannot infer that a shared cloud database may expose tenant content or
sensitive metadata to operators. Content, key grants and derived search
material stay within the customer-protected boundary or have an equivalent
attested, customer-keyed protection. Remote refs and object inventories can
reveal activity and must be classified, access-controlled and retained as
customer data even when their payload objects are encrypted.

Central fleet telemetry is an allowlist of deployment identity, signed
version, coarse resource use, health, backup age, bounded counts and typed
failure classes. No free-form error, raw command, filename, URL, prompt,
reply, secret, browser session or raw application log crosses it. Opaque
trace ids and timestamps can still be linkable personal data: they need a
purpose, access controls, retention and deletion. Customer Admin sees its
own detailed evidence within the customer boundary; fleet operators get no
impersonation or plaintext support override. A chosen inference provider and
external channel still receive the data needed for customer-authorized
calls, and their limits must be disclosed. Doc 79 §§5–6 and §9 own the
operator-blind threat test and data-protection release gates.

## 12. Sizing guidance

From the dev data home: about 100 MB of session ledgers for heavy
single-user development over ten days, with checkpoints dominating
everything else before D1. Only rough planning numbers are possible until
§7's policies run for a while:

- **Ledgers** grow with turns × tool-result size. Moving results to objects
  (§5) is the single largest control.
- **Objects** grow with *distinct* content. Dedupe across checkpoints,
  attachments and candidates is the second largest.
- **Telemetry** and **caches** must be capped by policy, not by hope.

Every tenant reports class totals in `vak data status` and the Operations
Center (invariant 26: measured, never synthetic), so real numbers replace
these estimates.

## 13. Decisions (locked 2026-09-25)

Resolved with the maintainer, with zero users and no backward compatibility:

- **Deployment:** local-first, with the cloud as a push/pull remote (§11).
- **Compliance bar for the first customer release:** retention labels,
  erasure by crypto-shred, legal hold, audit export. Region pinning, BYOK and
  eDiscovery come with the cloud phase.
- **Cutover:** a new **5.0.0** baseline (the workspace is at 4.0.2) that
  refuses every earlier data home with the single invariant-29 message. No
  migrator and no dual layout.
- **Runtime state leaves the project tree:** executions, environments,
  candidates and Agent workspaces move under the tenant; invariant 35 is
  rewritten; a space's `.vak/` holds only project intent.
- **Defaults:** `spc_` space ids with per-machine bindings; SQLite catalog
  locally and Postgres in the cloud behind one trait; `tracing` with JSON
  logs and optional OTLP; one schedule model (`TaskDef`, with
  `AgentSchedule` deleted).
- **Added by the review (revision 2):**
  - The key hierarchy and key grants of §7.3.
  - Content keyed to its conversation, and `derived_from` on derived
    writes.
  - Two-step deletion: trash, then erase.
  - Data roles (Owner, Steward, Operator, Member, Auditor).
  - Telemetry that carries no content.
  - The Document class.
  - Per-conversation expiry.

## 14. Phasing

The milestone plan (revision 2), exit tests, budgets and AGENTS.md changes
are in `docs/plans/data-architecture-plan.md`. The order is:

- **M0** fixes on 4.x, then **M1** trace key, then **M2** storage
  substrate.
- **M3a** is the Scope refactor with no behaviour change; **M3b** is the
  5.0.0 layout switch.
- **M4** runs, then **M6** catalog, then **M7** lifecycle and erasure, then
  **M8** artifacts and sharing, then **M9** remote.
- **M5** telemetry runs in parallel from M1.

The catalog comes before lifecycle because erasure needs lineage. Each
milestone ships whole (invariant 30).

Also in revision 2:
- The review and its fixes: `docs/plans/data-architecture-review.md`.
- What each milestone touches: `docs/plans/data-architecture-blast-radius.md`.
- Lifecycles, policies, screens and API: doc 74.

## Appendix A — every writer today and its target class

| Today (root / path) | Owner | Target class |
|---|---|---|
| data `agents/<id>/sessions/<cwd-hash>/<ses>.jsonl` | vak-session | Record (segmented, by space id) |
| data `agents/<id>/checkpoints/<ses>/NNNN.json`, `checkpoints/blobs/` | vak-core | Record manifest + Object |
| data `agents/<id>/memory/`, `skill-proposals/`, `entities/<cwd-hash>/` | vak-core | **Document** (rewritten by amend/forget; versions + `derived_from`) (review R9) |
| data `agents/<id>/sandbox/{records.jsonl,candidates,staging,revisions,previews,executions}` | vak-server / vak-sandbox | Record + Object + Workspace |
| data `agents/<id>/presentations.json` | vak-store | **Document** (presentation-pack library: definitions + activations) (review R9) |
| data `agents/<id>/flow-runs/` | vak-flow | Record (runs) |
| data `agents/<id>/{routing-evidence,intent-evidence,security-events,activity-log}.jsonl` | vak-core | Record (with TraceKey) |
| data `agents/<id>/coworking/grants.jsonl` | vak-server | Record (grants) |
| data `agents/<id>/agent-network/broker.sock` | vak-core | Ephemeral (runtime) |
| data `sandbox/promotions/` | vak-sandbox | Record |
| data `gateway/{bindings,allowlist,bots}.json`, `default-workspace` | vak-server | Desired |
| data `gateway/deliveries.jsonl`, `delivery/jobs/` | vak-server / vak-delivery | Record |
| data `operations/{incidents,actions}.jsonl` | vak-server | Record (audit) |
| data `tasks.json` | vak-core | Desired (schedules) + Run records |
| data `cost-log.jsonl`, `budget-alerts.jsonl` | vak-core | Record (finops) as segment chains; retention drops sealed segments, no compaction rewrite (D19) |
| data `commitments.jsonl` | vak-commit | Record; statement field keyed to its conversation |
| data `inbox.jsonl`, `inbox.dedupe.lock` | vak-core | Record (body keyed to its conversation) + Ephemeral |
| data `trusted/`, `workspaces.json`, `workspace-names.json` | vak-core / vak-server | Desired (space bindings) |
| data `feeds/feeds.duckdb` (written by Python, D18) | scripts/feeds | Record (external content, retention-bounded; path passed by Rust) |
| data `feeds.toml`, `output.toml`, `flows/` | various | Desired |
| data `skills/` | vak-core | Document |
| data `locks/`, `release/`, `update-check.json`, `install.json`, `desktop.json`, `tray.json` | various | Ephemeral / Desired |
| data `credential_index.json` | vak-config | Secret (index) |
| data `archive.json`, `deleted.json` (session archive/"delete" sidecars, D16) | vak-server | Ref (lifecycle state) + erasure requests (doc 74 §2.4) |
| data `agents/vak/agents/<id>/…` (nested homes, D2) | vak-server | none; the nesting is a defect removed in M0 |
| cache `store.db*` | vak-store | Derived (catalog) |
| logs `*.log` | vak-ops / vak-desktop | Telemetry (JSON, rotated) |
| `~/vak-home/.vak/{config.toml,skills,plugins,.seed-manifest.json}` | vak-config / vak-core / vak-plugin | Desired (Shared layer) |
| `~/vak-home/{credentials.enc,.credential_key,.credential_key.lock}` | vak-config | Secret |
| project `.vak/scratch/<agent>/<exe>/tmp`, `.vak/scratch/<agent>/cache` | vak-tools | Ephemeral |
| project `.vak/worktrees/<run>` | vak-core | Workspace (environment) |
| project `.vak/agents/<id>/workspace/` | vak-config | Workspace |
| project `.vak/{agents.json,agents_runs.jsonl}` | vak-server | Desired / Record |
| project `.vak/{flows,commands,launch.toml,permissions.local.toml}` | various | Desired (project intent) |
| project `.vak/prompts/` | vak-core | Document (prompt layers; project intent) |
| project `.vak/env` (NATS secrets in plaintext, D15) | vak-server | **removed**; values move to the Secret class |
| project `inbox/` | vak-server | Object + Artifact |
| temp `vak-provider-gates/`, browser profiles, `vak-prompt-*.md` | vak-llm / vak-tools / vak | Ephemeral (runtime) |
