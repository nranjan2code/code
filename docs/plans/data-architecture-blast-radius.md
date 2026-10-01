# Blast radius — data architecture plan

Status: **inventory, 2026-09-25**, taken from `main` at `767db1d0`. It lists
everything each milestone of `docs/plans/data-architecture-plan.md` changes:
code, HTTP API, UI, CLI, configuration, services, docs, scripts, the site,
tests and external boundaries. Re-run the scans in §0 at the start of each
milestone; a milestone is not done while its section still names something
unchanged.

## 0. Method and totals

Production counts skip `#[cfg(test)]` items by brace-matching, rather than
dropping everything after a file's first test module (which undercounted
`vak-server/src/lib.rs` about ten-fold). Test counts are total minus
production.

| Measure | Production | Tests |
|---|---|---|
| home-path identifiers (`sessions_home`, `shared_data_home`, `data_home`, `paths::{default_workspace,cache_home,logs_dir}`, `agent_home*`, `agent_workspace`) | **552 uses in 65 files** | — |
| `sessions_home()` calls | 200 in 20 files (vak-server/lib.rs 104, vak-core/lib.rs 35, vak/main.rs 15, vak-server/admin.rs 14) | — |
| `shared_data_home()` calls | 69 in 10 files (vak-server/lib.rs 37, gateway.rs 9, vak-core/lib.rs 7) | — |
| `.vak` path literals | 70 in 28 files | — |
| `hash_cwd(` | 10 in 7 files | — |
| layout references in tests (`set_sessions_home`, `sessions_home()`, `join("sessions")`, `.vak/`, `hash_cwd`, `isolate_home_for_tests`, …) | — | **500 in 66 files** (vak-server 328, vak-core 111, vak-config 29, vak-tools 12, vak 10) |
| `eprintln!` | 94 library/server (vak-server 68, vak-core 9, vak-sandbox 7, vak-desktop 7, vak-tools 2, vak-llm 1); 278 CLI/terminal user output | — |
| clock-derived or truncated ids (`as_nanos()`, `[..8]` on a UUID) | 13 in 11 files | — |

Scan script: `prod_lines()` plus the regex sets above. The date and source
commit are recorded here so the next run can be diffed.

## 1. Cross-cutting surfaces at a glance

| Surface | M0 | M1 | M2 | M3a | M3b | M4 | M5 | M6 | M7 | M8 | M9 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| vak-session | | ● | ● | ● | ● | ● | | ● | ● | | ● |
| vak-storage (new) | | | ● | ● | ● | | | ● | ● | ● | ● |
| vak-catalog (replaces vak-store) | | | | | | | | ● | ● | ● | ● |
| vak-lifecycle (new) | | | | | | | | | ● | | ● |
| vak-core | ● | ● | | ● | ● | ● | ● | ● | ● | ● | |
| vak-server | ● | ● | | ● | ● | ● | ● | ● | ● | ● | ● |
| vak-agent / vak-tools / worker protocol | | ● | | ● | ● | | ● | | | | |
| vak-sandbox | | ● | | | ● | ● | ● | | ● | ● | |
| vak-config (paths, credentials) | ● | | ● | ● | ● | | ● | | ● | | ● |
| vak-delivery / vak-commit / vak-flow / vak-bus | | ● | | ● | ● | ● | ● | ● | ● | | ● |
| vak (CLI, install, setup, backup) | ● | | | ● | ● | ● | ● | ● | ● | | ● |
| vak-ops / vak-tray / vak-desktop | ● | | | ● | ● | | ● | | ● | | |
| vak-client-ui | ● | | | | ● | ● | | ● | ● | ● | ● |
| vak-admin-ui | ● | | | | ● | ● | ● | ● | ● | ● | ● |
| vak-terminal | | | | | ● | ● | | ● | | | |
| scripts (feeds, service, linux) | ● | | | | ● | | | | | | |
| site (`crates/vak-server/site`) | | | | | ● | | | | ● | | |
| AGENTS.md invariants | ● | ● | | | ● | | ● | ● | ● | ● | ● |
| design docs | ● | ● | | | ● | ● | ● | ● | ● | ● | ● |

## M0 — fixes now (4.x)

