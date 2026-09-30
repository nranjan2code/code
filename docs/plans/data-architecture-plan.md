# Plan — data architecture, lifecycle, tracing and cloud

Status: **plan, revision 2 (2026-09-25). M0 is done (2026-09-25); M1 is
next and waits for the maintainer (see AGENTS.md, "Pending").**

- Design: `docs/design/73-data-architecture-and-lifecycle.md` (the model)
  and `docs/design/74-lifecycle-and-data-administration.md` (lifecycles,
  policies, screens, API).
- Revision 2 applies every fix in `docs/plans/data-architecture-review.md`.
- What each milestone touches: `docs/plans/data-architecture-blast-radius.md`.
- Baseline measurements: `docs/architecture/write-paths-and-growth.html`
  (v3.5.1) and doc 73 §2.

## 1. Decisions

Locked 2026-09-25 with the maintainer:

| # | Decision | Consequence |
|---|---|---|
| L1 | **Local-first, cloud as a remote.** A desktop or server works offline on its own store; a hosted cloud is a push/pull remote with session handoff. | One storage trait with two backends (local disk + SQLite; object store + Postgres). No path is ever an identity. |
| L2 | **Compliance bar for the first customer release:** retention labels, erasure (crypto-shred), legal hold, exportable audit trail. Region pinning, BYOK and eDiscovery come with the cloud phase. | Keys, holds, lineage and receipts are in the local design from day one. |
| L3 | **Cut a new 5.0.0 baseline.** The workspace is at 4.0.2; 5.0.0 refuses every earlier data home with the single invariant-29 message. | No migrator, no reader for old shapes, no dual layout. Dev data homes are purged at the cutover. |
| L4 | **Runtime state leaves the project tree.** A space's `.vak/` holds only committable intent. | Invariant 35 is rewritten; sandbox profiles grant the execution directory explicitly. |
| L5 | **Zero users, zero compatibility.** | Each milestone *replaces* what it supersedes in the same change (invariant 30). An early fix is made only when it is the target behaviour. |

Decided in the plan, each with a clear default:

- **Catalog:** SQLite locally, Postgres in the cloud, behind one trait.
- **Telemetry:** `tracing` + `tracing-subscriber` (JSON), OTLP optional,
  never carrying content; `eprintln!` banned in library crates.
- **One schedule model:** `TaskDef`; `AgentSchedule` is deleted.
- **Crypto** (review R1):
  - AEAD from `ring` (already a workspace dependency).
  - Per-object random keys, and object ids that are a keyed hash within the
    tenant.
  - Key *grants* per referencing scope.
  - Records encrypted per entry.
  - Keys per **conversation**, spanning session rotation.
- **Content is keyed to its conversation wherever it is written**, and
  every derived write records `derived_from` (review R2).
- **Provider-account deletion is a first-class erasure scope.** Mail/calendar
  content, attachments, candidates, previews, citations, automation cursors,
  cached/indexed copies and model-visible excerpts must retain account
  lineage across Agent and conversation boundaries. M1/M2/M6 must design and
  prove selective crypto-erasure of that source data without erasing unrelated
  conversation content; M7 must execute it and report external copies outside
  Vak's control. An account-scoped key/grant or equivalent selective-erasure
  mechanism is required; conversation-only keys are insufficient. See docs
  73 §7.3, doc 74 §2.15 and M7 below.
- **Deletion is two-step:** trash (hidden everywhere, restorable), then
  erase (crypto-shred with a receipt) (doc 74 §2.4).
- **Encryption at rest** is a per-tenant policy, on by default.
  `vak data cat|grep|export --plain` keeps the transparency the product
  promises (review R10).
- **Data roles:** Owner, Steward, Operator, Member, Auditor (doc 74 §3.6).
  Full product RBAC is a separate design.
- **Plans live in `docs/plans/`; designs stay in `docs/design/`.**

