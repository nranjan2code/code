# 82 — Library: one place for everything Vak makes

Status: **proposal, 2026-10-01; revision 3 after a second review the same
day. Nothing in this document is shipped.** Its product decisions (§1) are
taken: the maintainer delegated them, and in the second review chose
decision 3 (work on an artifact continues in its Agent's own conversation).
This document is the product specification for the Library: doc 74's client
Library (C4) and admin Library (A6), and the plan's M8 screens, build to it.
Phase L1 may start before the data architecture because it adds no durable
store (§9), and it is a **discovery prototype**: everything it derives or
links to is discarded by M3b's data-baseline reset (§9.2). Later phases ride
data-architecture milestones M4 (runs), M6 (catalog) and M8 (artifacts) in
`73-data-architecture-and-lifecycle.md`. Companion to
`81-pieces-and-home.md`: Home is the desk for live things, the Library is
the cabinet for everything.

## 0. The problem

Vak produces documents, spreadsheets, decks, pages, dashboards, images, data
and saved results — many per day. Today each one is reachable only through
the conversation that made it:

| What exists | Where | Scope today |
|---|---|---|
| Run outputs | `WorkbenchPanel.tsx`, `sessionWorkbenchMap`; `ArtifactGenerated` events in `<Agent home>/sandbox/executions/<session>.jsonl`, from `write`, `edit`, `office_apply` and files a command created or changed | the open conversation |
| The viewer | `ArtifactCanvas.tsx`, `canvasStack.ts`; identity in `canvasSubject.ts` | per conversation, eight tabs |
| Drafts, versions, comments, revisions, promotions, undo | `vak-sandbox` `DurableRecord` in `sandbox/records.jsonl` under the server's own Core home (`sandbox_records_path`), which is the built-in Agent's, so one file holds every Agent's records; comments are `CandidateComment` activities in the session ledger | read per session (`/sessions/{id}/sandbox/records`) |
| Version numbers | `DraftVersions` (`vak-server/src/projection.rs`), `draftVersions.ts`, `candidateVersions.ts` | per execution: versions of one result are alternatives, and accepting one settles the round |
| Name, description, draft/accepted/in-folder state | `ArtifactRef` (`vak-delivery`), projected by the server from the durable records | one card |
| "This draft is the result, for Review" | `Tool::delivered_file` | `office_apply` only |
| "This call made a user-facing file" | `Tool::produces_artifact` | `write` always, `office_apply` through `delivered_file`; completion evidence only |
| Cards | presentation entries in each ledger | inside their chat |
| Shared editing, sharing | Office rooms (one session, candidate and path), coworking (doc 69) | per conversation |
| Search | `/search`, `SearchModal.tsx`, over `vak-store`, which records a tool call as its name and a shell `command` only | conversation text, not things |
| Navigation | `Sidebar.tsx` | Search and Agents; the Inbox opens only by its shortcut |

Some provenance is recorded: `CandidateRecord` carries `session_id`,
`turn_id`, `result_id` and `execution_id`, and `parent_candidate_id` links a
revision a person asked for (and a narrowed Office draft) to the version it
came from. Four gaps matter for this design:

- **Most deliverables never become candidates.** `write` and `bash` work in
  the workspace (invariant 35). Only `office_apply` drafts and executions
  run in `.vak/scratch/` can become candidates, and only when someone opens
  Review, which is when the Workbench exports one. A dashboard written by
  `write`, or a chart a Python script saved, is recorded only as a tool call
  and an `ArtifactGenerated` event. No record says a draft was rejected.
- **A candidate is a changeset, not a thing.** `CandidateManifest.files` is
  a list; one run's report, chart and data are one candidate.
- **An agent's own later revision starts a new chain.**
  `parent_candidate_id` is not set when the agent revises its draft in a
  later turn without a person asking.
- **A version's author is not recorded.** `CandidateRecord` has no actor
  (data-architecture M1 adds one to every record, plan L9). A
  revision a person asked for copies its parent's `turn_id` and names its
  child session in `revision_session_id`. An Office room edit is marked only
  by `environment_id = "office-workspace"`; its editor is in the room's own
  revision record.

What is missing is an identity for the *thing* and a place that lists things
across conversations. So finding last month's report means remembering
which chat made it and scrolling.

## 1. Decisions

1. **Library** is a destination of its own: one sidebar row under Search,
   above the Agents list. The sidebar is still built around Agents (doc 64).
   The Library replaces doc 74's "Library panel" (C4) rather than joining
   it.
2. **Things first.** An artifact is the object; conversations are its
   history. Agents still own the work (doc 64); the Library is a view
   across their conversations.
3. **Work on an artifact continues in its Agent's own conversation.**
   Continue working attaches the artifact to the authoring Agent's
   continuous conversation for that workspace, the one doc 64 defines, and
   the person carries on there. No conversation is created per artifact.
   The conversations that made it are reached through `recall` (§6, phase
   L2). A thread per artifact was rejected because it contradicts doc 64.
4. **Ownership follows the data architecture.** Until M3b an artifact
   belongs to the Agent whose workspace holds it (invariant 37): the most
   specific Agent workspace containing the file, since a non-built-in
   Agent's workspace is nested under the base workspace
   (`vak_config::paths::agent_workspace`). From M3b it belongs to the Space,
   with the authoring Agent recorded. A conversation always belongs to
   exactly one Agent. (Resolves `81-pieces-and-home.md` §20 decision 6 the
   same way for pieces; doc 73 §5 now answers §10 question 3.)
5. **Only declared deliverables are entries** (§3). Supporting files are
   parts. Cards stay in chat unless saved.
6. **L1 ships as a read-only projection** over existing records and
   ledgers, behind the API M8 will keep. Anything that writes waits for M8,
   except what L2 records in the Agent's own conversation: the person's
   messages with their attachments, and `recall` results.

## 2. The model

```
Artifact "Sales dashboard"                     (art_ id at M8; derived key before)
 ├─ Versions       v1 Vak · "Q3 numbers", turn 12
 │                 v2 Vak · after your comment
 │                 v3 you · uploaded edit
 │                 v4 Vak · "Board prep", turn 4
 ├─ Parts          clean.py, data.csv, template, recipe (doc 81)
 ├─ Conversations  each conversation that touched it, at its exact turns
 ├─ Comments       per version
 ├─ Sources        files, sites, tools used
 ├─ Related        made from sales.xlsx · used by Board deck
 └─ Sharing        viewer / commenter / editor grants
```

- **Kinds**: document, spreadsheet, slides, page, dashboard, image, data,
  diagram, saved card. The kind comes from the declaration that made the
  entry (§3) when it names one, else from the bytes: the main part's content
  type for Open XML (invariant 39), the PDF header, and magic-byte sniffing
  for images and archives. That detection runs in the worker (invariant 14)
  once per version digest and is cached in the cache home, so a listing
  never starts a worker per file. Text formats (HTML, Markdown, CSV, JSON)
  have no reliable signature, so their kind is the declaration's, and the
  extension is used only as a display hint. "Dashboard" versus "page" is
  never inferred from bytes. A piece (doc 81) is not a kind: it is an
  artifact of any kind that is Live.
- **State and marks.** An artifact has one state: **Draft** (no version is
  in the folder yet), **Saved** (a version is in the folder: accepted
  through Review, written there by Vak, or saved by a person) or
  **Archived**. Marks add to it: **Waiting for review** (a draft not yet
  accepted; no record says a draft was rejected, so one waits until it is
  accepted or fades, §3), **Changed outside Vak**, **No longer in the
  folder**, **Replaced** (§2.1), **Live** (a piece, doc 81), **Shared** (has
  a grant) and **Starred**. A card shows the state and at most one mark, the
  first that applies in that order. L1 shows Draft and Saved and the first
  four marks; the others arrive with their phases.
- **Versions.** A version is one body of the artifact's file, with who made
  it, when, and the version it was based on. The timeline lists each
  accepted draft (made by Vak or by a person in an Office room), each
  `write` of the path, each `edit` or command change to it, and each file a
  person put back. Drafts awaiting Review, from Vak or from a room, count as
  one entry until one of them is accepted. Today's
  per-execution versions (`DraftVersions`, `draftVersions.ts`,
  `candidateVersions.ts`) are alternatives within one round of Review; they
  stay inside Review and the chat card, called drafts ("draft 2 of 3"), so
  their numbers never collide with the Library's. L1 builds the timeline by
  extending `DraftVersions`, not with a second projection.
- **Version bodies.** A `write` version's bytes are its call's arguments in
  the ledger. An Office draft's are in its scratch directory until it is
  frozen, then in its candidate; an accepted draft's and a room revision's
  are in their frozen candidates; a put-back file's are its upload. An
  `edit` or a command records that the file changed, not its bytes, so
  before M3b such a version shows "No copy of this version" and offers no
  diff. From M3b every produced file is an object (doc 74 §2.6), and the
  gap closes.

### 2.1 Identity before M8

An artifact is the **file it becomes**: the Agent, the workspace root and
the path relative to it. Candidates, promotions and recorded calls are
evidence for its versions, not its identity.

- **Several files in one candidate** are several entries when several are
  declared; the rest are parts of those entries.
- **A change built on the file's current content continues the artifact**:
  an `edit`, an `office_apply` that edits the file itself (its `source` is
  the file, with a `base_digest`), and an accepted draft (Review refuses one
  whose base changed). So does any undeclared change. A later revision by
  the agent is therefore the next version whether or not
  `parent_candidate_id` links it, and two chains accepted into the same
  path are one artifact with versions in acceptance order.
- **A declared overwrite may start a new artifact.** A `write` with a
  `title`, or an `office_apply` that makes the file from a template or from
  scratch, at a path that already holds an artifact, continues it when its
  conversation already made a version of it, when its turn carries the
  artifact's attachment (§6), or when its title is the same. Otherwise it
  starts a new artifact, and the earlier one is marked "Replaced by
  <title>" with its versions intact. This separates two unrelated
  `report.md` files made months apart. A false split shows two entries; a
  false merge would hide one artifact inside another's history, so the
  rule leans to splitting.
- **Make another** never writes to its source's path (§6).
- **A file renamed outside Vak** is not followed before M8; the old entry is
  marked "No longer in the folder" with its versions intact, and the new
  path is a new entry once something declares it.
- **A file deleted outside Vak** is marked the same way; its versions stay
  viewable while their bodies exist (§2).
- **A file changed outside Vak** is marked "Changed outside Vak" when its
  current digest matches no recorded version. Download and the Canvas show
  the file as it is in the folder.
- **A routine run** works in its own environment, today a git worktree per
  run (`spawn_isolated_run`), not in the workspace. Until M1 records a
  session's cause and M4 returns a run's changes to its space as a
  candidate, each run's declarations are entries of their own, rooted in
  that run and labelled with the routine's name where the run's
  `TaskSummary` inbox entry links its session to the routine. They are
  found by title and time. From M4 they are versions of one artifact at
  their destination path (L4).

M8 replaces derived keys with `art_` ids (doc 73 §3: ids are never derived
from a path). Which artifact a path currently holds becomes a ref, and the
rules above decide whether a change is a version or a new artifact. There
is no mapping from derived keys to `art_` ids, because no state from before
M3b survives to M8 (§9.2).

## 3. What becomes an entry

Volume control is the design. The rules:

1. **Declared deliverables only, by a typed signal.** A file is an entry
   when one of these recorded facts names it, and never because of how a
   file looks or which tool wrote it:
   - a call whose tool reports the file as a declared deliverable through
     `Tool::artifact` (below): an `office_apply` draft, or a `write` that
     carries a `title`;
   - a card that names the file in its payload's `artifact_path` (today the
     ui-preview card), **when the same session recorded a change to that
     file** (a `write`, `edit` or `office_apply` call, or an execution's
     `ArtifactGenerated` event) and the path resolves inside the declaring
     Agent's workspace without a symlink escape (invariant 10). A path a
     model wrote is never evidence of ownership on its own; the existing
     execution-artifact route applies the same rule
     (`execution_artifact_path`, `vak-server`). This is how a file a `bash`
     command made becomes an entry: the turn presents it.

   `Tool::artifact` **replaces** the boolean `Tool::produces_artifact` in
   the same change (invariant 30) and separates two facts the boolean could
   not:

   ```rust
   /// The workspace file a successful call with these arguments produces,
   /// and whether the call declares it a deliverable for the person.
   fn artifact(&self, args: &Value) -> Option<ArtifactClaim>;

   pub struct ArtifactClaim {
       pub path: String,
       pub declared: bool,
       pub title: Option<String>,
       pub summary: Option<String>,
   }
   ```

   `write` returns a claim for every call, declared only when it carries a
   `title`; `office_apply` returns a declared claim for the file its draft
   is for; every other tool returns `None`, as it returned `false` before.
   Completion evidence reads `artifact(..).is_some()`, so a
   successful untitled `write` still satisfies a turn that was asked for a
   file, exactly as today (`stop_policy.rs`); only Library entry reads
   `declared`. `BrokeredTool` forwards the method, because every production
   tool runs behind the broker. An MCP tool cannot declare a deliverable;
   its files become entries only through a card.

   `delivered_file` keeps its narrow meaning (a draft for Review) and its
   four loop behaviours: the presentation check stands down, an identical
   call gets the first call's result, a card previewing the draft is
   withheld, and a command copying the draft out of scratch is refused. The
   Library does not widen it, because applying those to `write` would let
   an identical rewrite after an intervening edit be skipped.
