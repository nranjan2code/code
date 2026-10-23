## 8.31.2 — 2026-10-23 — Readable source links and contained answers

- Show readable labels for source links while preserving their complete destinations.
  Long source names and unbroken content wrap within the answer instead of
  widening the conversation.

## 8.31.1 — 2026-10-23 — Rich response consolidation

- Preserve prose, rich cards, exact data and result notices in source order
  across channel delivery, saved answers, shared conversations, terminal,
  voice companions and transcript exports. Discarded drafts no longer return
  through conversation replay. CLI stdout emits the accepted answer at settlement.
- Keep heartbeat findings and urgency separate from presentation warnings, so
  a quiet reply stays quiet and notices do not become extra findings.
- Render eligible Slack charts natively, keeping full data in accessible text
  and explicit static fallbacks for unsupported chart shapes.

## 8.31.0 — 2026-10-23 — Topics and turn names

- Give answered requests useful names and group related work into Topics in an
  independent step after the answer. Plain replies and follow-ups no longer
  need headings or a planning tool to acquire navigation names. People can
  rename requests and retry or name the requests currently shown in Topics.
- Keep the Topic sidebar refreshing when a workspace changes during a fetch.

## 8.30.0 — 2026-10-23 — Conversation controls and file mentions

- Keep side questions out of the main conversation after reloads and later
  turns, and record each side question only once.
- Keep command status and pause/resume/stop feedback visible after an answer.
- Preserve file mentions with spaces, punctuation and non-ASCII names using
  quoted paths, including when combined with skills and custom commands.

## 8.29.0 — 2026-10-23 — Visible progress and clearer turn navigation

- For work with several steps, the agent can show and update progress in the top semicircle and Details panel.
  If it leaves steps open, it gets one reminder to finish, explain what it
  left out, or answer with what it has. Direct answers need no list. Progress
  is separate from cards and never counts as retrieved evidence.

- Turn navigator previews use the recorded answer heading or card title.
  Untitled turns show their turn number instead of the raw request; obsolete plan projection has been removed. Agent subtitles use the same answer titles.

## 8.28.0 — 2026-10-22 — Markdown answers and runtime cards

- The runtime adds source receipts and actual file diffs after the answer,
  directly from this turn's calls and file bytes. No model card call is
  required, failed fetches supply no source, and later turns do not recopy
  these cards into their historical dialogue.

- Answers use Markdown's existing interactive tables, lists, quotes, Mermaid
  and diff code. Only chart, stats, map, calendar and controls remain offered
  card tools; all other forms remain drawable. Shipped answer skills follow
  the same contract, without model-authored layout.

- The model no longer has to lay out its answer: the `plan` tool, the plan
  check, the card it was then forced to show, and the `deliverable`,
  `replaces`, `beside_previous` and `then` fields on every card are gone.
  An answer is no longer held back for a Topics placement either. Cards
  show in the order the model made them, and an answer that rests on
  material retrieved from outside your workspace still asks for its
  sources to be checked.

## 8.27.0 — 2026-10-22

- A conversation keeps its thread: each request now carries the newest
  exchanges whole, as far as the model's measured context allows, so a
  follow-up still finds its subject after clarifying or side questions.
  What no longer fits becomes one summary of the earlier conversation,
  written once when the window fills and reused after that; summaries now
  keep each request and answer whole. Earlier requests and approvals stay
  marked as history.
- Repeating an earlier answer word for word, when that is the right answer,
  is no longer treated as drifting off topic and sent back for a redo.
- Working in another Agent's workspace keeps the surface you are using, so
  the desktop and web apps are no longer described to the model as a plain
  HTTP client.

## 8.26.0 — 2026-10-22

- Answer design: align the shared presentation guidance, surface descriptions
  and shipped skills so useful form selection follows the information instead
  of a compulsory summary/sources recipe. Skill tool hints no longer exclude
  other admitted tools. Specifically planned forms require an active accepted
  receipt of that form; replacement retires the old receipt. Preserve the
  continuous answer surface and compact file collections. Numeric-table
  feedback explains missing values and decimal syntax without silently
  dropping numerical semantics.

## 8.25.0 — 2026-10-22

- Shared web and desktop UI: composed answers use open sections with fewer
  enclosing boxes, related files share a compact collection, and copy and
  request details join the result actions. Partial, stopped and denied outcomes
  retain one visible status. Long facts stack in narrow panes; the file queue
  expands on demand. Library opens as a compact list with a grid option and
  secondary descriptions on request. Preview code stays paused by default,
  with a clearer explanation; test reports show concise recorded outcomes.

## 8.24.0 — 2026-10-22

- Proposed a general lifecycle for saved interactive work, with edits creating
  versions on one Library item, visible operation controls, and script
  execution gated by current capabilities and enforceable isolation. The
  proposal records local everyday-use findings and leaves implementation open.

## 8.23.0 — 2026-10-22

- Channel cards now render from their typed forms through one shared delivery
  path: native Telegram rich messages, Slack rich-text/table blocks and Discord
  embeds. Observation labels, nested points, source findings and exact table
  values survive; long cards continue instead of being silently truncated.
  Interactive/visual features have explicit static representations. Removed
  generic payload-field renderers and Telegram's automatic plain-text downgrade.

## 8.22.0 — 2026-10-22

- Automatic Topics now checks for a recorded organization decision before an
  answer finishes, including answers delivered entirely as cards. If the Agent
  skips it, the runtime asks it to organize the work and can require a Topics
  call. Simple answers can explicitly remain ungrouped. Human placement and
  automatic-off take precedence; organization failures preserve the answer.

## 8.21.0 — 2026-10-22

- Closing Canvas returns focus to its external opener even after switching
  conversations or replacing its view.

- Enabled packages can contribute Canvas workspaces with typed fields and domain
  commands. Saved state has revision checks and undo/redo; disabling or updating
  a plugin refuses stale operations and keeps its work. Optional pure WebAssembly
  calculations run in disposable workers without host imports.
- Canvas workspaces can share selected saved fields with the Agent through a
  logged context entry. Conversation-bound Agent tools use the same edit service;
  pending UI edits remain local until applied. The shared web/desktop launcher
  opens saved work or starts an enabled workspace.

- Canvas preserves workspace edits across tab switches, protects them from automatic
  eviction, and asks before discarding them. Conflicting edits can be compared.
  Malformed saved Canvases are refused intact; inline previews use scoped SHA-256
  identities. The plaintext Canvas conversion path is removed.
- Shared Canvas workspace registration, viewport metrics and drawing scheduling
  support owned interactive renderers across web and desktop. The divider supports
  pointer and keyboard resizing; tabs support keyboard navigation. Disposed or
  replaced renderers lose their editing lease; independent note and domain edits
  merge without carrying stale values from the other field. The real 3D
  renderer is an acceptance fixture, not an installed domain plugin.

## 8.20.0 — 2026-10-22

- The turn navigator rolls with the mouse wheel, trackpad, drag and keyboard;
  it loads history at page edges and previews accepted plan names. Partial
  history counts say loaded. Agent subtitles name the latest work instead of
  the first greeting. Automatic Topics guidance now explicitly places
  ongoing and multi-outcome work, and Topic refresh no longer loops.

- Topic navigation keeps its turn positions aligned, uses recorded plan names
  for previews, and labels paged turn counts as loaded. Opening Topics no
  longer repeatedly refetches its list. Agent guidance explicitly organises
  ongoing, follow-up and multi-outcome work while respecting automatic-off.

- Details follows current work or pins an earlier request, with Activity, Files
  and Evidence scoped to that request. Conversation history is explicit;
  unrelated commands remain unlinked instead of being assigned by proximity.
