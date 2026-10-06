# Data architecture: working tracker

Status: **working tracker, temporary. Delete this file in the same commit
that marks M9 done.** The plan (`data-architecture-plan.md`) holds the
design, decisions and exit tests. This file holds only the order of work,
where it stands, and the rules every step follows. Update it in the commit
that finishes each step.

## How to work a step

1. Read AGENTS.md ("Pending: the data architecture refactor" and its
   "don't deepen the debt" list), then the plan section for the step.
2. Re-scan what the step touches (`rg` for the names it replaces). The
   blast-radius doc's counts are old.
3. Build it whole:
   - code, tests, UI (admin and client), docs and the AGENTS.md text;
   - the replaced path removed in the same change (invariant 30);
   - no compatibility code and no migration (there are no users);
   - rewrite old-shape tests, never patch them to pass.
4. Verify:
   ```
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace --no-fail-fast
   scripts/check-version.sh
   python3 scripts/check_doc_paths.py
   ```
   - If a frontend changed, run `npm run build` in `crates/vak-admin-ui`
     and `crates/vak-client-ui`. Rebuild before `cargo check`, or the
     stale-bundle guard fails.
   - Check that the real registry
     `~/Library/Application Support/vak/tenants/ten_00000000-0000-7602-9145-b8d712473797/spaces.toml`
     still names only `/private/tmp/vak-live/workspace`.
5. Commit and push straight to `main`: no PRs, no branches, no leftover
   worktrees. Then:
   - mark the row below done, with the commit;
   - update the plan's step row and Status line;
   - update the AGENTS.md "Pending" paragraph.
6. Never change a version number. The release is held for the
   maintainer's version decision.

Known slow tests (each over 60 s, not failures):
- `reinstall_from_the_installed_binary_leaves_a_working_install`
- `memory::tests::load_matrix_scales_real_store_from_low_to_high`
- `one_turn_one_id_from_intent_to_side_ledgers`

## Order and status

