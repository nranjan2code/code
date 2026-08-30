# 40 — Competitive landscape: deep feature-by-feature comparison

Written 2026-08-30. Sources: primary docs, repos, advisories, and public
changelogs for each product; vak design docs 00–39 and crate source. This
is a living document — update when a competitor ships or when a vak phase
lands.

## Positioning

vak is a **personal agent OS**: an always-on, self-hosted runtime that
compounds memory, automation, and multi-surface reach around one person's
work — not only coding. The closest structural peers are **OpenClaw** and
**Hermes Agent** (always-on agent platforms). Coding-focused tools
(Claude Code, Cursor, Aider, OpenCode, Cline, Codex CLI, OpenHands,
Goose, Devin) overlap on the agent-loop and tool layer but miss the
gateway, channel, automation, learning, and operational-control planes
that define the category.

---

## 1 — Architecture & runtime model

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Language** | Rust (Edition 2024) | TypeScript + Swift | Python | TypeScript (Rust rewrite underway) | TypeScript (VS Code fork) | Go | Python | Rust (Tauri) | TypeScript (SDK) | Rust (2026 rewrite) | Python + TypeScript | Proprietary |
| **Runtime shape** | Headless SDK → server → TUI/desktop/channels | Gateway process; plugin-first | Persistent daemon; five-pillar runtime | CLI tool; agentic loop | IDE (VS Code fork) | Client-server (Go TUI + HTTP server) | CLI pair-programmer | CLI + desktop + API | VS Code ext → SDK runtime | CLI + desktop + web | Web-based agent server + Docker sandbox | Cloud SaaS agent |
| **Local-first** | ✅ all data on disk; no cloud dependency | ✅ | ✅ self-hosted VPS/local | ✅ local execution | ❌ inference infra cloud-side | ✅ | ✅ | ✅ | ✅ | Hybrid (local + cloud sandbox) | ✅ self-hosted | ❌ cloud-hosted |
| **Daemon / always-on** | ✅ `serve --gateway`; launchd/systemd supervised | ✅ gateway loop | ✅ persistent runtime | ❌ session-scoped CLI | ❌ IDE process | ❌ session-scoped | ❌ session-scoped | ❌ session-scoped | ❌ session-scoped | ❌ session-scoped | ❌ task-scoped | ✅ cloud-persistent |
| **Binary size / overhead** | ~10 MB; no Node/Python runtime | ~155 MB (Node + plugins) | Python venv + deps | Node + dependencies | ~400 MB (Electron fork) | Go binary (~20 MB) | Python + pip | ~15 MB (Tauri) | Node + VS Code | ~15 MB (Rust) | Docker image ~2 GB | N/A (cloud) |

---

## 2 — Provider & model layer

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Multi-provider** | ✅ Anthropic, OpenAI Completions, OpenAI Responses, Google, OpenRouter, Ollama; raw APIs, no meta-SDK | ✅ via plugins | ✅ any OpenAI-compatible endpoint | ❌ Anthropic only | ✅ frontier models + proprietary | ✅ 75+ via config | ✅ any API or Ollama | ✅ any provider or local | ✅ any provider or local | ❌ OpenAI only | ✅ any API | Proprietary multi-model |
| **Frozen route ladder** | ✅ ordered candidate legs frozen at session admission; typed failure-domain fallback; demand-scored objectives (v2); cross-model opt-in (`[route]` config) | ❌ single provider per session | ❌ single endpoint | ❌ | ❌ | ❌ | ✅ Architect/Editor split (2 models) | ❌ | ❌ | ❌ | ❌ | Internal multi-model |
| **Evidence-driven routing** | ✅ `routing-evidence.jsonl` ledger; per-attempt attribution; belief demotion (domain-weighted, floor 0.1); diversity caps | ❌ | ❌ | ❌ | ❌ proprietary | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Work receipts** | ✅ every dispatch → typed `receipt` ledger entry (reason, domain, settlement, usage, leg attribution) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ACU billing (opaque) |
| **Live model discovery** | ✅ `models.rs` TTL-cached; warm legs only in ladder | ❌ | ❌ | ❌ | ✅ model picker | ✅ model list | ✅ model list | ❌ | ✅ model picker | ❌ | ✅ model picker | ❌ |

