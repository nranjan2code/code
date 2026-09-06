# 30 — Output engineering and channel delivery

Status: schema-v2 semantic timeline with result-scoped outcome/evidence metadata
and projected collaborative goal state, deterministic CommonMark compiler,
universal recipe catalog (research, coding diffs/tests, telemetry charts,
spreadsheet grids, terminal sessions, culinary recipes) and tool-provenance signal
engine, desktop native presentation suite (ResearchCards, DiffInspector,
TestMatrix, UniversalChart, DataGrid, TerminalConsole, RecipeCard),
snapshot/SSE projection with merged planner, isolated worker, trusted templates,
plugin-extensible skill registry, structured fence projection on all markup
surfaces, engagement posture integration, Telegram/Slack/Discord projections,
semantic webhook envelope, adapter registry, ordered multi-message output, and
durable retry outbox implemented. TUI remains follow-up work.

## Problem

The assistant's Markdown is currently close to the delivery contract. That is
safe for export but weak for channels: terminals, desktop, Telegram, Slack,
Discord, and generic webhooks have different widths, markup, interaction
models, accessibility needs, and hard payload limits.

Formatting must not become a second source of truth. The session ledger keeps
the complete assistant output. Delivery is a derived projection.

## Semantic timeline

The canonical path is:

`ledger + live events → OutputTimeline → SurfaceCapabilities → native renderer or text fallback`

Presentation is a deterministic, versioned projection. The ledger stays the
authority and exact source Markdown stays available for export, audit, legacy
clients, and emergency degradation. Schema v2 separates three axes that were
previously conflated in prose and CSS:

- `OutputRole`: system, user, assistant, tool, or subagent.
- `OutputKind`: message, information, approval, progress, retry, error,
  outcome, or artifact.
- `OutputStatus`: pending, running, succeeded, failed, denied, cancelled, or
  partial.

An `OutputItem` has a stable ID, timestamp, turn ID, typed content,
provenance, actions, and exact textual fallback. A `PresentationDocument`
contains a closed block and inline AST for headings, paragraphs, lists,
tables, quotes, code, diffs, callouts, citations, media, artifact references,
and opaque fallback. `pulldown-cmark` compiles model Markdown without mounting
raw HTML. Unsafe links remain readable but are not interactive. Every source
construct is either represented by a trusted node or retained as an explicit
fallback with a degradation diagnostic.

`SurfaceCapabilities` describes structured blocks, tables, code, links,
media, file references, actions, color, interactivity, accessibility/plain
mode, and size. It is a projector input, not permission: an action still uses
the existing approval and permission boundary.

## Contract

`vak-delivery` defines six layers:

1. `DeliveryKind`/`DeliveryContent` identify what is being delivered. An
   assistant answer, task summary, alert, approval, progress update, and tool
   result are not interchangeable. System/developer/internal messages are
   control-plane data and are rejected by the external delivery renderer.
2. `AnswerDraft` contains the exact source Markdown, its legacy conservative
   projection, and a `PresentationDocument`. Schema-v1 drafts are accepted and
   compiled on read. Unknown structures become inert fallback; they are never
   dropped.
3. `DeliveryProfile` declares the target's accepted markup and constraints and
   maps to `SurfaceCapabilities`.
4. `TemplateSpec`/`TemplateRegistry` provide replaceable, declarative layouts.
   Templates contain literal text, approved slots, and bounded conditionals;
   they cannot execute code or access tools. Built-in, project, and user
   templates can be active. Agent-created templates are proposals until an
   explicit activation operation approves them. Higher-precedence active
   templates replace lower-precedence ones by ID.
5. `DeliveryPacket` contains the rendered payload, optional semantic timeline,
   exact Markdown fallback, bounded chunks, block coverage, actions, and
   diagnostics. Existing fields remain stable for legacy consumers.
6. `vak-delivery-worker` is a separate line-oriented process. It renders jobs
   but does not own transport credentials, sessions, permissions, or the
   canonical ledger.

