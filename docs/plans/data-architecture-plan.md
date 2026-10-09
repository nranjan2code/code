# Plan — data architecture, lifecycle, tracing and cloud

Status: **plan, revision 4 (2026-10-03). M0 is done (2026-09-25, shipped in
5.0.0), and so are the two 5.x guards (§4, "Now", 2026-10-01) and M1
(2026-10-02). M2 is done (2026-10-06); M5 is done (2026-10-06); M6 is done (2026-10-06: M6.1 to M6.4), and so is M6.5 (2026-10-06); M3a is done (2026-10-03), M3b is done (2026-10-05) and M4 is done (M4.1 to M4.7 on 2026-10-05, M4.8 on 2026-10-06). M8 is done (2026-10-08: M8.1 to M8.4c-f). M7a's design was agreed on 2026-10-08 (§M7a "M7a design", steps M7a-a to M7a-i); M7a-a is done (2026-10-08), and M7a-c (2026-10-08: the reconciler observes and plans), M7a-i (2026-10-08: integrity, the soak, the acceptance run, and retention on by default), so M7a is done; M7a-h (2026-10-08: a restore applies every recorded erasure again and moves the writer epoch), M7a-g (2026-10-08: an Agent can be revoked, which cuts its bots, accounts and secrets at once), M7a-f (2026-10-08: the owner erases what a guest wrote or what a disconnected account returned), M7a-e (2026-10-08: the trash and the archive are one state ref per conversation; a conversation is erased from the trash by destroying its keys, with a signed receipt; old drafts go to the trash and are erased; the Trash, menu and admin screens; lifecycle and erasure records in the catalog), M7a-d (2026-10-08: a committing pass for executions, checkpoints, environments, rotated logs, expired chain segments and Document history, collection of what nothing names, and an install quota; retention acts only when `[lifecycle] mode = "commit"`), and so is M7a-b (2026-10-08: content in every shared chain is an object of its owner's scope; a guest's contributions are under their own key; what a connected account returned is under the account's key). M7a is done (2026-10-08) and M7b is done (2026-10-09, trimmed to one owner; acceptance in `docs/audits/acceptance-m7b-governance-2026-10-09.md`); and M9 is done (2026-10-09, a folder remote with two machines taking turns; acceptance in `docs/audits/acceptance-m9-remote-2026-10-09.md`), which completes the plan as cut for one owner. Each step waits for the maintainer (see AGENTS.md,
"Pending").**

- Design: `docs/design/73-data-architecture-and-lifecycle.md` (the model)
  and `docs/design/74-lifecycle-and-data-administration.md` (lifecycles,
  policies, screens, API).
- Revision 2 applied every fix in `docs/plans/data-architecture-review.md`.
- Revision 3 applies every fix and decision in
  `docs/plans/data-architecture-review-2.md`.
- Revision 4 records the maintainer's direction to ship 6.0.0 independently
  of M3b and moves the data baseline to 7.0.0; M2 remains in progress and M3a
  remains next.
- What each milestone touches: `docs/plans/data-architecture-blast-radius.md`.
- Baseline measurements: `docs/architecture/write-paths-and-growth.html`
  (v3.5.1) and doc 73 §2.

## 1. Decisions

Locked 2026-09-25 with the maintainer; L3 and L6 updated by revision 4:

| # | Decision | Consequence |
|---|---|---|
| L1 | **Local-first, cloud as a remote.** A desktop or server works offline on its own store; a hosted cloud is a push/pull remote with session handoff. | One storage trait with two backends (local disk + SQLite; object store + Postgres). No path is ever an identity. |
| L2 | **Compliance bar for the first customer release:** retention labels, erasure (crypto-shred), legal hold, exportable audit trail. Region pinning, BYOK and eDiscovery come with the cloud phase. | Keys, holds, lineage and receipts are in the local design from day one. M7a and M7b are both required before that release. |
| L3 | **Cut a new data baseline at 7.0.0.** 5.0.0 was spent on M0's removals; 6.0.0 ships independently of M3b. This row is the one place the baseline version is written: every other document says "the data baseline" and points here. | The baseline refuses every earlier data home with the single invariant-29 message. No migrator, no reader for old shapes, no dual layout. Dev data homes, and the maintainer's 6.x hosts, are purged at the cutover. |
| L4 | **Runtime state leaves the project tree.** A space's `.vak/` holds only committable intent. | Invariant 35 is rewritten; sandbox profiles grant the execution directory explicitly. |
| L5 | **Zero users, zero compatibility.** | Each milestone *replaces* what it supersedes in the same change (invariant 30). An early fix is made only when it is the target behaviour. |

Locked 2026-10-01 with the maintainer (review 2, §4):

| # | Decision | Consequence |
|---|---|---|
| L6 | **One release train through M3b.** When M3b's first slice merges, main becomes the 7.0 line and cuts no 6.x release. With no users there is no 6.x maintenance line: `release/6` was deleted on 2026-10-04. | M3b lands in slices on main (§4). A dev purge between slices is allowed (L5). 7.0.0 is released after the last slice. |
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
M2 storage substrate ───────────┴───────────────────┴─▶ M3b data baseline (7.0, slices 1–6)
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

### M2 — Storage substrate `vak-storage` (L, in parallel with M1) — done 2026-10-06

Landed (main, 2026-10-01): slices 1 and 2 in `crates/vak-storage` (key
authority trait with an in-memory implementation, writer-epoch refs in
memory and SQLite, keyed sealed objects, hash-chained records, the verified
seal with crash recovery, persistent scope keys with shred and hold, the
Documents helper, the `Store` trait with commit generation and a
pre-acknowledgement hook, the `Remote` trait declared, a fuzz skeleton).
The rest landed by 2026-10-06: the credential-store `KeyAuthority`
(`VaultKeyAuthority` over `vak_config::credentials`, in `vak-session`,
since M3b); the single-writer flock in the substrate (`SegmentSet::lock`
and `try_lock` return a `WriterLock` that `writer`, `seal` and `recover`
require, replacing the two locks `vak-session` kept); a byte-level
torn-write test (`a_write_torn_at_any_byte_leaves_the_entries_before_it`);
a committed seed corpus in `fuzz/corpus/` with a stable gate of 20,000
deterministic mutations per reader (`tests/fuzz_corpus.rs`; a libFuzzer
run needs nightly and is not part of the suite); streaming blobs
(`vak_storage::blobs`: chunks of at most 8 MiB as ordinary objects plus a
manifest object); and exact workspace pins for `async-nats`,
`webauthn-rs` and `webauthn-authenticator-rs`.

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

### M3a — Scope API on the current layout (L, no behaviour change; after M1) — done 2026-10-03

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

**Done (2026-10-03).** Every production helper that took a raw home now
takes a typed scope (security events, inbox, trash, checkpoints, learning,
digest, operations); the D25 stores resolve through the `AgentScope` D25
accessors with one `session_agent(state)` naming the Agent; `Core` has no
public raw home getter (`set_shared_scope(SharedScope)` replaces
`set_sessions_home`, and `scope()`/`shared_scope()` are the only way in);
project-layer paths are `WorkspaceScope` accessors or `vak_config::scope`
constants; `hash_cwd` is gone for `workspace_key`. The layout scan finds
zero `sessions_home()`, `shared_data_home()`, `.vak` literals and
`hash_cwd(` outside `vak-config`, the full suite passes, and the ratchet is
deleted. Two deviations: no separate `TestScope` type, because tests use
`AgentScope`/`SharedScope` directly and a second wrapper would be two ways
to say one thing (invariant 30); and stores that still take a root path
(`CommitmentLedger::new`, `TaskStore`, `FinOpsLedger`, `RateLimiter`, the
agent-network socket, memory and learning tool structs holding a
`sessions_home: PathBuf`) are reached through `scope().into_root()` and are
M3b's touchpoints to type.

### M3b — The data baseline, 7.0 (XL, in six slices)

**Release train (L6).** When slice 1 merges, main is the 7.0 line and cuts
no 6.x release, and there is no 6.x maintenance line. Each slice ships its own tests
and may purge dev data homes. 7.0.0 is released after slice 6.

**Slice 1 — baseline, purge and runtime root** — done 2026-10-03. Version 7.0.0-dev;
`BASELINE` is 7.0.0 and a data home with sessions or Agent state but no
`tenants/` tree is refused (`pre_baseline_home_refused_with_one_message`);
`paths::runtime_dir`/`tenant_home_at`, the agent-network socket in the
runtime root; purge removes owned roots wholesale
(`purge_removes_owned_roots_wholesale`); the registry is classes × roots
(doc 73 §5) with snapshots carrying the class; doctor checks the baseline.
(`release/6`, cut at the last 6.x commit, was deleted on 2026-10-04: no
users, so no 6.x line.) Not done in this slice: listing
Vak runtime leftovers in known spaces (review R7), which slice 4 makes moot
by moving runtime state out of the project tree.
- Version 7.0.0-dev; invariant 29 becomes "7.0.0 is the supported
  baseline". Pre-baseline state is refused by the one message.
- Purge: Vak-owned roots (data, cache, logs, runtime) are removed
  wholesale; Shared uses declared entries only; Vak runtime leftovers in
  known spaces are listed and offered (review R7).
- The tenant tree (doc 73 §6) and the runtime root exist; the state
  registry becomes classes × roots, driven by the §6 matrix. The upgrade
  gate uses class snapshots.

**Slice 2 — sessions on segments, slim ledgers** — done 2026-10-03. A
session ledger is a directory of `vak-storage` record segments
(`seg-NNNNNNNN.log`/`.sealed`, `seals.log`, `LOCK`); every reader goes
through `SessionLog` (`read_header`, `scan`, `text`, `segment_files`) and
the FTS index locates frames by (segment, offset). The tenant object store
(`tenants/<t>/objects`) has a durable key authority: the KEKs and object id
key are in the credential store (`VaultKeyAuthority`), revocations in
`tenants/<t>/keys`. Objects are granted per conversation. A windowed tool
result's whole body, the capability binding's prompt, schemas and index,
execution stdout/stderr (one object per 64 KiB per stream) and checkpoint
file contents are objects; writing one without a store fails closed. The
ledger syncs at commit points only (header, a person's message, tool
results, effects, the turn card, close). Measured on a scripted read-then-
answer turn: 6.1 KB and 3 record syncs per turn (`bytes_per_turn_budget`,
`fsyncs_per_turn_budget`); `derive_messages_identical_across_seal`.
Deviations: frames were written unencrypted until M7a-a sealed every one (2026-10-08), so the budget is measured without it (a sealed frame adds
a nonce and tag, well inside the margin); ledgers stay under the Agent home,
named by session id, and move with the D25 work in slice 3; checkpoints
keep `MAX_STORED_CHECKPOINTS` until retention policy lands in M7a.
- Session ledgers become record segments (encrypted per entry when tenant
  policy is on), keyed by TraceKey, so every Agent's records live in that
  Agent's scope (fixes D25 for sessions).
- The capability binding and large tool results move to objects; sandbox
  streams are chunked to objects.

**Slice 3 — side ledgers and Documents (review R9)** — done 2026-10-04. side ledgers are `vak_session::chain::RecordChain`s (costs,
activity, budget alerts, routing and intent evidence, security events, the
inbox, commitments, operations incidents and actions, the deliveries log
and the outbox, whose settled jobs leave no file), with no compaction
rewrites; telemetry rows append without their own sync. D25: every
server-side record about a session lives in that session's Agent home
(`session_agent_scope`; `agent_records_live_in_their_agent_scope`).
`auth/` and feeds are under `tenants/<t>/`; `no_undeclared_paths_any_root`
drives a real turn across the data, cache, logs and runtime roots. The
tenant store is a `vak-storage` `LocalStore` (objects and refs) at
`tenants/<t>/store`, opened once per process; `vak_session::documents`
names a Document by the path its file had, grants it to its Agent's scope
and writes by CAS. Memory (one Document per note), entities (one per
entity), skill proposals, Shared Office rooms and the presentation library
are Documents; the memory cleanup command and backup's memory-file merge
went with the files. Decisions (maintainer, 2026-10-04): Agent-authored
state moves to Documents; prompt layers (files a person writes in the
Shared and project `.vak/`) and accepted skills (`SKILL.md`, which skill
discovery reads) stay files. Deviations: Desired state (config layers,
bots, allowlist, tasks) is not versioned through Documents here, because a
second record beside the authoritative file is two contracts (invariant
30); it moves with the admin history of M7b. Merging Documents into an
existing home on backup import waits for M9's restore epochs; a backup
carries the tenant store whole.
- FinOps, alerts, commitments, inbox, routing, misread, security and
  operations become record chains without compaction rewrites.
- Memory, entities, skills, prompt layers, presentation packs and Office
  rooms become **Document** class. Desired state is versioned the same way.
- Sandbox records, candidates, execution streams and coworking grants move
  into their session's Agent scope (D25).
- `auth/` becomes tenant Desired (public keys and recovery-code digests,
  always backed up).

**Slice 4 — runtime out of the project tree (L4, L10)** — done 2026-10-04.
Step 1 done (2026-10-04): execution temp files and caches live in
`<runtime>/executions/<space>/<agent>/` (`vak_config::scope::execution_dir`),
and write sandboxes grant exactly the workspace and that space's execution
root (`sandbox_writes_only_execution_dir_and_space`). Step 2 done
(2026-10-04, decided by the maintainer the same day): drafts and frozen
candidates live there too, still named `.vak/scratch/<agent>/<execution>/…`
(`draft_location`, `vak_tools::drafts`), with a scoped exception to
invariant 10 (`drafts_live_outside_the_project`,
`another_agents_drafts_are_unreachable`); the Review flow has not yet been
checked live in a browser at the time. Step 3 done (2026-10-04): a non-built-in
Agent's workspace is `tenants/<tenant>/workspaces/<space>/<agent>/`
(`vak_config::paths::agent_workspace`, Workspace class), and a
`space-root` file beside it names the project whose layer defines the
Agent (`agent_workspace_is_not_an_environment`). Step 4 done
(2026-10-04): every run's environment (a Best-of-N or `--worktree` git
worktree, a revision's task copy, a staging tree) is
`tenants/<tenant>/environments/<run>/` (`vak_config::paths::environment_dir`),
never in a project or an Agent home. No `.vak` literal is left outside
`vak_config::scope` and tests, and the client's `.vak/scratch` checks stay:
step 2 kept that name for drafts, so they recognise drafts correctly.
Checked live on 2026-10-04 on a fresh 7.0 data home with a local Ollama
model: `office_apply` wrote its draft to the execution root, Review read
and diffed it, and accepting it put the file in the workspace, which
held nothing else but its `.vak/config.toml`. Step 2's design:
- A draft is addressed relative to its execution root
  (`<execution>/<workspace path>` under `execution_dir(space, agent)`),
  never as a `.vak/scratch/…` workspace path; `office_apply::draft_dir`,
  `delivered_file` and the ledger record that form.
- File tools may read exactly the current session's own execution root
  beside the workspace (canonicalised, no symlink or traversal escape);
  another Agent's or another space's executions stay unreachable. Tests:
  `drafts_live_outside_the_project`, `another_agents_drafts_are_unreachable`.
- Consumers resolve the root instead of joining `.vak/scratch`: candidate
  export and its scratch validation, task-copy revision drafts
  (`adopt_revision_drafts`), the agent loop's draft-copy refusal, preview
  scopes, projection artifacts, Review.
- The client and admin UIs stop recognising `.vak/scratch` paths and show
  drafts by their execution-relative path; both bundles are rebuilt.
- Invariants 10, 35 and 39 are reworded in the same change, and the Review
  flow is checked live in a browser.
- Executions and environments live under the tenant.
- Agent workspaces move to `workspaces/<spc>/<agt>/` as Workspace class,
  bound to (Space, Agent), never an Environment.
- Sandbox profiles grant only the execution directory, the Agent workspace
  and the space root.
- `App.tsx:903` loses its `.vak/scratch` check.

**Slice 5 — space identity (review R12)** — done 2026-10-04.
Decided by the maintainer on 2026-10-04: a space id is a `spc_` UUIDv7 in
the tenant registry (`tenants/<tenant>/spaces.toml`, Desired) with this
machine's folder bindings, nothing is written into the project, and every
path-keyed store re-keys, so `workspace_key(path)` is deleted. Step 1 done
(2026-10-04): `vak_config::spaces`, where only `bind` writes (opening a
`Core`, recording trust, saving a secret, creating an Agent workspace) and
resolving a key never does; session ledgers, memory, entities, skill
proposals, executions and Agent workspaces key by space id, and an Agent
workspace resolves to its space's; credential scopes are `space-<id>`,
`agent-<id>` or `tenant`; trust markers are named by space id, with one
writer (`trust::record`) and process-only trust for a `Core` opened
trusted (`serve --trust`), so `mark_trusted` is gone; the admin's
`project_hash` is `space_id`. Step 2a done (2026-10-04): a scheduled task
names its space (`TaskDef.space`), the scheduler and every task list
compare space ids, a server runs only its own space's tasks in the folder
it opened, and the task APIs project the folder as `workspace`. Test
binaries isolate their home automatically, so no test reaches the real
data home or keychain. Step 2b done (2026-10-04): allowlist entries, bots
and channel bindings store a space id (the admin API still takes and shows
a folder, binding it when set); `gateway/default-workspace` holds a space
id; recent, forgotten and named workspaces are fields of the space
registry, so `workspaces.json` and `workspace-names.json` are gone and the
console lists spaces instead of scanning ledger directories; the desktop
trust gate goes through `vak_core::trust`, keyed by space. An audit the
same day closed what slices 3 to 5 had left as bridges: a run's
environment keys as `env-<run>` and never enters the registry; a chat set
to a space with no folder here is refused, never moved to the default
workspace; space-keyed writers bind first and a ledger or Document under an
`unbound-` key is refused; sandbox records, execution streams and
coworking grants are record chains whose decision readers fail on an
undecodable row; the plugin invocation log, which duplicated Activity rows
and sat in the project's `.vak`, is gone with its unused endpoint; and
`Core` no longer falls back to a data home inside the project. The mail and
calendar connection ledger stays its own encrypted file (doc 80). Kept as paths, by
decision: `CorePool` keys (a `Core` is rooted at a folder, and one space's
Agent workspaces are separate folders) and `[server] workspace_roots` (a
browsing boundary, not a space).
Each item is keyed by space id in this slice:
- credential scopes (`scope_key_for`), trust markers
- CorePool identity, allowlist workspace fields, the
  `gateway/default-workspace` file
