# 21 — World-class agent terminal

Status: research synthesis and implementation specification, August 2026.

## Objective

Build an agent terminal that feels as deliberate as Codex, Claude Code, and
OpenCode without inheriting their most costly rendering and state-management
failures. Richness means strong hierarchy, fast navigation, visible state,
informed decisions, and expert-speed controls. It does not require owning the
terminal's scrollback or mouse.

## Competitive findings

### Codex CLI

Codex has a broad keyboard-first command surface: slash-command filtering,
model and permission pickers, configurable keymaps, Vim mode, conversation
fork/side paths, subagent navigation, background terminal inspection, raw
scrollback, image paste, steering, and follow-up queues. Its current tooltip
catalog is the clearest compact inventory of daily workflows.

Sources:

- https://github.com/openai/codex/blob/main/codex-rs/tui/tooltips.txt
- https://github.com/openai/codex/blob/main/codex-rs/tui/src/slash_command.rs
- https://github.com/openai/codex/blob/main/codex-rs/config/src/tui_keymap.rs

### Claude Code

Claude has the deepest context-specific interaction model. Keybindings are
scoped to chat, completion, confirmation, transcript, task, model picker,
attachments, footer, diff, and other modes. It supports chord bindings,
external-editor handoff, prompt stash, image paste, permission-mode cycling,
rewind/summarize, transcript navigation/export, background tasks, model and
thinking controls, and Vim editing.

Sources:

- https://code.claude.com/docs/en/keybindings
- https://code.claude.com/docs/en/interactive-mode
- https://code.claude.com/docs/en/permission-modes
- https://code.claude.com/docs/en/commands

### OpenCode

OpenCode leads in visual customization and navigable application structure:
many built-in and custom truecolor themes, a leader-key system, remappable
actions, configurable scroll behavior, attention sounds/notifications,
session and child-session navigation, snapshot-backed undo/redo, command and
settings dialogs, plugins, and compact/expanded thinking and tool views.

Sources:

- https://dev.opencode.ai/docs/keybinds
- https://dev.opencode.ai/docs/themes/
- https://dev.opencode.ai/docs/tui/
- https://opencode.ai/v2/docs/cli/config

Source audit (commit `3a31c4ea`, 2026-08-23): the current implementation is
built on OpenTUI Core + Solid. `packages/tui/src/app.tsx` creates one retained
renderer at a 60 FPS target; reactive state invalidates that renderer instead
of issuing and flushing cursor commands from every key handler. Agent/server
work runs in `packages/opencode/src/cli/tui/worker.ts`, isolated from terminal
input and rendering. Prompt autocomplete, permission dialogs, keymap contexts,
selection, and focus are renderable components rather than competing print
loops. vakcoder keeps native scrollback, but adopts the underlying rule: one
bounded transient state tree, one focus owner, and at most one atomic terminal
write for a changed frame.

### Gemini CLI

Gemini has the broadest documented input grammar: context-specific remappable
keybindings, external editor, queueing, rich completion focus, approval-mode
cycling, minimal/full detail modes, paste placeholders, Markdown and mouse
toggles, scroll controls, shortcuts overlay, settings UI, custom commands,
and a substantial Vim implementation.

Sources:

- https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/keyboard-shortcuts.md
- https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/commands.md
- https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/settings.md
- https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/custom-commands.md

### Aider

Aider remains a useful simplicity baseline. Prompt-toolkit supplies mature
Emacs and Vi editing, reliable multiline entry, external-editor handoff, chat
modes, file-set management, direct Git commands, diff/lint workflows, and
safe interruption that preserves partial output.

Sources:

- https://aider.chat/docs/usage/commands.html
- https://aider.chat/docs/config/options.html

## Recurring user pain

| Failure class | Evidence | Product requirement |
|---|---|---|
| Render flicker and cursor flashing | Gemini #21808; Claude #52825 | Never redraw committed transcript; rate-limit transient redraws and hide cursor during non-editable animation |
| Fullscreen breaks selection/copy | Claude #71438, #66269, #41954 | Native selection is the default; never capture mouse; offer explicit OSC52 copy as an addition |
| Resize corrupts scrollback | Claude #52660 | Track transient rows exactly; handle resize as an atomic clear-and-redraw; never replay committed rows |
| Large paste hangs or corrupts input | Codex #28116; Claude #13183 | Parse bracketed paste as one bounded event; collapse large payloads; never synchronously lay out megabytes |
| Approval steals composer input | Codex #27374 | Focus cannot change on arrival; approvals enter a visible pending queue and require an explicit focus transition |
| Approval state becomes stale | OpenCode #28312, #29422, #36835 | Approval IDs have one authoritative store; reconcile on resume/attach; expired prompts dismiss with a reason |
| Invisible approval causes permanent “thinking” | OpenCode #16367 | Waiting-for-user is a typed run state with timeout, notification, and headless failure—not a spinner |
| Long approval content hides actions | OpenCode #40793 | Decision actions are fixed; evidence is bounded, wrapped, scrollable/copyable, and expandable |
| View jumps or position is unknowable | OpenCode #37272, #33411 | Follow-tail is explicit; user scroll suspends auto-follow; show position and a jump-to-latest action |
| Input acknowledgment is delayed | OpenCode #30994 | Echo user input optimistically before server/model work; never wait for title/snapshot/tool resolution |
| Terminal escape input leaks into text | OpenCode #681; Codex #4260 | Decode events by protocol and context; unknown control sequences never become composer text |
| Shortcuts are powerful but undiscoverable | Gemini #7114; Codex #19968 | Contextual footer, searchable shortcut overlay, command palette, and actionable terminal diagnostics |