## 2. Exit criteria for the whole plan

All of these hold on a clean 5.0.0 install, proven by the standing tests
in §6:

1. **Traceable.** From any object, record, artifact, delivery or schedule
   slot, one catalog query returns its Run, Session, Turn, Agent, Space and
   Cause. It works after a restart, in under 50 ms at 1 M catalog nodes.
2. **Declared.** Every path any process writes belongs to a declared class.
   The scenario matrix runs across *every* root (tenant, runtime, cache,
   logs, Shared, the space `.vak/`, and system temp) and fails on anything
   undeclared.
3. **Bounded.**
   - Ephemeral state is zero once executions settle.
   - Caches and telemetry stay within quota.
   - Record growth per turn is at most 20 KB mean (today 66 KB) and at
     most 6 fsyncs (today about 13).
   - Settled delivery jobs leave no file behind.
4. **Flat reads.** No read on the turn path is O(history).
5. **Governed.**
   - Labels, holds, trash and erasure work end to end.
   - Erasure follows lineage into every derived copy.
   - It leaves append-only files byte-identical apart from appended
     tombstones.
   - It yields a receipt that names what it could not reach.
   - A restored backup re-applies every erasure.
6. **Honest deletion.** Nothing a person trashed or erased appears in any
   list, search, recall, prompt, export or digest.
7. **Portable.** A session pushed to a file-backed remote, with its lease
   released, continues on a second data home from the same HEAD, with
   identical `derive_messages()` output.
8. **Observable.**
   - Every log line is JSON with TraceKey fields and **no content**.
   - One run is one trace across the loop, tools, worker, delivery and bus.
9. **Administrable.** Every screen in doc 74 §6 is live on measured data,
   and the browser acceptance scenarios in doc 74 §9 pass.

Non-goals: the hosted cloud service itself, BYOK, an eDiscovery UI, region
routing, and full product RBAC.

## 3. Target architecture

```
                 ┌──────────── surfaces: server · desktop · CLI · term · channels ────────────┐
                 │ Conversations · Runs · Library · Schedules · Data · Governance · Diagnostics│
                 └─────────────────────────────────▲──────────────────────────────────────────┘
                                                   │ search · lineage · open (ACL first)
 vak-lifecycle ── reconciler: observe → plan → guard → stage → commit → record
        │  policies (labels, holds, quotas), erasure requests, receipts
        ▼
 vak-catalog (Derived) ── nodes · edges(lineage) · text(FTS) · lifecycle state
        ▲ idempotent ingest (chain, seq)
 vak-session · vak-commit · vak-delivery · vak-sandbox · vak-core  ── write through ──▶ vak-storage
 vak-storage (no vak deps; fuzzed):
    objects   keyed-hash ids, zstd, per-object key, key grants per scope
    records   append-only chains of segments; per-entry AEAD frames; verified seal
    refs      CAS pointers (heads, current versions, slot claims, leases)
    keys      tenant KEK (credential store) → conversation / space / artifact keys
    remote    trait (FileRemote first; S3 + Postgres later)
 tracing  spans keyed by TraceKey, content-free → JSON logs (+ OTLP)
```

## 4. Milestones

Each milestone ships whole: code, tests, docs, AGENTS.md, and the screens
listed for it. Sizes are relative: S ≈ days, M ≈ 1–2 weeks, L ≈ several
weeks, XL ≈ a month or more for one engineer. Dependencies are strict.

```
M0 ─▶ M1 ─▶ M2 ─▶ M3a ─▶ M3b (5.0.0) ─▶ M4 ─▶ M6 ─▶ M7 ─▶ M8 ─▶ M9
             └──────────▶ M5 (after M1, parallel with M2–M4) ─┘
```

The catalog (M6) now comes *before* lifecycle (M7), because erasure and
holds need lineage and scope queries (review R2).

### M0 — Fix what is broken now (M, on 4.x)