- `TaskDef.cwd` and the scheduler filter
- `/workspaces`, the desktop trust gate
- `[server] workspace_roots`

**Slice 6 — docs, site, scripts, services** — done 2026-10-05.
- Every item in blast-radius M3b slice 6. Then release 7.0.0.

Done so far: the everyday word for a Space is **Project** (doc 75 §7,
decided by the maintainer); Configure › Projects lists, names, hides and
shows projects by id (`/admin/api/projects`), replacing the workspace-names
section and its endpoint; session forensics groups by project. The exit
test `secret_scopes_keyed_by_id` exists. `scripts/install_gateway_service.sh`,
a second service installer reading a plaintext `.env`, is deleted
(`vak self services-sync` is the one way); `linux-stack.sh` reads the
gateway token through `vak open admin --print`; `linux-check.sh` checks the
7.0 roots after a purge. Backups now cover the data home (they had been
exporting one Agent's home, against a registry relative to the data home,
and so copied nothing), refuse a destination inside it
(`backup::within_home`), and record whether the tenant keys travel
(`content_keys_included`). The registry's top-level `sessions` entry and
the server's scans of it, a 6.x location, are gone. Docs 02, 04, 22, 28,
29, 33, 39, 46 (the layout table, now pointing at the registry), 72, 77,
78, 82 and 85 and AGENTS.md describe the 7.0 layout; CHANGELOG has the 7.0
entry; the site needed no change. `docs/architecture/write-paths-and-growth.html`
stays a dated v3.5.1 measurement with a 7.0 note: a growth remeasure needs
a 7.0 home with real use. The first live dev run failed to start: the tenant key vault's credential
scope (`<tenant>/keys/vault`) had been read as a project folder by slice
5's scope rule, so its key was looked up under a new name and a bogus
project was bound; `tenant`/`agent-<id>` scopes could also collide between
two data homes in one OS keychain. Credential scopes are now named by
explicit owner (`home-<id>`, `home-<id>-agent-<a>`,
`home-<id>-tenant-<t>-keys`, a folder in the data home by its relative
path, never bound; `space-<spc>` for a project), and the unreadable dev
home was moved to the Trash with the maintainer's agreement. The live dev
run on a fresh 7.0 home (local Ollama, a turn that saved a memory note,
Configure › Projects, a rename, the session view by project) then passed,
after fixing what it found: the search index (`store.db`, derived) kept a
file from before the `space_id` rename and failed every sessions query, so
the index now stamps its schema version and is dropped and rebuilt on a
mismatch (`an_index_from_another_schema_is_rebuilt_not_misread`); the
sessions list now carries the server's own project id. **M3b is done**; the
release is held until the maintainer decides the version.

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

#### M4 design (agreed 2026-10-05; done 2026-10-06)

The maintainer asked for the design that serves the later kinds best, since
there are no users to keep. Implementation starts in a later session, one
step at a time, in the order below.

**What the tree has today** (re-scanned at `bd326e08e`; the blast-radius
counts were taken at `438cfcd5`):
- Ids are already minted (M1): `RunId`, `TriggerId` (`trg_`) and `EffectId`
  (`eff_`) exist, and `SessionHeader` already carries `run` and `cause`. So
  "the ledger names its cause" is done. M4 adds the Run *record*.
- `TaskDef` (`vak-core/src/tasks.rs`) is still a plain JSON array in
  `tasks.json` under the shared scope, not a Document. Since the blast-radius
  scan it has gained `space` (a `spc_` key), `timezone`, `due_at` (once),
  `agent_revision`, `mail_calendar_scope` and `mail_calendar_last_check_at`,
  beside nine `last_*` fields. 14 files name `TaskDef`. `next_fire` is in 6
  files. In the server it is the in-memory marker map, used only by cron
  without a timezone; zoned cron anchors on `last_run_at`.
- The scheduler is about 1,100 lines of `vak-server/src/lib.rs`, from
  `run_task_now` at about :19672 to `start_scheduler`. Tick, catch-up and
  run-now are three paths. Nothing stops two processes on one space from
  firing the same slot. Busy and refused tasks keep their slot and retry
  every tick (M0's `cron_slot_not_lost_on_failure`).
- The only cross-process guard is the mail/calendar routine's OS file lock
  (`vak-mail-calendar/src/vault.rs` `try_acquire_routine_lease`). Session
  ledgers are guarded by an OS `try_lock` (`vak-session/src/log.rs`). Neither
  holds an epoch.
- The tenant store (`vak-session/src/objects.rs`, a `LocalStore`) already
  has refs with a generation and a writer epoch, and `Store::epoch` and
  `Store::restore`. No code outside `vak-storage` writes a ref yet, so M4 is
  their first consumer.
- The outbox has been a `RecordChain` since M3b slice 3. It carries `trace`
  and `actor`, with states `Pending | Delivered | DeadLetter`. It has no
  idempotency key and no receipt.
- Channel cursors are not durable at all. Discord and Slack keep an
  in-memory map and adopt the newest message on cold start. Telegram keeps
  its offset in the loop. Mail/calendar cursors live in the encrypted vault
  and advance atomically with the backlog.
- The model-facing tool is called `tasks` (`vak-core/src/tools_tasks.rs`).
  Flows keep their own `flow-runs/` directories and the
  `/flows/{name}/runs` routes. Best-of-N starts at about lib.rs:18431. The
  admin console links `#/operations/work/runs/` in 6 places.
- `EnvironmentBackend` exists (`vak-sandbox/src/lib.rs:59`). A non-git space
  is still refused, at about lib.rs:20099.

**Words** (doc 75 §7, L11). Everyday screens say **Automation** (a trigger
plus what it does), **Run** and **Action** (an effect, named by its kind:
"Sent to Telegram", "Calendar event changed"). An action's status reads
Sending · Sent · Didn't send · Not sure it was sent. "Trigger" and "effect"
appear only under Technical details. "Routine" goes from user-facing strings
(`taskWords.ts` and the Canvas), because later kinds (event, webhook, watch)
are not routines.

**Names.** In code, the API and the CLI the type is `Trigger`, as docs 73,
76, 80 and 81 already say. The model-facing tool `tasks` becomes
`automations`, the word the person and the model share. The CLI commands
are `vak triggers`, `vak runs` and `vak effects`.

**Fencing and where the epochs live.**
- There is one writer epoch per tenant store. It is `Store::epoch()`, held
  in the store's refs table and bumped only by `Store::restore()` (and later
  by a host handoff, doc 79, §11).
- `Core` reads the epoch once at open into a `WriterEpoch`. It passes that
  epoch to every ref CAS, which `refs::decide` refuses when the epoch is
  stale. The same check runs before every append to `runs/` or `effects/`,
  at `SessionLog` open and at `begin_turn`.
- Live processes on one store share the epoch, and CAS generations keep
  them apart. The epoch fences a restored or superseded copy.
- A fenced process stops its scheduler, pollers and dispatch, refuses new
  turns, and reports `fenced` in `/health`.
- Each server process mints a `ProcessId` (`prc_`, UUIDv7) at start. It
  renews a liveness ref `proc/<prc>` (`alive_until`) on every scheduler
  tick, and leases are judged by it. There is no lease per run.

**RunRecord.** A `runs/` record chain in the shared scope, declared in
`REGISTRY`. Its rows are events folded into a projection:

```
RunEvent { run: RunId, at, step: RunStep }
RunStep::Opened  { trace: TraceKey, cause: Cause, trigger: Option<TriggerId>,
                   slot: Option<Slot>, attempt: u32, holder: ProcessId }
RunStep::Skipped { reason }            // slot or event decided without starting
RunStep::Coalesced { into: RunId }     // missed slots folded into one run
RunStep::Session { session_id }        // each ledger the run writes or spawns
RunStep::Settled { outcome: Completed | Failed{reason} | Cancelled,
                   result_id, cost }   // a run's effects name it (M4.5)
RunStep::Abandoned { holder, noticed_by: ProcessId }
Slot = At(DateTime<Utc>) | Event(EventId)
```

`RunRecord` is the fold. Its status is Running, Completed, Failed,
Cancelled, Abandoned or Skipped.
- Every cause writes `Opened` before any side effect: user turn, channel,
  schedule, manual, delegation, revision, heartbeat, flow, best-of-N and
  export. If that append fails, admission fails.
- A refusal (no provider, a lease held elsewhere, the previous run still
  going) is a `Skipped` or `Failed` record, never an `eprintln!`.
- The `RoutineFailed` inbox note stays as the notification and names the
  run.
- On startup, and on each tick, an open run whose holder's liveness has
  expired gets `Abandoned`.
- A reader fails on a row it cannot decode.

**Trigger** (replaces `TaskDef` and `tasks.json`). Desired state is a
versioned Document `triggers/<trg>` in the tenant store:

```
Trigger { id: TriggerId, name, agent: AgentId, agent_revision: Option<u64>,
          space: SpaceId, enabled, kind: TriggerKind, action: TriggerAction,
          deliver_to: Option<String>, on_crash: OnCrash,
          scope: Option<RoutineScope>, created_at, created_by: PrincipalId }
TriggerKind  = Schedule(Schedule) | Manual
               // event, webhook, on_open, watch, source_poll arrive with
               // their consumers on this same shape
Schedule     = Cron { expr, timezone }      // zone resolved and stored at create
             | Interval { every_secs, anchor }
             | Once { at }
TriggerAction = Prompt { text, model_pin: Option<String> } | Script { path }
OnCrash      = Skip (default) | RetryOnce
```

- `agent` is required.
- Every `last_*` field and `mail_calendar_last_check_at` go. "Last run" is a
  query over `runs/`. The mail check position becomes a cursor (below).
- An interval's slots are `anchor + k·every`, so every kind has
  deterministic slots that can be claimed.

**Claims, and one `due(now)`.**
- Each trigger has a ref `trg/<id>/claim` whose target is
  `{ high_water: Option<Slot>, active: Option<{ run, holder, attempt }> }`.
- `due(trigger, claim, now)` is a pure function. It serves the tick,
  startup catch-up and run-now, and `next_fire`, `advance_marker` and the
  separate catch-up path go.
  - The newest slot after `high_water` fires.
  - Earlier missed slots are `Coalesced` into it, or `Skipped{missed}` when
    `[automation] catch_up_missed = false`.
  - Run-now is `Slot::Event` with a fresh id.
- A start CASes the claim (new `high_water`, `active` = this run) under the
  writer epoch, then writes `Opened`, then does the work. Settling clears
  `active` by CAS.
- If `active` names a live holder, the slot is recorded
  `Skipped{previous_run_running}` and spent. If the holder is dead, the old
  run is recorded `Abandoned` first, and `on_crash = RetryOnce` re-runs that
  slot once as attempt 2.
- A refused slot is spent, with a `Failed` record and the inbox note. "Never
  silently lost" now means "never unrecorded", so M0's
  `cron_slot_not_lost_on_failure` is rewritten as `skipped_slot_is_a_record`.
- The mail/calendar routine's OS lease is removed. The claim is the one
  lease.

**EffectRecord.** An `effects/` record chain in the shared scope:

```
EffectEvent { effect: EffectId, at, step: EffectStep }
EffectStep::Prepared   { kind: EffectKind, run: RunId, trace: TraceKey,
                         idempotency_key, payload_digest, target,
                         payload: ObjectRef, hold: Option<reason> }
EffectStep::Dispatched { attempt, holder: ProcessId }
EffectStep::Accepted   { receipt }   // the provider took it
EffectStep::Confirmed  { receipt }   // the provider proved it landed
EffectStep::Failed     { reason, proven_not_sent: bool }
EffectStep::Unknown    { reason }
EffectStep::Reconciled { outcome: Sent | NotSent, receipt: Option<Receipt>,
                         by: PrincipalId }
EffectStep::Superseded { by: EffectId }   // the owner chose Send again
Receipt = { provider, provider_id, at }
EffectKind = Delivery { surface, bot, chat }
           | MailSend | CalendarCreate | CalendarUpdate | CalendarCancel
           | CalendarRsvp                     // each { account }
```

- `idempotency_key` = hash(run, kind, target, payload_digest, ordinal). It
  is stable when a job is rebuilt.
- The process that wins the CAS of the ref `eff/<key>` from absent to
  dispatched, under the writer epoch, is the only one that calls the
  provider.
- After a restart, a restore or a handoff:
  - A `Prepared` effect may be dispatched.
  - A `Dispatched` effect with no outcome becomes `Unknown` and is never
    sent again.
  - A `Failed` effect is retried only when `proven_not_sent`.
- An unknown outcome settles in one of two ways:
  - Where the provider drops duplicates itself (Discord `nonce` with
    `enforce_nonce`, the key as nonce), a resend is safe and reconciles it.
  - Anywhere else, the owner sees "Not sure it was sent" and chooses Send
    again (a new linked effect) or marks it sent or not sent.
- Mail and calendar's single-use action claims become effects of their
  kinds. The claim *is* the dispatch CAS.
- `Outbox`, `OutboxRecord` and `DeadLetter` go. Delivery replay reads
  `Prepared` effects. A held digest is a `Prepared` with a `hold`.

**Cursors.** A cursor is a ref `cur/<owner>/<stream>` whose target is
`{ position, backlog: Option<ObjectRef>, resynced_from }`. It advances by
CAS under the writer epoch, so position and backlog move together.
- An expired position (Gmail history 404, Graph delta 410, a Slack or
  Discord gap) resyncs within a bound and appends a `gap` row to a
  `cursors/` chain. It never refetches everything.
- The Telegram, Discord and Slack pollers hold their cursor. A second
  poller of the same bot loses the CAS and stops.
- The mail vault's cursors and backlog move onto cursor refs, with the
  backlog stored as an encrypted tenant object.

**API and screens.**
- `/tasks…` go. Their replacements:
  - Triggers: `GET|POST /triggers`, `GET|PUT|DELETE /triggers/{id}`,
    `POST /triggers/{id}/run`, `GET /triggers/{id}/slots`.
  - Runs: `GET /runs?agent=&trigger=&cause=&flow=` and `GET /runs/{id}`.
  - Effects: `GET /effects?run=&state=`, `GET /effects/{id}`,
    `POST /effects/{id}/resend` and `POST /effects/{id}/reconcile`.
- `/tasks/{id}/retry-delivery` becomes `/effects/{id}/resend`.
- `/flows/{name}/runs` becomes `/runs?flow=`, and a flow's state directory
  is named by its `RunId`.
- Admin `#/operations/work/runs/<session>` becomes `#/runs/<run_id>`.
- New screens: Runs (A4) and Run detail (A4b), Automations (A5, moved from
  Configure), and the client Runs panel (C6), which replaces the last-run
  fields in `TasksModal`.

**Steps.** Each step ships whole (code, tests, docs and its screens,
invariant 30) and is committed green before the next starts.

