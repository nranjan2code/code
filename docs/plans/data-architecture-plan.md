# Plan — data architecture, lifecycle, tracing and cloud

Status: **plan, revision 3 (2026-10-01). M0 is done (2026-09-25, shipped in
5.0.0), and so are the two 5.x guards (§4, "Now", 2026-10-01) and M1
(2026-10-02). M2 is in progress and M3a is next. Each step waits for the maintainer (see AGENTS.md,
"Pending").**

- Design: `docs/design/73-data-architecture-and-lifecycle.md` (the model)
  and `docs/design/74-lifecycle-and-data-administration.md` (lifecycles,
  policies, screens, API).
- Revision 2 applied every fix in `docs/plans/data-architecture-review.md`.
- Revision 3 applies every fix and decision in
  `docs/plans/data-architecture-review-2.md`.
- What each milestone touches: `docs/plans/data-architecture-blast-radius.md`.
- Baseline measurements: `docs/architecture/write-paths-and-growth.html`
  (v3.5.1) and doc 73 §2.

## 1. Decisions

Locked 2026-09-25 with the maintainer; L3 re-locked 2026-10-01:

| # | Decision | Consequence |
|---|---|---|
| L1 | **Local-first, cloud as a remote.** A desktop or server works offline on its own store; a hosted cloud is a push/pull remote with session handoff. | One storage trait with two backends (local disk + SQLite; object store + Postgres). No path is ever an identity. |
| L2 | **Compliance bar for the first customer release:** retention labels, erasure (crypto-shred), legal hold, exportable audit trail. Region pinning, BYOK and eDiscovery come with the cloud phase. | Keys, holds, lineage and receipts are in the local design from day one. M7a and M7b are both required before that release. |
| L3 | **Cut a new data baseline at 6.0.0.** 5.0.0 was spent on M0's removals (2026-09-26). This row is the one place the number is written: every other document says "the data baseline" and points here. | The baseline refuses every earlier data home with the single invariant-29 message. No migrator, no reader for old shapes, no dual layout. Dev data homes, and the maintainer's 5.x hosts, are purged at the cutover. |
| L4 | **Runtime state leaves the project tree.** A space's `.vak/` holds only committable intent. | Invariant 35 is rewritten; sandbox profiles grant the execution directory explicitly. |
| L5 | **Zero users, zero compatibility.** | Each milestone *replaces* what it supersedes in the same change (invariant 30). An early fix is made only when it is the target behaviour. |

Locked 2026-10-01 with the maintainer (review 2, §4):

| # | Decision | Consequence |
|---|---|---|
| L6 | **One release train through M3b.** When M3b's first slice merges, main becomes the 6.0 line and cuts no 5.x release. Fixes a 5.x host needs go on a `release/5` branch until 6.0.0 ships. | M3b lands in slices on main (§4). A dev purge between slices is allowed (L5). 6.0.0 is released after the last slice. |
| L7 | **`TaskDef` grows into one Trigger model** in M4, with cursors, effect records and fencing beside it. | Docs 76, 80 and 81 use it instead of building their own; doc 81 §20.5 is settled; invariant 38 is restated at M4. |
| L8 | **The critical path is reordered** (§4): M1 ∥ M2, M3a straight after M1, M8 ∥ M7 after M6, M7 split into M7a and M7b, intake as M6.5. | Doc 73 §14 and the blast-radius inventory follow this order. |
| L9 | **Principals in M1.** `prn_` ids, with `actor` and `on_behalf_of` on the trace key. | Attribution, person-scope erasure, data roles and sharing share one id. How a person proves who they are stays with doc 78 and the collaboration plan's stage C4. |
| L10 | **An Agent's workspace is a Workspace bound to (Space, Agent), never an Environment.** | It lives as long as its Space and Agent; no environment label expires it. A non-built-in Agent's deliverables reach the Space's working tree through Review (doc 73 §5). |
| L11 | **The person-facing word for a Space** is settled in doc 75's glossary before any M3b screen is built. | Doc 75 §7 tracks it with the rest of the data vocabulary. |
| L12 | **Two guards land on 5.x now**: the home-path ratchet and the registry split (§4, "Now"). D25 is recorded in doc 73 and fixed in M3b. | The debt stops growing while M1 and M2 are built. |

Decided in the plan, each with a clear default:

- **Data catalog:** SQLite locally, Postgres in the cloud, behind one trait.
  It is called the *data catalog* in prose, because "catalog" also names the
  plugin marketplace's catalogs (doc 39).
- **Telemetry:** `tracing` + `tracing-subscriber` (JSON), OTLP optional,
  never carrying content; `eprintln!` banned in library crates.
- **One trigger model:** `TaskDef`, grown into Triggers at M4;
  `AgentSchedule` was deleted in M0.
- **Crypto** (review R1, review 2 R50–R52, R55):
  - AEAD from `ring` (already a workspace dependency).
  - Per-object random keys, and object ids that are a keyed hash within the
    tenant.
  - Key *grants* per referencing scope.
  - Records encrypted per entry, compressed *before* they are encrypted.
  - The hash chain covers each frame as stored, so it verifies without keys
    and after a shred.
  - Keys per **conversation**, spanning session rotation, and one key per
    non-owner contributor within a conversation.
  - Key custody behind a `KeyAuthority` trait; the credential store is the
    local implementation.