2. **Files written on the way are parts.** Scripts, intermediate data and
   other files that a turn wrote, and that no declaration names, are parts
   of the entries that turn declared. Temporary files and tool caches under
   `.vak/scratch/` are never parts (invariant 35). A turn that declared
   nothing adds nothing to the Library; its files stay in the
   conversation's Workbench (§10, question 1).
3. **Named at birth.** The declaring call carries the title and a one-line
   summary (`title` and `summary` on `write` and `office_apply`; a card's
   own `title`). They live in the call's arguments in the ledger, so naming
   adds no store. The tool descriptions say when to give a title: when the
   file is a deliverable for the person, not a step towards one; if the
   shipped prompt seed changes too, `07-prompt.md` gets its diff note. Small
   local models omit optional fields; a declared deliverable without a
   summary shows its title only, and an Office draft without a title falls
   back to a readable form of its file name. No model call is made later to
   name things.
4. **Where declarations are found.** In every session of the Agent's
   conversations, including sessions a route change rotated (invariant 17),
   and in the sessions work was handed to:
   - a delegated worker's session (`task`), attributed to the parent
     conversation's `task` call;
   - a revision a person asked for, which runs in a child session
     (`CandidateRecord.revision_session_id`) and is attributed to the
     version it made, not to the parent turn whose `turn_id` the record
     copies;
   - a routine run (§2.1).

   A file a person sent (`inbox/`) is not an entry by itself. It is a
   Source of what is made from it, and becomes version 1 of an entry ("you
   · sent in chat") when Vak makes a version of it.
5. **Versions, not copies.** A revision is a version of the same artifact
   (§2.1). An identical body (the same digest) is one version.
6. **Cards stay in chat** unless saved ("save this recipe"); a saved card is
   a `saved card` artifact holding the card's payload. Saving is a write, so
   saved cards arrive with M8; no save action exists today.
7. **Drafts fade (M7a/M8).** Fading is the lifecycle reconciler applying the
   default retention label (doc 74 §2.7 and §3.1), not a Library rule. A
   draft version not accepted, starred or shared is listed under "Older
   drafts" after 14 days, which is a view, not a state. After 60 days it
   moves to the trash, restorable for the trash window. Anything Saved,
   Live, Shared or starred stays. Opening a draft does not stop the clock;
   recording opens would be a write for every view. Starring replaces the
   Workbench's "keep candidate" (doc 74 C8), so there is one way to hold on
   to a draft, and "Keep" stays doc 81's word for making a piece.
8. **Trash and audience are honoured per session.** Trash is keyed by
   session (`vak_core::trash`), so the Library applies it per session: a
   declaration, version or comment recorded in a trashed session is absent,
   and so is everything the Library derives from it (its conversation in
   Conversations, version labels quoting it). A delegated worker's session
   and a revision's child session are absent when their parent session is.
   An artifact all of whose versions are absent is absent, unless its file
   is in the folder, in which case it shows the file as it is with "Its
   history was deleted". Every listing covers only the Agent homes and
   workspaces the viewer is admitted to (invariant 37), resolved through
   the same admission and trust checks as conversations. Records are
   filtered by the Agent named in their session's header, never by which
   file they were read from, because one `records.jsonl` holds every
   Agent's records (§0). The Library never builds on the unfiltered
   `GET /sandbox/records`. In L1 the viewer is the owner; a coworking
   participant (doc 69) has no Library.

## 4. The Library screen

- **Place**: one row in the sidebar's top group, under Search and above the
  Agents list (§1). Doc 70's four reference screens include no list view,
  so a reference for the Library and the artifact page, in doc 75's visual
  system, is added to doc 70's ledger before L1's screens are built. When
  Home exists (doc 81), its recent items link here; where Home and the
  Inbox sit in the navigation is doc 81's and doc 75's decision.
- **Default view: Recent.** Cards with a glimpse, title, summary, last
  change, authoring Agent, and the state badge (§2). A **glimpse** is drawn
  from text, not pixels: the first heading and lines of an Office file or
  PDF from the `doc_read` projection, made in the worker (invariants 14 and
  39); the first rows of a table; the title of a page; and an image itself,
  drawn by the client through the Canvas's media route. Glimpses are cached
  in the cache home by version digest, rebuildable, and declared as such
  (`vak_core::state::REGISTRY`). Vak has no page renderer for PDF or Office
  files (the Canvas shows a PDF through the browser and an Office file
  through its text projection), and a sandboxed preview cannot be captured
  by the page around it, so pixel thumbnails are a separate proposal. A kind
  without a glimpse shows its icon.
- **Filters**: kind; Agent; workspace (space from M3b); Waiting for review;
  changed (today, this week, this month, earlier); Starred (M8); Shared with
  me (M8); Live (doc 81 P1); Trash (M7a).
- **Grouping without filing**: by workspace, by Agent, by commitment (doc
  47); folders and tags are optional (M8).
- **Search** in the language people remember in: by title and content ("the
  deck with the churn chart") and by the conversation that made it ("the one
  for the board meeting"); time is the changed filter, never parsed from the
  query. L1 matches titles and summaries from the declaring calls through a
  derived table of declarations (§9), and the originating conversations'
  text through the existing FTS index (`vak-store`); a conversation hit
  lists that conversation's artifacts. M6 adds content and lineage through
  the catalog.
- **Everyday words only**: no ids, paths, hashes or byte counts unless Show
  technical details is on (doc 75).
- **Phone**: a single-column list with the same filters behind one control.

## 5. The artifact page

Opening an entry shows, in this order:

1. **The artifact**, drawn by the existing Canvas viewers (HTML, Office,
   PDF, tables, media, source, routines) and opened by identity
   (`canvasSubject.ts`): a version that is a frozen candidate opens as a
   `draft_file`, a draft still in scratch as an `execution_artifact`, and
   the file in the folder as a `file`, so the page never shows a different
   file than the one it names.
2. **Actions**: Continue working · Make another · Edit · Download · Put
   back · Share · Keep live · Rename · Move · Star · Archive. L1 has Open
   conversation (the conversation that made the current version, at its
   turn) and Download; Continue working and Make another arrive in L2 (§6),
   the rest with M8. **Download** serves the bytes through the existing
   authenticated, hash-checked routes and always as an attachment
   (`Content-Disposition: attachment`, `X-Content-Type-Options: nosniff`).
   An HTML artifact is viewed only in the Canvas's sandboxed preview
   (invariant 35), never rendered on the app's own origin.
3. **Versions**: the timeline (§2), with who made each one and the existing
   diffs: Office redlines, PDF and semantic diffs, file diffs. Who made a
   version is read from records, never guessed: a candidate with
   `revision_session_id` is Vak's, after a comment; a room revision is its
   editor's (`OfficeRevision.author_id`); a narrowed draft is Vak's changes
   as the person narrowed them; any other draft and any recorded call is
   Vak's; a file the person sent or put back is theirs. An accepted version
   also says when it was accepted. Where no record says, the version shows
   no author. From M8, concurrent edits are sibling versions
   (doc 73 §10), shown side by side and reconciled through Review.
4. **Conversations**: each conversation that touched it, opening at the
   exact turn.
5. **Comments** (L1, read-only, from each version's `CandidateComment`
   activities), **Sources** and **Related** (L3), **Sharing** (L5).

**One lister.** The Library is the only place that lists artifacts across
conversations (invariant 30). Inside a conversation, the chat's artifact
cards, the Workbench's per-execution files and the Canvas's tabs stay: they
are views of the same records through the same projection (`DraftVersions`,
extended), not second contracts. All three gain "Open in Library". At M8,
`WorkbenchPanel` and `ArtifactCanvas` move onto the Artifact API, as the
plan already says.

## 6. Continue working (L2)

Continue working opens the authoring Agent's own conversation for the
artifact's workspace (the continuous conversation doc 64 defines, opened in
the app even when the artifact was made through a channel), with the
artifact attached in the composer. Nothing is sent until the
person writes and sends. If that Agent is paused, archived or revoked,
Continue working is unavailable and says why (invariant 37).

- **The attachment.** The artifact travels as a typed attachment on the
  person's message, the way a sent file does (`MessageMeta::attachments`,
  invariant 39). A new `AttachedArtifact { block, key, path, version,
  digest, mode }` names the text block that tells the model about the
  artifact, and the client draws the artifact there instead of that block.
  The turn request carries artifact keys, and the server renders each block
  at admission from its own records, as it renders an inbox file's note, so
  a client cannot write what the model is told.