| Step | What | Exit tests |
|---|---|---|
| M4.1 | **Done 2026-10-05.** Fencing groundwork: `WriterEpoch` read at open; epoch on ref CAS, session open, `begin_turn` and chain appends; `ProcessId` and liveness ref; fenced state in `/health` | `restore_fences_old_writer`, `fenced_process_stops_background_work` |
| M4.2 | **Done 2026-10-05. A run's `work` (flow or plan) names what a run that is not a turn does. A turn's run record costs two syncs (opened with its ledger named, then settled), so M3b's per-turn budget is now eight.** `runs/` chain and `RunRecord`, written for every cause; abandoned sweep; `/runs`; `vak runs`; flows keyed by `RunId`; admin Runs and Run detail; `#/runs/<id>` | `every_cause_writes_run`, `abandoned_run_is_recorded`, `run_open_failure_refuses_admission` |
| M4.3 | **Done 2026-10-05. Until M4.4, slots count from the newest run that started (a refused slot stays due), each run keeps its own worktree, and delivery state is read from the outbox by trigger.** `Trigger` replaces `TaskDef`: Document store, kinds, `on_crash`, no `last_*`; `/triggers`; `vak triggers`; the `automations` tool; Automations screen and client Runs panel; the new words; `scheduled_runs.rs` and `scheduler_personal_os.rs` rewritten | `last_run_is_a_query`, `trigger_round_trips_as_document` |
| M4.4 | **Done 2026-10-05. `high_water` is the newest schedule slot spent (an instant: a run-now event never moves it). Missed slots are one record for the range (`Missed { from, through }`), coalesced into the run that starts or skipped, never one record per slot. The claimant renews its liveness before it moves a claim, and an `Abandoned` record names the trigger and slot, so a run abandoned before it opened is still that trigger's. "Last run" skips coalesced records.** Claims and the one `due(now)`; `next_fire` and the mail OS lease go; skipped and coalesced records; `on_crash` | `schedule_slot_at_most_once_under_restart` (property test over crash points), `two_processes_do_not_double_start`, `skipped_slot_is_a_record`, `retry_once_retries_once` |
| M4.5 | **Done 2026-10-05. `EffectKind` has only `Delivery` until M4.6 adds its kinds; `run` is optional, because a delivery with no admitted run (a budget alert) names none. An effect's status is held, queued, sending, sent, retrying, failed, unknown or superseded; a failure not proven unsent (part of a multi-message send) reads unknown. A held effect waits: no digest or completion flush exists yet. Webhooks get the key as `Idempotency-Key`. The admin Run detail lists a run's actions; the per-job Operations route is gone.** `effects/` chain; delivery as the first kind; the outbox goes; unknown outcomes and reconcile; `/effects`; Discord nonce; AGENTS.md's new effects invariant | `effect_unknown_until_reconciled`, `effect_not_replayed_after_restart`, `discord_resend_reuses_nonce` |
| M4.6 | **Done 2026-10-05. Cancel is its own kind. A candidate's effect is keyed by the candidate alone (`prepare_once`), so a second request finds it and a raced twin is superseded at dispatch; its payload is the candidate reference, never content. These kinds are not retried by the background sender, and one left queued when its five-minute approval lapses is failed (`fail_unsent`). The provider marker is the effect id's UUIDv7. The unused `ProviderAdapter` trait and `ActionReceipt` went with the vault receipts.** Mail and calendar send, create, update and RSVP become effects; their single-use claims go | `mail_send_is_one_effect`, `unknown_mail_send_never_resent` |
| M4.7 | **M4.7a done 2026-10-05: `vak_session::cursors`; the Telegram, Discord and Slack pollers hold one owner per bot (`cur/bot/<surface>/<bot>/holder`) and one cursor per stream, and record gap rows; the 24-hour resume bound replaces replaying a stale backlog; Telegram's per-token lock file goes; the Operations Center lists gaps. M4.7b done 2026-10-05: the mail vault's routine cursors are one cursor per Agent (`cur/agent/<agent>/mail-calendar/routines`) whose backlog, an encrypted tenant object, holds each routine's provider position and queued ids, moved by `Cursors::swap_if`; the credential-store blob and its lock file go. The vault keeps its routine run history, because it binds a run to an account and its item count and is removed on disconnect, which append-only run records cannot be until M7a. `seal::decompress` sizes its buffer to the frame's content, and two writers putting the same bytes at once no longer leave one scope with a key that does not open the body (the body is created only if absent; the loser re-grants with the winner's verified key).** Cursors: channel pollers and the mail vault; gap records | `cursor_resync_records_gap`, `second_poller_is_fenced` |
| M4.8 | **Done 2026-10-06. `vak_sandbox::copy::CopyEnvironment` is the first `EnvironmentBackend`: it copies the folder into `environments/<run>/`, skipping `.git`, `.vak`, `node_modules`, `target`, `.venv`, `__pycache__` and `.DS_Store` within 20,000 files and 512 MiB, and exports added, changed and deleted files (deletes as delete operations) frozen with `freeze_exported` as a candidate on the run's session; the copy is removed after. `skipped_slot_is_a_record` now refuses by an unknown Agent. A copy orphaned by a crash mid-run is left for M7a's reconciler.** `CopyEnvironment`; the non-git refusal goes; AGENTS.md invariant 38 restated; docs 22, 29, 64, 76, 80 and 81 cite the shipped shapes | `non_git_space_routine_runs_in_copy_environment` |

### M5 — Telemetry (M, after M1, in parallel) — done 2026-10-06

M5a (2026-10-06): `crates/vak-telemetry` (the subscriber, a content-free
layer writing JSON lines with allowlisted field names and the run's
`trace_id`, span-close lines with durations, `<logs>/vak-<service>.jsonl`
rotated at 16 MiB keeping five), `tracing` and `tracing-subscriber` pinned
(`std`, `registry`, `env-filter`; no `fmt` layer, so nothing bypasses the
allowlist), every library `eprintln!` converted, a root `clippy.toml`
banning `eprintln!`/`println!` with allows only where stderr is a
person's, and the exit tests `library_crates_have_no_eprintln`,
`log_lines_are_json_with_trace_fields` and `telemetry_carries_no_content`
(canaries at the layer, plus `library_log_messages_are_literals`). The
OTLP exporter is not built: nothing in the tree needs it yet, and the
optional `[telemetry] otlp_endpoint` is not offered until it is. M5b: the
span tree run › turn › step › (dispatch | tool_call › execution) ›
delivery with the worker continuing its parent span, `one_run_one_trace_id`,
the Traces & logs screen and Run waterfall, service log readers in
`vak-ops`, and bus stream `max_age`.

M5b1 (2026-10-06): the span tree. Whoever opens a run does its work in
`vak_session::runs::span` (the turn chain for an admitted run, a turn
with none, a delegated child, a flow, a plan, a claimed automation); a
turn opens `turn` before its run is minted and records its ids once it
is; the agent loop opens `step` per iteration, `dispatch` per provider
attempt and `tool_call` per call; an effect's attempt is
`vak_session::effects::delivery_span` under its run's `trace_id`. The
tool worker has no log (it is sandboxed and its environment scrubbed):
it captures its lines under an `execution` span naming the run and the
call's span (`parent_span`) and returns them in its response, and the
caller passes them to `vak_telemetry::forward`, which checks them
against the allowlist again and writes them under its own span path.
`one_run_one_trace_id` passes.

M5b2 (2026-10-06): the readers. `/telemetry/services`, `/telemetry/logs`
(service, level, run, timings) and `/runs/{id}/spans` sit behind the
server's authentication and read `<logs>/vak-<service>.jsonl` through
`vak_telemetry::read_services`; the admin console has System › Traces &
logs (`#/diagnostics`) and a timeline (span waterfall) in Run detail.
`vak-ops` maps each managed service to its structured logs
(`telemetry_services`, `recent_log_lines`), "Open log" opens them, and
`vak self status` prints each service's newest problem.

M5b3 (2026-10-06): the server's bus publishes a `SystemEvent` as its
kind and ids only (`vak_server::bus::reference`), and `NatsBus` creates
each work queue's stream with work-queue retention and a 24-hour
`max_age` (`work_stream_config`). With it, M5 is done.

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

### M6 — Data catalog, search, lineage (L) — design agreed 2026-10-06

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

#### M6 design (agreed 2026-10-06)

The maintainer chose each option below on 2026-10-06. Implementation goes
one step at a time, in the order below, each committed green.

**What the tree has today** (scanned at `795e440c4`):
- Three searches. `vak-store` (`store.db` in the cache home: entry
  locators, lineage, turn descriptors and FTS5 over every content block,
  thinking and raw tool output included) serves admin search and the
  turn recall in `vak-core/src/indexed_history.rs`.
  `vak_session::search_all_extended` scans ledgers with an mtime cache
  for `/search` and the model's `session_search`. `vak-store` also holds
  `PresentationStore`, a JSON file that is not an index at all.
- Three directory walks in `vak-server/src/lib.rs` find a session by id
  (`find_session_on_disk`, `read_historical_header`,
  `find_session_in_cwd`; 22 call sites).
- The turn path folds whole chains: `RouteEvidence::snapshot` reads every
  evidence row on each turn, `SessionLog::has_request_admission` scans the
  ledger's entries, and `vak_commit::Ledger::append` replays the
  commitment's events to check closure.
- Nothing ingests runs, effects, triggers, memory or commitments into an
  index, so no query joins a file, a delivery or an effect back to its run.

**Decisions.**
- **Ingest is a tailer** (chosen over write-through hooks). The catalog
  keeps one cursor per source, `(source, segment, frames read)`; sealed
  segments never change, so a cursor resumes where it stopped. It catches
  up after each turn, on the scheduler tick and before answering a query;
  a writer sends only a hint. A missed hint costs latency, never
  correctness (the invariant 31 pattern). Ingest is an idempotent upsert
  keyed by `(source, seq)`, from any process (WAL, `busy_timeout`), so no
  process owns it. `rebuild()` drops the file and replays every source
  from its start.
- **The text index holds doc 73's projections** (chosen over everything):
  user and assistant message text, each tool result's digest (never raw
  output), card text, memory notes and run summaries. No thinking. Admin
  search no longer matches raw tool output; the turn's recall still
  reaches it by evidence id through the ledger (invariant 36).
- **The catalog decides nothing that must be right** (chosen over catalog
  projections). Request admission is a ref `req/<request id>` moved by CAS
  under the writer epoch; routing evidence and commitment state are
  rollups kept current as Documents on each append, replayed only when
  rebuilt. The catalog answers search, lineage and "where is it"; a stale
  catalog can make a search miss, never admit a duplicate request or
  close a commitment wrongly.

**Shapes.**
- `crates/vak-catalog` (replaces `crates/vak-store`), one SQLite file per
  tenant at `<data>/tenants/<ten>/catalog.db` (doc 73 §6):
  - `nodes(id, kind, tenant, space, agent, session, turn, run, actor,
    cause, audience, created_at, sealed_at, size, locator)`: a session,
    turn, run, effect, trigger, memory note, commitment, file the turn
    wrote, candidate and delivery. `locator` is where its bytes live (a
    ledger directory and position, a chain, a Document path), so "where
    is session X" is one row.
  - `edges(from, kind, to)`: `caused` (run → run, run → session),
    `produced_by` (turn → run, file → turn), `delivered_as` (run → effect),
    `derived_from`, `references`, `version_of`, `promoted_to`.
  - `text` (FTS5 over the projections above), `cursors(source, segment,
    frames)`, `meta(schema, digest)`. The digest is a hash over every
    source's head (segment count and last frame); `stale()` compares it.
- **One API**: `Catalog::search(query, &Audience) -> hits`,
  `lineage(node) -> path to its cause`, `open(id) -> Node` (with its
  locator), `stale()`, `rebuild()`. `Audience` is the caller's principal
  and the Agents and conversation audiences it may read; every query
  filters by it before ranking (invariant 37). Trash is applied there too.
  HTTP: `/search`, `/lineage/{id}`, `/nodes/{id}`, `/catalog` (status) and
  `POST /catalog/rebuild`; `/admin/api/search` is gone.

**Steps.**
1. **M6.1** `vak-catalog`: schema, sources and tailer cursors, ingest of
   session ledgers, runs, effects, triggers, memory and commitments,
   `search`/`lineage`/`open`/`stale`/`rebuild`. Exit tests
   `catalog_rebuild_equals_incremental`, `search_respects_audience`,
   `lineage_from_any_artifact_to_cause`,
   `catalog_query_p95_under_50ms_at_1m_nodes`. Nothing calls it yet.
2. **M6.2** one search: `/search`, admin search, `session_search` and the
   turn recall call the catalog; `vak-store`, `search_all`, its mtime cache,
   the recall ledger cache and the three directory walks are deleted;
   `PresentationStore` becomes Documents (review R9); the server ingests
   after each turn and on its tick.
3. **M6.3** the flat turn path: the `req/<id>` ref, routing evidence and
   commitment rollups as Documents. Exit test `turn_path_reads_flat`.
4. **M6.4** screens: admin and client search on `/search`, the Lineage tab
   in conversation detail, catalog status and rebuild in Integrity; a
   browser run of each.

M6.1 (2026-10-06): `crates/vak-catalog` and `vak_session::tail` (a
position in a segment directory, valid across seals). Calls are nodes
`call:<session>:<tool use id>`, files `file:<space>:<path>`, memory notes
`memory:<path>` and commitments `commitment:<id>`; sessions, turns, runs,
effects and triggers keep their own ids. All six exit tests of the step
pass, among them `catalog_query_p95_under_50ms_at_1m_nodes` on a million
seeded nodes.

M6.2 (2026-10-06): one search. `vak-store` and `vak_session`'s `search`
module and its mtime ledger cache are deleted; `Core::catalog()` opens
the tenant's catalog (`tenants/<ten>/catalog/catalog.db`, a directory so
its SQLite sidecars are declared with it) once per process. `/search`
(with `all` and `kind`), the model's `session_search` and the admin
console read it; `/admin/api/search` and the store endpoints are gone,
replaced by `/catalog`, `POST /catalog/rebuild`, `/nodes/{id}` and
`/lineage/{id}`. The turn recall (`indexed_history.rs`) reads entry
locations, branch jumps and turn records from the catalog
(`vak-catalog/src/history.rs`, ported from `vak-store`). The three
directory walks are one catalog lookup (`locate_session`). The admin
sessions list reads session nodes; its transcript pages the ledger
itself. The catalog now also holds entities, splits memory notes per
Document with their tag and source turn, gives a note its source
conversation's audience and the profile tier the local one, indexes text
written outside any turn on its session, and takes a session's space
from its ledger's directory when the header names none. It catches up at
server start, after each turn and on the scheduler tick.
`PresentationStore` moved to `vak_core::presentation_store` (already a
Document).

M6.3 (2026-10-06): the flat turn path. `vak_session::rollup` keeps a
chain's projection as a Document with the chain position it covers; a
read folds only what was appended since, saves when it folded anything,
never moves the Document back past a later save, and rebuilds when the
chain is behind it (a cut unsynced tail). Routing evidence
(`routing-evidence-rollup`, day buckets per leg) and commitment state
(`commitments-rollup`, every commitment's projection; `project` split
into `start` and `apply`) read through it. A request's admission is the
ref `req/<session>/<request digest>`, moved by `SessionLog::append` after
the admission activity is synced and read by `has_request_admission` /
`request_admitted` without opening the ledger; an unreadable ref counts
as admitted. The in-process in-flight set stays: it guards concurrent
requests in one process and is released on failure, which the durable ref
is not. `turn_path_reads_flat` passes with every sealed segment of each
chain made unreadable.

M6.4 (2026-10-06): the screens. Search on `/search` landed with M6.2
(the admin console's kind filters and the client's search sheet).
Conversation detail has a Lineage tab: what the catalog holds for the
conversation (`GET /catalog/sessions/{id}/nodes`, `Catalog::in_session`),
filtered by kind, and for the chosen item its path up to its run with the
cause, Agent and actor (`/lineage/{id}`), linking to Run detail. The
Housekeeping panel beside **Rebuild search** says whether the catalog has
taken every record and its counts (`/catalog`). A trashed conversation's
node, lineage and holdings answer 404 (`trash.rs`). Browser run against
the live data home: a `glob` call traced to its run, caused by a person;
a rebuild took 77 records from 13 sources.

### M6.5 — Intake (L, after M6; doc 76)

- Sources are Desired, polled through M4's `source_poll` triggers with
  cursors; push intake (`save_to_inbox`) is one push connector.
- Items are objects plus catalog nodes with a TraceKey, `derived_from` and a
  disposition; detection labels and never drops.
- The agent reaches intake through the one retrieval tool doc 76 D1 settles.
- The Python pipeline, DuckDB store, `feed_mcp.py` and its search table are
  deleted in the same change.
- Doc 76's open decisions D1 and D2 are settled before it starts.

**M6.5 design (agreed 2026-10-06).** The maintainer settled doc 76's
decisions. **D1:** `session_search` is widened. Intake items become one
more kind it searches, and `recall` stays the in-session lookup by turn,
presentation or evidence id, so no new tool is added. **D2:** every
connector is Rust. That covers RSS/Atom, YouTube's channel XML, Hacker
News, Reddit and Lobsters JSON, and custom HTTP (JSON, RSS or text). No
Python runtime remains.

The shapes:
- **Source.** A Document `sources/<src>` in the tenant store, owned by an
  Agent. It holds `{id, name, agent, connector, tags, trust, enabled,
  created_at, created_by}`, where `connector` is a closed enum (`rss {url}`,
  `youtube {channel_id}`, `hacker_news {list}`, `reddit {subreddit, sort}`,
  `lobsters {list}`, `http {url, format}`). Sources are written only
  through the authenticated API. The `[feeds]` config key, `feeds.toml` and
  its untrusted-layer stripping go: there is no config path left to
  demote.
- **Polling.** A source polls through one trigger of its own with
  `TriggerAction::SourcePoll { source }` on an `interval` schedule. Every
  poll is a run with cause Schedule, and Poll now uses Run now's claim.
  The source's cursor `cur/agent/<agent>/intake/<src>` holds the ETag,
  Last-Modified and a bounded set of recent item keys. A connector that
  cannot resume records a gap.
- **Fetch and parse.** The server fetches with webfetch's SSRF guard, which
  is shared rather than copied: every redirect is re-checked, and bytes
  and time are bounded. Connectors are closed host code, so no workspace
  code runs and no secret is passed. The bytes are parsed by a
  network-denied worker task, `IntakeParse { connector, data }`
  (invariant 14), into normalised items. This follows the mail/calendar
  precedent: fetch from the host, parse hostile bytes in a worker.
- **Item.** The body (title, link, author, published, text) is a tenant
  object, deduped by content. Each item is one row in the `intake/` record
  chain: `Taken {item, source, key, object, trace, disposition, labels,
  evidence}`, then `Released` or `Quarantined` rows as an operator acts.
  The item id is `itm:<src>:<digest(key)>`, so a re-poll is idempotent. The
  catalog tails the chain and the Sources Documents. Item and source
  nodes carry `produced_by` the run and `derived_from` the source. The
  text is title plus body, and the audience is the source's Agent.
- **Detection labels, never drops.** The detector is ported from
  `feed_security.py`. It gives a disposition of accepted, quarantined or
  blocked, and never leaves the evidence empty. Its false-positive rate is
  checked against a committed corpus of ordinary headlines. A quarantined
  or blocked item is excluded from `session_search` and alerts until it is
  released.
- **Alerts.** An alert is a Document `alerts/<alr>` with keyword, tag and
  source matches and a cooldown. Alerts are evaluated after each poll on
  new accepted items. A match is an Inbox entry `Kind::IntakeMatch`, made
  idempotent per (alert, item) by a ref. Channel delivery, when an alert
  names a target, is a delivery effect.
- **Push intake.** `save_to_inbox` records the saved file as an item of
  the built-in `push` source, so a channel attachment is searchable like
  any other item.
- **Retention.** Each source's maximum item count and dedup window are
  enforced by M7a's reconciler. Until then nothing prunes items. That is a
  stated gap, not a feed cron.

The steps:
1. **M6.5a** sources, polling, connectors, items and detection: the
   chain, the catalog source, `/intake/sources` and `/intake/items`. Exit
   test `intake_item_has_trace_and_provenance`.
2. **M6.5b** retrieval and alerts: `session_search` over items, alerts
   into the Inbox, and push intake. Exit test
   `quarantined_item_absent_from_agent_retrieval`.
3. **M6.5c** screens and deletion: the admin Feeds section and the client
   reader on `/intake`, a browser run, and the deletion of `scripts/feeds`,
   `feeds.rs`, `FeedSettings`, the DuckDB store and the registry entries.
   Exit test `feed_pipeline_is_gone`.