- Activity names recorded operations, folds repeated successful calls, keeps
  failures and decisions visible, and preserves open details and reading
  position during streaming. Command output, checks, Review and Undo remain
  available through command details. Stopped calls without results are no
  longer presented as successful; interleaved worker updates follow labels.
- File details read current Library state, evidence distinguishes links from
  verification, and unavailable checks are shown as unavailable. Followed
  sources, workspace promises and conversation workers state their scope.

## 8.19.0 — 2026-10-21

- Controls across the app and Admin use consistent, compact desktop sizing,
  quieter secondary actions and explicit readable labels. Touch targets remain
  larger; narrow mouse panels no longer force oversized buttons. Settings,
  Topics, Library, Canvas and document controls share the sizing rules.
- Review centres the document, keeps file navigation separate, and groups
  secondary actions, checks and comments into clear disclosures. Text changes
  appear once, Focus view keeps the document visible, and failed checks remain
  visible. Decisions stay pinned and explain what prevents applying changes.

- HTML previews distinguish embedded scripts, local script files and external
  references. Canvas and Library offer a temporary “Run page code” permission
  for isolated embedded/bundled code, disclose possible browser network access,
  and keep external script downloads blocked. Stopping, reloading or changing
  the document revokes the permission; previews start paused.
- Review dialogs escape the Details panel’s layout container, keeping the full
  review, close control and decision buttons inside desktop and phone windows.
  Long filenames and technical document locations wrap correctly. Files without
  a text preview explain how to inspect or download the saved version.

## 8.18.0 — 2026-10-21

- A synchronous channel reply waits for the turn's settlement and session
  handoff, so an immediate follow-up is not accidentally queued as steering
  for a completed run. The initial reply still arrives promptly when already
  queued work continues.

- Topics organise ongoing work under each Agent, with model-selected placement,
  human corrections, direct navigation, pinning and explicit archive/reopen.
  Automatic organisation can be turned off; Topics preserve the original
  conversation and do not change access, approvals or historical context rules.
- Doctor reports Topic record integrity, missing turn references and catalog
  indexing status through the shared CLI, terminal, web and Admin health report.

## 8.17.2 — 2026-10-21

- Follow-ups use an explicitly labelled recent reference; older or multiple
  dependencies can be reopened together, and history searches can continue
  beyond their first candidate page without an age cutoff. Word overlap no
  longer chooses a prior thread or revives its work automatically.
- Context planning reads addressed closing records and the active turn,
  records replayable selections, and uses an explicit configurable resource
  envelope when a route's capacity is unknown.
- Answers preserve source order and stable placement anchors while streaming,
  settled and in Canvas. Grouped forms get space according to their reading
  widths and stack when they do not fit. Prompt guidance favors complete,
  concise answers with each block contributing distinct information. Settlement
  drains pending live events so delayed cards cannot overwrite saved answers
  or bring back a replaced card.

- Channel replies carry their own settled outcome metadata, so concurrent
  presentation updates cannot change their notices. Completed answers that
  require no retrieved evidence no longer receive a false unverified warning.


## 8.17.1 — 2026-10-21

- Opening a composed result in Canvas now scopes it to that result group, so
  other results from the same turn do not appear alongside it.

## 8.17.0 — 2026-10-21

- Updated TLS certificate validation and trust roots to reject distrusted
  emSign intermediates (RUSTSEC-2026-0381 and RUSTSEC-2026-0382).

- Conversations open with recent turns and load earlier messages on scroll.
  Shared readers and transcript views use the same pages; explicit exports
  remain complete. Unchanged chats use revision checks, live updates affect
  the current turn, and inactive chat views release reloadable history.
- Conversation lists use catalog-backed summary pages. History reads address
  canonical sealed frames directly and preserve branch, turn and entry identities.

## 8.16.1 — 2026-10-20

- Automatic document previews block arbitrary scripts, which can send WebRTC
  traffic around fetch CSP. Origin-served previews retain only the fixed theme
  bridge; HTML controls that need page scripts now require exporting/running
  the document outside the automatic preview. Layout and native inputs remain.

- Answer images, audio, video and link thumbnails no longer contact remote
  sites automatically. External media remain deliberate links; local media
  still render. Street-map tiles require a separate click. External hyperlinks
  open in the system browser on desktop and a separate tab on the web.
  Page metadata and headers disable speculative hyperlink DNS lookup.
- Restricted turns conservatively label owner input and editable context,
  and ask before mutations or outbound actions after private and untrusted
  material meet. Preallowed tools cannot bypass this approval. Draft revisions
  raise these approvals in their parent conversation before editing a copy.
- Real tool workers protect workspace secrets and control files independently
  of telemetry context, including computed shell paths. Restricted backends
  that cannot enforce these exclusions refuse execution (currently Linux
  Landlock and Docker require further work).
- Fetch connections screen their actual DNS addresses, and rendered browsing
  confines every connection to a screened proxy. Chrome rendering currently
  refuses to launch in the measured Mac setup; ordinary fetch remains available.
  Integrity checks reject
  damaged seal manifests and incomplete inventories without repairing them,
  and report the payload classes they do not verify.

- Voice settings and Admin offer an everyday listening/speaking setup from
  your connected account, save voice and speaking style with the Agent, and
  preview before enabling microphone conversations. Provider switches clear
  incompatible model and voice choices; readiness distinguishes keys from setup.
- Gemini's dedicated transcription models use their current speech endpoint
  in verbatim mode. Gemini speech calls request no stored interaction history;
  OpenAI GPT speech receives the selected speaking style.

- OpenRouter voice support discovers its separate speech/transcription
  catalogues, offers vendor presets and model-specific voices, and links current
  prices and languages. Playback and channel bridges handle routed MP3 and
  Gemini/HeyGen PCM wrapped as WAV. Discovery failures retain retry and exact-ID setup.

## 8.16.0 — 2026-10-20
- Library files open as readable, interactive previews: HTML pages, formatted
  Markdown, CSV/TSV tables, images, audio and video, alongside the existing
  Office, PDF and saved-page viewers. Source stays available on request.
- Library content now has its own reading area, separate History and Comments
  views, a version picker, clearer file types and grouped actions. Interactive
  page inputs survive visiting history, comments or HTML source.
## 8.15.0 — 2026-10-20
- A Gemini free-tier turn that reaches the per-minute request limit now
  waits as long as Google says and carries on, instead of failing: a rate
  limit is retried whatever its message says, and only a quota the provider
  reports as daily or billing-bound ends the turn.
- Admin pages and nested tabs consistently use the shared theme colors, fonts,
  cards, controls and status styling. Conversation summaries, filters and detail
  headers regain their missing styles; narrow tables and the operations diagram
  scroll within their panels, chart labels stay readable, and the Songbird uses
  the appropriate light or dark artwork.
- Admin diagnostics and memory confirmation dialogs open correctly and retain
  keyboard focus. Full-access conversations show their unsandboxed warning,
  memory inheritance badges show one state, and aggregate voice settings no
  longer request a nonexistent Agent.


- OpenRouter turns no longer stop the moment they start: checking the
  account's free-model allowance released its admission by cancelling the
  turn itself, so a turn ended "Stopped" with nothing sent. An admission now
  ends only its own work.
- The OpenRouter Responses route no longer fails with "store: true is not
  supported on this proxy": a stateless endpoint is sent the whole
  conversation and never asked to keep a response.

## 8.14.0 — 2026-10-19

- Blueprint adds a Storage tab to explore months of conversation, card and
  artifact growth, temporary retention, backup capacity and recovery estimates
  with editable assumptions and exportable scenarios.
- A card may be part of any entry of the answer's plan, so a diff of a planned
  change or a test report of a planned check is shown instead of refused.
