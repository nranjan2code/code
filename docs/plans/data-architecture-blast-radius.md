# Blast radius — data architecture plan

Status: **inventory, revision 3 (2026-10-01)**, re-scanned on `main` at
`438cfcd5` (first taken 2026-09-25 at `767db1d0`). It lists everything each
milestone of `docs/plans/data-architecture-plan.md` (revision 3) changes:
code, HTTP API, UI, CLI, configuration, services, docs, scripts, the site,
tests and external boundaries. Re-run the scans in §0 at the start of each
milestone; a milestone is not done while its section still names something
unchanged.

## 0. Method and totals

Production code is every `.rs` file under `crates/` outside `tests/`
directories, with `#[cfg(test)]` items removed by brace matching and `//`
comment lines skipped. Test code is the rest. The patterns are written out
below so the next run is comparable; the home-path ratchet (plan §4, "Now")
checks the first five in.

| Measure | Pattern | `767db1d0` | `438cfcd5` |
|---|---|---|---|
| `sessions_home()` calls | `sessions_home\(\)` | 200 in 20 files | **213 in 23 files** (vak-server/lib.rs 104, vak-core/lib.rs 37, vak/main.rs 15, vak-server/admin.rs 14) |
| `shared_data_home()` calls | `shared_data_home\(\)` | 69 in 10 files | **84 in 13 files** (vak-server/lib.rs 44, gateway.rs 10, vak-core/lib.rs 7) |
| home identifiers | `\b(sessions_home\|shared_data_home\|data_home\|default_workspace\|cache_home\|logs_dir\|agent_home\w*\|agent_workspace)\b` | 552 in 65 files (narrower set) | **626 in 69 files** (vak-server/lib.rs 167, vak-core/lib.rs 77, vak-core/checkpoints.rs 37, vak-server/admin.rs 31, vak/main.rs 28, vak-ops/services.rs 26, vak-core/commitments.rs 21, vak-server/gateway.rs 19) |
| `.vak` path literals | `"\.vak[/"]` | 70 in 28 files | 67 in 29 files |
| `hash_cwd(` | `hash_cwd\(` | 10 in 7 files | **13 in 8 files** |
| clock-derived or truncated ids | `as_nanos\(\)\|\[\.\.8\]` | 13 in 11 files | 14 in 12 files |
| `eprintln!`, library and server | `eprintln!` | 94 | 95 (vak-server 67, vak-core 11, vak-sandbox 7, vak-desktop 7, vak-tools 2, vak-llm 1); plus 278 in the CLI and terminal, which are user output |
| layout references in tests | `set_sessions_home\|sessions_home\(\)\|shared_data_home\(\)\|join\("sessions"\)\|\.vak/\|hash_cwd\|isolate_home_for_tests\|set_home_override` | 500 in 66 files (a different set) | **529 in 80 files** (vak-server 360, vak-core 115, vak-config 15, vak-tools 13, vak 10, vak-agent 6) |

The 2026-09-25 figures for home identifiers and test references used
narrower pattern sets that were not recorded, so only the first two rows
and `hash_cwd` compare exactly: about 10% more raw home-path calls in six
days (review 2, R41).

## 1. Cross-cutting surfaces at a glance

| Surface | Now | M1 | M2 | M3a | M3b | M4 | M5 | M6 | M6.5 | M7a | M7b | M8 | M9 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| vak-session | | ● | ● | ● | ● | ● | | ● | | ● | | | ● |
| vak-storage (new) | | | ● | ● | ● | ● | | ● | ● | ● | ● | ● | ● |
| vak-catalog (replaces vak-store) | | | | | | | | ● | ● | ● | ● | ● | ● |
| vak-lifecycle (new) | | | | | | | | | | ● | ● | | ● |
| vak-core | ● | ● | | ● | ● | ● | ● | ● | ● | ● | ● | ● | |
| vak-server | | ● | | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |
| vak-agent / vak-tools / worker protocol | | ● | | ● | ● | | ● | | ● | | | | |
| vak-sandbox | | ● | | | ● | ● | ● | | | ● | | ● | |
| vak-config (paths, credentials) | ● | ● | ● | ● | ● | | ● | | | ● | ● | | ● |
| vak-delivery / vak-commit / vak-flow / vak-bus | | ● | | ● | ● | ● | ● | ● | | ● | | | ● |
| vak (CLI, install, setup, backup) | | | | ● | ● | ● | ● | ● | | ● | ● | | ● |
| vak-ops / vak-tray / vak-desktop | | | | ● | ● | | ● | | | ● | | | |
| vak-client-ui | | | | | ● | ● | | ● | ● | ● | ● | ● | ● |
| vak-admin-ui | | | | | ● | ● | ● | ● | ● | ● | ● | ● | ● |
| vak-terminal | | | | | ● | ● | | ● | | | | | |
| scripts (feeds, service, linux) | | | | | ● | | | | ● | | | | |
| site (`crates/vak-server/site`) | | | | | ● | | | | | ● | | | |
| AGENTS.md invariants | ● | ● | ● | | ● | ● | ● | ● | ● | ● | | ● | ● |
| design docs | | ● | | | ● | ● | ● | ● | ● | ● | ● | ● | ● |