M6.5a (2026-10-06): `crates/vak-intake` (connectors, parsers, detection;
a committed corpus of ordinary headlines holds none),
`WorkerTask::IntakeParse` and `vak_tools::webfetch::guarded_get` (the
one guarded fetch, now shared by webfetch and intake),
`vak_core::intake` (sources as Documents, the `intake/` chain, `poll`),
`TriggerAction::SourcePoll` (the generic automation endpoints and the
model's `automations` tool refuse to make, retarget or remove one), the
catalog's `source` and `item` nodes with `derived_from` in lineage, and
`/intake/sources` and `/intake/items` with release and quarantine. Exit
test `intake_item_has_trace_and_provenance`; `intake_api.rs` covers the
endpoints. A live poll of the Rust blog, Hacker News and Lobsters took
64 items, held none, took nothing new on a second poll, and traced an
item through its source and run to its trigger.

M6.5b (2026-10-06): `session_search` and `/search` cover items
(`kind=item`); `vak_catalog::Audience::held` is false by default, so a
held or quarantined item reaches no Agent read until a person releases
it, and only a person's views (`/search`, `/intake/items`) set it. Items
belong to their Agent, not a space. Alerts are Documents `alerts/<alr>`
(`vak_core::intake_alerts`) with their matches in `alert-state/<alr>`,
moved by CAS: each item matches an alert once, a cooldown keeps matches
waiting rather than dropping them (they go at the next evaluation after
it, which every poll runs), and a notice is an `intake_match` inbox entry
or, with `deliver_to`, a delivery effect. A release evaluates alerts
too. Push intake: a file saved to the inbox, from a channel or a drop in
the client, is an item of the Agent's push source (`intake::push_source`,
`Intake::take_push`) with the receiving run's key; alerts do not watch
push items, which are the person's own. Exit test
`quarantined_item_absent_from_agent_retrieval`; `intake_api.rs` covers
alerts. A live Lobsters poll fired one alert notice for its one Rust
story, and the client's search sheet opens an item at its link.

M6.5c (2026-10-06): the admin console's Sources (`#/sources`: sources,
items with release and hold, alerts) and the client's Sources panel read
`/intake`; the client's separate feeds search sheet is gone, since the
search sheet finds items. Deleted: `scripts/feeds`, `vak-server`'s
`feeds.rs` and its routes and scheduler step, `FeedSettings` and the
`[feeds]` config section (a leftover section configures nothing), the
feed registry entries, the install and release bundling of the feed
runtime, and the feed cases of `scripts/compound_regression.py`. Exit
test `feed_pipeline_is_gone`. With it M6.5 is done.

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
  - Scopes: a conversation, one guest's contributions to it, and a
    connected provider account. Person, Agent, space and tenant are M7b.
  - Preview digests and approvals.
  - A lineage walk, key destruction, derived plaintext removal
    (`secure_delete` plus a WAL checkpoint), and a signed receipt naming
    what it couldn't reach.
  - Provider-account erasure follows source lineage across conversations and
    Agents, destroys the account-scoped key/grants, removes provider-derived
    records and indexes while preserving unrelated conversation data, and
    reports provider dispatches, recipients, backups, and other copies outside
    Vak's control. Disconnect/revocation alone is not erasure.
- **Holds on a conversation** (doc 74 §3.2), checked by the reconciler
  and by erasure; the other hold scopes are M7b.
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
  `provider_account_erasure_spans_agents_and_conversations`,
  `provider_account_erasure_preserves_unrelated_conversation_content`,
  `provider_account_erasure_reapplies_after_restore`,
  `hold_blocks_every_destructive_transition`.
- `stale_preview_cannot_authorise`, `quota_refuses_admission_not_records`,
  `restore_reapplies_erasures`, `revoke_cuts_endpoints_within_one_tick`.
- `thirty_day_soak_stays_within_budget`.
- The browser acceptance scenarios in doc 74 §9 that name these screens.

#### M7a design (agreed with the maintainer 2026-10-08)

What the tree had when the design was settled: session ledger frames are
written unencrypted (the M3b slice 2 deviation), only objects are keyed per
conversation, there is no contributor key, no provider-account key and no
keying of content in shared ledgers; `vak_storage::scopes` already has
`shred`, `hold` and `release`, and memory, entities and learning record
`derived_from`.

Decisions:

1. **Ledger frames are always encrypted** under their conversation key.
   There is no tenant policy switch and no plaintext path; a plaintext
   ledger is not read, so the dev data home is purged once when this lands.
2. **M7a erases three scopes:** a conversation, one guest's contributions
   to it, and a connected provider account. Person, Agent, space and tenant
   are M7b, with the data roles.
3. **Content in a shared ledger is a tenant object** granted to its
   conversation's scope (inbox body, commitment statement, delivery text),
   as an effect's payload already is; the row keeps ids and state. This
   replaces doc 73 §7.3's field encryption: one mechanism, not two.
4. **Holds in M7a are holds on a conversation**, set through the API and
   the CLI and checked by the reconciler's guard and by erasure. The other
   scopes, the query hold and the Retention & holds screen are M7b.
5. **A receipt is signed with a per-tenant Ed25519 key** held by the key
   authority, so a downloaded receipt verifies with the public key alone.
6. **Delete permanently** erases a trashed conversation on the reconciler's
   next pass once its owner types its title; otherwise the 30-day window
   closes it.
7. **The backup rework is M7a's last build step**; M7a is not done without
   `restore_reapplies_erasures`.
8. **Live verification never commits on the real data home.** A dev server
   on the real home runs the reconciler observe-only; commit is verified in
   tests and on a throwaway `VAK_HOME`.

Steps, each shipped whole and in this order:

| Step | What | Exit tests |
|---|---|---|
| M7a-a | **Done 2026-10-08 (note below).** Conversation keys: every session ledger frame encrypted under its conversation's scope key; the readers (the catalog tailer, `SessionLog`) open through it; the per-turn byte and fsync budgets re-measured | `compressed_before_encrypted`, `bytes_per_turn_budget`, `a_destroyed_key_leaves_the_bytes_and_the_chain_and_nothing_readable` |
| M7a-b | **Done 2026-10-08 (notes below).** Content out of shared ledgers into objects granted to the conversation; the contributor key for a guest's frames with the typed placeholder in `derive_messages()`; the provider-account scope and its grant on provider-derived objects | a guest's frames read through their own key; an account-derived object reads only with the account's grant |
| M7a-c | **Done 2026-10-08 (note below).** `crates/vak-lifecycle`: the reconciler, observe-only; the default tenant label; one plan object behind `vak data status`, `usage` and `plan`, `/data/status`, `/data/usage` and `/data/lifecycle`, and the admin Storage (A7) and Lifecycle (A8) screens | `reconciler_observe_only_commits_nothing`, `reconciler_is_idempotent` |
| M7a-d | **Done 2026-10-08 (notes below).** Commit for the ephemeral and derived classes (runtime scrub, environments, checkpoints, Document history, inbox entries, cost and activity segments), object GC and quotas; the scattered retention goes (`MAX_STORED_CHECKPOINTS`, finops and alerts compaction, `cleanup_artifacts`, `/memory/cleanup`); `vak data gc` | `settled_execution_leaves_nothing`, `gc_keeps_everything_reachable`, `quota_refuses_admission_not_records` |
| M7a-e | Trash as lifecycle state (the sidecars, their routes and `/workspaces/forget` go); conversation holds; conversation erasure: preview digest, the lineage walk with artifact versions, key destruction, catalog rows removed with `secure_delete` and a WAL checkpoint, the signed receipt; drafts fade; admin Conversations lifecycle (A2, A3); client menu, Trash, "Why is this gone?" and Workbench states (C1, C2, C3, C8); `vak data erase --scope conversation` | `erasure_follows_lineage`, `erasure_leaves_ledger_bytes_unchanged`, `stale_preview_cannot_authorise`, `hold_blocks_every_destructive_transition` |
| M7a-f | Erasing a guest's contributions and a provider account | `guest_erasure_keeps_owner_conversation`, `provider_account_erasure_spans_agents_and_conversations`, `provider_account_erasure_preserves_unrelated_conversation_content` |
| M7a-g | Agent lifecycle: `Revoked`, each state's data effects, the Agents lifecycle panel (A14) | `revoke_cuts_endpoints_within_one_tick` |
| M7a-h | Backup: ciphertext and wrapped keys, the coherent manifest, restore re-applies erasures and bumps the writer epoch; Backup & restore (A16); `vak data backup` | `restore_reapplies_erasures`, `provider_account_erasure_reapplies_after_restore` |
| M7a-i | Integrity (A9) and Data health (A1); `vak data verify`, `rebuild-catalog`, `export`, `cat` and `grep`; the soak; the new invariant and the docs of §5; the browser acceptance of doc 74 §9 | `thirty_day_soak_stays_within_budget`, the acceptance run |

**M7a-a, done 2026-10-08.** Every frame of a session ledger is sealed
under its conversation's scope key (`conversation:<session id>`, the scope
its objects are already granted to), so one destroyed scope makes the
ledger and its objects unreadable together.
- The tenant holds its scope keys in `<tenant>/keys/scopes`
  (`vak_storage::scopes::ScopeKeys`, wrapped by the key authority);
  `TenantObjects::create_scope_key`, `scope_key` and `destroy_scope_key`
  are the only way to them. Whether a scope was destroyed is read on every
  lookup, so a key another process unwrapped earlier stops opening
  anything. Two creators of one scope get one key.
- A ledger directory names its scope in `KEY`, written when the ledger is
  created, because the header (which carries the system prompt) is sealed
  too. `vak_session::keys` resolves a directory's key; `SessionLog` (open,
  read-only, `read_header`, `scan`, `text`, `read_record_at`) and the
  catalog's tailer (`vak_session::tail`) read through it.
- A ledger with no `KEY` is refused (`SessionError::Unencrypted`, naming
  the purge); a ledger whose key was destroyed is `SessionError::Erased`,
  its bytes unchanged and its chain still verifying.
- The frame's associated data is now its byte offset in the segment, not
  its sequence number, so the indexed read of one record
  (`read_record_at`) opens a frame without the ones before it. No sealed
  frame had been written before this, so nothing on disk changes meaning.
- Measured on the scripted read-then-answer turn: 6,596 bytes and 8 record
  syncs per turn sealed, against 6,214 bytes and 8 syncs unsealed on the
  same tree (budgets 20 KiB and 8).
- Not in this step: side record chains (runs, effects, grants, artifacts,
  inbox) stay unsealed. They hold ids and state for many conversations,
  so no one conversation's key fits them; their content moves into
  conversation-granted objects at M7a-b. A writer that holds a ledger
  open keeps appending after another process destroys its key; M7a-e's
  erasure refuses a conversation with a live turn. The dev data home is
  purged once: its ledgers have no key.

**Live check of M7a-a, 2026-10-08** (dev server on a purged data home,
one turn on local Ollama, through the HTTP API): the conversation read
back, was listed and was found by search, before and after a server
restart. On disk its ledger was `KEY`, `LOCK` and one 14,880-byte segment
holding neither the test phrase nor any JSON field name. The one
plaintext copy of the phrase in the data and cache homes was the
catalog's database, the derived copy doc 73 §7.3 names. Not checked live:
destroying a key (no route does it until M7a-e), a guest, and the app's
screens.

**M7a-b, in four parts; part 1 done 2026-10-08.** Part 1, content out of
shared chains:
- `vak_session::content` is the one mechanism: `seal_fields` and
  `seal_except` move a row's content into one tenant object granted to its
  conversation's scope and leave `sealed` (the scope and the object) beside
  the row's ids; `restore` puts it back, or says `Erased` when the
  conversation's key was destroyed. No row is removed or rewritten.
- An inbox entry that names a session keeps its `title` and `body` there;
  one about no conversation (a budget alert, a routine that could not
  start) stays whole. A reader drops an entry whose conversation was erased.
- A commitment event written under a key that names a session keeps only
  `event_id`, `commitment_id`, `ts`, `kind`, `trace` and `actor` in the
  chain. `events`, the rollup's fold and the catalog's ingest restore a
  row before reading it, so a commitment goes with the conversation that
  opened it and other conversations' commitments stay.
- An effect's payload is granted to its conversation's scope when its key
  names a session (`TraceKey::session_id`), and to the effect's own scope
  otherwise.
- `TenantObjects::get` refuses a destroyed scope from its tombstone, read
  on each call, because the key authority reads its revocations only when
  a process opens it.
- Tests: `a_shared_row_keeps_its_ids_and_loses_its_content_with_the_conversation`,
  `an_entry_about_a_conversation_goes_with_it`,
  `a_commitment_goes_with_the_conversation_that_made_it`,
  `a_delivery_from_a_conversation_goes_with_it`.
- Left for erasure (M7a-e): a commitments rollup Document written before
  an erasure still holds what it folded, and the catalog still holds the
  rows it indexed; erasure rebuilds the one and removes the other.

**Part 2, done 2026-10-08: the other chains, surveyed row by row.**

| Chain | What its rows say | What was done |
|---|---|---|
| `artifacts/` | a file's path, its title and summary, a comment's author name and text | those five fields are an object of the artifact's own scope (`artifact:<id>`), since a saved artifact outlives its conversation; a destroyed artifact is absent from every read |
| sandbox records (one chain per Agent) | a candidate's files, checks, receipts and details, each naming its session | the whole `record` is an object of its session's conversation; the row keeps its `kind`; an environment record names no session and stays whole |
| execution stream (one chain per session) | command output | the chain is its conversation's own: every frame sealed under the conversation's key (`RecordChain::of_conversation`), as the ledger's are |
| `intake/` | an item's title, link, key and detection evidence | an object of the item's source (`intake:<source>`), where its body already is |
| `runs/` | ids, a flow's name, a runtime-written skip reason | nothing: no content |
| cost, routing evidence, misread | ids, model names, counts, tool names | nothing: no content |
| security events | a runtime-written label and detail about a sign-in | nothing: about no conversation |
| `grants/` | a principal and the display name they were invited under | nothing here: it is about a person, not a conversation, and goes with person erasure (M7b) |
| `effects/` | a target address, a provider's receipt | the payload moved in part 1; the address stays, as doc 74 §4 lists it for the receipt |

- A record chain's reader takes its key from the chain's directory
  (`vak_session::keys::of`), so a chain is sealed or not by what it
  declares and no reader names a conversation.
- Office rooms and presentations are Documents, not chains; their content
  is a Document's and is erasure's to walk (M7a-e).
- Tests: `an_artifacts_rows_keep_no_content_and_go_with_its_key`,
  `a_conversations_own_chain_is_sealed_and_goes_with_it`.

**Part 3, done 2026-10-08: the contributor key.**
- A message a person other than the owner writes into a conversation (the
  one entry with `MessageMeta::author_id`) is stored with its `message`
  and `meta` as an object of `contributor:<session id>:<principal>`
  (`vak_session::objects::contributor_scope`); the frame, still sealed
  under the conversation's key, holds the reference.
- Every decode of a stored entry goes through `SessionLog::decode`
  (opening a ledger, `scan`, `read_record_at`, the catalog's ingest). When
  the contributor's key is destroyed it yields a user message with the
  fixed text `REMOVED_TEXT`, the author's id and `MessageMeta::removed`,
  so `derive_messages()` and the transcript keep the turn's shape and the
  model and the people in the conversation see that something was removed.
  No ledger byte changes and every other entry reads as before.
- A comment on an artifact is its author's too: its `author_name` and
  `text` are an object of `contributor:<artifact id>:<author>`
  (`artifacts::comment_scope`), and a read drops a comment whose author's
  key is gone.
- This differs from doc 73 §7.3's wording (a guest's frames under their
  own key): a frame has one key per segment reader, so the contribution is
  an object of the contributor's scope inside a conversation-keyed frame.
  What erasure gets is the same: one key destroyed, the frames in place,
  the chain verifying, a placeholder shown.
- Tests: `guest_erasure_keeps_owner_conversation`,
  `a_guests_comments_go_with_their_key_and_the_artifact_stays`.
- Left for erasure (M7a-e): find a person's contributor scopes across
  conversations and artifacts, destroy a conversation's or an artifact's
  contributor scopes with it, and rebuild the rollups and catalog rows
  that folded the text before.

**Part 4, done 2026-10-08: the provider-account scope.** Decided with the
maintainer: what is kept under an account's key is what the provider
returned, not what the Agent then wrote from it.
- The `mail_calendar` tool says which connected account a result came
  from (`ToolOutput::from_account`); the loop records it as
  `CallEffect::Account` and tells the ledger before the result is
  appended (`SessionLog::result_from_account`).
- The result's block in the tool-result message keeps its call id and
  holds what the provider returned as an object of `account:<account id>`
  (`vak_session::objects::account_scope`). Its whole body, when one is
  kept beside the window, is an object of the same scope, named on the
  `EvidenceBody` row.
- `SessionLog::decode` puts each such block back, or the fixed line
  `ACCOUNT_REMOVED_TEXT` when the account's key was destroyed, so a
  request still pairs every tool call with a result. The body then reads
  as absent, including one this process had already read. Other results
  in the same message and every other entry are untouched, and no ledger
  byte changes.
- The limit, which erasure's receipt must state (M7a-f): what the Agent
  wrote from that data (its answer, a card, the turn's summary, a later
  turn that repeats it) is the conversation's own content and stays until
  the conversation is erased. The vault's drafts and the routine backlog
  are not conversation content and already go on disconnect.
- Test: `provider_account_erasure_preserves_unrelated_conversation_content`.

M7a-b is done with this part.

**M7a-c, done 2026-10-08: the reconciler, observe-only.**
- `crates/vak-lifecycle` decides and has no vak dependency: the default
  tenant label (doc 74 §3.1's table as `Label::default_tenant`), and
  `plan(items, observed, label, now)`, which is deterministic. An action's
  key names one transition of one item (`remove:<id>`, `trash:<id>`), so
  committing a plan twice is committing it once; actions are ordered
  rebuildable classes first and records last (doc 74 §5).
- `vak_core::lifecycle` observes and writes nothing. Three classes are
  observed: a run's environment (aged from when its run settled, live
  while it runs, an orphan aged from its last write), a service's rotated
  log files (the file being written is live), and draft versions (kept
  while the artifact is starred or shared). `Core::data_usage` measures
  every path `state::REGISTRY` declares, each file once.
- A class the label has a rule for and no observer looks at is listed in
  `Plan::unobserved`, never reported as nothing due: checkpoints, the
  trash (its sidecar records no time until M7a-e), Document history, inbox
  entries, allowlist entries, runs, activity segments and incidents.
  Executions have no rule yet: an unreviewed draft's files live in its
  execution directory, so scrubbing one waits for M7a-d to settle where a
  draft's bytes are kept.
- Surfaces: `GET /data/status`, `/data/usage` and `/data/lifecycle/plan`
  (owner only); `vak data status | usage | plan [--json]`; the admin
  console's Operate › Data, with Retention (the plan, what is kept back and
  why, the label's rules and which are watched) and Storage (measured
  usage by root, class and owner).
