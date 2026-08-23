# 19 — Modern TUI: competitive analysis & roadmap

Goal: bring vak-tui to parity with the best agent CLIs (OpenCode, Codex CLI,
Claude Code) and beyond, grounded in documented user pain rather than
feature envy. Sources: official docs/keybind references for OpenCode +
Codex CLI; GitHub issues / Reddit / HN pain mining across Claude Code,
Codex, OpenCode, Gemini CLI, Aider, Crush (research notes, Aug 2026).

## What we have today (docs/design/18-tui.md)

Inline-streaming, **native-scrollback** rendering (no alt-screen). Markdown
line styling + fenced-code highlighting, edit diffs, tool cards with arg
hints, live status row (spinner · elapsed · ↑↓tokens · context %),
approval FIFO with diff previews (y/a/n), multiline input + bracketed
paste, Tab completion (commands + @paths), Ctrl-R history search,
readline word ops, `/resume /rewind /theme /transcript /doctor /cost
/context /sessions /model /clear`, persisted history, subagent live
streams, retry/compaction/stop-hook event surfacing.

## Validated architectural advantages (do not regress these)

1. **Scrollback-native rendering.** The #1 cross-tool pain theme is
   flicker/redraw/alt-screen damage: Claude Code shipped a differential
   renderer + NO_FLICKER + alt-screen opt-out; Gemini rolled back its
   alt-screen default in under a week; Codex skips alt-screen inside
   Zellij. We are immune by construction — printed history belongs to the
   terminal. Any feature that requires erasing/re-rendering history stays
   out of core.
2. **Append-only ledger + projection.** The #4 pain (resume silently
   losing context; compaction amnesia) cannot happen structurally here:
   `derive_messages()` rebuilds exactly what the model saw, compaction is
   an entry, branches are entries. This is a marketing-grade differentiator.
3. **Steering wired through the loop** (`run_turn_with` drains queues;
   model-visible ⇒ logged). Competents ship queueing bugs constantly
   (premature drains, drops-on-interrupt, mid-approval firing).
4. **Live token/cost visibility** in the status row — ahead of Claude
   Code's default (`/cost` returns nothing on subscriptions).
5. CJK width table + errors-as-values tool results.

## Pain ranking found in the wild → our exposure

| rank | pain (evidence signal) | our exposure |
|---|---|---|
| 1 | flicker/alt-screen damage (whole ecosystems exist to patch it) | immune (inline design) |
| 2 | mid-run message queueing bugs (drops, premature drain) | low — steering works; missing *queue* semantics + UI |
| 3 | approval fatigue: "always allow" doesn't persist; 93% approve-rate makes prompts noise | medium — `a` is per-tool-name, process-lifetime only; no pattern scope |
| 4 | resume loses context; post-compaction amnesia | structurally impossible; UX must *show* it |
| 5 | invisible token burn (users built ccusage/ccburn because CLIs hide spend) | covered; extend with cache/rate-limit data when providers expose it |
| 6 | paste/multiline/IME breakage | low (bracketed paste + width table); large-paste summary missing |
| 7 | stuck-state ambiguity (spinner says alive, work wedged) | partial — retry events render; no staleness heartbeat |
| 8 | discoverability (hidden shortcuts, wrong labels like "(esc to interrupt)" while queued) | medium — table-driven /help exists; no contextual hint bar |

## Feature gap matrix vs leaders

| capability | OpenCode | Codex | us |
|---|---|---|---|
| @file attaches content | ✅ | ✅ (@ + Tab) | ❌ path-complete only |
| `!` local shell passthrough | ✅ | ✅ | ❌ |
| steer-vs-queue split mid-run | partial | ✅ Enter/Tab | steering only |
| double-Esc transcript walk-back + fork | ❌ | ✅ | ❌ (branch machinery exists unused) |
| external `$EDITOR` handoff | ✅ | ✅ Ctrl-G | ❌ |
| model picker dialog + favorites/recents | ✅ | ✅ | ❌ arg-only `/model` |
| pattern-scoped persistent allow rules | ✅ | ✅ granular policies | ✅ `[p]` learned rules → `.vakcoder/permissions.local.toml` |
| doom-loop guard (repeat-call asks) | ✅ 3× rule | policy-based | ✅ 3× identical call re-routes through approval |
| collapsible tool-output detail | ✅ /details | undocumented | fixed last-6-lines |
| thinking visibility toggle | ✅ | ✅ | one-line indicator only |
| markdown conceal toggle | ✅ | n/a | ❌ |
| theme packs + live preview | ✅ 11+custom | ✅ .tmTheme | ✅ 5 designed palettes + plain, live preview + project persistence |
| keybind remapping | ✅ full | ✅ contexts | hardcoded keys.rs |
| vim modal editing | partial | ✅ /vim | ❌ |
| composer undo/redo | ✅ | ✅ | ❌ |
| paste summary (large pastes) | ✅ | burst-detect | ❌ prints raw |
| command palette | ✅ Ctrl-P | ✅ | ❌ |
| session fork | slot exists | ✅ /fork | ❌ (ledger supports it) |
| transcript export md | ✅ | ✅ | ❌ |
| subagent tree navigation | ✅ drill-in | ✅ /agent | linear stream only |
| unfocused notifications | ✅ sound+desktop | ✅ OSC9/bel | bell on finish only |
| git-backed undo of file changes | ✅ /undo | manual | checkpoints + /rewind (stronger: harness-level) |
| raw/native-selection mode | n/a | ✅ /raw | always-native ✓ |

