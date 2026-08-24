# 18 — TUI/UX Pass

Status: implemented post-v0.1.0. Rendering now uses a dependency-light,
alternate-screen retained workspace: fixed identity header, resize-aware
transcript viewport, bottom composer, and full-height modal surfaces.
Superseded in parts by 19-modern-tui.md and 21-world-class-tui.md (the
current theme list, composer modes, keymap rebind, subagent attach, and
accessibility surfaces live there and in 05-config.md).

## Upstream enablers (vak-agent / vak-core / vak-config / vak-server)

- `AgentEvent::ToolCallStart` carries `args_json`; `ToolCallEnd` carries
  `result_preview` (truncated output); `ApprovalRequested` carries `args_json`.
- `Approver::approve(tool, args_json, reason)` — approvers see the full input.
- `Core::run_turn_with` accepts `Option<Arc<SteeringQueues>>`: mid-run user
  input pushed by the TUI is now actually drained by the loop (previously the
  queues were never wired through and steering was a UI-side illusion).
  Drained messages append as user entries, preserving model-visible-means-logged.
- `[ui]` config section: `theme = "dark"|"light"|"neo"|"rich"|"teenage"|"plain"` (unknown → dark +
  warning), `bell = true|false`. Unknown keys warn, never fatal.

## Presentation modules (new, dependency-free)

| module | role |
|---|---|
| `theme.rs` | semantic palette presets (`dark`/`light`/`neo`/`rich`/`teenage`/`plain`) + deterministic 256-color SGR mapping |
| `markdown.rs` | stream line-styler: headings/bullets/quotes/hr/inline styles; fenced-code tracking with hand-rolled syntax highlighting (keywords/strings/comments/numbers) for common languages |
| `diffview.rs` | unified diffs via `similar` (hunk headers, +/- coloring, line cap) |
| `status.rs` | spinner frames, elapsed/token formatters |
| `width.rs` | display-width table (CJK wide=2, combining=0) |
| `complete.rs` | pure completion engine: slash commands + bounded `@path` walk |

## App spine (app.rs / render.rs / commands.rs)

- **Markdown streaming**: text deltas buffer until newline, then flush through
  `LineStyler`; fences persist across deltas; remainder flushed at turn end.
  Raw-delta printing removed — visible bytes preserved exactly.
- **Tool cards**: start lines show a summarized arg hint (bash→command,
  read/write/edit→path, grep→pattern in base); `edit` success renders a real
  diff from `edits[].old_string/new_string`; errors show the last 6 output
  lines in error color.
- **Live status row**: spinner · elapsed · ↑in ↓out · context-window %,
  refreshed at 200 ms while running; queued steering shown inline.
- **Approval overhaul**: FIFO queue, structured card (pretty-printed args +
  rule reason), keys `[y] allow once / [a] always-this-tool-this-session /
  [n·Esc·q] deny`. Stray keystrokes are ignored outright — they can no longer
  silently answer (or deny) a pending approval, nor leak into the editor.
- **Multiline input**: Alt-Enter / Ctrl-J insert newlines; bracketed paste
  enabled (RAII); input block renders multiline + wrap-aware with correct
  caret placement via display widths; erase covers the whole previous block.
- **Completions**: Tab completes commands (single → apply incl. trailing
  space; many → common-prefix extension + option list) and `@file` paths.
- **Sessions**: `/resume [n|id-prefix]` reopens past ledgers (mtime-sorted);
  `/rewind [seq]` lists/restores checkpoints of the active session via
  `vak-core::checkpoints`.
- **Misc**: persisted input history under sessions home; terminal bell on turn
  finish (config-gated); window title set at startup; theme applied to every
  rendered element; `/help` is table-driven off the same constant that feeds
  completion.

## Deliberate limits

- Markdown styling is line-granular (no table/column layout) by scrollback
  design; block-level re-rendering would require erasing printed history.
- "Always allow" scopes to tool name for the process lifetime, not per-rule;
  engine-level rule injection needs a Core seam and stays out of scope here.
- Wrapped-line caret placement assumes uniform terminal width between renders
  (resize redraws on next event/tick).

