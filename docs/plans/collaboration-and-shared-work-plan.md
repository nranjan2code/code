# Plan — collaboration, handover, Canvas and Review

Status: **planning proposal, 2026-09-29. No implementation authorized or started.**
Consolidates the maintainer's discussion of human–human, human–Agent,
Agent–human and Agent–Agent collaboration, including another person's own
Agent, the typing interaction, Canvas, Review, and headless use. Source and
design review checked at `b4a96685`; live usability testing has not been done
for this proposal. The maintainer explicitly requested planning only.

## 1. Direction and decisions

**Requested direction:** collaboration should feel natural for all four
relationships. People should understand who is doing what, see work change,
hand work to an Agent from where they are working, and review the result.
The same experience must make sense in desktop, web, and headless use. The
supplied September 29, 10:56 PM Review screenshot is a concrete example of
the current experience lacking clarity and finesse.

**Recommended design, still proposed:** one shared work loop, contextual
handover from conversation and Canvas, ordinary language with optional
`@` selection, and Canvas and Review as views of the same versioned result.
Person-bound accounts, participation by another person's Agent, remote
Agent transport, and the stages below are proposals requiring design review
and subsequent implementation authorization. Earlier conversational examples
are intended behavior, not evidence of existing features.

The shared loop is:

**Discuss → request or hand over → work → return a result → review → revise
or accept.** Ordinary messages and small requests stay lightweight. A
greeting, passing comment, or mention does not create a work item.

## 2. What the review found

| Area | Existing foundation | Gap to address |
| --- | --- | --- |
| Shared conversation | Scoped invitations, attributed messages, presence and refresh | Participant messages do not start work and can be refused while an Agent runs. There is no clear request-to-response state for the participant. |
| Shared drafts | Saved versions, anchored comments, Office/PDF branches and conservative conflict handling | Feedback lacks a complete discussion, assignment, response and resolution journey. |
| Review | Exact candidates, checks, selected files, promotion and undo | Document inspection and the decision are separated; selection and wording can imply changes where there are none. |
| Human identity | One owner; invitation-specific principals authenticated by bearer code | A name on a transferable code does not prove which person holds it. Accountable recurring collaborators and their Agents need a durable identity relationship. |
| Agent work | Managed work, child sessions, evidence, bounded delegation | Parent/worker execution does not provide peer participation or another person's Agent handover. |
| Desktop and headless web | Shared client, server-side execution, browser owner authentication | The guest view has a different experience; multi-person return, attention and decision journeys need validation. |

Code anchors: `crates/vak-server/src/lib.rs` contains
`create_coworking_message`, `participant_read_route_allowed`,
`request_revision_from_candidate_comment` and `create_coworking_invitation`.
Client anchors are `SharedConversation.tsx`, `CoworkingShare.tsx`,
`WorkbenchPanel.tsx`, `ArtifactCanvas.tsx` and `OfficeWorkspacePane.tsx`
under `crates/vak-client-ui/src/components/`.

Document 69 describes participant revision requests in its acceptance text;
the current route boundary permits comments and messages but excludes their
revision dispatch. Its text must be reconciled with the implemented scope.
Document 78 is in progress and scopes identity to one owner; document 79's
private hosted fleet remains a proposal. None establishes multiuser Agent
ownership as shipped.

## 3. One model for people and Agents

Participants share a conversation, selected results, and explicitly admitted
pieces of work. Each contribution records its real actor. An Agent also
records whose authority it acts under. The responsible person, executor,
reviewer, and person allowed to accept a result can be different.

| Relationship | Expected interaction |
| --- | --- |
| Human → human | Ask a colleague to review or take responsibility for a bounded piece of work; they can accept, decline, clarify, contribute and return it. |
| Human → Agent | Ask the conversation's Agent or the person's own Agent to do a bounded task; see progress, answer questions and review the result. |
| Agent → human | Ask a named person for missing information, judgment, review or an authorized decision, with the relevant material attached. |
| Agent → Agent | Request specialist work or review within an existing collaboration grant; receive clarification, findings and evidence with attribution. |