- Not built here: a background tick. With nothing to commit there is
  nothing for a loop to do; the plan is computed when asked. The tick,
  `POST /data/lifecycle/tick` and lifecycle records come with the first
  class that commits (M7a-d).
- Tests: `reconciler_is_idempotent` (vak-lifecycle),
  `reconciler_observe_only_commits_nothing` (vak-core: the file tree of
  the data home, logs and runtime is identical before and after),
  `data_routes_report_usage_and_the_dry_run_plan_to_the_owner_only`.

**M7a-d, in parts; part 1 done 2026-10-08: the committing pass.**
- `[lifecycle] mode` is `observe` by default and `commit` when set; it is
  privileged, so a project cannot turn another install's retention on.
  The default becomes `commit` at M7a-i, after the soak and the
  acceptance run; until then an install that has not asked keeps
  everything, and the dev machine's real data home is never acted on.
- `Core::lifecycle_tick(commit)` observes, plans and, when committing,
  carries out the due actions of `lifecycle::COMMITTED`: a settled run's
  environment and a rotated log, each removed in place with no grace.
  Every transition is a row of the `lifecycle/` chain, written `started`
  before the item is touched and `committed` or `failed` after, with ids,
  sizes and an `error_kind`, never content. A fenced process commits
  nothing; an item already gone is recorded committed.
- The server runs a pass at start and every ten minutes when the mode is
  `commit`. `POST /data/lifecycle/tick` runs one now and
  `GET /data/lifecycle/transitions` lists what was done; a request cannot
  ask for more than the install's mode allows. `vak data gc [--dry-run]`
  does the same from the CLI. The admin console's Data › Retention says
  which mode applies, has Run now, and lists what retention removed.
- Tests: `a_committing_pass_removes_what_was_due_and_records_it`; the
  data routes test covers the tick route observing.
- Live, on a throwaway `VAK_HOME` in commit mode (2026-10-08): a seeded
  stale environment and an old rotated log were planned, removed by
  `POST /data/lifecycle/tick` (17 bytes) and listed on the Retention
  screen; a fresh environment and the log being written stayed.
- Not in this part: the lifecycle chain is not yet a catalog source (it
  becomes one with "Why is this gone?" at M7a-e).

**Part 2, done 2026-10-08: executions and checkpoints; the last
scattered limit goes.**
- A rule can keep so many of a group: `Rule::keep_newest` keeps a group's
  first item and its newest N by `Item::rank`, and the rest are due at
  once (`Reason::Count`). The default label's checkpoint rule is "the
  first and the newest 20 of a session, and all of them 30 days after the
  session's last one" (doc 74 §2.9).
- `MAX_STORED_CHECKPOINTS` and the pruning inside `checkpoints::store` are
  deleted. `checkpoints::remove` releases a removed checkpoint's contents
  unless another of the session's names them, as the pruning did, and the
  reconciler is its only caller. Of the scattered retention the plan
  lists, this was the last still in the tree: finops and alerts
  compaction, `cleanup_artifacts` and `/memory/cleanup` were already gone.
- An execution's directory in the runtime root is due a week after the
  last write anywhere in it, and is removed whole. Settled where an
  unreviewed draft is kept: a draft put up for Review is frozen as a
  candidate under the Agent home (`sandbox/candidates`), outside the
  execution, so Review, narrowing and promotion do not need the execution
  directory; what goes with it is temp files and scratch that was never
  put up for Review. An Agent's tool cache beside its executions is not an
  execution and is left for quota eviction (part 4).
- Both classes are in `lifecycle::OBSERVED` and `COMMITTED`.
- Tests: `settled_execution_leaves_nothing` (vak-core, which also checks
  the checkpoint count), `a_group_keeps_its_first_and_its_newest_and_all_go_when_it_is_old`
  (vak-lifecycle), `removing_a_checkpoint_releases_only_what_no_other_names`.
- With retention observing by default (part 1), nothing caps checkpoints
  on an install that has not set `[lifecycle] mode = "commit"` until the
  default changes at M7a-i.

**Part 3, decided with the maintainer and done 2026-10-08: rows leave a
chain a sealed segment at a time.**
- `SegmentSet::drop_sealed` removes a sealed segment's file and keeps its
  seal entry, so the next segment still chains from its head and every
  later segment verifies. It refuses an open segment and a chain's newest
  sealed one, which numbers the next. `RecordChain::segments`,
  `seal_open` and `drop_segment` are what the reconciler uses.
- A chain whose rows are all of one class is observed one item per sealed
  segment, aged from its newest row, so it is due only when every row in
  it is: each Agent's inbox (inbox entries, 90 days) and its cost, routing
  and intent evidence (activity, 400 days), and the Operations Center's
  incidents and action receipts (400 days). A segment holding a row with
  no time is never planned away.
- A quiet chain would wait months to fill a segment, so a committing pass
  first seals any open segment whose first row is 30 days old
  (`SEAL_AFTER_DAYS`). A row can therefore outlive its rule by up to that
  long; no row leaves early.
- Run records are kept: those of no conversation share segments with the
  rest, and a record is ids and an outcome. The label's 180-day rule and
  the `Run` class are removed until M7b's audit rules. Allowlist entries
  are a settings file, not a chain, and move to M7b with its rules about
  people; that class is removed from the label too.
- Not done here: a dropped inbox row's content object keeps its grant
  until its conversation is erased, because the same content may be held
  by a row that stays. A rollup that folded a dropped row keeps what it
  folded (an aggregate, not the row).
- Test: `expired_rows_leave_a_chain_by_whole_segments_and_it_still_verifies`.

**Part 3b, done 2026-10-08: Document history.**
- A version record now says when it was saved (`at`) and which Document
  it is of (`doc`); both lines are additive, and a record without them
  reads as before with its age unknown.
- Naming the Document fixed a fault the first test of pruning found: two
  Documents created in the same second with the same content had the same
  version record, one object with one grant, so releasing it for one
  Document took it from the other.
- `Documents::prune(name, cutoff)` unlinks every version saved at or
  before the cutoff and everything older, never the current one. It
  releases version records only: a content object may still be another
  Document's, so what nothing names is left for part 4's collection.
  `history` ends where a version was pruned.
- The reconciler observes one item per Document that has earlier versions,
  by a digest of its name (a name is a path, and paths are content), aged
  from its oldest timed version, and prunes at the label's 90 days.
  Rollup Documents, which save a version at every fold, are covered like
  any other.
- Tests: `pruning_unlinks_old_versions_and_keeps_the_current_and_shared_content`
  (vak-storage), `a_documents_history_is_observed_and_pruned_and_its_current_version_stays`.
- Found and not fixed here: `Documents::forget` releases content grants
  one by one, so forgetting a Document whose text another Document in the
  same scope also holds makes that other one unreadable. It belongs with
  part 4, where release becomes collection by reachability.

**Part 4, done 2026-10-08: collection by reachability.**
- `Documents::collect(min_age)` releases, in one scope, the grant on
  every object no live Document of that scope names (its current version,
  the history behind it, their content). `vak_session::documents::collect`
  runs it for the `tenant` scope and each `agent:<id>` scope, which hold
  Documents and nothing else, then deletes the objects no scope holds
  (`Store::gc`). A committing pass ends with it.
- It fails closed: a Document that cannot be walked ends the pass for its
  scope with nothing released, because one that could not be read must
  never count as naming nothing.
- A grant younger than an hour is spared (`COLLECT_GRACE`): a save writes
  its objects before it moves its Document's ref, and one in flight names
  nothing yet. Putting an object a scope already holds restarts its
  grant's age, so content a save reuses is spared too.
- `Documents::forget` no longer releases content another live Document of
  its scope still names (the fault found in part 3b).
- Other scopes are not swept: a conversation's, an artifact's, a source's
  and an account's objects go when their grant is released by what owns
  them (a removed checkpoint) or when their scope's key is destroyed
  (erasure, M7a-e), after which `Store::gc` deletes them.
- Tests: `gc_keeps_everything_reachable` (vak-core), and the storage
  pruning test now covers collection and a forgotten twin.

**Part 5, decided with the maintainer and done 2026-10-08: the install
quota.** One limit for the whole install, none unless a person sets one,
measured by the retention pass.
- `[lifecycle] quota_gb` (privileged, like `mode`) is the most the install
  may store. Unset is no limit; usage is measured and shown either way.
- Every pass (and the first check of a process) measures the registry's
  paths and keeps two figures: what must be kept (records, objects,
  Documents, settings) and what can be rebuilt (derived, ephemeral,
  telemetry). The check before a turn reads those figures and walks
  nothing, so the install can overshoot by what ten minutes of work
  writes. The server now runs a pass every ten minutes in either mode; an
  observing pass removes nothing and still measures.
- Past four fifths of the limit the state is `soft` (a warning on the
  status and the Storage screen). At the limit, counting only what must
  be kept, it is `hard`: `Core::refuse_over_quota` refuses the next turn
  with `CoreError::OverQuota`, which says how much is kept, the limit and
  that nothing was removed. A committing pass at the limit removes each
  Agent's tool cache, which the next command rebuilds; no record, object
  or Document is ever removed to make room.
- `vak data status` and the admin console's Data › Storage show the limit
  and where the install stands.
- Not built: the inbox notification at the soft limit (the warning is on
  the two surfaces above only), and limits per space or per Agent (M7b).
- Test: `quota_refuses_admission_not_records`.

M7a-d is done with this part.

**M7a-e, in parts; part 1 done 2026-10-08: the trash is lifecycle
state.**
- `vak_session::conversation_state` keeps one ref per conversation,
  `lc/ses/<session id>`, moved by CAS under the writer epoch: whether it
  is archived, when it went into the trash, and when it was erased. An
  erased conversation's state never changes again.
- `archive.json` and `deleted.json`, their path accessors and their
  registry entries are deleted. `vak_core::trash` keeps the questions its
  readers ask (`trashed`, `is_trashed`, `search_exclusions`, `set`) and
  gains `archived`, `set_archived` and `states`; the server's archive
  handlers use it and write no file.
- Trashing records the time once; trashing again does not restart the
  window, and a restored conversation is archived again, as before.
- The reconciler observes the trash: one item per trashed conversation,
  aged from when it went in, due at the label's 30 days. With it every
  class that has a rule has an observer (`Plan::unobserved` is empty).
  Nothing carries that action out yet: it is erasure, part 2.
- Test: `the_trash_window_starts_once_and_the_archive_survives_a_restore`.

**Part 2, done 2026-10-08: erasing a conversation.**
- `Core::erasure_preview` gathers what an erasure would reach and gives
  it a digest; `Core::erase_conversation` acts only on a conversation in
  the trash, only when nothing in reach is on hold, and, for a person's
  request, only when the digest they confirmed still matches. The end of
  the trash window calls the same function with cause `policy`: the trash
  class is now in `lifecycle::COMMITTED`, and a held conversation is kept
  back (`Guard::Held`).
- Reach (the lineage walk): the conversation and every worker session it
  caused, by the catalog's `caused_by` edges; each one's conversation key
  and its contributor keys; drafts made there that nobody accepted,
  saved, starred or shared, with their artifact and comment keys; memory
  notes written from those conversations; effects already sent.
- What it does, in order: forgets the memory notes; removes the
  conversations' and artifacts' rows, text, edges and turn history from
  the catalog with `secure_delete` on, then rewrites the database and
  empties its write-ahead log (`Catalog::erase`); destroys the keys; marks
  each conversation erased, which is final; forgets the artifact and
  commitment rollups so they fold again without the erased rows; deletes
  objects no scope holds. No ledger byte changes.
- The receipt (`erasure::Receipt`, a row of the `erasures/` chain) holds
  ids and counts, never content: what was destroyed, a digest of the key
  scopes, what was sent outside and stays sent, and a fixed list of what
  the erasure did not reach. It is signed with the tenant's Ed25519 key
  (`VaultKeyAuthority::signing_key`, in the key vault) over the receipt
  including its public key, and `Receipt::verifies` checks it with that
  key alone.
- `Core::hold_conversation` sets or releases a hold on a conversation's
  key scope (`ScopeKeys::hold`).
- Not reached, and said so in every receipt: copies held by the AI
  services, what was already delivered, older backups, entities and
  skills derived from the conversation (not examined), and what an Agent
  wrote elsewhere from what it learned. Memory notes' stored bytes are
  deleted when their last grant goes; a shared-chain row of the
  conversation stays as ids with unreadable content.
- Tests: `erasure_follows_lineage` (which also checks the ledger's bytes
  are unchanged, the receipt and its signature) and
  `stale_preview_cannot_authorise` (which also checks the hold).

**Part 3, done 2026-10-08: the routes and the CLI.**
- `GET /conversations/{id}/erasure` returns the preview and what the
  person must type: the conversation's title, or the start of its id when
  it has none. `POST` erases with the digest and the typed title;
  `PUT /conversations/{id}/hold` sets or releases a hold;
  `GET /data/erasure/receipts` lists the receipts. All are the owner's
  and are checked against this workspace like the trash routes
  (invariant 22). A refusal carries a `reason` a client can act on
  (`not_in_trash`, `stale_preview`, `held`, `confirmation`,
  `already_erased`).
- `vak data erase <id>` shows the preview and asks for the title;
  `vak data hold <id> [--release]`; `vak data receipts` lists them and
  says whether each signature verifies.
- Test: `a_conversation_is_erased_from_the_trash_with_its_title_typed`,
  through the real router with its bearer check.

**Part 4, done 2026-10-08: drafts fade, and `/workspaces/forget` is
gone.**
- A draft version (nobody accepted or saved it, and its artifact is not
  starred or shared) goes to the trash 60 days after it was made or last
  restored: an `ArtifactStep::Trashed` row, so `Version::trashed_at`. In
  the trash it is whole and can be read. `DraftVersion` is now in
  `lifecycle::COMMITTED`.
- A version in the trash, or erased, is not a head: the version it was
  made from is current again. An artifact whose every version is there
  leaves `GET /library`; `GET /library/trash` lists every draft in the
  trash with the day it is erased.
- `PUT /library/{id}/versions/{version}/trash` with `{on}` is a person
  moving a draft there or restoring it. Accepting or saving a version
  takes it out; an accepted or saved version is refused, and so is an
  erased one. A restored draft's age starts again from the restore.
- After 30 days there `Core::erase_draft` erases it, as the trash's
  observer lists it (`draft/<artifact>/<version>`): an
  `ArtifactStep::Erased` row, then the version's bytes are released
  unless another version of the artifact holds the same ones, and
  collection deletes them. When that leaves the artifact with no version
  and nobody starred or shared it, its keys are destroyed and its search
  rows removed. A hold on the artifact's key keeps the draft back. Each
  erasure writes a signed receipt with scope `draft`.
- A guest sees no version that is in the trash or erased.
- `POST /workspaces/forget`, the client and desktop calls behind it and
  the path-keyed `spaces::forget` are deleted; no screen called them. A
  space is hidden by its id from the admin console's Projects
  (`spaces::set_forgotten`), which is unchanged.
- `Core::lifecycle_tick_at` and `lifecycle_plan_at` take the time to
  plan for, so a test can age what has no file to backdate.
- Not done here: a person erasing a draft at once (only the end of the
  trash window does); a row that names an erased draft keeps its path
  and title, sealed under the artifact's key, until the whole artifact
  goes; comments on an erased version stay while the artifact does.
- Tests: `an_old_draft_goes_to_the_trash_and_is_erased_when_its_time_there_ends`
  (which also checks the hold, the shared bytes and the receipts) and
  `a_draft_is_trashed_and_restored_and_a_saved_version_is_not`, through
  the real router.

**Part 5a, done 2026-10-08: the client's Trash.**
- Settings › Privacy and safety › Archived tasks lists the trash with
  the days each conversation has left (or that it is on hold), Restore,
  and Delete for good: a sheet that shows what the erasure reaches and
  cannot reach and takes the conversation's title typed
  (`EraseConversationSheet`). Drafts in the trash are listed below with
  Restore.
- `GET /sessions?trash=true` rows carry `trashed_at`, `erase_on` and
  `held`, and no longer list an erased conversation.
  `vak_core::lifecycle::trash_window` is the one source of the 30 days.
- `GET /conversations/{id}/gone` answers "Why is this gone?" for an
  erased conversation: when, by a person or by policy, and its receipt
  with whether the signature verifies. It ties the conversation to this
  workspace by its ledger, because erasure removed its catalog node. No
  screen calls it yet.
- Found live and fixed: the text to type was the start of the
  conversation's id, because the catalog's session node holds no title.
  `Core::erasure_confirmation` is now the one source for the app and
  `vak data erase`: the first 40 characters of the first thing the
  person said, which is the title the lists show.
- Checked in the browser at 1440 × 900 (dark) and 390 × 844 (light) on a
  throwaway data home with the local model: the trash list, the sheet,
  an erasure and its receipt. Drafts in the trash were not seen in the
  browser: nothing in that home made an artifact.
- Test: `a_conversation_is_erased_from_the_trash_with_its_title_typed`
  now also checks the trash row, the title and `gone`.

