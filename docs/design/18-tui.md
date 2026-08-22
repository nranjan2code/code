# 18 — TUI/UX Pass

Status: implemented post-v0.1.0. All rendering remains native-scrollback,
inline-streaming; nothing here introduces alternate-screen or TUI frameworks.

## Upstream enablers (vak-agent / vak-core / vak-config / vak-server)

- `AgentEvent::ToolCallStart` carries `args_json`; `ToolCallEnd` carries
  `result_preview` (truncated output); `ApprovalRequested` carries `args_json`.
- `Approver::approve(tool, args_json, reason)` — approvers see the full input.
- `Core::run_turn_with` accepts `Option<Arc<SteeringQueues>>`: mid-run user
  input pushed by the TUI is now actually drained by the loop (previously the
  queues were never wired through and steering was a UI-side illusion).
  Drained messages append as user entries, preserving model-visible-means-logged.
- `[ui]` config section: `theme = "dark"|"light"|"plain"` (unknown → dark +
  warning), `bell = true|false`. Unknown keys warn, never fatal.

## Presentation modules (new, dependency-free)

| module | role |
|---|---|
| `theme.rs` | palette presets (`dark`/`light`/`plain`) + deterministic `fg()` SGR mapping |
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
- **`/theme [dark|light|plain]`**: runtime switch via new
  `Core::set_theme/effective_theme` override seam; no-arg lists themes.

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