## Roadmap (each slice ships independently, CI-gated)

### Phase A — Interaction core (closes ranks 2, 3, 8)

Status: **A1, A2, A3, A6 (guard), A7 shipped**; A4 walk-back deferred to
Phase D; A5 picker deferred to the palette work (no provider model
catalog exists — history-backed list planned).

- **A1 Steer vs queue — SHIPPED**: during a run, Enter = steer current
  run (status row shows `steer: …`); Tab queues a follow-up for after
  completion (`⏳n queued` chip). Esc pops the newest queue item before
  cancelling. Queued prompts auto-submit only after non-aborted runs.
- **A2 @file attach — SHIPPED**: whitespace `@path` tokens resolve
  against cwd (trailing punctuation trimmed, ≤256KB each, ≤8 per prompt);
  contents appended as fenced attachments to the submitted message —
  model-visible ⇒ logged holds because the expansion IS the user entry.
  Unresolvable mentions are reported and left literal (`mentions.rs`,
  unit-tested).
- **A3 `!` shell passthrough — SHIPPED**: `!cmd` runs locally through the
  production BashTool; tail rendered inline; full output appended as a
  `[! shell]` user context entry so the next turn sees it.
- **A4 Truthful Esc semantics — PARTIAL**: queue-pop-before-cancel shipped;
  double-Esc transcript walk-back → edit-and-fork moves to Phase D
  (session branching machinery already proven in vak-session tests).
- **A5 Model picker** — deferred: no model catalog API in vak-llm;
  `/model <name>` remains arg-driven until the palette lands.
- **A6 Doom-loop guard — SHIPPED**: the 3rd identical (tool, args) call
  within one run re-routes from Allow through the approver with reason
  "identical X call repeated ×3" (`DOOM_LOOP_THRESHOLD`, tests in
  `vak-agent/tests/doom_loop.rs`). Pattern-scoped learned rules (`[p]`)
  pre-dated this plan.
- **A7 Hint bar — SHIPPED (status-row form)**: while running, the status
  line ends with `enter steer · tab queue · esc stop`; truthful labels
  replace the old mislabeled `[queued]` steering preview.

### Phase B — Input polish
- **B1 Composer undo/redo — SHIPPED**: Ctrl-Z / Alt-Z with coalesced edit
  groups and cursor-preserving snapshots.
- **B2 Large-paste summary**: >N chars/lines collapses to `[pasted k
  lines — Tab to expand]` placeholder in-editor; full text submitted.
- **B3 `$EDITOR` handoff**: Ctrl-G empties composer into temp md file,
  blocks on exit, reloads buffer (document Windows `--wait` caveat).
- **B4 Readline editing — PARTIAL**: Alt-D, line-aware Home/End, Ctrl-A/E,
  Ctrl-K/H/L, and Unicode-safe undo shipped; transpose remains.

### Phase C — Rendering depth (within scrollback constraints)
- **C1 Collapsible tool output — SHIPPED**: default tail-N preview + `/details`
  toggle raising the cap for subsequent calls (re-printing history stays
  forbidden; collapse applies to new output only).
- **C2 Thinking display modes — SHIPPED**: off / one-line / full blocks,
  cycled by Ctrl-T for the active terminal session.
- **C3 Theme packs**: port tokyonight, gruvbox, catppuccin, nord as
  built-in palette tables; `/theme <name>` previews live on next render;
  keep `plain` as ANSI-safe fallback.
- **C4 Width-adaptive diffs**: stacked single-column layout under narrow
  terminals (mirrors opencode `diff_style: auto`).

### Phase D — Navigation & sessions
- **D1 `/fork [n]`**: branch active session at message n (ledger
  `parent_id` machinery already proven in tests).
- **D2 `/export [path]`**: markdown dump via `derive_messages()`.
- **D3 Subagent tree view**: indented ◆ tree with per-child cost/status,
  navigable listing replacing the linear-only stream summary.
- **D4 Command palette — PARTIAL**: Ctrl-P fuzzy command picker with keyboard
  navigation shipped; recent-session results remain.
- **D5 `[ui.keybinds]` remap table**: named actions → key strings,
  unknown names warn-not-fatal (config contract).

### Phase E — Ambient awareness
- **E1 Staleness heartbeat — SHIPPED**: status row shows "no events 12s"
  once elapsed > threshold without any event; distinguishes wedged from
  working (rank-7 pain).
- **E2 Unfocused notification**: OSC9 (supported terminals) / bel fallback
  on approval-needed and run-finished; config-gated `[ui] notify`.
- **E3 `/usage`**: session totals + provider rate-limit headers where
  exposed (vak-llm already parses Retry-After; extend header capture).
- **E4 `/doctor` extensions**: terminal capability report (truecolor,
  bracketed paste, keyboard protocol), keybind-conflict scan.

## Non-goals

Alternate-screen full-screen mode, mouse-capture scrolling (conflicts
with native selection — our differentiator), cloud share links, terminal
pets.
