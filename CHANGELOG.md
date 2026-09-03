# Changelog

**Supported history begins at 2.0.0.** Every version before it is
unsupported and cannot be upgraded in place — see
`docs/design/46-stabilization-install-and-onboarding.md` Part VII.1. Entries
for those releases were removed from this file; `git log` holds them.

## Unreleased

The commitment kernel: vak learns what it was asked, and what "done" means.

### The intent kernel (`docs/design/47-commitment-kernel.md`)

vak decided a great deal before a turn ran — provider, permission mode,
capability packet, budget — and made every one of those decisions without any
model of what the user was trying to do. Three consequences, all now fixed:

- **The only intent classifier was a keyword hack.** `is_managed_work_request`
  looked for one of thirteen English verbs plus two conjunctions, so "explain
  what this and that mean" read as durable multi-step work. It is **deleted**;
  managed admission now follows from the reading's `horizon` axis. An explicit
  run-scoped `work_mode` still overrides it.
- **Demand-driven routing was wired but fed zeros.** `plan_route_ladder`
  passed `reasoning_required: false, evidence_required: false,
  structured_output: false, estimated_input_tokens: 0`, so every session
  scored identical demand and `order_ladder_v2` never actually varied. Those
  facts now come from the turn's reading.
- **Every tool was advertised on every turn.** A greeting carried the whole
  toolbox. Capability slicing narrows the admitted packet to what the reading
  plausibly needs — `vak exec "hi"` now sees zero tools and one ladder leg.

New `crates/vak-intent`: seven behavioural axes (`act`, `horizon`, `stakes`,
`evidence`, `clarity`, `modality`, `attendance`), deterministic signal
extraction, a cheap-first resolution cascade, the autonomy/envelope model, and
a narrowing lattice. No dependencies on other vak crates — a pure decision
layer, testable without a network, a model, or a config file.

- **It only ever narrows** (`AGENTS.md` invariant 31). `Limits` is a meet
  semilattice whose top element reproduces the previous behaviour exactly;
  `meet` is the only composition operator and there is deliberately no `join`.
  A property test proves no engagement derived from any reading in the
  reachable space widens the baseline.
- **Intent never gates the permission engine.** It supplies a *ceiling* on
  approval permissiveness, so an irreversible turn reaches a human even under
  `auto-approve` — and nothing it concludes can skip a gate the operator
  wanted. A resolution bug can make vak more cautious; it cannot authorize.
- **Uncertainty resolves to the general engagement**, byte-for-byte the
  behaviour before the kernel existed. Being unsure never removes a tool.
- Confidence is **per-axis**: capability slicing gates on `act`, commitment
  promotion on `horizon`. A single scalar let an unsignalled horizon suppress
  slicing the act reading was certain about.
- Ordered axes (`stakes`, `horizon`, `evidence`) take the highest supported
  level rather than an argmax over rivals. A lower level *corroborates* a
  higher one; scoring them as competitors made a dirty working tree's
  `reversible` vote argue against a request's own `irreversible`.

### Durable commitments

New `crates/vak-commit`: work outliving a session becomes a commitment in its
own append-only ledger, rather than the session being the unit of identity and
"the model stopped talking" being the completion signal.

- **The satisfaction lattice** — `asserted < cited < observed < attested`.
  Strength comes from *how* a criterion was established: a command the runtime
  ran is `observed`, an external receipt is `attested`, the model's own
  judgement is `asserted` however emphatically phrased.
- **The closure invariant** (`AGENTS.md` invariant 32). A commitment cannot
  close `fulfilled` below the strength its `evidence` axis demands, and the
  ledger refuses the event at append time. Failure verdicts are deliberately
  unconstrained, so the record can always tell the truth about work that went
  wrong — including an honest `unknown`.
- **Waiting is not failing.** Unattended durable work that needs a human
  suspends on a `Human` wake condition and queues the question, where a
  one-shot turn still fails closed. Every deferred question carries an
  escalation policy, and `assume-conservative` is refused above `costly`.
- **Progress versus motion.** Episodes end with `advanced`, `learned`,
  `blocked`, or `stalled`; consecutive stalls trip a breaker. `learned` exists
  so genuine exploration is not punished.
- **Lifetime economics.** Budget exhaustion *holds* a commitment rather than
  failing it, and expiry produces an explicit `expired` verdict — never a
  silent deletion. Supersession records lineage instead of orphaning work.
- A deterministic, inspectable portfolio scheduler: every priority decomposes
  into named components, and a user pin dominates all of them.

### Surfaces

- `vak intent explain "<prompt>"` — the reading, every signal with the weight
  it carried, the engagement diff against doing nothing, and the exact text
  the model would additionally be told. Costs nothing, dispatches nothing.