- **Content is keyed to its conversation wherever it is written**, and
  every derived write records `derived_from` (review R2).
- **Refs carry a generation and a writer epoch**, so a restore or a second
  host is fenced without a remote (review 2 R48).
- **Desired state is versioned** like a Document: each save is a version,
  and current is a ref (review 2 R49).
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

All of these hold on a clean install of the data baseline, proven by the
standing tests in §6:

1. **Traceable.** From any object, record, artifact, delivery, effect or
   trigger slot, one catalog query returns its Run, Session, Turn, Agent,
   Space, actor and Cause. It works after a restart, in under 50 ms at 1 M
   catalog nodes.
2. **Declared.** Every path any process writes belongs to a declared class.
   The scenario matrix runs across *every* root (tenant, runtime, cache,
   logs, Shared, the space `.vak/`, and system temp) and fails on anything
   undeclared.
3. **Bounded.**
   - Ephemeral state is zero once executions settle.
   - Caches and telemetry stay within quota.
   - Record growth per turn is at most 20 KB mean (today 66 KB), measured
     with encryption on, and at most 6 fsyncs (today about 13).
   - Settled delivery jobs leave no file behind.
4. **Flat reads.** No read on the turn path is O(history).
5. **Governed.**
   - Labels, holds, trash and erasure work end to end.
   - Erasure follows lineage into every derived copy.
   - It leaves append-only files byte-identical apart from appended
     tombstones.
   - A guest's contributions can be erased from a shared conversation
     without destroying the owner's.
   - It yields a receipt that names what it could not reach.
   - A restored backup re-applies every erasure.
6. **Honest deletion.** Nothing a person trashed or erased appears in any
   list, search, recall, prompt, export or digest.
7. **Portable.** A session pushed to a file-backed remote, with its lease
   released, continues on a second data home from the same HEAD, with
   identical `derive_messages()` output.
8. **Fenced.** A restore onto another machine bumps the writer epoch. The
   old writer can no longer commit a record, claim a slot, poll a channel
   or dispatch an effect.
9. **Honest effects.** Every external effect is an effect record. An
   ambiguous outcome stays `unknown` until it is reconciled, and nothing is
   replayed blindly after a restart or restore.
10. **Observable.**
    - Every log line is JSON with TraceKey fields and **no content**.
    - One run is one trace across the loop, tools, worker, delivery and bus.
11. **Administrable.** Every screen in doc 74 §6 is live on measured data,
    and the browser acceptance scenarios in doc 74 §9 pass.

Non-goals: the hosted cloud service itself, BYOK, an eDiscovery UI, region
routing, full product RBAC, identity proofing for invited people (doc 78,
collaboration stage C4), and doc 79's strong durability mode (M2 keeps it
possible; it does not build it).

## 3. Target architecture

```
                 ┌──────────── surfaces: server · desktop · CLI · term · channels ────────────┐
                 │ Conversations · Runs · Library · Triggers · Data · Governance · Diagnostics │
                 └─────────────────────────────────▲──────────────────────────────────────────┘
                                                   │ search · lineage · open (ACL first)
 vak-lifecycle ── reconciler: observe → plan → guard → stage → commit → record
        │  policies (labels, holds, quotas), erasure requests, receipts
        ▼
 vak-catalog (Derived: the data catalog) ── nodes · edges(lineage) · text(FTS) · lifecycle state
        ▲ idempotent ingest (chain, seq)
 vak-session · vak-commit · vak-delivery · vak-sandbox · vak-core  ── write through ──▶ vak-storage
   runs · triggers (claim by CAS + epoch) · cursors · effects (idempotency key, receipt)
 vak-storage (no vak deps; fuzzed):
    objects   keyed-hash ids, compress then seal, per-object key, key grants per scope
    records   append-only chains of segments; per-entry AEAD frames; chain over frames; verified seal
    refs      CAS pointers with generation + writer epoch (heads, current versions, claims, cursors, leases)
    keys      KeyAuthority (credential store locally) → tenant KEK → conversation / principal / space / artifact keys
    remote    trait (FileRemote first; S3 + Postgres later); commit generation hook
 tracing  spans keyed by TraceKey, content-free → JSON logs (+ OTLP)
```

## 4. Milestones

Each milestone ships whole: code, tests, docs, AGENTS.md, and the screens
listed for it. Sizes are relative: S ≈ days, M ≈ 1–2 weeks, L ≈ several
weeks, XL ≈ a month or more for one engineer. Arrows are strict
dependencies; milestones on separate arrows may run in parallel.

```
Now: ratchet + registry split (5.x)
M1 ids, trace key, principals ──┬─▶ M3a scope API ──┐
M2 storage substrate ───────────┴───────────────────┴─▶ M3b data baseline (6.0, slices 1–6)
M5 telemetry (after M1, in parallel)                       │
                                                           ▼
                                  M4 runs, triggers, effects, fencing
                                                           │
                                                           ▼
                                                M6 catalog ─┬─▶ M6.5 intake (doc 76)
                                                            ├─▶ M8 artifacts, sharing ──┐
                                                            └─▶ M7a ─▶ M7b ─────────────┴─▶ M9 remote
```

- The catalog (M6) comes *before* lifecycle (M7a/M7b), because erasure and
  holds need lineage and scope queries (review R2).
- M8 does not need lifecycle, and lifecycle needs the artifacts' lineage,
  so M8 runs beside M7a and M7b and erasure covers artifacts from its first
  commit (review 2 R58).
