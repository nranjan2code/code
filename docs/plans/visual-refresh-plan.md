# Plan — visual refresh (Ink and Saffron)

Status: **plan and tracker, 2026-09-25. Decisions D1 to D5 locked; V4.1
completed by explicit request; V1 complete (V1.12 closed as obsolete); V2.1
complete; V3.1 to V3.5 done. V3.6 onward and V4.2 to V4.5 remain unstarted.**

- Design: `docs/design/75-visual-refresh.md` (findings, the system, the
  glossary, the disclosure setting).
- Visual reference: `docs/assets/visual-refresh-2026/visual-refresh.html`
  (before-and-after mockups and the review screenshots in `img/`).
- Anchors that stay: `docs/design/70-calm-agent-experience-implementation.md`
  (the four screens) and `docs/design/71-agent-character-system.md`.
- Baseline: the 4.0.2 dev build at `acca0c88`. Line numbers below were taken
  there and will drift; re-find each one before changing it.

This file is the tracker. Tick an item only when its "done when" holds in the
running app, and add a dated line to the progress log (§6) saying what was
checked and what was not. A passing build or typecheck is not done.

## 1. Decisions

Locked by the maintainer on 2026-09-25.

| # | Decision | Consequence |
|---|---|---|
| D1 | **Brand: Ink and Saffron.** Indigo ink; the mark's saffron (`#F5A400`) as the one accent, for live states only; lighter paper; indigo-night dark theme. | Terracotta and sage green retire from the client, `DESIGN.md` and `docs/brand/README.md`. |
| D2 | **Display face: Newsreader**, bundled as WOFF2. | Only for the wordmark, agent names, page titles, the greeting and result headlines. |
| D3 | **Technical details off for new installs**, offered once in onboarding and kept as a switch in General. | One disclosure setting replaces Transcript detail (`Density`). |
| D4 | **Four theme choices:** Match system, Light, Dark, High contrast. | Warm dark, Sage, Paper, Mist and Dawn retire; Midnight's slot becomes the new Dark. A stored retired id resolves to Match system, with no migration. |
| D5 | **Order: V1, V2, then V3, then V4.** | V1 items may ship one at a time. No V3 screen starts before the V2 tokens merge. |

## 2. Rules for every stage

1. **Presentation only.** Nothing here changes permissions, authority,
   approvals, the ledger or what a model sees (invariants 1, 13, 16). The
   disclosure setting hides detail, never a safety state: pending approvals,
   failures and denials stay visible in Everyday.
2. **Replace, never layer** (invariant 30). A change deletes the tokens, CSS
   overrides, strings and components it supersedes in the same commit. No new
   rule may override an earlier rule for the same selector later in
   `styles.css`; edit the original.
3. **Verify live.** Each item's "done when" is checked in a running dev build
   (`docs/development.md`) in a browser at 1440 × 900 and 390 × 844, in light
   and dark. Save the proof under `docs/assets/visual-refresh-2026/after/`
   with the stage id in the file name, and say in the progress log what was
   and was not checked.
4. **Rebuild the bundles.** After any client change run the client build
   (both bundles), rebuild the server, and commit `dist-web`
   (AGENTS.md, "Touched a frontend").
5. **Standard checks** before every commit: the AGENTS.md verification
   commands, plus `node` over `crates/vak-client-ui/tests/*.mjs`.
6. **Accessibility holds.** Colour is never the only carrier; every text pair
   clears WCAG AA (compute it); focus stays visible; reduced motion works; no
   horizontal scroll at 390px.

## 3. Exit criteria for the whole refresh

1. No `font-size` below 12px anywhere in the client stylesheet, and
   conversation text at 16px at the default text size.
2. Hex literals only in theme token blocks and theme previews.
3. No selector defined twice outside media queries (baseline: 144).
4. No user-facing string contains a term on the banned list (§4, V2.5); the CI
   check enforces it.
5. A fresh install, in Everyday, shows no folder path, ID, byte count, hash,
   MIME type or sandbox label on the conversation, first-run and Settings
   screens.
6. Both themes and High contrast pass the contrast check; the brand README,
   `DESIGN.md` and doc 70 describe the shipped system.
7. The four anchor screens of doc 70 and the three screens of doc 75 match
   their references in the running app, with screenshots saved.

## 4. Stages and checklist

### V0 — Decisions

- [x] V0.1 Review of the 4.0.2 dev build, with screenshots and measurements
  (doc 75 §2 and §3).
- [x] V0.2 D1 to D5 locked (2026-09-25).

### V1 — Fix what is broken (small, independent)

Paths are relative to `crates/vak-client-ui/src` unless they start with
`crates/`.