Every participant can contribute and receive a response within their grant.
Reading, commenting, editing a draft, requesting execution, reviewing,
accepting files, and authorizing external effects are distinct capabilities.
Human approval gates remain human decisions under the existing contract.
Agent review is attributable advice and evidence, never a human signature.

Retain doc 64's durable owning Agent and private state boundaries. A
collaborating Agent participates in selected work; it does not become an
additional owner of the root session, acquire its private memory, or reroute
the human to another conversation. Human-to-human work can proceed while
the owning Agent is idle. No Agent run is required just to exchange comments.

## 4. How a person hands over work

The same action is available on a message, result, selected document area,
comment thread, or work item. It captures:

- **Who:** a person or admitted Agent, identified independently of its name.
- **What:** the requested outcome and any boundaries or existing criteria.
- **Context:** the selected result version, locations, comments and relevant
  conversation references. Additional context is shared explicitly.
- **Return:** where the result should appear and who should review it.

Most fields come from the current location. People should not fill in a
project-management form for every request. A short inline composer can show
“Mira · Draft v2, paragraph 4 · Shorten this paragraph” and a Send action.
Consequential or ambiguous scope needs a visible choice before dispatch.
Routine work inside existing grants proceeds without repeated confirmation.

Distinguish asking for help from handing over responsibility. “Check these
figures” assigns a review contribution; “take over the figures section”
assigns that piece of work while the human remains its requester. A human
recipient may decline or accept. An Agent may be admitted, queued, ask for
clarification, or report inability. Sending never fabricates acceptance.

Visible states: **Requested, Queued, Working, Needs an answer, Ready for
review, Completed, Declined, Failed, Cancelled.** Show only states applicable
to that work. Keep execution status separate from review status: producing
a draft does not establish that it has been reviewed or accepted.

The initiating surface shows the actual acknowledgment. A submitted request,
queued work and started execution are distinguishable. Retry must not create
a second assignment or repeat an effect. A handover survives reconnect and
has enough durable context to recover honestly after a restart.

## 5. Typing, mentions and Agent choice

Plain language is the default: “Mira, revise this draft using Asha's
comments.” Optional `@` autocomplete chooses an exact person or Agent and
adds a visible target chip. The picker includes “My Agents” and Agents
already admitted to this collaboration. No mandatory slash command or
special key is required; Send has the same meaning across the product.

An `@` mention alone addresses a message; it does not authorize execution.
Quoted text, pasted documents, and an Agent name mentioned in discussion
cannot dispatch work. The submitted authenticated request supplies intent;
the host validates the actual target, task and grant. Do not implement
handover by searching for an imperative verb or promoting a text prefix
into authority. This must be reconciled with doc 47's existing typed
control-plane rules before implementation.

For clear requests, Send submits the handover and the conversation shows
its receipt. When “this” could refer to several drafts, ask the person to
choose a result. If permission or context sharing is missing, explain the
specific missing grant instead of falsely reporting that the Agent started.

“Asha, could your Agent check the figures?” is a request to Asha. She can
choose her Agent; another participant cannot spend her Agent budget or
invoke it merely by naming it.

## 6. Another person's own Agent

The target includes both same-deployment Agents and Agents running on
another person's Vakyartha instance, including a headless server. These
require separate delivery stages; same-server support must not be labelled
complete support for “my Agent” everywhere.

Asha chooses Atlas, her Agent. Before Atlas receives shared material, the
host verifies Asha's Agent identity and the authority to disclose that
material to it. A previously established grant may cover this; otherwise
the authorized sharer sees the selected material and recipient. Ordinary
read access alone does not imply permission to disclose a conversation to
another Agent or its inference provider.

Atlas receives a bounded task packet and works under its own execution
policy intersected with the shared task's constraints. Its credentials and
private history stay with its owner. The initiating person sees which
account supplies the run budget; cost is not silently transferred to the
conversation owner. The exact provider/model remains technical detail,
while consequential disclosure and spending choices stay visible.