## M0 — fixes now (4.x) — done 2026-09-25, shipped in 5.0.0

Kept as the record of what M0 changed.

**Code**
- `crates/vak-server/src/agents.rs`: delete `AgentSchedule`, the
  `schedule` field on `AgentDefinition`, `AgentRunRecord`, `record_run`,
  `list_runs`, `runs_path`, `update_schedule` and their tests.
- `crates/vak-server/src/lib.rs`: routes `/agents/{id}/schedule` and
  `/agents/{id}/runs`; `fire_task` (full run ids, the git/non-git branch,
  refusal notifications); `advance_marker` call sites; `spawn_isolated_run`
  (handle id equals the ledger id, `set_sessions_home(shared_data_home())`);
  `/config/bus` PUT and DELETE to the credential store; every session read
  path honours trash.
- `crates/vak-server/src/admin.rs`: the admin sessions list, admin search
  and forensics honour trash.
- `crates/vak-core/src/session_search.rs`, `crates/vak-session/src/search.rs`,
  `crates/vak-store/src/query.rs`: filter trashed sessions (one source,
  `vak_core::trash`).
- `crates/vak-core/src/inbox.rs`: new `Kind::RoutineFailed`.
- `append_turn_capabilities`: an unchanged binding is a reference entry.
- `crates/vak/src/agents_cli.rs`: the schedule and run flags removed.
- `crates/vak/src/install/mod.rs`: `Root::Logs` in `purge_state`.
- `crates/vak-server/src/feeds.rs` and `scripts/feeds/feed_utils.py`:
  `VAK_FEEDS_DB`.

**UI, docs and tests** as listed in the plan's M0.

## Now — two guards on 5.x

**Code**
- `crates/vak-core/src/state.rs:139`: the single `agents` entry becomes
  one entry per subpath under an `agents/{agent}/` pattern (sessions,
  checkpoints, memory, skill-proposals, entities, sandbox, office-workspaces,
  coworking, presentations.json, flow-runs, commitments, routing, intent and
  security evidence, activity log, agent-network), each with its real kind.
- `crates/vak-core/tests/state_registry.rs`: fails on an unknown subpath
  under any Agent home.
- A new ratchet test in `crates/vak-core/tests/` with the §0 counts checked
  in beside it.

**Docs**
- AGENTS.md "Pending": the two guards in "don't deepen the debt".

**Tests**
- `home_path_uses_do_not_grow`, `agent_home_subpaths_are_declared`.

## M1 — ids, TraceKey, principals, provenance

**Code**
- New `crates/vak-session/src/ids.rs` (with `RunId` and `PrincipalId`
  first) and `trace.rs`.
- `crates/vak-session/src/types.rs`: `SessionHeader` gains `space`, `run`,
  `cause`. `Entry` stays the same; its payloads carry ids.
- Principals:
  - `crates/vak-server/src/auth_identity.rs`: the owner record's id becomes
    a `prn_` id.
  - `crates/vak-server/src/coworking.rs`: invitation principals and
    participant messages carry `prn_` ids.
  - `crates/vak-server/src/gateway.rs`: the resolved channel sender gets a
    `prn_` id with the allowlist entry.
  - `crates/vak-sandbox/src/lib.rs:611` (`CandidateRecord`), Office room
    revisions (`crates/vak-server/src/office_workspace.rs`) and
    `CandidateComment` activities: an `actor`.