- `vak intent show`, and `vak commit list|show|close|supersede|attest`.
- `GET /intent/explain`, `GET /intent/policy`, `GET /commitments`,
  `GET /commitments/{id}`, `POST /commitments/{id}/close` (409 on a refused
  closure — the request was fine; the evidence does not support the claim).
- New session entry type `intent`, carrying the exact model-visible text so a
  replay reproduces the prompt rather than re-deriving it (invariant 1). Only
  the newest applies, emitted immediately before the turn it governs.
- `[intent]` and `[commitment]` config. `intent.autonomy` and
  `intent.escalate = "cloud"` are privileged and stripped for an untrusted
  project: a cloned repository must not grant itself the right to act without
  asking, nor spend the user's credentials classifying.

### Fixed

- `Economics::default()` produced `stall_limit: 0`, so every commitment was
  born already stalled — `#[serde(default = "…")]` does not feed
  `Default::default()`.
- `vak intent explain --surface cron` derived attendance from the calling
  process's surface rather than the one being asked about, reporting an
  unattended cron run as interactive.
- The verb lexicon matched only bare stems, so "before deploying to
  production" contributed no act signal at all.

### Episodes and surfaces

- A durable turn now opens a commitment, brackets an **episode** around the
  work, and records what that episode achieved. `Learned` and `Stalled` are
  deliberately different: a turn that answered substantively but moved no
  criterion reduced uncertainty and must not count against the stall breaker,
  while exhausting the turn budget is the textbook motion-without-progress
  case the breaker exists to catch.
- A weak `horizon` reading opens nothing. A stray recurrence-ish word must not
  leave a month-long obligation behind.
- Seeded criteria are never stronger than `asserted`. The runtime may only
  propose what it could also check, and guessing a test command would
  manufacture `observed` evidence out of a guess — so a commitment held to
  `verified` stays visibly open until a checkable criterion or a human
  attestation arrives, rather than closing itself on a placeholder.
- **Admin console**: the commitment portfolio at `#/commitments`, built as a
  ledger of rows rather than cards. Its signature element is the evidence
  meter — the satisfaction lattice drawn, with the achieved level as fill and
  the required level as a rule beneath the track, the shortfall in the accent.
  Work closed on the model's own say-so and work closed on a check the runtime
  ran are not the same claim, and in an ordinary status column they look
  identical. Scheduler priority decomposes into its named components on click.
- **Desktop**: a composer intent strip, quiet in proportion to consequence.
  Chrome for ordinary work; only irreversible, deferred or must-ask readings
  take the accent and state the reason without needing a click.
- **Every surface**: a read-only `commitments` tool, so "what are you working
  on" is answerable on Telegram, the desktop and a cron check-in alike without
  a gateway slash-command layer that would serve one transport and add a
  second dispatch path. It has no write verb — the model may discuss a
  commitment and may never mark a criterion passed.

### Notes

- `AGENTS.md` invariants 31 and 32 are **appended**, not inserted. Roughly
  twenty code comments cite invariants by number; inserting in the middle
  would have silently invalidated every one of them.
- Phases I5–I8 (envelope wiring into live dispatch, episodes and the portfolio
  scheduler against the real scheduler, admin/desktop/gateway/delivery
  surfaces, and the misread evidence loop) are specified in doc 47 and not yet
  wired. The doc's `Status:` line says so.

## 2.0.1 — 2026-09-03

Universal output engineering, tool-provenance signal engine, and desktop presentation suite.

### Presentation and UI

- **Universal recipe catalog and tool-provenance signal engine:** Expanded built-in presentation recipes across common life and work domains (`research.synthesis`, `coding.diff_inspector`, `coding.change_summary`, `coding.test_report`, `terminal.session`, `data.multi_chart`, `data.spreadsheet_grid`, `lifestyle.culinary_recipe`). Introduced `SignalContext` and `signals_from_context()`, allowing tool names (`bash`, `write`, `edit`, `websearch`), CLI commands, exit codes, and output patterns to drive presentation recipe selection without manual user tagging.
- **Desktop presentation canvas suite:** Added 7 native, outcome-first renderers embedded in the continuous chat stream without external navigation (`ResearchCards`, `DiffInspector`, `TestMatrix`, `UniversalChart`, `DataGrid`, `TerminalConsole`, `RecipeCard`).
- **AST routing & promotion:** Wired `PresentationRenderer` to route diff blocks to `DiffInspector`, promote markdown tables with $\ge 3$ rows to interactive `DataGrid`, and dispatch specialized recipes.
- **Theme tokens and responsive layout:** Normalized `:root` color tokens (`--emerald-bright`, `--rose-bright`, `--text-main`, `--text-muted`) to dynamically cascade with theme changes, and added responsive stacking for compact split panes and mobile viewports.

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