Atlas returns findings or a proposed version into the original shared work,
with its identity and acting person visible. Applying that version follows
the receiving workspace's existing review and acceptance boundary. Imported
files remain untrusted and pass the existing worker and verification paths.

Remote delivery needs authenticated instance and Agent identity, scoped
data exchange, idempotency, cancellation, expiry, delivery receipts and
explicit outcome-unknown recovery. It must not share owner cookies,
gateway bearers or filesystem mounts. This is collaboration between
instances; it does not require starting the proposed customer fleet or
moving the root conversation to another server. Arbitrary third-party
agent protocols require a later adapter with the same contract.

## 7. Canvas, conversation and Review

Conversation carries discussion and the shared work history. Canvas shows
the current artifact. Review focuses on the exact proposed changes and
decision. They retain one result/version identity and navigation context.

Proposed desktop arrangement:

```text
Title · Version · Contributors · Draft / Working / Ready for review
-----------------------------------------------------------------
                          | Context panel
 Document / artifact      | Changes · Discussion · Activity
 at a useful size         | Selected change or comment
                          | Handover and returned work
-----------------------------------------------------------------
Review decision, when needed: Request changes · Keep draft · Accept
```

The conversation remains available beside Canvas when space allows. Avoid
forcing conversation, document, file navigation and collaboration into four
permanent narrow columns. The context panel opens for the selected object,
and can close for focused reading. On phones it becomes a bottom sheet or
dedicated full-height view without covering the document action or input.

Selecting a paragraph, cell range, slide or code range offers **Comment,
Ask Agent, Hand over**. Each action carries the actual selection and saved
version. An interactive website preview needs an explicit pointing mode so
ordinary clicks still operate the preview. Image/visual-region annotations
must identify a version and region; do not imply a stable semantic anchor
where the renderer cannot provide one.

Presence means an observed participant is present. Agent activity comes
from real execution events. Show useful milestones such as “Checking the
figures” or “New draft ready,” with evidence behind them; do not fabricate
cursors, percentage progress or completed edits. A new version gets a
visible notice and comparison action. Never silently switch the version a
person is reading or transfer old comments to newly numbered lines.

Shared views use the same result, discussion and review components for
owners and invitees. Visible content still respects each audience grant;
component reuse must not grant access to the general owner API. Controls
reflect actual capabilities and explain material limitations in plain words.

## 8. Review quality and the supplied screenshot

The screenshot shows a missing document preview, a disconnected focus
button, excessive width between related controls, repeated filename and
zero counts, truncated check explanation, and an unchanged selected file
described in the footer as a change to apply. These are both visual and
decision-state problems.

The proposed Review should answer, in order: **What am I looking at? What
changed? What remains uncertain? What will my decision do?**

- Keep the document visible in the shared workspace. Use the existing
  structured reader where available and accurately describe its fidelity;
  a reflowed Office view is not a page-faithful renderer.
- Selecting a change reveals its location and before/after content. Keep
  the exact version, relevant author and review state near the document.
- Distinguish new files, edits, deletions, unchanged files, conflicts,
  unreadable content and changes whose visual meaning is unavailable.
  “No visible change” does not prove byte identity or no effect.
- Do not preselect unchanged files as changes. If a current comparison
  verifies that all selected draft bytes already match the destination,
  say “No changes to apply” and offer inspection, revision and Close.
  A hash comparison against the export-time baseline alone cannot justify
  saying the workspace matches now. Report stale or unavailable comparison
  honestly and recheck the destination at acceptance.
- Use “Selected for acceptance” before the click and “Applying” only during
  the operation. The primary action names the actual accepted scope.
- Put concise check results by the decision and let the explanation wrap.
  Keep structural checks, visual inspection and semantic review distinct.
- Bind the decision to its exact version, selected changes and destination.
  Changed content invalidates affected review decisions. Acceptance,
  executable setup and external publishing retain separate authority.

