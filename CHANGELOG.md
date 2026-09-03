# Changelog

**Supported history begins at 2.0.0.** Every version before it is
unsupported and cannot be upgraded in place — see
`docs/design/46-stabilization-install-and-onboarding.md` Part VII.1. Entries
for those releases were removed from this file; `git log` holds them.

## 2.0.0 — 2026-09-03

The runtime was finished before anyone outside the project could install it.
2.0.0 closes exactly that, and establishes the contract that keeps every
later release non-destructive.

### Baseline

- 2.0.0 is the supported baseline. State written by an earlier version is
  refused with an explicit message and the one command that resolves it; it
  is never partially read and never migrated.
- The update feed carries no version below 2.0.0.
- `AGENTS.md` invariant 29 forbids accepting, migrating, or special-casing
  pre-baseline state, and invariant 30 requires one canonical way per
  capability.

### Admin console

- Rebuilt the console's first screen as **Home**. It answers four questions
  in order — is the system healthy, what is waiting on me, what is running,
  what is it costing — where the previous Overview showed four stat cards,
  three hardcoded attention tiles, and a key/value dump.
  - A **readiness ring** draws one arc per subsystem from that subsystem's
    own probe. A probe that did not answer reads `unknown` and is excluded
    from the healthy count instead of being counted as healthy; a subsystem
    switched off is excluded too and the count says so. State is carried by
    colour, stroke pattern, and a printed word, so the ring stays readable
    without colour vision.
  - An **attention queue** merges every blocking, asking, or drifting signal
    the product already knew about but never surfaced on the first screen:
    failed doctor checks, open incidents, dead-lettered and queued
    deliveries, chats knocking at the allowlist, a stopped gateway unit,
    drifted bindings, overdue scheduled jobs, provider errors and rate
    limits, skill proposals, unread inbox, and unfinished setup steps —
    each with its own repair as the row's detail. Incidents whose
    fingerprint Home already states in its own words are dropped, so one
    stuck outbox no longer reads as three separate problems. An empty queue
    names how many probes produced it and when, rather than rendering a
    decorative green tick.
  - **Approval gates** are answered on Home rather than linked to; a gate is
    a run that has already stopped.
  - **Pulse** reads a new 30-minute ring buffer of hub arrivals kept apart
    from the 60-item display feed, so the event rate no longer pins itself
    exactly when the system gets busy. The rate names the window it covers
    and never divides by less than a minute.
  - **Spend today** adds burn rate, when the cap lands at that rate, top
    model and provider, a 14-day trend, budget alerts, and the count of
    dispatches carrying no price — which makes the headline figure a floor,
    and says so.
- Extracted the console's shared presentation vocabulary into
  `crates/vak-admin-ui/src/display.tsx`: provider labels, permission-mode
  wording, security-event kinds, hub-event labels, path truncation, and the
  chat-surface catalog. One spelling of each, imported by every view.

### Permissions and capability reach

- **Configured capability is now reconciled against reachable capability.**
  The system prompt advertised what configuration declared while dispatch
  enforced what the composed policy permitted — permission mode, rules,
  channel overlay, approval mode, and the hosting surface's approver — and
  nothing compared the two. A chat-gateway turn was told it had an MCP
  server, spent four tool calls discovering that every call to it was
  refused by an approver that was never going to answer, and reported the
  capability as simply missing. `vak_core::reach` runs the real permission
  engine against each advertised capability and returns its standing;
  unreachable capabilities leave the capability set and the tool registry,
  and the prompt names them, says why, and gives the operator the fix. It
  is strictly subtractive: it never grants, and unattended surfaces still
  fail closed (invariant 15).
- **A channel overlay can no longer escalate.** `tools_allow` and
  `mcp_allow` were compiled into blanket `+` allow rules at two separate
  call sites, so `tools_allow = ["bash"]` — written to *narrow* a chat to
  one tool — handed that chat unattended shell execution with its approval
  gate removed, and `mcp_allow` did the same for MCP. Overlays are
  visibility narrowings, enforced by the registry filter and the MCP glob;
  the single shared translation now emits restrictive rules only
  (invariant 20).
