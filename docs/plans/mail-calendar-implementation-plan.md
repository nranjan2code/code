# Plan — secure mail and calendar package

Status: **Stage 0 and Stage 1A Agent/account linking complete; bounded
Google/Microsoft owner previews, owner-submitted bounded inbox search for all
three providers, the broker-owned local Agent read tool with clickable
provider-reverified message citations, and
the first Stage 2 increment (bounded Agent-vault local drafts with revisioned
save/delete and disconnect cleanup) are implemented on `codex/mail-calendar`;
the owner calendar preview now has grouped agenda, day, and week layouts;
the first Stage 4 increment adds scheduled read-only routines and a bounded,
encrypted scheduled mail-watch backlog on `TaskDef`; the first Stage 3
increment adds an owner-confirmed, permission-checked, digest-bound plain
email send and a
limited timed event create for Google and Microsoft without attendees,
recurrence, or reminders.
Calendar and availability previews now support an owner-selected, device-time-
zone date range up to 30 days, previous/next seven-day navigation, and a visible
refresh time. Owner folder/label selection now covers Gmail labels and up to 100
Microsoft top-level folders; Apple remains Inbox-only. Child-folder traversal,
the full connected-account workspace and complete thread workspace remain
open; Google and Microsoft now have an owner thread preview paged in batches
of at most 20 messages.
Apple UID, Gmail history, and Microsoft Graph per-folder delta pagination are
now implemented for the scheduled mail watch, with each bounded page's IDs and
continuation cursor stored atomically in the encrypted Agent vault. A local
browser smoke check passed at 1440 × 900 and 390 × 844 with no console warnings
or errors; full source-to-Review browser acceptance and live provider checks
remain open.
Apple availability now has a separate CalendarFreeBusy-only path that returns
busy intervals through CalDAV `free-busy-query`; event detail access remains a
separate CalendarRead grant. Mixed Apple capabilities remain unverified.
The maintainer authorized continuing against the current 4.x storage model on
2026-09-30.** The
owner opened this feature branch on 2026-09-29. The design contract is
`docs/design/80-mail-and-calendar.md`. This plan stages the work so a secure
read path, exact preview, and provider actions can be reviewed as concrete
increments. No milestone claims 24/7 operation until the durable service and
recovery acceptance checks pass. Current storage does not provide crypto-shred
or complete account-deletion erasure: provider content copied into append-only
Agent session history may remain until future lifecycle work provides lineage
and erasure. This limitation must be visible before a content feature is
enabled.

On 2026-09-30, the first Apple increment verified Mail-only accounts by checking
IMAP login before storing the credential, then reading at most 20 inbox
envelopes in a read-only mailbox with a transport byte cap. At that point,
Calendar selections were unverified and mail bodies were unavailable. The
later CalDAV increment below adds the separately verified CalendarRead path.
These increments still use the current storage model and do not provide
selective erasure of copied session content.

A fixed-host CalDAV read path verifies iCloud CalendarRead-only account links,
discovers the principal and calendar collections with authenticated PROPFIND,
validates every discovered href against the Apple HTTPS origin, and retrieves
a bounded time-range query. Apple CalendarFreeBusy-only links now use the
CalDAV `free-busy-query` report and return only worker-projected UTC busy
intervals; no event details cross the worker boundary for this operation.
CalendarRead, CalendarFreeBusy, and MailRead are each verified only as
individual selections; mixed selections remain `connected_unverified`.
No credentialed live Apple Calendar request has been made; local fixtures
verify response bounds, origin checks, busy projection, and worker parsing,
so the provider's authenticated discovery and free/busy semantics still need
live verification. Event changes remain unavailable.

The worker parser accepts bounded standalone VCALENDAR data and CalDAV
multistatus XML, projects at most 100 events, refuses ambiguous or unsupported
local times, and redacts private event details while preserving busy times. It
runs through the versioned tool-worker broker, with a private empty scratch
directory and the network-denied verification sandbox on supported platforms.
The same worker now has a bounded MIME decoder that selects explicit
`text/plain` parts, skips HTML and attachments, caps decoded text and part
count, and explicitly labels messages without plain text. Apple Mail does not
yet fetch and pass a selected message through this parser, so Apple message
bodies remain unavailable.
After parsing, the broker also applies the requested time-range overlap filter
locally to every provider response. This prevents a provider response that
ignores the requested range from broadening the preview.

## Scope and provider matrix

The first three provider targets are Google Workspace/Gmail, Microsoft
365/Outlook, and Apple iCloud Mail/Calendar. Providers share a typed domain
contract and Review flow, but not an authentication or transport
implementation.

| Provider | Authentication and APIs | Initial security boundary |
|---|---|---|
| Google Workspace / Gmail | Preferred: delegated OAuth authorization code with PKCE through the system browser. Additional: Google App Password over fixed-host Gmail IMAP, when the account offers App Passwords. | Incremental OAuth scope verification and separate effect capabilities. App Password is a long-lived credential, less secure than OAuth, and may be unavailable for managed, Advanced Protection, or some 2-Step Verification configurations. Its local-only route verifies TLS IMAP access, stores only in the Agent vault, and permits Inbox metadata plus selected worker-parsed messages, scheduled UID watches, and MailRead only. It never grants Calendar or send. |
| Microsoft 365 / Outlook | Preferred: delegated Entra OAuth with PKCE. Additional: local app password for personal Outlook.com/Live/Hotmail/MSN accounts, limited to fixed-host IMAP MailRead and verified before storage. | Microsoft 365 and work/school Exchange remain OAuth-only because Exchange Online disables Basic Authentication. Microsoft documents app passwords for consumer legacy clients, while Outlook.com's current IMAP setup requires OAuth2; this fallback may be rejected as legacy authentication changes. It is local-only, less secure, and never accepts the ordinary Microsoft password. No calendar or provider write access. Graph `getSchedule` does not support personal Microsoft accounts, so OAuth personal accounts report free/busy unavailable. |
| Apple iCloud | Current: local Apple app-specific password over fixed-host IMAP and CalDAV. Investigate Apple's newer account-authorization flow for supported third-party apps before claiming it as a Vakyartha option. | Exactly `MailRead` verifies TLS IMAP sign-in and exposes bounded inbox metadata plus explicitly selected plain-text message reads parsed in the worker; exactly `CalendarRead` verifies the fixed CalDAV endpoint and exposes bounded event previews through worker-isolated discovery and parsing; exactly `CalendarFreeBusy` uses CalDAV `free-busy-query` and returns only worker-projected UTC intervals. Event effects, HTML-only message bodies, and mixed capability selections are unavailable or `connected_unverified`. Apple app-specific passwords are broader than per-operation grants and must carry a warning before entry and revocation instructions after connection. |