Only fixes whose code survives into the target.

**Security and honesty**
- **Bus secrets** (review R6):
  - `PUT /config/bus` stores the NATS JWT and NKey seed through
    `vak_config::credentials` in the project secret scope.
  - Bus resolution reads them, and they apply live (invariant 31).
  - Every `.vak/env` path is deleted.
  - A new test fails if any API writes a secret-shaped value to a file.
- **Trash is honoured everywhere** (review R8). One `is_trashed(session)`
  source is used by the session list, `session_search`, `search_all`, admin
  search, the FTS query path, recall, `/transcript`, export and digest.
  Today's "delete" is labelled what it is: "Move to trash (hidden
  everywhere)". Real erasure lands in M7.
- **Purge includes Logs** (`crates/vak/src/install/mod.rs:1093`, review R7).
  The wholesale purge of Vak-owned roots comes in M3b.
- **Feeds path** (review R15): Rust passes `VAK_FEEDS_DB`; the resolver in
  `scripts/feeds/feed_utils.py` is deleted.

**Scheduling**
- **Delete `AgentSchedule`** everywhere, per the blast-radius M0 list. Fix
  the docs that claimed it shipped: AGENTS.md invariant 38, doc 65 §58-62,
  doc 72 §176/261.
- **`fire_task`** (`crates/vak-server/src/lib.rs:17472`):
  - Full UUIDv7 run ids.
  - A git worktree only when the space is a repository.
  - A **non-git space is refused loudly**: a new inbox kind `RoutineFailed`
    carries the reason and remedy (review R5). The real fix is M4's
    `CopyEnvironment`.
  - Every refusal is a `RoutineFailed` notification, never `eprintln!` +
    `return None`.
  - `advance_marker` runs only for a slot that fired.
- **Handle id = ledger id** in `spawn_isolated_run`, and `lib.rs:16528`
  uses `shared_data_home()`.

**Growth**
- `append_turn_capabilities` writes the full binding only when its digest
  changes; otherwise it writes a reference entry. `projection.rs:582`
  follows the reference. The bytes-per-turn test prints the new
  measurement (review R36).

**Docs**
- Doc 64's topology section describes the real 4.x tree.
- AGENTS.md lists docs 73 and 74 under "Proposals, not behaviour", with
  a "Pending" section on how to pick this plan up. Done 2026-09-25; keep
  that section current as milestones land.

**Screens**
- Client: conversation menu wording and the trash filter.
- Admin: remove agent-schedule fields.

**Exit tests**
- `bus_credentials_never_written_to_a_file`,
  `trashed_session_absent_from_every_search`, `purge_includes_logs`,
  `feeds_write_under_overridden_home`.
- `fire_task_records_refusal`, `scheduled_run_resolves_after_restart`,
  `two_tasks_due_same_tick_both_fire`, `cron_slot_not_lost_on_failure`.
- `non_git_space_routine_is_refused_loudly`,
  `child_core_home_is_not_nested`, `unchanged_capabilities_not_rewritten`.

### M1 — Identity, trace key, provenance (M)

- **Typed ids** in `vak-session/src/ids.rs`; **`TraceKey` and `Cause`** in
  `trace.rs` (doc 73 §4). `TraceKey::child` is the only way to make a span.
- **`SessionHeader` gains `space`, `run` and `cause`.**
- **Threading:**
  - `ToolContext.agent_id` becomes `trace`.
  - The broker protocol bumps its version.
  - `SandboxEventSink` and the vak-sandbox records carry the key.
  - `AgentConfig`, delivery packets, outbox jobs and `InboundRequest` carry
    it too.
  - The bus envelope gets the run's trace and a real `prev_hash` (review
    R3/D9).
- **A `Traced` bound on the one side-ledger append helper.** Every row type
  in blast-radius M1 carries the full key.
- **Provenance:** memory, learning, reflection and entity writes record
  `derived_from` (review R2). Checkpoint labels become the turn id (review
  R31).