- **It stays with its turn.** The block is part of the person's message, so
  it replays exactly from the ledger (invariant 1). It is that turn's own
  content, not tail material: doc 68 rebuilds the tail for every request,
  and closed turns do not carry theirs (invariant 36). L2 makes the turn's
  TurnCard name the artifact, so later turns find it by relevance and
  anaphora (doc 68) or by `recall`.
- **What the block says.** The artifact's title, kind and path; the current
  version's number and digest; one line per version (number, who, when, and
  the conversation and `turn_id` that made it); and its parts. It carries
  no conversation text. Comments on the current version that the person
  picks in the composer are quoted in their own message, attributed, as
  data the Agent works from and never as instructions to it.
- **Reach-back.** `recall` today answers only from the current session's
  history (`resolve_indexed_recall`, `vak-core`). L2 adds a
  cross-conversation scope to it, limited to the conversations named by the
  attachments in this conversation:
  - addressed by conversation and stable `turn_id`, never by turn number,
    which collides across conversations;
  - re-checked at every call for trash and audience, so a conversation
    trashed after the attachment was sent is unreachable;
  - limited to the same Agent and workspace; the existing scoped history
    already refuses a session outside this Agent's home and workspace.

  The result is an ordinary tool result in this conversation's ledger, so it
  is logged like any other. Nothing is pasted wholesale. Doc 68 gains a
  section for the cross-conversation scope when L2 is built.