Follow `DESIGN.md`: readable type, Ink and Saffron, restrained borders,
grouped actions, useful content width, accessible contrast, clear focus and
technical detail on request. Light, dark and mobile need designed states,
not just a compressed desktop layout. No new universal pixel editor or
character-level concurrent editor is implied by this proposal.

## 9. Discussion, review and decisions

Comments form anchored threads with replies. A thread can request a change,
be assigned for work, receive a linked response/version, and be resolved or
reopened with attribution. “Addressed in version 3” is separate from
“Reviewer confirmed”; an Agent claiming a fix does not automatically close
someone else's objection. Preserve disagreement and the eventual reason
for a decision. Required reviews are explicit; ordinary comments do not
silently block all acceptance.

A review request identifies the reviewer, object version and question or
criteria. Reviewers can recommend acceptance, request changes or explain
why they cannot review. Evidence supports the result; it does not invent
authority to accept it. Use ordinary language such as “Waiting for Asha's
review” and “Mira addressed two comments; one still needs a decision.”

## 10. Headless, return and attention

A headless installation runs the work and serves the same browser
experience. Closing a browser does not cancel admitted work. Show the
execution location when it matters: where files will be accepted, where
the work continues, and whether the relevant server is reachable.

Returning participants see what changed since their last visit, requests
addressed to them, and the exact draft awaiting review. Notifications follow
audience permissions and target the relevant person. Notify on a request,
blocking question, review-ready result, failure or access change according
to preferences; avoid a notification for every progress event. Reuse the
inbox/outbox mechanisms instead of adding another delivery system.

Distinguish browser reconnect from server restart. Reconnect restores
current state; restart must reconcile interrupted execution and outstanding
decisions without blindly rerunning effects. Presence is ephemeral and
must not survive as a false “online” claim. Pending work may remain durable
without preserving an expired approval gate; obtain a fresh bound decision
when necessary. Silence never accepts a draft or approves an action.

Channels and CLI present the same work and decision identities in an
appropriate text projection, with authorized browser links for rich review.
Support response and review without requiring a desktop shell. Do not
claim phone parity until a real invitation, handover and review journey
has been completed on the narrow screen.

## 11. Architecture and delivery boundaries

Reuse the session ledger, outcome/work contracts, result and candidate
identities, scoped grants, work receipts, and inbox/outbox. Extend their
vocabulary where a consumer needs it; do not add a parallel scheduler,
task store or collaboration engine. Domain editing stays in the existing
Office/PDF and artifact components. Remote protocols belong behind a
bounded integration boundary.

Before implementing, reconcile this plan with:

- `64-agent-owned-platform.md`: owning Agent, audience and private memory.
- `69-shared-conversation-coworking.md`: grants, feedback, editing and
  owner/participant scope.
- `42-managed-work-contracts.md`, `47-commitment-kernel.md` and
  `52-outcome-directed-runtime.md`: assignment, intent, typed control and
  requirement revisions.
- `24-agent-security.md`, `08-permissions.md` and
  `50-call-and-evidence-contract.md`: trust, dispatch and evidence.
- `54-task-environments-and-promotion.md`, `66-immersive-artifact-canvas.md`,
  `70-calm-agent-experience-implementation.md`, `72-openxml-documents.md`
  and `77-pdf-documents.md`: editing, review and surface contracts.
- `78-headless-identity.md`: invited person accounts are an explicit design
  extension, not an implication of the current single-owner login.

Coordinate with the pending data architecture plan before introducing
durable records. This proposal starts none of M1–M9 or the fleet work.
New implementation must respect the applicable canonical path, registry,
UUIDv7, content-free logging and trash rules. An incompatible ownership or
schema change needs an explicit version/baseline decision before coding.
Record which earlier surface/control rules are replaced when approved.

## 12. Proposed order of work

