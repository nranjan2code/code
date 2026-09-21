# 69 — Shared conversation and draft coworking

Status: **in progress**. Scoped invitations, shared reading, attributed conversation messages, observed participant presence, saved-draft comments, live refresh and isolated Agent revision are shipped. Participant-specific approval delegation remains open.

## Product contract

An owner may invite a named human to one Agent conversation and its selected draft results. Participants can read that conversation, speak to the Agent, and leave comments anchored to a result version, file, or line. The owner can separately grant a participant authority to approve a specific action. An invitation alone grants no workspace write, external effect, secret, or Agent policy change.

The same Agent and conversation stay visible to everyone admitted to that audience. A result retains one stable identity as the Agent revises it; each candidate version remains addressable for comments and review. A comment on an older version remains attached to that version rather than silently moving to new lines.

## Identity and admission

- Browser coworking needs distinct authenticated principals. The current single operator cookie cannot distinguish invited humans and must not be shared as an invitation.
- Invitations are scoped to conversation and audience, expire, can be revoked, and are stored as hashed bearer capabilities or verified account grants. A participant's display name is never the authority identity.
- Every transcript, projection, SSE, artifact, candidate, preview, and comment read checks the same audience grant. Revocation terminates live streams and takes effect on the next request, including cached reads.
- Inbound turns record the authenticated sender separately from user-authored text. The model-visible attribution is logged in the session; the UI does not infer sender from a text prefix.
- A participant's request uses that participant's authority ceiling and cannot inherit the owner's desktop FullAccess, secrets, or approval power. The Agent's own ceiling still applies.

## Shared work and decisions

- Comments are append-only records with author, conversation, object ID, candidate digest, optional file/line anchor, body, timestamp, and supersession link. Edits append a revision; deletion appends a tombstone.
- Agent feedback is an explicit intervention derived from a comment or a new message. Comments do not silently become model instructions. The conversion is logged, attributed, and permission checked.
- Candidate review is bound to the server-held candidate ID and exact selected paths. Review, candidate acceptance, target setup, and publish/deploy are separate decisions with separate receipts.
- Only an authorized approver can accept a candidate. The accept action records actor identity and candidate digest, rechecks destination state, and produces a promotion receipt. A conversation message or previewed document cannot resolve an approval.
- The conversation displays the current result, review state, outstanding comments, and any blocking decision. Technical execution remains available from that result without becoming the default layout.

## Implementation boundaries

1. Add a principal and scoped grant layer to browser authentication; retain the existing operator login as the owner principal. Do not pass an invitation token through the general `/app` API surface.
2. Bind Agent conversation read, run, event, and artifact routes to one audience authorization function. Reuse `ConversationKey` and Agent ownership from `64-agent-owned-platform.md`.
3. Add attributed message/comment ledger entries and projections. Preserve append-only replay and exact model-visible reconstruction.
4. Bind immutable candidate records, previews, checks, and comments to stable result and candidate IDs. Reuse the presentation timeline rather than a parallel coworking state store.
5. Add invitation, participant, comment, review, and revocation controls to desktop/web. Channel participants use their authenticated sender identity and the same audience grants.

## Acceptance

Two distinct humans join the same conversation and draft. Both see the same ordered result versions and attributed comments. Either can request an Agent revision within their authority. A comment on version one remains anchored to version one after version two arrives. A participant without accept authority cannot apply files or approve a tool call. Revocation blocks transcript, SSE, preview, candidate, and artifact reads immediately. The owner can review and accept selected files; a concurrent workspace edit produces a conflict instead of an overwrite. All turns, interventions, approvals, and receipts remain reconstructable from the ledger.
