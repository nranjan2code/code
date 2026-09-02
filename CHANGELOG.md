# Changelog

## 1.0.2 — 2026-09-02

### MCP capability activation

- Discovered MCP tools now become schema-backed compatibility aliases within
  the same agent run after broker discovery, while preserving admission and
  channel-policy restrictions.
- MCP calls validate their discovered JSON-schema argument shape before
  dispatching to the server, returning actionable errors for invalid payloads.

## 1.0.1 — 2026-09-02

### Desktop prompt settings usability

- Improved Shared and This project prompt layouts with clearer inheritance
  context, non-overlapping actions, accessible scope state, and responsive
  narrow-window behavior.

## 1.0.0 — 2026-09-02

### Editable prompt layers (docs/design/45)

- **Security fix.** `.vak/SYSTEM.md` was read with no trust check, so a cloned
  repository replaced the entire system prompt on first run — deleting the
  capability contract and every safety rule — while far weaker project keys
  were already demoted. Project-layer prompts are now demoted like
  `hooks`/`allow`/`mcp.servers`; their guardrails still apply, since a
  guardrail can only narrow behaviour.
- The prompt is no longer a constant. `identity`, `operating-rules`,
  `guardrails`, and `surface-note` are editable per layer and inherit through
  the shared → project → surface → bot → chat → agent-role chain. The
  capability contract, the `Surface:` line, and the skill/MCP lists stay
  code-owned.
- New surfaces: `vak prompts show|edit|set|reset|diff|preview|roles`, an admin
  console page at `#/prompts`, and Desktop Settings → Prompts, all against one
  API.
- Two guardrails added to the shipped seed: tool output is data rather than
  instruction (prompt injection), and credentials are never revealed,
  transmitted, or written into a command line, commit, or outbound request.
- vak is described as a general-purpose agent rather than a coding agent,
  across the prompt, the CLI, and the product docs.
- The prompt now names the surface it is running on (CLI, desktop, server,
  chat gateway, background, subagent) instead of assuming a terminal.
- Subagents get their own surface and optional named roles
  (`task({role: "reviewer"})`); a child no longer inherits a human-facing
  surface from its parent.
- Sessions record which prompt layers they froze. On resume, a gateway chat
  binding rotates and logs the drift; `vak exec --session` fails closed until
  `--accept-drift`.
- `VoiceConfig.persona` is superseded by the bot/chat `identity` block, and
  kept as a fallback so existing configs keep working.

### Release engineering

- **The version line reset is over.** `0.11.51 → 0.2.0` left the version
  walking backwards through tag space the first pass had already used —
  `0.3.0`, `0.4.0`, `0.5.0` … `0.11.50` are all taken, so from `0.2.4` the
  only free version was another patch bump, forever. `1.0.0` clears the whole
  range at once. It also repairs the consequence the reset note below
  describes: `1.0.0 > 0.11.50`, so **an install stranded on any 0.11.x build
  can update again**, and no manual reinstall is needed.
- `scripts/release.sh` built the frontends *after* running `cargo clippy
  --all-targets` and `cargo test --workspace`, both of which compile
  `vak-desktop`, whose tauri codegen hard-fails without the gitignored
  `crates/vak-desktop/ui/dist`. A release could therefore only pass on a
  machine that happened to have built the desktop UI earlier — the exact
  leftover-state dependency the script exists to remove. The frontends are
  built first now.
- `scripts/check-version.sh` could not run on any checkout without a local
  `dist/`: under `set -o pipefail` a failing `ls` killed the script mid-report,
  before the monotonicity check it feeds. With that fixed, the check
  underneath was also wrong — it matched the exact highest shipped version
  instead of the abandoned release *line* its own comment describes, and
  0.11.51 shipped untagged so the two never agreed. Releases were blocked.
- A stale frontend bundle is now a build error. `npm run build` stamps
  `dist/.src-manifest`, and both build scripts fail with the offending
  filename instead of silently embedding the previous bundle — the failure
  that shipped a blank admin console in v0.8.1. `scripts/build.sh` also builds
  the admin frontend now.

## The version line reset, and its end

The workspace version was deliberately reset from `0.11.51` to `0.2.0` in
`fc9c78f`, and ran as `0.2.0`–`0.2.4`. This is a real discontinuity, not a
typo, and it had two consequences.

**While it lasted, an install on any 0.11.x build could never be offered an
update.** Update checks compare by semver precedence and `0.2.x < 0.11.x`, so
`vak self update` reported nothing available and moving forward needed a
manual reinstall.

**It also walked into occupied tag space.** The first pass through the version
line had already shipped `0.3.0`, `0.4.0`, `0.5.0` and everything up to
`0.11.50`, so the reset line could never take a minor bump — only patches,
indefinitely.

`1.0.0` (2026-09-02) ends both. It sorts above the entire retired line, so
stranded `0.11.x` installs update normally again, and the whole `1.x` range is
unused. Releases before `1.0.0` belong to one of the two earlier lines; read
their dates, not their ordering.

Entries below `0.11.23` were written by hand. The `0.11.24`–`0.11.51` block is
**reconstructed from commit subjects** — those releases shipped without
changelog entries, and this records what went into them rather than leaving
28 releases invisible. Treat it as an index into `git log`, not as prose
written at the time.

<!-- reconstructed:begin 0.11.24..0.11.51 -->

## 0.11.51 — 2026-09-01

- fix(desktop): show inherited global MCP servers, hooks, and plugins in settings

## 0.11.50 — 2026-09-01

- Harden agent sandbox and workspace networking
- feat: consolidate pending platform and security updates

## 0.11.49 — 2026-09-01

- fix(desktop): stabilize transcript rendering modes

## 0.11.48 — 2026-09-01

- Fix desktop project bootstrap

## 0.11.47 — 2026-09-01

- Fix direct MCP tool dispatch
- Add VAK architecture documentation and story videos

## 0.11.46 — 2026-09-01

- Prevent premature agent completion
- Harden MCP discovery and secret redaction

## 0.11.44 — 2026-08-31

- Validate MCP tools against server discovery
- Add session ledger diagram
- Add runtime architecture diagrams

## 0.11.43 — 2026-08-31

- Add architecture diagrams
- Add managed flow dispatcher
- Close managed work verification gaps
- Harden managed work execution and recovery
- Harden managed work execution lifecycle
- Fix managed work review findings
- Implement managed work contracts
- Fix historical feed reader data
- Classify recovered tool errors correctly
- Rotate gateway sessions on capability changes

## 0.11.39 — 2026-08-31

- Unify capability dispatch and enforce write scopes
- Make capability evidence provider-neutral
- Make skill tool boundary explicit
- Harden regression harness accuracy and recovery
- Harden provider routing and agent reliability
- Fix operations active tab and refresh flicker
- Fix operations navigation and scrolling

## 0.11.36 — 2026-08-30

- Ship evidence-backed operations center

## 0.11.35 — 2026-08-30

- fix(feeds): Add/Edit/Remove on the Sources tab operated on the wrong store

## 0.11.34 — 2026-08-30

- fix(feeds): prune_seen's parameterized INTERVAL was a parser error
- fix(feeds): stop concurrent reads from racing DuckDB's exclusive lock

## 0.11.33 — 2026-08-30

- fix(feeds): style the Feeds tab bar like every other tab bar in the app

## 0.11.32 — 2026-08-30

- feat(feeds): expose full source/alert CRUD in admin UI, surface load errors
- fix(doctor): detect and repair the legacy/canonical home split
- chore(admin-ui): rebuild dist to match src

## 0.11.31 — 2026-08-30

- fix(mcp): bounded MCP warm-up for one-shot CLI turns (exec/flow exec/plan)
- fmt: apply cargo fmt to feeds.rs
- fix(feeds): unwrap MCP envelope, fix source/alert config edits, add manage UI

## 0.11.30 — 2026-08-30

- fix(mcp): start tool-inventory warm-up at boot, not at first turn

## 0.11.29 — 2026-08-30

- fix(mcp): reuse McpManager across turns and advertise live tool catalog
- fix(scheduler): close tasks.json reload race that could drop new tasks

## 0.11.28 — 2026-08-30

- fix(mcp,desktop): resolve toolchain PATH, non-mutating discovery, and env diagnostics
- fix: restore reliable turn and service lifecycles
- fix: report managed services accurately
- release: v0.11.25
- fix workspace defaults and scoped settings
- feat: complete secure plugin ecosystem
- feat: vak plugins CLI subcommand (inspect/install) over vak-plugin
- feat: vak-plugin — recognize agent-plugin/claude/copilot/cursor/gemini manifests
- chore: cargo fmt vak-plugin
- chore: fix clippy lints ahead of release (unwrap_used, field-reassign, panic)
- chore: cargo fmt

## 0.11.24 — 2026-08-30

_Version bump only; no other commits in this release._

<!-- reconstructed:end -->

## 0.11.23 — 2026-08-30

### Telegram, Slack, and Discord get dedicated output formatting

Telegram had a rich HTML projection; Slack and Discord did not — both got
raw GFM markdown passed straight through `chat.postMessage`/the Discord
message endpoint, which visibly mis-renders on both surfaces (docs/design/30):

- Telegram's `markdown_to_html` gained headings-as-bold (h1/h2 upshifted),
  ordered lists with real numbering, depth-based nested bullet markers
  (`•`/`◦`/`▪`), merged multi-line `<blockquote>` instead of one per line,
  fenced code with `<code class="language-x">`, GFM tables rendered as an
  aligned monospace grid, strikethrough (`<s>`), spoiler (`<tg-spoiler>`),
  and a horizontal-rule divider.
- New `Markup::SlackMrkdwn` (`vak-delivery/src/slack.rs`): swaps GFM's
  `**bold**`/`*italic*` for mrkdwn's `*bold*`/`_italic_`, converts
  `[text](url)` to `<url|text>`, and renders headings/tables the way
  Telegram's projection does since mrkdwn has neither.
- New `Markup::DiscordMarkdown` (`vak-delivery/src/discord.rs`): Discord's
  markdown already matches GFM for bold/italic/strike/spoiler/fences/quotes/
  headings, so this only fixes `[text](url)` (doesn't hyperlink in a plain
  message — becomes `text (<url>)`, angle brackets suppressing the
  link-preview embed) and adds the same table-as-monospace-block fallback.
