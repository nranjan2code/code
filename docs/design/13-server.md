# 13 — Server mode
Status: implemented in 2.0.0

The client/server bet from the original architecture: one headless agent
core, many surfaces. `vak serve --port 8901` exposes vak-core over
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
| GET | `/sessions/:id/events` | SSE stream of client events: `client_events::project` maps each `AgentEvent` to the user-facing `ClientEvent` (turn start, text/thinking deltas without snapshots, tool calls, approvals, workers, workbench, `DraftDiscarded`, and `RunFinished` with a fixed human message) and drops internal traffic (retries, route fallbacks, compaction, stop-gate notes, raw errors). `/stream` multiplexes the same projection for every session a tab follows. Every frame carries `id: <seq>`; a reconnect with `Last-Event-ID` is replayed the gap, or gets an `event: resync` frame when the gap is wider than the 1024-frame ring (docs/design/48-web-client.md §4.4). Under backpressure a still-full broadcast channel coalesces consecutive same-kind stream deltas (`StreamEvent::try_merge`) and retries the send rather than dropping one — lossless, never a gap the client cannot see |
| GET | `/sessions/:id/presentation` | reconnectable schema-v2 `OutputTimeline` snapshot; deterministic projection of the ledger and live state |
| GET | `/sessions/:id/presentation/events` | SSE stream of semantic presentation events (`Snapshot`, `ItemStarted`, `TextDelta`, `ItemReplaced`, `ItemCompleted`). A frame carries the full `snapshot` only when the stream opens, a run settles, or the consumer resyncs; live frames carry the delta alone. Runtime bookkeeping (admission, capacity, diagnostics, retries, goal-update rows) never becomes a timeline item |
| GET | `/stream?session=…&host=1&config=1` | every subscription a client holds on ONE SSE connection: per followed session its `agent`, `presentation`, `side`, `coworking` and `resync` frames, plus `host` and `config` changes. Agent frames carry the cursor vector `<session>:<seq>,…` as their id, so `Last-Event-ID` resumes every session. What the browser client uses; the per-session routes above serve single-session clients such as `vak term` (docs/design/48-web-client.md §4.7) |
| GET | `/sessions/:id/transcript` | derived messages + usage; historical (non-attached) sessions fall back to opening the ledger from disk — error bodies stay 200-wrapped for wire compatibility |
| GET | `/sessions/:id/transcript.md` | markdown export through the shared `transcript_md` renderer (byte-parity with TUI export); same disk fallback, proper 404 when unknown |
| GET | `/sessions` | persisted session summaries (sidebar projection) |
| POST | `/sessions/:id/attach` `{session_id}` | resume a persisted session into memory |
| GET | `/sessions/:id/diff` | git status + diff of the session workspace |
| POST | `/config/mode` `{mode}` | switch mode; a changed value cancels all active main/side runs and denies pending approvals before returning 200 |
| GET/PATCH | `/config` | effective configuration; provider/model is resolved and persisted as one atomic route before hot-apply; responses include route source/revision; max turns, permission mode, theme, MCP, and hooks share the cross-process refresh contract |
| PUT | `/config/key` `{provider,key}` | store a provider credential in the user secret scope (OS keychain, or an encrypted-file fallback) |
| DELETE | `/config/key` `{provider}` | revoke it; `shadowed_by_env` reports a key still exported in the real environment |
| GET | `/providers` | provider list with each one's `label` (the name everyday screens show, from `Core::provider_label`), configured pool size, and non-secret credential fingerprints (never key values) |
| GET | `/providers/:name/models` | models that provider's stored key can reach, live (502 + reason on failure — never a static fallback; a missing key adds `"kind": "no_ai_service"`) |
| GET/PUT | `/fs/file` | read/write a file confined to the workspace of the conversation named by `session` (required) |
| GET | `/fs/tree?limit=` | bounded recursive listing (@-mention autocomplete) |
| POST | `/sessions/:id/side` `{question}` | side chat: branched turn, main chain untouched |
| POST | `/sessions/:id/side/cancel` | cancel the side run |
| POST | `/sessions/:id/bestofn` `{prompt,n}` | fan out n worktree-isolated runs |
| POST | `/sessions/:id/keep` / `discard` | merge or drop a best-of-N candidate branch |
| GET | `/sessions/:id/pr` | gh-backed PR view + check rollup (`reason: no_pr\|gh_unavailable`) |
| POST | `/sessions/:id/pr/merge` `{number,method}` | `gh pr merge --auto` (squash/merge/rebase) |
| GET/POST | `/tasks`, PATCH/DELETE `/tasks/:id` | scheduled-task CRUD (persisted in the data home); additive `schedule` (5-field cron), `script` (zero-token watchdog), `model_pin` fields validated via `TaskDef::validate` → 400 |
| POST | `/tasks/:id/run-now` | fire immediately; resets schedule |
| GET | `/sessions/:id/launch` | dev-server configs (`.vak/launch.toml` + npm autodetect) |
| POST | `/sessions/:id/launch/start\|stop` `{name}` | manage a dev server process |
| GET | `/sessions/:id/launch/logs?name=` | ring-buffered output tail |
| GET | `/sessions/:id/workers` | live children spawned by this session (parent-scoped) |
| POST | `/sessions/:id/workers/:child/steer` `{text}` | queue steering for one child; 404 unless the child belongs to `:id` |
| POST | `/sessions/:id/workers/:child/stop` | cancel one child; same parent-scope check |
| GET/PUT | `/config/mcp` | read the effective MCP table / replace it: validates, persists `[mcp.servers]` to the project config without destroying other keys, hot-applies into the running Core |
| POST | `/sessions/:id/run` attachments | base64 image blocks ride the prompt as native vision content |
| POST | `/sessions/:id/steering` `{text,attachments?}` | queued input keeps image blocks — never degraded to bare text |
| GET | `/memory` | durable memory notes, workspace tier + global `USER.md` profile tier (`id`, `scope` per note) |
| POST | `/memory` `{text,kind?,tag?,scope?,session_id?}` | append to either tier (201; 400 on validation error) — desktop/gateway/CLI all write through this same API |
| PATCH | `/memory/:note_id` `{text,scope?}` | amend a note body, provenance header preserved |
| DELETE | `/memory/:note_id?scope=` | forget one block (byte-safe rewrite); 404 unknown id |
| GET | `/search?q=&limit=&all=true` | recall over the current project's ledgers; `all=true` spans every project hash (`space_id` annotated) |
| GET | `/sessions/:id/receipts` | dispatch forensics: per-attempt walk receipts |
| GET | `/doctor?session=` | `HealthReport` JSON (checks/facts/frozen-ladder) |
| GET | `/digest?days=N` | usage rollup from the cost ledger + memory/skill deltas (1–90) |
| POST | `/backup/export` `{dest_dir,include_secrets?}` / `/backup/import` `{src_dir,conflict?}` | home backup round-trip; secrets excluded by default; 400 when source/target equals the home itself |
| GET | `/skills/proposals` (+ promote/reject) | learned-skill review queue; proposals carry a `duplicate-of:` screening tag where applicable |
| GET | `/intent/explain?prompt=&surface=&act=&horizon=&stakes=&evidence=` | resolve a prompt without running it: reading, engagement, every contributing signal, and the diff against the unrestricted baseline. Free tiers only, so it costs nothing and dispatches nothing — safe to call from a composer as the user types (`docs/design/47-commitment-kernel.md`) |
| GET | `/intent/policy` | resolved `[intent]` + `[commitment]` policy for this workspace |
| GET | `/commitments?all=` | the portfolio plus `priorities`, in the order the scheduler would work it. Priorities ride alongside the rows rather than being baked in: the ordering is an opinion, and a UI should be able to show *why* as well as *what* |
| GET | `/commitments/:id` | one commitment's projected state plus its raw event stream |
| POST | `/commitments/:id/close` `{verdict,note?}` | 409 when the closure invariant refuses the claim — the request was well-formed and the server is fine; the recorded evidence simply does not support `fulfilled`. The message names the missing evidence |