- The changes card shows what this turn's writes really changed in each file,
  taken from the file before and after; it no longer shows lines or counts
  the model wrote.
- A coding turn whose plan is changes, checks and call-filled cards (a diff,
  a terminal, a test report) is no longer marked a partial result just
  because it read the file it fixed.
- Search, and recall across conversations, find a question asked in other
  words: any word matches, word forms meet ("recommend", "recommendation"),
  and the closest match comes first. An empty search tells the model to try
  the earlier answer's own words before saying it is not there.
- A fetched page's text keeps each link's address as `[text](address)`, so
  a model can open a post it read about without guessing its URL.
- A turn's record keeps its whole answer, so recall finds and shows the
  answer even when it opens with an assumption or a caveat, and a repeated
  answer is caught however long it is.
- A multi-part answer plans its leading summary like any other part, so the
  summary card no longer borrows the next part's place.
- pytest, mypy and ruff keep their caches outside the workspace; a test run
  no longer leaves `.pytest_cache` behind as a saved file.
- A command that fails because its program is not installed says what is
  installed under that name (`python` → `python3`), and the shell's tool
  description names `$TMPDIR` for temporary files, as the prompt does.
- Search finds an answer by what its cards said, not only by the question
  and the pages read; the catalog rebuilds itself to include them.
- On OpenAI, Anthropic and Gemini, a tool the turn has not used yet is sent
  as its name and one line until it is called, instead of its whole schema:
  a turn's tool schemas went from about 98k to 55k characters, and every
  tool stays callable.

## 8.13.1 — 2026-10-19

- File previews that match a delivered file share its compact result card.
  Standalone file and HTML previews start collapsed and expand on request,
  with Open still available for Canvas. Secondary file actions use labelled
  icons, and previews remain visible after switching technical details.

## 8.13.0 — 2026-10-19

- Restore and sync pull reset encrypted projections declared in the state
  registry before rebuilding from imported records and keys. Repeated
  takeovers no longer reuse a catalog encrypted under replaced local keys.

- Conversation history and answer cards reconcile from one server snapshot;
  deferred or out-of-order reads no longer leave only the closing paragraph
  visible after a run settles.
- Plans can declare evidence questions. Retrieval and output receipts no
  longer certify that researched claims are supported; unresolved evidence
  stays explicit. Source discovery is declared separately from URL fetching,
  and card acknowledgements describe acceptance for presentation.

## 8.12.0 — 2026-10-19

- A conversation is only locked while something is writing it. Before, once
  the app had answered a turn in a conversation it kept it locked until the
  app restarted, so the command line, a second server or a retry elsewhere
  was refused with "currently active in Vakyartha Desktop". The message for
  a conversation really being answered elsewhere now says so plainly.

## 8.11.0 — 2026-10-19

- A plan entry can be shown as several cards, and the plan can say which
  kind of card each part is, including where the provider enforces tool
  schemas strictly. Tables ask for short cell values.
- Work receipts record how many tokens a step spent reasoning when the
  provider reports it, so they no longer say a step did not reason because
  the provider returned no reasoning text.

- An answer still forming sits in the same column and indent as the finished
  answer, so it no longer spans the whole chat and then jumps into place.

## 8.10.0 — 2026-10-19

- Standalone CLI flows and planning share the configured optional model-call
  cap, with durable accounting initialized after run admission.

- Parent and child runs stop before workspace-changing tools when their
  required checkpoint cannot be captured or stored. Failed capture does not
  mark the checkpoint as complete. Failure to preserve file contents in the
  object store is reported too.

- Run status and crash-recovery queries report corrupt history instead of
  silently skipping it. Abandonment recovery stops before writing on a read error.
- Tool wrappers preserve card declarations and workspace-change overrides; a
  contract regression detects newly omitted Tool declarations.

- Capped budget admission uses encrypted indexed totals and reservation
  lookups instead of decoding settled history. Verified corrections and
  unresolved charges keep their existing accounting rules; cached monetary
  values retain their exact floating-point bits.

- Relocating an embedded Core's data home no longer reuses budget gates
  bound to its prior home. Existing dispatches settle to their original
  ledger. Admission reads prices and caps together, preventing a concurrent
  edit from admitting a call under a mixture of two policies.

- Effect detail, filtered history and sendable-work reads use an encrypted
  row index instead of decoding the whole history. Large journal batches
  now rotate segments as they fill; verified reads open only new payloads.
- Card calls fail the turn if their acceptance decision cannot be recorded,
  and close their live call indicator with an error.

- FinOps lets the owner record a provider-verified actual charge for an
  uncertain reservation. Unresolved charges remain reserved; live work and
  unverified histories refuse reconciliation.
- Optional `[control].max_model_calls` caps provider dispatches across a
  request, its workers and flow planner. Counts survive recreation of the
  same run; retries consume the shared cap. It is disabled by default.
- Required admission, framing, authorization, work receipts and successful
  endings stop on journal failures. Library review bindings report unreadable
  history instead of presenting an empty projection.

- Account erasure refuses unreadable connection state and checks the active
  install’s Agent ledgers. Receipts disclose that older results stored without
  account-scope protection can remain readable in their conversations.

- Session spend limits survive gate recreation and restart, include delegated
  calls, and coordinate concurrent admissions. Settlement IDs reconcile
  recorded costs with pending reservations; unknown costs stay conservative.

- Connected-account tool results, including provider errors, keep both their
  displayed text and full evidence under the account scope. Failed call-effect
  recording stops the turn before it continues.

- Library lists, file cards and version comparisons report unreadable history
  or workspace files instead of silently showing missing or incomplete state.

- Canvas note conflicts show both versions and let you choose what to keep
  for each note. Saving choices preserves unrelated notes and edits typed
  while saving; stale choices require another review.

- Ability tables infer columns from every recorded row, keeping later-only
  fields. Library review acceptance refuses unknown or unreadable file history
  instead of treating it as proof that a draft is safe to apply.

- Ability-backed tables and charts use the exact recorded call selected by
  the answer. Failed or unavailable calls no longer substitute an older
  successful result. Saved pages read captured file versions through their
  authorized page share instead of the currently open conversation.
- Credential writes preserve an unreadable encrypted store, publish presence
  metadata atomically, and report incomplete install cleanup for retry.
  Pending day-budget reservations survive restart and concurrent admissions.
- Charts support explicit numeric and time spacing and leave gaps for missing
  observations. Literal prose markers survive cleanup, and answer outlines
  reflect cards that fulfill planned text sections. Invalid adaptive displays
  retain the original answer. Live text frames send deltas and defer repeated
  Markdown parsing until completion.

- Unacknowledged Canvas edits can recover after a reload in the same browser
  tab. The browser stores encrypted checkpoints only; unlocking after reload
  requires connecting and signing in. Failed unlocks preserve the checkpoint
  and pause sync. Storage failures remain visible, and cleanup happens only
  after server acknowledgment. Browsers without the required storage/crypto
  features retain the existing unsaved warning and copy/retry path.

- Charts and maps size their plots to the space inside narrow cards, keeping
  the right edge visible on phones and in narrow panels.

- Saved Canvas state now uses versioned, fenced Documents with its content
  sealed under the conversation key inside each Document version. Conversation erasure makes saved
  Canvas notes and markup unreadable. Existing plaintext Canvas files are
  converted once and removed only after the encrypted copy is verified;
  failed or divergent conversions preserve the source and refuse the write.

- Canvas conflict replay restores a remotely closed tab when it holds a
  pending local note and there is safe room. Competing note edits or a full
  Canvas pause saving with the local state intact and a Copy unsent notes
  action. Retry retains the original revision until a safe merge succeeds,
  so it cannot silently overwrite the other window’s work.