- **Trash after sending.** The block holds only the artifact's own
  metadata, which the person brought into this conversation by sending it.
  The conversations that made the artifact are reached only through
  `recall`, so trashing one later hides it here too.
- **Revisions.** A change the Agent makes is a new version (§2.1). `write`
  writes into the workspace (invariant 35), so continuing a dashboard
  changes the saved file directly; the earlier version stays in the
  timeline, its body taken from the call that wrote it. An Office change is
  a draft for Review, as today. Sending every kind through Review waits for
  M4's copy environment and is not part of L2.
- **Make another.** The same, with the composer prefilled with an editable
  request to make a new artifact like this one, which the person sends. Its
  attachment's `mode` is `another`. In that turn the agent loop refuses a
  `write`, `edit` or `office_apply` call that targets the source's path,
  with a message the model can act on, as it already refuses copying a
  delivered draft out of scratch. The result is therefore a new entry. A
  command that changes the source anyway is recorded as what it is: a
  version of the source.
- **Another Agent.** Not in L2. Handing an artifact to another Agent is the
  collaboration plan's handover (`docs/plans/collaboration-and-shared-work-plan.md`
  §4 and §6, stage C4): the person sees exactly what is disclosed and to
  whom, the other Agent works from that packet, and its result returns into
  this artifact through Review. Ordinary read access never discloses a
  conversation to another Agent or its inference provider.

