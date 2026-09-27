# 29 — Personal OS
Status: implemented in 2.0.0

Personal-use completion pass, cross-referenced against a codebase inventory.
Enterprise concerns are explicitly deferred.

Thesis: vak already neutralizes the two worst failure modes in the
category — **lying** (stop-gate, audited goal completion, receipts) and
**money burn** (SpendGate, day caps). This doc closes the remaining gaps so
the system compounds for a single person: memory that follows the user,
automation that can be trusted unattended, first-run trust, reach, and
self-curating skills.

## User needs (2026-08)

| Signal | Source | Implication |
|---|---|---|
| Breaking updates can disrupt established setups | User feedback tracking | frozen contracts stay; add update *awareness*, never auto-update |
| Agents corrupt their own memory/config ("Alzheimer's") | 150+ tracked complaints | memory writes provenance-stamped; forget is explicit + audited |
| Cron/heartbeat unreliably fire; cheap-model jobs wake expensive models | same | per-task model pinning; silent-tick watchdog jobs never touch inference |
| $2,100 overnight bills; caps discovered only at denial | same | proactive budget alerts at 80%/100%, delivered to surfaces |
| People value editable profiles, procedural skills, and memory cleanup | User feedback and product review | adopt tiered memory, keep promotion human-gated |
| Session memory can be lost between runs | Community feedback | global profile tier recalled in every run |
| Skill collections become noisy over time | Product review | deduplicate proposals and require consolidation review |

## Phases

| Phase | Delivers | Exit criterion |
|---|---|---|
| **P1 ✅** | Memory that compounds: global `USER.md` profile tier (`<home>/memory/user/USER.md`), `MemoryStore::forget`/`amend` with rewrite-not-tombstone semantics (memory files are hand-editable by design; sessions stay append-only), reflection hook available on every surface behind `[memory] reflection`, M1 mtime-keyed search index, TUI `/search`, `search_all` across projects | warm 10k-line store searched < 50ms; forget removes exactly one block; USER.md notes outrank equal transcript hits; fmt/clippy/tests green |
| **P2 ✅** | Automation you can trust: 5-field cron expressions (`m h dom mon dow`, numbers, `*`, lists, ranges, steps; local time) beside existing intervals, CLI+TUI task CRUD without the server, `script:` watchdog tasks (brokered bash; empty stdout = silent tick = zero tokens; nonzero exit = error alert), per-task `model:` pinning (pinned task dispatches only that model — never escalates), missed-run catch-up on startup, budget alerts via delivery transports once per threshold window | cron next-fire unit matrix green; watchdog task with failing script delivers alert and costs 0 tokens; pinned task receipt shows pinned model only |
| **P3 ✅** | Daily-driver trust: `vak doctor` (health module extracted from TUI), first-run provider-key prompt (tty-only, skippable), opt-in `[update] url` version check (off by default, banner once/day, never auto-installs), `vak backup export/import <dir>` (directory copy of sessions/memory/config/checkpoints; secrets excluded unless `--include-secrets`; import conflicts skip-or-rename), `vak digest [--days N]` weekly usage report | doctor parity TUI/CLI; export→import round-trip preserves ledgers byte-identical; digest math matches ledger |
| **P4 ✅** | Reach: bounded `webfetch` tool (GET only, ≤3 redirects, 15s timeout, 512KB cap, SSRF guard rejecting private/link-local/loopback resolutions fail-closed, no credentials ever attached, permission-classified network-capable ⇒ Ask in restricted modes, Allow only in FullAccess or by explicit rule; ordinary logged tool result per invariant 1), headless `browse` tool driving local Chrome-family binaries from inside the broker worker (`--dump-dom`, sentinel completion, direct-pid SIGKILL — doc 04), server-side markdown transcript export reusing one shared renderer with the TUI, global `/search?all=true` | SSRF unit matrix (localhost/metadata/link-local denied); fetch lands on ledger; live browse probe: sentinel ~2s against real Chrome, zero orphan processes; exported md equals TUI export for same session |
| **P5 ✅** | Skill curation: proposal-time similarity screen (Jaccard vs existing accepted skills ≥ threshold ⇒ tagged `duplicate-of`), consolidation proposals into the existing human-gated review queue — no auto-promotion, no auto-deletion | duplicate skill proposal is flagged at submit; consolidate run produces reviewable merge entries only |
| **P6 ✅** | Inbox — the durable attention layer: every `gateway::deliver` also records an append-only `<home>/inbox.jsonl` entry (`task_summary · approval_pending · approval_denied · budget_alert · digest · heartbeat · proposal_opened`), read-state via ack tombstones (never delete), surfaces: desktop Inbox page + badge, TUI `/inbox`, CLI `inbox list/show/ack`; unattended signals survive without any chat channel configured | delivery chokepoint proven: a watchdog summary exists in inbox even with zero transports configured; unread math + idempotent ack unit-tested; each surface shows identical unread set |
| **P7 ✅** | Heartbeat on top of the inbox: `[heartbeat]` config (interval, model pin default cheap, quiet hours); proactive "anything need attention?" turns whose findings park as inbox `heartbeat` items — push to channels only for urgent kinds (anti-nag by architecture) | heartbeat with nothing to report costs tokens but produces zero pushes; finding parks read/unread correctly across surfaces |