## Personal-OS scheduler semantics (docs/design/29)

- Cron tasks fire via `cron_next_after` in local time (vixie dom∧dow
  OR-semantics, DST-gap skip-forward). `[automation] catch_up_missed = true`
  fires one missed slot at startup.
- `script:` tasks never touch the LLM: brokered bash, stdout trimmed →
  verbatim delivery, empty stdout → silent tick, failure/timeout → typed
  error alert (never silent).
- After every fire the day-spend is checked against `[finops] max_day_usd`;
  crossing 80%/100% records an audit-only `budget_alert` ledger row and
  delivers to configured surfaces once per level per window.

## Semantics

## Outcome-directed presentation

The server is the canonical projection boundary for the shared outcome runtime
(`docs/design/52-outcome-directed-runtime.md`). `GET
/sessions/:id/presentation` returns an `OutputTimeline` whose result items carry
their own status, evidence receipts, requirement links, diagnostics and exact
fallback Markdown. The timeline may also carry the current projected
`GoalState`, including the objective, additions, superseded revisions and
control state.

`GoalUpdate` entries are append-only collaboration records. A steering or
follow-up message can add, correct, replace, pause, resume, reprioritise or
cancel work; the runtime classifies the relation and applies only changes that
fit the admitted authority. Changes that affect permissions, irreversible work
or the selected route continue through the existing approval path. Web, desktop
and channel clients consume this same projection; a channel's shorter text
fallback does not change the underlying outcome or evidence.