- M2 has no vak dependency, so it runs beside M1. M3a needs M1's
  `TestScope` and nothing from M2.

### M0 — Fix what is broken now (M, on 4.x) — done 2026-09-25, shipped in 5.0.0

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
  everywhere)". Real erasure lands in M7a.
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
  a "Pending" section on how to pick this plan up. Keep that section
  current as milestones land.

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

### Now — two guards on 5.x (S) — done 2026-10-01

The debt grew while M1 waited: `sessions_home()` calls went from 200 to
213 and `shared_data_home()` from 69 to 84 in six days, and new per-Agent
stores were declared by nothing more specific than `agents/` (review 2
R41, R42). Two guards stop that before M3a:

- **Home-path ratchet.** `crates/vak-core/tests/home_path_ratchet.rs`
  counts raw `sessions_home()`, `shared_data_home()`, `.vak` literal and
  `hash_cwd(` uses by the blast-radius §0 method, against ceilings in
  `home_path_ratchet.txt` (213, 84, 67 and 13 at landing). It fails on an
  increase, naming the largest files, and on a decrease until the ceiling
  is lowered in the same change. M3a deletes both when the counts reach
  zero.
  M3a slice 1 (2026-10-02) added `vak_config::scope` (`AgentScope`,
  `SharedScope`, `WorkspaceScope`, `Core::scope()`/`shared_scope()`/
  `workspace_scope()`) and moved vak-core, vak-config, vak-session,
  vak-commit, vak-store, vak-flow and vak-agent onto it; the ceilings are
  now 166, 76, 41 and 6. vak-server, vak, vak-ops, vak-desktop, vak-tray,
  vak-terminal and the `.vak` guard strings in vak-tools and vak-sandbox
  are later slices.
- **Registry split.** The single `agents` entry in
  `crates/vak-core/src/state.rs` is now eighteen `agents/{agent}/…` entries,
  each with its real kind; a new `Document` kind covers the stores the
  runtime rewrites (memory, entities, skill proposals, presentation packs,
  Office rooms). Backup, `--purge` and the upgrade gate expand the
  `{agent}` segment (`StateEntry::expand`); the gate finds a file wherever
  it is declared now, so the split is no violation; a purge removes each
  emptied Agent home. The enforcement test drives real writes into an Agent
  home and fails on any subpath with no entry of its own.

**Exit tests**
- `home_path_uses_do_not_grow`, `agent_home_subpaths_are_declared`.

### M1 — Identity, trace key, principals, provenance (M) — done 2026-10-02

Landed (main, 2026-10-01): slice 1 (typed ids, `TraceKey` and `Cause`,
additive `space`/`run`/`cause` on `SessionHeader`, `TestScope`) and slice 2
(a `Traced` trait with additive `trace`/`actor` on 12 side-ledger row types,
`derived_from` on memory notes, entities and skill proposals, checkpoint
labels carrying the turn id), slice 3 (`ToolContext.trace`, the broker
protocol bump, `SandboxEventSink` and sandbox records, the bus envelope and
real `prev_hash`). Slice 4a makes the key real where work is admitted:
`Core::mint_trace` (`vak-core/src/admission.rs`) mints a `RunId` and builds
the `TraceKey` per admitted turn with the cause its surface implies
(`User`, `Channel`, `Schedule`, manual `Trigger`, `Heartbeat`, `Revision`,
`Delegation` for `task` children); `SessionHeader.run` and `.cause` are set
at creation (`space` stays `None` until a Space has an id); principals are
the owner (persisted additively in the owner record), the channel sender,
the Agent and the system; and the key reaches the agent loop, each tool
call's context, the broker worker, `SandboxEventSink`, the cost, activity
and sandbox rows, `execute_script` and the hub's bus emit. The four sandbox
record types are `Traced`.
Slice 4b finished M1: all 16 `Traced` row types are in the actor test with
sample rows, and the enumeration test counts every `impl_traced!` so a new
row type cannot be left out; the write sites that have a run in scope carry
its key (misread and routing evidence, commitment events through
`CommitmentLedger::with_trace`, inbox entries, delivery jobs, packets,
outbox records and the `deliveries.jsonl` line through
`Core::admitted_trace`); incident rows name the system principal and Office
room revisions and candidates name their actor; the clock-derived and
truncated ids are full UUIDv7 typed ids (the child worker's session id keeps
its sequence number; the gateway approval short code is the random tail of the
id, where the truncated head was a timestamp); `docs/reference/records.md` is
generated by `records_reference_is_current`; and `/finops` reports spend per
Agent and per run (`finops::rollup_by_agent`, `rollup_by_run`).
Left with no key in scope, so `None` by design: security events (written by
HTTP and auth paths, no run), budget alerts (raised by a task sweep after its
run), incident rows (probes, no run), voice dispatch evidence and cost rows,
and inbox entries written by schedulers, gate denials and commitment upkeep.
Coworking messages do not exist as a durable record yet; they take the key
when they do. `derived_from` landed in slice 2. The root work account is E1
of the reliable-work plan, which has not started: when it does it is keyed
by `RunId` and carries `root_work_account_keyed_by_run_id`; that test is
E1's, not an M1 gate, because there is no account to key yet.