## Surfaces rule

Every personal-os capability ships on **all three surfaces** in the same
phase: CLI/TUI, desktop SPA, and gateway channels where delivery applies.

| Capability | TUI/CLI | Desktop | Channels |
|---|---|---|---|
| Memory profile + forget/amend/cleanup | `/memory`, `vak memory` | Admin/desktop Learning page: profile tier, effective policy status, cleanup artifacts, forget (confirm), inline amend | — |
| Cross-project search | `/search --all`, CLI | Search page w/ global toggle | — |
| Tasks/cron/watchdog/pinning | `/tasks`, `vak tasks` | TasksModal: schedule grammar field, script field, model pin, catch-up badge | watchdog + task summaries delivered via existing transports |
| Budget alerts | TUI status strip marker | Budget card alert state | 80%/100% push once per window |
| Doctor/health | `vak doctor`, `/doctor` | Services panel diagnostics section | — |
| Backup export/import | `vak backup …` | Settings: folder picker → export/import buttons | — |
| Weekly digest | `vak digest` | Digest card | scheduled digest as an ordinary task (`deliver_to`) |
| webfetch / browse | approval prompt + transcript (url shown) | approval queue w/ URL shown | forwarded gates per doc 22 rules |
| Skill curation | `skills-review consolidate` | Learning page duplicate badges + merge review | — |
| Inbox | `/inbox`, `vak inbox list/show/ack/count` | InboxPage + bell badge + ack/ack-all | entries mirror every delivery (single chokepoint) |
| Heartbeat | status-row ✉ badge when findings unread | inbox kind chip + native notification while hidden | URGENT findings push; everything else parks |

Server gains the backing endpoints so the embedded router inside desktop
needs no special-casing; unattended surfaces keep failing closed (doc 22,
invariant 15).

## v0.5.0 dogfood postscript

The live-fire pass (restored-real-data sandbox home, `serve --gateway`)
caught two defects before any user did — both fixed in the release:

1. **transcript.md 404 for historical sessions.** Both transcript endpoints
   only saw the in-memory handle map; a fresh server process starts empty.
   Fixed with a disk fallback (`open_historical_session`), wire behavior of
   the JSON endpoint preserved byte-for-byte (error bodies stay 200-wrapped;
   pinned by the gateway contract suite).
2. **No HTTP memory-append.** Desktop wrote via a Tauri-only command, breaking
   surface symmetry. `POST /memory` now covers both tiers; an argument-order
   trap between `append_note` and `append_profile_note` (different param
   orders) was caught by the live test and is documented here as a warning.

Also proven live: backup export→import byte-identical on real data,
cron/watchdog/catch-up firing over the real scheduler loop, budget alerts
delivered once per window, and FNV note ids reproducible from header lines
outside the process.

## Non-negotiables carried over

- Invariant 1: USER.md recall, webfetch/browse output, digest data reaching a
  model all arrive as logged session entries. Nothing ambient.
- Invariant 2: sessions untouched; memory/skill stores remain plain,
  hand-editable markdown outside the ledger.
- Invariant 3: every new failure (SSRF reject, cron parse error, backup
  conflict) is a typed value, never a panic.
- No new dependencies. HTTP reuses workspace-pinned reqwest 0.12; cron and
  tar-free directory backups are hand-rolled.
- Secrets stay out of backups by default; `--include-secrets` warns loudly.
