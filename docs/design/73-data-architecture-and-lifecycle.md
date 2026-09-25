# 73 — Data architecture: information model, lifecycle, tracing, index and cloud

Status: **proposal, 2026-09-25; decisions locked (§13), plan in
`docs/plans/data-architecture-plan.md`.** Nothing here is shipped. §2 is an audit of
what the tree does today, taken from the source and from sizes (never
contents) of a real development data home; each defect names the file that
causes it and says whether it was reproduced or read from code.

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
(D6).

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
cnv_ ses_ trn_ run_ exe_ art_ ver_ sch_ dlv_`), or `sha256:<hex>` for
objects. Ids are never truncated, never clock-only, and never derived from a
path or a display name. A provider's tool-call id is data *inside* a Call,
never a directory name.

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
| **Record** | session ledgers, runs, commitments, promotions, audit, cost, deliveries | source of truth | append-only, segmented | crypto-shred only (§7.3) | always | sealed segments |
| **Object** | file contents, attachments, drafts, checkpoint contents, evidence bodies, stdout/stderr, exports | content-addressed | immutable | GC when unreferenced | always | by hash, deduped |
| **Ref** | artifact current version, session head, schedule cursor, bindings | pointer | CAS-updated | with its owner | always | CAS sync |
| **Desired** | config layers, Agents, schedules, endpoints, bots, allowlist, prompt layers | operator intent | atomic replace, additive schema | explicit | always | sync (no secrets) |
| **Secret** | provider keys, bot tokens | credential store | operator only | explicit | opt-in | never plaintext; references only |
| **Workspace** | a space's working tree, Agent workspaces, worktrees, task environments | projection of objects + human edits | mutable | lifecycle of its Run/Space | via checkpoints → objects | via objects |
| **Derived** | catalog, FTS, embeddings, belief state, route aggregates, thumbnails | rebuilt from records/objects | overwritten | any time | never | rebuilt remotely |
| **Ephemeral** | scratch tmp, caches, locks, sockets, gates, browser profiles | none | anything | end of Execution/process, boot sweep | never | never |
| **Telemetry** | logs, spans, metrics | none (records win) | rotated | size/age | never | optional OTLP export |

Two rules follow:

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
  objects/sha256/ab/cdef…             Object    zstd, encrypted per tenant
  records/
    spaces/<spc>/agents/<agt>/sessions/<ses>/{HEAD, 000001.jsonl.zst, 000002.jsonl}
    runs/<yyyy-mm>/<dd>.jsonl         Record    every Run, including skips
    commitments/…  deliveries/…  promotions/…  finops/…  audit/{security,operations}/…
  refs/                               Ref       small files or one KV table
  desired/{agents,schedules,endpoints,bots}.toml, allowlist.json
  executions/<exe>/{tmp,…}            Ephemeral; removed when the Execution settles
  environments/<run>/                 Workspace; worktrees & task envs by run id
<cache>/<ten>/                        Derived   caches (pip/npm per agent), fts, embeddings
<logs>/                               Telemetry vak-<service>.jsonl, rotated
<runtime>/                            Ephemeral locks, sockets, provider gates (wiped at boot)
~/vak-home/                           Shared layer (unchanged: config + credential store)
<space root>/.vak/                    project intent only
```

- **Segmented ledgers.** A session ledger is a directory of segments. The
  open segment is plain JSONL; a sealed segment is compressed, hashed, and
  chained to the previous one's hash in its header. Append-only still holds
  (invariant 2): sealing only rewrites *encoding*, never an entry, and the
  seal is a record entry. `derive_messages()` reads through the segments.
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
tick is the truth. It runs in the server and the desktop shell. A CLI surface
exposes it:
`vak data status | gc --dry-run | verify | export | hold`. `doctor --repair`
calls only its mechanical actions (invariant 19).

### 7.3 Deletion without rewriting ledgers

Append-only and "the customer asked us to delete this" are reconciled by
**crypto-shredding**:

- Each conversation's records and exclusively-owned objects are encrypted
  under a per-conversation data key, wrapped by the tenant key.
- Erasure destroys the data key and appends a tombstone to the audit record.
- The bytes remain append-only and unreadable. Objects shared with a
  surviving conversation stay readable through that conversation's key.
- This is the only deletion path for Record-class data.

### 7.4 Default policies

| What | Local dev default | Customer default |
|---|---|---|
| execution scratch | at Execution settle (+1 h grace) | same |
| tool caches (pip/npm/pycache) | LRU, 2 GB per tenant | quota by plan |
| environments / worktrees | 7 days after the run settles, or on promote/reject | 30 days |
| checkpoints | baseline + last 20 per open session; after seal: baseline + final, 30 days | policy label |
| candidates | until promoted/rejected + 30 days; receipts forever | policy label |
| session records | forever | retention label per space/agent (e.g. 1 y, 7 y, legal hold) |
| runs | forever (small) | same as sessions |
| telemetry logs | 14 days / 200 MB | exported, 30 days local |
| derived | rebuilt on demand | same |