- **Typed ids** in `vak-session/src/ids.rs`; **`TraceKey` and `Cause`** in
  `trace.rs` (doc 73 §4). `TraceKey::child` is the only way to make a span.
  `RunId` lands first, so the reliable-work plan's root work account (E1)
  is keyed by it and M4 adopts that account instead of replacing it (review
  2 R46).
- **Principals** (L9): `PrincipalId` (`prn_`) for the owner (doc 78), an
  invitee (doc 69), a channel sender (the gateway's resolved sender
  identity), an Agent and the system. `TraceKey` gains `actor` and
  `on_behalf_of`. Candidate records, Office room revisions, comments and
  coworking messages carry the actor, so a version's author is recorded
  rather than inferred (doc 82 §0).
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
- **The clock-derived or truncated ids become typed ids.**
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
- `every_ledger_row_type_is_traced`, `every_ledger_row_type_names_its_actor`,
  `broker_protocol_carries_trace`, `bus_envelope_trace_is_run_trace`,
  `bus_prev_hash_is_hash`.
- `session_header_names_cause_for_each_surface`,
  `derived_writes_record_provenance`, `records_reference_is_current`,
  `root_work_account_keyed_by_run_id`.

### M2 — Storage substrate `vak-storage` (L, in parallel with M1) — in progress

Landed (main, 2026-10-01): slices 1 and 2 in `crates/vak-storage` (key
authority trait with an in-memory implementation, writer-epoch refs in
memory and SQLite, keyed sealed objects, hash-chained records, the verified
seal with crash recovery, persistent scope keys with shred and hold, the
Documents helper, the `Store` trait with commit generation and a
pre-acknowledgement hook, the `Remote` trait declared, a fuzz skeleton).
Remaining: the credential-store `KeyAuthority` (outside this crate, because
it depends on `vak-config`), a byte-level torn-write test of the seal, a
fuzz run and corpus, a flock single-writer lock, streaming for large blobs,
and the workspace pins for `async-nats` and `webauthn-rs`.

A library only, with no behaviour change elsewhere. It absorbs doc 79's
requirements now, because they are cheap before the substrate exists and
expensive after (review 2 R48–R53).

- **`objects`:**
  - The id is HMAC-SHA256 under a tenant id key, so dedupe works inside a
    tenant but can't confirm a file across tenants.
  - zstd compression *then* a per-object random key sealed with
    `ring::aead`.
  - Key grants per referencing scope; streaming for large blobs; sharding;
    atomic writes.
- **`records`:**
  - Chains of segments; each entry is a compressed, then sealed, AEAD
    frame under its conversation's key, or under a contributor's key
    (doc 73 §7.3); plaintext frames when tenant policy is off.
  - The hash chain covers each frame **as stored**, carrying on from
    today's per-entry `prev_hash`, so integrity verifies without keys and
    after a shred.
  - **Verified seal:** copy, check count, hashes and chain, atomic swap,
    then a seal entry (review R17). A sealed segment is compressed only
    when its frames are plaintext.
  - Single writer by flock locally, plus the writer epoch below.
  - Torn-tail detection.
- **`refs`:** CAS in SQLite (rusqlite pinned in the workspace manifest), and
  an in-memory backend. Every ref carries a monotonic **generation** and the
  **writer epoch** of its holder. A write under a stale epoch is refused.
  Restoring a store bumps the epoch, which fences the old writer without
  any remote (doc 79 §8).
- **`keys`:**
  - A `KeyAuthority` trait: wrap, unwrap, rotate, revoke, health. The
    credential store (`vak_config::credentials`) is the local
    implementation; KMS and attested release (doc 79 §5) are later ones.
  - Tenant KEK under the authority; conversation, contributor, space and
    artifact keys wrapped by the KEK.
  - Tenant KEK through `vak_config::credentials`.
  - Account-scoped key/grant composition for provider-originated content, so
    deleting one connected account does not require deleting an entire
    conversation; prove append-only model-visible record behavior under that
    composition before admitting such content.
  - Conversation, space and artifact keys wrapped by the KEK.
  - `shred(scope)` and `hold(scope)` hooks.
  - An unhealthy authority fails closed: callers refuse new work rather
    than write unprotected data.
- **`Store` trait** with `LocalStore` and `MemoryStore`; the `Remote` trait
  is declared. `Store` exposes a commit generation and a pre-acknowledgement
  hook, so doc 79's strong durability mode can be added later.
- **Documents:** the versioned-named-content helper (version as object,
  current as ref) that Documents and Desired state both use.
- **New pinned dependency:** `zstd`. `async-nats` and `webauthn-rs` move
  into the workspace manifest with exact pins.

**Exit tests**
- Property tests: seal round-trip, tamper detection, a crash at every step
  leaves a readable prefix.
- `chain_verifies_without_keys`, `chain_verifies_after_shred`,
  `compressed_before_encrypted`.
- `stale_epoch_cannot_commit`, `restore_bumps_epoch`,
  `key_authority_unavailable_fails_closed`.
- Shred: file bytes before the tombstone are unchanged and content is
  unreadable.
- GC never collects an object with a live grant; keyed ids dedupe within a
  tenant and differ across tenants.
- Fuzz targets for the frame and segment readers.

### M3a — Scope API on the current layout (L, no behaviour change; after M1)

- `vak_config::paths` gains `Scope`/`StorageHandle` accessors that resolve
  to *today's* paths.