## 7. Editing by hand and putting it back (M8)

- **In the app**: text, Markdown and code in the editor; Office files in the
  existing rooms; HTML source in the Canvas. Saving makes a version credited
  to the person.
- **Outside**: Download, edit in Word or Excel, then Put back. Each download
  is recorded with the digest of the version downloaded. An upload is
  matched to the artifact whose download it most plausibly came from (the
  person confirms, and chooses when there is more than one candidate),
  saved as theirs, and diffed against the version recorded at download, not
  the latest one. When that is no longer the latest version, the upload is
  a sibling of the versions made since (doc 73 §10), reconciled through
  Review, never a silent overwrite. Nothing is written into the file to
  track it. A parse of an uploaded file runs in the worker (invariants 14
  and 39).
- The Agent sees the person's edit as the current version the next time the
  artifact is attached or its file read: the attachment lists it, credited
  to the person, so the Agent builds on it rather than on what it last
  wrote.

## 8. Sharing (M8)

Grants are per artifact (viewer, commenter, editor), following doc 73 §10. A
share shows:

- the current version, and earlier versions only when the owner includes
  the history, and then only from the version the owner picks, so a draft
  that held something later removed is not exposed by default;
- each shown version's author kind (Vak, the owner, a named collaborator)
  and date;
- comments made through the share; review comments made before sharing
  only when the owner adds them.

