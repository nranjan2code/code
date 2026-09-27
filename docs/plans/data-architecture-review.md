# Review — data architecture design and plan

Status: **review, 2026-09-25.** Covers
`docs/design/73-data-architecture-and-lifecycle.md` and
`docs/plans/data-architecture-plan.md` as first written. Every finding below
was checked against the source on `main` at `767db1d0`. Nothing was run
live, so each defect is marked *code* (read from source) or *measured* (file
names and sizes on a real data home; no ledger or credential content was
opened). The fixes have been applied to the plan, to doc 73, and to the new
`docs/design/74-lifecycle-and-data-administration.md`; the right-hand column
says where. The complete impact inventory is
`docs/plans/data-architecture-blast-radius.md`.

## Verdict

The direction holds: git-style storage, SharePoint-style information
architecture, CI-style runs, a trace key on every record, one reconciler, one
catalog. But the first draft had five problems that would each have shipped
something wrong:

1. **Its encryption design could not do what it promised.** Per-conversation
   keys on shared, deduplicated objects cannot give both dedupe and erasure.
   (R1)
2. **Erasure was scheduled before the lineage it depends on**, so derived
   copies (memory, entities, search rows, inbox and delivery text) would have
   survived an erasure. (R2)
3. **Its first milestone relied on a task-environment backend that does not
   exist.** (R5)
4. **It missed four live defects that matter more than layout**: plaintext
   NATS secrets written into the project tree, a "delete" that only hides, a
   purge that leaves personal data and all logs behind, and a Python writer
   with its own path resolver. (R6–R8, R15)
5. **It miscounted the work.** The layout change touches 552 production
   home-path uses in 65 files (200 `sessions_home()` calls alone) and 500
   test references in 66 files. That count is why the cutover is now split
   into a refactor that changes no behaviour, followed by a separate switch
   to the new layout. (R11, R30)

## Findings

Severity: **C** = would ship unsafe or incorrect behaviour or blocks the
plan; **H** = a significant gap or a wrong assumption; **M** = a correctness
or completeness fix; **L** = accuracy or hygiene.

### Security and compliance

