# 80 — Mail and calendar: governed account work

Status: **in progress on `codex/mail-calendar`: Agent/account linking, owner-only
bounded Google/Microsoft previews with selected Gmail labels and Microsoft
top-level folders (Apple remains Inbox-only), explicit phrase search across all
three providers, paginated Google/Microsoft conversation previews, brokered
Agent reads of selected Google/Microsoft folders and threads with per-message source
citations, and an Agent-vault working area for local drafts are
implemented. The work area supports saved candidates, exact-payload preview
and Review, email send, Google standalone-event update, and limited event create
for Google/Microsoft. Review requires opt-in provider scopes, owner confirmation,
permission-engine evaluation, and a durable single-use claim. Event creation has
no attendees, recurrence, or reminders and saves reviewed instants as UTC.
Google also supports conditional cancellation of one unchanged public,
standalone, timed event with no attendees when the connected owner is its
organizer. It uses the event ETag precondition; the UI requires exact event
review and a separate owner confirmation.
Scheduled and one-minute continuous read-only routines use `TaskDef` and a
bounded encrypted Agent-vault mail backlog with provider cursors. Starting a
local draft from a selected conversation message preserves its source reference
and can create a provider-threaded reply on Google or Microsoft. The full conversation workspace,
Apple conversation grouping/effects, event update/cancellation beyond the
Google standalone profiles, RSVP, complete
provider reconciliation, live provider checks, and the 24-hour service recovery
acceptance remain open, 2026-09-30.**
Mail previews and conversation pages also carry bounded sender, To, and Cc
fields from Google, Microsoft Graph, and Apple IMAP. Bcc is not requested from
Microsoft and is never projected into the owner or Agent result.
iCloud links now verify fixed-host IMAP MailRead-only, CalDAV CalendarFreeBusy-only,
or CalDAV CalendarRead-only access as separate account selections. Apple inbox previews expose bounded
metadata, and the person or Agent can request one selected message body through
a fixed-UID, read-only fetch parsed in the isolated worker. Calendar previews perform fixed-origin CalDAV discovery and
range-bounded event reads; provider XML and iCalendar content are parsed in the
network-denied worker. CalDAV results are filtered locally against the
requested time range after worker parsing. A bounded worker-side MIME parser
selects bounded plain-text parts and skips HTML and attachments. Apple free/busy
uses CalDAV `free-busy-query`; the worker returns only busy intervals without
event details. Apple event changes and mixed capability selections remain
unavailable or unverified. No credentialed live Apple Calendar request has
been made, so authenticated provider discovery and free/busy semantics still
need live verification.**
On 2026-09-30 the owner authorized mail/calendar implementation against the
current 4.x storage model, deferring the data-architecture refactor. The owner
authorized a feature branch after the design review. Typed contracts,
Agent-scoped credential storage, bounded Google/Microsoft PKCE linking, an
connection Settings panel with masked display identities, local-only Apple
app-specific-password enrollment, and a broker-owned read tool for the local
owner surface are implemented in that
branch. OAuth and all provider App Password activations hold the shared connection-ledger lock
across the Agent-vault credential write and pending-to-connected event, so a
concurrent disconnect either prevents the new credential write or runs after
activation and removes it. Apple enrollment records only read capabilities
and stores the password in the Agent vault. A Mail-only selection verifies
the app-specific password against `imap.mail.me.com:993` using TLS, read-only
`EXAMINE`, and a fixed session byte budget. Such an account is admitted only
for `MailRead`; inbox metadata is bounded to 20 items and an explicitly selected
message can be fetched separately without setting `Seen`. Raw MIME is passed
to the isolated worker and only bounded plain text is returned. A separate
`CalendarRead`-only selection is verified through
the fixed-host CalDAV authentication probe. The adapter discovers the
principal, home set and calendar collections, validates each href against
`https://caldav.icloud.com`, and sends bounded event reports to the isolated
worker. Mixed Apple capability combinations remain unverified.
Google also supports a separate local-only App Password path. It verifies
`imap.gmail.com:993` with TLS and read-only `EXAMINE`, stores the credential in
the same Agent vault, and grants only `MailRead`. Gmail inbox metadata and
selected MIME bodies use the bounded IMAP adapter and isolated worker; routine
watches use a provider-specific bounded UID cursor. Conversation reads,
Calendar, sending, and all provider writes are unavailable for this sign-in
method. The UI warns that the long-lived App Password is less secure than
OAuth and identifies provider-side revocation. Microsoft Exchange Online has
no app-password alternative; its connection remains OAuth-only.
Microsoft personal Outlook.com/Live/Hotmail/MSN accounts also have an optional
local-only app-password path, separately from OAuth. It accepts only those
consumer address domains, verifies a fixed-host IMAP sign-in to
`outlook.office365.com:993` over TLS with read-only `EXAMINE`, and grants only
MailRead. Inbox metadata, selected message bodies, and bounded scheduled
message-ID watches use the same fixed-host IMAP adapter and isolated MIME
worker as the other password-based read paths. It offers no calendar, send,
conversation, attachment, or provider-write access. Microsoft documents app
passwords for personal accounts and legacy clients while its current Outlook.com
IMAP setup requires OAuth2; Basic Authentication retirement and current
provider enforcement mean this fallback may stop working or be rejected. The
connect action verifies the credential before storing it. Microsoft 365 and
work/school Exchange accounts stay OAuth-only. The app password is never the
ordinary Microsoft password. It is local-device only, Agent-vault stored,
zeroized on disconnect, and warned as less secure before entry.
Google and Microsoft Agent reads are limited
to the local owner surface; channel audiences fail closed without an explicit
share grant. Provider previews redact private-event titles, locations,
descriptions, and attendee counts while preserving only the busy time. Reads
supplied to the model are retained in current append-only session history.
Local drafts and scheduled read-only routines are implemented.
The first plain-text email effect and one timed event-create profile are
implemented for Google and Microsoft. Google also supports a constrained
conditional update of one standalone timed event without attendees. These
effects require an unchanged saved
candidate revision and owner-only confirmation. Unknown outcomes remain
non-retryable and appear in the local candidate list. Google event update
re-reads the source event and sends a conditional ETag update; stale versions
conflict and require a fresh preview and candidate. Microsoft update,
cancellation outside the single-event Google profile above, RSVP operations,
Agent-initiated effects, and provider reconciliation are not implemented.
Google and Microsoft calendar-write consent is broader than this limited
create operation; the credential stays in the Agent vault and effects remain
broker-only. OAuth authorization attempts are bounded and single-use;
disconnect serializes with new links and atomically advances a durable,
Agent/provider OAuth fence in the append-only connection ledger. Each OAuth
attempt captures that fence at initiation; callback credential persistence
checks it under the same cross-process ledger lock as the vault write. A
disconnect in another server process therefore invalidates a callback already
in flight. The separate in-memory callback fence table is capped so eviction
invalidates stale commits instead of authorizing them.
The account API and Settings surface the unverified state directly. Apple
accounts are admitted only when verified for an exact supported selection:
MailRead alone or CalendarRead alone. Mixed capability selections remain
`connected_unverified`, and the read adapter refuses them. Mail verification
does not change calendar admission.
The repository-local native package is `packages/mail-calendar`. Its only
component is an inert skill that explains account setup and current limits; it
declares no executable, MCP, command, hook, or data-access capability. The
package registry inspects it through the same native manifest path used for
other local packages; installation leaves it disabled until a separate review
and enable action. The package itself grants no data access; mail/calendar
reads are available only through the Core broker tool. Provider effects do not
run through the package skill or a model tool; the send endpoint is an
owner-authenticated broker operation.
The owner account inventory reports whether each credential is actually
available in the Agent vault; a connection ledger row alone is not presented
as proof that saved sign-in material can be loaded. Owner-only bounded
previews for Gmail and Microsoft are implemented. The Agent read tool uses
the connected account's declared capability and the owning Agent's local
surface grant. Calendar and availability previews accept an owner-selected
local date range of up to 30 days and show when the result was refreshed;
times are rendered in the device's time zone. Inbox navigation remains a
bounded recent-message view and owner-submitted phrase search within the selected
inbox. Owner mail previews can select Gmail labels and Microsoft top-level
folders; Apple and Agent/watcher reads remain Inbox-only. Google and Microsoft
owners can open a paginated thread preview with up to 20 messages per page; Gmail
loads a bounded metadata-only thread snapshot and fetches full content only for
the selected page, while Microsoft follows a validated provider continuation.
Apple supports a selected-message preview without conversation grouping. These
results are transient and search phrases are not logged or retained. The owner
can start a local email draft from any message in an opened Google or Microsoft
conversation; the candidate retains that selected provider message as its
source reference. The owner can create a provider-threaded reply. The send
broker requires both MailRead and MailSend, re-fetches the selected provider
message immediately before dispatch, and blocks a changed message,
thread/conversation ID, or subject. Google serialization uses validated
provider Message-ID and References headers with the selected Gmail thread ID;
Microsoft uses the selected message's `/reply` operation. IDs remain fixed-host
path segments. Apple replies remain unsupported. Subject text alone never
establishes thread membership. The Agent tool can now read a bounded selected
Google/Microsoft thread page with a routine-scoped `mail_thread` permission.
Each message is separately labelled as untrusted and carries a citation bound
to provider, account, audience, thread, and message IDs. Click-through citation
navigation now accepts the exact inline `mailcite:` token from that citation
and opens the connected account's owner-only conversation preview; the server
re-fetches the thread from the provider, and the UI scrolls to the cited
message when it is in the loaded page. Full thread workspace and cross-folder
Agent access remain open. Local drafts,
scheduled read-only routines, and an explicitly
best-effort scheduled email watch are implemented. It scans up to 100 recent
provider IDs into a bounded encrypted Agent-vault backlog, then fetches at
most the routine's configured batch by explicit IDs. Apple carries a
UIDVALIDITY/UID cursor through that backlog, Gmail follows bounded history
pages from its stored history ID, and Microsoft follows bounded Graph delta
links for the inbox. IDs fetched by a tool are committed only after the
scheduler observes a completed run; failed or interrupted runs requeue them.
This gives at-least-once recovery across local restarts. Invalid or expired
cursors fail visibly and require the owner to recreate the routine; backlog
overflow also fails closed. A per-routine OS lease prevents duplicate
local server-process runs through
child completion; it does not provide multi-host coordination. Email send, a
constrained timed event create, Google standalone event update, and one Google
standalone event cancellation profile have effect-aware owner confirmation
paths; other event update/cancellation profiles, RSVP, standing grants,
complete receipt reconciliation, durable continuous service recovery, and
full provider conformance remain in progress. The
account-deletion limitation below is disclosed before content features are
enabled.
Google and Microsoft inbox previews now show bounded attachment metadata and
allow a person to preview one selected PDF, Open XML document, or plain-text
file up to 1 MiB. The provider response is revalidated against the parent
message; bytes are read only by the existing network-denied document worker and
only capped extracted text returns to the owner UI. HTML, archives, inline
attachments, unsupported formats, oversized files, and Apple iCloud attachments
remain unavailable. Attachments are not copied into Agent history unless a
person separately asks the Agent to read or use that content.
No crypto-shred guarantee is made. Apple Mail is available only for a verified
Mail-only account; inbox listing returns bounded metadata and a separately
selected message can return bounded plain text. Apple Calendar event previews
and availability checks require separately verified CalendarRead-only and
CalendarFreeBusy-only accounts, respectively. Availability uses a CalDAV
`free-busy-query` and worker-side VFREEBUSY projection that exposes only busy
intervals. Provider effects remain unavailable. No live credentialed Apple
Calendar verification has been performed.
Provider-specific API details and consent requirements must be rechecked
against current provider documentation before each implementation milestone.