| Stage | Deliverable | Exit condition |
| --- | --- | --- |
| C0 — Finish the design | Resolve ownership/grant semantics; interaction map; visual mockups for conversation, handover, Canvas, Review, unchanged/conflicted drafts and phone; account and remote boundary decision | Maintainer can walk the four actor relationships and the screenshot's case without an unexplained step. Mockups are visibly proposals. Implementation authorization is separate. |
| C1 — Make Review coherent | Shared result/version context, visible document, correct change states, clear decisions and responsive layout | Changed, new, deleted, unchanged, failed-check and stale-destination cases work at 1440×900 and 390×844 in light and dark; exact acceptance and undo remain correct. |
| C2 — Complete human coworking | Shared components, attributed threaded discussion, review requests, attention, safe concurrent contributions | Two distinct people can discuss, edit where allowed, request review and resolve feedback; neither must wait to send an ordinary contribution until an Agent finishes. Guest and account identity claims are accurate. |
| C3 — Hand work to the conversation's Agent | Contextual task request from input, selection and comment; scoped participant execution grants; visible return and review | Owner and permitted invitee can start bounded work, answer a clarification, follow its real state and review its returned version. Message-only guests stay message-only. |
| C4 — Bring one's own Agent and enable peer work | Person-bound Agent identity and grants; same-instance handover first, then authenticated cross-instance exchange; bounded peer requests | Two people use their own Agents on shared work without sharing private histories or ambient permissions; a second headless instance can return attributable work and recover from interrupted delivery. Report the same-instance and remote gates separately. |
| C5 — Complete the whole experience | Joint usability pass, accessibility, disconnected/restarted operation, notifications and cross-surface audit | All journeys below pass. Remove replaced UI paths and reconcile source docs/statuses in their implementation changes. |

Headless operation, accessibility and authorization are requirements of
every stage. C5 validates their integration; it is not where they first
become supported. Do not call this plan complete after C1's visual polish.

## 13. Acceptance journeys

1. **Human–human:** Asha comments on paragraph 4; Nisheeth replies and
   requests a revision. Both see the same thread, version and disposition.
2. **Human–Agent from input:** “Mira, take over this draft” targets the
   intended version. An ambiguous target asks for a choice. A casual mention
   and a quoted instruction never start work.
3. **Human–Agent from Canvas:** select cells, ask for a figures check, see
   the exact range attached, then inspect the findings at that range.
4. **Agent–human:** an Agent asks Asha for a missing figure; she returns
   later, sees why she is needed, answers, and the correct task resumes.
5. **Own Agent:** Asha selects Atlas under an explicit sharing grant;
   Atlas returns work with its identity and Asha's sponsorship. Unrelated
   files and private conversations remain inaccessible.
6. **Agent–Agent:** Mira requests Atlas's review; Atlas can clarify or
   disagree, evidence returns to the same work item, and neither gains
   human approval power or starts an unbounded delegation loop.
7. **Review and concurrency:** people comment while an Agent works; a
   new version arrives without replacing the one being inspected. Old
   comments and decisions retain their version; overlapping edits conflict.
8. **Screenshot case:** an unchanged document has a useful preview and
   no misleading selected-change acceptance. A changed destination causes
   a fresh comparison or conflict, not a false “already matches” message.
9. **Headless and remote:** close all clients, reopen on a phone, recover
   work and review requests; separately restart a server during a remote
   handover and prove no duplicate execution or false completion.
10. **Revocation and failure:** revoke a participant or Agent mid-work;
    future access and dispatch stop, partial work remains attributable and
    status is truthful. Explain that previously disclosed content cannot
    be recalled from another authorized recipient.
11. **Accessible operation:** complete handover and review by keyboard,
    verify focus return and screen-reader labels, and preserve unsent input
    through recoverable connection failures. No meaning relies on color.

Each implementation stage records automated evidence for authority and
state transitions, plus real browser/multi-client evidence for its visible
journeys. Current observations are source and supplied-screenshot findings;
this document makes no claim of a newly tested or shipped experience.