- Every production home-path use is replaced with typed accessors: at
  `438cfcd5`, 213 `sessions_home()` and 84 `shared_data_home()` calls among
  626 home-identifier uses in 69 files (blast-radius §0).
- The D25 call sites (server-side stores resolved from the server's own
  Core, doc 73 §2) get an accessor that names the session's Agent, with
  today's resolution kept, so M3b changes them in one place.
- `Core::sessions_home`, `shared_data_home` and `set_sessions_home` are
  removed at the end, with the home-path ratchet.
- Tests: every layout reference in test code moves to `TestScope`. The
  unchanged suite is the oracle.

**Exit:** the full `cargo test --workspace` passes with zero behaviour
diff; the layout scan (blast-radius §0) finds no raw home-path use outside
`vak-config`.

### M3b — The data baseline, 6.0 (XL, in six slices)

**Release train (L6).** When slice 1 merges, main is the 6.0 line and cuts
no 5.x release; 5.x fixes go on `release/5`. Each slice ships its own tests
and may purge dev data homes. 6.0.0 is released after slice 6.

**Slice 1 — baseline, purge and runtime root**
- Version 6.0.0-dev; invariant 29 becomes "6.0.0 is the supported
  baseline". Pre-baseline state is refused by the one message.
- Purge: Vak-owned roots (data, cache, logs, runtime) are removed
  wholesale; Shared uses declared entries only; Vak runtime leftovers in
  known spaces are listed and offered (review R7).
- The tenant tree (doc 73 §6) and the runtime root exist; the state
  registry becomes classes × roots, driven by the §6 matrix. The upgrade
  gate uses class snapshots.

**Slice 2 — sessions on segments, slim ledgers**
- Session ledgers become record segments (encrypted per entry when tenant
  policy is on), keyed by TraceKey, so every Agent's records live in that
  Agent's scope (fixes D25 for sessions).
- The capability binding and large tool results move to objects; sandbox
  streams are chunked to objects.

**Slice 3 — side ledgers and Documents (review R9)**
- FinOps, alerts, commitments, inbox, routing, misread, security and
  operations become record chains without compaction rewrites.
- Memory, entities, skills, prompt layers, presentation packs and Office
  rooms become **Document** class. Desired state is versioned the same way.
- Sandbox records, candidates, execution streams and coworking grants move
  into their session's Agent scope (D25).
- `auth/` becomes tenant Desired (public keys and recovery-code digests,
  always backed up).

**Slice 4 — runtime out of the project tree (L4, L10)**
- Executions and environments live under the tenant.
- Agent workspaces move to `workspaces/<spc>/<agt>/` as Workspace class,
  bound to (Space, Agent), never an Environment.
- Sandbox profiles grant only the execution directory, the Agent workspace
  and the space root.
- `App.tsx:903` loses its `.vak/scratch` check.

**Slice 5 — space identity (review R12)**
Each item is keyed by space id in this slice:
- credential scopes (`scope_key_for`), trust markers
- CorePool identity, allowlist workspace fields, the
  `gateway/default-workspace` file
- `TaskDef.cwd` and the scheduler filter
- `/workspaces`, the desktop trust gate
- `[server] workspace_roots`

**Slice 6 — docs, site, scripts, services**
- Every item in blast-radius M3b slice 6. Then release 6.0.0.

**Screens**
- Admin: Spaces (A13), with the word settled in doc 75 (L11).
- Admin: `SessionForensics.tsx` groups by space instead of project hash.

**Exit tests**
- `bytes_per_turn_budget`, `fsyncs_per_turn_budget`,
  `no_undeclared_paths_any_root`, `pre_baseline_home_refused_with_one_message`.
- `purge_removes_owned_roots_wholesale`,
  `sandbox_writes_only_execution_dir_and_space`,
  `derive_messages_identical_across_seal`, `secret_scopes_keyed_by_id`.
- `agent_records_live_in_their_agent_scope`,
  `agent_workspace_is_not_an_environment`.
- Full suite, plus a live dev run in `/tmp` per `docs/development.md`.

### M4 — Runs, triggers, effects and fencing (XL)

The layer docs 76, 80 and 81, the fleet (doc 79) and the collaboration plan
all need (review 2 R45). Each primitive ships with one consumer here; the
proposals bring the rest.

- **A `runs/` record chain.** A `RunRecord` is written for every cause:
  user turn, channel, trigger slot or event, delegation, revision,
  heartbeat, flow, best-of-N, and export.
- **One Trigger model (L7).** `TaskDef` grows into `Trigger` (`trg_`); the
  struct, `/tasks` routes and CLI are renamed in the same change
  (invariant 30). Kinds: `schedule` (cron, interval, once), `event`,
  `webhook`, `on_open`, `watch`, `source_poll`, `manual`. This milestone
  builds `schedule` and `manual`; the others arrive with their consumers
  (docs 76, 80, 81) on the same record shape.
- **At most one start per slot or event** (review R4):
  - Claim `(trigger, slot | event id)` by CAS, under the claimant's writer
    epoch, before any side effect.
  - A lease-expired run is recorded `abandoned`.
  - `on_crash = skip | retry_once`.
  - One `due(now)` serves the tick, catch-up and run-now; the in-memory
    `next_fire` map goes.
- **Cursors.** A durable position in an external stream is a Ref with CAS
  and an epoch; an expired cursor resyncs boundedly and records the gap.