- **External source provenance:** define typed, opaque provider-account source
  ids and carry them with `derived_from` on all provider-originated content and
  descendants, independently of conversation, Agent, and audience ownership.
  This is required for selective account erasure without erasing unrelated
  conversation content (docs 73 §7.3 and 74 §2.15).
- **The 13 clock-derived or truncated ids become typed ids.**
- **`TestScope` helper** (test feature of `vak-config`), so M3a's test
  migration is mechanical (review R30).
- **Data dictionary:** `docs/reference/records.md`, generated from the
  record types by a test that fails on drift (review R29).
- **Screens:** FinOps per Agent and per run (A19 data).

**Exit tests**
- `every_ledger_row_type_is_traced`, `broker_protocol_carries_trace`,
  `bus_envelope_trace_is_run_trace`, `bus_prev_hash_is_hash`.
- `session_header_names_cause_for_each_surface`,
  `derived_writes_record_provenance`, `records_reference_is_current`.

### M2 — Storage substrate `vak-storage` (L)

A library only, with no behaviour change elsewhere.

- **`objects`:**
  - The id is HMAC-SHA256 under a tenant id key, so dedupe works inside a
    tenant but can't confirm a file across tenants.
  - zstd compression; a per-object random key sealed with `ring::aead`.
  - Key grants per referencing scope; streaming for large blobs; sharding;
    atomic writes.
- **`records`:**
  - Chains of segments; each entry is an AEAD frame under its
    conversation's key (plaintext frames when tenant policy is off).
  - The hash chain carries on from today's per-entry `prev_hash`.
  - **Verified seal:** copy, check count, hashes and chain, atomic swap,
    then a seal entry (review R17).
  - Single writer by flock locally; leases only with a remote (review R18).
  - Torn-tail detection.
- **`refs`:** CAS in SQLite (rusqlite pinned in the workspace manifest), and
  an in-memory backend.
- **`keys`:**
  - Tenant KEK through `vak_config::credentials`.
  - Account-scoped key/grant composition for provider-originated content, so
    deleting one connected account does not require deleting an entire
    conversation; prove append-only model-visible record behavior under that
    composition before admitting such content.
  - Conversation, space and artifact keys wrapped by the KEK.
  - `shred(scope)` and `hold(scope)` hooks.
- **`Store` trait** with `LocalStore` and `MemoryStore`; the `Remote` trait
  is declared.
- **New pinned dependency:** `zstd`. `async-nats` moves into the workspace
  manifest with an exact pin.

**Exit tests**
- Property tests: seal round-trip, tamper detection, a crash at every step
  leaves a readable prefix.
- Shred: file bytes before the tombstone are unchanged and content is
  unreadable.
- GC never collects an object with a live grant; keyed ids dedupe within a
  tenant and differ across tenants.
- Fuzz targets for the frame and segment readers.

### M3a — Scope API on the current layout (L, no behaviour change)

- `vak_config::paths` gains `Scope`/`StorageHandle` accessors that resolve
  to *today's* paths.
- Every production home-path use is replaced with typed accessors: 552 uses
  in 65 files, including 200 `sessions_home()` and 69 `shared_data_home()`
  calls.
- `Core::sessions_home`, `shared_data_home` and `set_sessions_home` are
  removed at the end.
- Tests: all 500 layout references in 66 files move to `TestScope`. The
  unchanged suite is the oracle.

**Exit:** the full `cargo test --workspace` passes with zero behaviour
diff; the layout scan (blast-radius §0) finds no raw home-path use outside
`vak-config`.

### M3b — The 5.0.0 layout switch (XL)

- **Baseline:**
  - Version 5.0.0; invariant 29 becomes "5.0.0 is the supported baseline".
  - Pre-5.0.0 state is refused by the one message.
