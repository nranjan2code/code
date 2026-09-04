# 20 — Tauri desktop app: competitive research & architecture

> **Since this was written, the UI moved.** The client itself now lives in
> `crates/vak-client-ui` and is shared with the browser
> (docs/design/48-web-client.md); `crates/vak-desktop` is the native half —
> window and tray lifecycle, single-instance handover, the workspace trust
> gate, PTY commands, and the native save dialog. Everything below about
> *what the client does* still holds; where it says "the desktop app has X",
> read "the workspace client has X, on every host that can provide it."

Goal: a native desktop orchestrator for vak in **Tauri 2**, built by
unapologetically copying the best features of Claude Code Desktop, OpenAI's
Codex desktop app, and Cursor 3's Agents Window — then beating them where our
architecture already wins. Sources: official docs/changelogs/blogs for all
three apps (Feb–Aug 2026); Codex GitHub issues #24197/#34183; ChatML's public
Tauri 2 field report; Tauri-vs-Electron benchmarks (2026).

## Thesis

Every leader converged on the same product: **the human as orchestrator**.
Many parallel agent sessions, each isolated in its own worktree; the human
surfaces diffs, approves actions, monitors PRs, schedules routine work. The
desktop app is not "a chat window" — it is mission control. Our unfair
advantages transfer directly:

