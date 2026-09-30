## 5.2.10 — 2026-09-30

- Make every prompt universal and true to the runtime, from a full audit of every model-facing text (`docs/audits/prompts-universal-2026-09-30.md`): the assistant introduces itself as Vakyartha, replies in the person's language, checks work the way each result can be checked, and knows which blocks come from the runtime rather than the person.
- Stop sending ordinary writing back for a shell command: what a turn needs before it can end now comes from its reading, not English phrase lists, and a failed step passes only when the answer names the failure.
- Make Word, Excel, PowerPoint and PDF files only through the reviewed draft path, and say up front when a PDF cannot hold non-Latin text.
- Record the time and stance the model was given in the session log, name the time zone, and give scheduled runs their own time context.
- Make the managed-work contract prompt describe the JSON it is parsed as, stop reflection failing on long non-English conversations, and keep corrections, approvals and open failures through compaction and handoff.
- Apply an untrusted project's prompt text, guardrails included, only once the workspace is trusted, and retire the `.vak/SYSTEM.md` override.
- Offer a general-purpose first task instead of a codebase walkthrough.
- Restore a clean clippy and test gate.

## 5.2.9 — 2026-09-29

- Enhance Office document viewing and editing: render merged cells with accurate keyboard navigation, support text alignment and wrapping in workbook grids, bind image previews to source anchors, and render common chart variants including scaled scatter charts.
- Improve PDF document processing: expose aligned table structures to `doc_read`, repeat table headers across pages, and support searchable PDF authoring.
- Add architectural designs for secure mail & calendar integration and shared multi-agent collaboration.
- Bundle refreshed web client assets and add branded ensemble video assets.

## 5.2.8 — 2026-09-29

- Organize each agent's settings into clear sections and put capability controls in the selected agent's settings.
- Add agent-scoped privacy controls for permissions, approvals, and memory policies, with shared defaults and inheritance identified clearly.

## 5.2.7 — 2026-09-29

- Organize capabilities in Settings by discovery, agent inventory, and management, with shared inheritance shown alongside agent-specific configuration.
- Connect catalog discovery to the existing governed plugin lifecycle: validate the registered source, stage installs and updates disabled, and keep agent-scoped packages with the selected agent.
- Give Admin the same catalog installation path and refresh agent-specific resources when switching agents.

## 5.2.6 — 2026-09-29

- Fix Settings crashing on web and desktop when presentation styles read their configuration before initialization.
- Add owner passkey sign-in and recovery for headless web hosting.
- Make voice setup agent-owned, with discovered model choices and a speech test in Settings.
- Improve FinOps estimates for provider-qualified models and allow exact price overrides.

## 5.2.5 — 2026-09-28

- Carry typed result cards in channel delivery packets and render them using
  Telegram HTML, Slack Block Kit and Discord embeds, while retaining the
  readable text fallback.

## 5.2.4 — 2026-09-28

- Make scheduled tasks easier to create and understand by distinguishing AI tasks from scripts, explaining their Git requirements, and showing the latest result or reason a run could not start.
- Make Inbox updates easier to act on with plain-language labels, direct links back to conversations and task results, and clearer read controls.

## 5.2.3 — 2026-09-28

- Recognise the same model at another AI service. A model has a different
  name at each service, so a backup at another service is now used only
  after you confirm that its name is the same model; the admin portal's new
  Backup models panel finds likely matches across your connected services
  for you to confirm. A confirmed match is tried before any other backup
  model. Another key for the same service still counts automatically.
- Allow and remove other backup models in the admin portal, for every agent
  or for one, instead of by editing the config file. Routing changes apply
  from the next message, without a restart.

## 5.2.2 — 2026-09-28

- Add a full-screen reading view for PowerPoint decks, with slide navigation,
  keyboard controls and optional speaker notes. The view uses extracted slide
  content; original layout, images, animations and transitions are not yet
  rendered.
- Automatically check and populate models when selecting an already configured
  or keyless AI provider in the connection sheet.

## 5.2.1 — 2026-09-28

- Refine presentation cards across research, tables, charts, timelines and other modes with clearer branded surfaces, better narrow-screen fit, readable controls, and keyboard-accessible citations.
- Correct research citation indexing and improve source labels, dates, and citation details.

## 5.2.0 — 2026-09-27

- Read and write PDF files with `vak-pdf`, a new engine written from the
  PDF specification with no PDF library beneath it. `doc_read` gives a PDF's
  text by page and line with anchors, labels invisible, white, tiny and
  off-page text, comments, highlights and form fields, and flags
  JavaScript, actions, attachments and links without running or following
  them. Encrypted PDFs are refused with the reason, and scans are named as
  such (there is no OCR).
- Draft PDF changes the way Office changes are drafted: `office_apply`
  creates a PDF from scratch in the Word styles, or rewrites and deletes
  lines, adds comments and highlights, fills form fields, and rotates,
  moves, deletes and adds pages. Every change is a draft a person reviews,
  with its change list and the changes they can keep one by one, before it
  reaches the workspace; shared drafts, channel file-out, the verifier,
  citations such as `report.pdf#page:3`, and `vak office` read, apply,
  compare and verify PDFs too. Written files are clean rewrites, so removed
  text is gone from the file.

## 5.1.7 — 2026-09-27

- Fix FinOps in Platform Defaults by reading the installation-wide shared cost ledger, while preserving Agent-scoped ledger data.

## 5.1.6 — 2026-09-27

- Simplify first-time and everyday AI service setup, with clear API account
  billing and message destination details, and a single flow for changing a
  service or model from Settings or the message menu.
- Preserve the selected model when refreshing a service catalogue, reject
  stale discovery results, and keep Bedrock models unavailable when access is
  unknown.
- Surface failed provider, model and key writes in admin instead of reporting
  success before the server responds.

# Changelog

## 5.1.5 — 2026-09-27

- Open generated files from the Agent execution that produced them, so a
  workbook or other draft can be previewed and downloaded before Review.
- Keep draft reads confined to the owning conversation, recorded artifact and
  execution scratch directory. Saved versions continue to use their immutable
  candidate route.

## 5.1.4 — 2026-09-27

- Put the desktop window controls on the header's line. The top of the
  sidebar, the header, Settings and the Details panel share one row, and
  the sidebar button stays in place whether the sidebar is shown or hidden.
- Move the desktop window by dragging any empty part of that row, and
  double-click it for what System Settings says a title-bar double-click
  does: zoom, minimise or nothing.
- Keep the Details panel and a full-window Canvas clear of the window
  controls, and keep the header on one row beside them in a narrow window.
- Keep the conversation full width at phone size with the sidebar open, and
  open the sidebar above the header.
- Fix creating an agent, from a starting point or from scratch, when an
  older agent still has a retired character. A new agent now appears in the
  sidebar at once.
- Explain Vakyartha's architecture, Doctor and security controls on the
  website with illustrated stories, tell the follow-through story with the
  companions, add privacy and terms pages, and link the documentation.
- Add the repository license, rewrite the README for onboarding and
  administration, and describe Vakyartha on its own terms throughout the
  documentation.

## 5.1.3 — 2026-09-27

- Add daylight and dusk mobile wallpapers, composed for phone lock screens
  with all eight characters and space for the clock.
- Publish all desktop and mobile wallpaper sizes on the public website at
  `/wallpapers`, with downloads available without sign-in.

## 5.1.2 - 2026-09-27

- Rebuild the public website around everyday examples with short copy,
  white and charcoal themes, and three illustrated activity scenes featuring
  all eight characters. Examples cover cooking, travel, writing, data, code,
  celebrations and learning.
- Replace the old walkthroughs with keyboard-accessible examples and native
  request disclosures. Keep reduced-motion support and useful content without
  JavaScript. Explain the current repository-based installation honestly.
- Add the scene masters, generation prompts, responsive review captures and
  LinkedIn cover artwork to the brand library.

## 5.1.1 — 2026-09-27

- Keep the conversation reading position steady across run start, streaming,
  completion and conversation switches. Sending a new prompt follows its turn.
- Retain settled answer elements during live updates to avoid repaint flicker.
- Put Pause and Resume in the input tray beside the existing Stop button,
  without inserting a toolbar above the conversation.
- Isolate flow test workspaces so concurrent temporary-file changes cannot
  contaminate prompt substitution fixtures.

## 5.1.0 — 2026-09-27

- **Optional Dimensional 3D visual pack.** Choose it in Appearance to switch
  the Agent portraits and compact glyphs, Songbird branding, and browser icon.
  The eight characters keep their expression motion. A documented asset
  library includes transparent sources, reproducible exports, and day and
  dusk wallpapers in five laptop and desktop sizes. Installed platform icons
  remain a build-time choice.

- **Word documents, workbooks and decks from scratch.** Ask for "a memo on
  the Q3 results", "a budget workbook" or "a launch deck" in an empty
  folder and the assistant drafts the file from Vakyartha's own blank: a
  document with title, heading, list and quote styles and tables; a workbook
  whose sheets it can name, fill, format (bold, fills, number formats, column
  widths) and total with formulas; a deck from six standard layouts with
  speaker notes. It arrives as a draft marked new, you review it change by
  change, and nothing reaches your folder until you accept it. Visio
  drawings cannot be created from scratch yet; the request is refused with
  that reason.
- **Office drafts open in their preview again.** Opening a Word, Excel or
  PowerPoint draft or file in the Canvas showed only a prompt to start a
  shared workspace; it now shows the file, and a saved version still offers
  to start one.
- **A Word edit changes only the words that change.** When the assistant
  edits a paragraph, only the words that differ become tracked changes:
  "twelve months" to "twenty-four months" is one small redline in Review and
  in the file, not the whole clause struck out and typed again. Before, accepting
  such an edit made the whole paragraph take its first word's formatting (a
  bold heading word turned the clause bold), dropped its links and removed
  its footnote marks. A paragraph with a cross-reference, or with someone
  else's tracked changes, can now be edited, and a change to the
  cross-reference itself is refused with a message naming it. Revising a
  draft updates the assistant's earlier change instead of refusing the
  paragraph.
- **Word tables can be edited.** The assistant can change the text in a
  table's cells, and a citation of a cell opens its row.
- **You can counter the other side's changes.** Striking or replacing text
  someone else inserted as a tracked change is itself a tracked change, the
  way Word writes it, instead of being refused.
- **New documents are clean.** A document the assistant makes, for example
  from your template, has no tracked changes in it; only edits to a file you
  already have are tracked.
- **Word counts are right.** A file's word count no longer counts the
  "deleted by" and "inserted by" labels or the lines between table cells.
- **The Workbench shows Word, Excel and PowerPoint files** in the same view
  as the Canvas, where it used to say the file could not be previewed.

## 5.0.0 — 2026-09-26