- `crates/vak-tools/src/context.rs`: `ToolContext.agent_id` becomes
  `trace`. Update every construction site (`execute_script`, the agent loop,
  workers).
- `crates/vak-tools/src/broker.rs` and `crates/vak-server/src/bin/vak-tool-worker.rs`:
  protocol version bump, carrying the key.
- `crates/vak-tools/src/sandbox_events.rs` (`SandboxEventSink`);
  `crates/vak-sandbox/src/lib.rs` records (`EnvironmentRecord`,
  `PromotionRecord` and `PreviewPreparationRecord` gain the key).
- `Traced` rows for every side ledger:
  - `crates/vak-core/src/finops.rs` (`CostRow`, `ActivityRow`,
    `BudgetAlertRow`)
  - `crates/vak-core/src/routing.rs` (`EvidenceRow`)
  - `crates/vak-core/src/misread.rs`
  - `crates/vak-core/src/security_events.rs`
  - `crates/vak-core/src/inbox.rs`
  - `crates/vak-commit/src/ledger.rs`
  - `crates/vak-server/src/operations.rs` (incidents and actions)
  - `crates/vak-server/src/coworking.rs` (grants)
  - `crates/vak-server/src/delivery.rs` (deliveries rows, outbox jobs)
  - `crates/vak-core/src/checkpoints.rs` (manifest; label → turn id)
- Provenance (`derived_from`) on derived writes:
  `crates/vak-core/src/memory.rs`, `learning.rs`, `reflection.rs`,
  `entities.rs`.
- `crates/vak-server/src/bus.rs`: pass the run trace; real `prev_hash`.
- The clock-derived or truncated ids (§0) become typed ids:
  `vak-agent/src/task.rs`, `vak-flow/src/planner.rs`, `vak/src/main.rs`,
  `vak/src/tasks.rs`, `vak-core/src/tools_tasks.rs`,
  `vak-store/src/presentation.rs`, `vak-server/src/gateway.rs`,
  `vak-sandbox/src/docker.rs`. The checkpoint and webbrowse temp names and
  `vak-config/src/lib.rs` are temp names, not ids; they move to runtime in
  M3b.
- Reliable-work plan E1 (`vak-core/src/finops.rs`, the Core spend gate,
  `vak-llm/src/work.rs`): its root work account is keyed by `RunId`.
- New `TestScope` helper in `crates/vak-config` (test-only feature)
  wrapping `isolate_home_for_tests`.

**Docs**
- New `docs/reference/records.md` (generated). AGENTS.md: new TraceKey and
  actor invariant.

**Tests**
- `every_ledger_row_type_is_traced`, `every_ledger_row_type_names_its_actor`,
  `broker_protocol_carries_trace`, `bus_envelope_trace_is_run_trace`,
  `bus_prev_hash_is_hash`.
- `session_header_names_cause_for_each_surface`,
  `derived_writes_record_provenance`, `records_reference_is_current`,
  `root_work_account_keyed_by_run_id`.

## M2 — vak-storage

**Code**
- New crate `crates/vak-storage`: `objects`, `records`, `refs` (generation
  and writer epoch), `keys` (`KeyAuthority`), `documents`, `store` (commit
  generation and pre-acknowledgement hook), `remote` (trait only).
- `crates/vak-config/src/credentials.rs`: the local `KeyAuthority`
  implementation.
- Workspace `Cargo.toml` pins:
  - new: `zstd`
  - reused: `ring` (AEAD)
  - moved into the workspace manifest with exact versions: `rusqlite`
    (`vak-store/Cargo.toml:9`), `async-nats` (`vak-bus/Cargo.toml:22`),
    `webauthn-rs` (`vak-server/Cargo.toml:46`)
  - dev: `cargo-fuzz` targets under `crates/vak-storage/fuzz`.

**Tests**
- Property, crash-point and fuzz suites in the new crate only. No other
  crate changes behaviour. `chain_verifies_without_keys`,
  `chain_verifies_after_shred`, `compressed_before_encrypted`,
  `stale_epoch_cannot_commit`, `restore_bumps_epoch`,
  `key_authority_unavailable_fails_closed`.

## M3a — Scope API on the current layout (no behaviour change)

**Code**
- `crates/vak-config/src/paths.rs`: add `Scope`/`StorageHandle` accessors
  that resolve to *today's* paths.