Issue sources:

- https://github.com/openai/codex/issues/28116
- https://github.com/openai/codex/issues/27374
- https://github.com/google-gemini/gemini-cli/issues/21808
- https://github.com/anthropics/claude-code/issues/71438
- https://github.com/anthropics/claude-code/issues/66269
- https://github.com/anthropics/claude-code/issues/52660
- https://github.com/anthropics/claude-code/issues/13183
- https://github.com/anomalyco/opencode/issues/37272
- https://github.com/anomalyco/opencode/issues/28312
- https://github.com/anomalyco/opencode/issues/16367
- https://github.com/anomalyco/opencode/issues/30994
- https://github.com/anomalyco/opencode/issues/40793

## The vakcoder interaction architecture

### 1. Retained full-screen workspace

- **Committed transcript** is retained as logical styled lines and viewport-
  wrapped without changing the append-only session ledger that backs it.
- **Application chrome** is a fixed identity header and bottom-anchored
  composer/run surface with the transcript using every row between them.
- **Transient workspace** can expand into a full-height modal for settings,
  help, feature discovery, pickers, and future transcript navigation.
- Resize, theme, and animation rebuild only the retained regions they affect.
- Panels use the terminal width; narrow terminals degrade to compact rows
  rather than overflowing or hiding actions.
- A transient repaint is composed in memory and flushed once. The cursor's row
  inside the old surface is tracked, so clearing starts at the real surface
  origin and returns there before the replacement is written.
- An identical transient state is not repainted. High-frequency model deltas
  that do not complete a transcript line therefore produce zero terminal
  writes; input remains responsive while streaming.

### 2. Explicit focus state machine

`Composer | Completion | Palette | HistorySearch | Approval | Transcript |
ExternalEditor | RunningSteer`

External events may request focus but never take it. An approval arriving while
the user types becomes a pending chip; Enter continues to mean send until the
user explicitly opens the approval. This prevents accidental authorization.

### 3. Input as structured data

- Keymaps bind named actions within contexts, not raw global key branches.
- Bracketed paste is one event with byte/line metadata and a bounded preview.
- Attachments are chips backed by immutable payload references.
- Slash, file, skill, model, agent, and session completion share one ranked
  suggestion engine.
- Large inputs and tool evidence never trigger unbounded synchronous layout.

### 4. Run-state truth

The footer exposes one typed state: connecting, thinking, streaming, running a
tool, awaiting approval, retrying, compacting, steering queued, cancelling, or
stale. Every state includes elapsed time and the action currently available.
No generic spinner is allowed to represent waiting for the user or a dead
transport.

### 5. Informed, durable approvals

- Always show the exact operation, scope, rule, affected path/host, and bounded
  diff/output evidence.
- Decision controls remain visible regardless of evidence length.
- Allow-once, session scope, durable pattern, deny, and inspect are distinct.
- Composer text is preserved and cannot answer an approval accidentally.
- Local TUI, server, desktop, and remote clients resolve the same request ID.

## Feature target

### P0 — trust and daily speed

- Closed adaptive visual surfaces and clean launch.
- `/` opens command suggestions immediately; `@` opens ranked path/skill
  suggestions immediately.
- Contextual keymap engine and `/keymap` editor with conflict detection.
- Large-paste placeholder with Ctrl-O expand, byte/line count, and zero-copy
  submission.
- Ctrl-G external editor with crash-safe draft recovery.
- Approval arrival never steals focus; pending approvals are explicit.
- Resize-safe transient redraw and render-rate budget.
- Terminal capability diagnostics and literal-key capture.

### P1 — navigation and observability

- Transcript viewer with prompt jumps, search, copy/export, and jump-to-latest.
- Model, session, theme, permission, and subagent pickers.
- Background terminal and subagent tree with attach/steer/stop.
- Tool cards with duration, status, affected resources, bounded preview, and
  per-tool expand/copy.
- Visible context/cost/rate-limit meters and typed retry/compaction states.
- OSC9/bell notifications for approval and completion.

### P2 — personalization and extension

- Built-in truecolor theme packs plus custom theme files and live preview.
- Emacs and Vim composer modes.
- Custom commands, project command namespaces, and plugin-contributed palette
  actions.
- Optional OSC52 copy, never automatic clipboard mutation.
- Accessible plain mode, reduced-motion mode, and screen-reader transcript
  mode.

## Acceptance criteria

1. Typing latency p99 stays below one frame under streaming and tool events.
2. A 1 MB paste does not freeze, allocate proportional rendered rows, or lose
   bytes; the ledger contains the exact submitted input.
3. Approval arrival cannot consume a keystroke intended for the composer.
4. Resize under tmux never adds duplicate committed transcript rows.
5. CJK, combining marks, emoji sequences, and pasted control text round-trip.
6. Every visible mode has a discoverable exit and at least one displayed next
   action.
7. Every model-visible attachment, paste, steer, and command expansion remains
   reconstructable from the append-only session ledger.
