# 90 — What a turn knows about a file

Status: in progress. Decisions taken 2026-10-12 (D1–D4, §4). Built 2026-10-12: stage 1 (`vak_core::file_state`, `Version::turn`, `Version::undone`), stage 2 (the note on every path a call reads or changes, the record's fact first after a failure) and the listing half of stage 3 (`Tool::lists_paths`: a glob is told of matching drafts not in the folder). A draft for review is now recorded as a version from the call's own draft, its revision as its child (`ArtifactClaim::drafted`, `based_on`); before, none was. Next: `recall`/`session_search` reaching artifacts (§3.3), then stage 4. Builds on the
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
| An accepted file is missing from the workspace | "`notes.docx` was accepted in turn 12 and is no longer in the workspace." |
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
parent. Review shows what was based on what. Promotion refuses a version
whose base no longer matches the workspace file. This is to be verified;
it is promotion's own check, not the loop's.

### 3.5 Record what is missing (additive, invariant 29)

- `Version::turn`: the turn id from the event's trace, kept by the fold.
- `Version::undone`: set by `PromotionUndone`, cleared by a later promotion.
- `Observed` versions: when Vak reads a workspace file whose digest matches
  no version of a recorded artifact, the loop records an `Observed` version
  (source `Workspace`, actor the owner) with its bytes. "Changed outside Vak
  since" is then a fact in the record, and the history shows the person's
  edit. This is decision D2 below.

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
and on a phone, goes to the maintainer for review (doc 70's ledger).

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