- Replace all 213 `sessions_home()` and 84 `shared_data_home()` calls, and
  the remaining home identifiers (§0; the largest files are
  `vak-server/src/lib.rs`, `vak-core/src/lib.rs`,
  `vak-core/src/checkpoints.rs`, `vak-server/src/admin.rs`,
  `vak/src/main.rs`, `vak-ops/src/services.rs`,
  `vak-core/src/commitments.rs`, `vak-server/src/gateway.rs`,
  `vak/src/install/mod.rs`, `vak-core/src/health.rs`,
  `vak-server/src/delivery.rs`) with typed accessors such as
  `scope.records().sessions()` and `scope.shared().gateway()`.
- The D25 call sites get an accessor that names the session's Agent while
  still resolving to today's path: `sandbox_records_path`,
  `sandbox_candidates_root`, `session_sandbox_events_path`
  (`vak-server/src/lib.rs:11052-11082`), the coworking store
  (`lib.rs:7962`, `:8142`, `:8203`) and the Office rooms
  (`vak-server/src/office_workspace.rs:136,285`).
- Remove `Core::sessions_home`/`shared_data_home`/`set_sessions_home` and
  the home-path ratchet at the end of M3a (invariant 30).

**Tests**
- Migrate every §0 test layout reference to `TestScope`. Behaviour is
  byte-identical, and the whole existing suite is the oracle.

## M3b — the data baseline, in six slices

The release train is plan L6: main is the baseline's line from slice 1;
fixes for 5.x go on `release/5`.

### Slice 1 — baseline, purge, runtime root, registry

- `crates/vak-config/src/paths.rs`: `tenant_home`, `runtime_dir`.
- `crates/vak-core/src/baseline.rs`, `crates/vak-core/src/install.rs`, the
  gateway store and config: refuse pre-baseline state with the one
  invariant-29 message.
- `crates/vak-core/src/state.rs`: a registry of classes × roots.
  `crates/vak-core/tests/state_registry.rs` is driven by the plan §6 matrix.
- `crates/vak/src/install/mod.rs`: purge removes Vak-owned roots wholesale
  and uses declared entries for Shared only.
- `crates/vak/src/setup.rs`: the upgrade gate uses class snapshots.
- Version stamps (`scripts/check-version.sh` targets): README badge,
  CHANGELOG, workspace `Cargo.toml`.
- Tests: `pre_baseline_home_refused_with_one_message`,
  `purge_removes_owned_roots_wholesale`.

### Slice 2 — sessions on segments, slim ledgers

- `crates/vak-session/src/log.rs`: `SessionLog` on record segments, keyed by
  TraceKey. `SessionPath` and `hash_cwd` identity go (13 uses in 8 files:
  `vak-server/src/lib.rs`, `vak-session/src/log.rs`,
  `vak-core/src/{memory,entities,reflection,learning}.rs`,
  `vak-server/src/admin.rs`, `vak/src/memory.rs`).
- Tool results over the threshold go to objects: `vak-agent` loop,
  `vak-session` `EvidenceBodyRecord`, `vak-tools` `recall`.
- `TurnCapabilitiesBound` becomes an object reference.
- `crates/vak-server/src/lib.rs` `append_session_sandbox_event`,
  `projection.rs`: stream chunks to objects; the JSONL keeps only lifecycle
  events.
- `crates/vak-core/src/checkpoints.rs`: manifests reference objects; drop
  the per-session blob directory and `MAX_STORED_CHECKPOINTS` (policy moves
  to M7a).
- Tests: `bytes_per_turn_budget`, `fsyncs_per_turn_budget`,
  `derive_messages_identical_across_seal`.

### Slice 3 — side ledgers, Documents, D25

- FinOps: `crates/vak-core/src/finops.rs` becomes segment chains; remove the
  compaction rewrites.
- Outbox and deliveries: `crates/vak-delivery/src/outbox.rs`,
  `crates/vak-server/src/delivery.rs`; a settled job is sealed.
- Commitments, inbox, routing, misread, security and operations ledgers move
  to record chains.
- Memory, entities, skill proposals, prompt layers and skills become
  Document class (`memory.rs`, `entities.rs`, `learning.rs`, `reflection.rs`,
  `prompts.rs`, `skills.rs`); the presentation library
  (`vak-store/src/presentation.rs`) becomes a Document store; Office rooms
  (`office_workspace.rs`) become Documents.
