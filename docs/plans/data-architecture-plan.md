# Plan — data architecture, lifecycle, tracing and cloud

Status: **plan, 2026-09-25. Not started.** Implements
`docs/design/73-data-architecture-and-lifecycle.md` (the *what* and *why*);
this document is the *how* and *in what order*. Current measurements come
from `docs/architecture/write-paths-and-growth.html` (v3.5.1) and the
2026-09-25 audit in doc 73 §2.

## 1. Decisions (locked 2026-09-25)

| # | Decision | Consequence |
|---|---|---|
| L1 | **Local-first, cloud as a remote.** A desktop or server works offline on its own store; a hosted cloud is a push/pull remote with session handoff. | One storage trait with two backends (local disk + SQLite; object store + Postgres). Nothing in the runtime may assume a filesystem path is an identity. |
| L2 | **Compliance bar for the first customer release:** retention labels, erasure (crypto-shred), legal hold, exportable audit trail. Region pinning, BYOK and eDiscovery come with the cloud phase. | Encryption keys and holds are in the local design from day one, not bolted on later. |
| L3 | **Cut a new baseline, 5.0.0.** The workspace is at 4.0.2; 5.0.0 refuses every earlier data home with the single invariant-29 message. | No migrator, no reader for old shapes, no dual layout. Existing dev data homes are purged at cutover. |
| L4 | **Runtime state leaves the project tree.** A space's `.vak/` holds only committable intent. Executions, environments, candidates and Agent workspaces live under the data home, by id. | Invariant 35 is rewritten; sandbox profiles grant the execution directory explicitly. |
| L5 | **Zero users, zero compatibility.** | Every milestone *replaces* what it supersedes in the same change (invariant 30): code, tests and doc paragraphs. No "kept for compatibility". No throwaway interim fixes either: a defect is fixed early only if the fix is the target behaviour. |

Decided here without a question, because each has a clear default:

- **The catalog is SQLite locally**, behind a trait, with Postgres in the cloud.
- **Telemetry uses `tracing` + `tracing-subscriber`** (JSON), with OTLP export
  optional. `eprintln!` is banned in library crates.
- **There is one schedule model.** `TaskDef` is it; `AgentSchedule` is
  deleted.
- **Plans live in `docs/plans/`; designs stay in `docs/design/`.**

## 2. Goals and measurable exit criteria

The plan is done when all of these hold on a clean 5.0.0 install, proven by
the standing tests in §6:

1. **Traceable.** From any object, record, artifact, delivery or schedule
   slot, one catalog query returns its Run, Session, Turn, Agent, Space and
   Cause. It works after a restart, in under 50 ms at 1 M catalog nodes.
2. **Declared.** Every path any process writes belongs to a declared class
   (doc 73 §5). A test drives turns, schedules, delegations, revisions,
   deliveries and Office edits across every root and fails on an undeclared
   path.
3. **Bounded.** Ephemeral state is zero once executions settle. Caches and
   telemetry stay within their quotas. Record growth per turn is at most
   20 KB mean (today 66 KB) and at most 6 fsyncs (today about 13).
4. **Flat reads.** No read on the turn path is O(history). Today the routing
   ledger's full scan per turn, `has_request_admission`'s ledger scan, the
   commitment replay per append, and `store.db`'s whole-file re-import after
   every run all are.
5. **Governed.** Retention labels, erasure and legal hold work end to end;
   erasure leaves every append-only file byte-identical except for appended
   tombstones.
6. **Portable.** A session pushed to a file-backed test remote, with its
   lease released, continues on a second data home from the same HEAD, with
   identical `derive_messages()` output.
7. **Observable.** Every log line is JSON carrying the TraceKey fields. One
   run's spans form one tree (one trace id) across the loop, tools, worker,
   delivery and bus.

Non-goals for this plan: the hosted cloud service itself (only the remote
protocol and a reference backend), BYOK, eDiscovery UI, and region routing
(L2 defers them).

## 3. Target architecture in one page