**Review, 2026-09-29:** expanded the initial proposal with protocol semantics,
automation safety, connection security, delivery states, and explicit release
gates. This records the design coverage; it is not evidence that the proposed
security boundaries have been implemented or tested.

## Purpose and boundary

Mail and calendar should let a person ask Vakyartha to understand a selected
thread or time range, prepare a reply or meeting change, inspect the exact
effect, and commit it through the account they chose. They are also inputs to
the Agent's daily work: routines can notice a relevant message or upcoming
event, combine it with authorized files, commitments, and other activities,
then prepare a useful result. The useful loop is:

> connect an account → read a bounded selection → cite source items → prepare
> a proposed action → review its effects → commit once → show the receipt.

This resembles the Office file-in, draft, Review, file-out loop in
`72-openxml-documents.md` and `77-pdf-documents.md`. Its last step is different:
a sent message or invitation has external recipients and cannot be undone by
restoring local bytes. The review target is the **provider action**, including
its audience and account, not merely a local draft. A provider-side draft is
already a mutation and is not the safe initial draft surface.

This is an installable capability package on the existing plugin and broker
contracts (`39-plugin-ecosystem.md`, `24-agent-security.md`), not another agent
loop, broad network grant, channel transport, or Office suite. Mailbox access
does not make email an inbound chat channel. Adding email as a channel would
require separate admission, sender identity, allowlist, and reply rules from
`34-channel-onboarding.md` and `64-agent-owned-platform.md`.

## What it enables

| Person asks | Read | Proposed action |
|---|---|---|
| “What did Priya decide in this thread?” | Selected thread, with message citations | None |
| “Draft a reply with the revised deck.” | Thread and explicitly selected file | Local draft with recipients and attachment digest |
| “Find a time next week.” | Bounded free/busy first; event details only if needed | Suggested slots, then a proposed event |
| “Move Tuesday's review to Thursday.” | One selected event and conflicts | Exact event diff and notification impact |
| “Prepare follow-ups after this meeting.” | Selected event and authorized notes/thread | Separate drafts for each recipient |