- Desired state (config layers, bots, allowlist, tasks) is versioned through
  the Document helper.
- D25: sandbox records, candidates, execution streams, coworking grants and
  Office rooms live in their session's Agent scope.
- `crates/vak-server/src/auth_identity.rs`: `auth/` becomes tenant Desired.
- Feeds: `crates/vak-server/src/feeds.rs` passes a tenant path (the store
  itself is replaced in M6.5).
- Tests: `agent_records_live_in_their_agent_scope`,
  `no_undeclared_paths_any_root`.

### Slice 4 — runtime out of the project tree (L4, L10)

- `crates/vak-tools/src/bash.rs`: scratch, temp and caches go to
  `runtime/executions/<exe>` and the cache root.
- `crates/vak-tools/src/office_apply.rs` (drafts), `crates/vak-agent/src/task.rs`,
  `crates/vak-agent/src/stop_policy.rs`: `.vak` literals.
- `crates/vak-core/src/worktree.rs`: `environments/<run>`.
- `crates/vak-config/src/paths.rs` `agent_workspace`: a non-built-in Agent's
  workspace becomes `workspaces/<spc>/<agt>/`, a Workspace bound to (Space,
  Agent), never an Environment.
- `crates/vak-sandbox/src/lib.rs` and `landlock.rs`, plus the Seatbelt
  profile generator in `vak-tools`: grant the execution directory, the
  Agent workspace and the space root only.
- `crates/vak-tools/src/glob.rs` (the `.vak` exclusion), and the remaining
  `.vak` literals (§0) reduce to the intent set.
- `crates/vak-client-ui/src/App.tsx`: the `.vak/scratch` directory check
  goes; Workbench artifact paths are shown as execution-relative.
- Tests: `sandbox_writes_only_execution_dir_and_space`,
  `agent_workspace_is_not_an_environment`.

### Slice 5 — identity by space id (review R12)

- `crates/vak-config/src/credentials.rs` (`scope_key_for` path hash) keyed
  by `(tenant, space|agent)` ids, with every `read_env_file_var` hint path.
- `crates/vak-server/src/core_pool.rs`: pool identity by space id.
- `crates/vak-server/src/gateway.rs`: allowlist entry `workspace` fields
  become a space id; the `gateway/default-workspace` file becomes a ref.
- `crates/vak-core/src/tasks.rs`: `TaskDef.cwd` becomes `space`; the
  scheduler filter changes with it.
- `crates/vak-core/src/workspaces.rs`, `vak-server/src/admin.rs`
  (`workspace-names.json`) and `crates/vak-core/src/trust.rs`: the Spaces
  store.
- `crates/vak-server/src/web.rs`: `/workspaces{,/open,/forget}`, `/fs/dirs`
  roots from `[server] workspace_roots`.
- `crates/vak-desktop/src/main.rs`: trust gate, logs menu, PTY cwd.
- `crates/vak-tray/src/main.rs`: `locks/` moves to runtime.
- Client `host/web.ts` and `api.ts`: `/workspaces`. Admin
  `SessionForensics.tsx` groups by space; admin Spaces (A13).
- Tests: `secret_scopes_keyed_by_id`.

### Slice 6 — docs, site, scripts, services, CLI

**Services**
- `crates/vak-ops/src/services.rs`: `bots.json` path; units keep `HOME` and
  `WorkingDirectory` (invariant 18); log names change in M5.
- `crates/vak-ops/src/lib.rs`: log file names.
- `scripts/install_gateway_service.sh`, `scripts/linux-stack.sh`,
  `scripts/linux-check.sh`: path references.

**CLI** (`crates/vak/src/cli.rs` subcommands whose storage moves)
- `Sessions`, `Export`, `Checkpoints`, `Memory`, `Entities`, `Agents`,
  `Tasks`, `Inbox`, `Backup`, `Digest`, `Self` (state, verify, uninstall
  `--purge`), `Workspace`, `Doctor`, `Serve`, `Setup`.
- `crates/vak-core/src/backup.rs`: interim object-aware export (full rework
  in M7a).

**Docs** (current path citations)
- CHANGELOG.md, 46-stabilization, AGENTS.md, 32-release-engineering,
  64-agent-owned-platform, README.md, hosting.md and `docs/hosting/`,
  05-config, 45-prompt-layers, 33-admin-console, development.md,
  39-plugin-ecosystem, 15-reliability, 14-checkpoints, 72, 54, 44, 35, 00,
  70, 62, 78 (owner record location), 82 (§9.2).