```
                    ┌──────────────── surfaces (server, desktop, CLI, term, channels) ───────────────┐
                    │  Conversations · Runs · Library · Spaces · Activity/Audit · Data (admin)       │
                    └───────────────────────────────▲───────────────────────────────────────────────┘
                                                    │ search / lineage / open (ACL-filtered)
  vak-catalog  (Derived) ───── nodes · edges · text · lifecycle state ◀── rebuilt from ──┐
                                                                                        │
  vak-session / vak-commit / vak-delivery / vak-sandbox / vak-core ledgers   ──write──▶ vak-storage
                                                                                        │
  vak-storage  (substrate, no vak deps):                                                │
     objects   content-addressed, zstd, encrypted (tenant key → conversation key)       │
     records   append-only segment chains (open JSONL → sealed, hashed, chained)        │
     refs      small CAS-updated pointers (heads, current version, cursors, leases)     │
     keys      key hierarchy, crypto-shred, holds                                       │
     remote    push/pull trait (file remote for tests; object store + Postgres later) ◀─┘
  vak-lifecycle  one level-triggered reconciler: seal · tier · GC · quotas · retention
  tracing        spans keyed by TraceKey → JSON logs (+ optional OTLP)
```

Crate changes:

- **New `crates/vak-storage`** is the substrate, with no vak dependencies so
  it is testable and fuzzable alone, as `vak-ooxml` is.
- **New `crates/vak-lifecycle`** is the reconciler, with policies as data.
- **`crates/vak-store` is replaced by `crates/vak-catalog`** in the same
  change (invariant 30).
- **Typed ids and `TraceKey` live in `vak-session`** next to `Entry`,
  because every ledger type already depends on it.

## 4. Milestones

Each milestone ships whole: code, tests, docs, and AGENTS.md edits together.
Sizes are relative (S ≈ days, M ≈ one to two weeks, L ≈ several weeks of one
engineer). Dependencies are strict.

```
M0 ─▶ M1 ─▶ M2 ─▶ M3 (5.0.0 cut) ─▶ M4 ─▶ M6 ─▶ M7 ─▶ M8 ─▶ M9
             └────────▶ M5 (after M1, parallel) ─────┘
```

### M0 — Fix what is broken now, with target-shaped fixes (S, stays on 4.x)

Only fixes whose code survives into the target.

- **Delete `AgentSchedule`** (`crates/vak-server/src/agents.rs:47`,
  `record_run`, `list_runs`, `runs_path`, `agents_runs.jsonl`, the `PUT`
  route at `lib.rs:15040`, `vak agents` schedule flags in
  `crates/vak/src/agents_cli.rs`, the client and admin UI fields, and their
  tests). Scheduling an Agent means a `TaskDef` with `agent_id`. Invariant 38
  loses its `AgentSchedule`/`AgentRunRecord` sentence.
- **Stop losing scheduled runs** in `fire_task` (`lib.rs:17472`):
  - Run ids are the full UUIDv7, never a truncated prefix.
  - `worktree::create` runs only when the space is a git repository;
    otherwise the run uses the doc-54 task environment.
  - `advance_marker` runs only for a slot that actually fired.
  - Every refusal ("no provider", "previous run active") becomes an inbox
    `TaskSummary` error carrying `task_id`, never `eprintln!` +
    `return None`. M4 turns these into Run records.
- **The handle id is the ledger session id.** Drop the `-<rid>` suffix in
  `spawn_isolated_run`, so `TaskDef.last_session_id` and `inbox.session_id`
  resolve after a restart.
- **Remove the double agent nesting.** `lib.rs:16528` uses
  `shared_data_home()`, like `lib.rs:12044`, `agent_chats.rs:210` and
  `heartbeat.rs:177`.
- **Stop writing the capability binding whole every turn.**
  `append_turn_capabilities` writes a `TurnCapabilitiesBound` only when its
  digest differs from the session's previous one; an unchanged turn appends
  a small entry naming the prior entry id. This is additive, keeps
  invariant 1 (reconstruction follows the reference), cuts about 70% of
  ledger bytes now, and becomes an object reference in M3.
- **Retired writers:** delete `mcp-artifacts` handling and the legacy
  data-home credential location from any reader. Add them to the M3 purge
  list, not to a cleanup tool.