- **Purge:** Vak-owned roots (data, cache, logs, runtime) are removed
  wholesale; Shared uses declared entries only; Vak runtime leftovers in
  known spaces are listed and offered (review R7).
- **Layout:** tenant tree (doc 73 §6); every writer moves (doc 73 Appendix
  A).
- **Classes (review R9):**
  - Memory, entities, skills, prompt layers and presentation packs become
    **Document** class.
  - FinOps and alerts become segment chains without compaction rewrites.
- **Slim ledgers:**
  - The capability binding and large tool results move to objects.
  - Sandbox streams are chunked to objects.
  - A settled outbox job is sealed.
- **Runtime out of the project tree:**
  - Executions and environments live under the tenant.
  - Sandbox profiles grant only the execution directory and the space root.
  - `App.tsx:903` loses its `.vak/scratch` check.
- **Space identity** (review R12): each item is keyed by space id in this
  same change:
  - credential scopes (`scope_key_for`), trust markers
  - CorePool identity, allowlist workspace fields, the
    `gateway/default-workspace` file
  - `TaskDef.cwd` and the scheduler filter
  - `/workspaces`, the desktop trust gate
  - `[server] workspace_roots`
- **State registry:** classes × roots, driven by the §6 matrix. The upgrade
  gate uses class snapshots.
- **Docs, site, scripts, services:** every item in blast-radius M3b.

**Screens**
- Admin: Spaces (A13).
- Admin: `SessionForensics.tsx` groups by space instead of project hash.

**Exit tests**
- `bytes_per_turn_budget`, `fsyncs_per_turn_budget`,
  `no_undeclared_paths_any_root`, `pre_baseline_home_refused_with_one_message`.
- `purge_removes_owned_roots_wholesale`,
  `sandbox_writes_only_execution_dir_and_space`,
  `derive_messages_identical_across_seal`, `secret_scopes_keyed_by_id`.
- Full suite, plus a live dev run in `/tmp` per `docs/development.md`.

### M4 — Runs and schedules (L)

- **A `runs/` record chain.** A `RunRecord` is written for every cause:
  user turn, channel, schedule slot, delegation, revision, heartbeat, flow,
  best-of-N, and export.
- **At most one start per slot** (review R4):
  - Claim `(schedule, slot)` by CAS before any side effect.
  - A lease-expired run is recorded `abandoned`.
  - `on_crash = skip | retry_once`.
  - One `due_slots(now)` serves the tick, catch-up and run-now; the
    in-memory `next_fire` map goes.
- **`TaskDef`:** the `last_*` fields go; "last run" is a query.
- **`CopyEnvironment`:** the first real `EnvironmentBackend`
  (`vak-sandbox`). It copies a non-git space with ignore rules and size caps
  into `environments/<run>/`, runs there, and exports the changes as a
  candidate for Review. This replaces M0's refusal.
- **Screens:**
  - Admin: Runs (A4), Run detail (A4b), Schedules (A5, with definitions
    moved from Configure).
  - `#/operations/work/runs/<session_id>` becomes `#/runs/<run_id>`
    (review R13).
  - Client: Runs panel (C6) replaces the `TasksModal` last-run fields.

**Exit tests**
- `schedule_slot_at_most_once_under_restart` (property test over crash
  points), `two_processes_do_not_double_start`, `abandoned_run_is_recorded`.
- `every_cause_writes_run`, `skipped_slot_is_a_record`,
  `non_git_space_routine_runs_in_copy_environment`.

### M5 — Telemetry (M, after M1, in parallel)

- Pinned `tracing`/`tracing-subscriber` (json, env-filter); optional
  `tracing-opentelemetry` + `opentelemetry-otlp` via
  `[telemetry] otlp_endpoint`.
- The span tree runs `run › turn › step › (dispatch | tool_call ›
  execution) › delivery`. The worker continues the parent span.
- **Content-free** (review R3): fields are ids, kinds, sizes, digests,
  durations and outcomes only.