- **Effect records.** An `effects/` chain records every external effect:
  `prepared → dispatched → accepted | confirmed | failed | unknown →
  reconciled`, with an idempotency key and the provider's receipt. The
  outbox and deliveries become its first kind, and a settled job is
  sealed. Nothing marked `dispatched` or `unknown` is replayed after a
  restart or restore; it is reconciled from receipts.
- **Fencing.** Routine leases, channel pollers and session writers hold an
  epoch from M2's refs.
- **`Trigger`:** the `last_*` fields go; "last run" is a query.
- **`CopyEnvironment`:** the first real `EnvironmentBackend`
  (`vak-sandbox`). It copies a non-git space with ignore rules and size caps
  into `environments/<run>/`, runs there, and exports the changes as a
  candidate for Review. This replaces M0's refusal.
- **Connections** (an account and its grants) are defined in doc 73 §5 and
  built with their first consumer (doc 80 stage 2 or doc 81 P3), not here.
- **Screens:**
  - Admin: Runs (A4), Run detail (A4b), Triggers (A5, with definitions
    moved from Configure).
  - `#/operations/work/runs/<session_id>` becomes `#/runs/<run_id>`
    (review R13).
  - Client: Runs panel (C6) replaces the `TasksModal` last-run fields.

**Exit tests**
- `schedule_slot_at_most_once_under_restart` (property test over crash
  points), `two_processes_do_not_double_start`, `abandoned_run_is_recorded`.
- `every_cause_writes_run`, `skipped_slot_is_a_record`,
  `non_git_space_routine_runs_in_copy_environment`.
- `effect_unknown_until_reconciled`, `effect_not_replayed_after_restart`,
  `restore_fences_old_writer`, `cursor_resync_records_gap`.

### M5 — Telemetry (M, after M1, in parallel)

- Pinned `tracing`/`tracing-subscriber` (json, env-filter); optional
  `tracing-opentelemetry` + `opentelemetry-otlp` via
  `[telemetry] otlp_endpoint`.
- The span tree runs `run › turn › step › (dispatch | tool_call ›
  execution) › delivery`. The worker continues the parent span.
- **Content-free** (review R3): fields are ids, kinds, sizes, digests,
  durations and outcomes only, checked against an allowlist of field
  names. Filenames, URLs, paths and titles are content (doc 79 §6).
- The library/server `eprintln!` become leveled events; the CLI keeps
  user output; `clippy.toml` sets `disallowed-macros` for library crates.
- Service logs become `vak-<service>.jsonl` with rotation (`vak-ops`).
- Bus payloads carry references only; streams get `max_age` (review R22).
- **Screens:** System › Diagnostics › Traces & logs (A17); span waterfall
  in Run detail.

**Exit tests**
- `library_crates_have_no_eprintln`, `one_run_one_trace_id`,
  `log_lines_are_json_with_trace_fields`.
- `telemetry_carries_no_content`: planted canaries (a secret, a filename, a
  URL, a prompt) in every error and tool path never appear in a log line,
  span or metric (doc 79 §9 scenario 4).

### M6 — Data catalog, search, lineage (L)

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

### M6.5 — Intake (L, after M6; doc 76)

- Sources are Desired, polled through M4's `source_poll` triggers with
  cursors; push intake (`save_to_inbox`) is one push connector.
- Items are objects plus catalog nodes with a TraceKey, `derived_from` and a
  disposition; detection labels and never drops.
- The agent reaches intake through the one retrieval tool doc 76 D1 settles.
- The Python pipeline, DuckDB store, `feed_mcp.py` and its search table are
  deleted in the same change.
- Doc 76's open decisions D1 and D2 are settled before it starts.

**Exit tests**
- `intake_item_has_trace_and_provenance`,
  `quarantined_item_absent_from_agent_retrieval`,
  `feed_pipeline_is_gone`.

### M7a — Lifecycle: honest deletion (L)

- **`crates/vak-lifecycle`**: the reconciler (doc 74 §5). It first ships
  **observe-only** (it computes and shows the plan and commits nothing)
  across the whole scenario corpus. Commit is then enabled one action
  class at a time, ephemeral first and records last.
- **Remove the scattered retention** (review R25):
  - checkpoints `MAX_STORED_CHECKPOINTS`, finops and alerts compaction,
    memory `cleanup_artifacts`, `/memory/cleanup`
  - the `archive.json`/`deleted.json` sidecars and their routes
  - `/workspaces/forget`
- **The default tenant label** (doc 74 §3.1) applies from here, replacing
  those limits, so drafts fade and the trash window closes. Editing labels
  and attaching them anywhere else is M7b.
- **Trash → erase for conversations** (doc 74 §2.4, §4), including a
  guest's contributions from a shared conversation (doc 74 §3.4): preview
  digests, a lineage walk that covers artifact versions (doc 74 §4), key
  destruction, derived plaintext removal (`secure_delete` plus a WAL
  checkpoint), and a signed receipt naming what it couldn't reach.
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
- **Quotas** (doc 74 §3.3).
- **Backup rework** (review R21): ciphertext plus wrapped keys, a coherent
  manifest (record heads, ref generation, object inventory, key grants,
  Desired revision, erasure watermark), and restore re-applies erasures
  and bumps the writer epoch.