- **Stream-open handshake**: the SSE handler subscribes to the broadcast
  ring *before* notifying, then publishes a `StreamOpened` marker. A freshly
  admitted turn chain gives an SSE consumer up to 2s to attach before its
  first leg starts, so its terminal event is seen — but only when nobody is
  watching yet (`wait_for_external_subscriber`); a client already attached
  incurs no wait. Clients fire `/run` only after seeing `StreamOpened` — no
  subscribe/publish races, verified by both the rust e2e test and the python
  smoke driver.
- **Approvals over HTTP**: permission asks publish `ApprovalRequested{id,
  tool, reason}` and park on a oneshot; `POST /approvals/:rid` resolves it.
  Grants are consumed exactly once (same invariant as the TUI approver).
- **Session ledger returns** after each run (`run_turn_with` now yields
  `(TurnOutcome, SessionLog)`), so transcripts stay queryable between runs.
- `/run` and `/steering` never reject on a busy session: both admit through
  `admit_or_queue` and return `202 {"request_id", "state"}`, where `state`
  is `"started"` (this call owns the run), `"queued"` (a run is already in
  flight; the input is pushed onto the steering queue and runs as the next
  leg of the same turn chain once the live leg settles —
  `continue_or_release`), or `"duplicate"` (this `request_id` was already
  admitted). There is no concurrent-run rejection. `POST
  /sessions/:id/cancel` stops the live leg and discards whatever was queued
  for a continuation leg (input arriving after the discard is unaffected —
  it becomes the next leg like any other steering); it never synthesizes a
  `RunFinished` itself, since the chain's own settle path emits the one
  terminal event once the run actually stops.

## Browser surface (docs/design/48-web-client.md)

The same process serves the workspace client at `/app` and the operations
console at `/admin`, from one asset-serving path (`embedded_ui.rs`).

| method | path | purpose |
|---|---|---|
| GET | `/app`, `/app/*` | the workspace client shell (auth-exempt: the login form is part of the bundle) |
| POST | `/auth/login` `{token}` | token → `vak_session` cookie; shared by `/app` and `/admin` |
| POST | `/auth/logout` | clear it |
| GET | `/auth/session` | `{authenticated}` — deliberately not a 401, so "no session" is distinguishable from "unreachable" |
| GET | `/host` | what `backend_info` is on the desktop; carries no base URL or token, which is how the client knows it is same-origin and cookie-authenticated |
| GET | `/workspaces` | known workspaces, active first |
| POST | `/workspaces/open` `{path, trust?}` | resolve through `CorePool` and make it active for NEW sessions (a session freezes its `Core` at creation, invariant 17) |
| GET | `/fs/dirs?path=` | folder names only, rooted at `[server] workspace_roots` |
| GET | `/pty` (WS) | a real shell. Off unless `[server.web] terminal`, loopback-pinned unless `terminal_requires_loopback = false`. The socket IS the shell's lifetime |

Exposure is governed by `[server]` — privileged in full, so an untrusted
project cannot widen it — and by invariant 34: `Host` pinned to loopback
plus `trusted_hosts`, cross-origin mutations refused, `?token=` loopback-only.

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