1. **One append-only ledger shared by every surface.** Codex's #1 desktop
   complaint is CLI sessions being invisible in the app (#24197, still open).
   Ours cannot happen structurally: `vak-tui`, `exec`, `serve`, and the
   desktop app all read/write the same JSONL trees under the data home.
2. **Delta+snapshot streaming everywhere** (invariant 4). Every pane — chat,
   diff, subagent view — consumes the same event stream at whichever level it
   wants. No re-derivation hacks.
3. **The backend is already Rust.** ChatML needed a Go sidecar + node-pty +
   bundled Node runner (155 MB installed). We link/spawn Rust we already
   ship; no Node tax, ~10 MB-class shell.
4. Worktrees + checkpoints + permission modes + approvals + steering +
   MCP + skills + hooks + flows/planner already exist server-side. The
   desktop app is mostly *projection*, not new core.

## What we have today (assets to project)

| asset | crate | desktop use |
|---|---|---|
| HTTP+SSE contract: sessions/run/steering/cancel/approvals/events/transcript | vak-server | the entire agent protocol; zero new API surface |
| append-only JSONL trees, branching via parent_id, compaction-as-entry | vak-session | unified session history across TUI/desktop; side chats as branches |
| permission engine (modes × rules → Allow/Ask/Deny) | vak-permission | mode selector UI maps 1:1 |
| approval FIFO with diff previews | vak-agent | approval cards in-app |
| steering queues mid-run | vak-agent | "type while running" prompt box |
| checkpoints + rewind | vak-core | time-travel UI |
| worktree isolation | vak-core | per-session isolation toggle |
| subagents (task tool) + live streams | vak-agent | tasks/subagent pane |
| skills, hooks, lazy MCP meta-tool | vak-core/hooks/mcp | slash palette, connectors manager |
| static flows + dynamic planner | vak-flow | best-of-N / fan-out orchestration |

## Steal-list A — Claude Code Desktop (leader; redesign June 2026)

| feature | copy? | mechanism here |
|---|---|---|
| Multi-session sidebar: parallel sessions, filter by status/project, group by repo, Cmd+N | D1 | session store query projection; statuses from run state |
| Per-session git-worktree isolation, branch prefix, archive-on-PR-merge | D1 | existing vak-core worktrees behind a session flag |
| Drag-and-drop panes: chat/diff/browser/terminal/file/plan/tasks | D1/D2 | dockable layout (frontend lib), panes = event consumers |
| Integrated PTY terminal per session (shares env/cwd) | D2 | `portable-pty` behind Tauri command → xterm.js over event channel |
| In-app file editor w/ on-disk conflict warning | D2 | CodeMirror 6 + mtime check |
| Diff viewer: file tree, per-line comments batched into feedback (Cmd+Enter) | D1 | comments become **steering entries** (model-visible ⇒ logged) |
| "Review code" button (high-signal review: logic/security only) | D3 | one-shot review prompt against current diff |
| PR monitor: CI status bar via `gh`, auto-fix failing checks, auto-merge on green | D3 | gh polling loop; failures fed back as steering |
| Browser pane: dev-server preview from `.claude/launch.json`-style config, auto-verify loop | D4 | `.vak/launch.toml`; screenshot/DOM verify tool |
| Side chats (`/btw`): ask using session context without derailing main thread | D2 | **branch entry** (parent_id child, marked non-canonical); never merges back; satisfies invariants 1–2 |
| Cross-session messaging ("tell payments session the schema changed") + task chips | D4 | new tool + recipient-side entry quoting sender session id |
| Permission-mode selector remembered per folder | D0 | existing modes; persist selection per cwd in config layering |
| @file mentions, drag-drop/paste attachments | D1 | new session entry types (invariant 1) |
| Skills/slash palette, plugin browser, connectors (MCP graphical setup) | D3/D4 | enumerate existing registries |
| Transcript density modes Normal/Verbose/Summary | D1 | three projections over same ledger |
| Context usage ring (per session) | D0 | usage events already stream |
| OS notification when unviewed session finishes | D0 | tauri-plugin-notification |
| Split view: two sessions side-by-side | D2 ✅ (Aug 2026) | two consumers, independent cursors |
| Scheduled routines/recurring tasks | D4 | daemon mode spawning normal runs in worktrees |
| Environments: Local/SSH/cloud | D5 | SSH = tunnel to remote `serve`; no cloud infra needed |

## Steal-list B — Codex desktop app (task-centric challenger)

| feature | copy? | notes |
|---|---|---|
| Task-based workflow: discrete tasks w/ status, review before keep | D1 | sessions get task titles/status chips |
| Parallel agents default-isolated in background worktrees | D1 | make worktree-per-session the *default* for background runs |
| Scheduled tasks run locally in worktrees; sandbox scope (file/network); dry-run first recommendation | D4 | pair scheduler w/ vak-permission rules scoped per schedule |
| Skills = folder + SKILL.md + YAML frontmatter | adopt format | compatibility with the de-facto standard both majors use |
| Local app-server architecture (localhost API + sqlite index + JSONL) | validated | matches our vak-server contract; keep local-first |

**Anti-patterns witnessed (do not repeat):**
- #24197 — CLI-created sessions invisible in app sidebar. Impossible here:
  one store, all surfaces. Marketing-grade differentiator; surface a badge:
  *"started in terminal · continued here"*.
- #34183 — remote outage replaced healthy local sidebar with cloud shell;
  active task orphaned. Rule: **remote surfaces degrade non-blockingly**;
  local ledger is always source of truth; never hide local runs behind
  account/metadata fetches.

## Steal-list C — Cursor 3 Agents Window

| feature | copy? | mechanism here |
|---|---|---|
| One window managing agents across repos/environments | D2 | sidebar spans all projects under the data home |
| Tabbed agent chats | D1 | trivial |
| Best-of-N: same prompt fanned across models/worktrees, side-by-side compare, color-coded agreement (agree/partial/diverge) | D4 | N child branches off one prompt entry (ledger-native!); comparison = sibling projection; optional judge via vak-flow |
| Aggregated diff viewer across parallel runs | D4 | union of sibling diffs, agreement coloring |
| Hand off session between surfaces (local ↔ elsewhere) | D5 | SSH target covers it; ledger syncs by file path |
| Design Mode: click DOM element → context to agent | D5 | browser-pane element select feeding an attachment entry |

## Architecture decision

**Tauri 2 shell, sidecar-first, hybrid native.**

```
┌─ vak-desktop (Tauri 2, crates/vak-desktop) ─────────────────────┐
│ webview UI (SolidJS + Vite SPA, CodeMirror 6, xterm.js)         │
│   │  HTTP+SSE 127.0.0.1:<ephemeral> + loopback token            │
│   │  = the existing vak-server contract (docs/design/13)        │
│ ├─ sidecar: `vak serve --port 0` (bundled binary)          │
│ │    health-gated startup; owns sessions/runs/approvals         │
│ ├─ native commands (Rust): PTY terminal, dialogs, notifications,│
│ │    deep links (gh OAuth), global shortcuts, tray              │
│ └─ plugins: single-instance, updater, deep-link, notification,  │
│      dialog, global-shortcut, store                             │
└──────────────────────────────────────────────────────────────────┘
```

Why this shape:

- **Zero protocol drift.** The webview speaks the exact contract `serve`
  already exposes; tui/exec/serve/desktop stay behaviorally identical, and
  evals keep covering the real path. A pure-command bridge would fork the
  API surface for no user value.
- **Sidecar beats in-process linking** for now: process isolation keeps the
  crash domain out of the shell, lets us attach the desktop to a *remote*
  `serve` later (SSH target) by changing one URL, and costs nothing since
  the binary already exists. If IPC latency ever matters, commands can move
  into the shell incrementally — the frontend doesn't change.
- **Frontend: SolidJS + Vite SPA.** Signals map directly onto delta+snapshot
  SSE; no VDOM overhead in transcript-heavy views; small runtime. Alternatives
  considered: Svelte 5 (fine), React (heavier, ecosystem not needed).
- **Editor/diff: CodeMirror 6** (merge-view for diffs) — an order lighter
  than Monaco, good enough for spot edits; xterm.js for PTY panes.
- **Secrets:** unchanged — `.env` loading stays in vak_config; the renderer
  never sees keys. Settings in tauri-plugin-store; keychain integration later.
- **Security:** strict CSP, no remote content in the webview, capabilities
  least-privilege, loopback token auth on the local port.

### Invariant mapping (non-negotiables → desktop features)

| invariant | consequence |
|---|---|
| 1 model-visible ⇒ logged | @mentions, attachments, diff line-comments, side-chat prompts are **new entry types or branches**, never renderer-only strings |
| 2 append-only | side chat = branch (child entry), archived ≠ deleted; rewind = checkpoint entries |
| 3 errors are values | tool/provider errors render as cards; breaker/endurance state visible in status row |
| 4 delta AND snapshot | every pane subscribes at its level; Verbose=delta firehose, Summary=snapshot projection |
| 5 abort preserves partial output | stop button cancels token; partial transcript remains visible and persisted |
| 7 retry/breaker/endurance | watchdog + breaker states surfaced (Codex lesson: never fake-alive spinners) |
| 8 secrets | .env/keychain only; renderer storage forbidden for keys |

## What we will NOT copy

- **Computer use / screen control.** Different trust boundary entirely;
  revisit after D5. Our sandbox story (Seatbelt/Landlock) is a better spend.
- **Cloud-managed session infra.** No backend service exists or is wanted;
  SSH-to-serve covers remote machines without custody of user code.
- **Phone dispatch.** Depends on cloud push plumbing; skip.
- **Merged super-app shell** (Codex folding Codex into ChatGPT UI): their own
  bug reports show the cost. Desktop stays a focused agent surface.

## Gap matrix (us vs leaders, post-D5 target)

| capability | CC Desktop | Codex app | Cursor 3 | us (target) |
|---|---|---|---|---|
| multi-provider models | ❌ Anthropic-only | ❌ OpenAI-first | partial | ✅ day-1 differentiator |
| unified CLI+GUI history | partial | ❌ (#24197) | n/a | ✅ structural |
| worktree isolation | ✅ | ✅ | ✅ | ✅ (exists) |
| best-of-N compare | ❌ | ❌ | ✅ | ✅ D4 (ledger-native) |
| diff line-comment feedback | ✅ | basic | partial | ✅ D1 |
| PR monitor + auto-fix | ✅ | partial | ✅ | ✅ D3 |
| scheduled local tasks | ✅ | ✅ | ❌ | ✅ D4 |
| browser preview/auto-verify | ✅ | partial | ✅ | ✅ D4 |
| transparency (full ledger UI) | verbose mode | transcript files | hidden | ✅ structural |
| reliability surfacing | minimal | minimal | minimal | ✅ breaker/watchdog UI |

## Phases

- **D0 — shell parity (walk before orchestrate).** Scaffold crates/vak-desktop
  + ui/; bundle binary as sidecar w/ health gate; session list + single chat
  pane streaming SSE; approvals cards; steering/cancel; permission-mode
  selector; notifications; context ring. Exit: daily-drive a real task.
- **D1 — orchestrator spine.** Multi-session sidebar (filter/group/status),
  tabs + split view, worktree-per-session default for background runs, diff
  viewer w/ line-comment→steering, transcript density modes, task titles/
  status chips, shortcut sheet.
- **D2 — workspace.** PTY terminal pane, file editor pane, side chats as
  branches (/btw), @mentions + attachments (new entry types), cross-project
  sidebar, drag-and-drop layout.
- **D3 — delivery loop.** gh PR status bar, auto-fix/auto-merge toggles,
  review-code action, skills/slash palette + plugin manager UI, launch
  config (`.vak/launch.toml`) parsing.
- **D4 — fleet features.** Best-of-N runner + aggregated/agreement diff view;
  cross-session messaging tool + task chips; scheduled-task daemon (local
  cron, worktree-scoped, permission-scoped); connectors/MCP manager; browser
  preview pane + auto-verify tool.
- **D5 — ship it.** Signing/notarization (macOS/Windows/Linux), auto-updater
  channel, deep-link OAuth (gh), tray + global shortcuts, SSH target
  (tunnel to remote serve), packaging sizes verified <25 MB.

Verification gates per phase: `cargo fmt/clippy -D warnings/test` workspace
green; PTY smoke script extended for desktop sidecar boot; e2e through the
real binary (never a mock protocol) for every new entry type added.

## Implementation status (first vertical slice, Aug 2026)

Shipped in `crates/vak-desktop` (+ additive `vak-server` endpoints):

- **Task-centric shell redesign**: a restrained native-dark visual system,
  task identity and run state in a persistent workspace header, icon-based
  pane navigation, a two-level session sidebar with project switching, a
  structured composer, useful active-session empty states, and unified
  styling for transcript, review, delivery, modal, loading, and error
  surfaces. The hierarchy keeps chat primary while diff/editor/terminal/
  preview/PR remain one click away.
  The shell supports remembered sidebar/review-pane resizing, a keyboard
  focus mode, scroll-position-safe streaming, explicit transcript loading,
  non-blocking error toasts, reduced motion, and accessible focus states.
  Header-only abandoned session drafts remain in the append-only store but
  are suppressed from the task switcher so accidental new-task clicks do not
  create permanent sidebar clutter. Session summaries also project live run
  state, refreshed periodically, so background and scheduled work remains
  accurate before a transcript stream is attached.

- **Shell**: Tauri 2 window; embedded `secured_router` on an ephemeral
  loopback port with per-process bearer token (deviation from sidecar-first:
  in-process linking keeps one binary and the exact same HTTP+SSE contract;
  remote/SSH targets still work later by pointing the webview at a URL).
  Project picker gate on first run; last project persisted to
  `data_home()/desktop.json`; project switch restarts the backend.
- **Chat**: SSE streaming with markdown-lite renderer, thinking blocks,
  tool cards (args/result/error), approvals inline (allow/deny), steering
  while running, stop (Esc/cancel), transcript density modes
  (summary/normal/verbose), context-usage ring from `/health.context_window`.
- **Sidebar**: persisted-session listing (`GET /sessions`), search + status
  filters, resume via `POST /sessions/{id}/attach`, new session (⌘N).
  The project-first navigation keeps up to eight valid recent workspaces in
  `data_home()/desktop.json`; selecting one performs a two-phase backend
  handoff. The current backend remains live until the replacement has bound
  successfully, repeated selections are no-ops, concurrent switches are
  serialized, and stale frontend refreshes cannot repopulate the new view
  with sessions from the previous workspace. File-sourced environment values
  are replaced as a user+project scope on each handoff so secrets and config
  from an earlier workspace do not remain ambient.
- **Diff pane** (⌘D): git diff/status projection (`GET /sessions/{id}/diff`),
  click-a-line → comment → steering entry.
- **Terminal pane** (⌃\`): real PTY (`portable-pty`) → xterm.js over a Tauri
  channel, per session cwd, resize-aware.
- **Server additions** (all bearer-gated, tested in
  `vak-server/tests/server_ext.rs`): CORS for webview origins,
  `GET /sessions`, `POST /sessions/{id}/attach`, `GET /sessions/{id}/diff`,
  `GET|PUT /fs/file` (symlink-resolved confinement to cwd),
  `GET /fs/tree` (bounded listing, vendored dirs skipped),
  `POST /config/mode`, `/health` gains `context_window`+`cwd`.
  Later additions: subagent control plane (`GET /sessions/{id}/subagents`,
  `POST /sessions/{id}/subagents/{child}/steer|stop`, parent-scoped so one
  session can never touch another's child), MCP server management
  (`GET|PUT /config/mcp` — PUT validates, persists the project config's
  `[mcp.servers]` table without destroying other keys, and hot-applies into
  the running Core via `Core::set_mcp_servers`; tested in mcp_endpoints.rs),
  and image attachments on both `/run` and `/steering`.
- **D2 completion**: dock tab bar (diff/terminal/editor), file-editor pane
  (open-from-chat links + diff headers, ⌘S save, disk-conflict warn via
  refetch-compare), @file mention autocomplete in the composer (backed by
  `/fs/tree`; the mention text ships inside the prompt so it is logged with
  the user message — invariant 1 holds without a new entry type),
  per-session PTY lifecycle fix (terminals keyed by session id), diff pane
  auto-refresh on run finish.

- **Subagents dock tab** ("Subagents"): live list of the session's running
  children polled from the control-plane endpoints, with per-child steering
  input and stop. Children already stream lifecycle/tool events into the
  session SSE; this closes the attach/steer gap that previously existed only
  in the TUI. Attachments: the composer accepts images via button, paste, or
  drag-drop; they ride `/run` and mid-run steering as base64 vision blocks,
  never degraded to bare text. Goal-armed runs refuse image attachments with
  an inline composer error (server-side rule surfaced client-side).

- **Side chats as branches** (`/btw`, ⌘;): server runs the Q as a sibling
  branch off the current main tail (`branch_at` + append + restore), streams
  deltas over a dedicated `GET /sessions/{id}/side/events` channel so the
  main transcript never sees them; after completion the branch pointer is
  restored and future main turns continue exactly where they were. Side
  entries stay in the ledger — reconstructable via their parent chain,
  invisible to `derive_messages()` on the main line. Tested:
  `side_chat_branches_off_and_restores_main_chain` proves main count/contents
  unchanged while the JSONL retains question+answer.

- **Best-of-N runner**: `POST /sessions/{id}/bestofn` fans the prompt across
  1–4 isolated git worktrees (`.vak/worktrees/<rid>`, branch
  `vak/<rid>`), each a fully registered session with its own Core,
  provider, event stream, and cwd — the diff endpoint is now per-session so
  each candidate's changes render in the diff pane. `POST /{child}/keep`
  merges the branch into the checkout (conflicts → merge aborted + error
  surfaced), `POST /{child}/discard` drops worktree+branch. UI: N× button →
  config modal (prompt + candidates) → live compare grid with per-candidate
  streams, final answers, keep/discard/diff actions. Tested end-to-end:
  fan-out, per-child transcripts, worktree cleanup on both paths.

- **PR monitor + auto-fix/auto-merge** (dock tab `pr`): `GET
  /sessions/{id}/pr` resolves the branch's PR via `gh pr view` with check
  rollup; tooling absence surfaces as structured `{reason: no_pr |
  gh_unavailable}` values, never hangs. Panel polls every 30s while watching;
  toggles persist per session. Auto-fix dispatches a fix run (failing-check
  names + log commands in the prompt) when the session is idle; auto-merge
  calls `POST /{id}/pr/merge` (`gh pr merge --auto --squash --delete-branch`)
  exactly once per run when all checks pass. Tested:
  structured status on seeded repo, deterministic merge failure as a value.

- **Scheduled-task daemon** (Codex/Claude routines, local-first): task CRUD
  (`/tasks`) persisted in the data home, scheduler loop spawned by
  `secured_router` (20s ticks; fires tasks for the active workspace when due).
  Each fire reuses `spawn_isolated_run` — best-of-N's shared primitive — so
  every run lands in its own worktree with a full session in the ledger.
  Latest-run worktree retention: replaced on the next fire, deleted with the
  task (only while idle). Summary recorded via event watcher on the child
  stream. UI: ⏱ sidebar button → modal with add/pause/resume/run-now/delete +
  diff-of-last-run. Interval floor 60s (validated as a value). Tested:
  CRUD + run-now end-to-end incl. worktree churn and cleanup.

- **Preview pane + dev-server lifecycle** (dock tab `preview`): launch
  configs from `.vak/launch.toml` (`[[server]] name/cmd/args/port`),
  auto-detecting `npm run dev` as fallback. Start waits up to 15s for the
  declared port to bind, then an interactive iframe renders the app (CSP
  frame-src scoped to loopback); logs tail into a ring buffer viewable
  in-pane; stop kills via `kill_on_drop`. Tested against a real
  `python3 -m http.server`: bind detection, double-start 409, log flow,
  port release after stop.

E2E proof: app boots, embedded server answers on the ephemeral port,
`/health` reflects the live Core (provider/model/sandbox/mode), unauthenticated
`/fs/*`, `/side/*`, `/bestofn`, `/pr`, `/tasks`, `/launch` rejected with 401;
workspace fmt+clippy(-D warnings)+230 tests green.

## Composer intent strip

Above the composer box sits a read-out of how vak has read what you are about
to send (`docs/design/47-commitment-kernel.md`): the act, whether it opens a
commitment, how many tools it will see, whether it will ask before acting.
Expanding it gives the axes, the narrowing list, and every contributing signal
with the weight it carried — the same evidence `vak intent explain` prints.

The design constraint is that this element sits between the user and *every*
message they send, so its failure mode is not "unclear", it is "in the way".
A strip that announced itself on every keystroke would be ignored within a
day, and an ignored safety signal is worse than none, because the one time it
says something important nobody is looking. So it is quiet in proportion to
consequence:

- ordinary work reads as chrome — micro-label type, `--faint`, no border;
- work that opens a commitment or owes evidence steps up to `--muted` with a
  filled mark;
- irreversible, deferred, or must-ask readings take the accent **and state
  the reason inline without needing a click**, because a warning behind a
  disclosure is a warning nobody reads.

Resolution runs the free deterministic tiers only, so it costs nothing and
dispatches nothing. It is debounced, abortable, silent on error, and never
blocks sending: an optional read-out that could delay a turn would be a worse
trade than not having it.

## Settings workspace

The desktop includes a native-feeling, searchable settings workspace available
from either settings icon or `Cmd+,`. General and appearance preferences are
stored locally and applied live, including theme, text and code scale, task-list
density, transcript detail, prompt suggestions, notifications, and reduced
motion. Agent defaults and permission mode update the live Core through the
authenticated `GET|PATCH /config` endpoint. Provider and model are one atomic
route: a partial patch is completed from the effective route, both values are
persisted together, and both are hot-applied together. Every new-session
admission refreshes persisted defaults unless the Core carries an explicit
scoped runtime pin, so an admin-console change reaches an already-running
desktop backend without relying on stale startup memory. Changing the mode revokes active
main and side runs plus pending approvals so no task retains a stale security
snapshot; the next run starts under the selected mode. Agent defaults and
the permission mode are persisted in the workspace config before the live
Core is updated, so a restart does not lose an applied choice. Existing
sessions retain their immutable provider/model contract and the transcript
API reports when it differs from current workspace defaults. New sessions use
the new pair; history is never rewritten. The endpoint
deliberately returns only a safe, secret-free configuration projection.
Reliability, integration, context, and path pages expose the effective runtime
configuration without
pretending read-only values are editable. Persistent project settings open the
workspace-confined `.vak/config.toml`, creating a minimal starter only when
the file does not already exist. The Reliability page also carries a Route
ladder group (Phase R): the frozen objective, the cross-model fallback
allowlist, and the ladder length cap.

## Dispatch forensics panel (Phase R)

Header toolbar gains a receipt button next to Time travel: a per-session
drill-down over `GET /sessions/{id}/receipts` styled after vakrouter's trace
view. Every paid model step lists its purpose (model step / compaction /
completion audit), serving leg (`provider/model`), attempt count, delivery
state, and latency; expanding a step reveals the frozen-ladder attempt walk —
one row per dispatch with settlement chip (ok / failed / cancelled / unknown),
reason (initial / retry / route fallback / endurance retry), failure domain,
latency, token usage, error preview, and per-attempt leg attribution when a
fallback walk restamped it. A summary line counts steps, dispatches, and
ladder walks. Read-only by design: the ledger is the truth; this surface only
projects it. Live leg changes during runs additionally render inline via the
`RouteFallback` event in the transcript.

## End-to-end hardening + remaining platform capabilities (Aug 2026)

Wiring audit fixes and capability additions, all behind the same HTTP+SSE
contract (`vak-server/tests/server_ext.rs` covers every one):

- **Resume fix (critical).** `activate()` now attaches the persisted session
  server-side before opening streams. Previously a task resumed from the
  sidebar after an app restart answered `POST /run` with 404 — the handle was
  never re-registered. Attach is idempotent for live sessions.
- **Secrets actually reach the shell (critical).** The desktop boot now loads
  the user `.env` always plus the picked workspace's trusted `.env` — the
  exact CLI contract — before starting the Core. Previously the shell loaded
  no env files at all, so CLI users' keys never worked in the app until they
  happened to export them globally.
- **Provider/key/model setup is first-class** (shared layer, not desktop-only):
  - `GET /providers` — registry names, which env var authenticates each,
    whether credentials resolve right now, curated model suggestions per
    provider. Never returns secret values.
  - `PUT /config/key {provider, key}` — upserts into the user-level
    the user `.env` (0600, atomic replace, shared by TUI/exec/serve) and
    registers a runtime override so the very next request uses it — no
    restart. Keys are accepted once and never echoed back.
  - Lookup precedence stays: runtime override → real environment → .env
    files (invariant 8 intact). `openai`/`openrouter` auth now honors `.env`
    like every other provider (they read raw process env previously).
  - UI: a "Connect a model" gate step appears when the workspace has no
    usable credential — provider picker (with ✓ for configured), model field
    fed by the curated lists (free-form still wins), password input, one
    save-and-continue. Settings → Agent gains the same controls plus a
    Replace-key flow.
- **Time travel** (the D1 checkpoint UI, previously CLI-only): `GET
  /sessions/{id}/checkpoints` lists turn-start snapshots; `POST …/{seq}/restore`
  rewinds the workspace (409 while a run is active; observed-at-capture files
  are rewritten, never removed — ledgers survive; best-of-N children restore
  into their worktree cwd). UI: ⌘H / header history button opens a modal with
  two-step confirm per snapshot. Missing snapshot dirs list as empty, not 500.
- **Archive**: `POST /sessions/{id}/archive {archived}` toggles a sidecar map
  at `<sessions-home>/archive.json`; ledger files are untouched (invariant 2).
  `/sessions` gains an `archived` field; sidebar gains an archived filter chip
  plus hover archive/restore action on each row.
- **Skills slash palette**: `GET /skills` exposes name+description for
  project/user skills; typing `/` first in the composer opens a palette that
  inserts "use the <name> skill" — plain prompt text, so it ships inside the
  user message and invariant 1 holds without new entry types.
- **Review changes**: diff pane gains a one-shot high-signal review dispatch
  (logic/security findings only), sent as a normal logged run.
- **Mode parsing parity**: `/config/mode` and `PATCH /config` now accept the
  Debug spellings surfaced by `/health` (`ReadOnly` etc.) in addition to config
  kebab-case — the status-bar selector used to get a silent 400.
- **Boot failures are visible**: an auto-boot failure at launch lands in
  `backend_info.boot_error`, rendered on the project gate (a bundled app has
  no stderr). The gate also polls backend state as a fallback for the
  `backend-ready` event, so a missed event can never strand the UI.

## Steal-list status: COMPLETE (minus agent-driven auto-verify)

Everything from the Claude/Codex/Cursor steal-lists that fits our no-cloud,
local-first thesis has shipped. The one deliberate remainder is the
**agent-driven half of auto-verify**: screenshots/DOM inspection fed back to
the model. That needs a headless-browser dependency decision
(chromiumoxide + system Chrome vs bundled WebView probe) — tracked as future
work; until then the agent verifies via bash (`curl`), and the human verifies
visually in the preview pane.

Aug 2026 additions closing the last interactive gaps: the **subagent dock
tab** (attach/steer parity with the TUI, over parent-scoped server endpoints),
a **graphical MCP manager** in Settings → Integrations (add/edit/remove
servers, network toggle, persisted to project config + hot-applied),
**composer image attachments** (button/paste/drag-drop, riding `/run` and
`/steering` as native vision blocks), and **split view** (⌘\ — two sessions
side-by-side with independent transcripts; `activeId` names the focused pane
so composer/approvals/stop/dock act where the user is looking, a badge strip
on each half moves focus by swapping contents without repositioning panes,
and a keyboard-accessible divider persists its ratio).

Next up (in steal-list order): browser preview/auto-verify — the last major
item. Everything else from the Claude/Codex/Cursor steal-lists that fits our
no-cloud, local-first thesis is now shipped.

### User scope and project scope

Desktop deliberately has two configuration entry points. The **Settings**
button at the bottom of the left sidebar opens *User settings*: provider and
model defaults, permission defaults, and the shared MCP registry stored in
the user's global `config.toml`. The gear in the workspace header opens
*Project settings*: the active folder's `.vak/config.toml` overlay. A project
inherits the user layer through `vak_config::load_with_trust`; a project value
overrides the corresponding user value and no project selection ever changes
the user default by accident.

The Settings surface itself always exposes a **Shared / This project** switch,
so the entry point does not become an invisible scope rule. Shared is the
default layer; a project can add a local capability or explicitly replace a
same-named MCP server for that project only. Hooks are additive across the two
layers, while skills and plugins visibly retain their source scope. Plugin,
catalog, and key actions carry an explicit scope too: an action on a shared
item can never silently mutate an equally named project item.

The API makes this distinction explicit: `PATCH /config/global` and
`GET|PUT /config/mcp/global` mutate the user layer; `PATCH /config` and
`GET|PUT /config/mcp` address the active project. Writes are atomic and reload
the running Core before responding, so Desktop, Admin, gateway cores, and
future turns all observe the same real configuration. Secrets remain in the
user `.env`/secret store and MCP configuration only contains an environment
variable reference such as `${TAVILY_API_KEY}`. Feeds remain workspace-scoped:
they are data and sources belonging to the active project, never an implicit
global capability.