The first release should cover bounded reads, free/busy, local drafts, reviewed
send, and reviewed event create/update/cancel, including scheduled routines
that use those reads and prepare drafts. Background whole-mailbox ingestion,
unbounded autonomous outreach, bulk mail, rules/filters, mailbox deletion,
and organization administration are outside the first release.

## Daily-life and cross-activity work

The package is useful when it drives an Agent's existing commitments and
automations, not only when a person opens a mailbox screen. Examples:

| Routine or trigger | Bounded input | Safe output |
|---|---|---|
| Morning briefing | Today's calendar, selected high-priority mail, due commitments | Private briefing with cited sources and suggested next actions |
| Prepare for a meeting | One upcoming event, its selected thread, authorized workspace files | Agenda and questions; optional reply or file draft for Review |
| After a meeting | One ended event, authorized notes, related thread | Follow-up drafts and commitments, with each recipient reviewed |
| Travel day | Selected itinerary mail and calendar events | Timing/conflict alert and a proposed calendar change |
| Time-sensitive request | A message matching a user-defined sender or label rule | Inbox alert, proposed reply, or a task; no automatic send by default |
| Weekly planning | Calendar availability and open commitments | Proposed focus blocks and reschedules, each with attendee impact |
| Bills and renewals | Selected bill or renewal notices and relevant commitments | Due-date reminder or proposed task; payment remains a separately authorized capability |
| Family and appointments | Authorized shared calendar and selected confirmation messages | Preparation reminder or proposed schedule change without disclosing private event details |

An automation definition names its trigger, connected account, selection
rule, Agent, audience, cadence, allowed reads, possible outputs, expiry, and
notification destination. It uses `TaskDef` for scheduled work and the
commitment kernel for obligations; it is not a second automation engine.
Event-driven triggers, when later supported, enter the same admission path as
a scheduled run and are deduplicated by provider event ID and account. A
provider notification is a hint to fetch and verify current state, never an
instruction or authorization. A missed trigger is recorded and can be
reconciled; it does not silently become permission to replay an old send.

The Agent may combine mail/calendar facts with files, tasks, memory, and other
connected services only after each source's own audience and capability check.
It logs exactly what reaches the model. “Related” is a retrieval hint, not a
grant to read an entire mailbox or move private content into shared memory.
Automation results land in the owning Agent's conversation or inbox with
source citations, actions taken, actions awaiting Review, and failures. A
routine that cannot get a credential, approval, or source does not report
success; it leaves an actionable failure entry. Quiet hours and delivery
cadence control notifications, not the underlying permission decision.

An event ending does not prove that a meeting occurred; follow-ups need notes,
an explicit completion signal, or a clearly labelled assumption. Similarly,
a receipt or shipping message can inform a task, but cannot authorize a
purchase, payment, booking, or disclosure through another integration.

## Decisions

### Operating modes — continuous, scheduled, and on demand

Continuous operation is a first-class requirement. A routine can be **on
demand**, **scheduled** (a time, interval, or offset before/after an event), or
**watching** a bounded source continuously. Watching uses provider change
notifications where supported, otherwise bounded polling with an explicit
freshness target. Initial implementations may use polling; push support does
not justify delaying all continuous routines. Continuous means the service
remains available to detect work, not that an LLM runs without interruption.
Cheap deterministic selection and deduplication happen before a model call.

Run these through the existing durable gateway/task runtime and service
manager (`28-operations.md`, `31-network-resilience.md`, and `docs/hosting.md`).
Closing the client or Canvas does not stop a routine. True 24/7 availability
requires an awake, connected service host; a sleeping or offline laptop cannot
execute locally. The UI identifies the execution host and last successful
check, and distinguishes **watching**, **scheduled**, **running**, **paused**,
**waiting for review**, **reconnecting**, and **needs attention**. Neither a
green service PID nor an open client alone proves a source is current.

Use durable cursors, deduplication, bounded queues, and one active execution
lease per routine. Overlapping ticks coalesce pending work; competing service
instances cannot commit the same action. Multi-host operation requires a
verified shared lease/fencing mechanism and is unavailable without it. Respect
provider retry delays, apply cancel-aware backoff with jitter, and keep
uncommitted read recovery separate from ambiguous effect reconciliation.
Refresh credentials and renew subscriptions without human action while the
grant is valid. Revocation, invalid credentials, quota exhaustion, or changed
consent parks work with a reason; it never expands access.

Declare catch-up and freshness rules per routine: a missed morning briefing
can expire, while a missed bill notice can still create a relevant reminder.
Calendar changes recalculate meeting-relative triggers and cancel obsolete
ones. Recovery preserves the logical trigger identity across retries and
records gaps. Suspension or revocation stops new admissions and cancels work
that has not crossed dispatch; dispatched effects remain visible for
reconciliation. Outage alerts are deduplicated, and recovery clears the
attention state. Owner-visible pause-all and per-connection/per-routine pause
controls remain available from an authenticated surface.

### D1 — One contract, separate provider adapters

