# Plan — secure mail and calendar package

Status: **Stage 0 and Stage 1A Agent/account linking complete; bounded
Google/Microsoft owner previews, the broker-owned local Agent read tool, and
the first Stage 2 increment (bounded Agent-vault local drafts with revisioned
save/delete and disconnect cleanup) are implemented on `codex/mail-calendar`;
the first Stage 4 increment adds scheduled read-only routines and a best-effort
scheduled mail watch on `TaskDef`; the first Stage 3 increment adds an
owner-confirmed, permission-checked, digest-bound plain email send and a
limited timed event create for Google and Microsoft without attendees,
recurrence, or reminders.
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

## Scope and provider matrix

The first three provider targets are Google Workspace/Gmail, Microsoft
365/Outlook, and Apple iCloud Mail/Calendar. Providers share a typed domain
contract and Review flow, but not an authentication or transport
implementation.

| Provider | Authentication and APIs | Initial security boundary |
|---|---|---|
| Google Workspace | Delegated OAuth authorization code with PKCE; Gmail API and Calendar API | Incremental granted-scope verification; separate read and effect capabilities; provider OAuth review requirements are a release gate |
| Microsoft 365 / Outlook | Delegated Entra OAuth with PKCE; Microsoft Graph | `Mail.Read`, `Calendars.ReadBasic` for free/busy, and `Calendars.Read` for event details; no tenant-wide application permissions. Graph `getSchedule` does not support personal Microsoft accounts, so free/busy must report unsupported for that account type unless a separately reviewed least-privilege adapter is available. |
| Apple iCloud | Apple Account authorization when a supported client flow is available; app-specific-password fallback for IMAP/SMTP and CalDAV | The current implementation uses the local app-specific-password fallback. It grants broader authority than selected Vak capabilities, so content access and effects stay disabled until a separately reviewed adapter and credential verification contract exist. Apple documents Account & Organizational Data Sharing OAuth for Apple School Manager Roster API scopes, not iCloud Mail/Calendar; EventKit is a native on-device calendar permission, not an unattended server grant. |

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
and [Microsoft Graph create event](https://learn.microsoft.com/graph/api/user-post-events?view=graph-rest-1.0).

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
and Microsoft, and the iCloud app-specific credential form. Provider clients use
fixed hosts; OAuth tokens and the iCloud credential remain in the credential
service. OAuth refresh preserves the existing grant ceiling and rotates
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

The first slice implements owner-only bounded inbox, event, and free/busy
previews for Google and Microsoft through fixed-host, read-only adapters. A
broker-owned Agent tool now offers bounded reads on the local owner surface;
channel audiences fail closed until explicitly shareable grants are built.
The tool never exposes vault handles to workers. Returned provider content is
model-visible and enters append-only session history, which current storage
cannot selectively erase. The UI and tool description disclose this before
reads. Do not claim complete account deletion. Provider health/freshness,
citations and safe prompt-injection handling remain to be completed. The
provider exit requirements for
bounded responses, no remote HTML loads, prompt-injection containment,
revocation during a read, and no message-read mutation apply here. Each
provider must return useful structured views or an explicit unsupported or
reauthentication state.

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
broker enforces the narrower action. Event updates, cancellations, RSVP, and
provider reconciliation remain disabled.

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
account, and fences reads. An optional mail watch deduplicates a bounded
rolling set of message IDs in the encrypted Agent vault. A content-free
provider-ID poll skips model dispatch when the bounded latest-message window
contains no unseen IDs; only a triggered run fetches message content through
the brokered tool. This is best-effort polling: it has no provider-native
cursor, can miss messages outside the returned window, does not coordinate
competing service instances with a durable lease, and has no freshness
guarantee. The ordinary Vakyartha service must remain running for schedules to
fire. Durable trigger leases, push subscriptions, awake-host health, and
restart/recovery acceptance are not implemented yet. Current-storage session
history is append-only and cannot be selectively erased; M7 remains the
deletion gate.

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

## Progress log

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