- **Screens:**
  - Admin: Home › Data health (A1), Conversations lifecycle (A2/A3),
    Storage (A7), Lifecycle (A8), Integrity (A9), Agents lifecycle panel
    (A14), Backup & restore (A16).
  - Client: menu (C1), Trash (C2), "Why is this gone?" (C3), Workbench
    states (C8).
- **CLI:** `vak data status | usage | plan | gc | verify | rebuild-catalog |
  erase --scope conversation | export | backup | cat | grep`.

**Exit tests**
- `reconciler_is_idempotent`, `reconciler_observe_only_commits_nothing`,
  `settled_execution_leaves_nothing`, `gc_keeps_everything_reachable`.
- `erasure_follows_lineage`, `erasure_leaves_ledger_bytes_unchanged`,
  `guest_erasure_keeps_owner_conversation`,
  `person_erasure_spans_agents_and_chats`,
  `provider_account_erasure_spans_agents_and_conversations`,
  `provider_account_erasure_preserves_unrelated_conversation_content`,
  `provider_account_erasure_reapplies_after_restore`,
  `hold_blocks_every_destructive_transition`.
- `stale_preview_cannot_authorise`, `quota_refuses_admission_not_records`,
  `restore_reapplies_erasures`, `revoke_cuts_endpoints_within_one_tick`.
- `thirty_day_soak_stays_within_budget`.
- The browser acceptance scenarios in doc 74 §9 that name these screens.

### M7b — Lifecycle: governance (L, after M7a)

- **Labels** attach to any labelable catalog node (tenant, space, Agent,
  conversation, artifact, piece, source, connection) with the doc 74 §3.1
  resolution (review 2 R61).
- **Holds** (doc 74 §3.2).
- **Erasure scopes** person, Agent, space and tenant (doc 74 §3.4).
- **Allowlist PII retention**, with hashed sticky-deny fingerprints.
- **Data roles** (doc 74 §3.6).
- **Screens:** Retention & holds (A10), Erasure requests (A11), Keys (A12),
  Audit export (A15); client Your data (C7).
- **CLI:** `vak data labels … | hold … | erase --scope person|agent|space|tenant |
  keys …`.

**Exit tests**
- `person_erasure_spans_agents_and_chats`,
  `hold_blocks_every_destructive_transition`, `label_on_any_node_resolves`.
- The remaining browser acceptance scenarios in doc 74 §9.

### M8 — Artifacts, sharing, information architecture (L, after M6, beside M7a/M7b)

- **Artifacts and Versions.** Candidates and promotions (doc 54), Office
  drafts (doc 72), changesets, datasets, dashboards and saved cards all
  become artifact version events; a card stays in its chat unless a person
  saves it (`docs/design/82-library.md` §3). A changeset promotes into git
  as a commit on a branch. A version holds only its conversation's key
  grant until it is Saved, starred or shared (doc 74 §4).
- **Grants** with inheritance and explicit breaks; doc 69's coworking grants
  move into the same table, keyed by principal (M1).
- **Navigation reconciliation** in the admin console (doc 74 §6.1) and the
  client.
- **Screens:**
  - Admin Library (A6).
  - Client Library (C4), Share dialog (C5), version history, built to
    `docs/design/82-library.md` (its phases L4 and L5 ride this milestone).
  - `SharedConversation.tsx`, `ArtifactCanvas`, `OfficeRedline` and
    `WorkbenchPanel` move onto the Artifact API.

**Exit tests**
- `concurrent_edit_creates_sibling_versions`, `share_inherits_and_breaks`,
  `revoked_grant_hides_from_search`, `saved_version_survives_origin_erasure`.
- A browser acceptance run: create → review → promote → share → comment →
  revise → erase.

### M9 — Cloud remote (L; the protocol and a reference backend)

- **The `Remote` trait**, with `FileRemote` (tests and personal
  multi-machine) and `S3Remote` + Postgres refs behind a feature flag.
- **`vak sync`** and a sync plane following doc 31 (store-and-forward):
  - Secrets never sync.
  - Tombstones, holds and receipts sync both ways.
- **Leases** extend M2's writer epochs across remotes; handoff happens at a
  turn boundary.
- **Tenancy:** the key-escrow and key-release design for a hosted service
  is written as its own doc before any hosted backend ships (doc 79 §5).
- **Screens:** Sync (A18).

**Exit tests**
- `push_pull_roundtrip_identical_derive_messages`,
  `handoff_at_turn_boundary`, `lease_prevents_dual_writer`.
- `erasure_propagates_and_cannot_resurrect`, `sync_survives_network_loss`.

## 5. AGENTS.md and design-doc changes

