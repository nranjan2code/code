---
name: mail-calendar
description: Helps people manage email and calendar account connections and accurately explains which mail and calendar actions are currently available.
---

# Email and calendar

## Account setup

- Direct the person to **Settings → Email and calendar** to connect or
  disconnect an account. Account connections belong to the selected Agent.
- Explain that connecting an account does not grant every Agent access.
  A selected send or calendar-write capability still requires owner Review
  and confirmation for each supported effect.
- Google and Microsoft account linking uses delegated sign-in. Apple iCloud
  credentials are currently saved as `connected_unverified`; they have not
  been verified and are not available to Agents.
- For an account that needs cleanup, direct the person back to its account row
  in Settings. Do not ask them to paste passwords, OAuth codes, access tokens,
  or refresh tokens into chat.

## Current action boundary

This package contributes guidance only; it declares no provider tools. In the
application, the owner can preview bounded Google/Microsoft data, save local
email/event drafts, and configure scoped scheduled read routines. Agent reads
are brokered and currently limited to the local owner surface. A first
plain-text email send is available only from the owner Settings Review flow,
with an explicit provider send grant, a fresh exact-candidate confirmation,
and the Agent Core permission decision. Agents cannot dispatch that effect
through this package. Provider acceptance does not prove delivery, and an
ambiguous attempt must not be retried. A limited timed event create is also
available for Google/Microsoft through the owner Settings Review flow. It has
no attendees, recurrence, or reminders. Google also allows exact-reviewed
updates to public standalone timed events without attendees; it rechecks and
conditionally matches the source ETag. Microsoft event updates, cancellations,
and RSVP are unavailable. Calendar-write provider consent is broader than the
supported operations. Attachments, Apple content, and reliable continuous service are
not available.

If asked to send, direct the owner to review and confirm the exact saved draft
in Settings. Do not claim an Agent sent it or work around this boundary with
Bash, arbitrary HTTP requests, an MCP server, another Agent, another
connected-service credential, or a provider's web interface. A connected
account or provider grant is not itself permission for an Agent action.

Follow per-call account, audience, capability, permission, and Review
requirements. An email send or event create requires approval of the exact
provider effect. Never add attendees to an event create or update.