- [x] **V1.1 New users never see the welcome.** Opening an agent creates its
  conversation, so the empty state takes the "has session" branch ("Nothing
  here yet. Ask a follow-up or open its details to inspect prior work.").
  Where: `components/ChatPane.tsx`, empty state. Fix: show the welcome
  whenever the conversation has no turns.
  Done when: a fresh home (empty `HOME` and `VAK_HOME`) shows the greeting
  and the four starters; a conversation with turns never shows them.
- [x] **V1.2 "Opening agent…" floats mid-sidebar** for about 3.5 s on load
  and 5.5 s on a switch (debug build).
  Where: `components/Sidebar.tsx` (the `agentOpening` status line),
  `styles.css` `.sb-empty { margin: auto 0 }`. Fix: a spinner on the chosen
  row, a skeleton in the conversation, and the cached conversation shown at
  once.
  Done when: no loading text appears in the sidebar, and a switch shows the
  target agent's last conversation without a blank interval.
- [x] **V1.3 A Reconnecting bar shows on every load** for about 3.4 s,
  before the first connection, and its tooltip says the stream dropped.
  Where: `components/StatusBar.tsx` and the connection state in `store.ts`.
  Fix: a silent `connecting` state; later drops show as a small pill above
  the message box after a grace period.
  Done when: a normal load shows no connection UI; stopping the server shows
  the pill within the grace period, and restarting it clears the pill.
- [x] **V1.4 Share dialog.** The close button sits under the description on
  the left, and the dialog pins to the conversation's top-left corner.
  Where: `components/CoworkingShare.tsx`. Fix: close at top right, centred.
  Done when: Escape closes it and focus returns to Share, at both widths.
- [x] **V1.5 Review overlap.** "Open saved version in Canvas" overlaps the
  "187 B · draft hash" line. Where: `components/WorkbenchPanel.tsx`
  (candidate review). Fix: stack the button under the caption.
  Done when: no overlap at either width.
- [x] **V1.6 Letter icon.** Background tasks shows a letter W.
  Where: `components/WorkspaceHeader.tsx` (`menu-letter`). Fix: a real icon.
- [x] **V1.7 Phone message box.** At 390px the send button covers Attach.
  Where: `components/Composer.tsx` toolbar. Fix: +, mic and send, with the
  rest in the + menu.
  Done when: nothing overlaps at 390 × 844 and every control is reachable
  by keyboard.
- [x] **V1.8 Empty-state mark.** A ◌ character in a pink tile with a chat
  icon hanging below it. Where: `components/ChatPane.tsx`,
  `styles.css` `.vak-companion`. Fix: the agent's portrait.
- [x] **V1.9 Design-doc tooltip.** The goal chip's tooltip cites
  "docs/design/27 Phase H". Where: `components/Composer.tsx`. Fix: plain
  words.
- [x] **V1.10 Setup leaves the app.** Finish setup opens the admin console's
  Setup & Readiness page. Where: `components/SetupBanner.tsx`. Fix: plain
  copy ("Connect an AI service to start. It takes about a minute.") and open
  the client's own Settings credentials section; the full connect sheet is
  V3.4.
  Done when: on a fresh home the banner opens Settings at the credentials
  group, in the same window.
  *Revised when built:* opening Settings would have been a second setup path
  beside the one wizard (`docs/design/46-stabilization-install-and-onboarding.md`
  D7, invariant 30). The banner now uses plain words for the provider step,
  keyed by step id, and "Connect" still opens the one wizard. Moving that
  wizard into the client is V3.4, and it must replace the admin wizard, not
  join it.
- [x] **V1.11 Ready with no AI service.** The header says Ready on first run.
  Where: `components/WorkspaceHeader.tsx` (`taskStatus`). Fix: "Needs an AI
  service" from the setup status the client already reads.
  Done when: a fresh home shows it; a configured home shows no status at
  rest.
- [x] **V1.12 Revision requests show as the user's message**, with raw IDs.
  *Closed without a change:* the current server runs a revision in an
  isolated child run and records a `CandidateRevision` activity in the
  conversation, not a user message; the revision text reaches only the
  child's prompt (`dispatch_candidate_revision`), and no other code writes
  it. The bubble in the review is a historical ledger entry from an earlier
  revision path; append-only ledgers keep it. Checked in
  source, not reproduced live. The original analysis follows.
  Where: the text is built in `crates/vak-server/src/lib.rs` (the candidate
  revision route, "Revise candidate … Owner selected comment …"), rendered
  by `components/ChatPane.tsx`. Fix: a typed kind on the message
  (`MessageMeta`, additive per invariant 29) that the chat renders as a
  revision event with the file, line, author and quote. It is not a
  `ControlKind`: those are runtime nudges that clients never see, and this
  request must stay visible. The model-visible text stays as it is; nothing
  matches on text.
  Done when: a live revision from a coworker's comment shows as an event, the
  ledger entry is unchanged apart from the new tag, and replay shows the same
  event.
- [x] **V1.13 Settings names disagree.** Services opens a page titled
  Operations; Integrations opens Capabilities. Where:
  `components/Settings.tsx`, `components/OperationsPanel.tsx`. Fix: one name
  per page.
- [x] **V1.14 Permission icons.** Full access, the riskiest option, uses a
  shield; Workspace write and Approve automatically use a code icon. Where:
  `components/Settings.tsx` (permission and approval option lists). Fix:
  eye, pencil and warning.
- [x] **V1.15 Agent picker badges.** A filled CURRENT badge and monospace
  IDs. Where: `components/AgentPickerModal.tsx`. Fix: a check mark; IDs move
  under Technical details once V2.6 exists, and are dropped from the row now.

### V2 — Foundation

- [x] **V2.1 `DESIGN.md` rewrite.** The new north star (working name: the
  Good Listener), the rules kept from today (doc 75 §5.8), the changes (doc
  75 §5.9), and the Ink and Saffron tokens. The Auditor's Desk sections go.
- [x] **V2.2 Token layer.** Light, Dark and High contrast as Ink and Saffron
  (doc 75 §5.1); the type scale, spacing, radius and motion as tokens. The old
  `:root` palette, the seven `data-theme` blocks and the Settings theme grid
  are replaced in the same change; the five retired palettes' previews go.
  Done when: exit criteria 2 holds for the token layer, and every component
  still renders in all three themes.
- [x] **V2.3 Newsreader.** WOFF2 files for Latin and Latin Extended in the
  client, loaded with `font-display: swap` and a serif fallback; licence file
  alongside. Done when: the desktop shell and the web app both render it with
  no network access.
- [x] **V2.4 Type scale.** Every `font-size` moves to the nine steps; three
  weights; conversation text at 16px; answers wrap at about 68 characters;
  the text-size preference still scales everything.
  Done when: exit criterion 1 holds and the conversation screen's measured
  size distribution is saved in the progress log.
- [x] **V2.5 Plain words.** Apply the glossary (doc 75 §7) and the voice
  rules (doc 75 §5.7) to every user-facing string; add a check to the client
  tests that fails on banned terms (sandbox, ledger, receipt, frozen,
  candidate, execution, provenance, manifest, verifier, persona, fleet,
  daemon, MCP) outside technical-detail views.
- [x] **V2.6 Disclosure setting.** Show technical details replaces
  `Density` (`store.ts`) and the Transcript detail row in Settings. Everyday
  hides what doc 75 §8 lists; onboarding asks once (D3). The setting uses a
  new key and starts off; the old `vak.density` value is not read.
  Done when: every row of doc 75 §8 is checked in both states, and a pending
  approval shows the same in both.
- [x] **V2.7 Icons.** Choose Lucide or Phosphor, vendor it (pinned, with its
  licence), replace `components/Icon.tsx`'s set and every letter glyph.
- [x] **V2.8 Delete what is superseded.** The 21 dead `.everyday-rail`
  rules, every duplicate selector (exit criterion 3), the `!important` rules
  that only existed to win against a later override, and unused hex
  literals.

#### Handoff for V2.5 to V2.8 (written 2026-09-25, at `9d945101`)

V2.1 to V2.4 are done. Start a fresh session here; re-find each line first.

1. **V2.5 words.** Apply doc 75 §7 string by string (grep each "Today"
   text in `crates/vak-client-ui/src`). The setup banner already has plain
   words (V1.10). For the CI check, add a node test in
   `crates/vak-client-ui/tests/` that scans JSX text and label/title/
   aria-label/placeholder strings for the banned terms, with an explicit
   allowlist of technical files (Workbench, Operations, Terminal, Diff,
   Receipts, Workers panels) and of individual strings that stay technical.
2. **V2.6 disclosure.** Replace `Density` (`store.ts`, key `vak.density`,
   used in `ChatPane.tsx` and the Settings "Transcript detail" row) with one
   boolean `technicalDetails` (new key, default off). Gate, per doc 75 §8:
   the header folder chip (`WorkspaceHeader.tsx`), the … menu Developer
   group, result card byte counts and paths, Review hashes and provenance,
   the message-box model and permission selects (`Composer.tsx`), and the
   technical Settings pages. Pending approvals and failures never hide.
3. **V2.7 icons.** Ask before downloading an icon set (Lucide ISC or
   Phosphor MIT); `components/Icon.tsx` holds about 45 hand-drawn paths.
4. **V2.8 cleanup.** Dead rules first (`.everyday-rail`, 21), then the
   duplicate selectors (the handoff baseline was 144), then `!important`.
   Known open item: long agent names truncate in the sidebar since V2.4
   (V3.1 fixes the row).

### V3 — Surfaces (after V2 merges)

- [x] **V3.1 Sidebar and header.** Complete tile and Newsreader wordmark;
  Search as a row; agents with their own character and a one-line status;
  status shown only when something is wrong; one footer menu.
- [x] **V3.2 Conversation.** Answers on the page, results as the only card,
  revision events (V1.12) styled, no YOU label, quiet working state.
- [x] **V3.3 Message box.** One field; + menu (attach, mention, skills,
  upload a recording); mic as the voice button, saffron while listening;
  model and permission switches in the + menu only with technical details
  on (doc 75 §8); "Full access" always in view.
- [x] **V3.4 First run.** The mascot greeting, starters with examples, and an
  in-app "Connect an AI service" sheet (local model if found, or an account
  key), replacing the admin-console hand-off.
  Done when: a fresh home reaches a first answer without leaving the app.
- [x] **V3.5 Settings.** Everyday, Agents and Advanced navigation (doc 75
  §6.3), outcome wording, one agent header, Technical details rows; on phones
  a list that opens each page.
- [ ] **V3.6 Review and Canvas.** Review as a sheet with the preview first;
  Canvas with device icons and plain labels.
- [ ] **V3.7 One sheet component** for every dialog: title, close at top
  right, focus trap, Escape, return focus.
- [ ] **V3.8 Agents.** Picker and creation wizard in plain words; every
  agent gets its own character by default. The one-time "Meet Vakyartha"
  dialog folds into the greeting instead of covering it on first run.
- [ ] **V3.9 Motion.** The durations and curve from doc 75 §5.4; message
  arrival, result reveal, skeletons; no hover movement.
- [ ] **V3.11 Finish the CSS cleanup inside each surface rewrite:** the 30
  duplicate selector groups whose merge would change the cascade, the 3
  remaining `!important` rules (`.good-chip`, `.prompt-page > header`,
  `.office-outline button`) and the 35 hex literals outside the tokens.
  Decide each as its screen is rebuilt.
- [ ] **V3.12 Plain words for a failed turn.** A message sent before an AI
  service is connected fails with the server's own text ("provider auth
  missing: set ANTHROPIC_API_KEY for provider 'anthropic'"). It should say
  that no AI service is connected and offer Connect. The failure needs a
  typed kind from the server, never matching on the text.
  Done when: a fresh home that sends before connecting sees one plain
  sentence and a Connect button.
- [ ] **V3.13 One frame per card.** The first answer on a fresh home (a
  small local model's entity card) shows a "Kind: entity" row and sits in a
  frame inside a frame. Everyday view drops the kind row and draws one
  frame.
  Done when: that answer shows one frame and no kind row, light and dark.
- [ ] **V3.10 Anchor check.** The four doc 70 screens and the three doc 75
  screens compared with their references in the running app; doc 70's
  ledger updated with the evidence.

#### Handoff for V3.5 (written 2026-09-25, after V3.4)

Settings is one 1,954-line component (`crates/vak-client-ui/src/components/Settings.tsx`)
and deserves a session of its own. What it has today, and where each part
goes (doc 75 §6.3):

| Today (`Page` id, nav group) | Holds | Goes to |
|---|---|---|
| `general` (Experience) | notifications, quiet hours, sound cues, suggested prompts, Show technical details, presentation styles, shortcuts, the Voice section, desktop working directory | Everyday: General; Notifications (alerts, quiet hours); Voice and sound (Voice section, sound cues) |
| `appearance` (Experience) | theme, layout and text, cards and previews | Everyday: Appearance |
| `permissions` (This agent) | approvals, isolation, rules | Everyday: Privacy and safety (outcome words: Look only · Edit files in this folder · Full access to this computer; Every time · Only outside this folder · Don't ask) |
| `learning` (This agent) | memory notes and skill proposals | Everyday: Privacy and safety (what Vakyartha remembers) |
| `archived` (Experience) | archived tasks | Everyday: Privacy and safety, or History; keep it reachable |
| `integrations` (This agent) | connections (MCP), skills, hooks, plugins | Everyday: Connections; hooks and plugins behind the Technical details row |
| `agent` (This agent) | provider, model, key, turn limit, freshness, helpers, chat bots | Agents: one page per agent (name once, model, one notice line); Chat bots to Connections; turn limit, freshness, helpers and context size behind a Technical details row, values intact |
| `prompts`, `reliability` | prompt layers; retries, breaker, route ladder | Advanced: Prompts; Reliability; Models and routing (route ladder) |
| `services`, `advanced` | services and health; paths, context, configuration | Advanced: Services and health; Storage and backup |

Keep:
- `TECHNICAL_PAGES` becomes the Advanced group, shown only with technical
  details on.
- The scope toggle (Shared defaults or This agent) applies only to Agent
  pages and Advanced; Everyday pages are local preferences.
- Invariant 21 and 27: a GET that seeds a PUT still reports only the layer
  that PUT writes.

Re-point the three deep links when the ids change: `BudgetBanner.tsx`
("services"), `VoiceControl.tsx` ("general" plus section "voice") and the
message box's Full access notice in `Composer.tsx` ("permissions").
`SettingsPageId` in `store.ts` is the one list of ids.

On phones, Settings is a list that opens each page, with a back row. Check
it at 390 as well as 1440, light and dark, like every other item.

### V4 — Brand assets and desktop chrome

- [x] **V4.1 Master export (approved logo correction).** Use the exact supplied
  `docs/brand/mark/vak-logo-master.png`; regenerate the desktop, Dock, browser,
  iOS, Android, tray, client, admin, favicon and website exports. Preserve the
  paper tile and navy/saffron artwork. The sole theme-aware exception is the
  macOS menu-bar alpha template.
  Done when: generated exports match the master; the Dock export has native
  padding; tray variants render correctly; and client, admin and website checks
  pass at 1440 × 900 and 390 × 844 in light and dark. Evidence: `after/` files
  named `V4.1-*`.
- [ ] **V4.2 Character glyphs.** A flat two-colour glyph per character for 32px
  and below, plus 64 and 128px WebP copies of the portraits; doc 71 updated.
- [ ] **V4.3 Desktop chrome.** Overlay title bar in
  `crates/vak-desktop/tauri.conf.json` (`titleBarStyle`, `hiddenTitle`,
  `trafficLightPosition`), drag regions on the sidebar head and header, and
  the sidebar padded for the traffic lights.
  Done when: checked in the running desktop app on macOS, including window
  drag, double-click to zoom and full screen.
- [ ] **V4.4 Brand README.** `docs/brand/README.md`: the Ink and Saffron
  palette, Newsreader, the campaign line as the product line, the
  transparent master.
- [ ] **V4.5 Close out.** Doc 75's status changed to shipped with evidence;
  doc 70's visual rules pointed at doc 75; the AGENTS.md "Pending: the visual
  refresh" section removed and the design-doc list updated (§5).

## 5. AGENTS.md and design-doc changes

| When | Change |
|---|---|
| Now | "Pending: the visual refresh" section added; doc 75 listed under Proposals. |
| V2.1 | `DESIGN.md` rewritten; AGENTS.md's pending section notes that `DESIGN.md` now describes the target. |
| V2.6 | Doc 57 notes that the disclosure setting is its Everyday/Advanced projection in the client. |
| V3.10 | Doc 70's ledger records the anchor-screen evidence. |
| V4.5 | Doc 75 marked shipped and moved into the "Experience direction" list; the pending section removed. |

## 6. Progress log

Newest last. One dated line per change: what landed, what was checked live,
what was not.

- 2026-09-25: Review of the 4.0.2 dev build (`acca0c88`) with real
  configuration and a fresh home; doc 75, the visual reference and this plan
  written. D1 to D5 locked by the maintainer. Nothing in the client changed.
- 2026-09-25: V4.1 completed by explicit maintainer request. Exact supplied
  raster kept unchanged; 63 platform exports regenerated; public app, admin and
  site branding set to Vakyartha while internal `vak` names remain. Screenshots
  cover client, admin and website at desktop/mobile sizes in light/dark; native
  macOS AppKit, Dock ICNS, and tray-template renders checked. Color/trademark
  uniqueness scan was a limited visual web search; no registry search or legal
  conclusion. Remaining V4 work was not started.
- 2026-09-25: V1 landed except V1.12. Built both client bundles and the
  server; checked live in headless Chrome and the browser pane on the 4.1.0
  dev build, against the real configuration (port 8933) and an empty home
  (port 8934). Checked: no status pill and no "Opening agent" text during
  load; the row spinner clears on switch (0.16 s and 2.3 s, debug build); a
  stopped server shows "Reconnecting…" about 1.5 s later and a restart
  clears it; Share and Background tasks are centred sheets with the close
  button beside the title, and Escape returns focus to Share; the Review
  caption sits 6px below its button; Background tasks has an icon; no
  overlap and no sideways scroll in the 390px message box (light and dark);
  the agent picker shows a check mark and no IDs; Services and Integrations
  titles match the navigation; permission icons are eye, pencil and
  warning; on the empty home, the welcome with four starters and the
  agent's portrait, the plain provider banner with Connect, and "Needs an
  AI service" in the header, with no status at rest on a configured home.
  Evidence: `after/V1.*`. Not checked live: the goal-chip tooltip (V1.9,
  source only); V1.12 not started. Client typecheck and node tests pass.
- 2026-09-25: V1.12 closed without a code change: the review's revision
  bubble is a historical ledger entry from an earlier revision path; today's
  path records a `CandidateRevision` activity and sends the text only to the
  isolated child (source check only).
- 2026-09-25: V2.1 done. `DESIGN.md` now describes the target system: the
  Good Listener north star, one system with everyday and operator
  densities, Ink and Saffron tokens for light and dark, Newsreader and the
  nine-step scale, the kept rules (colour never alone, contrast
  calibration, no second palette, grid areas, one breakpoint per layout)
  and the V1 behaviours (row spinner, connection pill, status only when
  something needs attention). Docs only; the stylesheet is unchanged.
  Next: V2.2 (token layer), in a fresh session.
- 2026-09-25: V2.3 done, with the maintainer's approval to download.
  Newsreader latin and latin-ext WOFF2 subsets (opsz 6-72, weights
  400-600; 132 KB and 87 KB) from Google Fonts are bundled in
  `crates/vak-client-ui/src/fonts/`, with the SIL OFL in
  `public/fonts/Newsreader-OFL.txt` so it ships in both bundles. New token
  `--display`; the wordmark (20px) and the greeting (28px) use it. Checked
  live on the web app: the face loads from `/app/assets/`, and the page
  makes no request to any other host. The desktop shell was not launched;
  its bundle carries the files and references them by relative URL
  (checked in `dist/`). Evidence: `after/V2.3-*`. `cargo test -p
  vak-server`: 33 binaries, 460 tests, all pass (this run also covers the
  V1 bundle; the V1 entry's count was taken from a truncated summary).
- 2026-09-25: V2.2 done. `:root` is now the Ink and Saffron light palette,
  `dark` is indigo night and `contrast` is rebuilt; the Warm dark, Sage,
  Paper, Mist and Dawn palettes, their previews and their selectors are
  gone, and a stored retired theme resolves to Match system. New tokens:
  `--live` (saffron) and its wash and ink, type sizes, spacing, radius
  (8/12/16), motion and `--shadow-float`. Running dots, voice capture and
  the approval card use saffron. 50 old-palette rgba literals became token
  mixes; the terminal and diagram colour maps follow indigo night. In dark
  the primary is `#A3ADF7` with dark text, so one token serves as fill and
  link (DESIGN.md and doc 75 updated). Computed contrast: every text token
  on every ground passes AA (lowest 4.73 light, 5.27 dark, 6.24 contrast).
  Checked live: each theme's ground, text, accent and live colour; the
  retired-id fallback; four theme choices; no sideways scroll at 390.
  Evidence: `after/V2.2-*`. Not done here: rules still use literal sizes
  (V2.4) and other literal hex values remain outside the tokens (V2.8).
- 2026-09-25: V2.4 done. Every px font size in the stylesheet (579) and in
  six components moved one step onto the nine-step scale; none is under
  12px. Weights are 400, 500 and 600 only; 25 capitalised labels and 22
  wide letter-spacings removed. Interface text is 15px, and conversation
  text and the message box are 16px with a 1.6 line height, still scaled
  by the text-size preference. Measured on the conversation screen at
  1440 × 900 (light), visible characters by rendered size: 12px 342, 13px
  351, 14px 14, 14.4px 75 (code), 15px 115, 16px 845, 18px 9; none under
  12px, also on Settings and at 390px dark (no sideways scroll). Before:
  44% under 13px and nothing over 15px. Known regression for V3.1: long
  agent names now truncate in the sidebar. Evidence: `after/V2.4-*`.
- 2026-09-25: V2.5 done. Everyday strings follow doc 75 §7: the agent
  picker ("Your agents", "Agents", "Folder"), result cards ("Draft · N
  bytes", from `vak-server` so every surface gets it; "Open", "Review
  changes"), the message box ("Upload a recording", "Folder:", permission
  names), Canvas ("Desktop", "Tablet", "Phone", "Safe preview"), Review
  ("Saved copy is intact", plain check messages, "Technical details"),
  Settings (permissions and approvals as outcomes, "Cards and previews",
  "Images from the web", "Check freshness", "Helpers", prompt-layer help),
  Services ("System health", "Where Vakyartha runs") and the Activity log
  menu item. `tests/plain-words.mjs` fails on sandbox, ledger, receipt,
  frozen, candidate, execution, provenance, manifest, verifier, persona,
  fleet, daemon or MCP in a user-facing string outside technical views
  (only Activity receipts today; search keywords are skipped). CI now runs
  every client node test, which it did not before. Checked live: each
  rewritten string on its screen; the message box menu's strings are
  checked in source (the menu was closed). `cargo test -p vak-server`: 460
  pass. Not done: the Settings navigation names wait for V3.5's structure.
- 2026-09-25: V2.6 done. `technicalDetails` (key `vak.technicalDetails`,
  off by default) replaces `Density` and the Transcript detail row; the old
  key is not read. Off hides the header folder chip, the … menu's Developer
  group, Compare approaches and Activity log, the message box's model and
  permission pickers, Review's technical details and hash, full file paths
  in Details, and the Services, Advanced, Prompts and Reliability pages; on
  shows them and full tool output. The Meet Vakyartha dialog offers it once
  ("I build software"), and General keeps the switch. Checked live in both
  states. Not checked live: the onboarding checkbox, tool output in a live
  run, and an approval (approval and error rendering are not gated in
  code). Result cards still show byte counts, because that text comes from
  the server.
- 2026-09-25: V2.8 done for everything that is provably neutral. Removed
  the dead `.everyday-rail` rules (21, including media and grouped uses) and
  merged 34 of 64 groups of identical top-level selectors into their last
  occurrence; the other 30 were skipped because a rule between them sets
  the same property on an overlapping selector, so merging would change the
  cascade (moved to V3.11). Reviewed all 27 `!important`: 24 override inline
  or library styles or enforce reduced motion; 3 go to V3.11. 35 hex
  literals remain outside the tokens (V3.11). `styles.css` is 6,165 lines.
  Checked by computed style: 30 properties of every rendered element on 7
  screens (3,472 elements: conversation light, dark and 390px, agent picker,
  Review, Settings General and Permissions) are identical before and after.
- 2026-09-25: V2.7 done, with the maintainer's approval to download. Lucide
  1.48.0 (pinned tag; ISC licence shipped at
  `public/licenses/lucide-LICENSE.txt`) replaces the hand-drawn icons: the 44
  shapes the app uses are copied into `components/Icon.tsx` under the app's
  own names, so no call site changed and no dependency was added. New names
  `at` and `slash` replace the "@" and "/" glyphs in the message box menu;
  the ✓, ✕ and × glyphs in the welcome, goal chip, reply target and
  Background tasks became icons. Checked live: every rendered icon draws
  (39 on the conversation, 122 on Settings). Evidence: `after/V2.7-*`. The
  PR panel's "◌" pending mark stays (a technical view).
- 2026-09-25: V3.1 done. Sidebar: the complete tile with a 22px Newsreader
  wordmark; a Search row (⌘K) that opens the global search instead of a
  hidden agent filter (the agent picker keeps its own search); agent rows
  with a 28px character, the full name (it wraps instead of truncating, the
  V2.4 regression) and one quiet line with what the agent is doing or its
  latest conversation; "New agent"; one footer menu (Keyboard shortcuts,
  Sign out) in place of two icon buttons. Header: the agent's character at
  28px and its name in Newsreader 22px, set on the original
  `.workspace-title h1` rule. Checked live at 1440 (light and dark) and 390
  (no sideways scroll). Evidence: `after/V3.1-*`. All three reviewed agents
  still share the bird; distinct characters by default are V3.8.
- 2026-09-25: V3.2 done (commit `b05e3bb5`). The answer sits on the page:
  the result surface lost its border, tint and padding, the caution became a
  rounded note, and the evidence and action rows lost their dividers; cards
  and file results keep their own styling, so the result is the only card.
  "You" shows only on messages someone else wrote. Checked live at 1440
  (light and dark) and 390. Evidence: `after/V3.2-*`. Test note: the first
  `cargo test -p vak-server` run failed once in
  `sandbox_promotion_tests::a_draft_changed_after_office_apply_is_offered_only_whole`
  (`SessionLog::open` returned an error under full-suite load) and the
  commit was pushed anyway because of a command-chaining mistake; the test
  then passed 3 of 3 in isolation and the full suite passed (460) with
  `--no-fail-fast`. The flake predates this work and is server-side.
- 2026-09-25: V3.3 done (commit `37efa996`). The message box is one field
  with three round buttons: + (Attach files, Upload a recording, Folder,
  Mention a file, Use a skill or command), the mic and send. The separate
  Attach button and the inline recording upload are gone; a recording
  chosen from + reaches the voice control through one `vak:voice-recording`
  event. Listening, processing and speaking take the saffron wash. Model and
  permission switches show in the + menu only with technical details on, as
  doc 75 §8's table says; §6.1 said "move to the agent menu", contradicting
  it, and now follows §8. A "Full access" notice stays beside + whenever the
  agent may act outside its folder, whatever the setting, and opens
  Settings at Permissions. Two pre-existing bugs fixed on the way: the
  toolbar's `overflow-x: auto` and the box's `overflow: hidden` clipped the
  + menu, so it never showed; and the web app never loaded `/health`
  (`loadHealth` returned early on the web's empty same-origin base), so the
  browser never knew the permission mode or model. The review workspace had
  been in Full access with auto-approve and the web app showed nothing.
  Checked live at 1440 (light and dark) and 390: the three buttons are 34px
  circles, the menu shows its five items, nothing overlaps, no sideways
  scroll; the notice is 13px at 5.6:1 (light) and 5.5:1 (dark). Evidence:
  `after/V3.3-*`. Seen for later: an older revision request still shows as a
  raw user message carrying a comment id (check under V3.6), and a small
  dash sits at the left edge of the phone conversation (check under V3.10).
- 2026-09-25: V3.4 done (commit `ad5e53b4`). An empty conversation greets
  in the mascot's voice ("Hi, I'm Vakyartha.", Newsreader 36px, 28px on a
  phone) and offers the four starters with everyday examples; "Nothing here
  yet" and its "inspect prior work" are gone. The setup card rides in the
  greeting (the app-wide banner stands down while one is on screen) and
  keeps the saffron wash elsewhere; the header's "Needs an AI service" is a
  saffron button. Both open the in-app Connect sheet: a model already
  running on this computer (found through the providers that need no key)
  is one click, or an account key with each key offered once. The sheet
  makes the wizard's own calls with the wizard's scopes (key in the Shared
  scope, provider and model in the Agent's project layer), so identical
  choices give identical configuration (doc 46 D7); the other setup steps
  still open the one wizard. One `setupEpoch` signal refetches every
  `GET /onboarding` reader. Also fixed: the model lookup's failure no
  longer shows as a message-box error (it named ANTHROPIC_API_KEY on a
  fresh home, surfaced by V3.3's health fix) and now titles the model
  switch; conversations with no message no longer list as "Recent results";
  and an empty conversation reads from the top instead of pinning to the
  bottom, with no "Latest" button. Done-when checked on a fresh home
  (`/tmp/vak-fresh-v1`, fresh browser profile): greeting, Connect, "Use
  this model" (Ollama), the sheet closed, the card and status cleared, and
  the first answer arrived in 93 s without leaving the app; `/onboarding`
  then reported `core_ready`. Checked at 1440 (light and dark) and 390
  (light and dark): no sideways scroll, the sheet inside the viewport with
  focus inside it, and a conversation with content still pins to the
  bottom. Evidence: `after/V3.4-*`. Follow-ups found on the way are V3.12,
  V3.13 and the V3.8 note.
  Test note: the full `cargo test -p vak-server` run failed once in
  `configuration_control_tests::forwarding_can_be_turned_on_and_survives_a_reload`
  (`Core::new` returned an error under full-suite load); it passed 3 of 3
  alone and the whole library target passed twice (319). V3.4 changed no
  server code; like V3.2's, it is a server-side flake under load.
- 2026-09-25: V3.5 done. Settings has three groups: Everyday (General,
  Appearance, Voice and sound, Notifications, Connections, Privacy and
  safety), Agents (one entry per active agent, opening that agent's page)
  and Advanced (Models and routing, Reliability, Prompts, Services and
  health, Storage and backup), shown only with technical details on.
  `SettingsPageId` is the one id list; the old ids (`permissions`,
  `integrations`, `learning`, `advanced`) and `pendingSettingsSection` are
  gone, and the deep links point at `privacy` (the Full access notice),
  `voice` (the voice control) and `services` (the budget banner). An agent
  page names the agent once, with one notice line and Manage agents; the
  editing, saved-to and scope callouts, the agent switcher select and the
  folder path are gone. Turn limit, check freshness, helpers, context size,
  maximum output, config sources and key storage sit behind a closed
  Technical details row, values intact; the voice limits and models do the
  same on Voice and sound. Chat bots moved to Connections; Automations and
  Add-ons show there only with technical details on. Privacy and safety
  holds the permission and approval choices (outcome words), a count of
  custom rules (the patterns, isolation and the rules editor with technical
  details on), what the agent remembers (note and conversation ids only
  with technical details on) and a link to Archived tasks. Deviation from
  the handoff: the Shared defaults link stays on Connections and Privacy
  and safety as well as the Agent page and Prompts, because those pages
  write agent configuration and removing it would drop the only way to set
  shared connections and permissions in the app. On a phone Settings is a
  list that opens each page, with a Settings back row. Settings page titles
  are Newsreader 22/28. Checked live (headless Chrome over CDP, dev server
  on `/tmp/vak-screen1-live`): every page at 1440 light and dark, the list
  and two pages at 390 light and dark, no sideways scroll anywhere; the Full
  access notice opens Privacy and safety; searching "remember" leaves only
  Privacy and safety. Evidence: `after/V3.5-*`. Not checked live: the voice
  control and budget banner deep links (id change only), saving a model or
  a key from the new agent page (handlers unchanged), and the Shared
  defaults view of each page.
  Test note: `cargo test -p vak-config --test design_tokens` fails 3 of 7
  (DESIGN.md palette, desktop and admin token drift, unused `--space-*`,
  `--dur*`, `--fs-section` and `--shadow-float` tokens) at `cd1a45a1` as
  well, before this change; V3.5 touches none of those tokens. Every other
  workspace test passed.