User and trusted-project configuration can select a template by ID and replace
it without changing Rust code. Files are loaded from `<home>/output.toml` and
trusted `<cwd>/.vak/output.toml`; user definitions win by ID:

```toml
[templates.compact-result]
revision = 2
format = "{title}\n\n{body}"

[channels.telegram]
template = "compact-result"
max_chars = 3500
```

Allowed slots are `{title}`, `{body}`, `{source_markdown}`, `{block_count}` and
`{metadata.KEY}`. Templates are schema- and size-bounded data: no scripts,
includes, tools, filesystem lookup, network access, or expression language.
An untrusted project output file is ignored. Agent proposals remain inactive
until explicitly activated.

## Engagement posture

`DeliveryProfile` carries a `DeliveryPosture` (cadence × urgency) that the
session or task sets. The posture decides **when** a packet goes out, never
**what** it says — the renderer, packet contents, and outbox are untouched.

- `Disposition::Send` — render and deliver immediately (the pre-kernel default).
- `Disposition::HoldUntilComplete` — enqueue to the outbox; the replay loop
  skips held packets until the turn completes or the posture resolves to
  `Send`.
- `Disposition::HoldForDigest` — enqueue to the outbox; the digest flush
  delivers it.

Two rules override the cadence: an `Interrupt` urgency always sends (an
irreversible step's confirmation must not wait), and any packet needing a
person (`DeliveryKind::Approval`, `Alert`) always sends because a held gate is
a stopped run. The outbox replay checks posture before attempting delivery,
so held packets do not burn retry budget.

Every block receives an ID and a coverage disposition. Unknown structures stay
in the exact fallback even when the target cannot render them richly. Silent
loss is a protocol error.

Templates apply only to outward-facing answer-like messages. An approval keeps
its action IDs, verbs, data, and expiry as separate fields even when its text
fallback is rendered. Progress and tool results retain their typed payloads.
This prevents a template from accidentally turning a human-in-the-loop gate,
system message, or tool event into ordinary prose.

## Structured fence projection (schema-v2)

Assistant answers and tool results may contain ```vak fenced blocks carrying
provider-agnostic structured JSON instead of raw model output. Because models
and providers switch turn-by-turn (OpenAI `choices[0].message.content`, Anthropic
`content[0].text`, Google `candidates[0].content`), the raw JSON shape is never
stable. Rather than per-provider adapters, the worker validates each fence's
payload against a `SkillRegistry` and replaces it with a deterministic
`structured_markdown()` projection: human-readable heading + typed body +
inspectable JSON appendix. Ordinary Markdown and invalid/incomplete fences stay
exact.

```vak fences are projected on all markup-passing surfaces (TelegramHtml,
SlackMrkdwn, DiscordMarkdown, Markdown, Plain). For `Markup::Json` (the desktop
native AST path) the raw fence is preserved for the structured component
registry.

The skill registry is merged from builtins plus plugin-contributed
`PresentationSkillManifest` files (declared in a plugin's
`components.presentation` list). `parse_fragment_with` resolves the owning skill
by `semantic_type` via `SkillRegistry::find_by_type`, attributing the output to
its real `skill_id` rather than assuming `"core"`. Plugin manifests may declare
an optional JSON Schema (`schema` field) for payload validation; without one,
the plugin renderer owns shape correctness and the payload is accepted.


Production CLI and desktop binaries host a private `__delivery_worker`
subcommand. The server keeps one empty-environment child alive per sessions
home and exchanges versioned JSON lines. The renderer receives no provider
keys, channel credentials, tools, session handles, or network access. A
five-second watchdog restarts a broken process once; a deterministic in-process
fallback adds a diagnostic rather than suppressing output. Rendering calls no
LLM and has bounded payload/template work.

The `DeliveryJob` carries an optional `skill_registry` field. The Core
populates it with the merged registry (builtins + plugin-contributed
presentation skills), which the worker uses for ```vak fence validation. When
`None`, the worker falls back to `built_in_skill_registry()`.

Before push, the server creates an append-only job-state JSONL under
`<home>/delivery/jobs/`. It records pending, delivered, failed-attempt, and
dead-letter snapshots without deleting the exact job. Writes are synced; Unix
also syncs the containing directory on creation. Replay scans at most 100 jobs
every 30 seconds and stops after ten attempts. The inbox copy is written before
transport, so missing credentials or remote failure cannot erase the signal.

Attached TUI and desktop clients should normally render semantic events locally:
they know terminal width, theme, accessibility mode, and window state. The
worker is primarily for Telegram, webhooks, scheduled tasks, and unattended
surfaces. A small plain-text emergency path may remain in the server for
critical failure alerts.

## Durable lifecycle and reconnect

`vak-session` stores projection-neutral `ActivityRecord` entries for approval
transitions, retries, route fallback, significant diagnostics, and run
completion. They are skipped by model-context derivation just like receipts;
token deltas are intentionally not persisted. Old ledgers remain readable and
are projected best-effort from messages, tool blocks, and receipts.

The server exposes `GET /sessions/:id/presentation` for an idempotent snapshot
and `GET /sessions/:id/presentation/events` for `Snapshot`, `ItemStarted`,
`TextDelta`, `ItemReplaced`, and `ItemCompleted`. The first event on every SSE
connection is a complete snapshot, so reconnect never depends on replaying a
lost broadcast delta. The snapshot is built with the live Core's merged
presentation planner (builtins + plugin recipes/signals); historical session
views use the builtin-only planner since no Core is available. Raw
`AgentEvent` and Markdown transcript endpoints stay available for
compatibility and forensic inspection.

## Desktop projection

The SolidJS desktop uses a closed component registry keyed by semantic node
type. A completed turn is outcome-first: user request, blocking approval,
outcome, artifact shelf, optional activity/audit, and persistent recovery for
failed work. Outcome, Balanced, and Audit modes are different projections of
the same timeline. Prose is continuous; only code, tables, diffs, callouts,
artifacts, approvals, and recovery states get purpose-built containment.
An intermediate tool error in a run that later records successful completion
remains in Activity as a failed, recoverable step; only an unresolved tool
error or terminal run failure enters the persistent recovery banner.

Active assistant text is appended without reparsing or mounting HTML. When a
turn settles, hydration replaces it with the ledger-derived AST. Unknown nodes
fall back visibly, links are scheme-checked, raw HTML is inert, and artifact
and approval actions route through existing desktop commands.

The desktop client provides a universal presentation suite embedded in the
continuous chat canvas:
- **`ResearchCards`**: Key takeaway rows with numbered badges, superscript
  citation tags (`[1]`, `[2]`), hover popovers displaying quoted snippets and
  source badges, and verified source link tiles with domain favicons.
- **`DiffInspector`**: Zed/Cursor-grade diff viewer with file drawer, delta
  counters (`+` / `-`), unified vs. side-by-side mode toggle, gutter line
  numbering, and one-click `openInEditor` host integration.
- **`TestMatrix`**: Test suite dashboard featuring an SVG circular pass-rate
  progress ring, filter chips (`All` vs `Failed Only`), and collapsible
  assertion traceback drawers.
- **`UniversalChart`**: Telemetry and benchmark stage with KPI pods,
  multi-series SVG curves with gradient area fills, live mouse-tracking
  crosshair line with floating glass data bubble, and one-click CSV export.
- **`DataGrid`**: Interactive tabular grid with numeric-aware column sorting,
  real-time client-side search filtering, tabular alignment, and CSV export.
  Standard markdown tables with $\ge 3$ rows automatically promote to this
  grid.
- **`TerminalConsole`**: Authentic dark terminal container with prompt line,
  exit code status pill (`Exit 0`), execution duration, and formatted output.
- **`RecipeCard`**: Dynamic servings scaler (`-` 2 `+`) that recalculates
  ingredient weights and measurements, paired with live countdown step timers.

## Channel-specific markup projections

`Markup` has one variant per target dialect, not one generic "Markdown" for
every chat surface: `TelegramHtml`, `SlackMrkdwn`, `DiscordMarkdown`, plus
`Plain`/`Markdown`/`Json` for surfaces that render the source directly.
Passing raw GFM through to Slack or Discord is not a safe default — each
dialect disagrees with GFM in specific, visible ways:

- **Telegram** (`telegram::markdown_to_html`) projects to the HTML subset
  Telegram's Bot API accepts: `<b>/<i>/<s>/<tg-spoiler>/<code>/<pre>/<a>`.
  Headings become bold (h1/h2 upshifted for visual weight since Telegram HTML
  has no heading tags), nested bullets get depth markers (`•`/`◦`/`▪`),
  consecutive `> ` lines merge into one `<blockquote>` instead of one per
  line, fenced code keeps its language as `<code class="language-x">`, and
  GFM tables render as an aligned monospace grid inside `<pre>` since
  Telegram HTML has no table element.
- **Slack** (`slack::markdown_to_mrkdwn`) projects to `mrkdwn`, which
  disagrees with GFM on the two markers that matter most: bold is a single
  `*`, italic is `_` (GFM's single `*` would collide with Slack's bold), and
  links are `<url|text>` rather than `[text](url)`. mrkdwn has no heading or
  table syntax at all, so headings become a bold line and tables become an
  aligned monospace block inside a fenced code span.
- **Discord** (`discord::markdown_to_discord`) is the smallest delta from
  GFM — bold, italic, strikethrough, spoilers, fences, blockquotes, and
  `#`/`##`/`###` headings already match natively and pass through untouched.
  The two gaps: `[text](url)` does not hyperlink in a plain message (only
  inside an embed), so it becomes `text (<url>)` — the angle brackets also
  suppress Discord's own link-preview embed; and there is no table syntax,
  so tables become the same aligned monospace block as the other two.

All three keep the exact `source_markdown` as `fallback_markdown` regardless
of projection, so a channel-specific rendering bug never loses the original
answer.

## Multi-message rule

When one message cannot satisfy a channel limit, `DeliveryPacket.chunks` holds
every ordered part. Adapters must send all chunks sequentially; truncation is
not normal delivery. Telegram closes and reopens HTML tags at boundaries so
each chunk parses independently; Slack and Discord close and reopen an open
fenced code block (` ``` `) at a chunk boundary the same way, so a split never
leaves a dangling fence that swallows the rest of the message as code. Actions
appear once, normally on the final chunk. A partial send returns an error and
retains the same job ID for replay. `fallback_markdown` always carries the
exact unsplit answer.

Chunking is Unicode-scalar safe, but not yet grapheme-cluster aware; it can
split a visible emoji sequence while remaining valid UTF-8. That limitation is
explicit until grapheme golden tests land.

## Adapter boundary

Native outbound adapters implement three operations: routing `scheme`, hard
`DeliveryProfile`, and async `send`. The registry currently contains `log:`
and `webhook:`. Credentials are resolved only inside the adapter. Generic
webhooks receive `{target,text,ts,job_id,delivery}` plus `Idempotency-Key`, so a
relay can deduplicate and consume semantics while old receivers keep using
`text`.

Telegram, Slack, and Discord are sidecar adapters: `/gateway/inbound` returns
the semantic packet and each bridge sends every chunk through its channel's
own send call (`sendMessage`, `chat.postMessage`, and the Discord message
endpoint respectively). Future Teams or Matrix sidecars can post optional
`capabilities` (`markup`, `max_chars`, tables, code, links, actions) and
consume the same packet without changing the agent loop. Unknown surfaces
start conservative; declared limits are capped at 100,000.

For context, Telegram text is limited to 4096 characters, Slack recommends
4000 top-level characters and imposes per-block limits, and Discord message
content is limited to 2000 characters. Discord Components V2 also changes
whether traditional content and embeds may coexist. These are distinct
protocol contracts, not styling preferences.

Primary references:

- [CloudEvents core specification](https://github.com/cloudevents/spec/blob/main/cloudevents/spec.md)
- [Telegram Bot API](https://core.telegram.org/bots/api#sendmessage)
- [Slack `chat.postMessage`](https://api.slack.com/methods/chat.postMessage)
- [Slack Block Kit limits](https://api.slack.com/reference/block-kit/blocks)
- [Discord message resource](https://docs.discord.com/developers/resources/message)
- [Discord components](https://docs.discord.com/developers/components/overview)

## Safety and performance rules

- The worker is deterministic by default; no LLM editorial pass is in the
  delivery critical path.
- The server does not wait on remote rendering or transport calls after a
  committed run; the outbox is the handoff point.
- Worker concurrency, payload size, and per-target retry budgets are bounded.
- Template files are data, not code: they are schema-validated, size-bounded,
  and resolved by explicit precedence.
- Credentials are injected only into the transport adapter that needs them.
- The original answer remains available if a worker is unavailable or a target
  rejects its projection.
- Rendering is versioned. A packet records the renderer/schema version and
  degradation decisions for debugging and replay.

## Governed presentation skills

Presentation is extensible through the versioned `presentation.v1` skill
contract. A skill registers typed semantic outputs (for example
`link.preview`, `metric`, `chart`, `research.synthesis`, `coding.diff`,
`test.report`, `terminal.view`, `data.grid`, `recipe.card`, or `media.image`)
and a capability-scoped renderer binding. Skills contribute data and metadata
only; they cannot ship executable UI code or mount raw HTML. The trusted renderer
registry validates the closed payload schema, checks the target surface
capabilities, and records the selected renderer or an explicit fallback diagnostic.

The planner first extracts deterministic signals from the completed timeline
and tool provenance via `SignalContext` (inspecting tool names, CLI commands,
exit codes, and output patterns), then selects the highest-specificity built-in
recipe using `RecipeCatalog::choose`. Built-in recipes span universal life and
work scenarios:
- `research.synthesis` (`["research", "synthesis", "takeaways"]`)
- `coding.diff_inspector` (`["diff", "files_changed"]`)
- `coding.change_summary` (`["files_changed"]`)
- `coding.test_report` (`["tests", "pass_fail"]`)
- `terminal.session` (`["terminal", "command_exec"]`)
- `data.multi_chart` (`["chart", "telemetry"]`)
- `data.spreadsheet_grid` (`["table_data", "tabular"]`)
- `lifestyle.culinary_recipe` (`["recipe", "ingredients"]`)
- `weather.forecast` (`["temperature", "forecast"]`)
- `workflow.approval` (`["approval", "action"]`)
- `artifact.collection` (`["artifact"]`)

If no specific recipe matches, `answer.basic` is used as the deterministic
last-resort composition. The decision (recipe id/version, matched signals,
renderer, and rejected candidates) is persisted in presentation metadata so a
user can inspect “Why this rendering?” and an operator can reproduce it.

Rich cards are therefore a projection, not a second transcript. Desktop renders
trusted link previews, metrics, universal multi-series SVG charts, test matrices,
diff inspectors, terminal consoles, and culinary recipe cards with countdown
timers; terminal and chat surfaces receive deterministic compact text or links
when their capabilities do not include the richer component. The exact source
Markdown remains the export and emergency fallback for every projection.

## Integration status

1. Durable outbox records and a persistent worker supervisor: complete.
2. Telegram conversion behind the worker with tag-safe chunking: complete.
3. Slack `mrkdwn` and Discord markdown conversion behind the worker, both
   with fence-safe chunking: complete.
4. Semantic webhook envelope plus `text` fallback: complete.
5. Stable semantic packet returned to sidecars and available to native clients:
   complete.
6. Desktop universal presentation suite embedded in continuous chat canvas
   (ResearchCards with citation popovers, Zed-grade DiffInspector, TestMatrix with
   pass-rate ring, UniversalChart with mouse crosshair tracking and KPI pods, DataGrid
   with column sorting and search, TerminalConsole with exit codes, RecipeCard with
   scaling and timers): complete.
7. TUI projector: planned.
8. Golden fixtures cover parser losslessness, nested structures, tables, code,
   diffs, unsafe links/HTML, artifacts, lifecycle states, legacy drafts, and
   unsupported capabilities; channel-specific accessibility and visual checks
   remain part of each surface release gate.
