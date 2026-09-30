# Mail and calendar package

This is a native, skills-only Vakyartha plugin package. It helps an Agent
explain account setup and the current availability boundary. It does not add
provider access, commands, MCP servers, network access, or executable code.

From the repository root, inspect and install it with the existing plugin
lifecycle:

```sh
vak plugin inspect packages/mail-calendar
vak plugin install packages/mail-calendar --scope user
```

Installation does not enable the package or grant a permission. Review and
enable it through Settings → Capabilities if the person wants the account
setup guidance in their Agent. Account linking and cleanup remain in Settings
→ Email and calendar.

The owner can preview bounded Gmail and Microsoft inbox, event, and free/busy
results in Settings. A broker-owned Agent tool also reads these accounts on the
local owner surface after exact Agent and capability checks; channel audiences
are blocked until an explicit share flow exists. Apple remains unverified and
unavailable. A bounded Agent-vault working area now stores local email and
event drafts with revision checks and disconnect cleanup. The first scheduled
read-only routine slice uses the existing TaskDef scheduler and is scoped to
one Agent revision, one account, and selected read operations. A scheduled
email watch can deduplicate a bounded set of seen message IDs in the encrypted
Agent vault and skip model dispatch when a content-free ID poll finds no new
items. It may miss messages outside the provider's latest-item window and
does not coordinate multiple service instances. It is not reliable continuous
monitoring. The first provider effect is plain-text email sending from an
unchanged saved candidate on the owner Settings surface. It requires a
separately selected Google or Microsoft send grant, Core permission approval,
and explicit review of the exact recipients, subject, and message. The
provider accepting a request is not delivery confirmation; ambiguous outcomes
cannot be retried. A limited timed event create is also available for Google
and Microsoft from an unchanged saved candidate. It has no attendees,
recurrence, or reminders and requires its own selected calendar-write grant,
Core permission approval, and exact review. The provider scope can authorize
more than this create-only operation; the credential remains private and the
broker enforces the narrower contract. Google also supports an exact-reviewed
update for public standalone timed events without attendees. It rechecks the
source ETag and conditionally patches only event fields; stale versions require
a fresh draft. Microsoft updates, cancel/RSVP, Agent/model initiated effects,
attachments, the third provider, multi-instance routine leases, and reliable continuous
service recovery remain unimplemented. This uses current 4.x storage. Before a
read, the product explains that disconnect removes the
saved credential and blocks future reads, while content recorded in append-only
Agent session history cannot currently be erased. This package does not claim
account-content crypto-shredding; that requires future data-architecture
lifecycle work.