| # | Step | Exit tests (plan) | Status |
|---|---|---|---|
| 1 | M4.1 fencing | `restore_fences_old_writer`, `fenced_process_stops_background_work` | Done, 5ba742a6b |
| 2 | M4.2 run records for every cause | `every_cause_writes_run`, `abandoned_run_is_recorded` | Done, 9fc4e5b5c |
| 3 | M4.3 `Trigger` replaces `TaskDef` | `last_run_is_a_query`, `trigger_round_trips_as_document` | Done, 430ef4e38 |
| 4 | M4.4 claims, one `due(now)`, `on_crash` | `schedule_slot_at_most_once_under_restart`, `two_processes_do_not_double_start`, `skipped_slot_is_a_record`, `retry_once_retries_once` | Done, 7b3a0ceaf |
| 5 | M4.5 `effects/` chain; delivery is its first kind; the outbox goes; `/effects`; Discord nonce; the new effects invariant in AGENTS.md | `effect_unknown_until_reconciled`, `effect_not_replayed_after_restart`, `discord_resend_reuses_nonce` | Done, 94b826ed6 |
| 6 | M4.6 mail and calendar send, create, update and RSVP become effects; their single-use claims go | `mail_send_is_one_effect`, `unknown_mail_send_never_resent` | Done, 9eb561d78 |
| 7 | M4.7 cursors: channel pollers and the mail vault; gap records | `cursor_resync_records_gap`, `second_poller_is_fenced` | Done (7a, 7b) |
| 7a | M4.7a cursor primitive, gap chain, Telegram/Discord/Slack pollers on cursors | `cursor_resync_records_gap`, `second_poller_is_fenced` | Done, 6a03aab5f |
| 7b | M4.7b mail vault cursors and backlog onto cursor refs (backlog an encrypted tenant object); fold or justify the vault's routine run history | `routine_cursors_live_in_a_cursor_ref_with_an_encrypted_backlog` | Done, 4263f9af3 |
| 8 | M4.8 `CopyEnvironment`; the non-git refusal goes; invariant 38 restated; docs 22, 29, 64, 76, 80 and 81 restated against the shipped shapes | `non_git_space_routine_runs_in_copy_environment` | Done, 9c9780a5b |
| 9 | M2 remainder: credential-store `KeyAuthority`, torn-write seal test, fuzz corpus, flock single-writer lock, blob streaming | plan §M2 | Done, f22e9406e |
| 10 | M5 telemetry (may run beside M4) | `library_crates_have_no_eprintln`, `one_run_one_trace_id`, `log_lines_are_json_with_trace_fields`, `telemetry_carries_no_content` | Done (10a, 10b) |
| 10a | M5a vak-telemetry, content-free JSON lines, library `eprintln!` converted, clippy ban | `library_crates_have_no_eprintln`, `log_lines_are_json_with_trace_fields`, `telemetry_carries_no_content` | Done, 0e343e9cc |
| 10b | M5b span tree run › turn › step › dispatch/tool_call › delivery; worker continues the span; Traces & logs screen and Run waterfall; vak-ops log readers; bus `max_age` | `one_run_one_trace_id` | Done (10b1–10b3), 08abe9410 |
| 10b1 | M5b1 the span tree in every run opener, the worker's `execution` span forwarded under its caller, delivery spans | `one_run_one_trace_id` | Done, 08abe9410 |
| 10b2 | M5b2 authenticated log and span endpoints, admin System › Diagnostics › Traces & logs, the Run waterfall, `vak-ops` service log readers | `telemetry_readers_filter_and_order`, `service_log_reader_reads_the_structured_log`, browser run of both screens | Done, 08abe9410 |
| 10b3 | M5b3 bus payloads carry references only; JetStream streams get `max_age` | `bus_payload_carries_references_only`, `work_streams_have_a_max_age` | Done, 08abe9410 |
| 11 | M6 data catalog, search, lineage | `lineage_from_any_artifact_to_cause`, `search_respects_audience`, `catalog_rebuild_equals_incremental`, `turn_path_reads_flat`, `catalog_query_p95_under_50ms_at_1m_nodes` | In progress (design agreed; 11a–11d) |
| 11a | M6.1 `vak-catalog`: schema, tailer cursors, ingest of ledgers, runs, effects, triggers, memory, commitments; search, lineage, open, stale, rebuild | `catalog_rebuild_equals_incremental`, `search_respects_audience`, `lineage_from_any_artifact_to_cause`, `catalog_query_p95_under_50ms_at_1m_nodes` | Done, b2f2e5d2d |
| 11b | M6.2 one search: `/search`, admin search, `session_search` and recall on the catalog; `vak-store`, `search_all`, recall cache and directory walks deleted; presentations as Documents | plan §M6 design; `trashed_session_absent_from_every_search`, the recall tests on the catalog | Done, df6e1fc86 |
| 11c | M6.3 flat turn path: `req/<id>` ref, routing evidence and commitment rollups as Documents | `turn_path_reads_flat` | Done (`ba7ba293f`) |
| 11d | M6.4 screens: search on `/search`, Lineage tab, catalog status and rebuild | browser run | Done (`ea1b01934`) |
| 12 | M6.5 design (D1 widen `session_search`, D2 Rust connectors) | plan §M6.5 design | Done (`a737ccaa8`) |
| 12a | M6.5a sources, `source_poll`, Rust connectors, `intake/` chain, detection, catalog source, `/intake` API | `intake_item_has_trace_and_provenance` | Done (`f187fed3d`) |
| 12b | M6.5b `session_search` over items, alerts into the Inbox, push intake | `quarantined_item_absent_from_agent_retrieval` | Done (`81bcf6385`) |
| 12c | M6.5c screens on `/intake`, browser run, Python pipeline and `feeds.rs` deleted | `feed_pipeline_is_gone`, browser run | Done (`29385cd9a`) |
| 13 | M7a lifecycle: honest deletion (after M6) | see plan §M7a (reconciler, erasure, hold, quota and soak tests, doc 74 §9 browser runs) | |
| 14 | M7b lifecycle: governance (after M7a) | `label_on_any_node_resolves` and the remaining doc 74 §9 runs | |
| 15 | M8 design (only declared deliverables, optional title, versions as an `artifacts/` chain with a rollup) | plan §M8 design | Done (`52c470d01`) |
| 15a | M8.1 identity and versions: `artifacts/` chain + rollup, `Tool::artifact`, candidates/promotions/Office drafts as versions, catalog nodes, `/library` | `concurrent_edit_creates_sibling_versions`, `saved_version_survives_origin_erasure` | Done (`ce275405b`) |
| 15b | M8.2 grants: `grants/` chain, coworking grants moved in, inheritance and breaks, catalog filtering | `share_inherits_and_breaks`, `revoked_grant_hides_from_search` | Done (`d09c0c450`) |
| 15c-a | M8.3a client Library page and artifact page (versions, Star, Rename, Archive, Keep, Download, Open conversation), admin Library, `/library` changes | browser run | Done (`81394dd71`) |
| 15c-b | M8.3b Continue working and Make another (doc 82 L2): the attachment rendered at admission, `recall`'s cross-conversation scope | live run | Done (`0d8d2a772`) |
| 15d-a | M8.4a sharing: artifact share links (role, expiry, history from a chosen version), guest view at `/shared/artifact`, comments, revoke | `library_sharing.rs`, live run | Done (`2e6526e8a`) |
| 15d-b | M8.4b edit in the app, Put back (upload as a sibling of the version downloaded), saved cards | browser run | Done (50b8d3bb3) |
| 15d-c | M8.4c Canvas, Redline, Workbench and SharedConversation on the Artifact API; navigation reconciliation; acceptance run create → review → promote → share → comment → revise | browser acceptance run | Next |
| 16 | M9 cloud remote (last) | `push_pull_roundtrip_identical_derive_messages`, `handoff_at_turn_boundary`, `lease_prevents_dual_writer`, `erasure_propagates_and_cannot_resurrect`, `sync_survives_network_loss` | |

Some large steps may need more than one commit, as M3b did. Add sub-rows
when you split one, and finish every part before you mark it done.

## Open items carried along

- A copy environment orphaned by a crash mid-run (`environments/<run>/`
  with no live run) is left in place; M7a's reconciler should remove it.

- Nothing releases a held (digest / until-complete) effect; no flush exists.

- A deleted trigger's claim ref (`trg/<id>/claim`) is left behind,
  because refs have no delete. M7a's reconciler should remove it.
- The mail vault keeps its private routine run history beside the run
  records (justified at M4.7b: it binds runs to an account and is removed
  on disconnect). Fold it into runs once M7a can erase run records.
- Still open from M3b:
  - remeasure write-path growth on a used 7.0 home;
  - a backup policy for the Workspace data class;
  - a small model echoes the stop guard's "Please continue."