**Code**
- `crates/vak-server/src/agents.rs`: delete `AgentSchedule`, the
  `schedule` field on `AgentDefinition`, `AgentRunRecord`, `record_run`,
  `list_runs`, `runs_path`, `update_schedule` and their tests
  (`agents.rs:600-630`).
- `crates/vak-server/src/lib.rs`:
  - Routes `/agents/{id}/schedule` (`:1078`) and `/agents/{id}/runs`
    (`:1079`), with their handlers near `:15040-15110`.
  - `fire_task` (`:17472`): full run ids, the git/non-git branch, and
    refusal notifications.
  - `advance_marker` call sites in `scheduler_tick` (`:17893`) and
    `catch_up_missed_tasks`.
  - `spawn_isolated_run` (`:16482`): the handle id equals the ledger id,
    and `set_sessions_home(shared_data_home())` at `:16528`.
  - `/config/bus` PUT and DELETE (`:13470-13560`): move to the credential
    store, apply live, and remove `.vak/env`.
  - Every session read path honours trash: `list_sessions` (`:4113`),
    search, transcript, export, recall.
- `crates/vak-server/src/admin.rs`: the admin sessions list (`:88`), admin
  search, and forensics honour trash.
- `crates/vak-core/src/session_search.rs`, `crates/vak-session/src/search.rs`,
  `crates/vak-store/src/query.rs`: filter trashed sessions (a single
  `is_trashed` source).
- `crates/vak-core/src/inbox.rs`: new `Kind::RoutineFailed`.
- `crates/vak-core/src/lib.rs:6658` (`append_turn_capabilities` call site),
  `crates/vak-session/src/log.rs:716`: skip an unchanged binding and write
  a reference entry. `crates/vak-server/src/projection.rs:582` follows the
  reference.
- `crates/vak/src/agents_cli.rs:132,175`: remove the schedule and run flags.
- `crates/vak/src/install/mod.rs:1093`: add `Root::Logs` to `purge_state`.
- `crates/vak-server/src/feeds.rs:139,199,248`: pass `VAK_FEEDS_DB`.
  `scripts/feeds/feed_utils.py:38-57`: delete the resolver and read
  `VAK_FEEDS_DB`.

**UI**
- `crates/vak-admin-ui/src/types.ts:1101` (`last_status` on agent
  schedule), plus any agent-schedule form. Verify the admin agent editor has
  none left.
- `crates/vak-client-ui/src/components/Settings.tsx`,
  `crates/vak-client-ui/src/api.ts`: the delete wording becomes "Move to
  trash (hidden everywhere)" until M7.

**Docs**
- AGENTS.md invariant 38 (the `AgentSchedule`/`AgentRunRecord` bullet).
- `docs/design/65-universal-adaptive-platform.md:58-62`.
- `docs/design/72-openxml-documents.md:176,261`.
- `docs/design/64-agent-owned-platform.md` topology section (real 4.x tree).
- AGENTS.md "Reading the design docs": list 73 and 74 under proposals.

**Tests (new)**
- `fire_task_records_refusal`, `scheduled_run_resolves_after_restart`,
  `two_tasks_due_same_tick_both_fire`, `cron_slot_not_lost_on_failure`.
- `non_git_space_routine_is_refused_loudly`, `child_core_home_is_not_nested`.
- `unchanged_capabilities_not_rewritten`, `trashed_session_absent_from_every_search`.
- `bus_credentials_never_written_to_a_file`, `purge_includes_logs`,
  `feeds_write_under_overridden_home`.

## M1 — ids, TraceKey, provenance

**Code**
- New `crates/vak-session/src/ids.rs` and `trace.rs`.
- `crates/vak-session/src/types.rs`: `SessionHeader` gains `space`, `run`,
  `cause`. `Entry` stays the same; its payloads carry ids.
- `crates/vak-tools/src/context.rs:7`: `ToolContext.agent_id` becomes
  `trace`. Update every construction site (`execute_script` at
  `lib.rs:17640`, the agent loop, workers).
- `crates/vak-tools/src/broker.rs` and `crates/vak-server/src/bin/vak-tool-worker.rs`:
  protocol version bump, carrying the key.
