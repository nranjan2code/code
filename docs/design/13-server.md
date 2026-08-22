# 13 — Server mode

The client/server bet from the original architecture: one headless agent
core, many surfaces. `vakcoder serve --port 8901` exposes vak-core over
HTTP+SSE; the TUI, web clients, IDE extensions, and curl are all equal
consumers.

## Endpoints

| method | path | purpose |
|---|---|---|
| GET | `/health` | liveness |
| POST | `/sessions` | create session → `{session_id}` |
| POST | `/sessions/:id/run` `{prompt}` | 202; events stream on SSE |
| POST | `/sessions/:id/steering` `{text}` | queue mid-run input |
| POST | `/sessions/:id/approvals/:rid` `{approve}` | resolve a permission gate |
| GET | `/sessions/:id/events` | SSE stream of `AgentEvent` JSON |
| GET | `/sessions/:id/transcript` | derived messages + usage |
| GET | `/sessions` | persisted session summaries (sidebar projection) |
| POST | `/sessions/:id/attach` `{session_id}` | resume a persisted session into memory |
| GET | `/sessions/:id/diff` | git status + diff of the session workspace |
| POST | `/config/mode` `{mode}` | switch permission mode at runtime |
| GET/PUT | `/fs/file` | read/write a file confined to the workspace root |
| GET | `/fs/tree?limit=` | bounded recursive listing (@-mention autocomplete) |
| POST | `/sessions/:id/side` `{question}` | side chat: branched turn, main chain untouched |
| GET | `/sessions/:id/side/events` | SSE for the side-chat branch |
| POST | `/sessions/:id/side/cancel` | cancel the side run |
| POST | `/sessions/:id/bestofn` `{prompt,n}` | fan out n worktree-isolated runs |
| POST | `/sessions/:id/keep` / `discard` | merge or drop a best-of-N candidate branch |
| GET | `/sessions/:id/pr` | gh-backed PR view + check rollup (`reason: no_pr\|gh_unavailable`) |
| POST | `/sessions/:id/pr/merge` `{number,method}` | `gh pr merge --auto` (squash/merge/rebase) |
| GET/POST | `/tasks`, PATCH/DELETE `/tasks/:id` | scheduled-task CRUD (persisted to `~/.vakcoder/tasks.json`) |
| POST | `/tasks/:id/run-now` | fire immediately; resets schedule |
| GET | `/sessions/:id/launch` | dev-server configs (`.vakcoder/launch.toml` + npm autodetect) |
| POST | `/sessions/:id/launch/start\|stop` `{name}` | manage a dev server process |
| GET | `/sessions/:id/launch/logs?name=` | ring-buffered output tail |

## Semantics

- **Stream-open handshake**: the SSE handler subscribes to the broadcast
  ring *before* notifying, then publishes a `StreamOpened` marker. Clients
  fire `/run` only after seeing it — no subscribe/publish races, verified by
  both the rust e2e test and the python smoke driver.
- **Approvals over HTTP**: permission asks publish `ApprovalRequested{id,
  tool, reason}` and park on a oneshot; `POST /approvals/:rid` resolves it.
  Grants are consumed exactly once (same invariant as the TUI approver).
- **Session ledger returns** after each run (`run_turn_with` now yields
  `(TurnOutcome, SessionLog)`), so transcripts stay queryable between runs.
- Second concurrent `/run` on a live session → `409 Conflict`.

## Desktop extensions (docs/design/20)

- **secured_router** returns `(Router, token)` so embedded surfaces (the
  Tauri shell) share the exact same contract; CORS allows webview origins
  only. A scheduler task fires due `/tasks` while the server lives.
- **Side chats** append Q+A as a sibling branch off the current tail and then
  restore the branch pointer — reconstructable in the JSONL, invisible to
  `derive_messages()` on the main line.
- **spawn_isolated_run** is the shared primitive behind best-of-N and
  scheduled tasks: child Core rooted in a fresh worktree, registered handle,
  fired turn. Latest-run worktree retention for tasks; keep/discard for N-runs.
- All new endpoints are bearer-gated like the originals; failures surface as
  structured values (`{"error"}`, `{"reason": ...}`) — never hangs.

## Implementation notes

- axum + broadcast ring; a single mpsc→broadcast bridge per run.
- The bridge forwards into the **broadcast** channel — an earlier draft fed
  the mpsc back into itself, producing an infinite echo loop. The e2e test's
  event-kind assertions now pin this class of bug.
- SDK seams added to Core for embedding/tests: `set_provider_instance`,
  `set_sessions_home`.