## Slice 2 — informed decisions + input polish

- **Approval diff preview**: `edit` approvals render the proposed change as a
  unified diff (up to 3 edits × 6 lines each) before y/n — the decision is
  informed by what will change, not by a JSON blob.
- **Readline editing**: Ctrl-U clear line, Ctrl-W / Alt-Backspace delete word
  back, Alt-b/f word motion (`Editor::clear/delete_word_back/word_left/
  word_right`).
- **Thinking indicator**: first `ThinkingDelta` of a burst prints one dim
  `· thinking…` line; reset on TurnStart and on next text delta.
- **Subagent visibility**: `TaskDeps.events` forwards child lifecycles to the
  parent event stream as `AgentEvent::SubagentStarted/SubagentFinished`
  (label, error flag, elapsed). Previously subagent events were pumped into a
  void; parallel tasks interleave naturally through the shared channel.
- **`/theme [name]`**: opens a searchable theme picker when no name is given;
  selection previews live, Esc restores the previous theme, Enter applies for
  the session, and Ctrl-S persists `[ui].theme` to project config.

## Slice 3 — cost, search, session browser, subagent streams

- **Cost estimates**: `pricing.rs` maps model families to USD/MTok
  (substring-matched; unknown models omit dollars instead of guessing).
  `/cost` shows `~$x`; the completed-turn footer includes it.
- **Ctrl-R reverse history search**: readline-style — type to refine, Ctrl-R
  for older matches, Enter accepts, Esc restores the pre-search buffer
  (`Editor::begin_search/…/cancel_search` with draft snapshot).
- **Session browser**: `/sessions` shows each ledger's first user prompt
  (bounded JSONL head scan) and relative age; mtime-sorted.
- **Subagent tool streams**: child `ToolCallEnd`s forward to the parent as
  `AgentEvent::SubagentToolCall{label,name,is_error}` rendered indented under
  the ◆ header — parallel fan-out is now observable live. The child channel
  is always drained (a full channel would deadlock the subagent); forwarding
  is best-effort on top of that invariant.

## Slice 4 — introspection commands

- **`/doctor`**: checklist of provider/auth readiness, sessions-home
  writability, config warnings (fail), plus info lines for model/provider/
  mode/sandbox, context window + retry budget, and extension surface
  (skills/hooks/mcp/subagents).
- **`/transcript [n]`**: read-only dump of the active session's last n
  messages (default 40) from `derive_messages()` — roles styled, tool calls
  summarized, results truncated; reinforces model-visible-means-logged by
  making the ledger human-readable in place.

## Slice 5 — visual system

- **Closed adaptive surfaces**: the header, composer, active-run steering area,
  and command palette render as complete width-capped panels with foreground,
  background, padding, and right-edge alignment rather than open text rails.
- **Clean launch**: the viewport clears before the TUI renders, so Cargo build
  output remains in scrollback without becoming part of the application UI.
- **Compact identity**: the header shows a short session identifier and model;
  the full ledger path remains available through session commands and outcome
  metadata.
- **State hierarchy**: palette selection uses a distinct surface, while idle,
  history-search, running, approval, user, and tool states have consistent
  cards and contextual shortcut footers.

## Slice 6 — settings and terminal correctness

- **Raw-mode newline safety**: all committed multiline strings normalize LF,
  CRLF, and CR to terminal CRLF. Help, shortcuts, tool output, and future
  multiline notices cannot drift diagonally across the viewport.
- **Structured help**: `/help` and `/keys` render width-bounded panels and the
  composer is restored immediately after every command or shell action.
- **Provider picker**: `/provider` searches every registered provider and shows
  whether its credential resolves right now (real env → runtime overrides →
  `.env` files — the same lookup runs use, via `Core::provider_configured`);
  choosing one selects that provider's first discovered model.
- **Key lifecycle**: `/key` opens a status board of every provider's env var
  and readiness; `/key <provider> SECRET` stores it through the shared Core
  into `~/.vakcoder/.env` (0600) and it is effective immediately;
  `/key <provider> --remove` revokes it (also `--revoke` / `--clear`) and
  warns when the variable is still exported in the real environment, which
  no app-level action can unset. Identical to the desktop Settings pane and
  `PUT` / `DELETE /config/key`.