- `crates/vak-tools/src/sandbox_events.rs` (`SandboxEventSink`);
  `crates/vak-sandbox/src/lib.rs` records (`CandidateRecord` already has
  most ids; `EnvironmentRecord`, `PromotionRecord` and `PreviewPreparationRecord`
  gain the key).
- `Traced` rows for every side ledger:
  - `crates/vak-core/src/finops.rs:33,55,552` (`CostRow`, `ActivityRow`,
    `BudgetAlertRow`)
  - `crates/vak-core/src/routing.rs:24` (`EvidenceRow`)
  - `crates/vak-core/src/misread.rs:75`
  - `crates/vak-core/src/security_events.rs:11`
  - `crates/vak-core/src/inbox.rs:57`
  - `crates/vak-commit/src/ledger.rs:114`
  - `crates/vak-server/src/operations.rs` (incidents and actions)
  - `crates/vak-server/src/coworking.rs` (grants)
  - `crates/vak-server/src/delivery.rs` (deliveries rows, outbox jobs)
  - `crates/vak-core/src/checkpoints.rs:73` (manifest; label → turn id at
    `vak-core/src/lib.rs:6804`)
- Provenance (`derived_from`) on derived writes:
  - `crates/vak-core/src/memory.rs:114` (`append_note`)
  - `crates/vak-core/src/learning.rs:171`
  - `crates/vak-core/src/reflection.rs:310`
  - `crates/vak-core/src/entities.rs`
- `crates/vak-server/src/bus.rs:132-178`: pass the run trace; real
  `prev_hash`.
- The 13 clock-derived or truncated ids (§0) become typed ids:
  `vak-agent/src/task.rs:883`, `vak-flow/src/planner.rs:388`,
  `vak-flow/src/exec.rs:522`, `vak/src/main.rs:988,1587`,
  `vak/src/tasks.rs:205`, `vak-core/src/tools_tasks.rs:40`,
  `vak-store/src/presentation.rs:80`. The checkpoint and webbrowse temp
  names and `vak-config/src/lib.rs:1860` are temp names, not ids; they stay
  but move to runtime in M3b.
- New `TestScope` helper in `crates/vak-config` (test-only feature)
  wrapping `isolate_home_for_tests`.

**Docs**
- New `docs/reference/records.md` (generated). AGENTS.md: new TraceKey
  invariant.

**Tests**
- `every_ledger_row_type_is_traced`, `broker_protocol_carries_trace`,
  `bus_envelope_trace_is_run_trace`, `bus_prev_hash_is_hash`.
- `session_header_names_cause_for_each_surface`,
  `derived_writes_record_provenance`, `records_reference_is_current`.

## M2 — vak-storage

**Code**
- New crate `crates/vak-storage`: `objects`, `records`, `refs`, `keys`,
  `store`, `remote` (trait only).
- Workspace `Cargo.toml` pins:
  - new: `zstd`
  - reused: `ring` (AEAD)
  - moved into the workspace manifest with exact versions: `rusqlite`
    (currently `vak-store/Cargo.toml:9`), `async-nats` (currently
    `vak-bus/Cargo.toml:22`)
  - dev: `cargo-fuzz` targets under `crates/vak-storage/fuzz`.

**Tests**
- Property, crash-point and fuzz suites in the new crate only. No other
  crate changes behaviour.

## M3a — Scope API on the current layout (no behaviour change)

**Code**
- `crates/vak-config/src/paths.rs`: add `Scope`/`StorageHandle` accessors
  that resolve to *today's* paths.
- Replace all 200 `sessions_home()` and 69 `shared_data_home()` calls, and
  the remaining home identifiers (552 uses in 65 files; the largest are
  `vak-server/src/lib.rs` 158, `vak-core/src/lib.rs` 72,
  `vak-core/src/checkpoints.rs` 37, `vak/src/main.rs` 27,
  `vak-server/src/admin.rs` 24, `vak-ops/src/services.rs` 20,
  `vak-core/src/commitments.rs` 16, `vak-server/src/gateway.rs` 16,
  `vak-core/src/health.rs` 13, `vak-server/src/delivery.rs` 12,
  `vak/src/install/mod.rs` 10) with typed accessors such as
  `scope.records().sessions()` and `scope.shared().gateway()`.