It **never shows conversation text**: version labels drawn from a turn,
Conversations, Sources and attachment blocks are omitted unless the owner
adds a conversation to the share explicitly. A person outside the owner's
devices signs in through doc 69's invitations, its principal and scoped
grant layer; doc 78 covers only the owner. Reaching the server by a
hostname follows invariant 34, and a share relaxes none of these. Office
rooms become an artifact's editing mode; coworking on a conversation stays
what it is.

## 9. Phases

| Phase | Rides on | Delivers |
|---|---|---|
| **L1 Read-only Library** | nothing in the data architecture | `Tool::artifact` replacing `produces_artifact`, forwarded by `BrokeredTool`; `title`/`summary` on `write` and `office_apply`; a derived table of declarations (one row per declaring call: Agent, workspace, session, turn, call, tool, path, title, summary, time) in the `vak-store` cache, fed by the incremental ingest that already runs after each turn and rebuilt when its generation changes; the Library screen and artifact page over declarations, drafts and promotions, filtered per session for trash and per Agent for audience; glimpses; Open conversation; Download; Open in Library from chat, Canvas and Workbench |
| **L2 Continue working** | L1 | `AttachedArtifact` and its server-rendered block; Continue working and Make another in the composer; the TurnCard naming the artifact; the cross-conversation `recall` scope with per-call trash and audience checks; the Make another write refusal; doc 68's section |
| **L3 Search** | M6 | content and lineage search through the catalog; Sources and Related |
| **L4 Identity and edits** | M4, M8 | `art_` ids; a routine's runs as versions of one artifact; sibling versions; rename, star, folders and tags, move, archive; edit in app; Put back; saved cards; draft fading as a retention label (M7a) |
| **L5 Sharing and live** | M8, doc 81 P1 | artifact grants; Keep live hands an artifact to the piece platform |