Retention labels attach to a Space or Agent and inherit downward, with an
explicit break point, like SharePoint retention labels. A label may only
lengthen retention below a hold. It may never make a hold shorter.

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
- **Exactly once per slot.** The idempotency key is `(schedule, slot)`.
  Catch-up, restarts and a second server all consult the run records rather
  than an in-memory marker, so a slot can be neither lost nor fired twice.
- **Environments are named by the full run id.** A git worktree is used only
  when the space *is* a git repository. Otherwise the run gets a task
  environment (doc 54) and never a silent skip.
- **The ledger names its cause.** `SessionHeader` gains `run: RunId` and
  `cause: Cause`, an additive field (invariant 29). The handle id is the
  ledger id, with no suffix (fixes D6).
- **The Runs view** works like an Actions tab: per Agent and per schedule,
  showing status, duration, cost, outputs and deliveries, and drilling into
  the session, executions and artifacts through the catalog's edges.

## 9. Catalog, index and search

`catalog.db` is one Derived store per tenant. It replaces `store.db`,
`workspaces.json`, `workspace-names.json`, `presentations.json` (as a
projection) and every scanning lookup.

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

## 14. Phasing

The milestone plan, exit tests, budgets and AGENTS.md changes are in
`docs/plans/data-architecture-plan.md` (M0–M9). Each milestone ships whole
(invariant 30), and an early fix is made only when it is the target
behaviour, never an interim patch.

## Appendix A — every writer today and its target class

| Today (root / path) | Owner | Target class |
|---|---|---|
| data `agents/<id>/sessions/<cwd-hash>/<ses>.jsonl` | vak-session | Record (segmented, by space id) |
| data `agents/<id>/checkpoints/<ses>/NNNN.json`, `checkpoints/blobs/` | vak-core | Record manifest + Object |
| data `agents/<id>/memory/`, `skill-proposals/`, `entities/<cwd-hash>/` | vak-core | Record (+ Derived index) |
| data `agents/<id>/sandbox/{records.jsonl,candidates,staging,revisions,previews,executions}` | vak-server / vak-sandbox | Record + Object + Workspace |
| data `agents/<id>/presentations.json` | vak-server | Derived (catalog) |
| data `agents/<id>/flow-runs/` | vak-flow | Record (runs) |
| data `agents/<id>/{routing-evidence,intent-evidence,security-events,activity-log}.jsonl` | vak-core | Record (with TraceKey) |
| data `agents/<id>/coworking/grants.jsonl` | vak-server | Record (grants) |
| data `agents/<id>/agent-network/broker.sock` | vak-core | Ephemeral (runtime) |
| data `sandbox/promotions/` | vak-sandbox | Record |
| data `gateway/{bindings,allowlist,bots}.json`, `default-workspace` | vak-server | Desired |
| data `gateway/deliveries.jsonl`, `delivery/jobs/` | vak-server / vak-delivery | Record |
| data `operations/{incidents,actions}.jsonl` | vak-server | Record (audit) |
| data `tasks.json` | vak-core | Desired (schedules) + Run records |
| data `cost-log.jsonl`, `budget-alerts.jsonl` | vak-core | Record (finops) |
| data `commitments.jsonl` | vak-commit | Record |
| data `inbox.jsonl`, `inbox.dedupe.lock` | vak-core | Record + Ephemeral |
| data `trusted/`, `workspaces.json`, `workspace-names.json` | vak-core / vak-server | Desired (space bindings) |
| data `feeds/feeds.duckdb`, `feeds.toml`, `output.toml`, `flows/`, `skills/` | various | Derived / Desired |
| data `locks/`, `release/`, `update-check.json`, `install.json`, `desktop.json`, `tray.json` | various | Ephemeral / Desired |
| data `credential_index.json` | vak-config | Secret (index) |
| cache `store.db*` | vak-store | Derived (catalog) |
| logs `*.log` | vak-ops / vak-desktop | Telemetry (JSON, rotated) |
| `~/vak-home/.vak/{config.toml,skills,plugins,.seed-manifest.json}` | vak-config / vak-core / vak-plugin | Desired (Shared layer) |
| `~/vak-home/{credentials.enc,.credential_key,.credential_key.lock}` | vak-config | Secret |
| project `.vak/scratch/<agent>/<exe>/tmp`, `.vak/scratch/<agent>/cache` | vak-tools | Ephemeral |
| project `.vak/worktrees/<run>` | vak-core | Workspace (environment) |
| project `.vak/agents/<id>/workspace/` | vak-config | Workspace |
| project `.vak/{agents.json,agents_runs.jsonl}` | vak-server | Desired / Record |
| project `.vak/{prompts,flows,commands,launch.toml,permissions.local.toml,env}` | various | Desired (project intent) |
| project `inbox/` | vak-server | Object + Artifact |
| temp `vak-provider-gates/`, browser profiles, `vak-prompt-*.md` | vak-llm / vak-tools / vak | Ephemeral (runtime) |