- Canvas keeps unacknowledged changes after failed saves and shows a recovery
  message with Retry saving. Authentication, unavailable-conversation and
  oversized-write failures pause retries; temporary failures retry later.
  Returning to the window retries pending work, and leaving with unsaved
  changes requests the browser’s unload warning. Delayed reads cannot roll
  back an already observed Canvas revision.

- Card replacements use one active-card view for runtime validation and
  display. Unused controls can retain their published names when replaced;
  retired controls no longer supply new bindings or worker transfers. A
  source with active dependent cards is refused with their IDs, preserving
  those links until the dependencies are changed.

- Linked-card controls share current inputs across views of the same
  conversation turn. Saved pages keep separate state per immutable version.
  Closing the final view clears its temporary values; invalid inputs restore
  linked fields to their recorded fallback until the input is valid again.

- Delegated workers inherit configured model-step defaults, protected paths
  and completion policy. Dispatches through the shared budget keep each
  child's own trace and actor without resetting aggregate spending limits.
- Worker cards keep their live bindings when copied into the parent
  conversation, including when duplicate cards reuse existing IDs.
  Background result/wait calls retain each worker's cards and provenance;
  multi-worker waits preserve separate source identities. Failed transfers
  are reported and do not count as emitted cards.
- Opening another Canvas tab no longer evicts a tab with an unsent note.
  If all eight tabs contain notes, Canvas explains why a new tab cannot open.

- Composed answers retain leading, interleaved and closing prose around
  cards, including multipart text-only answers. Adjacent result material
  shares its author header and actions without discarding earlier paragraphs
  or moving cards across prose. Technical view also retains earlier cards.

- Tables preserve textual IDs, comma-containing values and large integers.
  Numeric columns are explicit for authored tables and follow source JSON
  types for ability tables. Decimal strings sort exactly without rounding;
  display, text delivery and CSV keep their spelling. Existing numeric
  payloads and linked controls remain supported.

- Route planning stops on unreadable evidence instead of planning from an
  incomplete history. Day-budget admission verifies recorded costs before
  dispatch, including costs written by another gate on the same day.
- Budget reservations retain unresolved costs when recording fails and keep
  their original attribution. Approving a run-budget increase no longer
  bypasses a remaining day-budget denial.
- Live outcome revisions and steering messages must be recorded before they
  change the running contract or reach the model.

- Two dependency updates clear seven known vulnerabilities: rustls 0.23.46
  (RUSTSEC-2026-0346 to 0349, under every outbound TLS connection) and
  quinn-proto 0.11.19 (RUSTSEC-2026-0338, 0340, 0343). The gate and the
  release now run `cargo audit` (`scripts/check-advisories.sh`); the three
  zip advisories, which concern extraction Vak never performs, are recorded
  exceptions in `.cargo/audit.toml`.

- Concurrent credential updates preserve every presence marker, and trust
  checks treat unreadable presence metadata conservatively. Failed secret
  removals retain their markers instead of appearing absent.
- Commitment admission and portfolio reads report unreadable history instead
  of treating it as an empty portfolio. Maintenance stops on that error.
- Chart category labels retain their original text, including leading zeroes
  and punctuation. Ambiguous formatted y values are refused instead of
  silently stripping commas.
- Charts retain tiny ranges, distinguish nearby values and position extreme
  finite values without overflowing. Axis labels use compact, precise text.

- Tools stop before execution if their assistant message cannot be saved.
  A failed compaction write releases the session lock before recording its
  diagnostic, avoiding a stalled turn.
- Saved answer and page views reopen in the Canvas after a reload.
- Calls whose effects are unknown wait for other potentially mutating calls
  instead of bypassing resource conflict checks. Explicitly read-only calls
  keep their existing concurrency.

- Artifact changes stop before deciding or writing when their history is
  unreadable. Strict readers rebuild projection caches created by tolerant
  readers instead of trusting records those readers may have skipped.

- Search, lists and tool-history queries refresh sharing permissions before
  returning results, and report unreadable sharing records instead of
  trusting stale grants. The derived search index rebuilds once to discard
  cursors created by the old tolerant reader.
- Stopping a recovery probe no longer leaves a provider permanently
  unavailable. Late results from an older probe cannot release a newer one.

- Grant-service permission checks, outbound-action decisions and connected
  account reads now fail when new records cannot be read, rather than
  silently trusting cached state.
- Ref writes and removals check the current writer epoch atomically with
  their commit record, so a restored store refuses a stale writer even
  when the two operations race.

## 8.9.0 — 2026-10-18

- A turn can be asked again: Retry on your message answers it afresh, and
  that turn and everything after it leave the conversation the Agent sees.
  Nothing is deleted.

- On OpenAI and Anthropic routes, tool calls are held to their tools'
  schemas again. Since 8.8.0 the document tool's schema was refused in its
  strict form, and that switched schema enforcement off for every tool,
  so a plan could lose a deliverable to a malformed call.

- A message you stop before anything happens no longer stands between
  your next message and the reply before it: the next turn sees the last
  real exchange.

- When a card's link cannot be fetched, the Agent is told to give the link
  in its reply and plan that part as text, rather than asking again for a
  card it cannot show.

## 8.8.0 — 2026-10-17

- On OpenAI, Anthropic and Gemini routes the Agent can use every tool it
  has, including Word, Excel, PowerPoint and PDF files, mail, calendar and
  automations. Before, a tool kept back until asked for could not be
  reached there, and a request for a Word document came back as a web page.

- A skill, once the Agent opens it, names the tools that do its kind of
  work, such as the document tools for writing.

- A long earlier turn reopens whole, instead of being reported as history
  still being prepared.

## 8.7.0 — 2026-10-17

- A turn can be set aside from the conversation: later replies no longer
  see it, find it or cite it, and Undo brings it back. Nothing is deleted;
  the turn stays in the conversation, folded.

- A file or a saved note the Agent made in an earlier turn is read back
  with how that turn went: which of its steps failed, what it planned and
  did not deliver, and how it ended. Nothing is hidden; it is labelled.

- `scripts/poison_replay.sh` (`vak eval --poison-replay`) asks one
  request in a clean conversation and after a turn that went wrong, on the
  same route, and compares the two answers.

## 8.6.0 — 2026-10-17

- A turn whose calls failed, whose cards were refused or that ended
  without what it planned now says so wherever a later turn sees it: in
  the conversation's carried history, in search and when reopened. A page
  address the Agent only typed, or one that failed to load, can no longer
  be cited by a card.

- A card may now give what the Agent planned as text, instead of being
  turned away and replaced by plain prose.

- Asking the Agent to reopen "turn #2" opens it, instead of reporting
  history as still being prepared.

## 8.5.0 — 2026-10-17

- The Agent is offered mail and calendar reading only while an account
  is connected, so a turn without one sends a smaller request.

- Mail and calendar account connections are kept as an encrypted record
  chain like the rest of an Agent's records. A data home that connected
  an account before this needs the account connected again.

- A markup preview can draw diagrams offline when its card names the
  diagram library.

- A saved answer is a page in the Library, drawn with its cards working
  as they did in the chat. Keep current asks for it again on a schedule
  and adds each answer as a new version; the Agent can make such a page
  too. A page opens in the Canvas and can be shared; downloaded, it is
  its text.

- The admin console's Limits panel shows the size of a calculated series
  beside the time limits.

- Hovering a selected admin theme or tab keeps its selected colours and
  readable label.

- On a phone, admin cards and controls stay inside the screen; long tables
  can scroll within their own panel.

- Operations breadcrumbs follow the section you open.

- Pages Vakyartha writes and previews look like the app: its colours,
  fonts and type in light and dark, following a theme switch while open.

- A whole answer opens in the Canvas: its text and every card, laid out
  in as many columns as fit, scrolling, and still working there; change
  an input and the linked figures, charts and tables follow.