L1 touches the Tool trait and the broker, the agent loop's completion
evidence, the `vak-store` ingest, the server projection (`DraftVersions`),
the sidebar and the Canvas. `docs/plans/data-architecture-blast-radius.md`
lists what M6 and M8 remove of it, so the prototype is deleted rather than
kept for compatibility (invariant 30).

The API (`GET /library`, `GET /library/{key}`) keeps its shape across
phases; the key's format changes at M8. A turn request names artifacts by
key the way it names inbox files, and the server renders their blocks at
admission (§6). The API sits behind the owner's authentication, and an
invitation token never reaches it (doc 69).

### 9.1 Exit tests

- **L1**:
  - `library_lists_declared_deliverables_across_conversations`: a report
    from `office_apply` and a dashboard from a titled `write`, made in two
    conversations a week apart, are both found by title and by "the board
    meeting" and open in the Canvas;
  - `untitled_write_still_satisfies_saved_file_outcome`;
  - `undeclared_write_is_not_an_entry`;
  - `card_path_needs_a_recorded_change_in_the_workspace`;
  - `library_omits_versions_from_trashed_sessions`, including a worker's
    session and a revision's child session;
  - `library_lists_only_admitted_agents`, with two Agents sharing one
    records file;
  - `generic_name_reuse_starts_a_new_artifact`;
  - `routine_run_output_is_listed`;
  - `download_is_an_attachment`;
  - nothing durable is written beyond the session ledgers, and every cache
    it adds is declared.