- **Model picker**: `/model` lists the models the current key actually
  reaches (`Core::discover_models`, live from the provider — never a
  hardcoded list) while still accepting any exact model ID typed by the
  user. Discovery failure degrades to the current model rather than
  inventing a catalogue.
- **Settings dashboard**: `/config` and `/settings` show effective agent,
  permission, sandbox, appearance, limit, auth, and file-path state.
- **Explicit scope**: Enter applies a provider/model choice for the current
  process; Ctrl-S atomically upserts those two top-level keys in the project
  config while preserving unrelated content.
- **Editor telemetry**: the composer header identifies `provider/model`; its
  footer shows the live line and column alongside contextual actions.

## Slice 7 — full-workspace application shell

- **Real terminal ownership**: the TUI enters the alternate screen and restores
  the prior terminal exactly on exit. Header, transcript, and composer occupy
  distinct retained regions rather than appearing as cramped scrollback cards.
- **Full-width responsive layout**: surfaces use the live terminal width; a
  resize atomically rebuilds header wrapping, transcript capacity, active
  modal/picker geometry, and caret placement.
- **Retained transcript viewport**: committed output is kept as styled logical
  lines, ANSI-aware wrapped to the current width, and tail-fitted between the
  fixed header and composer. The session JSONL remains the durable truth.
- **Modal workspace**: `/help`, `/keys`, `/settings`, and `/features` replace
  the central workspace, support arrow/PageUp/PageDown/Home/End navigation,
  and restore the transcript and editor on Esc.
- **Settings navigation**: the modal surfaces every resolved agent, safety,
  reliability, extension, interface, and path setting. P opens provider,
  M model, and F the capability explorer without leaving the modal flow.
- **Feature explorer**: exposes implemented agent execution, safety, session,
  checkpoint, reliability, tool, skill, hook, MCP, flow, eval, server, desktop,
  and TUI entry points instead of leaving them discoverable only in source or
  CLI help.

## Slice 8 — full system surface (Aug 2026)

The TUI now reaches every operator-relevant core capability; nothing is
display-only anymore.

- **`/mode [read-only|workspace-write|full-access]`** — runtime permission
  switching. Honors invariant 11: pending approvals are denied, session-allow
  grants are cleared, an attached subagent is stopped, and an in-flight run is
  cancelled *before* the new mode is reported — when a run is active the
  switch is deferred and applied the moment the run actually stops, before any
  queued follow-up fires.
- **`/compact`** — manual compaction via `Core::compact_session_now`: plans
  the partition, runs the summarizer through the provider with a `Summarize`
  work receipt appended to the ledger, applies the compaction entry
  (append-only), and reports before/after token estimates. The session always
  returns to the composer even on failure (`CompactOutcome`); refused while a
  run is active.
- **`/budget`** — FinOps view: today's and trailing-7d estimated spend from
  the cost ledger vs configured `[finops]` run/day caps and price overrides.
- **`/memory [note]`** — lists this workspace's durable memory notes
  (newest first) or appends one directly; same store the `remember` tool and
  the server's `GET /memory` read.
- **`/proposals [promote|reject <id>]`** — review queue for learned skill
  proposals: bare lists pending proposals, promote installs to user-level
  discovery, reject discards. Closes the last human-unreachable part of the
  learning loop (docs/design/26).
- **`/mcp`** — configured MCP servers (command/args) plus discovered
  meta-tools.
- **`/sandbox [os|docker|default]`** — runtime bash-backend selection via
  `Core::set_sandbox_backend`; docker selection probes daemon availability
  first but still fails closed at call time per docs/design/25. Bare shows
  effective backend/name/image.
- **Reliability visibility** — `/doctor` and `/config` now show live circuit-
  breaker state (closed / open + cooldown remaining), today's spend against
  the day cap, and the frozen route ladder committed at admission.
- **`!cmd` parity** — shell passthrough executes through the brokered bash
  tool (`__tool_worker`) under the active mode's sandbox instead of raw
  in-process execution; output still lands in the ledger as user context.