- Remove `Core::sessions_home`/`shared_data_home`/`set_sessions_home` at the
  end of M3a (invariant 30).

**Tests**
- Migrate all 500 layout references in 66 test files to `TestScope`.
  Behaviour is byte-identical, and the whole existing suite is the oracle.

## M3b — 5.0.0 layout switch

**Code: storage roots and writers** (each row of doc 73 Appendix A)
- `crates/vak-config/src/paths.rs`: `tenant_home`, `runtime_dir`; remove
  `agent_home`, `agent_workspace`, `gateway_workspace*`. Every Scope
  accessor now resolves to the new tree.
- `crates/vak-session/src/log.rs`: `SessionLog` on record segments.
  `SessionPath` and `hash_cwd` identity go (10 uses in 7 files:
  `vak-core/src/{entities,memory,learning,reflection}.rs`,
  `vak-session/src/log.rs`, `vak-server/src/admin.rs`, `vak/src/memory.rs`).
- `crates/vak-core/src/checkpoints.rs`: manifests reference objects; drop
  the per-session blob directory and `MAX_STORED_CHECKPOINTS` (policy
  moves to M7).
- `crates/vak-server/src/lib.rs` `append_session_sandbox_event` (`:10716`),
  `projection.rs:217,390`: stream chunks to objects; the JSONL keeps only
  lifecycle events.
- Tool results over the threshold go to objects: `vak-agent` loop,
  `vak-session` `EvidenceBodyRecord`, `vak-tools` `recall`.
- `TurnCapabilitiesBound` becomes an object reference (`vak-core/src/lib.rs:6658`).
- Presentation library (`vak-store/src/presentation.rs`) becomes a Document
  store; `presentations.json` callers at `vak-server/src/lib.rs:3072,3746,9393`.
- FinOps: `crates/vak-core/src/finops.rs` becomes segment chains; remove the
  compaction rewrites (`:22-30`, `:600-610`).
- Outbox and deliveries: `crates/vak-delivery/src/outbox.rs:205`,
  `crates/vak-server/src/delivery.rs:261,785`; a settled job is sealed.
- Commitments, inbox, routing, misread, security and operations ledgers move
  to record chains.
- Memory, entities, skill proposals, prompt layers and skills become
  Document class (`memory.rs`, `entities.rs`, `learning.rs`, `reflection.rs`,
  `prompts.rs:789`, `skills.rs`).
- Feeds: `crates/vak-server/src/feeds.rs` passes a tenant path;
  `scripts/feeds/*`.
- `workspaces.json` (`vak-core/src/workspaces.rs:57`),
  `workspace-names.json` (`vak-server/src/admin.rs:883`) and `trusted/`
  (`vak-core/src/trust.rs:44`) become the Spaces store.
- Credential scopes: `crates/vak-config/src/credentials.rs:51-60`
  (`scope_key_for` path hash) are keyed by `(tenant, space|agent)` ids.
  Callers: `vak-core/src/lib.rs:10287` (agent `.env` hint) and every
  `read_env_file_var` hint path.

**Code: runtime state out of the project tree (L4)**
- `crates/vak-tools/src/bash.rs:70-77,128-130`: scratch, temp and caches go
  to `runtime/executions/<exe>` and the cache root.
- `crates/vak-tools/src/office_apply.rs` (drafts), `crates/vak-agent/src/task.rs`,
  `crates/vak-agent/src/stop_policy.rs`: `.vak` literals.
- `crates/vak-core/src/worktree.rs:50`: `environments/<run>`.
- `crates/vak-config/src/paths.rs:91-97`: Agent workspaces become
  environments of kind `agent`.
- `crates/vak-sandbox/src/lib.rs` and `landlock.rs`, plus the Seatbelt
  profile generator in `vak-tools`: grant the execution directory and space
  root only.
- `crates/vak-tools/src/glob.rs` (the `.vak` exclusion), and the remaining
  `.vak` literals (70 in 28 files) reduce to the intent set.

**Code: identity by space id** (R12)
- `crates/vak-server/src/core_pool.rs`: pool identity by space id.
- `crates/vak-server/src/gateway.rs`: allowlist entry `workspace` fields
  become a space id; the `gateway/default-workspace` file
  (`paths.rs:101-130`) becomes a ref.