Exit:
- `fire_task_records_refusal`, `scheduled_run_resolves_after_restart`,
  `two_tasks_due_same_tick_both_fire`, `cron_slot_not_lost_on_failure`,
  `non_git_space_runs_scheduled_task`, `child_core_home_is_not_nested`, and
  `unchanged_capabilities_not_rewritten` (asserts bytes per turn) all pass.

### M1 — Identity and the trace key (M)

- **Typed ids.** Add `vak-session/src/ids.rs`: newtypes `TenantId SpaceId
  AgentId ConversationId SessionId TurnId RunId SpanId ExecutionId
  ArtifactId VersionId ScheduleId DeliveryId`, each `<prefix>_<uuidv7>` with
  parse and validate. `ObjectId` is `sha256:<hex>`. No `String` ids in new
  or touched signatures.
- **`TraceKey` and `Cause`** (doc 73 §4) in `vak-session/src/trace.rs`.
  `TraceKey::child(span_kind)` is the only way to make a span. There is no
  `Default` and no constructor from ambient state.
- **Threading:**
  - `vak_tools::ToolContext` replaces `agent_id: Option<String>` with
    `trace: TraceKey`. The broker `__tool_worker` protocol bumps its version
    and carries it.
  - `SandboxEventSink` and `CandidateRecord` embed it.
  - `vak_agent::AgentConfig` holds the turn's key.
  - Delivery packets, outbox jobs and gateway inbound requests carry it.
  - `vak-server/src/bus.rs::emit` passes the run's trace context and a real
    `prev_hash` (sha256 of the previous envelope), replacing `trace: None`
    and `format!("{}", seq - 1)`.
- **`SessionHeader` gains `space`, `run` and `cause`** (required for new
  sessions; with L3 there are no old ones).
- **Side ledgers carry the full key**, replacing their partial `session_id`
  fields: `CostRow`, `BudgetAlertRow`, `ActivityRow`, `EvidenceRow`
  (routing), `MisreadRow`, `SecurityEvent`, inbox `Entry`, commitment
  `Event`, operations incidents and actions, checkpoint `Manifest`.
- **Enforcement.** A `Traced` trait (`fn trace(&self) -> &TraceKey`) is
  required by the single append helper every side ledger goes through. A
  record type that does not implement it cannot be appended; the compiler is
  the review.

Exit:
- `every_ledger_row_type_is_traced` (compile-level + a registry test listing
  types), `broker_protocol_carries_trace`, `bus_envelope_trace_is_run_trace`,
  `bus_prev_hash_is_hash`, and `session_header_names_cause_for_each_surface`
  pass. The last one covers user, channel, schedule, delegation, revision and
  heartbeat.

### M2 — Storage substrate: `vak-storage` (L)

Pure library, no vak dependencies, fuzzed.

- **`objects`**: `put(bytes) -> ObjectId`, `get`, `has_many`, streaming for
  large blobs, zstd, atomic write, fsync policy, sharded directories.
  Encryption envelope per object with a key id.
- **`records`**: a chain is a directory of segments. `append(entry)` goes to
  the open segment. `seal()` compresses it and writes a header with the
  segment hash and the previous segment's hash. A reader iterates across
  segments. Single-writer by an exclusive lock plus a lease ref (the
  groundwork for M9). Crash consistency: torn final line detection, exactly
  as today's `SessionError::Corrupt`.
- **`refs`**: `get`, `cas(old, new)`, `list(prefix)`. Local backend is one
  SQLite table (reusing the rusqlite pin); an in-memory backend for tests.
- **`keys`**: tenant KEK (sealed through `vak_config::credentials`: OS
  keychain or the encrypted-file fallback), per-conversation DEKs wrapped by
  the KEK, `shred(conversation)`, `hold(scope)`.
- **`Store` trait** over all four, with `LocalStore` implemented. A
  `MemoryStore` is used for unit tests.
- **Dependencies:** zstd, and an AEAD crate (`chacha20poly1305` or
  `aes-gcm`; `aes-gcm` is already used by `vak-bus`, so reuse it). Pin exact
  versions per the code rules.

Exit:
- Property tests: a seal round-trip preserves the entry sequence; hash-chain
  tamper detection; crash at every write step leaves a readable prefix;
  shredding makes content unreadable while file bytes before the tombstone
  are unchanged; GC never removes an object reachable from a ref or record.