- **Requests are read more accurately, and what is decided takes effect.**
  A review of how a message is understood, then a live run with a local
  model, found and fixed:
  - Pasted logs, spreadsheets and code are read as material, not as
    instructions: a 300-line log is no longer 301 requests, and a large
    paste is read in milliseconds instead of minutes.
  - Statements, questions and greetings are told apart from instructions;
    words like "customer" or "payment" no longer make an edit look risky;
    and "current" asks for fresh data only when you want a fact about the
    world, not "the current directory".
  - A destructive request asks first even in unusual wording ("force push to
    the production branch", `rm -rf`, `git reset --hard`), and more everyday
    instructions are understood (append, replace, merge, commit, revert,
    push and others).
  - Grants on long-running work now apply: what `vak grant` covers goes
    ahead without asking, and `vak revoke` takes effect at the next step.
    Before, a grant never reached the work.
  - The assistant can see its own long-running work when asked, and one
    chat never sees work another chat asked for.
  - A correct answer is no longer thrown away because an optional card
    failed, and an edit is no longer sent back for a check nobody asked for.
  - A message after `/stop` starts over instead of adding to the stopped
    work, and "hi" is no longer treated as the goal of the conversation.

  One thing is left as a decision: in Full access nothing asks for
  approval, so a destructive request still runs unasked there.
- **Office documents: the loop is closed.** Asking about, redlining and
  reviewing Word, Excel and PowerPoint files was run end to end with a real
  model, and what that turned up is fixed:
  - A citation opens its file with context: a workbook shows its header
    row and selects exactly the cited cells; a paragraph shows its heading.
  - A formula with no calculated value shows its formula instead of an
    empty cell.
  - A citation whose sheet name has a space no longer shows raw brackets.
  - Review says when formulas will show old values until Excel
    recalculates.
  - The assistant can no longer copy a draft out of its working folder to
    skip your review, and it is told that only its Office tool changes an
    Office file.
  - It is told that the shortened file fingerprint it was shown is enough,
    so it stops inventing the rest.

  What is left (templates, macro explanations, a live Telegram round trip)
  is listed with reasons in doc 72.
- **Trash means hidden everywhere.** "Delete" on an archived conversation is
  now "Move to trash", and it does what it says: a trashed conversation is
  gone from the sidebar, every search (the assistant's own search of past
  conversations included), the admin console, transcripts, exports and the
  digest, and nothing reopens it. Settings › Archived tasks lists the trash,
  and Restore brings a conversation back. Nothing is erased yet.
- **The digest counts spend again.** The digest read the cost ledger from
  the Agent's folder instead of the shared one it is written to, so it could
  report no spend at all.
- **A conversation is no longer reported busy when it is not.** Continuing a
  conversation could fail with "This conversation is currently active in
  Vakyartha Desktop", or open it read-only, while nothing else had it open,
  if a tool happened to be starting at that moment.
- **Docs describe the real layout.** Doc 64 and AGENTS.md now draw where an
  Agent's sessions, memory and workspace actually live in 4.x. This finishes
  M0 of the data architecture plan.

## 4.1.0 — 2026-09-25

- **A routine that cannot run says why.** A scheduled task that is due but
  cannot start (no model connected, a folder that is not a git repository,
  its Agent gone or paused) leaves a "Routine failed" inbox entry with the
  reason and what to do, once per missed slot, instead of silently not
  running. A cron slot is no longer lost when a run fails to start, two
  routines due in the same tick both run, and a routine's run opens from its
  task after a restart.
- **One schedule model.** The Agent schedule field, its run ledger and the
  `vak agents schedule|runs` commands are gone: they were stored and never
  run. Scheduled work is a task (`vak tasks`, `/tasks`).
- **Bus credentials in the secret store.** `PUT /config/bus` keeps the NATS
  credentials in the secret store instead of a plaintext `.vak/env` that
  nothing read, and applies the bus at once; removing them from the admin
  console works again.
- **Purge removes logs, and feeds honour `VAK_HOME`.**
  `vak self uninstall --purge` now removes the logs directory, and the feed
  pipeline writes only under the data home the server names.
- **Failing services show as failing.** `vak self status` and `vak doctor`
  read how each managed service's process last exited: one that keeps
  exiting with an error is reported with its status and fails `doctor`,
  instead of showing as a plain "down ✓". `self status` also notes an
  install that is not the tagged release it claims to be.
- **Release scripts refuse early.** `release.sh` checks for a clean tree and
  a changelog section for the version before anything else, and
  `bump-version.sh` refuses a dirty tree.
- **Smaller ledgers.** A turn bound to the same tools and system prompt as
  the last one records a reference instead of writing them again (on a
  typical binding, 375 bytes instead of about 5 KB per turn).
- **Office files on channels and from scripts.** A file sent on Telegram
  and edited by the Agent comes back to the same chat as a document, under
  its own name, with a caption saying what changed; a file carrying a
  sensitivity label is held for review in Vak instead. Channel documents
  may now be up to 20 MiB, and the words sent with a file reach the Agent.
  `vak office read|apply|diff|verify` does the same reads, edits,
  comparisons and checks from a script, as JSON.
- **Office files in, cited and out.** Dropping a file on the conversation
  saves it to the workspace inbox and names it to the Agent, as a channel
  attachment always was; a Word, Excel or PowerPoint file never reaches the
  model as bytes (the composer used to paste any file in as text). The
  message shows the file as a card with the reader's facts, Open, Download
  and, in the desktop app, Open with. An answer that cites
  `` `file.docx#anchor` `` links to that place in the file's view. The
  desktop and web host save a file through one `saveFile`; `saveText` is
  gone.
- **Steadier long turns.** The per-turn working-set plan, tool list, and
  request tail are now each resolved once per turn and reused byte-identical
  across every step, instead of being rebuilt (and silently reshaped) on
  every step — required for Claude's preserved-thinking check and it keeps
  provider prompt caching hot across a whole turn.
- **Drift detection is quieter.** A step is now only flagged as drifting off
  the user's request when it verbatim-repeats a prior turn's answer;
  cross-domain false positives on legitimately related follow-ups are gone.
- **Turns stay open through a runtime nudge.** A turn now correctly stays
  open while its last message is a system redo nudge or a tool result, so
  the nudge always reaches the model on the next request.
- **`/run` and `/steering` no longer reject a busy session.** Both endpoints
  admit through a shared queue and return `202 {"request_id","state":
  "started"|"queued"|"duplicate"}`; queued input runs as the next turn once
  the live one settles. Cancelling a run no longer manufactures a duplicate
  "finished" event.
- **Faster turn-close.** A turn's closing summary card is now built
  deterministically (verbatim when short, first-sentence fallback when
  long) instead of a separate model call, removing a multi-second tail
  latency on some providers.
- **Capacity probing moved off the critical path.** A model's context-window
  probe now runs in the background after a turn finishes rather than
  blocking the next request; model metadata is served stale-while-revalidate.
- **Anthropic: reasoning effort and an opt-in fast mode.** Requests now carry
  `output_config.effort` (never disabling extended thinking outright), and
  `[providers.anthropic] fast_mode` opts into `speed: "fast"` on models that
  support it.
- **Checkpoints are faster and lighter on disk.** Captures are now
  incremental (unchanged files are skipped via a size/mtime fast path) and
  stored as content-addressed blobs shared across a session's checkpoints,
  instead of full base64 copies embedded in every checkpoint file.
- Streaming events now coalesce under backpressure instead of ever being
  dropped.
- **Clients receive only what a person should see.** The live event stream
  sends text deltas without per-frame snapshots (a ~20 KB answer went from
  ~21 MB to ~260 KB on the wire), drops retries, route fallbacks, compaction
  and stop-gate notes, and reports run endings as short human messages
  instead of raw errors; channel replies use the same messages. Presentation
  frames carry a snapshot only on open, settle or resync, and runtime
  bookkeeping (admission, capacity, diagnostics, goal-update rows) is never
  projected. Drafts the runtime sends back for a redo are removed from the
  chat.
- **Answers in prose are never executed.** A code block in the model's
  answer is no longer run as a command; only explicit `<tool_call>`
  envelopes naming a loaded tool are treated as calls.
- **Writing requests no longer demand a command or file edit** unless the
  request names a file to save.
- **A slow model step is retried, not aborted.** The step watchdog now
  cancels only the timed-out attempt.
- **An Office draft is not re-presented as a card.** A tool that delivers a
  reviewable file (`office_apply`) now stands down the run's presentation
  check, so the answer after a draft ends the turn instead of taking another
  model call. A malformed `office_apply` op names the valid ops.
- **The repair directive is typed control traffic.** It is recorded as a
  `ControlKind::RepairDirective` message (`[repair-directive]`) rather than an
  inline marker, so no client shows it as the person's words.
- The workspace header fits its own column when the Canvas is open.

## 4.0.2 — 2026-09-24

- **Presentation hydration.** Added timeline snapshot reconciliation and presentation hydration helper in `vak-client-ui` to preserve and smoothly merge presentation state in the client store.
- **Tool input recovery and normalization.** Added schema-driven unwrapping for model tool call wrapper dialects, normalized tool calls in assistant responses, and strengthened MCP tool validation and roundtrip handling.
- **Intent kernel and strand refinement.** Refined act extraction for delivery verbs and locate phrases, updated tier-1 lexicon digest, and improved primary strand selection in composite reading to prioritize independent clauses over dependent modifier clauses.
- **Freshness and retrieval hints.** Added discovery-aware retrieval hints naming only admitted routes when repairing answers, and added search URL shape identification.

## 4.0.1 — 2026-09-24

- **`self install` no longer says a linked CLI is missing from PATH.** It checked only whether the install's own directory was on PATH, so a `~/.local/bin/vak` symlink into the app bundle was reported as "not on PATH". It now resolves `vak` the way a shell does — the first executable match wins — and compares real paths. A different `vak` earlier on PATH is named as shadowing the install, with only the PATH-order fix offered, since a link placed after it would change nothing.

## 4.0.0 — 2026-09-24

The first release since 3.5.0. It consolidates every line of work into `main` — the context engine, the intent kernel's strands and control plane, the calm agent experience, and the workspace-execution fixes — and ships 3.5.1's topic-mismatch check, which was stamped but never published. The supported baseline stays 2.0.0, so a 3.x install updates in place.

- **rustls 0.23.45** (RUSTSEC-2026-0285: TLS 1.3 handshake messages were accepted across encryption-level boundaries). The workspace manifest now requires at least the patched version. The yanked `chacha20 0.10.1` moves to 0.10.2. `cargo audit` reports no vulnerabilities.

- **The Workbench offers Review only for a draft.** A command that worked in the workspace already put its files where they belong, so its run shows "Written directly to the workspace" instead of a *Review candidate* button the server would refuse; an execution that ran inside `.vak/scratch/` keeps the review and accept flow. Checked in the component harness with one run of each kind.
- **The desktop app builds again.** The brand refresh left every icon in `crates/vak-desktop/icons/` as an opaque RGB PNG, and Tauri's build refuses anything but RGBA, so `vak-desktop` did not compile. Each icon gained an opaque alpha channel with every pixel's colour unchanged (verified by decoding both).
- **Dev-server previews start again in a restricted mode.** Moving previews into the sandboxed worker left them under a profile that denies all network use, so a `.vak/launch.toml` server died on `listen()`. The preview worker now runs under `Sandbox::listening_variant`: it may listen on a port, and outbound connections stay denied. On macOS a server that binds every interface is reachable from the LAN, as it would be run by hand; Seatbelt cannot narrow that. Linux previews stay closed until the Landlock rule is verified there.
- **Old failures fixed.** The agent test fixtures named a character preset that no longer exists (ten `agent_chats` tests and `duplicate_agent_names_fail_closed`, which also read the real `~/vak-home`); the gateway busy tests waited for a transcript message the server no longer sends; the server extension tests launched previews with the test harness as the tool worker. The tray's glyph had silently become the grey fallback dot since the brand icon became an opaque RGB PNG; it now decodes RGB. `cargo clippy --workspace --all-targets -D warnings` is clean again.
- **`bash` works in the workspace.** Every agent command used to run in its own empty `.vak/scratch/<agent>/<execution-id>/` folder, while `read`/`write`/`edit` worked on the workspace. Found live on a local model: asked to sort `unsorted.txt` into `sorted.txt`, it could not see the input, and once it could, `read` could not find the output, so it reran the sort until its turn limit. Commands now run in the workspace (or a `cwd` inside it — `"."` was documented as the workspace root but ignored); temp files, caches and bytecode go to `.vak/scratch/<agent>/`. The per-execution quarantine flag, the eval runner's copy-from-scratch step and its scratch verification fallback are gone; candidate export refuses an execution that already worked in the workspace.
- **A listener that falls behind no longer paces the model.** The loop awaited every stream piece on its event channel, so a slow listener slowed the model and one that never read stalled a finished reply until the 600 s step watchdog. Found live: the eval runner's event receiver was alive but never read, and a long reasoning reply hung the suite. Stream pieces are now dropped when the listener has no room — each carries the message so far — and the eval runner drains its events.
- **A worker's cards reach the conversation that delegated to it.** A `task` worker records its cards in its own ledger, and they travel back with its answer as data, not text. The parent records each as a presentation of the `task` call and lists its id in the result, so the user sees it in the conversation and the parent can `recall` it to review or fix it. Before, a worker's cards were validated and then lost: the parent received only the worker's final text.
- **Tool output is kept whole and never cut blind** (docs/design/68-context-engine.md §3). Tools return their whole result; the loop records it in the ledger and a request carries up to 30,000 characters verbatim, past that a window of whole lines that names the omitted range and the `recall` call that returns it. The old cut kept a character-counted head and tail, cut every line at 2,000 characters even in short results, and put the rest in a `/tmp` spill file that a restricted run could not read and `recall` never saw. MCP results no longer write a `.vak/mcp-artifacts/` copy. The compaction and handoff summarisers see results as digests instead of their first 600 or 300 characters; the grounding and topic checks read every retrieval in the batch instead of the first 2,000 characters of the first; the flow planner sees whole tool descriptions and settled outputs; observed MCP tool descriptions are no longer cut to 90 characters.

- **Intent kernel, resolver version 2** (docs/design/47-commitment-kernel.md, *What the review changed*). A deep review of `vak-intent` and its seam found the kernel's lattice sound and its edges wrong, and this fixes all of it. `DomainSet::All` means everything everywhere now — it meant "nothing declared" in the runtime, so `[intent] enabled = false` did not reproduce pre-kernel behaviour; a weak reading gets the explicit *orienting* engagement (the orientation floor, everything else through `find_tools`) and is never excluded at stage 4. Horizon phrases match whole words (`then` no longer fires inside *authentication*), sub-floor votes on an ordered axis abstain instead of winning at 0.76 confidence, stakes words apply only to effectful acts ("where does this config live" is no longer irreversible), `source`/`green` leave the evidence lexicon, and a digest of every tier-1 table is pinned to `RESOLVER_VERSION` so the next lexicon change cannot forget the bump. `assisted` autonomy now auto-approves reversible work and asks from costly up, as documented; the stop rule follows the primary act, not a noun that is a verb somewhere; `now` is an input to `resolve` and `derive`.
- **Strands.** A request resolves to its parts: each clause gets its own reading and engagement, a relation to its neighbours (`Sequential`, `Dependent`, `Independent`) and a lineage to earlier turns (`Continues`, or `Corrects`/`Replaces` from an explicit `/goal fix` / `/goal replace` — never inferred). The turn's engagement is the strictest strand on every authority-bearing limit and the *union* of the strands' domains; a durable strand opens or continues its own commitment (`CommitmentSpec.thread_id`); the intent note lists the parts in order. `vak intent explain` and `GET /intent/explain` show them.
- **Everything the engagement decides now governs the turn.** Envelope permission and spend ceilings reach `cfg.mode` and the run cap; a vision turn is served only by legs matching `[route] modality_hints` (none declared ⇒ every leg; none capable ⇒ a typed `UnsupportedModality` error, never a dropped image); the per-turn ladder honours the engagement's prefix; `HilMode::Defer` parks an unanswerable gate in the inbox and suspends the commitment (`intent::DeferringApprover`); the gateway takes cadence and urgency from the turn's intent entry; the stop gate reads the typed `StopProfile` and strand acts instead of requirement prose; `ContextProfile` steers the working-set planner (`minimal`: no relevance retrieval and only the two most recent turns at `Full`), the tail (`working`/`full`: the workspace delta since the session began, written to the ledger as a `workspace_delta` activity before the model sees it) and the note (`full`: the commitment's criteria); the run-start checkpoint is always taken on a session's first turn and otherwise only for a reading that expects an effect.
- **Resolver tiers 2/3 are wired.** `[intent] escalate = "local" | "cloud"` runs a `Classify` dispatch on a weak reading — spend-gated under `max_classify_usd`, a `classify_timeout_secs` watchdog (default 10), receipt on the session, fail-open to the free-tier reading with the reason recorded. The request sets `ChatRequest.think = Some(false)` (Ollama `think`): live on `gemma4:e2b-mlx` the default spent its output budget thinking and returned nothing. An answer with a different number of parts than the segmenter found folds on the cautious side instead of being dropped. A classifier may raise stakes or evidence and never lower either; its limits meet the free tier's. Verified live against local Ollama: six of six runs settled `local-model` in 0.8–3.9 s.
- **Control plane.** Authority comes from the channel, never from the text: `ControlSource` (human / agent / system) is stamped by the transport and `evaluate_intervention` decides from it — agents may only pause or cancel their own children, systems only observe. Human free text is always steering; only an explicit command (`/stop`, `/pause`, `/resume`, `/status`, `/replan …`, `/add …`, `/drop …`, `/prioritize …`, `/goal replace …`, `/goal fix …`, `/approve <gate>`, or a whole message that is exactly `stop`/`cancel`/`pause`/`resume`/`status`) is control. "Stop using semicolons in the output" no longer cancels the run, and "replace the deprecated API call" no longer wipes the goal. `classify_intervention` and `classify_goal_update` are deleted.
- **Thinking off for structured side-dispatches.** `ChatRequest.think` (Ollama `think`; other adapters ignore it) is `Some(false)` for the intent classifier, the compaction summariser, the handoff and the flow planner. Measured live on `gemma4:e2b-mlx`, six runs per shape: usability unchanged (classify 0/3 → 3/3, the rest 6/6 either way) at 3–10× lower latency. The completion judge keeps the provider default: with thinking 6/6 verdicts parsed, without it 3/6 (mis-escaped quotes), and a judge that fails closed is worth the latency. `parse_verdicts` now reads every balanced JSON object in the reply — the model's thinking-on failure mode was one `{"results":[…]}` per criterion, back to back.
- **Misread ledger.** Escalation is measured against the turn's own tool calls (the whole-chain scan counted a tool used three turns ago); a verbatim restatement records `Restated` against the previous reading; `vak intent show` reports the weak cells.

- **A compaction packet is a cache, never a boundary** (docs/design/68-context-engine.md §4). A `Compaction` entry is keyed by the inclusive turn range it summarises (`first_turn_id..=last_turn_id`, plus the model whose plan asked for it) and is rendered only when the current plan's `packet_range` is exactly that range; `Full`/`Card` turns always come from the ledger. Found by a two-model replay: a session packeted under a small model kept sending that packet to a frontier model bound later, although the planner had put those turns at `Full` — the projection was skipping everything before a cumulative boundary the ledger had absorbed. The reset-with-handoff entry is the one true boundary; turns behind it are `behind_reset` in the `TurnIndex` and never planned. Growing a packet seeds the summariser from the longest stored packet over the same first turn. The conversation thread lists only directives the *actual* plan leaves out. Message-level `keep_recent` compaction and its eval sweep are gone; `vak eval` runs the context-engine gate, which now includes the two-model replay.
- **`vak-context` crate.** The context engine is its own crate — `capacity` (the measured `CapacityProfile` and probe ladder), `planner` (`WorkingSetPlan`, and `plan_for_session`, the one entry point the agent loop, `/compact` and the eval gate all plan through), `assemble` (stable prefix and digest, per-turn tail, cache breakpoints, char accounting, summariser request) — pure functions over the ledger in `vak-session`, no I/O. `/compact` previously planned without provisional cards or an open-turn reserve; it cannot differ from the loop now.

## 3.5.1 — 2026-09-20

Fixed a real regression found within hours of the 3.5.0 release, on the actual production install: asked "what is the current top news in AI", the model correctly called `tavily_search` and got real AI-news results back, then wrote `emit_metric_card` for "Noida Weather, 28°C" — a payload copied verbatim from an unrelated, much older turn still sitting in context — instead of answering from the evidence it had just retrieved. The freshness check added in 3.5.0 had nothing to say: a retrieval genuinely had succeeded this run.

- **Topic-mismatch check** (`[topic-mismatch]`, docs/design/68-context-engine.md §7): after a retrieval succeeds this run, an `emit_*_card` call is intercepted before execution if its own payload shares no word with either the directive or this turn's own retrieved evidence text — checked against both, not the directive alone, so a card correctly titled from what was actually found ("OpenAI announces GPT-6" for a directive that only said "AI news") still passes on the evidence's own words. One repair; the second strike fails closed with the raw evidence quoted, never a wrong card and never nothing. Deliberately scoped to fire only when a retrieval has already succeeded this run — checked unscoped first, which broke a real, previously-passing test: a legitimate chart card with a sparse, structural payload (`{"chart_type":"line","series":[]}`) shares no vocabulary with any directive, retrieved or not, whether it is right or wrong.

## 3.5.0 — 2026-09-20

Context engine (docs/design/68-context-engine.md). Found from a weather question on `gemma4:e2b-mlx` answered in prose instead of a card: the request carried 29.9k tokens of history because every turn's text went out verbatim until 80% of the declared window, historical tool results were cut to 300 characters blind, the system prompt carried a per-turn timestamp that broke the provider prefix cache on every turn (Ollama re-prefilled ~27k tokens, ~40s), and thirty tool schemas plus an inline MCP catalogue cost 12k tokens before any history. Replayed against the live model: it called a tool 6/6 with ≤13k tokens of context and 0/6 at ≥18k. Zero users, so this is a replacement, not a migration.

- **Measured capacity, never assumed.** A per-model `CapacityProfile` (then in `vak-agent`; now `vak-context/src/capacity.rs`) is probed at bind time — metadata rung, a geometric ladder of synthetic turns each requiring one `probe_ack` tool call, binary-searched to the largest prompt at which the model still follows an instruction, then a cache rung — and revised from every receipt (tokens per char, prefill rate, first-token latency; an instruction failure near the horizon tightens it, feedback never widens). Local providers are always probed; hosted ones start from the declared window with low confidence unless `[probe] hosted = "full"`. Recorded as ledger activities; keyed by provider, model and quantisation.
- **The turn is the unit.** `TurnIndex` (`vak-session/src/turns.rs`) walks the ledger into turns; a turn is never split. The open turn is verbatim. A closed turn projects at `Full` (directive, one trace line per tool call with its evidence id, each card payload, narration — no tool blocks, so past turns are API-valid everywhere and byte-stable), `Card` (one `TurnCard` line) or `Packet`, chosen by the `WorkingSetPlanner` against the measured budget, newest first, with relevance (BM25 over cards plus intent overlap) and anaphora promoting older turns. Compaction is incremental over cards, not an 80% emergency. The 300-character trim, `ContextPolicy`, `keep_recent`, `dynamic_history_budget_cap` and the chars/4 estimator are deleted.
- **Nothing is cut blind.** Every tool result stays in the ledger and is reachable through `recall` by evidence id (with a line range), presentation id or turn number; digests are schema-driven (JSON keys and array lengths, titles and URLs of search hits, headings of text, exit code and head/tail of command output), never a character count.
- **Presentations and TurnCards are ledger entries.** An `emit_*_card` call that validates writes `EntryPayload::Presentation` (canonical payload, SHA-256 digest, `derived_from` evidence, schema-driven identity digest); an inline `vak` fence that validates writes the same entry, so a duplicate is one rule (same digest in the same turn). The server projects cards from the chain, feedback and selection key on the entry id, and the client sends it. Every closed turn writes a `TurnCard` (asked / did / answered as presentations + narration / outcome / reading / measured cost).
- **Stable prefix, moving tail.** Temporal context, epistemic stance, intent note, work contract, conversation thread and nudges leave the system prompt and render as one tail block on the last user message, identical across the steps of a turn; the thread lists only directives not already verbatim in the request; the stance no longer forbids a card the prompt asks for. `ChatRequest.cache` carries a session key and breakpoints (after the prefix, after the previous turn, on the current step), rendered as Anthropic `cache_control` (≤4), OpenAI `prompt_cache_key`, OpenRouter `session_id`. Thinking is replayed only within the current turn and only where the provider needs it. Receipts carry the prefix digest; a changed digest writes a `prefix-changed` activity so cache breaks are visible in the ledger.
- **Tool surface.** Core tools (`recall`, `find_tools`, `skill`, `mcp`, the `emit_*_card` tools, the intent-selected domain tools) plus a one-line index of everything else; `find_tools` returns full schemas that are appended for the rest of the turn; Anthropic legs send deferred tools with `defer_loading` and the server-side tool search. Slicing fails narrow: a low-confidence reading advertises no domain tools instead of all of them. MCP is reached only through the broker; the inline schema catalogue is gone.
- **Providers.** Native Ollama provider over `/api/chat` with `keep_alive` and `num_ctx` (`[providers.ollama]`), prompt-eval timing into `Usage`, quantisation from `/api/show`; over-length responses from any provider map to `LlmError::Context` and replan once instead of failing the turn; OpenAI Responses can chain `previous_response_id` within a turn.
- **No first-class integrations.** The `tavily` name match in delivery signals, the weather-API result adapter, the skill-name → domain table and topic-named prompt examples are gone; skills declare `serves`; `vak-eval` fails the build on vendor or topic tokens in production source.
- **Drift.** A directive that changes domain marks earlier directives as paused in the thread; a step that serves a different domain than the reading, or restates a previous turn's answer, gets a steering nudge naming the current directive, and three in a row end the turn with the system-authored degraded outcome.
- **Tail placement.** The per-turn tail sits before the user's words in the last user message and echoes the directive when no user text follows, so a small model reads the ask last, not the stance; a thinking-only step with no text and no tool call gets one `[empty-step]` redo.
- **Freshness.** Temporal deixis in a directive ("current", "right now", "today", "latest") sets the `live-data` domain on the intent reading; an answer or card produced without any retrieval succeeding in the run gets one `[freshness-check]` redo. A card-only answer counts as response content for the outcome evaluator.
- **Verified live** on `gemma4:e2b-mlx` against the session that produced the original prose answer: six replays of "what is the current weather in new delhi" under the new engine emitted a metric card by tool call six times out of six (before: zero of six), with a horizon of 64k tokens measured identically by all six probes. The replays also exposed that a prose rendering of past tool calls was imitated as prose, that single-sample probe rungs gave 7k–48k for one model, and that answers reused stale figures — each fixed above.

## 3.4.11 — 2026-09-19

- Cards are Vak's own display channel: presenting tools (`emit_*_card`) need no approval in any mode, an identical repeat card call is acknowledged as a no-op instead of erroring or tripping the repeat breaker into an approval prompt, and identical card calls in a turn collapse to one card in the projection.
- Chat: cards, artifacts and their narration share one assistant header, and an approval renders only while pending.

## 3.4.10 — 2026-09-19

- Made the grounding check typed. It decided a tool "looked like retrieval" from name keywords ("search", "fetch", "query"…) and a count of URLs in the output — the harness-side table `capability/domain.rs` exists to delete — and needed `emit_*_card` excluded by hand. `AgentConfig::retrieval_check`, built by `Core`, now decides from what each capability declares it serves (`Web`/`LiveData`): built-ins through `builtin_domains`, an MCP call through its server's `serves` (a server that declares nothing inherits the `mcp` broker's claim, so an unconfigured Tavily keeps working; listing tools is not retrieval), and anything unclassified is not retrieval. Verified that Tavily sets no MCP annotations, so annotations could not have been the source. Also fixed a latent ordering hazard: the alias map was replaced after being captured; it is now filled in place.
- Card-presenting tools declare it (`Tool::presents_cards`) instead of the agent matching an `emit_` prefix.
- `vak` fences are found with a Markdown parse instead of substring scanning: indented and `~~~` fences are found, a `vak` block quoted inside a longer fence is not, an answer cut off mid-card is reported as malformed (it was ignored), and a duplicate is decided by the parsed `semantic_type`, not by a substring that a card's own text could contain.
- The `tabular` and `diff` signals are read from the parsed document instead of raw-text substrings: a table shown inside a code sample is no longer a table, an indented table is found, one stray tab no longer counts, and a diff needs a diff code block, a `diff --git` line or a real hunk header.
- Fixed the `fanout` timing test failing 24/24 under 8-way CPU load: it asserted the run finished in under 1.35s, a proxy for "disjoint workers run at the same time and a conflicting one waits". It now proves the ordering with marker files (a rendezvous only concurrent workers can pass), and passes 48/48 at 12-way load; mutation-checked in both directions.

## 3.4.9 — 2026-09-19

- Fixed the chat showing an answer twice: the question repeated, the card on one copy and only prose on the second. The chat paired its Nth turn with the server's `turn-N`, and the two sides counted turns independently; one runtime nudge that the client counted as a user message and the server did not displaced every later turn, so the server's turn was drawn in the previous question's slot and the real one fell back to a local copy without the card. Pairing is now by identity: `/transcript` returns each message's ledger `entry_id` and the chat matches it to the projection's `provenance.entry_id`.
- Made runtime-authored traffic typed instead of sniffed. New `vak_intent::control` is the single vocabulary (`ControlKind`, `InlineHint`, the context-block tags) and the single implementation of `clean_scaffolding`, replacing five hand-copied prefix lists (server projection, `vak-delivery`, the desktop client, the admin console, stop policy) that had drifted — `vak-delivery`'s copy lacked every nudge added in 3.4.6–3.4.8, so Telegram, Slack and Discord would have shown their text. Nudges are created with `MessageRecord::control(kind, body)`, which sets `MessageMeta::control`; that tag is the only way any layer recognises one. No backward compatibility: text is never sniffed.
- Fixed nudges being treated as user turns in five more places, two of them behavioural: the runtime's per-turn evidence and receipt state was cleared by a nudge in the middle of a run; turn numbering for outcome evidence counted them; compaction carried them as "user directives"; session titles and session search could surface them; reflection would memorise them as user statements; markdown export listed them as messages from the user.
- Runtime-authored and derived traffic is no longer sent to clients: `/transcript` omits nudges, derived context blocks and the frozen contract (system prompt and prompt layers) that no client read. An answer the runtime sent back for a redo is an internal draft and is no longer projected, so the user sees the redo, not both.
- Fixed channels, webhooks, the inbox and routine summaries receiving no card content: a card emitted through a tool is not in the model's final text, so they got "the chart is shown above" with nothing above it. `projection::text_with_run_cards` prepends the latest turn's cards (deterministic text form) to the narration.
- Fixed a spurious "session is locked by another process" failure when several `task` workers launch in the same wave. A child session's id was the clock's nanoseconds alone; parallel launches can read the same value (clock resolution is coarser than the launch rate), so two children shared one ledger file and the second failed to lock it, after which the stop gate added an extra parent turn. Ids now carry a process-wide sequence number and cannot collide. This was the cause of the intermittent `fanout` test failure that blocked a release (the parent's script was "exhausted" by that extra turn); the id scheme dates from 2026-08-21.
- `vak-server/tests/control_vocabulary_sync.rs` fails the build unless the client's two lists equal the Rust vocabulary exactly (mutation-checked).

## 3.4.8 — 2026-09-19

- Added a presentation check to the agent loop: when a final answer reads as something the app presents as a card (by the app's own `signals_from_text` → recipe detection, via the new `RecipeCatalog::intended_outputs`) but no card was emitted and none is written inline, the model gets exactly one nudge naming the offered `emit_*_card` tool and type; it may decline by resending unchanged. Driven by the existing recipe catalog with no per-type rules; `Core` supplies the check through `AgentConfig::presentation_check` so the agent crate stays free of card knowledge. Found from a real answer (a gpt-5.6-luna reply to "how did the Indian stock market perform last week": a markdown table plus bullets, tools offered and unused). Also fixed the data-grid recipe requiring a second, keyword-based signal (`table_data`) on top of `tabular`, so a plain table of index levels or prices was never recognised as a grid.
- Fixed tool-produced cards never appearing in the chat view, even when the model called the tool correctly and the server projected the card: they were classified `OutputKind::Information`, which the client's `Turn` filter folds away as activity chatter, so only cards written into the assistant's own text could ever show (link previews from assistant text were hidden the same way). Added a dedicated `OutputKind::Card` for cards that are part of the answer, projected for both paths, so the client needs no special case. Verified by rendering the real projected timeline of the failing session (research card, chart with inspect slider, data grid) through the real `PresentationTimelineView` in a browser.
- Fixed card layout bugs that only appear with real-world text lengths: source-chip titles in the research card overflowed and overlapped their neighbours (flex children with `nowrap` text and no `min-width: 0`); card headers (data grid, diff, preview) pushed their action buttons past the card's edge; chart x-axis labels ran off the chart and collided; the chart's inspect slider was 2px wider than its container. Found by adding a `?stress=1` mode to the card harness that pads every string in every fixture with long prose and an unbroken URL, then measuring text escaping its container across all 535 card variants in a browser (9,766 elements: zero overflow and zero tile overlaps after the fixes).

## 3.4.7 — 2026-09-19

- Fixed rich cards not rendering for any card over ~2000 characters (found from a real research answer that displayed as plain prose). The 3.4.6 `emit_*_card` tools carried the card in the tool *result* text, and the tool framework line-truncates results, cutting the JSON mid-string so nothing parsed; small charts fit, real research cards did not. Redesigned as call and response: the call's arguments carry the card (schema-constrained, recorded untruncated), `execute()` validates them against the real `SkillRegistry` and answers with a short ack — or a tool error the model repairs in the same turn instead of a silent drop — and the projection rebuilds the card from the call arguments. Covered by a test that runs all 97 registered types through call → ledger → `snapshot()` at normal and oversized payloads (mutation-checked: 194 failures when the path is broken).
- Fixed the grounding-check firing on the card tool itself: `emit_research_card` contains "search" and carries URLs, so it matched both retrieval signals and forced a redo of an already grounded, cited answer.
- Fixed agent repair nudges (`[grounding-check]`, `[fence-check]`, `[duplicate-card-check]`) appearing in the chat as a message from the user. A retry after a grounding nudge now also supersedes the earlier card instead of duplicating it.

## 3.4.6 — 2026-09-19

- Presentation cards are now emitted through twelve per-shape `emit_*_card` tools (`vak-core/src/presentation_tools.rs`) instead of a hand-written `vak` fence. Measured against the real local model this app ships (`gemma4:e2b-mlx` via Ollama's OpenAI-compatible endpoint): a free-text fence in prose parsed as valid JSON only ~20% of the time; a tool call constrained by a precise per-shape JSON Schema was 5/5 valid, and a single generic schema was worse than fences. `execute()` echoes the schema-validated arguments back as a bare `{"semantic_type","payload"}` envelope, which the existing tool-result rendering pipeline already picks up — no new render path. The fence path remains the fallback when no matching tool is present. Coverage is exhaustive and tested: `every_registered_semantic_type_across_all_shapes_renders` runs every one of the 97 registered semantic types through its tool and the real `SkillRegistry`, which caught and fixed schema/validator mismatches before any model saw them.
- Fixed the `emit_*_card` tools (and `data_query`, `doc_read`, `entity_query`, `entity_record`, which had the same defect and had never reached any model) being silently dropped before the model was offered them: `Core::tool_names()` is a second, hand-maintained allowlist that `tools.retain(...)` filters the assembled tool list against, and a tool absent from it is dropped no matter how it was registered. Found by running the shipped build against real Ollama and reading the session's recorded `tool_schemas`.
- Registered ten semantic types that the client renderer had supported since 3.x but the server registry never accepted, so `SkillRegistry::validate()` rejected every one with `UnknownType` before it could reach the client: `decision_matrix`, `criteria_matrix`, `tradeoff_analysis`, `metric_chart`, `comparison_chart`, `telemetry.chart`, `telemetry.metric`, `weather`, `lifestyle.recipe`, `lifestyle.culinary_recipe`. The registry now holds 97 types.
- Added three bounded repair turns to the agent loop, each reusing the existing turn-loop `continue` mechanism: grounding-check (a search/fetch tool succeeded and the next answer ignored it), fence-check (a `vak` fence whose JSON does not parse, naming the exact parse error), and duplicate-card-check (the model restated a card it had just emitted by tool as a trailing fence). Also fixed two order-dependent receipt bugs in the same file.
- Fixed a card being shown twice when the model, on a repair nudge, re-called an `emit_*_card` tool instead of only fixing its text: `vak-server`'s projection now supersedes the earlier same-type tool card after a `[fence-check]`/`[duplicate-card-check]` nudge. Two distinct same-type cards emitted intentionally with no nudge between them are both kept. Confirmed against a real recorded session (2 chart items before, 1 after).
- A `vak` fence that fails to parse no longer renders as total silence in server-rendered blocks; it shows a visible fallback. Multiple cards in one answer now lay out as a connected grid.
- Added a render harness covering every registered semantic type (`vak-client-ui/src/harness/`), MCP health now reads from the capability registry rather than the disconnected `mcp_cache`, and misread self-correction now observes the mechanism that actually runs.
- Documented the above in `AGENTS.md`, `docs/design/07-prompt.md` (prompt diff note), `docs/design/30-render-architecture.md`, and `docs/design/67-presentation-renderer-guide.md` (new Step 0: confirm a type is server-registered before adding a client renderer).

## 3.4.5 — 2026-09-18

- Fixed the 3.4.2 per-turn diagnostic logging (`log_turns_with_no_visible_answer`) firing repeatedly for a turn that was simply still running: it excluded `Progress`/`Retry`/`Information` items but not a bare `Outcome` item whose own status was still `Pending`/`Running`, so it logged a mid-flight turn as a failure on every client poll — caught from the live gateway log, where one in-flight turn produced ~20 consecutive log lines. Now skips a turn whose only non-progress item hasn't settled yet.

## 3.4.4 — 2026-09-18

- Found the actual mechanism behind the still-blank "top news" turn after 3.4.3: the model's `research.synthesis` fence contained malformed JSON (a missing comma/key near the end of its `takeaways` array — the local `gemma4:e2b-mlx` model produced invalid output). `assistantParts` in `structured.ts` deliberately appends *nothing* when an explicit `vak`-tagged fence fails to parse even on its relaxed retry — by design, to avoid dumping raw control JSON into chat, but with no fallback for the failure case, so a genuinely malformed response rendered as total silence with the document's `source_markdown` non-empty (holding the raw broken fence), which meant the 3.4.3 empty-document fallback never triggered either — that fallback only fires when `source_markdown` is *also* empty. `assistantParts` now appends a short "could not be rendered" note instead of nothing when a fence never parses.

## 3.4.3 — 2026-09-18

- Found the actual cause of the blank-transcript bug 3.4.1 only partly addressed: `PresentationDocumentView` rendered a completely empty `<div class="semantic-document">` whenever a turn's document had zero `blocks` *and* an empty `source_markdown` — the raw-markdown fallback in that component only fires when `source_markdown` is non-empty, so an empty document produced no visible output at all, with no error and no "no result" notice, because the item itself was present (just empty), so the 3.4.1 `Turn`-level fallback never triggered. `PresentationDocumentView` now shows the same "no result" notice for this case. The server's turn diagnostic logging added in 3.4.2 (`log_turns_with_no_visible_answer`) had the identical blind spot — any `Document` content counted as a real answer regardless of whether it actually held anything — and now checks `document_has_content` (non-empty blocks or non-empty `source_markdown`) instead.

## 3.4.2 — 2026-09-18

- Added per-turn diagnostic logging to `vak-server`'s projection layer (`log_turns_with_no_visible_answer`): whenever a turn has items but none of them qualify as a real answer under the same rule the client's `Turn` filter uses (document/structured/adaptive content, or an outcome carrying a document), the gateway now logs the turn id, session id, and the kind/status of every non-progress item it produced. Previously this class of failure (a turn ending with only a tool error, a bare document-less outcome, etc. — the exact case the 3.4.1 blank-transcript fix backstops) was invisible in `~/Library/Logs/vak/gateway.log`, which only ever logged process startup; there was no way to tell after the fact whether a turn actually produced a result that got filtered, or never produced one at all.

## 3.4.1 — 2026-09-18

- Fixed a turn that fails before producing a document rendering as a completely blank transcript entry: tool errors, document-less outcomes, and lifecycle-only fallback text are all filtered out of the chat view in production (`showOperatorChrome()` is hardcoded `false`), so the whole item list could empty out with no fallback, leaving the user staring at an empty `Turn` section with no answer and no explanation. `Turn` now shows a neutral "no result" notice pointing to task details instead of rendering nothing.
- Fixed `.thinking-row` (the animated "working" ellipsis) missing from the chat column's alignment selector, so it rendered flush against the left edge of the pane instead of aligned with the rest of the transcript.
- Fixed `AdaptiveTreeView`'s "Show original" disclosure being the one presentation fallback not gated behind `showOperatorChrome()`, unconditionally shipping the raw internal `vak`-fence JSON payload to every user; also stopped its metric node rendering a bare "—" placeholder card when no real value ever bound.
- Gave `.sidebar-settings` its base styling — it had no rule beyond a stray `min-height`, so the sidebar's Settings button rendered as an unstyled native `<button>`.

## 3.4.0 — 2026-09-18

- Live provider/model/credential config: Core previously resolved its provider/model route once at construction and cached it forever, so a config write from another process (e.g. `vak setup`) never reached an already-running desktop/server Core. Fixed by tracking a cheap config-file fingerprint and re-deriving on change, and wiring the desktop/web Settings UI to the server's existing config-change event stream.
- Credential storage moved off `.env`: secrets now go through `vak_config::credentials`, an OS-native secret service (macOS Keychain / Windows Credential Manager / Linux Secret Service) by default, with an AES-256-GCM encrypted-file fallback for hosts with no reachable secret service. No plaintext secret file is written by this codebase anymore.
- Fixed periodic UI flicker on running executions: streamed updates produced new object references each chunk, causing Solid's reference-keyed `<For>` to remount rows and restart pulse animations every tick. Switched the affected lists (vak-client-ui `WorkbenchPanel`, vak-admin-ui `Home`) to `<Index>`, gated several previously unconditional polling intervals on tab visibility, and stopped the workbench pulse timer from ticking when nothing is running.
- Fixed two dead/undefined CSS custom properties (`--danger`, `--panel-alt`) that silently fell through to fallback values, replaced hardcoded status colors with theme tokens so they re-theme correctly across presets, and removed a handful of dead duplicate CSS rules left over from prior design passes.

## 3.3.0 — 2026-09-18

- Self-checking presentation prompt coverage: A drift test now validates every worked ```vak example in `system-prompt.md` against the real delivery registry (`vak_delivery::skills::SkillRegistry::validate`) instead of just a matching enum name, and a dynamic catalogue (`RuntimeSections::presentation_catalogue`) lists any registry-accepted semantic type still missing a worked example. Fixed a live bug found in the process: `table`, `dataframe`, `recipe`, and `news` had seed presentations but were missing from the core skill's `provides` list, silently rejecting every `table` block the model emitted. Removed the dead, already-drifted `list_presentation_primitives` endpoint.
- Generic presentation renderer migration: Replaced nine bespoke presentation components with a single declarative `GenericSpecRenderer`, extended the `Primitive` vocabulary with Recipe/Research/UiPreview, and aligned the model system prompt with the actual renderer contract. Gave the generic metric card the same canvas-card chrome as every other presentation card.
- Onboarding and agent picker: Added `AgentCreateWizard` and `OnboardingWelcome`, replaced `AgentsPanel` with an updated `AgentPickerModal`, and wired agent-glyph/recents/capability-icon helpers through App, ChatPane, Settings, Sidebar, and WorkspaceHeader.

## 3.2.5 — 2026-09-18

- Per-Agent config-layer isolation: Generalized `resolve_scoped_core`/`AgentScopeQuery` from memory/learning-proposal endpoints to hooks, MCP servers/integrations, permission rules, prompt layers (including the assembled effective-prompt endpoint), skills/custom commands, the plugin store, general config endpoints, `set_permission_mode`, commitments, and checkpoints — closing the same-shape gap `list_sessions` once had, everywhere it still existed.
- Client and admin console wiring: `vak-client-ui` and `vak-admin-ui` now thread the active/selected Agent id through every config-layer API call (prompts, permissions, hooks, MCP, plugins, checkpoints, FinOps), and the Prompts admin view's layer/effective/roles resources now react to switching the selected Agent instead of only reading it once.
- Fixed a structural trust gap underneath all of the above: a user-created Agent's isolated workspace was never separately trust-prompted, so its own `permission_mode`, `hooks`, `mcp.servers`, and other privileged config were silently discarded regardless of how trusted the parent workspace was. `agents::save`/`update_schedule` now carry the creating context's own trust decision onto each Agent's isolated workspace, with a retroactive backstop in `resolve_agent_core` for Agents created before this fix.
- `set_permission_mode` now forwards `resolve_scoped_core`'s real error status (e.g. 409 for a paused/archived Agent) instead of collapsing every failure to 404; `apply_permission_mode`'s live-session cancellation is now scoped to the mutated Agent's own sessions rather than every agent's.
- Renamed the internal ephemeral subagent/task-runner concept to "worker" throughout the codebase, distinct from user-facing Agents.
- Factored the resulting ~56 duplicated `resolve_scoped_core` call sites into one `scoped_core!` macro; added regression tests for hooks/MCP/plugin-key/permission-rule/FinOps-cap/checkpoint/prompt-effective isolation and for the trust-propagation fix.

## 3.2.4 — 2026-09-17

- Active agent session preservation in discovery: Ensured `list_sessions` never skips actively registered in-memory agent sessions with `state.get(&session_id).is_some()`, allowing newly opened agent conversations to be listed immediately before the first message is sent.
- Client active agent synchronization: Added explicit `[activeAgent, setActiveAgent]` signal and resilient fallback logic in `store.ts` so switching between agent specialists immediately updates sidebar selection, header title, and composer without reverting to `Vak`.
- Client session cache protection: Updated `refreshSessions` to preserve the currently active session during background polling so fresh empty agent sessions are not evicted from the local store.
- Execution target and scope transparency: Clarified the project working directory tooltip/aria-label in `WorkspaceHeader.tsx` (`Project Working Directory: <path>`) and aligned settings scope toggle terminology to "This Workspace" (`Platform Defaults` vs `This Workspace`).
- Full stack local macOS release: Synchronized release binaries, verified manifests, and updated launchd services.

## 3.2.3 — 2026-09-17

- Dynamic turn-boundary capability admission across turns without session contract lockout: Eliminated the flawed `epoch <= 1` lock in `TurnCapabilities::build` Stage 3, and made live turns evaluate capabilities dynamically from the live capability registry rather than restricting ongoing turns to the creation snapshot contract.
- Reactive digest-driven secret resolution in `CapabilityProvider::declare`: Hashed command, args, env, and resolved secret status into `Declaration.digest` so that any API key changes in `.env` are recognized immediately by `has_pending_changes()`.
- Out-of-the-box MCP integration catalog parity: Removed any residual privileged references to Tavily in architecture documentation; ensured all out-of-the-box and custom MCP integrations, skills, tools, and hooks participate uniformly in discovery, probing, and admission.
- Verified live end-to-end integration: Validated that attaching or configuring keys between turns immediately admits tools into ongoing sessions and dispatches real-time queries through `mcp` without manual restarts or session rotation.

## 3.2.2 — 2026-09-17

- Dynamic capability admission across turn boundaries: Level-triggered `reconcile` on `has_pending_changes()` before turn admission, bumping `CapabilityRegistry` epoch and relaxing Stage 3 contract filtering so newly available MCP tools, skills, tools, and hooks are automatically admitted mid-session without process restart or session rotation.
- Reactive MCP cache & secret invalidation: `set_mcp_secret_scoped`, `remove_mcp_secret_scoped`, and `apply_persisted_mcp_servers` automatically invalidate `mcp_cache`, bypass exponential backoff via `force_probe(&id)`, and trigger immediate reconcile.
- Out-of-the-box MCP integration catalog parity: Removed all legacy hardcoded privileges or first-class status for Tavily; Tavily is an MCP catalog peer alongside Context7, Exa, and Firecrawl adhering to standard 4-tier configuration and sandboxed broker execution.
- Universal search & research signal classification: Generalized presentation signals and forensics telemetry to match all search, research, and crawl tools rather than hardcoded tool names.
- Full stack local macOS release: Synchronized release binaries, verified manifests, and updated launchd services.

## 3.2.1 — 2026-09-17

- Persistent tray & autostart control: Added "Launch at login" check item to desktop menu bar tray and toggle to Settings Operations panel.
- Configurable service supervisor: Synchronized `RunAtLoad` in `com.vak.desktop.plist` and `systemd` user units with platform `launchctl enable/disable` and `systemctl --user enable/disable`.
- Cross-process autostart persistence: Persisted `autostart` setting in `tray.json` so `vak self services-sync` preserves operator choice.
- Always-available desktop tray: Window close action hides the window while preserving menu bar residency; explicit "Quit Vak" cleanly terminates the process.
- Full stack local macOS release: Synchronized release binaries, verified manifests, and updated launchd services.

## 3.2.0 — 2026-09-17

- Agent-owned platform release (`docs/design/64-agent-owned-platform.md`): Agents are authoritative owners of conversations, private session ledgers, memory boundaries, channel targets, and delivery provenance.
- Dedicated agent workspaces: `~/vak-home/agents/<agent_id>/` isolates sessions, memories, and local configuration per specialist, while cross-agent infrastructure remains shared at `~/vak-home/` via `Core::shared_data_home()`.
- Quarantined execution scratch: tool executions strictly quarantined to `.vak/scratch/<agent_id>/<execution_id>/` preventing cross-agent artifact collision.
- Scoped secret lookup precedence: strict resolution chain from Agent private `.env` → project `.env` → Shared platform `.env` → process environment.
- Frontend Agent-First UI (`vak-client-ui`): Added `AgentPickerModal` with fleet roster, specialist creation templates, and working directory inspector; partitioned workbench telemetry by session in `sessionWorkbenchMap`.
- Health and doctor diagnostics: Added `agent_roster_check` auditing all agent workspaces and ledgers.
- Full stack local macOS release: Synchronized binaries, verified manifests, and updated launchd services.

## 3.1.2 — 2026-09-16

- Bulk presentation activation runtime: added atomic `POST /presentations/activate-all` and `POST /presentations/deactivate-all` endpoints to `vak-server` with single-write store persistence.
- Settings UI bulk presentation controls: added `Activate all` and `Deactivate all` controls to `vak-client-ui` presentation toolbar with responsive styling, working states, and toast notifications.
- Global presentation pack activation: enabled all 72 built-in presentation definitions for the global `Shared` scope on local installations.
- Full stack local macOS release: synchronized binaries, verified manifests, and updated launchd services.

## 3.1.1 — 2026-09-16

- Clause-initial imperative verb balancing: implemented `is_clause_initial` in `vak-intent` to recognize coordinating clause heads (following `and`, `then`, `also`, `plus`, `,`, `;`, `:`, `\n`, `-`), granting all coordinate operational clauses equal $1.6\times$ imperative footing with opening verbs.
- Calibrated contender dual-gating: upgraded `contenders()` in `vak-intent` to enforce an absolute noise floor (`ESCALATION_FLOOR = 0.5`) while retaining strong explicit signals (`weight >= 1.0`), preventing secondary acts from being crowded out by winner score inflation.
- Universal stop-guard hardening: differentiated code files from documentation/content (`is_code_path`), separated `code_files_modified` vs `doc_files_modified`, and disambiguated `demands_code_execution` from `demands_verification`. Documentation, research synthesis, notes, and lifestyle tasks with inspection or substantive content now complete cleanly without false-positive `bash` execution demands.
- Interactive terminal test isolation: added `#[cfg(test)]` guards to `confirm()` in `vak::install` to prevent test suites from blocking on stdin when run in interactive terminal sessions.

## 3.1.0 — 2026-09-16

- Bounded `DomainSet` Semilattice: replaced overloaded `BTreeSet<String>` with formal algebraic enum `DomainSet { All, Only { names }, Empty }`, guaranteeing disjoint domain meets collapse strictly to bottom (`Empty`) rather than widening to unconstrained top (`All`), mathematically preserving $\text{meet}(a, b) \sqsubseteq a$.
- Bidirectional inflection & silent-'e' stemming: upgraded `token_matches` to handle bidirectional suffixes (`-ing`, `-ed`, `-es`, `-s`) and silent-'e' deletion (`"ensuring"` → `"ensure"`, `"audited"` → `"audit"`, `"proved"` → `"prove"`) across evidence criteria and operational verbs.
- Conversational preamble stripping: eliminated polite conversational fillers (*"please"*, *"could you please"*, *"can you help me"*) to preserve the $1.6\times$ leading imperative verb bonus on operational verbs.
- Structural HIL isolation: hardened `derive_hil` with exact `Stakes::Costly` check, structurally isolating `Irreversible` actions from ever entering the `Defer` queue.
- Full stack local macOS release: synchronized binaries, verified manifests, and updated launchd services.

## 3.0.99 — 2026-09-16

- 3D Citadel tour modal isolation: resolved CSS class specificity clash on `.world-modal` and `.world-tooltip` overriding the `[hidden]` attribute, ensuring the Ask modal remains hidden on page load and dismisses cleanly on approval or denial.
- Resilient display state management: coupled HTML `[hidden]`, `.is-hidden` class styling, and inline `style.display` across tour lifecycle events and interactive missions.
- Full stack local macOS release: synchronized binaries, verified manifests, and updated launchd services.

## 3.0.98 — 2026-09-16

- Interactive 3D Citadel World Tour (`/tour`): Procedural zero-dependency 3D vector engine with perspective projection, painter's algorithm depth sorting, and 10 explorable architectural megastructures representing each subsystem of the Vak turn pipeline.
- Generative Web Audio synthesizer & tactile SFX: Native in-browser generative soundtrack cycling D-minor pentatonic modal progressions with analog sub-bass drones and spatial delay, paired with responsive tick, click, warp, ask, and deny audio feedback.
- Kinetic pipeline simulation (`/surfaces`): 10-stage live telemetry HUD with Single Turn, 3-Turn Tool Chain, and 100-Turn Swarm modes alongside real-world fault injectors (429 overload, out-of-bounds shell, rm -rf denial, context flood).
- Camera flight & landmark raycasting: Seamless switching between cinematic tour mode with letterbox framing and free-flight orbit mode with WASD flight, screen-space hover raycasting, tactical minimap radar, and click-to-warp.
- Full stack local macOS release: Synchronized release binaries, verified manifests, and updated launchd services.

## 3.0.97 — 2026-09-16

- Multi-runtime dev server auto-detection: expanded `detect_launch` across Python (FastAPI/Uvicorn, Flask, Streamlit, Django), Go, Rust, and modern JS/TS frameworks (Vite, Next, Astro, Nuxt, Remix) with package managers (`pnpm`, `bun`, `yarn`, `npm`) and default ports.
- PreviewPane dev server activation: enabled unconditional `activeServer` selection and immediate server tab switching on launch start, ensuring process logs stream seamlessly for both fixed and dynamic ports.
- Artifact linking & presentation canvas parity: enabled inline artifact previews, code-block fallback extraction, and safe local path classification across all agents and file formats.
- Full stack local macOS release: synchronized binaries, verified manifests, and updated launchd services.

## 3.0.96 — 2026-09-16

- Custom agent outcome-density parity: preserved substantive answers, reports, and detailed prose alongside structured presentation cards, filtering only truly fleeting transitional commentary.
- Resilient structured output extraction: relaxed transport fence parsing to tolerate indentation, CRLF newlines, and embedded JSON while strictly preventing raw transport blocks from rendering as syntax-highlighted code.
- Calm conversational controls: suppressed developer harness controls (`Plan v0`, `Change plan`) during direct conversational Q&A and custom agent interactions, displaying plan controls only when an active multi-step contract exists.
- Stop-guard non-interference: refined `evaluate_receipts` to allow substantive direct answers without forcing continuation loops when tool verification is not demanded by the prompt.
- Full stack local macOS release: synchronized binaries, verified manifests, and updated launchd services.

## 3.0.95 — 2026-09-16

- Scaffolding cleanup refinement: ensured `clean_scaffolding` trims both leading and trailing whitespace/empty lines created when control blocks are stripped from the beginning or end of model responses while preserving trailing newlines.
- Presentation & projection test coverage: validated synthetic turn filtering and scaffolding stripping with zero-panic assertions across server test suites.
- Release & local macOS installation: fresh clean-room release build with updated desktop and web bundles, full verification, and live services synchronization.

## 3.0.94 — 2026-09-16

- Presentation & delivery output normalization: scrubbed synthetic `[stop-guard]` and `[stop-hook]` user messages across client and server projection pipelines, preventing them from creating phantom turns or rendering as user bubbles.
- Outcome density intermediate message filtering: suppressed intermediate narration before subsequent assistant messages in outcome density across both `ChatPane` and `PresentationRenderer`, keeping the continuous chat canvas outcome-first while retaining operational details in Workbench.
- Scaffolding-free channel delivery & routine summaries: added comprehensive control block and scaffolding stripping across `vak-delivery`, `delivery.rs`, and background routine summaries in `lib.rs` for Telegram, Discord, and Slack channels.
- Filtered retry telemetry in outcome density: moved `RetryScheduled` notifications from the main chat canvas to activity details.

## 3.0.93 — 2026-09-15

- FinOps dashboard modernization: transformed the FinOps view into a high-density, real-time command dashboard with a responsive 5-column executive KPI strip, eliminating vertical layout inflation.
- Interactive spend dynamics & trend telemetry: upgraded the 14-day spend trend chart with interactive SVG hover crosshairs, floating tooltips, daily average baseline, budget cap guides, and summary badges.
- Spend allocation visualization: added visual distribution progress bars and share breakdown across active dispatches for providers and models.
- Real-time synchronization: wired live updates via the SSE event hub (`statsVersion`) and background telemetry polling, complete with live status pulsing badge and manual refresh controls.
- Rebuilt admin UI bundle: refreshed and stamped committed assets under `crates/vak-admin-ui/dist`.

## 3.0.92 — 2026-09-15

- Settled presentation rendering parity: enabled delivery engine `compile_markdown` and `find_semantic_json_spans` to detect and extract unfenced and prefixed semantic JSON cards from paragraph blocks, cleanly splitting prose and structured output cards with validated fallbacks.
- Defense-in-depth presentation renderer: added card extraction fallback in `PresentationRenderer` for paragraph and raw markdown blocks containing `"semantic_type"` cards, ensuring completed turns, historical replays, and settled views never regress to raw JSON.
- Rebuilt client UI bundles: stamped updated web and desktop client assets.

## 3.0.91 — 2026-09-15

- Universal semantic card extraction: empowered `assistantParts()` in `vak-client-ui` to detect, parse, and extract `{ "semantic_type": ... }` JSON presentation blocks (metrics, weather, recipes, timelines, tables, diffs, etc.) from arbitrary assistant prose even when models omit markdown backtick fences or prepend `Vak` prefixes.
- Presentation prompt hardening: updated system prompt to explicitly require markdown code fences (` ```vak\n{...}\n``` `) and forbid raw JSON or bare `Vak {` prefixes.
- Delivery parser resilience: extended document block parser in `vak-delivery` to handle `Vak\n` and `Vak ` prefixes alongside lowercase variants.
- Rebuilt client UI bundles: refreshed `dist/` (Tauri desktop) and `dist-web/` (server web) with updated presentation parser.

## 3.0.90 — 2026-09-15

- Dynamic per-turn provider and model routing: decoupled provider and model selection from session admission, assembling the route ladder dynamically on every turn from the live evidence ledger, session belief state, and current effective route.
- Runtime provider switching across desktop and gateway: changes to the global workspace route or channel provider take immediate effect on the very next turn without stale route retention or unnecessary session rotation.
- Gateway channel rotation isolation: preserved explicit channel-scoped route overrides while allowing same-workspace sessions to seamlessly route to new providers and models per turn.
- Capability drift independence: ensured capability descriptor drift acts as an authoritative, independent session rotation boundary.

## 3.0.89 — 2026-09-15

- Full-stack local macOS release & packaging: end-to-end verified build and install for macOS `/Applications/Vak.app`, `vak` CLI, `vak-desktop`, `vak-delivery-worker`, and embedded frontend bundles.
- LaunchAgent gateway daemon integration: persistent background service activation via `~/Library/LaunchAgents/com.vak.gateway.plist` with canonical workspace and token management.
- Release engineering & toolchain stabilization: clean-room isolated release staging, bundle manifest validation across desktop and web clients, and complete system health verification.

## 3.0.88 — 2026-09-15

- ArtifactCanvas layout & responsive behavior: opening ArtifactCanvas now automatically collapses the left navigation sidebar and docked panels, smoothly resizing the main chat to fit alongside the canvas (~65% canvas, ~35% chat). Closing ArtifactCanvas preserves the sidebar's closed state and expands the chat canvas to full 100% width.
- Multi-client shared session rehydration: introduced `SessionLog::open_read_only` and `Core::open_session_read_only`, allowing the web client and gateway (`vak serve --gateway`) to open, view, and rehydrate canonical agent sessions without exclusive write-lock collisions when the desktop app (`vak-desktop`) is active.
- Read-only turn protection & clippy cleanups: added lock upgrade checks on execution and collapsed nested `if` statements across server routing and delivery endpoints.

## 3.0.87 — 2026-09-14

- Directory path navigation: routed directory and folder paths (e.g. `.vak/scratch/`, workspace directories) away from Artifact Canvas directly into the Workbench Files panel (`openWorkbenchFolder()`), displaying generated sandbox files, sizes, and previews.
- Explicit folder vs deliverable classification: added `isDirectoryPath` and `isScratchDirectory` checks to ensure folder paths ending with `/` or without extensions are never classified as previewable artifact files in chat markdown or presentation blocks.
- Reactive Workbench tab synchronization: lifted `workbenchTab` into shared store state, enabling one-click deep linking into the Files/Artifacts viewer or Terminal execution viewer from any markdown or presentation action.

## 3.0.86 — 2026-09-14

- Universal sandbox & authentic deliverable resolution: eliminated dummy `dashboard.html` and code-specific bias from system prompt and contracts. The execution sandbox is universal across research, data analysis, writing, operations, engineering, and media (spreadsheets, PDFs, charts, data files, prototypes).
- Dynamic execution artifact discovery: chat outcome chips now derive directly from real execution events and workbench records with authentic filenames, sizes, and MIME types rather than brittle prose regexes.
- Robust path and punctuation normalization: server-side `resolve_confined_file` and client canvas loaders now automatically sanitize trailing punctuation (e.g. trailing periods) and resolve relative or bare filenames directly to execution scratch subdirectories (`.vak/scratch/<execution_id>/...`).
- Seamless one-click Artifact Canvas access: proactive outcome deliverable chips under chat turns, smart markdown link navigation routing previewable artifacts to Canvas, and Workbench viewer popout actions.
- Presentation store migration fix: resolved presentation store revision conflict on built-in presentation seeds during fresh app installation.

## 3.0.85 — 2026-09-14

- Immersive polyglot Artifact Canvas: user-activated slide-over overlay (`ArtifactCanvas`) supporting rich polyglot deliverables (sandboxed HTML prototypes, live localhost dev servers, PDFs via authenticated blob streams, responsive raster/vector images, and formatted syntax-highlighted code).
- Dual split and focused display modes: interactive split screen (`68%` width) keeping chat fully functional with pass-through backdrop (`pointer-events: none`) alongside responsive viewport simulation (100% desktop, 768px tablet, 375px mobile) and full-screen focused presentation.
- Automated dev-server lifecycle management: dynamic acquisition, listening port discovery, and graceful shutdown on canvas dismiss or artifact switch via `api.startLaunch`/`api.stopLaunch`.
- Reactivity decoupling & hardening: decoupled store signals (`canvasArtifact`, `canvasMode`, `canvasDevice`) preventing unnecessary iframe reloads during UI mode toggles.
- Deep review edge case hardening: resolved race conditions across dev-server port binding and rapid close-animation re-opening, enabled view-mode auto-reset by artifact type, ensured CSP parameter parity in inline cards, and added text/code standalone popout support.

## 3.0.84 — 2026-09-14

- Living outcome canvas: every markdown table in chat is now an interactive surface (`InteractiveTable`) with inline search filtering, column sorting, and instant CSV export.
- Universal document ingestion (`doc_read`): token-bounded, workspace-confined inspection and section/table extraction across Markdown, plain text, CSV, TSV, JSON, YAML, TOML, INI, ENV, and HTML/XML formats.
- Specialist domain archetypes (`researcher`, `writer`, `operator`, `analyst`): standardized templates, prompts, and CLI (`vak agents`) / REST API surfaces with subagent role inheritance.
- Semantic memory distillation: autonomous consolidation distills procedural constraints and domain entities during session compaction.
- Cross-domain multi-agent collaboration evaluations: added `general_multi_agent_collaboration` verifying multi-role delegation across quantitative metrics, citations, and synthesis.
- Proactive Agent operations: Agent-owned scheduled execution (`POST /agents/{id}/schedule`, `vak agents schedule`) and append-only execution run ledgers (`GET /agents/{id}/runs`, `vak agents runs`).

## 3.0.83 — 2026-09-13

- Harden universal prompt layering and additive custom Agent instructions.
- Preserve Agent and flow prompt provenance through delegated sessions.
- Add UTC/local temporal context, IANA timezone cron schedules, and one-shot
  task due times.
- Add source/event-aware evidence freshness evaluation and delivery posture
  mapping for unattended work.
- Rebuild the committed client bundle and refresh release metadata.

## 3.0.82 — 2026-09-13

- Preserve and read existing presentation stores written with the prior
  capitalized scope spelling while emitting the corrected lowercase API form.
- Restore gateway and Settings startup for users who already have presentation
  activations on disk.

## 3.0.81 — 2026-09-13

- Fix presentation-pack activation, deactivation, and reset requests by
  aligning scoped JSON values with the client contract.
- Allow built-in presentation packs to be explicitly activated at either
  Shared or workspace scope, with regression coverage for both.

## 3.0.80 — 2026-09-13

- Make the Agent-owned platform contract authoritative across conversations,
  channels, bots, scheduled work, and delivery provenance.
- Update repository engineering and release documentation for the Agent-first
  runtime and bundled local release workflow.

## 3.0.79 — 2026-09-13

- Make user-facing conversations agent-first, with Vak and named agents available directly in the main UI.
- Give each agent a durable, independent conversation with frozen identity and revision continuity across reloads.
- Keep internal tasks, delegation, tools, sandbox work, and subagent details behind the agent conversation.
- Add agent admission, profile-scope handling, concurrency protection, end-to-end coverage, and release documentation.

## 3.0.78 — 2026-09-13

- Add universal delegation with named helper profiles, character styles, personalities, behaviours, animation, and voice settings.
- Simplify permission and approval presentation while keeping detailed requests available on demand.
- Remove internal execution and stop-policy scaffolding from the normal chat surface.
- Expand rich result presentation across recipes, timelines, comparisons, research, and structured outputs.

## 3.0.77 — 2026-09-12

- Remove the legacy AWS Hyper/Rustls client path so the dependency audit is clear of blocking vulnerabilities.
- Verify the complete renderer, sandbox, scenario, Docker, and macOS install pipeline before release.

## 3.0.76 — 2026-09-12

- Isolate sandbox artifacts cleanly from chat turn presentation.
- Standardize error rendering with generic, robust presentation across message views.
- Rebuild and synchronize client web bundles with up-to-date frontend assets.

## 3.0.75 — 2026-09-12

- Fix chat presentation and sandbox terminal continuity across Everyday and Advanced modes.
- Preserve deliverable artifact inventory across long-running bash operations and timeouts.
- Robust watchdog script stdout extraction across execution directory headers.
- Add comprehensive presentation test suite and artifact preview verification.
- Rebuild client bundles and embed synchronized frontend assets.

## 3.0.74 — 2026-09-12

- Emit a first-class artifact event whenever write, edit, or an MCP result creates a file.
- Show every artifact type in Workbench, with a clear download fallback for formats without an inline preview.
- Add regression coverage for generated HTML results.

## 3.0.73 — 2026-09-12

- Made generated results first-class in the workspace UI with automatic result presentation.
- Reframed the right panel around Result, Review, and Activity, with clearer specialist tools.
- Promoted bounded workspace outputs to artifact events and added regression coverage.

## 3.0.72 — 2026-09-12

### Clean-Room Release Assembly and Execution Reliability

- Make release assembly delete all release-owned generated output before rebuilding.
- Bound scratch artifact discovery and reject escaped or symlinked execution paths.
- Preserve execution output across broker cancellation and Workbench replay.

## 3.0.71 — 2026-09-12

### Advanced Chat Duplication Fixes, Quarantined Scratch Resolution, and Multi-Workspace File Access

- Eliminate duplicate event timeline rendering and duplicate avatar badges in Advanced chat mode.
- Add dynamic scroll height observer in client UI to synchronize code blocks and live assistant outputs seamlessly.
- Automatically provision relative `.vak/scratch -> ..` symlink within quarantined execution directory, allowing commands targeting `.vak/scratch/...` or `./...` to write deliverables reliably without path errors.
- Resolve multi-workspace file operations in `vak-server` (`read_file`, `preview_file`, `write_file`) against the active session's authentic workspace rather than falling back only to the server's default root.
- Bind re-attached sessions to their canonical workspace directory and pooled `Core` instance.

## 3.0.70 — 2026-09-12

### Universal Sandbox Execution Lifecycle, Artifact-Aware Timeout, and Fenced Bash Parser

- Universal sandbox execution contract in system prompt and Bash tool description, keeping sandbox general-purpose while explicitly prohibiting blocking foreground preview servers.
- Artifact-aware timeout handling in `BashTool`: when deliverables are produced before a timeout, return `is_error: false` with the inventory of created artifacts.
- Support parsing fenced bash/sh scripts in text tool call fallback.
- Enforce execution receipt gate when model response claims execution or emits shell scripts without tool calls having run.

## 3.0.69 — 2026-09-12

### Stop Policy Refinement, Error Acknowledgment Reporting, and Full Release Consolidation

- Refine execution stop policy and outcome guard with idiomatic Rust cleanups.
- Recognize error recovery and explicit error acknowledgment in blocker reporting when tool failures occur.
- Verify frontend client, admin console, and site bundles against source manifests with no staleness.
- Full release distribution assembly, checksum provenance validation, and synchronized macOS local component update.

## 3.0.68 — 2026-09-11

### Intent-Driven and Receipt-Gated Execution Runtime

- Replace brittle verification regex checks with intent kernel and runtime receipt gates.
- Require substantive tool execution receipts for requests classified as requiring authoring, modification, verification, or operation.
- Implement automatic stop-guard continuation nudges when model attempts completion prematurely without tool receipts.
- Guard against unaddressed tool failures by enforcing either concrete error reporting or corrective tool invocation before completion.
- Render assistant cards exclusively from validated runtime event payloads, eliminating synthetic card scraping from raw prose.

## 3.0.67 — 2026-09-11

### Dynamic Capability Refresh and Non-Rotating Gateway Turn Boundaries

- Enable dynamic capability visibility resolution at turn boundaries without discarding session history.
- Eliminate forced gateway session rotation upon live prompt layer or capability set updates.
- Preserve immutable historical ledger contracts while seamlessly picking up runtime capability discoveries.

## 3.0.66 — 2026-09-11

### Universal Sandbox Runtime Discovery and Environment Fortification

- Improve universal sandbox runtime discovery and tool execution environment detection.
- Expand proactive tool execution prompts and system runtime capability reporting.
- Upgrade Docker sandbox environment with extended toolchain and package support.

## 3.0.65 — 2026-09-11

### Presentation Resiliency, Frontend Build Tooling Fortification, and macOS Release

- Ensure npm rebuild runs during frontend builds to link CLI binaries reliably across environments.
- Fortify syntax highlighting against lazy-load race conditions.
- Polish universal outcome cards and user message presentation.
- Refresh compiled client web and desktop distribution bundles.

## 3.0.64 — 2026-09-11

### Universal Semantic Outcome Cards and Full macOS Release

- Render fenced semantic outputs directly as interactive outcome cards across universal outcome types.
- Update compiled client web and desktop UI distribution bundles with latest presentation components.
- Prepare full production release artifacts and synchronize system service installation on macOS.

## 3.0.63 — 2026-09-11

### Full Release Consolidation, Quarantine Working Directory Control, and Anti-Evasion Verification

- Consolidate unconstrained proactive execution across all local stacks in system prompt and tool schemas.
- Enable explicit `cwd` navigation and `quarantine` override in `BashTool`, resolving workspace-root execution and scratch preview isolation.
- Fortify stop guard substantive command detection against evasive echo assertions while maintaining shell probes.
- Update compiled web and desktop distribution bundles.

## 3.0.62 — 2026-09-11

### Presentation Rails Modernization, Protocol-Driven Card Rendering, and Stop Policy Fortification

- Modernize presentation rails and everyday actions across the client UI, keeping chrome minimal and outcome-focused.
- Streamline rich presentation card rendering to protocol-driven registration with clean fallback preservation.
- Fortify stop policy substantive command detection against evasive echo assertions while maintaining seamless shell probes.
- Update compiled client web and desktop UI distribution bundles.

## 3.0.61 — 2026-09-11

### Unconstrained Sandbox Execution, Anti-Evasion Stop Guard, and Working Directory Control

- Establish universal execution sandbox in system prompt (`bash`, `.vak/scratch/`, live Workbench preview) for anything and everything (web apps, pipelines, scripts, media/documents, tests, installations, system tools) without model hesitation or refusal.
- Mandate proactive self-directed sandbox execution: the agent autonomously executes and verifies work without waiting for user commands.
- Implement `is_substantive_command` in `stop_policy` to catch dummy `echo`/`printf`/`true`/`exit 0` evasions from satisfying verification obligations.
- Expose `cwd` and `quarantine` in `BashTool::schema` and execution logic, allowing explicit execution in workspace root or subdirectories while preserving quarantined scratch prototyping.
- Register `sandbox`, `sandbox_exec`, `terminal`, `shell`, `sh`, and `exec` as canonical aliases for `bash`, and support `code` and `input` tool call parameters.
- Verify complete subagent sandbox continuity and parent Workbench streaming.

## 3.0.60 — 2026-09-11

### Presentation Layer Scaffolding & Leakage Elimination Across All Modes

- Eliminate prompt scaffolding and internal control blocks (`<conversation_thread>`, `<context_summary>`, `<intent>`, `<work_contract>`, `<context_packet>`) across both Everyday and Advanced presentation modes.
- Filter internal engine telemetry notes (context compaction, route fallbacks, and stop gates) from Everyday mode while preserving actionable errors.
- Sanitize tool previews in `ToolCard` to scrub code fences, card JSON, and scaffolding from in-flight and historical tool cards.
- Scrub control blocks from backend presentation timeline projection and sanitize assistant text in `SideChatPanel`.
- Prevent un-fenced or malformed semantic card JSON from leaking into assistant prose.

## 3.0.59 — 2026-09-11

### MCP & Tool Capability Inventory Coherence Fortification

- Unify MCP tool inventory extraction directly from `CapabilitySet` as single source of truth across all tools and surfaces.
- Fortify `TurnCapabilities::build` to extract tools from surviving MCP server configurations if missing from probe inventory, preventing prompt and tool schema disagreement.
- Ensure turn admission in `Core::run_turn_inner` merges `CapabilitySet` inventory with cache so LLMs always receive native function call definitions.
- Eliminate model text-fallback loops on web search and MCP invocations across all providers.

## 3.0.58 — 2026-09-11

### Continuous Chat Canvas & Structured Card Pipeline Fortification

- Unify continuous chat canvas and robustly parse structured cards without repaint or leakage.
- Streamline inline presentation parsing in `vak-delivery` to extract structured cards directly from outcome blocks.
- Clean up presentation renderer data normalization across timeline, grid, research, and recipe outcomes.
- Fortify presentation pipeline formatting and clippy compliance.

## 3.0.57 — 2026-09-11

### Universal Outcome Card Architecture & Leakage Elimination

- Map all 62+ registered semantic outcome types directly in `STRUCTURED_RENDERERS`
  within `PresentationRenderer.tsx`, including research synthesis, briefs,
  diff inspectors, test matrices, terminal consoles, data grids, tabular comparisons,
  interactive recipe cards, timeline plans, checklists, milestones, schedules,
  scorecards, budgets, financial summaries, and interactive preview cards.
- Add robust data normalizers (`normalizeRecipe`, `normalizeTimeline`, `normalizeDataGrid`,
  `normalizeResearch`) with shape-based fallbacks for dynamic and plugin skills.
- Fortify `RecipeCard` with dynamic servings scaling, flexible ingredient shapes,
  and countdown cooking timers across minutes and seconds.
- Eliminate backend scaffolding and diagnostic leakage (`Surface:`, `Outcome:`,
  `primary deliverable:`, `completed`, `contract_id:`) across Everyday and Advanced modes.
- Preserve consistent Vak avatar mark and header on settled assistant turns matching live streaming.

## 3.0.56 — 2026-09-10

### Presentation Delivery Test Suite Expansion

- Add `audit_final_output_10k.rs` — a 10,000-scenario deep-verification suite
  that renders each input through all six delivery surfaces (Telegram HTML,
  Discord Markdown, Slack Mrkdwn, Desktop Markdown, Terminal Plain, Admin
  Plain) and asserts on the **actual final output content**, not just
  non-panic. 5,000 red-team content-verification scenarios (headings, code
  blocks, lists, emphasis, tables, links, quotes, HTML escaping, spoilers,
  mixed structures, special characters, horizontal rules, empty/whitespace
  inputs, paragraphs, nested constructs) + 5,000 blue-team structural
  verification scenarios (packet fields, fallback preservation, coverage
  integrity, chunking validity, payload types, serialization round-trip,
  max_chars truncation, cross-surface consistency, kind matching). Each
  scenario exercises all 6 surfaces × markup combinations.
- Total delivery test-suite scenario count now: 922 (presentation_audit) +
  10,000 (audit_sweep_10k) + 10,000 (audit_final_output_10k) = 20,922
  scenarios, all passing.

## 3.0.55 — 2026-09-10

- Support the universal presentation card pipeline across the initial seeded
  presentation definitions; the current pack is 70 definitions spanning
  universal, everyday, coding, research, data, and lifestyle experiences.
- Expand `SkillRegistry` provides whitelist and add TOML/lenient fragment deserialization in `vak-delivery`.
- Update model system prompt with concrete payload examples for weather, metrics, timelines, checklists, and research synthesis.
- Add frontend semantic type aliases in `PresentationRenderer.tsx` for seamless outcome card rendering.

## 3.0.54 — 2026-09-10

- Implement systemic prompt scaffolding scrubbing (`Surface:`, `Outcome:`,
  `primary deliverable:`, `completed`) in projection.
- Partition Everyday vs Advanced mode error card presentation, suppressing raw
  protocol JSON tracebacks in Everyday conversation bubbles.
- Enhance MCP tool error formatting with actionable model self-repair guidance.

## 3.0.53 — 2026-09-10

- Ship Modern Presentation System 2026 update with dual-density Everyday vs
  Advanced modes and tactile sliding segmented control.
- Introduce elevated user message bubbles, Vak brand pulse header, clean prose
  scaffolding scrubbing, and collapsible reasoning traces.
- Implement uniform outcome card architecture across coding diffs, research
  synthesis, telemetry charts, terminal sessions, and lifestyle recipes.
- Refresh committed web/desktop UI bundles and document specification in
  docs/design/60-modern-presentation-system-2026.md.

## 3.0.52 — 2026-09-10

- Keep outcome, review, provenance, and diagnostic chrome out of production
  presentation surfaces; reserve it for development inspection.
- Preserve clean user-facing responses when structured adapter payloads are
  malformed instead of surfacing parser/debug residue.
- Make subtle outcome feedback controls available without verbose review copy.

## 3.0.51 — 2026-09-10

- Make Everyday the calm conversation surface and keep operator metadata out
  of the reader flow.
- Keep the helpful-details rail closed on launch and remove duplicate header
  search in favour of the sidebar conversation search.
- Hide model, sandbox, warning, and transcript-density diagnostics from
  Everyday while retaining them in Advanced.

## 3.0.50 — 2026-09-10

- Keep provisional streaming shells and tool diagnostics out of Everyday;
  retain them for Advanced while the final rich result is being produced.
- Remove outcome/evidence status chrome from Everyday so the answer card stays
  the primary reader surface.

## 3.0.49 — 2026-09-10

- Release the corrected integration-owned presentation adapter boundary and
  refreshed local desktop/web bundles.

## 3.0.48 — 2026-09-10

- Remove the hard-coded Tavily adapter; MCP integrations own their result
  contracts and may self-declare or provide their own presentation adapter.

## 3.0.47 — 2026-09-10

- Validate plugin-provided structured results through the merged presentation
  skill registry during projection.

## 3.0.46 — 2026-09-10

- Wire strict WeatherAPI and Tavily result adapters into specialist
  presentation rendering.

## 3.0.45 — 2026-09-10

- Keep presentation envelopes and ledger provenance out of the user-facing
  Everyday answer surface, preventing duplicate rich cards and debug text.

## 3.0.44 — 2026-09-10

- Make the rich presentation path universal for assistant answers, with a
  consistent Answer card in Everyday and Advanced modes.
- Keep specialist renderers and the original document content available.

## 3.0.43 — 2026-09-10

- Stabilize Everyday and Advanced UI flows, seed/update reconciliation, and
  release verification with refreshed embedded frontend bundles.

## 3.0.42 — 2026-09-09

- Expand fake-server MCP regression coverage for missing servers and malformed arguments.

## 3.0.41 — 2026-09-09

- Add the data-driven Everyday helpful-details rail while preserving the Advanced dock.

## 3.0.40 — 2026-09-09

- Improve Everyday navigation labels and preserve developer-oriented labels in Advanced mode.

## 3.0.39 — 2026-09-09

- Repair MCP calls from providers that omit the action discriminator and complete strict release-gate cleanup.

## 3.0.38 — 2026-09-09

- Ship the adaptive presentation runtime, reusable experience packs, Everyday onboarding, cross-surface fallbacks, and release verification improvements.

## 3.0.37 — 2026-09-09

- Record every provider dispatch in FinOps, including explicitly unpriced usage.

## 3.0.36 — 2026-09-09

- Added operation-specific voice model configuration and scoped bot/chat overrides.

## 3.0.34 — 2026-09-09

- First-class voice capability across provider routing, realtime sessions, local ASR/TTS, channels, admin, quotas, audit receipts, and protocol versioning.

**Supported history begins at 2.0.0.** Every version before it is
unsupported and cannot be upgraded in place — see
`docs/design/46-stabilization-install-and-onboarding.md` Part VII.1. Entries
for those releases were removed from this file; `git log` holds them.

## 3.0.33 — 2026-09-09

### Cross-Surface Sandbox and Artifact Release

- Harden sandbox streaming, cancellation, reconnect, and artifact lifecycle handling.
- Add safe compound web-app previews with workspace confinement and CSP isolation.
- Improve Workbench artifact rendering, responsive layout, accessibility, and delivery projections.
- Ship regenerated desktop, web, and admin bundles.

### First-Class Voice Foundation

- Add governed voice sessions with bounded inbound/outbound audio, cancellation,
  append-only transcript/playback activities, and live capacity reporting.
- Add Gemini and OpenAI-compatible transcription/synthesis adapters with explicit
  configured models, capability discovery, and secure credential provenance.
- Add configurable local transcription and native voice handling for Telegram,
  Discord, and Slack, including Telegram Ogg/Opus conversion.
- Add voice settings, provider discovery, administration, and deterministic
  security/channel integration coverage.

## 3.0.32 — 2026-09-09

### UI Modernization Release

- Refresh the client and admin interfaces with updated workbench, terminal,
  settings, inbox, presentation, and status surfaces.
- Rebuild and ship the embedded frontend bundles alongside the release.

## 3.0.31 — 2026-09-08

### Rich Modern Terminal Surface (`vak term`)

- **Ultra-Dense Cinematic Command Cockpit.** Greenfield interactive console surface in `crates/vak-terminal`, invoked via `vak term` or `Surface::Terminal`. Preserves the headless Unix-composable CLI (`vak exec`, `vak plan`, etc.) intact.
- **Screen 1: Dual-Deck Studio (60/40).** Left deck with 24-bit TrueColor halfblock wireframe (`▀▄█`), external browser launcher (`[o]`), collapsible markdown card (`Space`), and live bash worker card with animated Braille spinner (`⠋⠙⠹`). Right HUD with CPU/RSS gauges, 360° rotating radar sweep (`6°/tick`), token burn-rate waveform, and git diff inspector.
- **Screen 2: Remote Observability & Operations.** Top KPI bar (FinOps spend with budget cap meter, 100% Healthy circuit breaker, 3/3 online services, DLQ). Left deck with real-time incident feed, audit receipts with SHA-256 fingerprints, and Merkle causal DAG. Right HUD with network traffic waveform and provider latency meters.
- **Screen 3: Settings & Remote Admin.** CorePool multi-tenant directory tree displaying Shared Base Layer and active workspace, interactive security engine (`WorkspaceWrite`, `ReadOnly`, `FullAccess`, `Ask`, `AutoApprove`), MCP server inventory (`tavily`, `docker`, `github`), bot gateways, and pending authorization queue with live `[Approve]` and `[Deny]` action states.
- **Screen 4: Attention Inbox & Memory.** Prioritized attention inbox with dynamic unread badge, budget alert ack (`[a]`), watchdog service recovery (`[r]`), self-evolved skill candidate queue with promotion (`[p]`), and cron automation countdown timers.
- **Human-in-the-Loop (HIL) Dialog.** Floating modal with in-place interactive shell command editor (`[e]`), risk assessment, execution directory, cost impact, and fast decision keys (`[y]`, `[a]`, `[d]`, `[Esc]`).
- **5 TrueColor Themes & Minimalist Unicode.** Vak Warm, Vak Slate, Vak Paper, Vak Contrast, and Tokyo Night cyclable via `F2` or `/theme`. Strict geometric Unicode glyphs with zero emojis workspace-wide.

## 3.0.30 — 2026-09-08

### Intent-Driven Active Skill Inlining & Progressive Disclosure

- **Zero-Round-Trip Active Skill Inlining.** Transformed skill handling from voluntary
  tool-calling ceremony to intent-driven active context inlining. Surviving skills with
  the highest domain affinity to the turn (e.g. `software-development` for code authoring/modification)
  are automatically promoted to active status and inlined into the system prompt under
  `## Active Skill Guidelines` on Turn 1.
- **Progressive Skill Disclosure.** Remaining catalog skills are concisely indexed for on-demand
  loading via `skill({"name": "..."})`, preventing context window bloat while ensuring
  specialized procedures remain accessible.
- **Starter Skill Domain Classification.** Added domain classifications for built-in starter skills
  (`software-development`, `debugging`, `code-review`, `data-and-spreadsheets`, etc.) in
  `crates/vak-core/src/capability/provider.rs`.
- **Full React Benchmark Verified Across Models.** Validated full production Vite + React builds
  with `npm install` and `npm run build` across both frontier (`gpt-5.6-luna`) and local small
  models (`gemma4:e2b-mlx` 2B) in both single-agent and multi-agent subagent configurations.

## 3.0.29 — 2026-09-08

### Intent Pre-Flight HUD & Multi-Turn Context

- **Lossless Realtime Intent Pre-Flight HUD.** Upgraded Composer Intent Pre-Flight
  HUD to expose complete 6-axis commitment projections (`Risk`, `Pacing`,
  `Scope`, `Autonomy`, `Budget`, `Verification`), capability bounds, and raw
  signal weight telemetry.
- **Session-Grounded History Facts.** Connected real-time intent explanation to
  active session state via `session_id`, supplying multi-turn conversation
  depth, previous conversational acts, and active commitment status to the
  deterministic intent classification kernel.
- **Strict Theme Adaptation.** Full compliance with Vak core theme tokens
  across Light, Dark, Warm, and High Contrast palettes with zero hardcoded
  colors and native collapse/expand HUD controls.

## 3.0.28 — 2026-09-08

### Admin Console Bus + Sandbox Coverage

- Add Operations Center Bus status panel (backend, connection, encryption,
  metrics grid with byte formatting).
- Add `#/operations/sandbox` section with Environments, Candidates (with
  Promote-to-workspace action), and Promotion ledger sub-views.
- Add Event bus panel to Settings with NATS URL, JWT, NKey seed, and
  workspace secret env var fields; credential-never-returned semantics.
- Show per-session sandbox execution events in RunDetail.
- Add sandbox topology node to the system map inspector.

## 3.0.27 — 2026-09-08

### Fresh Startup & Workspace Panel Controls

- Start web and desktop with a fresh task canvas instead of reopening the
  latest persisted task.
- Keep the workspace More menus visible above the header and add a shared
  right-panel expand/collapse control.

## 3.0.26 — 2026-09-08

### Sandbox Acceptance & Documentation

- **Sandbox crate unifies execution backends.** Added `crates/vak-sandbox`
  as a workspace member, consolidating the Seatbelt (macOS) and Landlock
  (Linux) sandbox implementations behind a single trait surface used by
  the broker, the execution engine, and the runtime permission model.
- **Full-stack release artifact matrix.** Release and install runbooks
  (`docs/release-and-install.md`, `docs/design/32-release-engineering.md`)
  verified across the macOS native bundle (`.app` + DMG), the headless
  Linux Docker image, and the web surface, with end-to-end checks on both
  platforms.

## 3.0.25 — 2026-09-08

### Retired Tool Lifecycle & Plugin Cleanup

- **Retired tools registry.** Added `crates/vak-tools/src/retired.rs` — a
  compiled-in, append-only list of tool names that no longer exist
  (`python_eval`, `react_preview`). Each entry carries the retirement
  version and the replacement instruction (`bash`). This is the single
  source of truth for tool retirements; there is no second list.
- **Plugin retirement detection.** `PluginStore::retired_plugins()` scans
  installed plugin packages for `SKILL.md` files referencing retired tool
  names. A startup warning fires in `Core::new_with_trust`; `vak doctor`
  flags them as a failed check; `vak doctor --repair` and `vak setup seed`
  remove them automatically.
- **Skill validation at discovery.** `skills::validate()` rejects any
  `SKILL.md` whose description or body text backtick-references a retired
  tool name, so stale skills never enter the model's capability contract.
- **Tool name aliasing.** `canonical_tool_name()` redirects retired tool
  names to `bash`, so a model that still reaches for `python_eval` gets a
  `Correctable` error (missing `command` arg) instead of `unknown_capability`
  and can self-repair.
- **Config scrubbing.** `vak_config::prune_plugins_network_allow()` removes
  stale entries from `[plugins] network_allow` when a retired plugin is
  removed.
- **Admin API.** `GET /plugins/retired` lists retired plugins;
  `DELETE /plugins/retired` removes them all and prunes config.
- **Local install remediation.** Removed `python-sandbox` and
  `react-sandbox` plugin packages (v1.0.0 and v1.0.1) that survived the
  3.0.21 sandbox unification and still instructed the model to call
  `python_eval`/`react_preview`. Fixed `permission_mode` from `full-access`
  to `workspace-write` to restore sandbox enforcement.

## 3.0.24 — 2026-09-08

### Sandboxed Broker Wrapping, Env Scrub Security & Auto-Approval Verification

- **Brokered execution command wrapping for containerized sandboxes.**
  Added transparent command wrapping for `SandboxTarget::ToolCommand` targets in `crates/vak-tools/src/broker.rs`. Host-level workers remain protocol adapters while bash commands inside tool arguments are securely encapsulated for Docker-style execution sandboxes.
- **Defense-in-depth subprocess environment scrubbing.**
  Hardened operational variable allowlist predicate in `crates/vak-tools/src/bash.rs`. Verified complete rejection of provider credentials, cloud API tokens, proxy credentials, and authentication keys, backed by comprehensive end-to-end unit tests.
- **Fail-closed DenySandbox and Seatbelt read-only traits.**
  Verified `DenySandbox` exit 126 error reporting with full command redaction, and enforced invariant that `read_only_variant()` on Seatbelt profiles strips all `file-write*` entitlements.
- **Comprehensive auto-approval policy matrix verification.**
  Implemented regression tests in `crates/vak-agent` proving that `AskSource::Rule` and `AskSource::CircuitBreaker` cannot be bypassed by `AutoApprove`, and that `ApproveSafe` requires active container or Seatbelt sandbox enforcement before authorizing bash execution.

## 3.0.23 — 2026-09-07

### Distributed Event & Message Fabric (`crates/vak-bus`, Doc 53)

- **Green-field distributed event fabric for multi-agent fleets.**
  Added `crates/vak-bus` providing high-throughput, zero-trust event distribution and durable
  asynchronous work queues for thousands of concurrent agents across local, headless Linux, hybrid,
  and multi-cloud environments.
- **Universal CloudEvents 1.0 with Merkle Causal Lineage & W3C Tracing.**
  Standard CloudEvents 1.0 envelope (`id`, `source`, `type`, `time`, `subject`, `data`) augmented with
  W3C Distributed Tracing (`traceparent`, `tracestate`, `child_span`) and SHA-256 Merkle causal hash chaining
  for cryptographic provenance and causal verification across multi-agent workflows.
- **Defense-in-depth zero-trust envelope security.**
  End-to-end authenticated AES-256-GCM encryption with HKDF-SHA256 key derivation and 12-byte random nonces.
  Binds event ID, type, workspace, and sequence via Authenticated Additional Data (AAD), failing fast on tampering.
- **Deterministic hierarchical subject algebra & least-privilege ACLs.**
  Structured taxonomy for broadcast events (`vak.events.<ws>.<sess>.*`), durable work queues
  (`vak.work.<ws>.<role>.task`), direct inboxes (`vak.agent.<ws>.<agent>.inbox`), and dead letters
  (`vak.dlq.<ws>.<agent>.failed`). Governed by role-based `AclPolicy` rules.
- **Lock-free telemetry, W3C context propagation & Dead-Letter Queues.**
  Atomic performance counters (`BusMetrics`), automated cross-process W3C trace propagation, and
  dead-letter event routing with poison-pill quarantine and diagnostic logs.
- **Production NATS Core + JetStream engine and zero-dependency InMemoryBus.**
  Native asynchronous `NatsBus` with pull-consumer work claiming, ACK/NAK semantics, alongside a
  real, zero-dependency `InMemoryBus` for local development and CI testing. Zero mocks or stubs.

## 3.0.22 — 2026-09-07

### 2026 State-of-the-Art Sandboxed Execution Engine across CLI, Desktop, and Headless Linux

- **High-fidelity ANSI terminal & progress bar streaming.**
  Added streaming line folding for carriage returns (`\r`) in `crates/vak-tools` and `crates/vak` so
  progress bars (`npm`, `pip`, `cargo`, `docker`, `curl`) update smoothly in-place without log noise.
  Implemented a lightweight 16-color ANSI-to-HTML parser in the SolidJS Workbench panel with auto-scrolling
  to bottom during streaming.
- **Sub-second real-time process telemetry.**
  Added `SandboxEvent::ProcessTelemetry` emitting elapsed execution time (ms) and resident set size (RSS)
  memory every 500ms during execution. Probes `/proc/<pid>/statm` directly on Linux and Docker containers,
  and queries `ps -o rss=` on macOS. Displayed live as RAM and timer badges in the Workbench header and CLI
  execution box footer.
- **Quarantined scratch artifact auto-detection & in-situ preview.**
  Intermediate scripts, assets, and data files are isolated in `.vak/scratch/`. Automatically detects newly
  generated scratch files upon command completion, classifies MIME types, and provides in-situ visualizers:
  sandboxed HTML/web apps in isolated `<iframe>` with `srcdoc`, image inspection with click-to-zoom modal,
  and syntax-formatted code and data viewers.
- **Interactive process controls.**
  Added one-click "Stop Process" in Desktop and Web Workbench calling `api.cancelRun()` to cleanly terminate
  commands via process group signaling (`SIGTERM` -> `SIGKILL`), along with one-click "Copy Output".
- **Cross-surface parity.**
  Full feature parity verified across CLI terminal box, Desktop App (`/Applications/Vak.app`), Web UI,
  and Headless Linux on Docker (`vak:local`).

## 3.0.21 — 2026-09-07

### Unified Bash Sandbox with Workbench Live Observability

- **Unified neutral `bash` sandbox execution.** Replaced fragmented language-specific runners
  (`python_eval`, `react_preview`) with a single universal, stack-neutral `bash` execution engine.
  Any programming stack (Python, Node/TypeScript, Rust, Go, shell scripts) can be run and installed directly.
- **Real-time execution streaming in Workbench.** Added live event streaming (`SandboxEvent::Stdout`,
  `SandboxEvent::Stderr`, `ExecutionStarted`, `ExecutionFinished`) to `vak-tools` and `vak-agent`,
  broadcasting real-time execution telemetry directly to the client Workbench panel.
- **Package installation tracking & scratch isolation.** Automatically tracks package installations
  (`pip`, `npm`, `cargo`) and provides complete visibility into execution lifecycle, duration, and
  quarantined intermediate artifacts in `.vak/scratch/`.
- **Complete legacy cleanup.** Thoroughly removed obsolete tools, runtimes, plugin seeds, and
  presentation adapters across permissions, config, core, delivery, server, and client UI.

## 3.0.20 — 2026-09-07

### Sandbox package installation robustness & Docker build reproducibility

- **Default sandbox network seeding for package installation.**
  `seed_shared_capabilities` now automatically seeds `[plugins] network_allow = ["python-sandbox", "react-sandbox"]`
  into the Shared layer (`~/vak-home/.vak/config.toml`) via `seed_plugins_network_allow_if_empty`
  and `seed_global_plugins_network_allow_if_empty` in `vak-config`. The LLM can dynamically install
  any required Python packages (via `python_eval`'s `install_packages`) and load React preview assets
  out-of-the-box without manual configuration, while preserving strict operator governance via `network_deny`
  and channel policies.
- **Robust pip argument quoting and diagnostics.**
  `python_runner` now properly shell-quotes each package argument in `pip install` individually
  (handling specifiers like `pandas>=2.0` without shell redirects) and falls back to stdout if stderr is
  empty to guarantee complete diagnostic reporting.
- **Reproducible Docker container builds.**
  The Dockerfile cargo build now enforces `--locked` dependency resolution and receives `VAK_GIT_SHA`
  as a build argument, guaranteeing container binaries match the workspace lockfile and carry commit provenance.

## 3.0.19 — 2026-09-07

### Connected sandbox runtimes: python_eval and react_preview

- **Connected write→execute→debug→result loop.** The system prompt now
  establishes `python_eval` and `react_preview` as first-class sandbox
  runtimes with an explicit write→execute→debug→result cycle, and documents
  the shared scratch-space connection between them: Python processes data and
  generates figures, and React components consume that data via props or
  scratch files to render interactive visualizers inside the chat and the
  Preview Dock.
- **Dual-artifact scratch management for react_preview.** Components are
  written as `.tsx` source and compiled into an isolated `.html` preview under
  `.vak/scratch/previews/`, with `component_path` support. Window `error` and
  `unhandledrejection` diagnostics are injected into the preview HTML so
  client-side failures surface inside the sandbox instead of vanishing into a
  blank frame.
- **Headless Python + Matplotlib.** `python_eval` runs headless (Agg) and
  auto-captures figures into `.vak/scratch/python/`; `python3 -m pip install`
  is supported with cross-platform package management, and `ModuleNotFoundError`
  now carries targeted debug hints. All artifacts are quarantined to
  `.vak/scratch/python/` and never contaminate the project source tree.
- **Quarantined, scrubbed-execution runtimes.** Both runtimes execute in
  disconnected process groups with fully scrubbed environments (`env_clear`),
  zero parent API keys or secrets leaked, and a strict CSP
  (`connect-src 'none'`) on previews — `pip install` is network-gated by the
  same privileged network model as MCP (invariant 35).
- **Broker-level tool normalization + canonical resolution.** `vak-agent`
  normalizes tool calls through a single normalization seam, and `vak-tools`
  resolves built-in runtime tool names canonically so the tool vector, the
  derived name packet, and the per-channel filter all describe the same
  executable surface (invariant 30).
- **Permission + delivery signals.** Runtime admission now carries
  permission descriptions in `vak-permission` and the corresponding
  presentation signals in `vak-delivery`.

## 3.0.18 — 2026-09-07

### Plugin runtime admission unifies on the store, and network knobs speak one language

- **One admission decision point, one tool set.** `built_in_runtime_plugin_tools`
  is now the single place the built-in `python_eval`/`react_preview` runtime
  tools are decided; the plugin-tier and tool-name-tier filter sets agree, so
  the tool vector, the derived name packet, and the per-channel filter all
  describe the same executable surface (invariant 30).
- **Network is a strict lattice, decided once.** Config-level `network_deny`
  beats `network_allow` (entry wins, `*` wins), an absent allowlist denies by
  default, and the channel overlay can only narrow egress — never grant it.
  The Settings toggle persists its grant atomically and the server refuses a
  non-empty grant for an untrusted project layer.
- **The plugin store owns package lifecycle, and the runtime follows it.**
  Enable/Disable/Remove on a runtime package now changes the executable
  surface, not just the listing: the runtimes live and die with their owning
  plugin. A store that was never written is vacuous (config alone decides),
  so pre-setup homes and synthetic test homes stay deterministic. `[plugins]`
  remains the privileged overlay above.
- **`react.preview` becomes a real typed result.** The preview tool emits a
  semantic envelope, the exact single-render invariant holds (projection emits
  one structured item), and `count` equals the model-visible projection length.

## 3.0.17 — 2026-09-06

### Multi-turn continuity, conversational drift handling, and local context discovery

- **Dynamic `<conversation_thread>` projection**: When a session has multiple turns (`revision > 1`), `derive_messages()` projects the chronological request timeline across turns alongside explicit rules directing the model to follow intent across conversational drifts without complaint or resistance, resolve references against earlier turns, and prohibiting clarification as an exception-handling escape hatch.
- **Historical tool result pruning**: Tool execution results prior to the active turn exceeding 600 characters are safely pruned down to 300 characters plus summary metadata in runtime memory projections, preventing past search dumps and stack traces from crowding out context while keeping on-disk JSONL ledgers 100% verbatim.
- **System prompt operating rules**: Updated core `operating_rules` in `system-prompt.md` to universally enforce drift adaptability, reference resolution, and prohibit using demands for manual input/URLs as an excuse to avoid using tools.
- **Provider model context discovery for Ollama**: Added native `/api/show` model context discovery in `vak-llm` inspecting `num_ctx`, `details.context_length`, and `model_info`. Local models default conservatively to 8,192 tokens instead of 128,000, ensuring context compaction triggers reliably on local hardware.

## 3.0.16 — 2026-09-06

### Sandboxed Python & React runtime plugins with chat presentation and preview dock integration

- **Sandboxed Python execution (`python_eval`)**: Implemented `PythonTool` running in isolated process groups with fully scrubbed environments (`env_clear`, zero parent API keys or secrets leaked). All generated scripts and pip installations are quarantined in `.vak/scratch/python/site-packages/` and never pollute regular project files.
- **Sandboxed React component preview (`react_preview`)**: Implemented `ReactPreviewTool` compiling self-contained, offline HTML preview bundles with React 18, Babel Standalone, and Tailwind CSS. Quarantined in `.vak/scratch/previews/` with strict Content Security Policy (`connect-src 'none'`) and built-in ErrorBoundary.
- **Multi-tier network governance**: Follows the privileged MCP network model (`network = true | false`). When network access is disabled, `pip install` package management is blocked with a typed error and React preview CSP strictly enforces `connect-src 'none'`. Channel overlays (`plugins_network_deny`, `plugins_deny`) enable remote bot lockdown.
- **Shared capability seeding**: Automatically seeds `python-sandbox` and `react-sandbox` plugins along with their skills (`python-execution`, `react-component-preview`) into `~/vak-home/.vak/plugins/packages/` so every new workspace inherits them seamlessly.
- **Interactive chat rendering**: Created `UIPreviewCard` rendering live sandboxed component previews directly inside chat turns with reload, popout, source inspection, and a direct "Right Bar" open action.
- **Dual-mode Right Bar Preview Dock**: Upgraded `PreviewPane` to seamlessly toggle between live sandboxed Component Previews and local dev servers from `.vak/launch.toml`.

## 3.0.15 — 2026-09-06

### Reconfirmation modals, workspace removal safety, and composer layout stability

- **Accidental removal protection with `ConfirmModal`**: Added a dedicated confirmation modal with clear action semantics, danger badges, and explicit assurances that local project files and git commits remain intact on disk before forgetting a workspace, archiving a task, or permanently deleting ledger history.
- **Graceful in-flight cancellation**: Removing a workspace or archiving a task automatically and safely halts any active agent runs in that workspace or session, recording partial outputs cleanly without data corruption.
- **Automatic canvas clearing on removal**: Archiving an active task or removing an active workspace immediately switches the canvas to the next available task or opens a fresh clean session rather than leaving stale chats rendered.
- **Active workspace removal support**: Updated server and desktop host ports to seamlessly switch the active workspace to an available workspace or default home when removing the active directory, clearing both desktop and web recent lists.
- **Workspace and session context menus**: Right-click context menus on sidebar workspace rows ("Switch to workspace", "Copy folder path", "Remove from list") and session rows ("Open task", "View transcript", "Copy task ID", "Archive task", "Delete permanently").
- **Composer layout stability**: Eliminated composer toolbar distortion when sending prompts by enforcing non-wrapping horizontal alignment, moving detail density controls to the status bar (`st-density`), compacting token meters into a tight pill, and morphing the Send button directly into a Stop button within the exact same 28x28 footprint.
- **Discoverable slash palette and skills button**: Added a dedicated `[/] skills` button to the composer toolbar alongside a unified slash palette supporting built-in commands (`/clear`, `/btw`, `/compact`, `/diff`, `/terminal`, `/files`, `/help`) and registered skills with full keyboard navigation.

## 3.0.14 — 2026-09-06

### Composer toolbar alignment and control streamlining

- **Streamlined composer controls**: Removed the standalone spark/goal control from the primary input bar to keep focus on direct turn entry.
- **Precision vertical alignment**: Standardized all bottom toolbar interactive elements (workspace selector, permission mode, model picker, file mention, file attachment, density selector, token counts, context ring, and send button) to a uniform 26px baseline plane with centered flex distribution.
- **Attachment buttons**: Clean, dedicated `@ files` mention and `+ Attach` file picker buttons with consistent styling.

## 3.0.13 — 2026-09-06

### Chat surface UX overhaul and web workspace management

- **Progressive streaming markdown**: Dynamic unclosed code block detection renders headings, bold, and code blocks in real time with debounced Shiki highlighting, preventing post-turn DOM swap and layout reflow.
- **Collapsible tool cards**: Completed tool calls collapse by default to preserve transcript scannability, with full outputs expandable on click.
- **Message action bar**: Floating hover bar provides 1-click response copying with checkmark feedback and prompt editing for user messages.
- **Composer ergonomics**: Prompt history navigation (`↑`/`↓`), multi-file drag-and-drop code attachments, image preview thumbnails, and mobile virtual keyboard Enter handling.
- **Quick model switcher**: Inline dropdown in composer populated dynamically from discovered provider models.
- **Web workspace management**: Added `WorkspacePickerModal` backed by `DirectoryPicker` allowing web users to browse server directories and add/switch workspaces directly from the sidebar.
- **Workspace lifecycle clarity**: Workspace removal is strictly a presentation decision ("forget") that never deletes local files or directories on disk.

## 3.0.12 — 2026-09-06

### System-driven tool-failure recovery (no more silent `produced`)

- Add a typed `ToolErrorKind` classification for tool failures (single
  authority shared by the recovery hint, the repair budget, and the outcome
  gate). Webfetch byte-cap and MCP/mcp broker omissions now classify
  `Correctable`.
- `McpTool` schema is contract-honest: `oneOf` list/call branches with
  `additionalProperties: false`, so the model no longer omits `server`/`tool`
  by trusting an incomplete `required` list.
- The schema validators (`vak-tools` contract and `vak-mcp`) now enforce
  `oneOf` (exactly one branch) and `additionalProperties: false`.
- Per-run repair budget in the agent turn loop: failed attempt -> hint;
  second consecutive correctable failure -> system-authored, schema-resurfacing
  directive; third -> bounded degraded stop (honest "could not repair" answer),
  instead of spinning on a fault or signing off a fabricated result.
- Outcome linkage: a turn that ends with unresolved correctable tool failures
  and no successful receipts is downgraded `Produced` -> `Unknown`.

## 3.0.11 — 2026-09-06

### Provider and runtime contract audit

- Harden provider routing, fallback identity, watchdog cancellation, discovery freshness, child/flow policy propagation, and outcome evaluation evidence.
- Regenerate shipped frontend bundles and add focused regression coverage for the audited runtime paths.

## 3.0.10 — 2026-09-06

### Admin feed workflow UX

- Fix feed source add/edit/delete flows to use stable source identities and
  responsive, keyboard-accessible controls.
- Improve wizard hierarchy, field alignment, mobile behavior, and close/cancel
  affordances.

## 3.0.9 — 2026-09-05

### Scoped feed system

- Add global and workspace feed scopes with stable source identities,
  permission checks, provenance, quarantine, search, alerts, and delivery
  receipts across the server, pipeline, MCP, scheduler, and UIs.
- Rebuild shipped frontend bundles and add feed contract/concurrency coverage.

## 3.0.8 — 2026-09-05

### Harness lifecycle hardening

- Revoke active permission leases immediately when a session's effective mode changes, including warm pooled gateway cores.
- Deny forwarded approval gates during revocation and reject late replies.
- Refuse dollar-capped dispatch for models whose pricing is unknown instead of allowing an unenforceable budget.
- Persist child-run terminal markers and recover completed children into verification after a process restart.
- Recover orphaned commitment episodes, wake commitments after fulfilled dependencies, and classify tool-only or empty turns as stalled.
- Add regression coverage across permissions, gateway approvals, budgets, commitments, child sessions, subagents, flows, and full workspace execution.

## 3.0.7 — 2026-09-05

### Stream lifecycle management and multi-task scale

- Prune inactive Server-Sent Events (SSE) connections when switching between tasks or dismissing panes.
- Prevent HTTP/1.1 connection pool exhaustion in desktop WebKit and browser runtimes, allowing seamless navigation and creation across $N$ chats.
- Reconcile background running streams and disconnect completion listeners on task completion.

## 3.0.6 — 2026-09-05

### Enhanced renderer visualization and canvas harmony

- Integrate native Mermaid SVG diagram generation into Markdown and presentation views.
- Expand multi-theme syntax highlighting support across code canvas blocks.
- Refine canvas harmony and layout transitions for rich outcome-first desktop and web presentations.

## 3.0.5 — 2026-09-05

### Capability-driven presentation planning

- Separate recipe composition intent from per-output renderer resolution.
- Record surface-aware renderer decisions and mixed-output audit details.
- Make recipe signal/type matching deterministic and reject ambiguous semantic
  type ownership.

## 3.0.4 — 2026-09-05

### Universal call-contract admission

- Validate every model-proposed tool call against its admitted schema before
  authorization and dispatch, independent of tool or domain.
- Record malformed calls as non-dispatching error values rather than treating
  them as provider or network failures.

## 3.0.3 — 2026-09-05

### Bounded tool context and MCP artifacts

- Bound model-visible tool input and output so oversized results cannot consume
  the entire turn context.
- Persist large MCP results as redacted, bounded artifacts with offset/limit
  reads, allowing follow-up inspection without replaying the full payload.
- Reject malformed MCP action requests before authorization and dispatch.

## 3.0.2 — 2026-09-05

### Endpoint capability routing

- Frozen provider routes now include their API dialect. Agentic OpenAI and
  OpenRouter turns select Responses before dispatch, so function tools and
  reasoning do not accidentally travel through Chat Completions.
- Primary and fallback legs preserve the admitted dialect, preventing retries
  from silently changing request semantics.

## 3.0.1 — 2026-09-05

### Turn capability assembly

- Unified live capability selection, prompt projection, tool schemas, hooks,
  MCP, skills, plugins, and child execution around one turn contract.
- Added append-only per-turn capability bindings and immediate revocation
  checks across authorization and approval waits.
- Routed standalone CLI flows, eval runs, and heartbeat admission through the
  public prepared-turn API.

## 3.0.0 — 2026-09-05

### Capability assembly consolidation

- **Root cause fix:** MCP aliases, hooks, skills, and `flow`/`work` tool
  admission previously assembled through four divergent code paths in
  `vak-agent` and `vak-core`, each skipping different filter stages.
  MCP aliases bypassed reach + domain-slice entirely; the `work` tool
  definition was appended *after* all filters had already run; hooks from
  plugin capabilities were never filtered by domain slice.

  A single `TurnCapabilities::build()` pipeline in `crates/vak-core/src/
  capability/turn.rs` is now the **only** assembly point for all four
  capability kinds. Every kind passes through the same four stages —
  channel policy → reach standings → frozen contract → domain slice —
  before reaching the agent or the model.

- `Agent::tool_definitions()` in `vak-agent` no longer appends `work`
  unconditionally when `work_mode == Managed`; it now checks
  `flow_dispatcher.is_some()`, which is only set when the `flow`
  capability survived the domain slice.

- Removed dead code: `Core::mcp_aliases_for_session`, the old
  `mcp_aliases_from_inventory` free function, and
  `Core::hooks_from_capabilities`. These are superseded by
  `turn::mcp_aliases_from_inventory` and `TurnCapabilities::build`.

- Added 10 end-to-end tests covering all four capability kinds through
  the full pipeline (tool, MCP, skill, hook) — both survive and blocked
  cases, plus a combined greeting test and a live-data query test.

### Rendering system fix

Structured ```vak fences in assistant answers and tool results are now
projected to readable text on all chat surfaces (Telegram, Slack, Discord),
not shown as raw JSON. The worker validates each fence against a merged skill
registry (builtins + plugin-contributed) and renders a deterministic text
projection with an inspectable JSON appendix. Plugin-contributed semantic
types are recognized through `SkillRegistry::find_by_type` and optional JSON
Schema validation.

`DeliveryPosture` (cadence × urgency) is now wired into the delivery path.
Packets with `HoldUntilComplete` or `HoldForDigest` disposition are enqueued
to the outbox without immediate delivery; the replay loop skips held packets
until their posture resolves to `Send`. Approvals and interrupt-urgency
packets always send.

The SSE snapshot endpoint now uses the Core's merged presentation planner
(builtins + plugin recipes) for live sessions.

## 2.3.1 — 2026-09-05

A review of the schema-v2 rendering pipeline (2.3.0) turned out to be
auditing an earlier snapshot of it — its five findings about AST
flattening, recipe validation, chart integrity, and delivery-surface
formatting no longer matched what shipped in 2.3.0. Verifying that
surfaced one real, currently-failing regression, plus the actual gap
behind "rendering doesn't do enough": nothing connected a tool's own
result to the recipe/renderer vocabulary that already existed.

### Recipe selection was rejecting almost everything

`RecipeCatalog::choose_for_types` had a guard that rejected every
signal-matched recipe whenever no structured candidates had already
been validated — exactly the state of plain-text classification (no
`\`\`\`vak` fence, no tool JSON). That silently broke selection of
`research.synthesis`, `weather.forecast`, `coding.diff_inspector`,
`coding.test_report`, `terminal.session`, `lifestyle.culinary_recipe`,
`data.spreadsheet_grid`, and `data.multi_chart` for most plain-text
answers, falling back to `answer.basic` instead. Two existing unit
tests were already red on `main` because of it; the redundant guard is
removed.

### A tool's own result can now render richly, with nothing hardcoded to it

Only an assistant's final text was ever scanned for structured data
(inline `\`\`\`vak` fences, bare URLs). A tool's raw result — the thing
most likely to actually carry a chart, a metric, a grid, a test report
— was only ever shown as opaque progress text, except for
`write`/`edit`/`apply_patch`/`imagegen`, matched by literal tool name
for artifacts. Extending that per-name pattern to every domain would
mean a hardcoded branch per tool, forever.

Instead, `structured_outputs_from_text` — the same self-declared,
schema-validated contract a model already uses inline — now also
recognizes a tool result that is itself a bare
`{"semantic_type": ..., "payload": ...}` envelope, with no Markdown
fence required (a tool's result is rarely Markdown to begin with).
Nothing here inspects a tool's name, and no type is ever guessed from
a payload's field names; a result with no declared `semantic_type`
still renders as plain text, same as before.

For the much larger set of tools we don't control — a weather API, a
ticketing system, any third-party MCP server — nothing can make them
adopt our envelope, and rewriting their actual output to force it
would reach past our own boundary into the same value the ledger
records and a later turn's model reads. So a new `ResultAdapter` /
`AdapterRegistry` (`vak-delivery::adapters`) puts provider-shape
translation at render composition instead: an adapter recognizes one
provider's specific, known response shape and projects a candidate for
that one rendering pass only, still subject to the same
`SkillRegistry::validate` as everything else. `built_in_adapters()`
ships empty — no real third-party provider is wired into this
codebase yet, and inventing one to demo would fabricate a shape
nothing actually returns.

Covered end-to-end across five personas' tools — a general user's
weather metric, a developer's test report, a knowledge worker's
research synthesis, a data analyst's grid, and a chart consumer's
telemetry — run through `snapshot()` with tool names the pipeline has
never seen, proving the mechanism is domain-neutral rather than
demonstrating it only at the unit level.

## 2.3.0 — 2026-09-05

- Rebuilt rendering and delivery around the schema-v2 semantic AST.
- Added deterministic recipe validation, stream snapshots, safe Mermaid source
  preservation, richer charts/grids/recipes, and regenerated web bundles.
- Added release and installation verification for macOS, Linux, and Docker.

## 2.2.1 — 2026-09-04

2.2.0 rebuilt the capability subsystem and still answered "I do not have a
tool that can provide real-time weather information" on the desktop, with a
connected, admitted Tavily server attached. This is that fix.

### Admission decides when a prompt may freeze, not each surface

The admitted packet was correct all along — 26 capabilities, `mcp` present,
`tavily` present, the per-turn slice removing nothing. What was wrong was the
**prompt**: it froze the name-only MCP line with no catalog behind it, and a
model handed a server name and no tools reasonably concludes it cannot reach
anything.

The cause was that "when may a prompt be frozen?" was answered in each
surface's startup code, three different ways:

| Surface | Waited for discovery? |
| --- | --- |
| CLI (`exec`, `flow exec`, `plan`) | `warm_mcp_bounded(2s)` — yes |
| `vak serve` (gateway, web `/app`) | `warm_mcp()` — fired, never waited |
| Desktop | nothing at all |

So the same question produced a different packet depending on where it was
asked, and on the desktop it was not a race but a certainty: every session
froze name-only, permanently, because nothing re-renders a frozen contract.

Three changes, each **removing** a second way to do one thing rather than
adding a fourth (invariant 30):

- **`Core::admitted_capabilities` is the only wait.** All three per-surface
  warm calls are deleted. Bounded by `ADMISSION_BUDGET`, so an optional
  integration still cannot block a turn indefinitely.
- **The catalog renders from the admitted packet**, where the registry's
  probe puts it, instead of from a parallel `mcp_cache` inventory that could
  — and did — disagree with the packet beside it.
- **`rebound_capabilities` re-renders a live session's admitted set** at each
  turn boundary, matching on typed `(kind, name)` identity. Admission is
  unchanged; only the description of an already-admitted capability is
  refreshed. This is doc 41 invariant 6 in practice: no restart, no rotation,
  and a session admitted during discovery stops being degraded for life.

`capability_descriptors` also stopped hand-building descriptors a second
time. The two constructions had already drifted — the same built-in tool came
out stamped `"vak-core"` one way and `"builtin"` the other — which a gateway
test caught as a contract mismatch. It now projects from
`CapabilityProvider::declare`, like the registry does.

### `doctor` no longer cries wolf

The `capability health` check added in 2.2.0 reported every diagnostic as a
failure, and on a real machine the first thing it did was go red for a hook
the operator had deliberately disabled. "Configured but not usable" covers
two different things, and reporting a chosen state as a failure trains people
to scroll past the check — which costs more than the noise saves, because a
silently unreachable MCP server is exactly what it exists to surface.
Diagnostics now carry `deliberate`, and the check fails only on breakage
while still counting the rest.

## 2.2.0 — 2026-09-04

Capabilities become a live subsystem, and the release pipeline starts proving
what it ships.

### One capability registry, no restarts, no rotation (`docs/design/41-capability-registry.md`)

vak was running a **process-per-session lifetime model inside a daemon**.
Every mechanism in the capability subsystem was edge-triggered — discovery
warmed once, a prompt frozen once, a connection opened once, a catalog cached
once, with a retry guard written so that caching a *failure* counted as
success. There was no loop behind any of it, so a missed edge was permanent
and uptime turned small races into dead integrations.

The visible symptom: a configured, connected search server was unreachable
for an ordinary question about the weather. Nothing errored. The agent
answered from memory and sounded certain, and `doctor` stayed green
throughout, because the diagnostics that explained the failure were rendered
only into the system prompt — the model was told, and the person who could
repair the configuration was not.

All five kinds (tool, skill, MCP server, hook, command) now share one
registry, one reconcile loop, and one projection:

- **Capabilities declare what they serve.** A `serves` vocabulary
  (`live-data`, `web`, `filesystem`, …) replaces the static table that mapped
  each act to built-in *tool names* — a shape that could never mention a
  capability the operator had installed, so every integration needed a
  harness edit and only got one after somebody reported a wrong answer.
  Adding an integration now edits nothing. Undeclared capabilities are never
  narrowed away, and domains are never guessed from tool names.
- **Usability is a state machine, not data.** A failed probe carries a
  reason, a remedy and a `retry_at` with exponential backoff. It is never a
  catalog entry — the old code stored it as a tool literally named `error` —
  and never blocks its own retry, so a server that was down at boot rejoins
  on its own.
- **One level-triggered reconcile loop** replaces every warm, cache and
  per-turn filesystem walk. Hints (filesystem, config, an MCP
  `notifications/tools/list_changed` that the transport used to discard) make
  it run sooner; a ticker makes it run anyway. A dropped hint costs one tick
  of latency and never costs correctness.
- **Turn-atomic epochs replace session rotation.** The registry publishes
  immutable versioned snapshots and a *turn* binds one for its whole
  duration, so a session alive for weeks picks up a skill added today at its
  next turn — with no restart and no rotation. Audit strengthens: every turn
  records the epoch it ran at.
- **Additions land at the next turn boundary; revocations land immediately**
  and fail closed. Dispatch checks availability against the bound epoch and
  authorization against current policy.
- **One report** behind the model's standing section, `doctor`'s new
  `capability health` check, and the console. `doctor`'s extension counts move
  from raw config to the effective set, so plugin-contributed hooks and
  servers stop being invisible to the operator.

MCP transport also pools connections behind a liveness check with an idle
TTL, and probes concurrently under a 10s budget so N unreachable servers cost
the max rather than the sum.

### The release pipeline proves its own artifacts

Every gate verified the *tree* and nothing verified the *binary*.

- **The tag-triggered release workflow could never succeed.** It fires on
  `push: tags: v*`, checkout materialises that tag, and `release.sh` then
  refused to build because the tag existed — the exact tag it was asked to
  build. It now refuses only a tag pointing at a *different* commit.
- **A release now runs what it is about to publish.** `vak --version` must
  report this version and this commit, or nothing ships. This is the
  permanent answer to a release bundling a stale binary.
- **`--no-build` collected from the developer's `target/release`**, entirely
  unrelated to the gates that had just passed. It now requires an explicit
  directory and still faces the provenance gate.
- **Nothing was built `--locked` except the Docker image**, so the released
  binaries and the container could resolve different dependency graphs than
  `Cargo.lock` records.
- **The Docker image was never built or tested anywhere.** CI now builds it,
  serves it, and checks `/health` and `/app` — which is what proves the
  committed bundles actually reached the binary.
- `install.sh` documented `VAK_VERSION` from the start and never implemented
  it, so pinning a version silently installed whatever the feed served.

## 2.1.0 — 2026-09-04

The web client: the workspace surface stops being desktop-only.

### One client, three hosts (`docs/design/48-web-client.md`)

`vak serve` now serves the **full workspace client at `/app`** — the same
client the desktop app ships, in a browser. A headless Linux box is a place
to *use* vak, not only to host it.

It is one source tree, not a second client. The workspace UI moved out of
`crates/vak-desktop` into `crates/vak-client-ui` and sits behind a small
`Host` port with a Tauri and a
web implementation, chosen by a build-time alias so the web bundle never
carries the Tauri IPC layer. No component knows which host it got; they ask
`host.can(...)`, and a capability the host cannot provide is **absent rather
than broken** — there is no Terminal tab on a host without a terminal, rather
than a disabled one that cannot explain itself.

Three things blocked this and are fixed:

- **The server answered only to loopback hostnames.** `[server]
  trusted_hosts` now names the hostnames it will accept, keeping the
  DNS-rebinding defence that pinning provided. Wildcards are refused: one
  there reopens exactly the hole the list closes.
- **`vak serve --host` did not exist**, though `docs/hosting.md` had
  documented it for releases. It exists, and **refuses to start** on a
  non-loopback bind with no `trusted_hosts` — naming the setting, and
  offering the SSH tunnel that needs none of it — rather than starting and
  then rejecting every request with a 421 nobody can diagnose.
- **A dropped event stream lost the gap permanently.** Events now carry a
  sequence number, the last 1024 are retained, and a reconnect replays
  exactly what was missed — or says `resync` when the gap is wider than the
  ring, so a client is never handed a stream with a hole it cannot see. A
  phone that slept, or a closed laptop lid, is now survivable.

Auth is one login for every browser surface: `/auth/login|logout|session`
replace `/admin/login|logout`, which were **removed**, not kept alongside —
two endpoints against one cookie is two contracts that must agree forever.
The cookie is `HttpOnly; SameSite=Strict`, and `Secure` only behind real TLS,
because a `Secure` cookie over plain http is silently discarded and the
session then never persists. Cross-origin mutations are refused even holding
a valid cookie, and `?token=` is now loopback-only.

The terminal is a real shell, so it is **off by default** (`[server.web]
terminal`) and loopback-pinned even when on: every other effect the client
can reach is permission-gated, and a shell is not.

### The surface earns a phone

- A **light theme**, and `system` as the new default. All three previous
  themes were dark. Light is built warm rather than inverted, and its accent
  *darkens* — Burnt Terracotta is 2.4:1 on white and unreadable as a button
  label. Contrast is now measured in CI for every theme, not asserted.
- **Responsive to 375px.** Below 900px the sidebar and dock become overlays;
  below 600px it is a single column. The narrow target is deliberate: read,
  review, **answer an approval**, steer. An approval is a run that has
  stopped and is waiting on a person, which is the one thing that cannot
  wait for someone to reach a laptop — so it notifies, and the notification
  deep-links to the card rather than to the app.
- A **connection indicator** with four states, because "Working" over a
  network is a claim about a round trip that may not have happened.
- Installable as a PWA.

### Also

- **The commitment portfolio reached the workspace client.** The kernel
  shipped it in the admin console only; a durable obligation the runtime
  verifies is workspace information, and there was no way to see or close one
  without leaving to a second application.
- A **second, cold palette** across the presentation components (indigo,
  emerald, rose, amber, sky, slate on near-black grounds) is gone, along with
  ~90 one-off literals. Everything resolves to a token or a `color-mix` of
  tokens.
- Desktop fixes worth naming on their own: the default transcript density
  rendered **nothing at all** while a run was working; the header could say
  "Retrying" forever after one transient failure; seven CSS custom properties
  were used and never defined (removing a focus outline, a border, and a
  chart series); the integrated terminal **leaked a shell process per session
  switch**; sidebar row actions were mouse-only; no modal trapped focus; and
  one render exception blanked the entire application.

### The commitment kernel (docs/design/47-commitment-kernel.md)

Vak learns what it was asked, and what "done" means.

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

### Authority, upkeep, and the loop closing

- **Envelopes are live.** `vak grant` delegates authority to one commitment
  for a bounded time and scope; `vak revoke` withdraws it. A grant reaches the
  running turn's authority, and revocation is honoured on read rather than
  remembered, so it lands at the next authority check rather than the next
  session. No grant, at any autonomy level, lets an irreversible action past
  without a human — proven exhaustively.
- `--on-silence assume` is refused for irreversible work. A default nobody
  confirmed cannot stand in for consent there.
- **Commitment upkeep runs on its own tick**, deliberately not gated on
  `heartbeat.enabled`: heartbeat is an opt-in model pass that costs tokens,
  this is clock and filesystem work that costs none, and tying durable work's
  upkeep to an opt-in prober would mean a commitment stopped being durable the
  moment somebody switched the prober off. It wakes scheduled commitments,
  evaluates predicate suspensions for free (so "watch X, tell me when Y" costs
  nothing at all while Y stays false), applies escalation policies, and closes
  lapsed work as `expired`. A question with no policy waits forever by design.
- **Per-channel autonomy ceiling** joins the existing restrictive-only
  `ChannelPolicy` chain: a chat may cap delegation below what the workspace
  granted and may never raise it.
- **Delivery posture** decides when a packet goes out, never what it says. An
  unattended overnight run rolls its chatter into a digest instead of sending
  forty notifications; an approval and an interrupt-urgency packet are never
  batched, because a held gate is a stopped run and batching it would turn a
  question into a hang.
- **The loop closes.** When an engagement withholds a capability and the model
  then asks for that exact tool, the reading was *measurably* wrong — and the
  row names the capability to put back. Slicing does not merely improve the
  turn; it is what makes misclassification observable at all. Per-cell
  accuracy uses the routing ledger's epistemics: held / contradicted /
  unknown, Laplace-shrunk, 30-day TTL, absence a neutral prior. Abandonment
  deliberately does not count against a reading — a user who walked away told
  us the turn ended, not that it was misread.

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