- **`approval_mode = "auto-approve"` now reaches `webfetch` and `browse`.**
  Their restricted-mode gate was injected as a synthetic `?webfetch` rule,
  indistinguishable from one an operator typed, and `auto_approve`
  correctly refuses rule-sourced asks — so auto-approve silently worked for
  every tool except those two. The classification moved into
  `PermissionEngine`'s mode arms, where a mode default is sourced as a mode
  default. A deliberate `?webfetch` still outranks auto-approve.
- Collapsed `build_engine_for_mode` into `build_engine_with`. With the
  injection gone they were identical, and two constructors for the object
  that decides access is how these layers drifted apart.
- An approver declares whether a gate reaches anyone (`Approver::
  answerable`). A refusal from an unattended surface no longer reports
  itself as "denied by user" when no user was asked.
- `doctor` gained a **capability reach** check, and a blocked capability
  records a `capability_unreachable` security event — previously the only
  evidence was a denied tool call inside a session transcript.

### Universal presentation and output engineering

- **Universal recipe catalog and tool-provenance signal engine.**
  Expanded built-in presentation recipes across common life and work domains:
  `research.synthesis`, `coding.diff_inspector`, `coding.change_summary`,
  `coding.test_report`, `terminal.session`, `data.multi_chart`,
  `data.spreadsheet_grid`, and `lifestyle.culinary_recipe`. Added
  `SignalContext` and `signals_from_context()`, allowing tool names (`bash`,
  `write`, `edit`, `websearch`), CLI commands, exit codes, and output patterns
  to drive presentation recipe selection without manual user tagging.
- **Desktop presentation canvas suite.** Added 7 native, outcome-first
  renderers embedded in the continuous chat stream without external navigation:
  - `ResearchCards`: numbered takeaway rows, superscript citation chips with
    hover popovers displaying quoted snippets and source tags, and verified
    source link tiles.
  - `DiffInspector`: multi-file drawer with additions/deletions counts,
    unified vs. side-by-side mode toggle, line gutter numbering, and
    one-click host editor opening.
  - `TestMatrix`: SVG circular pass-rate ring, filter chips (`All` vs `Failed
    Only`), and collapsible traceback drawers.
  - `UniversalChart`: KPI metric pods with delta trends, multi-series SVG
    curves with area gradients, live mouse-tracking crosshair line with data
    bubble, and one-click CSV export.
  - `DataGrid`: interactive table with numeric-aware column sorting, real-time
    search filtering, and CSV export. Standard markdown tables with $\ge 3$
    rows automatically promote to this grid.
  - `TerminalConsole`: dark terminal container with command prompt, exit code
    badge, execution duration, and formatted monospace output.
  - `RecipeCard`: dynamic servings stepper (`-` 2 `+`) that recalculates
    ingredient measurements, paired with live countdown step timers.
- **Theme tokens and responsive layout.** Normalized `:root` color tokens
  (`--emerald-bright`, `--rose-bright`, `--text-main`, `--text-muted`) to
  dynamically cascade with theme changes, and added responsive stacking for
  compact split panes and mobile viewports.

### Documentation

- Deleted eight design documents that described pre-baseline behavior or
  recorded provenance rather than contract: research notes, achievements,
  the vakyartha adoption study, the Tavily integration doc, the first-run
  onboarding and distribution proposals, the competitive landscape, and the
  unimplemented self-evolving-agent proposal.
- Added `docs/design/46-stabilization-install-and-onboarding.md`: bundling,
  release, install, first-run onboarding, the configuration and inheritance
  contract, and the forward-compatibility contract.
- Rewrote `docs/design/00-roadmap.md` against the baseline; it states what
  is true now and what is planned, not what was shipped.
- Repointed every code and doc citation of the deleted study at the document
  that actually owns each contract.
- `README.md` documents the canonical secret path correctly:
  `~/vak-home/.env`, not `data_home()/.env`.
