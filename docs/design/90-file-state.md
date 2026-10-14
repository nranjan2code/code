# 90 — What a turn knows about a file

Status: in progress. Decisions taken 2026-10-12 (D1–D4, §4). Built 2026-10-12: stage 1 (`vak_core::file_state`, `Version::turn`, `Version::undone`), stage 2 (the note on every path a call reads or changes, the record's fact first after a failure) and the listing half of stage 3 (`Tool::lists_paths`: a glob is told of matching drafts not in the folder). A draft for review is now recorded as a version from the call's own draft, its revision as its child (`ArtifactClaim::drafted`, `based_on`); before, none was. A `session_search` hit names its file and state. Stage 4 part 1 built: an edit made outside Vak is a `VersionSource::Workspace` version recorded when Vak next reads the file, and a draft made before the workspace's current version is `behind`. Stage 4 part 2 built: accepting a version that is behind the workspace's current copy is refused until the person says to replace it (§3.4), the workspace's own history is its accepted and found versions, and a file that was in the folder before the record knew it is the record's first version. Fixed 2026-10-13 from live runs: a call's versions are recorded before its file is described, so Vak's own write is never found as an edit made outside it; a call that writes the workspace file itself (`write`, `edit`, a file `bash` reports changing) makes a `VersionSource::Call { in_workspace: true }` version, the child of what the workspace last held (`file_state::last_held`), which is never a draft; a draft without a name is described by its source, never as made by a person Vak did not record; and a copy found in the workspace is history once the workspace moves on, never a draft. Built 2026-10-13, §5.1's Agent half: a version names its conversation (from its source) and its turn (from its event's trace, `Version::turn`) by the plain ids `recall` takes (`turn_id`, with `conversation` for another one), and the note says "made in turn T of this conversation" or "of conversation C", and "made from version V" (`file_state::Reader`); a new file named by an absolute path through a link (`/tmp` for `/private/tmp`) is the workspace's (`vak_tools::context::resolved`), where before its draft was never recorded. Not yet: the inputs a turn read (`derived_from`) in the note. Decided 2026-10-14 (maintainer): `recall` opens another conversation only through an artifact attached in this one, as it does now, so conversations stay apart; its refusal says so. Stage 5 built 2026-10-13, to the mockup the maintainer approved as it stands: each version's state (`file_state::VersionIs`, `version_states`) and what a file needs of the person (`file_state::Needs`) are served by `GET /library` and `/library/{id}` (`now`, `needs`) and `GET /library/made/{session}/{call}` (the version a card's call made, as it is today); the artifact page has the version graph, each version's state as a word and a mark, a notice when a waiting draft was made before the folder's copy, and where the chosen version came from (who made it, its conversation, the version it was made from); a conversation's file card shows its version's state today and opens its history; the composer has the "Needs you" strip; the Library filters by state. A copy the workspace held is never a draft, so the lifecycle never trashes it (`Version::held_by_workspace`). Built 2026-10-13, the rest of §5 and §5.1: a version's own title is the `change` its call gave (`write`, `office_apply`; `Version::title`), "Version N" without one; what its turn read first is a record field (`Version::derived_from`: each workspace file, draft and page the turn's earlier calls recorded reading, from `CallEffect::FileRead` and the new `CallEffect::Fetched`, which a tool declares with `Tool::fetched`), sealed in the artifact's rows and told in the note ("after reading …"); "You asked" is read on demand from the turn's directive (`GET /library/{id}/versions/{v}/origin`, owner-only, the trash honoured), never copied into a shared row; and Compare with your folder (`…/compare`) says whether the folder holds the version, which version it holds, and marks the changed words side by side; for an Office file or PDF it lists what the version changes from the folder's copy, worked out in the worker as Review works out a draft's (`review`, with no choices, since nothing is accepted there; built 2026-10-13). Deciding whether bytes are already a version and recording one happen under one lock (`artifacts::recording`), so a call's write and an observation of the folder never record the same bytes twice. Builds on the
artifact record of data-architecture M8 (`vak_core::artifacts`,
docs/design/82-library.md) and the Review loop of
docs/design/72-openxml-documents.md and 54-task-environments-and-promotion.md.

## 1. The failure

Live on 2026-10-12, the model made `porto-notes.docx`. The draft went to
Review, and two turns later it was asked to add a point to "the draft". It
read `porto-notes.docx` and was told the file does not exist. It globbed the
workspace and found nothing. It searched past sessions and found nothing.
Once, it named the draft's own path and was refused as "inside .vak". Then
it gave up or made the file again from scratch. Every one of those answers
was true about the workspace folder, and every one was wrong about the
file.

Two turns is the easy case. The same request can come a hundred turns
later, in another session, after the person accepted, undid, edited the
file by hand, or kept two sibling drafts. Whatever happened, the model must
be told what the file is now, at the moment it touches it, from the record,
never from its memory of the conversation and never from a scan of folders.

## 2. What the record already holds

Everything Vak does to a path is in the artifact record (the M8 map,
`crates/vak-core/src/artifacts.rs`):

| Fact | Where |
|---|---|
| The artifact is keyed by space and workspace path | `artifact_id(space, path)`, :349 |
| Every version with its parent, digest, bytes and source (a call, a Review candidate, a person) | `Version`, :208; bytes in the tenant object store, so a draft outlives its scratch file |
| Proposed for Review by which candidate | `Version::proposed` |
| Accepted into the workspace, and undone | `Promoted`, `PromotionUndone` |
| Saved, downloaded, commented, trashed, erased by a person or by the lifecycle | `Saved`, `Downloaded`, `Commented`, `Trashed`, `Erased` |
| A deletion proposed by a draft | `Removed` |
| Siblings (two drafts from one parent) | `Artifact::heads()` |

What it does not hold, or does not let anyone ask:

- A lookup by path, and a query of the versions with their states. A reader
  must hash the space and path and walk the parents by hand.
- "Undone" as a state. An undone version reads as pending again, with
  nothing to say it was once accepted.
- "Superseded" as a state. It is computed from the children.
- The turn a version was made in. The fold drops the trace that carries it.
- The workspace file as it is now. Nothing watches the workspace, so an
  edit made outside Vak, a rename or a deletion is never seen, and an
  accepted version looks current after the person rewrote the file.

## 3. The design

### 3.1 One projection: the state of a path

`vak_core::file_state(space, path) -> FileState` is a pure function of the
artifact record and one observation of the workspace (whether the file
exists, and its digest, read at that moment):

```text
FileState {
    path,
    workspace: Absent | Present { digest },
    // How the workspace file relates to the record:
    workspace_is: Unrecorded            // no artifact for this path
                | Version(id)          // the bytes of a recorded version (accepted, or saved)
                | Changed { since }    // differs from every version: edited outside Vak since `since`
                | Missing { accepted } // an accepted version's file is gone from the workspace
    drafts: [ Draft { version, draft_name, made_in: turn, from: parent, state } ]
                // every head that is not in the workspace: waiting for review,
                // undone, saved, trashed (restorable until erased), erased (no bytes)
    owner: this Agent | another Agent
}
```

States are typed and exclusive. `Draft::state` is one of `Waiting` (proposed
and not accepted), `Undone` (accepted once, then put back), `Saved` (kept by
a person, not in the workspace), `Trashed { until }` and `Erased`. A version
superseded by a child is not a head and does not appear. Siblings appear
side by side with the turn each came from.

### 3.2 Told when a path is touched

The loop resolves the state of every path a call names. A tool says which
path it reads or writes with `Tool::file_access`, so no tool is named. The
loop attaches one typed note to the result, worded in `vak_control`, when
the record says something the folder alone does not. Read failures and
successes both get it:

| Situation | What the note says |
|---|---|
| The file is absent; drafts wait | "`porto-notes.docx` is not in the workspace. 2 drafts await review: `.vak/scratch/vak/call_4yhpeqr0/porto-notes.docx` (turn 41, newest), `…/call_uk75anxh/…` (turn 38). To change it, pass the draft as `source`." |
| The file is present; a newer draft waits | "You read the workspace copy. A draft made after it awaits review: `…` (turn 63). The draft is what the person has not yet seen applied." |
| The file was changed outside Vak since the version a draft came from | "`report.docx` changed in the workspace after draft `…` was made from it. The draft does not include those changes." |
| A file the workspace held is missing from it | "`notes.docx` was in the workspace as version ver_… and is not there now." Said of the version the workspace last held, accepted or found there; "accepted" only for one Review promoted. |
| A draft was undone | "Draft `…` was accepted, then undone by the person." |
| A draft was erased | "The draft from turn 9 was erased; its contents are gone." |
| Another Agent owns the file | "`plan.xlsx` belongs to Agent Ops; this Agent cannot continue it." |

Nothing is noted when the workspace file is the only version, or there is
no record. The note states facts. It never chooses for the model, and it
never changes a call.

### 3.3 Found without the name

"The notes you made last week" names no path. The data catalog already
indexes artifacts (M8). `session_search` and `recall` return artifacts with
their path and state, and `glob` lists pending drafts whose workspace path
matches its pattern beside the workspace files, each marked as a draft. All
of these are read loop-side or Core-side, so the worker never reaches the
store (invariant 14).

### 3.4 Changing a file

A change names the file it is for (`path`) and the bytes it starts from
(`source`, default the workspace file). `office_apply` already reads a draft
named as `path` as a revision from that draft (2026-10-12). `write` and
`edit` get the same: a call that changes a path with waiting drafts is told
which bytes it started from. The record links the new version to its
parent. Review shows what was based on what.

Accepting has two checks, and neither is the loop's. Promotion's own
(`vak_sandbox::promote_recoverable`) compares the workspace file with what
it held when the candidate was frozen for Review, so it catches a change
made while the draft was in Review. It cannot see a change made between the
draft and the freeze: a draft made on Monday, the file edited by hand on
Tuesday and Review opened on Wednesday froze Tuesday's file as its base and
would replace it. That one is the record's: `POST
/library/{id}/versions/{version}/accept` observes each file it would
replace (which records an outside edit as a version, D2) and, when the
version is behind the workspace's current one (`file_state::behind`),
answers 409 with `kind: "behind"` and the files, and changes nothing.
Review then says the file changed after the draft was made and that the
draft does not include it, and the person either keeps the draft or
accepts with `replace_newer`. The replaced copy stays a version, and Undo
puts it back.

Three facts make `behind` true only when it is:

- The version put up for Review is the draft the call recorded, with the
  parent it was made from, never a second version made from the newest
  head. A candidate with no recorded draft (work done by a command) is a
  child of the version holding the bytes the workspace had at the freeze.
- A draft made from the workspace file is a child of the workspace's
  current version, read when the draft is recorded, never of whichever
  draft is newest.
- The workspace's own history is the versions accepted into it and the
  versions found in it. A file that was in the folder before Vak recorded
  anything of it is found as the first version, with no parent, and is not
  an edit made outside Vak.

### 3.5 Record what is missing (additive, invariant 29)

- `Version::turn`: the turn id from the event's trace, kept by the fold.
- `Version::undone`: set by `PromotionUndone`, cleared by a later promotion.
- `Observed` versions: when Vak reads a workspace file whose digest matches
  no version of a recorded artifact, the loop records an `Observed` version
  (source `Workspace`, actor the owner) with its bytes. "Changed outside Vak
  since" is then a fact in the record, and the history shows the person's
  edit. This is decision D2 below.
- One decision, one append: whether bytes are already a version, and
  the version that records them, are decided in one
  `Artifacts::transaction`. Its rows are appended only if the `artifacts/`
  chain did not move since the transaction read its rollup
  (`RecordChain::append_all_after`, checked under the chain's writer
  lock, which every process takes); when another writer appended first,
  the decision is made again on what the chain holds now. So the server,
  the CLI and any other process on one data home never record a call's
  write and the folder's copy of it as two versions, and no writer holds
  a lock of its own (built 2026-10-14; it replaces a lock that held only
  within one process).

## 4. Decisions (the maintainer, 2026-10-12)

- **D1. What "change the file" starts from when a draft waits:** the model
  is told the workspace copy and every waiting draft, and names its
  `source`; the runtime never picks.
- **D2. Edits outside Vak:** recorded. When Vak next reads a file whose
  bytes match no version, it records an `Observed` version, so the history
  shows the edit and a draft made from an older base is flagged as missing
  it.
- **D3. Sibling drafts:** all heads are listed, newest first, each with the
  turn it came from.
- **D4. Vak's own record and words only:** versions live in Vak's storage
  and are described in Vak's own terms. No Git repository, Git vocabulary or
  Git binding.

## 5. One view for the person and the Agent

The model's note (§3.2) and what the person sees are two renderings of one
`FileState`, never two contracts (invariant 30). The person sees it in
three places, from the same server projection (`GET /library/files?path=`
and `/library/{id}/state`):

1. **The artifact page's history** (doc 82 §5.3) becomes a version graph,
   not a list. One lane for the workspace file, with its accepted versions
   and `Observed` outside edits, and a branch for each draft, with siblings
   side by side. Each node carries its state as a word and a mark: waiting,
   accepted, undone, saved, in the trash until a date, erased, and changed
   outside Vak. It also says who made it and links to the turn that did, in
   that conversation. Every node's actions are the ones its state allows:
   Review, Accept, Undo, Restore, Compare with the workspace copy, Continue
   from this version, and Download. Nothing offered can fail for a reason
   the page already knows.
2. **In the conversation**, a draft card shows the file's live state, not
   the state when the card was made. "Waiting for review" becomes "Accepted
   12 Oct" or "Undone" or "Changed outside Vak since", read from the
   projection when the card is drawn. A strip above the composer lists
   the files this conversation has in play that need the person: drafts
   waiting, and files changed since a draft was made.
3. **The Library list** (doc 82 §4) filters by state ("Waiting for review",
   "Changed outside Vak", "In the trash"). Its glimpse is the head the
   person would act on: the newest waiting draft, else the workspace file.

### 5.1 Where it came from, and the way back

Every version names its origin from the record, never from guessing:

| Origin | From the record | Reached by |
|---|---|---|
| A turn of a conversation | the event's trace: Agent, session, turn, call, run (§3.5 keeps `turn`; stage 1) | opening the conversation at that turn, with the call highlighted |
| What the turn was given | the reads and fetches before the call in that turn (`derived_from`, the catalog's lineage, M6.4): the workspace files, the earlier drafts and the web pages it read | each input as a link: a file to its own page, a page to its URL |
| Why the turn ran | the run's cause: the person's message, an automation and its trigger, a channel message, a worker for a parent | the message, the automation, or the parent conversation |
| A person | `Person` source, with the action (put back, edited, saved) | that version's comments and its Review |
| Outside Vak | an `Observed` version (D2), with when Vak noticed | the version before it, to compare |

On the artifact page each node opens its origin. The Agent gets the same
facts in the note (§3.2), "made in turn 41 of this conversation from
`budget.xlsx` and the earlier draft", and reaches them with `recall` by
turn or evidence id. In another conversation, `recall` reaches back only
where the grants allow (M8.3b), and the note says when the origin is a
conversation this one cannot open.

The Agent reaches the same thing through `recall` and `session_search`
(§3.3) and the note on every path it touches (§3.2). Whatever the person
does on the page, the next turn reads it, because both come from the
record.

The design follows doc 75 and DESIGN.md: everyday words, no ids or hashes
unless Show technical details is on, one accent for live state, and a
working layout at 390 × 844. Before the page is built, a reference mockup of
the version graph, the card states and the in-play strip, in light and dark
and on a phone, goes to the maintainer for review (doc 70's ledger): it is
`docs/assets/file-history-2026/file-history.html` (2026-10-12, awaiting
review).

## 6. Stages

1. The projection and its record fields (§3.1, §3.5 `turn` and `undone`),
   with a test per row of §3.2's table, built from real record steps.
2. The note on every path a call touches (§3.2), with a live acceptance
   run: make a file, revise it 40 turns later, accept, edit it by hand,
   revise again, undo, and ask for "the old draft" in a new session.
3. Finding without the name (§3.3).
4. `Observed` versions and the base check at promotion (§3.4, §3.5), per D2.
5. The person's view (§5): the mockup for review first, then the version
   graph on the artifact page, live card states, the in-play strip and the
   Library's state filters, verified in the browser at 1440 × 900 and
   390 × 844, light and dark.