- `crates/vak-core/src/tasks.rs`: `TaskDef.cwd` becomes `space`; the
  scheduler filter at `vak-server/src/lib.rs:17863` changes with it.
- `crates/vak-server/src/web.rs`: `/workspaces{,/open,/forget}`, `/fs/dirs`
  roots from `[server] workspace_roots`.
- `crates/vak-desktop/src/main.rs`: trust gate, logs menu
  (`desktop.{prefix}.log`), PTY cwd.
- `crates/vak-tray/src/main.rs:460`: `locks/` moves to runtime.

**Code: purge, state registry, install**
- `crates/vak-core/src/state.rs`: a registry of classes × roots.
  `crates/vak-core/tests/state_registry.rs` is driven by the §6 matrix.
- `crates/vak/src/install/mod.rs:1090-1135`: purge removes Vak-owned roots
  wholesale and uses declared entries for Shared only.
- `crates/vak/src/setup.rs:645-674`: the upgrade gate uses class snapshots.
- `crates/vak-core/src/backup.rs`: interim object-aware export (full rework
  in M7).
- Refusal of pre-5.0.0 state: data home, install manifest
  (`vak-core/src/install.rs:30`), gateway store and config, all through the
  one invariant-29 message.
- Version stamps (`scripts/check-version.sh` targets): README badge,
  CHANGELOG, workspace `Cargo.toml`.

**Services**
- `crates/vak-ops/src/services.rs:178-194`: `bots.json` path; units keep
  `HOME` and `WorkingDirectory` (invariant 18), and log names change to
  `vak-<service>.jsonl` in M5.
- `crates/vak-ops/src/lib.rs:427-428`: log file names.
- `scripts/install_gateway_service.sh`, `scripts/linux-stack.sh`,
  `scripts/linux-check.sh`: path references.

**UI**
- `crates/vak-client-ui/src/App.tsx:903`: the `.vak/scratch` directory
  check. `host/web.ts` and `api.ts`: `/workspaces`.
- Workbench artifact paths are shown as execution-relative, never as a
  project path.
- The admin forensics (`SessionForensics.tsx`) project-hash grouping becomes
  a space.

**CLI** (`crates/vak/src/cli.rs:32` subcommands whose storage moves)
- `Sessions`, `Export`, `Checkpoints`, `Memory`, `Entities`, `Agents`,
  `Tasks`, `Inbox`, `Backup`, `Digest`, `Self` (state, verify, uninstall
  `--purge`), `Workspace`, `Doctor`, `Serve`, `Setup`.

**Docs** (current path citations; counts from a grep of layout strings)
- Must change:
  - CHANGELOG.md (20), 46-stabilization (19), AGENTS.md (17)
  - 32-release-engineering (8), 64-agent-owned-platform (7), README.md (7)
  - hosting.md (6), 05-config (6), 45-prompt-layers (4), 33-admin-console (4)
  - development.md (3), 39-plugin-ecosystem (3), 15-reliability (3),
    14-checkpoints (3)
  - 72, 54, 44, 35, 00 (2 each), 70 and 62 (1 each)
  - `docs/architecture/write-paths-and-growth.html` (29; re-measure).
- Historical audits and research keep their paths as dated records.

**Site**
- `crates/vak-server/site/src/pages/security.html`, `install.html`,
  `site.js`: data-location statements. Rebuild with `build.py`.

**Tests**
- New: `bytes_per_turn_budget`, `fsyncs_per_turn_budget`,
  `no_undeclared_paths_any_root`, `pre_baseline_home_refused_with_one_message`,
  `sandbox_writes_only_execution_dir_and_space`.
- New: `derive_messages_identical_across_seal`,
  `secret_scopes_keyed_by_id`, `purge_removes_owned_roots_wholesale`.

## M4 — runs and schedules

**Code**
- New record chain `runs/`, plus `RunRecord` in `vak-session` or `vak-core`.
- `crates/vak-server/src/lib.rs`: `scheduler_tick` (`:17845`),
  `catch_up_missed_tasks`, `fire_task`, `fire_script_task`, `run_task_now`
  (`:17381`) and `advance_marker` become one `due_slots` plus a slot claim.
  The `next_fire` in-memory map is removed.