- `cargo fuzz` targets exist for the segment reader and object header.

### M3 — The 5.0.0 cutover: layout, writers and slim ledgers (L)

One release that moves every writer onto `vak-storage`, keyed by id.

- **Baseline:**
  - Bump to 5.0.0.
  - `scripts/check-version.sh` passes; this is a forward move, so no
    `.version-reset` entry.
  - Invariant 29 becomes "5.0.0 is the supported baseline".
  - Any pre-5.0.0 data home, manifest, gateway store or config is refused by
    the one shared message (`vak self uninstall --purge`, then install and
    setup).
- **`vak_config::paths` rewritten:**
  - Roots become `tenant_home(ten)`, `runtime_dir()` (wiped at boot),
    `cache_home()` and `logs_dir()`.
  - `agent_home`, `agent_workspace` and the `sessions_home`/
    `shared_data_home` pair are removed.
  - `Core` exposes a `Scope { tenant, space, agent }` and a `StorageHandle`;
    the roughly 296 `sessions_home()`/`shared_data_home()` call sites move to
    typed accessors (`scope.records().sessions()`,
    `scope.shared().gateway()` …). The class of D2 bug becomes
    unrepresentable.
- **Spaces:**
  - `trusted/`, `workspaces.json` and `workspace-names.json` become one
    Desired store of spaces: id, display name, per-machine bindings, trust.
  - `hash_cwd` (9 files) is removed as an identity. A path resolves to a
    space through bindings; an unbound path is admitted as a new space by
    the existing trust gate.
- **Writers moved** (doc 73 Appendix A is the checklist, one PR-sized step
  per row):
  - Session ledgers go on segments.
  - Checkpoints become manifests referencing objects, with no per-session
    blob directory.
  - Sandbox execution streams: stdout/stderr and telemetry go to objects
    chunked per execution; the per-session JSONL keeps only start, finish
    and artifact events with object refs. This bounds today's
    2 rows/s-forever file.
  - Tool results over a threshold go to objects; the ledger keeps the digest
    plus the object id; `recall` resolves it (doc 68 contract unchanged:
    nothing cut blind).
  - `TurnCapabilitiesBound` becomes an object reference.
  - `presentations.json` becomes a catalog projection (M7; until then a ref).
  - Outbox jobs: a settled job's record is sealed into the deliveries
    ledger and its file removed.
  - Feeds and entities move under the tenant.
- **Runtime out of the project tree (L4):**
  - Executions go to `<tenant>/executions/<exe>/`, environments and
    worktrees to `<tenant>/environments/<run>/`, candidates to objects plus
    records.
  - Agent workspaces for non-built-in Agents become environments of kind
    `agent`.
  - The Seatbelt/Landlock profiles in `vak-sandbox` grant exactly the
    execution directory and the space root.
  - Invariant 35 is rewritten; `<space>/.vak/` keeps only `config.toml`,
    `prompts/`, `flows/`, `commands/`, `launch.toml` and
    `permissions.local.toml`.
- **State registry** (`crates/vak-core/src/state.rs`):
  - It becomes a registry of *classes and locations* over five roots
    (tenant, runtime, cache, logs, Shared) plus the space `.vak/` intent set.
  - The enforcement test drives the scenario matrix in §6 and fails on any
    undeclared path in *every* root, including system temp, which the
    runtime directory replaces.
- **Docs:**
  - Doc 64's topology section is rewritten to the real tree.
  - Doc 46 Part VII points to the class registry.
  - Invariants 35 and 37 are updated.
  - The AGENTS.md layout map gains `vak-storage`, and `vak-store` is renamed
    to `vak-catalog` in M7.
  - `docs/architecture/write-paths-and-growth.html` is re-measured and
    republished.

Exit:
- `bytes_per_turn_budget` holds (≤ 20 KB mean over the scripted 50-turn
  corpus).
- `fsyncs_per_turn_budget` holds (≤ 6).
- `no_undeclared_paths_any_root` passes.
- `pre_baseline_home_refused_with_one_message` passes.
- `sandbox_writes_only_execution_dir_and_space` passes.
- `derive_messages_identical_across_seal` passes.
- The full `cargo test --workspace` suite passes, and a live dev run in `/tmp`
  follows `docs/development.md`.