The implemented read adapters use Gmail's bounded message list/get methods and
Calendar's event-list/free-busy methods, plus Microsoft Graph's Inbox message
list, bounded `calendarView`, and `getSchedule` endpoints. They use fixed
provider hosts and delegated scopes. Current API contracts:
[Gmail messages.list](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/list),
[Google Calendar events.list](https://developers.google.com/calendar/api/v3/reference/events/list),
[Google Calendar freebusy.query](https://developers.google.com/calendar/api/v3/reference/freebusy/query),
[Graph messages.list](https://learn.microsoft.com/graph/api/user-list-messages?view=graph-rest-1.0),
[Graph calendarView](https://learn.microsoft.com/graph/api/user-list-calendarview?view=graph-rest-1.0), and
[Graph getSchedule](https://learn.microsoft.com/graph/api/calendar-getschedule?view=graph-rest-1.0).
The limited create adapters follow [Google events.insert](https://developers.google.com/workspace/calendar/api/v3/reference/events/insert)
and [Microsoft Graph create event](https://learn.microsoft.com/graph/api/user-post-events?view=graph-rest-1.0). Google standalone event updates use [ETags and conditional requests](https://developers.google.com/calendar/api/guides/version-resources); Microsoft updates remain disabled until a conditional update contract is verified.

The third adapter is not a generic arbitrary-host IMAP/CalDAV feature. Custom
servers, Yahoo, Fastmail, Exchange EWS, and generic SMTP are out of scope for
this plan. iCloud's app-specific-password fallback is broader than Vak's
per-operation capabilities; the adapter must disclose that difference and
cannot claim provider-enforced least privilege. [Apple's support guide](https://support.apple.com/en-us/121539)
now describes authorizing supported third-party apps with the Apple Account
and revoking that access from Account Data Sharing. Apple's developer OAuth
service documents `edu.users.read` and `edu.classes.read` for the Apple School
Manager Roster API, not iCloud Mail/Calendar
([authorization scopes](https://developer.apple.com/documentation/accountorganizationaldatasharing/request-an-authorization),
[Roster API](https://developer.apple.com/documentation/rosterapi/)). EventKit
requires a native app's on-device calendar permission and does not define a
server-side grant for unattended routines
([EventKit access](https://developer.apple.com/documentation/eventkit/accessing-the-event-store)).
Apple's published iCloud Mail configuration documents IMAP on
`imap.mail.me.com:993` with TLS and an app-specific password, and SMTP on
`smtp.mail.me.com:587`; SMTP remains outside the read-only profile
([Mail settings](https://support.apple.com/en-us/102525)). Apple does not
publish a corresponding iCloud Calendar server API contract on that page.
Before enabling Apple previews, implement and verify both fixed-host protocol
adapters, credential checks for the requested mail and calendar capabilities,
and worker-isolated parsing of untrusted IMAP MIME and CalDAV/iCalendar
responses. Do not treat saving an app-specific password as verification or
provider support. The app-specific password has broader authority than Vak's
per-operation grants and must never be injected into Agent tools.
Do not substitute Sign in with Apple: it authenticates the person to
Vakyartha but does not grant iCloud Mail/Calendar access ([Apple's Sign in with
Apple overview](https://developer.apple.com/documentation/signinwithapple/authenticating-users-with-sign-in-with-apple)).
Before iCloud content access or always-on service support ships, resolve the
supported integration protocol, token storage/refresh, and revocation contract
with Apple documentation or Apple Developer Support. No Apple Account password
is accepted.

### OAuth setup for the current account-linking stage

Google and Microsoft account linking is configured by the installation
operator, not by an end user pasting OAuth credentials into the UI:

- Set `VAK_GOOGLE_OAUTH_CLIENT_ID` to the public client ID for a Google
  **Desktop app** OAuth client, and enable the OAuth consent screen for the
  intended test users. No client secret is configured or stored.
- Set `VAK_MICROSOFT_OAUTH_CLIENT_ID` to the application (client) ID for a
  Microsoft Entra **public client** registration. Enable public-client flows
  and register the native redirect `http://localhost/mail-calendar/oauth/callback`;
  Entra ignores the port for localhost redirects, allowing the desktop's
  ephemeral loopback port. No client secret is configured or stored.
- Start Vakyartha and its system browser on the same device. Google returns to
  the local loopback IP and the server's actual listener port; Microsoft
  returns to the registered `localhost` path and ephemeral listener port. A
  remote browser, hosted callback, or public server URL is rejected.
- The UI requests only the selected capability scopes. Current Gmail read
  access uses Google's restricted `gmail.readonly` scope; any deployment
  distributing this feature must complete Google's applicable verification
  and security-assessment requirements before enabling that content path.
- Google authorization disables `include_granted_scopes`, and the callback
  rejects any granted scope outside the selected request. Refresh responses
  receive the same excess-scope check when a provider returns a scope list.
  This prevents a prior broader Google consent from silently widening a new
  account link. See Google's [native-app scope verification guidance](https://developers.google.com/identity/protocols/oauth2/native-app)
  and [incremental authorization behavior](https://developers.google.com/identity/protocols/oauth2/web-server#incrementalAuth).

These instructions configure OAuth account linking. Bounded Google and
Microsoft content reads are enabled for owner-only previews on the feature
branch using the current storage model. Agent/model reads are a separate
in-progress gate. Apple iCloud currently uses its separate local
app-specific-password enrollment path and has no OAuth client setup.
The redirect behavior follows Microsoft's [localhost port matching rules](https://learn.microsoft.com/entra/identity-platform/reply-url)
and Google's [desktop installed-app loopback flow](https://developers.google.com/identity/protocols/oauth2/native-app).

Microsoft free/busy requests use `Calendars.ReadBasic`, the least delegated
permission documented for Graph `calendar: getSchedule`; event-detail reads
request `Calendars.Read`. `getSchedule` is not supported for delegated
personal Microsoft accounts, so the later content adapter must expose that
limitation rather than silently requesting a broader scope.

## Stage 0 — complete contract and boundary audit

Resolve the review findings before building an effectful path:

- Reconcile the 4.x storage and secret-store reality with doc 80's future
  storage assumptions. Do not add a parallel mailbox database or claim
  encryption/erasure that the current substrate cannot provide.
- Define privileged connection admission, callback handling, provider
  credential redemption, pinned egress, exact typed operation schemas,
  candidate lifecycle, durable claim/idempotency, and approval wiring.
- Define an Agent-scoped vault record for provider credentials and identity;
  keep durable connection records limited to opaque vault references and
  enforce owning-Agent plus audience checks on every operation.
- Fix and validate doc 80's descriptions and add the three-provider contract.
- Record which fields must reach session entries for model reconstruction and
  which belong only in content records or redacted telemetry.

**Exit:** threat/data-flow diagram and operation/provider matrix reviewed in
the plan and design docs; each persistent path and provider effect has one
owner, authorization boundary, and failure behavior. Any boundary unavailable
today remains blocked until implemented; it is not replaced by broad MCP or
process environment access.

## Stage 1 — owner-linked providers

Stage 1 separates account linking from content reads. The original M7 gate
was explicitly waived by the owner on 2026-09-30 to continue against current
storage; the append-only history limitation is disclosed at every content
entry point, and no claim of complete account-deletion erasure is made.

### Stage 1A — account linking

Build Agent-owned account records, OAuth callback/state handling for Google
and Microsoft, and local app-password forms for Google Mail and iCloud. Google
App Password setup verifies `imap.gmail.com:993` with read-only `EXAMINE`, and
its account receives only `MailRead`; it never grants Calendar or send. Both
password forms are refused on public or hosted listeners and keep credential
material in the Agent credential vault. Provider clients use fixed hosts;
OAuth tokens and app passwords remain in the credential service. OAuth refresh preserves the existing grant ceiling and rotates
refresh tokens inside the Agent vault. Google supports the provider's per-token
revocation endpoint; Microsoft disconnection reports when its broader
revocation is not available to this app.
Add no mailbox poller before its cursor, queue, quota, and service lifecycle
are durable.

Owner-only account inventory and disconnect cleanup remain available while an
Agent is paused or archived, so a non-runnable Agent does not strand provider
credentials. Connecting or refreshing credentials still requires an active
Agent. Hard deletion of Agent-owned data remains part of the data-architecture
lifecycle work.

The Stage 1A account-linking increment initially accepted only `mail.read`,
`calendar.freebusy`, and `calendar.read`. For Microsoft, the calendar scopes
are separated: `calendar.freebusy` requests `Calendars.ReadBasic`, while
`calendar.read` requests `Calendars.Read`. Stage 3 now adds optional
`mail.send` for Google and Microsoft because its exact-draft broker path is in
place; it remains unselected by default. Calendar writes and prepare scopes
remain rejected even if a caller bypasses the Settings UI.

**Exit:** local contract tests and provider test doubles prove callback replay
rejection, wrong-principal rejection, scoped account/audience isolation,
credential redaction, bounded OAuth responses, exact granted-scope handling,
refresh grant-ceiling preservation, duplicate-principal rejection, and
idempotent disconnect. The append-only connection ledger carries a durable
Agent/provider OAuth fence. Each OAuth attempt captures its value at
initiation, and the callback checks it while holding the cross-process ledger
lock across its vault write and account activation. A disconnect therefore
invalidates older callbacks even when it runs in another server process. A
regression using independent ledger handles proves a stale callback cannot run
its credential-write operation. Each provider reports linked, unverified,
reauthentication-required, or explicitly unsupported status without implying
that content access has been tested. Multiple distinct accounts per provider
remain supported; pending links serialize per Agent/provider, and an active
principal cannot accumulate a second capability grant. A second attempt made
while a provider link is pending receives a conflict and can be retried after
the first link finishes or is cleaned up.

### Stage 1B — bounded content reads, current storage

The first slice implements owner-only bounded folder, event, and free/busy
previews for Google and Microsoft through fixed-host, read-only adapters. A
broker-owned Agent tool now offers bounded reads on the local owner surface;
channel audiences fail closed until explicitly shareable grants are built.
The tool never exposes vault handles to workers. Returned provider content is
model-visible and enters append-only session history, which current storage
cannot selectively erase. The UI and tool description disclose this before
reads. Do not claim complete account deletion. Per-message citation metadata
is present for selected Google/Microsoft threads. Exact inline citation tokens
open the owner-only conversation preview and focus the cited message when it
is on the loaded page. Provider health/freshness and further prompt-injection
handling remain to be completed. The
provider exit requirements for
bounded responses, no remote HTML loads, prompt-injection containment,
revocation during a read, and no message-read mutation apply here. Each
provider must return useful structured views or an explicit unsupported or
reauthentication state.

**Implemented increment (2026-09-30):** the Agent tool accepts an optional
owner-selected folder/label for `recent_mail` on Google, Microsoft, or Apple.
Before reading, it verifies that the folder ID appears in the bounded folder
inventory returned by that same account. Scheduled routines persist one
`mail_folder_id`, verify it against the provider on creation and at every
read, and reject model-supplied folder changes. Older routines remain scoped
to Inbox. Continuous new-mail watches remain Inbox-only because their durable
provider cursors are currently Inbox-scoped; Apple exposes Inbox only.

**Implemented increment (2026-09-30):** each Agent-read thread message now
includes a bounded, percent-encoded `mailcite:` token. Both streaming Markdown
and settled presentation rendering recognize only that token form. Selecting
it opens the mail/calendar Settings page, verifies the cited account belongs
to the currently selected Agent, re-fetches the thread through the owner-only
provider preview API, and scrolls to the cited message when it is in the
loaded page. IDs stay opaque; the UI never constructs provider URLs. Citation
navigation now follows up to 20 more pages within that conversation; beyond
that cap, the person can continue with the manual Load more control. Apple
thread citations and the full thread workspace remain open.

## Stage 2 — working area and local candidates (in progress)

Build the shared account picker, mail reader/composer, calendar agenda and
event editor, local autosave, structured payload preview, and revisioned
candidates. Attachment preview uses existing worker readers. Implement
conflict display and accessibility. No provider writes in this stage.

**Exit:** a person can navigate source → edit → preview → Review → discard,
save/reopen, and resolve concurrent local edits on desktop and phone, light
and dark. Browser screenshots meet doc 80's sizes. Preparing and editing
candidates cannot contact provider write endpoints.

**Implemented increment (2026-09-30):** the Settings surface can create,
reopen, revise, and delete email/event draft candidates. The broker stores at
most 32 candidates and 2 MiB per Agent in that Agent's credential vault,
enforces revision compare-and-swap, validates Agent/account/source lineage,
and removes candidates for an account during disconnect cleanup. Drafts are
local only; this does not complete source-to-edit navigation, exact-effect
preview, Review choices, mobile/accessibility screenshots, or provider writes.
An exact local-field preview now shows the currently edited email recipients,
subject, and body or the event title, local start/end time, location, and
description. This preview explicitly states that it does not send or alter
provider data; it is not the later effect-aware Review step.

**Implemented increment (2026-10-01):** owners can create new mail and
standalone event drafts without a linked provider account. These incomplete
drafts are encrypted in the owning Agent's vault, accept no provider source
references, and use a reserved local identity that cannot authorize an effect.
Assigning one to a linked account creates a separate account-bound candidate;
it does not contact the provider or skip exact Review. Replies and provider
updates/deletions remain unavailable in the unassigned path. Verification:
111 mail-calendar unit tests, Core mail-calendar tests, server account-scoped
candidate HTTP tests, server library check, formatting and diff checks, and
web build pass. The isolated preview refresh and signed-in browser acceptance
passed for accountless local-draft workflows; connected-account
source-to-Review acceptance remains open.

**Implemented increment (2026-10-01):** browser acceptance in the isolated
`mail-calendar-dev-test` profile exercised a synthetic local event through
create, exact local preview, save, edit, close, and reopen with no provider
account. It exposed a duplicate autosave race after an explicit save. The UI
now clears stale debounce timers on explicit save, draft switch/open, and
close; a browser recheck waited beyond the debounce window and confirmed the
candidate stayed at revision 4 rather than receiving a duplicate revision.
Provider credential warning/forms were also checked at 1440×900 and 390×844
in both themes; their phone layout was fixed to keep warnings full-width and
stack the inputs. These are visual observations from the running browser;
the screenshot files required for the full evidence package, source-to-Review
acceptance, and connected-account checks remain open.

**Implemented increment (2026-09-30):** owner calendar and availability
previews now accept a selected local date range of up to 30 days, convert the
boundaries to instants using the device time zone, and show the result refresh
time and queried dates. This adds bounded date navigation without expanding
provider scopes, adds previous/next seven-day navigation, and flags overlapping
timed or all-day entries in the selected account preview. An explicit owner
comparison now checks up to five other readable calendar accounts; calendar-
source selection and browser acceptance with connected test accounts remain
open.

**Implemented increment (2026-09-30):** the calendar preview now offers an
agenda grouped by local date, a day timeline, and a seven-day timeline over the
currently selected preview range. Timed events follow local wall-clock hours,
all-day entries have a separate lane, and current-preview conflicts remain
labelled without relying on colour alone. Owners can explicitly compare up to
five other connected calendars for the same range; partial read failures are
shown and compared events are read-only. This remains an incremental preview,
not a full calendar workspace: source selection, editing attendees, proposed
slots, occurrence/series choices, and browser acceptance remain open.

**Implemented increment (2026-09-30):** Google and Microsoft owner inbox
previews now return bounded attachment cards. A person can select one supported
PDF, Open XML, or plain-text attachment up to 1 MiB; the server rechecks its
parent message and provider metadata, then passes bounded bytes only to the
network-denied document worker. The owner UI receives capped extracted text,
not attachment bytes. Inline content, Apple attachments, images, HTML, archives,
and unsupported or oversized files are unavailable. Local staging, source
version citations, attachment selection during compose, and send-with-attachment
remain open.

**Implemented increment (2026-09-30):** owner mail previews can list and select
Gmail labels or up to 100 Microsoft top-level folders, and inbox phrase search
is scoped to the selected label/folder. Gmail label IDs are allowlist-validated;
Microsoft opaque folder IDs are validated and encoded as one fixed-host path
segment. Folder inventory and reads remain owner-only and pass through the
existing MailRead account/audience admission. Apple exposes its Inbox only;
scheduled Agent tools and mail watches remain Inbox-scoped. Nested folders,
pagination beyond the bounded list and full thread workspace remain open.

**Implemented increment (2026-09-30):** a person can open one selected Google
or Microsoft conversation from the owner mail preview. Results are capped at
20 messages and a 256 KiB provider response, every returned message must match
the requested thread/conversation ID, and the surface labels message content
untrusted. Opaque IDs are handled as fixed-host path/query data, with Microsoft
OData quoting escaped. Apple remains selected-message-only. This does not yet
provide cross-folder Agent reads, full thread navigation/citations, reply
headers, or compose-in-thread semantics.

**Implemented increment (2026-09-30):** the owner can draft and send a reply to
a selected message in an opened Google or Microsoft conversation. Reply actions
require both MailRead and MailSend, retain message and conversation IDs in the
reviewed candidate, and re-fetch the source immediately before dispatch. A
changed source ID, thread/conversation ID, or subject fails closed. Gmail sends
validated `In-Reply-To`/`References` headers and `threadId`; Graph uses its
message-scoped `/reply` endpoint. IDs are appended as fixed-host path segments.
The complete conversation workspace and live-provider conformance remain open.

**Implemented increment (2026-09-30):** the brokered Agent read tool can read
one selected Google or Microsoft conversation page using the `thread_id` from
`recent_mail`. `MailRead` admission is rechecked after provider I/O; routines
must explicitly grant the `mail_thread` operation and stay within their
configured per-run item ceiling. Page size is limited to the requested ceiling
and at most 20; provider continuation cursors are bound to their thread and
page size. Every result carries a per-message source record bound to provider,
account, Agent audience, thread, and message IDs, and wraps bodies as untrusted
external content. Apple conversation reads remain unavailable. Click-through
source navigation, cross-folder Agent access, and live-provider conformance
remain open.

## Stage 3 — reviewed provider effects

Add typed send, event create/update/cancel, and RSVP operations through the
broker. Build single-use digest-bound approval, standing-grant checks,
durable action claims, provider-specific reconciliation, receipts, conflicts,
and unknown outcomes. Support a provider action only when its adapter can
explain relevant notification and recurrence effects. Apple iCloud external
effects stay disabled if broad app-specific-password authority cannot be
contained to the confirmed operation.

**Exit:** one reviewed payload causes at most one blind dispatch; an ambiguous
timeout never retries blindly. Edited candidates, changed permissions,
expired approvals, spoofed recipient fields, recurring-series changes, and
revoked accounts fail closed. Provider-accepted is not shown as delivered.

**Implemented increment (2026-09-30):** the Settings work area now supports
an explicit opt-in provider send grant for Google and Microsoft, exact
candidate digest/revision review, and a confirmed owner-only send action. The
broker evaluates `mail_calendar_send` through the Agent Core permission engine,
rechecks the Agent/account/capability/vault, validates the plain-text profile,
then writes a cross-process single-use action claim before dispatch. Receipts
are encrypted in the Agent vault, included as status in the candidate list,
and removed on account disconnect. Unknown network outcomes cannot be retried;
accepted means provider accepted, not delivered. Attachments, aliases, reply
semantics, event updates/cancellations/RSVP, model-initiated effects, standing grants and full
provider reconciliation remain unavailable. The first server integration
test proves stale digests and accounts without `mail_send` are rejected before
a claim or provider call. The effect adapter tests cover provider payloads and
ambiguous responses.

**Implemented increment (2026-09-30):** the same work area now has an opt-in
`calendar_write` grant and exact-review create action for Google and Microsoft.
It accepts only one timed event with no attendees, recurrence, occurrence
target, or reminders. Provider payloads explicitly disable reminders and
include no invitees; event times are submitted as UTC instants. A distinct
`mail_calendar_event_create` Core permission decision and durable claim protect
the effect. CalendarWrite provider scopes can authorize broader operations
than this package currently exposes, so credentials remain private and the
broker enforces the narrower action.

**Implemented increment (2026-09-30):** Google calendar preview now retains
the provider ETag and recurrence marker. The owner can stage an update only for
a public, standalone, timed event with no attendees. Review binds the event ID,
source ETag, candidate revision, and digest. Immediately before the update the
broker re-reads the event, verifies it is still the same eligible event, then
sends a partial PATCH with `If-Match`; reminders and attendees are not replaced
or notified. A changed/deleted event is a conflict and consumes the single-use
attempt, requiring fresh preview and review. Redirects and ambiguous provider
failures are never retried. Microsoft Graph update remains unavailable because
the current verified contract does not establish an equivalent conditional
write precondition; Apple remains unavailable.

**Implemented increment (2026-09-30):** Google now supports owner-reviewed
cancellation of one unchanged public, standalone, timed event with no attendees
when the connected owner is the organizer. The cancellation candidate binds the
event ID and ETag; the broker re-reads and validates the source, then sends a
DELETE with `If-Match` and `sendUpdates=none`. Occurrence and whole-series
cancellation are rejected. Stale events conflict; redirects and ambiguous
provider failures remain non-retryable. Microsoft and Apple cancellations,
events with guests, and recurring or all-day events remain unavailable. Tests
must prove the exact conditional request and all eligibility boundaries before
this increment is considered verified.

**Security correction (2026-09-30):** Google Calendar previews now redact the
title, location, description, and attendee count of private events, matching
the existing Microsoft projection while preserving the busy interval. A
provider-parser regression test verifies the private fields do not escape.

## Stage 4 — scheduled and continuous routines

Use `TaskDef` and the existing service manager for on-demand, scheduled, and
watching modes. Begin with bounded polling; add provider push subscriptions
only with verified callback authentication and durable cursor reconciliation.
Build routine preview, per-source read grants, freshness, budgets, dedupe,
pause/run-once/revoke, quiet delivery, catch-up rules, host health, and
automation history. Multi-host watchers remain off until fencing is present.

**Implemented increment (2026-09-30):** owner Settings can create, pause,
resume, run once, inspect, and delete a scheduled read-only routine using the
existing `TaskDef` scheduler. Each routine pins one admissible Agent revision,
one linked account, a nonempty allowlist of mail/calendar read operations, and
a maximum of 20 returned items. The server rechecks the selected account's
current capability grants and vault credential at creation; the broker repeats
the operation, account, grant, credential, and revision checks at execution.
The child runs unattended in `ReadOnly` mode with only the mail/calendar
brokered tool exposed, no script, no delivery target, and no MCP/skills/hooks/
plugins. It stores run content only in the Agent's append-only run session,
not in shared task summaries or Inbox delivery. Disconnect removes the vault
credential, local candidates, and mail-watch IDs, pauses routines for that
account, and fences reads. An optional mail watch scans up to 100 recent
provider IDs into a bounded encrypted Agent-vault backlog and fetches at most
the routine's configured batch by explicit IDs. The content-free poll does
not fetch message bodies. Fetched IDs remain staged until the scheduler
observes a completed run; failed or interrupted runs requeue them, preserving
at-least-once recovery across local restarts. Apple now advances a bounded
UIDVALIDITY/UID cursor, Gmail follows bounded history pages from its stored
history ID, and Microsoft follows bounded Graph delta links for the inbox.
Each page advances its encrypted cursor atomically with queued IDs, so arrivals
beyond one page can be drained on later checks; backlog overflow fails closed. An
Agent-vault OS
advisory lease now prevents duplicate polls or dispatches by
local server processes; it is held through child completion
and released by the OS on process exit. It does not coordinate separate hosts
or provide a freshness guarantee. The ordinary Vakyartha service must remain
running for schedules to fire. Provider push subscriptions, awake-host health,
and the 24-hour restart/recovery acceptance are not implemented yet. A focused
server regression now joins startup's `working` → `interrupted` recovery marker
to a reopened Agent vault and verifies staged mail IDs are requeued while the
provider continuation cursor is retained. This is process-restart state coverage,
not a 24-hour service acceptance run. Current-storage session
history is append-only and cannot be selectively erased; M7 remains the
deletion gate.

**Implemented increment (2026-09-30):** mail watches can run on their selected
five-field schedule or check continuously at a one-minute interval through the
existing `TaskDef` scheduler. Continuous mode remains read-only, uses the
encrypted bounded backlog above, and skips model runs while no mail is waiting.
It depends on the service host staying awake and connected; this does not
establish push delivery, a source-freshness target, or 24/7 availability.

The routine's configured item limit is enforced across all mail/calendar tool
calls in one run. Concurrent calls reserve from the same per-run budget before
provider I/O; unused capacity is returned, while failed or cancelled calls
release their reservation.

Missed scheduled slots now use each task's configured IANA timezone during
startup catch-up, with instant-based comparison across daylight-saving gaps
and folds. A server regression covers a New York fall-back slot missed while
the process was down. This corrects the shared `TaskDef` scheduler; it does
not provide Google/Microsoft provider-native cursors or turn best-effort polling into a 24/7
availability guarantee.

**Implemented increment (2026-10-01):** continuous mail watches now persist a
separate `mail_calendar_last_check_at` only after the provider cursor poll
succeeds. The routine list uses this value for “last successful check” instead
of treating a model-run start or failed attempt as source freshness. Legacy
`tasks.json` files load with the field unset and continue to round-trip. This
does not prove that the service process is awake or healthy between checks;
host-health reporting and sustained recovery acceptance remain open.

**Implemented increment (2026-10-01):** mail/calendar routines are now saved
paused. The owner can run a one-off, read-only preview and inspect the Agent
run history before choosing Resume; this keeps a newly configured schedule or
watch from starting before its scope and sample result are reviewed. Other
TaskDef kinds retain their existing create-and-run behavior. Server regression
coverage verifies this creation default, and the owner UI labels the first
manual execution “Preview run”.

**Implemented increment (2026-10-01):** a continuous one-minute watch now
shows a plain-language overdue state when it has not completed a provider
check for more than three minutes. The timestamp continues to distinguish
source freshness from run status; scheduled watches are not judged by the
continuous threshold, and an overdue label does not claim to diagnose host
health. Pure client tests cover unknown, current, overdue, and scheduled
states.

**Exit:** 24-hour service test with restart, sleep/wake, network and provider
outages, expired tokens/cursors, duplicate triggers, queue limits, pause during
run, and recovery. Every missed/expired trigger has an explicit state, and
there are no duplicate effects or unbounded backlogs. A sleeping local host is
reported as offline; 24/7 availability requires an awake service host.

## Stage 5 — provider conformance and release readiness

Run the full acceptance suite against Google, Microsoft, and Apple adapter
test doubles, then authorized live test accounts. Verify provider scope
reviews, Apple credential revocation/setup, calendaring semantics, callback
registration, rate limits, localized errors, data retention, backup/purge,
UI, and docs. Update AGENTS.md and doc 80 statuses only for behavior that has
actually shipped.

**Exit:** all supported provider paths pass the same audience/approval
contract; each provider's unsupported cases are visible. Security review,
running-browser evidence, and release checklists are complete. If Apple's
broad credential cannot meet the chosen permission guarantee, the provider
remains read-only until an acceptable credential model exists.

## Build order and repository constraints

Provider content reads may use the current 4.x substrate under the maintainer's
2026-09-30 direction. Do not add a second durable content cache. The current
direct preview stays owner-only; Agent reads use the broker-owned Core tool
and local surface grant. Account disconnect removes credentials and revokes
access where supported, but does
not erase copied content from append-only session history; account/Agent
deletion must present this limitation until the lifecycle architecture ships.
This feature work does not start a data-architecture milestone.

The user's build is running in the primary checkout. All feature edits are in
the managed worktree for `codex/mail-calendar`; do not change primary checkout
files or stop its process. Do not use a second schedule model. New durable
files must be registered in `vak_core::state::REGISTRY`, all paths resolved
through `vak_config::paths` and Core home accessors, and records use full
UUIDv7 IDs. Model-visible inputs must be append-only session entries, logs
must not carry content, and all provider effects must pass the same broker
authorization and approval boundary on every execution path.

**Implemented increment (2026-10-01):** bounded run history is persisted as
content-free metadata in each owning Agent's encrypted vault and shown under
each routine in Settings. Entries identify the routine/account, trigger,
status, timestamps, and optional Agent session ID; message content and model
output stay in the append-only Agent session. The history endpoint verifies
the owner Agent and workspace routine scope. Account and routine deletion
remove their matching history. Vault lifecycle tests cover isolation, bounds,
restart persistence, settlement, interruption, and deletion. This metadata
cleanup does not erase content already copied into append-only sessions; M7
remains the deletion gate.

**Implemented increment (2026-10-01):** opening a cited message now follows
the provider's continuation cursor within the same selected account and
conversation until the target is loaded, the provider is exhausted, or 20
additional pages have been fetched. Repeated cursors are rejected, duplicate
message IDs are collapsed, and any remaining cursor stays available through
the manual Load more action. The page cap bounds automatic reads at 420
messages. Unit tests cover first-page targets, later-page targets, exhausted
conversations, repeated cursors, and the ceiling; signed-in browser acceptance
remains open.

## Progress log

- 2026-10-01: Conversation citations now follow at most 20 additional
  same-thread pages (420 messages total) to locate a cited message. Provider
  cursors are tracked to stop cycles, duplicate message IDs are collapsed, and
  further pages remain manually loadable. Five focused client tests pass;
  browser verification requires an unlocked local session.

- 2026-10-01: Strengthened routine duplicate-trigger coverage with 16
  independent Agent-vault handles contending at once for one routine lease.
  Exactly one may dispatch at a time, and the lease can be reacquired after
  the winner exits. This checks local process contention, not multi-host
  coordination or the sustained 24-hour recovery gate.

- 2026-10-01: Added owner-visible routine run history backed by bounded,
  content-free records in the encrypted Agent vault. The server verifies Agent
  ownership and workspace scope; account or routine deletion removes matching
  history. All 112 mail/calendar unit tests, the durable state-registry test,
  14 focused server tests, the production web build, formatting, and diff
  checks pass. No provider content is duplicated into this index.

- 2026-10-01: Added persisted, backward-compatible source-check timestamps for
  continuous mail watches. Successful provider cursor polls update the field;
  failures leave the previous successful-check time intact. The routine UI now
  shows that timestamp separately from run status and explicitly labels a
  watch with no successful poll. The focused status test, task-store
  compatibility/round-trip suite, formatting, server library check, and web
  build pass. Awake-host health, freshness targets, and 24-hour service
  acceptance remain open.
- 2026-10-01: New mail/calendar routines are saved paused so scheduled or
  continuous work cannot begin before the owner reviews a sample. The owner
  can run the read-only routine once, inspect its Agent run history, then
  Resume it. Generic scheduled tasks keep their prior active-on-create
  behavior. The server creation-default test and web build verify this
  increment; broader preview/browser acceptance remains open.
- 2026-10-01: Continuous watches now surface a stale-check warning after three
  minutes without a successful provider poll, updating while the Settings page
  stays open. Cron-scheduled watches keep their own cadence and are not marked
  overdue by this continuous-mode threshold. Client status tests and the web
  build verify the distinction; this does not establish awake-host health.
- 2026-10-01: Added explicit accessible names to the Google, Microsoft, and
  Apple app-password email and password fields after the running preview
  exposed anonymous text fields in its accessibility tree. The UI typecheck
  and web bundle build pass. The existing preview tab was left untouched so
  any in-progress owner credential entry is not cleared; a separate isolated
  profile will be used to verify the rebuilt UI.
- 2026-10-01: Re-ran the provider and security-boundary test suites from the
  feature worktree after the isolated local-draft browser fix. All 111
  `vak-mail-calendar` unit tests and its state-registry test pass; all 11 Core
  mail/calendar boundary tests, 13 server mail/calendar unit tests, 4 matching
  HTTP end-to-end tests, 10 worker parser tests, and 5 isolated-worker
  integration tests pass. `npm run build:web` passes from
  `crates/vak-client-ui` and regenerates the committed bundle without a worktree
  diff. These local fixtures do not establish live account/provider behavior,
  the full connected source-to-Review flow, or 24-hour scheduled-service
  recovery; those Stage 2, 4, and 5 acceptance checks remain open.
- 2026-09-30: Added the isolated worker-side MIME parsing foundation needed
  for selected Apple Mail content. It extracts bounded explicit `text/plain`
  content, ignores HTML and attachment parts, and distinguishes HTML-only
  messages from usable text. This is now wired to a fixed-host IMAP `UID FETCH`
  using `BODY.PEEK[]`; the source UIDVALIDITY must match, and each message is
  capped at 128 KiB within the 512 KiB transport budget. The owner preview and
  Agent read tool can request one selected message, and the UI labels its
  content untrusted. Local protocol fixtures verify the `BODY.PEEK[]` request
  and stale UIDVALIDITY rejection before fetch; the implementation enforces the
  128 KiB message cap. The UID parser, real-worker MIME path,
  owner-only/unverified HTTP boundary, server check, and web build pass; no live
  Apple message fetch has been performed.

- 2026-09-30: Implemented a desktop-native OAuth return path for the Tauri
  bearer-authenticated UI. The owner-authenticated start creates a bounded,
  single-use 256-bit state and PKCE verifier; the system-browser callback must
  hit the loopback listener and match that state, with no webview cookie
  fallback or relaxed web-session checks. Microsoft uses its localhost
  loopback redirect form; Google retains the loopback IP form. The desktop
  router now passes its actual ephemeral listener port into OAuth URL
  construction. Domain suite passes (21 tests), and server callback-cookie/
  redirect tests pass (4 tests). The desktop crate test also builds now that
  its configured frontendDist has been generated.
- 2026-09-30: Completed the desktop side of that handoff: Settings opens the
  authorization URL in the system browser, and the Tauri command validates
  the HTTPS provider hostname and exact authorization path before dispatching
  to the OS. Added rejection tests for spoofed hosts, HTTP, and unrelated
  paths. `npm run typecheck`, both UI bundle builds, and the desktop
  authorization-URL test passed.
- 2026-09-30: Added an HTTP end-to-end test for native Google and Microsoft
  OAuth initiation. It verifies owner-bearer admission, rejects unauthenticated
  initiation, checks provider-specific ephemeral loopback redirects and
  configured public client IDs, and asserts PKCE S256 plus 256-bit state.
- 2026-09-30: Extended that HTTP test to bypass the Settings capability picker
  and verify the server rejects `mail_prepare`, `mail_send`, and
  `calendar_write` for both OAuth providers during Stage 1.
- 2026-09-30: Added the same HTTP-level read-only capability check for iCloud's
  broader app-specific-password credential. The API rejects `mail_send` even
  when the caller bypasses the UI; the existing credential-storage and
  disconnect lifecycle test still passes.
- 2026-09-30: Hardened callback responses with `Cache-Control: no-store` and
  `Pragma: no-cache`, alongside the existing global no-referrer header. The
  Google/Microsoft native OAuth HTTP test now follows the callback error path
  and verifies these headers without redeeming or storing provider data.
- 2026-09-30: Callback result pages now clear the OAuth query from browser
  history before presenting fixed status text; only a successful connection
  page requests window close. Unit and HTTP tests confirm the state value is
  absent from returned HTML and the no-store protections remain active.
- 2026-09-30: Disconnect now serializes with new account linking at the
  Agent/provider boundary as well as with per-account refresh. This closes
  the interval where a newly started flow could be created after pending
  OAuth attempts were cancelled but before the disconnect tombstone landed.
  Apple credential enrollment uses the same provider lock. Server library
  check, account metadata/OAuth HTTP tests, and the in-flight OAuth fence
  regression pass.
- 2026-09-30: Bounded the in-memory disconnect-generation table used by
  callbacks. Eviction makes an old callback stale, including at the atomic
  final-persist check, so account churn cannot grow the always-on OAuth state
  without limit or turn cleanup into authorization. Added a regression that
  fills the table past its cap and proves the evicted grant cannot commit.
- 2026-09-30: Updated the HTTP test server fixture to include the loopback
  peer address, as the production server does. This lets `/health` return its
  authenticated/local operational projection in the lifecycle test. The full
  server HTTP end-to-end target now passes (16 tests).

- 2026-09-30: Corrected the embedded desktop router to use the actual
  ephemeral listener port when constructing operational URLs. The OAuth
  review identified that Tauri bearer auth cannot establish a webview cookie
  for the system browser; desktop now uses a separate native PKCE/state
  callback path, while the same-origin web flow retains its cookie and session
  checks.

- 2026-09-29: implementation goal opened by the owner; created managed
  worktree and branch `codex/mail-calendar`. Active release build remains in
  the primary checkout.
- 2026-09-29: provider feasibility checked against current Google, Microsoft,
  and Apple documentation. The Apple adapter needs separate treatment because
  iCloud's user-supported client path relies on an app-specific password.
- 2026-09-30: Rechecked Apple's published third-party and iCloud Mail setup
  guidance. IMAP read is documented, while a complete unattended Mail and
  Calendar contract is not. Apple enrollment remains explicitly unverified;
  previews require fixed-host adapter verification and worker-isolated parsing
  of untrusted MIME and calendar payloads. Do not promote enrollment to
  provider support based on a credential save alone.
- 2026-09-29: Stage 0 recorded the current-storage constraint without starting
  data-architecture M1. Added the first provider-neutral typed contract in
  `crates/vak-mail-calendar`; no provider requests or credentials are exposed
  by this domain crate.
- 2026-09-29: Added an explicit Agent/account lineage and vault-reference
  contract. Content reads are held behind the M7 erasure gate; disconnect
  requires secret deletion and watcher fencing, while account deletion also
  removes all catalog-linked local copies and derived records.
- 2026-09-30: The data-architecture proposal now names connected provider
  accounts as an erasure scope, including opaque lineage ids, selective
  account-key/grant composition, cross-Agent/conversation catalog traversal,
  restore protection, and a provider-account erasure acceptance suite. No
  architecture implementation milestone was started.
- 2026-09-30: Added `AccountVault`, which stores provider credentials and
  account identity only through `vak_config::credentials` in the owning
  Agent's credential scope, zeroizes in-memory material on drop, and derives
  vault keys only from UUIDv7 account ids. The connection ledger remains
  reference-only; provider access is still disabled pending erasure support.
- 2026-09-30: Added bounded single-use Google/Microsoft authorization state
  with PKCE S256, OIDC nonce, fixed authorization hosts, short expiry, and
  explicit Agent/audience/capability and initiating-session binding. Added
  fixed-host token redemption, bounded responses, actual-scope checks, and
  RS256 OIDC signature/issuer/audience/expiry/nonce validation before storing
  the identity and tokens in the Agent vault. Initial redirects are loopback
  only; confidential web callbacks and provider revocation remain unimplemented.
  Apple stays on the separate app-password path. A connection records whether
  a refresh token exists; unattended routines must refuse admission without it.
- 2026-09-30: Focused OAuth tests passed for session binding, one-time callback
  consumption, PKCE challenge/verifier agreement, loopback redirect rejection,
  per-Agent pending-flow bounds, and least-capability scope mapping.
- 2026-09-30: Added owner-only server endpoints for safe account metadata,
  loopback Google/Microsoft OAuth initiation and callback, and local
  disconnect. The callback is single-use and browser-cookie flows retain the
  initiating-session binding. The connection Settings page is Agent-scoped,
  offers least-capability selection, and does not expose principal identity or
  vault references. Disconnect fences local use before attempting Google's
  fixed-host token revocation, and reports revocation separately from content
  erasure. Apple setup, Microsoft provider-side revocation, external hosted
  callbacks, mail/calendar content access, the
  working area, and routines are still incomplete. Frontend typecheck and the
  web bundle build passed. Added symlink checks at the Agent data boundary;
  OAuth unit tests (4), the owner/Agent HTTP boundary test (1), the server
  check, test-target compilation, formatting, and `git diff --check` passed.
- 2026-09-30: Tightened the shared candidate boundary in the domain crate:
  scope and source metadata are bounded, candidate revisions cannot saturate,
  and approvals match one exact account/Agent/audience/payload revision,
  require single use, reject future approval times, and expire within five
  minutes. A pure broker preflight helper maps each action to its capability
  and requires both current account/audience authority and exact fresh review;
  host permission-engine evaluation remains an additional required gate. Eleven
  focused unit tests cover these contract checks and account admission.
- 2026-09-30: Added fixed-host OAuth access-token refresh for Google and
  Microsoft. Returned scope sets are checked against the recorded grant,
  rotated refresh tokens remain inside the owning Agent vault, and updated
  expiry metadata is appended to the Agent connection ledger. Added an
  authenticated refresh route and Settings action. Disconnect first appends a
  revocation tombstone, then best-effort revokes Google's token before local
  vault deletion; Microsoft and content-erasure outcomes remain explicit. A
  refresh-scope test proves that dropping any previously granted data scope
  refuses refresh.
- 2026-09-30: OAuth refresh and owner/Agent boundary verification passed:
  `cargo fmt --all -- --check`, `git diff --check`, all 11
  `vak-mail-calendar` tests, the focused account metadata/refresh HTTP boundary
  test, and `cargo check --locked -p vak-server`. Secret material is also
  zeroized on constructor validation and size-limit failures. The broader
  provider adapters, review/working area, content-erasure implementation, and
  scheduled/continuous routines remain outstanding.
- 2026-09-30: Added local-only Apple iCloud app-specific-password enrollment.
  It accepts only read capabilities, stores identity and password in the
  Agent vault, and leaves provider scopes empty and token expiry absent. The
  Settings UI discloses the password's broader authority and Apple-side
  revocation path. Enrollment does not yet verify the password with Apple or
  enable content access. The HTTP integration check proves authentication and
  confirms the response and account listing disclose neither email nor
  password.
- 2026-09-30: Verification passed after Apple enrollment: web TypeScript check
  and production bundle build, `cargo fmt --all -- --check`, `git diff --check`,
  11 `vak-mail-calendar` unit tests, the iCloud email validation unit test,
  the account/Agent HTTP boundary plus enrollment integration test, and
  `cargo check --locked -p vak-server`. The enrollment check also confirms a
  non-loopback Host is rejected before credential storage.
- 2026-09-30: Hardened the iCloud credential path further: app-password request
  bytes are zeroized when uniquely owned, vault payload clones zeroize on drop,
  and iCloud addresses use bounded ASCII/label validation. The UI now labels
  Apple credentials as saved but unverified and does not offer OAuth refresh
  for them. Formatting, diff checks, the credential validation unit test, and
  the HTTP enrollment/access-boundary test passed after these changes.
- 2026-09-30: Extended the HTTP account lifecycle test through disconnect. It
  verifies the local vault secret is no longer loadable, the ledger keeps a
  revocation tombstone, and the response explicitly says the Apple grant was
  not remotely revoked and provider content was not erased.
- 2026-09-30: Account-list testing uncovered a path-stability defect: the
  credential scope was hashed before the Agent home existed, then the ledger
  created the directory and changed the canonical scope hash. `AccountVault`
  now validates and creates private Agent directories before deriving its
  canonical credential scope. The owner-only account view exposes only a
  masked identity from the vault; OAuth stable subjects and full identities
  remain private. The end-to-end test now proves masked identity survives
  linking and that disconnect removes the local secret while preserving the
  tombstone. All 13 domain tests, focused HTTP lifecycle test, server check,
  formatting, and diff checks passed.
- 2026-09-30: The vault now also tightens existing `agents/` and Agent-home
  directories to owner-only permissions and rejects symlink paths. Added a
  Unix regression test for both behaviors. The expanded 14-test domain suite,
  masked-identity link/list/disconnect HTTP test, server check, formatting,
  and diff checks passed.
- 2026-09-30: Added provider-specific OAuth setup instructions to the
  implementation plan and Agent Settings. Google requires a Desktop OAuth
  client ID; Microsoft requires an Entra public-client ID. Both use the local
  operations-port loopback callback, with no client secret or hosted callback.
  The UI discloses same-device requirements and Google's restricted Gmail
  scope review gate. The web TypeScript/build check, 14 domain tests, focused
  owner/Agent HTTP lifecycle test, formatting, and diff checks passed.
- 2026-09-30: Closed a least-privilege gap at the OAuth domain boundary:
  Stage 1 now rejects `mail.send`, `calendar.write`, and `mail.prepare` for
  both Google and Microsoft, independent of the Settings UI. All 15 domain
  tests pass, including direct rejection tests for both provider families.
- 2026-09-30: Made account disconnect cleanup retryable after the ledger
  tombstone. An interrupted cleanup can now remove any leftover local
  credential on retry, while keeping provider-grant revocation unconfirmed and
  content erasure distinct. Settings exposes a “Finish cleanup” action for
  tombstoned accounts. The HTTP integration test simulates the crash window
  and proves the retry removes the credential; the web build, 15 domain tests,
  focused HTTP lifecycle test, formatting, and diff checks passed.
- 2026-09-30: Clarified Settings copy that disconnect removes saved sign-in
  details, provider-side messages/events remain with the provider, and
  Vakyartha's saved copies require the separate lifecycle deletion path. The
  refreshed TypeScript and production web build passed.
- 2026-09-30: Hardened connection-ledger directory creation to inspect the
  target and Agent path for symlinks before creating the ledger directory.
  Added a regression test proving a symlink target outside the Agent home is
  not created. All 16 `vak-mail-calendar` tests pass.
- 2026-09-30: Preserved the application's `SameSite=Strict` session cookie
  while making loopback OAuth callbacks work after a provider redirect. Each
  authenticated begin now binds state to a separate 10-minute, HttpOnly,
  `/mail-calendar`-scoped `SameSite=Lax` cookie; pending flows in the same
  browser session can share the binding. Missing, duplicate, malformed, mismatched,
  or replayed bindings fail closed. Domain tests cover binding reuse and
  session separation, and server tests cover callback-cookie parsing and
  attributes.
- 2026-09-30: Follow-up verification caught and fixed the Strict-cookie return
  case: the OAuth state consumer now allows the short-lived callback binding
  when the cross-site redirect omits the normal session cookie, while still
  rejecting a present mismatched session. All 17 domain tests and both server
  callback-cookie unit tests pass.
- 2026-09-30: Kept that callback cookie available to later same-browser OAuth
  starts by scoping it to `/mail-calendar` (the shared path of initiation and
  callback), rather than only the callback route. It remains excluded from
  unrelated application paths; the server cookie-attribute test covers this
  path.
- 2026-09-30: The callback now also revalidates the initiating session after
  consuming state and before token redemption. Session ids held in pending
  flows and grants are zeroized at drop, so logging out or expiring the
  initiating session prevents a callback from completing. The 17 domain tests
  pass with this session check.
- 2026-09-30: Hardened ledger reads to reject broken directory symlinks and
  symlinked ledger files, fail closed if the file changes to a non-file, and
  cap bytes read even if the file grows after its initial metadata check.
  Added regression coverage for broken directory links, file links, and
  oversized ledgers. All 20 domain tests and the focused owner/Agent server
  lifecycle test pass with the hardened reader.
- 2026-09-30: OAuth callback now rechecks that the owning Agent is still
  active after consuming the one-time state and before redeeming provider
  authorization. Pausing or archiving the Agent during the browser handoff
  therefore cannot finish account linking. Focused callback and native OAuth
  end-to-end tests pass.
- 2026-09-30: Added a vault-isolation regression test: the owning Agent can
  resolve its provider secret, while a second Agent cannot resolve it even
  when given the opaque account id. The test removes its credential after
  verification.
- 2026-09-30: Serialized refresh and disconnect requests per Agent/account.
  This closes an in-process race where refresh could rotate and persist
  credentials after disconnect had tombstoned the account and removed its
  vault entry. Agent activity is checked again after acquiring the lock. The
  lock test verifies same-account serialization and separation across Agents
  and accounts; cross-process fencing remains a Stage 4 requirement.
  The server check, account-lock unit test, existing disconnect/retry HTTP
  lifecycle test, formatting, and diff checks pass.
- 2026-09-30: The iCloud app-specific password now clears from reactive UI
  state before the network request begins, and the short-lived local variable
  is cleared after submission or failure. Typechecking and production bundle
  builds pass for both web and Tauri desktop.
- 2026-09-30: Rechecked Apple's current third-party access guidance. Apple now
  documents Apple Account authorization for supported apps, with app-specific
  passwords as the fallback when an app does not support that flow. The plan
  records that distinction and requires checking cross-host support before
  enabling iCloud content access; the implemented path remains the disclosed,
  read-only app-specific-password fallback.
- 2026-09-30: Split Stage 1 into account linking before M7 and content reads
  after M7. This resolves the plan's conflicting exit condition that required
  useful provider views while its own deletion gate prohibited fetching
  provider content. Stage 1A now exits on safe account lifecycle behavior;
  provider-view, parser, and read-revocation checks belong to Stage 1B.
- 2026-09-30: OAuth callback query state, authorization code, and provider
  error strings now zeroize on drop; the code used during token redemption is
  also held in a zeroizing wrapper. Seven callback unit tests and the native
  OAuth HTTP end-to-end test pass after this change.
- 2026-09-30: The mail/calendar account endpoints now carry
  `Cache-Control: no-store` and `Pragma: no-cache`. Initiation contains
  single-use OAuth state; account listing contains masked identity and grant
  metadata. The HTTP tests assert protections on both paths, and all 16 server
  HTTP end-to-end tests, formatting, and diff checks pass.
- 2026-09-30: The append-only connection-event and connected-account schemas
  now reject unknown fields, so unexpected credential-shaped properties fail
  closed instead of being ignored as metadata. Both regression tests, all 24
  domain tests, the account lifecycle HTTP test, formatting, and diff checks
  pass.
- 2026-09-30: Narrowed Microsoft free/busy authorization to
  `Calendars.ReadBasic`, keeping `Calendars.Read` for event-detail reads.
  Microsoft Graph documents the former as the least delegated `getSchedule`
  permission and marks that API unsupported for personal Microsoft accounts;
  the Stage 1B adapter must surface that limit. All 24 package tests and all
  16 server HTTP end-to-end tests pass, along with formatting and diff checks.
- 2026-09-30: Made account revocation sticky for an opaque account UUID. The
  append-only ledger now rejects a later `Connected` event for an ID that has
  already been revoked, even with a higher revision; reconnecting creates a
  fresh identity. Regression coverage and all 25 package tests pass.
- 2026-09-30: Kept owner-only account inventory and disconnect cleanup
  available for registered paused or archived Agents. New links and token
  refresh still require an active Agent. This lets operators remove stored
  credentials after pausing an Agent without granting it any execution access.
  An HTTP end-to-end test connects an account, pauses its Agent, proves list and
  disconnect work, and proves connect and refresh are refused; the credential
  is absent from the Agent vault after cleanup.
- 2026-09-30: Google account linking now disables scope union from prior grants,
  and both initial authorization and refresh reject any returned scopes beyond
  the requested capability set. Tests cover Google mail-read versus mail-modify
  and Microsoft free/busy versus calendar-write. All 26 package tests pass.
- 2026-09-30: OAuth callback now checks both Agent activity and initiating
  browser-session validity again after the provider token exchange and before
  persisting credentials. The grant is retained only long enough for that
  recheck and still zeroizes its verifier and nonce on drop. All seven
  mail/calendar server unit tests and all 26 package tests pass.
- 2026-09-30: Refresh now stages rotated access/refresh tokens in zeroizing
  memory. The server rechecks Agent activity after the provider response and
  only then explicitly persists those tokens. A package regression test proves
  dropping a staged rotation leaves the old vault credentials intact and that
  explicit persistence replaces them; the test passes.
- 2026-09-30: The append-only account ledger now validates stored provider
  scopes against the account's selected read capabilities and refuses any
  non-read capability during Stage 1. Higher-revision connection events may
  update expiry metadata only; Agent, audience, capability, scope, provider,
  credential reference, and provider identity remain immutable. Regression
  tests cover write-scope rejection, over-scoped records, and authority changes;
  all 29 package tests pass.
- 2026-09-30: Connected account rows now show readable capability names
  (`Read email`, `Check availability`, and `Read calendar events`) instead of
  internal enum values. The Settings UI typecheck passes.
- 2026-09-30: Account linking now records a durable `pending` account before
  writing credentials to the Agent vault, then appends a `connected` revision
  only after the vault write succeeds. Interrupted links stay owner-visible
  and cleanable, while pending records cannot authorize provider access.
  Vault or activation failures attempt to revoke the ledger record and remove
  any staged credential. The account-admission regression, all 29 package
  tests, both mail/calendar HTTP end-to-end tests, web build, and UI typecheck
  pass.
- 2026-09-30: Added regression coverage for the pending-link crash boundary:
  pending records deny account admission, cannot be activated without a
  preceding pending event, and can be listed and cleaned by the owner even
  after the Agent is paused. The two mail/calendar HTTP end-to-end tests and
  all 31 package tests pass.
- 2026-09-30: Re-audited the data-architecture gate against the current
  `AGENTS.md` and M0/M7 plan: M0 is still the latest completed data milestone,
  and M7 erasure has not started. Provider-content reads, retained drafts,
  content previews, and routines that consume provider content therefore
  remain deferred; account metadata, vault lifecycle, and owner cleanup are
  still the active pre-M7 implementation surface. Formatting, diff checks,
  UI typechecking, the web bundle build, the account lifecycle HTTP test, and
  all 31 package tests pass on this branch.
- 2026-09-30: Added local Google and Microsoft provider test doubles for the real OAuth
  redemption function. It records and checks the authorization-code form
  (including PKCE verifier, exact loopback redirect, and no client secret),
  returns a token response, serves the OIDC signing-key endpoint, and proves
  malformed signatures fail closed. Production endpoint selection remains
  fixed to provider constants; only the private test seam accepts fixture
  URLs. The test also checks Microsoft's delegated-scope form and both
  production endpoint pairs. Positive Google and Microsoft fixtures sign
  valid OIDC tokens with a test-only RSA key and verify each provider's
  signature, issuer, audience, nonce, scope, and account-persistence path into
  the Agent vault and ledger.
  Both focused tests and all 33 package tests pass.
- 2026-09-30: Added a vault regression proving provider credentials round-trip
  through the shared credential service without creating an Agent `.env`
  plaintext file. All 34 package tests pass.
- 2026-09-30: Added Google and Microsoft refresh-token test doubles. They
  exercise the production refresh request, verify the provider-specific scope
  contract, prove that over-scoped refresh responses are rejected without
  changing stored credentials, and verify valid rotations remain staged until
  the explicit persistence call. All 35 package tests pass.
- 2026-09-30: Addressed production Clippy findings in the mail/calendar
  ledger and OAuth parsing. The production library passes Clippy with warnings
  denied; all 35 package tests, both mail/calendar HTTP tests, formatting, and
  diff checks pass.
- 2026-09-30: Added a Google revocation test double. It confirms the revocation
  request contains only the vaulted refresh token; Microsoft returns
  unsupported without a provider request. Production remains pinned to Google's
  fixed revocation endpoint. All 36 package tests pass.
- 2026-09-30: Extracted OIDC claim validation and added rejection tests for
  wrong audience, nonce, issuer/tenant, expired claims, and future-issued
  claims for Google and Microsoft. The successful signed-token fixtures still
  cover both persistence paths. All 37 package tests pass.
- 2026-09-30: Added an HTTP fixture proving OAuth provider responses larger
  than the 64 KiB bound are rejected before JSON parsing. All 38 package tests
  pass.
- 2026-09-30: Rechecked the account-deletion design across docs 73, 74, 80 and
  the data-architecture plan. Clarified in doc 73 that disconnect removes
  credentials and access, while account erasure selectively removes retained
  content through M7. Provider-account lineage, cross-Agent/conversation
  erasure, backup restore protection, and preservation of unrelated content
  remain explicit M1/M2/M6/M7 requirements. Verification: all 38
  `vak-mail-calendar` tests, both focused server HTTP tests, production library
  Clippy with warnings denied, and `git diff --check` pass.
- 2026-09-30: Closed a local-only setup boundary gap. OAuth and iCloud setup
  now require both a loopback `Host` and an actual loopback socket peer, so a
  remote caller cannot satisfy the restriction by spoofing `Host: 127.0.0.1`.
  Added a regression for that spoof and ran both account HTTP tests through
  Axum's production-style connect-info service. The regression, both HTTP
  tests, formatting, and diff checks pass. A server-wide Clippy run remains
  red on existing warnings in server/OpenXML code (including the pre-existing
  disconnect branch); the mail/calendar library's production Clippy check
  remains clean, and the server library passes `cargo check`.
- 2026-09-30: Provider refresh refusal now appends a `reauthentication_required`
  state to the Agent account ledger. This fences the account from admission and
  survives restart, instead of leaving inventory to display a revoked provider
  grant as connected. Settings labels the stale connection and instructs the
  owner to remove its saved credential before creating a new link. The ledger
  regression, all 39 package tests, both account HTTP tests, web build/typecheck,
  server `cargo check`, formatting, and diff checks pass. The account HTTP
  test additionally seeds a persisted stale Google grant and verifies its
  owner-visible status while asserting that the full identity and refresh
  token stay out of the response; a repeated refresh is refused before any
  provider request.
- 2026-09-30: The refresh endpoint now rejects iCloud app-specific-password
  connections as unsupported for OAuth token refresh instead of mislabeling
  them as needing OAuth reauthentication. HTTP coverage proves that the
  unverified Apple credential remains unchanged while OAuth-backed account
  reauthentication continues to work as designed.
- 2026-09-30: Mail/calendar capability selections now reset to least-privilege
  defaults when the owner switches Agents, preventing one Agent's broader
  selection from silently carrying into another Agent's account-linking flow.
  The embedded web bundle rebuild and TypeScript check pass.
- 2026-09-30: Disconnect now cancels pending OAuth handoffs for the Agent and
  provider and fences callbacks already exchanging a code. The final vault and
  ledger commit runs under the same fence lock as disconnect, so either the
  link commits before disconnect or it is rejected as cancelled. Regression
  coverage proves pending callbacks are removed, in-flight grants go stale,
  and a later owner-started link remains valid. The OAuth regression, both
  mail/calendar HTTP tests, server `cargo check`, formatting, and diff checks
  pass.
- 2026-09-30: The owner account inventory now reports whether each referenced
  credential can be loaded from the Agent vault. Settings distinguishes an
  unavailable saved credential from a healthy connection and offers OAuth
  reconnect where supported; it never returns credential material. HTTP
  coverage verifies the available and missing states and that a missing
  credential does not expose a stale masked identity.
- 2026-09-30: Added direct persistence coverage for a redeemed provider grant
  presented with a different Agent's vault. The commit returns an Agent
  mismatch before writing either vault or connection ledger, covering the
  wrong-principal isolation check in Stage 1A's exit criteria.
- 2026-09-30: OAuth initiation and direct iCloud credential enrollment now
  recheck that the Agent is active after waiting for the per-provider lock.
  This prevents a setup request admitted before a pause/archive from relying
  on that stale lifecycle decision after it resumes from lock contention. A
  regression pauses the Agent while the shared admission helper is blocked
  and verifies it refuses the link after acquiring the lock.
- 2026-09-30: Stage 1A exit criteria are met: 43 package tests, 9 mail/calendar
  server unit tests, and 2 mail/calendar HTTP end-to-end tests pass. Coverage
  includes one-time/replay-safe callbacks, wrong-Agent vault rejection,
  capability and audience isolation, bounded provider responses, exact scope
  and refresh ceilings, credential redaction, vault filesystem boundaries,
  and idempotent local disconnect. Stage 1B remains unstarted until M7.
- 2026-09-30: Refresh now performs its final connected-account revision check,
  vault token rotation, and ledger revision append under the ledger's
  cross-process exclusive lock. A disconnect in another Vakyartha process
  therefore wins before the credential write, or runs afterward and removes
  the rotated credential. A conditional-update regression proves revoked
  accounts skip the vault operation; server library check passes.
- 2026-09-30: The refresh commit also rechecks that its Agent remains active
  inside the conditional ledger lock immediately before writing rotated
  credentials. Pausing or archiving an Agent during provider/network or lock
  wait therefore discards the zeroizing staged tokens.
- 2026-09-30: Added a two-handle concurrency regression proving a disconnect
  blocks behind an in-flight refresh's shared exclusive ledger lock, then
  appends its tombstone after the refreshed revision. The full
  `vak-mail-calendar` suite now passes (44 tests).
- 2026-09-30: Initial OAuth activation now holds the same cross-process ledger
  lock while it writes the Agent vault credential and appends the connected
  revision. A disconnect that wins first prevents that vault write; a link
  that wins first is followed by disconnect cleanup. Regressions cover both
  revoked-pending admission and concurrent link/disconnect ordering. The
  mail/calendar suite passes (46 tests), as do 9 server unit tests, 2 HTTP
  end-to-end tests, crate Clippy, formatting, and diff checks.
- 2026-09-30: Applied the same atomic activation boundary to local iCloud
  app-specific-password enrollment and recorded the shared-lock guarantee in
  doc 80. Added a failure-path regression proving a failed vault write leaves
  the account pending, never connected. The 47 package tests, 9 server unit
  tests, 2 account lifecycle HTTP tests, crate Clippy, formatting, and diff
  checks pass.
- 2026-09-30: Expanded the direct iCloud HTTP test to reject each unsupported
  Stage 1 capability (`mail.prepare`, `mail.send`, and `calendar.write`),
  enforcing the read-only ceiling even when the Settings capability picker is
  bypassed. The test also confirms these rejected requests leave the account
  inventory empty. The account-lifecycle HTTP test passes.
- 2026-09-30: Added zeroizing drop for the iCloud setup request and wrapped its
  sensitive fields in `Zeroizing` as soon as serde decodes each one, so a
  later JSON parse error also clears already-decoded values; the trimmed email
  copy is zeroized too. The iCloud HTTP test covers malformed JSON after the
  password field and confirms the error response does not echo the password.
  Strict server Clippy remains blocked by existing
  warnings in `vak-ooxml` and unrelated server modules.
- 2026-09-30: The Settings UI now hides the iCloud app-specific-password form
  on non-local web hosts, avoiding transmission of Apple's broad credential
  to a hosted server that would reject it. Server-side local-only and
  loopback checks remain authoritative. UI typecheck and production web build
  pass. A browser smoke check loaded the sign-in screen; Settings could not be
  inspected because this local web preview requires an owner passkey.
- 2026-09-30: Settings now offers “Connect again” for OAuth accounts that
  require owner reauthentication or whose saved credential is unavailable.
  The row explains that this creates a new account entry and that the old
  entry should then be removed, matching the append-only connection ledger.
- 2026-09-30: Tightened the local-only OAuth and iCloud setup boundary to
  reject standard forwarded/proxy headers even when a local proxy connects
  over loopback and rewrites `Host`. Unit and authenticated HTTP regressions
  pass; the HTTP check confirms proxy-marked enrollment returns `403` and
  creates no account.
- 2026-09-30: Applied that same direct-loopback boundary to the OAuth return
  endpoint before it consumes authorization state or exchanges a code. This
  protects the callback code from reverse-proxy handling; Microsoft `localhost`
  and Google loopback-IP callbacks remain supported. The server unit guard
  and native OAuth HTTP flow pass, including a forwarded-header rejection
  that verifies a supplied authorization code is not echoed.
- 2026-09-30: Added a package integration regression that resolves the actual
  per-Agent connection-ledger path and checks it against `vak_core::state::REGISTRY`.
  The mail/calendar suite now passes 47 unit tests plus this registry test,
  confirming that the ledger is covered by backup, purge, and upgrade checks.
- 2026-09-30: Account link, refresh, reauthentication-required, and disconnect
  transitions now emit a dedicated `mail_calendar_account` security event with
  only opaque Agent/account IDs, provider, selected capabilities, and outcome.
  The HTTP lifecycle test verifies events omit mailbox identity and credential
  values, and distinguishes an unconfirmed provider revocation from local
  credential cleanup.
- 2026-09-30: Disconnect now distinguishes confirmed, unsupported, unconfirmed,
  and not-retried provider-revocation outcomes in its response and audit entry.
  Settings gives provider-specific guidance for unsupported revocation and
  does not imply that a cleanup retry reattempted provider revocation.
- 2026-09-30: Added a server regression for account churn proving idle
  per-account serialization locks are reclaimed from the shared weak-reference
  table as new accounts are handled. This protects long-lived service memory;
  the focused test and workspace formatting check pass.
- 2026-09-30: Account linking now rejects a second active connection for the
  same provider principal and Agent, preventing repeated links with different
  capability selections from accumulating authority. The ledger performs the
  principal check atomically with pending-link append across processes; pending
  same-provider links reject a second attempt before its secret exists. iCloud account
  email matching is case-insensitive, and the ledger retains no identity
  fingerprint. OAuth domain and owner/Agent HTTP regressions pass.
- 2026-09-30: Kept the recovery path usable with the active-principal guard:
  reauthentication-required links are already fenced from admission, so the
  owner can reconnect that identity and remove the old entry as Settings says.
  The OAuth regression covers active-duplicate rejection, pending-link
  conflict, and reconnect after reauthentication is required.
- 2026-09-30: Account inventory now marks a reauthentication-required row when
  the same vaulted provider principal has a newer active link. Settings then
  directs the owner to remove the old row and hides a reconnect action that
  would be rejected. Missing or nonrenewable credentials direct the owner to
  disconnect before relinking. The HTTP test checks the flag without exposing
  principal data; the Settings typecheck and production web build pass.
- 2026-09-30: Account inventory skips vault reads for disconnected and pending
  entries, and returns neither masked identity nor credential availability
  for those rows. Settings prioritizes pending cleanup guidance. The HTTP
  lifecycle assertions, complete server HTTP target, and all 49 domain tests
  pass.
- 2026-09-30: Rechecked Apple's documented third-party account authorization.
  Apple Support describes Apple Account authorization and revocation for
  supported apps. The developer OAuth flow reviewed applies to the Apple School
  Manager Roster API, and EventKit describes native on-device calendar access;
  neither specifies a server-side iCloud Mail/Calendar client grant for
  unattended routines. Recorded this as a provider gate; Sign in with Apple is
  not a data-access grant. M7 remains the gate for all content reads.
- 2026-09-30: Security audit found that disconnect cancellation was
  process-local while account ledgers are cross-process. An OAuth callback
  started in one server process can therefore outlive a disconnect handled by
  another process; the callback appends its pending row only after redemption,
  so the other process cannot tombstone that attempt. Stage 1A remains
  incomplete until a durable provider authorization fence was checked
  atomically with final vault persistence.
- 2026-09-30: Added an opaque per-provider fence to the append-only Agent
  account ledger's atomic `Disconnected` event. OAuth start captures the
  current fence; disconnect appends its account tombstone and advances that
  fence as one locked ledger transaction. Callback activation compares the
  captured fence while holding the same cross-process lock through vault
  persistence. Regressions use independent ledger handles to prove that a
  disconnect in one invalidates a callback in the other before its
  credential-write closure runs, and that the captured fence follows the
  consumed OAuth grant. All 52 mail/calendar domain tests, the state-registry
  test, and all 16 server HTTP tests pass. The account-admission contract also
  rejects unverified Apple iCloud credentials even when the stored account
  lists a read capability. Content reads, previews, retained copies, and
  routines remain disabled behind the M7 gate.
- 2026-09-30: Apple credential enrollment now persists and returns the explicit
  `connected_unverified` account status instead of `connected`. Settings shows
  that it is not verified or available to Agents. Ledger activation accepts
  this status without making it admissible; disconnect remains available.
- 2026-09-30: Rechecked Apple's published iCloud Mail settings: manual access
  uses IMAP and an app-specific password. Documented that current enrollment
  stores but does not authenticate that credential. A future mail-only login
  probe must not select a mailbox or fetch messages, and cannot verify
  Calendar/CalDAV; Apple remains unavailable to Agents until both services
  have reviewed verification and authorization contracts. All 52 domain tests,
  the state-registry test, and all 16 server HTTP tests pass.
- 2026-09-30: Reduced owner account-inventory secret exposure. It now loads
  each account credential only while rendering that account, and loads a
  replacement credential only while comparing one reauthentication-required
  row, instead of retaining every decrypted account credential together. The
  owner/Agent account lifecycle HTTP test passes. That Clippy run reported no
  warning in the changed inventory code; it also found the pre-existing
  disconnect conditional warning fixed in the next entry. Formatting and diff
  checks pass.
- 2026-09-30: Simplified the disconnect tombstone conditional identified by
  that Clippy audit. The focused account lifecycle test, formatting, and diff
  checks pass. Strict server Clippy remains blocked by four existing
  `collapsible_if` findings in unrelated server modules; none are in the
  mail/calendar module.
- 2026-09-30: Added the native `packages/mail-calendar` plugin package. It
  contributes one inert skill for account setup and honest action availability;
  the manifest declares no tools, MCP servers, hooks, commands, scripts, or
  executable files. The package lifecycle regression inspects and installs it,
  proves the only component is its skill, and confirms install leaves it
  disabled. At that point, content operations were still held behind the then-
  active M7 gate.
- 2026-09-30: The maintainer authorized feature implementation against the
  current 4.x storage model and deferred the data-architecture refactor. The
  plan and doc 80 now state that disconnect removes credentials and fences
  future reads but cannot erase content already written to append-only Agent
  sessions; the UI must disclose this before Agent content access is enabled.
  Added bounded, fixed-host, read-only Gmail and Microsoft adapters and
  owner-only Settings previews for inbox, event-range, and free/busy data.
  Provider reads are serialized with account disconnect, require the selected
  account capability/audience, cap time ranges and payload sizes, and write
  content-free audit outcomes. Apple remains unavailable pending credential
  verification. The preview clears when switching Agents/pages and is not
  persisted into a separate cache. Verification: 56 mail/calendar tests
  (including the local Gmail HTTP double) and 16 server HTTP tests pass;
  web build/typecheck passes. Agent/model tools, drafts, Review, and routines
  remain unimplemented. Provider contracts were checked against the Google
  and Microsoft API references linked above.
- 2026-09-30: Added `mail_calendar` as a Core-owned read tool. It binds the
  admitted session's Agent/audience, reads only that Agent's connection ledger
  and vault, and calls bounded Google/Microsoft adapters without exposing
  credentials to a worker. Agent-wide grants are accepted only for `local`;
  channel audiences fail closed until an explicit share flow exists. The tool
  returns provider data as untrusted content, and a post-read ledger check
  discards content if the account or grant changed during the request. The UI
  now discloses that model-visible data enters append-only session history.
  Permission tests prove it is a read in ReadOnly mode and explicit deny wins.
  Verification: 2 Core boundary tests, 2 permission unit tests, 40 permission
  integration tests, 59 mail-calendar tests, server check, and desktop/web UI
  build pass. Strict Core Clippy passes with the repository's pre-existing
  `collapsible_if` finding allowed; no new lint was reported. Working area,
  drafts, Review, effects, and routines remain open.
- 2026-09-30: Added the first Stage 2 working-area increment: owner Settings
  can create, reopen, revise, and delete email/event drafts. The broker stores
  at most 32 candidates and 2 MiB per Agent in its encrypted credential vault,
  validates Agent/account/source lineage, and uses revision compare-and-swap.
  Account disconnect removes unsent candidates for that account. This remains
  local-only: exact-effect preview, Review choices, full source editing,
  provider effects, and routines are still open. Verification: 60
  mail-calendar tests, the candidate lifecycle HTTP test, `cargo fmt --check`,
  and the web build pass.
- 2026-10-01: Added unassigned Agent-vault drafts for new email and standalone
  calendar events. These may be incomplete, remain source-free, and are denied
  at effect authorization. Assigning creates a separate provider-bound Review
  candidate. The 111 mail-calendar tests, Core and server boundary tests, web
  build, format check, and diff check pass; the isolated preview and signed-in
  browser acceptance remain open.
- 2026-10-01: Exercised the no-account draft UI in the isolated home-folder
  test profile with a clearly synthetic event: local preview, save, revision,
  close, and reopen. Fixed a stale autosave timer that could create a second
  revision after manual Save; a browser wait past 900 ms confirmed the
  revision remained single. Responsive provider-credential cards were
  reviewed at 1440×900 and 390×844 in light and dark, and warning text and
  fields now stack correctly on phones. The preview contains only test data
  and no provider account; full source-to-Review screenshots and provider
  account acceptance remain open.
- 2026-09-30: Added a live local-field preview to the draft editor. It shows
  the current email recipients/subject/body or calendar title/local times/
  location/description and source references, and states that no provider
  action occurs. `npm run build:web` and TypeScript checking pass. Browser
  inspection reached the running app's passkey sign-in gate, so the signed-in
  working-area interaction and responsive layout are not visually verified.
- 2026-09-30: Added the first Stage 3 effect boundary increment: fixed-host
  Gmail and Microsoft Graph plain-text send adapters with no proxy override,
  redirects, or retries; bounded provider response parsing; exact
  `mail_send` account admission; and conservative unknown-outcome handling.
  The Agent credential vault now supports a bounded encrypted action journal
  that durably claims a candidate before dispatch, rejects duplicate claims,
  and removes receipts when the account disconnects. Unit tests cover exact
  Google MIME fields, Graph payload acceptance, ambiguous server failures,
  durable single-use claims, and disconnect cleanup. This is only adapter and
  journal groundwork: no route, effect-aware Review UI, permission-engine
  wiring, calendar writes, or provider conformance has shipped yet.
- 2026-09-30: Added the first reviewed calendar effect for Google and
  Microsoft. The owner-only endpoint binds the exact saved event candidate,
  account capability, Agent permission decision, and durable single-use claim.
  The provider adapters create only timed standalone events; they send UTC
  instants, include no attendees, and disable default reminders. Calendar-write
  consent is broader than the create-only broker contract and is disclosed in
  Settings and the design doc. Unit tests cover provider payload semantics,
  unsupported attendee/recurrence/all-day candidates, OAuth scopes, ledger
  grant validation, permission asks/denies, and response classification.
  `npm run build:web`, 69 mail-calendar tests, 4 permission tests, and
  `cargo check --locked -p vak-server` pass. Full server integration tests are
  still compiling; no live provider test or signed-in browser review has run.
- 2026-09-30: Added a verified Apple Mail-only path. Selecting MailRead alone
  now verifies the app-specific password against fixed-host TLS IMAP and a
  read-only INBOX `EXAMINE` before the credential is committed. Recent reads
  fetch only bounded envelopes and body structures (20 items, 512 KiB per
  session); routine preflight fetches only UIDs. A local protocol fixture
  covers the IMAP response mapping, and a duplex-stream test proves the read
  and write budgets stop excess bytes. Apple Calendar selections remain
  `connected_unverified`; Apple mail bodies and provider effects are not
  available. Verification: 74 mail-calendar tests pass, including the state
  registry test; `cargo check -p vak-server` and formatting pass. End-to-end
  behavior has not been tested against Apple's live IMAP service.
- 2026-09-30: Added a fixed-host Apple CalDAV credential probe using TLS,
  Basic authentication, no proxy, and redirects disabled. It accepts only a
  `207 Multi-Status` response and maps authentication rejection separately.
  The probe is not used to activate calendar capabilities; CalDAV discovery,
  bounded event reads, and worker-side iCalendar parsing are still required.
  A local HTTP fixture verifies authorization is sent to the selected endpoint
  and a redirect is rejected. `cargo test -p vak-mail-calendar` passes (75
  unit tests plus the state-registry test). No credentialed live Apple request
  was made.
- 2026-09-30: Added the worker-only CalDAV/iCalendar parsing path. It accepts
  bounded standalone VCALENDAR data or extracts calendar-data from a bounded
  CalDAV multistatus response, applies a 64-level XML depth limit, caps calendar
  and event counts, rejects unsupported time zones and malformed event bounds,
  and suppresses private event details. The broker passes input over bounded
  IPC to an isolated worker with a private empty scratch directory and no
  network access on supported platforms; unsupported platforms fail closed.
  A server integration test exercises the real worker executable and checks
  missing-worker and oversized-input refusals. Apple Calendar is still not
  connected to this parser and remains unavailable. Verification:
  `cargo test -p vak-tools mail_calendar --lib` and
  `cargo test -p vak-server --test mail_calendar_worker` pass; server check
  passes.
- 2026-09-30: Added a local time-window overlap check after provider parsing.
  The broker drops events outside the requested interval even if a provider
  ignores its range filter, while retaining timed events that overlap the
  boundary and all-day events whose exclusive date range overlaps. This runs
  for Google, Microsoft, and Apple results. The focused Core regression passes.
- 2026-09-30: Connected Apple CalendarRead to the verified local read path.
  The owner preview and broker-owned Agent read now discover the CalDAV
  principal, home set and collections through authenticated fixed-origin
  requests, reject cross-origin hrefs, issue range-bounded calendar-query
  REPORTs, and send each response to the network-denied worker for parsing.
  CalendarRead-only Apple links can activate after a fixed-host authenticated
  CalDAV probe; mixed selections and CalendarFreeBusy remain unverified. The
  Settings copy describes this single-capability boundary. Verification:
  package tests (76 plus registry), worker integration tests, the focused
  account lifecycle HTTP test, server check, and web build pass. No credentialed
  live Apple request was made.
- 2026-09-30: Hardened scheduled mail-watch delivery for local restarts. The
  encrypted Agent vault now queues up to 100 opaque message IDs per routine,
  fetches explicit bounded batches without downloading content during polling,
  stages fetched IDs until the TaskDef run settles, consumes them after a
  completed run, and puts them back at the front after a failed or interrupted
  run. A per-routine OS lease also serializes the watcher across local server
  processes; deleting a routine clears its cursor while holding that lease.
  Google, Microsoft Graph, and Apple IMAP now fetch only selected IDs, and
  Apple uses UID FETCH metadata without setting Seen or fetching bodies.
  Limits are explicit: polling scans only the latest 100 IDs, so a larger
  arrival burst between checks can still hide older mail; provider-native
  cursor reconciliation and 24-hour restart/sleep/outage acceptance remain
  open. Verification: 81 mail-calendar tests, 5 Core mail/calendar tests, 4
  worker integration tests, 3 mail/calendar HTTP tests, 4 scheduled-run tests,
  formatting and diff checks pass. Live provider and signed-in browser checks
  remain open.
- 2026-09-30: Added a continuous email-watch option alongside scheduled checks.
  Continuous mode uses the existing TaskDef interval scheduler at one-minute
  intervals, while scheduled mode retains the selected cron expression. Both
  use the same owner/account-scoped read-only broker and bounded encrypted
  backlog. Settings explains that a sleeping local host is offline; native
  push, awake-host health, and 24-hour outage/restart acceptance remain open.
  `npm run build:web` and TypeScript checking pass.
- 2026-09-30: Enforced routine `max_items` as a per-run ceiling across repeated
  and concurrent broker calls, reserving capacity before provider reads and
  releasing unused capacity after results or failures. Core regression tests
  verify reservations cannot exceed the configured ceiling and cancelled work
  does not strand capacity.
- 2026-09-30: Added bounded Apple IMAP watch pagination using UIDVALIDITY and
  UIDNEXT. Each page searches at most 100 UID values, persists the resulting
  cursor atomically with queued IDs in the encrypted routine vault, and drains
  later pages after the current backlog is handled. A UIDVALIDITY change fails
  visibly instead of comparing unrelated mailbox IDs; the owner must recreate
  that routine to establish a new baseline. Local protocol, cursor, vault
  restart, worker, HTTP, formatting, and diff checks pass. Gmail and Microsoft
  remained on the latest-100 scan at that checkpoint.
- 2026-09-30: Added Gmail history-based watcher pagination. Initial admission
  captures Gmail's mailbox history ID and a bounded inbox snapshot; subsequent
  polls follow opaque `nextPageToken` values and atomically advance the encrypted
  cursor with the IDs from each page. Only message additions and inbox-label
  additions enter the queue. An expired Gmail history ID or an overfull history
  page fails visibly rather than advancing past unqueued mail. Test doubles
  verify inbox filtering, page continuation, cursor advancement, expired-token
  handling, and bearer-token scope. Microsoft Graph delta pagination follows
  the bounded per-folder API continuation URLs; continuation origins are
  validated against the configured Graph origin before any bearer-auth request.
  Live provider verification remains open.
- 2026-09-30: Updated provider and routine status after all three mail-watch
  cursors landed. A manually served, isolated worktree build rendered the
  email/calendar settings in a browser at 1440 × 900 and 390 × 844; its console
  had no warnings or errors. This smoke check did not connect a provider or
  verify the complete source-to-Review flow. Routine host-health, 24-hour
  recovery, and live-provider acceptance remain open.
- 2026-09-30: Exercised calendar range preview with a local browser fixture.
  Changing the dates and refreshing sent the matching UTC instant bounds for
  the selected Asia/Calcutta dates, and the updated dates and refresh time were
  displayed. No provider was connected; this verifies UI wiring, not provider
  calendar behavior. Corrected the routine explanation to describe all three
  native cursors and visible expiry recovery rather than Microsoft's retired
  latest-100 scan.
- 2026-09-30: Added explicit owner-submitted phrase search scoped to the selected
  Inbox for Google, Microsoft, and Apple IMAP. Requests are bounded to 20 items,
  query text is length/control-character validated, and the phrase is not written
  to audit events; results remain transient previews. TypeScript, UI build, and
  compile checks pass; provider request tests and browser fixture verification
  remain to run.
- 2026-09-30: Added previous/next seven-day controls to calendar and availability
  previews. Navigation preserves the selected range length, updates the date
  fields, and refreshes the same account immediately using local calendar-day
  arithmetic so daylight-saving transitions do not shift the chosen dates.
- 2026-09-30: Calendar previews now flag overlapping timed and all-day events
  without exposing additional private-event details. All-day bounds use an
  exclusive end date; invalid or missing time bounds are not treated as
  conflicts. Node regression tests cover timed overlaps, adjacent intervals,
  exclusive all-day ends, and malformed bounds. Agenda/day/week workspace and
  cross-source conflict detection remain open.
- 2026-09-30: Added selected attachment previews for Google and Microsoft. The
  inbox shows provider metadata, but the user explicitly selects each file;
  server rechecks membership under the message, bounds it to 1 MiB, and invokes
  the document reader only in the network-denied worker. The response contains
  at most 32 KiB of extracted text, never file bytes. Provider URL-segment and
  message-membership logic is covered by tests; worker extraction/refusal tests
  pass. UI TypeScript and production web build pass. Apple attachments, image
  rendering, attachment composition, and source-to-Review browser acceptance
  remain open.
- 2026-09-30: Added owner-only folder/label listing and selection to mail
  previews. Gmail uses validated label IDs; Microsoft uses validated opaque
  top-level folder IDs in encoded Graph path segments; Apple remains Inbox-only.
  Selected-folder searches stay provider-scoped. Agent and scheduled reads now
  use a verified selected folder; continuous watches remain Inbox-only. Google and Microsoft request fixtures, owner
  HTTP boundary coverage, UI typecheck, and production web build pass. Child
  folder traversal, folder pagination, and the full thread workspace remain open.
- 2026-09-30: Added bounded owner-only conversation previews for Google and
  Microsoft. Each preview returns at most 20 messages and 256 KiB, validates
  that every returned message belongs to the requested conversation, and
  rejects unauthenticated or non-owner requests. Message content stays in the
  transient owner preview and is labeled untrusted; it is not copied to Agent
  history. Apple remains selected-message only. UI typecheck and production
  build pass; all 93 mail/calendar crate tests and the server mail/calendar
  account, candidate, OAuth, and thread-preview HTTP tests pass.
  Reply-in-thread semantics, citations, and the complete thread workspace
  remain open.
- 2026-09-30: Exact-effect confirmation now displays the complete saved email
  recipient list and body inside the confirmation surface. Calendar create and
  update confirmations likewise show the complete description and affected
  fields in the review sheet instead of asking the person to inspect a preview
  hidden behind it. The content is rendered as escaped text with a bounded,
  scrollable review area. UI typecheck and production build pass; visual browser
  acceptance at desktop and phone sizes remains open.
- 2026-09-30: Each message in a Google or Microsoft conversation preview can
  start a local draft that retains that provider message as its source
  reference. The UI explicitly describes it as a new email, not a threaded
  reply; recipient, reply headers, and thread-aware provider send remain
  unsupported. Design and implementation docs record this boundary. UI
  typecheck and production build pass; source-to-Review browser acceptance
  remains open.
- 2026-09-30: Conversation previews now page in batches of at most 20. Gmail
  continuation offsets are bounded and tied to the requested thread. Microsoft
  continuation URLs are restricted to the fixed Graph origin, exact messages
  path, unchanged conversation filter, selected fields, and page size; returned
  messages are still checked for membership. The UI offers explicit “Load more”
  and keeps each page transient. Cursor-tampering tests, two-page provider
  fixtures, all 94 mail/calendar tests, server owner-authentication HTTP test,
  UI typecheck, and production web build pass. Gmail now fetches a bounded
  metadata-only thread snapshot and full content only for the selected page's
  message IDs, each with per-message and aggregate byte caps; the full thread
  workspace remains open.
- 2026-09-30: Added reviewed Google cancellation for one public, standalone,
  timed event without attendees when the connected owner is its organizer.
  The broker rechecks the source and conditionally deletes with the saved ETag;
  series, occurrence, guest, private, and all-day cancellations are excluded.
  The regression tests verify the exact `If-Match` header and `sendUpdates=none`
  request, source eligibility and stale-version behavior. The mail/calendar
  crate suite (101 tests), server check, permission test, and UI typecheck pass.
- 2026-09-30: Agent `recent_mail` can now read one selected folder or label
  after validating membership in the connected account\x27s bounded folder
  inventory. Scheduled routines persist that selection and cannot widen it at
  run time; creation and each read revalidate the folder. Continuous watches
  remain Inbox-only, matching their provider cursor contract. The mail/calendar
  crate suite (101 tests), 11 Core mail/calendar tests, server check, UI
  typecheck, and production web build pass.
- 2026-09-30: Routine creation now rechecks the linked account revision,
  capabilities, and vault credential after provider folder discovery, so a
  disconnect or scope reduction during that round-trip cannot leave a newly
  created routine bound to stale authorization.
- 2026-09-30: Added local Google App Password sign-in as a Gmail IMAP
  MailRead-only alternative. The connection is verified at the fixed TLS
  endpoint, stored in the owning Agent vault, excluded from public/hosted
  setup, and cannot request Calendar or send access. It supports bounded
  Inbox metadata, selected-message MIME parsing through the isolated worker,
  and the existing bounded UID watch. OAuth conversation operations are
  rejected for this credential type. Password-format and provider-host tests,
  the mail/calendar crate suite (104 tests), focused server validation and
  account-lifecycle HTTP tests, UI typecheck, and production web build pass.
  No live provider sign-in was made; owner-provided credentials should be
  entered only in the local Settings form.
- 2026-09-30: Made all currently supported provider sign-in choices explicit in
  Settings and the design matrix. Google OAuth is recommended with a clearly
  limited, less-secure Gmail App Password fallback; Microsoft is OAuth-only,
  with Outlook.com/Live legacy password guidance documented as an unstable
  path that is not offered; iCloud warns that its app-specific password has
  broader provider authority. Apple documents account authorization for
  supported third-party apps, but its Mail/Calendar grant is not yet verified
  for Vakyartha and remains a follow-up. UI typecheck and production web build
  pass; no provider credentials were used.
- 2026-09-30: Added an optional Microsoft personal-account app-password route
  for Outlook.com/Live/Hotmail/MSN. It is restricted to those address domains,
  verified against fixed-host TLS IMAP before storage, and grants read-only
  email access only. Microsoft 365/work Exchange stay OAuth-only; the UI warns
  that consumer app-password IMAP may be rejected as legacy auth is retired.
  Synthetic validation and the complete live credential path still require
  testing with a user-provided local account; no password was collected here.
- 2026-09-30: Added Apple Calendar availability as a separate
  `CalendarFreeBusy`-only access selection. The adapter uses CalDAV
  `free-busy-query`, handles its bounded 200 response, and passes the raw
  VFREEBUSY document to the network-denied worker, which emits only UTC busy
  intervals. Discovery remains fixed-origin and each request rechecks the
  exact account capability. Synthetic parser tests cover a full 100-period
  agenda, overflow, free periods, malformed values and size limits; provider response tests cover
  the CalDAV success status; the account-selection test rejects mixed grants.
  No live iCloud request was made. Worker, provider, Core and server checks and
  the web build pass; live provider conformance remains open.
- 2026-10-01: Mail previews and conversation pages now include bounded sender,
  To, and Cc fields for Gmail, Microsoft Graph, and Apple IMAP. Microsoft
  requests omit Bcc, and the typed result does not expose Bcc to either the
  owner interface or Agent thread output. The UI labels recipient fields
  separately from message content. Provider parser fixtures verify projection,
  control-character cleanup, and Bcc omission; Apple IMAP and Microsoft Graph
  integration fixtures verify the adapter paths. Live provider conformance and
  the broader source-to-Review browser acceptance remain open.
- 2026-10-01: Added large synthetic provider-data cases: Gmail returns 5,000
  listed messages with oversized selected bodies; Google Calendar returns
  1,000 events; Microsoft Graph returns 1,000 messages and 1,000 events. They
  verify the 20-message and 100-event caps, Gmail's 16 KiB text projection
  limit, and that the bounded requested page is used even when a simulated
  provider returns more. These fixtures use no real account or credential;
  Apple IMAP now also presents a synthetic 2,000-message mailbox and verifies
  that only the newest 20 metadata records are fetched. Apple CalDAV retains
  separate protocol/parser budget fixtures.
- 2026-10-01: Reverified the current branch: all 109 mail/calendar unit tests
  and its state-registry test pass; all 11 Core mail/calendar boundary tests,
  13 server mail/calendar unit tests, 4 owner-authenticated HTTP tests, 5
  isolated parser-worker tests, and the interrupted-watch restart recovery
  test pass. The production web build, workspace formatting check, and
  `git diff --check` pass. This verifies synthetic/provider-double behavior,
  not live provider conformance or the sustained 24-hour service acceptance;
  those remain open. The local preview uses the separate empty workspace
  `~/vak-home/.mail-calendar-routine-preview-check`; its isolated `VAK_HOME`
  data profile is `~/Library/Application Support/vak/mail-calendar-routine-preview-check`.
  `vak-home` is the workspace, not the data home. No account or credential has
  been connected.