- `crates/vak-core/src/tasks.rs`: `TaskDef` loses the `last_*` fields
  (`:222-235`) and gains `on_crash`.
- `crates/vak-server/src/heartbeat.rs`, `crates/vak-flow` (`flow-runs/`),
  `crates/vak-agent/src/task.rs` (delegation) and best-of-N all create Runs.
- New `EnvironmentBackend` impl `CopyEnvironment` in `crates/vak-sandbox`.
- The Library prototype (added 2026-10-01 with `docs/design/82-library.md`):
  its rule that lists each routine run's declarations as entries of their
  own goes; a routine's runs become versions of one artifact at their
  destination path (doc 82 §2.1, L4).

**UI**
- `crates/vak-client-ui/src/components/TasksModal.tsx` and `types.ts`:
  last-run fields become a run query.
- `crates/vak-admin-ui/src/OperationsCenter.tsx:255,406,418,530,542` and
  `Home.tsx:458`: `#/operations/work/runs/<session_id>` becomes
  `#/runs/<run_id>`.
- `crates/vak-admin-ui/src/App.tsx` scheduled-task editor (`:1491,1526,1652,1675`).
- `crates/vak-terminal`: run status in the session views.

**API**
- `/tasks`, `/tasks/{id}/run-now`, `/tasks/{id}/retry-delivery`.
- New `/runs`, `/runs/{id}`, `/schedules/{id}/slots`.

**Docs**
- 29-personal-os (scheduler P2), 22-gateway, 64 (scheduled-task ownership).

## M5 — telemetry

**Code**
- Workspace pins: `tracing`, `tracing-subscriber` (json, env-filter);
  optional `tracing-opentelemetry`, `opentelemetry-otlp`.
- Replace the 94 library/server `eprintln!`:
  - vak-server 68 (`lib.rs` 23, `surfaces/telegram.rs` 15, `gateway.rs` 7,
    `delivery.rs` 6, `heartbeat.rs` 3, `surfaces/{discord,slack}.rs` 3 each,
    `web.rs` 2, others 1 each)
  - vak-core 9, vak-sandbox 7 (`landlock.rs`), vak-desktop 7,
    vak-tools 2 (`broker.rs`), vak-llm 1 (`openai.rs`)
- `clippy.toml` `disallowed-macros` for library crates.
- `crates/vak-ops/src/lib.rs:427-428`, `services.rs:138,277`: JSON log
  files with rotation.
- `crates/vak-bus`: payloads become references; stream `max_age`.

**UI**
- An admin System › Diagnostics trace view.

**Tests**
- `library_crates_have_no_eprintln`, `one_run_one_trace_id`,
  `log_lines_are_json_with_trace_fields`, `telemetry_carries_no_content`.

## M6 — catalog, search, lineage

**Code**
- New `crates/vak-catalog`; delete `crates/vak-store`. Its callers:
  `vak-server` (2 production uses and 18 test uses of `vak_store::`),
  `vak-core` index spawns, admin search, `search_all`.
- The Library prototype's declarations table goes with `vak-store`; Library
  search moves to the catalog (`docs/design/82-library.md` §9, L3).
- Remove scanning lookups: `find_session_on_disk` (`vak-server/src/lib.rs:7988`),
  `read_historical_header` (`:8039`), `find_session_in_cwd`, and the recall
  ledger cache in `vak-tools`.
- Flat turn path:
  - `vak-core/src/routing.rs:68` (`snapshot` full scan)
  - `vak-session/src/log.rs:44` (`has_request_admission` scan)
  - `vak-commit/src/ledger.rs:186-300` (replay per append)

**API**
- New `/search`, `/lineage/{node}`, `/nodes/{id}`. `/admin/api/search` and
  `/search` merge into one ACL-filtered implementation.

**UI**
- Admin Search (`App.tsx` `#/search`) and client search call the one API.
  A lineage tab is added in forensics.

**Tests**
- `lineage_from_any_artifact_to_cause`, `search_respects_audience`,
  `catalog_rebuild_equals_incremental`, `turn_path_reads_flat`,
  `catalog_query_p95_under_50ms_at_1m_nodes`.

## M7 — lifecycle, retention, erasure, hold, backup