The user-facing mail and calendar operations are provider-neutral typed
operations. Each account connection names an immutable provider, a vault
reference for its external principal, granted scope set, credential reference,
and owner/audience scope.
Google Workspace, Microsoft Graph, and Apple iCloud have separate adapters;
adding one must not change the policy or Review contract. Google and Microsoft
use delegated OAuth. Apple now documents Apple Account authorization for
supported third-party apps, and app-specific passwords when an app does not
support that flow ([Apple's iCloud third-party app guide](https://support.apple.com/en-us/121539)).
The current Vakyartha branch implements the local app-specific-password
fallback for a verified, Mail-only IMAP read path. The credential has broader
protocol access than Vak's selected capabilities and must be disclosed as
such. Apple's support guide describes the user authorization and revocation
experience.
Apple's manual iCloud Mail configuration documents IMAP at
`imap.mail.me.com:993` and an app-specific password
([server settings](https://support.apple.com/en-us/102525)). In Vakyartha,
the Mail-only connection path authenticates and runs `EXAMINE INBOX` before
storing the credential, then the preview reads at most 20 message envelopes
and body structures with a 512 KiB protocol-session limit. It does not fetch
message bodies during the inbox listing. A separate selected-message request
uses `UID FETCH BODY.PEEK[]`, checks UIDVALIDITY, caps the full message at
128 KiB within the 512 KiB IMAP session budget, and sends raw MIME directly to
the network-denied worker. It returns only bounded `text/plain`; HTML and
attachments are not exposed, and HTML-only messages are labelled as lacking a
plain-text body. Calendar event access and availability each require a separate
capability selection and a successful fixed-host CalDAV authentication probe.
Both paths perform bounded discovery, validate every provider href against
the fixed Apple origin, and parse provider responses inside the network-denied
worker. The availability path uses CalDAV `free-busy-query` and returns only
busy intervals. Mixed capability selections remain `connected_unverified`;
Apple effects are unavailable.
Apple's developer OAuth service is Account & Organizational Data Sharing; its
documented scopes are for the Apple School Manager Roster API
(`edu.users.read`, `edu.classes.read`), not iCloud Mail or Calendar
([authorization scopes](https://developer.apple.com/documentation/accountorganizationaldatasharing/request-an-authorization),
[Roster API](https://developer.apple.com/documentation/rosterapi/)). EventKit
is a native on-device calendar permission model, not a server-side account
grant for unattended routines ([EventKit access](https://developer.apple.com/documentation/eventkit/accessing-the-event-store)).
**Do not treat Sign in with Apple as this permission:** it authenticates a
person to Vakyartha, rather than granting access to their iCloud Mail or
Calendar ([Sign in with Apple overview](https://developer.apple.com/documentation/signinwithapple/authenticating-users-with-sign-in-with-apple)).
The current fixed-host IMAP, CalDAV event-read, and free/busy paths are
implemented, but credentialed live Apple Calendar conformance, provider
revocation behavior, and all Apple write operations remain unverified or
unsupported. No generic arbitrary-URL, raw-HTTP, or model-selected MCP call is
an escape hatch to the account. Custom IMAP/CalDAV hosts are out of initial
scope.

The package declares capabilities such as `mail.read`, `mail.prepare`,
`mail.send`, `calendar.freebusy`, `calendar.read`, and `calendar.write`. These
are independently visible and authorized. The provider's OAuth scope is an
outer ceiling, never evidence that a model call is approved. Where a provider
cannot express Vak's narrower per-call authority, Vak enforces it at the
broker and refuses an adapter that cannot identify the account and target.
The OAuth connection and the plugin installation are separate reviews; a
plugin update that changes either capability or outbound destination requires
the existing digest-bound plugin review.

### D2 — Explicit account, audience, and source scope

Every read and action names one connection. There is no “default mailbox”
fallback when multiple accounts exist. A connection is owned by a human or
authorized space, then narrowed to the Agent and audience allowed to use it;
linking an account does not publish its content to every Agent or channel.
Child runs inherit at most the parent call's scoped authority. Account
unlinking revokes active leases before the UI reports disconnection.
An Agent may link multiple distinct accounts for one provider, but may have
only one active link for the same provider principal. A duplicate link cannot
silently accumulate a second capability selection; disconnect the existing
link before reconnecting that identity with a different selection. While a
link is pending, another link for that provider and Agent is rejected until it
finishes or is cleaned up. Principal comparison stays inside the Agent vault;
the connection ledger contains no identity fingerprint or mailbox address.
A link marked reauthentication-required is not active: its access is fenced,
and the owner may connect again and then remove the old entry.

Reads select message IDs, threads, folders, calendars, or a bounded time
window. Search is a deliberate, scoped operation with a result cap. Free/busy
is the default for scheduling; event titles, locations, attendees, and notes
are fetched only when the task requires them. A calendar event marked private
never becomes full detail merely because a free/busy query can see its slot.

### D3 — Local proposal, then provider commit

`mail.prepare` and `calendar.prepare` produce local, immutable candidate
versions. They make no provider write. The candidate records the account,
operation, normalized recipients/attendees, subject/title, body/description,
attachments or conference settings, event time and time zone, source IDs and
versions, and a content digest. Review displays both the visible diff and
effects beyond the edited text: reply-all expansion, external domains,
attachment disclosure, attendee invitations, notification emails, recurring
series versus one occurrence, and cancellation consequences.

The person can accept, edit, or discard a proposal. For a normal interactive
send or calendar write, commit requires a fresh human approval bound to the
exact candidate digest, connection, target, recipient set, and operation. An
approval for one candidate cannot cover a changed body or a different
account. The model cannot approve its own proposal. FullAccess does not
silently waive this product-level external-effect gate; that gap in the
general commitment kernel (`47-commitment-kernel.md`) must be closed for these
operations before ship.

A user may separately create a narrow, revocable standing grant for unattended
work. It must constrain account, Agent, operation, audience/recipient or
calendar, time window, rate, and expiry; it cannot grant bulk sending or
arbitrary recipients through a vague natural-language phrase. Anything
outside it waits for a person. A scheduled task uses the existing `TaskDef`
and the same broker decision; no second scheduler is created.

For daily routines, read and private summarization can run under a scoped
connection and task authorization. A proposed external action can be parked
for Review without blocking the rest of the routine. A standing grant may
cover a repetitive, tightly bounded effect, such as creating a private focus
block on one owned calendar, but a new recipient, changed attachment, or
expanded attendee list escapes the grant and requires review. Grant usage is
visible and revocable from the same surface as the automation.

### D4 — One exact commit and an honest outcome

Before dispatch, the broker rechecks the connection and permission generation,
candidate digest, source version/ETag where available, current recipient and
calendar target, attachment bytes, and the approval or standing grant. A
changed source or event yields a conflict card and a new review, not a blind
overwrite. The connector sends only the reviewed payload to a pinned provider
host under an operation-scoped network rule; redirects to another host fail
closed.

Each logical action has a durable idempotency key, preserved across retries,
and each attempt has a distinct dispatch record written before network I/O.
Provider-native idempotency or conditional writes are used where
available. A timeout after a send is **unknown**, not failed: reconcile with
provider IDs or a safe lookup before retrying. If the provider cannot prove
whether a message was sent, stop and ask the person to inspect it; never
automatically send a possible duplicate. Calendar retries likewise reconcile
the event and attendee notification state. The receipt records the provider
item ID, account, effective audience, exact approved candidate, and outcome.

Concurrent commits of the same candidate are serialized through a durable
claim; restart recovery reconciles an unfinished claim before redispatch.
Approval is single-use, expiring, and invalidated by an edit or permission
change. A last-second read without a conditional provider write cannot prove
absence of a race: adapters must declare this limitation, and operations
requiring conflict-safe replacement stay unavailable without that primitive.

Track `prepared`, `awaiting_approval`, `dispatching`, `provider_accepted`,
`confirmed`, `failed`, `unknown`, `cancelled`, and `expired` separately.
Provider acceptance does not prove recipient delivery or reading. A bounce
can arrive after acceptance; absence of a bounce proves neither. For example,
[Graph sendMail](https://learn.microsoft.com/en-us/graph/api/user-sendmail?view=graph-rest-1.0)
returns acceptance before processing is complete. Never promise universal
exactly-once delivery. Cancellation prevents undispatched work; after dispatch
it requires reconciliation and cannot recall an email. A compensating event
change or cancellation is a new authorized action, not an automatic rollback.

### D5 — External content is evidence, never authority

Message bodies, headers, invitations, event descriptions, embedded links,
attachments, and provider metadata are untrusted input. Fetching or parsing
them cannot run instructions, change tools, approve a send, alter a grant, or
select a new destination. The model receives bounded, attributed excerpts
with source IDs and explicit “external content” labels. Every model-visible
excerpt is logged in the session so `derive_messages()` can reconstruct it
(invariant 1); a citation opens the authorized original item and is bound to
its account, item ID, version, and audience.

HTML mail is sanitized to structured text in a bounded, network-denied worker.
Remote images, tracking pixels, remote CSS, scripts, and links are never
fetched while reading. Attachments go through the existing inbox and
document-worker path, with type and size limits; Office/PDF parsers stay out
of the server and connector process (invariant 14). “Forward this” and
“attach this” are disclosure actions, even when the content was already
read. Source sensitivity labels can only narrow recipients and destinations.

### D6 — Credentials and data lifecycle

Use delegated user authorization and incremental scopes where the provider
supports them. Prefer separate read and write connections/scopes; never ask
for full mailbox control merely to schedule or send. Refresh tokens and client
secrets live only behind `vak_config::credentials` with recipient-scoped
injection; no model tool, subprocess environment, session JSONL, log, or
browser response receives them. Account-wide application permissions and
domain-wide delegation are excluded from the first release. Revocation and
expiry fail closed and produce an actionable reconnect state.

Fetched content is not copied into memory, search, RAG, or a feed by default.
Owner previews return bounded content directly to the UI and do not store a
second content copy. The broker-owned Agent read tool returns bounded content
to the model; invariant 1 records its result in append-only session history.
Current storage has no independent retention control or selective erasure for
those transcript copies.
Local email and event drafts are now stored as bounded, revisioned candidate
records in the owning Agent's encrypted credential vault, with compare-and-swap
updates. Disconnect removes that account's unsent local candidates alongside
its saved credentials. This cleanup does not erase prior copies in append-only
session history, nor does it implement general-purpose retention or crypto-shred.
Any future background mailbox source must use the single intake
and catalog lifecycle proposed in `76-intake-and-knowledge.md`, after its
data-architecture dependencies land; this document does not start M1 or a
later data milestone. Delete, export, legal hold, and erasure must follow
`73-data-architecture-and-lifecycle.md` and
`74-lifecycle-and-data-administration.md` when implemented. Provider deletion
or revocation cannot promise deletion of copies held by external recipients.

### D12 — Agent boundary, account deletion, and current-storage limit

Every connection belongs to exactly one Agent. Its account identifier,
principal/display data, refresh token or app-specific password, and any
provider-specific recovery material are stored through the vault/credential
service; durable Agent metadata contains opaque references only. The vault
entry is recipient-scoped to the owning Agent and connector operation. A
different Agent, workspace, channel, or audience cannot resolve that secret
by guessing an account id or reusing a reference. Every read, candidate,
preview, attachment, citation, schedule, watcher cursor, and derived record
must carry the owning Agent, account lineage, and allowed audience, and each
read rechecks them at the broker boundary.

Disconnect revokes provider authorization where supported, deletes the vault
secrets, stops and fences schedules/watchers, cancels undispatched work, and
leaves only a content-free revocation tombstone needed to reject stale work.
No second durable mailbox/event cache is added. Content that reaches a model is
recorded in the owning Agent's append-only session history to preserve model
reconstruction. Under current 4.x storage, disconnect does not remove that
history, and deleting an account does not erase copies already in sessions.

The UI and tool description state this before enabling connected-content features and must
distinguish account disconnection (credential removal and future-read fencing)
from deletion of previously recorded content. Do not describe account or
Agent deletion as complete erasure of mail/calendar content. The future data
architecture M6/M7 work remains required for catalog-based lineage and
crypto-shred, but this feature branch does not start those milestones. External
recipient copies remain outside Vak's deletion control in every storage model.

Audit records contain IDs, scope, policy generation, decisions, payload
digest, provider outcome, and trace key, but no message body, subject,
recipient address, event title, token, or attachment bytes in telemetry.
Content needed for a person's Review and session reconstruction lives in
access-controlled content records, not general logs.

### D7 — Mail identity and content semantics

Review distinguishes From, Reply-To, To, Cc, and Bcc, showing the actual
addresses as well as display names. Only a provider-verified sending identity
or authorized alias may be used. A message's display name, Reply-To, or
authentication-looking header is not proof of a trusted sender; provider
authentication verdicts are evidence with provenance, not permission to act.
Reply and reply-all resolve recipients before approval and never infer a
hidden Bcc audience. Group/list expansion that Vak cannot inspect is stated
as unknown and cannot satisfy a standing grant restricted to known people.

Thread IDs and reply headers are scoped to the connection; a matching subject
does not establish a thread. MIME parsing preserves explicit text/HTML
alternatives, character-set errors, quoted history, and attachments. Outbound
headers are generated from validated fields, rejecting header injection.
Attachment names are untrusted; extracted paths cannot escape the owned
staging directory. Bound total decoded bytes, part count, nesting, and time;
encrypted, signed, or unsupported content is labelled unread/unverified,
never silently treated as an empty message. Reading does not mark a message
read or acknowledge a receipt. Mark-read, labels, archive, trash, and remote
draft changes are distinct write capabilities and deferred unless enabled
as a separately reviewed feature.

### D8 — Calendar semantics

Candidates carry an explicit named time zone and resolved instants, or an
all-day date range; ambiguous or nonexistent local times require resolution
before commit. Review shows the event zone and the user's zone when they
differ. Preserve daylight-saving behavior, all-day end-date semantics,
recurrence rules, exclusions, and occurrence identity. Bound recurrence
expansion by window and count; never expand an unbounded series in memory.
See [iCalendar, RFC 5545](https://www.rfc-editor.org/rfc/rfc5545.html) for the
interchange model; adapter-specific mappings require conformance fixtures.

Organizer edits, attendee RSVP, decline, removal from one's own calendar,
and cancellation for all attendees are distinct operations. Review identifies
one occurrence, the whole series, or a supported future-series edit, and all
notifications the provider can cause. RSVP is included as a reviewed action;
unsupported series edits are refused. Conference creation and resource/room
booking are explicit effects with their own capability checks. A free slot
is a snapshot, not a reservation: refresh availability before commit and
report conflicts without claiming a cross-calendar atomic booking.

### D9 — Secure account linking and disclosure to models

Account linking is an authenticated owner action. Use authorization code with
PKCE, a single-use state bound to the initiating session, exact registered
redirects, and issuer/account validation; no model-supplied authorization or
token endpoints. Refresh-token rotation is serialized, secrets are never
placed in URLs or logs, and reconnection cannot silently substitute another
principal. The web flow binds a callback to the authenticated initiating
session and uses a loopback redirect. The desktop installed-client flow uses
the RFC 8252 loopback redirect with PKCE and a high-entropy single-use state;
its bearer-authenticated owner initiation is tied to the callback by that
state because a Tauri webview cookie cannot cross into the system browser.
The callback remains loopback-only, is consumed once, and exchanges the code
only with the fixed provider token endpoint. Because the browser's
`SameSite=Strict` application session cookie is withheld on the provider's
cross-site top-level return, the callback also uses a separate random,
short-lived, `HttpOnly`, `/mail-calendar`-scoped `SameSite=Lax` cookie bound to
that pending state and initiating session. The package path lets multiple
pending account-link flows share the browser binding without sending it to
unrelated app routes. The callback also revalidates that the initiating
session is still active before redeeming the authorization code. Never weaken
the application session cookie to make OAuth work. A headless/web callback needs a separately
configured confidential-client or device-flow contract. Apply
[OAuth security BCP, RFC 9700](https://www.rfc-editor.org/rfc/rfc9700.html).

Before fetching model context, enforce which inference providers may receive
this account's content, including route-ladder retries and fallbacks. Account
access is not consent to send private mail to any model endpoint. If no
permitted inference route is available, show the reason; never widen the
disclosure policy to keep a routine running. Citations and exports recheck
source/audience authorization. A stale or removed source can be shown only as
an authorized retained observation, labelled with when it was read.

### D10 — Automation control and recovery

Each routine has pause, resume, run-once, last-result, pending-review, and
revoke controls. Put a finite per-run bound on total messages/events returned
across repeated and concurrent tool calls, as well as lookback, model cost,
actions per run, and daily actions. Quiet no-change runs stay
quiet; meaningful changes, failures, and required action reach the configured
audience. Configure time zone, daylight-saving schedule behavior, lateness
limit, and missed-run policy. On restart, do not burst-replay expired reminders
or externally effectful work.
Unattended admission also requires a usable refresh credential and a service
host that can renew it; if the provider did not issue a refresh token or later
revoked it, pause the routine and request owner reconnection. A valid access
token at setup time is not evidence of 24/7 readiness.

Carry trigger causality into each action. Suppress self-generated mail and
calendar update loops, repeated matches, bounce/auto-reply loops, and repeated
notifications with a durable deduplication record and cooldown. Future push
subscriptions must validate provider authentication or subscription secrets,
bind subscription and account, reject replay, and renew under bounded retry.
Duplicates and out-of-order events are expected; authenticated callbacks only
wake a scoped fetch. Expired cursors trigger a bounded resync with a visible
gap, not an unbounded full-mailbox download.

A workflow that sends mail, changes a calendar, and creates a task is not one
remote transaction. Each step retains its own authorization and receipt;
dependencies halt after a failed or unknown prerequisite, while independent
safe work may finish. Report partial success explicitly and never repeat an
already successful step to recover another. External messages can trigger a
rule the owner configured, but cannot create or broaden that rule.

### D11 — Storage prerequisites and package boundary

Declare every new durable record in `vak_core::state::REGISTRY`, resolve paths
through `vak_config::paths` and Core home accessors, and use full UUIDv7 IDs.
Connection grants are privileged desired state; observations, candidates,
approvals, dispatch attempts, receipts, cursors, and deduplication state have
explicit owners, retention, quotas, and recovery behavior. Candidates are
versioned and approvals/receipts are append-only. Trash filters apply across
UI, search, model retrieval, citations, exports, and automations.

Disconnect stops reads, writes, refresh, and subscriptions immediately; it
does not claim to erase retained transcripts. The connection screen states
what remains and provides the applicable retention/removal controls. Durable
content storage cannot ship under a promised encrypted/erasable contract
until the shared substrate and lifecycle it needs exist. The first
implementation stage must resolve that dependency explicitly; this proposal
does not authorize a parallel mailbox database or a new data milestone.

The package supplies domain capabilities, adapter declarations, skills, and
presentation through the existing registries. Generic connection admission
and external-effect enforcement belong to the shared broker. A credential
handle is redeemed only by a trusted credential/egress boundary that injects
authentication for the validated operation; untrusted content parsers receive
no credential and no network. Implementing that boundary is a release gate,
not a capability to assume already exists in today's MCP workers. FullAccess
with independently supplied credentials is outside this package's containment
claim; it is not a promise that Vak can police unrelated account clients.

## Surfaces and architecture

```text
shared client / CLI / channel request
            │
     Agent + audience admission
            │
  typed read or local proposal ──► broker permission + scoped connection
            │                                  │
   Review candidate + approval                 ▼
            │                         pinned provider adapter
            └──────── commit gate ────────────► provider
                          │                      │
                          └──── receipt ◄────────┘
```

The desktop and web client use one account picker, a source card, a local
draft card, Review, and a receipt. The account and recipient are visible on
every proposed action. A channel can ask for a draft, but an untrusted sender
cannot use the owner's account; account admission
is checked before a read; a commit requires a valid standing grant or a
reachable approver. Silence or missing approval means no new authority.
The UI derives action status from the connector's receipt, with provider
acceptance and confirmed outcome distinguished, never from the model's prose.

The connector is a host-owned, versioned external worker, with a scrubbed
environment, bounded input/output, deadlines, and a provider-specific egress
allowlist. It receives one validated operation and a short-lived credential
handle, never the policy engine, raw credential store, session store, or
general filesystem access. Capability filtering applies when tools are
advertised and again at call time, including plugin and channel overlays.
Every execution path, including CLI, automation, flows, and child runs, uses
the same broker and commit gate (invariants 14–16).

## Preview and working area

Mail and calendar need a complete reading and editing area in the shared
client (`crates/vak-client-ui`), accessible from conversations and the Agent's
work views. Extend the existing Canvas and Review contracts from
`66-immersive-artifact-canvas.md` and `72-openxml-documents.md`. A person opens
a source or draft explicitly, works beside the conversation or in a focused
view, and returns without losing selection or draft state. Background work
never steals focus by opening a preview. On a phone, use full-width stacked
views with the same capabilities. The visual system follows `DESIGN.md` and
`75-visual-refresh.md`.

| Area | Required behavior |
|---|---|
| Source navigation | Scoped account/folder/thread or calendar/date selection; bounded search; refresh time; loading, empty, partial, offline, and denied states |
| Mail reader | Thread with separate messages, actual sender/recipient fields, timestamps and zones, labelled quoted history, safe structured body, attachment cards, and source citations |
| Mail workspace | Editable To/Cc/Bcc, approved From alias, subject and body, reply context, selected attachment previews, draft versions, Ask Agent about a selection, and explicit Review and send |
| Calendar workspace | Agenda and day/week views, time-zone-aware availability, private busy blocks, editable event details and attendees, proposed slots, visible conflicts, and occurrence/series choice |
| Preview | Mail subject/body/attachments as they will be submitted; event summary and before/after time placement; audience, notification effects, and sensitive disclosures remain visible |
| Review | Deterministic changes from the candidate payload, source changes/conflicts, per-action choices, expiring approval, and explicit final Send/Create/Update/Cancel/RSVP wording |
| Automation workspace | Trigger and next run/check, host and freshness, scope and standing grant, preview/sample run, queued drafts, last result and history, budgets, pause/resume/run-once/revoke |
| Activity and receipts | Separate actions waiting for review, accepted by the provider, confirmed, failed, expired, and unknown; source/run links and safe recovery actions |

The current owner calendar preview implements agenda, one-day, and seven-day
read-only layouts over its selected date range. The day and week layouts show
local device time, all-day entries, and conflict markers; the agenda groups
entries by local date. An owner may explicitly compare up to five other
connected calendar accounts with CalendarRead for the same date range; partial
read failures are shown, compared events are read-only, and detected overlaps
identify conflicts across accounts. This remains an incremental preview, not
the complete calendar workspace: source selection, event attendee editing,
proposed slots, and event occurrence/series choices remain open. Cancellation
is limited to one standalone Google event with no attendees and does not cover
occurrences or series. A previewed free slot is not an atomic booking.

Owner mail previews can select among Gmail labels and up to 100 Microsoft
top-level mail folders, then search only inside the selected label or folder.
Apple iCloud currently exposes only Inbox. Google labels are labels and may
contain the same message in more than one label; the UI names this choice
"Folder or label" instead of implying identical provider semantics. Child
folder traversal, paging beyond the bounded folder list, and the full thread
workspace remain open. The Agent can read a selected folder/label after the
provider confirms it belongs to the account. Scheduled routines are pinned to
one owner-selected folder/label and recheck membership; continuous new-mail
watches remain Inbox-only because provider watch cursors are Inbox-scoped.

Preview is derived from the immutable normalized payload actually submitted
by the adapter, not from model-written explanatory prose. Render sanitized
structured content with remote resources blocked; do not mount mail HTML in
the application's trusted DOM or in a general executable HTML Canvas.
Attachment previews reuse the Office/PDF/image readers with their existing
worker and audience checks. Pixel-perfect rendering in every recipient's mail
client is not promised; unsupported content is named, and the preview cannot
silently omit a recipient, attachment, hidden part, or provider-added effect.
Provider transformations discovered after dispatch are reflected in the
receipt. Tracking images stay blocked during preview as during reading.

Every manual or Agent edit creates a new local candidate revision and
invalidates approval. Autosave saves the local draft only. A shared working
draft uses the existing collaboration identity and audience checks; concurrent
edits use a revision precondition and show conflicts instead of losing work.
Choosing some actions rebuilds and verifies the resulting candidate before
approval. Local draft saving, provider draft saving, and sending are visibly
different operations. Closing a draft does not send, discard, or pause the
routine that created it. A routine can leave a draft waiting while subsequent
independent checks continue within its limits.

Automation preview runs the configured selector on a bounded sample, shows
which source items would match and what actions would be prepared, and cannot
commit external effects. It still needs real read authorization and applies
the same model-disclosure rules. The person can adjust the rule, inspect the
new preview, then enable it. History links a trigger to its observation,
candidate, approval/grant, dispatch, and receipt; technical IDs stay behind
Show technical details, while account, audience, freshness, and safety states
remain visible. Provide keyboard editing, labelled controls, screen-reader
status updates, and non-colour indicators throughout.

## Provider feasibility notes

### Sign-in choices and password-based fallbacks

The connection screen must show the sign-in methods that actually work for
each provider, with separate capability labels and a plain-language warning
for every password-based fallback. OAuth is the preferred method because it
does not collect the provider account password and can request narrower
permissions. Never accept an ordinary account password as a fallback.

| Provider | Preferred method | Additional method | Boundary and warning |
| --- | --- | --- | --- |
| Google | Local OAuth authorization with PKCE | Google App Password over fixed-host Gmail IMAP | App Password is a long-lived account credential and is less secure than OAuth. The local-only enrollment path verifies it against `imap.gmail.com:993`, stores it only in the owning Agent's credential vault, supports bounded Inbox metadata and selected worker-parsed message reads, and grants MailRead only. No Calendar, send, or provider-write access. Never request the user's ordinary Google password. |
| Microsoft | Local delegated OAuth authorization with PKCE | Local app password for personal Outlook.com/Live/Hotmail/MSN accounts, IMAP MailRead only | Microsoft 365 and work/school Exchange remain OAuth-only. Microsoft documents app passwords for personal accounts and legacy clients, while Outlook.com's current IMAP setup requires OAuth2 and Microsoft is retiring Basic Authentication. This fallback may be rejected; it verifies the fixed-host IMAP connection before storing. It is a long-lived, less-secure credential and grants email reading only. Never collect the ordinary Microsoft password. OAuth setup requires a public-client registration. |
| Apple iCloud | Apple documents account authorization for supported third-party apps, but Vak has no verified integration path for it yet | Current: Apple app-specific password over fixed-host IMAP and CalDAV | The app-specific password is broader and longer-lived than OAuth and its scope is controlled by Apple, not Vak. It is stored only in the owning Agent's credential vault; show the warning before entry and revocation steps after connection. Current support is read-only and separately verified as MailRead, CalendarFreeBusy, or CalendarRead. Never request the Apple Account password. |

Password-based alternatives are local-device setup only: refuse them on a
public/hosted listener, do not put values in the connection ledger, logs,
session history, config, environment files, or repository, and zeroize request
buffers. Show a warning before the user reveals the secret field and a clear
revocation instruction after connection. Do not broaden a password-based
account's capabilities just because its protocol technically permits writes.
The supported choices are intentionally provider-specific: Google offers
OAuth and its limited Gmail-only App Password fallback; Microsoft offers
delegated OAuth plus a verified, read-only personal Outlook.com/Live/Hotmail/MSN
app-password fallback that may be rejected under its changing legacy-auth
policy (Microsoft 365 and work/school Exchange stay OAuth-only);
iCloud currently uses an Apple app-specific password for the supported
protocols; Apple's account authorization for supported third-party apps is a
candidate to investigate, not a Vakyartha connection option until its grant
and protocol contract are verified. No provider accepts its ordinary account password here. A device
authorization code flow, generic IMAP/SMTP credentials, or a Sign in with Apple
token is not silently treated as an equivalent account grant.
Provider documentation: [Outlook.com IMAP settings](https://support.microsoft.com/en-us/outlook/pop-imap-and-smtp-settings-for-outlook.com), [Outlook.com Basic Authentication retirement](https://support.microsoft.com/en-us/support/known-issues/outlook-and-other-apps-are-unable-to-connect-to-outlook-com-when-using-basic-authentication), [Microsoft account app passwords](https://support.microsoft.com/en-us/accounts-billing/manage/how-to-get-and-use-app-passwords), and [Apple account authorization for supported third-party apps](https://support.apple.com/en-us/121539).
For Gmail, the App Password form accepts Google's 16-character value (with
display spaces), verifies it using TLS and read-only `EXAMINE`, and stores it
in the Agent vault. OAuth remains the recommended Google method. The Gmail
mail watch uses a bounded UID cursor and the same encrypted Agent backlog as
other providers.

These are constraints to validate during adapter design, not frozen scope
names in Vak's product contract:

- Google documents distinct Gmail read, compose, and send scopes; mailbox
  reading is classed as restricted and can trigger verification or security
  assessment obligations. See [Gmail scopes](https://developers.google.com/workspace/gmail/api/auth/scopes).
- Google Calendar has narrower free/busy and event scopes than full calendar
  control. See [Calendar scopes](https://developers.google.com/workspace/calendar/api/auth).
- Microsoft Graph distinguishes delegated user permissions from application
  permissions; application permissions can reach mailboxes beyond the signed-in
  user. The first release uses delegated permissions only. Graph's
  [`calendar: getSchedule` API](https://learn.microsoft.com/en-us/graph/api/calendar-getschedule?view=graph-rest-1.0)
  names `Calendars.ReadBasic` as the least delegated permission and does not
  support personal Microsoft accounts. Vak should request that scope for
  free/busy and `Calendars.Read` only for event details; the later adapter must
  report the unsupported personal-account case. See [Graph permission
  guidance](https://learn.microsoft.com/en-us/graph/best-practices-graph-permission)
  and [Exchange application RBAC](https://learn.microsoft.com/en-us/exchange/permissions-exo/application-rbac).
- Apple documents Apple Account authorization for supported third-party apps;
  apps that do not support that flow may use an app-specific password. The
  current branch uses the latter with fixed-host IMAP/SMTP and CalDAV. This
  fallback is not operation-scoped by Apple, so it must remain disclosed and
  read-only when the broker cannot constrain an effect safely. Verify a
  supported authorization flow across desktop and always-on server clients
  before enabling content access. See [Apple's third-party access guidance](https://support.apple.com/en-us/121539),
  [iCloud Mail server settings](https://support.apple.com/en-us/102525), and
  [Apple app-specific passwords](https://support.apple.com/en-us/102654).

## Implementation order and exit tests

Each stage replaces any temporary path it supersedes in the same change
(invariant 30). No stage is marked shipped from a typecheck alone.

Implementation stages, provider matrix, and dependencies are tracked in
`docs/plans/mail-calendar-implementation-plan.md`. That plan was opened by
the owner's explicit request on 2026-09-29; its status governs implementation
progress. This document remains the proposed behavior and acceptance
contract, and its stages are not claims of shipped behavior.

1. **Contract and threat review.** Fix typed operations, connection identity,
   candidate and receipt schemas, policy predicates, retention, provider
   scope matrix, and data-flow diagram. Demonstrate that raw provider calls
   and generic MCP tools cannot bypass the broker or commit gate.
2. **Read-only connection.** One provider, account picker, bounded mail and
   calendar reads, free/busy, citations, attachment handoff, disconnect.
   Test cross-Agent/audience isolation, token absence from every output,
   prompt injection, remote-content blocking, and revocation in flight.
3. **Local drafts and Review.** Reply, new mail, event create/update/cancel,
   source-version conflicts, attachment and recipient disclosure. Verify
   that preparing never causes a provider write and that a changed candidate
   invalidates approval.
4. **Committed effects.** Reviewed send/event writes, exact payload binding,
   idempotency and ambiguous-outcome reconciliation. Exercise timeout after
   provider acceptance, duplicate callbacks, retries, recurring-event scope,
   and changed attendee lists against a provider test double.
5. **Daily routines and second provider.** Add scheduled morning briefing,
   meeting preparation, and follow-up drafts through `TaskDef`, then narrow
   unattended grants for selected effects and continuous bounded watchers.
   Run the same acceptance scenarios
   for all three adapters and for local, web, channel, task, and CLI paths. A
   channel with no approver, a revoked grant, or a narrowed policy must fail
   closed. Exercise missed runs, duplicate triggers, quiet delivery, and
   cross-activity audience isolation. Browser checks cover desktop and phone
   sizes, light and dark, with the actual account, automation, Review,
   conflict, and receipt states.

The acceptance suite must also exercise D7–D11: forged Reply-To and header
injection, Bcc and group disclosure, MIME bombs, unread state preservation,
daylight-saving transitions and all-day dates, recurring exceptions, RSVP
versus organizer cancellation, account-link callback replay, inference-route
disclosure restrictions, concurrent commits and restart during dispatch,
partial workflow success, automation loops, quota exhaustion, revoked source
access, trash, and retention. Accessible keyboard and screen-reader Review
must expose account, audience, effects, and unknown outcomes. Provider test
doubles establish failure behavior; authorized live test accounts establish
actual provider effects and notification semantics before release.

For continuous operation, test service restart, host sleep/wake, network
outage, subscription expiry, invalid cursors, token refresh races, duplicate
service instances, bounded backlog, and pause during an active run. A sustained
24-hour service test must show bounded memory/queue growth, correctly expired
or caught-up work, no duplicate effects, and useful degraded/recovered states.
For the working area, verify source → edit → preview → Review → receipt,
save/reopen, concurrent edits, stale sources, partial action selection, and
automation preview in the running browser at 1440 × 900 and 390 × 844 in both
light and dark themes. Save screenshots and evidence with the execution plan.

## Deferred and open design checks

- Bulk sync and search across the whole mailbox wait for the shared intake
  catalog and lifecycle work in `76-intake-and-knowledge.md`.
- Mailbox deletion, rules, filters, forwarding settings, delegated/shared
  mailboxes, and organization-wide application permissions need separate
  threat reviews and explicit owner direction.
- Provider-side drafts and offline queued sends need a separate write and
  reconciliation contract. A local proposal alone does not justify either.
- Cross-provider invitations and recurring-event edge cases require adapter
  fixtures before promising equivalent behavior.
- Local `.eml` and `.ics` import/export are a useful later extension beside
  Office/PDF: bounded worker parsing, structured preview, and portable output.
  Importing an invitation never accepts it or contacts attendees. Calendar
  file actions, alarms, and remote attachments never execute automatically.
  PST/OST/mbox archives, S/MIME/PGP decryption and signing, custom IMAP/SMTP,
  custom CalDAV, and providers beyond Google, Microsoft, and Apple are not
  implied by the first adapters.
- Before implementation, verify each provider's current OAuth review rules,
  scope availability, conditional-write support, send reconciliation options,
  webhook authentication, and rate limits. A missing safe reconciliation path
  narrows the supported action; Vak promises no blind retry of an ambiguous
  effect, not universal exactly-once delivery.