- A chart can follow a calculator card above it: change an input and the
  line is drawn again across the range the calculator gives.

- Cards in one answer can move together: a figure or a table cell can
  follow the values of a calculator card above it, and changes as you
  change the calculator.

- An answer can be a working calculator: a card of inputs you change (a
  number, a slider, a choice, an on-off switch or a date) and results
  that update as you do, such as a bill split or a loan payment.
  Vakyartha computes the results itself, from a small fixed set of
  arithmetic, and "Ask about this" sends what you set, with the results
  worked out again, as your next message.

- A card no longer drops part of what was written for it without a word:
  a field the card does not have (choices written inside a step, say) is
  sent back to be written again under the card's own shape, where before
  the card was shown without it.

- Reloading the page while a turn waits for your approval, or for your
  answer to a worker's question, shows that card again at once. Before,
  it came back only after about ten seconds, and until then nothing on
  screen said the turn was waiting on you.

## 8.4.0 — 2026-10-16

Answers come in as they are made, cards can show places on a map, and a
media card shows workspace files. Additive within 8.x: no purge, the
data baseline stays 8.0.

- Places can be shown on a map: numbered markers with their names, a
  route through them in order or an area they outline, a scale and a
  north mark, with each place's details and the distances below. Maps are
  drawn plain, with nothing sent anywhere. Street tiles under them come
  only when you turn on Settings → General → Map tiles, which names the
  server each map's area is sent to.
- A media card showing an image, video or audio file in the workspace
  draws the file; it was empty before. A web image or video shows in the
  desktop app too, where it was refused. When web images are off, or the
  file cannot be read, the card says so instead of standing empty.
- While a turn runs, its cards appear in the conversation as each one is
  ready, instead of all at once at the end. Hovering "Working" shows the
  plan as an arc under the agent: what is shown moves left, what is being
  prepared sits at the bottom with its name and how many parts are done.
- Ticks on a checkable list are kept: they survive a reload, and the next
  turn knows what you ticked. "Use this" on a step's choice starts the
  turn with your choice as your message, naming the card, instead of
  putting a sentence in the composer.
- Details → Activity shows what each turn did, newest first, in plain
  words: thinking, every call with its state ("Showed a table card",
  "Planned: …", "Looked back in the conversation", refusals with why),
  approvals, questions and workers, live from the moment a turn starts.
  The conversation itself still shows only the answer.
- An answer shown as cards ends with its cards: every card says whether
  the answer goes on after it, and when the last card of a step says it is
  done (and the plan has nothing left), the turn ends there instead of
  asking for another step, which wrote the cards' content again as text
  under them or sent them again.
- A markup preview is given the app's theme as CSS variables (`--vak-bg`,
  `--vak-fg`, `--vak-muted`, `--vak-accent`, `--vak-rule`, `--vak-font`,
  `--vak-font-mono`, `--vak-radius`) and its colour scheme, so generated
  markup can match every theme; switching theme redraws an open preview.
- `GET /sessions/{id}/interface` shows what a conversation's last turn was
  given: every tool with its schema, which were held back under "More
  tools", and each installed capability with what it says it serves.
- A proposed skill is no longer tagged "duplicate-of" by a word-overlap
  score picked in code; the review shows each proposal as written.
- With only abilities installed, the skill loader no longer sits in every
  request: it is listed under "More tools", for opening an ability to
  improve it.
- "Learn from conversations" is gone: the optional pass after a run that
  asked a second model call for notes and skill drafts, filtered by
  numbers picked in code, is removed with its `[memory] reflection`
  setting. Notes and skills are written only when Vakyartha calls
  `remember` or `propose_skill` in the conversation. An `agent_network`
  call now states its permission instead of falling to the default.
- One tool reads a web page: `browse` is folded into `webfetch`, which
  takes `render` to run a page's script in a local headless browser when
  `[tools] browse` is on. Its description no longer tells the model to
  look for a search tool first, a step that no longer exists.
- An ability is listed once, as a tool, rather than also among the
  skills under a line saying a skill is never a tool.
- The two cards filled from an ability's answer are offered only when an
  ability is there to answer.