- `docs/architecture/write-paths-and-growth.html` (re-measure).
- Historical audits and research keep their paths as dated records.

**Site**
- `crates/vak-server/site/src/pages/security.html`, `install.html`,
  `site.js`: data-location statements. Rebuild with `build.py`.

Then release 6.0.0.

## M4 — runs, triggers, effects, fencing

**Code**
- New record chains `runs/` and `effects/`, plus `RunRecord` and
  `EffectRecord` in `vak-session` or `vak-core`.
- `crates/vak-core/src/tasks.rs`: `TaskDef` becomes `Trigger` with kinds;
  the `last_*` fields go; it gains `on_crash`.
- `crates/vak-server/src/lib.rs`: `scheduler_tick`, `catch_up_missed_tasks`,
  `fire_task`, `fire_script_task`, `run_task_now` and `advance_marker`
  become one `due(now)` plus a claim under the writer epoch. The
  `next_fire` in-memory map is removed.
- Effects: `crates/vak-delivery/src/outbox.rs` and
  `crates/vak-server/src/delivery.rs` become effect records of kind
  delivery, with idempotency keys and receipts.
- Cursors: the channel bridges' update offsets
  (`crates/vak-server/src/surfaces/telegram.rs:328` and the Discord and
  Slack bridges) become cursor refs.
- Fencing: session writers (`crates/vak-session/src/log.rs`), routine leases
  and channel pollers hold an epoch.
- `crates/vak-server/src/heartbeat.rs`, `crates/vak-flow` (`flow-runs/`),
  `crates/vak-agent/src/task.rs` (delegation) and best-of-N all create Runs.
- New `EnvironmentBackend` impl `CopyEnvironment` in `crates/vak-sandbox`.
- `crates/vak/src/tasks.rs`: the CLI becomes `vak triggers`.
- The Library prototype (`docs/design/82-library.md`): its rule that lists
  each routine run's declarations as entries of their own goes; a routine's
  runs become versions of one artifact at their destination path (doc 82
  §2.1, L4).

**UI**
- `crates/vak-client-ui/src/components/TasksModal.tsx` and `types.ts`:
  last-run fields become a run query.
- `crates/vak-admin-ui/src/OperationsCenter.tsx` and `Home.tsx`:
  `#/operations/work/runs/<session_id>` becomes `#/runs/<run_id>`.
- `crates/vak-admin-ui/src/App.tsx` scheduled-task editor becomes the
  Triggers screen (A5).
- `crates/vak-terminal`: run status in the session views.

**API**
- `/tasks`, `/tasks/{id}/run-now`, `/tasks/{id}/retry-delivery` become
  `/triggers…` and `/effects…`.
- New `/runs`, `/runs/{id}`, `/triggers/{id}/slots`, `/effects/{id}`.

**Docs**
- AGENTS.md invariant 38 and the new effects invariant; 29-personal-os
  (scheduler), 22-gateway, 64 (scheduled-task ownership); 76, 80 and 81
  cite the shipped shapes.

**Tests**
- `schedule_slot_at_most_once_under_restart`, `two_processes_do_not_double_start`,
  `abandoned_run_is_recorded`, `every_cause_writes_run`,
  `skipped_slot_is_a_record`, `non_git_space_routine_runs_in_copy_environment`.
- `effect_unknown_until_reconciled`, `effect_not_replayed_after_restart`,
  `restore_fences_old_writer`, `cursor_resync_records_gap`.

## M5 — telemetry

**Code**
- Workspace pins: `tracing`, `tracing-subscriber` (json, env-filter);
  optional `tracing-opentelemetry`, `opentelemetry-otlp`.
- Replace the 95 library/server `eprintln!` (§0): vak-server 67 (`lib.rs`,
  `surfaces/telegram.rs`, `gateway.rs`, `delivery.rs`, `heartbeat.rs`,
  `surfaces/{discord,slack}.rs`, `web.rs`), vak-core 11, vak-sandbox 7
  (`landlock.rs`), vak-desktop 7, vak-tools 2 (`broker.rs`), vak-llm 1.