### M4 — Runs and schedules (M)

- **`runs` record chain per tenant.** A `RunRecord` carries `run`, `cause`,
  `schedule?`, `slot?`, `fired_at`, `attempt`, `decision` (`fired |
  skipped{reason} | coalesced | failed{reason}`), `sessions[]`,
  `result_id?`, `deliveries[]`, `cost`, `settled_at` and `outcome`. It is
  written for *every* trigger: user turn, channel request, schedule slot,
  delegation, revision, heartbeat, flow.
- **Exactly once per slot.** `(schedule, slot)` is the idempotency key,
  checked through a ref, not an in-memory `next_fire` map.
  `catch_up_missed_tasks` and `scheduler_tick` share one `due_slots(now)`
  function. A second server process sharing the store cannot double-fire:
  the slot ref is compare-and-swapped.
- **`TaskDef` holds definition only.** The `last_*` fields are removed; "last
  run" is a query over run records.
- **Heartbeats, flows (`flow-runs/`) and the delegation `task` tool** create
  Run records; `vak-flow` state files become records.

Exit:
- `schedule_slot_exactly_once_under_restart` (property test over random
  crash points), `two_processes_do_not_double_fire`, `every_cause_writes_run`
  and `skipped_slot_is_a_record` pass.

### M5 — Telemetry (M, can start right after M1)

- Add pinned `tracing`, `tracing-subscriber` (json, env-filter), and
  optionally `tracing-opentelemetry` + `opentelemetry-otlp` behind the
  `[telemetry] otlp_endpoint` config.
- **Span tree:** `run › turn › step › (dispatch | tool_call › execution) ›
  delivery`. Fields are the TraceKey plus kind and outcome. The worker
  process continues the parent span via the broker protocol's TraceKey.
- **Logs:**
  - Replace every `eprintln!` in library and server crates (vak-server 69,
    vak-core 10, vak-sandbox 8, vak-desktop 7, vak-tools 3, vak-llm 1) with
    leveled events.
  - The CLI keeps `eprintln!` for user-facing output only; a clippy
    `disallowed_macros` config enforces this in library crates.
  - Service logs become `vak-<service>.jsonl` with size-based rotation in
    `vak-ops`.
- **Telemetry is never authoritative.** No code reads logs to make a
  decision.

Exit:
- `library_crates_have_no_eprintln` (clippy config) and
  `one_run_one_trace_id` (collects spans from a scripted run including a
  worker tool and a delivery) pass.
- `log_lines_are_json_with_trace_fields` passes.

### M6 — Lifecycle, retention, erasure and hold (L)

- **`crates/vak-lifecycle`:** policies as data (doc 73 §7.4 defaults).
  `LifecycleReconciler::tick(now)` is idempotent and level-triggered
  (invariant 31). It runs in the server and desktop shell; `vak data` runs
  it once.
  - Transitions: seal idle chains, compress and tier sealed segments, remove
    settled executions, tear down environments after
    promote/reject/expiry, prune checkpoints to policy, mark-and-sweep
    objects (roots = records + refs + holds, with a grace period), enforce
    quotas, and quarantine undeclared paths for one cycle, then delete.
  - Every transition is a record in `audit/lifecycle`.
- **Retention labels** attach to a space or Agent, inherit downward, and
  break explicitly. A label can lengthen retention, never shorten a hold.
- **Erasure:** `vak data erase --conversation <id>` (and the admin API)
  crypto-shreds, appends a tombstone, and GC then reclaims exclusive objects.
- **Legal hold:** `vak data hold --space|--agent|--conversation`. A hold
  blocks every destructive transition and is itself an audit record.
- **Audit export:** `vak data export --audit --since …` writes a signed,
  hash-chained bundle of audit, promotion, grant, erasure and lifecycle
  records.
- **Surfaces:**
  - `vak data status | gc --dry-run | verify | export | erase | hold`.
  - `doctor` shows class totals and quota state; `doctor --repair` invokes
    only mechanical reconciler actions (invariant 19).
  - The Operations Center gets a Data area with measured totals only
    (invariant 26).