- The 94 library/server `eprintln!` become leveled events; the CLI keeps
  user output; `clippy.toml` sets `disallowed-macros` for library crates.
- Service logs become `vak-<service>.jsonl` with rotation (`vak-ops`).
- Bus payloads carry references only; streams get `max_age` (review R22).
- **Screens:** System › Diagnostics › Traces & logs (A17); span waterfall
  in Run detail.

**Exit tests**
- `library_crates_have_no_eprintln`, `one_run_one_trace_id`,
  `log_lines_are_json_with_trace_fields`, `telemetry_carries_no_content`.

### M6 — Catalog, search, lineage (L)

- **`crates/vak-catalog` replaces `crates/vak-store`** (same change):
  `nodes`, `edges`, `text` (FTS5), optional vectors.
  - Ingest is idempotent by `(chain, seq)` from any process, WAL with
    `busy_timeout` (review R19).
  - `rebuild()` recreates it, and a staleness digest detects drift.
- **Flat turn-path reads:**
  - Route evidence aggregates and belief become projections.
  - Request admission is a ref lookup.
  - Commitment state is a projection.
- **One API: `search`, `lineage`, `open`**, ACL-filtered before ranking.
  The scanning lookups (`find_session_on_disk`, `read_historical_header`,
  `find_session_in_cwd`, and the recall ledger cache) are deleted.
- **Screens:**
  - Admin Search and client search call the one API.
  - Conversation detail gains the Lineage tab (A3).
  - Integrity (A9) shows catalog staleness and offers rebuild.

**Exit tests**
- `lineage_from_any_artifact_to_cause`, `search_respects_audience`,
  `catalog_rebuild_equals_incremental`.
- `turn_path_reads_flat`, `catalog_query_p95_under_50ms_at_1m_nodes`.

### M7 — Lifecycle, retention, erasure, hold, backup (XL)

- **`crates/vak-lifecycle`**: the reconciler, lifecycles, policies (doc 74
  §2–§5). It first ships **observe-only** (it computes and shows the plan
  and commits nothing) across the whole scenario corpus. Commit is then
  enabled one action class at a time, ephemeral first and records last.
- **Remove the scattered retention** (review R25):
  - checkpoints `MAX_STORED_CHECKPOINTS`, finops and alerts compaction,
    memory `cleanup_artifacts`, `/memory/cleanup`
  - the `archive.json`/`deleted.json` sidecars and their routes
  - `/workspaces/forget`
- **Erasure (doc 74 §4):**
  - Scopes: provider account, conversation, person, Agent, space, tenant.
  - Preview digests and approvals.
  - A lineage walk, key destruction, derived plaintext removal
    (`secure_delete` plus a WAL checkpoint), and a signed receipt naming
    what it couldn't reach.
  - Provider-account erasure follows source lineage across conversations and
    Agents, destroys the account-scoped key/grants, removes provider-derived
    records and indexes while preserving unrelated conversation data, and
    reports provider dispatches, recipients, backups, and other copies outside
    Vak's control. Disconnect/revocation alone is not erasure.
- **Holds, labels and quotas** (doc 74 §3).
- **Agent lifecycle:** add `Revoked`, and wire each state's data effects
  (review R26).
- **Allowlist PII retention**, with hashed sticky-deny fingerprints.
- **Backup rework** (review R21): ciphertext plus wrapped keys, and restore
  re-applies erasures.
- **Data roles** (doc 74 §3.6).
- **Screens:**
  - Admin: Home › Data health (A1), Conversations lifecycle (A2/A3),
    Storage (A7), Lifecycle (A8), Integrity (A9).
  - Admin: Retention & holds (A10), Erasure requests (A11), Keys (A12),
    Agents lifecycle panel (A14), Audit export (A15), Backup & restore
    (A16).
  - Client: menu (C1), Trash (C2), "Why is this gone?" (C3), Your data
    (C7), Workbench states (C8).