| ID | Sev | Finding | Evidence | Fix | Applied |
|---|---|---|---|---|---|
| R6 | C | `PUT /config/bus` writes the NATS credentials JWT and NKey seed **in plaintext to `<project>/.vak/env`**, inside a repository a person may commit. Nothing reads that file back, so the feature is also broken. Its response says "takes effect on next server restart". | code: `crates/vak-server/src/lib.rs:13470-13540` (write), `:13549` (delete); no reader of `.vak/env` anywhere | Store the values through `vak_config::credentials` in the project secret scope, read them in bus resolution, and apply live (invariant 31). Remove every `.vak/env` path. Add an escapes-style test: no API writes a secret-shaped value to any file. | plan M0 |
| R7 | C | **Purge is incomplete.** `purge_state` iterates Data, Cache and Shared, not **Logs**, although the Logs root was added precisely because logs survived a purge. Anything undeclared survives by design: `commitments.jsonl` (goal text), `feeds/feeds.duckdb` (ingested content), `credential_index.json`, `archive.json`, `deleted.json`, `delivery/`, `sandbox/`, `workspaces.json`, a legacy `credentials.enc`/`.credential_key` pair. Project `.vak/` state (scratch, worktrees, Agent workspaces, R6's secrets) is never purged. | code: `crates/vak/src/install/mod.rs:1090-1110`; measured on a real data home | At 5.0.0 the roots Vak owns (data, cache, logs, runtime) are removed wholesale; only the Shared home `~/vak-home` uses declared entries, because it is a person's directory. Purge also lists and offers any leftover Vak runtime directories inside known spaces. The completeness test covers all roots. | plan M0 (Logs), M3b |
| R8 | H | **"Delete" only hides.** `DELETE /sessions/{id}` adds the id to `deleted.json`. The ledger, checkpoints, index rows, memory and anything else derived stay on disk. The model's `session_search`, admin search, `search_all` and the FTS store never consult the deleted map, so a "deleted" conversation stays searchable and can reach a prompt. | code: `lib.rs:9140-9177`, `:9060`; only `lib.rs:4113` and `admin.rs:88` read the map | M0: every read path honours one `is_trashed` check (list, search, recall, admin, export). M7: "delete" becomes *trash*, then *erase* (crypto-shred) with a receipt (doc 74 §2.4). | plan M0, M7; doc 74 |
| R1 | C | The encryption model in doc 73 §7.3 is inconsistent. Whether an object is exclusive to one conversation is unknown when it is written. Per-conversation keys make dedupe impossible. Convergent encryption would leak content equality across tenants. | design | Each object gets its own random key. The object id is a **keyed** hash (HMAC with a tenant id key), so dedupe works within a tenant and cannot confirm files across tenants. Each referencing scope (conversation, space, artifact) holds the object key wrapped under its own key (a *key grant*). Shredding a scope destroys its grants; an object stays readable while any grant survives, and GC collects it at zero grants. Records are encrypted **per entry** so appends need no rewrite. | doc 73 §7.3; plan M2 |
| R2 | C | **Erasure must follow lineage, and lineage came later.** A conversation's content is copied into memory notes, entities, skill proposals, commitment text, inbox bodies, delivery text, outbox jobs, FTS rows, embeddings and checkpoint labels. The plan scheduled erasure (M6) before the catalog and lineage (M7). | code: `inbox.rs` `Entry.body`, `checkpoints.rs` label `"turn: {prompt}"` at `vak-core/src/lib.rs:6804`, memory/entity writers take no provenance | Order becomes catalog then lifecycle. A new design rule: **content is keyed to its conversation wherever it is written** (field-level encryption of content fields in shared ledgers), and every derived write records `derived_from`. Checkpoint labels carry a turn id, not prompt text. | plan resequence (M6 ↔ M7), M1 provenance; doc 73 §7.3; doc 74 §4 |
| R3 | H | **Telemetry would leak content.** Logs are unencrypted, rotated and exported, and erasure cannot reach them. The plan said nothing about what a span may carry. | design | Spans and log events carry ids, kinds, sizes, digests, durations and outcomes only. A test scans emitted events for any string over 64 bytes outside an allowlist of fields. | plan M5 |
| R20 | H | **The catalog's FTS and embedding stores are plaintext derived copies**, so they defeat encryption at rest and survive erasure unless removed explicitly. | design | Erasure deletes catalog rows (with `secure_delete`, then a WAL checkpoint). Doc 73 states the local limit honestly: at-rest protection of derived stores relies on OS disk encryption. The cloud uses managed encryption. | doc 73 §9; doc 74 §4 |
| R21 | H | **Backups defeat crypto-shred.** `backup::export_to` copies plaintext files, so an erased conversation lives on in every backup. | code: `crates/vak-core/src/backup.rs:119-196` | Backups carry ciphertext plus *wrapped* keys, never a KEK unless escrow is asked for. Restore re-applies every erasure tombstone from the audit record before anything becomes readable. | plan M7 |
| R22 | M | **The bus (NATS JetStream, when configured) persists envelopes outside Vak's store.** Retention and erasure cannot reach them. | code: `crates/vak-bus/src/bus.rs:112-135,276` | Envelopes carry references (ids), never conversation content. Streams get `max_age`. The erasure receipt lists processors it cannot reach (channels, LLM providers, external NATS). | plan M5, M7; doc 74 §4 |
| R33 | M | **Erasure scopes were conversation-only.** Data-subject requests are by *person* (a sender across many chats), by Agent, by space and by tenant. The gateway allowlist and security events hold personal data (sender ids, IP addresses). | code: `SecurityEvent.ip`, allowlist entries | Erasure scopes: conversation, audience/person, Agent, space, tenant. Pending and denied allowlist entries get a retention policy. | doc 74 §3 |
| R10 | M | **Encryption conflicts with the transparency thesis**: a person can no longer `cat` a ledger. | AGENTS.md Identity | Encryption is a per-tenant policy, on by default because it is what makes erasure possible. `vak data cat / grep / export --plain` gives the transparent view. Doc 73 states that on headless hosts the KEK sits in the encrypted-file store, whose key lives beside it, so at-rest protection there is nominal while crypto-shred still works. | doc 73 §7.3 |

### Correctness of the design

| ID | Sev | Finding | Evidence | Fix | Applied |
|---|---|---|---|---|---|
| R4 | H | **"Exactly once per slot" overclaims.** A run has side effects; a crash between start and record can't be exactly-once. | design | **At most one start per slot**: the slot is claimed with CAS before any side effect. A lease-expired run is recorded `abandoned`. A per-schedule `on_crash = skip | retry_once` makes retry explicit. | plan M4; doc 73 §8 |
| R9 | H | **Wrong classes in Appendix A.** Memory notes, entities and skill proposals are rewritten (amend, forget, whole-file rewrite), so they are not append-only Records. `presentations.json` is the presentation-pack **library** (definitions and activations), not a derived projection. `cost-log` and `budget-alerts` are compacted by rewrite despite being declared ledgers. | code: `memory.rs:417-449`, `vak-store/src/presentation.rs:1-20`, `finops.rs:22-30,600-610` | A new **Document** class: mutable named content where every save is an immutable version, "current" is a ref, and history follows retention. Memory, entities, skills, prompt layers and presentation packs are Documents. Cost and alerts become segment chains whose retention drops sealed segments, never rewriting a file. | doc 73 §5, App. A; plan M3b |
| R17 | M | **Sealing a segment writes a new file and removes the open one.** Invariant 2 must allow this explicitly or it reads as a violation. | design | Seal is copy, verify (entry count, hashes, chain continuity), then atomic swap, and is itself a record entry. Invariant 2 is amended: "entries are never rewritten; encoding changes only by a verified seal". Session entries are already hash-chained per entry (`vak-session/src/log.rs:225`); segments carry that chain across files. | plan M2, §5 |
| R18 | M | **The plan used a lock plus a lease for the local single writer.** A lease needs expiry and heartbeats; a local flock is released on process death. | code: `vak-session/src/log.rs:61,167` (`try_lock`) | Local single writer stays a flock. A lease is taken *in addition* only when a remote is configured (M9). | plan M2, M9 |
| R19 | M | **Multiple processes (desktop, server, CLI, gateway) share the catalog.** Offset-following ingest in each would double-ingest. | design | Ingest is idempotent: upsert keyed by `(chain, seq)`, WAL with `busy_timeout`, and any process may ingest. | plan M6 |
| R34 | M | **Time-based retention on a hash-chained ledger** can't drop individual entries without breaking `derive_messages()` and the chain. | design | **Records expire per conversation, never per entry** (crypto-shred of the whole conversation after `last_activity + delete_after`). Shared, non-content ledgers (cost, routing) expire by sealed segment. | doc 73 §7; doc 74 §3 |
| R31 | M | **Checkpoint labels embed the raw prompt** (`"turn: {prompt}"`). Checkpoints belong to the space, so erasing the conversation would leave its prompt text in space-owned data. | code: `vak-core/src/lib.rs:6800-6806` | The label is the turn id. The manifest carries the TraceKey. | plan M1 |
| R35 | L | **Sessions rotate inside one conversation** (invariant 17). Keys must span the rotation. | design | Keys are per **conversation**, not per session. | doc 73 §7.3 |

### Wrong assumptions in the plan

| ID | Sev | Finding | Evidence | Fix | Applied |
|---|---|---|---|---|---|
| R5 | C | **M0 said non-git spaces would use "the doc-54 task environment".** No `EnvironmentBackend` implementation exists: the trait, `EnvironmentPlan` and `EnvironmentRecord` are used only inside `vak-sandbox` and its tests. | code: `crates/vak-sandbox/src/lib.rs:59-66,572`; no `impl EnvironmentBackend` anywhere | M0: a non-git space's routine is refused loudly (new inbox kind `RoutineFailed`, with reason and remedy). M4 builds the first real backend, `CopyEnvironment`: copy the space with ignore rules and size caps into `environments/<run>/`, run there, and export changes as a candidate through the existing Review path. | plan M0, M4 |
| R11 | H | **Every count was wrong.** A scan that skips only `#[cfg(test)]` items (not whole files after the first test module) finds, in production code: **`sessions_home()` 200 calls in 20 files** (104 in `vak-server/src/lib.rs`, 35 in `vak-core/src/lib.rs`); **`shared_data_home()` 69 in 10 files**; **552 home-path identifier uses in 65 files**; `.vak` literals 70 in 28 files; `hash_cwd` 10 in 7 files. Tests: **500 layout references in 66 files** (vak-server 328, vak-core 111). `eprintln!` in production library/server code: **94** (vak-server 68, vak-core 9, vak-sandbox 7, vak-desktop 7, vak-tools 2, vak-llm 1), plus 278 in the CLI and terminal that are user output. AEAD in the workspace is **`ring`** (used by vak-bus), not `aes-gcm`. zstd and `tracing` are new dependencies. `async-nats` and `rusqlite` are not pinned in the workspace manifest (a code-rule violation). | measured: scripted scan, method and file lists in the blast-radius doc | Numbers corrected. `ring::aead` reused. The two unpinned dependencies are pinned when their crates are touched. | plan M2, M3, M5; blast-radius doc |
| R30 | H | **The one-shot M3 is too big to review safely.** | R11 | M3 is split: **M3a** introduces `Scope`/`StorageHandle` on the *current* layout, migrating all production and test call sites with no behaviour change. **M3b** switches the layout at 5.0.0 behind that API. A `TestScope` helper lands in M1 so test migration is mechanical. | plan M1, M3a, M3b |
| R12 | H | **Space identity reaches far beyond sessions.** Things keyed by path today include: credential scopes (`scope_key_for` hashes the canonical path, so moving to ids orphans every stored secret unless re-keyed in the same change), trust markers, CorePool identity, gateway allowlist workspace fields, `gateway/default-workspace`, `[server] workspace_roots`, `TaskDef.cwd` plus the scheduler filter `t.cwd == state.core.cwd()`, `SessionHeader.cwd`, the client's `/workspaces` API and the desktop trust gate. | code: `vak-config/src/credentials.rs:51-60`; `lib.rs:17863` | M3b owns the whole list (blast-radius doc §M3b). Secret scopes are re-keyed by `(tenant, space|agent)`. | plan M3b |
| R13 | M | **The admin Operations Center already has "runs"**, keyed by session id (`#/operations/work/runs/<session_id>`, `Home.tsx:458`). A new Run model beside it would create two meanings of "run". | code: `crates/vak-admin-ui/src/OperationsCenter.tsx:255,530,542` | M4 replaces the route with `#/runs/<run_id>`. The Live work table lists runs. There is no session-keyed run route. | plan M4; doc 74 §6 |
| R15 | H | **The Python feeds pipeline has its own path resolver** (`scripts/feeds/feed_utils.py:38`). It ignores `VAK_HOME`, and Rust passes `VAK_SESSIONS_HOME` (`feeds.rs:139`), which the Python never reads. Tests and portable installs write into the real data home. | code | M0: Rust passes the exact file path (`VAK_FEEDS_DB`); the Python resolver is deleted. M3b: the path is under the tenant. | plan M0, M3b |
| R24 | M | **The registry enforcement test** (`crates/vak-core/tests/state_registry.rs`) starts one session and records one security event, which is why most drift went unnoticed. | code | The §6 scenario matrix drives the test, across every root, including Logs, runtime and the space `.vak/`. | plan §6 |
| R25 | M | **Retention is scattered and hardcoded** in four places: 20 checkpoints per session, the cost-log 5 MB / 90-day rewrite, the alerts 1 MB / 2 000-row rewrite, and 24 h memory write debris. None is configurable or recorded. | code: `checkpoints.rs:52`, `finops.rs:22-30`, `memory.rs:57` | All four are removed when the reconciler lands (invariant 30); their limits become default policies. | plan M7 |
| R26 | M | **`AgentLifecycle` is `Active | Paused | Archived`**, but invariant 37 names *revoked*. Nothing defines what archive or delete means for an Agent's data. | code: `crates/vak-server/src/agents.rs:15-19` | Doc 74 §2.3 defines Agent data semantics, including `Revoked`. | doc 74 |
| R14 | M | **Docs claim behaviour that isn't built.** Doc 64 (layout tree), doc 65 ("implemented and audited", `AgentSchedule`/`AgentRunRecord`), doc 72 (`AgentSchedule` automations and receipts in `agents_runs.jsonl`), and AGENTS.md invariant 38. | code vs docs | M0 corrects each in the same change that deletes `AgentSchedule`. | plan M0 |
| R16 | L | **Plan M3 said `presentations.json` "becomes a catalog projection".** | R9 | It becomes a Document store (pack library). | plan M3b |
| R29 | M | **There's no data dictionary.** The additive-only rule (invariant 29), audit export and the cloud protocol all need one schema per record type. | design | M1 adds `docs/reference/records.md`, generated by a test from the record types (name, version, fields, class). CI fails when it drifts. | plan M1 |
| R28 | H | **The plan had no screens for lifecycle work** (retention, holds, erasure, storage, reconciler, integrity, runs, library), and no end-user trash, restore or "why is this gone". | — | New design `docs/design/74-lifecycle-and-data-administration.md`, with screens assigned per milestone. | doc 74; plan §4 |
| R36 | L | **The measured baseline in the plan came from v3.5.1.** | `docs/architecture/write-paths-and-growth.html` | The measurement is re-taken at M0 exit (the bytes-per-turn test prints it) and again at M3b. | plan M0, M3b |

## What stays as written

- The model split: git-style storage, SharePoint-style IA, CI-style runs.
- `TraceKey` with `Cause`, and enforcing it through a `Traced` bound.
- The data classes (now with Document added), one reconciler, one catalog.
  Cloud as a remote with lease handoff.
- The decisions L1–L5.
- The standing scenario matrix. It is the most valuable single deliverable,
  because it would have caught R7, R8 and every item of D10.