Exit:
- `reconciler_is_idempotent`, `settled_execution_leaves_nothing`,
  `gc_keeps_everything_reachable`, `erasure_leaves_ledger_bytes_unchanged`,
  `hold_blocks_every_destructive_transition`, `quota_enforced` and
  `thirty_day_soak_stays_within_budget` (simulated clock, scripted workload)
  pass.

### M7 — Catalog, search and lineage (L)

- **`crates/vak-catalog` replaces `crates/vak-store`**, with tables `nodes`,
  `edges`, `text` (FTS5) and an optional vector index.
  - Incremental ingest follows record segments by offset; there is no
    whole-file re-import (fixes the O(ledger) re-read after every run).
  - `rebuild()` recreates it from records, refs and object manifests.
  - A staleness digest over `(record HEADs, refs)` detects drift.
- **Flat turn-path reads:**
  - Route belief state and routing evidence aggregates become catalog
    projections updated on append (replacing the full scan in
    `plan_route_ladder`).
  - Request admission is a catalog/ref lookup (replacing
    `has_request_admission`'s ledger scan).
  - Commitment state is a projection (replacing replay per append).
- **One API: `search(query, scope)`, `lineage(node, direction, depth)`,
  `open(node)`.** ACL is filtered before ranking (invariant 37).
  - `find_session_on_disk`, `session_search`, `search_all`, admin and
    terminal lookups, `/transcript` resolution and the recall cache all call
    it.
  - The scanning code is deleted in the same change.

Exit:
- `lineage_from_any_artifact_to_cause` passes.
- `search_respects_audience` passes (adversarial cases in
  `vak-server/tests`).
- `catalog_rebuild_equals_incremental` passes.
- `turn_path_reads_flat` passes (turn latency flat from 10 to 10 000 prior
  turns within tolerance).
- `catalog_query_p95_under_50ms_at_1m_nodes` passes.

### M8 — Artifacts, sharing and the information architecture (L)

- **Artifacts and Versions** as records plus objects. Lifecycle is `draft →
  candidate → promoted | shared | published`.
  - Office drafts (doc 72), code changesets, datasets, dashboards and cards
    all become Artifacts. Promotion of a `changeset` into a git space is a
    commit on a branch.
  - Doc 54's candidate and promotion records become Artifact version events
    rather than a parallel store.
- **Grants** with inheritance (space → artifact / conversation), roles
  `viewer | commenter | editor`, and explicit break points. Doc 69's
  coworking grants move into this one table.
- **Client IA** (`crates/vak-client-ui`, `crates/vak-admin-ui`). Six
  first-level places, all backed by the catalog:
  - **Conversations**: per Agent.
  - **Runs**: an Actions-style list with drill-down to sessions, executions,
    outputs and deliveries.
  - **Library**: artifacts by space, kind and audience, with versions,
    comments and shares.
  - **Spaces**: working trees, bindings, trust, git state.
  - **Activity/Audit**: records, erasures, holds.
  - **Data** (admin): classes, quotas, retention, reconciler state.
- The existing six-area admin navigation (doc 33) is reconciled with this.
  There is one navigation model, not two.

Exit:
- `concurrent_edit_creates_sibling_versions`, `share_inherits_and_breaks`
  and `revoked_grant_hides_from_search` pass.
- A browser acceptance run (AGENTS.md acceptance contract) covers create →
  review → promote → share → comment → revise.

### M9 — Cloud remote (L; the protocol and a reference backend, not the service)

- **`Remote` trait in `vak-storage`:** `has_objects`, `put_objects`,
  `get_objects`, `put_segments`, `list_segments`, `cas_ref`, `get_refs`,
  `acquire_lease`, `release_lease`. Backends are a `FileRemote` (tests, and
  personal multi-machine per doc 56) and an `S3Remote` + Postgres ref
  backend behind a feature flag.
- **`vak sync push|pull|status`** plus a background sync plane following
  doc 31 (store-and-forward; network loss never loses data).
  - Secrets never sync.
  - Tombstones, holds and retention decisions sync both ways.
- **Session handoff:** release the lease at a turn boundary; the remote
  acquires it and continues from the same HEAD.
- **Tenancy:** one tenant per remote namespace; keys never leave the
  tenant; the key-escrow design for the hosted service is written as a
  separate doc before any hosted backend ships.

Exit:
- `push_pull_roundtrip_identical_derive_messages`,
  `handoff_at_turn_boundary`, `lease_prevents_dual_writer`,
  `erasure_propagates_and_cannot_resurrect` and
  `sync_survives_network_loss` pass.

## 5. AGENTS.md and design-doc changes, by milestone

| When | Change |
|---|---|
| M0 | Invariant 38: drop `AgentSchedule`/`AgentRunRecord`. Doc 64: fix the topology section to the real 4.x tree. List doc 73 under "Proposals, not behaviour". |
| M1 | New invariant: **every durable record, span and envelope carries a `TraceKey`; a writer that cannot fill it does not write.** |
| M3 | Invariant 29 → 5.0.0 baseline. Invariant 35 rewritten (runtime state under the tenant). Invariant 37 paths updated. New invariant: **every path belongs to a declared class; ledgers hold references, not bulk content.** Layout map: `vak-storage`. Doc 73 status → "in progress". |
| M5 | Code rule: no `eprintln!` in library crates; telemetry is never read to decide. |
| M6 | New invariant: **destruction is the reconciler's alone, is recorded, and yields to holds; Record-class data is erased only by crypto-shred.** Invariant 19 references `vak data`. |
| M7 | Layout map: `vak-store` → `vak-catalog`. Invariant 37: search and lineage filter ACL before ranking. |
| M8 | Doc 54 and doc 69 sections superseded by Artifact/Grant. |
| M9 | Doc 56 superseded by the Remote section. Doc 31 gains the sync plane. |

## 6. Standing test matrix

One scripted scenario corpus (`crates/vak-eval`, deterministic, no network)
is reused by every milestone's exit tests:

- **Interactive:** a CLI turn, a web turn with approval, a desktop turn with
  an image attachment.
- **Channels:** a Telegram inbound with a document, two bots in one chat
  (invariant 24).
- **Schedules:** a cron slot, an interval, a one-shot, a script watchdog, a
  catch-up after downtime, two tasks due in the same tick, a non-git space.
- **Delegation:** a `task` child worker with cards (invariant 38),
  best-of-N, a flow, a dynamic plan.
- **Work products:** an Office edit through review and promotion
  (invariant 39), a code changeset promoted into a git space, a revision of
  a candidate.
- **Delivery:** outbox success, outbox retry after failure, an approval
  forwarded to a channel.
- **Lifecycle:** erasure of one conversation sharing objects with another;
  a hold during GC; a crash at each storage write step.

Each run asserts: no undeclared paths; every record traced; one trace per
run; budgets (bytes/turn, fsyncs/turn); zero ephemeral residue after
settle; lineage resolvable from every produced node.

## 7. Risks and mitigations

| Risk | Mitigation |
|---|---|
| M3 is a big-bang change across about 300 call sites. | M1 and M2 land first with no behaviour change. M3 moves one Appendix-A row per commit on a branch behind the 5.0.0 version, and `no_undeclared_paths_any_root` is the running checklist. |
| Moving scratch out of the project tree breaks tools that expect relative temp paths. | The broker already sets `TMPDIR`/`XDG_CACHE_HOME` explicitly. The `sandbox_writes_only_execution_dir_and_space` test plus a live run of the acceptance dashboard scenario gate the change. |
| Encryption on a single-user laptop adds key-loss risk. | The KEK lives in the OS keychain (or the existing encrypted-file fallback). `vak data export` includes a key-escrow bundle when asked. A lost key is loud (`doctor`), never a silent empty history. |
| Catalog drift from records. | The catalog is Derived: a staleness digest plus `rebuild()`, and `catalog_rebuild_equals_incremental` runs in CI. |
| Telemetry volume. | Leveled, sampled per span kind, size-rotated, quota-managed like any Telemetry class. |
| Cloud design leaks into local complexity. | M9 is last. Before it, `Remote` exists only as a trait plus `FileRemote`. |

## 8. Before M3 on this dev machine

With L3 and L5 there is no cleanup tool to build for old state. Until M3,
the dead pre-`31c1bb9c` checkpoints (about 1.6 GB) and the
`agents/vak/agents/` nesting may be removed by hand. At M3, every dev data
home is purged with `vak self uninstall --purge` and set up fresh.