- `clippy.toml` `disallowed-macros` for library crates.
- `crates/vak-ops/src/lib.rs`, `services.rs`: JSON log files with rotation.
- `crates/vak-bus`: payloads become references; stream `max_age`.

**UI**
- An admin System › Diagnostics trace view.

**Tests**
- `library_crates_have_no_eprintln`, `one_run_one_trace_id`,
  `log_lines_are_json_with_trace_fields`, `telemetry_carries_no_content`
  (field allowlist and planted canaries).

## M6 — data catalog, search, lineage

**Code**
- New `crates/vak-catalog`; delete `crates/vak-store`. Its callers:
  `vak-server`, `vak-core` index spawns, admin search, `search_all`.
- The Library prototype's declarations table goes with `vak-store`; Library
  search moves to the catalog (`docs/design/82-library.md` §9, L3).
- Remove scanning lookups: `find_session_on_disk`, `read_historical_header`,
  `find_session_in_cwd` (`vak-server/src/lib.rs`), and the recall ledger
  cache in `vak-tools`.
- Flat turn path: `vak-core/src/routing.rs` (`snapshot` full scan),
  `vak-session/src/log.rs` (`has_request_admission` scan),
  `vak-commit/src/ledger.rs` (replay per append).

**API**
- New `/search`, `/lineage/{node}`, `/nodes/{id}`. `/admin/api/search` and
  `/search` merge into one ACL-filtered implementation.

**UI**
- Admin Search and client search call the one API. A lineage tab is added
  in forensics.

**Tests**
- `lineage_from_any_artifact_to_cause`, `search_respects_audience`,
  `catalog_rebuild_equals_incremental`, `turn_path_reads_flat`,
  `catalog_query_p95_under_50ms_at_1m_nodes`.

## M6.5 — intake (doc 76)

**Code**
- Delete `scripts/feeds/*` (the Python pipeline, `feed_mcp.py`,
  `feed_search.py`) and the DuckDB store; `crates/vak-server/src/feeds.rs`
  becomes Sources over M4's `source_poll` triggers and cursors, with
  connectors in the broker worker.
- `crates/vak-server/src/inbox.rs` (`save_to_inbox`): one push connector.
- `crates/vak-core/src/session_search.rs`: the one retrieval tool doc 76 D1
  settles.
- `crates/vak-core/src/inbox.rs`: a match kind for alerts.

**Docs**
- Doc 51 retired; doc 76 status.

**Tests**
- `intake_item_has_trace_and_provenance`,
  `quarantined_item_absent_from_agent_retrieval`, `feed_pipeline_is_gone`.

## M7a — lifecycle: honest deletion

**Code**
- New `crates/vak-lifecycle`.
- Remove the scattered retention (invariant 30): `checkpoints.rs`
  (`MAX_STORED_CHECKPOINTS`), `finops.rs` compaction, `memory.rs`
  (`cleanup_artifacts`), and the `/memory/cleanup` route.
- Session archive and delete: the `archive.json` and `deleted.json` sidecars
  (`vak_core::trash`, `vak-server/src/lib.rs`) become lifecycle state refs,
  trash, and erasure requests.
- Contributor keys: erasing a guest from a shared conversation, with the
  placeholder in `derive_messages()` (`vak-session`).
- Backup: `crates/vak-core/src/backup.rs`, `crates/vak/src/backup.rs`,
  `/backup/export|import` become ciphertext plus wrapped keys and a
  coherent manifest, with a tombstone replay and an epoch bump on restore.
- Agent lifecycle: `crates/vak-server/src/agents.rs` gains `Revoked`, with
  data effects wired to the lifecycle.

**API**
- `/data/status|usage|lifecycle|integrity|catalog/rebuild|erasure` for
  conversations, `/data/backups`, `/conversations/{id}/…` (doc 74 §7).

**UI**
- Admin: Data health, Conversations lifecycle, Storage, Lifecycle,
  Integrity, Agents lifecycle panel, Backup.