**Code**
- New `crates/vak-lifecycle`.
- Remove the scattered retention (invariant 30): `checkpoints.rs:52,485`,
  `finops.rs:22-30,600-610`, `memory.rs:57` (`cleanup_artifacts`), and the
  `/memory/cleanup` route.
- Session archive and delete: `vak-server/src/lib.rs:742-744,9056-9210`
  (`archive.json`, `deleted.json` sidecars) become lifecycle state refs,
  trash, and erasure requests.
- Backup: `crates/vak-core/src/backup.rs`, `crates/vak/src/backup.rs`,
  `/backup/export|import` become ciphertext plus wrapped keys, with a
  tombstone replay on restore.
- Agent lifecycle: `crates/vak-server/src/agents.rs:15` gains `Revoked`,
  with data effects wired to the lifecycle.
- Allowlist PII retention: `crates/vak-server/src/gateway.rs`.

**API**
- New `/data/*` (doc 74 §7).

**UI**
- Admin: Storage, Lifecycle, Integrity, Retention & holds, Erasure
  requests, Keys, Audit export, Backup.
- Client: Trash, conversation menu, "Why is this gone?", the Settings
  storage page (doc 74 §6).

**CLI**
- `vak data status|gc|verify|export|erase|hold|cat|grep`. `vak backup` is
  reworked. `vak doctor --repair` hooks.

**Docs**
- 23-memory (retention of notes), 14-checkpoints, 28-operations,
  33-admin-console, 44-shared-config (keys), 46 Part VII.

**Tests**
- `reconciler_is_idempotent`, `settled_execution_leaves_nothing`,
  `gc_keeps_everything_reachable`, `erasure_follows_lineage`.
- `erasure_leaves_ledger_bytes_unchanged`,
  `hold_blocks_every_destructive_transition`, `quota_enforced`.
- `restore_reapplies_erasures`, `thirty_day_soak_stays_within_budget`.

## M8 — artifacts, sharing, information architecture

**Code**
- Artifact and Version records.
- `crates/vak-sandbox` candidates and promotions become artifact version
  events.
- `crates/vak-server/src/coworking.rs` grants become one grants table.
- `crates/vak-tools/src/office_apply.rs` and `doc_read.rs`: a draft is an
  artifact version.
- The Library prototype (`docs/design/82-library.md` L1/L2): its projection
  over `sandbox/records.jsonl` and the session ledgers, its path-derived
  keys and the key in `AttachedArtifact` are replaced by Artifact records
  and `art_` ids. Nothing maps across, because M3b discarded their data.

**UI**
- Client: Library, share dialog, version history, Runs panel.
  `SharedConversation.tsx`, `ArtifactCanvas`, `OfficeRedline` and
  `WorkbenchPanel` move to the Artifact API.
- Admin: navigation reconciliation (doc 74 §6.1) across the Overview, Work,
  Operate, Configure and System groups in `App.tsx:6862-6945`.

**Docs**
- 54, 69 and 72 sections superseded; 33 and 58 navigation; 82's notes on
  what holds before M8.

## M9 — cloud remote

**Code**
- `crates/vak-storage/src/remote/` (`FileRemote`; `S3Remote` plus Postgres
  refs behind a feature); `vak sync` CLI; the sync plane in `vak-server`
  (doc 31 contract); leases.

**External**
- The hosted service is out of scope; the key-escrow design is a separate
  doc before any hosted backend.

**Docs**
- 56 superseded, 31 extended, 53 (bus subject `workspace_id` becomes a
  space id).

## External boundaries (every milestone)

| Boundary | Effect |
|---|---|
| Channel bridges (Telegram, Discord, Slack) | Inbound payloads gain a TraceKey through `InboundRequest`. Delivery provenance is unchanged. Copies already sent to a platform are outside erasure and are named in the receipt. |
| LLM providers | Prompts are sent to a processor. The erasure receipt names the provider and model per turn from `WorkReceipt`. |
| NATS (when configured) | Subjects move to space ids. Payloads are references only. Stream `max_age` is set. |
| Backups taken before M7 | They are plaintext; at 5.0.0 they are refused like any other pre-baseline state. |
| OS keychain / encrypted-file store | Hosts the tenant KEK (M2); rotation and escrow land in M7. |