- **CLI:** `vak data …` (doc 74 §8).

**Exit tests**
- `reconciler_is_idempotent`, `reconciler_observe_only_commits_nothing`,
  `settled_execution_leaves_nothing`, `gc_keeps_everything_reachable`.
- `erasure_follows_lineage`, `erasure_leaves_ledger_bytes_unchanged`,
  `person_erasure_spans_agents_and_chats`,
  `provider_account_erasure_spans_agents_and_conversations`,
  `provider_account_erasure_preserves_unrelated_conversation_content`,
  `provider_account_erasure_reapplies_after_restore`,
  `hold_blocks_every_destructive_transition`.
- `stale_preview_cannot_authorise`, `quota_refuses_admission_not_records`,
  `restore_reapplies_erasures`, `revoke_cuts_endpoints_within_one_tick`.
- `thirty_day_soak_stays_within_budget`.
- The browser acceptance scenarios in doc 74 §9.

### M8 — Artifacts, sharing, information architecture (L)

- **Artifacts and Versions.** Candidates and promotions (doc 54), Office
  drafts (doc 72), changesets, datasets, dashboards and cards all become
  artifact version events. A changeset promotes into git as a commit on a
  branch.
- **Grants** with inheritance and explicit breaks; doc 69's coworking grants
  move into the same table.
- **Navigation reconciliation** in the admin console (doc 74 §6.1) and the
  client.
- **Screens:**
  - Admin Library (A6).
  - Client Library (C4), Share dialog (C5), version history.
  - `SharedConversation.tsx`, `ArtifactCanvas`, `OfficeRedline` and
    `WorkbenchPanel` move onto the Artifact API.

**Exit tests**
- `concurrent_edit_creates_sibling_versions`, `share_inherits_and_breaks`,
  `revoked_grant_hides_from_search`.
- A browser acceptance run: create → review → promote → share → comment →
  revise → erase.

### M9 — Cloud remote (L; the protocol and a reference backend)

- **The `Remote` trait**, with `FileRemote` (tests and personal
  multi-machine) and `S3Remote` + Postgres refs behind a feature flag.
- **`vak sync`** and a sync plane following doc 31 (store-and-forward):
  - Secrets never sync.
  - Tombstones, holds and receipts sync both ways.
- **Leases** layered over local flocks; handoff happens at a turn boundary.
- **Tenancy:** the key-escrow design for a hosted service is written as its
  own doc before any hosted backend ships.
- **Screens:** Sync (A18).

**Exit tests**
- `push_pull_roundtrip_identical_derive_messages`,
  `handoff_at_turn_boundary`, `lease_prevents_dual_writer`.
- `erasure_propagates_and_cannot_resurrect`, `sync_survives_network_loss`.

## 5. AGENTS.md and design-doc changes

| When | Change |
|---|---|
| M0 | **Invariant 38:** drop `AgentSchedule`/`AgentRunRecord`. **Docs 65, 72:** remove the claims. **Doc 64:** the real 4.x topology. Docs 73 and 74 listed under proposals, plus the AGENTS.md "Pending" section (done 2026-09-25). **Invariant 8:** add "no API writes a secret to a file". |
| M1 | **New invariant:** every durable record, span and envelope carries a `TraceKey`; derived writes record `derived_from`. |
| M2 | **Invariant 2:** entries are never rewritten; encoding changes only by a verified seal. |
| M3b | **Invariant 29** → 5.0.0 baseline. **Invariant 35** rewritten (runtime state under the tenant). **Invariant 37** paths. **New invariant:** every path belongs to a declared class; ledgers hold references, not bulk content; content is keyed to its conversation wherever it is written. Layout map gains `vak-storage`. Doc 73 status → "in progress". |
| M4 | **Invariant 38:** scheduled runs as Run records with the at-most-once-per-slot contract. |
| M5 | **Code rule:** no `eprintln!` in library crates; telemetry is content-free and never read to decide anything. |
| M6 | **Layout map:** `vak-store` → `vak-catalog`. **Invariant 37:** search and lineage filter ACL before ranking. |
| M7 | **New invariant:** destruction is the reconciler's alone, is recorded, yields to holds, and follows lineage; Record data is erased only by crypto-shred; trash is hidden everywhere. **Invariant 19** references `vak data`. **Invariant 37:** `Revoked`. Docs 14, 23, 28, 33, 44 and 46 Part VII updated. |
| M8 | Docs 54, 69 and 72 sections superseded by Artifact/Grant; docs 33 and 58 navigation. |
| M9 | Doc 56 superseded; doc 31 gains the sync plane; doc 53 subjects use space ids. |