- Client: conversation menu, Trash, "Why is this gone?", Workbench states
  (doc 74 §6, in doc 75's words).

**CLI**
- `vak data status|usage|plan|gc|verify|rebuild-catalog|erase --scope
  conversation|export|backup|cat|grep`. `vak doctor --repair` hooks.

**Docs**
- 23-memory (retention of notes), 14-checkpoints, 28-operations, 46 Part VII.

**Tests**
- `reconciler_is_idempotent`, `reconciler_observe_only_commits_nothing`,
  `settled_execution_leaves_nothing`, `gc_keeps_everything_reachable`,
  `erasure_follows_lineage`, `erasure_leaves_ledger_bytes_unchanged`,
  `guest_erasure_keeps_owner_conversation`, `quota_refuses_admission_not_records`,
  `restore_reapplies_erasures`, `revoke_cuts_endpoints_within_one_tick`,
  `thirty_day_soak_stays_within_budget`.

## M7b — lifecycle: governance

**Code**
- Labels on any labelable node, holds, erasure scopes person, Agent, space
  and tenant, data roles (`crates/vak-lifecycle`, `vak-server`).
- Allowlist PII retention: `crates/vak-server/src/gateway.rs`.
- The per-space object-id key, if a tenant has a second person (doc 73
  §7.3).

**API**
- `/data/labels`, `/data/holds`, `/data/erasure` for the wider scopes,
  `/data/keys` (doc 74 §7).

**UI**
- Admin: Retention & holds, Erasure requests, Keys, Audit export.
- Client: Your data.

**CLI**
- `vak data labels|hold|erase --scope person|agent|space|tenant|keys`.

**Docs**
- 33-admin-console, 44-shared-config (keys).

**Tests**
- `person_erasure_spans_agents_and_chats`,
  `hold_blocks_every_destructive_transition`, `label_on_any_node_resolves`,
  `stale_preview_cannot_authorise`.

## M8 — artifacts, sharing, information architecture

**Code**
- Artifact and Version records.
- `crates/vak-sandbox` candidates and promotions become artifact version
  events; a version's key grant follows doc 73 §7.3.
- `crates/vak-server/src/coworking.rs` grants become one grants table keyed
  by principal.
- `crates/vak-tools/src/office_apply.rs` and `doc_read.rs`: a draft is an
  artifact version.
- The Library prototype (`docs/design/82-library.md` L1/L2): its projection
  over the sandbox records and the session ledgers, its path-derived keys
  and the key in `AttachedArtifact` are replaced by Artifact records and
  `art_` ids. Nothing maps across, because M3b discarded their data.

**UI**
- Client: Library, share dialog, version history, Runs panel.
  `SharedConversation.tsx`, `ArtifactCanvas`, `OfficeRedline` and
  `WorkbenchPanel` move to the Artifact API.
- Admin: navigation reconciliation (doc 74 §6.1) across the Overview, Work,
  Operate, Configure and System groups in `crates/vak-admin-ui/src/App.tsx`.

**Docs**
- 54, 69 and 72 sections superseded; 33 and 58 navigation; 82's notes on
  what holds before M8.

**Tests**
- `saved_version_survives_origin_erasure` with the plan's M8 list.

## M9 — cloud remote

**Code**
- `crates/vak-storage/src/remote/` (`FileRemote`; `S3Remote` plus Postgres
  refs behind a feature); `vak sync` CLI; the sync plane in `vak-server`
  (doc 31 contract); leases over M2's epochs.

**External**
- The hosted service is out of scope; the key-escrow and key-release design
  is a separate doc before any hosted backend (doc 79 §5).

**Docs**
- 56 superseded, 31 extended, 53 (bus subject `workspace_id` becomes a
  space id).

## External boundaries (every milestone)

| Boundary | Effect |
|---|---|
| Channel bridges (Telegram, Discord, Slack) | Inbound payloads gain a TraceKey and a sender principal through `InboundRequest`. Update offsets become cursors (M4). Delivery is an effect record. Copies already sent to a platform are outside erasure and are named in the receipt. |
| LLM providers | Prompts are sent to a processor. The erasure receipt names the provider and model per turn from `WorkReceipt`. |
| NATS (when configured) | Subjects move to space ids. Payloads are references only. Stream `max_age` is set. |
| Mail, calendar and other accounts (docs 80, 81) | Connections and grants are Desired plus Secret; every send or change is an effect record with a receipt; external recipients' copies are outside erasure. |
| Backups taken before M7a | They are plaintext; at the data baseline they are refused like any other pre-baseline state. |
| OS keychain / encrypted-file store | The local `KeyAuthority` (M2); rotation and escrow land in M7b. A KMS or attested release is a later implementation (doc 79 §5). |