| When | Change |
|---|---|
| M0 | Done: invariant 38 drops `AgentSchedule`/`AgentRunRecord`; docs 65 and 72 lose the claims; doc 64 draws the real 4.x topology; docs 73 and 74 listed under proposals with the "Pending" section; invariant 8 gains "no API writes a secret to a file". |
| Revision 3 | Done 2026-10-01: "Pending" names the data baseline by reference, the new order and the two guards; docs 76, 79, 80, 81 and 82, the collaboration plan and the reliable-work plan point at the primitives they use. |
| Now | "Until the next milestone lands": no raw home-path call beyond the ratchet; every Agent-home subpath declared. |
| M1 | **New invariant:** every durable record, span and envelope carries a `TraceKey` with its actor; derived writes record `derived_from`. |
| M2 | **Invariant 2:** entries are never rewritten; encoding changes only by a verified seal; the hash chain covers frames as stored. |
| M3b | **Invariant 29** → 6.0.0 baseline. **Invariant 35** rewritten (runtime state under the tenant; Agent workspaces bound to Space and Agent). **Invariant 37** paths. **New invariant:** every path belongs to a declared class; ledgers hold references, not bulk content; content is keyed to its conversation wherever it is written. Layout map gains `vak-storage`. Doc 73 status → "in progress". |
| M4 | **Invariant 38:** scheduled and triggered work is a Trigger, with at most one start per slot or event. **New invariant:** every external effect is an effect record, `unknown` until reconciled and never replayed blindly; writers, claims and pollers are fenced by epoch. |
| M5 | **Code rule:** no `eprintln!` in library crates; telemetry is content-free and never read to decide anything. |
| M6 | **Layout map:** `vak-store` → `vak-catalog`. **Invariant 37:** search and lineage filter ACL before ranking. |
| M6.5 | Doc 51 retired; doc 76 status → shipped. |
| M7a | **New invariant:** destruction is the reconciler's alone, is recorded, yields to holds, and follows lineage; Record data is erased only by crypto-shred; trash is hidden everywhere. **Invariant 19** references `vak data`. **Invariant 37:** `Revoked`. Docs 14, 23, 28 and 46 Part VII updated. |
| M7b | Docs 33 and 44 updated (governance, keys). |
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
- **Triggers:** a cron slot; an interval; a one-shot; a script watchdog; a
  catch-up after downtime; two triggers due in the same tick; a non-git
  space; a crash mid-run; a webhook and an event (with their consumers); a
  cursor that expired and resyncs.
- **Effects:** an outbox delivery that succeeds; a retry after failure; an
  approval forwarded to a channel; a dispatch whose outcome is unknown and
  is reconciled, never replayed.
- **Fencing:** a restore onto a second data home while the first is still
  running.
- **Delegation:** a `task` child with cards; best-of-N; a flow; a dynamic
  plan.
- **Work products:** an Office edit through review and promotion; a code
  changeset into a git space; a candidate revision; an artifact with
  versions from two conversations.
- **People:** a guest's messages and comments in the owner's conversation.
- **Lifecycle:**
  - trash → restore → erase;
  - erasure of one conversation that made one version of a Saved artifact;
  - a guest's erasure from a shared conversation;
  - person erasure across two Agents;
  - a hold during erasure and during GC;
  - label shortening;
  - quota pressure;
  - a restore that re-applies erasures;
  - a crash at every storage write step.

Each run asserts:
- no undeclared paths in any root;
- every record traced, with its actor;
- one trace per run;
- no content in telemetry;
- bytes and fsyncs per turn within budget;
- zero ephemeral residue after settle;
- lineage resolvable from every produced node;
- trashed or erased items absent from every read path;
- no effect dispatched twice.

## 7. Risks and mitigations

| Risk | Mitigation |
|---|---|
| M3 size (626 production home-identifier uses, 529 test references at `438cfcd5`) | M3a (refactor, oracle = the existing suite), then M3b in six slices behind the finished API; the ratchet stops growth until M3a starts. |
| Releases during M3b | One train (L6): main is the 6.0 line from slice 1; 5.x fixes on `release/5`. |
| M4 grows with every proposal | Each primitive ships with one consumer (schedule, manual, the outbox); the other trigger and effect kinds arrive with their proposals on the same record shape. |
| Principal ids before identity proofing | Records carry ids from M1; how a person proves who they are stays with doc 78 and collaboration C4, and nothing grants authority from an id alone. |
| A reconciler bug destroys data | Staged quarantine before commit. It ships observe-only first, then commits one action class at a time. Every destructive transition is recorded. Holds are checked at commit time, not only at plan time. |
| The crypto design is subtle | The substrate has no vak dependencies and gets property tests and fuzzing (M2). Primitives come from `ring` only. There is no custom construction beyond HMAC ids and AEAD frames. The key hierarchy is documented in doc 73 §7.3. |
| Moving scratch out of the project tree breaks tools | The broker already sets `TMPDIR`/caches. A sandbox test plus the acceptance dashboard scenario gate it. |
| Encryption hides data from people | `vak data cat/grep/export --plain`, and an honest headless note. |
| Catalog drift | It is Derived: a staleness digest, `rebuild()`, and a rebuild-equals-incremental test in CI. |
| Erasure misses a copy | The content-keyed rule plus `derived_from` on every derived write; `erasure_follows_lineage` runs over the full corpus; receipts name what couldn't be reached. |
| Telemetry volume and leakage | Content-free by allowlist and canaries; leveled; sampled; rotated; quota-managed. |
| Cloud leaks into local complexity | M9 is last; before it `Remote` is only a trait plus `FileRemote`. Epochs and the key authority are local features that M9 reuses. |

## 8. Until M3b on this dev machine

There is no migration tool to build (L3, L5). The dead pre-`31c1bb9c`
checkpoints (about 1.6 GB) and the `agents/vak/agents/` nesting may be
removed by hand. At M3b's first slice every dev data home is purged, and
the new purge removes the owned roots wholesale. Data on a 5.x host,
including the maintainer's EC2 host, does not cross the baseline: download
what you want to keep as ordinary files (transcripts, documents) before
upgrading it, because no 5.x backup restores into 6.0.
