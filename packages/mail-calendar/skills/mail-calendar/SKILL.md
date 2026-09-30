---
name: mail-calendar
description: Helps people manage email and calendar account connections and accurately explains which mail and calendar actions are currently available.
---

# Email and calendar

## Account setup

- Direct the person to **Settings → Email and calendar** to connect or
  disconnect an account. Account connections belong to the selected Agent.
- Explain that connecting an account does not grant every Agent access and
  does not authorize an email send or calendar change.
- Google and Microsoft account linking uses delegated sign-in. Apple iCloud
  credentials are currently saved as `connected_unverified`; they have not
  been verified and are not available to Agents.
- For an account that needs cleanup, direct the person back to its account row
  in Settings. Do not ask them to paste passwords, OAuth codes, access tokens,
  or refresh tokens into chat.

## Current action boundary

This package currently provides owner-managed account setup and cleanup only.
It does not provide tools to read email or calendar content, prepare drafts,
send messages, change events, create previews, or run scheduled or continuous
routines. Do not claim to have read, changed, sent, scheduled, or monitored
anything in an account.

If asked to perform one of those actions, explain that the mail/calendar
operation is not available yet and that the account can be managed in Settings.
Do not work around this boundary with Bash, arbitrary HTTP requests, an MCP
server, another Agent, another connected-service credential, or a provider's
web interface. A connected account or provider grant is not itself permission
for an Agent action.

When these operations become available, follow their per-call account,
audience, capability, permission, and Review requirements. An email send or
calendar mutation requires approval of the exact provider effect and its
recipients or attendees.
