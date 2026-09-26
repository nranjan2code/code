# Plan — visual refresh (Ink and Saffron)

Status: **plan and tracker, 2026-09-25. Decisions D1 to D5 locked; V1, V2
and V3 complete. V4.1's previous woven-V identity has been superseded by the
Vakyartha Songbird at the maintainer's request (2026-09-26); platform exports
and all four-color/one-color lockups are regenerated from its SVG master. V4.2
done (WebP copies; the glyphs moved to V4.6). V4.3 runtime desktop verification
and V4.6 remain.**

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
- [x] **V2.9 One palette, used.** The admin console's `:root` and theme
  blocks become Ink and Saffron (the client's values, token for token) with
  the client's four theme choices; DESIGN.md records every shipped light and
  dark ground and text colour; rules use the type, spacing, motion and
  float-shadow tokens V2.2 defined. Done when: `cargo test -p vak-config
  --test design_tokens` passes 7 of 7 and both surfaces are checked live.

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
- [x] **V3.6 Review and Canvas.** Review as a sheet with the preview first;
  Canvas with device icons and plain labels.
- [x] **V3.7 One sheet component** for every dialog: title, close at top
  right, focus trap, Escape, return focus.
- [x] **V3.8 Agents.** Picker and creation wizard in plain words; every
  agent gets its own character by default. The one-time "Meet Vakyartha"
  dialog folds into the greeting instead of covering it on first run.
- [x] **V3.9 Motion.** The durations and curve from doc 75 §5.4; message
  arrival, result reveal, skeletons; no hover movement.
- [x] **V3.11 Finish the CSS cleanup inside each surface rewrite:** the 30
  duplicate selector groups whose merge would change the cascade, the 3
  remaining `!important` rules (`.good-chip`, `.prompt-page > header`,
  `.office-outline button`) and the 35 hex literals outside the tokens.
  Decide each as its screen is rebuilt.
- [x] **V3.12 Plain words for a failed turn.** A message sent before an AI
  service is connected fails with the server's own text ("provider auth
  missing: set ANTHROPIC_API_KEY for provider 'anthropic'"). It should say
  that no AI service is connected and offer Connect. The failure needs a
  typed kind from the server, never matching on the text.
  Done when: a fresh home that sends before connecting sees one plain
  sentence and a Connect button.
- [x] **V3.13 One frame per card.** The first answer on a fresh home (a
  small local model's entity card) shows a "Kind: entity" row and sits in a
  frame inside a frame. Everyday view drops the kind row and draws one
  frame.
  Done when: that answer shows one frame and no kind row, light and dark.
- [x] **V3.10 Anchor check.** The four doc 70 screens and the three doc 75
  screens compared with their references in the running app; doc 70's
  ledger updated with the evidence.
- [x] **V3.14 The result card.** A file result as doc 75 §6.1's card: a
  preview, the draft status in words ("Draft, version 2, waiting for your
  review"), one primary Review changes on the newest result, Open and Ask
  for changes; no byte count (it comes from `vak-server`).
  Done when: the review conversation's newest result shows that card at
  1440 and 390, light and dark.
- [x] **V3.15 The Agent page.** The Agent's character beside its name on
  its Settings page and in the Agents navigation (doc 75 §6.3 mockup).
- [x] **V3.16 Plan composition.** Screen 2's plan-plus-options relationship
  from a real plan answer, not only an options table (doc 70 screen 2).
- [x] **V3.17 Plain words for a model list that needs a key.** An agent's
  Model row showed the models request's own text ("provider auth missing:
  set ANTHROPIC_API_KEY for provider 'anthropic'"). It should say that this
  AI service needs an account key and point at the Account key row, with a
  typed kind from the server, never matching on the text.
  Done when: on a fresh home the Model row says so in one plain sentence,
  with no provider or variable name, at 1440 and 390, light and dark.
- [x] **V3.18 Service names, not ids.** An agent's AI service picker listed
  the raw ids ("anthropic", "openai-responses"). It should list the names
  people know, from one source the server owns.
  Done when: the picker and its notes show only names at 1440 and 390, light
  and dark, and the Connect sheet reads the same names.

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

- [x] **V4.1 Master export (superseded 2026-09-26).** The previously approved
  woven-V raster is preserved under `docs/brand/history/`; current source art is
  `docs/brand/mark/vakyartha-songbird.svg`. Regenerate the desktop, Dock,
  browser, iOS, Android, tray, client, admin, favicon and website exports from
  the vector master. The sole theme-aware exception is the macOS menu-bar alpha
  template.
  Done when: generated exports match the master; the Dock export has native
  padding; tray variants render correctly; and client, admin and website checks
  pass at 1440 × 900 and 390 × 844 in light and dark. Evidence: `after/` files
  named `V4.1-*`.
- [x] **V4.2 Character WebP copies.** WebP copies of the portraits and
  atlases; doc 71 updated. *Revised by the maintainer on 2026-09-26:* the
  glyphs moved to V4.6.
- [ ] **V4.3 Desktop chrome.** Overlay title bar in
  `crates/vak-desktop/tauri.conf.json` (`titleBarStyle`, `hiddenTitle`,
  `trafficLightPosition`), drag regions on the sidebar head and header, and
  the sidebar padded for the traffic lights.
  Done when: checked in the running desktop app on macOS, including window
  drag, double-click to zoom and full screen.
- [x] **V4.4 Brand README and rollout.** `docs/brand/README.md` documents the
  Songbird meaning, Ink and Saffron, Manrope-outlined public lockups, one-ink
  print exports, icon variants, and generation workflow. The shared client,
  admin and website use the full Vakyartha lockup; desktop and mobile exports
  use the complete bird tile; install/public copy uses Vakyartha.
  Done when: generator check and client, admin and public-site builds pass;
  header SVG paths load under their deployed base paths; screenshot review is
  saved in `after/V4.4-*`.
- [ ] **V4.6 Character glyphs.** A flat two-colour glyph per character for
  32px and below, in the mark's style. Decided 2026-09-26: each glyph uses
  its character's own two colours (fox orange with cream, the songbird's
  indigo with saffron, and so on), checked for contrast on every theme's
  ground; who draws them is still open.
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
- 2026-09-26: The maintainer selected the Listening Bird concept. Replaced the
  public woven-V mark with the Vakyartha Songbird vector master; preserved the
  old logo as history. Regenerated desktop, Dock, Windows, Android, iOS, client,
  admin, browser, site and print lockups. Moved the product's website header,
  client sidebar, admin header and login, notification and first-run screen to
  the full public identity. Runtime screenshots and full builds are still to
  be recorded; this item remains open until those checks pass.
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
- 2026-09-26: V3.6 done. Review is one scrolling sheet between a fixed
  header (Newsreader title, close at top right; the second "Back to
  conversation" button is gone) and the decision footer, in three
  sections: the preview first (an HTML draft drawn as a page in a
  `sandbox="allow-scripts"` frame with no network, other text as its
  source, Office as a pointer to Canvas), then Changes (version, counts,
  the files with their checkboxes, "Compare with the current file" folded,
  or the Office change list), then Before you accept (where it goes, the
  saved copy, format checks, any failed check named in plain words, the
  comments). The destination folder path, file size and hash, run and
  draft ids, the check names and commands and the preview-environment log
  moved into Technical details; byte counts left the file rows. The
  two-pane layout and its three responsive override blocks are gone.
  Canvas: Desktop, Tablet and Phone are Lucide monitor, tablet and
  smartphone icons (added to `Icon.tsx`) with the name as label and
  `aria-pressed`; the badge reads "Draft preview · Version N" and replaces
  the separate Saved draft chip; the file path shows only with technical
  details on; "Back to the answer"; the footer reads "Safe preview,
  offline". Checked live at 1440 (light and dark) and 390 (light and dark):
  Review opens with the preview on top, scrolls to the decision, the
  technical disclosure opens, Canvas opens from Review; no sideways scroll.
  Evidence: `after/V3.6-*`. The review workspace's draft renders blank in
  both Review and Canvas because its `<title>` is unclosed (the draft the
  review comment asks to fix), not because of the frame. Not checked live:
  an Office draft and a multi-file draft in the new layout.
- 2026-09-26: V2.9 done, by maintainer decision. The three `design_tokens`
  failures began at V2.2 (`1ef0f8c3`), not `cd1a45a1`. (1) The admin console
  still carried the pre-refresh warm-dark palette although DESIGN.md says it
  uses the same colours: its token layer is now the client's, its themes are
  Match system, Light, Dark and High contrast (a stored retired id resolves
  to Match system), its old-palette rgba literals are token mixes, and text
  on accent and approve fills uses `--on-accent`. (2) CSS now follows the
  spec: `--surface` `#ffffff`, `--border` `#e3e1da`; DESIGN.md and doc 75
  gain `muted` and `info` and the shipped dark line and ink-3; the test reads
  DESIGN.md's role names and now checks dark as well as light. (3) 565 font
  sizes, 616 spacing values and 57 transitions moved onto the tokens
  (values unchanged apart from snapping transitions to 120/200ms and one
  curve); the message box carries `--shadow-float`, and the two later
  `.composer-box` overrides that removed it were folded into the one rule.
  Also fixed while checking: leftover warm-dark literals on `.gate-card`,
  `.gate-features` and `.tool`/`.worker`. Checked live against the
  installed config: client at 1440 light/dark and 390 light, admin at 1440
  light/dark, no sideways scroll. Not checked: 390 dark, the admin at 390,
  High contrast in the admin, and every settings page after the spacing
  sweep.
- 2026-09-26: V3.7 done. `components/Sheet.tsx` is the one dialog frame:
  a Newsreader 22/28 title with an optional line under it, the close
  button at the top right, a scrolling body, an optional pinned footer,
  focus kept inside (`use:trapFocus`) and moved inside once drawn, Escape
  to close, and focus returned to what opened it; a `busy` sheet ignores
  Escape, the backdrop and close. On a phone it is a bottom sheet. Every
  dialog uses it: Review, Connect, Your agents, Agent identity, New agent,
  Meet Vakyartha, Keyboard shortcuts, Search, Search feeds, History,
  Scheduled tasks, Activity log, Background tasks, Compare approaches,
  Invite someone, the transcript, the confirmation dialog and the Always
  allow rule. Their own headers, close buttons, Escape handlers and the
  confirm dialog's button styles are gone, and their titles now match the
  menu items that open them. Settings and Canvas stay full-screen panels.
  Bug fixed on the way: App's Escape handler fell through to `stopRun()`
  whenever a dialog it did not list was open (Feeds, Background tasks, New
  agent, Meet Vakyartha, the identity editor, Connect, Invite, Review), so
  Escape there could stop the running turn. The newest sheet now takes
  Escape in the capture phase and stops it, and the handler's dialog list
  is gone. Two more: V3.6's Review preview read a pending resource inside
  the dock's `<Suspense>`, which pulled the panel out of the page and
  remounted the sheet (now a signal); and the … menu dropped focus to the
  page when it closed (it now returns to the … button). Focus return
  falls back to the visible control with the same name when the opener
  was re-rendered (Review's "Review changes" button is). Checked live in
  headless Chrome over CDP with real key events at 1440 × 900 and
  390 × 844, light and dark: nine sheets (Review, Your agents, New agent,
  Keyboard shortcuts, Search, History, Background tasks, Activity log,
  Compare approaches) each open with the title labelling the dialog, the
  close button at the top right, focus inside, inside the viewport and no
  sideways scroll; Escape closes each with no stop or cancel request; focus
  returns to the opener, or to the page for the two opened by keyboard
  shortcut. Evidence: `after/V3.7-*` (the History shot predates renaming
  its title from "Time travel"). Not returned on a phone: Review's opener
  is covered by the details panel after closing, so focus goes to the
  page. Not checked live: Connect, Agent identity, Meet Vakyartha, Search
  feeds, Scheduled tasks, Invite someone, the transcript, the
  confirmation dialog and the Always allow rule (same component, not
  opened).
- 2026-09-26: V3.8 done. New agent: plain steps ("name it and pick a
  character", "say how it should work"), "Personality", "How it should
  work", "Create agent"; the agent ID shows only with technical details on
  (a taken name says so in words); Back and Next sit in the sheet's footer
  and the inline styles moved to the stylesheet. Every agent gets its own
  character by default: the wizard picks the template's companion when no
  agent wears it, otherwise the first companion none does (only past seven
  agents does one repeat), and offers the seven companions but not the
  mascot, which doc 71 reserves for Vakyartha; the identity editor does
  the same for agents other than Vakyartha. The four template
  descriptions (`vak-server/src/agents.rs`) are in plain words. Your
  agents: "Open"/"Current", "Change name and character", plain folder
  text, "Your own agent"; the identity editor's layer line shows only with
  technical details on. The one-time Meet Vakyartha dialog is gone
  (`OnboardingWelcome.tsx` deleted): the same offer, create an agent or not
  now plus the technical details choice, is a note in the greeting on a
  home with no agents of its own, and it goes for good once answered (same
  `vak.onboarded` key). Checked live: the picker at 1440 light and 390
  dark; the wizard's starting points, a template's default (Moss, free)
  and from scratch (Mira, the first free), and step 3 at 390, light and
  dark; the note on a fresh home at 1440 light and dark and 390 light, no
  dialog covering it, "Not now" removes it and it stays gone on the next
  visit; no sideways scroll anywhere. Evidence: `after/V3.8-*`. Not
  checked live: creating or saving an agent (the review server uses the
  real data home, so nothing was written), a default when a template's
  companion is already taken, and Create an agent from the note. Found,
  not fixed: this workspace's two agents store `leaf` and `wave`, ids from
  an older character set, so they still draw the Vakyartha bird; that is
  pre-baseline data (invariant 29), fixed by choosing their characters in
  Your agents, not by code. Also seen: the fresh home at `/tmp/vak-fresh-v1`
  reports no AI service again, and a "rate limit exceeded" toast appeared
  after the capture script's rapid reloads.
- 2026-09-26: V3.9 done. One-shot motion is on the tokens: sheets
  (`--dur-slow`), Settings, toasts and Canvas closing (`--dur`), tooltips
  (`--dur-fast`, their show delay kept), all on `--ease`; V2.9 had already
  moved every transition. No hover moves anything: the nudges on sidebar
  rows, dock tabs, the Canvas button and file chips are gone (tooltips and
  citation popovers still slide in as they appear). New turns fade up 6px
  over `--dur`, and a settled answer reveals over `--dur-slow`, once: a turn
  is marked when it is created, and only if its conversation's load has
  been seen to start and finish, so opening a conversation animates
  nothing. `components/Skeleton.tsx` is the one loading placeholder; it
  replaces the three shimmer implementations (transcript, Settings,
  sidebar) and the "Loading…" text in Connections, System health,
  Background tasks, Invite, the folder picker, the file viewer and the two
  Suspense fallbacks. The per-rule reduced-motion lines this replaced are
  gone; the global reduced-motion rule covers everything. Checked live in
  headless Chrome: opening a conversation (1440 light, 390 dark) draws 3
  turns with none animated; sending on the fresh home (1440 light, 390
  dark) animates the new turns from opacity 0 and 6px to rest in about
  200ms; skeletons showed during every load and "Loading" text never did.
  Evidence: `after/V3.9-*`. Not checked live: the result reveal on a real
  answer (the fresh home has no AI service and the review server uses the
  real data home, so no model answered), and the hover rules beyond
  reading the stylesheet. Bug found on the way: Solid applies a static
  `classList` in an effect after insertion, so the class has to be decided
  when the turn is created.
- 2026-09-26: V3.11 done; `styles.css` is 5,957 lines (was 6,083). No
  selector is defined twice at the top level: the 30 groups V2.8 skipped
  are one rule each. Each merged at the first or last copy, whichever left
  the cascade alone, with a shorthand replacing the longhands before it;
  four needed more, because live responsive rules sat on both sides:
  `.composer-wrap`, `.chat`, `.workspace-head` and `.dock` now have one
  base rule with the values that won, and the responsive declarations that
  never applied (always replaced by a later unconditional rule) are gone,
  since they would have come alive once the base moved before them. The
  shared `.sidebar, .dock` transparent-border rule folded into each. The
  three `!important` are gone: `.chip.good-chip`, `.prompt-page > header`
  (nothing competed), and the Office outline's indent is a `--depth`
  variable instead of an inline padding. No hex literal is outside a theme
  block (was 32): token fallbacks dropped, the old palette's status colours
  are `--yellow` and `--red`, text on filled buttons is `--on-accent`, and
  the letterbox, preview stage, previewed page and terminal have named
  tokens in `:root`. Two were bugs: the Workbench command text and package
  chips were light-on-dark colours on the theme background, nearly
  invisible in light mode; they are `--text` and `--green` now. Checked by
  computed style (43 properties, each border side separately, reduced
  motion to freeze animation) of every element and its ::before/::after,
  headless Chrome: the colour and `!important` changes left 4,322
  elements on 11 screens unchanged; the merges left 11,678 elements on 29
  screens unchanged (conversation, details, Review, agents and Settings
  at 1440, 1240, 1000, 950, 850, 760, 700, 680, 620, 560, 480 and 390,
  light and dark). Two identical baseline runs differed in nothing. Not
  checked: the terminal and the Workbench chips live (no execution on these
  screens), the Office outline's narrow layout, and screens other than
  those listed.
- 2026-09-26: V3.12 done. `provider_unavailable` (vak-server) keeps
  `error` as the precise message an operator or the CLI needs and adds
  `"kind": "no_ai_service"` for `CoreError::MissingAuth` only (tests pin
  both shapes). The client's request errors are a typed `ApiError` with the
  status and `kind`; a refused send of that kind adds a system item marked
  `needs: "ai-service"`, drawn as "No AI service is connected yet, so this
  message wasn't sent." with a Connect button that opens the Connect sheet.
  Nothing matches on the message text. Done-when checked on the fresh home
  (`/tmp/vak-fresh-v1`, no AI service): sending shows the sentence and
  Connect, no provider or key name appears, and Connect opens "Connect an
  AI service", at 1440 light and 390 dark. Evidence: `after/V3.12-*`.
  Regression found and fixed on the way: V3.11's merge tool dropped a
  declaration that followed a comment inside a rule, and so lost `.app`'s
  `grid-template-rows`; with setup incomplete, the app-wide banner then
  grew into a band that squashed the app. The snapshot missed it because
  it did not compare grid rows and no snapshot screen had the banner.
  Restored; the other 29 merged groups were re-audited declaration by
  declaration and lost nothing else (`.composer-wrap` differs only by its
  intended hand merge); the snapshot now compares grid rows, areas and
  placement.
- 2026-09-26: V3.13 done. The first answer's card reaches the page by two
  paths, and both drew the problem. Adaptive path (a presentation pack,
  here `seed.entity`): `.adaptive-presentation` had its own border,
  padding and background around the card's own `canvas-card`, the frame
  in a frame; it is frameless now, since the only thing it holds is the
  rendered card. Both paths showed "Kind: entity": the entity-shaped
  primitives listed every prop as a row, including the `kind` that
  `AdaptiveTreeView` injects as the card's label (no longer a row), and
  the universal card listed the model's own `kind` field. A field named
  kind, type or semantic_type that only restates the card's type is shown
  with technical details on and left out otherwise; a card with no fields
  left draws no empty list. Checked in a temporary harness page (the
  client's Vite dev server, both paths with the heat-pump payload from the
  V3.4 answer): one bordered frame per card and no field rows at 1440 and
  390, light and dark, no sideways scroll; with technical details on, the
  tool path's Kind row returns. Evidence: `after/V3.13-*`. Not checked
  live against a model answer (the fresh home has no AI service), nor
  other primitives that sat in the adaptive frame without a frame of
  their own; those now sit on the page like prose, as V3.2 intends.
- 2026-09-26: V3.10 done: the four doc 70 screens and the three doc 75
  screens compared in the running app with the review data and a fresh
  home, and doc 70's ledger records the result ("Anchor check after the
  visual refresh"). First run matches its mockup; Settings, Review and
  Canvas hold their relationships; the result card (screen 1 and §6.1),
  the Agent page's character and the plan composition fall short and are
  V3.14 to V3.16. Evidence: `after/V3.10-*` (16 screenshots, 1440 light
  and dark, four at 390, no sideways scroll). Seen and left as data, not
  code: the review conversation's old revision request with raw ids
  (recorded before V1.12) and the header's "Vak" (the Agent name frozen in
  that conversation before the rename).
- 2026-09-26: V3.14 done. `ArtifactRef` (vak-delivery) gains two additive
  fields, `size_bytes` and `status` (`draft` with its version and newest
  saved version, `accepted`, or `in_folder`), and the projection derives the
  status from the durable sandbox records with Review's own rule (versions
  count per run; an acceptance settles the round, an undo reopens it, a
  version saved later starts a new one); unreadable records leave it
  unknown. "Draft · N bytes" and "Produced by write" are gone from all three
  server paths. The client's `ResultCard` replaces the file chip in results
  and the older live-turn deliverable chip (and their CSS): a preview (a web
  page drawn offline in a sandboxed frame from the newest saved version, an
  image, or the file's kind), the status in words ("Draft, version 2,
  waiting for your review"), kind, age and "your folder hasn't changed
  yet", then Review changes, primary only on the conversation's newest
  waiting draft, Open (a saved version opens as that version in Canvas),
  Download (not for a waiting draft) and Ask for changes; size and path show
  only with technical details on. The answer now comes before its file, and
  a narrow column (a phone, or beside Canvas) gets the compact layout by
  container query. Checked live on the review conversation at 1440 and 390,
  light and dark (`after/V3.14-review-*`): the turn-1 draft reads "Draft,
  version 2, waiting for your review" with the primary Review changes; its
  thumbnail is blank because the draft's `<title>` is unclosed (Canvas says
  the same); Open showed "Draft preview · Version 2", Review opened with
  versions 1 and 2, Ask for changes set the reply target. That
  conversation's newest result was written straight to the folder by the
  old revision path, so it reads "Saved in your folder" with no review (the
  file is no longer there). Checked against real models on the real home
  with a throwaway workspace, at the maintainer's request
  (`after/V3.14-real-*`): the local Ollama model wrote a web page with
  `bash`, which today always works in the folder, and the card showed its
  real thumbnail and "Saved in your folder"; `openai-responses/gpt-6-luna`
  edited a Word file with `office_apply` and the card read "Draft, version
  1, waiting for your review" with the reader's facts, at 1440 and 390,
  light and dark; accepting in Review turned it into "Accepted, version 1 ·
  now in your folder", after a reload and, in a second run, live. Harness
  `tests/result-card.html` (16 checks) covers what the data could not: a
  rendered thumbnail of a saved version, an older waiting draft beside the
  newer primary one, an accepted image and a result with two files
  (`after/V3.14-harness-*`). Not reached live: version 2 from a real
  revision, because the review-comment revision of the Word draft failed on
  the server ("revision did not change candidate files"). Seen, not fixed:
  Canvas opened in the first seconds after load is closed again by the
  workbench reset; one luna turn made two drafts of the same file, and
  accepting one leaves the other waiting (and primary); Office files show
  their kind, not a page preview.
- 2026-09-26: V3.15 done. Settings' Agents navigation draws each agent's
  character (24px, so the row keeps the 38px rhythm of the other rows) in
  place of the shared spark icon, and an agent's page puts its character
  (60px, as the mockup's) beside the Newsreader name; the Shared defaults
  view has no character. "Manage agents" no longer wraps beside a long
  description. Checked live at 1440 and 390, light and dark: on the review
  home (`after/V3.15-settings-*`) all three agents draw the Vakyartha bird,
  because the two custom agents store the retired ids `leaf` and `wave`
  (known data, not code); on the fresh home an agent created through the
  real New agent wizard from the Research Analyst template got its default
  character, Moss, which the navigation and the page both show
  (`after/V3.15-agents-*`, which also closes V3.8's unchecked "creating an
  agent" and "a template's default"). Seen, not fixed: on that home the
  agent page's Model row shows the server's raw "provider auth missing: set
  ANTHROPIC_API_KEY for provider 'anthropic'"; the mockup's "Edit
  personality" button was not added, since Your agents already holds
  "Change name and character".
- 2026-09-26: Fixed the Canvas that closed itself soon after load (seen in
  V3.14). On a load of `/app`, `refreshBackend` opens the conversation with
  `activate`, which sets `#/s/<id>`; `init()` then applies that route and
  calls `activate` for the same conversation again, and its
  `resetWorkbenchExecutions()` closed a Canvas opened in between. The reset
  now takes the conversation being shown and keeps a Canvas opened in that
  conversation (`openArtifactCanvas` records it: the preview's `sessionId`,
  else the active one); a reset for a different conversation still closes
  it. The unused history-clearing argument went with it. Checked live on the
  review conversation (`/tmp/vak-screen1-live`, dev build on the real home)
  before and after: before, Open on the draft card opened Canvas and the
  second `activate` closed it about 3.5 s later with the sidebar left
  collapsed; after, in headless Chrome at 1440 × 900 and 390 × 844, light and
  dark (`after/V3.14-canvas-after-load-*`), Open was clicked within 5 ms of
  the card rendering and Canvas stayed on "Draft preview · Version 2"
  through a same-conversation `activate` (forced by a route change in every
  run; the load-time one also landed after the click in two of the four),
  and a route to another conversation closed it. Not met as written: the card
  was not on screen until 4.7 to 9.6 s after load on the debug build, so no
  click could come within 2 s of load.
- 2026-09-26: A load no longer opens its conversation twice. `applyRoute`
  skips `activate` for the conversation already shown (the route
  `openAgentChat` just set), and only closes the inbox and, on a narrow
  screen, the sidebar, as `activate` would. Checked in headless Chrome at
  1440 × 900 and 390 × 844, light and dark, on the review conversation: each
  load made one attach call (it was two); Canvas opened from the draft card
  stayed open; with the inbox open, a notification-style route to the same
  conversation (`?approval=`) closed the inbox and made no attach call; a
  route to another conversation attached it and closed Canvas. No
  screenshots: nothing on screen changed. Seen, not fixed: on this dev
  server `GET /sessions` took 5.6 to 12.5 s with no client running, so the
  card took 20 to 45 s to appear on some loads.
- 2026-09-26: V3.17 done (the Model row V3.15 saw, not fixed). One
  function in vak-server, `provider_error_body`, now decides a provider
  failure's body for a refused turn (`provider_unavailable`) and for
  `GET /providers/{name}/models` and `/availability` alike: `error` stays
  the precise message for operators and the CLI, and a missing key adds
  `"kind": "no_ai_service"`, as V3.12 did for a send. Settings reads that
  kind from the typed `ApiError` and says "This AI service needs an account
  key. Add one under Account key below."; anything else still shows the
  server's message. The row's other two notes lost "provider" too:
  "Looking for models…" and "This AI service offers no models to this
  account." Nothing matches on the message text. `server_ext`'s key
  round-trip test now asks a keyless provider for its models and pins the
  502, the kind and the precise message. Checked live on the fresh home
  (`/tmp/vak-fresh-v1`, no AI service) in headless Chrome at 1440 × 900 and
  390 × 844, light and dark: the agent page's Model row shows the sentence,
  no provider or variable name is visible (the variable stays behind the
  closed Technical details row), and there is no sideways scroll.
  Evidence: `after/V3.17-*`. Seen, not fixed: the AI service picker still
  lists the raw ids ("anthropic").
- 2026-09-26: V3.16 done. Real plan answers came first: four luna
  (`openai-responses/gpt-6-luna`) runs on the real home with a throwaway
  workspace never sent a separate options card; the alternatives sat inside
  the plan (items marked "option", "Option 1:" labels, or "A or B" prose),
  with nothing typed to tell them apart. By maintainer decision a timeline
  step now carries typed `options` (label, detail, facts) and a `time`,
  additive in `emit_timeline_card`'s schema (whose description tells the
  model to put a step's alternatives there, with an example) and validated
  by vak-delivery, whose text form lists them for channels
  (`plan_step_options_are_typed_and_reach_the_text_form`). The timeline
  renderer draws a plan whose step offers options as the plan with an
  options card beside it (doc 70 screen 2), the step saying "2 options to
  choose from", each option with its details, facts and Use this; a column
  under 680px stacks them. A status every step shares ("suggested" on each)
  is no longer shown, nor on a step with options. A card-only result now
  offers Adjust plan (its actions take the turn's result id or the card's
  own item id, as the option buttons already did, and an adaptive tree with
  a timeline root counts as a plan), and Use this now asks for "the whole
  updated plan". Checked live with luna at 1440 and 390, light and dark
  (`after/V3.16-sunday-*`): with the example in the description, three of
  three plan requests put the alternatives in one step's options; Use this
  on "Park or garden stroll" prepared the reply, sending it returned a
  plan holding only the chosen step (`V3.16-sunday-revised`), and after the wording change a
  fresh plan's Use this returned the whole updated day with the choice
  marked selected (`V3.16-kids-*`). Found on the way and since fixed
  separately (`1f27ee6c`): "we live in the city" reads as a live-data request, so the
  freshness gate refused the plan card and the person got "I could not
  retrieve a current value". Not done: time-of-day icons, the reference's
  date chip and venue pictures (nothing typed carries them), and Add to
  calendar, which needs a connector (doc 70 row 2).
- 2026-09-26: V3.18 done (the picker V3.17 saw, not fixed). `Core::provider_label`
  (vak-core) is the one table of service names, and it is now also the set
  of known providers: `provider_known` is "has a label", so the two lists
  that had to agree are one, and a test fails if the registry gains a
  service without a name. `GET /providers` sends each one's `label`. The
  Connect sheet's own name table, which missed the two Responses API ids,
  is gone (invariant 30); it and Settings read the label through
  `api.providerLabel`. Settings' AI service picker, the Account key note
  ("Ollama runs on this computer and needs no key."), the key field's
  label, and the key-removed notices use the name; the notices no longer
  name the environment variable ("Key removed."). The second API style of
  one account is named "OpenAI (Responses API)" and "OpenRouter (Responses
  API)", because both entries are real choices in the picker and must be
  told apart. Ids stay in technical views (Operations, receipts, the model
  menu shown with technical details on). Checked live on the fresh home in
  headless Chrome at 1440 × 900 and 390 × 844, light and dark: the picker
  shows "Anthropic" and its nine options are names, no raw id is in the
  page text, and there is no sideways scroll; in the browser pane the
  Connect sheet lists the same names, one per account, and picking Ollama
  (not applied) shows the Ollama note. Evidence: `after/V3.18-*` (the
  native option list does not draw in a screenshot; its text was read from
  the page). `server_ext` pins the label in the listing.
- 2026-09-26: V4.2 done, as the maintainer scoped it: WebP copies now, the
  glyphs later (V4.6, own two colours each). The 16 source PNGs (512px
  portraits, 1024 × 512 atlases, 7.9 MB) moved to `docs/brand/characters/`
  and no longer ship; `scripts/brand/generate.mjs` (sharp 0.35.4, already
  pinned for V4.1) writes each portrait and atlas as WebP with 64, 128 and
  256px frames (48 files, 2.0 MB) and its `--check` covers them. 256 is one
  tier past the plan's 64 and 128, because the 104px greeting mascot needs
  208 pixels on a 2× screen. `AgentMark` loads the smallest copy covering
  its size at the screen's pixel ratio (`characterTier`,
  `tests/character-tier.mjs`); `vak-server` serves `.webp` as `image/webp`,
  and `packaged_character_assets_load_with_the_unauthenticated_shell` now
  fetches a WebP atlas and pins that the source PNG is not served. Checked
  live on the fresh home at 1440 and 390, light and dark, at pixel ratio 1
  and 2 (`after/V4.2-*`): the 28px sidebar marks load the 64px copies, the
  greeting mascot the 128px copy at ratio 1 and the 256px copy at ratio 2,
  no PNG is requested, and the page's character art is 104 KB at ratio 1 and
  181 KB at ratio 2 (the same two characters' sources were 1.9 MB); the art
  stays sharp. Not checked: the desktop shell, which ships the same `dist/`.
- 2026-09-26: Fixed the flicker the maintainer saw in the desktop and web
  apps. The session heartbeat re-reads every visible settled conversation
  every 10 s (so an owner sees a coworker's messages), and applying that
  read replaced every transcript item, so every turn was rebuilt; since
  V3.14 each file result holds a preview frame, which reloaded each time and
  read as the page refreshing. The sidebar's 10 s agents refresh also
  rebuilt every agent row and character. `hydrate` now skips a read
  identical to the last one it applied for that conversation, and the
  sidebar keeps its list when the agents are unchanged; a real change (a new
  message, an accepted draft) still applies. Measured live on the review
  conversation over 30 s at 1440 light and 390 dark, with a mutation
  observer: before, 6 turns and 6 result cards were rebuilt, 3 preview
  frames reloaded and their files were fetched again; after, no element was
  removed and no frame reloaded, while the heartbeat still asked the server
  for changes. Not checked in the desktop shell itself (it runs the same
  bundle).
- 2026-09-26: Fixed the flicker that remained after the first fix, found by
  tracing layout changes inside the real desktop window (WebKit) through a
  temporary command, since headless Chrome did not reproduce it. The window
  showed an empty conversation, and every 10 s the heartbeat's re-read raised
  the loading state, so the greeting was swapped for the loading skeleton
  (V3.9) and back, and the session refresh rebuilt the greeting's recent
  results. A background re-read (the heartbeat, a stream resync) no longer
  raises the loading state; only opening a conversation shows the skeleton.
  An unchanged session list keeps its objects, and a transcript re-read
  keeps each unchanged item's object, so its row stays mounted. Checked in
  the running desktop app: 27 s after load with no element removed, no
  attribute changed and no scroll movement (before: the greeting rebuilt and
  the skeleton flashed every 10 s); the maintainer confirmed the flicker
  gone. Web: 30 s on a conversation with content, nothing removed.
- 2026-09-26: V4.3 in progress (not ticked). The desktop window uses an
  overlay title bar (`titleBarStyle: Overlay`, `hiddenTitle`,
  `trafficLightPosition` 18,22 in `crates/vak-desktop/tauri.conf.json`), and
  `core:window:allow-start-dragging` is granted for the drag regions. The
  shell reports `window_chrome` (`overlay` on macOS, `native` elsewhere) in
  `backend_info`, and the client sets `data-chrome="overlay"` on the root
  unless the window is in full screen (`Host::onFullscreenChange`; the web
  host reports nothing). With it set, the sidebar and Settings keep a 34px
  draggable strip above their content for the window controls (so does a
  Settings page on a phone-width window), a header that reaches the left
  edge (sidebar hidden, or a narrow window) starts 76px further in, and the
  sidebar head and header are drag regions. Checked in the browser with the
  attribute forced and the controls drawn in their place, at 1440 and 390,
  light and dark (`after/V4.3-simulated-*`): the controls sit in the strip
  above the wordmark, clear of the header with the sidebar hidden, and
  above "Back to Vakyartha" in Settings. The desktop app ran with it (the
  maintainer moved the window); still to confirm in the running app: drag,
  double-click to zoom, full screen and the controls' exact position.