- **L2**: `artifact_attachment_replays_exactly`;
  `attachment_is_part_of_its_turn` (a later turn reaches it through the
  working set or `recall` after its turn closed);
  `recall_refuses_trashed_origin`; `recall_refuses_unlisted_conversation`;
  `make_another_never_writes_the_source`; and a live acceptance run
  (AGENTS.md acceptance contract) in which continuing a week-old dashboard
  answers a question about its first version's data without the person
  restating it.
- **L3**: "the deck with the churn chart" finds the deck by its content.
- **L4**: an Excel file edited outside Vak and put back becomes the person's
  version with a correct diff, and the Agent builds on it; a weekly
  routine's reports are versions of one artifact.
- **L5**: a guest invited through doc 69 views a shared dashboard's current
  version with no conversation text and no earlier versions exposed.

### 9.2 What M3b discards

M3b removes every Vak-owned data root and refuses state from before the
data baseline (plan §1, L3)
(invariant 29). Derived keys, glimpses, the declarations table, artifact
attachments in L1/L2 ledgers and any link a person saved to a Library entry
do not survive it, and nothing maps them forward. L1 and L2 are worth
building before M3b for what they teach (§9.3), all of which carries
forward as design; their data does not.

### 9.3 What the prototype must teach

1. **Do models declare?** How often a requested deliverable gets a `title`
   or a presenting card, per model family, and how often a step file gets a
   title by mistake: measured in `vak eval` with fixtures that ask for files
   (§10, question 2).
2. **Can people find things?** How many find-it tasks ("last week's
   report", "the deck for the board meeting") acceptance runs answer from
   the Library without opening a conversation.
3. **Does reach-back work?** In L2's acceptance runs, how often a Continue
   working turn calls `recall` across conversations when it needs history,
   and whether its answer then uses what it read.

The answers settle §10's questions and shape the M8 screens. None of this
is product telemetry: there is none before M5, and it never carries
content.

## 10. Open questions

1. **A turn that wrote files but declared none.** Recommended: nothing
   enters the Library, and the Workbench says the files exist; a person can
   save one, which is a declaration (M8). The alternative is one "Files from
   this conversation" entry per turn, which brings back the volume problem.
2. **Whether `write` should require `title`.** Recommended: keep it
   optional and measure how often small models omit it on deliverables in
   the eval suite before deciding (§9.3). The same eval compares a titled
   `write` with presenting the file through a card, the channel already
   measured as reliable with small local models (`07-prompt.md` v3.4.5).
3. **Where a non-built-in Agent's deliverables live after M3b.**
   Resolved by plan revision 3 (L10) and doc 73 §5, as decision 4 assumed:
   an Agent's workspace is a Workspace bound to (Space, Agent), never an
   Environment, and its deliverables reach the Space's working tree
   through Review, like any candidate.

## 11. Non-goals

- A general file manager for the workspace. The Library lists what Vak made
  and what people saved, not every file on disk.
- A conversation per artifact. Work on an artifact happens in its Agent's
  conversation (decision 3).
- Git projects and large codebases. They stay in their repositories; a
  changeset is an artifact only as doc 73 §10 describes.
- Listing every card. Cards are moments unless saved.