- One place for what Vakyartha remembers: the separate "people and
  things" store is gone, with its tool, the `/entities` routes, `vak
  entities` and the "People and things" search filter. Notes kept with
  `remember` are what recall finds.
- The `data_query` tool is gone: it needed the whole table copied into
  the call. Workbooks are read with `doc_read`, a CSV is profiled by the
  `table-profile` ability, and sums and groupings are worked out with
  Python in `bash`, which the data skill now says.
- Memory is no longer copied behind your back: the consolidation pass
  that ran after every compaction, turning notes with words like
  "always" or "must" into duplicate "invariant" notes and guessing
  contradictions from word pairs, is gone, with `vak memory consolidate`
  and its server route.
- Asking Vakyartha to remember something works on the first step: the
  remember tool is loaded on every turn (it was one tool-discovery call
  away, described as a note "about this workspace"), and it says it keeps
  what you ask it to remember, your name included.
- Finding something said earlier in a conversation no longer misses it
  when the search has words the turn lacks: the word list and the "half
  the words must match" rule are gone, and the best match comes first.
- A card refused for something that is not there (a preview of a file
  that does not exist) is taken out of the turn after it keeps failing,
  as a wrongly shaped call already was, so a turn no longer repeats it
  until it is stopped.
- A table written as one value per line is refused with the right shape
  and the cards for one-value items and for labelled values, instead of
  wording that led to the same mistake again.
- A step's record says which of its tools were sent without their
  schema.
- One way to look back: recall searches this conversation, your other
  conversations, saved notes, your profile, followed sources and files in
  one call, and each hit says where it is from. There is no separate
  conversation search to pick wrongly: asked about something said several
  turns earlier, the smallest model chose the right tool 9 times in 16
  instead of 3.
- Asked to reopen something and search at once, recall now does both
  instead of only reopening.
- Entities are read through recall too: a hit on one carries its whole
  record (type, summary, attributes, relations); the separate entity
  query tool is gone.
- A tool listed under "More tools" is called by its name; there is no
  separate tool to look tools up first.
- Cards that only show what a call produced (a command's output, test
  results, a diff, a file preview, sources, an ability's answer) are offered
  once the conversation has such a call, from the very next step, instead of
  on every request. A conversation that has run nothing is not offered a
  terminal or file preview card, and every request is smaller.

## 8.3.0 — 2026-10-15

The skills Vakyartha ships are real procedures, and two of them are
tested abilities. Additive within 8.x: no purge, the data baseline stays
8.0.

- The skills Vakyartha ships are rewritten as real procedures that name
  the tools and cards it has today: getting started, research and
  sources, planning, data and spreadsheets, writing and editing,
  software development, debugging and code review. Two new abilities
  come with them: a date calculator (days, working days and months
  between dates, with weekdays) and a table profile (a CSV's columns,
  types, missing values, ranges and duplicate rows), each tested.
- Layout guidance no longer names card kinds that were retired.

## 8.2.0 — 2026-10-15

Agents learn abilities they can keep (docs/design/93-self-evolving-capabilities.md,
stages E2 to E9): a program an Agent writes is tested, offered, reviewed
and kept with its versions, and every ability has a page of its own.
Scheduled work speaks your time and answers in the conversation that asked
for it (docs/design/94). Additive within 8.x: no purge, the data baseline
stays 8.0.

- An ability's page names each conversation that used it by its first
  words, as the sidebar does, rather than by its Agent.
- A table or chart of what one of your Agent's abilities answered now
  shows exactly the values it answered, filled by Vakyartha from that
  answer rather than copied by the model, however many rows there are.
  Two ability lines under one answer no longer run together.
- An Agent can look at its abilities that failed each night and propose
  fixes, every one waiting for your review. Turn it on in Settings,
  Capabilities, Learned, or from the inbox the first time an ability
  fails; a night with nothing failed costs nothing. Anything proposed by
  scheduled work now always waits for your review.
- The admin console has Learned abilities: every Agent's abilities and
  the skills every Agent finds, with Take back, which stops the next
  call at once. How the models did adds each ability version's calls,
  failures and what the promotion rule would do. `vak skills list`,
  `show`, `use`, `approve`, `decline`, `promote` and `turn-off` replace
  `vak skills-review`.
- Every learned ability has a sheet, opened from the line under an
  answer, the Library, Settings and the inbox: what it can do, why it
  waits, each version with what it changed and how its tests went, its
  recent uses, and Use this version, Turn off and Review. Settings →
  Capabilities is now Learned, Installed, Connections and Discover, and
  the Library finds abilities by name, Agent and state, with waiting
  ones under Needs you.
- Under an answer, a quiet line says when an Agent used, learned or
  improved one of its abilities, or that one waits for you; it opens
  Learned abilities.
- A proposed ability can be approved for just the Agent that made it, and
  the inbox and Settings take you straight to Learned abilities.
- The Library has "Learned abilities": what your Agents taught
  themselves, whether each works, what it can reach, and what waits for
  you, with Approve, Decline and Give to every Agent. Abilities proposed
  while working in any folder show there, and an ability's versions are
  kept rather than fading like drafts.
- An ability an Agent made is offered to all your Agents once it has
  passed its tests, worked in a later conversation and never failed; an
  ability that keeps failing after a fix is turned off for that Agent.
  Both show in your inbox. An ability now refuses arguments it cannot
  use without that counting as a failure.
- An ability an Agent made says what kind of work it serves, so it no
  longer takes room in every request: it is listed among "more tools" and
  loaded when needed. Asking an Agent to improve one shows it the
  ability's own files, and the improvement is kept as a new version of
  the same ability. Looking back through the same conversation no longer
  counts as reading someone else's words.
- An ability an Agent makes for itself now says what it reaches ("Reads
  the files in this project", "Connects to …") in its approval, in the
  proposal waiting for you and in your inbox. Settings → Capabilities has
  "Let Agents switch on new abilities that only work with your files"
  (on by default); turned off, every new ability waits for your review.
  An Agent's own skill can carry presentation packs, active while the
  skill is installed.
- A scheduled run's prompt shows as a "Scheduled" line with the
  automation's name and time in the conversation, not as a message you
  wrote, also when its answer is only cards; and "run it now" on a
  paused automation runs it once instead of resuming its schedule.
- Schedules mean your time. Vakyartha has one time zone for you
  (`[owner] timezone`, this machine's unless you set it), every repeating
  schedule is saved with it, and "remind me at 9" is 9 in that zone
  whichever computer runs it. A time that falls in a skipped daylight-
  saving hour runs at the first minute after it instead of a day late.
  Settings → General → Your time zone shows and changes it.
- An automation's result appears in the conversation where you asked for
  it, as the Agent's reply marked "Scheduled" with its name and time, and
  you can answer it there ("make it shorter"). A run that could not start
  says why in the same place. The Inbox still lists each one.
- Automations is a page of its own: every automation in plain words with
  its next and last run and its history, the same buttons as in chat, a
  link to the conversation its results go to, and New automation with a
  schedule picker (once, every day, weekdays, chosen days, every few
  minutes or hours) in place of a cron box.
- When the Agent makes or changes an automation, a card under its answer
  shows it in plain words with its next and last run, and lets you
  pause or resume it, run it now, change its time with a picker, or
  delete it.
- Managing automations no longer asks for approval each time in
  workspace-write mode; adding or changing a shell check still asks, and
  can be remembered as a command rule.
- An automation in a folder that is not a git repository runs with that
  folder's settings (its model, permissions, prompts and skills); it ran
  with none. Cards an automation's run shows appear as cards in your
  conversation. A tool the Agent used earlier in a conversation stays at
  hand after Vakyartha restarts. A scheduled run whose model call
  failed is reported as failed, with the reason, instead of finished.
- Ask the Agent to change an automation by its name: "move my morning
  line to 9", "pause it", "run it now". `vak triggers` says `pause` and
  `resume` in place of `disable` and `enable`.
- Schedules read in plain words everywhere ("Weekdays at 8:00 AM",
  "Every 2 hours"), in the Agent's answers and in `vak triggers list`.
- `vak inbox` shows what automations and the gateway deliver. It read a
  different inbox from the one they write, so it always said there was
  nothing.
- An Agent can save a skill with a program it wrote and tested. Vakyartha
  runs the program's tests itself; a skill whose tests pass, that reaches
  no network and was not written after reading someone else's words is
  the Agent's from its next turn, and any other waits for your review.
  Each saved skill is a version in your Library, with how its tests went,
  and saving it again under its name makes a new version.
- `vak skills verify <name>` runs a skill's program against its own tests
  (`tests/<name>.json`: the arguments a call sends and the result it must
  give, or that it must fail) and says, case by case, what passed and
  where an answer differed. A skill's program now needs at least one
  test; without one it is refused with what to add.
- Setup saves the model and the access level for the whole install,
  whatever folder it was started from. Started from a project folder, it
  used to write them into that folder's own settings and leave the
  install with no model.
- A new data home records its version when its first records are made,
  not only when the app first opens it, so a home whose records were made
  by another command first is no longer refused as older than 8.0.
- A command's terminal shows warnings and errors only; the rest of its log
  stays in the log file. Set `VAK_LOG` to see more there.
- Starting a second server (such as the one `vak setup` runs) while the
  gateway is running no longer warns that the agent network stopped: the
  gateway keeps serving it.

## 8.1.0 — 2026-10-15

Skills can carry programs (docs/design/93-self-evolving-capabilities.md,
stage E1), and the data baseline is 8.0: a data home now names the version
that created it. Homes set up with 8.0.0 predate that record, so purge
and set up again after installing.

- A skill can carry a program: a Python, shell or Node script with a
  `capability.json` naming its entry and arguments. Vakyartha offers it as
  a tool named after the skill and runs it in a sandbox built from what it
  declares, whatever the conversation's access level: it reads the
  workspace, writes only its own drafts when it says so, and reaches no
  network. It answers in JSON, and its output streams to the Workbench.
  A program may name the hosts it connects to, and then reaches those
  alone, through a proxy of its own. A Python program may pin its packages
  with their hashes and the package index they come from: the first call
  installs them (and asks first, like anything that reaches the network),
  and later calls run offline from that environment.
  (docs/design/93-self-evolving-capabilities.md, stage E1.)
- A data home refused for predating the baseline now says it was written
  by "a version of vak that did not record itself", instead of wrongly
  saying 7.x or earlier.
- The supported data baseline is 8.0. A data home now records the version
  that created it, and one made before 8.0 (or with no record of its
  version) is refused with the usual message: purge, install and run setup
  again. Sync remotes and backups name the version that wrote them, and one
  from before 8.0 is refused the same way. This includes data homes set up
  with 8.0.0 itself, which predate the record.
- The web app on a server you are not signed in to shows the sign-in screen
  again, instead of waiting forever with "Vakyartha has not answered yet".

## 8.0.0 — 2026-10-14

Cards are rebuilt as eighteen forms (docs/design/92-card-forms.md). This
release cannot read presentation libraries saved by earlier versions: set
the data home up again after installing.

- Every card is one of eighteen forms, chosen by the shape of what it
  shows: a summary, steps, a list, details, a table, a comparison, figures,
  a chart, a callout, sources, a calendar, a link, media, a page preview,
  markup, a diff, a test report or a terminal. An answer uses as many of
  them as it has parts.
- Every card has the same frame, text at the conversation's reading size,
  and prose held to a comfortable line length.
- Charts space their bars properly, label every category, use round axis
  numbers, show each bar's value and a legend for several series, keep
  their text at your text size, and lose the point slider.
- Tables are never a scroll box inside the conversation, and sort by any
  column.
- A list is a list, not numbered steps; it can be ticked off when it is
  things to bring, buy or do. Steps lead with their time.
- A calendar card shows events by day.
- Sources show who published each finding and when, beside the finding.
- Removed: the recipe card (a recipe is details, a list and steps), the
  starter layouts that only renamed a card, the social add-ons' alternative
  research layouts, and the reading of an answer's words to decide how it
  is shown.

- A request for several files is held until each has a file of its own:
  two drafts of one workbook no longer count as the report and the
  workbook that were planned, so the turn is told what is missing.
- A link or emphasis written in a card's text shows as a link or emphasis,
  as it does in the conversation, and a long address wraps inside the card
  instead of pushing it off a phone screen.
- Today (your mail and calendar) is a button in the top bar, beside Share
  and Details, instead of floating over the conversation.
- An answer's cards arrive together rather than one at a time.
- When the assistant improves a card it already showed, the new card takes
  its place instead of appearing as a second copy; and two short cards it
  means to be read together sit side by side on a wide screen.
- Accepting a draft no longer shows the file twice: the draft's card says
  it is now in your folder, and no second card for the same file appears.

## 7.3.0 — 2026-10-14

Card layouts become extensions, like skills and MCP servers, and answers
are laid out better.

- Vakyartha ships "answer-layouts", enabled: skills for laying out a plan
  in time, a choice between options and a how-to guide from cards, and a
  layout that shows key figures in a row.
- An installed presentation pack can add its own kinds of card: while it is
  active, the card tool it names offers them, and the pack draws them. A
  card whose pack is later turned off is drawn by the card it was built
  from. The social add-ons' cards now come from their own packs rather than
  being built into Vakyartha.
- A plugin's presentation packs are on while the plugin is enabled and off
  when it is disabled, like its skills. The social add-ons' separate "Add
  layouts" step is gone: enabling the add-on brings its layouts.
- Presentation packs appear in the capability inventory beside tools,
  skills, MCP servers, hooks and commands. A project's choice of card
  layout wins over your Shared one, and a channel can allow or deny card
  layouts, like skills, in the admin console's channel settings.
- A pack that lays things out in rows, stacks or titled sections is drawn
  that way; before, every part stood in one column.
- A summary, figures, callout, checklist or comparison card no longer gets
  two frames' spacing and a doubled entrance in an answer.
- A card call with a missing or wrong field is rewritten under its schema
  at once, so a card is no longer lost when the model does not try again
  after being told what was wrong.
- The chart card no longer offers a `bar_chart` kind beside its `bar`
  style, a pair models confused; a bar chart is `chart_type: "bar"`.
- Full access no longer asks before shell commands in a conversation that
  read the web or a connected service. The check that a turn holding both
  your data and someone else's words asks before anything leaves the
  machine applies only in the sandboxed modes; Full access asks only what
  your own rules ask.
- Searching for a tool (`find_tools`) no longer asks for approval; it only
  reads the tools the turn already has.
- Every package Vakyartha ships comes from one place, so the starters, the
  social add-ons and answer-layouts install and update the same way.
- `vak plugins … --scope user` and `vak skills validate` use the Shared
  layer every workspace reads (`~/vak-home/.vak`). Before, the CLI put
  user-scope plugins in the data home, where nothing read them.

## 7.2.0 — 2026-10-14

- Security: when a turn holds both your own data and words from someone
  else (a fetched page, a connected service), anything that could send data
  off this machine asks you first, and nothing can pre-approve it.
- Security: an MCP server reaches only the hosts it declares, through an
  egress proxy of its own.
- A command stopped at its time limit on macOS now really stops there.
  Before, the sandbox refused the stop and the command ran on to the end.
- A terminal card shows what the command really printed and how it ended
  ("Stopped · 5.0 s", "Exit 1 · 4 ms"), taken from the run itself. A
  test-report card says how its run ended the same way, and can no longer
  list every test as passed when the run failed.
- A tool you decline is not offered again for the rest of that request.
- A skill you promote from the review queue is now available to the next
  request. Before, it was installed where no conversation looked for skills.
- `vak term`: the approval box no longer offers to edit the command. An
  edit was never sent: approving after one approved the original command.
- Library: Compare with your folder lists what an Office file or a PDF
  version changes, the file's history shows each version's state, cards
  show their live state, and Needs you lists files waiting for you.
- Lineage lists the web pages a conversation fetched and the files it
  touched, each traced to the request that caused it. Search and lineage
  are rebuilt once after updating.
- Two Vakyartha processes on one data home (the app and the command line)
  no longer record the same file change twice.
- A request held with `/until-done` ends when an answer adds nothing new,
  and the work stays open to continue.
- Admin console: System › How the models did shows how each model did on
  real requests and lets you accept or dismiss the patterns it found.
- A new Office file can be saved into a folder that does not exist yet,
  and an answer given only as cards counts as an answer.
- A tool is called only by its exact name; a name that is not a tool is
  answered with the tools that exist (and, for a retired one, what
  replaced it).

- Security: a WebSocket upgrade (the web terminal, a voice session) is
  refused unless it comes from the server's own page or carries the access
  token. Before, a web page on another local port could open the terminal
  with your session when `[server.web] terminal` was on.
- Interface sounds are recorded, in five instrument families chosen in
  Settings › Voice and sound; the start sound plays once for a run. Each
  character has its own sound, played when you click it.
- In the Dimensional 3D visual pack the character that is working moves;
  Reduce motion keeps it still.
- The app's sounds and images are checked for a newer copy instead of being
  kept by the browser for a year, and sounds are sent as audio.
- Dependencies updated for security advisories: solid-js (seroval),
  source-map-js, katex, sharp, postcss-selector-parser and ratatui (lru).

## 7.1.0 — 2026-10-13

- The model plans its own deliverables and the runtime checks them against
  what was produced (cards, text, files, changes, checks); a missing part is
  named once. Nothing decides from a request's words or a tuned threshold
  any more.
- Cards are shown by calling the card for each part, in reading order; a
  card names the plan item it delivers in `deliverable`. Tool arguments are
  enforced by the provider where it can, and a call that cannot be read is
  written again under its schema.
- A preview is two cards: a file in the workspace, or markup written in the
  call. A preview of a file that does not exist is not shown.
- Files across turns: Vak tells each turn what happened to a file since
  (a draft waiting, accepted, undone, changed outside Vak, deleted).
  Accepting a draft made before an outside edit asks first. Vak's own
  writes, edits and shell changes are recorded as its own, never as an edit
  made outside it.
- The prompt names the workspace folder; errors on a path outside it say
  which folder the workspace is.
- A step that answered only in its thinking is shown that thinking and asked
  for the answer; on providers that do not replay thinking, turns no longer
  loop rewriting finished work.
- Time limits (commands, waits) are operator settings with their defaults
  shown in Settings; a call stopped at its limit is told so.
- Frosted light and Frosted dark themes; Inter and a calmer type scale.
- Linux: deleting a bot removes its service.

## 7.0.1 — 2026-10-10

- Linux: commands, document previews, Review and `doc_read` run again.
  The sandbox restricts files with Landlock and refuses internet sockets
  (TCP and UDP) and io_uring with a seccomp filter; it still refuses to
  run on a kernel that cannot enforce both.
- Chart cards draw their data again (they showed "No data points
  supplied"); a chart needs at least one point, and a number written as
  text such as "24,850.5" is read as a number.
- `recall` returns the result it was pointed at when a query is given
  beside it, instead of refusing.
- A spreadsheet edit that formats a block and then changes part of it
  is written, instead of failing its own check.
- The Library previews spreadsheets, Word files, slide decks and PDFs
  instead of offering only a download.
- Linux: `vak setup`, `setup status`, `vak self services-sync`, `vak self
  status` and `vak doctor` say when systemd user lingering is off, so the
  durable services would stop at logout, and print
  `sudo loginctl enable-linger <user>`.

## 7.0.0 — 2026-10-10

- The 7.0 data baseline (data-architecture M3b). A data home written by
  6.x or earlier is refused with one message; run
  `vak self uninstall --purge`, then install and run setup. No 6.x data
  crosses the baseline, and there is no 6.x maintenance line.
- Conversations are kept as segmented, hash-chained records written with
  group commit (about 6 KB and 6 syncs per turn); large payloads and
  Agent memory, entities, skill proposals, Office rooms and presentations
  are encrypted objects and versioned Documents in the tenant store.
- Every project gets a lasting id, so its history, secrets, trust, schedules
  and channel settings follow the project rather than a folder path. A new
  Configure › Projects screen in the admin console names, hides and shows
  projects, and the session view groups conversations by project.
- Nothing Vakyartha needs at runtime is written into your project folder:
  temp files, caches, drafts, Agent workspaces and run environments live
  in the data home and runtime directory.
- Backups say plainly when encrypted history and memory can be restored
  only on this machine.
- Every unit of work has a run record, every schedule is one automation
  (a trigger) that starts a slot only through its claim, and every action
  outside Vakyartha (a channel message, a mail send, a calendar change) is
  an effect sent at most once. A process fenced by a restore stops writing,
  and `vak serve` starts itself again.
- One data catalog serves every search and traces anything to its run and
  cause; the Library keeps every declared deliverable with its versions,
  comments and shares, and Review works on artifact versions.
- Retention rules, holds and erasure: the trash ends in erasure, and the
  owner can erase a conversation, a draft, a guest's messages, what a
  disconnected account returned, an Agent's data, a project's data, one
  channel sender, or the whole install, each with a signed receipt.
  Backups restore without overwriting and apply every erasure again.
  Keys can be rotated and earlier ones retired.
- A second machine can keep a copy in a folder and take the work over:
  push, pull, hand-over and take-over under a lease, with a passphrase-
  sealed key file carried by hand and no secret ever written to the folder.
- Content-free structured telemetry with traces per run.
- Mail and calendar routines recover by themselves: a watch run is told
  where its new mail is, a token the provider refuses early is refreshed
  at once, and a history the provider expired resyncs with a recorded gap.

## 6.0.0 — 2026-10-03

- Ship 6.0.0 as an independent major release. The separately planned data
  baseline is deferred to 7.0.0; this release does not cut over or purge data.
- Expand connected mail and calendar work with bounded previews and reads,
  local drafts, owner-reviewed actions, and scheduled read-only routines.
- Add provider-aware capacity admission, shared quota tracking, and clearer
  Traffic visibility; reserve run budgets against dispatches.
- Make the Canvas shared across client surfaces, isolate previews, and keep
  file reads attached to their named conversation.
- Improve conversation history selection and context planning, including
  explicit recovery of relevant prior evidence and provider context limits.
- Give the social add-ons their own Social accounts page in Settings, as grouped
  platform cards, and an oversight tab in the admin console. YouTube keys and
  LinkedIn Client IDs can be saved once under Shared and overridden per agent.
- Add owner-only Reddit and X search previews. Reddit needs only an installed-app
  Client ID; every X search counts against a monthly limit you set. Results are
  never saved or shown to the agent.
- Complete the `/` and `@` composer menus with chaining, and show the text
  people entered in search results and other human-facing views.
- Tighten Bash and sandbox execution boundaries, enforce parent worker
  budgets, and require clear confirmation before Escape stops a run.

## 5.3.2 — 2026-10-01

- Turn the compact commitment arc upward from the right through the active
  center to completed work on the left, while keeping the in-chat working
  status visible during a response.
- Avoid issuing duplicate progress requests when the arc is first revealed.

## 5.3.1 — 2026-10-01

- Keep unrelated older turns out of the model's working context, bound history
  retrieval, and let the Agent reopen specific prior evidence when needed.
- Avoid empty history-recall calls and carry a prior action forward only when
  the person explicitly refers to a follow-up.
- Center the Agent glyph and switcher in the conversation header, with a
  compact hover/focus commitment arc while work is active.
- Keep desktop and browser clients attached to the same local gateway and
  conversation state.

## 5.3.0 — 2026-09-30

- Make the Canvas a workspace rather than a file viewer. What it shows is named by identity (a workspace file, a run's file, one saved version, markup from the conversation, a dev server or a scheduled routine), so opening something can never quietly show a different file, and an unreadable file is an error, not a stand-in page. Each conversation keeps its own Canvas with tabs that remember the reader's view, selection and unsent note.
- Serve previews from origins of their own. A page with files behind it (a saved draft's site, a run's output, a workspace page) loads its own stylesheets, scripts, modules and links from a loopback origin that shares no origin, cookie or storage with the app; dev servers are framed under the other loopback name. A preview's sandbox is chosen by the client and never by the data describing it, its network is closed, and the dock's old preview pane, its URL bar and the unused `/fs/preview` and `/canvas/preview` endpoints are gone. The dock keeps a Live preview list. Previews need the app on this computer; a remote browser gets a single-page view.
- Let readers point at a place in a draft (source lines or a document place), comment on it or ask the Agent about it, see what changed and what happened to a draft, and be told when a newer version arrives without it replacing the one being read. A scheduled routine opens in the Canvas from the task list, with run now, pause and resume.
- Fix a result saved in the folder failing to open in the Canvas, and picking a line reloading the whole viewer.
- Require an artifact receipt before an authoring outcome counts as delivered, and add deterministic generic scenario batches to the eval suite with more reliable large runs.

## 5.2.14 — 2026-09-30

- Coordinate rate-limit cooldowns across OpenAI Chat Completions and Responses routes that share the same endpoint and credential. Preserve retry timing from streamed Responses failures and wait cancelably before dispatch.

## 5.2.13 — 2026-09-30

- Remove the AWS Bedrock SDK and its model availability preflight. Bedrock now follows the same provider-neutral model catalogue and invocation path as every other provider.

## 5.2.12 — 2026-09-30

- Stop the continuation guard from mistaking ordinary sentences such as “They’re going to the park tomorrow” for an unfinished plan, and recognize plan markers after list bullets while accepting sentence punctuation followed by closing quotes or brackets.

## 5.2.11 — 2026-09-30

- Hold a turn open for the person only with the explicit `/until-done …` command; "keep working until the tests pass" now names a condition instead of keeping a turn running until someone types done.
- Shorten the Office and PDF tool's instructions by moving each format's styles, layouts and default sheet into the fields that use them.
- Keep an MCP server's latest failure out of the cached system prompt and report it, with the fix, when the server is used.
- End a turn honestly when a runtime correction cannot be recorded, instead of retrying without it.
- Give workflow agent steps a prompt written for their own tools and reader, and record an Agent's added instructions under their own name.

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
- `vak-server/tests/it/control_vocabulary_sync.rs` fails the build unless the client's two lists equal the Rust vocabulary exactly (mutation-checked).

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
- Added a render harness covering every registered semantic type (removed on 2026-10-11), MCP health now reads from the capability registry rather than the disconnected `mcp_cache`, and misread self-correction now observes the mechanism that actually runs.
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