## 6. Standing scenario matrix

One deterministic corpus in `crates/vak-eval` (no network), reused by every
milestone's exit tests. It also drives the state-registry enforcement test,
which today covers only one session and one security event (review R24).

- **Interactive:** a CLI turn; a web turn with approval; a desktop turn
  with an image attachment.
- **Channels:** a Telegram inbound with a document; two bots in one chat
  (invariant 24).
- **Schedules:** a cron slot; an interval; a one-shot; a script watchdog; a
  catch-up after downtime; two tasks due in the same tick; a non-git space;
  a crash mid-run.
- **Delegation:** a `task` child with cards; best-of-N; a flow; a dynamic
  plan.
- **Work products:** an Office edit through review and promotion; a code
  changeset into a git space; a candidate revision.
- **Delivery:** outbox success; retry after failure; an approval forwarded
  to a channel.
- **Lifecycle:**
  - trash → restore → erase;
  - person erasure across two Agents;
  - a hold during erasure and during GC;
  - label shortening;
  - quota pressure;
  - a restore that re-applies erasures;
  - a crash at every storage write step.

Each run asserts:
- no undeclared paths in any root;
- every record traced;
- one trace per run;
- no content in telemetry;
- bytes and fsyncs per turn within budget;
- zero ephemeral residue after settle;
- lineage resolvable from every produced node;
- trashed or erased items absent from every read path.

## 7. Risks and mitigations

| Risk | Mitigation |
|---|---|
| M3 size (552 production uses, 500 test references) | Split into M3a (refactor, oracle = the existing suite) and M3b (switch behind the finished API). |
| A reconciler bug destroys data | Staged quarantine before commit. It ships observe-only first, then commits one action class at a time. Every destructive transition is recorded. Holds are checked at commit time, not only at plan time. |
| The crypto design is subtle | The substrate has no vak dependencies and gets property tests and fuzzing (M2). Primitives come from `ring` only. There is no custom construction beyond HMAC ids and AEAD frames. The key hierarchy is documented in doc 73 §7.3. |
| Moving scratch out of the project tree breaks tools | The broker already sets `TMPDIR`/caches. A sandbox test plus the acceptance dashboard scenario gate it. |
| Encryption hides data from people | `vak data cat/grep/export --plain`, and an honest headless note. |
| Catalog drift | It is Derived: a staleness digest, `rebuild()`, and a rebuild-equals-incremental test in CI. |
| Erasure misses a copy | The content-keyed rule plus `derived_from` on every derived write; `erasure_follows_lineage` runs over the full corpus; receipts name what couldn't be reached. |
| Telemetry volume and leakage | Content-free by test; leveled; sampled; rotated; quota-managed. |
| Cloud leaks into local complexity | M9 is last; before it `Remote` is only a trait plus `FileRemote`. |

## 8. Until M3b on this dev machine

There is no migration tool to build (L3, L5). The dead pre-`31c1bb9c`
checkpoints (about 1.6 GB) and the `agents/vak/agents/` nesting may be
removed by hand. At M3b every dev data home is purged, and the new purge
removes the owned roots wholesale.