---

## 3 — Session & state layer

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Session format** | Append-only JSONL trees; frozen contract; branching via `parent_id`; compaction-as-entry; projection-neutral receipts | Proprietary state | SQLite + dual-file (user.md / memory.md) | Conversation history | IDE state + chat | SQLite local | Git commits as history | Session state | Workspace state | Persisted state machine | Server-managed conversations | Cloud state |
| **Cross-surface continuity** | ✅ one ledger shared by TUI, desktop, exec, server, gateway channels | ❌ per-surface state | Partial (memory persists) | ❌ CLI ≠ desktop (#24197 complaint) | ❌ IDE-only | ✅ client-server separation | ❌ terminal-only | ❌ per-surface | ❌ per-surface | Partial (CLI + web) | ❌ web-only | ✅ cloud-unified |
| **Side chats / branches** | ✅ ledger branches (`parent_id` child, non-canonical); never merges back | ❌ | ❌ | ✅ `/btw` side threads | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Checkpoints + rewind** | ✅ atomic checkpoint ledgers; `/rewind` | ❌ | ❌ | ✅ snapshots | ❌ | ✅ `/undo` `/redo` (Git-based) | ✅ Git commits | ❌ | ❌ | ✅ branch-based | ❌ | ❌ |
| **Context compaction** | ✅ partition-aware compaction; selected/dropped accounting; 6-metric deterministic scorecard (recall floor, verbatim exclusion, evidence visibility, tool-pair boundary, repeat-compaction) | ❌ | ❌ | Basic summarization | Proprietary | ❌ | ❌ repo-map structure | ❌ | ❌ | ❌ | ❌ | Internal |
| **FTS5 search index** | ✅ SQLite FTS5 over all content blocks (tool calls, results, thinking); BM25 + snippet; rebuildable from JSONL | ❌ | ✅ SQLite FTS5 | ❌ | ❌ vector embeddings | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## 4 — Agent loop & reliability

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Goal mode / audited completion** | ✅ `set_goal` / `run_goal_turn_with`; deterministic `verify:` criteria via brokered bash + skeptical judge; regression obligations; findings re-injected; capped audits degrade to Unverified (never trap) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ `/goal` persisted state machine | ❌ | Partial (CI verification) |
| **Retry + backoff** | ✅ per-step (configurable max_retries + jitter + Retry-After); run-level endurance (6 attempts, exponential backoff, cancel-aware) | Basic | Basic | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | Internal |
| **Circuit breaker** | ✅ per-process; blind-failures-only counting (429 w/ Retry-After never opens circuit); configurable threshold + cooldown; half-close probe | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Watchdog** | ✅ `request_timeout` (default 600s); hung stream → retryable deadline error | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Internal |
| **Stop gate** | ✅ premature-completion guard; model cannot declare "done" without passing verification | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Reset-with-handoff** | ✅ one-shot rescue on over-context; `reset_all` compaction semantics; history handed off, not lost | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Parallel tool execution** | ✅ fan-out scheduler with resource-claim waves | ❌ | ✅ sub-agent delegation | ❌ sequential | ❌ | ❌ | ❌ | ✅ multi-agent | ✅ multi-agent coordinator | ✅ parallel agents | ❌ | ✅ parallel agents |
| **Subagents** | ✅ `task` tool; parent-scoped `SubagentRegistry`; brokered tool registries; attach/steer/stop | ❌ | ✅ spawned sub-agents | ✅ researcher/migrator subagents | ❌ | ❌ | ❌ | ✅ | ✅ coordinator agents | ✅ | ❌ | ✅ multi-agent |
| **Static flows + dynamic planner** | ✅ DAG flows; bounded replan (1 attempt); `flow adopt` from session/ledger; `flow diff`; `--resume` recovery audit | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## 5 — Security & sandboxing

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Permission engine** | ✅ modes × rules → Allow/Ask/Deny; severity aggregation (Deny > Ask > Allow); learned allow rules (`[p]` persists scoped rule); per-call approval with diff preview | ✅ workspace-only tools, default-denied exec | ✅ credential passthrough separation | ✅ tiered Allow/Deny/Ask; managed policies | ❌ implicit trust | ❌ | ❌ | ❌ | ✅ human-in-the-loop per action | ✅ granular approval tiers | Docker sandbox | Cloud-isolated |
| **OS sandbox** | ✅ Seatbelt (macOS) + Landlock (Linux); deny-by-default; explicit read roots (OS/workspace/toolchain/temp); network denied in restricted modes; fail-closed on missing enforcement | ✅ but had wrapper bypass advisories | ✅ container recommended | ✅ Seatbelt + bubblewrap | ❌ | ❌ | ❌ | ❌ local execution | ❌ | ✅ Firecracker/gVisor cloud | ✅ Docker sandbox | ✅ cloud sandbox |
| **Broker-worker protocol** | ✅ every built-in tool → versioned JSON protocol → disposable child process; worker receives scrubbed env, bounded response, no policy/session handles | ❌ | ❌ | ❌ in-process tools | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ runtime-mediated | ❌ |
| **Docker exec backend** | ✅ no-network, RO root, bounded tmpfs, CPU/mem/PID ceilings, dropped caps, `no-new-privileges` | ❌ | ✅ recommended | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ cloud containers | ✅ Docker sandbox | ✅ cloud sandbox |
| **MCP server sandboxing** | ✅ external workers; empty-by-default env; process-group teardown; same Seatbelt/Landlock policy | ❌ had auth bypass advisories | ✅ env clearing + safe set | ❌ | ❌ | ❌ | ❌ | N/A (MCP extensions) | ❌ | ❌ | ❌ | ❌ |
| **Credential isolation** | ✅ Bash inherits only operational allowlist (PATH, locale, temp); provider keys never inherited even in full-access; MCP servers get only declared vars | ❌ had sanitizer gaps | ✅ read-only credential mounts | ❌ relied on Seatbelt | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Docker-scoped | Cloud-scoped |
| **Permission revocation** | ✅ mode change → cancel all in-flight runs + deny pending approvals before reporting new state | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## 6 — Memory & learning

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Tiered memory** | ✅ global `USER.md` profile tier + per-workspace `MEMORY.md`; provenance-stamped blocks; hand-editable markdown | ❌ | ✅ `user.md` + `memory.md` (dual-file) | ✅ `CLAUDE.md` per-project | ✅ `.cursor/rules/` | ✅ Agent Skills config | ✅ `CONVENTIONS.md` | ✅ recipes | ✅ `.clinerules` | ✅ project memory | ❌ | Knowledge graph + RAG |
| **Model-invoked `remember`** | ✅ tool writes durable notes mid-run; provenance (timestamp, kind, tag, session); recalled via search with outranking bonus | ❌ | ✅ self-improving skill loop | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Forget / amend** | ✅ explicit `forget_note` / `amend_note`; byte-safe rewrites; FNV id per provenance header; audited | ❌ (150+ "Alzheimer's" complaints) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Cross-session search** | ✅ dependency-free scan+score; mtime-indexed cache; `search_all` across projects; model-visible `session_search` tool | ❌ | ✅ SQLite FTS5 | ❌ | ✅ vector embeddings | ❌ | ❌ tree-sitter repo map | ❌ | ❌ | ❌ | ❌ | Internal |
| **Skill proposals** | ✅ `propose_skill` → human-gated review queue; promote/reject via HTTP/CLI/desktop; Jaccard dedup at proposal time; consolidation review | ❌ | ✅ auto-documented skills; reinforcement loop; Curator pruning | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Reflection loop** | ✅ post-completion auxiliary model call; ≤2 notes + optional skill draft; Jaccard dedup (≥0.55); detached best-effort — never affects reply | ❌ | ✅ self-evaluation loop | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | RL-based improvement |

---

## 7 — Gateway, channels & multi-surface

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Always-on gateway** | ✅ `POST /gateway/inbound`; persisted `surface:chat → session` bindings; busy→steering→continuation chain; multi-tenant `CorePool` | ✅ gateway-owns-everything | ✅ gateway to 20+ platforms | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ cloud API |
| **Channel bridges** | ✅ Telegram (long-poll + ownership lock + hot-standby), Discord, Slack; `InboundChannel` + `InboundRequest::new` rejects empty identity | ✅ WhatsApp, Telegram, Discord, Slack | ✅ Telegram, Discord, Slack, WhatsApp, Signal, Matrix (20+) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ Slack, Teams, Linear, Jira |
| **Chat allowlist lifecycle** | ✅ `allowlist.json` with pending/allowed/denied states; unknown chats land as reviewable `pending`; denied stays sticky; admin API CRUD | ❌ config-file only | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Multi-bot-per-channel** | ✅ `Bot` identity independent of surface; bot→chat→workspace three-tier policy/permission/route chain; per-bot token; `bots.json` store; `/gateway/bots` CRUD | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Approval forwarding** | ✅ gates announced via delivery transports; resolved by yes/no chat replies (strict verdict vocabulary); Telegram inline-keyboard buttons; timeout fail-closed (300s) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Channel capability overlays** | ✅ per-chat: inherit/narrow built-in tools, MCP patterns, visible skills, hooks; `None` inherits, `Some([])` blocks; deny wins | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Semantic output engineering** | ✅ `OutputTimeline` / `PresentationDocument`; closed AST; capability projection per surface; deterministic CommonMark compiler; Telegram HTML projection; adapter registry; durable retry outbox | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## 8 — Automation & scheduling

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Cron scheduler** | ✅ 5-field cron expressions + intervals; local time; missed-run catch-up on startup; `deliver_to` push; CLI + TUI + desktop CRUD | ❌ | ✅ built-in cron (Pillar 4) | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ cron/CI pipelines | ❌ | ✅ event-driven GitHub | ❌ |
| **Script watchdog tasks** | ✅ brokered bash; empty stdout = silent tick = 0 tokens; nonzero exit = error alert; per-task model pinning (never escalates) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Heartbeat proactive check-ins** | ✅ `[heartbeat]` config; model-pinned cheap; quiet hours; findings park as inbox items; URGENT pushes to channels; anti-nag by architecture | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Budget alerts** | ✅ 80%/100% thresholds; delivered to surfaces once per window; `budget_alert` inbox items | ❌ ($2,100 overnight complaints) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ACU billing |
| **Task model pinning** | ✅ per-task `model:` pin; pinned dispatch only uses that model; never escalates to expensive models | ❌ (cheap-model jobs wake expensive models) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## 9 — FinOps & cost control

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Pricing model** | BYOK (direct API costs) | BYOK | BYOK | Subscription (Anthropic) | Subscription | BYOK | BYOK | BYOK | BYOK | Usage-based subscription | BYOK | Subscription + ACU |
| **Budget admission gate** | ✅ `SpendGate` checked before every paid dispatch; `[finops]` day/month caps + price overrides; denial = one bounded Ask (unattended auto-denies) then permanent typed failure | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ACU limits |
| **Cost ledger** | ✅ `cost-log.jsonl`; estimated-USD rows keyed by session attribution; per-leg FinOps attribution (`CostRow.provider`) | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ token cost tracking | ❌ | ✅ cost tracking | ❌ | ❌ | ACU tracking |
| **Spend visibility** | ✅ TUI `/cost`; desktop budget banner; admin console; subagent token rollup; weekly digest | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ per-session costs | ❌ | ✅ per-session costs | ❌ | ❌ | Dashboard |

---

## 10 — Surfaces & UX

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **TUI** | ✅ rich inline TUI: markdown+syntax rendering, tool cards w/ diffs, approval queue, multiline/paste input, completions, Ctrl-R history, session browser, themes (truecolor packs + custom), vim/emacs composer modes, accessibility modes | ❌ | ❌ CLI | ✅ terminal-based | ❌ IDE | ✅ Bubble Tea TUI | ✅ terminal chat | ✅ CLI | ❌ (IDE extension) | ✅ terminal | ❌ web UI | ❌ web UI |
| **Desktop app** | ✅ Tauri 2; parallel sessions + worktrees; streaming chat; inline approvals; diff review w/ line-comment steering; split view; subagents tab; MCP manager; tasks/cron; side chats; memory editor; global search; budget banner; voice narration | ❌ | ❌ | ✅ Claude Desktop | ✅ (is the IDE) | ❌ | ❌ | ✅ Tauri desktop | ❌ | ✅ macOS/Windows | ✅ web canvas | ✅ Devin Desktop |
| **Web admin console** | ✅ SolidJS SPA at `/admin`; observation (overview/search/security/inbox); operation (approvals/config/cancel/live-tail); interaction (prompts/steering/best-of-N); cookie auth; real-time SSE | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ web canvas | ✅ web dashboard |
| **Inbox / attention layer** | ✅ durable `inbox.jsonl`; task_summary / approval_pending / budget_alert / digest / heartbeat / proposal_opened; ack tombstones; desktop badge + TUI `/inbox` + CLI `inbox` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Service tray** | ✅ menu-bar controller; colour-coded service dot; start/stop/restart/install/uninstall; logs; persisted watchdog w/ auto-restart + notifications | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## 11 — Tools & extensibility

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Built-in tools** | read/write/edit/bash/glob/grep/webfetch/browse/session_search/remember/propose_skill/task | file/exec/browser/etc. | sandboxed Python/bash, browser automation | file/bash/search | IDE-integrated | file/bash/search | file edits via diff | MCP extensions | file/terminal/browser | file/bash | file/shell/browser | full IDE + terminal + browser |
| **MCP client** | ✅ lazy meta-tool; sandboxed external workers; empty env by default; process-group teardown | ✅ (but had auth advisories) | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ (architecture core) | ✅ | ✅ | ✅ | ❌ |
| **Skills system** | ✅ SKILL.md + YAML frontmatter; user/project/plugin namespaces; visibility controls (not permission bypass); model-proposed + human-gated | ✅ ClawHub | ✅ auto-documented procedural skills | ❌ | ❌ | ✅ Agent Skills | ❌ | ✅ recipes | ❌ | ✅ skills + marketplace | ❌ | ❌ |
| **Hooks** | ✅ lifecycle hooks: pre/post-tool-use, stop, session-start; `enabled` flag; disabled = data not absence | ❌ | ❌ | ✅ hooks for auditing | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Plugin ecosystem** | ✅ versioned capability packages (`vak-plugin.json`); inspect→install→review→enable→update→rollback→uninstall lifecycle; content-addressed storage; digest-bound approvals; immutable provenance chain | ✅ ClawHub extensions | ✅ plugin system | ❌ | ✅ extensions (VS Code) | ✅ oh-my-opencode | ❌ | ✅ MCP extensions | ✅ VS Code extensions | ✅ plugin marketplace (2026) | ✅ SDK | ❌ |
| **Web fetch + browse** | ✅ bounded webfetch (GET, ≤3 redirects, 15s, 512KB, SSRF guard); headless browse (Chrome, sentinel completion, direct-pid SIGKILL) | ✅ browser automation | ✅ vision-based browser | ❌ limited | ❌ | ❌ | ❌ image/web context | ❌ Computer Controller | ✅ headless browser | ❌ | ✅ browser | ✅ built-in browser |

---

## 12 — Operations & deployment

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Service management** | ✅ `vak-ops` over launchd/systemd; tray + TUI + desktop + HTTP control; idempotent start/stop/restart/install/uninstall | ❌ manual process management | ✅ self-hosted daemon | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Docker Compose | Cloud-managed |
| **Doctor / diagnostics** | ✅ `vak doctor`; health checks; `--repair` only for mechanical fixes; gateway channels check w/ repair for expired pending | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **First-run wizard** | ✅ tty-only provider-key setup; skippable; marker file; runs once | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ web setup | ❌ |
| **Backup export/import** | ✅ directory copy of sessions/memory/config/checkpoints; secrets excluded unless `--include-secrets`; import conflicts skip-or-rename | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Weekly digest** | ✅ `vak digest [--days N]`; usage report; schedulable as task | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Network resilience** | ✅ bridge survives DHCP/outage/hibernation; capped backoff + cursor resume; scheduler catch-up; inbox stores durably | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Cloud HA |

---

## 13 — Voice & output

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Voice synthesis** | ✅ Gemini Live `BidiGenerateContent` WebSocket; per-bot/per-chat voice override (inheritance chain); `POST /voice/speak`; desktop narration via shared `<audio>` element; admin Preview button; WorkReceipt auditing | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ voice-to-code input | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Semantic output contract** | ✅ `OutputRole` × `OutputKind` × `OutputStatus`; `PresentationDocument` closed AST; `SurfaceCapabilities` projection; trusted templates (propose/activate lifecycle); ordered multi-message; durable retry outbox | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Channel-adapted rendering** | ✅ CommonMark, Telegram HTML, Slack markdown, Discord markdown, webhook JSON — all from same AST; degradation diagnostics | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## 14 — Eval & verification

| Capability | vak | OpenClaw | Hermes | Claude Code | Cursor | OpenCode | Aider | Goose | Cline | Codex CLI | OpenHands | Devin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Built-in eval harness** | ✅ deterministic suite (~100ms) + live-model mode; context-quality scorecard (6 metrics); scenario scripts (exec, audited goal, plan→adopt→run→diff) | ❌ | ❌ | ❌ internal | ❌ internal | ❌ | ✅ SWE-Bench benchmarks | ❌ | ❌ | ❌ | ✅ SWE-Bench evaluation | ✅ internal benchmarks |
| **Flow adopt / diff** | ✅ `flow adopt <ledger|session>` (frozen-TOML reuse w/ provenance); `flow diff <A> <B>` deterministic node comparison; `--resume` recovery audit | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## Summary: categorical positioning

| Dimension | Leader | Unique to vak |
|---|---|---|
| **Personal agent OS** | vak, OpenClaw, Hermes | Frozen route ladder, stop gate, audited goal completion, broker-worker protocol, permission revocation, FinOps admission gate, semantic output engineering, multi-bot-per-channel, channel capability overlays, flow adopt/diff, context scorecard |
| **Always-on multi-channel** | vak, Hermes, OpenClaw | Allowlist lifecycle with pending state, approval forwarding to channels, bot→chat→workspace policy chain, Telegram ownership lock + hot-standby |
| **Coding agent** | Claude Code, Cursor, Codex CLI, vak | Same-boundary permission+sandbox (not just Bash), learned allow rules, reset-with-handoff, regression obligations |
| **Reliability engineering** | vak | Circuit breaker (blind-failures-only), run-level endurance, watchdog, frozen-ladder fallback, evidence-driven routing beliefs |
| **Cost sovereignty** | vak | SpendGate pre-dispatch admission, per-leg attribution, proactive alerts to surfaces, task model pinning (never escalates) |
| **Learning that compounds** | Hermes, vak | Human-gated skill promotion (Hermes auto-promotes), explicit forget/amend (Hermes lacks), Jaccard dedup, reflection loop |
| **Operational maturity** | vak | launchd/systemd service units, tray controller, doctor w/ mechanical repair, backup export/import, weekly digest, network resilience matrix |

---

## Known gaps (vak trails)

| Gap | Who leads | Path to close |
|---|---|---|
| **Community / ecosystem size** | OpenClaw (100k+ stars), Cursor (millions of users) | Plugin ecosystem (doc 39) + distribution (doc 37) |
| **Cloud sandbox / remote execution** | Devin, Codex CLI, OpenHands | P2 in doc 24: pinned full-worker image + VM/remote backend |
| **LSP / language-aware editing** | Cursor, OpenCode | Not planned — vak is not an IDE; surfaces project into user's editor |
| **Real-time voice conversation** | Pi (Inflection) | Non-goal this pass (doc 38); current = one-shot text→audio per turn |
| **Team / enterprise multi-user** | Devin, Cursor | Explicitly deferred (doc 29); single-user personal OS is the thesis |
| **GUI automation (screen control)** | Goose (Computer Controller) | Not planned — different threat model |
| **WhatsApp / Signal / Matrix bridges** | Hermes (20+ platforms) | Extension of existing `InboundChannel` + adapter pattern; no architecture change needed |