- Both new surface delivery profiles in `vak-server/src/delivery.rs` (the
  `discord`/`slack` cases previously shared generic `Markup::Markdown`), and
  a fence-aware chunker (`chunk_markdown_preserving_fences`) so a message
  split at the per-channel character cap closes and reopens an open ` ``` `
  block instead of leaving it dangling across chunks, matching the tag-safe
  chunking Telegram already had.

## 0.11.22 — 2026-08-30

### Voice & personality — Gemini Live synthesis for bots and desktop

Every bot/chat already inherited policy, permission mode, and model route
down the `Workspace → Bot → Chat` chain (docs/design/34). Voice was the
missing tier: no per-bot spoken identity, no way to hear a persona before
committing to it, and no audio capability in desktop at all. Closed with
one mechanism instead of two parallel integrations
(docs/design/38-voice-personality.md):

- `VoiceConfig` (`voice_name` + `persona`) joins `route`/`permission_mode`
  on both `Bot` and `AllowlistEntry`, resolved by the same
  `inherit_bot_policy` switch and the same absent/`null`/object wire
  idiom — no new override semantics to learn.
- `crates/vak-llm/src/google_live.rs`: a from-scratch `BidiGenerateContent`
  WebSocket client (the Live API has no SSE/REST form), mirroring
  `google.rs`'s conventions. Enforces a 25s wall-clock timeout and a
  2,000-char input cap that nothing upstream provided otherwise — without
  them a stalled socket blocks the request task forever, and an unbounded
  `text` field runs up billing on a paid per-call API.
- `POST /voice/speak` serves three callers off one code path: the gateway
  reply pipeline, desktop narration, and the admin console's Preview
  button — all behind the existing bearer middleware, no new auth code.
- Admin console gained a `VoiceConfigEditor` on both the bot and chat
  panels, built from the exact `ChannelPermissionPicker` inherit-toggle
  idiom already established for route/permission-mode.
- Desktop gained its first audio capability: no native audio crate, the
  webview just plays the WAV `Blob` `/voice/speak` returns through one
  shared `<audio>` element, narrating turn completions ("Done."/"Task
  failed.") and approval prompts (tool name + primary arg) — terse by
  design, so narration never becomes the bottleneck on a verbose turn.

## 0.11.21 — 2026-08-29

### A live-editable Settings/Memory/FinOps pass, and a real duplicate-row fix

Follow-on to 0.11.20's bot-scoped channel identity, found while verifying
it live against real Telegram traffic.

Fixed a live duplicate-row bug: a chat manually bot-bound before
auto-attribution existed kept a dead, bound session alive under its legacy
key once a bot-scoped sibling took over — it never received traffic again
but still showed as a confusing near-duplicate in Chats. Its stale
session is now unbound the moment a bot-scoped key inherits from it; the
legacy allowlist row itself stays, so a later bot still has an ancestor to
inherit from.

The admin console's Memory, Settings, and FinOps pages all had the same
shape of gap: a value the backend tracked but either never actually
returned to the console, or had no live-apply path at all — a change
would silently do nothing until a restart, or the field simply always
read `undefined`/`$0`. Fixed all three the same way, extending the
override+refresh mechanism route/theme/max_turns already used:

- Memory's search/write/reflection/skill-proposals toggles are real
  checkboxes now, not dead status text.
- Settings gained working Max turns and Sub-agents controls.
- FinOps gained a whole dedicated page — spend trend chart, budget caps
  you can actually edit, by-provider/by-model breakdowns, recent
  alerts — replacing an Overview stat that always read $0 because its
  type never matched what `/finops` actually sent.

## 0.11.20 — 2026-08-29

### Bot-scoped channel identity, and the multi-bot units that make it real

0.11.19 gave a bot its own token, policy, and permission tier, but shipped
with two gaps this release closes.

First, the boring but load-bearing one: `services-sync` never actually
spawned a bridge process per bot — the only Telegram unit was a single
static one with no `--bot-id`, so it fell back to the dead legacy
`TELEGRAM_BOT_TOKEN` slot and neither configured bot ever polled Telegram.
Service sync now reads `bots.json` and reconciles one `com.vak.<surface>-
<id>` unit per bot on every install, update, and manual sync, pruning units
for bots that get deleted. Creating, deleting, or rotating a bot's key now
also reconciles its unit immediately from the admin console, no restart
needed. Along the way, each bridge process now tells the gateway which bot
it is on every inbound message (it always knew — it just never said), so a
new chat is attributed to its bot from the first message instead of
forcing an operator to pick one by hand for a fact the bridge already had.

Second, the bigger one: a *channel*'s identity was still `surface:chat`,
shared by every bot on it — two bots in the same physical chat collided
onto one conversation and one policy, and outbound replies picked one
bot's token for the whole surface arbitrarily (0.11.19's documented
"known gap"). A channel key is now `surface:chat:bot_id` whenever a bridge
names its own bot, giving each bot on a physical chat its own session,
policy, and permission tier — genuinely independent, not just
independently configured. An already-approved chat, or a `chat_allowlist`
config row, needs no re-approval the moment a bot id starts arriving: it's
inherited forward automatically. Outbound delivery now resolves the exact
bot a reply belongs to instead of guessing. See
docs/design/34-channel-onboarding.md Phase 6.

Also: the admin console's Connect page gained a real settings panel per
bot (workspace, model, permission mode, and the same tools/MCP/skills/
automations/network policy editor the per-chat view already had), and the
Chats list gained a BOT column so a channel's bot binding is visible at a
glance instead of only in the row editor. Fixed a `PATCH` bug along the
way where clearing a bot's (or a chat's bot binding's) field back to
"unset" via an explicit `null` silently did nothing — a classic
`Option<Option<T>>` deserialization trap.

## 0.11.19 — 2026-08-29

### Multi-bot-per-channel

A channel used to be its bot: `TELEGRAM_BOT_TOKEN`/`DISCORD_BOT_TOKEN`/
`SLACK_BOT_TOKEN` were one env var per surface, and setting a second token
silently overwrote the first — there was no way to run two bots on the same
platform, and no way to give one bot its own tools/MCP/skills/hooks/network
policy, permission mode, or model separate from a chat's own settings.

`Bot` is now a first-class identity, independent of surface: its own token,
its own `ChannelPolicy`, permission mode, route, and workspace, stored in
`bots.json` alongside the existing allowlist/bindings stores. Resolution is
a three-tier chain — bot → chat → workspace — composed with the same
inherit-or-override convention the rest of the gateway already uses:
capability allow-lists let the more specific tier win while deny-lists
accumulate, permission modes cap in sequence (never escalating), and
model/route falls through chat → bot → legacy binding → workspace default.
Each chat's `inherit_bot_policy` flag is the explicit "break inheritance"
switch — flip it off and the chat resolves purely against the workspace,
ignoring its bot's tier entirely.

`vak telegram/discord/slack --bot-id <id>` runs a bridge process against a
specific bot's token, so a second bot on the same platform actually
receives its own messages — not just holds a saved credential nobody reads.
Existing single-token deployments auto-migrate into one synthesized `Bot`
row per configured surface on first load, so nothing changes for anyone
who doesn't touch this. The admin console's Connect and Credentials tabs
gained an "+ Add another bot" list, and the live chat editor gained a bot
picker and the inherit toggle. See docs/design/34-channel-onboarding.md
Phase 5 for the full design, including the one known gap: outbound replies
aren't yet bot-scoped when two bots share a surface.

## 0.11.18 — 2026-08-29

### Feed pipeline, and an admin console pass for non-technical operators

Added an extensible feed ingestion system end to end: Python drivers pull
RSS/Atom, YouTube, Reddit, Hacker News, Lobsters, and custom HTTP sources
into a DuckDB store with BM25 search, Tavily-compatible responses, alert
rules with outbox delivery, and an MCP stdio server. The Rust server
exposes sources/items/search/stats/alerts/ingest behind a persisted TOML
config with atomic writes; both the admin console and desktop SPA gained
a Feeds section — a three-pane reader, a guided source wizard, and search.

Reworked the admin console's screens that assumed a technical operator.
Channel capability restrictions (tools/MCP servers/skills/hooks) were
nine hand-typed glob fields; they are now `AccessPicker` controls built
from live data (`GET /config/mcp`, `/config/skills`, `/config/hooks`) —
*Everything the workspace allows / Only what I pick / None*, plus a
disclosure for the raw pattern grammar. Scheduled tasks got a preset +
time picker in place of a bare cron string; hook matchers got a
tool/argument builder in place of `Bash(git *)`. Permission modes,
decision outcomes, provider ids, and event/security-kind tags are said
in plain language everywhere, with the wire value kept on hover. Settings
and every other screen were rewritten screen by screen against the same
rule: never bend the wire shape to fit the wording, and never let a
picker discard a stored value it doesn't recognise.

QA on that pass caught three real defects, since fixed: a provider
`<select>` populated by a later fetch kept showing the browser's default
option instead of the configured provider — Settings could display
"Anthropic" while about to save `openai-responses` — fixed with a
`syncSelect` helper reapplied after every async-populated list; a hook
matcher's tool dropdown rendered blank for `Bash(git *)` because the
picker's option values are lowercase and the rule grammar's case
sensitivity meant no option matched (fixed by offering the operator's
own spelling as a first-class option); and editing an interval-based
task sent its `interval_secs` back as an invalid cron string, which
`TaskDef::validate` rejected with a 400 (fixed by leaving the schedule
field empty, which the patch endpoint reads as "keep the interval").

## 0.11.14 — 2026-08-29

### Per-channel MCP network override

`ChannelPolicy` could restrict which MCP servers a channel sees, but
outbound network access was only ever a global, per-server setting
(`McpServerConfig.network`) — there was no way to let one channel use a
server with network on while another channel using the same server got
it forced off. Added `mcp_network_deny: Vec<String>` to `ChannelPolicy`
(same `name/*` pattern shape as `mcp_allow`/`mcp_deny`), enforced in
`Core::filter_mcp`: restrictive only — a channel can take network away
from a server the config already grants it to, never grant it to one the
server config itself denies. Exposed on the Gateway → Channel → Edit
access screen, under MCP tools.

## 0.11.13 — 2026-08-29

### Admin console: real CRUD, and the write bugs that surfaced building it

The admin console's Sessions, Hooks, and MCP screens were read-mostly.
This closes the gaps and fixes what fixing them turned up:

- **Sessions** — archive/unarchive, delete (archived-only, soft), bulk
  "delete all archived", and markdown export, all from the console. Rows
  now show whether this console instance can actually reach them: session
  mutation only ever touches this process's own workspace, even though the
  list spans every project the store indexes.
- **Hooks** — edit in place, and a real enabled/disabled toggle.
  `HookConfig` had no `enabled` field: disabling a hook deleted it from
  `config.toml` instead of recording it as off. Fixed by adding the field
  (`vak-config`) and skipping disabled hooks at build time (`vak-core`).
- **MCP servers** — `env` and `network` are now editable, not file-only.
- **A real, active bug**: hooks merge global+project by `Vec::extend` with
  no dedup, and `GET /config/hooks` reported the *merged* list — which
  `PUT /config/hooks` then always resubmitted whole. Editing any hook in a
  workspace with a global hook active wrote that inherited hook into the
  project file too; the next edit doubled it again, without bound, each
  copy re-firing a side-effecting command per matching call. Fixed by
  scoping `GET /config/hooks` and `GET /config/mcp` to the project layer
  alone (never the merged view) and adding the missing dedup in
  `vak_config::merge_into`.
- **Cross-workspace data-loss bug**: `DELETE /sessions/archived` (bulk)
  had no workspace boundary check, unlike single-session delete — running
  it from one workspace's console could soft-delete another workspace's
  archived sessions. Fixed to match the single-item scoping.
- **Diff endpoint bug, unrelated to the above**: `git diff` was invoked as
  `git --no-color diff` (flag before the subcommand — invalid syntax, git
  exits 129), silently swallowed into an empty diff. The Worktree Diff tab
  — admin console, desktop, and `openFileSmart`'s diff-routing — has never
  shown real diff content until now.
- `session_diff`, `create_session`, and `/admin/api/sessions` now return
  real HTTP status codes on failure instead of 200-with-`{"error"}`, which
  the console previously rendered as an empty state rather than a failure.
- CI now builds, typechecks, and rebuilds `crates/vak-admin-ui/dist` on
  every push, failing if it drifts from `src/` — the exact drift that
  shipped a blank admin console in v0.8.1 now fails before merge, not only
  at release time.
- **Known, documented, not yet fixed**: a warm `CorePool` entry for a
  *different* workspace than this gateway's own does not see a permission-
  mode change made to that workspace's `.vak/config.toml` until idle
  eviction (up to 30 minutes, or indefinitely on a channel that stays
  active) — pinned down by a new `core_pool` test; a first fix attempt
  caused an unrelated test to hang for reasons not yet root-caused and was
  reverted rather than shipped. Also newly documented: no "paused" channel
  state (only permanent deny or config-losing revoke), and a project skill
  silently and invisibly shadows a user-level one of the same name. See
  `docs/design/34-channel-onboarding.md` and `docs/design/09-extensibility.md`.

## 0.11.12 — 2026-08-28

### Desktop: the project gate now releases to the workspace

Opening a project appeared to do nothing. The backend booted correctly
every time — listening on its port, serving `/providers` — but the
window stayed on the welcome screen, on launch with a saved project and
after picking a folder in the dialog.

`ChatPane` used `activeId()` without importing it. `vite build` does not
typecheck, so the missing binding shipped and only failed at runtime,
when the workspace first mounts. Because Solid runs dependent renders
synchronously inside `setBackend()`, the `ReferenceError` propagated
into `refreshBackend`'s catch, which returns `false` silently — no
banner, no console output, no way to tell the boot had actually
succeeded.

`npm run build` now runs `tsc --noEmit` first, so a missing import
cannot reach a bundle again.

## 0.11.2 — 2026-08-28

### Admin Extensions console

- Expanded the admin console with dedicated MCP servers, Skills, Hooks, and Scheduled Tasks views.
- Added extension permission-scope reporting, MCP network posture, injected environment-variable names, and resolved MCP rules.
- Added resolved permission-rule visibility in Settings and improved workspace-path/table rendering.

## 0.11.0 — 2026-08-28

### Extensions: what is loaded, and what it is allowed to do

Gateway channels had a screen that said who may talk to the agent and under
what permission. The agent's other extension points had no equivalent. MCP
servers, hooks, and skills were four local tabs on one `Integrations` view,
each a list beside an add-form, and none of them said what an extension was
permitted to do once loaded — only that it existed.

`Integrations` becomes `Extensions`, four hash routes shown in a tab bar and
as sidebar sub-rows the same way Gateway's are: `#/integrations` (MCP
servers), `/skills`, `/hooks`, `/tasks`. Each is a scan-first table with the
add-form demoted to a `<details>`, so the configured state reads first.

Each extension now reports its scope from real config:

- **MCP servers** show whether outbound network is permitted, how many
  environment variables are injected (names only in the detail pane; values
  stay in `config.toml` and never cross the wire), and every `mcp(server/*)`
  rule that reaches them, colour-coded allow/ask/deny. Where no rule reaches
  a server, the mode default is named instead — `ask` under workspace-write,
  `deny` under read-only, `allow` under full-access, which is exactly what
  `PermissionEngine::evaluate` does for a tool that is neither read nor write.
- **Hooks** print their matcher verbatim — it is a permission rule, parsed by
  the same `Rule::parse` — and say plainly that a hook is the one extension
  that governs rather than being governed: it runs as the vak process,
  outside the permission engine, and `pre_tool_use` can block a call.
- **Skills** report the discovery root they came from. A workspace skill and
  a user-wide one are different trust propositions.

`GET /admin/api/config` now reports the resolved `allow`/`ask`/`deny` lists,
and `GET /skills` reports each skill's `path` and `scope`. The console
derives scope from these rather than guessing. An older server that omits
`permissions` renders "scope not reported" — an unknown scope must never be
drawn as an unrestricted one.

### Workspace paths stopped shredding themselves in table cells

The Channels table's `WORKSPACE` column carried `word-break: break-all`, so
`/Users/nisheethranjan/Projects/vakcoder` set as three lines broken
mid-word — `/Users/nis` / `heethranja` / `n/Projects/vakcoder`. With one
channel it looked untidy; the table is built for a hundred.

Paths now render through one primitive that drops whole middle segments and
never breaks one, budgeted in characters so the trailing directory — the half
that actually distinguishes two workspaces — survives to the last possible
character, with the full path on the title attribute. Applied everywhere a
path sits in a constrained cell: Channels, Core pool, skill sources, the
Overview workspace, the routing summary, and Best-of-N repos. `.wrap` keeps
its job for free text but breaks at spaces first now.

Table headers stick while a long list scrolls (`.table`'s `overflow: hidden`,
there only to round two corners, had been silently disabling `position:
sticky`), and chips no longer wrap and drag a row taller than its neighbours.
Verified at 100 rows against real payload shapes: every row one line, filter
under 4ms, no horizontal overflow.

`GET /admin/api/gateway/allowlist` and the gateway status bindings are still
unpaginated — everything is returned in one response. A hundred rows is fine;
several thousand would want a server-side page and search, and no paginated
UI has been built over an API that cannot page.

### Settings is six panels that each answer one question

Settings had six `h2`s inside two panels — `h2` is the panel-title element
everywhere else — a "Gateway & Security" box restating what the Gateway
section owns, and Sign out parked at the bottom of it. It is now Model,
Provider key, Appearance, Permissions, Maintenance, and This session, each a
real panel, with the duplicate gateway summary dropped in favour of the
section that owns it. Permissions gained the resolved rule lists and states
the precedence the engine actually applies: deny outranks ask outranks allow
among matching rules, and config order never decides it. Rules are read-only
here on purpose — a console that could widen its own reach is not a control.

### Chip tones that named colours the stylesheet never defined

`INBOX_KIND_TONE` mapped `approval_denied` and `budget_alert` to `alert`, and
`approval_pending` to `warn`. Neither class exists — the vocabulary is
`warning`/`success`/`danger` — so the two inbox entries most worth catching
the eye rendered in the same muted grey as a heartbeat. `proposal_opened`
asked for an `info` tone that was never defined; it exists now, in blue. Mono
chips also stopped being lowercased: they carry identifiers copied from
config (`TAVILY_API_KEY`, `Bash(git *)`) where case is significant, and the
console must not disagree with the file it is reporting.

### The login screen's own logo was 401ing on every fresh login

`auth_exempt_path` allowlisted `/admin/favicon.svg` and everything under
`/admin/assets/` (the hashed JS/CSS bundle) as reachable before a cookie
exists to authenticate the request that would fetch them, but not the
`vak-icon.png` the login screen's `<img>` references — a root-level dist
file outside `assets/`. A brand-new visitor with no cookie yet always saw
a broken image on the one screen that's supposed to work before login.

### The Gateway page is four screens, one per job

Gateway had accreted into a single scroll: routing summary, Core pool, bot
tokens, pending channels, registered channels, each an independent panel
stacked on the last. Nothing said which box did what, and an operator pasted
a bot token into the routing-key field — a credential and a routing key were
two boxes apart on the same page. Adding a validator to that field treated
the symptom; the layout was the cause.

The section now splits along the task boundaries an operator actually has,
as four hash routes shown both in a tab bar and as sidebar sub-rows:
`#/gateway` (Channels), `#/gateway/connect`, `#/gateway/credentials`,
`#/gateway/routing`. "Add a bot", "look at my channels", and "check if
things are healthy" are three different destinations you can name from the
nav instead of three scroll positions.

Channels is a scan-first table — channel, surface, workspace, effective
route, effective permission, status, including *capped* and *rotates next* —
and editing is a row expansion, so the settled state reads first and the
form appears on demand. Connect is a three-step sequence (set the bot token,
message the bot, review and approve) where each step reports its own state
from the backend rather than asking you to remember where you are: token
presence comes from `GET /config`'s `chat_surfaces`, the knock comes from the
pending allowlist. Credentials is bot tokens and nothing else, with no
routing field on the screen at all; the manual routing-key registration is
demoted to a disclosure on Channels that says in its own copy that a token
does not belong there. Routing & pool keeps the defaults, provenance, and
Core pool.

No functionality was dropped: approve, deny, revoke, remove, rotate, save
route, edit access, set/remove bot token all survive with the same calls.
Empty states now teach the flow instead of saying "nothing here", loading is
skeletons rather than bare text, removing a bot token asks for confirmation,
and DESIGN.md's documented `:focus-visible` accent ring, themed scrollbars,
and tabular numerals — specified for the desktop client and never carried
across — reach the console.

### One channel can be read-only while another shares its workspace

Permission mode was workspace-scoped and nothing else: set once in a
workspace's `.vak/config.toml`, inherited by every channel routed there.
An operator who wanted a personal Telegram chat kept read-only while a
team channel kept workspace-write had exactly one option — stand up a
second, otherwise identical workspace purely to vary trust level. An
allowlist entry now carries an optional `permission_mode`, the same
inherit-or-override shape its `route` field already had. Absent means
inherit the workspace's own mode, unchanged for every existing entry.

An override can only ever *reduce*. The workspace's own configured mode
is a hard ceiling, and a pin is clamped to it: pinning `full-access` on a
channel routed to a `read-only` workspace yields `read-only`. That keeps
the standing invariant that a channel never gets more than a local `vak`
run in that workspace would, and makes an operator mistake fail closed. A
clamped grant is recorded as a new `permission_capped` security event, at
both the moment it is set and the moment it is enforced, so a silently
reduced grant is visible in the audit log rather than swallowed.

The Core pool is now keyed by `(workspace, permission override)` rather
than workspace alone — two channels sharing a workspace with different
pins get separate `Core` instances, because a `Core` holds exactly one
permission mode and sharing one would let whichever channel resolved
first dictate the other's permissions. Un-overridden channels keep
sharing the instance they already shared; idle-eviction and cap semantics
are unchanged. `approve` and `PATCH .../allowlist/{key}` take
`permission_mode` in the same request body that already carries
`workspace`/`route`, and the Gateway page grows a permission-mode pin in
both the approve form and "Edit access", plus an "Effective permission"
readout beside the existing "Effective route".

### Admin console can finally set a bot token, and stops accepting one as a routing key

The Gateway page's "Registered channels" box could register a
`surface:chat` routing key, but nowhere in the admin console could an
operator actually set a bridge's bot token — that capability only
existed in Desktop Settings, or a raw API call. A bot token
("`8229314494:AAHuujv...`") happens to be syntactically indistinguishable
from a routing key by a bare colon check, so pasting one into the wrong
box silently created a nonsense binding instead of doing anything useful
— exactly what happened in the field. Fixed both ends: a new "Bot
tokens" panel on the Gateway page (`PUT/DELETE /config/bot-token/{surface}`,
already existed server-side, never had admin-console UI) lets an operator
set Telegram/Discord/Slack credentials directly; the routing-key box now
validates the surface prefix against the actual known surfaces and
rejects anything else with a message pointing at the right field, instead
of accepting any string containing a colon.

### The menu bar really does survive a logout now

The `com.vak.desktop` LaunchAgent added last release did bring the
desktop app back at login — but the app it brought back could not draw
anything. Its plist was missing `LimitLoadToSessionType`, so launchd ran
the job in the plain background `gui/<uid>` domain rather than the Aqua
login session. The process started and stayed up, but it never checked
in with LaunchServices and was given no WindowServer connection, which
makes a status item impossible to place. `lsappinfo` showed the damage
plainly: `bundle path=[NULL]`, `executable path=[NULL]`, `Arch=!!none`,
`!cgsConnection`. With the key set, the same launch reports
`type="Foreground"`, a real session token, and the resolved bundle — and
the icon appears. GUI units now render that key; the headless gateway
and telegram units deliberately do not, since pinning them to Aqua would
stop them loading in a session with no logged-in GUI user.

A second, independent fault made this almost impossible to diagnose in
the field. `tauri-plugin-single-instance` calls `std::process::exit(0)`
from inside its own plugin setup when another instance already owns
`/tmp/dev_vak_desktop_si.sock`. That happens before anything this app
writes a line, so a login where macOS had already reopened Vak left
`~/Library/Logs/vak/desktop.log` completely empty and the job reporting
`last exit code = 0` — indistinguishable from a unit that never ran.
`vak-desktop` now writes one startup line naming its pid and argv before
the builder runs, so an early hand-off is always attributable.

That hand-off path also ignored `--tray`. A login launch from the
LaunchAgent passes its argv to the already-running instance, which
unconditionally revealed the window — throwing a window on screen at
login, exactly what `--tray` exists to prevent. The single-instance
callback now honours the flag; ordinary second launches (Dock, Finder,
`open`) carry no flag and still reveal the window as before.

### The live-event stream now recovers from a stale session too

The previous fix for a session going stale after a server restart only
covered the polled unread-count check; the SSE event stream
(`/admin/api/events`) kept silently reconnecting forever on the same
dead cookie, since a browser `EventSource` error carries no HTTP status
to detect the failure by. `connectEvents` now side-channels a real
`fetch` (which does carry a status) on every reconnect attempt and drops
back to the login screen on a real 401/403, instead of retrying a
connection that can never succeed again without a fresh login.

### Broken desktop welcome-screen icon, and the admin console silently going dead after a token change

The desktop app's welcome screen (`ProjectGate`) has referenced
`/vak-icon.png` since the rebrand, but the actual asset was never renamed
from `vakcoder-icon.png` — a broken-image placeholder on every first
launch. Renamed the file to match.

The admin console's "unread count" poll swallowed a 401 silently instead
of falling back to the login screen like every other API call site in the
app does. In practice: any time the server's bearer token changes under
an already-open tab (a restart with a fresh per-process token, or a full
`self uninstall --purge` + reinstall), the page kept its stale-looking
"live" UI forever, quietly failing every request in the background with
nothing telling the operator to log back in.

### The menu bar now survives a logout

`vak-desktop` owns the tray — the surface that starts, stops and watches
every other service — but it was the only component with no unit of its
own, so it was also the only one that did not come back after a logout or
a reboot. Nothing brought the menu-bar icon back except opening the app by
hand. It now has `com.vak.desktop` (macOS) / `vak-desktop.service`
(Linux), generated by `self services-sync` like the others and started on
a fresh install.

It is not simply a copy of the gateway's unit:

- **KeepAlive/Restart is off.** The tray's `Quit Vak` calls `app.exit(0)`;
  with KeepAlive on, launchd would relaunch it a second later and Quit
  would visibly not quit. `RunAtLoad` still gives the come-back-at-login
  behaviour that is the point of the unit.
- **It starts as `vak-desktop --tray`** — menu-bar icon, no window. A new
  `--tray` flag leaves the main window hidden at startup; every other way
  in (double-click, Dock, `Open Vak`, a second launch handed over by the
  single-instance plugin) reveals it. A login launch that threw a
  1440×900 window on screen every boot would have been a worse regression
  than the missing persistence. The window is now created hidden and
  revealed explicitly, so an ordinary launch does not flash one either.
- **Its working directory is the account home**, not the workspace
  `services-sync` ran from: the desktop app picks its project in its own
  UI, so recording a workspace would record one it never honours.
- **It is skipped when the build shipped no `vak-desktop`**, so a headless
  install does not acquire a GUI unit that could only ever fail. `self
  status` no longer lists it as unregistered in that case either.

This deliberately re-creates the shape that broke `com.vak.tray` — one
binary that is both the bundle's `CFBundleExecutable` and a launchd
service — without the cause. That bug was fatal because macOS activated
the already-running process instead of launching one, and that process had
no window and no reopen handling, so nothing happened at all. Both halves
are now fixed: `RunEvent::Reopen` reveals the window when macOS activates
the running instance, and `tauri_plugin_single_instance` hands a genuinely
new launch over to the live one when it does not. Every launch path ends
with a window on screen, and no path adds a second menu-bar icon.

### Fresh-install fixes: reachable admin console, real icon, honest provider errors

`self install` now leaves the gateway server running after bootstrapping
the default workspace (`~/vak-home`) — a fresh install previously
generated and then immediately stopped the service, so opening the admin
console needed an extra manual step just to configure anything.
Unattended remote chat surfaces stay fail-closed regardless (empty
`gateway.chat_allowlist`); this only affects whether the loopback,
bearer-token-gated HTTP server itself is reachable. Telegram still stays
stopped until a bot token is actually configured — starting it with none
would just crash-loop.

Fixed the installed `.app` bundle never carrying an icon: `Info.plist` had
no `CFBundleIconFile` key and nothing copied `icon.icns` into
`Contents/Resources`, so Finder always showed the generic placeholder.

`vak doctor`'s "provider" check no longer implies Anthropic is the only
option when its credential is missing — it now names whichever other
providers are actually already usable (including Ollama, which needs no
credential) so the fix on offer is "point config at what you have"
as often as "set a key."

### Channel onboarding: editing, doctor/repair, Discord + Slack bridges

**Editing an already-allowed channel.** `PATCH
/admin/api/gateway/allowlist/{key}` re-points an `allowed` entry's
`workspace`/`route` in place, preserving `added_at`/`added_by` — an
operator can move a channel to another project or change its pinned model
without revoke-and-re-approve, which lost provenance and 403'd the channel
in between. Only `allowed` entries are editable (404 otherwise);
pending/denied still move through approve/deny.

This closes a real drift risk rather than adding a second config: the
allowlist entry's `route` was written by Phase 1's approve flow but never
read at dispatch, which only consulted the binding's own provider/model
override. There is now one `effective_route_override` both surfaces
resolve through — the entry's pinned route when it has one, the binding
override otherwise — and `PATCH .../bindings/{key}` writes through to the
entry when one exists, so the two admin surfaces can no longer disagree
about what a channel routes to. The edit goes through the same
stale-detection seam the binding editor already uses: the cached route
revision is dropped (never the ledger), so the next inbound message
rotates to a fresh frozen session only if the effective route really
changed.

**`vak doctor` gains a `gateway channels` check.** It fails when an
`allowed` entry's workspace no longer exists or isn't readable, or when a
`pending` entry has sat past the expiry window; the pass detail is counts,
the failure detail names the offending keys. It reads the allowlist store
directly — no `Core` is started per workspace just to check one.

**`[gateway] pending_expiry_days`** (default 7) sets that window.
`vak doctor --repair` auto-denies expired pending entries, stamping
`added_by: "expiry"` so they stay visibly distinct from an operator's own
deny rather than being silently deleted. A gateway applies the same expiry
on startup, so the online and offline paths converge. An `allowed` entry
with an unreachable workspace is deliberately *not* auto-repaired —
re-pointing it is a judgment call — and `--repair` says so instead of
guessing.

**Workspace picker.** `GET /admin/api/gateway/status` now reports
`known_workspaces` (workspaces vak has session ledgers for, plus the
gateway's own cwd and any pooled workspace), and the Admin UI's approve
and edit forms offer them as a dropdown with a "custom path" fallback that
visibly notes the path is unverified until a Core actually starts there.

**Discord and Slack bridges** (`vak discord` / `vak slack`, same flag
shape as `vak telegram`) implement `InboundChannel` with `chat` = channel
id and `sender` = user id, so Phase 1's pending/approve/deny/revoke
lifecycle applies to them unchanged. Delivery adapters are registered
alongside `TelegramAdapter` when `DISCORD_BOT_TOKEN` / `SLACK_BOT_TOKEN`
are set. The Admin UI channel panels gain a per-surface badge.

Desktop Settings now has a bot-token field per chat surface, all stored
the same way as the existing Telegram one (user `.env`, owner-only), via
new `PUT/DELETE /config/bot-token/{surface}` routes.

Two deliberate limits, both noted in `docs/design/34`: the new bridges
**poll** (`DISCORD_CHANNEL_IDS` / `SLACK_CHANNEL_IDS`) rather than using
Discord's gateway websocket or Slack Socket Mode, which would add a
websocket dependency the workspace does not have; and forwarded approvals
on these surfaces are **typed yes/no prompts**, not interactive
buttons/Block Kit — the same fallback Telegram used before its inline
keyboard, resolving through the one existing `parse_verdict` path.

### Channel onboarding: live allowlist store + Admin UI approvals

`gateway.chat_allowlist` is no longer a config-file-only, restart-required
setting. Unknown inbound chats now land as a reviewable **pending** entry
in a new schema-versioned, live-reloadable store
(`<sessions_home>/gateway/allowlist.json`, sibling to `bindings.json`)
instead of a flat rejection — the operator has a forward path from "I see
it was rejected" to "let it through" that doesn't require hand-editing
`.vak/config.toml` and bouncing the gateway process. A repeat message from
an already-pending chat is logged lightly instead of spamming a fresh
security event each time. `chat_allowlist_open = true` still bypasses the
store entirely, as before. Existing `gateway.chat_allowlist` entries are
imported into the new store once, on first load, as `allowed`; after that
the store is authoritative, matching `bindings.json`'s relationship to
route overrides.

New admin API routes: `GET /admin/api/gateway/allowlist`, `POST
/admin/api/gateway/allowlist/{key}/approve` (body: `{workspace?, route?}`
— approving always makes the effective workspace explicit in the
response, even when the caller didn't supply one, so it's never a silent
inherited default), `POST /admin/api/gateway/allowlist/{key}/deny`, and
`DELETE /admin/api/gateway/allowlist/{key}` (revoke an allowed entry).
Every transition is recorded via `vak_core::security_events` under new
`chat_pending` / `chat_approved` / `chat_denied` / `chat_revoked` kinds.

The Admin UI's Gateway page gains a "Pending channels" panel (chat key,
first-seen text, arrival time, Approve/Deny — approving lets the operator
confirm or edit the workspace and optionally pin a provider/model) above
"Registered channels," and registered channels now show an allowlist
status chip (allowed/pending/denied) with a Revoke action for allowed
entries.

**Phase 2: multi-tenant Core pool.** An allowlist entry's `workspace`
field used to only pick provider/model — the gateway ran every channel
through its own single `Core`, fixed at process start, regardless of what
workspace an entry named. `GatewayState` now holds a `CorePool`
(`crates/vak-server/src/core_pool.rs`): a canonical-workspace-path →
lazily-started `Core` map. The gateway's own default workspace is the
pool's permanent, never-evicted entry; on inbound dispatch, once the
allowlist resolves an entry as `allowed`, the gateway looks up (or lazily
starts) that entry's own workspace `Core` via `Core::new_with_trust` —
the exact same trust/permission/sandbox resolution a local `vak` run in
that workspace gets — and routes the turn's session, ledger, and turn
execution through it instead of the gateway's own `Core`. Pooling never
grants a channel more access than a local session in that workspace
already has; the allowlist approval step is still what gates a channel
reaching a workspace at all.

Pooled Cores for non-default workspaces are idle-evicted after
`[gateway] core_pool_idle_secs` (default 1800 = 30 minutes) of no inbound
activity, and capped at `[gateway] core_pool_max` (default 8) concurrently
pooled Cores — over the cap, the oldest-idle non-default entry is evicted
to make room; the default workspace is never evicted. Both are new
recognized `[gateway]` config keys (unknown-keys-warn-not-fail, like every
other gateway key). `GET /admin/api/gateway/status` gains a `core_pool`
field (max, idle_secs, and the list of currently warm workspaces with
idle time) and the Admin UI's Gateway page shows a "Core pool" panel plus
a warm/cold indicator next to the workspace field in the pending-channel
approve flow, so the operator can see whether approving a workspace will
reuse a live Core or start a fresh one.

### `vak doctor --repair` and `scripts/vak.sh`

`vak doctor` gained `--repair`: it acts on the checks that have a known
mechanical fix (today: self version parity, via `self install --force`)
and re-collects the report, leaving checks with no mechanical fix
(provider auth, config warnings) for the operator — it never guesses at
those. `scripts/vak.sh <verb>` is a new thin dispatcher over
`scripts/build.sh` / `scripts/release.sh` / `vak self <verb>` / `vak
doctor`, for anyone who'd rather remember one entry point than which
script or subcommand owns a given lifecycle step; it only routes, it
never reimplements a verb.

## 0.10.0 — 2026-08-28

### Rebrand: VakCoder → Vak

The project and CLI are renamed from "VakCoder"/`vakcoder` to "Vak"/`vak`
across source, docs, config, and CI. The umbrella CLI crate moves from
`crates/vakcoder` to `crates/vak` (package and binary both `vak`); the
tray helper binary is now `vak-tray` (was `vakcoder-tray`). The Tauri
desktop app identifier moves to `dev.vak.desktop`. The on-disk config/data
layout already used `~/Library/Application Support/vak` (etc.) as of the
0.8 release with its own `~/.vak` legacy-dotdir migration
(`crates/vak-config/src/paths.rs`), so no further path migration was
needed here. Older `vakcoder`-named artifacts (release archives, prior
changelog entries) are left as historical record.

## 0.9.0 — 2026-08-28

### Telegram document attachments and inline-keyboard approvals

Telegram messages can now include a `document` (code, logs, CSVs, ...) up
to 256 KiB; it's inlined into the prompt as a fenced text block (capped
at 64 KiB decoded) rather than silently dropped or truncated. Forwarded
approval gates (`[gateway] approver = "telegram:<chat>"`) now render as
tappable inline-keyboard buttons instead of requiring a typed `yes`/`no`
— a new `TelegramAdapter` gives the gateway's async delivery path an
actual route to Telegram, which it never had before (a forwarded gate to
a Telegram approver previously failed the push and denied closed with no
adapter registered for that scheme). Button taps resolve through the
same verdict-parsing path a typed reply already used.

### Gateway chat allowlist fails closed

`gateway.chat_allowlist` now fails closed: an empty list rejects every
inbound chat with `403` instead of allowing all of them, unless the
operator explicitly sets `chat_allowlist_open = true`. Inbound channel
bridges (Telegram today; Slack/Discord later) now build their
`/gateway/inbound` payload through a new `InboundChannel` trait and
`InboundRequest::new` constructor, which rejects an empty or
placeholder-equal `chat`/`sender` so a careless new bridge cannot
silently collapse every remote user into one session. The Telegram
bridge now sends the message sender's real Telegram user id instead of
a fixed `"telegram"` placeholder.

## 0.8.12 — 2026-08-28

### Canonical service home

Generated launchd/systemd units now preserve the invoking user's non-secret
`HOME` alongside the workspace. This prevents a sanitized service-manager
environment from treating the workspace as the user home and relocating
project `.vak/config.toml` into a nested platform-data directory. The
canonical path resolver also falls back to the operating-system account home
when GUI launch environments omit `HOME`; it never falls back to the current
workspace.

## 0.8.11 — 2026-08-28

### System-wide route control plane

Provider and model are now one atomic route across Core, configuration writes,
session admission, task and heartbeat pins, desktop, gateway, and admin
surfaces. Authenticated workspace changes persist the complete pair before
hot-apply; independent long-running local processes refresh persisted defaults
when admitting a new session. Explicit scoped pins remain isolated.

The same refresh path now covers max turns, theme, MCP servers, hooks, and
permission mode. A permission change is applied only after active main/side
runs are cancelled and pending approvals denied, preserving capability
revocation across process boundaries.

Gateway bindings now use a backward-compatible, versioned record containing
the bound session, optional channel route, workspace, and route revision.
Admin-default precedence is deterministic (`channel override > workspace
default`). A frozen session that no longer matches its effective route is
reported stale and rotates to a new session on the next inbound message; the
old append-only ledger is preserved.

The admin console adds a dedicated Gateway control view with workspace default
and provenance, registered-channel creation/edit/removal, live-discovered model
selection, inherited versus overridden routes, frozen-contract comparison,
stale reasons, and explicit conversation rotation.

Non-interactive server logs no longer print generated bearer tokens, and
Telegram transport/decode failures are rendered without request URLs so bot
tokens embedded in Bot API paths cannot enter service logs.

## 0.8.10 — 2026-08-27

### Workspace-aware durable services

Generated launchd/systemd units now retain the workspace directory captured by
`self services-sync`. Gateway and Telegram deployments therefore load the
selected workspace's provider/model config and project `.env` instead of
starting from the service manager's root directory and incorrectly returning
503 for a missing provider credential.

## 0.8.9 — 2026-08-27

### Configuration audit follow-up

Profile-derived provider/model settings now report accurate provenance, and
the admin transcript projection exposes the same frozen session contract and
configuration-mismatch status as the normal session endpoint.

## 0.8.8 — 2026-08-27

### System-wide configuration authority

Provider/model settings saved through the secured configuration API now
persist atomically in the workspace configuration, report their effective
source across health, provider, and admin endpoints, and survive process
restart. Existing sessions continue to use their immutable frozen provider
and model contract, with transcript metadata exposing mismatches against
current workspace defaults. Desktop settings now describe the saved-workspace
behavior instead of implying that changes are runtime-only.

The follow-up audit also aligned the admin transcript projection with the
normal transcript contract and added profile-aware provenance reporting.

## 0.8.7 — 2026-08-27

### Desktop tray lifecycle

The macOS desktop app now owns the menu-bar tray and its full service
controls in one process. Finder/Dock activation, tray Open, and repeated
launches reveal the same window; Gateway and Telegram controls, logs, Admin
Console, watchdog, and Quit remain available from the tray.

## Unreleased

### The desktop app could never receive a single agent event

Every desktop SSE connection had always been rejected with 401, so the
chat window never received one event: runs completed and were durably
logged while the UI showed no reply, "Working" that never cleared, and
`0 in / 0 out`. Nothing appeared in the console either, because a 401 on
an `EventSource` surfaces only as a bare `onerror`.

`require_bearer` accepted an `Authorization` header or the `vak_session`
cookie. `EventSource` cannot set headers, and the desktop never performs
the `/admin/login` cookie exchange -- that is the browser console's flow.
`?token=` was the only channel it had, `openEventStream` and
`openSideStream` have always used it, and the startup banner has always
advertised it -- but the middleware never accepted it. The middleware now
matches the contract it advertises.

Found by opening the real SSE endpoint with curl and getting zero bytes
back. Three earlier releases shipped fixes for this symptom -- a
swallowed exception in the message handler, a dead stream with no
reconnect, and missing server-state reconciliation. All three were real
defects and are worth keeping, but none of them was the cause, because
none of them was ever tested against the actual event stream.

### Two menu-bar icons, and an app that opened nothing

The bundle's `CFBundleExecutable` is `vak-tray`, and
`com.vak.tray` also runs it as a launchd service with `RunAtLoad`.
Nothing guarded against both. The ordinary path -- install,
`services-sync`, then open Vak from Finder or Spotlight -- produced
two identical menu-bar icons; and once macOS began merely re-activating
the already-running app rather than spawning a new process, launching it
did nothing visible at all, because the tray only opens the chat window
once at startup.

The tray now takes a single-instance lock. A launch that finds a live
holder opens the chat window -- what launching the app actually asks for
-- and exits. Same mechanism `vak_server::telegram::InstanceLock`
already used: an O_EXCL marker plus a liveness probe on the recorded pid,
so a crashed holder leaves a marker the next launch reclaims rather than
one that wedges the menu bar until reboot. Not flock, which would need
`unsafe`; the workspace denies it. Verified all three behaviours against
real processes: a second launch adds no icon and opens the window, and a
`kill -9`'d holder's lock is reclaimed.

### Install bloat

`self install`'s bundle-asset copy (`copy_dir`) only ever adds files; it
never removes ones absent from the source. The desktop frontend's
filenames are content-hashed (Vite) and change on every rebuild, so every
reinstall left the *previous* build's JS and CSS sitting in
`Contents/Resources/assets/` alongside the new one — harmless to which
file actually gets served (`index.html` always names the current hash),
but unbounded bloat, and confusing to anyone inspecting the bundle with
no way to tell which files are actually live. `write_metadata` now clears
the hashed `assets/` subtree before copying — only that subtree, not all
of Resources, which also holds `install.json` and `Info.plist`.

### Admin console serving a blank shell

The admin console loaded to a blank dark screen with nothing in the
console — no JavaScript ever ran, so nothing had a chance to error.
Found by opening the one-click link this release added and seeing
exactly that; confirmed fixed by loading the real page and reading its
network requests and rendered content, not by inspecting code.

Three layered defects, found one at a time as each fix exposed the next:

- **The actual bug**: `crates/vak-server/src/admin_ui.rs` registered a
  route for every embedded file via `Dir::files()` — which is not
  recursive. Everything under `dist/assets/` (the JS bundle, the CSS)
  therefore had no route at all; a request for either 404'd from axum's
  router itself, before ever reaching the file lookup (which — via
  `get_entry` — was recursive and would have found them fine). Only
  `index.html`, at the top level, ever actually loaded, which is exactly
  why the failure read as "the page loads, but nothing on it does."
  Fixed by walking the embedded tree recursively when registering
  routes. A `#[tokio::test]` now drives the real `Router` `routes()`
  builds through every asset URL `index.html` references, exactly as a
  browser would — confirmed it fails with the exact defect's diagnostic
  when the non-recursive version is restored, and passes clean
  otherwise. An earlier, narrower test that only checked the assets were
  *embedded* passed throughout; embedding was never the problem.
- `vak-server` embedded a stale, mismatched build of the SPA on top of
  that: nothing told Cargo that `crates/vak-admin-ui/dist` was a build
  input, so a `cargo build` after `npm run build` regenerated it could
  reuse an incremental build of `vak-server` from before the
  regeneration, embedding an `index.html` that referenced filenames the
  embedded directory no longer had at all. Added `crates/vak-server/build.rs`
  declaring the directory a build input.
- The `vak-admin-ui` source fix for one-click login shipped without ever
  being compiled: `dist/` is committed (`vak-server` embeds it, not
  reads it live) and nothing forced a rebuild before release. `scripts/release.sh`
  now rebuilds both `vak-admin-ui` and `vak-desktop/ui` from source as
  part of the gate, before the working-tree-clean check, and fails the
  release outright — naming the exact fix — if the rebuilt
  `vak-admin-ui/dist` differs from what is committed.

Rebuilt the install, update, and release lifecycle around one owner, one
version, and verifiable artifacts. Found by exercising every command
against a real prefix rather than reading the code.

### Desktop chat replies that never appear

A desktop task could complete a full turn — correct provider, correct
model, correct reply, durably logged to the session's JSONL — and the
window would still show "Working" forever, with no reply, no error, and
nothing in the console. Confirmed the backend was never at fault by
reading the session log directly: the assistant's message was there,
timestamped, `settlement: "ok"`.

- Fixed `openEventStream`'s message handler swallowing exceptions raised
  by its own caller. `es.onmessage` wrapped both `JSON.parse(m.data)`
  *and* the call to the event handler in one `try/catch` commented as
  "ignore keep-alive/comment frames" — but that catch also silently
  discarded any exception thrown while handling a successfully parsed
  event, including `RunFinished`, the one that clears "Working" and
  reveals the reply. The connection stayed healthy throughout, so
  nothing ever looked wrong from the outside. The parse and the handler
  now have separate try/catches; a handler exception is reported via
  `console.error` with the event attached, not discarded.
- Fixed a dead event stream having no way back. `openStream`'s error
  callback was a no-op, and its own guard (`if (streams.has(id)) return`)
  meant a session whose `EventSource` had genuinely closed could never
  be reopened — every future run on that session would complete on the
  backend and never reach the UI, permanently, until the app was
  relaunched. It now clears the stale entry and retries after a short
  delay, but only once the browser's own reconnect has actually given up
  (`readyState === CLOSED`), so a transient error the browser is already
  retrying isn't torn down and duplicated.
- Neither of the above was actually the fault: verified live, in a build
  containing both fixes, against a brand-new task — same symptom, same
  correct reply already sitting in the session log. The real cause is a
  race the two fixes above don't touch: `openStream` constructs the
  `EventSource` but never awaits its connection actually opening before
  the prompt is sent, and the server's broadcast channel does not
  replay history to a subscriber that attaches after an event has
  already fired. A run that finishes before that connection is fully
  live loses `RunFinished` permanently — nothing dropped, nothing
  errored, the event simply never had a listener at the moment it was
  sent. `refreshSessions()` already polls the session list every 10s
  but only refreshed the sidebar; it never reconciled the "running" flag
  gating the header pill and `hydrate()`'s own guard (`if
  (!isRunning(id))`), so even once the server knew the run was done, the
  client had no path back to that fact without a relaunch. It now
  cross-checks every session's server-reported `running` state on each
  poll and, on a mismatch, corrects it and loads the transcript the push
  path missed — self-healing within 10 seconds instead of requiring a
  relaunch.

### Desktop credential visibility and silent run failures

Found by installing fresh, launching the desktop app, sending a message,
and getting nothing back — then reproducing the exact request against the
live gateway instead of guessing.

- Fixed the desktop app being unable to see a provider credential saved
  anywhere else. `vak-desktop` hardcoded its own data home as
  `~/.vak`; the wizard and the TUI's `/key` command save through
  `Core::set_provider_key`, which writes to the canonical home
  (`vak_config::paths::data_home()`, doc 32). Those are different
  directories, so a key saved through either path was invisible to a
  desktop launch — every run failed `Core::provider()` regardless of
  whether the user had ever configured a key. This is also why "home
  migration skipped: both ... exist" kept appearing: the desktop was
  actively writing into the legacy dir, so migration could never
  complete. `vak_home()` now resolves the same canonical home as every
  other surface.
- Fixed `/sessions/{id}/run`, `/side`, and `/bestofn` returning a bare
  503 with an empty body and nothing logged when no provider credential
  is configured. A client saw an empty response and an operator reading
  gateway.log saw nothing at all — "the agent never replied" was a
  symptom with no server-side trail. All three now return
  `{"error": "provider auth missing: set ANTHROPIC_API_KEY for provider
  'anthropic'"}` and log `[run] refused: ...` server-side. The message
  reuses `CoreError::MissingAuth`'s existing text and is deliberately a
  single `error` field, matching every other handler in this file — a
  `{"error": <code>, "detail": <message>}` shape was tried first and
  reverted because the desktop frontend's existing error handling reads
  `.error` as the human-readable string, and would have shown the user a
  machine code instead of the fix.

### Concurrency

- Fixed a self-deadlock in `Core::cache_home` that hung `cargo test
  --workspace` indefinitely. It locked `sessions_home_override` and then,
  still holding the guard, called `sessions_home()` — which locks the same
  mutex. `std::sync::Mutex` is not reentrant, so the thread wedged. The
  branch is only reached when the override is set, which production never
  does and every test fixture does; all seven tests in
  `crates/vak-server/tests/gateway.rs` blocked on it and now run in 0.35s.
- Removed the idiom that made this possible. `if let Ok(g) =
  slot.lock() && …` keeps the guard alive for the whole body, so the
  hazard is invisible at the call site. All fifteen override accessors now
  go through `Core::read_override` / `write_override`, which clone out
  under a minimal scope, so no guard is ever held across another call.
- Added regression coverage that runs each accessor on a worker thread
  with a deadline. A reintroduced deadlock fails the suite in 10s with a
  message naming the cause, rather than hanging it — a test that hangs
  reports nothing and blocks every gate behind it.
- Fixed `doctor_reports_checks_facts_and_optional_ladder`, which had
  expected four health checks since before the "install layout" check
  joined the ladder in f6131a5. Cargo runs test binaries sequentially, so
  the hang in `gateway.rs` meant this binary never ran and the stale
  assertion stayed invisible for the whole 0.8.0 cycle.

### Install and uninstall

- Fixed `self status` telling a freshly installed machine its service
  units "exec outside the managed prefix" when no unit files existed at
  all. `ServiceRow::unit_points_at_installed` is false both for a missing
  unit and for one pointing at the wrong binary, and status collapsed the
  two into the misconfiguration message — which every first install hits,
  and which sends the operator after a problem they do not have. A
  `unit_present` flag now separates them, so an unsynced install reads
  "not registered — run `self services-sync`".

- Removed the second installer. `build-install.sh` copied a Tauri bundle
  to `~/Applications/Vak.app` while `self install` managed
  `vak.app` — the same directory on a case-insensitive volume, which
  every macOS default is. The script's `rm -rf` destroyed the manifest of
  a managed install, after which `status` reported nothing installed and
  `uninstall` could not clean up. Placement is now solely `self install`;
  `scripts/build.sh` builds and hands off.
- `--prefix` is accepted by every `self` subcommand, not just `install`.
  Installing to a custom prefix previously left an install that could not
  be inspected, updated, or removed.
- Install, reinstall, and update are transactions. Every file is staged
  and verified before any is placed, and a failure restores the prior
  state — no half-installed prefix, and no window where the CLI is new
  while the tray is still old.
- Added `self verify` and `self reinstall`. The manifest (schema 2) now
  records a SHA-256 per component, so tampering and truncation are
  detected instead of assumed absent. Schema 1 manifests migrate on read.
- Install verifies its own result before reporting success, and reports
  how to put the CLI on PATH when it is not.
- Uninstall names components living outside the prefix that removing the
  prefix will not reach, and no longer treats an absent install as an
  error.

### Update

- Fixed `self update` panicking before it did anything. It builds a
  `reqwest::blocking` client inside the CLI's tokio runtime, which aborts
  with "Cannot drop a runtime in a context where blocking is not
  allowed". The transfer now runs on its own thread, the confinement the
  passive update check already used.
- Fixed version comparison being lexical. `manifest.version <= CARGO_PKG_VERSION`
  compares strings, and `"0.10.0" <= "0.8.0"` is true — the first release
  past `0.9` would have reported "up to date" permanently. Ordering now
  goes through `semver::Version`, with build metadata stripped, because
  the crate's own `Ord` ranks `0.8.0+build.7` above `0.8.0` while semver
  §10 requires build metadata be ignored for precedence.
- Downloaded artifacts are verified against a SHA-256 from the feed
  before anything is written. A feed entry without a digest is refused
  rather than trusted.
- Update replaces every component in the release, not only `vak`.
  Previously the manifest version was rewritten while the tray, desktop,
  and worker stayed on the old build, so `status` reported a clean
  install that was actually mixed-version.
- Update compares against the installed version rather than the running
  build, and gained `--dry-run`.

### Versioning, build, and release

- Restored version singularity per docs/design/32. `tauri.conf.json` and
  both frontend `package.json` files carried their own `0.7.0` stamp
  while the workspace was at `0.8.0`, so the shipped app reported the
  wrong version. Tauri now derives the version from its crate, the
  private frontends are pinned to `0.0.0`, and `scripts/check-version.sh`
  fails if a second stamp reappears.
- Added `scripts/release.sh`, which produces the `release.json` feed that
  `self update` consumes. That feed had a consumer and no producer, so
  the update path could never work end to end. It gates on version
  singularity, fmt, clippy, tests, a clean tree, and an unused tag before
  building, then emits binaries, `SHA256SUMS`, and the feed.
- Added `scripts/bump-version.sh` (one edit plus a lockfile refresh) and
  `scripts/build.sh` (build, install, verify), replacing the root
  `build-install.sh`.

## 0.8.0 — canonical layout release

Platform-standard filesystem locations. One-time automatic migration from
`~/.vak` to `~/Library/Application Support/vak` (macOS) or
`~/.local/share/vak` (Linux). Logs to `~/Library/Logs/vak`,
cache (store.db) to `~/Library/Caches/vak`. Desktop app now ships
in the install bundle with frontend assets.

- **`vak-config::paths` module**: single source of truth for data home,
  cache home, and log directory. `VAK_HOME` override nests everything
  under one directory for tests and portable installs.
- **`user_env_path()` follows data home**: secrets live in
  `data_home()/.env`, not a hardcoded `~/.vak/.env`.
- **Desktop in bundle**: `self install` copies `vak-desktop` binary and
  frontend assets into the app bundle. `Resources/` now contains the
  SolidJS SPA.
- **Stale reference purge**: 36 files changed — all user-facing
  `~/.vak` strings replaced in source code, 19 doc references fixed
  across 16 design docs, AGENTS.md, SECURITY.md, README.md, hosting.md.
- **Legacy migration**: one-time rename of `~/.vak` → canonical data
  home. store.db* → cache. logs → Library/Logs. Skipped when
  `VAK_HOME` is set. Called at all binary entry points.

## 0.7.0 — admin console release

The web admin console: one binary, one URL, full control of a running
vak from any browser (docs/design/33-admin-console.md).

- **`vak-store` crate**: SQLite FTS5 rebuildable index over session JSONL.
  BM25 full-text search with snippets across ALL content blocks (tool
  commands, results, thinking — not just prose), structured metadata
  queries (role/kind/provider/model/date/project), idempotent import,
  WAL mode. JSONL stays the source of truth; the index is disposable and
  rebuilt automatically (startup, post-run, on-demand `refresh=true`).
- **Global event hub + SSE**: typed SystemEvent broadcast (agent runs,
  session lifecycle, config changes, gateway inbound,
  approval requested/granted/denied, security alerts) streamed at
  `/admin/api/events`; browsers reconnect with backoff and lag explicitly.
- **Cookie auth alongside bearer**: `POST /admin/login` validates with
  constant-time comparison and sets an HttpOnly SameSite=Strict cookie —
  EventSource cannot send headers, so this unlocks browser SSE. Login is
  rate-limited; failures land in the security-events log and alert live.
- **Embedded console SPA** (`crates/vak-admin-ui`, SolidJS+Vite, dist
  committed so cargo needs no node): overview with live activity feed and
  stat cards, session catalog → transcripts with role rails/tool badges/
  error highlighting and pagination, global FTS5 search with highlighted
  snippets, color-coded security audit log, inbox with unread badge.
- **Operational console**: answer approval gates from anywhere (args
  preview, live refetch via SSE), edit provider/model, switch permission
  mode via consequence-labeled cards, rebuild the index, cancel runs,
  live-tail transcripts during active runs (session-scoped SSE → debounced
  refresh), sign out.
- **Canonical-client interaction**: composer sends prompts (Enter),
  mid-run sends become steering automatically, ×1–×4 selector fans prompts
  out as best-of-N candidates in isolated worktrees; "+ New session"
  creates sessions from the browser.
- **Security floor** (Phase 0 hardening): per-IP sliding-window rate
  limiting (`[gateway.rate_limit]`), path confinement for tool resolution,
  hook/PTY env allowlisting, SHA-256 trust markers, temp-file atomicity,
  Telegram chat allowlist + sender attribution, append-only security-
  events JSONL surfaced live, config audit trail across all mutation
  endpoints, SECURITY.md disclosure policy.

## Unreleased

- **Channel-aware message formatting**: the agent writes GFM markdown once;
  delivery converts per surface. Telegram replies now render as native HTML
  (bold headings/links/inline-code, fences and tables as monospace,
  blockquotes, • bullets) with tag-safe chunking at 4096 chars and an
  automatic plain-text fallback. Future channels add flavors (Slack mrkdwn,
  Discord cards) without touching agent code.
- **Tavily web search via MCP**: stdio servers get `${VAR}` interpolation in
  env values (resolved through the standard secret path — keys stay out of
  config files), a per-server egress flag (`[mcp.servers.X] network =
  true`, privileged) for remote-API tools, and operational env (PATH with
  the server's own dir, HOME/TMPDIR) so npx-based servers run correctly
  under launchd/systemd.

- **TUI `/services`**: status lines plus start/stop/restart for the gateway
  and Telegram bridge via vak-ops.
- **Desktop ▸ Settings ▸ Services**: live status dots with
  Stop/Start/Restart per service and Install/Uninstall for the pair; polls
  every 5 s while open.
- **Desktop ▸ Settings ▸ Learning** (L2): skill-proposal queue with
  Promote/Reject and a recent-notes viewer with provenance.
- **L1 reflection loop**: opt-in `[memory] reflection = true` runs an
  auxiliary call after clean completions proposing ≤2 notes (+ optional
  skill draft), deduped by Jaccard against existing notes; drafts always
  land in the human review queue.
- Gateway-path test fixtures are hermetic against the developer's global
  config so personal defaults (reflection etc.) never leak into CI.

## 0.3.0 — 2026-08-23 · platform release

The always-on platform phase (docs/design/22-gateway.md through 27):
one headless core, many surfaces, durable services.

- **Gateway**: `POST /gateway/inbound` (+wait long-poll) routes chat
  surfaces to persistent sessions; bindings survive restarts; busy turns
  queue as logged steering; unattended approvals fail closed by default or
  forward to an approver surface (`approvals = "forward"`, timeout-deny).
- **Transports**: Telegram bridge (`vak telegram`) and outbound
  webhook targets with fail-closed bearer auth; cron routines push real
  final answers to any surface (`TaskDef.deliver_to`).
- **Media passthrough**: images from chat reach vision models as native
  content blocks (Anthropic/OpenAI/Google/Responses wire shapes); Telegram
  photos auto-download.
- **Memory & recall**: `session_search` tool + `/search` endpoint rank
  curated MEMORY.md notes above transcripts; `remember` persists decisions
  with provenance.
- **Learning loop**: `propose_skill` queues drafts for human promotion via
  HTTP/CLI into user-level skill discovery — never automatic.
- **Docker exec backend**: `[sandbox] backend = "docker"` runs bash in a
  no-network container with the workspace bind-mounted at its real path.
- **Security pass**: broker-worker tool execution, bounded subprocess
  environments, workspace-rooted restricted reads, permission-change
  revocation, threat model in docs/design/24-agent-security.md.
- **Operations**: `scripts/install_gateway_service.sh` installs
  launchd/systemd services; `vak-tray` menu-bar controller with live
  indicators and a watchdog that auto-restarts crashed services;
  `VAK_GATEWAY_TOKEN` pins auth across restarts; hosting guide in
  docs/hosting.md.
- **Reliability fix found in production**: OpenAI-compatible endpoints that
  close tool-call turns without canonical finish reasons no longer strand
  dangling tool calls, and subscriber-less runs are never self-cancelled.

## 0.4.0–0.6.0 (released)

- **Model catalogues are discovered, not hardcoded.** `Core::models_for` — a
  static per-provider table — is gone. `vak_llm::models::list_models` asks the
  provider what the user's key actually reaches (`GET /models`), following
  pagination for Anthropic (`has_more`/`last_id`, 20/page default) and Google
  (`nextPageToken`, 50/page default) so long catalogues are not silently
  truncated; `Core::discover_models` memoises for 5 minutes. Exposed as
  `GET /providers/:name/models`, consumed by the desktop gate, desktop
  Settings and the TUI `/model` picker. Discovery failure reports the reason
  (invalid key, provider down) instead of substituting a stale list — the old
  table advertised 2 opencode-zen models where the key reaches 64, and offered
  4 Anthropic models for a key that no longer authenticates.
- **Provider keys are revocable.** `DELETE /config/key`, a Remove key control
  in desktop Settings, and `/key <provider> --remove` in the TUI. Revoking
  strips the entry from the user `.env`, clears the runtime override and
  the loaded-dotenv copy, drops the cached provider client and discovered
  models, and reports `shadowed_by_env` when the variable is also exported in
  the real environment (which the app cannot unset).
- **Desktop onboarding fixes.** The project gate could strand on the folder
  picker: a `Composer` ref was dereferenced before assignment
  (`ta.selectionStart`), and the throw propagated out of `setBackend` inside
  `refreshBackend`, leaving providers unset. Also: `backend-ready` was emitted
  before the webview subscribed, the gate's recovery poll tore down its own
  timer, `showConnect` captured a non-reactive boolean, and CORS omitted
  PATCH/DELETE while the router and client both used them.
- **Desktop UI**: project switcher and run controls (permission mode,
  transcript density, tokens, context ring) moved from the sidebar footer and
  the status strip into the composer toolbar; sidebar gutters normalised.
- **Single instance**: a second launch refocuses the live window instead of
  starting a rival shell with its own backend.

- **Doc 21 closed out — world-class TUI pass complete.** Interactive
  keymap rebind UI (`/keymap`: ↑↓ select, `r` capture, conflicts surfaced);
  subagent picker + attach/steer (`Alt-S` / `/subagents`): every live child
  registers its steering queues and cancel token in a shared
  `SubagentRegistry`, so Enter steers the child, Tab queues its follow-up,
  Ctrl-C stops only it, Esc detaches.
- **Personalization**: three truecolor theme packs (midnight, synthwave,
  forest) plus custom themes via `[ui.themes.<name>]` (hex or named colors
  over the dark base), all previewed live in the theme picker; Emacs/Vim
  composer modes (`[ui] composer`, `/composer [emacs|vim]`, `/vim`,
  `/emacs`) with normal-mode motions, operator+motion edits, yank/paste
  register, and undo/redo.
- **Custom slash commands**: markdown prompt templates from
  `.vak/commands/*.md` (project), `.vak/plugins/*/commands/*.md`
  (plugin-contributed palette actions), and user `commands/*.md`;
  `$ARGUMENTS` substitution; project > plugin > user precedence; wired into
  completion, the Ctrl-P palette, and `/help`.
- **Opt-in OSC52 clipboard copy** (`[ui] osc52 = true`, then `Alt-Y` or
  `/copy` copies the last response) — never automatic.
- **Accessibility modes** (`[ui.accessibility]`, runtime `/a11y
  plain|motion|reader on|off`): plain/screen-reader strip imposed colors and
  fold box-drawing/decorative glyphs to ASCII in committed history and
  transient panels; reduced motion renders a static spinner glyph.

- **Stop gate** (built-in premature-completion policy, on by default):
  blocks completions that look truncated (trailing plan marker, non-heading
  colon line, unclosed code fence) or that skip verification the prompt
  explicitly demanded with zero commands run. Reuses stop-hook
  continuation (`[stop-guard]` prefix), capped at `max_blocks` per run so
  it can nudge but never trap. `[stop_policy]` config section to tune or
  disable. Born from the dogfood campaign's 5/7 premature-stop rate.
- Workspace-write sandboxes now allow OS temp areas (/tmp plus macOS's
  /var/folders TMPDIR and $TMPDIR): test suites using tempfile/std::env::temp_dir
  no longer die under `cargo test` driven through the agent. Found by
  dogfooding — the sandbox was doing its job a little too well.
- **Learned allow rules**: `[p]` on an approval persists a scoped rule
  (`bash(cargo *)`, `edit(src/x.rs)`, `mcp(server/*)`, …) to
  `.vak/permissions.local.toml`; loaded into every future run in that
  workspace (trusted only), round-trip validated, and unable to shadow
  explicit denies. `[a]` stays session-only for unscopeable calls.
- Sandbox network parity: all sandboxed modes now deny TCP bind/connect —
  Landlock (Linux) handles it explicitly via ABI v4 with fail-closed
  enforcement checks in both read-only and workspace-write; Seatbelt
  (macOS) already denied network implicitly via deny-default profiles.
  FullAccess remains unsandboxed. Smoke scenarios extended to 7.

## 0.2.0 (2026-08-22)

Five post-0.1.0 phases. Headline changes:

### Safety

- **Linux Landlock sandbox** behind the existing `Sandbox` trait: reads and
  execute everywhere, writes only under the canonicalized cwd in
  workspace-write mode, nothing writable in read-only mode. Commands run
  through a hidden `__sandbox` self-exec runner that applies the ruleset
  before spawning the command (children inherit containment). Fail-closed:
  unsupported kernels report sandbox `off` instead of pretending.
  Enforcement smoke (`scripts/landlock_smoke.sh`, 5 scenarios) gates CI's
  new ubuntu job; macOS keeps Seatbelt.
- New dependency: `landlock = "0.4"` — safe wrapper over the Landlock LSM
  syscalls, keeping raw `unsafe` out of the workspace.

### TUI/UX (details in docs/design/18-tui.md)

- Markdown rendering of assistant output with syntax-highlighted fenced
  code; unified diffs for `edit` calls and approval previews; error tails
  on tool failures; thinking indicator.
- Live status row: spinner · elapsed · ↑/↓ tokens · context-window %.
- Approval queue with structured cards (pretty-printed args, rule reason),
  `[y]` allow-once / `[a]` always-this-tool-this-session / `[n·Esc]` deny;
  stray keystrokes can no longer silently answer or deny a pending approval.
- Input: multiline (Alt/Ctrl-J), bracketed paste, wrap- and CJK-aware caret,
  readline keys (Ctrl-U, Ctrl-W, Alt-b/f), Tab completion for slash commands
  and `@file` paths, Ctrl-R reverse history search, persisted history.
- Sessions: `/resume [n|id-prefix]`, `/rewind [seq]` checkpoint browser,
  `/transcript [n]` dump, session browser with first-prompt snippets.
- `/doctor` health panel; `/theme dark|light|plain` runtime switch; bell on
  turn finish; window title.
- Subagents: lifecycle and per-tool-call streams visible live in the parent
  UI; child token usage rolls up into `/cost` and the completed-turn footer.

### Core / reliability

- Steering queues wired through `Core::run_turn_with` — mid-run user input
  is now actually consumed by the loop (previously a UI-side dead end).
- `AgentEvent` carries tool args/result previews; `Approver::approve` sees
  full tool input, enabling informed approval cards.
- `[ui]` config section (`theme`, `bell`); cost estimates from a per-family
  pricing table (unknown models omit dollar figures rather than guessing).

## 0.1.0

Initial release — the full original roadmap:

- Kernel: agent loop + six core tools + append-only JSONL sessions with a
  frozen execution contract (`model-visible means logged`)
- Multi-provider over raw APIs: Anthropic, OpenAI responses + completions,
  OpenRouter, OpenCode Zen (incl. free models), Gemini, Ollama
- Permission engine (rules × modes) + macOS Seatbelt sandbox
- Extensibility: skills, blocking subagents with lineage-linked child
  sessions, lifecycle hooks, MCP client via a lazy meta-tool
- Agentic depth: parallel fan-out gated by resource-claim waves, static flow
  DAGs, dynamic planner with bounded replan (fail-closed)
- Eval harness (deterministic + live-model), HTTP+SSE server, checkpoints/
  rewind, git-worktree isolation
- Reliability pass: retry/backoff honoring Retry-After, per-step watchdog,
  cross-run circuit breaker with run-level endurance, session resume, server
  cancel, graceful shutdown; auto-compaction for long-horizon sessions