**Part 5b, done 2026-10-08: the rest of the screens, and the catalog.
M7a-e is done.**
- The catalog reads the `lifecycle/` and `erasures/` chains
  (`Source::Lifecycle`, `Source::Erasures`). A transition is a
  `transition` node whose status is its last row; a receipt is an
  `erasure` node. Where an erased conversation was (or an artifact, when
  a draft's erasure took the whole of it) there is an `erased` tombstone
  with no title, text or location, traced to its receipt by `caused_by`,
  so `lineage` of an erased conversation ends at the receipt. A rebuild
  from the records gives the same.
- Client, the conversation's More menu (C1): Archive conversation and
  Move to trash. The Agent's next conversation opens in its place. The
  client had no way to archive or trash a conversation before this.
- Client, "Why is this gone?" (C3): a link to an erased conversation
  says when and why it was deleted and that a signed record is kept,
  then opens the Agent's current conversation. A notice now stays long
  enough to read its length.
- Client, draft expiry (C8): the Library's version list says when a
  draft nobody keeps goes to the trash (`draft_until` on
  `GET /library/{id}`, from `lifecycle::draft_window`), with Move to
  trash and, for one in the trash, Restore.
- Admin, Conversations (A2): a Trash panel (since when, erased on, On
  hold; Restore, Hold, Erase with the title typed) and the erasure
  receipts with whether each signature verifies
  (`GET /data/erasure/receipts` now says so).
- Admin, conversation detail (A3): a Lifecycle tab
  (`GET /conversations/{id}/lifecycle`): its state, how long it is kept,
  the rule, and its hold.
- Not built: Export and "Ask an admin" in the client menu (one owner, no
  second person to ask); a label badge and the label's inheritance chain
  (labels are M7b); a search hit on an erased conversation (search
  returns none: its rows are removed); execution states in the Workbench.
- Checked in the browser on a throwaway data home with the local model,
  at 1440 × 900: the admin Trash (hold, erase, receipt), the Lifecycle
  tab, the More menu's Move to trash, and the dead link's notice. Draft
  expiry in the Library was not seen in the browser: nothing in that
  home made an artifact.
- Tests: `erasure_follows_lineage` (tombstone, lineage, rebuild),
  `an_old_draft_goes_to_the_trash_and_is_erased_when_its_time_there_ends`
  (catalog rows), `a_conversation_is_erased_from_the_trash_with_its_title_typed`
  (lifecycle route, verified receipts),
  `a_draft_is_trashed_and_restored_and_a_saved_version_is_not`
  (`draft_until`). `gc_keeps_everything_reachable`,
  `quota_refuses_admission_not_records`.

**M7a-f, done 2026-10-08: erasing a guest's contributions and a
connected account's data.** Vak has one owner; a guest is someone who
joined through a link the owner shared. Both erasures are the owner's.
- `Core::erase_guest(session, principal, …)` destroys the keys that hold
  what one guest wrote in a conversation and in comments on the files
  made there (`contributor:<session>:<principal>`,
  `contributor:<artifact>:<principal>`). Each message then reads as
  removed and each comment is gone; the conversation, the owner's and
  the other guests' messages stay, and no ledger byte changes. It is
  refused while the conversation or the key is on hold, and when a
  confirmation's digest no longer matches the preview
  (`Core::guest_erasure_preview`). `Core::conversation_guests` lists who
  has such a key.
- `Core::erase_account(account, …)` destroys `account:<id>`: what that
  mail or calendar account returned reads as a fixed line in every
  conversation of every Agent that read it. It is refused while any
  Agent still has the account connected (`ErasureError::StillConnected`),
  because a result fetched afterwards could not be stored, and while the
  key is held. An account nothing was kept for is not found.
- Search held what those keys protected (a turn's text, a result's
  digest), and nothing says which conversations read an account, so both
  end with `Catalog::rebuild_after_erasure`: every row is deleted with
  `secure_delete` on and replayed from the records, which now read as
  removed, then the database is rewritten and its log emptied. This costs
  a full rebuild per erasure, which is rare.
- Erasures take turns within a process (`erasure::ERASING`): one that
  worked out its reach from the catalog while another was rebuilding it
  reached less than it should, which the full suite caught once in five
  runs. Two processes are not ordered against each other, and a search
  made during a rebuild can come back short until it finishes.
- Each writes a signed receipt (scope `guest` or `account`) that says
  what was not reached. For an account: what an Agent wrote from its data
  stays until that conversation is erased.
- `GET /conversations/{id}/guests`,
  `GET|POST /conversations/{id}/guests/{principal}/erasure` (the digest),
  `POST /data/erasure/accounts/{account}`;
  `vak data erase <id> --scope guest --guest <principal>` and
  `vak data erase <account> --scope account`, each asking for the word
  `erase`.
- Screens: a disconnected account in the client's settings has Delete
  saved copies, and the disconnect wording no longer says copies cannot
  be erased; the admin Lifecycle tab lists the guests who wrote in a
  conversation with Erase what they wrote.
- Not done: a guest's grant is not ended by erasing what they wrote;
  memory notes and summaries an Agent wrote from a guest's words are not
  examined; the vault's private routine run history still goes only on
  disconnect. Neither screen was seen in a browser: a throwaway home has
  no connected account and no guest.
- Tests: `guest_erasure_keeps_owner_conversation` (through `Core`, with
  search and the receipt), `provider_account_erasure_spans_agents_and_conversations`,
  `provider_account_erasure_preserves_unrelated_conversation_content`
  (M7a-b's), and `a_guests_contributions_are_erased_and_the_conversation_stays`
  through the real router.

**M7a-g, done 2026-10-08: revoking an Agent.**
- `AgentLifecycle::Revoked`: for when an Agent may have been
  compromised. `POST /agents/{agent}/revoke` takes the Agent's name
  typed and, in this order, writes the lifecycle (so no turn is admitted
  from then on), cancels its live runs, removes the token of every bot
  bound to it, disconnects its mail and calendar accounts
  (`mail_calendar::disconnect`, the function the owner's Disconnect now
  calls too) and removes every secret in its private scope. All of it is
  done before the request answers. What it made is kept.
- It is final: Resume answers 409, and saving Agent definitions neither
  revokes an Agent nor brings a revoked one back (`PUT /config/agents`
  refuses a change either way).
- A token set in the server's own environment is not Vak's to remove;
  the answer names what was left (`not_removed`), and the Agent is
  refused regardless.
- `GET /agents/{agent}/lifecycle` gives its state and counts of what it
  holds that reaches outside (bots with a token, connected accounts,
  private secrets, automations switched on, open conversations): never
  a secret or a name.
- Screen (A14): the client's agent picker has Revoke on each saved
  Agent, a sheet that says what is removed and what is kept and takes
  the name typed, a Revoked badge and a Revoked filter. The admin
  console has no Agents screen, so the panel is in the client.
- Not done: its automations are not switched off; each slot is refused
  at admission and recorded as a failed run, as for a paused Agent.
  Archiving does not start retention clocks (labels are M7b). Erasing an
  Agent is M7b.
- Checked in the browser on a throwaway data home: the sheet with the
  real counts, the revoke, the bot token gone and Resume refused. After
  that check the sheet stopped listing zero counts and the picker now
  refreshes the sidebar; those two changes were not seen again in the
  browser.
- Test: `revoke_cuts_endpoints_within_one_tick`.

**M7a-h, done 2026-10-08: backup and restore.**
- A backup is the data home as it is stored: records and objects, which
  are encrypted, and the wrapped scope keys with their tombstones.
  `Core::backup_create` writes manifest version 2, which adds the tenant
  store's writer epoch and ref generation, how many scopes had a key and
  how many were destroyed, and the erasure watermark (the id of every
  receipt the backup holds).
- The hazard this closes, shown by the tests: a backup taken before an
  erasure holds the destroyed scope's wrapped key, and a restore put that
  key file back.
- `Core::backup_restore` copies as before (nothing here is overwritten or
  deleted), then, before it ends: destroys again the key of every scope
  that has a tombstone (`ScopeKeys::reshred`,
  `TenantObjects::reapply_destroyed_keys`), marks each erased
  conversation erased, forgets the rollups, rebuilds search from the
  records (`Catalog::rebuild_after_erasure`), deletes what no key holds,
  and last moves the store's writer epoch (`Store::restore`). Every
  process that had the store open is then fenced, the restoring one
  included: it must be started again, and `/health` says `fenced`.
- `Core::restore_preview` reads a backup's manifest and names the
  erasures recorded here since it was taken. A folder with no manifest
  is not a backup.
- `POST /data/backups`, `/data/backups/restore/preview` and
  `/data/backups/restore` replace `/backup/export` and `/backup/import`.
  `vak backup export|import` keeps its name and now does the same; a
  `vak data backup` beside it would have been a second way to do one
  thing (invariant 30).
- Screen (A16): the client's Settings backup card asks before restoring,
  says how many things deleted since the backup stay deleted, and says
  Vakyartha must be started again.
- Not built, against the plan's list for the manifest: record heads, an
  object inventory, per-key grants and a Desired revision (Desired state
  is not versioned). The copy is not a snapshot: a backup taken while a
  turn is writing can hold a torn tail, which the readers already
  recover from. An erasure made after a backup cannot be applied when
  that backup is restored into a different, empty home: nothing there
  records it. Merging a backup into a home that has moved on is M9's.
  No list of past backups, no expiry and no typed confirmation: a
  restore overwrites nothing. The restore confirmation was not seen in a
  browser.
- Tests: `restore_reapplies_erasures`,
  `provider_account_erasure_reapplies_after_restore`, and
  `backup_roundtrip_reapplies_erasures_and_rejects_self_backup` through
  the real router. Each has a test binary of its own, because a restore
  fences the process.

**M7a-i, done 2026-10-08: integrity, the soak, acceptance, and retention
on by default. M7a is done.**
- `Core::data_integrity` verifies every conversation ledger and record
  chain from its stored bytes with no key (`vak_session::verify`: each
  segment's hash chain, chaining from the sealed head before it), so an
  erased conversation verifies too and one changed byte is found and
  named by its place. It also counts keys in use, destroyed and held,
  checks every receipt's signature, and says whether search is behind.
  It reads every record once and changes nothing: `GET /data/integrity`,
  `vak data verify`, the admin console's Operate › Data › Integrity
  (A9, with Rebuild search), and Home › Data health (A1, links and
  `/data/status`'s numbers).
- `vak data rebuild-catalog`, `vak data cat <conversation>` (a plain
  transcript, refused for one in the trash) and `vak data grep <text>`
  (search). `vak export` already writes a conversation out, so there is
  no `vak data export` beside it (invariant 30).
- The soak, `thirty_day_soak_stays_within_budget`: thirty simulated days
  each leave an environment, an execution and a rotated log, with a
  committing pass each day. What is left is what the rules keep (at most
  the rule's days plus one of each), every removal is a recorded
  transition, a second pass does nothing, the conversation made on day
  one reads back, and the home verifies.
- **Retention acts by default.** `[lifecycle] mode` is `commit` unless an
  install sets `observe`; an unknown word still means observe. The server
  runs the pass every ten minutes. This is the first build that removes
  things on an install that asked for nothing, the dev machine's real
  data home included.
- AGENTS.md: invariant 42 (deletion), invariant 19 names `vak data
  verify` and `rebuild-catalog`, and the Pending section's rules.
- The acceptance run: `docs/audits/acceptance-m7a-lifecycle-2026-10-08.md`.
  Guest erasure, draft expiry, Delete saved copies and the restore
  confirmation have test evidence only.
- Not built: sampled and scheduled verification (it runs when asked),
  object-by-object verification of the store, undeclared-path incidents
  on the Integrity screen, the per-screen "no value without an API
  source" test, and docs 23 and 46 Part VII from §5's list.
- Tests: `a_changed_byte_is_found_and_an_erased_conversation_still_verifies`,
  `thirty_day_soak_stays_within_budget`.

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

#### M7b design (agreed with the maintainer 2026-10-09)

Vak has one owner. M7b is cut to what one owner uses; the rest of the
list above is deferred with the enterprise work and is not built.

Decisions:

1. **Trimmed for one owner.** No data roles, no second-person approval of
   an erasure, no erasure-request queue, no audit export. Every action is
   the owner's.
2. **One set of retention rules for the install.** The owner edits the
   keep times in one place, with a preview of what a shorter time would
   remove and a confirmation when anything is shortened. No labels on
   Agents, projects or conversations, and no inheritance.
3. **Four more things can be erased for good:** an Agent's data, a
   project's data, one person across chats and Agents (with their
   allowlist entry), and everything (the whole install). Each is the
   owner's act, previewed, confirmed by typing a name, refused under a
   hold, and receipted like M7a's.
4. **Retention stays on by default** (M7a-i).

Steps, each shipped whole and in this order:

| Step | What | Exit tests |
|---|---|---|
| M7b-a | The install's retention rules as a Document the owner edits: `Core::retention_label`, the impact preview, confirmation when a time is shortened; the admin Retention screen's editor; `vak data rules` | `shortened_retention_previews_what_it_removes`, `edited_rules_are_the_ones_the_pass_uses` |
| M7b-b | Holds listed in one place and placed on an artifact as well as a conversation; release | `hold_blocks_every_destructive_transition` |
| M7b-c | Erasing an Agent's data (after it is revoked or archived): its conversations, memory, files and automations | `agent_erasure_takes_what_it_owns_and_nothing_else` |
| M7b-d | Erasing a project's data: what Vak stored for one folder, never the folder | `project_erasure_leaves_the_folder` |
| M7b-e | Erasing one person across chats and Agents, with their allowlist entry and a hashed sticky-deny fingerprint | `person_erasure_spans_agents_and_chats` |
| M7b-f | Erasing everything, with a receipt the owner keeps | `install_erasure_leaves_a_receipt_and_nothing_else` |
| M7b-g | Key rotation and the Keys screen (A12); the client's Your data page (C7) | `rotation_keeps_everything_readable` |
| M7b-h | The acceptance run | the run |

Not built, by decision 1 and 2: labels on any node (`label_on_any_node_resolves`),
data roles, the Erasure requests queue (A11), Audit export (A15), a query
hold.

**M7b-a, done 2026-10-09: the install's retention rules.**
- The rules the reconciler runs under are `lifecycle::retention_label()`:
  the defaults, with the keep times the owner changed, which are kept as
  one Document (`SharedScope::retention_rules`), so every change is a
  version. `trash_window` and `draft_window` read it too, so the days a
  screen shows are the install's.
- `Core::retention_preview(keep_days)` says which kinds get a shorter
  time and what the next pass would remove that the current rules keep,
  with a digest. `Core::set_retention` saves; when anything is shortened
  it needs that digest, and is refused when what it would remove has
  changed since. Lengthening needs nothing. A kind left out goes back to
  its default; a keep time is 1 to 3,650 days; size limits and the
  checkpoint count are not editable.
- `GET|PUT /data/rules`, `POST /data/rules/preview`;
  `vak data rules [--set kind=days] [--reset]`, which asks for the word
  `shorten`; the admin console's Retention screen has a days field per
  kind, Save keep times and Back to the defaults, and a shorter time
  asks with what it would remove.
- Screen text that named 30 or 60 days now points to the day shown,
  since the owner can change them.
- Checked in the browser on a throwaway data home: the editor, a longer
  trash time saved. The shorter-time confirmation is the browser's own
  dialog and was not driven; the route test covers it.
- Tests: `shortened_retention_previews_what_it_removes`,
  `edited_rules_are_the_ones_the_pass_uses`,
  `the_owner_changes_a_keep_time_and_a_shorter_one_is_confirmed` (a
  binary of its own: the rules are one Document for the home).

**M7b-b, done 2026-10-09: holds in one place.**
- `Core::hold_artifact` holds a file's key, as `Core::hold_conversation`
  holds a conversation's. A held file's drafts do not go to the trash
  (`Guard::Held`), none of its versions is erased, and a conversation
  whose erasure would take it is refused.
- `Core::holds` lists everything on hold by what the owner knows it as
  (a conversation's title, a file's name): `GET /data/holds`, and an On
  hold panel with Release under the admin console's Conversations.
- A file is held from its page in the client's Library (Hold, Release
  hold; `POST /library/{id}/hold`), which then shows no day for its
  drafts to go by.
- Not built: a hold on an Agent, a project or a connected account, and a
  hold by search query. A hold has no reason or expiry recorded.
- Neither new control was seen in a browser.
- Tests: `hold_blocks_every_destructive_transition` (the owner's
  erasure, the end of the trash window, a guest's erasure, a draft's
  expiry and erasure, and a conversation that would take a held file;
  then each goes ahead once released), and the Library route test.

**M7b-c, done 2026-10-09: erasing an Agent's data.**
- `Core::erase_agent` erases everything a saved Agent holds, once it is
  archived or revoked (an active or paused one is refused,
  `ErasureError::AgentInUse`; the built-in Agent is never erased this
  way): every conversation in its home, by destroying each one's keys
  and its guests' keys, with no stop in the trash; its memory, entities
  and other Documents; the drafts it made that nobody accepted, saved,
  starred or shared; its automations; and the workspace Vakyartha keeps
  for it in the data home.
- Not taken, and said in the receipt: files of its work a person kept,
  anything it wrote into a folder the owner made, its run, cost and
  delivery records (ids, times and numbers), and what it already sent.
- It is previewed with counts (`Core::agent_erasure_preview`), refused
  while anything in its reach is on hold or when the preview is stale,
  ends with search rebuilt from the records, and leaves a signed receipt
  with scope `agent`. Its definition stays, archived or revoked.
- `GET|POST /agents/{agent}/erasure` (the digest and the Agent's name
  typed); `vak data erase <agent> --scope agent`; the client's agent
  picker has Delete everything it holds on an archived or revoked Agent.
- The sheet was not seen in a browser.
- Tests: `agent_erasure_takes_what_it_owns_and_nothing_else`, and the
  revoke route test, which now erases the revoked Agent.

**M7b-d, done 2026-10-09: erasing a project's data.**
- `Core::erase_project(space)` erases everything Vakyartha stored for
  one project: every Agent's conversations there (keys destroyed), what
  every Agent remembers of it (its space-keyed Documents), every copy of
  its files in the Library, kept or not, its automations, its Agents'
  workspaces in the data home and its executions' scratch files.
- The project's own folder, its files and its `.vak` settings are never
  touched. The project stays in the registry, hidden until it is opened
  again.
- Previewed with counts (`Core::project_erasure_preview`), refused under
  a hold or with a stale preview, and receipted with scope `project`.
  The server also refuses while work is running in the project.
- An Agent's and a project's erasure share `erase_reach`, and both now
  leave out a conversation that was already erased.
- `GET|POST /data/erasure/projects/{space}` (the digest and the
  project's name typed); `vak data erase <space> --scope project`; the
  admin console's Projects has Erase its data, which shows the counts
  and asks for the name.
- The Projects action was not seen in a browser.
- Tests: `project_erasure_leaves_the_folder`,
  `a_projects_data_is_erased_and_its_folder_is_left` through the real
  router.

**M7b-e, done 2026-10-09: erasing one person who wrote to a bot.** A
person here is someone outside who messages the owner's bots on
Telegram, Discord, Slack or a webhook, never another user of Vakyartha.
- What the records hold of a channel sender is the run each message
  caused, whose actor is their principal for that chat
  (`trace::local::channel_sender(surface, chat, sender)`). The message
  itself is the conversation's, under the conversation's key, so one
  sender's lines cannot be separated out of a group chat.
- `Core::erase_person(fingerprint, principals, …)` therefore sorts the
  conversations in which they caused a run (`Catalog::sessions_of_actor`,
  `run_actors`) into theirs alone, which are erased whole across every
  bot and Agent, and shared with other people, which are not touched.
  The owner, the system and the Agents do not count as other people.
  Anything they wrote as a guest elsewhere goes too. The preview and the
  receipt say how many shared conversations there were.
- The server makes one principal per chat the gateway knows on that
  surface (`GatewayState::chats_on`), drops the allowlist entry and the
  binding of each chat that was theirs alone (`forget_chat`), and keeps
  a fingerprint, the hash of the surface and their id
  (`gateway::person_fingerprint`, `gateway/erased-people.json`). A
  message from a fingerprinted sender is refused before any chat is
  looked up, even when the allowlist is open, and never comes back for
  review. The receipt's subject is the fingerprint, never the id.
- `POST /data/erasure/people/preview` and `POST /data/erasure/people`
  take the surface and the id in the body, never the address; the admin
  console's Conversations has Erase a person.
- Not built: a way to lift the refusal; erasing one sender's lines from
  a shared conversation (it needs channel messages kept per sender, as a
  guest's are); memory notes an Agent kept about them are not examined.
  The panel was not seen in a browser.
- Test: `person_erasure_spans_agents_and_chats`, through the gateway
  with real turns: two chats of her own erased, the group chat kept,
  her next message refused in an old chat and a new one, the other
  person untouched, and neither the receipt nor the kept list naming her.

**M7b-f, done 2026-10-09: erasing everything.**
- `Core::erase_install` destroys every scope key, removes every stored
  secret (`vak_config::credentials::forget_all`, which reads the presence
  index because the OS store cannot be listed: the tenant's keys, the
  receipt-signing key, provider keys and bot tokens), and empties the
  data, cache, log and runtime directories
  (`vak_core::state::remove_wholesale`, the one removal the purge uses
  too). Folders a person owns, the Vakyartha folder and its settings
  included, are never touched.
- All it leaves is the signed receipt, one file under the data home's
  `erased/` (`SharedScope::install_receipts`, declared in the registry).
  The receipt carries its public key, so it checks after the key that
  signed it is gone. A home that holds only that file starts as a first
  run, and `Core::erasure_receipts` lists it before the new install's own.
- It is previewed (`Core::install_erasure_preview`: conversations, files,
  keys, how much is on hold), refused while anything is on hold or the
  preview is stale, and confirmed by typing `erase everything`.
- `GET|POST /data/erasure/install`, refused while work is running. The
  server stops two seconds after it answers, because nothing it holds
  open exists any more; a service manager starts it again as new.
  `vak data erase everything --scope install`; the admin console's
  Operate › Data has Erase everything, which shows the receipt and
  offers it as a file.
- It differs from `vak self uninstall --purge`: the purge removes the
  install and leaves nothing; this keeps the install and leaves the
  receipt.
- Not reached, and said so in the receipt: a person's own folders, what
  was sent outside, what the AI services received, and backups made
  earlier (one that holds the keys can still be restored).
- Tests: `install_erasure_leaves_a_receipt_and_nothing_else` (3 of 3
  runs), `erasing_everything_is_previewed_and_needs_the_words` through
  the real router. Seen in a browser in a throwaway home: the panel, the
  erasure (20 files to the 1 receipt), the server stopping, and after a
  restart no conversations, the receipt listed and verifying.

**M7b-g, done 2026-10-09: key rotation, the Keys screen and Your data.**
- A rotation was only "start a new key": every stored key stayed under
  the old one. `TenantObjects::rotate_keys` now starts the new tenant key
  and wraps again every scope key (`ScopeKeys::rotate`, `rewrap`) and
  every object grant (`LocalObjectStore::rewrap`) under it, leaving a
  destroyed scope's as they are. `KeyAuthority::version` names the key in
  use, and a process that opened the vault before another one rotated
  reads the versions it lacks when it meets one.
- Earlier tenant keys stay in the credential store, so a backup made
  before a rotation still opens. Retiring them is not built.
- `Core::rotate_keys` records each rotation in the `key-rotations/`
  chain; `Core::key_status` says where the keys are kept
  (`vak_config::credentials::backend`: the keychain, or the encrypted
  file with what that means), the key in use, the oldest still
  protecting anything, the counts, and every rotation. No key material
  is returned anywhere.
- `GET /data/keys`, `POST /data/keys/rotate`; `vak data keys [--rotate]`;
  the admin console's Operate › Data › Keys (A12, under Data rather than
  a Security group, which the console does not have).
- The client's Settings › Your data (C7): how many conversations and
  Library files are kept and the space they use, the keep time of each
  kind a person meets, where the key is kept and Change the key, the way
  to Archive and trash and to Storage and backup, and Erase everything
  with the words typed, which shows the receipt and offers it as a file.
- Not built: escrow bundles, retiring an earlier key, a KMS authority,
  and "what Vak keeps about this person" per person (there is one
  owner).
- Tests: `rotation_keeps_everything_readable` (a conversation, a Library
  file, an erased conversation that stays erased, two rotations, the
  integrity check), `the_owner_sees_the_keys_and_rotates_them` through
  the real router. Seen in a browser in a throwaway home: the Keys
  screen and a rotation, the Your data page, Change the key, and Erase
  everything from it after two rotations (the receipt still signed).

**M7b-h, done 2026-10-09: the acceptance run**, recorded in
`docs/audits/acceptance-m7b-governance-2026-10-09.md`. With it M7b is
done. It found four defects, each fixed with a test:
- A conversation the server had open was still served after its Agent's,
  project's or sender's data was erased; those erasures now drop the
  handle of every hidden conversation.
- An open conversation kept showing a guest's words or an account's data
  after their erasure; the handles are dropped so each is read again, and
  both erasures are refused while a turn that could hold the data runs.
- Background work wrote files in the two seconds between erasing
  everything and the stop; the process is fenced when the erasure ends
  (`vak_session::fence::retire`) and clears again before it exits
  (`Core::sweep_erased_install`).
- A second erasure of everything removed the first one's receipt; the
  receipts directory is never cleared.

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

**M8 design (agreed 2026-10-06).** M8 carries doc 82's L1 to L5. None of
them was built before it, so `produces_artifact` and the derived drafts
projection still stand, and M8 replaces them rather than building beside
them (invariant 30). The maintainer chose:
- **Only declared deliverables enter the Library** (doc 82 §10 question
  1). A turn that writes files but declares none adds nothing; Workbench
  still shows those files, and a person's Save is the declaration.
- **`title` stays optional** on `write` and `office_apply` (doc 82 §10
  question 2). The Library falls back to the file name, and the eval suite
  measures how often small models omit it.
- **Versions are records.**

The shapes:
- **Artifact.** An `art_` id, owned by its Space, with its authoring Agent
  recorded.
- **Versions.** The `artifacts/` record chain in the shared scope holds
  `declared`, `versioned` (with the version's parent, so concurrent edits
  are siblings), `renamed`, `starred`, `archived` and `saved` rows. Each
  row carries the trace key of the run or the person behind it.
- **Bytes and current state.** A version's bytes are a tenant object.
  Current state (head version, siblings, title, star) is read through a
  rollup Document over the chain (`vak_session::rollup`, M6.3); nothing
  replays the chain to decide.
- **Declarations.** `Tool::artifact` replaces `produces_artifact`: a call
  that names its deliverable, forwarded by `BrokeredTool`.
- **Reviewed work.** A Review candidate and its promotion, an Office
  draft and a changeset are versions of the artifact they change.
- **Catalog.** The chain is a catalog source: an `artifact` node with its
  versions, lineage to the producing run and call, and `derived_from` for
  a version made from another.
- **Grants.** One `grants/` chain with a rollup: a principal, an object
  (an artifact or a conversation), a role (viewer, commenter, editor) and
  an inherit-or-break flag. Doc 69's coworking grants move into it. Search
  and every read filter by grant before ranking, so a revoked grant hides
  the object at once.

The steps:
1. **M8.1** identity and versions: the `artifacts/` chain and rollup,
   `Tool::artifact`, candidates, promotions and Office drafts as versions,
   catalog nodes, and the read-only `/library` API. Exit tests
   `concurrent_edit_creates_sibling_versions` and
   `saved_version_survives_origin_erasure`. Survival is shown by the saved
   version's own grant and object, since erasure itself is M7a.
2. **M8.2** grants: the `grants/` chain, coworking grants moved into it,
   inheritance and breaks, and catalog filtering. Exit tests
   `share_inherits_and_breaks` and `revoked_grant_hides_from_search`.
3. **M8.3** the client and admin Library, the artifact page, version
   history, and Continue working (doc 82 L2). Browser run.
4. **M8.4** edit and Put back, saved cards, the Share dialog, and
   `SharedConversation`, `ArtifactCanvas`, `OfficeRedline` and
   `WorkbenchPanel` on the Artifact API, plus the navigation
   reconciliation. The browser acceptance run covers create → review →
   promote → share → comment → revise; erase waits for M7a.

M8.1 (2026-10-06): `vak_core::artifacts` (the `artifacts/` chain, read
through `artifacts-rollup`; ids derived from space and path, so one file
is one artifact; a version names its parent, and the same bytes again are
no new version). `Tool::artifact` and `vak_tools::ArtifactClaim` replace
`produces_artifact`: every claim is completion evidence, and a declared
one (an `office_apply` draft, or a `write` with a `title`) reaches
`AgentConfig::artifacts`, Core's `CallSink`, which records the artifact
and a version under the run's key. A Review candidate of a declared
deliverable is a version of it, and its promotion is recorded on that
version (`vak-server` `library::note_review`). The catalog has `artifact`
nodes produced by their call and run, which `/search` and `session_search`
find. `/library`, `/library/{id}` and `/library/{id}/versions/{ver}` are
read-only. Exit tests `concurrent_edit_creates_sibling_versions` and
`saved_version_survives_origin_erasure`; `library_api.rs` covers the
endpoints. Live: a real model turn's titled `write` became an artifact,
traced artifact → call → run → turn → session, cause user.

M8.2 (2026-10-06): `vak_core::grants`, the `grants/` chain read through
`grants-rollup`. A grant gives a principal a role (viewer, commenter or
editor) on an artifact or a conversation. Breaking inheritance makes an
object's grants its only audience besides its owner; restoring it brings
the parent's audience back (`Grants::may`). A row that cannot be read
fails every grant decision closed. Doc 69's coworking invitations are
conversation grants in the same chain, and the per-Agent
`coworking/grants` store and `AgentScope::coworking_grants` are gone.
The catalog tails the chain into `grants` and `broken`. Its schema is
version 2, so an older catalog rebuilds. `Audience::principal` returns
only what an active, unexpired grant opens, and an Agent's read skips an
artifact whose inheritance is broken. Exit tests
`share_inherits_and_breaks` and `revoked_grant_hides_from_search`. The
server's coworking tests run on the chain.

M8.3 is split in two. M8.3a (2026-10-06): the client's Library, a
sidebar row under Search, is a full page like the Inbox. It shows cards
filtered by kind, change time, Starred and Archived, with a name search.
The artifact page shows a text preview for small text files, Download,
Open conversation, Star, Rename, Archive, and the versions, each with
who made it and when, side-by-side siblings, and Keep. The admin console
has a Library tab beside Memory, Sources and Search. Changes are records
credited to the person: `POST /library/{id}/{star|rename|archive}` and
`/library/{id}/versions/{ver}/save`. A browser run covered the real
model-made "Harbour poem" at 1440 × 900 and 390 × 844, in light and
dark. M8.3b is Continue working and Make another (doc 82 L2).

M8.3b (2026-10-06): a run request names Library artifacts
(`artifacts: [{id, mode}]`). The server writes each one's block from the
artifact's records at admission (`library::attach`), refusing another
Agent's or another workspace's. The message records it as a typed
`AttachedArtifact`, which the chat draws as a card in the block's place.
The block names the tool for the artifact's kind and carries no digest. A write of a file that is already an artifact is a new
version even without a `title`; a supporting file never becomes one. Make
another sets `AgentConfig::protected_paths`, and the loop refuses a
write, edit or `office_apply` to the source. `recall` with `conversation`
reaches only conversations an attached artifact names (doc 68).
The client's Continue working and Make another open the authoring Agent's
conversation with the artifact in the composer. Tests:
`make_another_keeps_its_source`,
`recall_reaches_only_conversations_attached_artifacts_name`,
`library::tests`, and the extended `saved_version_survives_origin_erasure`.
Live: Continue working on "Harbour poem" made version 2 from version 1.
The first attempt went wrong, and M8.4b found three causes in Vak, not
the model:
- `office_apply` answered a Markdown path with "pass the sha256 doc_read
  prints as base_digest", a digest `doc_read` never prints for text, so
  the call looped;
- `doc_read`'s `   1: ` line prefixes read as part of the text and were
  written back into the file;
- intent and the goal read the server-written artifact block as part of
  the request.

M8.4 is split into three steps: a (sharing), b (editing and Put back) and
c (the Artifact API moves and the acceptance run). Step c has six parts
of its own (its design is below).

M8.4a (2026-10-07): `POST /library/{id}/shares` makes an artifact grant
with a token, a role, an expiry and an optional `history_from` version,
and breaks the artifact's inheritance. `GET` lists the shares without
tokens, and `DELETE` revokes one at once. The auth middleware turns an
artifact share's token into `AuthenticatedPrincipal::ArtifactGuest`,
which reaches only `/shared/artifact`. That view shows the current
version, and earlier versions from the chosen one on, with no
conversation text or ids. A guest downloads a shown version, and a
commenter or editor comments on one. Comments are `Commented` rows in the
`artifacts/` chain, and the owner sees them and comments too. The
client's artifact page has Share (name, role, versions shown, expiry;
the code is shown once) and Comments; `?shared=artifact` is the guest
page. Test `library_sharing.rs`. Live run: a commenter share of "Harbour
poem" showed version 2 only, the guest's comment reached the owner, and
the revoked link answered 401.

M8.4b (2026-10-07): Download is recorded as a `Downloaded` row, and the
rollup keeps the last downloaded version. `POST /library/{id}/versions`
takes an edit (`text`) or a file put back (`data`) as a version credited
to the person, made from `parent`, the last download, or the head. Made
from an older version, it is a sibling of what changed since. Made from
the current one, it also becomes the workspace file, for a plain file of
this workspace with no symlink or `..` (documents change only through
Review), so the Agent builds on it. `POST /library/cards` keeps a chat
card: its payload is the first version of a `card` artifact, and that
version is kept. The client's artifact page has Edit for text files and
Put back for any, and a card's action row has Save to Library. Tests
`library_editing.rs` (edit, a late Put back becoming a sibling, a saved
card). Live: an edit of "Harbour poem" became version 3, credited to the
person, and the workspace file matches it; a table card saved from a
chat appears in the Library.

**M8.4c design (agreed 2026-10-08).** The maintainer chose:
- **A full move.** Anything that is an artifact (a declared deliverable, a
  Review candidate, an Office or PDF draft) is read, compared and reviewed
  by artifact id and version through `/library`. The session's candidate
  file routes go (invariant 30). The execution routes stay only for files
  nobody declared and for live output.
- **One comment thread.** A comment is a `Commented` row on an artifact
  version. Asking Vak to revise is an action on a comment, and a guest
  with the commenter role sees the same thread. Candidate comments and
  their routes go.
- **A shared conversation keeps its view, and its files inherit.** A
  guest's conversation grant reaches the conversation's artifacts by
  inheritance through the artifact routes, and an artifact that breaks
  inheritance is hidden from that guest.
- **The acceptance run uses both models and a Word document:** the whole
  run on `gpt-6-luna`, then create and review again on local Ollama
  (`gemma4`).

The steps, each shipped whole:
1. **M8.4c-a** Review by version (server). A version made from a
   candidate answers what the candidate routes answer today, by artifact
   id and version: its text and bytes, its Office or PDF projection, its
   review (the diff against its parent) and a narrowed draft. Accept, undo
   and the workspace checks are actions on the version. A session's
   sandbox records name the artifact and version of each candidate and
   each declared file. Exit test `review_by_artifact_version`.
2. **M8.4c-b** One comment thread. Candidate comments become `Commented`
   rows; `POST /library/{id}/comments/{comment}/revise` asks Vak to
   revise, and the revision is a version whose parent is the one
   commented on. The candidate comment routes and the
   `CandidateComment` activity go. Exit test
   `one_comment_thread_per_version`.
3. **M8.4c-c** The client. The Canvas opens an artifact version (the
   subject that replaces `draft_file`), its Office, PDF and diff viewers
   and Redline read `/library`, and the Workbench lists what is waiting
   for review as versions, with their comments, Accept and Undo. Chat
   cards, Canvas tabs and Workbench files gain Open in Library, and the
   Workbench gains the Library's star (doc 74 C8). The candidate file
   routes and their client calls are deleted. Browser run.
4. **M8.4c-d** The shared conversation. `SharedConversation` reads the
   conversation's files, versions and comments through the artifact
   routes under the guest's conversation grant. Exit test
   `conversation_grant_reaches_its_artifacts_until_broken`.
5. **M8.4c-e** Navigation. The admin groups of doc 74 §6.1 in the words
   already chosen (Conversations, Runs, Library, Automations), the
   client's sidebar, and every link that still points at a replaced
   screen. Browser check at both sizes.
6. **M8.4c-f** The acceptance run: create → review → promote → share →
   comment → revise, recorded under `docs/audits/`. M8 is done when it
   passes; erase waits for M7a.

M8.4c-a (2026-10-08): a version records the Review candidate that
proposes it (a `Proposed` row; `Version::proposed`), because the bytes of
an Office draft are usually already the current version and made no
version of their own. A candidate made from another (narrowed, revised, or
edited in a shared workspace) is a version made from the one its parent
proposed, and every writer of a candidate record notes it. Under
`/library/{id}/versions/{version}`: `text` reads any version; `document`,
`review` and `narrow` read and narrow an Office or PDF version in Review
through the worker; `accept`, `undo` and `checks` act on it through the
one promotion path. An unknown version is not found, a version nobody
proposed is a conflict, and a version whose conversation is in the trash
is not found. A promotion marks only the versions of the files it
applied, and an undo unmarks them (`PromotionUndone`).
`GET /sessions/{id}/sandbox/records` also returns `artifacts`: the
artifact, version and path of each candidate file and each declared file
of that conversation. The session's candidate routes still stand; M8.4c-c
deletes them with the client calls that use them. Test
`review_by_artifact_version`.

M8.4c-b (2026-10-08): a version has one thread.
`GET` and `POST /library/{id}/versions/{version}/comments` read it and
write in it, for the owner and for a guest of the conversation the version
is reviewed in (the guest's invitation must allow reading, and commenting
to write). A comment has an id (`cmt_`) and may point at lines of a text
file or an anchor in an Office file or PDF.
`POST /library/{id}/comments/{comment}/revise` asks Vak to revise the
version the comment is on, and the revision is a version made from it. An
artifact share's guest still writes through `/shared/artifact/comments`,
into the same thread. So that every reviewed file has a thread, putting a
file up for Review declares it: each file of a candidate is an artifact,
where M8.1 took only files a call had declared. The session's candidate
comment routes, their handlers and the client calls are deleted; the
client's Review, Canvas and shared conversation find the version from the
`artifacts` list of the sandbox records. The ledger's `candidate_comment`
activity kind still decodes, because ledgers are never rewritten, and
nothing writes or reads it. Tests `one_comment_thread_per_version`, and
`human_feedback_produces_new_candidate_without_workspace_write` for the
revision's parent.

M8.4c-c (2026-10-08): the client reads and decides a draft only by
artifact version. `GET /library/{id}/versions/{version}/raw` gives a
viewer the bytes (never recorded as a download), and `text`, `raw`,
`document` and `review` also answer a guest of the conversation the
version is reviewed in. `accept` takes `also`, the other versions the
same draft proposes, because a draft is promoted once. A draft that
deletes a file proposes a version in which the file is gone (a `Removed`
row; it has no bytes), so a deletion is reviewed and accepted like any
other version. A version keeps every draft that proposes it, so a draft
put up for Review again with the same bytes keeps its address. The
session's candidate file routes are deleted (`files`, `files/raw`,
`office`, `office-review`, `office-narrow`, `promote`, and a promotion's
`undo` and `checks`); making a draft from a run's files
(`POST /sessions/{id}/sandbox/candidates`) stays. In the client, the
Canvas, the chat cards, Review in the Workbench and the shared
conversation find a draft file's version from the sandbox records
(`api.versionOf`) and call only `/library`; chat cards, the Canvas and
Review have Open in Library, and Review has the Library's star. The
Canvas still names a draft by the draft that proposes it: its subject
keeps that identity, and what it reads is the version. Tests
`a_draft_of_several_files_is_accepted_in_one_promotion`,
`a_removed_file_is_a_version_with_no_bytes`, and
`promotion_uses_frozen_candidate_after_scratch_changes` on the version
routes. Live on `gpt-6-luna`: a three-paragraph Word draft was reviewed,
starred, narrowed to version 2, accepted into the folder and undone by
version, and the old routes answer 404.

M8.4c-d (2026-10-08): a guest's reach is decided by the grants. A
conversation guest reaches a version when their conversation grant is
active and strong enough for the request (reading needs a viewer,
commenting a commenter), the version is reviewed in that conversation,
and the artifact still inherits (`Grants::may`, called from
`library::discusser`). An artifact that broke inheritance, as sharing it
by a link does, is refused to that guest and left out of the `artifacts`
list their sandbox records return; the owner is unaffected, and restoring
inheritance gives it back. The live run found that no guest request to a
`/library` path had ever reached its handler: the audience check after
authentication read the conversation id from the path's second segment,
which there is the artifact id, and refused all of them. It now rechecks
the guest's own conversation on a Library path, and the tests run
through that check. The shared conversation reads its files, their
Office view, their review and their thread only through the artifact
routes. Test `conversation_grant_reaches_its_artifacts_until_broken`.
Live on `gpt-6-luna` with an invited guest: the guest read a Word draft
(3 units), its bytes and its review, and commented, in the guest page
too; was refused accept, the Library list and the artifact's page; and
after the owner shared the artifact by a link was refused every read
and saw no binding for it. Left as it is: the Canvas names a draft by
the draft that proposes it (its tab identity and version numbers are the
draft's), and reads the version.

M8.4c-e (2026-10-08): navigation. In the admin console the Work group
is Conversations (the screen that was called Sessions; its route is
unchanged), Runs, Library, Automations and Commitments: Library moved
out of Configure › Knowledge, which keeps Memory, Sources and Search.
Automations stays in Work, where they are made and edited, and
Operations keeps its own Automations tab for what is running (the
maintainer's choice; doc 74 §6.1 had one place under Operate). Two Inbox
links pointed at `#/skills`, a route that does not exist and showed Home;
they open Extensions › Skills. In the client the sidebar has Library,
Inbox with its unread count, and Automations: since the agent-first
change Inbox opened only by a keyboard shortcut and Automations only
from an Inbox entry. Settings calls hooks Hooks, as the admin console
does, so Automations means scheduled work everywhere. The Automations
sheet no longer says a prompt automation needs a Git project (untrue
since M4.8), and the admin Inbox says automation where it said scheduled
task. Checked live: all 18 admin navigation items and 26 sub-screens
open their own screen, and the client's three rows open theirs. Not
added: the items of milestones not built (Data health, Storage,
Lifecycle, Integrity, Sync, Governance, Keys, Audit export, Backup &
restore).

M8.4c-f (2026-10-08): the acceptance run, recorded in
`docs/audits/acceptance-m8-library-2026-10-08.md`. On `gpt-6-luna` every
step passed through the running app: create, review, the Canvas, accept,
Undo, accept again, a conversation guest, a share by link (which broke
the guest's inheritance), comments from both guests in one thread,
Continue working, and a second version made from the first and accepted.
On local Ollama create and review passed on 5 of 8 create attempts
(2 of 6 before the run's fixes). The run found and fixed four faults in
Vak: the Workbench never showed Review, Undo or the checks for a draft
(it looked for `.vak/scratch` in a path drafts left at M3b); Continue
working did not tell the Agent the comments on the version; a text
tool's refusal of a document sent a file not yet made to `doc_read`; and
an optional parameter sent as null was refused. With it M8.4c and M8 are
done, except erase, which is M7a's. The audit lists what stays open.

Investigating the live failures of M8.3b and M8.4b found faults in
Vak's own contract, each now fixed (AGENTS.md "Investigating a failure"):
- `emit_table_card` refused a correct flat `{columns, rows}` table
  because of its `{semantic_type, payload}` envelope. `Tool::canonical_input`
  now rewrites an unambiguous variant into the canonical form before
  validation. For a card that means a payload's fields at the top level, a
  type inside the payload, a type the tool's name already settles, and a
  table's column names and list rows. Superseded the same day: the envelope itself
  was the fault, so every card tool's arguments became flat (the card
  argument contract, docs/design/30-render-architecture.md §30.1) and the
  hook was removed.
- A card's result was replaced by a bare `{"ok": true}`, which dropped
  the tool's "it is on screen; finish" and led to repeat calls. The
  result now keeps that text, in plain words, with the presentation id.
- Save to Library sat behind Show technical details, and cards in a
  turn's result never had it. It is now on every recorded card, and an
  untitled card is named from its columns.
- `office_apply` now refuses a non-Office path first, naming `edit` and
  `write`.
- `doc_read` marks its line numbers as a gutter (`   1│`) and says they
  are not part of the file.
- Intent, the goal and the outcome read only the person's words, never
  a block the runtime wrote about an attachment.

`webfetch` returning a whole page's raw HTML is a separate fault, filed
on its own.

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

#### M9 design (agreed with the maintainer 2026-10-09)

Vak has one owner. M9 is cut to what one owner with two machines uses.

Decisions:

1. **The remote is a folder** (`FileRemote`): an external drive, a
   network share, or a folder another tool syncs. S3 and Postgres are not
   built, and neither is the hosted-tenancy key-escrow doc; both wait for
   a hosted service.
2. **Two machines take turns.** One machine holds the lease and writes;
   the other stands by and begins no turn. Work moves at a turn boundary:
   the holder hands over (nothing running, a last push, the lease
   released) and the other takes over (a pull, the lease, a new writer
   epoch). A lost machine is taken over from by force, which the owner
   confirms; when it comes back it is fenced and says what it never
   pushed. Two machines that both wrote are never merged.
3. **The remote carries exactly what a backup without secrets carries**
   (the registry's backup targets), as files checked by a signed-off
   index: a pull verifies every file against the index before it changes
   anything. Erasures travel with it: a pull destroys again every key the
   remote says is destroyed, so nothing erased on one machine comes back
   on the other.
4. **Secrets never sync.** A second machine reads the remote with a key
   file the owner exports under a passphrase and carries there by hand.
   Vak never puts it on the remote. Provider keys and bot tokens are
   entered again on the second machine.
5. **Syncing is automatic and on demand.** The holder pushes after work
   settles; a push that cannot reach the folder is tried again later and
   loses nothing; Sync now and `vak sync` do it by hand.

Steps, each shipped whole and in this order:

| Step | What | Exit tests |
|---|---|---|
| M9-a | The folder remote: the index, push, pull, `vak sync setup`, `now` and `status` | `push_pull_roundtrip_identical_derive_messages` |
| M9-b | The key file: export under a passphrase, import on another machine | `key_file_opens_the_remote_on_another_machine` |
| M9-c | The lease: standing by, handing over, taking over, fencing the machine that lost it | `handoff_at_turn_boundary`, `lease_prevents_dual_writer` |
| M9-d | Erasures, holds and receipts travel both ways | `erasure_propagates_and_cannot_resurrect` |
| M9-e | Automatic push with retry; `/sync`; the admin Sync screen (A18); the client's line in Your data | `sync_survives_network_loss` |
| M9-f | Doc 56 superseded, doc 31's sync plane, the acceptance run | the run |

Not built, by decisions 1 and 2: `S3Remote`, Postgres refs, region
pinning, key escrow and release, handing over one live conversation while
others keep running, and merging.

**M9-a and M9-b, done 2026-10-09: the folder remote and the key file.**
They ship together, because the round trip needs a second machine that
holds the keys.
- `vak_core::sync`: `Core::sync_setup(folder)` names the remote and mints
  this machine's id; `sync_push` adds each new or changed file to the
  remote under its SHA-256 (`blobs/`, never changing one that is there),
  then replaces `index.json` (each path's size and digest, the push
  generation, the store's writer epoch, the machine), which is what makes
  the push count, and only then removes what the new index no longer
  names, so a push stopped at any point leaves a whole copy; `sync_pull`
  checks every file
  against the index before it changes anything, refuses to replace work
  that was never pushed unless told to discard it, and ends as a restore
  does (`Core::settle_imported`: erasures applied again, search rebuilt,
  the writer epoch moved), so the processes on that home start again;
  `sync_status` counts what changed here since the last sync.
- The remote holds the registry's backup targets under the data home,
  less locks, scratch and `credential_index.json`. The refs database
  never travels as a file: `LocalStore::export_refs` writes every ref to
  a portable file and `import_refs` replaces them in one transaction
  (`RefStore::replace_all`).
- What this machine knows about its remote is `sync/local.json`, class
  Derived, so neither a backup nor a push carries one machine's identity
  to another.
- The key file: `KeyMaterial` (every tenant key version, the object id
  key and the receipt-signing key) sealed under a passphrase of at least
  12 characters with PBKDF2-HMAC-SHA256 (600,000 rounds) and the store's
  AEAD. `Core::export_key_file`, and `Core::import_key_file`, which is
  refused once an install holds data under its own keys. Vakyartha never
  writes it to the remote.
- `vak sync` (status), `setup <folder>`, `now`, `pull [--discard]`,
  `forget`, `key export|import <file>`. The passphrase is typed, or read
  from `VAK_KEY_PASSPHRASE` for a script.
- Found on the way: `vak data cat` found no conversation on a data home
  no server had run on; it now reads the records in first.
- Tests: `push_pull_roundtrip_identical_derive_messages` drives two
  homes with the `vak` binary (nothing readable and no secret in the
  remote; a second push copies nothing; a machine without the key file
  reads nothing; with it both conversations read the same on both; a
  remote file changed by one byte is refused whole; what an unfinished
  push left is ignored by a pull and cleared by the next push), and
  `a_key_file_opens_only_with_its_passphrase`.

**M9-c, M9-d and M9-e, done 2026-10-09: the lease, erasures across
machines, and the automatic push.**
- The remote's `lease.json` names the machine that may write and whether
  it has handed over. Each machine notes where it stands
  (`sync::Role`: holder, standing by, lost) in `sync/local.json`. A
  machine that stands by or has lost the work begins no turn
  (`Core::refuse_standing_by`, `CoreError::StandingBy`; the run route
  refuses at once) and runs no scheduled work.
- `Core::sync_handover` is refused while any run is open under a live
  process, then pushes, releases the lease and stands by.
  `Core::sync_takeover` is refused until the other machine has handed
  over unless forced; it pulls when the remote is ahead and takes the
  lease. A machine that pushes and finds the lease is another's has lost
  the work: its push is refused with how many files it never pushed, and
  it pulls (told to discard them) to stand by again. The lease and the
  generation are read once more just before a push counts.
- The index carries the pusher's key version, and a pull is refused
  before it changes anything when this machine's key file is older. A
  key file made after a rotation is taken by a machine that already
  holds the earlier keys.
- An import keeps the puller's own writer epoch
  (`LocalStore::import_refs` stamps each ref no newer than it) and
  `Core::settle_imported(past)` ends by moving the epoch past both
  machines'.
- Erasures need nothing of their own: a push carries the tombstone and
  drops the key, and a pull removes the key here and applies every
  recorded erasure again. A machine with a stale copy cannot push it
  back, and taking over, forced or not, pulls first.
- `Core::sync_auto` is the scheduler's push: between turns, when this
  machine holds the work and something changed, paced after a failure
  from one minute to about half an hour. `GET /sync`, `POST
  /sync/setup|now|handover|takeover|forget|key/export|key/import`; `vak
  sync handover|takeover [--force] [--discard]|place`; the admin
  console's Operate › Data › Second copy; the client's line in Your data.
- A project made on the other machine is placed in its folder here with
  `vak_config::spaces::attach` (`vak sync place`, Projects › Its folder
  here), refused when that folder is already a project with
  conversations.
- Tests: `handoff_at_turn_boundary`, `lease_prevents_dual_writer`,
  `erasure_propagates_and_cannot_resurrect`,
  `sync_survives_network_loss`,
  `the_owner_sets_up_copies_hands_over_and_takes_back`.

**M9-f, done 2026-10-09: the acceptance run**, recorded in
`docs/audits/acceptance-m9-remote-2026-10-09.md`: two machines in Docker
(`scripts/sync-lab`), real turns, 45 of 45 checks on the third run, nine
defects found and fixed on the way. Doc 56 is superseded and doc 31 has
the sync plane. With it M9 is done, and with M9 the plan.

## 5. AGENTS.md and design-doc changes

| When | Change |
|---|---|
| M0 | Done: invariant 38 drops `AgentSchedule`/`AgentRunRecord`; docs 65 and 72 lose the claims; doc 64 draws the real 4.x topology; docs 73 and 74 listed under proposals with the "Pending" section; invariant 8 gains "no API writes a secret to a file". |
| Revision 3 | Done 2026-10-01: "Pending" names the data baseline by reference, the new order and the two guards; docs 76, 79, 80, 81 and 82, the collaboration plan and the reliable-work plan point at the primitives they use. |
| Revision 4 | Done 2026-10-03: 6.0.0 is an independent major release; L3 moves the data baseline to 7.0.0 and L6 moves the future release train and 6.x maintenance branch accordingly. |
| Now | "Until the next milestone lands": no raw home-path call beyond the ratchet; every Agent-home subpath declared. |
| M1 | **New invariant:** every durable record, span and envelope carries a `TraceKey` with its actor; derived writes record `derived_from`. |
| M2 | **Invariant 2:** entries are never rewritten; encoding changes only by a verified seal; the hash chain covers frames as stored. |
| M3b | **Invariant 29** → 7.0.0 baseline. **Invariant 35** rewritten (runtime state under the tenant; Agent workspaces bound to Space and Agent). **Invariant 37** paths. **New invariant:** every path belongs to a declared class; ledgers hold references, not bulk content; content is keyed to its conversation wherever it is written. Layout map gains `vak-storage`. Doc 73 status → "in progress". |
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
| Releases during M3b | One train (L6): main is the 7.0 line from slice 1; no 6.x line. |
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
the new purge removes the owned roots wholesale. Data on a 6.x host,
including the maintainer's EC2 host, does not cross the baseline: download
what you want to keep as ordinary files (transcripts, documents) before
upgrading it, because no 6.x backup restores into 7.0.

## 9. Open items after the plan

The working tracker was deleted when the plan closed (2026-10-09); its
step rows are in git history and each milestone's section above. What it
still carried, checked against the code on 2026-10-09:

- **Held effects are never released.** An effect prepared with a `hold`
  (a digest or until-complete packet) has status `Held`, and no
  `EffectStep` moves it on: there is no release and no flush.
- **A deleted trigger's claim ref stays.** `trg/<id>/claim` is left
  behind because refs have no delete. The lifecycle reconciler should
  remove it once refs can be deleted.
- **The mail vault keeps its own routine run history**
  (`list_routine_runs` in `vak-mail-calendar`'s vault), beside the run
  records. It was justified at M4.7b because it binds a run to an account
  and is removed on disconnect; run records can now be erased (M7a), so it
  can fold into them.
- **Write-path growth on a used 7.0 home** has not been remeasured (§M3b;
  `docs/architecture/write-paths-and-growth.html` is still the v3.5.1
  measurement).
- **The Workspace data class has no backup policy.**
- **The stop guard's "Please continue." was seen echoed back** in a
  model's answer. Not investigated: the open question is what in the
  guard's control message (`vak-agent` `stop_policy.rs`, the
  `[stop-guard]` text) invites the copy and whether `clean_scaffolding`
  reaches the place it showed up.

Closed since the tracker listed it: a copy environment orphaned by a
crash is now aged from its last write and removed by the reconciler
(`environment_items` in `vak_core::lifecycle`, M7a-d).

The handover for the session after the plan
(`docs/plans/handover-2026-10-09.md`) lists the product limits and the
checks not yet run in a browser or live.
