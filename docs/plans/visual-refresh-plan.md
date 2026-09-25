# Plan — visual refresh (Ink and Saffron)

Status: **plan and tracker, 2026-09-25. Decisions D1 to D5 locked; V4.1
completed by explicit request; V1 complete (V1.12 closed as obsolete). V2,
V3 and V4.2 to V4.5 remain unstarted.**

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

- [ ] **V2.1 `DESIGN.md` rewrite.** The new north star (working name: the
  Good Listener), the rules kept from today (doc 75 §5.8), the changes (doc
  75 §5.9), and the Ink and Saffron tokens. The Auditor's Desk sections go.
- [ ] **V2.2 Token layer.** Light, Dark and High contrast as Ink and Saffron
  (doc 75 §5.1); the type scale, spacing, radius and motion as tokens. The old
  `:root` palette, the seven `data-theme` blocks and the Settings theme grid
  are replaced in the same change; the five retired palettes' previews go.
  Done when: exit criteria 2 holds for the token layer, and every component
  still renders in all three themes.
- [ ] **V2.3 Newsreader.** WOFF2 files for Latin and Latin Extended in the
  client, loaded with `font-display: swap` and a serif fallback; licence file
  alongside. Done when: the desktop shell and the web app both render it with
  no network access.
- [ ] **V2.4 Type scale.** Every `font-size` moves to the nine steps; three
  weights; conversation text at 16px; answers wrap at about 68 characters;
  the text-size preference still scales everything.
  Done when: exit criterion 1 holds and the conversation screen's measured
  size distribution is saved in the progress log.
- [ ] **V2.5 Plain words.** Apply the glossary (doc 75 §7) and the voice
  rules (doc 75 §5.7) to every user-facing string; add a check to the client
  tests that fails on banned terms (sandbox, ledger, receipt, frozen,
  candidate, execution, provenance, manifest, verifier, persona, fleet,
  daemon, MCP) outside technical-detail views.
- [ ] **V2.6 Disclosure setting.** Show technical details replaces
  `Density` (`store.ts`) and the Transcript detail row in Settings. Everyday
  hides what doc 75 §8 lists; onboarding asks once (D3). The setting uses a
  new key and starts off; the old `vak.density` value is not read.
  Done when: every row of doc 75 §8 is checked in both states, and a pending
  approval shows the same in both.
- [ ] **V2.7 Icons.** Choose Lucide or Phosphor, vendor it (pinned, with its
  licence), replace `components/Icon.tsx`'s set and every letter glyph.
- [ ] **V2.8 Delete what is superseded.** The 21 dead `.everyday-rail`
  rules, every duplicate selector (exit criterion 3), the `!important` rules
  that only existed to win against a later override, and unused hex
  literals.

### V3 — Surfaces (after V2 merges)

- [ ] **V3.1 Sidebar and header.** Complete tile and Newsreader wordmark;
  Search as a row; agents with their own character and a one-line status;
  status shown only when something is wrong; one footer menu.
- [ ] **V3.2 Conversation.** Answers on the page, results as the only card,
  revision events (V1.12) styled, no YOU label, quiet working state.
- [ ] **V3.3 Message box.** One field; + menu (attach, mention, skills,
  upload a recording); mic as the voice button, saffron while listening;
  model and permissions move to the agent menu.
- [ ] **V3.4 First run.** The mascot greeting, starters with examples, and an
  in-app "Connect an AI service" sheet (local model if found, or an account
  key), replacing the admin-console hand-off.
  Done when: a fresh home reaches a first answer without leaving the app.
- [ ] **V3.5 Settings.** Everyday, Agents and Advanced navigation (doc 75
  §6.3), outcome wording, one agent header, Technical details rows; on phones
  a list that opens each page.
- [ ] **V3.6 Review and Canvas.** Review as a sheet with the preview first;
  Canvas with device icons and plain labels.
- [ ] **V3.7 One sheet component** for every dialog: title, close at top
  right, focus trap, Escape, return focus.
- [ ] **V3.8 Agents.** Picker and creation wizard in plain words; every
  agent gets its own character by default.
- [ ] **V3.9 Motion.** The durations and curve from doc 75 §5.4; message
  arrival, result reveal, skeletons; no hover movement.
- [ ] **V3.10 Anchor check.** The four doc 70 screens and the three doc 75
  screens compared with their references in the running app; doc 70's
  ledger updated with the evidence.

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
