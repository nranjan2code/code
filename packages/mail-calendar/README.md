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

Mail/calendar content reads, citations, drafts, previews, provider effects,
scheduled work, and continuous routines are not available yet. These remain
behind the data-architecture M7 gate; the package skill must not imply that a
connected account is usable by an Agent.
